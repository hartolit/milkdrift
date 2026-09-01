//! End-to-end command, controller, authority, and stream workflow behavior.

use super::support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_sequence_validate_import_inspect_and_restart_are_one_control_path() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 32)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let document = prompt_sequence()?;

    let validated = daemon
        .client
        .submit(&request(
            "prompt-sequence-validate",
            None,
            Command::ValidatePromptSequence {
                document: document.clone(),
            },
        ))
        .await?;
    assert_eq!(validated.result_type, "prompt_sequence_valid");
    let revision = validated.value["revision_id"]
        .as_str()
        .ok_or("validation omitted revision")?
        .to_owned();
    assert!(daemon.client.revision(&revision).await.is_err());

    let imported = daemon
        .client
        .submit(&request(
            "prompt-sequence-import",
            None,
            Command::ImportPromptSequence {
                document: document.clone(),
            },
        ))
        .await?;
    assert_eq!(imported.result_type, "prompt_sequence_imported");
    assert_eq!(imported.value["revision_id"], revision);
    assert_eq!(imported.value["stages"].as_array().map(Vec::len), Some(1));
    assert!(
        imported.value["import_digest"]
            .as_str()
            .is_some_and(|value| value.starts_with("b3_"))
    );
    let read = daemon.client.revision(&revision).await?;
    assert_eq!(read.summary.workflow_id, "milkdrift-core-convergence");
    assert_eq!(read.node_count, 7);
    assert!(read.document.is_some());

    let duplicate = daemon
        .client
        .submit(&request(
            "prompt-sequence-import-duplicate",
            None,
            Command::ImportPromptSequence { document },
        ))
        .await?;
    assert!(duplicate.replayed);
    daemon.stop().await?;

    let restarted = start(config, CONTROLLER_TOKEN).await?;
    let reopened = restarted.client.revision(&revision).await?;
    assert_eq!(reopened.node_count, 7);
    assert_eq!(
        reopened.summary.semantic_digest,
        read.summary.semantic_digest
    );
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn headless_dogfood_failure_remediation_and_restart_are_durable() -> TestResult {
    let directory = tempfile::tempdir()?;
    let repository = directory.path().join("repository");
    fs::create_dir(&repository)?;
    let profiles = dogfood_process_profiles(&directory, &repository)?;
    let profile_paths = vec![
        profiles.coding,
        profiles.good_verification,
        profiles.weak_verification,
        profiles.reviewer,
    ];
    let config = configuration_with_process_profiles(&directory, 64, profile_paths)?;
    let sequence = executable_dogfood_sequence()?;
    let sequence_value = serde_json::to_value(&sequence)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;

    let imported = daemon
        .client
        .submit(&request(
            "dogfood-import",
            None,
            Command::ImportPromptSequence {
                document: sequence_value,
            },
        ))
        .await?;
    let revision_id = imported.value["revision_id"]
        .as_str()
        .ok_or("dogfood import omitted revision")?
        .to_owned();
    daemon
        .client
        .submit(&request(
            "dogfood-start",
            None,
            Command::StartRun {
                run_id: "run-headless-dogfood".to_owned(),
                workflow_id: "daemon-headless-dogfood".to_owned(),
                revision_id: revision_id.clone(),
            },
        ))
        .await?;

    let waiting = wait_for_run(&daemon.client, "run-headless-dogfood", |state| {
        state
            .nodes
            .iter()
            .any(|node| node.node_id == "stage-two-approval")
    })
    .await?;
    assert_eq!(waiting.lifecycle, "running");
    assert!(waiting.terminal.is_none());
    assert!(!waiting.nodes.iter().any(|node| {
        node.node_id == "sequence-succeeded" || node.node_id.starts_with("stage-three-")
    }));
    daemon.stop().await?;

    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let waiting = daemon.client.run("run-headless-dogfood").await?;
    assert_eq!(waiting.lifecycle, "running");
    assert!(
        waiting
            .nodes
            .iter()
            .any(|node| node.node_id == "stage-two-approval")
    );
    assert!(!waiting.nodes.iter().any(|node| {
        node.node_id == "sequence-succeeded" || node.node_id.starts_with("stage-three-")
    }));
    let reviewer =
        attempt_id_for_node(&daemon.client, "run-headless-dogfood", "stage-two-review").await?;
    let reviewer_attempt = daemon
        .client
        .attempt("run-headless-dogfood", &reviewer)
        .await?;
    assert_eq!(reviewer_attempt.context_access, "authorized");
    let reviewer_context = reviewer_attempt
        .context
        .ok_or("reviewer context is absent")?;
    assert_eq!(reviewer_context.policy["session"], "fresh");
    assert!(
        serde_json::to_string(&reviewer_context.policy)?.contains("prior_prompt"),
        "review policy must explicitly exclude chronological prior prompts"
    );

    daemon
        .client
        .submit(&request(
            "dogfood-pause-after-failure",
            Some(waiting.sequence),
            Command::PauseRun {
                run_id: "run-headless-dogfood".to_owned(),
            },
        ))
        .await?;
    let paused = daemon.client.run("run-headless-dogfood").await?;
    assert_eq!(paused.lifecycle, "paused");

    let revision_read = daemon.client.revision(&revision_id).await?;
    let base_bytes = serde_json::to_vec(
        revision_read
            .document
            .as_ref()
            .ok_or("base revision document is absent")?,
    )?;
    let (_base_document, base) = BlueprintRevisionDocument::from_json(&base_bytes)?;
    let good_verification = sequence.sequence().stages[0].verification.clone();
    let proposal = build_remediation_proposal(
        &sequence,
        &base,
        RemediationProposalSpec {
            run: RunId::new("run-headless-dogfood")?,
            observed_sequence: milkdrift_persistence::RunSequence::new(paused.sequence),
            proposal: ProposalId::new("proposal-headless-remediation-1")?,
            proposer: ActorRef::new("human:integration-controller")?,
            stage_id: "two".to_owned(),
            generation: 1,
            prompt: PromptSource::InlineMarkdown {
                content: "Remediation fresh process repairs the weak implementation.\n".to_owned(),
            },
            verification_override: Some(good_verification),
        },
    )?;
    let proposal_digest = proposal.proposal().digest().as_str().to_owned();
    let proposal_value = decode_json(&proposal.to_canonical_json()?)?;
    let mut submit_request = request(
        "dogfood-submit-remediation",
        Some(paused.sequence),
        Command::SubmitProposal {
            document: proposal_value,
        },
    );
    submit_request.expected_revision = Some(revision_id.clone());
    let submitted = daemon.client.submit(&submit_request).await?;
    let proposed_revision = submitted.value["proposed_revision"]
        .as_str()
        .ok_or("proposal response omitted proposed revision")?
        .to_owned();
    assert!(!submitted.value["applied"].as_bool().unwrap_or(false));
    daemon.stop().await?;

    let restarted = start(config.clone(), CONTROLLER_TOKEN).await?;
    let proposal_status = restarted
        .client
        .proposal(
            "run-headless-dogfood",
            "proposal-headless-remediation-1",
            &proposed_revision,
        )
        .await?;
    assert!(!proposal_status.approved);
    let decision_boundary = restarted.client.run("run-headless-dogfood").await?.sequence;
    let mut approve_request = request(
        "dogfood-approve-remediation",
        Some(decision_boundary),
        Command::DecideProposal {
            run_id: "run-headless-dogfood".to_owned(),
            proposal_id: "proposal-headless-remediation-1".to_owned(),
            proposal_digest: proposal_digest.clone(),
            proposed_revision: proposed_revision.clone(),
            decision_id: "decision-headless-remediation-1".to_owned(),
            decision: milkdrift_control_protocol::ProposalDecision::Approve,
        },
    );
    approve_request.expected_revision = Some(proposed_revision.clone());
    restarted.client.submit(&approve_request).await?;
    let apply_boundary = restarted.client.run("run-headless-dogfood").await?.sequence;
    let mut apply_request = request(
        "dogfood-apply-remediation",
        Some(apply_boundary),
        Command::ApplyProposal {
            run_id: "run-headless-dogfood".to_owned(),
            proposal_id: "proposal-headless-remediation-1".to_owned(),
            proposal_digest,
            proposed_revision: proposed_revision.clone(),
        },
    );
    apply_request.expected_revision = Some(proposed_revision.clone());
    restarted.client.submit(&apply_request).await?;
    let adopted = restarted.client.run("run-headless-dogfood").await?;
    assert_eq!(
        adopted.revision_id.as_deref(),
        Some(proposed_revision.as_str())
    );
    assert_eq!(adopted.lifecycle, "paused");
    restarted.stop().await?;

    let resumed = start(config, CONTROLLER_TOKEN).await?;
    let signal_boundary = resumed.client.run("run-headless-dogfood").await?.sequence;
    resumed
        .client
        .submit(&request(
            "dogfood-release-approved-remediation",
            Some(signal_boundary),
            Command::SignalRun {
                run_id: "run-headless-dogfood".to_owned(),
                signal_id: "signal-headless-remediation-1".to_owned(),
                signal_type: "sequence.approved".to_owned(),
                correlation: None,
                broadcast: false,
                payload: serde_json::json!({
                    "proposal": "proposal-headless-remediation-1",
                    "revision": proposed_revision
                }),
            },
        ))
        .await?;
    let resume_boundary = resumed.client.run("run-headless-dogfood").await?.sequence;
    resumed
        .client
        .submit(&request(
            "dogfood-resume-remediation",
            Some(resume_boundary),
            Command::ResumeRun {
                run_id: "run-headless-dogfood".to_owned(),
            },
        ))
        .await?;
    let completed = wait_for_run(&resumed.client, "run-headless-dogfood", |state| {
        state.terminal.as_deref() == Some("succeeded")
    })
    .await?;
    assert_eq!(completed.lifecycle, "terminal");
    for node in &completed.nodes {
        if node.node_id.contains("coding")
            || node.node_id.contains("verification")
            || node.node_id.contains("review")
        {
            assert_eq!(
                node.attempt_count, 1,
                "{} executed more than once",
                node.node_id
            );
        }
    }
    for coding_node in [
        "stage-one-coding",
        "stage-two-coding",
        "stage-two-remediation-1-coding",
    ] {
        let attempt =
            attempt_id_for_node(&resumed.client, "run-headless-dogfood", coding_node).await?;
        let inspected = resumed
            .client
            .attempt("run-headless-dogfood", &attempt)
            .await?;
        assert_eq!(inspected.context_access, "authorized");
        assert_eq!(
            inspected.context.ok_or("coding context absent")?.policy["session"],
            "fresh"
        );
    }
    let progress = fs::read_to_string(repository.join("progress.md"))?;
    assert!(progress.contains("First fresh process"));
    assert!(progress.contains("Second fresh process"));
    assert!(progress.contains("Remediation fresh process"));
    resumed.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_auth_startup_readiness_and_authority() -> TestResult {
    let directory = tempfile::tempdir()?;
    let daemon = start(configuration(&directory, 16)?, CONTROLLER_TOKEN).await?;
    let raw = reqwest::Client::new();
    let unauthenticated = raw.get(daemon.endpoint.join("v1/health")?).send().await?;
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);
    let invalid = raw
        .get(daemon.endpoint.join("v1/health")?)
        .bearer_auth("invalid-token")
        .send()
        .await?;
    assert_eq!(invalid.status(), reqwest::StatusCode::UNAUTHORIZED);

    let authority = daemon.client.authority().await?;
    assert_eq!(authority.actor, "human:integration-controller");
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let denied = observer
        .submit(&request(
            "observer-import-denied",
            None,
            Command::ImportBlueprint {
                document: blueprint()?,
            },
        ))
        .await;
    assert!(matches!(
        denied,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_accepts_commands_across_hot_receipt_turnovers_and_replays_cold_after_restart()
-> TestResult {
    let directory = tempfile::tempdir()?;
    let mut document = configuration_document_with_process_profiles(&directory, 16, Vec::new())?;
    document.application_receipts.hot_receipt_bound = 1;
    document.application_receipts.archive_batch_size = 1;
    let config = document.validate(directory.path())?;
    let restart_config = config.clone();
    let daemon = start(config, CONTROLLER_TOKEN).await?;
    let import_request = request(
        "capacity-import",
        None,
        Command::ImportBlueprint {
            document: blueprint()?,
        },
    );
    let imported = daemon.client.submit(&import_request).await?;
    let revision = imported
        .value
        .get("revision_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("capacity import omitted revision")?;
    let start_request = request(
        "capacity-start-remains-replayable",
        None,
        Command::StartRun {
            run_id: "run-capacity-accepted".to_owned(),
            workflow_id: "golden".to_owned(),
            revision_id: revision.to_owned(),
        },
    );
    let accepted = daemon.client.submit(&start_request).await?;
    for ordinal in 0..8 {
        daemon
            .client
            .submit(&request(
                &format!("capacity-validate-{ordinal}"),
                None,
                Command::ValidateBlueprint {
                    document: blueprint()?,
                },
            ))
            .await?;
    }
    let health = daemon.client.health().await?;
    assert!(health.application_receipts.cold_count >= 9);
    assert!(health.application_receipts.hot_count <= 1);
    let before_restart = daemon.client.run("run-capacity-accepted").await?;
    daemon.stop().await?;

    let restarted = start(restart_config, CONTROLLER_TOKEN).await?;
    let replay = restarted.client.submit(&start_request).await?;
    assert!(replay.replayed);
    assert_eq!(replay.resulting_sequence, accepted.resulting_sequence);
    assert_eq!(
        restarted
            .client
            .run("run-capacity-accepted")
            .await?
            .sequence,
        before_restart.sequence
    );
    assert!(restarted.client.submit(&import_request).await?.replayed);
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_read_matrix_and_continuations_fail_closed() -> TestResult {
    let directory = tempfile::tempdir()?;
    let profile = configured_process_profile(&directory)?;
    let document = configuration_document_with_process_profiles(&directory, 16, vec![profile])?;
    let config = document.clone().validate(directory.path())?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let golden_revision = import_blueprint(&daemon.client, "matrix-import-golden").await?;
    let process_import = daemon
        .client
        .submit(&request(
            "matrix-import-process",
            None,
            Command::ImportBlueprint {
                document: process_blueprint()?,
            },
        ))
        .await?;
    let process_revision = process_import
        .value
        .get("revision_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("process import omitted revision")?
        .to_owned();
    daemon
        .client
        .submit(&request(
            "matrix-start-golden",
            None,
            Command::StartRun {
                run_id: "run-matrix".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: golden_revision.clone(),
            },
        ))
        .await?;
    daemon
        .client
        .submit(&request(
            "matrix-start-process",
            None,
            Command::StartRun {
                run_id: "run-matrix-process".to_owned(),
                workflow_id: "daemon-process".to_owned(),
                revision_id: process_revision.clone(),
            },
        ))
        .await?;
    let layout = LayoutDocument {
        schema_version: 1,
        workflow_id: "golden".to_owned(),
        revision_id: golden_revision.clone(),
        generation: 1,
        author: "human:client-placeholder".to_owned(),
        digest: String::new(),
        nodes: BTreeMap::from([(
            "first".to_owned(),
            LayoutPoint {
                x: 1.0,
                y: 2.0,
                width: None,
                height: None,
            },
        )]),
        collapsed_groups: std::collections::BTreeSet::new(),
        annotations: BTreeMap::new(),
        viewport: None,
    }
    .seal()?;
    daemon
        .client
        .submit(&request(
            "matrix-put-layout",
            None,
            Command::PutLayout { layout },
        ))
        .await?;

    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(observer.readiness().await?.ready);
    assert!(matches!(
        observer.health().await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    let visible = observer.capabilities().await?;
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].capability_id, "milkdrift-workflow-control");
    assert!(visible[0].provider_profile.is_none());
    assert_eq!(
        observer
            .revisions(
                Some("golden"),
                &PageRequest {
                    cursor: None,
                    limit: 10,
                },
            )
            .await?
            .items
            .len(),
        1
    );
    assert!(matches!(
        observer.revision(&process_revision).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::NotFound
    ));
    assert!(matches!(
        observer.revisions(
            Some("daemon-process"),
            &PageRequest { cursor: None, limit: 10 },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert_eq!(
        observer.run("run-matrix").await?.workflow_id.as_deref(),
        Some("golden")
    );
    let hidden_run = observer.run("run-matrix-process").await;
    assert!(
        matches!(
            hidden_run,
            Err(ClientError::Api(ref error)) if error.code == ErrorCode::NotFound
        ),
        "out-of-scope exact run was distinguishable from absence: {hidden_run:?}"
    );
    assert!(
        observer
            .timeline(
                "run-matrix",
                &PageRequest {
                    cursor: None,
                    limit: 10,
                },
            )
            .await?
            .items
            .iter()
            .all(|event| event.run_id == "run-matrix")
    );
    assert!(
        observer
            .proposals(
                "run-matrix",
                &PageRequest {
                    cursor: None,
                    limit: 10,
                },
            )
            .await?
            .items
            .is_empty()
    );
    assert!(matches!(
        observer.runs(
            None,
            Some("daemon-process"),
            &PageRequest { cursor: None, limit: 10 },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert!(matches!(
        observer.layout("golden", &golden_revision).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));

    let first_page = daemon
        .client
        .revisions(
            None,
            &PageRequest {
                cursor: None,
                limit: 1,
            },
        )
        .await?;
    let cursor = first_page
        .next_cursor
        .ok_or("expected a second revision page")?;
    assert!(matches!(
        observer.revisions(
            None,
            &PageRequest {
                cursor: Some(cursor.clone()),
                limit: 1,
            },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput
    ));

    let mut health = daemon.client.subscribe("v1/stream/health", None);
    let health_cursor = tokio::time::timeout(Duration::from_secs(3), health.next())
        .await?
        .ok_or("health stream closed before an observation")??
        .cursor;
    drop(health);
    daemon.stop().await?;

    let mut narrowed = document;
    narrowed.actors[0].grant_revision = 2;
    narrowed.actors[0].authority.resources.capability = CapabilityAuthorityScope::deny_all();
    let narrowed = narrowed.validate(directory.path())?;
    let restarted = start(narrowed, CONTROLLER_TOKEN).await?;
    assert!(matches!(
        restarted.client.revisions(
            None,
            &PageRequest {
                cursor: Some(cursor),
                limit: 1,
            },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput
    ));
    let mut reconnect = restarted
        .client
        .subscribe("v1/stream/health", Some(health_cursor));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), reconnect.next())
            .await?
            .ok_or("narrowed stream ended without a typed error")?,
        Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput
    ));
    restarted.stop().await
}
