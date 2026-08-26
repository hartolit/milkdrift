use super::*;
use super::{
    discovery::{apply_indexes, record_artifact_references},
    queries::{decode_stored_event, validated_run_head},
    workspace::apply_workspace,
};
const COMMAND_RECORD_SCHEMA_VERSION: u32 = 2;

pub(crate) struct RunnableHeadState {
    pub(crate) previous_bytes: Option<Vec<u8>>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCommandRecord<'a> {
    schema_version: u32,
    command: &'a CommandId,
    run: &'a RunId,
    actor: &'a ActorRef,
    expected_sequence: RunSequence,
    submitted_at: TimestampMillis,
    canonical_document: &'a [u8],
    canonical_intent: &'a [u8],
    fingerprint: &'a IntegrityDigest,
    result: &'a CommandResultDocument,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnedCommandRecord {
    pub(crate) schema_version: u32,
    pub(crate) command: CommandId,
    pub(crate) run: RunId,
    pub(crate) actor: ActorRef,
    pub(crate) expected_sequence: RunSequence,
    pub(crate) submitted_at: TimestampMillis,
    pub(crate) canonical_document: Vec<u8>,
    pub(crate) canonical_intent: Vec<u8>,
    pub(crate) fingerprint: IntegrityDigest,
    pub(crate) result: CommandResultDocument,
}

impl RunJournal for RedbStore {
    #[tracing::instrument(
        name = "milkdrift.redb_store.commit_command",
        skip_all,
        fields(
            run = %request.receipt().run(),
            command = %request.receipt().command(),
            expected_sequence = request.receipt().expected_sequence().get(),
            event_count = request.events().len()
        )
    )]
    fn commit_command(
        &self,
        request: &AtomicRunCommitRequest,
    ) -> Result<AtomicRunCommitOutcome, PersistenceError> {
        if let Some(accounting) = request.workspace_accounting() {
            for event in request.events() {
                if let RunEventKind::RunCreated {
                    workspace_budget, ..
                } = event.kind()
                    && workspace_budget != &accounting.budget
                {
                    return Err(PersistenceError::InvalidDocument(
                        "run-created workspace budget differs from atomic accounting budget"
                            .to_owned(),
                    ));
                }
            }
        }
        let command_key = codec::pair(
            request.receipt().run().as_str(),
            request.receipt().command().as_str(),
        )?;
        let command_bytes = encode_command_record(request)?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let actual_head = {
            let heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
            let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
            validated_run_head(&heads, &events, request.receipt().run())?
        };
        let previous_membership = validate_run_history_membership_in_transaction(
            &write,
            request.receipt().run(),
            actual_head,
        )?;

        // Idempotency deliberately precedes the optimistic sequence conflict,
        // but replay never bypasses authoritative journal-integrity checks.
        {
            let commands = write.open_table(COMMAND_RESULTS).map_err(error::redb)?;
            if let Some(stored) = commands.get(command_key.as_slice()).map_err(error::redb)? {
                let stored_bytes = stored.value().to_vec();
                let stored = decode_command_record(&stored_bytes)?;
                if stored.run != *request.receipt().run()
                    || stored.command != *request.receipt().command()
                {
                    return Err(error::corruption(
                        "command-result key does not match its stored identities",
                    ));
                }
                let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
                validate_command_record_history(&stored, actual_head, &events)?;
                drop(events);
                validate_command_history_chain_in_transaction(&write, &stored)?;
                if stored.fingerprint == *request.receipt().fingerprint() {
                    return Ok(AtomicRunCommitOutcome::Replayed(stored.result));
                }
                return Err(PersistenceError::IdempotencyConflict {
                    run: request.receipt().run().clone(),
                    command: request.receipt().command().clone(),
                    existing: stored.fingerprint,
                    supplied: request.receipt().fingerprint().clone(),
                });
            }
        }
        if actual_head != request.receipt().expected_sequence() {
            return Err(PersistenceError::SequenceConflict {
                run: request.receipt().run().clone(),
                expected: request.receipt().expected_sequence(),
                actual: actual_head,
            });
        }
        if let Some(expected) = request.expected_lease_revision() {
            let actual = lease_set_revision_in_transaction(&write)?;
            if &actual != expected {
                return Err(PersistenceError::LeaseRevisionConflict {
                    expected: expected.clone(),
                    actual,
                });
            }
        }

        validate_required_artifacts(self, &write, request.required_artifacts())?;
        validate_artifact_accounting_references(&write, request)?;
        validate_workspace_accounting(&write, request)?;
        append_events(&write, request, self.faults.as_ref())?;
        if let Some(checkpoint) = request.projection_checkpoint() {
            crate::snapshot::attach_projection_checkpoint(
                &write,
                request.receipt().run(),
                request.result().resulting_sequence(),
                checkpoint,
            )?;
        }
        apply_workspace(&write, request)?;
        apply_indexes(&write, request)?;
        record_artifact_references(&write, request)?;

        {
            let mut commands = write.open_table(COMMAND_RESULTS).map_err(error::redb)?;
            if commands
                .insert(command_key.as_slice(), command_bytes.as_slice())
                .map_err(error::redb)?
                .is_some()
            {
                return Err(error::corruption(
                    "new command result unexpectedly replaced an existing row",
                ));
            }
        }
        if !request.events().is_empty() {
            let mut heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
            heads
                .insert(
                    request.receipt().run().as_str(),
                    request.result().resulting_sequence().get(),
                )
                .map_err(error::redb)?;
            let current = validate_run_history_membership_in_transaction(
                &write,
                request.receipt().run(),
                request.result().resulting_sequence(),
            )?;
            if previous_membership.is_some() && current.is_none() {
                return Err(error::corruption(
                    "run aggregate disappeared during command acceptance",
                ));
            }
        }
        persist_workspace_accounting(&write, request)?;

        self.faults.check(FaultPoint::BeforeCommandCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterCommandCommit)?;
        Ok(AtomicRunCommitOutcome::Committed(request.result().clone()))
    }

    fn head(&self, run: &RunId) -> Result<RunSequence, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, run)?;
        validate_run_history_membership(&read, run, head)?;
        Ok(head)
    }

    fn command_result(
        &self,
        run: &RunId,
        command: &CommandId,
    ) -> Result<Option<CommandResultDocument>, PersistenceError> {
        let key = codec::pair(run.as_str(), command.as_str())?;
        let read = self.database().begin_read().map_err(error::redb)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, run)?;
        let _membership = validate_run_history_membership(&read, run, head)?;
        let table = read.open_table(COMMAND_RESULTS).map_err(error::redb)?;
        let Some(record) = table.get(key.as_slice()).map_err(error::redb)? else {
            return Ok(None);
        };
        let record_bytes = record.value().to_vec();
        let record = decode_command_record(&record_bytes)?;
        if record.run != *run || record.command != *command {
            return Err(error::corruption(
                "command-result key does not match its stored identities",
            ));
        }
        validate_command_record_history(&record, head, &events)?;
        validate_command_history_chain(&read, &record)?;
        Ok(Some(record.result))
    }
}

pub(crate) fn encode_command_record(
    request: &AtomicRunCommitRequest,
) -> Result<Vec<u8>, PersistenceError> {
    json::encode(
        &StoredCommandRecord {
            schema_version: COMMAND_RECORD_SCHEMA_VERSION,
            command: request.receipt().command(),
            run: request.receipt().run(),
            actor: request.receipt().actor(),
            expected_sequence: request.receipt().expected_sequence(),
            submitted_at: request.receipt().submitted_at(),
            canonical_document: request.receipt().canonical_document(),
            canonical_intent: request.receipt().canonical_intent(),
            fingerprint: request.receipt().fingerprint(),
            result: request.result(),
        },
        "command record",
    )
}

pub(crate) fn decode_command_record(bytes: &[u8]) -> Result<OwnedCommandRecord, PersistenceError> {
    let record: OwnedCommandRecord = json::decode(bytes, "command record")?;
    if record.schema_version != COMMAND_RECORD_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "command_record",
            found: record.schema_version,
            supported: COMMAND_RECORD_SCHEMA_VERSION,
        });
    }
    let receipt = CommandReceipt::new_idempotent(
        record.command.clone(),
        record.run.clone(),
        record.actor.clone(),
        record.expected_sequence,
        record.submitted_at,
        record.canonical_document.clone(),
        record.canonical_intent.clone(),
    )
    .map_err(|cause| {
        PersistenceError::Corruption(format!("stored command receipt failed validation: {cause}"))
    })?;
    if receipt.fingerprint() != &record.fingerprint {
        return Err(error::corruption(
            "stored command receipt fingerprint does not match its semantic intent",
        ));
    }
    validate_command_result(&record.result)?;
    if record.result.command() != &record.command
        || record.result.run() != &record.run
        || record.result.command_fingerprint() != &record.fingerprint
    {
        return Err(error::corruption(
            "stored command result does not match its receipt",
        ));
    }
    let emitted = u64::try_from(record.result.event_ids().len())
        .map_err(|_| error::corruption("stored command result event count exceeds u64"))?;
    let expected_resulting = record
        .expected_sequence
        .get()
        .checked_add(emitted)
        .ok_or_else(|| error::corruption("stored command result sequence overflows"))?;
    if record.result.resulting_sequence().get() != expected_resulting {
        return Err(error::corruption(
            "stored command result sequence does not match its receipt and event count",
        ));
    }
    Ok(record)
}

pub(crate) fn validate_command_result(
    result: &CommandResultDocument,
) -> Result<(), PersistenceError> {
    if !matches!(
        result.schema_version(),
        COMMAND_RESULT_SCHEMA_VERSION_V1 | COMMAND_RESULT_SCHEMA_VERSION_V2
    ) {
        return Err(PersistenceError::UnsupportedVersion {
            document: "command_result",
            found: result.schema_version(),
            supported: COMMAND_RESULT_SCHEMA_VERSION_V2,
        });
    }
    let rebuilt = match result.authorization() {
        Some(decision) => CommandResultDocument::new_authorized(
            result.command().clone(),
            result.run().clone(),
            result.command_fingerprint().clone(),
            result.disposition(),
            result.resulting_sequence(),
            result.event_ids().to_vec(),
            result.result().clone(),
            decision.clone(),
        ),
        None => CommandResultDocument::new(
            result.command().clone(),
            result.run().clone(),
            result.command_fingerprint().clone(),
            result.disposition(),
            result.resulting_sequence(),
            result.event_ids().to_vec(),
            result.result().clone(),
        ),
    }
    .map_err(|cause| {
        PersistenceError::Corruption(format!("stored command result failed validation: {cause}"))
    })?;
    if &rebuilt != result {
        return Err(error::corruption(
            "stored command result is not canonical for its exact schema",
        ));
    }
    Ok(())
}

pub(crate) fn validate_command_result_head(
    result: &CommandResultDocument,
    head: RunSequence,
) -> Result<(), PersistenceError> {
    if result.resulting_sequence() > head
        || (result.disposition() == milkdrift_persistence::CommandDisposition::Accepted
            && result.resulting_sequence() == RunSequence::ZERO)
    {
        return Err(error::corruption(
            "stored command result is beyond the authoritative journal head",
        ));
    }
    Ok(())
}

pub(crate) fn validate_command_record_history<E>(
    record: &OwnedCommandRecord,
    head: RunSequence,
    events: &E,
) -> Result<(), PersistenceError>
where
    E: redb::ReadableTable<&'static [u8], &'static [u8]>,
{
    validate_command_result_head(&record.result, head)?;
    let mut sequence = record.expected_sequence;
    for expected_event in record.result.event_ids() {
        sequence = sequence.next()?;
        let key = codec::run_sequence(record.run.as_str(), sequence)?;
        let bytes = events
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| {
                error::corruption(format!(
                    "stored command result names missing event sequence {sequence}"
                ))
            })?;
        let event = decode_stored_event(bytes.value())?;
        if event.run_id() != &record.run
            || event.sequence() != sequence
            || event.event_id() != expected_event
        {
            return Err(error::corruption(
                "stored command result does not match its authoritative event range",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_stored_command_record<H, E>(
    key: &[u8],
    bytes: &[u8],
    heads: &H,
    events: &E,
) -> Result<(), PersistenceError>
where
    H: redb::ReadableTable<&'static str, u64>,
    E: redb::ReadableTable<&'static [u8], &'static [u8]>,
{
    let record = decode_command_record(bytes)?;
    if codec::pair(record.run.as_str(), record.command.as_str())?.as_slice() != key {
        return Err(error::corruption(
            "command-result key does not match its stored identities",
        ));
    }
    let head = validated_run_head(heads, events, &record.run)?;
    validate_command_record_history(&record, head, events)
}

pub(crate) fn validate_command_history_chain_in_transaction(
    write: &redb::WriteTransaction,
    record: &OwnedCommandRecord,
) -> Result<(), PersistenceError> {
    let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    let mut sequence = record.expected_sequence;
    for _ in record.result.event_ids() {
        sequence = sequence.next()?;
        let key = codec::run_sequence(record.run.as_str(), sequence)?;
        let bytes = events
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("command event is absent from the journal"))?
            .value()
            .to_vec();
        let event = decode_stored_event(&bytes)?;
        crate::snapshot::validate_history_link_in_transaction(write, &event)?;
    }
    Ok(())
}

pub(crate) fn validate_command_history_chain(
    read: &redb::ReadTransaction,
    record: &OwnedCommandRecord,
) -> Result<(), PersistenceError> {
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let mut sequence = record.expected_sequence;
    for _ in record.result.event_ids() {
        sequence = sequence.next()?;
        let key = codec::run_sequence(record.run.as_str(), sequence)?;
        let bytes = events
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("command event is absent from the journal"))?
            .value()
            .to_vec();
        let event = decode_stored_event(&bytes)?;
        crate::snapshot::validate_history_link(read, &event)?;
    }
    Ok(())
}

pub(crate) fn validate_run_history_membership_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<Option<()>, PersistenceError> {
    let summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let result = expected_run_membership(&summaries, run, head)?;
    drop(summaries);
    crate::snapshot::validate_history_head_in_transaction(write, run, head)?;
    if result.is_some() {
        let summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        let bytes = summaries
            .get(run.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authoritative run has no summary"))?;
        let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
        validate_nonterminal_membership_in_transaction(write, &summary)?;
    }
    Ok(result)
}

pub(crate) fn validate_run_history_membership(
    read: &redb::ReadTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<Option<()>, PersistenceError> {
    let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let result = expected_run_membership(&summaries, run, head)?;
    drop(summaries);
    crate::snapshot::validate_history_head(read, run, head)?;
    if result.is_some() {
        let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        let bytes = summaries
            .get(run.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authoritative run has no summary"))?;
        let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
        validate_nonterminal_membership(read, &summary)?;
    }
    Ok(result)
}

pub(crate) fn expected_run_membership<S>(
    summaries: &S,
    run: &RunId,
    head: RunSequence,
) -> Result<Option<()>, PersistenceError>
where
    S: redb::ReadableTable<&'static str, &'static [u8]>,
{
    let summary = summaries.get(run.as_str()).map_err(error::redb)?;
    match (head == RunSequence::ZERO, summary) {
        (true, None) => Ok(None),
        (true, Some(_)) => Err(error::corruption(
            "run summary exists without an authoritative journal head",
        )),
        (false, None) => Err(error::corruption(
            "authoritative run head has no discoverability summary",
        )),
        (false, Some(bytes)) => {
            let summary_bytes = bytes.value();
            let summary: RunSummaryIndex = json::decode(summary_bytes, "run summary")?;
            if summary.run != *run || summary.through_sequence != head {
                return Err(error::corruption(
                    "run membership summary disagrees with its head or identity",
                ));
            }
            Ok(Some(()))
        }
    }
}

pub(crate) fn validate_nonterminal_membership(
    read: &redb::ReadTransaction,
    summary: &RunSummaryIndex,
) -> Result<(), PersistenceError> {
    let marker = read
        .open_table(NONTERMINAL_RUNS)
        .map_err(error::redb)?
        .get(summary.run.as_str())
        .map_err(error::redb)?
        .map(|marker| marker.value());
    match (summary.state, marker) {
        (IndexedRunState::Terminal, None) => Ok(()),
        (IndexedRunState::Terminal, Some(_)) => Err(error::corruption(
            "terminal run remains in nonterminal discovery",
        )),
        (_, Some(1)) => Ok(()),
        (_, Some(_)) => Err(error::corruption(
            "nonterminal discovery contains an invalid marker",
        )),
        (_, None) => Err(error::corruption(
            "nonterminal run is absent from discovery",
        )),
    }
}

pub(crate) fn validate_nonterminal_membership_in_transaction(
    write: &redb::WriteTransaction,
    summary: &RunSummaryIndex,
) -> Result<(), PersistenceError> {
    let marker = write
        .open_table(NONTERMINAL_RUNS)
        .map_err(error::redb)?
        .get(summary.run.as_str())
        .map_err(error::redb)?
        .map(|marker| marker.value());
    match (summary.state, marker) {
        (IndexedRunState::Terminal, None) => Ok(()),
        (IndexedRunState::Terminal, Some(_)) => Err(error::corruption(
            "terminal run remains in nonterminal discovery",
        )),
        (_, Some(1)) => Ok(()),
        (_, Some(_)) => Err(error::corruption(
            "nonterminal discovery contains an invalid marker",
        )),
        (_, None) => Err(error::corruption(
            "nonterminal run is absent from discovery",
        )),
    }
}

pub(crate) fn validate_required_artifacts(
    store: &RedbStore,
    write: &redb::WriteTransaction,
    required: &[ArtifactReference],
) -> Result<(), PersistenceError> {
    let table = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    for reference in required {
        let Some(bytes) = table
            .get(reference.artifact().as_str())
            .map_err(error::redb)?
        else {
            return Err(PersistenceError::ArtifactNotCommitted(
                reference.artifact().to_string(),
            ));
        };
        let metadata: milkdrift_workspace::ArtifactMetadata =
            json::decode(bytes.value(), "artifact metadata")?;
        if metadata.reference() != reference {
            return Err(error::corruption(format!(
                "artifact metadata {} does not match the required reference",
                reference.artifact()
            )));
        }
        crate::artifact::verify_blob(
            &store.content_path(reference.digest()),
            reference,
            store.max_artifact_bytes,
        )?;
    }
    Ok(())
}

pub(crate) fn validate_artifact_accounting_references(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let newly_referenced: BTreeSet<_> = request.newly_referenced_artifacts().iter().collect();
    for reference in request.required_artifacts() {
        let previously_referenced =
            crate::artifact::validated_run_artifact_reference_in_transaction(
                write,
                request.receipt().run(),
                reference,
            )?;
        if newly_referenced.contains(reference) == previously_referenced {
            return Err(PersistenceError::InvalidDocument(format!(
                "artifact {} must be charged exactly on its first reference by run {}",
                reference.artifact(),
                request.receipt().run()
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_workspace_accounting(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(accounting) = request.workspace_accounting() else {
        return Ok(());
    };
    let actual = validate_or_initialize_workspace_domain(
        write,
        request.receipt().run(),
        &accounting.budget,
    )?;
    let reservations = write
        .open_table(ARTIFACT_RESERVATIONS)
        .map_err(error::redb)?;
    if reservations
        .get(request.receipt().run().as_str())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(PersistenceError::Storage {
            class: milkdrift_persistence::StorageFailureClass::OwnerBusy,
            message: format!(
                "run {} has an active artifact publication",
                request.receipt().run()
            ),
        });
    }
    if actual != accounting.expected_usage {
        return Err(PersistenceError::WorkspaceUsageConflict {
            run: request.receipt().run().clone(),
        });
    }
    Ok(())
}

pub(crate) fn workspace_domain_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<Option<(milkdrift_workspace::WorkspaceBudget, WorkspaceUsage)>, PersistenceError> {
    let budget: Option<milkdrift_workspace::WorkspaceBudget> = {
        let table = write.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
        table
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace budget"))
            .transpose()?
    };
    let usage = {
        let table = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
        table
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace usage"))
            .transpose()?
    };
    match (budget, usage) {
        (None, None) => Ok(None),
        (Some(budget), Some(usage)) => {
            budget.validate_usage(&usage).map_err(|cause| {
                error::corruption(format!("workspace usage exceeds its budget: {cause}"))
            })?;
            Ok(Some((budget, usage)))
        }
        _ => Err(error::corruption(
            "workspace budget and usage records are incomplete",
        )),
    }
}

pub(crate) fn persist_workspace_value_usage_accounting_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    usage: WorkspaceUsage,
) -> Result<(), PersistenceError> {
    let Some((_budget, stored_usage)) = workspace_domain_in_transaction(write, run)? else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if stored_usage != usage {
        return Err(error::corruption(
            "workspace usage argument disagrees with its durable document",
        ));
    }
    Ok(())
}

pub(crate) fn validate_workspace_domain_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    supplied: &milkdrift_workspace::WorkspaceBudget,
) -> Result<WorkspaceUsage, PersistenceError> {
    let Some((budget, usage)) = workspace_domain_in_transaction(write, run)? else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if &budget != supplied {
        return Err(PersistenceError::ImmutableConflict {
            entity: "workspace_budget",
            identity: run.to_string(),
        });
    }
    budget.validate_usage(&usage).map_err(|cause| {
        error::corruption(format!(
            "workspace usage exceeds its durable budget: {cause}"
        ))
    })?;
    Ok(usage)
}

pub(crate) fn validate_or_initialize_workspace_domain(
    write: &redb::WriteTransaction,
    run: &RunId,
    supplied: &milkdrift_workspace::WorkspaceBudget,
) -> Result<WorkspaceUsage, PersistenceError> {
    match workspace_domain_in_transaction(write, run)? {
        None => {
            supplied
                .validate_usage(&WorkspaceUsage::EMPTY)
                .map_err(|cause| {
                    PersistenceError::InvalidDocument(format!(
                        "empty workspace usage violates the supplied budget: {cause}"
                    ))
                })?;
            let budget_bytes = json::encode(supplied, "workspace budget")?;
            let usage_bytes = json::encode(&WorkspaceUsage::EMPTY, "workspace usage")?;
            write
                .open_table(WORKSPACE_BUDGETS)
                .map_err(error::redb)?
                .insert(run.as_str(), budget_bytes.as_slice())
                .map_err(error::redb)?;
            write
                .open_table(WORKSPACE_USAGE)
                .map_err(error::redb)?
                .insert(run.as_str(), usage_bytes.as_slice())
                .map_err(error::redb)?;
            Ok(WorkspaceUsage::EMPTY)
        }
        Some((_budget, _usage)) => validate_workspace_domain_in_transaction(write, run, supplied),
    }
}

pub(crate) fn advance_workspace_global_usage_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    expected: WorkspaceUsage,
    resulting: WorkspaceUsage,
) -> Result<(), PersistenceError> {
    let Some((budget, actual)) = workspace_domain_in_transaction(write, run)? else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if actual != expected {
        return Err(error::corruption(
            "workspace usage changed before artifact accounting advance",
        ));
    }
    budget.validate_usage(&resulting).map_err(|cause| {
        PersistenceError::InvalidDocument(format!("workspace usage exceeds budget: {cause}"))
    })?;
    Ok(())
}

pub(crate) fn persist_workspace_accounting(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(accounting) = request.workspace_accounting() else {
        return Ok(());
    };
    let Some((_budget, actual)) = workspace_domain_in_transaction(write, request.receipt().run())?
    else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if actual != accounting.expected_usage {
        return Err(error::corruption(
            "workspace usage changed before command accounting persistence",
        ));
    }
    let bytes = json::encode(&accounting.resulting_usage, "workspace usage")?;
    let mut table = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    table
        .insert(request.receipt().run().as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn append_events(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
    faults: &dyn crate::FaultInjector,
) -> Result<(), PersistenceError> {
    let mut events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    let mut signal_receipts = write.open_table(SIGNAL_RECEIPTS).map_err(error::redb)?;
    let mut metadata = write.open_table(METADATA).map_err(error::redb)?;
    for event in request.events() {
        let key = codec::run_sequence(event.run_id().as_str(), event.sequence())?;
        if events.get(key.as_slice()).map_err(error::redb)?.is_some() {
            return Err(error::corruption(format!(
                "event slot {}:{} already exists beyond the authoritative head",
                event.run_id(),
                event.sequence()
            )));
        }
        let document = event.to_canonical_json()?;
        faults.check(FaultPoint::BeforeEventInsert)?;
        if events
            .insert(key.as_slice(), document.as_slice())
            .map_err(error::redb)?
            .is_some()
        {
            return Err(error::corruption("event append overwrote an existing slot"));
        }
        match event.kind() {
            RunEventKind::NodeScheduled { invocation, .. } => {
                let key = crate::journal::invocation_fact_key(event.run_id(), invocation);
                if metadata
                    .insert(key.as_str(), event.sequence().get())
                    .map_err(error::redb)?
                    .is_some()
                {
                    return Err(error::corruption(
                        "invocation fact index rejected a duplicate stable identity",
                    ));
                }
            }
            RunEventKind::SignalReceived { signal, .. } => {
                let signal_key = codec::pair(event.run_id().as_str(), signal.as_str())?;
                if signal_receipts
                    .insert(signal_key.as_slice(), event.sequence().get())
                    .map_err(error::redb)?
                    .is_some()
                {
                    return Err(error::corruption(
                        "signal receipt index rejected a duplicate stable identity",
                    ));
                }
            }
            RunEventKind::SignalDeduplicated { signal, .. } => {
                let signal_key = codec::pair(event.run_id().as_str(), signal.as_str())?;
                if signal_receipts
                    .get(signal_key.as_slice())
                    .map_err(error::redb)?
                    .is_none()
                {
                    return Err(error::corruption(
                        "signal deduplication has no authoritative receipt index",
                    ));
                }
            }
            _ => {}
        }
        faults.check(FaultPoint::AfterEventInsert)?;
        crate::snapshot::append_history_checkpoint(write, event)?;
        faults.check(FaultPoint::AfterHistoryChainUpdate)?;
    }
    Ok(())
}
