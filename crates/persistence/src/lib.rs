//! Durable, adapter-neutral persistence documents and narrow synchronous ports.
//!
//! This crate owns stable execution identities, schema-v1 checksummed run facts,
//! atomic command/journal contracts, immutable revision storage, verifiable recovery
//! indexes, snapshot integrity, workspace transaction mutations, content-addressed
//! artifact streaming, and storage lifecycle/health boundaries. It owns no runtime
//! transition decisions, async executor, wall clock, filesystem path, database handle,
//! table name, transaction object, generic key/value API, or `redb` type.
//!
//! [`RunJournal::commit_command`] is the sole accepted-command write boundary. A first
//! delivery checks the aggregate sequence and atomically records the exact command
//! receipt/result, contiguous events, workspace state/accounting, and discoverability
//! indexes after proving every artifact reference is committed. Exact redelivery
//! returns the original result without writes; identity reuse with different canonical
//! command bytes is a typed conflict.

mod admin;
mod application;
mod artifact;
mod bounded;
mod document;
mod error;
mod event;
mod identity;
mod journal;
mod peer;
mod revision;
mod snapshot;

pub use admin::{
    IntegrityScanCursor, IntegrityScanFamily, IntegrityScanRequest, IntegrityScanResult,
    MAX_INTEGRITY_SCAN_CURSOR_KEY_BYTES, StorageAdmin, StorageComponentHealth, StorageHealth,
    StorageHealthStatus, StorageSchemaCompatibility, StorageSchemaInfo,
};
pub use application::{
    APPLICATION_COMMAND_RECEIPT_SCHEMA_VERSION_V1, APPLICATION_LAYOUT_RECORD_SCHEMA_VERSION_V1,
    ApplicationCommandCommit, ApplicationCommandCommitOutcome, ApplicationCommandEffect,
    ApplicationCommandReceipt, ApplicationCommandResult, ApplicationCommandStore,
    ApplicationCursor, ApplicationEffectReference, ApplicationLayout, ApplicationLayoutStore,
    ApplicationLayoutUpdate, ApplicationPage, ApplicationPageQuery,
    ApplicationReceiptArchiveOutcome, ApplicationReceiptArchiveRequest, ApplicationReceiptStatus,
    ProposalIndexEntry, ProposalIndexStore, SecurityAuditEntry, SecurityAuditRecord,
    SecurityAuditStore,
};
pub use artifact::{
    ArtifactReadAuthority, ArtifactReadChunk, ArtifactReadRequest, ArtifactStore,
    ArtifactWriteProgress, BeginArtifactOutcome, BeginArtifactPublication, CommitArtifactOutcome,
    MAX_ORPHAN_CLEANUP_CURSOR_KEY_BYTES, OrphanCleanupCursor, OrphanCleanupFamily,
    OrphanCleanupRequest, OrphanCleanupResult, authorize_artifact_read,
};
pub use bounded::{
    BoundedDetail, CurrencyCode, EvidenceKind, EvidenceReference, MAX_ARTIFACT_CHUNK_BYTES,
    MAX_DETAIL_BYTES, MAX_EVENTS_PER_COMMIT, MAX_EVIDENCE_REFERENCES, MAX_PAGE_SIZE,
    MAX_REASON_BYTES, PageSize, Reason,
};
pub use document::{MAX_EVENT_DOCUMENT_BYTES, RUN_EVENT_SCHEMA_VERSION_V1, RunEventEnvelope};
pub use error::{PersistenceError, StorageFailureClass};
pub use event::{
    AttemptUsage, AuthorityDecision, BranchResultReference, JoinRule,
    MAX_RECONCILIATION_PLAN_ITEMS, MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS,
    MAX_REPEAT_CONTINUATION_CYCLES, MAX_REPEAT_CONTINUATION_DECISIONS,
    MAX_REPEAT_EFFECTIVE_ITERATIONS, MonetaryUsage, NodeExecutionMode, NodeOutcome,
    ReconciliationAction, ReconciliationClassification, ReconciliationItem, ReconciliationPolicy,
    RecoveryClassification, RepeatContinuationCause, RepeatContinuationDecision,
    RepeatTerminationReason, RunEventKind, RunOutcome, SignalDeliveryMode, SubworkflowOwnership,
    WaitCondition, WaitSatisfaction,
};
pub use identity::{
    ArtifactPublicationId, AttemptId, CommandId, CorrelationKey, EventId, EvidenceId,
    IntegrityDigest, LeaseId, NodeExecutionId, PublicationId, ReconciliationDecisionId,
    ReconciliationId, ReconciliationPlanId, RepeatDecisionId, RunSequence, SignalId, SignalTypeId,
    SnapshotId, TimerId, TimestampMillis, WorkerId,
};
pub use journal::{
    ActiveLeaseSnapshot, AtomicRunCommitOutcome, AtomicRunCommitRequest,
    COMMAND_RESULT_SCHEMA_VERSION_V1, COMMAND_RESULT_SCHEMA_VERSION_V2, CommandDisposition,
    CommandReceipt, CommandResultDocument, EventCursor, EventPage, EventPageQuery, IndexedRunState,
    LeaseIndexEntry, LeaseIndexMutation, MAX_COMMAND_DOCUMENT_BYTES,
    MAX_COMMAND_RESULT_DOCUMENT_BYTES, MAX_INDEX_MUTATIONS_PER_COMMIT,
    MAX_REQUIRED_ARTIFACTS_PER_COMMIT, MAX_VALUE_PROVENANCE_DEPTH,
    MAX_WORKSPACE_MUTATIONS_PER_COMMIT, RunDiscoveryIntegrityStore, RunIndexUpdate, RunJournal,
    RunQueryStore, RunSummaryCursor, RunSummaryFilter, RunSummaryIndex, RunSummaryPage,
    RunSummaryPageQuery, RunnableCursor, RunnableIndexEntry, RunnableIndexMutation, RunnablePage,
    TimerIndexEntry, TimerIndexMutation, WorkspaceAccounting, WorkspaceMutation, WorkspaceStore,
};
pub use peer::{
    PEER_EXECUTION_RECORD_SCHEMA_VERSION_V1, PeerAdmission, PeerAdmissionOutcome,
    PeerAdmissionRejection, PeerCancellationRecord, PeerCatalogState, PeerClaimOutcome,
    PeerDispatchClaim, PeerDispatchClaimRequest, PeerEntryEvidence, PeerEntryOutcome,
    PeerEntryRequest, PeerExecutionAccounting, PeerExecutionPhase, PeerExecutionRecord,
    PeerExecutionRetention, PeerExecutionStore, PeerObservationAppend, PeerObservationPage,
    PeerRecoveryResult, PeerRelationshipState, PeerRetentionPage, PeerRetentionRequest,
};
pub use revision::{
    ImmutableRevisionPut, RevisionCursor, RevisionFilter, RevisionPage, RevisionPageQuery,
    RevisionStore, RevisionSummary,
};
pub use snapshot::{
    MAX_SNAPSHOT_DOCUMENT_BYTES, MAX_SNAPSHOT_ENCODED_PAYLOAD_BYTES, MAX_SNAPSHOT_PAYLOAD_BYTES,
    ProjectionCheckpoint, SNAPSHOT_ENVELOPE_SCHEMA_VERSION_V2, SnapshotDocument, SnapshotLoad,
    SnapshotStore, history_digest, history_genesis_digest, history_link_digest,
};

// Canonical identities already owned by inward crates are re-exported rather than
// duplicated into wire-incompatible persistence-local wrappers.
/// Compatibility re-export; the canonical actor identity is owned by `milkdrift-authority`.
pub use milkdrift_authority::ActorRef;
pub use milkdrift_capability::InvocationId;
pub use milkdrift_workspace::{BranchId, IterationId, RunId, ScopeId, SubworkflowId};
