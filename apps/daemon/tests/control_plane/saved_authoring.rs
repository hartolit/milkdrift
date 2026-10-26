//! Saved sketches and deliberate scope admission cross the actual daemon/control/runtime/store path.
use super::support::*;
use milkdrift_control_protocol::authoring::*;
use milkdrift_control_protocol::tasks::WorkflowReceipt;

async fn empty(client: &ControlClient, key: &str) -> TestResult<WorkflowReceipt> {
    let mut request = create_request(key, false);
    request.tasks.clear();
    request.finish_when_settled = true;
    client.create_workflow(&request).await.map_err(Into::into)
}

fn card(id: &str, generation: u64, blank: bool) -> AuthoringTaskDto {
    let mut definition = task_definition("golden-local-process", "process.execute");
    definition.inputs.clear();
    definition.title = if blank { String::new() } else { id.into() };
    definition.instructions = if blank {
        String::new()
    } else {
        "Copy the connected evidence exactly".into()
    };
    if blank {
        definition.capability.id.clear();
        definition.capability.operation.clear();
    }
    AuthoringTaskDto {
        id: id.into(),
        version: None,
        definition,
        title_edited: false,
        contract_generation: (!blank).then_some(generation),
    }
}

fn wire(id: &str, source: &str, target: &str, output: Option<&str>) -> AuthoringConnectionDto {
    AuthoringConnectionDto {
        id: id.into(),
        source: source.into(),
        target: target.into(),
        input: Some("file".into()),
        output: output.map(str::to_owned),
        kind: AuthoringConnectionKindDto::Data,
        optional: false,
        outcome: false,
    }
}

fn scope(generation: u64, tasks: &[&str]) -> ReviewAuthoringRequest {
    ReviewAuthoringRequest {
        authoring_generation: generation,
        tasks: tasks.iter().map(|id| (*id).into()).collect(),
        changes: Vec::new(),
        use_current: Vec::new(),
    }
}

fn run(key: &str, review: &AuthoringReviewDto) -> RunAuthoringRequest {
    RunAuthoringRequest {
        request_id: key.into(),
        expected_generation: review.expected_generation,
        authoring_generation: review.authoring_generation,
        tasks: review.tasks.clone(),
        changes: review.changes.clone(),
        use_current: review.use_current.clone(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_workflows_sketches_positions_and_exact_subset_run_survive_restart() -> TestResult {
    let directory = TempDir::new()?;
    let profile = captured_file_profile(&directory)?;
    let plan = configuration_with_process_profiles(&directory, 32, vec![profile])?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let first = empty(&daemon.client, "saved-first").await?;
    let second = empty(&daemon.client, "saved-second").await?;
    let session = daemon.client.task_session().await?;
    assert_eq!(session.authoring_bounds.pending_tasks, 200);
    let contract = session.authoring.first().ok_or("contract missing")?;
    let capture = daemon
        .client
        .upload_task_input(&TaskUploadRequest {
            request_id: "saved-source".into(),
            name: "evidence.txt".into(),
            media_type: "text/plain".into(),
            text: "Captured evidence crosses two real tasks".into(),
        })
        .await?;
    let document = AuthoringDocumentDto {
        tasks: vec![
            card("producer", contract.generation, false),
            card("consumer", contract.generation, false),
            card("idea", 0, true),
        ],
        sources: vec![AuthoringSourceDto {
            id: "source".into(),
            label: "evidence.txt".into(),
            value: Some(InputValue::Uploaded {
                input_id: capture.input_id,
            }),
            shared_name: None,
        }],
        connections: vec![
            wire("source-wire", "source", "producer", None),
            wire("result-wire", "producer", "consumer", Some("report")),
        ],
        usual_model: Some(AuthoringModelDto {
            capability: contract.capability.clone(),
            generation: contract.generation,
        }),
        ..AuthoringDocumentDto::default()
    };
    let save = CheckpointAuthoringRequest {
        expected_generation: 0,
        document: document.clone(),
    };
    let saved = daemon
        .client
        .checkpoint_workflow_authoring(&first.workflow_id, &save)
        .await?;
    assert_eq!(saved.document, document);
    assert!(
        matches!(daemon.client.checkpoint_workflow_authoring(&first.workflow_id, &save).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    let other = AuthoringDocumentDto {
        tasks: vec![card("different-idea", 0, true)],
        ..AuthoringDocumentDto::default()
    };
    daemon
        .client
        .checkpoint_workflow_authoring(
            &second.workflow_id,
            &CheckpointAuthoringRequest {
                expected_generation: 0,
                document: other.clone(),
            },
        )
        .await?;
    let positions = PutPositionsRequest {
        expected_generation: 0,
        positions: BTreeMap::from([
            ("producer".into(), CanvasPositionDto { x: 19.0, y: 45.0 }),
            (
                "idea".into(),
                CanvasPositionDto {
                    x: 700.0,
                    y: -200.0,
                },
            ),
        ]),
    };
    daemon
        .client
        .checkpoint_workflow_positions(&first.workflow_id, &positions)
        .await?;
    let observed = daemon.client.workflow(&first.workflow_id).await?;
    assert_eq!(observed.task_count, 0);
    assert_eq!(observed.admitted_tasks, 0);
    assert_ne!(observed.state, "completed");
    assert_eq!(observed.generation, first.generation);
    daemon.stop().await?;

    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    assert_eq!(
        daemon.client.workflow_authoring(&first.workflow_id).await?,
        saved
    );
    assert_eq!(
        daemon
            .client
            .workflow_authoring(&second.workflow_id)
            .await?
            .document,
        other
    );
    assert_eq!(
        daemon
            .client
            .workflow_positions(&first.workflow_id)
            .await?
            .positions,
        positions.positions
    );
    let review = daemon
        .client
        .review_workflow_authoring(&first.workflow_id, &scope(saved.generation, &["consumer"]))
        .await?;
    assert!(review.issues.is_empty(), "{:?}", review.issues);
    assert_eq!(review.tasks, vec!["consumer", "producer"]);
    let mut unreviewed = run("unreviewed", &review);
    unreviewed.tasks = vec!["consumer".into()];
    assert!(
        matches!(daemon.client.run_workflow_authoring(&first.workflow_id, &unreviewed).await,
        Err(ClientError::Api(error)) if error.details.get("authoring_issues").is_some_and(|issues| issues.contains("missing_predecessor")))
    );
    assert_eq!(
        daemon.client.workflow(&first.workflow_id).await?.task_count,
        0
    );
    let request = run("reviewed-chain", &review);
    let receipt = daemon
        .client
        .run_workflow_authoring(&first.workflow_id, &request)
        .await?;
    assert_eq!(receipt.aliases.len(), 2);
    let after = daemon.client.workflow_authoring(&first.workflow_id).await?;
    assert_eq!(after.document.tasks.len(), 1);
    assert_eq!(
        after.document.tasks.first().ok_or("idea missing")?.id,
        "idea"
    );
    assert_eq!(after.document.accepted, receipt.aliases);
    assert_eq!(after.document.connections, document.connections);
    let consumer = receipt.aliases.get("consumer").ok_or("consumer missing")?;
    let completed = wait_task(
        &daemon.client,
        &first.workflow_id,
        consumer,
        TaskStateDto::Completed,
    )
    .await?;
    assert_eq!(
        completed.result.ok_or("result missing")?.text,
        "Captured evidence crosses two real tasks"
    );
    let mut newer = after.document.clone();
    newer
        .tasks
        .first_mut()
        .ok_or("idea missing")?
        .definition
        .instructions = "A later unsubmitted idea".into();
    daemon
        .client
        .checkpoint_workflow_authoring(
            &first.workflow_id,
            &CheckpointAuthoringRequest {
                expected_generation: after.generation,
                document: newer.clone(),
            },
        )
        .await?;
    assert_eq!(
        daemon
            .client
            .run_workflow_authoring(&first.workflow_id, &request)
            .await?,
        receipt
    );
    assert_eq!(
        daemon
            .client
            .workflow_authoring(&first.workflow_id)
            .await?
            .document,
        newer
    );
    let mut changed = request.clone();
    changed.tasks.push("idea".into());
    assert!(
        matches!(daemon.client.run_workflow_authoring(&first.workflow_id, &changed).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    cli_ok(
        &daemon,
        &directory,
        "saved-authoring-cli",
        &["workflow", "authoring", &first.workflow_id],
    )?;
    daemon.stop().await?;
    let daemon = start(plan, CONTROLLER_TOKEN).await?;
    assert_eq!(
        daemon
            .client
            .run_workflow_authoring(&first.workflow_id, &request)
            .await?,
        receipt
    );
    assert_eq!(
        daemon
            .client
            .workflow_authoring(&first.workflow_id)
            .await?
            .document,
        newer
    );
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unfinished_wires_cycles_and_scope_denial_preserve_saved_work() -> TestResult {
    let directory = TempDir::new()?;
    let profile = captured_file_profile(&directory)?;
    let daemon = start(
        configuration_with_process_profiles(&directory, 32, vec![profile])?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let workflow = empty(&daemon.client, "repair-sketch").await?.workflow_id;
    let session = daemon.client.task_session().await?;
    let contract = session.authoring.first().ok_or("contract missing")?;
    let mut unfinished = wire("unfinished", "one", "two", None);
    unfinished.input = None;
    let document = AuthoringDocumentDto {
        tasks: vec![card("one", 0, true), card("two", 0, true)],
        connections: vec![unfinished],
        ..AuthoringDocumentDto::default()
    };
    let saved = daemon
        .client
        .checkpoint_workflow_authoring(
            &workflow,
            &CheckpointAuthoringRequest {
                expected_generation: 0,
                document,
            },
        )
        .await?;
    let review = daemon
        .client
        .review_workflow_authoring(&workflow, &scope(saved.generation, &["two"]))
        .await?;
    assert!(review.issues.iter().any(|issue| issue.code == "incomplete"));
    assert_eq!(review.tasks, vec!["one", "two"]);
    let mut cyclic = saved.document.clone();
    cyclic.tasks = vec![
        card("one", contract.generation, false),
        card("two", contract.generation, false),
    ];
    cyclic.connections = vec![
        wire("one-two", "one", "two", Some("report")),
        wire("two-one", "two", "one", Some("report")),
    ];
    let saved = daemon
        .client
        .checkpoint_workflow_authoring(
            &workflow,
            &CheckpointAuthoringRequest {
                expected_generation: saved.generation,
                document: cyclic,
            },
        )
        .await?;
    let review = daemon
        .client
        .review_workflow_authoring(&workflow, &scope(saved.generation, &["one"]))
        .await?;
    assert!(
        review
            .issues
            .iter()
            .any(|issue| issue.code == "topology" && issue.message.contains("cycle"))
    );
    assert_eq!(daemon.client.workflow(&workflow).await?.admitted_tasks, 0);
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(matches!(observer.workflow_authoring(&workflow).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized));
    assert!(matches!(observer.checkpoint_workflow_authoring(&workflow,
        &CheckpointAuthoringRequest { expected_generation: saved.generation, document: AuthoringDocumentDto::default() }).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized));
    assert_eq!(daemon.client.workflow_authoring(&workflow).await?, saved);
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_alias_requires_explicit_predecessor_choice_and_preserves_started_candidate()
-> TestResult {
    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let daemon = start(
        configuration_with_process_profiles(&directory, 32, vec![profile])?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let workflow = empty(&daemon.client, "alias-choice").await?.workflow_id;
    let observed = daemon.client.workflow(&workflow).await?;
    daemon
        .client
        .change_workflow(
            &workflow,
            &ChangeWorkflowRequest {
                request_id: "hold-candidates".into(),
                expected_generation: observed.generation,
                changes: vec![TaskChangeDto::Wait {
                    question: "Review the future plan".into(),
                }],
            },
        )
        .await?;
    let session = daemon.client.task_session().await?;
    let contract = session.authoring.first().ok_or("contract missing")?;
    let mut connection = wire("ordered", "producer", "consumer", None);
    connection.kind = AuthoringConnectionKindDto::AfterSuccess;
    connection.input = None;
    let document = AuthoringDocumentDto {
        tasks: vec![
            card("producer", contract.generation, false),
            card("consumer", contract.generation, false),
        ],
        connections: vec![connection],
        ..AuthoringDocumentDto::default()
    };
    let saved = daemon
        .client
        .checkpoint_workflow_authoring(
            &workflow,
            &CheckpointAuthoringRequest {
                expected_generation: 0,
                document,
            },
        )
        .await?;
    let review = daemon
        .client
        .review_workflow_authoring(&workflow, &scope(saved.generation, &["producer"]))
        .await?;
    assert!(review.issues.is_empty(), "{:?}", review.issues);
    let receipt = daemon
        .client
        .run_workflow_authoring(&workflow, &run("admit-producer", &review))
        .await?;
    let producer = receipt
        .aliases
        .get("producer")
        .ok_or("producer alias missing")?;
    let saved = daemon.client.workflow_authoring(&workflow).await?;
    let accepted = daemon.client.task(&workflow, producer).await?;
    let mut document = saved.document.clone();
    let mut candidate = card(producer, contract.generation, false);
    candidate.version = Some(accepted.version);
    candidate.definition.instructions = "A deliberately different future instruction".into();
    document.tasks.push(candidate);
    let saved = daemon
        .client
        .checkpoint_workflow_authoring(
            &workflow,
            &CheckpointAuthoringRequest {
                expected_generation: saved.generation,
                document,
            },
        )
        .await?;
    let review = daemon
        .client
        .review_workflow_authoring(&workflow, &scope(saved.generation, &["consumer"]))
        .await?;
    assert!(
        review
            .issues
            .iter()
            .any(|issue| issue.code == "predecessor_choice"
                && issue.task_id.as_ref() == Some(producer)
                && issue.connection_id.as_deref() == Some("ordered"))
    );
    let mut current = scope(saved.generation, &["consumer"]);
    current.use_current.push(producer.clone());
    let current = daemon
        .client
        .review_workflow_authoring(&workflow, &current)
        .await?;
    assert!(current.issues.is_empty(), "{:?}", current.issues);
    assert_eq!(current.tasks, vec!["consumer"]);
    let review = daemon
        .client
        .review_workflow_authoring(&workflow, &scope(saved.generation, &["consumer", producer]))
        .await?;
    assert!(review.issues.is_empty(), "{:?}", review.issues);
    // Neither accepted aliases nor task IDs can be reused as new cards or source identities.
    for source_collision in [false, true] {
        let mut collision = saved.document.clone();
        if source_collision {
            collision.sources.push(AuthoringSourceDto {
                id: "producer".into(),
                label: "Retarget".into(),
                value: None,
                shared_name: None,
            });
        } else {
            collision
                .tasks
                .push(card("producer", contract.generation, false));
        }
        assert!(
            daemon
                .client
                .checkpoint_workflow_authoring(
                    &workflow,
                    &CheckpointAuthoringRequest {
                        expected_generation: saved.generation,
                        document: collision
                    }
                )
                .await
                .is_err()
        );
    }
    daemon
        .client
        .change_workflow(
            &workflow,
            &ChangeWorkflowRequest {
                request_id: "start-reviewed-predecessor".into(),
                expected_generation: review.expected_generation,
                changes: vec![TaskChangeDto::Continue],
            },
        )
        .await?;
    wait_task(&daemon.client, &workflow, producer, TaskStateDto::Completed).await?;
    assert!(
        matches!(daemon.client.run_workflow_authoring(&workflow, &run("raced-apply", &review)).await,
        Err(ClientError::Api(error)) if error.details.get("authoring_issues").is_some_and(|issues| issues.contains("start_conflict")))
    );
    assert_eq!(daemon.client.workflow_authoring(&workflow).await?, saved);
    assert_eq!(daemon.client.workflow(&workflow).await?.admitted_tasks, 1);
    daemon.stop().await
}
