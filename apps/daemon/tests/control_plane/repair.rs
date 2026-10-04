//! The operator repairs an actual failed model check through public CLI and daemon operations.
use super::{
    inputs::{start_request, upload, workflow},
    results::{Responses, prose},
    support::*,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn final_model_review_repair_preserves_history_context_and_approval_guards() -> TestResult {
    let directory = tempfile::tempdir()?;
    let model = Responses::start(vec![
        prose("PRIVATE EARLIER DRAFT", "stop"),
        prose("SELECTED FAILED RESULT", "length"),
        prose("Complete repaired notes\n\u{1b}[31m", "stop"),
    ])
    .await?;
    let plan = super::authoring::model_configuration(&directory, model.address)?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let revision = workflow(&daemon.client).await?;
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
    preparation.command_id = "prepare-ended".into();
    preparation.expected_sequence = Some(completed.sequence);
    preparation.expected_revision = completed.revision_id;
    assert!(
        matches!(daemon.client.submit(&preparation).await,Err(ClientError::Api(error)) if error.message.contains("ended"))
    );
    daemon.stop().await?;
    let daemon = start(plan, CONTROLLER_TOKEN).await?;
    assert_eq!(
        daemon.client.run_result("repair-run").await?.outputs,
        result.outputs
    );
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 3);
    daemon.stop().await?;
    Ok(())
}
