use std::sync::Arc;

use milkdrift_persistence::{PersistenceError, StorageFailureClass};

/// Stable durability boundaries exposed for deterministic crash/failure tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FaultPoint {
    /// Immediately before committing a command transaction.
    BeforeCommandCommit,
    /// Immediately after redb has committed a command transaction.
    AfterCommandCommit,
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
    /// After the empty stream is durable, before committing its publication session.
    BeforeArtifactBeginCommit,
    /// Immediately after the publication session transaction commits.
    AfterArtifactBeginCommit,
    /// Before appending a validated artifact chunk to its temporary stream.
    BeforeArtifactChunkWrite,
    /// After an appended artifact chunk has been synchronized to durable storage.
    AfterArtifactChunkSync,
    /// After artifact bytes are synced, before their atomic rename.
    BeforeArtifactRename,
    /// After artifact rename and directory sync, before metadata commit.
    AfterArtifactRename,
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
#[must_use]
pub fn injected_failure(point: FaultPoint) -> PersistenceError {
    PersistenceError::Storage {
        class: StorageFailureClass::Unavailable,
        message: format!("fault injected at {point:?}"),
    }
}
