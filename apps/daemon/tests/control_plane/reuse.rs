//! Reuse keeps definitions, source provenance and each execution's writable values independent.
use super::published::fixture;
use super::{
    inputs::{ModelFixture, start_request, upload, workflow},
    support::*,
};
use milkdrift_control_protocol::BlueprintDraft;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_copy_is_editable_and_retains_source_without_run_state() -> TestResult {
    let directory = TempDir::new()?;
    let model = ModelFixture::start().await?;
    model.checked(async |model| {
    // Two accepted runs may enter the controlled endpoint concurrently.
    let mut config =
        super::authoring::model_configuration_with_capacity(&directory, model.address, 2)?;
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .preset = AuthorityPresetConfig::Invoker;
    let plan = config.validate(directory.path())?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let mut retained = None;
    daemon.checked(model, async |daemon| {
    let source = workflow(&daemon.client).await?;
    let original = daemon.client.revision(&source).await?;
    assert_eq!(original.inputs.first().ok_or("input absent")?.name, "brief");
    assert_eq!(
        original.outputs.first().ok_or("output absent")?.name,
        "notes"
    );
    let listing = cli_ok(
        daemon,
        &directory,
        "find",
        &["workflow", "list", "--workflow", "release-notes"],
    )?;
    assert!(
        listing
            .pointer("/items")
            .ok_or("fixture field /items absent")?
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item
                .pointer("/revision_id")
                .and_then(serde_json::Value::as_str)
                == Some(source.as_str())))
    );
    let shown = cli_ok(daemon, &directory, "show", &["workflow", "show", &source])?;
    assert_eq!(
        shown
            .pointer("/inputs/0/name")
            .ok_or("fixture field /inputs/0/name absent")?,
        "brief"
    );
    assert!(
        shown
            .pointer("/document")
            .ok_or("fixture field /document absent")?
            .is_null()
    );
    let hidden = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let mut copy = request(
        "copy-denied",
        None,
        Command::CopyBlueprint {
            source_revision: source.clone(),
            workflow_id: "independent-notes".into(),
            name: "Independent notes".into(),
        },
    );
    copy.expected_revision = Some(source.clone());
    let denied = hidden.submit(&copy).await;
    assert!(
        matches!(&denied, Err(ClientError::Api(error)) if matches!(error.code, ErrorCode::Unauthorized | ErrorCode::NotFound)),
        "{denied:?}"
    );
    assert!(hidden.revision(&source).await.is_err());
    copy.command_id = "copy-stale".into();
    copy.expected_revision = None;
    assert!(
        matches!(daemon.client.submit(&copy).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    let copied = cli_ok(
        daemon,
        &directory,
        "copy-independent",
        &[
            "workflow",
            "copy",
            &source,
            "independent-notes",
            "--name",
            "Independent notes",
            "--file",
            "copy.json",
        ],
    )?;
    let copied_id = copied
        .pointer("/revision_id")
        .ok_or("fixture field /revision_id absent")?
        .as_str()
        .ok_or("copied revision")?;
    let definition = daemon.client.revision(copied_id).await?;
    assert_eq!(definition.summary.workflow_id, "independent-notes");
    assert!(definition.reason.contains(&source));
    assert_eq!(definition.inputs, original.inputs);
    assert_eq!(definition.outputs, original.outputs);
    let (_, copied_definition) =
        BlueprintRevisionDocument::from_json(&serde_json::to_vec(&definition.document)?)?;
    let (_, source_definition) =
        BlueprintRevisionDocument::from_json(&serde_json::to_vec(&original.document)?)?;
    assert_eq!(
        copied_definition.semantic().nodes(),
        source_definition.semantic().nodes()
    );
    assert_eq!(
        copied_definition.semantic().edges(),
        source_definition.semantic().edges()
    );
    let first = upload(
        &daemon.client,
        "harbor-reuse",
        b"Harbor Host 1.4 original brief",
    )
    .await?;
    let second = upload(
        &daemon.client,
        "lantern-copy",
        b"Lantern Rally 0.8 independent brief",
    )
    .await?;
    daemon
        .client
        .submit(&start_request("original-run", &source, vec![first]))
        .await?;
    let mut independent = start_request("copy-run", copied_id, vec![second]);
    if let Command::StartRun { workflow_id, .. } = &mut independent.command {
        *workflow_id = "independent-notes".into();
    }
    daemon.client.submit(&independent).await?;
    fs::write(
        directory.path().join("changed.txt"),
        "Review the draft with a new future instruction.",
    )?;
    cli_ok(
        daemon,
        &directory,
        "edit-copy",
        &[
            "workflow",
            "prompt",
            "copy.json",
            "review",
            "--prompt",
            "changed.txt",
        ],
    )?;
    let saved = cli_ok(
        daemon,
        &directory,
        "save-copy",
        &["workflow", "save", "copy.json"],
    )?;
    assert_ne!(
        saved
            .pointer("/revision_id")
            .ok_or("fixture field /revision_id absent")?,
        copied_id
    );
    let file: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.path().join("copy.json"))?)?;
    let draft: BlueprintDraft = serde_json::from_value(
        file.pointer("/draft")
            .ok_or("fixture field /draft absent")?
            .clone(),
    )?;
    assert_eq!(draft.workflow_id, "independent-notes");
    for (run, pinned, output) in [
        (
            "original-run",
            source.as_str(),
            "Harbor revised release notes",
        ),
        ("copy-run", copied_id, "Lantern revised release notes"),
    ] {
        let state = wait_for_run(&daemon.client, run, Duration::from_secs(45), |state| {
            state.terminal.is_some()
        })
        .await?;
        assert_eq!(state.revision_id.as_deref(), Some(pinned));
        assert_eq!(state.terminal.as_deref(), Some("succeeded"));
        assert_eq!(
            daemon
                .client
                .run_result(run)
                .await?
                .outputs
                .first()
                .ok_or("output absent")?
                .preview
                .as_deref(),
            Some(output)
        );
    }
    assert_eq!(daemon.client.revision(&source).await?, original);
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 4);
    let (base, governed) = fixture::definition()?;
    for (id, definition) in [("base-protected", base), ("governed", governed.clone())] {
        daemon
            .client
            .submit(&request(
                id,
                None,
                Command::ImportBlueprint {
                    document: serde_json::from_slice(
                        &BlueprintRevisionDocument::new(&definition).to_canonical_json()?,
                    )?,
                },
            ))
            .await?;
    }
    copy.command_id = "copy-protected".into();
    copy.expected_revision = Some(governed.id().to_string());
    if let Command::CopyBlueprint {
        source_revision, ..
    } = &mut copy.command
    {
        *source_revision = governed.id().to_string();
    }
    assert!(
        matches!(daemon.client.submit(&copy).await, Err(ClientError::Api(error)) if error.message.contains("governing agreement"))
    );
    retained = Some((source, original, copied_id.to_owned()));
    Ok(())
    }).await?;
    let (source, original, copied_id) = retained.ok_or("successful copy absent")?;
    let daemon = start(plan, CONTROLLER_TOKEN).await?;
    daemon.checked(model, async |daemon| {
    assert_eq!(daemon.client.revision(&source).await?, original);
    assert_eq!(
        daemon.client.run("copy-run").await?.revision_id.as_deref(),
        Some(copied_id.as_str())
    );
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 4);
    Ok(())
    }).await
    }).await
}
