use super::*;

#[test]
fn cleanup_file_delete_fault_boundaries_are_restart_safe() -> Result<(), Box<dyn std::error::Error>>
{
    for (index, point) in [
        FaultPoint::BeforeArtifactCleanupDelete,
        FaultPoint::AfterArtifactCleanupDelete,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let bytes = format!("cleanup-delete-boundary-{index}").into_bytes();
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-cleanup-delete-{index}"))?,
            RunId::new(format!("run-cleanup-delete-{index}"))?,
            artifact_metadata(
                &format!("artifact-cleanup-delete-{index}"),
                &bytes,
                ArtifactSensitivity::Public,
            )?,
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?;
        {
            let store = RedbStore::open(directory.path())?;
            store.begin_publication(&request)?;
            store.write_chunk(request.publication(), 0, &bytes[..1])?;
        }
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        let orphan = std::fs::read_dir(directory.path().join("artifacts/.tmp"))?
            .next()
            .ok_or("publication did not create a temporary artifact")??
            .path();
        let request = OrphanCleanupRequest {
            observed_at: TimestampMillis::new(u64::MAX),
            created_before: TimestampMillis::new(u64::MAX - 1),
            limit: PageSize::new(100)?,
            cursor: None,
        };
        assert!(store.cleanup_orphans(request.clone()).is_err());
        if point == FaultPoint::BeforeArtifactCleanupDelete {
            assert!(orphan.exists());
            assert_eq!(
                store
                    .cleanup_orphans(request)?
                    .temporary_publications_removed,
                1
            );
        } else {
            assert!(!orphan.exists());
            assert_eq!(
                store
                    .cleanup_orphans(request)?
                    .temporary_publications_removed,
                0
            );
        }
    }
    Ok(())
}

#[test]
fn artifact_path_intent_and_finalize_faults_resume_after_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    for (index, point) in [
        FaultPoint::BeforeArtifactPathDeleteIntentCommit,
        FaultPoint::AfterArtifactPathDeleteIntentCommit,
        FaultPoint::BeforeArtifactPathFinalizeCommit,
        FaultPoint::AfterArtifactPathFinalizeCommit,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let bytes = format!("path-fault-boundary-{index}").into_bytes();
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-path-fault-{index}"))?,
            RunId::new(format!("run-path-fault-{index}"))?,
            artifact_metadata(
                &format!("artifact-path-fault-{index}"),
                &bytes,
                ArtifactSensitivity::Public,
            )?,
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?;
        {
            let store = RedbStore::open(directory.path())?;
            store.begin_publication(&request)?;
            store.write_chunk(request.publication(), 0, &bytes[..1])?;
        }
        let cleanup_request = OrphanCleanupRequest {
            observed_at: TimestampMillis::new(u64::MAX),
            created_before: TimestampMillis::new(u64::MAX - 1),
            limit: PageSize::new(100)?,
            cursor: None,
        };
        let crashing = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        assert!(crashing.cleanup_orphans(cleanup_request.clone()).is_err());
        drop(crashing);

        let reopened = RedbStore::open(directory.path())?;
        let mut cursor = None;
        for _ in 0..10_000 {
            let page = reopened.scan_integrity(IntegrityScanRequest {
                limit: PageSize::new(1)?,
                verify_artifact_content: false,
                cursor,
            })?;
            assert!(
                page.failures.is_empty(),
                "legal artifact crash state failed scrub at {point:?}: {:?}",
                page.failures
            );
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor = Some(next);
        }
        for _ in 0..4 {
            let _ = reopened.cleanup_orphans(cleanup_request.clone())?;
        }
        assert!(
            std::fs::read_dir(directory.path().join("artifacts/.tmp"))?
                .next()
                .is_none()
        );
        assert_eq!(
            reopened.workspace_usage(request.run())?,
            WorkspaceUsage::EMPTY
        );
        assert!(
            reopened
                .metadata(request.metadata().reference().artifact())?
                .is_none()
        );
    }
    Ok(())
}
