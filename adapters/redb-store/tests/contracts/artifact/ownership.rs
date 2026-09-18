use super::*;
use milkdrift_capability::{InvocationId, PeerId};
use milkdrift_workspace::ArtifactOwner;

#[test]
fn client_input_recovery_releases_only_interrupted_uploads_with_bounded_restart_safe_pages()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let bytes = b"unpublished input";
    let host = PeerId::new("upload-host")?;
    let client = CausalId::new("human:upload-owner")?;
    let mut uploads = Vec::new();
    for id in ["first", "second"] {
        let metadata = artifact_metadata(
            &format!("upload-{id}"),
            bytes,
            ArtifactSensitivity::Restricted,
        )?;
        let actor = CausalId::new(format!("{client}:{id}"))?;
        let metadata = ArtifactMetadata::new(
            metadata.reference().clone(),
            metadata.sensitivity(),
            metadata.retention().clone(),
            ArtifactProvenance::new(
                CausalReference::ClientUpload {
                    host: host.clone(),
                    client: actor.clone(),
                    upload: CausalId::new(id)?,
                },
                Vec::new(),
            )?,
        )?;
        uploads.push(BeginArtifactPublication::for_client_input(
            ArtifactPublicationId::new(format!("upload-{id}"))?,
            host.clone(),
            actor,
            metadata,
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?);
    }
    let workflow = BeginArtifactPublication::new(
        ArtifactPublicationId::new("resumable-workflow-output")?,
        RunId::new("real-workflow")?,
        artifact_metadata("workflow-partial", bytes, ArtifactSensitivity::Restricted)?,
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    {
        let store = RedbStore::open(directory.path())?;
        for publication in uploads.iter().chain([&workflow]) {
            store.begin_publication(publication)?;
            store.write_chunk(publication.publication(), 0, &bytes[..3])?;
        }
    }
    let mut cursor = None;
    let mut removed = 0;
    let mut completed = false;
    for _ in 0..64 {
        // Tear down between pages, as after interruption of startup recovery.
        let store = RedbStore::open(directory.path())?;
        let page = store.recover_interrupted_client_inputs(OrphanCleanupRequest {
            observed_at: TimestampMillis::new(u64::MAX),
            created_before: TimestampMillis::new(u64::MAX),
            limit: PageSize::new(1)?,
            cursor,
        })?;
        removed += page.temporary_publications_removed;
        cursor = page.next_cursor;
        if cursor.is_none() {
            completed = true;
            break;
        }
    }
    assert!(completed);
    assert_eq!(removed, 2);
    let store = RedbStore::open(directory.path())?;
    for upload in &uploads {
        assert!(
            store
                .metadata(upload.metadata().reference().artifact())?
                .is_none()
        );
        assert_eq!(store.artifact_usage(upload.owner())?, WorkspaceUsage::EMPTY);
        assert!(matches!(
            store.begin_publication(upload)?,
            BeginArtifactOutcome::Writable
        ));
        store.write_chunk(upload.publication(), 0, bytes)?;
        store.commit_publication(upload.publication())?;
    }
    assert!(matches!(
        store.begin_publication(&workflow)?,
        BeginArtifactOutcome::Resumed { next_offset: 3 }
    ));
    let mut cursor = None;
    for _ in 0..1000 {
        let page = store.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(4)?,
            verify_artifact_content: true,
            cursor,
        })?;
        assert!(page.failures.is_empty(), "{:?}", page.failures);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok(());
        }
    }
    Err("input recovery integrity scan did not exhaust".into())
}

#[test]
fn serving_output_and_transfer_have_distinct_durable_owners_without_runs()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let host = PeerId::new("execution-host")?;
    let invocation = InvocationId::new("accepted-invocation")?;
    let owner = ArtifactOwner::HostInvocation {
        host: host.clone(),
        invocation: invocation.clone(),
    };
    let bytes = b"useful output";
    let source = artifact_metadata("host-output", bytes, ArtifactSensitivity::Restricted)?;
    let metadata = ArtifactMetadata::new(
        source.reference().clone(),
        source.sensitivity(),
        source.retention().clone(),
        ArtifactProvenance::new(
            CausalReference::HostInvocation {
                host: host.clone(),
                invocation: invocation.clone(),
            },
            Vec::new(),
        )?,
    )?;
    let budget = WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?;
    assert!(
        BeginArtifactPublication::for_host_invocation(
            ArtifactPublicationId::new("forged-producer")?,
            host.clone(),
            invocation.clone(),
            source,
            budget.clone(),
            WorkspaceUsage::EMPTY,
        )
        .is_err()
    );
    let publication = BeginArtifactPublication::for_host_invocation(
        ArtifactPublicationId::new("host-output-publication")?,
        host.clone(),
        invocation,
        metadata.clone(),
        budget.clone(),
        WorkspaceUsage::EMPTY,
    )?;
    assert_eq!(publication.owner(), &owner);
    assert!(publication.controller_owner().is_none());
    let transfer = BeginArtifactPublication::for_transfer(
        ArtifactPublicationId::new("import-publication")?,
        host,
        CausalId::new("import-transfer")?,
        artifact_metadata("import-output", bytes, ArtifactSensitivity::Restricted)?,
        budget,
        WorkspaceUsage::EMPTY,
        None,
    )?;
    let store = RedbStore::open(directory.path())?;
    for request in [&publication, &transfer] {
        assert_eq!(
            store.artifact_usage(request.owner())?,
            WorkspaceUsage::EMPTY
        );
        store.begin_publication(request)?;
        store.write_chunk(request.publication(), 0, bytes)?;
        store.commit_publication(request.publication())?;
        assert_eq!(
            store.artifact_usage(request.owner())?,
            request.resulting_usage()
        );
    }
    drop(store);
    let reopened = RedbStore::open(directory.path())?;
    for request in [&publication, &transfer] {
        assert!(matches!(
            reopened.begin_publication(request)?,
            BeginArtifactOutcome::AlreadyCommitted(_)
        ));
        assert_eq!(
            reopened.artifact_usage(request.owner())?,
            request.resulting_usage()
        );
        assert!(reopened.is_committed(request.metadata().reference())?);
    }
    let runs = reopened.run_summaries(&RunSummaryPageQuery {
        filter: RunSummaryFilter::default(),
        cursor: None,
        limit: PageSize::new(16)?,
    })?;
    assert!(runs.runs.is_empty() && runs.next.is_none());
    let mut cursor = None;
    for _ in 0..1_000 {
        let page = reopened.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(4)?,
            verify_artifact_content: true,
            cursor,
        })?;
        assert!(page.failures.is_empty(), "{:?}", page.failures);
        let Some(next) = page.next_cursor else {
            return Ok(());
        };
        cursor = Some(next);
    }
    Err("artifact owner integrity scan did not exhaust".into())
}
