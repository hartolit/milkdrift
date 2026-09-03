use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Bound,
};

use milkdrift_authority::ActorRef;
use milkdrift_capability::InvocationId;
use milkdrift_persistence::{
    ActiveLeaseSnapshot, AtomicRunCommitOutcome, AtomicRunCommitRequest,
    COMMAND_RESULT_SCHEMA_VERSION_V1, COMMAND_RESULT_SCHEMA_VERSION_V2, CommandId, CommandReceipt,
    CommandResultDocument, ControllerTransitionId, EventCursor, EventPage, EventPageQuery,
    IndexedRunState, IntegrityDigest, LeaseIndexEntry, LeaseIndexMutation,
    MAX_VALUE_PROVENANCE_DEPTH, PageSize, PersistenceError, RunDiscoveryIntegrityStore,
    RunEventEnvelope, RunEventKind, RunJournal, RunQueryStore, RunSequence, RunSummaryIndex,
    RunSummaryPage, RunSummaryPageQuery, RunnableCursor, RunnableIndexEntry, RunnableIndexMutation,
    RunnablePage, TimerIndexEntry, TimerIndexMutation, TimestampMillis, WorkspaceMutation,
    WorkspaceStore,
};
use milkdrift_workspace::{
    ArtifactReference, MAX_SCOPE_DEPTH, RunId, ScopeId, ScopeKind, ScopeReference, ValueKey,
    ValueOrigin, WorkspaceScope, WorkspaceUsage, WorkspaceValueEntry, WorkspaceValueReference,
};
use redb::{ReadableTable, ReadableTableMetadata, Table};
use serde::{Deserialize, Serialize};

use crate::{
    RedbStore, codec, error,
    fault::FaultPoint,
    json,
    schema::{
        ARTIFACT_METADATA, ARTIFACT_RESERVATIONS, COMMAND_RESULTS, LEASE_ENTRIES, LEASE_INDEX,
        LEASE_SET_REVISION_KEY, METADATA, NONTERMINAL_RUNS, NONTERMINAL_SET_COUNT_KEY, ROOT_SCOPES,
        RUN_EVENTS, RUN_HEADS, RUN_SUMMARIES, RUNNABLE_ENTRIES, RUNNABLE_INDEX, RUNNABLE_RUN_HEADS,
        SCOPES, SIGNAL_RECEIPTS, TIMER_ENTRIES, TIMER_INDEX, VALUES, WORKSPACE_BUDGETS,
        WORKSPACE_USAGE, WORKSPACE_VALUE_HEADS,
    },
};

mod append;
mod discovery;
mod queries;
mod workspace;

pub(crate) use append::{
    OwnedCommandRecord, advance_workspace_global_usage_in_transaction, decode_command_record,
    persist_workspace_value_usage_accounting_in_transaction, validate_command_record_history,
    validate_or_initialize_workspace_domain, validate_run_history_membership,
    validate_run_history_membership_in_transaction, validate_stored_command_record,
    validate_workspace_domain_in_transaction, workspace_domain_in_transaction,
};
pub(crate) use discovery::{
    first_runnable_for_run, lease_order_key, lease_set_revision_in_transaction, runnable_order_key,
    timer_order_key, validate_runnable_head,
};
pub(crate) use queries::{
    validate_signal_receipt_row, validated_run_head, validated_run_head_in_transaction,
};
pub(crate) use workspace::{
    validate_owning_workspace_scope, validate_scope_lineage_in_transaction,
    validate_workspace_value_provenance,
    validate_workspace_value_storage_provenance_in_transaction, validated_workspace_domain,
    workspace_value_key,
};

const INVOCATION_FACT_PREFIX: &str = "invocation_fact\0";

pub(crate) fn invocation_fact_key(run: &RunId, invocation: &InvocationId) -> String {
    format!("{INVOCATION_FACT_PREFIX}{run}\0{invocation}")
}

pub(crate) fn validate_invocation_fact_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    invocation: &InvocationId,
) -> Result<(), PersistenceError> {
    let metadata = write.open_table(METADATA).map_err(error::redb)?;
    let sequence = metadata
        .get(invocation_fact_key(run, invocation).as_str())
        .map_err(error::redb)?
        .map(|sequence| RunSequence::new(sequence.value()))
        .ok_or_else(|| PersistenceError::NotFound {
            entity: "artifact_provenance_invocation",
            identity: invocation.to_string(),
        })?;
    drop(metadata);
    let head = validated_run_head_in_transaction(write, run)?;
    if sequence == RunSequence::ZERO || sequence > head {
        return Err(error::corruption(
            "invocation fact points outside authoritative history",
        ));
    }
    validate_run_history_membership_in_transaction(write, run, head)?;
    let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    let event = events
        .get(codec::run_sequence(run.as_str(), sequence)?.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("invocation fact event is absent"))?;
    let event = RunEventEnvelope::from_json(event.value())?;
    if event.run_id() != run
        || event.sequence() != sequence
        || !matches!(
            event.kind(),
            RunEventKind::NodeScheduled {
                invocation: scheduled,
                ..
            } if scheduled == invocation
        )
    {
        return Err(error::corruption(
            "invocation fact does not name its authoritative scheduling event",
        ));
    }
    crate::snapshot::validate_history_link_in_transaction(write, &event).map(|_| ())
}

pub(crate) fn validate_invocation_fact_row(
    read: &redb::ReadTransaction,
    key: &str,
    sequence: u64,
) -> Result<(), PersistenceError> {
    let suffix = key
        .strip_prefix(INVOCATION_FACT_PREFIX)
        .ok_or_else(|| error::corruption("metadata contains an unknown record"))?;
    let mut components = suffix.split('\0');
    let run = components
        .next()
        .ok_or_else(|| error::corruption("invocation fact key has no run"))?;
    let invocation = components
        .next()
        .ok_or_else(|| error::corruption("invocation fact key has no invocation"))?;
    if components.next().is_some() {
        return Err(error::corruption(
            "invocation fact key has trailing components",
        ));
    }
    let run = RunId::new(run)
        .map_err(|cause| error::corruption(format!("invalid invocation-fact run: {cause}")))?;
    let invocation = InvocationId::new(invocation)
        .map_err(|cause| error::corruption(format!("invalid invocation-fact identity: {cause}")))?;
    let metadata = read.open_table(METADATA).map_err(error::redb)?;
    if metadata
        .get(key)
        .map_err(error::redb)?
        .map(|stored| stored.value())
        != Some(sequence)
    {
        return Err(error::corruption("invocation fact changed during scrub"));
    }
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
    let head = validated_run_head(&heads, &events, &run)?;
    validate_run_history_membership(read, &run, head)?;
    let sequence = RunSequence::new(sequence);
    if sequence == RunSequence::ZERO || sequence > head {
        return Err(error::corruption(
            "invocation fact points outside authoritative history",
        ));
    }
    let event = events
        .get(codec::run_sequence(run.as_str(), sequence)?.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("invocation fact event is absent"))?;
    let event = RunEventEnvelope::from_json(event.value())?;
    if !matches!(
        event.kind(),
        RunEventKind::NodeScheduled {
            invocation: scheduled,
            ..
        } if scheduled == &invocation
    ) {
        return Err(error::corruption(
            "invocation fact does not name its authoritative scheduling event",
        ));
    }
    crate::snapshot::validate_history_link(read, &event).map(|_| ())
}
