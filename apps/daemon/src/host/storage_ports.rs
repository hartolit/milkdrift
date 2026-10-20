//! Direct atomic storage calls whose lifetime remains owned by the daemon owner.
//!
//! Adapters can outlive an orderly owner shutdown through an HTTP service handle. They
//! must neither keep the physical store locked nor enter the owner queue while holding
//! a task execution gate. Each call borrows a strong store reference only for its duration.

use std::sync::{Arc, Weak};

use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactReadChunk, ArtifactReadRequest, ArtifactStore,
    ArtifactWriteProgress, BeginArtifactOutcome, BeginArtifactPublication, CommitArtifactOutcome,
    NativePathClaim, NativePathClaimStore, OrphanCleanupRequest, OrphanCleanupResult,
    PersistenceError, StorageFailureClass,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactOwner, ArtifactReference, RunId, WorkspaceUsage,
};

pub(super) struct AdapterStorage {
    store: Weak<RedbStore>,
}

impl AdapterStorage {
    pub(super) fn new(store: &Arc<RedbStore>) -> Self {
        Self {
            store: Arc::downgrade(store),
        }
    }

    fn store(&self) -> Result<Arc<RedbStore>, PersistenceError> {
        self.store
            .upgrade()
            .ok_or_else(|| PersistenceError::Storage {
                class: StorageFailureClass::Unavailable,
                message: "daemon storage owner has stopped".to_owned(),
            })
    }
}

impl NativePathClaimStore for AdapterStorage {
    fn claim_native_paths(&self, claim: &NativePathClaim) -> Result<(), PersistenceError> {
        self.store()?.claim_native_paths(claim)
    }

    fn claim_serving_native_paths(
        &self,
        record: &milkdrift_persistence::PeerExecutionRecord,
        claim: &NativePathClaim,
    ) -> Result<(), PersistenceError> {
        self.store()?.claim_serving_native_paths(record, claim)
    }

    fn native_path_claim(&self, owner: &str) -> Result<Option<NativePathClaim>, PersistenceError> {
        self.store()?.native_path_claim(owner)
    }

    fn release_native_paths(&self, owner: &str) -> Result<(), PersistenceError> {
        self.store()?.release_native_paths(owner)
    }
}

impl ArtifactStore for AdapterStorage {
    fn artifact_usage(&self, owner: &ArtifactOwner) -> Result<WorkspaceUsage, PersistenceError> {
        self.store()?.artifact_usage(owner)
    }

    fn begin_publication(
        &self,
        request: &BeginArtifactPublication,
    ) -> Result<BeginArtifactOutcome, PersistenceError> {
        self.store()?.begin_publication(request)
    }

    fn write_chunk(
        &self,
        publication: &ArtifactPublicationId,
        offset: u64,
        bytes: &[u8],
    ) -> Result<ArtifactWriteProgress, PersistenceError> {
        self.store()?.write_chunk(publication, offset, bytes)
    }

    fn commit_publication(
        &self,
        publication: &ArtifactPublicationId,
    ) -> Result<CommitArtifactOutcome, PersistenceError> {
        self.store()?.commit_publication(publication)
    }

    fn abort_publication(
        &self,
        publication: &ArtifactPublicationId,
    ) -> Result<(), PersistenceError> {
        self.store()?.abort_publication(publication)
    }

    fn metadata(
        &self,
        artifact: &ArtifactId,
    ) -> Result<Option<ArtifactMetadata>, PersistenceError> {
        self.store()?.metadata(artifact)
    }

    fn is_committed(&self, reference: &ArtifactReference) -> Result<bool, PersistenceError> {
        self.store()?.is_committed(reference)
    }

    fn is_referenced_by_run(
        &self,
        run: &RunId,
        reference: &ArtifactReference,
    ) -> Result<bool, PersistenceError> {
        self.store()?.is_referenced_by_run(run, reference)
    }

    fn read_chunk(
        &self,
        request: &ArtifactReadRequest,
    ) -> Result<ArtifactReadChunk, PersistenceError> {
        self.store()?.read_chunk(request)
    }

    fn cleanup_orphans(
        &self,
        request: OrphanCleanupRequest,
    ) -> Result<OrphanCleanupResult, PersistenceError> {
        self.store()?.cleanup_orphans(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_adapter_ports_release_the_store_and_refuse_after_owner_exit()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let store = Arc::new(RedbStore::open(root.path())?);
        let adapter = AdapterStorage::new(&store);
        let artifact = ArtifactId::new("adapter-lifetime-artifact")?;
        assert_eq!(adapter.native_path_claim("absent-owner")?, None);
        assert_eq!(adapter.metadata(&artifact)?, None);
        drop(store);
        for failure in [
            adapter.native_path_claim("absent-owner").map(|_| ()),
            adapter.metadata(&artifact).map(|_| ()),
        ] {
            assert!(matches!(
                failure,
                Err(PersistenceError::Storage {
                    class: StorageFailureClass::Unavailable,
                    ..
                })
            ));
        }
        let reopened = RedbStore::open(root.path())?;
        assert_eq!(reopened.metadata(&artifact)?, None);
        // A retired service cannot attach its stale port to the next daemon instance.
        assert!(matches!(
            adapter.metadata(&artifact),
            Err(PersistenceError::Storage {
                class: StorageFailureClass::Unavailable,
                ..
            })
        ));
        Ok(())
    }
}
