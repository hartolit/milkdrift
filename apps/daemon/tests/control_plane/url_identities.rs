//! Transport preserves accepted identities as one segment through real daemon routes.
use super::{authoring::model_configuration_document, inputs, support::*};

fn artifacts(root: &std::path::Path) -> TestResult<Vec<String>> {
    let ids = vec![
        "artifact/team/build-1".into(),
        "artifact/a/../../content/./".into(),
        format!("a{}", "/".repeat(191)),
    ];
    let store = RedbStore::open(root)?;
    for (index, id) in ids.iter().enumerate() {
        let bytes = b"URL identity fixture";
        let metadata = ArtifactMetadata::new(
            ArtifactReference::new(
                ArtifactId::new(id)?,
                ContentDigest::for_bytes(bytes),
                MediaType::new("text/plain")?,
                bytes.len() as u64,
            ),
            ArtifactSensitivity::Restricted,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::External {
                    source: CausalId::new("url-fixture")?,
                },
                vec![],
            )?,
        )?;
        let publication = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("url-publication-{index}"))?,
            RunId::new(format!("url-fixture-{index}"))?,
            metadata,
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?;
        store.begin_publication(&publication)?;
        store.write_chunk(publication.publication(), 0, bytes)?;
        store.commit_publication(publication.publication())?;
    }
    Ok(ids)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_names_survive_reads_waits_artifacts_and_saved_requests() -> TestResult {
    let directory = TempDir::new()?;
    let model = inputs::ModelFixture::start().await?;
    let config =
        model_configuration_document(&directory, model.address)?.validate(directory.path())?;
    let artifacts = artifacts(&directory.path().join("data"))?;
    let daemon = start(config, CONTROLLER_TOKEN).await?;
    model
        .checked(async |model| {
            daemon
                .checked(model, async |daemon| {
                    for (index, id) in artifacts.iter().enumerate() {
                        let metadata = daemon.client.artifact_metadata(id).await?;
                        assert_eq!(&metadata.artifact_id, id);
                        let content = daemon.client.artifact_range(id, 4, 11).await?;
                        assert_eq!(content.bytes, b"identity");
                        cli_ok(
                            daemon,
                            &directory,
                            &format!("download-{index}"),
                            &[
                                "artifact",
                                "get",
                                id,
                                "--output",
                                &format!("artifact-{index}.txt"),
                            ],
                        )?;
                        assert_eq!(
                            fs::read(directory.path().join(format!("artifact-{index}.txt")))?,
                            b"URL identity fixture"
                        );
                    }
                    let revision = inputs::workflow(&daemon.client).await?;
                    let input =
                        inputs::upload(&daemon.client, "url-brief", b"URL-safe workflow brief")
                            .await?;
                    let names = [
                        "team/build-1".into(),
                        "team/../build/./".into(),
                        "team//build".into(),
                        "team:._-9".into(),
                        format!("r{}", "/".repeat(127)),
                    ];
                    let mut prior_attempt = None;
                    for (index, run) in names.iter().enumerate() {
                        let saved = daemon
                            .client
                            .prepare_run(inputs::start_request(run, &revision, vec![input.clone()]))
                            .await?;
                        daemon.client.submit_saved_run(&saved).await?;
                        let completed =
                            wait_for_run(&daemon.client, run, Duration::from_secs(45), |read| {
                                read.terminal.is_some()
                            })
                            .await?;
                        assert_eq!(completed.run_id, *run);
                        assert_eq!(completed.terminal.as_deref(), Some("succeeded"));
                        assert!(daemon.client.submit_saved_run(&saved).await?.replayed);
                        assert_eq!(daemon.client.run_result(run).await?.run.run_id, *run);
                        for node in &completed.nodes {
                            assert_eq!(daemon.client.node(run, &node.execution_id).await?, *node);
                            if let Some(id) = &node.latest_attempt_id {
                                let attempt = daemon.client.attempt(run, id).await?;
                                assert_eq!(attempt.attempt_id, *id);
                                if index == 0 {
                                    prior_attempt = Some(id.clone());
                                }
                            }
                        }
                        if index > 0 {
                            assert!(
                                daemon
                                    .client
                                    .attempt(
                                        run,
                                        prior_attempt.as_deref().ok_or("prior attempt absent")?
                                    )
                                    .await
                                    .is_err()
                            );
                        }
                        cli_ok(
                            daemon,
                            &directory,
                            &format!("inspect-{index}"),
                            &["run", "show", run],
                        )?;
                        cli_ok(
                            daemon,
                            &directory,
                            &format!("wait-{index}"),
                            &["run", "wait", run],
                        )?;
                    }
                    // Percent-looking text is not admitted by these domain validators.
                    // The transport must nevertheless send that literal value, never alias
                    // an existing slash name by decoding it twice.
                    for invalid in ["team%2Fbuild-1", "team%252Fbuild-1", "team/build-1/result"] {
                        assert!(matches!(
                            daemon.client.run(invalid).await,
                            Err(ClientError::Api(_))
                        ));
                    }
                    assert!(matches!(
                        daemon
                            .client
                            .artifact_metadata("artifact%2Fteam%2Fbuild-1")
                            .await,
                        Err(ClientError::Api(_))
                    ));
                    assert!(RunId::new(".").is_err());
                    assert!(RunId::new("..").is_err());
                    assert!(RunId::new("team%2Fbuild-1").is_err());
                    Ok(())
                })
                .await
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slash_feed_reconnect_uses_raw_resource_cursor_binding() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = start(configuration(&directory, 16)?, CONTROLLER_TOKEN).await?;
    let revision = import_blueprint(&daemon.client, "url-stream-import").await?;
    let run = "team/stream/../build-1";
    daemon
        .client
        .submit(&request(
            "url-stream-start",
            None,
            Command::StartRun {
                run_id: run.into(),
                workflow_id: "golden".into(),
                revision_id: revision,
                inputs: vec![],
            },
        ))
        .await?;
    let path = format!("v1/runs/{run}/stream");
    let feed = format!("run:{run}");
    let mut stream = daemon.client.subscribe(&path, None);
    let first = tokio::time::timeout(Duration::from_secs(30), stream.next())
        .await?
        .ok_or("stream closed")??;
    assert_eq!(first.feed, feed);
    let position = first.cursor.position_for(&feed)?;
    assert!(
        first
            .cursor
            .position_for("run:team%2Fstream%2F..%2Fbuild-1")
            .is_err()
    );
    drop(stream);
    let sequence = daemon.client.run(run).await?.sequence;
    daemon
        .client
        .submit(&request(
            "url-stream-pause",
            Some(sequence),
            Command::PauseRun { run_id: run.into() },
        ))
        .await?;
    let mut stream = daemon.client.subscribe(&path, Some(first.cursor.clone()));
    let next = tokio::time::timeout(Duration::from_secs(30), stream.next())
        .await?
        .ok_or("reconnect closed")??;
    assert_eq!(next.feed, feed);
    assert!(next.cursor.position_for(&feed)? > position);
    drop(stream);
    let mut wrong = daemon
        .client
        .subscribe("v1/runs/another/run/stream", Some(first.cursor));
    assert!(
        tokio::time::timeout(Duration::from_secs(30), wrong.next())
            .await?
            .ok_or("wrong feed closed")?
            .is_err()
    );
    drop(wrong);
    daemon.stop().await
}
