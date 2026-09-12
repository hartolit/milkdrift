//! Drives actual daemon/client operations and gathers the process and model observations.
//!
//! Restart checkpoints first reach a settled wait, then kill and reap the daemon. Their recovery
//! evidence does not establish graceful signal handling or interruption during external entry.

use super::{
    AgentProfile, Arc, ArtifactEvidence, AtomicUsize, BTreeSet, CapabilityId, Command,
    CommandRequest, ControlClient, DaemonLaunch, Duration, HarnessResult, MODEL_RUN,
    MODEL_WORKFLOW, ModelResponseDocument, Mutex, Ordering, PROCESS_RUN, PROCESS_WORKFLOW, Path,
    PromptSource, ProtocolVersion, RemediationProposalSpec, RestartEvidence, ScenarioEvidence,
    Value, build_remediation_proposal, client_error, decode_json, json, workflows,
};

#[allow(clippy::too_many_arguments)] // Scenario inputs keep source provenance and the wait bound explicit.
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
    let sequence = workflows::process_sequence(&agent.capability, &agent.output_names)?;
    let sequence_value = serde_json::to_value(&sequence).map_err(|error| error.to_string())?;
    let (mut daemon, client) = config
        .start(token)
        .await
        .map_err(|error| error.to_string())?;
    client.readiness().await.map_err(client_error)?;
    let imported = client
        .submit(&request(
            "external-process-import",
            None,
            Command::ImportPromptSequence {
                document: sequence_value,
            },
        ))
        .await
        .map_err(client_error)?;
    let revision = json_string(&imported.value, "revision_id")?;
    client
        .submit(&request(
            "external-process-start",
            None,
            Command::StartRun {
                run_id: PROCESS_RUN.to_owned(),
                workflow_id: PROCESS_WORKFLOW.to_owned(),
                revision_id: revision.clone(),
            },
        ))
        .await
        .map_err(client_error)?;
    let before_restart = wait_for_run(&client, PROCESS_RUN, timeout, |state| {
        [
            "stage-repair-coding",
            "stage-repair-verification",
            "stage-repair-review",
            "stage-repair-approval",
        ]
        .iter()
        .all(|expected| {
            state.nodes.iter().any(|node| {
                node.node_id == *expected
                    && (*expected == "stage-repair-approval" || node.latest_attempt_id.is_some())
            })
        })
    })
    .await?;
    let original_agent_attempt = attempt_for_node(&before_restart, "stage-repair-coding")?;
    let weak_attempt = attempt_for_node(&before_restart, "stage-repair-verification")?;
    let reviewer_attempt = attempt_for_node(&before_restart, "stage-repair-review")?;
    daemon.terminate().map_err(|error| error.to_string())?;

    let (mut daemon, client) = config
        .start(token)
        .await
        .map_err(|error| error.to_string())?;
    let recovered = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?;
    if recovered.sequence != before_restart.sequence
        || !recovered
            .nodes
            .iter()
            .any(|node| node.node_id == "stage-repair-approval")
    {
        return Err("process run did not recover the durable review boundary".to_owned());
    }
    client
        .submit(&request(
            "external-process-pause",
            Some(recovered.sequence),
            Command::PauseRun {
                run_id: PROCESS_RUN.to_owned(),
            },
        ))
        .await
        .map_err(client_error)?;
    let paused = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?;
    let revision_read = client
        .revision(&revision)
        .await
        .map_err(|error| error.to_string())?;
    let base_value = revision_read
        .document
        .as_ref()
        .ok_or_else(|| "process base revision document is absent".to_owned())?;
    let mut good_verification = sequence.sequence().stages[0].verification.clone();
    good_verification.profile.capability =
        CapabilityId::new("evidence-verifier-good").map_err(|error| error.to_string())?;
    let proposal = build_remediation_proposal(
        &sequence,
        &serde_json::to_vec(base_value).map_err(|error| error.to_string())?,
        RemediationProposalSpec {
            run: PROCESS_RUN.to_owned(),
            observed_sequence: paused.sequence,
            proposal: "proposal-external-remediation-1".to_owned(),
            proposer: "human:external-process".to_owned(),
            stage_id: "repair".to_owned(),
            generation: 1,
            prompt: PromptSource::InlineMarkdown {
                content: "In a fresh process, inspect the current accepted repository state, make only any remaining bounded repair needed for the unittest, run it, and do not commit.\n"
                    .to_owned(),
            },
            verification_override: Some(good_verification),
        },
    )
    .map_err(|error| error.to_string())?;
    let proposal_digest = proposal.proposal().digest().as_str().to_owned();
    let proposal_value = decode_json(
        &proposal
            .to_canonical_json()
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let mut submit = request(
        "external-process-proposal",
        Some(paused.sequence),
        Command::SubmitProposal {
            document: proposal_value,
        },
    );
    submit.expected_revision = Some(revision.clone());
    let submitted = client
        .submit(&submit)
        .await
        .map_err(|error| error.to_string())?;
    let proposed_revision = json_string(&submitted.value, "proposed_revision")?;
    let decision_boundary = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?
        .sequence;
    let mut approve = request(
        "external-process-approve",
        Some(decision_boundary),
        Command::DecideProposal {
            run_id: PROCESS_RUN.to_owned(),
            proposal_id: "proposal-external-remediation-1".to_owned(),
            proposal_digest: proposal_digest.clone(),
            proposed_revision: proposed_revision.clone(),
            decision_id: "decision-external-remediation-1".to_owned(),
            decision: milkdrift_control_protocol::ProposalDecision::Approve,
        },
    );
    approve.expected_revision = Some(proposed_revision.clone());
    client
        .submit(&approve)
        .await
        .map_err(|error| error.to_string())?;
    let apply_boundary = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?
        .sequence;
    let mut apply = request(
        "external-process-apply",
        Some(apply_boundary),
        Command::ApplyProposal {
            run_id: PROCESS_RUN.to_owned(),
            proposal_id: "proposal-external-remediation-1".to_owned(),
            proposal_digest,
            proposed_revision: proposed_revision.clone(),
        },
    );
    apply.expected_revision = Some(proposed_revision.clone());
    client
        .submit(&apply)
        .await
        .map_err(|error| error.to_string())?;
    let signal_boundary = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?
        .sequence;
    client
        .submit(&request(
            "external-process-signal",
            Some(signal_boundary),
            Command::SignalRun {
                run_id: PROCESS_RUN.to_owned(),
                signal_id: "signal-external-remediation-1".to_owned(),
                signal_type: "sequence.approved".to_owned(),
                correlation: None,
                broadcast: false,
                payload: json!({"proposal":"proposal-external-remediation-1","revision":proposed_revision}),
            },
        ))
        .await
        .map_err(client_error)?;
    let resume_boundary = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?
        .sequence;
    client
        .submit(&request(
            "external-process-resume",
            Some(resume_boundary),
            Command::ResumeRun {
                run_id: PROCESS_RUN.to_owned(),
            },
        ))
        .await
        .map_err(|error| error.to_string())?;
    let completed = wait_for_run(&client, PROCESS_RUN, timeout, |state| {
        state.terminal.as_deref() == Some("succeeded")
    })
    .await?;
    for node in &completed.nodes {
        if node.latest_attempt_id.is_some() && node.attempt_count != 1 {
            return Err(format!(
                "task {} did not retain exactly one attempt",
                node.node_id
            ));
        }
    }
    let remediation_attempt = attempt_for_node(&completed, "stage-repair-remediation-1-coding")?;
    let good_attempt = attempt_for_node(&completed, "stage-repair-remediation-1-verification")?;
    let agent_read = client
        .attempt(PROCESS_RUN, &original_agent_attempt)
        .await
        .map_err(|error| error.to_string())?;
    let agent_provenance = agent_read
        .capability_provenance
        .as_ref()
        .ok_or_else(|| "agent attempt omitted capability provenance".to_owned())?;
    if agent_provenance.implementation_content_digest.as_deref()
        != Some(agent.content_digest.as_str())
        || agent_provenance.implementation_size_bytes != Some(agent.size_bytes)
        || agent_provenance.process_profile_digest.is_none()
        || agent_provenance.execution_policy_digest.is_none()
        || agent_provenance.configured_path_digest.is_none()
        || agent_provenance.canonical_path_digest.is_none()
    {
        return Err("agent attempt omitted exact executable/profile provenance".to_owned());
    }
    let good_read = client
        .attempt(PROCESS_RUN, &good_attempt)
        .await
        .map_err(|error| error.to_string())?;
    let weak_read = client
        .attempt(PROCESS_RUN, &weak_attempt)
        .await
        .map_err(client_error)?;
    let reviewer_read = client
        .attempt(PROCESS_RUN, &reviewer_attempt)
        .await
        .map_err(client_error)?;
    let remediation_read = client
        .attempt(PROCESS_RUN, &remediation_attempt)
        .await
        .map_err(client_error)?;
    let required_artifacts = ["verification_result", "verification_logs"];
    if required_artifacts
        .iter()
        .any(|name| !good_read.outputs.iter().any(|output| output.name == *name))
    {
        return Err("good verification omitted checkpoint/check and log artifacts".to_owned());
    }
    let process_reads = [
        ("initial_coding", &agent_read),
        ("controlled_verification", &weak_read),
        ("independent_review", &reviewer_read),
        ("remediation_coding", &remediation_read),
        ("final_verification", &good_read),
    ];
    let mut invocation_ids = BTreeSet::new();
    let mut invocations = Vec::new();
    let mut artifacts = Vec::new();
    for (role, read) in process_reads {
        let invocation = read
            .invocation_id
            .as_ref()
            .ok_or_else(|| format!("{role} attempt omitted its invocation identity"))?;
        if !invocation_ids.insert(invocation.clone()) {
            return Err("process attempts reused an external invocation identity".to_owned());
        }
        let provenance = read
            .capability_provenance
            .as_ref()
            .ok_or_else(|| format!("{role} attempt omitted capability provenance"))?;
        invocations.push(json!({
            "role":role,
            "attempt_id":read.attempt_id,
            "invocation_id":invocation,
            "capability_id":read.capability_id,
            "descriptor_revision":read.descriptor_revision,
            "snapshot_digest":provenance.snapshot_digest,
            "process_profile_digest":provenance.process_profile_digest,
            "context_manifest_digest":read.context.as_ref().map(|context| context.digest.clone()),
            "terminal":read.terminal,
            "uncertain":read.uncertain,
        }));
        artifacts.extend(read.outputs.iter().map(|output| ArtifactEvidence {
            artifact_id: output.artifact.artifact_id.clone(),
            digest: output.artifact.digest.clone(),
            size: output.artifact.size,
            content_type: output.artifact.content_type.clone(),
            role: format!("{role}:{}", output.name),
        }));
    }
    let final_commit = workflows::git(repository, &["rev-parse", "HEAD"])?;
    let final_tree = workflows::git(repository, &["rev-parse", "HEAD^{tree}"])?;
    let dirty_diff = workflows::git(repository, &["diff", "--binary", "HEAD"])?;
    daemon.terminate().map_err(|error| error.to_string())?;
    let evidence = ScenarioEvidence {
        qualifying: !fixture,
        outcome: "succeeded".to_owned(),
        profile: json!({
            "capability":agent.capability,
            "server_settings":{"status":"unknown","scope":"endpoint_inside_coding_agent","thinking":null,"model_identity":null},
            "executable_configured_path_digest":agent_provenance.configured_path_digest,
            "executable_canonical_path_digest":agent_provenance.canonical_path_digest,
            "executable_content_digest":agent.content_digest,
            "executable_size_bytes":agent.size_bytes,
            "version_output":agent.version_output,
            "profile_digest":agent_provenance.process_profile_digest,
            "policy_digest":agent_provenance.execution_policy_digest,
            "fixture_rejected_for_qualification":fixture,
        }),
        commands: vec![
            "external-process-import".to_owned(),
            "external-process-start".to_owned(),
            "external-process-pause".to_owned(),
            "external-process-proposal".to_owned(),
            "external-process-approve".to_owned(),
            "external-process-apply".to_owned(),
            "external-process-signal".to_owned(),
            "external-process-resume".to_owned(),
        ],
        runs: vec![PROCESS_RUN.to_owned()],
        revisions: vec![revision, proposed_revision],
        attempts: vec![
            original_agent_attempt,
            weak_attempt,
            reviewer_attempt,
            remediation_attempt,
            good_attempt,
        ],
        proposals: vec!["proposal-external-remediation-1".to_owned()],
        artifacts,
        restart_boundaries: vec![RestartEvidence {
            boundary: "after controlled verification failure and independent review".to_owned(),
            sequence_before: before_restart.sequence,
            sequence_after: recovered.sequence,
            recovered_state: "awaiting remediation approval".to_owned(),
            duplicate_attempts: false,
        }],
        facts: json!({
            "repository_initial_commit":initial_commit,
            "repository_initial_tree":initial_tree,
            "repository_final_commit":final_commit,
            "repository_final_tree":final_tree,
            "dirty_diff_digest":format!("b3_{}",blake3::hash(dirty_diff.as_bytes())),
            "dirty_diff_bytes":dirty_diff.len(),
            "controlled_failure":"orchestration verifier report deliberately failed a required check",
            "fresh_agent_attempts":2,
            "distinct_process_invocations":invocation_ids.len(),
            "process_invocations":invocations,
            "terminal_sequence":completed.sequence,
            "wait_timeout_secs":timeout.as_secs(),
        }),
        failure_reason: None,
    };
    evidence.validate_process_semantics()?;
    Ok(evidence)
}

#[allow(clippy::too_many_arguments)] // Exact profile, fixture observations, and operator bounds have separate owners.
pub(super) async fn run_model_scenario(
    config: &DaemonLaunch,
    token: &str,
    model_capability: &str,
    profile: &workflows::ModelProfileFacts,
    fixture_requests: Option<Arc<AtomicUsize>>,
    fixture_request_lines: Option<Arc<Mutex<Vec<String>>>>,
    fixture: bool,
    timeout: Duration,
    max_output_units: u64,
) -> HarnessResult<ScenarioEvidence> {
    let (mut daemon, client) = config
        .start(token)
        .await
        .map_err(|error| error.to_string())?;
    let blueprint = workflows::model_revision(model_capability, profile, max_output_units)?;
    let imported = client
        .submit(&request(
            "external-model-import",
            None,
            Command::ImportBlueprint {
                document: blueprint,
            },
        ))
        .await
        .map_err(client_error)?;
    let revision = json_string(&imported.value, "revision_id")?;
    client
        .submit(&request(
            "external-model-start",
            None,
            Command::StartRun {
                run_id: MODEL_RUN.to_owned(),
                workflow_id: MODEL_WORKFLOW.to_owned(),
                revision_id: revision.clone(),
            },
        ))
        .await
        .map_err(client_error)?;
    let before_restart = wait_for_run(&client, MODEL_RUN, timeout, |state| {
        state
            .nodes
            .iter()
            .any(|node| node.node_id == "model-release")
    })
    .await?;
    if fixture_requests
        .as_ref()
        .is_some_and(|count| count.load(Ordering::SeqCst) != 0)
    {
        return Err("fixture model endpoint was entered before release boundary".to_owned());
    }
    daemon.terminate().map_err(|error| error.to_string())?;

    let (mut daemon, client) = config
        .start(token)
        .await
        .map_err(|error| error.to_string())?;
    let recovered = client
        .run(MODEL_RUN)
        .await
        .map_err(|error| error.to_string())?;
    if recovered.sequence != before_restart.sequence
        || !recovered
            .nodes
            .iter()
            .any(|node| node.node_id == "model-release")
    {
        return Err("model workflow did not recover the pre-entry release boundary".to_owned());
    }
    client
        .submit(&request(
            "external-model-release",
            Some(recovered.sequence),
            Command::SignalRun {
                run_id: MODEL_RUN.to_owned(),
                signal_id: "signal-external-model-release".to_owned(),
                signal_type: "evidence.model.release".to_owned(),
                correlation: None,
                broadcast: false,
                payload: json!({"approved":true}),
            },
        ))
        .await
        .map_err(|error| error.to_string())?;
    let completed = wait_for_run(&client, MODEL_RUN, timeout, |state| {
        state.terminal.is_some()
            || state.nodes.iter().any(|node| {
                node.node_id == "model"
                    && (node.state.contains("uncertain")
                        || node.state.contains("rejected")
                        || node.state.contains("failed"))
            })
    })
    .await?;
    if completed.terminal.as_deref() != Some("succeeded") {
        let health = client.health().await.map_err(client_error)?;
        let failed_attempt = attempt_for_node(&completed, "model")?;
        let failed_read = client
            .attempt(MODEL_RUN, &failed_attempt)
            .await
            .map_err(client_error)?;
        if let Ok(acceptance_attempt) = attempt_for_node(&completed, "model-acceptance") {
            let acceptance = client
                .attempt(MODEL_RUN, &acceptance_attempt)
                .await
                .map_err(client_error)?;
            if let Some(decision) = acceptance.result_acceptance
                && !decision.accepted
            {
                return Err(format!(
                    "model result not accepted: {}; invocation terminal={}",
                    decision.reason,
                    failed_read.terminal.as_deref().unwrap_or("unobserved")
                ));
            }
        }
        return Err(format!(
            "model attempt did not complete; fixture_requests={}; fixture_request_lines={:?}; worker_failure={:?}; attempt_state={}; progress={}; terminal={:?}; outputs={:?}",
            fixture_requests
                .as_ref()
                .map_or(0, |count| count.load(Ordering::SeqCst)),
            fixture_request_lines
                .as_ref()
                .and_then(|lines| lines.lock().ok().map(|lines| lines.clone()))
                .unwrap_or_default(),
            health.last_failure,
            failed_read.state,
            failed_read.progress_observations,
            failed_read.terminal,
            failed_read
                .outputs
                .iter()
                .map(|output| output.name.clone())
                .collect::<Vec<_>>()
        ));
    }
    let model_attempt = attempt_for_node(&completed, "model")?;
    let attempt = client
        .attempt(MODEL_RUN, &model_attempt)
        .await
        .map_err(|error| error.to_string())?;
    if completed
        .nodes
        .iter()
        .find(|node| node.node_id == "model")
        .is_none_or(|node| node.attempt_count != 1)
    {
        return Err("model external entry was duplicated".to_owned());
    }
    let context = attempt
        .context
        .as_ref()
        .ok_or_else(|| "model attempt omitted authorized context manifest".to_owned())?;
    let denied_attempt = attempt_for_node(&completed, "evidence-denied")?;
    let denied = client
        .attempt(MODEL_RUN, &denied_attempt)
        .await
        .map_err(client_error)?;
    let denied_artifact = denied
        .outputs
        .iter()
        .find(|output| output.name == "evidence")
        .ok_or_else(|| "denied evidence task omitted its artifact".to_owned())?;
    let entries = serde_json::to_string(&context.entries).map_err(|error| error.to_string())?;
    let omissions = serde_json::to_string(&context.omissions).map_err(|error| error.to_string())?;
    if !entries.contains("evidence-a")
        || !entries.contains("evidence-b")
        || entries.contains(&denied_artifact.artifact.artifact_id)
        || !omissions.contains(&denied_artifact.artifact.artifact_id)
    {
        return Err("frozen context manifest selected/omitted the wrong evidence".to_owned());
    }
    if profile.streaming && attempt.progress_observations == 0 {
        return Err("streaming profile produced no durable fragment observation".to_owned());
    }
    let provenance = attempt
        .capability_provenance
        .as_ref()
        .ok_or_else(|| "model attempt omitted capability provenance".to_owned())?;
    if provenance.model_profile_digest.is_none()
        || provenance.model_profile_revision != Some(profile.revision)
        || provenance.provider_protocol.as_deref() != Some(profile.protocol.as_str())
        || provenance.model_alias.as_deref() != Some(profile.model_alias.as_str())
        || provenance.endpoint_origin.as_deref() != Some(profile.endpoint_origin.as_str())
    {
        return Err(
            "model attempt omitted exact profile/protocol/model/origin provenance".to_owned(),
        );
    }
    let response_output = attempt
        .outputs
        .iter()
        .find(|output| output.name == "model_response")
        .ok_or_else(|| "model response artifact is absent".to_owned())?;
    let provider_output = attempt
        .outputs
        .iter()
        .find(|output| output.name == "provider_metadata")
        .ok_or_else(|| "provider metadata artifact is absent".to_owned())?;
    let response_bytes = client
        .artifact_range(
            &response_output.artifact.artifact_id,
            0,
            response_output.artifact.size.saturating_sub(1),
        )
        .await
        .map_err(|error| error.to_string())?
        .bytes;
    let response = ModelResponseDocument::from_json(&response_bytes)
        .map_err(|error| error.to_string())?
        .body()
        .clone();
    let provider_bytes = client
        .artifact_range(
            &provider_output.artifact.artifact_id,
            0,
            provider_output.artifact.size.saturating_sub(1),
        )
        .await
        .map_err(client_error)?
        .bytes;
    let provider_metadata: Value =
        serde_json::from_slice(&provider_bytes).map_err(|error| error.to_string())?;
    if provider_metadata
        .as_object()
        .is_none_or(serde_json::Map::is_empty)
    {
        return Err("provider metadata artifact is empty or malformed".to_owned());
    }
    let provider_identity = provider_metadata.as_object().is_some_and(|metadata| {
        metadata.values().any(|value| {
            value
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|identity| !identity.is_empty())
                && value
                    .get("model")
                    .and_then(Value::as_str)
                    .is_some_and(|model| !model.is_empty())
        })
    });
    if !provider_identity {
        return Err("provider metadata omitted response identity or model".to_owned());
    }
    let structured_ok = response
        .structured()
        .and_then(|value| value.value().get("ok"))
        .and_then(Value::as_bool)
        == Some(true);
    let parseable_ok = response.text().trim() == "MILKDRIFT_EVIDENCE_OK";
    if profile.structured_output && !structured_ok || !profile.structured_output && !parseable_ok {
        return Err("model response did not satisfy the bounded parseable contract".to_owned());
    }
    if fixture_requests
        .as_ref()
        .is_some_and(|count| count.load(Ordering::SeqCst) != 1)
    {
        return Err("fixture endpoint observed a duplicate or missing remote request".to_owned());
    }
    if fixture_request_lines.as_ref().is_some_and(|lines| {
        lines.lock().map_or(true, |lines| {
            lines.as_slice() != ["POST /v1/chat/completions HTTP/1.1"]
        })
    }) {
        return Err("fixture endpoint observed the wrong request target".to_owned());
    }
    let artifacts = attempt
        .outputs
        .iter()
        .map(|output| ArtifactEvidence {
            artifact_id: output.artifact.artifact_id.clone(),
            digest: output.artifact.digest.clone(),
            size: output.artifact.size,
            content_type: output.artifact.content_type.clone(),
            role: output.name.clone(),
        })
        .collect::<Vec<_>>();
    let finish_reason = format!("{:?}", response.finish_reason()).to_lowercase();
    if response.finish_reason() != milkdrift_model::FinishReason::Stop {
        return Err("provider response did not finish normally; truncated or incomplete output cannot qualify".to_owned());
    }
    let usage = response.usage();
    if usage.input_units.is_none()
        && usage.output_units.is_none()
        && usage.cached_input_units.is_none()
        && usage.cost_micros.is_none()
    {
        return Err("provider response omitted all usage facts".to_owned());
    }
    let durable_usage = attempt
        .usage
        .as_ref()
        .ok_or_else(|| "model terminal evidence omitted durable usage facts".to_owned())?;
    if durable_usage.input_units != usage.input_units
        || durable_usage.output_units != usage.output_units
    {
        return Err("durable attempt usage differs from the committed response".to_owned());
    }
    let context_digest = context.digest.clone();
    let selected_count = context.entries.len();
    let omitted_count = context.omissions.len();
    daemon.terminate().map_err(|error| error.to_string())?;
    Ok(ScenarioEvidence {
        qualifying: !fixture,
        outcome: "succeeded".to_owned(),
        profile: json!({
            "profile_digest":provenance.model_profile_digest,
            "profile_revision":profile.revision,
            "provider_protocol":profile.protocol,
            "model_alias":profile.model_alias,
            "server_settings":{"status":"unknown","scope":"direct_endpoint","thinking":null},
            "generation_limits":{"requested_output_units":max_output_units,"reasoning_request":null,"profile_token_limit":null,"contract_ceiling":milkdrift_model::MAX_MODEL_OUTPUT_UNITS,"harness_default":64,"harness_ceiling":65536},
            "endpoint_origin":profile.endpoint_origin,
            "fixture_rejected_for_qualification":fixture,
        }),
        commands: vec![
            "external-model-import".to_owned(),
            "external-model-start".to_owned(),
            "external-model-release".to_owned(),
        ],
        runs: vec![MODEL_RUN.to_owned()],
        revisions: vec![revision],
        attempts: vec![model_attempt],
        proposals: Vec::new(),
        artifacts,
        restart_boundaries: vec![RestartEvidence {
            boundary: "durable signal wait before model adapter entry".to_owned(),
            sequence_before: before_restart.sequence,
            sequence_after: recovered.sequence,
            recovered_state: "model request unreleased; no external entry".to_owned(),
            duplicate_attempts: false,
        }],
        facts: json!({
            "context_manifest_digest":context_digest,
            "max_output_units":max_output_units,
            "wait_timeout_secs":timeout.as_secs(),
            "selected_count":selected_count,
            "omitted_count":omitted_count,
            "denied_context_artifact_digest":denied_artifact.artifact.digest,
            "streaming_observations":attempt.progress_observations,
            "streaming_bytes":attempt.progress_bytes,
            "finish_reason":finish_reason,
            "usage":{"input_units":usage.input_units,"output_units":usage.output_units,"cached_input_units":usage.cached_input_units,"duration_ms":durable_usage.duration_ms,"cost_micros":usage.cost_micros,"currency":usage.currency},
            "provider_metadata_artifact_digest":provider_output.artifact.digest,
            "terminal":"succeeded",
            "uncertain":attempt.uncertain,
            "capability_generation":attempt.descriptor_revision,
            "invocation_id":attempt.invocation_id,
        }),
        failure_reason: None,
    })
}

async fn wait_for_run(
    client: &ControlClient,
    run: &str,
    timeout: Duration,
    predicate: impl Fn(&milkdrift_control_protocol::RunRead) -> bool,
) -> HarnessResult<milkdrift_control_protocol::RunRead> {
    let mut last = None;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let Ok(state) = tokio::time::timeout_at(deadline, client.run(run)).await else {
            break;
        };
        let state = state.map_err(|error| error.to_string())?;
        if predicate(&state) {
            return Ok(state);
        }
        if state.lifecycle == "terminal" {
            return Err(format!(
                "run {run} reached unexpected terminal state: {}",
                serde_json::to_string(&state).map_err(|error| error.to_string())?
            ));
        }
        last = Some(state);
        if tokio::time::timeout_at(deadline, tokio::time::sleep(Duration::from_millis(25)))
            .await
            .is_err()
        {
            break;
        }
    }
    Err(format!(
        "run {run} did not reach the expected state within {} seconds; last={}",
        timeout.as_secs(),
        serde_json::to_string(&last).map_err(|error| error.to_string())?
    ))
}

fn attempt_for_node(
    run: &milkdrift_control_protocol::RunRead,
    node: &str,
) -> HarnessResult<String> {
    run.nodes
        .iter()
        .find(|value| value.node_id == node)
        .and_then(|value| value.latest_attempt_id.clone())
        .ok_or_else(|| format!("node {node} has no attempt"))
}

fn request(command_id: &str, sequence: Option<u64>, command: Command) -> CommandRequest {
    CommandRequest {
        protocol: ProtocolVersion::CURRENT,
        command_id: command_id.to_owned(),
        expected_sequence: sequence,
        expected_revision: None,
        reason: "external interoperability evidence harness".to_owned(),
        evidence: Vec::new(),
        command,
    }
}

fn json_string(value: &Value, name: &str) -> HarnessResult<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("command response omitted {name}"))
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
            wait_for_run(&client, "test-run", Duration::from_millis(50), |_| true),
        )
        .await;
        server.abort();
        let Err(error) = result? else {
            return Err("a stalled read cannot establish workflow completion".into());
        };
        assert!(error.contains("did not reach the expected state"));
        Ok(())
    }
}
