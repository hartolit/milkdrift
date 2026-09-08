//! Save a runtime decision together with the facts needed to recover it.
//!
//! Runtime plans events; [`RunJournal`] saves them with the command result, workspace
//! changes, account transitions, and discovery indexes in one transaction. A
//! [`CommandReceipt`] lets the caller recover a lost reply without another append.
//!
//! Use [`AtomicRunCommitRequest`] to assemble that transaction. [`RunQueryStore`] and
//! [`WorkspaceStore`] read the saved facts, while [`RevisionStore`] owns immutable
//! definitions and [`ArtifactStore`] owns content publication. [`SnapshotStore`] provides
//! optional verified replay checkpoints. None of these ports decides what a run should do.
//!
//! Application receipts retain external responses; peer records retain remote acceptance;
//! controller accounts reserve cumulative resources. Their ports document which changes
//! must commit together and how to recover an interrupted call. [`ClockWatermarkStore`]
//! remembers observed time and [`StorageAdmin`] supports explicit integrity inspection.
//! Storage adapters implement these synchronous contracts without exposing database types.

mod admin;
mod application;
mod artifact;
mod bounded;
mod clock;
mod controller_account;
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
pub use clock::{ClockWatermarkObservation, ClockWatermarkStore};
pub use controller_account::{
    ControllerAccountAction, ControllerAccountBlock, ControllerAccountDeclaration,
    ControllerAccountId, ControllerAccountState, ControllerAccountStore,
    ControllerAccountTransaction, ControllerAdmissionDenial, ControllerAdmissionOutcome,
    ControllerArtifactChargeOutcome, ControllerArtifactOwner, ControllerReservation,
    ControllerReservationId, ControllerResourceBudget, ControllerResourceTotals,
    ControllerTransitionId,
};
pub use document::{
    MAX_EVENT_DOCUMENT_BYTES, RUN_EVENT_SCHEMA_VERSION_V1, RUN_EVENT_SCHEMA_VERSION_V2,
    RUN_EVENT_SCHEMA_VERSION_V3, RunEventEnvelope,
};
pub use error::{PersistenceError, StorageFailureClass};
pub use event::{
    AttemptUsage, AuthorityDecision, BranchResultReference, ControllerAssessmentBoundary,
    ControllerAssessmentOutcome, JoinRule, MAX_RECONCILIATION_PLAN_ITEMS,
    MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS, MAX_REPEAT_CONTINUATION_CYCLES,
    MAX_REPEAT_CONTINUATION_DECISIONS, MAX_REPEAT_EFFECTIVE_ITERATIONS, MonetaryUsage,
    NodeExecutionMode, NodeOutcome, ReconciliationAction, ReconciliationClassification,
    ReconciliationItem, ReconciliationPolicy, RecoveryClassification, RepeatContinuationCause,
    RepeatContinuationDecision, RepeatTerminationReason, RunEventKind, RunOutcome,
    SignalDeliveryMode, SubworkflowOwnership, SubworkflowResourceUsage, WaitCondition,
    WaitSatisfaction,
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
    PEER_EXECUTION_RECORD_SCHEMA_VERSION_V2, PEER_EXECUTION_RECORD_SCHEMA_VERSION_V3,
    PEER_EXECUTION_TOMBSTONE_SCHEMA_VERSION_V1, PeerAcceptedAuthoritySummary, PeerAdmission,
    PeerAdmissionOutcome, PeerAdmissionRejection, PeerArchivedDisposition, PeerCancellationRecord,
    PeerCatalogState, PeerClaimOutcome, PeerDispatchClaim, PeerDispatchClaimRequest,
    PeerEntryEvidence, PeerEntryOutcome, PeerEntryRequest, PeerExecutionAccounting,
    PeerExecutionPhase, PeerExecutionRecord, PeerExecutionSnapshot, PeerExecutionStatus,
    PeerExecutionStore, PeerExecutionTombstone, PeerObservationAppend, PeerObservationPage,
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
