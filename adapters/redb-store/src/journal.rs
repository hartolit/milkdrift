use std::{collections::BTreeSet, ops::Bound};

use milkdrift_persistence::{
    ActorRef, AtomicRunCommitOutcome, AtomicRunCommitRequest, COMMAND_RESULT_SCHEMA_VERSION_V1,
    CommandId, CommandReceipt, CommandResultDocument, EventCursor, EventPage, EventPageQuery,
    IndexedRunState, IntegrityDigest, LeaseIndexEntry, LeaseIndexMutation, PageSize,
    PersistenceError, RunEventKind, RunJournal, RunQueryStore, RunSequence, RunSummaryIndex,
    RunSummaryPage, RunSummaryPageQuery, RunnableCursor, RunnableIndexEntry, RunnableIndexMutation,
    RunnablePage, TimerIndexEntry, TimerIndexMutation, TimestampMillis, WorkspaceMutation,
    WorkspaceStore,
};
use milkdrift_workspace::{
    ArtifactReference, MAX_SCOPE_DEPTH, RunId, ScopeId, ScopeKind, ScopeReference, ValueKey,
    ValueOrigin, WorkspaceScope, WorkspaceUsage, WorkspaceValueEntry, WorkspaceValueReference,
};
use redb::{ReadableTable, Table};
use serde::{Deserialize, Serialize};

use crate::{
    RedbStore, codec, error,
    fault::FaultPoint,
    json,
    schema::{
        ARTIFACT_METADATA, ARTIFACT_REFERENCES, ARTIFACT_RESERVATIONS, COMMAND_RESULTS,
        EVENT_CHECKSUMS, LEASE_ENTRIES, LEASE_INDEX, NONTERMINAL_RUNS, ROOT_SCOPES, RUN_EVENTS,
        RUN_HEADS, RUN_SUMMARIES, RUNNABLE_ENTRIES, RUNNABLE_INDEX, SCOPES, TIMER_ENTRIES,
        TIMER_INDEX, VALUES, WORKSPACE_BUDGETS, WORKSPACE_USAGE,
    },
};

const COMMAND_RECORD_SCHEMA_VERSION: u32 = 1;

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
    fingerprint: &'a IntegrityDigest,
    result: &'a CommandResultDocument,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnedCommandRecord {
    schema_version: u32,
    command: CommandId,
    run: RunId,
    actor: ActorRef,
    expected_sequence: RunSequence,
    submitted_at: TimestampMillis,
    canonical_document: Vec<u8>,
    fingerprint: IntegrityDigest,
    result: CommandResultDocument,
}

impl RunJournal for RedbStore {
    #[tracing::instrument(
        name = "milkdrift.redb_store.commit_command",
        skip_all,
        fields(
            run = %request.receipt.run(),
            command = %request.receipt.command(),
            expected_sequence = request.receipt.expected_sequence().get(),
            event_count = request.events.len()
        )
    )]
    fn commit_command(
        &self,
        request: &AtomicRunCommitRequest,
    ) -> Result<AtomicRunCommitOutcome, PersistenceError> {
        let verified_request = AtomicRunCommitRequest::new(
            request.receipt.clone(),
            request.events.clone(),
            request.workspace.clone(),
            request.workspace_accounting.clone(),
            request.required_artifacts.clone(),
            request.newly_referenced_artifacts.clone(),
            request.result.clone(),
            request.indexes.clone(),
        )?;
        if &verified_request != request {
            return Err(PersistenceError::InvalidDocument(
                "atomic run commit is not canonical".to_owned(),
            ));
        }
        if let Some(accounting) = &request.workspace_accounting {
            for event in &request.events {
                if let RunEventKind::RunCreated {
                    workspace_budget, ..
                } = event.kind()
                {
                    if workspace_budget != &accounting.budget {
                        return Err(PersistenceError::InvalidDocument(
                            "run-created workspace budget differs from atomic accounting budget"
                                .to_owned(),
                        ));
                    }
                }
            }
        }
        let command_key = codec::pair(
            request.receipt.run().as_str(),
            request.receipt.command().as_str(),
        )?;
        let command_bytes = encode_command_record(request)?;
        let write = self.database().begin_write().map_err(error::redb)?;

        // Idempotency deliberately precedes the optimistic sequence guard.
        {
            let commands = write.open_table(COMMAND_RESULTS).map_err(error::redb)?;
            if let Some(stored) = commands.get(command_key.as_slice()).map_err(error::redb)? {
                let stored = decode_command_record(stored.value())?;
                if stored.run != *request.receipt.run()
                    || stored.command != *request.receipt.command()
                {
                    return Err(error::corruption(
                        "command-result key does not match its stored identities",
                    ));
                }
                if stored.fingerprint == *request.receipt.fingerprint() {
                    return Ok(AtomicRunCommitOutcome::Replayed(stored.result));
                }
                return Err(PersistenceError::IdempotencyConflict {
                    run: request.receipt.run().clone(),
                    command: request.receipt.command().clone(),
                    existing: stored.fingerprint,
                    supplied: request.receipt.fingerprint().clone(),
                });
            }
        }

        let actual_head = {
            let heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
            heads
                .get(request.receipt.run().as_str())
                .map_err(error::redb)?
                .map_or(RunSequence::ZERO, |value| RunSequence::new(value.value()))
        };
        if actual_head != request.receipt.expected_sequence() {
            return Err(PersistenceError::SequenceConflict {
                run: request.receipt.run().clone(),
                expected: request.receipt.expected_sequence(),
                actual: actual_head,
            });
        }

        validate_required_artifacts(self, &write, &request.required_artifacts)?;
        validate_artifact_accounting_references(&write, request)?;
        validate_workspace_accounting(&write, request)?;
        append_events(&write, request)?;
        apply_workspace(&write, request)?;
        apply_indexes(&write, request)?;
        record_artifact_references(&write, request)?;

        {
            let mut commands = write.open_table(COMMAND_RESULTS).map_err(error::redb)?;
            commands
                .insert(command_key.as_slice(), command_bytes.as_slice())
                .map_err(error::redb)?;
        }
        if !request.events.is_empty() {
            let mut heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
            heads
                .insert(
                    request.receipt.run().as_str(),
                    request.result.resulting_sequence().get(),
                )
                .map_err(error::redb)?;
        }
        persist_workspace_accounting(&write, request)?;

        self.faults.check(FaultPoint::BeforeCommandCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterCommandCommit)?;
        Ok(AtomicRunCommitOutcome::Committed(request.result.clone()))
    }

    fn head(&self, run: &RunId) -> Result<RunSequence, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(RUN_HEADS).map_err(error::redb)?;
        Ok(table
            .get(run.as_str())
            .map_err(error::redb)?
            .map_or(RunSequence::ZERO, |value| RunSequence::new(value.value())))
    }

    fn command_result(
        &self,
        run: &RunId,
        command: &CommandId,
    ) -> Result<Option<CommandResultDocument>, PersistenceError> {
        let key = codec::pair(run.as_str(), command.as_str())?;
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(COMMAND_RESULTS).map_err(error::redb)?;
        let Some(record) = table.get(key.as_slice()).map_err(error::redb)? else {
            return Ok(None);
        };
        let record = decode_command_record(record.value())?;
        if record.run != *run || record.command != *command {
            return Err(error::corruption(
                "command-result key does not match its stored identities",
            ));
        }
        Ok(Some(record.result))
    }
}

fn encode_command_record(request: &AtomicRunCommitRequest) -> Result<Vec<u8>, PersistenceError> {
    json::encode(
        &StoredCommandRecord {
            schema_version: COMMAND_RECORD_SCHEMA_VERSION,
            command: request.receipt.command(),
            run: request.receipt.run(),
            actor: request.receipt.actor(),
            expected_sequence: request.receipt.expected_sequence(),
            submitted_at: request.receipt.submitted_at(),
            canonical_document: request.receipt.canonical_document(),
            fingerprint: request.receipt.fingerprint(),
            result: &request.result,
        },
        "command record",
    )
}

fn decode_command_record(bytes: &[u8]) -> Result<OwnedCommandRecord, PersistenceError> {
    let record: OwnedCommandRecord = json::decode(bytes, "command record")?;
    if record.schema_version != COMMAND_RECORD_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "command_record",
            found: record.schema_version,
            supported: COMMAND_RECORD_SCHEMA_VERSION,
        });
    }
    let receipt = CommandReceipt::new(
        record.command.clone(),
        record.run.clone(),
        record.actor.clone(),
        record.expected_sequence,
        record.submitted_at,
        record.canonical_document.clone(),
    )
    .map_err(|cause| {
        PersistenceError::Corruption(format!("stored command receipt failed validation: {cause}"))
    })?;
    if receipt.fingerprint() != &record.fingerprint {
        return Err(error::corruption(
            "stored command receipt fingerprint does not match its bytes",
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
    Ok(record)
}

fn validate_command_result(result: &CommandResultDocument) -> Result<(), PersistenceError> {
    if result.schema_version() != COMMAND_RESULT_SCHEMA_VERSION_V1 {
        return Err(PersistenceError::UnsupportedVersion {
            document: "command_result",
            found: result.schema_version(),
            supported: COMMAND_RESULT_SCHEMA_VERSION_V1,
        });
    }
    let rebuilt = CommandResultDocument::new(
        result.command().clone(),
        result.run().clone(),
        result.command_fingerprint().clone(),
        result.disposition(),
        result.resulting_sequence(),
        result.event_ids().to_vec(),
        result.result().clone(),
    )
    .map_err(|cause| {
        PersistenceError::Corruption(format!("stored command result failed validation: {cause}"))
    })?;
    if &rebuilt != result {
        return Err(error::corruption(
            "stored command result is not canonical schema v1",
        ));
    }
    Ok(())
}

fn validate_required_artifacts(
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

fn validate_artifact_accounting_references(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let newly_referenced: BTreeSet<_> = request.newly_referenced_artifacts.iter().collect();
    let table = write.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
    for reference in &request.required_artifacts {
        let digest = reference.digest().to_hex();
        let prefix = codec::components(&[
            &digest,
            reference.artifact().as_str(),
            request.receipt.run().as_str(),
        ])?;
        let end = codec::prefix_end(prefix.clone())
            .ok_or_else(|| error::corruption("artifact-reference prefix has no range end"))?;
        let mut previously_referenced = false;
        for item in table
            .range(prefix.as_slice()..end.as_slice())
            .map_err(error::redb)?
        {
            let (_, bytes) = item.map_err(error::redb)?;
            let stored: ArtifactReference = json::decode(bytes.value(), "artifact reference")?;
            if &stored != reference {
                return Err(error::corruption(
                    "artifact-reference index prefix contradicts its stored document",
                ));
            }
            previously_referenced = true;
        }
        if newly_referenced.contains(reference) == previously_referenced {
            return Err(PersistenceError::InvalidDocument(format!(
                "artifact {} must be charged exactly on its first reference by run {}",
                reference.artifact(),
                request.receipt.run()
            )));
        }
    }
    Ok(())
}

fn validate_workspace_accounting(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(accounting) = &request.workspace_accounting else {
        return Ok(());
    };
    validate_or_initialize_workspace_budget(write, request.receipt.run(), &accounting.budget)?;
    let reservations = write
        .open_table(ARTIFACT_RESERVATIONS)
        .map_err(error::redb)?;
    if reservations
        .get(request.receipt.run().as_str())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(PersistenceError::Storage {
            class: milkdrift_persistence::StorageFailureClass::OwnerBusy,
            message: format!(
                "run {} has an active artifact publication",
                request.receipt.run()
            ),
        });
    }
    let table = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    let actual = table
        .get(request.receipt.run().as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode(bytes.value(), "workspace usage"))
        .transpose()?
        .unwrap_or(WorkspaceUsage::EMPTY);
    if actual != accounting.expected_usage {
        return Err(PersistenceError::WorkspaceUsageConflict {
            run: request.receipt.run().clone(),
        });
    }
    Ok(())
}

pub(crate) fn validate_or_initialize_workspace_budget(
    write: &redb::WriteTransaction,
    run: &RunId,
    supplied: &milkdrift_workspace::WorkspaceBudget,
) -> Result<(), PersistenceError> {
    let supplied_bytes = json::encode(supplied, "workspace budget")?;
    let mut budgets = write.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
    let existing = budgets
        .get(run.as_str())
        .map_err(error::redb)?
        .map(|bytes| bytes.value().to_vec());
    match existing {
        Some(bytes) => {
            let stored: milkdrift_workspace::WorkspaceBudget =
                json::decode(&bytes, "workspace budget")?;
            if &stored != supplied {
                return Err(PersistenceError::ImmutableConflict {
                    entity: "workspace_budget",
                    identity: run.to_string(),
                });
            }
        }
        None => {
            budgets
                .insert(run.as_str(), supplied_bytes.as_slice())
                .map_err(error::redb)?;
        }
    }
    Ok(())
}

fn persist_workspace_accounting(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(accounting) = &request.workspace_accounting else {
        return Ok(());
    };
    let bytes = json::encode(&accounting.resulting_usage, "workspace usage")?;
    let mut table = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    table
        .insert(request.receipt.run().as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

fn append_events(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let mut events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    let mut checksums = write.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
    for event in &request.events {
        let key = codec::run_sequence(event.run_id().as_str(), event.sequence())?;
        if events.get(key.as_slice()).map_err(error::redb)?.is_some() {
            return Err(error::corruption(format!(
                "event slot {}:{} already exists beyond the authoritative head",
                event.run_id(),
                event.sequence()
            )));
        }
        if checksums
            .get(event.event_id().as_str())
            .map_err(error::redb)?
            .is_some()
        {
            return Err(PersistenceError::ImmutableConflict {
                entity: "event",
                identity: event.event_id().to_string(),
            });
        }
        let document = event.to_canonical_json()?;
        events
            .insert(key.as_slice(), document.as_slice())
            .map_err(error::redb)?;
        checksums
            .insert(event.event_id().as_str(), event.checksum().as_str())
            .map_err(error::redb)?;
    }
    Ok(())
}

fn apply_workspace(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let mut scopes = write.open_table(SCOPES).map_err(error::redb)?;
    let mut roots = write.open_table(ROOT_SCOPES).map_err(error::redb)?;
    let mut values = write.open_table(VALUES).map_err(error::redb)?;

    for mutation in &request.workspace {
        match mutation {
            WorkspaceMutation::CreateScope { scope } => {
                put_scope(&mut scopes, &mut roots, scope)?;
            }
            WorkspaceMutation::PutValue { entry } => {
                put_value(&scopes, &mut values, entry)?;
            }
        }
    }
    Ok(())
}

fn put_scope(
    scopes: &mut Table<'_, &[u8], &[u8]>,
    roots: &mut Table<'_, &str, &str>,
    scope: &WorkspaceScope,
) -> Result<(), PersistenceError> {
    let reference = scope.reference();
    let key = codec::pair(reference.run().as_str(), reference.scope().as_str())?;
    if scopes.get(key.as_slice()).map_err(error::redb)?.is_some() {
        return Err(PersistenceError::ImmutableConflict {
            entity: "workspace_scope",
            identity: format!("{}/{}", reference.run(), reference.scope()),
        });
    }
    match (scope.kind(), scope.parent()) {
        (ScopeKind::RunRoot, None) => {
            if roots
                .get(reference.run().as_str())
                .map_err(error::redb)?
                .is_some()
            {
                return Err(PersistenceError::ImmutableConflict {
                    entity: "workspace_root_scope",
                    identity: reference.run().to_string(),
                });
            }
            roots
                .insert(reference.run().as_str(), reference.scope().as_str())
                .map_err(error::redb)?;
        }
        (_, Some(parent)) => {
            validate_new_scope_depth(scopes, parent)?;
        }
        _ => {
            return Err(PersistenceError::InvalidDocument(
                "workspace scope kind/parent invariant failed".to_owned(),
            ));
        }
    }
    let bytes = json::encode(scope, "workspace scope")?;
    scopes
        .insert(key.as_slice(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

fn put_value(
    scopes: &Table<'_, &[u8], &[u8]>,
    values: &mut Table<'_, &[u8], &[u8]>,
    entry: &WorkspaceValueEntry,
) -> Result<(), PersistenceError> {
    let reference = entry.reference();
    let scope = reference.scope();
    let scope_key = codec::pair(scope.run().as_str(), scope.scope().as_str())?;
    if scopes
        .get(scope_key.as_slice())
        .map_err(error::redb)?
        .is_none()
    {
        return Err(PersistenceError::NotFound {
            entity: "workspace_scope",
            identity: format!("{}/{}", scope.run(), scope.scope()),
        });
    }
    let key = workspace_value_key(reference)?;
    if values.get(key.as_slice()).map_err(error::redb)?.is_some() {
        return Err(PersistenceError::ImmutableConflict {
            entity: "workspace_value",
            identity: format!(
                "{}/{}/{}/{}",
                scope.run(),
                scope.scope(),
                reference.key(),
                reference.version()
            ),
        });
    }
    match entry.origin() {
        ValueOrigin::Initial => {}
        ValueOrigin::Successor { previous } => {
            let _previous = require_value(values, previous, "previous_workspace_value")?;
        }
        ValueOrigin::Inherited { source } => {
            let _source = require_value(values, source, "inherited_workspace_value")?;
            require_ancestor(scopes, source.scope(), scope)?;
        }
        ValueOrigin::Imported { source } => {
            let imported_source = require_value(values, source, "imported_workspace_value")?;
            if imported_source.value() != entry.value() {
                return Err(PersistenceError::InvalidDocument(
                    "an imported workspace value must preserve its exact source content".to_owned(),
                ));
            }
        }
    }
    let bytes = json::encode(entry, "workspace value")?;
    values
        .insert(key.as_slice(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

fn require_value(
    values: &Table<'_, &[u8], &[u8]>,
    reference: &WorkspaceValueReference,
    entity: &'static str,
) -> Result<WorkspaceValueEntry, PersistenceError> {
    let key = workspace_value_key(reference)?;
    let bytes = values
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| PersistenceError::NotFound {
            entity,
            identity: format!(
                "{}/{}/{}/{}",
                reference.scope().run(),
                reference.scope().scope(),
                reference.key(),
                reference.version()
            ),
        })?;
    let stored: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
    if stored.reference() != reference {
        return Err(error::corruption(
            "workspace-value key does not match its document",
        ));
    }
    Ok(stored)
}

fn validate_new_scope_depth(
    scopes: &Table<'_, &[u8], &[u8]>,
    parent: &ScopeReference,
) -> Result<(), PersistenceError> {
    let mut current = Some(parent.clone());
    let mut total_depth = 1_usize; // Include the new child being validated.
    let mut seen = BTreeSet::new();
    while let Some(reference) = current {
        total_depth = total_depth.saturating_add(1);
        if total_depth > MAX_SCOPE_DEPTH {
            return Err(PersistenceError::InvalidDocument(format!(
                "workspace scope lineage may contain at most {MAX_SCOPE_DEPTH} entries"
            )));
        }
        if !seen.insert(reference.clone()) {
            return Err(error::corruption(
                "workspace scope lineage contains a cycle",
            ));
        }
        let key = codec::pair(reference.run().as_str(), reference.scope().as_str())?;
        let bytes = scopes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| PersistenceError::NotFound {
                entity: "parent_workspace_scope",
                identity: format!("{}/{}", reference.run(), reference.scope()),
            })?;
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        if scope.reference() != &reference {
            return Err(error::corruption(
                "workspace-scope key does not match its document",
            ));
        }
        current = scope.parent().cloned();
    }
    Ok(())
}

fn require_ancestor(
    scopes: &Table<'_, &[u8], &[u8]>,
    candidate: &ScopeReference,
    leaf: &ScopeReference,
) -> Result<(), PersistenceError> {
    let mut current = leaf.clone();
    for _ in 0..MAX_SCOPE_DEPTH {
        if &current == candidate {
            return Ok(());
        }
        let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
        let bytes = scopes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("workspace scope lineage is incomplete"))?;
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        let Some(parent) = scope.parent() else {
            break;
        };
        current = parent.clone();
    }
    Err(PersistenceError::InvalidDocument(format!(
        "scope {candidate:?} is not an ancestor of {leaf:?}"
    )))
}

fn workspace_value_key(reference: &WorkspaceValueReference) -> Result<Vec<u8>, PersistenceError> {
    codec::value(
        reference.scope().run().as_str(),
        reference.scope().scope().as_str(),
        reference.key().as_str(),
        reference.version().get(),
    )
}

fn apply_indexes(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(summary) = &request.indexes.summary else {
        return Ok(());
    };
    let summary_bytes = json::encode(summary, "run summary")?;
    {
        let mut summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        summaries
            .insert(summary.run.as_str(), summary_bytes.as_slice())
            .map_err(error::redb)?;
    }
    {
        let mut nonterminal = write.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
        if summary.state == IndexedRunState::Terminal {
            let _removed = nonterminal
                .remove(summary.run.as_str())
                .map_err(error::redb)?;
        } else {
            nonterminal
                .insert(summary.run.as_str(), 1)
                .map_err(error::redb)?;
        }
    }
    apply_runnable_mutations(write, &request.indexes.runnable)?;
    apply_timer_mutations(write, &request.indexes.timers)?;
    apply_lease_mutations(write, &request.indexes.leases)?;
    Ok(())
}

fn apply_runnable_mutations(
    write: &redb::WriteTransaction,
    mutations: &[RunnableIndexMutation],
) -> Result<(), PersistenceError> {
    let mut entries = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let mut ordered = write.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    for mutation in mutations {
        let (run, execution) = match mutation {
            RunnableIndexMutation::Upsert { entry } => (&entry.run, &entry.execution),
            RunnableIndexMutation::Remove { run, execution } => (run, execution),
        };
        let identity = codec::pair(run.as_str(), execution.as_str())?;
        if let Some(previous) = entries.get(identity.as_slice()).map_err(error::redb)? {
            let previous: RunnableIndexEntry = json::decode(previous.value(), "runnable index")?;
            let key = runnable_order_key(&previous)?;
            let _removed = ordered.remove(key.as_slice()).map_err(error::redb)?;
        }
        match mutation {
            RunnableIndexMutation::Upsert { entry } => {
                let bytes = json::encode(entry, "runnable index")?;
                let order_key = runnable_order_key(entry)?;
                entries
                    .insert(identity.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
                ordered
                    .insert(order_key.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
            }
            RunnableIndexMutation::Remove { .. } => {
                let _removed = entries.remove(identity.as_slice()).map_err(error::redb)?;
            }
        }
    }
    Ok(())
}

fn apply_timer_mutations(
    write: &redb::WriteTransaction,
    mutations: &[TimerIndexMutation],
) -> Result<(), PersistenceError> {
    let mut entries = write.open_table(TIMER_ENTRIES).map_err(error::redb)?;
    let mut ordered = write.open_table(TIMER_INDEX).map_err(error::redb)?;
    for mutation in mutations {
        let (run, timer) = match mutation {
            TimerIndexMutation::Upsert { entry } => (&entry.run, &entry.timer),
            TimerIndexMutation::Remove { run, timer } => (run, timer),
        };
        let identity = codec::pair(run.as_str(), timer.as_str())?;
        if let Some(previous) = entries.get(identity.as_slice()).map_err(error::redb)? {
            let previous: TimerIndexEntry = json::decode(previous.value(), "timer index")?;
            let key = timer_order_key(&previous)?;
            let _removed = ordered.remove(key.as_slice()).map_err(error::redb)?;
        }
        match mutation {
            TimerIndexMutation::Upsert { entry } => {
                let bytes = json::encode(entry, "timer index")?;
                let order_key = timer_order_key(entry)?;
                entries
                    .insert(identity.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
                ordered
                    .insert(order_key.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
            }
            TimerIndexMutation::Remove { .. } => {
                let _removed = entries.remove(identity.as_slice()).map_err(error::redb)?;
            }
        }
    }
    Ok(())
}

fn apply_lease_mutations(
    write: &redb::WriteTransaction,
    mutations: &[LeaseIndexMutation],
) -> Result<(), PersistenceError> {
    let mut entries = write.open_table(LEASE_ENTRIES).map_err(error::redb)?;
    let mut ordered = write.open_table(LEASE_INDEX).map_err(error::redb)?;
    for mutation in mutations {
        let (run, lease) = match mutation {
            LeaseIndexMutation::Upsert { entry } => (&entry.run, &entry.lease),
            LeaseIndexMutation::Remove { run, lease } => (run, lease),
        };
        let identity = codec::pair(run.as_str(), lease.as_str())?;
        if let Some(previous) = entries.get(identity.as_slice()).map_err(error::redb)? {
            let previous: LeaseIndexEntry = json::decode(previous.value(), "lease index")?;
            let key = lease_order_key(&previous)?;
            let _removed = ordered.remove(key.as_slice()).map_err(error::redb)?;
        }
        match mutation {
            LeaseIndexMutation::Upsert { entry } => {
                let bytes = json::encode(entry, "lease index")?;
                let order_key = lease_order_key(entry)?;
                entries
                    .insert(identity.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
                ordered
                    .insert(order_key.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
            }
            LeaseIndexMutation::Remove { .. } => {
                let _removed = entries.remove(identity.as_slice()).map_err(error::redb)?;
            }
        }
    }
    Ok(())
}

fn runnable_order_key(entry: &RunnableIndexEntry) -> Result<Vec<u8>, PersistenceError> {
    codec::ordered_timestamp(
        entry.eligible_at.get(),
        &format!("{}\0{}", entry.run, entry.execution),
    )
}

fn timer_order_key(entry: &TimerIndexEntry) -> Result<Vec<u8>, PersistenceError> {
    codec::ordered_timestamp(
        entry.fire_at.get(),
        &format!("{}\0{}", entry.run, entry.timer),
    )
}

fn lease_order_key(entry: &LeaseIndexEntry) -> Result<Vec<u8>, PersistenceError> {
    codec::ordered_timestamp(
        entry.expires_at.get(),
        &format!("{}\0{}", entry.run, entry.lease),
    )
}

fn record_artifact_references(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let mut references = write.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
    for reference in &request.required_artifacts {
        let digest = reference.digest().to_hex();
        let key = codec::components(&[
            &digest,
            reference.artifact().as_str(),
            request.receipt.run().as_str(),
            request.receipt.command().as_str(),
        ])?;
        let bytes = json::encode(reference, "artifact reference")?;
        references
            .insert(key.as_slice(), bytes.as_slice())
            .map_err(error::redb)?;
    }
    Ok(())
}

impl RunQueryStore for RedbStore {
    fn events(&self, query: &EventPageQuery) -> Result<EventPage, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let observed_head = heads
            .get(query.run.as_str())
            .map_err(error::redb)?
            .map_or(RunSequence::ZERO, |value| RunSequence::new(value.value()));
        let mut next_sequence = query
            .cursor
            .as_ref()
            .map_or(RunSequence::FIRST, |cursor| cursor.next_sequence);
        if observed_head == RunSequence::ZERO {
            if query.cursor.is_some() {
                return Err(PersistenceError::InvalidCursor(
                    "event cursor names an absent run".to_owned(),
                ));
            }
            return Ok(EventPage {
                events: Vec::new(),
                next: None,
                observed_head,
            });
        }
        if next_sequence > observed_head {
            return Err(PersistenceError::InvalidCursor(format!(
                "event cursor sequence {next_sequence} is beyond observed head {observed_head}"
            )));
        }
        let events_table = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let checksum_table = read.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
        let mut events = Vec::with_capacity(query.limit.get() as usize);
        while next_sequence <= observed_head && events.len() < query.limit.get() as usize {
            let key = codec::run_sequence(query.run.as_str(), next_sequence)?;
            let bytes = events_table
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| {
                    error::corruption(format!(
                        "run {} is missing authoritative event sequence {next_sequence}",
                        query.run
                    ))
                })?;
            let event = decode_stored_event(bytes.value())?;
            if event.run_id() != &query.run || event.sequence() != next_sequence {
                return Err(error::corruption(
                    "stored event key does not match its envelope",
                ));
            }
            let checksum = checksum_table
                .get(event.event_id().as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("event checksum index entry is missing"))?;
            if checksum.value() != event.checksum().as_str() {
                return Err(error::corruption(
                    "event checksum index does not match its envelope",
                ));
            }
            events.push(event);
            if next_sequence == observed_head {
                break;
            }
            next_sequence = next_sequence.next()?;
        }
        let next = if events.len() == query.limit.get() as usize
            && events
                .last()
                .is_some_and(|event| event.sequence() < observed_head)
        {
            Some(EventCursor {
                run: query.run.clone(),
                next_sequence: events
                    .last()
                    .ok_or_else(|| error::corruption("event page lost its cursor"))?
                    .sequence()
                    .next()?,
            })
        } else {
            None
        };
        Ok(EventPage {
            events,
            next,
            observed_head,
        })
    }

    fn run_summary(&self, run: &RunId) -> Result<Option<RunSummaryIndex>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        table
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| {
                let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
                if summary.run != *run {
                    return Err(error::corruption(
                        "run-summary key does not match its document",
                    ));
                }
                validate_summary_head(&heads, &summary)?;
                Ok(summary)
            })
            .transpose()
    }

    fn run_summaries(
        &self,
        query: &RunSummaryPageQuery,
    ) -> Result<RunSummaryPage, PersistenceError> {
        const MAX_SCANNED_SUMMARIES: usize = 100_000;
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let mut runs = Vec::with_capacity(query.limit.get() as usize);
        let mut has_more = false;
        let mut scanned = 0_usize;
        for item in table.iter().map_err(error::redb)? {
            scanned += 1;
            if scanned > MAX_SCANNED_SUMMARIES {
                return Err(PersistenceError::Storage {
                    class: milkdrift_persistence::StorageFailureClass::ResourceExhausted,
                    message: format!(
                        "run-summary query scan exceeds {MAX_SCANNED_SUMMARIES} records"
                    ),
                });
            }
            let (key, bytes) = item.map_err(error::redb)?;
            if query
                .cursor
                .as_ref()
                .is_some_and(|cursor| key.value() <= cursor.after_run.as_str())
            {
                continue;
            }
            let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
            if key.value() != summary.run.as_str() {
                return Err(error::corruption(
                    "run-summary key does not match its document",
                ));
            }
            validate_summary_head(&heads, &summary)?;
            if query
                .filter
                .state
                .is_some_and(|state| state != summary.state)
                || query
                    .filter
                    .workflow
                    .as_ref()
                    .is_some_and(|workflow| workflow != &summary.workflow)
            {
                continue;
            }
            if runs.len() == query.limit.get() as usize {
                has_more = true;
                break;
            }
            runs.push(summary);
        }
        let next = if has_more {
            let after_run = runs
                .last()
                .map(|summary| summary.run.clone())
                .ok_or_else(|| error::corruption("non-empty summary page lost its cursor"))?;
            Some(milkdrift_persistence::RunSummaryCursor { after_run })
        } else {
            None
        };
        Ok(RunSummaryPage { runs, next })
    }

    fn nonterminal_run_page(
        &self,
        cursor: Option<&milkdrift_persistence::RunSummaryCursor>,
        limit: PageSize,
    ) -> Result<RunSummaryPage, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let index = read.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
        let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let mut results = Vec::with_capacity(limit.get() as usize);
        let mut has_more = false;
        let lower = cursor.map_or(Bound::Unbounded, |cursor| {
            Bound::Excluded(cursor.after_run.as_str())
        });
        for item in index
            .range::<&str>((lower, Bound::Unbounded))
            .map_err(error::redb)?
        {
            let (run, marker) = item.map_err(error::redb)?;
            if marker.value() != 1 {
                return Err(error::corruption(
                    "nonterminal index contains an invalid marker",
                ));
            }
            if results.len() == limit.get() as usize {
                has_more = true;
                break;
            }
            let bytes = summaries
                .get(run.value())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("nonterminal run summary is missing"))?;
            let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
            if summary.run.as_str() != run.value() || summary.state == IndexedRunState::Terminal {
                return Err(error::corruption(
                    "nonterminal index contains an invalid summary",
                ));
            }
            validate_summary_head(&heads, &summary)?;
            results.push(summary);
        }
        let next = if has_more {
            let after_run = results
                .last()
                .map(|summary| summary.run.clone())
                .ok_or_else(|| error::corruption("non-empty nonterminal page lost its cursor"))?;
            Some(milkdrift_persistence::RunSummaryCursor { after_run })
        } else {
            None
        };
        Ok(RunSummaryPage {
            runs: results,
            next,
        })
    }

    fn runnable_page(
        &self,
        eligible_through: TimestampMillis,
        cursor: Option<&RunnableCursor>,
        limit: PageSize,
    ) -> Result<RunnablePage, PersistenceError> {
        // The timestamp-ordered index is useful for integrity and rebuildability, but
        // taking its first N rows can return N executions from one noisy run. The
        // identity index is grouped by run, so retain only the best eligible row for
        // each group and apply the page bound to distinct runs. Memory remains bounded
        // by the requested page even when one run owns many indexed executions.
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
        let mut results = Vec::with_capacity(limit.get() as usize);
        let mut current_run: Option<RunId> = None;
        let mut best: Option<RunnableIndexEntry> = None;
        let lower = cursor
            .map(|cursor| codec::component(cursor.after_run.as_str()))
            .transpose()?
            .and_then(codec::prefix_end);
        let mut rows = match &lower {
            Some(lower) => table.range(lower.as_slice()..).map_err(error::redb)?,
            None => table.iter().map_err(error::redb)?,
        };
        let mut has_more = false;
        for item in &mut rows {
            let (key, value) = item.map_err(error::redb)?;
            let entry: RunnableIndexEntry = json::decode(value.value(), "runnable index")?;
            let expected_key = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
            if key.value() != expected_key.as_slice() {
                return Err(error::corruption(
                    "runnable identity key does not match its document",
                ));
            }

            if current_run.as_ref().is_some_and(|run| run != &entry.run) {
                if let Some(selected) = best.take() {
                    results.push(selected);
                    if results.len() == limit.get() as usize {
                        has_more = true;
                        break;
                    }
                }
            }
            if current_run.as_ref() != Some(&entry.run) {
                current_run = Some(entry.run.clone());
            }
            if entry.eligible_at <= eligible_through
                && best
                    .as_ref()
                    .is_none_or(|selected| runnable_precedes(&entry, selected))
            {
                best = Some(entry);
            }
        }
        if !has_more && results.len() < limit.get() as usize {
            if let Some(selected) = best {
                results.push(selected);
            }
        }
        let next = if has_more {
            let after_run = results
                .last()
                .map(|entry| entry.run.clone())
                .ok_or_else(|| error::corruption("non-empty runnable page lost its cursor"))?;
            Some(RunnableCursor { after_run })
        } else {
            None
        };
        Ok(RunnablePage {
            entries: results,
            next,
        })
    }

    fn active_leases(&self, limit: PageSize) -> Result<Vec<LeaseIndexEntry>, PersistenceError> {
        read_ordered_index(
            self,
            LEASE_INDEX,
            TimestampMillis::new(u64::MAX),
            limit,
            "lease index",
            |entry: &LeaseIndexEntry| entry.expires_at,
            lease_order_key,
        )
    }

    fn due_timers(
        &self,
        due_through: TimestampMillis,
        limit: PageSize,
    ) -> Result<Vec<TimerIndexEntry>, PersistenceError> {
        read_ordered_index(
            self,
            TIMER_INDEX,
            due_through,
            limit,
            "timer index",
            |entry: &TimerIndexEntry| entry.fire_at,
            timer_order_key,
        )
    }

    fn expired_leases(
        &self,
        expired_through: TimestampMillis,
        limit: PageSize,
    ) -> Result<Vec<LeaseIndexEntry>, PersistenceError> {
        read_ordered_index(
            self,
            LEASE_INDEX,
            expired_through,
            limit,
            "lease index",
            |entry: &LeaseIndexEntry| entry.expires_at,
            lease_order_key,
        )
    }
}

fn runnable_precedes(candidate: &RunnableIndexEntry, selected: &RunnableIndexEntry) -> bool {
    candidate.priority > selected.priority
        || (candidate.priority == selected.priority
            && (candidate.eligible_at < selected.eligible_at
                || (candidate.eligible_at == selected.eligible_at
                    && candidate.execution < selected.execution)))
}

fn validate_summary_head<T>(heads: &T, summary: &RunSummaryIndex) -> Result<(), PersistenceError>
where
    T: redb::ReadableTable<&'static str, u64>,
{
    let head = heads
        .get(summary.run.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("run summary has no authoritative journal head"))?;
    if head.value() != summary.through_sequence.get() {
        return Err(error::corruption(
            "run summary sequence does not match the authoritative journal head",
        ));
    }
    Ok(())
}

pub(crate) fn decode_stored_event(
    bytes: &[u8],
) -> Result<milkdrift_persistence::RunEventEnvelope, PersistenceError> {
    milkdrift_persistence::RunEventEnvelope::from_json(bytes).map_err(|cause| match cause {
        PersistenceError::UnsupportedVersion { .. } | PersistenceError::Corruption(_) => cause,
        other => {
            PersistenceError::Corruption(format!("stored run event failed verification: {other}"))
        }
    })
}

fn read_ordered_index<T: for<'de> Deserialize<'de> + Serialize>(
    store: &RedbStore,
    definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    through: TimestampMillis,
    limit: PageSize,
    family: &'static str,
    timestamp: impl Fn(&T) -> TimestampMillis,
    order_key: impl Fn(&T) -> Result<Vec<u8>, PersistenceError>,
) -> Result<Vec<T>, PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    let table = read.open_table(definition).map_err(error::redb)?;
    let mut results = Vec::with_capacity(limit.get() as usize);
    for item in table.iter().map_err(error::redb)? {
        let (key, value) = item.map_err(error::redb)?;
        let entry: T = json::decode(value.value(), family)?;
        if key.value() != order_key(&entry)?.as_slice() {
            return Err(error::corruption(format!(
                "{family} key does not match its document"
            )));
        }
        if timestamp(&entry) > through {
            break;
        }
        results.push(entry);
        if results.len() == limit.get() as usize {
            break;
        }
    }
    Ok(results)
}

impl WorkspaceStore for RedbStore {
    fn workspace_usage(&self, run: &RunId) -> Result<WorkspaceUsage, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
        table
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace usage"))
            .transpose()
            .map(|usage| usage.unwrap_or(WorkspaceUsage::EMPTY))
    }

    fn scope(
        &self,
        run: &RunId,
        scope: &ScopeId,
    ) -> Result<Option<WorkspaceScope>, PersistenceError> {
        let key = codec::pair(run.as_str(), scope.as_str())?;
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(SCOPES).map_err(error::redb)?;
        table
            .get(key.as_slice())
            .map_err(error::redb)?
            .map(|bytes| {
                let stored: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
                if stored.reference().run() != run || stored.reference().scope() != scope {
                    return Err(error::corruption(
                        "workspace-scope key does not match its document",
                    ));
                }
                Ok(stored)
            })
            .transpose()
    }

    fn value(
        &self,
        reference: &WorkspaceValueReference,
    ) -> Result<Option<WorkspaceValueEntry>, PersistenceError> {
        let key = workspace_value_key(reference)?;
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(VALUES).map_err(error::redb)?;
        table
            .get(key.as_slice())
            .map_err(error::redb)?
            .map(|bytes| {
                let stored: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
                if stored.reference() != reference {
                    return Err(error::corruption(
                        "workspace-value key does not match its document",
                    ));
                }
                Ok(stored)
            })
            .transpose()
    }

    fn latest_value(
        &self,
        scope: &ScopeReference,
        key: &ValueKey,
    ) -> Result<Option<WorkspaceValueEntry>, PersistenceError> {
        let prefix =
            codec::value_prefix(scope.run().as_str(), scope.scope().as_str(), key.as_str())?;
        let end = codec::prefix_end(prefix.clone())
            .ok_or_else(|| error::corruption("workspace value prefix has no range end"))?;
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(VALUES).map_err(error::redb)?;
        let mut range = table
            .range(prefix.as_slice()..end.as_slice())
            .map_err(error::redb)?;
        let Some(item) = range.next_back() else {
            return Ok(None);
        };
        let (stored_key, bytes) = item.map_err(error::redb)?;
        let entry: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
        if entry.reference().scope() != scope || entry.reference().key() != key {
            return Err(error::corruption(
                "workspace latest-value range contains a mismatched document",
            ));
        }
        if stored_key.value() != workspace_value_key(entry.reference())?.as_slice() {
            return Err(error::corruption(
                "workspace-value key does not match its document",
            ));
        }
        Ok(Some(entry))
    }

    fn scope_lineage(
        &self,
        leaf: &ScopeReference,
    ) -> Result<Vec<WorkspaceScope>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(SCOPES).map_err(error::redb)?;
        let mut current = leaf.clone();
        let mut reversed = Vec::new();
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_SCOPE_DEPTH {
            if !seen.insert(current.clone()) {
                return Err(error::corruption(
                    "workspace scope lineage contains a cycle",
                ));
            }
            let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
            let bytes = table
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| PersistenceError::NotFound {
                    entity: "workspace_scope",
                    identity: format!("{}/{}", current.run(), current.scope()),
                })?;
            let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
            if scope.reference() != &current {
                return Err(error::corruption(
                    "workspace-scope key does not match its document",
                ));
            }
            let parent = scope.parent().cloned();
            reversed.push(scope);
            match parent {
                Some(parent) => current = parent,
                None => {
                    reversed.reverse();
                    milkdrift_workspace::ScopeLineage::new(reversed.clone()).map_err(|cause| {
                        error::corruption(format!(
                            "stored workspace scope lineage failed validation: {cause}"
                        ))
                    })?;
                    return Ok(reversed);
                }
            }
        }
        Err(error::corruption(format!(
            "workspace scope lineage exceeds {MAX_SCOPE_DEPTH} entries"
        )))
    }
}
