use super::super::{
    inputs,
    results::{Responses, prose},
};
use super::*;

const LEGACY: &[u8] = include_bytes!("../../fixtures/authoring/legacy-workflow.json");
const ID: &str = "rev_1ea128cd02e5808b28d8028faf51b5e46941111ae9dedfc68d55b83588784fde";
const DIGEST: &str = "b3_22ef6c2e01d1c7a00a92d6a987431d7c01afd320ae0f8093fad1e30990d5ebc8";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_editor_ids_survive_noops_child_edits_copy_restart_and_exact_replay() -> TestResult
{
    let original: serde_json::Value = serde_json::from_slice(LEGACY)?;
    let (_, definition) = BlueprintRevisionDocument::from_json(LEGACY)?;
    assert_eq!(definition.id().as_str(), ID);
    assert_eq!(definition.content_digest().as_str(), DIGEST);
    let directory = tempfile::tempdir()?;
    let model = Responses::start(vec![
        prose("OLD DRAFT", "stop"),
        prose("OLD RESULT", "stop"),
    ])
    .await?;
    let plan = model_configuration(&directory, model.address)?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let imported = request(
        "import-old",
        None,
        Command::ImportBlueprint {
            document: original.clone(),
        },
    );
    daemon.client.submit(&imported).await?;
    let retained = BlueprintDraft {
        workflow_id: "release-notes".into(),
        base_revision: Some(ID.into()),
        mutations: vec![],
    };
    for (command, edit, save) in [
        ("open-old", None, false),
        ("save-old", None, true),
        (
            "same-name",
            Some(BlueprintEdit::Rename {
                name: "release-notes".into(),
            }),
            true,
        ),
    ] {
        let result = author(&daemon.client, &retained, command, edit, save).await?;
        assert_eq!(result.value.get("document"), Some(&original));
        assert_eq!(draft(&result)?, retained);
    }
    let accepted = author(&daemon.client, &retained, "save-old", None, true).await?;
    assert!(accepted.replayed);
    let input = inputs::upload(&daemon.client, "legacy-brief", b"LEGACY AUTHORIZED BRIEF").await?;
    let saved_run = daemon
        .client
        .prepare_run(inputs::start_request("legacy-run", ID, vec![input]))
        .await?;
    daemon.client.submit_saved_run(&saved_run).await?;
    let completed = wait_for_run(
        &daemon.client,
        "legacy-run",
        Duration::from_secs(45),
        |run| run.terminal.is_some(),
    )
    .await?;
    assert_eq!(completed.terminal.as_deref(), Some("succeeded"));

    let added = author(
        &daemon.client,
        &retained,
        "extend-old",
        Some(add("review:x")),
        false,
    )
    .await?;
    let connected = author(
        &daemon.client,
        &draft(&added)?,
        "connect-child",
        Some(BlueprintEdit::Connect {
            step: "review:x".into(),
            input: "brief".into(),
            source: ModelInputSource::Step {
                step: "draft".into(),
            },
        }),
        true,
    )
    .await?;
    let child = draft(&connected)?;
    assert_ne!(child.base_revision.as_deref(), Some(ID));
    let child_doc = connected.value.get("document").ok_or("child document")?;
    assert_eq!(child_doc.pointer("/revision/parents"), Some(&json!([ID])));
    let (_, child_definition) =
        BlueprintRevisionDocument::from_json(&serde_json::to_vec(child_doc)?)?;
    for edge in definition
        .semantic()
        .edges()
        .values()
        .filter(|edge| edge.kind() == EdgeKind::Data)
    {
        assert_eq!(
            child_definition.semantic().edges().get(edge.id()),
            Some(edge)
        );
    }
    let reopened = author(&daemon.client, &child, "reopen-child", None, true).await?;
    assert_eq!(reopened.value.get("document"), Some(child_doc));
    assert_eq!(
        daemon.client.revision(ID).await?.document,
        Some(original.clone())
    );
    let mut copy = request(
        "copy-old",
        None,
        Command::CopyBlueprint {
            source_revision: ID.into(),
            workflow_id: "copied-old".into(),
            name: "Copied old workflow".into(),
        },
    );
    copy.expected_revision = Some(ID.into());
    let copied = daemon.client.submit(&copy).await?;
    assert_eq!(
        copied.value.pointer("/document/revision/semantic/edges"),
        original.pointer("/revision/semantic/edges")
    );
    author(&daemon.client, &draft(&copied)?, "open-copy", None, false).await?;

    // Recognition must verify the retained IDs, not simply discard them before comparison.
    let edge = definition
        .semantic()
        .edges()
        .values()
        .next()
        .ok_or("legacy edge")?;
    let forged = definition.revise(
        definition.id(),
        MutationBatch::new(vec![
            Mutation::RemoveEdge {
                edge: edge.id().clone(),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("arbitrary-id")?,
                    edge.kind(),
                    edge.source_node().clone(),
                    edge.source_port().clone(),
                    edge.target_node().clone(),
                    edge.target_port().clone(),
                ),
            },
        ])?,
        definition.author().clone(),
        "unsupported edge identity",
    )?;
    let forged_doc =
        serde_json::from_slice(&BlueprintRevisionDocument::new(&forged).to_canonical_json()?)?;
    daemon
        .client
        .submit(&request(
            "import-forged",
            None,
            Command::ImportBlueprint {
                document: forged_doc,
            },
        ))
        .await?;
    let unsupported = BlueprintDraft {
        base_revision: Some(forged.id().to_string()),
        ..retained.clone()
    };
    assert!(
        matches!(author(&daemon.client, &unsupported, "refuse-forged", None, true).await, Err(ClientError::Api(error)) if error.message.contains("unsupported editor edge identity"))
    );
    let duplicate = BlueprintDraft {
        mutations: vec![serde_json::to_value(Mutation::AddEdge {
            edge: edge.clone(),
        })?],
        ..retained.clone()
    };
    assert!(
        matches!(author(&daemon.client, &duplicate, "refuse-duplicate", None, true).await, Err(ClientError::Api(error)) if error.message.contains("DuplicateIdentity"))
    );
    assert_eq!(
        daemon.client.revision(ID).await?.document,
        Some(original.clone())
    );
    daemon.stop().await?;
    let daemon = start(plan, CONTROLLER_TOKEN).await?;
    assert!(daemon.client.submit(&imported).await?.replayed);
    let replay = author(&daemon.client, &retained, "save-old", None, true).await?;
    assert!(replay.replayed);
    assert_eq!(replay.value, accepted.value);
    assert!(daemon.client.submit_saved_run(&saved_run).await?.replayed);
    assert_eq!(daemon.client.run("legacy-run").await?, completed);
    assert_eq!(daemon.client.revision(ID).await?.document, Some(original));
    assert_eq!(
        author(
            &daemon.client,
            &child,
            "open-child-after-restart",
            None,
            false
        )
        .await?
        .value
        .get("document"),
        Some(child_doc)
    );
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);
    daemon.stop().await
}
