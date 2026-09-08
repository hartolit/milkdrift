//! Save a runtime decision together with the facts needed to recover it.
//!
//! The runtime decides whether a command is allowed and which events it produces.
//! This crate gives it storage documents and synchronous traits that a storage adapter
//! implements. For example, accepting a new run must save its creation event, root
//! workspace scope, usage, discovery summary, and command result together. Otherwise
//! restart could find a run whose inputs or saved response are missing.
//!
//! Follow [`CommandReceipt`] into [`AtomicRunCommitRequest`] and call
//! [`RunJournal::commit_command`] through the configured storage implementation.
//! Constructing the request checks its internal consistency; it writes nothing.
//! Storage checks current history and commits all consequences or none. A command the
//! runtime rejects can still have a saved result, with no events or workspace changes.
//!
//! If a response is lost after commit, redelivering the same command returns
//! [`AtomicRunCommitOutcome::Replayed`]. Reusing its identity for different intent
//! conflicts. [`RunJournal::command_result`] retrieves the saved outcome, and
//! [`RunQueryStore::events`] reads the supporting history in bounded pages. Neither
//! a saved command acceptance nor an error proves whether external work finished;
//! the runtime must interpret durable attempt observations before allowing another try.
//!
//! Other owners use [`RevisionStore`] for immutable definitions, [`ArtifactStore`] for
//! content publication, and [`SnapshotStore`] for optional recovery checkpoints.
//! Application, peer, clock, and controller-account ports keep their own explicit
//! transactions. This library opens no database and runs no capability or worker.
//! The package README traces the runtime caller and the redb implementation; detailed
//! commit obligations and recovery guidance start at [`RunJournal`].

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
