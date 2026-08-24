use std::ops::Bound;

use crate::{
    RedbStore, codec, error, json,
    schema::{
        ARTIFACT_ACCOUNTING, ARTIFACT_MANIFEST, ARTIFACT_METADATA, ARTIFACT_REFERENCES,
        ARTIFACT_TEMP_MANIFEST, ARTIFACT_TEMP_OWNERS, ARTIFACTS_BY_DIGEST, COMMAND_RESULTS,
        LEASE_ENTRIES, LEASE_INDEX, METADATA, NONTERMINAL_RUNS, REVISIONS,
        REVISIONS_BY_DIGEST, ROOT_SCOPES, RUN_ARTIFACT_OWNERSHIP, RUN_EVENTS, RUN_HEADS,
        RUN_SUMMARIES, RUNNABLE_ENTRIES, RUNNABLE_INDEX, SCHEMA_VERSION_KEY, SCOPES,
        TIMER_ENTRIES, TIMER_INDEX, VALUES, WORKSPACE_BUDGETS, WORKSPACE_USAGE,
    },
};
use milkdrift_blueprint::BlueprintRevisionDocument;
use milkdrift_persistence::{
    ArtifactPublicationId, BoundedDetail, IndexedRunState, IntegrityScanCursor,
    IntegrityScanFamily, IntegrityScanRequest, IntegrityScanResult, LeaseIndexEntry,
    CURRENT_STORAGE_SCHEMA_VERSION, PersistenceError, RevisionSummary, RunnableIndexEntry,
    StorageAdmin, StorageComponentHealth, StorageHealth, StorageHealthStatus,
    StorageSchemaCompatibility, StorageSchemaInfo, TimerIndexEntry, TimestampMillis,
};
use milkdrift_workspace::{
    ArtifactMetadata, ArtifactReference, RunId, ScopeId, ScopeKind, WorkspaceBudget,
    WorkspaceScope, WorkspaceUsage, WorkspaceValueEntry,
};

mod cursor;
mod integrity;
mod service;
