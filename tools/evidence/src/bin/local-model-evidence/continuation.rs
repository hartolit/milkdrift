//! Operator-selected continuation and Fresh isolation through actual daemon/CLI binaries.

use super::{
    ACTOR, Arguments, AuthorRef, BlueprintRevision, BlueprintRevisionDocument, CliRunner,
    ControlledEndpoint, Duration, EvidenceConfig, EvidenceResult, ModelProfileConfig, Mutation,
    MutationBatch, Node, NodeId, NodeKind, OperationId, PortId, SessionSelection, TOKEN,
    TaskConfig, TaskContextPolicy, TerminalOutcome, WorkflowId, ensure, fs, json, node, path_text,
    required_text, required_u64, reserve_endpoint, start_daemon, wait_for_readiness, wait_for_run,
    write_config, write_model_profile, write_private,
};
use milkdrift_capability::{ArtifactReference, BoundedJson};
use milkdrift_control::{
    ClaimedStopCondition, ProposalApplicationPolicy, ProposalId, ProposalProvenance,
    WorkflowProposal, WorkflowProposalDocument,
};
use milkdrift_model::{
    ContentPart, Message, MessageRole, ModelTaskRequest, ModelTaskRequestDocument,
};
use std::collections::BTreeMap;

const RUN: &str = "continuation-run";

pub(super) fn scenario(
    arguments: &Arguments,
    output: &std::path::Path,
) -> EvidenceResult<serde_json::Value> {
    Ok(
        json!({"successful": run_case(arguments, output, false)?, "rejected_answer": run_case(arguments, output, true)?}),
    )
}

fn run_case(
    arguments: &Arguments,
    output: &std::path::Path,
    rejected: bool,
) -> EvidenceResult<serde_json::Value> {
    let directory = output.join(if rejected {
        "continuation-rejected"
    } else {
        "continuation-session"
    });
    fs::create_dir(&directory)?;
    let mut first = ControlledEndpoint::response("EXACT_PRIOR_ANSWER", "stop")?;
    let mut continued = ControlledEndpoint::response("CONTINUED_ANSWER", "stop")?;
    let mut fresh = ControlledEndpoint::response("FRESH_ANSWER", "stop")?;
    let mut profiles = Vec::new();
    let mut facts = Vec::new();
    for (id, endpoint) in [
        ("first", &first),
        ("continued", &continued),
        ("fresh", &fresh),
    ] {
        let profile =
            write_model_profile(&directory, id, endpoint.address, "continuation-fixture")?;
        facts.push(super::inspect_profile(&profile)?);
        profiles.push(ModelProfileConfig {
            capability_id: id.to_owned(),
            profile,
        });
    }
    let token_file = write_private(&directory.join("controller.token"), TOKEN.as_bytes())?;
    let mut authority = milkdrift_daemon::ActorGrantConfig::dangerous_administrator();
    authority.resources.network = milkdrift_authority::NetworkScope::new(
        facts
            .iter()
            .map(|facts| milkdrift_authority::NetworkProfileRef::new(&facts.profile_id))
            .collect::<Result<_, _>>()?,
        facts
            .iter()
            .map(|facts| super::profile_destination(&facts.endpoint_origin))
            .collect::<Result<_, _>>()?,
    )?;
    let bind = reserve_endpoint()?;
    let config = write_config(
        &directory,
        bind,
        &token_file,
        ACTOR,
        EvidenceConfig {
            process_profiles: Vec::new(),
            model_profiles: profiles,
            secret_sources: BTreeMap::new(),
            lease_duration_ms: 5000,
            authority,
        },
    )?;
    let runner = CliRunner {
        executable: arguments.cli.clone(),
        endpoint: format!("http://{bind}/"),
        token_file,
        forbidden_storage_path: directory.join("data"),
    };
    let mut operations = vec![
        Mutation::AddNode {
            node: model(
                "first",
                &facts[0],
                "ORIGINAL_QUESTION",
                SessionSelection::Fresh,
                false,
            )?,
        },
        Mutation::AddNode {
            node: Node::new(
                NodeId::new("hold")?,
                NodeKind::SignalWait {
                    signal: OperationId::new("continuation.release")?,
                },
            )?
            .with_control_input(PortId::new("in")?)?
            .with_control_output(PortId::new("next")?)?,
        },
        Mutation::AddNode {
            node: model(
                "continued",
                &facts[1],
                "placeholder",
                SessionSelection::Fresh,
                true,
            )?,
        },
        Mutation::AddNode {
            node: model(
                "fresh",
                &facts[2],
                "FRESH_QUESTION",
                SessionSelection::Fresh,
                true,
            )?,
        },
        Mutation::AddNode {
            node: Node::new(
                NodeId::new("done")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?
            .with_control_input(PortId::new("in")?)?,
        },
        super::edge("hold-continued", "hold", "continued")?,
        super::edge("continued-fresh", "continued", "fresh")?,
        super::edge("fresh-done", "fresh", "done")?,
    ];
    use milkdrift_control::{ResultAcceptanceContract, ResultRequirement, result_acceptance_task};
    let requirement = if rejected {
        ResultRequirement::ModelTools {
            names: std::collections::BTreeSet::from(["lookup".to_owned()]),
        }
    } else {
        ResultRequirement::ModelProse
    };
    operations.push(Mutation::AddNode {
        node: result_acceptance_task(
            NodeId::new("acceptance")?,
            ResultAcceptanceContract::new(requirement, std::collections::BTreeSet::new())?,
            milkdrift_blueprint::SchemaRef::new(
                milkdrift_capability::SchemaId::new("milkdrift.artifact-reference")?,
                1,
            )?,
        )?,
    });
    operations.push(super::edge("first-acceptance", "first", "acceptance")?);
    for (id, kind, source, source_port, target, target_port) in [
        (
            "acceptance-hold",
            milkdrift_blueprint::EdgeKind::Control,
            "acceptance",
            "out",
            "hold",
            "in",
        ),
        (
            "acceptance-subject",
            milkdrift_blueprint::EdgeKind::Data,
            "first",
            "model_response",
            "acceptance",
            "result",
        ),
    ] {
        operations.push(Mutation::AddEdge {
            edge: milkdrift_blueprint::Edge::new(
                milkdrift_blueprint::EdgeId::new(id)?,
                kind,
                NodeId::new(source)?,
                PortId::new(source_port)?,
                NodeId::new(target)?,
                PortId::new(target_port)?,
            ),
        });
    }
    let initial = BlueprintRevision::genesis(
        WorkflowId::new("continuation-workflow")?,
        MutationBatch::new(operations)?,
        AuthorRef::new(ACTOR)?,
        "Exact-reference continuation with an ordinary prospective revision",
    )?;
    let blueprint = directory.join("blueprint.json");
    fs::write(
        &blueprint,
        BlueprintRevisionDocument::new(&initial).to_canonical_json()?,
    )?;
    let mut daemon = start_daemon(&arguments.daemon, &config)?;
    wait_for_readiness(&runner, &mut daemon)?;
    runner.success(&[
        "--command-id",
        "continuation-import",
        "blueprint",
        "import",
        path_text(&blueprint)?,
    ])?;
    runner.success(&[
        "--command-id",
        "continuation-start",
        "run",
        "start",
        RUN,
        "continuation-workflow",
        initial.id().as_str(),
    ])?;
    let waiting = wait_for_run(&runner, RUN, Duration::from_secs(20), |run| {
        node(run, "hold").is_some() && node(run, "continued").is_none()
    })?;
    let id = required_text(
        node(&waiting, "first").ok_or("first invocation absent")?,
        &["latest_attempt_id"],
    )?;
    let inspected = runner.success(&["attempt", "inspect", RUN, &id])?;
    let manifest = reference(&inspected["value"]["context_manifest"])?;
    let response = inspected["value"]["outputs"]
        .as_array()
        .and_then(|outputs| {
            outputs
                .iter()
                .find(|output| output["name"] == "model_response")
        })
        .ok_or("canonical response missing")?;
    let response = reference(&response["artifact"])?;
    let replacement = model(
        "continued",
        &facts[1],
        "NEW_QUESTION",
        SessionSelection::ExplicitContinuation {
            manifest: manifest.clone(),
            response: response.clone(),
        },
        true,
    )?;
    let proposal = WorkflowProposalDocument::new(WorkflowProposal::new(
        ProposalId::new("continuation-proposal")?,
        milkdrift_authority::ActorRef::new(ACTOR)?,
        ProposalProvenance::Direct,
        initial.semantic().workflow().clone(),
        Some(milkdrift_workspace::RunId::new(RUN)?),
        initial.id().clone(),
        initial.content_digest().clone(),
        Some(milkdrift_persistence::RunSequence::new(required_u64(
            &waiting,
            &["value", "sequence"],
        )?)),
        MutationBatch::new(vec![Mutation::ReplaceNode { node: replacement }])?,
        "Continue the exact inspected response and manifest",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::ProposeOnly,
        None,
        ClaimedStopCondition::Continue,
    )?);
    let proposal_path = directory.join("proposal.json");
    fs::write(&proposal_path, proposal.to_canonical_json()?)?;
    let submitted = runner.success(&[
        "--command-id",
        "continuation-submit",
        "--expected-sequence",
        &required_u64(&waiting, &["value", "sequence"])?.to_string(),
        "--expected-revision",
        initial.id().as_str(),
        "proposal",
        "submit",
        path_text(&proposal_path)?,
    ])?;
    let revision = required_text(&submitted, &["value", "value", "proposed_revision"])?;
    for action in ["approve", "apply"] {
        let state = runner.success(&["run", "show", RUN])?;
        let sequence = required_u64(&state, &["value", "sequence"])?.to_string();
        let command = format!("continuation-{action}");
        let mut args = vec![
            "--yes",
            "--command-id",
            &command,
            "--expected-sequence",
            &sequence,
            "--expected-revision",
            &revision,
            "proposal",
            action,
            RUN,
            "continuation-proposal",
            proposal.proposal().digest().as_str(),
            &revision,
        ];
        if action == "approve" {
            args.push("continuation-decision");
        }
        runner.success(&args)?;
    }
    daemon.terminate()?;
    daemon = start_daemon(&arguments.daemon, &config)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let reopened = runner.success(&["run", "show", RUN])?;
    ensure(
        node(&reopened, "continued").is_none(),
        "restart entered an unreleased continuation",
    )?;
    runner.success(&[
        "--command-id",
        "continuation-release",
        "--expected-sequence",
        &required_u64(&reopened, &["value", "sequence"])?.to_string(),
        "run",
        "signal",
        RUN,
        "--signal-id",
        "continue",
        "--signal-type",
        "continuation.release",
        "--payload",
        "{}",
    ])?;
    if rejected {
        let stopped = wait_for_run(&runner, RUN, Duration::from_secs(20), |run| {
            run["value"]["terminal"] == "failed"
        })?;
        ensure(
            node(&stopped, "continued").is_some_and(|node| node["attempt_count"] == 0),
            "rejected predecessor was scheduled",
        )?;
        let inspected = runner.success(&["run", "timeline", RUN, "--limit", "100"])?;
        let acceptance_id = required_text(
            node(&stopped, "acceptance").ok_or("acceptance absent")?,
            &["latest_attempt_id"],
        )?;
        let acceptance = runner.success(&["attempt", "inspect", RUN, &acceptance_id])?;
        ensure(
            acceptance["value"]["result_acceptance"]["accepted"] == false,
            "fixture did not reject its complete prior answer",
        )?;
        ensure(
            continued.requests.load(super::Ordering::SeqCst) == 0
                && fresh.requests.load(super::Ordering::SeqCst) == 0,
            "rejected prior answer reached a dependent endpoint",
        )?;
        fs::write(
            directory.join("refusal.json"),
            serde_json::to_vec_pretty(&inspected)?,
        )?;
        first.join()?;
        daemon.terminate()?;
        return Ok(json!({"requests":1,"rejected_prior_refused":true,"restart_boundaries":1}));
    }
    let completed = wait_for_run(&runner, RUN, Duration::from_secs(20), |run| {
        run["value"]["terminal"] == "succeeded"
    })?;
    let id = required_text(
        node(&completed, "continued").ok_or("continued invocation missing")?,
        &["latest_attempt_id"],
    )?;
    let consumed = runner.success(&["attempt", "inspect", RUN, &id])?;
    let selection = consumed["value"]["context"]["entries"]
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["reason"] == "continuation")
        })
        .ok_or("inspector omitted continuation selection")?;
    let artifact = &selection["source"]["reference"];
    let history_id = artifact["artifact"]
        .as_str()
        .ok_or("continuation artifact identity missing")?;
    let history_file = directory.join("consumed-history.json");
    runner.success(&[
        "artifact",
        "get",
        history_id,
        "--output",
        path_text(&history_file)?,
    ])?;
    let history =
        milkdrift_model::ContinuationHistoryDocument::from_json(&fs::read(&history_file)?)?;
    ensure(
        history.body().turns().len() == 1
            && history.body().turns()[0].manifest == manifest
            && history.body().turns()[0].response == response,
        "inspected history changed the selected predecessor",
    )?;
    first.join()?;
    let request = continued.join()?;
    let wire: serde_json::Value = serde_json::from_str(
        request
            .split("\r\n\r\n")
            .nth(1)
            .ok_or("continued request body absent")?,
    )?;
    let messages = wire["messages"]
        .as_array()
        .ok_or("continued messages absent")?;
    ensure(
        messages.iter().any(|message| {
            message["role"] == "assistant" && message.to_string().contains("EXACT_PRIOR_ANSWER")
        }) && messages.iter().any(|message| {
            message["role"] == "user" && message.to_string().contains("ORIGINAL_QUESTION")
        }) && messages
            .iter()
            .any(|message| message.to_string().contains("NEW_QUESTION")),
        "wire lost exact role-labelled continuation",
    )?;
    let fresh_request = fresh.join()?;
    ensure(
        !fresh_request.contains("ORIGINAL_QUESTION")
            && !fresh_request.contains("EXACT_PRIOR_ANSWER"),
        "Fresh implicitly included earlier conversation",
    )?;
    fs::write(directory.join("continued-request.http"), request)?;
    fs::write(directory.join("fresh-request.http"), fresh_request)?;
    daemon.terminate()?;
    daemon = start_daemon(&arguments.daemon, &config)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let retained = runner.success(&["attempt", "inspect", RUN, &id])?;
    ensure(
        retained["value"] == consumed["value"],
        "restart changed exact continuation inspection",
    )?;
    daemon.terminate()?;
    Ok(
        json!({"requests":3,"explicit_continuation":true,"fresh_isolated":true,"restart_boundaries":2,"inspection_preserved":true,"history_artifact":history_id}),
    )
}

fn reference(value: &serde_json::Value) -> EvidenceResult<ArtifactReference> {
    Ok(ArtifactReference::new(
        required_text(value, &["artifact_id"])?,
        required_text(value, &["digest"])?,
        Some(required_text(value, &["content_type"])?),
        Some(required_u64(value, &["size"])?),
    )?)
}

fn model(
    id: &str,
    facts: &super::ModelFacts,
    text: &str,
    session: SessionSelection,
    input: bool,
) -> EvidenceResult<Node> {
    let template = super::model_node(id, facts, false, 64)?;
    let NodeKind::Task { config } = template.kind() else {
        return Err("model template is not a task".into());
    };
    let mut policy = serde_json::to_value(TaskContextPolicy::default())?;
    policy["session"] = json!(if matches!(session, SessionSelection::Fresh) {
        "fresh"
    } else {
        "explicit_continuation"
    });
    policy["exclude_categories"] = json!([]);
    let mut node = Node::new(
        NodeId::new(id)?,
        NodeKind::Task {
            config: TaskConfig::new(
                config.requirement().clone(),
                serde_json::from_value(policy)?,
            )?,
        },
    )?
    .with_control_output(PortId::new("next")?)?;
    if input {
        node = node.with_control_input(PortId::new("in")?)?;
    }
    for (port, output) in template.data_outputs() {
        node = node.with_data_output(port.clone(), output.clone())?;
    }
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: text.to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        session,
        None,
        64,
        facts.streaming,
        BTreeMap::new(),
    )?;
    node = node.with_data_input(
        PortId::new(milkdrift_model::MODEL_TASK_INPUT_NAME)?,
        milkdrift_blueprint::DataPort::input(
            milkdrift_blueprint::SchemaRef::new(
                milkdrift_capability::SchemaId::new("milkdrift.model-task")?,
                1,
            )?,
            true,
            Some(milkdrift_blueprint::BindingSource::Literal {
                value: BoundedJson::new(serde_json::to_value(ModelTaskRequestDocument::new(
                    task,
                ))?)?,
            }),
        )?,
    )?;
    Ok(node)
}
