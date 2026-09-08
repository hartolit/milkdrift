//! Inspect physical storage without changing workflow history.
//!
//! Health samples selected components. Explicit scans traverse records and derived
//! relationships in bounded pages, optionally rehashing artifact content. The cursor
//! carries the physical phase and verification mode; schema/phase order is part of its
//! compatibility contract. Repair and artifact cleanup are not hidden inside inspection.

use std::ops::Bound;

use crate::{
    RedbStore, codec, error, json, schema::APPLICATION_COLD_RECEIPT_COUNT_KEY,
    schema::APPLICATION_COMMAND_RECEIPTS_COLD, schema::APPLICATION_COMMAND_RECEIPTS_HOT,
    schema::APPLICATION_HOT_RECEIPT_COUNT_KEY, schema::APPLICATION_HOT_RECEIPTS_BY_COMPLETION,
    schema::APPLICATION_LAYOUTS, schema::APPLICATION_PROPOSALS, schema::ARTIFACT_ACCOUNTING,
    schema::ARTIFACT_DELETE_GUARDS, schema::ARTIFACT_DIGEST_RESERVATIONS,
    schema::ARTIFACT_MANIFEST, schema::ARTIFACT_METADATA, schema::ARTIFACT_PATHS,
    schema::ARTIFACT_PUBLICATIONS, schema::ARTIFACT_PUBLICATIONS_BY_AGE,
    schema::ARTIFACT_REFERENCES, schema::ARTIFACT_RESERVATIONS, schema::ARTIFACT_TEMP_MANIFEST,
    schema::ARTIFACT_TEMP_OWNERS, schema::ARTIFACTS_BY_DIGEST, schema::COMMAND_RESULTS,
    schema::CONTROLLER_ACCOUNT_REVISIONS, schema::CONTROLLER_ACCOUNTS,
    schema::CONTROLLER_ARTIFACT_CHARGES, schema::CONTROLLER_RUN_BINDINGS,
    schema::CONTROLLER_TRANSITIONS, schema::LEASE_ENTRIES, schema::LEASE_INDEX, schema::METADATA,
    schema::NONTERMINAL_RUNS, schema::REVISIONS, schema::REVISIONS_BY_DIGEST, schema::ROOT_SCOPES,
    schema::RUN_ARTIFACT_OWNERSHIP, schema::RUN_EVENTS, schema::RUN_HEADS, schema::RUN_SUMMARIES,
    schema::RUNNABLE_ENTRIES, schema::RUNNABLE_INDEX, schema::RUNNABLE_RUN_HEADS,
    schema::SCHEMA_VERSION_KEY, schema::SCOPES, schema::SECURITY_AUDIT,
    schema::SECURITY_AUDIT_COUNT_KEY, schema::SIGNAL_RECEIPTS, schema::SNAPSHOT_LATEST,
    schema::SNAPSHOTS, schema::TIMER_ENTRIES, schema::TIMER_INDEX, schema::VALUES,
    schema::WORKSPACE_BUDGETS, schema::WORKSPACE_USAGE, schema::WORKSPACE_VALUE_HEADS,
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
mod scan;
mod service;
