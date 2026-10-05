use super::*;
use milkdrift_control_protocol::{BlueprintDraft, BlueprintEdit, ModelInputSource, ModelRepair};
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reserved_edits_and_imported_repair_input_conflicts_refuse_without_mutation() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let model = Responses::start(vec![
        prose("PRIVATE EARLIER DRAFT", "stop"),
        prose("SELECTED FAILED RESULT", "length"),
    ])
    .await?;
    let daemon = start(
        super::super::authoring::model_configuration(&directory, model.address)?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let revision = workflow_with_review_input(&daemon.client, "brief").await?;
    let original = daemon
        .client
        .revision(&revision)
        .await?
        .document
        .ok_or("original definition")?;
    let draft = BlueprintDraft {
        workflow_id: "release-notes".into(),
        base_revision: Some(revision.clone()),
        mutations: vec![],
    };
    let mut edit = request(
        "reserved-input",
        None,
        Command::AuthorBlueprint {
            draft: draft.clone(),
            edit: Some(BlueprintEdit::Connect {
                step: "review".into(),
                input: "milkdrift.failed_result".into(),
                source: ModelInputSource::RunInput {
                    name: "brief".into(),
                },
            }),
            save: true,
        },
    );
    edit.expected_revision = Some(revision.clone());
    assert!(
        matches!(daemon.client.submit(&edit).await, Err(ClientError::Api(error)) if error.message.contains("reserved input name"))
    );
    assert_eq!(
        daemon.client.revision(&revision).await?.document,
        Some(original.clone())
    );
    let mut unchanged = request(
        "unchanged-after-refusal",
        None,
        Command::AuthorBlueprint {
            draft: draft.clone(),
            edit: None,
            save: true,
        },
    );
    unchanged.expected_revision = Some(revision.clone());
    assert_eq!(
        daemon
            .client
            .submit(&unchanged)
            .await?
            .value
            .get("document"),
        Some(&original)
    );

    // The advanced command admits explicit blueprint content. That saved graph is not proof
    // that a later convenience operation may overwrite a port in its reserved namespace.
    let mut node = original
        .pointer("/revision/semantic/nodes/review")
        .ok_or("review node")?
        .clone();
    let inputs = node
        .get_mut("data_inputs")
        .and_then(Value::as_object_mut)
        .ok_or("inputs")?;
    let input = inputs.remove("brief").ok_or("brief port")?;
    inputs.insert("milkdrift.failed_result".into(), input);
    let imported_draft = BlueprintDraft {
        mutations: vec![json!({"type":"replace_node","node":node})],
        ..draft
    };
    let mut construct = request(
        "import-reserved",
        None,
        Command::ConstructBlueprint {
            draft: imported_draft,
            store: true,
        },
    );
    construct.expected_revision = Some(revision.clone());
    let imported = daemon.client.submit(&construct).await?;
    let imported_id = imported
        .value
        .get("revision_id")
        .and_then(Value::as_str)
        .ok_or("imported revision")?;
    let input = upload(&daemon.client, "brief", b"Harbor Host 1.4").await?;
    daemon
        .client
        .submit(&start_request("reserved-run", imported_id, vec![input]))
        .await?;
    let waiting = wait_for_run(
        &daemon.client,
        "reserved-run",
        Duration::from_secs(45),
        |run| {
            run.nodes
                .iter()
                .any(|node| node.node_id == "author.review.hold")
        },
    )
    .await?;
    daemon
        .client
        .submit(&request(
            "pause-reserved",
            Some(waiting.sequence),
            Command::PauseRun {
                run_id: "reserved-run".into(),
            },
        ))
        .await?;
    let paused = daemon.client.run("reserved-run").await?;
    let mut prepare = request(
        "reserved-repair",
        Some(paused.sequence),
        Command::PrepareModelRepair {
            run_id: "reserved-run".into(),
            proposal_id: "reserved-proposal".into(),
            repair: ModelRepair {
                failed_step: "review".into(),
                repair_step: "repair".into(),
                capability: "writing-model".into(),
                prompt: "Repair selected evidence".into(),
                maximum_output_units: 512,
            },
        },
    );
    prepare.expected_revision = Some(imported_id.into());
    assert!(
        matches!(daemon.client.submit(&prepare).await, Err(ClientError::Api(error)) if error.message.contains("repair evidence input milkdrift.failed_result is already occupied"))
    );
    assert_eq!(daemon.client.run("reserved-run").await?, paused);
    assert!(
        daemon
            .client
            .proposals(
                "reserved-run",
                &PageRequest {
                    limit: 20,
                    cursor: None
                }
            )
            .await?
            .items
            .is_empty()
    );
    assert_eq!(
        daemon.client.revision(imported_id).await?.document.as_ref(),
        imported.value.get("document")
    );
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);
    daemon.stop().await
}
