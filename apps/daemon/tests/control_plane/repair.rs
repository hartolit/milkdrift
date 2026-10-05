//! The operator repairs an actual failed model check through public CLI and daemon operations.
use super::{
    inputs::{start_request, upload, workflow_with_review_input},
    results::{Responses, prose},
    support::*,
};

#[path = "repair/reserved_inputs.rs"]
mod reserved_inputs;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn final_model_review_repair_preserves_history_context_and_approval_guards() -> TestResult {
    repair_case("brief", false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_failed_result_input_remains_distinct_from_selected_repair_evidence() -> TestResult {
    repair_case("failed_result", false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_repair_mutations_preserve_evidence_ids_approval_and_exact_replay() -> TestResult {
    repair_case("brief", true).await
}

async fn repair_case(user_input: &str, retained: bool) -> TestResult {
    let directory = tempfile::tempdir()?;
    let model = Responses::start(vec![
        prose("PRIVATE EARLIER DRAFT", "stop"),
        prose("SELECTED FAILED RESULT", "length"),
        prose("Complete repaired notes\n\u{1b}[31m", "stop"),
    ])
    .await?;
    let plan = super::authoring::model_configuration(&directory, model.address)?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let revision = if retained {
        let document: serde_json::Value =
            serde_json::from_slice(include_bytes!("../fixtures/authoring/legacy-workflow.json"))?;
        let id = document
            .pointer("/revision/id")
            .and_then(serde_json::Value::as_str)
            .ok_or("retained id")?
            .to_owned();
        daemon
            .client
            .submit(&request(
                "import-retained",
                None,
                Command::ImportBlueprint { document },
            ))
            .await?;
        id
    } else {
        workflow_with_review_input(&daemon.client, user_input).await?
    };
    let original_definition = daemon.client.revision(&revision).await?.document;
    let input = upload(
        &daemon.client,
        "brief",
        include_bytes!("../../../../examples/operator/release-notes/harbor-brief.txt"),
    )
    .await?;
    upload(&daemon.client, "unrelated", b"UNRELATED PRIVATE HISTORY").await?;
    daemon
        .client
        .submit(&start_request("repair-run", &revision, vec![input]))
        .await?;
    let waiting = wait_for_run(
        &daemon.client,
        "repair-run",
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
            "pause-repair",
            Some(waiting.sequence),
            Command::PauseRun {
                run_id: "repair-run".into(),
            },
        ))
        .await?;
    let paused = daemon.client.run("repair-run").await?;
    let mut preparation = request(
        "prepare-denied",
        Some(paused.sequence),
        Command::PrepareModelRepair {
            run_id: "repair-run".into(),
            proposal_id: "repair-final".into(),
            repair: milkdrift_control_protocol::ModelRepair {
                failed_step: "review".into(),
                repair_step: "repair".into(),
                capability: "writing-model".into(),
                prompt: "Repair using only the selected result and original brief.".into(),
                maximum_output_units: 512,
            },
        },
    );
    preparation.expected_revision = Some(revision.clone());
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(
        matches!(observer.submit(&preparation).await,Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized)
    );
    preparation.command_id = "prepare-stale".into();
    preparation.expected_sequence = Some(paused.sequence.saturating_sub(1));
    assert!(
        matches!(daemon.client.submit(&preparation).await,Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    preparation.command_id = "prepare-preview".into();
    preparation.expected_sequence = Some(paused.sequence);
    preparation.expected_revision = Some(format!("rev_{}", "0".repeat(64)));
    assert!(
        matches!(daemon.client.submit(&preparation).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    preparation.command_id = "prepare-current".into();
    preparation.expected_revision = Some(revision.clone());
    fs::write(
        directory.path().join("prepare.request.json"),
        serde_json::to_vec(&preparation)?,
    )?;
    let prepared = daemon.client.submit(&preparation).await?;
    assert_eq!(daemon.client.run("repair-run").await?, paused);
    assert!(
        daemon
            .client
            .proposals(
                "repair-run",
                &PageRequest {
                    limit: 20,
                    cursor: None
                }
            )
            .await?
            .items
            .is_empty()
    );
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);
    fs::write(
        directory.path().join("repair.txt"),
        "Repair using only the selected failed result and original brief.",
    )?;
    let args = [
        "proposal",
        "repair",
        "repair-run",
        "review",
        "--proposal",
        "repair-final",
        "--new-step",
        "repair",
        "--model",
        "writing-model",
        "--prompt",
        "repair.txt",
        "--maximum-output-units",
        "512",
        "--file",
        "repair.json",
    ];
    cli_ok(&daemon, &directory, "prepare-final", &args)?;
    if retained {
        retain_old_repair_mutations(&directory.path().join("repair.json"))?;
    }
    assert_eq!(
        daemon.client.run("repair-run").await?.sequence,
        paused.sequence
    );
    let denied = cli(
        &daemon,
        &directory,
        "submit-stale",
        &[
            "--expected-sequence",
            "0",
            "proposal",
            "submit",
            "repair.json",
        ],
        true,
    )?;
    assert!(!denied.0);
    let original_history = daemon
        .client
        .timeline(
            "repair-run",
            &PageRequest {
                limit: 1000,
                cursor: None,
            },
        )
        .await?
        .items;
    let mut submission = request(
        "submit-final",
        None,
        Command::SubmitProposal {
            document: serde_json::from_slice(&fs::read(directory.path().join("repair.json"))?)?,
        },
    );
    submission.reason = "operator CLI command".into();
    fs::write(
        directory.path().join("submit.request.json"),
        serde_json::to_vec(&submission)?,
    )?;
    let submitted = cli_ok(
        &daemon,
        &directory,
        "submit-final",
        &["proposal", "submit", "repair.json"],
    )?;
    let submitted = &submitted
        .pointer("/value")
        .ok_or("fixture field /value absent")?;
    let proposed = submitted
        .pointer("/proposed_revision")
        .ok_or("fixture field /proposed_revision absent")?
        .as_str()
        .ok_or("proposed revision")?
        .to_owned();
    let digest = submitted
        .pointer("/proposal_digest")
        .ok_or("fixture field /proposal_digest absent")?
        .as_str()
        .ok_or("proposal digest")?
        .to_owned();
    let proposed_document = daemon
        .client
        .revision(&proposed)
        .await?
        .document
        .ok_or("proposed document")?;
    let repair_inputs = proposed_document
        .pointer("/revision/semantic/nodes/repair/data_inputs")
        .ok_or("repair inputs")?;
    assert_eq!(
        repair_inputs
            .get(user_input)
            .and_then(|input| input.get("binding")),
        Some(&serde_json::json!({"type":"workflow_input","field":"brief"}))
    );
    let evidence_input = if retained {
        "failed_result"
    } else {
        "milkdrift.failed_result"
    };
    assert_eq!(
        repair_inputs
            .get(evidence_input)
            .and_then(|input| input.get("binding")),
        Some(
            &serde_json::json!({"type":"node_output","node":"review","port":"model_response","path":[]})
        )
    );
    let (_, proposed_definition) =
        BlueprintRevisionDocument::from_json(&serde_json::to_vec(&proposed_document)?)?;
    if retained {
        assert_eq!(
            proposed_definition.content_digest().as_str(),
            "b3_0f69bdf831d5ca7cf8406e11a915c9bd26e6d425103207124673285b78c12f90"
        );
    }
    for edge in proposed_definition
        .semantic()
        .edges()
        .values()
        .filter(|edge| !retained && edge.target_node().as_str() == "author.repair.done")
    {
        let encoded = match edge.kind() {
            EdgeKind::Data => {
                r#"["milkdrift.author.edge.v2","data","repair","final_text","author.repair.done","notes"]"#
            }
            EdgeKind::Control => {
                r#"["milkdrift.author.edge.v2","control","author.repair.gate","pass","author.repair.done","in"]"#
            }
        };
        assert_eq!(
            edge.id().as_str(),
            format!("author.{}", blake3::hash(encoded.as_bytes()))
        );
    }
    assert_eq!(
        submitted
            .pointer("/applied")
            .ok_or("fixture field /applied absent")?,
        false
    );
    let impact = daemon
        .client
        .proposal("repair-run", "repair-final", &proposed)
        .await?;
    assert!(!impact.approved);
    assert_eq!(impact.applied_sequence, None);
    assert!(
        impact.impact.as_ref().is_some_and(|items| items
            .iter()
            .any(|item| item.node.as_deref() == Some("author.review.accept")
                && item.classification == "unchanged_completed"
                && item.action == "preserve")),
        "{impact:?}"
    );
    let apply_args = [
        "proposal",
        "apply",
        "repair-run",
        "repair-final",
        &digest,
        &proposed,
    ];
    assert!(
        !cli(
            &daemon,
            &directory,
            "apply-before-approval",
            &apply_args,
            true
        )?
        .0
    );
    daemon.stop().await?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let saved_submission: CommandRequest =
        serde_json::from_slice(&fs::read(directory.path().join("submit.request.json"))?)?;
    let submitted_again = daemon.client.submit(&saved_submission).await?;
    assert!(submitted_again.replayed);
    assert_eq!(&submitted_again.value, *submitted);
    let saved_preparation: CommandRequest =
        serde_json::from_slice(&fs::read(directory.path().join("prepare.request.json"))?)?;
    let replayed = daemon.client.submit(&saved_preparation).await?;
    assert!(replayed.replayed);
    assert_eq!(replayed.value, prepared.value);
    assert!(
        !daemon
            .client
            .proposal("repair-run", "repair-final", &proposed)
            .await?
            .approved
    );
    let approve_args = [
        "proposal",
        "approve",
        "repair-run",
        "repair-final",
        &digest,
        &proposed,
        "approve-final",
    ];
    cli_ok(&daemon, &directory, "approve-final", &approve_args)?;
    let approved_sequence = daemon.client.run("repair-run").await?.sequence;
    let mut application = request(
        "apply-stale",
        Some(approved_sequence.saturating_sub(1)),
        Command::ApplyProposal {
            run_id: "repair-run".into(),
            proposal_id: "repair-final".into(),
            proposal_digest: digest.clone(),
            proposed_revision: proposed.clone(),
        },
    );
    application.expected_revision = Some(proposed.clone());
    assert!(matches!(daemon.client.submit(&application).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict));
    application.command_id = "apply-unauthorized".into();
    application.expected_sequence = Some(approved_sequence);
    assert!(
        matches!(client(&daemon.endpoint, OBSERVER_TOKEN)?.submit(&application).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized)
    );
    assert_eq!(
        daemon
            .client
            .run("repair-run")
            .await?
            .revision_id
            .as_deref(),
        Some(revision.as_str())
    );
    application.command_id = "apply-final".into();
    application.reason = "operator CLI command".into();
    fs::write(
        directory.path().join("apply.request.json"),
        serde_json::to_vec(&application)?,
    )?;
    cli_ok(&daemon, &directory, "apply-final", &apply_args)?;
    assert_eq!(
        daemon
            .client
            .run("repair-run")
            .await?
            .revision_id
            .as_deref(),
        Some(proposed.as_str())
    );
    let signal_sequence = daemon.client.run("repair-run").await?.sequence.to_string();
    let signalled = cli_ok(
        &daemon,
        &directory,
        "signal-final",
        &[
            "--expected-sequence",
            &signal_sequence,
            "run",
            "signal",
            "repair-run",
            "--signal-id",
            "release-repair",
            "--signal-type",
            "workflow.reviewed",
        ],
    )?;
    let resume_sequence = signalled
        .pointer("/resulting_sequence")
        .ok_or("fixture field /resulting_sequence absent")?
        .as_u64()
        .ok_or("signal sequence")?
        .to_string();
    cli_ok(
        &daemon,
        &directory,
        "resume-final",
        &[
            "--expected-sequence",
            &resume_sequence,
            "run",
            "resume",
            "repair-run",
        ],
    )?;
    let completed = wait_for_run(
        &daemon.client,
        "repair-run",
        Duration::from_secs(45),
        |run| run.terminal.is_some(),
    )
    .await?;
    assert_eq!(completed.terminal.as_deref(), Some("succeeded"));
    for node in paused
        .nodes
        .iter()
        .filter(|node| node.state.starts_with("terminal"))
    {
        assert_eq!(
            completed
                .nodes
                .iter()
                .find(|current| current.execution_id == node.execution_id),
            Some(node)
        );
    }
    let history = daemon
        .client
        .timeline(
            "repair-run",
            &PageRequest {
                limit: 1000,
                cursor: None,
            },
        )
        .await?
        .items;
    assert_eq!(
        history
            .get(..original_history.len())
            .ok_or("retained history shortened")?,
        original_history.as_slice()
    );
    assert_eq!(
        daemon.client.revision(&revision).await?.document,
        original_definition
    );
    let result = daemon.client.run_result("repair-run").await?;
    assert_eq!(
        result
            .outputs
            .first()
            .ok_or("output absent")?
            .preview
            .as_deref(),
        Some("Complete repaired notes\n\u{1b}[31m")
    );
    assert!(
        result
            .run
            .nodes
            .iter()
            .filter_map(|n| n.latest_attempt.as_ref())
            .filter_map(|a| a.result_acceptance.as_ref())
            .any(|a| !a.accepted)
    );
    let (ok, human) = cli(
        &daemon,
        &directory,
        "inspect-result",
        &[
            "run",
            "result",
            "repair-run",
            "--details",
            "--field",
            "notes",
            "--output",
            "notes.txt",
        ],
        false,
    )?;
    assert!(ok, "{human}");
    assert!(human.contains("Workflow outcome: succeeded"));
    assert!(human.contains("Required result: rejected"));
    assert!(human.contains("Required result: accepted"));
    assert!(!human.contains('\u{1b}'));
    assert_eq!(
        fs::read_to_string(directory.path().join("notes.txt"))?,
        "Complete repaired notes\n\u{1b}[31m"
    );
    let requests = model.requests.lock().map_err(|_| "fixture lock")?.clone();
    assert_eq!(requests.len(), 3);
    let repaired = requests.get(2).ok_or("repair request absent")?.to_string();
    assert!(repaired.contains("SELECTED FAILED RESULT"));
    assert!(repaired.contains("Harbor Host 1.4"));
    assert!(!repaired.contains("PRIVATE EARLIER DRAFT"));
    assert!(!repaired.contains("UNRELATED PRIVATE HISTORY"));
    assert_repair_evidence(
        requests.get(2).ok_or("repair request absent")?,
        user_input,
        evidence_input,
    )?;
    preparation.command_id = "prepare-ended".into();
    preparation.expected_sequence = Some(completed.sequence);
    preparation.expected_revision = completed.revision_id;
    assert!(
        matches!(daemon.client.submit(&preparation).await,Err(ClientError::Api(error)) if error.message.contains("ended"))
    );
    daemon.stop().await?;
    let daemon = start(plan, CONTROLLER_TOKEN).await?;
    let saved_application: CommandRequest =
        serde_json::from_slice(&fs::read(directory.path().join("apply.request.json"))?)?;
    let before_replay = daemon.client.run("repair-run").await?;
    assert!(daemon.client.submit(&saved_application).await?.replayed);
    assert_eq!(daemon.client.run("repair-run").await?, before_replay);
    assert_eq!(
        daemon.client.revision(&proposed).await?.document,
        Some(proposed_document)
    );
    assert_eq!(
        daemon.client.run_result("repair-run").await?.outputs,
        result.outputs
    );
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 3);
    daemon.stop().await?;
    Ok(())
}

fn retain_old_repair_mutations(path: &std::path::Path) -> TestResult {
    // Freeze the old writer's mutation bytes, while binding this test's fresh pause/evidence.
    // Reading and replaying an accepted proposal must never regenerate its graph from a gesture.
    let old = WorkflowProposalDocument::from_json(include_bytes!(
        "../fixtures/authoring/legacy-repair-proposal.json"
    ))?;
    assert_eq!(
        old.proposal().mutation().id().as_str(),
        "batch_d2e18969f788c99ee06f0d0341c574bd0dd430808e8dc50c7c5cc6242669a270"
    );
    let document: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    let mut draft = document
        .get("proposal")
        .and_then(serde_json::Value::as_object)
        .ok_or("prepared proposal")?
        .clone();
    draft.remove("digest");
    draft.insert(
        "mutation".into(),
        serde_json::to_value(old.proposal().mutation().operations())?,
    );
    let rebound = WorkflowProposalDocument::from_json(&serde_json::to_vec(
        &serde_json::json!({"schema_version":1,"draft":draft}),
    )?)?;
    fs::write(path, rebound.to_canonical_json()?)?;
    Ok(())
}

fn assert_repair_evidence(
    request: &serde_json::Value,
    user_input: &str,
    evidence_input: &str,
) -> TestResult {
    let mut evidence = BTreeMap::new();
    for message in request
        .get("messages")
        .and_then(serde_json::Value::as_array)
        .ok_or("messages")?
    {
        for part in message
            .get("content")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(text) = part
                .get("text")
                .and_then(serde_json::Value::as_str)
                .and_then(|text| text.strip_prefix("BEGIN MILKDRIFT EVIDENCE "))
            else {
                continue;
            };
            let (label, body) = text.split_once('\n').ok_or("evidence label")?;
            let label: serde_json::Value = serde_json::from_str(label)?;
            let name = label
                .pointer("/source/name")
                .and_then(serde_json::Value::as_str)
                .ok_or("direct input name")?;
            assert!(evidence.insert(name.to_owned(), body.to_owned()).is_none());
        }
    }
    assert_eq!(evidence.len(), 2);
    let brief = evidence.get(user_input).ok_or("brief evidence")?;
    assert!(brief.contains("Harbor Host 1.4"));
    assert!(!brief.contains("SELECTED FAILED RESULT"));
    let rejected = evidence.get(evidence_input).ok_or("failed evidence")?;
    assert!(rejected.contains("SELECTED FAILED RESULT"));
    assert!(!rejected.contains("Harbor Host 1.4"));
    Ok(())
}
