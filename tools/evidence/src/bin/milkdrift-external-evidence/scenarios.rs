//! Drives ordinary daemon tasks; client disposal and restart never authorize another entry.
use super::{
    AgentProfile, Arc, ArtifactEvidence, AtomicUsize, BTreeMap, ControlClient, DaemonLaunch,
    Duration, HarnessResult, ModelResponseDocument, Mutex, Ordering, Path, RestartEvidence,
    ScenarioEvidence, Value, client_error, json, workflows,
};
use milkdrift_control_protocol::tasks::{
    ChangeWorkflowRequest, InputBinding, InputValue, TaskChangeDto, TaskReadDto, TaskStateDto,
    WorkflowReadDto,
};

#[expect(
    clippy::too_many_arguments,
    reason = "Scenario source provenance and the deadline have separate owners."
)]
pub(super) async fn run_process_scenario(
    config: &DaemonLaunch,
    token: &str,
    agent: &AgentProfile,
    initial_commit: &str,
    initial_tree: &str,
    repository: &Path,
    fixture: bool,
    timeout: Duration,
) -> HarnessResult<ScenarioEvidence> {
    let request = workflows::process_request(&agent.capability, repository);
    let (mut daemon, client) = config.start(token).await.map_err(|e| e.to_string())?;
    let receipt = client
        .create_workflow(&request)
        .await
        .map_err(client_error)?;
    if client
        .create_workflow(&request)
        .await
        .map_err(client_error)?
        != receipt
    {
        return Err("exact process create replay changed the receipt".into());
    }
    let workflow = &receipt.workflow_id;
    let coding = alias(&receipt.aliases, "coding")?;
    let weak = alias(&receipt.aliases, "verify")?;
    let review = alias(&receipt.aliases, "review")?;
    let before = wait_for_workflow(&client, workflow, timeout, |state| {
        state
            .tasks
            .iter()
            .all(|task| matches!(task.state, TaskStateDto::Completed | TaskStateDto::Failed))
    })
    .await?;
    let original = client.task(workflow, &coding).await.map_err(client_error)?;
    let controlled = client.task(workflow, &weak).await.map_err(client_error)?;
    let reviewed = client.task(workflow, &review).await.map_err(client_error)?;
    require_state(&original, TaskStateDto::Completed)?;
    require_state(&controlled, TaskStateDto::Failed)?;
    require_state(&reviewed, TaskStateDto::Completed)?;
    let controlled_artifacts =
        read_artifacts(&client, &controlled, "controlled_verification").await?;
    if !controlled_artifacts.iter().any(|(_, bytes)| {
        serde_json::from_slice::<Value>(bytes)
            .is_ok_and(|value| value.pointer("/checks/rust.tests") == Some(&json!(false)))
    }) {
        return Err("controlled verification did not retain its failed check".into());
    }
    daemon.terminate().map_err(|e| e.to_string())?;
    let (mut daemon, client) = config.start(token).await.map_err(|e| e.to_string())?;
    let recovered = client.workflow(workflow).await.map_err(client_error)?;
    if recovered != before
        || client.task(workflow, &weak).await.map_err(client_error)? != controlled
    {
        return Err("settled review boundary changed across restart".into());
    }
    let prompt = "In a fresh process, inspect the current accepted repository state, make only any remaining bounded repair needed for the Rust test, run it, and do not commit.";
    let mut correction = workflows::definition(
        &agent.capability,
        "Corrective coding",
        prompt,
        Some(repository),
    );
    correction
        .inputs
        .insert("prompt".into(), workflows::text(prompt));
    correction.dependencies.push(review.clone());
    correction.inputs.insert(
        "review".into(),
        InputBinding::TaskOutput {
            task: review.clone(),
            output: "review".into(),
            optional: false,
        },
    );
    workflows::output(&mut correction, "diff", None, 1_048_576);
    let mut verification = workflows::definition(
        "evidence-verifier-good",
        "Corrective verification",
        "Run the real independent Rust checks",
        Some(repository),
    );
    verification.dependencies.push("corrective-coding".into());
    verification.corrective_of = Some(weak.clone());
    workflows::output(
        &mut verification,
        "verification_result",
        Some("application/json"),
        262_144,
    );
    workflows::output(
        &mut verification,
        "verification_logs",
        Some("text/plain"),
        1_048_576,
    );
    workflows::output(
        &mut verification,
        "verification_acceptance",
        Some("text/plain"),
        4_096,
    );
    let change = ChangeWorkflowRequest {
        request_id: "external-process-correction".into(),
        expected_generation: recovered.generation,
        changes: vec![
            TaskChangeDto::Add {
                alias: "corrective-coding".into(),
                definition: correction,
            },
            TaskChangeDto::Add {
                alias: "corrective-verification".into(),
                definition: verification,
            },
        ],
    };
    let added = client
        .change_workflow(workflow, &change)
        .await
        .map_err(client_error)?;
    if client
        .change_workflow(workflow, &change)
        .await
        .map_err(client_error)?
        != added
    {
        return Err("exact corrective batch replay changed the receipt".into());
    }
    let corrective = alias(&added.aliases, "corrective-coding")?;
    let good = alias(&added.aliases, "corrective-verification")?;
    let settled = wait_for_workflow(&client, workflow, timeout, |state| {
        state.tasks.iter().any(|task| {
            task.id == good
                && matches!(
                    task.state,
                    TaskStateDto::Completed | TaskStateDto::Failed | TaskStateDto::Uncertain
                )
        })
    })
    .await?;
    let corrective_read = client
        .task(workflow, &corrective)
        .await
        .map_err(client_error)?;
    let good_read = client.task(workflow, &good).await.map_err(client_error)?;
    require_state(&corrective_read, TaskStateDto::Completed)?;
    require_state(&good_read, TaskStateDto::Completed)?;
    let good_artifacts = read_artifacts(&client, &good_read, "final_verification").await?;
    if !good_artifacts.iter().any(|(_, bytes)| {
        serde_json::from_slice::<Value>(bytes).is_ok_and(|value| {
            value.pointer("/checks/rust.tests") == Some(&json!(true))
                && value.pointer("/checks/git.diff") == Some(&json!(true))
                && value.get("checkpoint") == value.get("checked_checkpoint")
        })
    }) {
        return Err(
            "independent verification omitted passing checks at an unchanged checkpoint".into(),
        );
    }
    client
        .change_workflow(
            workflow,
            &ChangeWorkflowRequest {
                request_id: "external-process-adopt-and-finish".into(),
                expected_generation: settled.generation,
                changes: vec![
                    TaskChangeDto::AdoptCorrection {
                        task: weak.clone(),
                        correction: good.clone(),
                        redirects: vec![],
                    },
                    TaskChangeDto::Finish,
                ],
            },
        )
        .await
        .map_err(client_error)?;
    let completed = wait_for_workflow(&client, workflow, timeout, |state| {
        state.state == "completed"
    })
    .await?;
    if client.task(workflow, &weak).await.map_err(client_error)? != controlled {
        return Err("adoption rewrote the original failed task".into());
    }
    let mut artifacts = Vec::new();
    let mut process_tasks = Vec::new();
    for (role, task) in [
        ("initial_coding", &original),
        ("controlled_verification", &controlled),
        ("independent_review", &reviewed),
        ("remediation_coding", &corrective_read),
        ("final_verification", &good_read),
    ] {
        let frozen = task
            .frozen
            .as_ref()
            .ok_or_else(|| format!("{role} omitted its frozen start"))?;
        if !frozen.entered || task.started_at.is_none() || task.ended_at.is_none() {
            return Err(format!("{role} has no durable entry and settlement"));
        }
        process_tasks.push(json!({"role":role,"task_id":task.id,"terminal":task.state,"uncertain":false,"capability_id":task.definition.capability.id,"capability_generation":frozen.capability_generation,"actor_id":frozen.actor_id,"grant_id":frozen.grant_id}));
        artifacts.extend(
            read_artifacts(&client, task, role)
                .await?
                .into_iter()
                .map(|(metadata, _)| metadata),
        );
    }
    let final_commit = workflows::git(repository, &["rev-parse", "HEAD"])?;
    let final_tree = workflows::git(repository, &["rev-parse", "HEAD^{tree}"])?;
    let diff = workflows::git(repository, &["diff", "--binary", "HEAD"])?;
    let profile_bytes = std::fs::read(&agent.path).map_err(|e| e.to_string())?;
    daemon.terminate().map_err(|e| e.to_string())?;
    let evidence = ScenarioEvidence {
        qualifying: !fixture,
        outcome: "succeeded".into(),
        profile: json!({"capability":agent.capability,"executable_content_digest":agent.content_digest,"executable_size_bytes":agent.size_bytes,"version_output":agent.version_output,"profile_digest":format!("b3_{}",blake3::hash(&profile_bytes)),"server_settings":{"status":"unknown","scope":"endpoint_inside_coding_agent"},"fixture_rejected_for_qualification":fixture}),
        commands: vec![
            request.request_id,
            change.request_id,
            "external-process-adopt-and-finish".into(),
        ],
        workflows: vec![workflow.clone()],
        tasks: vec![coding, weak, review, corrective, good.clone()],
        corrections: vec![good],
        artifacts,
        restart_boundaries: vec![RestartEvidence {
            boundary: "settled failed verification and independent review".into(),
            generation_before: before.generation,
            generation_after: recovered.generation,
            recovered_state: "failed original retained; explicit correction required".into(),
            duplicate_tasks: false,
        }],
        facts: json!({"repository_initial_commit":initial_commit,"repository_initial_tree":initial_tree,"repository_final_commit":final_commit,"repository_final_tree":final_tree,"dirty_diff_digest":format!("b3_{}",blake3::hash(diff.as_bytes())),"dirty_diff_bytes":diff.len(),"controlled_failure":"independent verifier process returned a failed required Rust check","fresh_agent_tasks":2,"distinct_process_tasks":5,"process_tasks":process_tasks,"terminal_generation":completed.generation,"wait_timeout_secs":timeout.as_secs(),"original_failure_preserved":true}),
        failure_reason: None,
    };
    evidence.validate_process_semantics()?;
    Ok(evidence)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Profile provenance, fixture observations, and deadline have separate owners."
)]
pub(super) async fn run_model_scenario(
    config: &DaemonLaunch,
    token: &str,
    model_capability: &str,
    profile: &workflows::ModelProfileFacts,
    repository: &Path,
    fixture_requests: Option<Arc<AtomicUsize>>,
    fixture_lines: Option<Arc<Mutex<Vec<String>>>>,
    fixture: bool,
    timeout: Duration,
) -> HarnessResult<ScenarioEvidence> {
    let request = workflows::model_request(model_capability, repository);
    let (mut daemon, client) = config.start(token).await.map_err(|e| e.to_string())?;
    let receipt = client
        .create_workflow(&request)
        .await
        .map_err(client_error)?;
    let workflow = &receipt.workflow_id;
    let model = alias(&receipt.aliases, "model")?;
    let before = wait_for_workflow(&client, workflow, timeout, |state| {
        state
            .tasks
            .iter()
            .filter(|task| task.id != model)
            .all(|task| task.state == TaskStateDto::Completed)
            && state.tasks.iter().any(|task| {
                task.id == model && task.state == TaskStateDto::Queued && task.reason.is_some()
            })
    })
    .await?;
    if fixture_requests
        .as_ref()
        .is_some_and(|count| count.load(Ordering::SeqCst) != 0)
    {
        return Err("model endpoint entered before required release input".into());
    }
    daemon.terminate().map_err(|e| e.to_string())?;
    let (mut daemon, client) = config.start(token).await.map_err(|e| e.to_string())?;
    let recovered = client.workflow(workflow).await.map_err(client_error)?;
    if recovered != before {
        return Err("model pre-entry wait changed across restart".into());
    }
    let release = ChangeWorkflowRequest {
        request_id: "external-model-release".into(),
        expected_generation: recovered.generation,
        changes: vec![TaskChangeDto::BindInput {
            name: "release".into(),
            value: InputValue::Text {
                text: "approved".into(),
            },
        }],
    };
    client
        .change_workflow(workflow, &release)
        .await
        .map_err(client_error)?;
    let _completed = wait_for_workflow(&client, workflow, timeout, |state| {
        state.tasks.iter().any(|task| {
            task.id == model
                && matches!(
                    task.state,
                    TaskStateDto::Completed | TaskStateDto::Failed | TaskStateDto::Uncertain
                )
        })
    })
    .await?;
    let task = client.task(workflow, &model).await.map_err(client_error)?;
    require_state(&task, TaskStateDto::Completed)?;
    let frozen = task.frozen.as_ref().ok_or("model frozen start absent")?;
    let selected = frozen
        .inputs
        .iter()
        .filter(|(name, _)| name.as_str() != "release")
        .collect::<BTreeMap<_, _>>();
    if selected.len() != 2
        || !selected.contains_key(&"evidence-a".to_owned())
        || !selected.contains_key(&"evidence-b".to_owned())
        || selected
            .values()
            .any(|input| input.artifact_id.is_none() || input.digest.is_none())
        || !frozen.entered
    {
        return Err("model frozen start omitted exact selected evidence".into());
    }
    let retained = read_artifacts(&client, &task, "model").await?;
    let response = retained
        .iter()
        .find_map(|(_, bytes)| ModelResponseDocument::from_json(bytes).ok())
        .ok_or("canonical model response artifact absent")?;
    if response.body().finish_reason() != milkdrift_model::FinishReason::Stop {
        return Err(format!(
            "provider response finish_reason={:?}; truncated or incomplete output cannot qualify",
            response.body().finish_reason()
        ));
    }
    if response.body().text().trim() != "MILKDRIFT_EVIDENCE_OK" {
        return Err("model response did not satisfy the bounded parseable contract".into());
    }
    let usage = response.body().usage();
    if usage.input_units.is_none() || usage.output_units.is_none() {
        return Err("model response omitted provider usage facts".into());
    }
    let provider = retained
        .iter()
        .find(|(_, bytes)| {
            serde_json::from_slice::<Value>(bytes).is_ok_and(|value| {
                value.as_object().is_some_and(|object| {
                    object.values().any(|item| {
                        item.get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.is_empty())
                            && item
                                .get("model")
                                .and_then(Value::as_str)
                                .is_some_and(|s| !s.is_empty())
                    })
                })
            })
        })
        .ok_or("provider metadata omitted response identity/model")?;
    if fixture_requests
        .as_ref()
        .is_some_and(|count| count.load(Ordering::SeqCst) != 1)
        || fixture_lines.as_ref().is_some_and(|lines| {
            lines.lock().map_or(true, |lines| {
                lines.as_slice() != ["POST /v1/chat/completions HTTP/1.1"]
            })
        })
    {
        return Err("fixture observed missing/duplicate request or wrong target".into());
    }
    let selected_digest = format!(
        "b3_{}",
        blake3::hash(&serde_json::to_vec(&selected).map_err(|e| e.to_string())?)
    );
    let facts = json!({"selected_inputs_digest":selected_digest,"selected_count":2,"unselected_count":1,"model_output_units":4096,"streaming":false,"wait_timeout_secs":timeout.as_secs(),"finish_reason":"stop","usage":{"input_units":usage.input_units,"output_units":usage.output_units,"cached_input_units":usage.cached_input_units,"cost_micros":usage.cost_micros,"currency":usage.currency},"provider_metadata_artifact_digest":provider.0.digest,"terminal":"completed","uncertain":false,"task_id":task.id,"capability_generation":frozen.capability_generation,"actor_id":frozen.actor_id,"grant_id":frozen.grant_id});
    daemon.terminate().map_err(|e| e.to_string())?;
    let evidence = ScenarioEvidence {
        qualifying: !fixture,
        outcome: "succeeded".into(),
        profile: json!({"profile_digest":profile.digest,"profile_revision":profile.revision,"provider_protocol":profile.protocol,"model_alias":profile.model_alias,"endpoint_origin":profile.endpoint_origin,"request_mode":"ordinary fresh task","model_output_units":4096,"streaming":false,"fixture_rejected_for_qualification":fixture}),
        commands: vec![request.request_id, release.request_id],
        workflows: vec![workflow.clone()],
        tasks: receipt.aliases.values().cloned().collect(),
        corrections: vec![],
        artifacts: retained.into_iter().map(|(artifact, _)| artifact).collect(),
        restart_boundaries: vec![RestartEvidence {
            boundary: "missing required release input before model entry".into(),
            generation_before: before.generation,
            generation_after: recovered.generation,
            recovered_state: "queued without external entry".into(),
            duplicate_tasks: false,
        }],
        facts,
        failure_reason: None,
    };
    evidence.validate_model_semantics()?;
    Ok(evidence)
}
fn alias(aliases: &BTreeMap<String, String>, name: &str) -> HarnessResult<String> {
    aliases
        .get(name)
        .cloned()
        .ok_or_else(|| format!("receipt omitted task alias {name}"))
}
fn require_state(task: &TaskReadDto, state: TaskStateDto) -> HarnessResult {
    if task.state != state {
        return Err(format!(
            "task {} reached {:?}, expected {:?}; reason={}",
            task.id,
            task.state,
            state,
            task.reason.as_deref().unwrap_or("none")
        ));
    }
    Ok(())
}
async fn read_artifacts(
    client: &ControlClient,
    task: &TaskReadDto,
    role: &str,
) -> HarnessResult<Vec<(ArtifactEvidence, Vec<u8>)>> {
    let result = task
        .result
        .as_ref()
        .ok_or_else(|| format!("{role} retained no result"))?;
    if result.artifacts.len() > 32 {
        return Err("scenario artifact count exceeded bound".into());
    }
    let mut artifacts = Vec::new();
    for id in &result.artifacts {
        let metadata = client.artifact_metadata(id).await.map_err(client_error)?;
        if metadata.size > 1_048_576 {
            return Err("scenario artifact exceeds one MiB evidence bound".into());
        }
        let mut bytes = Vec::new();
        let mut start = 0;
        while start < metadata.size {
            let end = start
                .saturating_add(65535)
                .min(metadata.size.saturating_sub(1));
            let chunk = client
                .artifact_range(id, start, end)
                .await
                .map_err(client_error)?;
            bytes.extend(chunk.bytes);
            start = end.saturating_add(1);
        }
        if bytes.len() as u64 != metadata.size
            || blake3::hash(&bytes).to_hex().as_str() != metadata.digest
        {
            return Err("retained artifact failed complete size/digest verification".into());
        }
        artifacts.push((
            ArtifactEvidence {
                artifact_id: id.clone(),
                digest: metadata.digest,
                size: metadata.size,
                content_type: metadata.content_type,
                role: format!(
                    "{role}:{}",
                    metadata
                        .disposition_name
                        .unwrap_or_else(|| "retained-output".into())
                ),
            },
            bytes,
        ));
    }
    Ok(artifacts)
}
async fn wait_for_workflow(
    client: &ControlClient,
    workflow: &str,
    timeout: Duration,
    predicate: impl Fn(&WorkflowReadDto) -> bool,
) -> HarnessResult<WorkflowReadDto> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last = String::new();
    loop {
        let Ok(state) = tokio::time::timeout_at(deadline, client.workflow(workflow)).await else {
            break;
        };
        let state = state.map_err(client_error)?;
        if state.next_task_cursor.is_some() {
            return Err("scenario exceeded its bounded single task page".into());
        }
        if predicate(&state) {
            return Ok(state);
        }
        last = format!("{} tasks, state {}", state.task_count, state.state);
        if tokio::time::timeout_at(deadline, tokio::time::sleep(Duration::from_millis(25)))
            .await
            .is_err()
        {
            break;
        }
    }
    Err(format!(
        "workflow {workflow} did not reach the expected state within {} seconds; last={last}",
        timeout.as_secs()
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use milkdrift_control_client::{BearerCredential, ClientConfig};
    #[tokio::test]
    async fn workflow_deadline_includes_a_stalled_daemon_read()
    -> Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = url::Url::parse(&format!("http://{}/", listener.local_addr()?))?;
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await?;
            std::future::pending::<std::io::Result<()>>().await
        });
        let mut config = ClientConfig::new(endpoint);
        config.request_timeout = Duration::from_secs(10);
        config.safe_query_retries = 0;
        let client = ControlClient::new(config, BearerCredential::new("deadline-test")?)?;
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_workflow(&client, "test-workflow", Duration::from_millis(50), |_| {
                true
            }),
        )
        .await;
        server.abort();
        let Err(error) = result? else {
            return Err("stalled read cannot establish completion".into());
        };
        assert!(error.contains("did not reach the expected state"));
        Ok(())
    }
}
