//! Same-store resource transactions and execution-owned lifetime holds.
pub(crate) mod evaluation;
mod execution;
pub(crate) mod integrity;
mod uses;
pub(crate) use execution::{
    accept_serving_resources, apply_run_resources, enter_serving_resources,
    terminal_serving_resources,
};

use crate::{
    RedbStore, codec, error, json,
    schema::{MANAGED_INSTALLATIONS, MANAGED_RECEIPTS, MANAGED_TRANSITIONS},
};
use milkdrift_authority::AuthorityDecisionSnapshot;
use milkdrift_capability::managed::{
    MANAGED_SCHEMA_VERSION, ManagedAction, ManagedName, ManagedRequest,
};
use milkdrift_persistence::{
    PageSize, PersistenceError,
    managed::{
        InstallationRecord, ManagedChange, ManagedChangePhase, ManagedObservation,
        ManagedResourceStore, ManagedTransition, ManagedUse, QuiescenceEvidence, ResourceReceipt,
    },
};
use redb::{ReadableTable, ReadableTableMetadata};

const MAX_INSTALLATIONS: u64 = 1024;

// Repeat the semantic owner's scope binding inside the transaction: concurrent callers can both
// pass its initial receipt lookup before one of them commits the actor/command key.
fn same_receipt_scope(
    prior: &ResourceReceipt,
    request: &ManagedRequest,
    authorization: &AuthorityDecisionSnapshot,
) -> bool {
    let old = prior.authorization.request();
    let new = authorization.request();
    prior.request == *request
        && old.actor == new.actor
        && old.grant == new.grant
        && old.grant_revision == new.grant_revision
        && old.grant_digest == new.grant_digest
        && old.revocation_generation == new.revocation_generation
        && old.resources.workflow == new.resources.workflow
        && old.resources.run == new.resources.run
        && old.resources.workspace_scope == new.resources.workspace_scope
}

fn validate_authorization(
    request: &ManagedRequest,
    decision: &AuthorityDecisionSnapshot,
) -> Result<(), PersistenceError> {
    let authority = decision.request();
    if !decision.is_allowed()
        || authority.operation != milkdrift_authority::AuthorityOperation::AdministerCapabilities
        || authority
            .resources
            .capability
            .as_ref()
            .is_none_or(|id| id.as_str() != format!("managed.{}", request.installation))
        || authority
            .resources
            .capability_operation
            .as_ref()
            .is_none_or(|operation| operation.as_str() != request.operation())
    {
        return Err(invalid(
            "managed intent requires exact allowing installation authority",
        ));
    }
    Ok(())
}

pub(crate) fn verify_link(
    read: &redb::ReadTransaction,
    child: &str,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let link: execution::ChildLink = json::decode(bytes, "managed child link")?;
    let table = read
        .open_table(crate::schema::RUN_EVENTS)
        .map_err(error::redb)?;
    let key = codec::run_sequence(link.parent.as_str(), link.sequence)?;
    let bytes = table
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| invalid("child link lost authoritative event"))?;
    let event = crate::journal::decode_stored_event(bytes.value())?;
    if !matches!(event.kind(), milkdrift_persistence::RunEventKind::SubworkflowCreated { child_run, .. } if child_run.as_str() == child)
    {
        return Err(invalid("managed child link conflicts with runtime event"));
    }
    Ok(())
}

pub(super) fn invalid(message: &str) -> PersistenceError {
    PersistenceError::InvalidDocument(message.to_owned())
}
pub(super) fn conflict(identity: &str) -> PersistenceError {
    PersistenceError::ImmutableConflict {
        entity: "managed_resource",
        identity: identity.to_owned(),
    }
}
pub(super) fn busy(message: &str) -> PersistenceError {
    PersistenceError::Storage {
        class: milkdrift_persistence::StorageFailureClass::OwnerBusy,
        message: message.to_owned(),
    }
}
pub(super) fn bump(record: &mut InstallationRecord) -> Result<(), PersistenceError> {
    record.version = record
        .version
        .checked_add(1)
        .ok_or_else(|| invalid("managed version exhausted"))?;
    Ok(())
}
pub(crate) fn decode_record(bytes: &[u8]) -> Result<InstallationRecord, PersistenceError> {
    let r: InstallationRecord = json::decode(bytes, "managed installation")?;
    r.validate()?;
    Ok(r)
}
pub(super) fn read_record(
    write: &redb::WriteTransaction,
    name: &str,
) -> Result<Option<InstallationRecord>, PersistenceError> {
    write
        .open_table(MANAGED_INSTALLATIONS)
        .map_err(error::redb)?
        .get(name)
        .map_err(error::redb)?
        .map(|v| decode_record(v.value()))
        .transpose()
}
pub(super) fn put_record(
    write: &redb::WriteTransaction,
    record: &InstallationRecord,
) -> Result<(), PersistenceError> {
    record.validate()?;
    let bytes = json::encode(record, "managed installation")?;
    write
        .open_table(MANAGED_INSTALLATIONS)
        .map_err(error::redb)?
        .insert(record.name.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}
fn save_transition(
    write: &redb::WriteTransaction,
    name: &ManagedName,
    transition: &ManagedTransition,
) -> Result<(), PersistenceError> {
    let key = format!("{}:{}", name, transition.change.identity);
    let bytes = json::encode(transition, "managed transition")?;
    write
        .open_table(MANAGED_TRANSITIONS)
        .map_err(error::redb)?
        .insert(key.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

impl ManagedResourceStore for RedbStore {
    fn begin_managed_evaluation(
        &self,
        request: &ManagedRequest,
        authorization: &AuthorityDecisionSnapshot,
        evidence: &milkdrift_workspace::CandidateEvaluation,
    ) -> Result<ResourceReceipt, PersistenceError> {
        evaluation::begin(self, request, authorization, evidence)
    }
    fn finish_managed_evaluation(
        &self,
        evidence: &milkdrift_workspace::CandidateEvaluation,
    ) -> Result<(), PersistenceError> {
        evaluation::finish(self, evidence)
    }
    fn managed_evaluation(
        &self,
        identity: &str,
    ) -> Result<Option<milkdrift_workspace::CandidateEvaluation>, PersistenceError> {
        evaluation::read(self, identity)
    }
    fn managed_installation(
        &self,
        name: &ManagedName,
    ) -> Result<Option<InstallationRecord>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        read.open_table(MANAGED_INSTALLATIONS)
            .map_err(error::redb)?
            .get(name.as_str())
            .map_err(error::redb)?
            .map(|v| decode_record(v.value()))
            .transpose()
    }
    fn managed_installations(
        &self,
        after: Option<&ManagedName>,
        limit: PageSize,
    ) -> Result<Vec<InstallationRecord>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read
            .open_table(MANAGED_INSTALLATIONS)
            .map_err(error::redb)?;
        let bounds = (
            after.map_or(std::ops::Bound::Unbounded, |n| {
                std::ops::Bound::Excluded(n.as_str())
            }),
            std::ops::Bound::Unbounded,
        );
        table
            .range::<&str>(bounds)
            .map_err(error::redb)?
            .take(limit.get() as usize)
            .map(|r| {
                let (_, v) = r.map_err(error::redb)?;
                decode_record(v.value())
            })
            .collect()
    }
    fn managed_receipt(
        &self,
        actor: &str,
        command: &ManagedName,
    ) -> Result<Option<ResourceReceipt>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let key = codec::pair(actor, command.as_str())?;
        read.open_table(MANAGED_RECEIPTS)
            .map_err(error::redb)?
            .get(key.as_slice())
            .map_err(error::redb)?
            .map(|v| {
                integrity::receipt(key.as_slice(), v.value())?;
                json::decode(v.value(), "managed receipt")
            })
            .transpose()
    }
    fn begin_managed_change(
        &self,
        request: &ManagedRequest,
        authorization: &AuthorityDecisionSnapshot,
        change: &ManagedChange,
    ) -> Result<ResourceReceipt, PersistenceError> {
        request.validate().map_err(|e| invalid(&e.to_string()))?;
        validate_authorization(request, authorization)?;
        change.candidate.validate()?;
        if !authorization.is_allowed()
            || change.steps.is_empty()
            || change.steps.len() > 16
            || !milkdrift_contracts::is_canonical_blake3_digest(&change.identity)
            || change.generation == 0
        {
            return Err(invalid("invalid managed transition acceptance"));
        }
        let write = self.database().begin_write().map_err(error::redb)?;
        let key = codec::pair(
            authorization.request().actor.as_str(),
            request.command.as_str(),
        )?;
        if let Some(bytes) = write
            .open_table(MANAGED_RECEIPTS)
            .map_err(error::redb)?
            .get(key.as_slice())
            .map_err(error::redb)?
        {
            let prior: ResourceReceipt = json::decode(bytes.value(), "managed receipt")?;
            if !same_receipt_scope(&prior, request, authorization) {
                return Err(conflict(request.command.as_str()));
            }
            return Ok(prior);
        }
        let existing = read_record(&write, request.installation.as_str())?;
        if existing.as_ref().map_or(0, |r| r.version) != request.expected_version {
            return Err(conflict(request.installation.as_str()));
        }
        if existing.is_none()
            && write
                .open_table(MANAGED_INSTALLATIONS)
                .map_err(error::redb)?
                .len()
                .map_err(error::redb)?
                >= MAX_INSTALLATIONS
        {
            return Err(busy("managed installation namespace limit reached"));
        }
        let mut record = existing.unwrap_or(InstallationRecord {
            schema_version: MANAGED_SCHEMA_VERSION,
            name: request.installation.clone(),
            version: 0,
            generation: 0,
            current: None,
            pending: None,
            observation: None,
            desired_running: false,
            admission_open: false,
            removed: false,
            uses: Vec::new(),
        });
        if record.removed {
            return Err(conflict("removed installation identity cannot be reused"));
        }
        if matches!(request.action, ManagedAction::Recover {}) {
            let pending = record
                .pending
                .as_mut()
                .ok_or_else(|| conflict("no pending change"))?;
            if pending.change != *change
                || !matches!(pending.phase, ManagedChangePhase::Uncertain { .. })
            {
                return Err(conflict(
                    "recovery requires an uncertain, unchanged accepted intent",
                ));
            }
            pending.phase = ManagedChangePhase::Pending {};
        } else {
            if record.pending.is_some() {
                return Err(busy("installation has an unresolved platform change"));
            }
            if !record.uses.is_empty() {
                return Err(busy(
                    "accepted resource users block maintenance; inspect exact blockers",
                ));
            }
            if let Some(current) = &record.current
                && (current.ownership != change.candidate.ownership
                    || current.platform_owner != change.candidate.platform_owner)
            {
                return Err(conflict("ownership cannot change during maintenance"));
            }
            record.pending = Some(ManagedTransition {
                change: change.clone(),
                next_step: 0,
                evidence: Vec::new(),
                phase: ManagedChangePhase::Pending {},
            });
        }
        // Recovery above already compared the entire saved intent. It resumes that exact
        // publication; it cannot substitute a newly supplied candidate or authorization.
        if !matches!(request.action, ManagedAction::Recover {})
            && let Some(current) = &record.current
        {
            check_protected_change(
                &write,
                current,
                record.generation,
                request,
                authorization,
                change,
            )?;
        }
        record.admission_open = false;
        bump(&mut record)?;
        let receipt = ResourceReceipt {
            schema_version: MANAGED_SCHEMA_VERSION,
            request: request.clone(),
            authorization: authorization.clone(),
            response: record.view(),
        };
        let bytes = json::encode(&receipt, "managed receipt")?;
        write
            .open_table(MANAGED_RECEIPTS)
            .map_err(error::redb)?
            .insert(key.as_slice(), bytes.as_slice())
            .map_err(error::redb)?;
        if let Some(pending) = &record.pending {
            save_transition(&write, &record.name, pending)?;
        }
        put_record(&write, &record)?;
        self.faults
            .check(crate::fault::FaultPoint::BeforeManagedCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults
            .check(crate::fault::FaultPoint::AfterManagedCommit)?;
        Ok(receipt)
    }
    fn advance_managed_change(
        &self,
        installation: &ManagedName,
        transition: &str,
        expected_step: u32,
        observation: ManagedObservation,
    ) -> Result<InstallationRecord, PersistenceError> {
        if !milkdrift_contracts::is_canonical_blake3_digest(&observation.digest)
            || observation.summary.len() > 512
        {
            return Err(invalid("invalid platform observation"));
        }
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut record = read_record(&write, installation.as_str())?
            .ok_or_else(|| conflict("missing installation"))?;
        let pending = record
            .pending
            .as_mut()
            .ok_or_else(|| conflict("no pending transition"))?;
        if pending.change.identity != transition
            || pending.next_step != expected_step
            || !matches!(pending.phase, ManagedChangePhase::Pending {})
        {
            return Err(conflict("stale platform callback"));
        }
        pending.evidence.push(observation.clone());
        pending.next_step += 1;
        let complete = pending.next_step as usize == pending.change.steps.len();
        if complete {
            pending.phase = ManagedChangePhase::Complete {};
        }
        save_transition(&write, &record.name, pending)?;
        if complete {
            record.current = Some(pending.change.candidate.clone());
            record.generation = pending.change.generation;
            record.removed = pending.change.removing;
            record.desired_running = pending.change.running;
            record.admission_open = !record.removed;
            record.pending = None;
        }
        record.observation = Some(observation);
        bump(&mut record)?;
        put_record(&write, &record)?;
        self.faults
            .check(crate::fault::FaultPoint::BeforeManagedCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults
            .check(crate::fault::FaultPoint::AfterManagedCommit)?;
        Ok(record)
    }
    fn fail_managed_change(
        &self,
        installation: &ManagedName,
        transition: &str,
        expected_step: u32,
        diagnostic: &str,
    ) -> Result<(), PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut record = read_record(&write, installation.as_str())?
            .ok_or_else(|| conflict("missing installation"))?;
        let pending = record
            .pending
            .as_mut()
            .ok_or_else(|| conflict("missing pending transition"))?;
        if pending.change.identity != transition || pending.next_step != expected_step {
            return Err(conflict("stale failure callback"));
        }
        pending.phase = ManagedChangePhase::Uncertain {
            diagnostic: milkdrift_contracts::truncate_utf8(diagnostic, 512).to_owned(),
        };
        save_transition(&write, &record.name, pending)?;
        bump(&mut record)?;
        put_record(&write, &record)?;
        write.commit().map_err(error::redb)
    }
    fn managed_use(&self, use_id: &str) -> Result<Option<ManagedUse>, PersistenceError> {
        uses::read_use(self, use_id)
    }
    fn begin_managed_resolution(
        &self,
        request: &ManagedRequest,
        authorization: &AuthorityDecisionSnapshot,
    ) -> Result<ResourceReceipt, PersistenceError> {
        uses::begin_resolution(self, request, authorization)
    }
    fn enter_managed_use(
        &self,
        use_id: &str,
        expected_claim: u64,
        physical_identity: &str,
    ) -> Result<ManagedUse, PersistenceError> {
        uses::enter(self, use_id, expected_claim, physical_identity)
    }
    fn quiesce_managed_use(
        &self,
        use_id: &str,
        expected_claim: u64,
        evidence: &QuiescenceEvidence,
    ) -> Result<(), PersistenceError> {
        uses::quiesce(self, use_id, expected_claim, evidence)
    }
    fn release_managed_use(
        &self,
        use_id: &str,
        expected_claim: u64,
    ) -> Result<(), PersistenceError> {
        uses::release(self, use_id, expected_claim)
    }
    fn transfer_managed_editing(
        &self,
        request: &ManagedRequest,
        authorization: &AuthorityDecisionSnapshot,
    ) -> Result<ResourceReceipt, PersistenceError> {
        uses::transfer(self, request, authorization)
    }
    fn verify_managed_integrity(&self) -> Result<(), PersistenceError> {
        integrity::verify(self)
    }
}

fn check_protected_change(
    write: &redb::WriteTransaction,
    current: &milkdrift_persistence::managed::ApprovedSetup,
    generation: u64,
    request: &ManagedRequest,
    authorization: &AuthorityDecisionSnapshot,
    change: &ManagedChange,
) -> Result<(), PersistenceError> {
    let Some(protected) = &current.protection else {
        if change.candidate.protection.is_some() {
            return Err(conflict("protect a distinct new installation"));
        }
        return Ok(());
    };
    let candidate = change
        .candidate
        .protection
        .as_ref()
        .ok_or_else(|| conflict("protected target cannot become unprotected"))?;
    if candidate.agreement != protected.agreement
        || candidate.policy != protected.policy
        || change.candidate.recipe != current.recipe
    {
        return Err(conflict(
            "protected target policy cannot be changed by lifecycle update",
        ));
    }
    if let ManagedAction::Publish { evaluation } = &request.action {
        let table = write
            .open_table(crate::schema::MANAGED_EVALUATIONS)
            .map_err(error::redb)?;
        let evidence = table
            .get(evaluation.as_str())
            .map_err(error::redb)?
            .map(|b| evaluation::decode(evaluation, b.value()))
            .transpose()?
            .ok_or_else(|| conflict("unknown trusted evaluation"))?;
        protected
            .policy
            .require_pass(&evidence, authorization.request().evaluated_at.get())
            .map_err(|e| invalid(&e.to_string()))?;
        if change.entry_authorization.as_ref() != Some(authorization)
            || evidence.subject.generation != generation
            || candidate.evidence.as_ref() != Some(&evidence)
            || evidence.subject.target != request.installation
            || evidence.subject.generation.checked_add(1) != Some(change.generation)
            || evidence.subject.configuration != current.recipe.digest
        {
            return Err(conflict(
                "publication candidate, target or generation differs",
            ));
        }
    } else if candidate != protected {
        return Err(conflict("raw lifecycle cannot replace protected candidate"));
    }
    Ok(())
}
