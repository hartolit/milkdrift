use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use milkdrift_persistence::{ArtifactPublicationId, ArtifactStore, PersistenceError};
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};

use super::*;

struct FailPublicationAndAbort {
    publication_point: FaultPoint,
    publication: AtomicBool,
    abort: AtomicBool,
}

impl FaultInjector for FailPublicationAndAbort {
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
        let fail = if point == self.publication_point {
            self.publication.swap(false, Ordering::SeqCst)
        } else if point == FaultPoint::BeforeArtifactAbortCommit {
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
fn context_publication_reports_failed_abort_and_recovers_on_exact_retry()
-> Result<(), Box<dyn std::error::Error>> {
    for publication_point in [
        FaultPoint::BeforeArtifactChunkWrite,
        FaultPoint::BeforeArtifactMetadataCommit,
    ] {
        let directory = tempfile::tempdir()?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path()).with_fault_injector(Arc::new(
                FailPublicationAndAbort {
                    publication_point,
                    publication: AtomicBool::new(true),
                    abort: AtomicBool::new(true),
                },
            )),
        )?;
        let bytes = b"context publication retry";
        let run = RunId::new("run-context-publication")?;
        let publication = ArtifactPublicationId::new("context-publication")?;
        let metadata = ArtifactMetadata::new(
            WorkspaceArtifactReference::new(
                ArtifactId::new("context-artifact")?,
                ContentDigest::for_bytes(bytes),
                MediaType::new("application/json")?,
                u64::try_from(bytes.len())?,
            ),
            ArtifactSensitivity::Restricted,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::External {
                    source: CausalId::new("context-fixture")?,
                },
                Vec::new(),
            )?,
        )?;
        let budget = WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?;
        let error = publish_context_artifact(
            &store,
            &run,
            metadata.clone(),
            publication.clone(),
            bytes,
            budget.clone(),
            WorkspaceUsage::EMPTY,
        )
        .err()
        .ok_or("publication unexpectedly succeeded")?;
        assert!(
            error
                .to_string()
                .contains("publication cleanup also failed and the session may remain")
        );
        assert!(!store.is_committed(metadata.reference())?);
        let reference = publish_context_artifact(
            &store,
            &run,
            metadata.clone(),
            publication,
            bytes,
            budget,
            WorkspaceUsage::EMPTY,
        )?;
        assert_eq!(
            reference.identity(),
            metadata.reference().artifact().as_str()
        );
        assert!(store.is_committed(metadata.reference())?);
    }
    Ok(())
}
