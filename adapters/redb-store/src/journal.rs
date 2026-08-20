use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Bound,
};

use milkdrift_persistence::{
    ActiveLeaseSnapshot, ActorRef, AtomicRunCommitOutcome, AtomicRunCommitRequest,
    COMMAND_RESULT_SCHEMA_VERSION_V1, CommandId, CommandReceipt, CommandResultDocument,
    EventCursor, EventPage, EventPageQuery, IndexedRunState, IntegrityDigest, LeaseIndexEntry,
    LeaseIndexMutation, MAX_VALUE_PROVENANCE_DEPTH, PageSize, PersistenceError, RunEventKind,
    RunJournal, RunQueryStore, RunSequence, RunSummaryIndex, RunSummaryPage, RunSummaryPageQuery,
    RunnableCursor, RunnableIndexEntry, RunnableIndexMutation, RunnablePage, TimerIndexEntry,
    TimerIndexMutation, TimestampMillis, WorkspaceMutation, WorkspaceStore,
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
        ARTIFACT_METADATA, ARTIFACT_RESERVATIONS, COMMAND_RESULTS, EVENT_CHECKSUMS, LEASE_ENTRIES,
        LEASE_INDEX, NONTERMINAL_RUNS, ROOT_SCOPES, RUN_EVENTS, RUN_HEADS, RUN_SUMMARIES,
        RUNNABLE_ENTRIES, RUNNABLE_INDEX, RUNNABLE_RUN_HEADS, SCOPES, TIMER_ENTRIES, TIMER_INDEX,
        VALUES, WORKSPACE_BUDGETS, WORKSPACE_USAGE, WORKSPACE_VALUE_ACCOUNTING,
        WORKSPACE_VALUE_HEADS,
    },
};

mod append;
mod discovery;
mod queries;
mod workspace;

pub(crate) use append::{
    advance_workspace_global_usage_in_transaction, event_catalog_path,
    migrate_nonterminal_membership, persist_run_membership,
    persist_workspace_value_usage_accounting_in_transaction,
    validate_or_initialize_workspace_domain, validate_run_history_membership,
    validate_run_history_membership_in_transaction, validate_stored_command_record,
    validate_workspace_domain_in_transaction, validate_workspace_value_accounting,
    validate_workspace_value_accounting_in_transaction,
};
pub(crate) use discovery::{
    lease_catalog_ordered_path, lease_order_key, migrate_runnable_run_heads,
    runnable_catalog_identity_path, runnable_catalog_ordered_path, runnable_order_key,
    timer_catalog_ordered_path, timer_order_key,
};
pub(crate) use queries::{validated_run_head, validated_run_head_in_transaction};
pub(crate) use workspace::{
    migrate_workspace_catalogs, validate_owning_workspace_scope,
    validate_workspace_value_provenance, validated_workspace_domain, workspace_value_key,
};
