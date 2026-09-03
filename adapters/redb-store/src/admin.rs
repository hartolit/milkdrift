use std::ops::Bound;

use crate::{
    RedbStore, codec, error, json,
    schema::{
        APPLICATION_COLD_RECEIPT_COUNT_KEY, APPLICATION_COMMAND_RECEIPTS_COLD,
        APPLICATION_COMMAND_RECEIPTS_HOT, APPLICATION_HOT_RECEIPT_COUNT_KEY,
        APPLICATION_HOT_RECEIPTS_BY_COMPLETION, APPLICATION_LAYOUTS, APPLICATION_PROPOSALS,
        ARTIFACT_ACCOUNTING, ARTIFACT_DELETE_GUARDS, ARTIFACT_DIGEST_RESERVATIONS,
        ARTIFACT_MANIFEST, ARTIFACT_METADATA, ARTIFACT_PATHS, ARTIFACT_PUBLICATIONS,
        ARTIFACT_PUBLICATIONS_BY_AGE, ARTIFACT_REFERENCES, ARTIFACT_RESERVATIONS,
        ARTIFACT_TEMP_MANIFEST, ARTIFACT_TEMP_OWNERS, ARTIFACTS_BY_DIGEST, COMMAND_RESULTS,
        CONTROLLER_ACCOUNT_REVISIONS, CONTROLLER_ACCOUNTS, CONTROLLER_ARTIFACT_CHARGES,
        CONTROLLER_RUN_BINDINGS, CONTROLLER_TRANSITIONS, LEASE_ENTRIES, LEASE_INDEX, METADATA,
        NONTERMINAL_RUNS, REVISIONS, REVISIONS_BY_DIGEST, ROOT_SCOPES, RUN_ARTIFACT_OWNERSHIP,
        RUN_EVENTS, RUN_HEADS, RUN_SUMMARIES, RUNNABLE_ENTRIES, RUNNABLE_INDEX, RUNNABLE_RUN_HEADS,
        SCHEMA_VERSION_KEY, SCOPES, SECURITY_AUDIT, SECURITY_AUDIT_COUNT_KEY, SIGNAL_RECEIPTS,
        SNAPSHOT_LATEST, SNAPSHOTS, TIMER_ENTRIES, TIMER_INDEX, VALUES, WORKSPACE_BUDGETS,
        WORKSPACE_USAGE, WORKSPACE_VALUE_HEADS,
    },
};
use milkdrift_blueprint::BlueprintRevisionDocument;
use milkdrift_persistence::{
    ApplicationCommandStore, ArtifactPublicationId, BoundedDetail, IndexedRunState,
    IntegrityScanCursor, IntegrityScanFamily, IntegrityScanRequest, IntegrityScanResult,
    LeaseIndexEntry, PersistenceError, RevisionSummary, RunnableIndexEntry, SignalId,
    SnapshotDocument, SnapshotId, StorageAdmin, StorageComponentHealth, StorageHealth,
    StorageHealthStatus, StorageSchemaCompatibility, StorageSchemaInfo, TimerIndexEntry,
    TimestampMillis,
};
use milkdrift_workspace::{
    ArtifactMetadata, ArtifactReference, RunId, ScopeId, ScopeKind, WorkspaceBudget,
    WorkspaceScope, WorkspaceUsage, WorkspaceValueEntry, WorkspaceValueReference,
};

mod cursor;
mod integrity;
mod service;
