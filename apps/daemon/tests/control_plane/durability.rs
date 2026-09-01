//! Command replay, layout, and proposal-index restart durability.

use super::support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_command_idempotency_restart_and_stale_conflict() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let revision = import_blueprint(&daemon.client, "import-retry").await?;
    let replay = daemon
        .client
        .submit(&request(
            "import-retry",
            None,
            Command::ImportBlueprint {
                document: blueprint()?,
            },
        ))
        .await?;
    assert!(replay.replayed);
    assert!(matches!(
        daemon
            .client
            .submit(&request(
                "import-retry",
                None,
                Command::ValidateBlueprint {
                    document: blueprint()?,
                },
            ))
            .await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict
    ));

    let started = daemon
        .client
        .submit(&request(
            "start-run",
            None,
            Command::StartRun {
                run_id: "run-integration".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: revision,
            },
        ))
        .await?;
    assert!(started.resulting_sequence.is_some());
    let stale = daemon
        .client
        .submit(&request(
            "pause-stale",
            Some(0),
            Command::PauseRun {
                run_id: "run-integration".to_owned(),
            },
        ))
        .await;
    let stale_error = match stale {
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict => error,
        other => return Err(format!("expected durable stale conflict, got {other:?}").into()),
    };
    let repeated_stale = daemon
        .client
        .submit(&request(
            "pause-stale",
            Some(0),
            Command::PauseRun {
                run_id: "run-integration".to_owned(),
            },
        ))
        .await;
    assert!(matches!(
        repeated_stale,
        Err(ClientError::Api(ref error))
            if error.code == stale_error.code
                && error.message == stale_error.message
                && error.details == stale_error.details
    ));
    daemon.stop().await?;

    let restarted = start(config, CONTROLLER_TOKEN).await?;
    let restart_replay = restarted
        .client
        .submit(&request(
            "import-retry",
            None,
            Command::ImportBlueprint {
                document: blueprint()?,
            },
        ))
        .await?;
    assert!(restart_replay.replayed);
    let restarted_stale = restarted
        .client
        .submit(&request(
            "pause-stale",
            Some(0),
            Command::PauseRun {
                run_id: "run-integration".to_owned(),
            },
        ))
        .await;
    assert!(matches!(
        restarted_stale,
        Err(ClientError::Api(ref error))
            if error.code == stale_error.code
                && error.message == stale_error.message
                && error.details == stale_error.details
    ));
    assert!(restarted.client.run("run-integration").await?.sequence > 0);
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn layout_is_optimistic_restart_durable_and_semantically_inert() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let revision_id = import_blueprint(&daemon.client, "layout-import").await?;
    let semantic_digest = daemon
        .client
        .revision(&revision_id)
        .await?
        .summary
        .semantic_digest;
    let layout = LayoutDocument {
        schema_version: 1,
        workflow_id: "golden".to_owned(),
        revision_id: revision_id.clone(),
        generation: 1,
        author: "ignored:client-author".to_owned(),
        digest: String::new(),
        nodes: BTreeMap::from([(
            "first".to_owned(),
            LayoutPoint {
                x: 10.0,
                y: 20.0,
                width: Some(120.0),
                height: None,
            },
        )]),
        collapsed_groups: Default::default(),
        annotations: Default::default(),
        viewport: None,
    }
    .seal()?;
    daemon
        .client
        .submit(&request(
            "layout-put-one",
            None,
            Command::PutLayout {
                layout: layout.clone(),
            },
        ))
        .await?;
    let mut stale = layout.clone();
    stale.nodes.get_mut("first").ok_or("missing layout node")?.x = 30.0;
    stale.digest = stale.computed_digest()?;
    let first_conflict = daemon
        .client
        .submit(&request(
            "layout-stale",
            None,
            Command::PutLayout {
                layout: stale.clone(),
            },
        ))
        .await;
    assert!(matches!(
        first_conflict,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict
    ));
    assert!(matches!(
        daemon
            .client
            .submit(&request(
                "layout-stale",
                None,
                Command::PutLayout { layout: stale },
            ))
            .await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict
    ));
    assert_eq!(
        daemon.client.layout("golden", &revision_id).await?.nodes,
        layout.nodes
    );
    assert_eq!(
        daemon
            .client
            .revision(&revision_id)
            .await?
            .summary
            .semantic_digest,
        semantic_digest
    );
    assert!(!directory.path().join("data/control-state-v1.json").exists());
    daemon.stop().await?;

    let restarted = start(config, CONTROLLER_TOKEN).await?;
    assert_eq!(
        restarted.client.layout("golden", &revision_id).await?.nodes,
        layout.nodes
    );
    assert_eq!(
        restarted
            .client
            .revision(&revision_id)
            .await?
            .summary
            .semantic_digest,
        semantic_digest
    );
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proposal_listing_uses_durable_projection_and_survives_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let revision_id = import_blueprint(&daemon.client, "proposal-index-import").await?;
    daemon
        .client
        .submit(&request(
            "proposal-index-start",
            None,
            Command::StartRun {
                run_id: "run-proposal-index".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: revision_id.clone(),
            },
        ))
        .await?;
    let started_sequence = daemon.client.run("run-proposal-index").await?.sequence;
    daemon
        .client
        .submit(&request(
            "proposal-index-pause",
            Some(started_sequence),
            Command::PauseRun {
                run_id: "run-proposal-index".to_owned(),
            },
        ))
        .await?;
    let run = RunId::new("run-proposal-index")?;
    let sequence = daemon.client.run(run.as_str()).await?.sequence;
    let mut submit = request(
        "proposal-index-submit",
        Some(sequence),
        Command::SubmitProposal {
            document: proposal_document(&run, sequence)?,
        },
    );
    submit.expected_revision = Some(revision_id);
    daemon.client.submit(&submit).await?;
    let listed = daemon
        .client
        .proposals(
            run.as_str(),
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await?;
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].proposal_id, "proposal-daemon-index");
    daemon.stop().await?;

    let restarted = start(config, CONTROLLER_TOKEN).await?;
    let reopened = restarted
        .client
        .proposals(
            run.as_str(),
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await?;
    assert_eq!(reopened.items, listed.items);
    restarted.stop().await
}
