use super::*;
use milkdrift_persistence::PersistenceError;
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct PublicationFaults {
    point: FaultPoint,
    publication: AtomicBool,
    abort: AtomicBool,
    aborts: AtomicUsize,
}
impl FaultInjector for PublicationFaults {
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
        let fail = if point == self.point {
            self.publication.swap(false, Ordering::SeqCst)
        } else if point == FaultPoint::BeforeArtifactAbortCommit {
            self.aborts.fetch_add(1, Ordering::SeqCst);
            self.abort.swap(false, Ordering::SeqCst)
        } else {
            false
        };
        if fail {
            Err(injected_failure(point))
        } else {
            Ok(())
        }
    }
}

#[test]
fn failed_input_upload_reports_unconfirmed_abort_and_preserves_exact_retry()
-> Result<(), Box<dyn std::error::Error>> {
    for point in [
        FaultPoint::BeforeArtifactChunkWrite,
        FaultPoint::BeforeArtifactMetadataCommit,
    ] {
        let root = tempfile::tempdir()?;
        let faults = Arc::new(PublicationFaults {
            point,
            publication: AtomicBool::new(true),
            abort: AtomicBool::new(true),
            aborts: AtomicUsize::new(0),
        });
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(root.path()).with_fault_injector(faults.clone()),
        )?;
        let host = milkdrift_capability::PeerId::new("upload-host")?;
        let client = CausalId::new("upload-client")?;
        let bytes = b"supplied input";
        let publication = ArtifactPublicationId::new("input:abort-fault")?;
        let metadata = ArtifactMetadata::new(
            ArtifactReference::new(
                ArtifactId::new("input:abort-fault")?,
                ContentDigest::for_bytes(bytes),
                MediaType::new("text/plain")?,
                bytes.len() as u64,
            ),
            ArtifactSensitivity::Restricted,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::ClientUpload {
                    host: host.clone(),
                    client: client.clone(),
                    upload: CausalId::new("upload-fault")?,
                },
                Vec::new(),
            )?,
        )?;
        let begin = BeginArtifactPublication::for_client_input(
            publication.clone(),
            host,
            client,
            metadata.clone(),
            milkdrift_workspace::WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            milkdrift_workspace::WorkspaceUsage::default(),
        )?;
        store.begin_publication(&begin)?;
        let original = store
            .write_chunk(&publication, 0, bytes)
            .and_then(|_| store.commit_publication(&publication))
            .map(|outcome| public_artifact_metadata(outcome.metadata()))
            .map_err(public_persistence);
        let original_code = original.as_ref().err().ok_or("fault did not fire")?.code;
        let error = {
            let mut cleanup = UploadCleanup {
                store: &store,
                publication: &publication,
                finished: false,
            };
            cleanup
                .finish(original)
                .err()
                .ok_or("failed upload accepted")?
        };
        assert_eq!(error.code, original_code);
        assert_eq!(
            error.details.get("publication_cleanup").map(String::as_str),
            Some("unconfirmed")
        );
        assert!(error.details.contains_key("cleanup_error_code"));
        assert_eq!(
            faults.aborts.load(Ordering::SeqCst),
            1,
            "ordinary cleanup must not retry invisibly in Drop"
        );
        assert!(!store.is_committed(metadata.reference())?);
        let state = store.begin_publication(&begin)?;
        let offset = state.next_offset().ok_or("resume offset absent")?;
        let remaining = bytes
            .get(usize::try_from(offset)?..)
            .ok_or("resume offset invalid")?;
        if !remaining.is_empty() {
            store.write_chunk(&publication, offset, remaining)?;
        }
        assert_eq!(
            store.commit_publication(&publication)?.metadata(),
            &metadata
        );
    }
    Ok(())
}
