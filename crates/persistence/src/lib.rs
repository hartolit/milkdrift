//! Atomic task snapshots and exact receipts, bounded artifacts, peer executions and host lifetime guards.

mod admin;
mod artifact;
mod bounded;
mod clock;
mod daemon;
mod error;
mod identity;
mod native_paths;
mod peer;
mod project;
mod security;
mod task;
pub use native_paths::{NativePathClaim, NativePathClaimStore, NativePathRoot};
pub use project::{
    MAX_PROJECT_OPERATION_BYTES, MAX_PROJECT_OPERATIONS, MAX_PROJECTS, ProjectMutation,
    ProjectOperation, ProjectOperationState, ProjectRecord, ProjectStore,
};

pub use task::{
    MAX_WORKFLOW_DOCUMENT_BYTES, MAX_WORKFLOW_REQUEST_BYTES, WorkflowCommandAcceptance,
    WorkflowCommandReceipt, WorkflowCommit, WorkflowCommitOutcome, WorkflowManagedExecution,
    WorkflowPage, WorkflowSnapshot, WorkflowStorageLimits, WorkflowStorageUsage, WorkflowStore,
};

pub use admin::{
    IntegrityScanCursor, IntegrityScanFamily, IntegrityScanRequest, IntegrityScanResult,
    MAX_INTEGRITY_SCAN_CURSOR_KEY_BYTES, StorageAdmin, StorageComponentHealth, StorageHealth,
    StorageHealthStatus, StorageSchemaCompatibility, StorageSchemaInfo,
};
pub use artifact::{
    ArtifactReadAuthority, ArtifactReadChunk, ArtifactReadRequest, ArtifactStore,
    ArtifactWriteProgress, BeginArtifactOutcome, BeginArtifactPublication, CommitArtifactOutcome,
    MAX_ORPHAN_CLEANUP_CURSOR_KEY_BYTES, OrphanCleanupCursor, OrphanCleanupFamily,
    OrphanCleanupRequest, OrphanCleanupResult, authorize_artifact_read,
};
pub use bounded::{
    BoundedDetail, MAX_ARTIFACT_CHUNK_BYTES, MAX_DETAIL_BYTES, MAX_PAGE_SIZE, PageSize,
};
pub use clock::{ClockWatermarkObservation, ClockWatermarkStore};
pub use daemon::{
    DaemonDisplayName, DaemonMetadata, DaemonMetadataStore, DaemonRename, DaemonRenameOutcome,
    DaemonRenameReceipt, MAX_DAEMON_RENAME_RECEIPT_BYTES, MAX_DAEMON_RENAME_RECEIPTS,
};
pub use error::{PersistenceError, StorageFailureClass};
pub use identity::{
    ArtifactPublicationId, AttemptId, EvidenceId, IntegrityDigest, LeaseId, NodeExecutionId,
    RunSequence, TimestampMillis, WorkerId,
};
pub use peer::{
    PeerAcceptedAuthoritySummary, PeerAdmission, PeerAdmissionOutcome, PeerAdmissionRejection,
    PeerArchivedDisposition, PeerCancellationRecord, PeerClaimOutcome, PeerDispatchClaim,
    PeerDispatchClaimRequest, PeerEntryEvidence, PeerEntryOutcome, PeerEntryRequest,
    PeerExecutionAccounting, PeerExecutionPhase, PeerExecutionRecord, PeerExecutionSnapshot,
    PeerExecutionStatus, PeerExecutionStore, PeerExecutionTombstone, PeerObservationAppend,
    PeerObservationPage, PeerRecoveryResult, PeerRetentionPage, PeerRetentionRequest,
    SERVING_EXECUTION_RECORD_SCHEMA_VERSION, SERVING_EXECUTION_TOMBSTONE_SCHEMA_VERSION,
    ServingCallerState, ServingCatalogState,
};

/// Managed resource inventory and atomic lifetime/editing ownership ports.
pub mod managed;

pub use security::{
    ApplicationCursor, ApplicationPage, ApplicationPageQuery, SecurityAuditEntry,
    SecurityAuditRecord, SecurityAuditStore,
};
