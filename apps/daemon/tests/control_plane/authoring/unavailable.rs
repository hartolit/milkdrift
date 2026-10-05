//! Retained selections can be repaired one step at a time without inventing provenance.
use super::super::{
    inputs,
    results::{Responses, prose},
};
use super::*;
use serde_json::Value;

#[derive(Clone, Copy)]
enum Change {
    Removed,
    Requirements,
    Descriptor,
    Acceptance,
}

fn unresolved(value: &Value) -> TestResult<usize> {
    Ok(value
        .pointer("/workflow/selection_diagnostics")
        .and_then(Value::as_array)
        .ok_or("diagnostics absent")?
        .len())
}

fn stale_acceptance(bytes: &[u8]) -> TestResult<BlueprintRevision> {
    let (_, original) = BlueprintRevisionDocument::from_json(bytes)?;
    let mutations = ["author.draft.accept", "author.review.accept"]
        .into_iter()
        .map(|id| {
            let node = original
                .semantic()
                .nodes()
                .get(&NodeId::new(id)?)
                .ok_or("acceptance absent")?;
            let mut node = serde_json::to_value(node)?;
            *node
                .pointer_mut("/kind/config/requirement/trust_zones")
                .ok_or("requirement absent")? = json!(["unavailable-acceptance-zone"]);
            Ok(Mutation::ReplaceNode {
                node: serde_json::from_value(node)?,
            })
        })
        .collect::<TestResult<Vec<_>>>()?;
    Ok(original.revise(
        original.id(),
        MutationBatch::new(mutations)?,
        AuthorRef::new("human:fixture")?,
        "retained acceptance requirement",
    )?)
}

async fn repair_case(change: Change) -> TestResult {
    let directory = tempfile::tempdir()?;
    let model = Responses::start(vec![
        prose("original draft", "stop"),
        prose("original result", "stop"),
        prose("repaired draft", "stop"),
        prose("repaired result", "stop"),
    ])
    .await?;
    let mut config = model_configuration_document(&directory, model.address)?;
    config
        .actors
        .first_mut()
        .ok_or("actor absent")?
        .authority
        .resources
        .capability = CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown);
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let original_id = inputs::workflow(&daemon.client).await?;
    let input = inputs::upload(
        &daemon.client,
        "unavailable-brief",
        b"immutable original brief",
    )
    .await?;
    daemon
        .client
        .submit(&inputs::start_request(
            "original-run",
            &original_id,
            vec![input.clone()],
        ))
        .await?;
    let old_run = wait_for_run(
        &daemon.client,
        "original-run",
        Duration::from_secs(45),
        |run| run.terminal.is_some(),
    )
    .await?;
    assert_eq!(old_run.terminal.as_deref(), Some("succeeded"));
    let original = daemon.client.revision(&original_id).await?;
    let mut base_id = original_id.clone();
    if matches!(change, Change::Acceptance) {
        let document = original.document.as_ref().ok_or("definition absent")?;
        let stale = stale_acceptance(&serde_json::to_vec(document)?)?;
        daemon
            .client
            .submit(&request(
                "import-stale-acceptance",
                None,
                Command::ImportBlueprint {
                    document: serde_json::from_slice(
                        &BlueprintRevisionDocument::new(&stale).to_canonical_json()?,
                    )?,
                },
            ))
            .await?;
        base_id = stale.id().to_string();
    }
    daemon.stop().await?;
    let a = config
        .adapters
        .model_profiles
        .first()
        .ok_or("profile absent")?
        .clone();
    config
        .adapters
        .model_profiles
        .push(milkdrift_daemon::ModelProfileConfig {
            capability_id: "replacement-model".into(),
            profile: a.profile.clone(),
        });
    match change {
        Change::Removed => {
            config.adapters.model_profiles.remove(0);
        }
        Change::Requirements | Change::Descriptor => {
            let mut profile: Value = serde_json::from_slice(&fs::read(&a.profile)?)?;
            *profile
                .get_mut("revision")
                .ok_or("profile revision absent")? = json!(2);
            if matches!(change, Change::Requirements) {
                *profile
                    .get_mut("trust_zones")
                    .ok_or("profile zones absent")? = json!(["new-trust-zone"]);
            }
            fs::write(&a.profile, serde_json::to_vec(&profile)?)?;
        }
        Change::Acceptance => {}
    }
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let base = BlueprintDraft {
        workflow_id: "release-notes".into(),
        base_revision: Some(base_id.clone()),
        mutations: vec![],
    };
    let opened = author(&daemon.client, &base, "open-unavailable", None, false).await?;
    let count = if matches!(change, Change::Descriptor) {
        0
    } else {
        2
    };
    assert_eq!(unresolved(&opened.value)?, count);
    let hidden = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let hidden_read = author(&hidden, &base, "hidden-retained-read", None, false).await?;
    assert!(unresolved(&hidden_read.value)? >= 2);
    assert!(!serde_json::to_string(&hidden_read.value)?.contains("replacement-model"));
    assert!(
        author(
            &hidden,
            &base,
            "hidden-replacement",
            Some(BlueprintEdit::Model {
                step: "draft".into(),
                capability: "replacement-model".into()
            }),
            false
        )
        .await
        .is_err()
    );
    let document = opened.value.get("document").ok_or("definition absent")?;
    let (_, definition) = BlueprintRevisionDocument::from_json(&serde_json::to_vec(document)?)?;
    let mut forged_node = serde_json::to_value(
        definition
            .semantic()
            .nodes()
            .get(&NodeId::new("draft")?)
            .ok_or("draft node absent")?,
    )?;
    *forged_node
        .pointer_mut("/kind/config/requirement/exact_capability")
        .ok_or("selection absent")? = json!("invented-unavailable-model");
    let mut forged = base.clone();
    forged
        .mutations
        .push(serde_json::to_value(Mutation::ReplaceNode {
            node: serde_json::from_value(forged_node)?,
        })?);
    assert!(
        author(
            &daemon.client,
            &forged,
            "forged-retained-selection",
            None,
            false
        )
        .await
        .is_err()
    );
    let reopened = author(&daemon.client, &base, "save-unavailable", None, true).await?;
    assert_eq!(
        draft(&reopened)?.base_revision.as_deref(),
        Some(base_id.as_str())
    );
    assert_eq!(unresolved(&reopened.value)?, count);
    if matches!(change, Change::Removed | Change::Requirements) {
        daemon
            .client
            .submit(&inputs::start_request(
                "unresolved-run",
                &base_id,
                vec![input.clone()],
            ))
            .await?;
        // Start records durable intent. Admission to an attempt still requires
        // an exact match, so unavailable work remains queued without entry.
        let queued = daemon.client.run("unresolved-run").await?;
        assert!(queued.terminal.is_none());
        assert!(queued.nodes.iter().all(|node| node.attempt_count == 0));
        assert_eq!(model.requests.lock().map_err(|_| "model lock")?.len(), 2);
    }
    // The real CLI receives and displays the daemon's diagnostics while retaining its draft.
    let opened_cli = cli_ok(
        &daemon,
        &directory,
        "cli-open-unavailable",
        &["workflow", "open", &base_id, "--file", "repair.json"],
    )?;
    assert_eq!(unresolved(&opened_cli)?, count);
    cli_ok(
        &daemon,
        &directory,
        "cli-inspect-unavailable",
        &["workflow", "inspect", "repair.json"],
    )?;
    let first = cli_ok(
        &daemon,
        &directory,
        "cli-repair-first",
        &[
            "workflow",
            "model",
            "repair.json",
            "draft",
            "replacement-model",
        ],
    )?;
    assert_eq!(unresolved(&first)?, usize::from(count != 0));
    let partial_file: Value =
        serde_json::from_slice(&fs::read(directory.path().join("repair.json"))?)?;
    let partial: BlueprintDraft = serde_json::from_value(
        partial_file
            .get("draft")
            .ok_or("saved draft absent")?
            .clone(),
    )?;
    let saved_partial = author(&daemon.client, &partial, "save-partial-repair", None, true).await?;
    assert_eq!(unresolved(&saved_partial.value)?, usize::from(count != 0));
    let finished = cli_ok(
        &daemon,
        &directory,
        "cli-repair-second",
        &[
            "workflow",
            "model",
            "repair.json",
            "review",
            "replacement-model",
        ],
    )?;
    assert_eq!(unresolved(&finished)?, 0);
    let saved = cli_ok(
        &daemon,
        &directory,
        "cli-save-repair",
        &["workflow", "save", "repair.json"],
    )?;
    let repaired = saved
        .get("revision_id")
        .and_then(Value::as_str)
        .ok_or("repaired revision absent")?;
    let prepared = daemon
        .client
        .prepare_run(inputs::start_request("repaired-run", repaired, vec![input]))
        .await?;
    daemon.client.submit_saved_run(&prepared).await?;
    let completed = wait_for_run(
        &daemon.client,
        "repaired-run",
        Duration::from_secs(45),
        |run| run.terminal.is_some(),
    )
    .await?;
    assert_eq!(completed.terminal.as_deref(), Some("succeeded"));
    for node in completed
        .nodes
        .iter()
        .filter(|node| ["draft", "review"].contains(&node.node_id.as_str()))
    {
        let attempt = daemon
            .client
            .attempt(
                "repaired-run",
                node.latest_attempt_id.as_deref().ok_or("attempt absent")?,
            )
            .await?;
        assert_eq!(attempt.capability_id.as_deref(), Some("replacement-model"));
    }
    assert!(daemon.client.submit_saved_run(&prepared).await?.replayed);
    assert_eq!(daemon.client.revision(&original_id).await?, original);
    assert_eq!(daemon.client.run("original-run").await?, old_run);
    assert_eq!(model.requests.lock().map_err(|_| "model lock")?.len(), 4);
    if matches!(change, Change::Removed | Change::Requirements) {
        // Executing the repaired child does not retarget the older run to B.
        let queued = daemon.client.run("unresolved-run").await?;
        assert!(queued.terminal.is_none());
        assert!(queued.nodes.iter().all(|node| node.attempt_count == 0));
    }
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removed_models_remain_editable_through_api_and_cli() -> TestResult {
    repair_case(Change::Removed).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_requirements_remain_editable_through_api_and_cli() -> TestResult {
    repair_case(Change::Requirements).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn descriptor_revision_alone_does_not_invalidate_retained_requirements() -> TestResult {
    repair_case(Change::Descriptor).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_acceptance_requirements_can_be_repaired_in_parts() -> TestResult {
    repair_case(Change::Acceptance).await
}
