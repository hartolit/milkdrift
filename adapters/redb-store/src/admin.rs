//! Bounded read-only inspection of task, artifact and independent host records.
use crate::{
    RedbStore, codec, error, json,
    schema::{
        ARTIFACT_ACCOUNTING, ARTIFACT_DELETE_GUARDS, ARTIFACT_DIGEST_RESERVATIONS,
        ARTIFACT_MANIFEST, ARTIFACT_METADATA, ARTIFACT_OWNERSHIP, ARTIFACT_PATHS,
        ARTIFACT_PUBLICATIONS, ARTIFACT_PUBLICATIONS_BY_AGE, ARTIFACT_REFERENCES,
        ARTIFACT_RESERVATIONS, ARTIFACT_TEMP_MANIFEST, ARTIFACT_TEMP_OWNERS, ARTIFACTS_BY_DIGEST,
        METADATA, NATIVE_PATH_CLAIMS, PEER_EXECUTION_TOMBSTONES, PEER_EXECUTIONS,
        SCHEMA_VERSION_KEY, SECURITY_AUDIT, TASK_ACCOUNTING, TASK_ACTION_OWNERS, TASK_OUTPUT_USAGE,
        TASK_RECEIPTS, TASK_WORKFLOWS, WORKSPACE_BUDGETS, WORKSPACE_USAGE,
    },
};
use milkdrift_persistence::{
    ArtifactPublicationId, BoundedDetail, IntegrityScanCursor, IntegrityScanFamily,
    IntegrityScanRequest, IntegrityScanResult, PersistenceError, StorageAdmin,
    StorageComponentHealth, StorageHealth, StorageHealthStatus, StorageSchemaCompatibility,
    StorageSchemaInfo, TimestampMillis,
};
use milkdrift_workspace::{ArtifactMetadata, ArtifactReference};
use std::ops::Bound;
mod cursor;
mod integrity;
mod scan;
mod service;
