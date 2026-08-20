use super::*;

#[derive(Debug)]
struct FixedArtifactClock(TimestampMillis);

impl ArtifactClock for FixedArtifactClock {
    fn now(&self) -> Result<TimestampMillis, PersistenceError> {
        Ok(self.0)
    }
}

#[test]
fn artifact_cleanup_uses_the_injected_publication_clock() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = TempDir::new()?;
    let created_at = TimestampMillis::new(50);
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_artifact_clock(Arc::new(FixedArtifactClock(created_at))),
    )?;
    let bytes = b"clocked artifact publication";
    let request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-clock")?,
        RunId::new("run-clock")?,
        artifact_metadata("artifact-clock", bytes, ArtifactSensitivity::Public)?,
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&request)?;
    store.write_chunk(&request.publication, 0, &bytes[..1])?;

    let retained = store.cleanup_orphans(OrphanCleanupRequest {
        observed_at: TimestampMillis::new(100),
        created_before: created_at,
        limit: PageSize::new(10)?,
        cursor: None,
    })?;
    assert_eq!(retained.temporary_publications_removed, 0);
    assert_eq!(store.begin_publication(&request)?.next_offset(), Some(1));

    let removed = store.cleanup_orphans(OrphanCleanupRequest {
        observed_at: TimestampMillis::new(100),
        created_before: TimestampMillis::new(51),
        limit: PageSize::new(10)?,
        cursor: None,
    })?;
    assert_eq!(removed.temporary_publications_removed, 1);
    Ok(())
}
#[test]
fn artifact_begin_and_chunk_fault_boundaries_resume_exact_durable_offsets()
-> Result<(), Box<dyn std::error::Error>> {
    for (index, point) in [
        FaultPoint::BeforeArtifactBeginCommit,
        FaultPoint::AfterArtifactBeginCommit,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let bytes = format!("artifact-begin-{index}").into_bytes();
        let metadata = artifact_metadata(
            &format!("artifact-begin-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-begin-{index}"))?,
            RunId::new(format!("run-begin-{index}"))?,
            metadata,
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        assert!(store.begin_publication(&request).is_err());
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        if point == FaultPoint::BeforeArtifactBeginCommit {
            assert_eq!(
                reopened.begin_publication(&request)?,
                BeginArtifactOutcome::Writable
            );
        } else {
            assert_eq!(reopened.begin_publication(&request)?.next_offset(), Some(0));
        }
        reopened.abort_publication(&request.publication)?;
    }

    for (index, point) in [
        FaultPoint::BeforeArtifactChunkWrite,
        FaultPoint::AfterArtifactChunkSync,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let bytes = format!("artifact-chunk-{index}").into_bytes();
        let metadata = artifact_metadata(
            &format!("artifact-chunk-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-chunk-{index}"))?,
            RunId::new(format!("run-chunk-{index}"))?,
            metadata.clone(),
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        store.begin_publication(&request)?;
        assert!(store.write_chunk(&request.publication, 0, &bytes).is_err());
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        let durable_offset = if point == FaultPoint::AfterArtifactChunkSync {
            bytes.len() as u64
        } else {
            0
        };
        assert_eq!(
            reopened.begin_publication(&request)?.next_offset(),
            Some(durable_offset)
        );
        if durable_offset == 0 {
            reopened.write_chunk(&request.publication, 0, &bytes)?;
        }
        reopened.commit_publication(&request.publication)?;
        assert!(reopened.is_committed(metadata.reference())?);
    }
    Ok(())
}

#[test]
fn artifact_abort_fault_boundaries_are_retryable_and_release_ownership()
-> Result<(), Box<dyn std::error::Error>> {
    for (index, point) in [
        FaultPoint::BeforeArtifactAbortCommit,
        FaultPoint::AfterArtifactAbortCommit,
        FaultPoint::BeforeArtifactAbortDelete,
        FaultPoint::AfterArtifactAbortDelete,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let bytes = format!("artifact-abort-{index}").into_bytes();
        let metadata = artifact_metadata(
            &format!("artifact-abort-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-abort-{index}"))?,
            RunId::new(format!("run-abort-{index}"))?,
            metadata,
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        store.begin_publication(&request)?;
        store.write_chunk(&request.publication, 0, &bytes[..3])?;
        assert!(store.abort_publication(&request.publication).is_err());
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        if point == FaultPoint::BeforeArtifactAbortCommit {
            assert_eq!(reopened.begin_publication(&request)?.next_offset(), Some(3));
        }
        reopened.abort_publication(&request.publication)?;
        assert_eq!(
            reopened.begin_publication(&request)?,
            BeginArtifactOutcome::Writable
        );
        reopened.abort_publication(&request.publication)?;
    }
    Ok(())
}

#[test]
fn cleanup_fault_boundaries_expire_writable_sessions_and_release_reservations()
-> Result<(), Box<dyn std::error::Error>> {
    for (index, point) in [
        FaultPoint::BeforeArtifactCleanupCommit,
        FaultPoint::AfterArtifactCleanupCommit,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let bytes = format!("artifact-cleanup-{index}").into_bytes();
        let run = RunId::new(format!("run-cleanup-{index}"))?;
        let metadata = artifact_metadata(
            &format!("artifact-cleanup-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-cleanup-{index}"))?,
            run.clone(),
            metadata,
            WorkspaceBudget::new(0, 0, 0, 2, 2048, 2048)?,
            WorkspaceUsage::EMPTY,
        )?;
        let cleanup_request = OrphanCleanupRequest {
            observed_at: TimestampMillis::new(u64::MAX),
            created_before: TimestampMillis::new(u64::MAX - 1),
            limit: PageSize::new(100)?,
            cursor: None,
        };
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        store.begin_publication(&request)?;
        store.write_chunk(&request.publication, 0, &bytes[..3])?;
        assert!(store.cleanup_orphans(cleanup_request.clone()).is_err());
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        if point == FaultPoint::BeforeArtifactCleanupCommit {
            assert_eq!(reopened.begin_publication(&request)?.next_offset(), Some(3));
        }
        let cleanup = reopened.cleanup_orphans(cleanup_request)?;
        assert_eq!(cleanup.temporary_publications_removed, 1);

        let replacement_bytes = format!("replacement-cleanup-{index}").into_bytes();
        let replacement = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-replacement-{index}"))?,
            run,
            artifact_metadata(
                &format!("artifact-replacement-{index}"),
                &replacement_bytes,
                ArtifactSensitivity::Public,
            )?,
            WorkspaceBudget::new(0, 0, 0, 2, 2048, 2048)?,
            WorkspaceUsage::EMPTY,
        )?;
        assert_eq!(
            reopened.begin_publication(&replacement)?,
            BeginArtifactOutcome::Writable
        );
        reopened.abort_publication(&replacement.publication)?;
    }
    Ok(())
}

#[test]
fn cleanup_expires_a_session_crashed_after_content_rename() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = TempDir::new()?;
    let bytes = b"renamed-before-metadata";
    let metadata = artifact_metadata(
        "artifact-renamed-orphan",
        bytes,
        ArtifactSensitivity::Public,
    )?;
    let request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-renamed-orphan")?,
        RunId::new("run-renamed-orphan")?,
        metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_fault_injector(Arc::new(FailOnce::new(FaultPoint::AfterArtifactRename))),
    )?;
    store.begin_publication(&request)?;
    store.write_chunk(&request.publication, 0, bytes)?;
    assert!(store.commit_publication(&request.publication).is_err());
    assert!(!store.is_committed(metadata.reference())?);
    drop(store);

    let reopened = RedbStore::open(directory.path())?;
    let cleanup = reopened.cleanup_orphans(OrphanCleanupRequest {
        observed_at: TimestampMillis::new(u64::MAX),
        created_before: TimestampMillis::new(u64::MAX - 1),
        limit: PageSize::new(100)?,
        cursor: None,
    })?;
    assert_eq!(cleanup.temporary_publications_removed, 0);
    assert_eq!(cleanup.unreferenced_blobs_removed, 1);
    assert_eq!(
        reopened.begin_publication(&request)?,
        BeginArtifactOutcome::Writable
    );
    reopened.abort_publication(&request.publication)?;
    Ok(())
}

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
            store.write_chunk(&request.publication, 0, &bytes[..1])?;
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
fn orphan_cleanup_cursors_visit_every_family_without_starvation()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let artifact_budget = WorkspaceBudget::new(0, 0, 0, 1, 4096, 4096)?;

    let referenced_bytes = b"durably-referenced-content";
    let referenced = artifact_metadata(
        "artifact-cleanup-referenced",
        referenced_bytes,
        ArtifactSensitivity::Public,
    )?;
    let referenced_request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-cleanup-referenced")?,
        RunId::new("run-cleanup-referenced")?,
        referenced.clone(),
        artifact_budget.clone(),
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&referenced_request)?;
    store.write_chunk(&referenced_request.publication, 0, referenced_bytes)?;
    store.commit_publication(&referenced_request.publication)?;

    for index in 0..7 {
        let bytes = format!("abandoned-publication-{index}").into_bytes();
        let metadata = artifact_metadata(
            &format!("artifact-abandoned-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-abandoned-{index}"))?,
            RunId::new(format!("run-abandoned-{index}"))?,
            metadata,
            artifact_budget.clone(),
            WorkspaceUsage::EMPTY,
        )?;
        store.begin_publication(&request)?;
        store.write_chunk(&request.publication, 0, &bytes[..1])?;
    }
    drop(store);
    for index in 0..4 {
        let bytes = format!("unowned-content-{index}").into_bytes();
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-unowned-content-{index}"))?,
            RunId::new(format!("run-unowned-content-{index}"))?,
            artifact_metadata(
                &format!("artifact-unowned-content-{index}"),
                &bytes,
                ArtifactSensitivity::Public,
            )?,
            artifact_budget.clone(),
            WorkspaceUsage::EMPTY,
        )?;
        let crashing = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(FaultPoint::AfterArtifactRename))),
        )?;
        crashing.begin_publication(&request)?;
        crashing.write_chunk(&request.publication, 0, &bytes)?;
        assert!(crashing.commit_publication(&request.publication).is_err());
    }

    let store = RedbStore::open(directory.path())?;
    let observed_at = TimestampMillis::new(u64::MAX);
    let created_before = TimestampMillis::new(u64::MAX - 1);
    let mut page = store.cleanup_orphans(OrphanCleanupRequest {
        observed_at,
        created_before,
        limit: PageSize::new(2)?,
        cursor: None,
    })?;
    let first_cursor = page
        .next_cursor
        .clone()
        .ok_or("first cleanup page must have a continuation")?;
    assert!(matches!(
        store.cleanup_orphans(OrphanCleanupRequest {
            observed_at,
            created_before: TimestampMillis::new(u64::MAX - 2),
            limit: PageSize::new(2)?,
            cursor: Some(first_cursor),
        }),
        Err(PersistenceError::InvalidCursor(_))
    ));

    let mut pages = 1_u32;
    let mut temporary_removed = page.temporary_publications_removed;
    let mut content_removed = page.unreferenced_blobs_removed;
    while let Some(cursor) = page.next_cursor {
        page = store.cleanup_orphans(OrphanCleanupRequest {
            observed_at,
            created_before,
            limit: PageSize::new(2)?,
            cursor: Some(cursor),
        })?;
        pages += 1;
        assert!(pages < 20, "cleanup cursor failed to converge");
        temporary_removed = temporary_removed.saturating_add(page.temporary_publications_removed);
        content_removed = content_removed.saturating_add(page.unreferenced_blobs_removed);
    }

    assert!(pages >= 6);
    assert_eq!(temporary_removed, 7);
    assert_eq!(content_removed, 4);
    assert!(store.is_committed(referenced.reference())?);
    Ok(())
}

#[test]
fn artifact_publication_fault_boundaries_recover_without_dangling_metadata()
-> Result<(), Box<dyn std::error::Error>> {
    let points = [
        FaultPoint::BeforeArtifactRename,
        FaultPoint::AfterArtifactRename,
        FaultPoint::BeforeArtifactMetadataCommit,
        FaultPoint::AfterArtifactMetadataCommit,
    ];
    for (index, point) in points.into_iter().enumerate() {
        let directory = TempDir::new()?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        let bytes = format!("fault-boundary-content-{index}").into_bytes();
        let metadata = artifact_metadata(
            &format!("artifact-fault-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-fault-{index}"))?,
            RunId::new(format!("run-fault-{index}"))?,
            metadata.clone(),
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?;
        assert_eq!(
            store.begin_publication(&request)?,
            BeginArtifactOutcome::Writable
        );
        assert!(
            store
                .write_chunk(&request.publication, 0, &bytes)?
                .complete_size
        );
        assert!(store.commit_publication(&request.publication).is_err());

        let committed_after_failure = store.is_committed(metadata.reference())?;
        if point == FaultPoint::AfterArtifactMetadataCommit {
            assert!(committed_after_failure);
        } else {
            assert!(!committed_after_failure);
        }
        let cleanup = store.cleanup_orphans(OrphanCleanupRequest {
            observed_at: TimestampMillis::new(u64::MAX),
            created_before: TimestampMillis::new(0),
            limit: PageSize::new(100)?,
            cursor: None,
        })?;
        assert_eq!(cleanup.temporary_publications_removed, 0);
        assert_eq!(cleanup.unreferenced_blobs_removed, 0);
        let recovered = store.commit_publication(&request.publication)?;
        assert!(store.is_committed(metadata.reference())?);
        if point == FaultPoint::AfterArtifactMetadataCommit {
            assert!(!recovered.was_published());
        } else {
            assert!(recovered.was_published());
        }
    }
    Ok(())
}

#[test]
fn artifact_rejects_bad_digest_offsets_chunks_and_budget() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let expected = b"expected";
    let actual = b"mismatch";
    let metadata = artifact_metadata("artifact-invalid", expected, ArtifactSensitivity::Public)?;
    let request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-invalid")?,
        RunId::new("run-invalid")?,
        metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&request)?;
    assert!(matches!(
        store.write_chunk(&request.publication, 1, &actual[..1]),
        Err(PersistenceError::ImmutableConflict { .. })
    ));
    assert!(matches!(
        store.write_chunk(&request.publication, 0, &[]),
        Err(PersistenceError::Bounds { .. })
    ));
    let oversized_chunk = vec![0_u8; milkdrift_persistence::MAX_ARTIFACT_CHUNK_BYTES + 1];
    assert!(matches!(
        store.write_chunk(&request.publication, 0, &oversized_chunk),
        Err(PersistenceError::Bounds { .. })
    ));
    store.write_chunk(&request.publication, 0, actual)?;
    assert!(matches!(
        store.commit_publication(&request.publication),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));

    let too_small_budget = WorkspaceBudget::new(0, 0, 0, 1, 1, 1)?;
    assert!(
        BeginArtifactPublication::new(
            ArtifactPublicationId::new("publication-budget")?,
            RunId::new("run-budget")?,
            metadata,
            too_small_budget,
            WorkspaceUsage::EMPTY,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn artifact_publication_resumes_deduplicates_verifies_and_cleans_orphans()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let content = b"durable artifact bytes";
    let metadata = artifact_metadata("artifact-one", content, ArtifactSensitivity::Restricted)?;
    let budget = WorkspaceBudget::new(0, 0, 0, 10, 1024, 4096)?;
    let request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-one")?,
        RunId::new("run-artifact")?,
        metadata.clone(),
        budget.clone(),
        WorkspaceUsage::EMPTY,
    )?;
    {
        let store = RedbStore::open(directory.path())?;
        assert_eq!(
            store.begin_publication(&request)?,
            BeginArtifactOutcome::Writable
        );
        assert!(matches!(
            store.write_chunk(&request.publication, 0, &[0_u8; 64]),
            Err(PersistenceError::Bounds { .. })
        ));
        let first = &content[..7];
        let progress = store.write_chunk(&request.publication, 0, first)?;
        assert_eq!(progress.bytes_received, 7);
        assert!(!progress.complete_size);
    }
    let store = RedbStore::open(directory.path())?;
    assert_eq!(store.begin_publication(&request)?.next_offset(), Some(7));
    store.write_chunk(&request.publication, 7, &content[7..])?;
    let first_commit = store.commit_publication(&request.publication)?;
    assert!(first_commit.was_published());
    assert_eq!(first_commit.content_deduplicated(), Some(false));
    assert!(store.is_committed(metadata.reference())?);
    assert!(store.is_referenced_by_run(&request.run, metadata.reference())?);
    assert_eq!(
        store.workspace_usage(&request.run)?,
        request.resulting_usage
    );
    assert!(
        !store.is_referenced_by_run(&RunId::new("run-without-artifact")?, metadata.reference())?
    );
    assert!(matches!(
        store.begin_publication(&BeginArtifactPublication::new(
            ArtifactPublicationId::new("publication-reused-artifact")?,
            RunId::new("run-reused-artifact")?,
            metadata.clone(),
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?),
        Err(PersistenceError::ImmutableConflict {
            entity: "artifact_publication",
            ..
        })
    ));
    let public_read = ArtifactReadRequest::new(
        metadata.reference().clone(),
        0,
        16,
        ArtifactReadAuthority::PublicOnly,
    )?;
    assert!(store.read_chunk(&public_read).is_err());

    let second_metadata = artifact_metadata("artifact-two", content, ArtifactSensitivity::Public)?;
    let second = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-two")?,
        request.run.clone(),
        second_metadata.clone(),
        budget,
        request.resulting_usage,
    )?;
    assert_eq!(
        store.begin_publication(&second)?,
        BeginArtifactOutcome::Writable
    );
    assert!(
        store
            .write_chunk(&second.publication, 0, content)?
            .complete_size
    );
    let second_commit = store.commit_publication(&second.publication)?;
    assert!(second_commit.was_published());
    assert_eq!(second_commit.content_deduplicated(), Some(true));
    assert!(store.is_referenced_by_run(&request.run, second_metadata.reference())?);
    assert_eq!(store.workspace_usage(&request.run)?, second.resulting_usage);
    let read = ArtifactReadRequest::new(
        second_metadata.reference().clone(),
        0,
        1_024,
        ArtifactReadAuthority::PublicOnly,
    )?;
    let chunk = store.read_chunk(&read)?;
    assert_eq!(chunk.bytes, content);
    assert!(chunk.end_of_artifact);

    let temp_bytes = b"abandoned-temporary-publication";
    let temp_request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-cleanup-temporary")?,
        RunId::new("run-cleanup-temporary")?,
        artifact_metadata(
            "artifact-cleanup-temporary",
            temp_bytes,
            ArtifactSensitivity::Public,
        )?,
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&temp_request)?;
    store.write_chunk(&temp_request.publication, 0, &temp_bytes[..1])?;
    drop(store);

    let orphan_bytes = b"unreferenced";
    let orphan_metadata = artifact_metadata(
        "artifact-cleanup-content",
        orphan_bytes,
        ArtifactSensitivity::Public,
    )?;
    let orphan_request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-cleanup-content")?,
        RunId::new("run-cleanup-content")?,
        orphan_metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    let crashing = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_fault_injector(Arc::new(FailOnce::new(FaultPoint::AfterArtifactRename))),
    )?;
    crashing.begin_publication(&orphan_request)?;
    crashing.write_chunk(&orphan_request.publication, 0, orphan_bytes)?;
    assert!(
        crashing
            .commit_publication(&orphan_request.publication)
            .is_err()
    );
    drop(crashing);

    let orphan_digest = orphan_metadata.reference().digest().to_hex();
    let orphan_path = directory
        .path()
        .join("artifacts")
        .join(&orphan_digest[..2])
        .join(&orphan_digest[2..]);
    let store = RedbStore::open(directory.path())?;
    let cleanup = store.cleanup_orphans(OrphanCleanupRequest {
        observed_at: TimestampMillis::new(u64::MAX),
        created_before: TimestampMillis::new(u64::MAX - 1),
        limit: PageSize::new(100)?,
        cursor: None,
    })?;
    assert_eq!(cleanup.temporary_publications_removed, 1);
    assert_eq!(cleanup.unreferenced_blobs_removed, 1);
    assert!(!orphan_path.exists());
    assert!(store.is_committed(metadata.reference())?);

    let committed_digest = metadata.reference().digest().to_hex();
    let committed_path = directory
        .path()
        .join("artifacts")
        .join(&committed_digest[..2])
        .join(&committed_digest[2..]);
    std::fs::write(&committed_path, b"corrupted artifact bytes")?;
    assert!(matches!(
        store.is_committed(metadata.reference()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn artifact_publication_and_reads_refuse_symlink_redirection()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let shard_directory = TempDir::new()?;
    let escaped_directory = TempDir::new()?;
    let bytes = b"must remain inside the artifact root";
    let metadata = artifact_metadata("artifact-shard-link", bytes, ArtifactSensitivity::Public)?;
    let request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-shard-link")?,
        RunId::new("run-shard-link")?,
        metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    let store = RedbStore::open(shard_directory.path())?;
    store.begin_publication(&request)?;
    store.write_chunk(&request.publication, 0, bytes)?;
    let digest = metadata.reference().digest().to_hex();
    let shard = shard_directory.path().join("artifacts").join(&digest[..2]);
    symlink(escaped_directory.path(), &shard)?;
    assert!(matches!(
        store.commit_publication(&request.publication),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(!escaped_directory.path().join(&digest[2..]).exists());
    assert!(store.metadata(metadata.reference().artifact())?.is_none());

    let content_directory = TempDir::new()?;
    let external_file = tempfile::NamedTempFile::new()?;
    let content_metadata = artifact_metadata(
        "artifact-content-link",
        b"verified content",
        ArtifactSensitivity::Public,
    )?;
    let content_request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-content-link")?,
        RunId::new("run-content-link")?,
        content_metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    let content_store = RedbStore::open(content_directory.path())?;
    content_store.begin_publication(&content_request)?;
    content_store.write_chunk(&content_request.publication, 0, b"verified content")?;
    content_store.commit_publication(&content_request.publication)?;
    let digest = content_metadata.reference().digest().to_hex();
    let path = content_directory
        .path()
        .join("artifacts")
        .join(&digest[..2])
        .join(&digest[2..]);
    std::fs::remove_file(&path)?;
    symlink(external_file.path(), &path)?;
    assert!(matches!(
        content_store.is_committed(content_metadata.reference()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    Ok(())
}
