use std::sync::Arc;

use milkdrift_persistence::PersistenceError;
#[cfg(feature = "test-admin")]
use milkdrift_persistence::StorageFailureClass;

/// Stable durability boundaries exposed for deterministic crash/failure tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FaultPoint {
    /// After schema rows are prepared, immediately before the initializing commit.
    BeforeSchemaCommit,
    /// Immediately after the schema-initialization transaction commits.
    AfterSchemaCommit,
    /// Immediately before committing a command transaction.
    BeforeCommandCommit,
    /// Immediately before an event row is inserted into a command transaction.
    BeforeEventInsert,
    /// After an event row is inserted but before its chain/index/head state is complete.
    AfterEventInsert,
    /// After a history-chain link is updated but before the command transaction commits.
    AfterHistoryChainUpdate,
    /// Immediately after redb has committed a command transaction.
    AfterCommandCommit,
    /// Immediately before committing an application receipt/same-store effect transaction.
    BeforeApplicationCommit,
    /// Immediately after an application receipt/same-store effect transaction commits.
    AfterApplicationCommit,
    /// Immediately before a hot receipt gains cold ownership in an archival transaction.
    BeforeApplicationReceiptColdInsert,
    /// After cold ownership is prepared but before hot ownership is removed atomically.
    AfterApplicationReceiptColdInsert,
    /// After hot ownership is removed but before its completion index and transaction commit.
    AfterApplicationReceiptHotRemove,
    /// Immediately before committing an explicit bounded receipt archival transaction.
    BeforeApplicationReceiptArchiveCommit,
    /// Immediately after an explicit bounded receipt archival transaction commits.
    AfterApplicationReceiptArchiveCommit,
    /// Immediately before committing an immutable revision transaction.
    BeforeRevisionCommit,
    /// Immediately after redb has committed an immutable revision transaction.
    AfterRevisionCommit,
    /// Immediately before committing a projection snapshot transaction.
    BeforeSnapshotCommit,
    /// Immediately after redb has committed a projection snapshot transaction.
    AfterSnapshotCommit,
    /// Immediately before committing a snapshot-discard transaction.
    BeforeSnapshotDiscardCommit,
    /// Immediately after redb has committed a snapshot-discard transaction.
    AfterSnapshotDiscardCommit,
    /// After peer admission rows and accounting are prepared, before their atomic commit.
    BeforePeerAdmissionCommit,
    /// Immediately after durable peer admission commits, before its response is returned.
    AfterPeerAdmissionCommit,
    /// After a peer dispatch claim and its indexes are prepared, before their atomic commit.
    BeforePeerClaimCommit,
    /// Immediately after a durable peer dispatch claim commits.
    AfterPeerClaimCommit,
    /// After a peer observation row and execution head are prepared, before their atomic commit.
    BeforePeerObservationCommit,
    /// Immediately after a durable peer observation commit.
    AfterPeerObservationCommit,
    /// After a compact tombstone is written but before hot ownership is removed atomically.
    AfterPeerTombstoneInsert,
    /// After detailed observation rows/mappings are removed but before hot ownership is removed.
    AfterPeerObservationCleanup,
    /// After hot ownership is removed but before archival accounting and commit.
    AfterPeerHotRemove,
    /// After archival counters are updated but before the transaction commits.
    AfterPeerArchiveAccounting,
    /// Immediately before committing one explicit/automatic peer archival transaction.
    BeforePeerArchiveCommit,
    /// Immediately after a peer archival transaction commits.
    AfterPeerArchiveCommit,
    /// After the empty stream is durable, before committing its publication session.
    BeforeArtifactBeginCommit,
    /// Immediately after the publication session transaction commits.
    AfterArtifactBeginCommit,
    /// After durable pending-temp intent, before creating and syncing the file.
    BeforeArtifactTempCreate,
    /// After the new empty temp file and its directory are durably synchronized.
    AfterArtifactTempCreate,
    /// Before committing the pending-to-ready temp-inventory transition.
    BeforeArtifactTempReadyCommit,
    /// After the pending-to-ready temp-inventory transition commits.
    AfterArtifactTempReadyCommit,
    /// Before appending a validated artifact chunk to its temporary stream.
    BeforeArtifactChunkWrite,
    /// After an appended artifact chunk has been synchronized to durable storage.
    AfterArtifactChunkSync,
    /// After artifact bytes are synced, before their atomic rename.
    BeforeArtifactRename,
    /// After artifact rename and directory sync, before metadata commit.
    AfterArtifactRename,
    /// Before committing durable final-content-path intent ahead of rename.
    BeforeArtifactContentIntentCommit,
    /// After final-content-path intent commits and before rename may proceed.
    AfterArtifactContentIntentCommit,
    /// Immediately before artifact metadata transaction commit.
    BeforeArtifactMetadataCommit,
    /// Immediately after artifact metadata transaction commit.
    AfterArtifactMetadataCommit,
    /// Immediately before committing an artifact-abort transaction.
    BeforeArtifactAbortCommit,
    /// Immediately after an artifact-abort transaction commits.
    AfterArtifactAbortCommit,
    /// After abort state commits, immediately before deleting its temporary stream.
    BeforeArtifactAbortDelete,
    /// After the aborted temporary stream deletion is durably synchronized.
    AfterArtifactAbortDelete,
    /// Immediately before committing expired publication-session cleanup.
    BeforeArtifactCleanupCommit,
    /// Immediately after expired publication-session cleanup commits.
    AfterArtifactCleanupCommit,
    /// Immediately before deleting one unowned artifact file during cleanup.
    BeforeArtifactCleanupDelete,
    /// After deleting and synchronizing one unowned artifact file during cleanup.
    AfterArtifactCleanupDelete,
    /// Before committing removal of a durably deleted path-inventory leaf.
    BeforeArtifactPathFinalizeCommit,
    /// After removal of a durably deleted path-inventory record commits.
    AfterArtifactPathFinalizeCommit,
    /// Before committing a durable delete guard ahead of filesystem unlink.
    BeforeArtifactPathDeleteIntentCommit,
    /// After a durable delete guard commits and before filesystem unlink.
    AfterArtifactPathDeleteIntentCommit,
}

/// Synchronous test hook. Production configuration defaults to a no-op hook.
pub trait FaultInjector: Send + Sync {
    /// Returns an error to fail at the selected boundary.
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError>;
}

#[derive(Default)]
pub(crate) struct NoFaults;

impl FaultInjector for NoFaults {
    fn check(&self, _point: FaultPoint) -> Result<(), PersistenceError> {
        Ok(())
    }
}

pub(crate) fn no_faults() -> Arc<dyn FaultInjector> {
    Arc::new(NoFaults)
}

/// Constructs a stable injected-failure error for simple test injectors.
#[cfg(feature = "test-admin")]
#[must_use]
pub fn injected_failure(point: FaultPoint) -> PersistenceError {
    PersistenceError::Storage {
        class: StorageFailureClass::Unavailable,
        message: format!("fault injected at {point:?}"),
    }
}
