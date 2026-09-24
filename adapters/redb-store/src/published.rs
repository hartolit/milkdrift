//! Immutable method generations; catalog admission changes never delete accepted definitions.
use crate::{
    RedbStore, error, json,
    schema::{
        PUBLISHED_LOCAL_LINKS, PUBLISHED_LOCAL_PENDING, PUBLISHED_METHOD_HEADS, PUBLISHED_METHODS,
        RUN_EVENTS,
    },
};
use milkdrift_authority::{AuthorityDecisionSnapshot, AuthorityOperation};
use milkdrift_capability::CapabilityId;
use milkdrift_persistence::{
    PageSize, PersistenceError,
    published::{
        PublishedInvocationPlan, PublishedInvocationSource, PublishedMethod, PublishedMethodRecord,
        PublishedMethodStore,
    },
};
use redb::{ReadableTable, ReadableTableMetadata};

const MAX_PUBLICATIONS: u64 = 4096;

fn key(capability: &CapabilityId, generation: u64) -> String {
    format!("{capability}/{generation:020}")
}
fn invalid(message: &str) -> PersistenceError {
    PersistenceError::InvalidDocument(message.to_owned())
}
fn conflict(capability: &CapabilityId) -> PersistenceError {
    PersistenceError::ImmutableConflict {
        entity: "published_method",
        identity: capability.to_string(),
    }
}
fn authorize(
    decision: &AuthorityDecisionSnapshot,
    capability: &CapabilityId,
    operation: &str,
) -> Result<(), PersistenceError> {
    let request = decision.request();
    if !decision.is_allowed()
        || request.operation != AuthorityOperation::AdministerCapabilities
        || request.resources.capability.as_ref() != Some(capability)
        || request
            .resources
            .capability_operation
            .as_ref()
            .is_none_or(|id| id.as_str() != operation)
    {
        return Err(invalid(
            "publication requires exact capability administration authority",
        ));
    }
    Ok(())
}
fn decode(bytes: &[u8]) -> Result<PublishedMethodRecord, PersistenceError> {
    let record: PublishedMethodRecord = json::decode(bytes, "published method")?;
    record.method.validate()?;
    if record.version == 0 || !record.authorization.is_allowed() {
        return Err(invalid("invalid publication state or authority"));
    }
    Ok(record)
}
impl milkdrift_persistence::published::PublishedInvocationStore for RedbStore {
    fn published_local_pending(
        &self,
        source: &PublishedInvocationSource,
    ) -> Result<bool, PersistenceError> {
        let read = self.database.begin_read().map_err(error::redb)?;
        let table = read
            .open_table(PUBLISHED_LOCAL_PENDING)
            .map_err(error::redb)?;
        Ok(table
            .get(local_key(source)?.as_str())
            .map_err(error::redb)?
            .is_some())
    }

    fn published_local_page(
        &self,
        after: Option<&PublishedInvocationSource>,
        limit: PageSize,
    ) -> Result<
        (
            Vec<PublishedInvocationPlan>,
            Option<PublishedInvocationSource>,
        ),
        PersistenceError,
    > {
        let read = self.database.begin_read().map_err(error::redb)?;
        let pending = read
            .open_table(PUBLISHED_LOCAL_PENDING)
            .map_err(error::redb)?;
        let after_key = after.map(local_key).transpose()?;
        let range = match after_key.as_deref() {
            Some(key) => {
                pending.range::<&str>((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
            }
            None => pending.range::<&str>(..),
        }
        .map_err(error::redb)?;
        let mut rows = range.take(limit.get() as usize + 1);
        let mut plans = Vec::new();
        for _ in 0..limit.get() {
            let Some(row) = rows.next() else {
                return Ok((plans, None));
            };
            let (key, sequence) = row.map_err(error::redb)?;
            let source: PublishedInvocationSource =
                serde_json::from_str(key.value()).map_err(|e| invalid(&e.to_string()))?;
            plans.push(local_plan(&read, &source, sequence.value())?);
        }
        let next = rows
            .next()
            .transpose()
            .map_err(error::redb)?
            .and_then(|_| plans.last().map(|p| p.source.clone()));
        Ok((plans, next))
    }

    fn published_invocation(
        &self,
        source: &milkdrift_persistence::published::PublishedInvocationSource,
    ) -> Result<Option<milkdrift_persistence::published::PublishedInvocationPlan>, PersistenceError>
    {
        self.read_published_invocation(source)
    }
}

impl PublishedMethodStore for RedbStore {
    fn publish_method(
        &self,
        method: &PublishedMethod,
        expected_previous_version: Option<u64>,
        authorization: &AuthorityDecisionSnapshot,
        request: &milkdrift_persistence::IntegrityDigest,
    ) -> Result<PublishedMethodRecord, PersistenceError> {
        method.validate()?;
        let id = method.descriptor.identity();
        authorize(authorization, id, "method.publish")?;
        let generation = method.descriptor.descriptor_revision();
        let write = self.database.begin_write().map_err(error::redb)?;
        let record = {
            let mut table = write.open_table(PUBLISHED_METHODS).map_err(error::redb)?;
            let mut heads = write
                .open_table(PUBLISHED_METHOD_HEADS)
                .map_err(error::redb)?;
            let exact = key(id, generation);
            if let Some(prior) = table.get(exact.as_str()).map_err(error::redb)? {
                let mut prior = decode(prior.value())?;
                if prior.method != *method
                    || &prior.publication_request != request
                    || prior.expected_previous_version != expected_previous_version
                {
                    return Err(conflict(id));
                }
                prior.version = 1;
                prior.retired = false;
                prior.retirement_request = None;
                return Ok(prior);
            }
            if table.len().map_err(error::redb)? >= MAX_PUBLICATIONS {
                return Err(invalid("publication retention bound reached"));
            }
            let previous = heads
                .get(id.as_str())
                .map_err(error::redb)?
                .map(|v| v.value());
            match previous {
                None if expected_previous_version.is_none() && generation == 1 => {}
                Some(old_generation) if old_generation.checked_add(1) == Some(generation) => {
                    let old_key = key(id, old_generation);
                    let old = table
                        .get(old_key.as_str())
                        .map_err(error::redb)?
                        .ok_or_else(|| invalid("publication head missing"))?;
                    if Some(decode(old.value())?.version) != expected_previous_version {
                        return Err(conflict(id));
                    }
                }
                _ => return Err(conflict(id)),
            }
            let record = PublishedMethodRecord {
                publication_request: request.clone(),
                expected_previous_version,
                retirement_request: None,
                method: method.clone(),
                version: 1,
                retired: false,
                authorization: authorization.clone(),
            };
            let bytes = json::encode(&record, "published method")?;
            table
                .insert(exact.as_str(), bytes.as_slice())
                .map_err(error::redb)?;
            heads.insert(id.as_str(), generation).map_err(error::redb)?;
            record
        };
        write.commit().map_err(error::redb)?;
        Ok(record)
    }
    fn published_method(
        &self,
        capability: &CapabilityId,
        generation: u64,
    ) -> Result<Option<PublishedMethodRecord>, PersistenceError> {
        let read = self.database.begin_read().map_err(error::redb)?;
        let table = read.open_table(PUBLISHED_METHODS).map_err(error::redb)?;
        table
            .get(key(capability, generation).as_str())
            .map_err(error::redb)?
            .map(|v| decode(v.value()))
            .transpose()
    }
    fn published_methods(
        &self,
        after: Option<(&CapabilityId, u64)>,
        limit: PageSize,
    ) -> Result<Vec<PublishedMethodRecord>, PersistenceError> {
        let read = self.database.begin_read().map_err(error::redb)?;
        let table = read.open_table(PUBLISHED_METHODS).map_err(error::redb)?;
        let after = after.map(|(id, generation)| key(id, generation));
        let range = match after.as_deref() {
            Some(key) => {
                table.range::<&str>((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
            }
            None => table.range::<&str>(..),
        }
        .map_err(error::redb)?;
        range
            .take(limit.get() as usize)
            .map(|row| {
                let (_, bytes) = row.map_err(error::redb)?;
                decode(bytes.value())
            })
            .collect()
    }
    fn retire_method(
        &self,
        capability: &CapabilityId,
        generation: u64,
        expected_version: u64,
        authorization: &AuthorityDecisionSnapshot,
        request: &milkdrift_persistence::IntegrityDigest,
    ) -> Result<PublishedMethodRecord, PersistenceError> {
        authorize(authorization, capability, "method.retire")?;
        let write = self.database.begin_write().map_err(error::redb)?;
        let record = {
            let mut table = write.open_table(PUBLISHED_METHODS).map_err(error::redb)?;
            let key = key(capability, generation);
            let mut record = table
                .get(key.as_str())
                .map_err(error::redb)?
                .map(|v| decode(v.value()))
                .transpose()?
                .ok_or_else(|| invalid("published method absent"))?;
            if record.retired {
                if record.retirement_request.as_ref() == Some(request)
                    && expected_version.checked_add(1) == Some(record.version)
                {
                    return Ok(record);
                }
                return Err(conflict(capability));
            }
            if record.version != expected_version {
                return Err(conflict(capability));
            }
            if !record.retired {
                record.retirement_request = Some(request.clone());
                record.retired = true;
                record.version = record
                    .version
                    .checked_add(1)
                    .ok_or_else(|| invalid("publication version exhausted"))?;
                let bytes = json::encode(&record, "published method")?;
                table
                    .insert(key.as_str(), bytes.as_slice())
                    .map_err(error::redb)?;
            }
            record
        };
        write.commit().map_err(error::redb)?;
        Ok(record)
    }
}

impl RedbStore {
    pub(crate) fn read_published_invocation(
        &self,
        source: &milkdrift_persistence::published::PublishedInvocationSource,
    ) -> Result<Option<milkdrift_persistence::published::PublishedInvocationPlan>, PersistenceError>
    {
        use milkdrift_persistence::published::PublishedInvocationSource;
        use milkdrift_persistence::{PeerExecutionSnapshot, PeerExecutionStore};
        match source {
            PublishedInvocationSource::Serving { caller, execution } => Ok(self
                .peer_execution(caller, execution)?
                .and_then(|record| match record {
                    PeerExecutionSnapshot::Hot(record) => record.published_invocation,
                    PeerExecutionSnapshot::Archived(record) => record.published_invocation,
                })),
            PublishedInvocationSource::Local { .. } => {
                let read = self.database.begin_read().map_err(error::redb)?;
                let links = read
                    .open_table(PUBLISHED_LOCAL_LINKS)
                    .map_err(error::redb)?;
                links
                    .get(local_key(source)?.as_str())
                    .map_err(error::redb)?
                    .map(|sequence| local_plan(&read, source, sequence.value()))
                    .transpose()
            }
        }
    }
}

fn local_key(source: &PublishedInvocationSource) -> Result<String, PersistenceError> {
    if !matches!(source, PublishedInvocationSource::Local { .. }) {
        return Err(invalid("local association requires a local source"));
    }
    serde_json::to_string(source).map_err(|e| invalid(&e.to_string()))
}

fn local_plan(
    read: &redb::ReadTransaction,
    source: &PublishedInvocationSource,
    sequence: u64,
) -> Result<PublishedInvocationPlan, PersistenceError> {
    let PublishedInvocationSource::Local { run, attempt } = source else {
        return Err(invalid("local index references a serving source"));
    };
    let key = crate::codec::run_sequence(
        run.as_str(),
        milkdrift_persistence::RunSequence::new(sequence),
    )?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let bytes = events
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| invalid("published index lost its journal event"))?;
    let event = crate::journal::decode_stored_event(bytes.value())?;
    match event.kind() {
        milkdrift_persistence::RunEventKind::PublishedInvocationPlanned {
            attempt: actual,
            plan,
        } if actual == attempt && &plan.source == source => {
            plan.validate()?;
            Ok(plan.as_ref().clone())
        }
        _ => Err(invalid("published index differs from its journal event")),
    }
}

/// Derived links and pending membership share the attempt's authoritative journal transaction.
pub(crate) fn apply_local_links(
    write: &redb::WriteTransaction,
    request: &milkdrift_persistence::AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    use milkdrift_persistence::RunEventKind;
    let mut links = write
        .open_table(PUBLISHED_LOCAL_LINKS)
        .map_err(error::redb)?;
    let mut pending = write
        .open_table(PUBLISHED_LOCAL_PENDING)
        .map_err(error::redb)?;
    for event in request.events() {
        match event.kind() {
            RunEventKind::PublishedInvocationPlanned { attempt, plan } => {
                let source = PublishedInvocationSource::Local {
                    run: request.receipt().run().clone(),
                    attempt: attempt.clone(),
                };
                if plan.source != source {
                    return Err(invalid(
                        "published association differs from its journal owner",
                    ));
                }
                validate_plan(write, plan)?;
                let key = local_key(&source)?;
                if links
                    .insert(key.as_str(), event.sequence().get())
                    .map_err(error::redb)?
                    .is_some()
                {
                    return Err(invalid("published attempt was already associated"));
                }
                pending
                    .insert(key.as_str(), event.sequence().get())
                    .map_err(error::redb)?;
            }
            RunEventKind::NodeTerminal { attempt, .. } => {
                let key = local_key(&PublishedInvocationSource::Local {
                    run: request.receipt().run().clone(),
                    attempt: attempt.clone(),
                })?;
                pending.remove(key.as_str()).map_err(error::redb)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Resolve the authoritative association while validating a child's atomic account binding.
pub(crate) fn association_in_transaction(
    write: &redb::WriteTransaction,
    source: &PublishedInvocationSource,
) -> Result<Option<PublishedInvocationPlan>, PersistenceError> {
    let plan = match source {
        PublishedInvocationSource::Local { run, attempt } => {
            let links = write
                .open_table(PUBLISHED_LOCAL_LINKS)
                .map_err(error::redb)?;
            let Some(sequence) = links
                .get(local_key(source)?.as_str())
                .map_err(error::redb)?
            else {
                return Ok(None);
            };
            let key = crate::codec::run_sequence(
                run.as_str(),
                milkdrift_persistence::RunSequence::new(sequence.value()),
            )?;
            let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
            let bytes = events
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| invalid("published account link lost its journal event"))?;
            match crate::journal::decode_stored_event(bytes.value())?.kind() {
                milkdrift_persistence::RunEventKind::PublishedInvocationPlanned {
                    attempt: actual,
                    plan,
                } if actual == attempt => Some(plan.as_ref().clone()),
                _ => return Err(invalid("published account link differs from its event")),
            }
        }
        PublishedInvocationSource::Serving { caller, execution } => {
            let records = write
                .open_table(crate::schema::PEER_EXECUTIONS)
                .map_err(error::redb)?;
            let Some(bytes) = records.get(execution.as_str()).map_err(error::redb)? else {
                return Ok(None);
            };
            let record: milkdrift_persistence::PeerExecutionRecord =
                json::decode(bytes.value(), "peer execution")?;
            if &record.caller != caller {
                return Err(invalid("published serving caller differs"));
            }
            record.published_invocation
        }
    };
    if let Some(plan) = &plan {
        plan.validate()?;
        if &plan.source != source {
            return Err(invalid("published association has another source"));
        }
    }
    Ok(plan)
}

/// Read an association using the integrity scanner's consistent snapshot, including archival.
pub(crate) fn association_read(
    read: &redb::ReadTransaction,
    source: &PublishedInvocationSource,
) -> Result<Option<PublishedInvocationPlan>, PersistenceError> {
    let plan = match source {
        PublishedInvocationSource::Local { .. } => {
            let links = read
                .open_table(PUBLISHED_LOCAL_LINKS)
                .map_err(error::redb)?;
            links
                .get(local_key(source)?.as_str())
                .map_err(error::redb)?
                .map(|sequence| local_plan(read, source, sequence.value()))
                .transpose()?
        }
        PublishedInvocationSource::Serving { caller, execution } => {
            let hot = read
                .open_table(crate::schema::PEER_EXECUTIONS)
                .map_err(error::redb)?;
            if let Some(bytes) = hot.get(execution.as_str()).map_err(error::redb)? {
                let record: milkdrift_persistence::PeerExecutionRecord =
                    json::decode(bytes.value(), "peer execution")?;
                if &record.caller != caller {
                    return Err(invalid("published link caller changed"));
                }
                record.published_invocation
            } else {
                let cold = read
                    .open_table(crate::schema::PEER_EXECUTION_TOMBSTONES)
                    .map_err(error::redb)?;
                let Some(bytes) = cold.get(execution.as_str()).map_err(error::redb)? else {
                    return Ok(None);
                };
                let record: milkdrift_persistence::PeerExecutionTombstone =
                    json::decode(bytes.value(), "peer execution tombstone")?;
                if &record.caller != caller {
                    return Err(invalid("published archived link caller changed"));
                }
                record.published_invocation
            }
        }
    };
    if let Some(plan) = &plan {
        plan.validate()?;
        if &plan.source != source {
            return Err(invalid("published source changed"));
        }
    }
    Ok(plan)
}

/// Resource acceptance must match the approved publication's exact catalog projection.
pub(crate) fn validate_managed_publication(
    write: &redb::WriteTransaction,
    snapshot: &milkdrift_capability::ResolvedCapabilitySnapshot,
) -> Result<(), PersistenceError> {
    let table = write.open_table(PUBLISHED_METHODS).map_err(error::redb)?;
    let bytes = table
        .get(key(snapshot.capability(), snapshot.descriptor_revision()).as_str())
        .map_err(error::redb)?
        .ok_or_else(|| invalid("unpublished managed capability"))?;
    let record = decode(bytes.value())?;
    if record.retired {
        return Err(invalid("managed publication retired before acceptance"));
    }
    snapshot
        .validate_against(&record.method.capability_descriptor()?)
        .map_err(|error| invalid(&error.to_string()))
}

/// Entry links are accepted only against their exact immutable implementation and allowance.
/// The control owner validates commands and inputs; persistence prevents a link from widening
/// the reviewed service identity, nesting or account when journal/serving entry is committed.
pub(crate) fn validate_plan(
    write: &redb::WriteTransaction,
    plan: &PublishedInvocationPlan,
) -> Result<(), PersistenceError> {
    plan.validate()?;
    let table = write.open_table(PUBLISHED_METHODS).map_err(error::redb)?;
    let key = key(&plan.capability, plan.generation);
    let value = table
        .get(key.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| invalid("published association references an absent implementation"))?;
    let record: PublishedMethodRecord = json::decode(value.value(), "published method")?;
    let method = &record.method;
    let expected = milkdrift_persistence::ControllerAccountDeclaration::for_published_invocation(
        plan.child_run.clone(),
        plan.invocation.clone(),
        method.digest()?,
        method.allowance.clone(),
    )?;
    if plan.method_digest != method.digest()?
        || plan.service != method.service
        || plan.allowance != expected
        || plan.maximum_depth != method.maximum_depth
    {
        return Err(invalid(
            "published association differs from its immutable service, allowance or depth",
        ));
    }
    Ok(())
}
