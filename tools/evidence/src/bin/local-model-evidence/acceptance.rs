//! Production acceptance composition shared by the normal smoke and its rejection scenario.

use super::{
    BTreeSet, Edge, EdgeId, EdgeKind, EvidenceResult, Mutation, Node, NodeId, NodeKind, PortId,
    SchemaId, SchemaRef, TerminalOutcome,
};
use milkdrift_control::{
    ACCEPTED_RESULT_OUTPUT, ResultAcceptanceContract, ResultRequirement, result_acceptance_gate,
    result_acceptance_task,
};

pub(super) fn scenario(
    arguments: &super::Arguments,
    output: &std::path::Path,
) -> EvidenceResult<serde_json::Value> {
    use super::{
        ACTOR, ActorGrantConfig, AuthorRef, BlueprintRevision, BlueprintRevisionDocument,
        CliRunner, ControlledEndpoint, Duration, EvidenceConfig, ModelProfileConfig, MutationBatch,
        NetworkProfileRef, NetworkScope, OperationId, Ordering, TOKEN, WorkflowId, ensure, fs,
        inspect_profile, json, node, path_text, profile_destination, required_text,
        reserve_endpoint, start_daemon, wait_for_readiness, wait_for_run, write_config,
        write_model_profile, write_private,
    };
    use std::collections::BTreeMap;
    let directory = output.join("acceptance-session");
    fs::create_dir(&directory)?;
    let mut empty = ControlledEndpoint::response("", "length")?;
    let mut review = ControlledEndpoint::response(
        "Reviewed the supplied evidence; continuation is justified.",
        "stop",
    )?;
    let mut next = ControlledEndpoint::response("Continuation entered.", "stop")?;
    let mut profiles = Vec::new();
    let mut facts = Vec::new();
    for (name, endpoint) in [
        ("empty-review", &empty),
        ("remedied-review", &review),
        ("dependent", &next),
    ] {
        let profile =
            write_model_profile(&directory, name, endpoint.address, "controlled-acceptance")?;
        facts.push(inspect_profile(&profile)?);
        profiles.push(ModelProfileConfig {
            capability_id: name.to_owned(),
            profile,
        });
    }
    let mut authority = ActorGrantConfig::dangerous_administrator();
    authority.resources.network = NetworkScope::new(
        facts
            .iter()
            .map(|fact| NetworkProfileRef::new(&fact.profile_id))
            .collect::<Result<_, _>>()?,
        facts
            .iter()
            .map(|fact| profile_destination(&fact.endpoint_origin))
            .collect::<Result<_, _>>()?,
    )?;
    let token_file = write_private(&directory.join("controller.token"), TOKEN.as_bytes())?;
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
    let wait = |id: &str, signal: &str| -> EvidenceResult<Mutation> {
        Ok(Mutation::AddNode {
            node: Node::new(
                NodeId::new(id)?,
                NodeKind::SignalWait {
                    signal: OperationId::new(signal)?,
                },
            )?
            .with_control_input(PortId::new("in")?)?
            .with_control_output(PortId::new("next")?)?,
        })
    };
    let model = |id: &str,
                 capability: &str,
                 fact: &super::ModelFacts,
                 predecessor: bool|
     -> EvidenceResult<Mutation> {
        let template = super::model_node(capability, fact, false, 64)?;
        let mut node = Node::new(NodeId::new(id)?, template.kind().clone())?
            .with_control_output(PortId::new("next")?)?;
        if predecessor {
            node = node.with_control_input(PortId::new("in")?)?;
        }
        for (port, input) in template.data_inputs() {
            node = node.with_data_input(port.clone(), input.clone())?;
        }
        for (port, output) in template.data_outputs() {
            node = node.with_data_output(port.clone(), output.clone())?;
        }
        Ok(Mutation::AddNode { node })
    };
    let mut operations = vec![
        model("model", "empty-review", &facts[0], false)?,
        wait("before-acceptance", "acceptance.evaluate")?,
        wait("remediation", "acceptance.remediate")?,
        model("repair", "remedied-review", &facts[1], true)?,
        wait("before-continuation", "acceptance.continue")?,
        model("dependent", "dependent", &facts[2], true)?,
        model("forbidden-dependent", "dependent", &facts[2], true)?,
        terminal("done", TerminalOutcome::Success)?,
        terminal("unexpected", TerminalOutcome::Success)?,
        terminal("failed", TerminalOutcome::Failure)?,
        super::edge("model-evaluate-wait", "model", "before-acceptance")?,
        super::edge(
            "evaluate-wait-acceptance",
            "before-acceptance",
            "first-acceptance",
        )?,
        super::edge("remediation-repair", "remediation", "repair")?,
        super::edge("repair-acceptance", "repair", "second-acceptance")?,
        super::edge("continue-dependent", "before-continuation", "dependent")?,
        super::edge("dependent-done", "dependent", "done")?,
        super::edge("forbidden-unexpected", "forbidden-dependent", "unexpected")?,
    ];
    operations.extend(path(
        "first",
        "model",
        "forbidden-dependent",
        "remediation",
    )?);
    operations.extend(path("second", "repair", "before-continuation", "failed")?);
    let revision = BlueprintRevision::genesis(
        WorkflowId::new("acceptance-workflow")?,
        MutationBatch::new(operations)?,
        AuthorRef::new(ACTOR)?,
        "Controlled result acceptance, authorized remediation, and restart",
    )?;
    let bytes = BlueprintRevisionDocument::new(&revision).to_canonical_json()?;
    let blueprint = directory.join("blueprint.json");
    fs::write(&blueprint, bytes)?;
    let mut daemon = start_daemon(&arguments.daemon, &config)?;
    wait_for_readiness(&runner, &mut daemon)?;
    runner.success(&[
        "--command-id",
        "acceptance-import",
        "blueprint",
        "import",
        path_text(&blueprint)?,
    ])?;
    runner.success(&[
        "--command-id",
        "acceptance-start",
        "run",
        "start",
        "acceptance-run",
        "acceptance-workflow",
        revision.id().as_str(),
    ])?;
    let mut rejected = None;
    let mut accepted = None;
    for (boundary, signal, absent) in [
        (
            "before-acceptance",
            "acceptance.evaluate",
            "first-acceptance",
        ),
        ("remediation", "acceptance.remediate", "repair"),
        ("before-continuation", "acceptance.continue", "dependent"),
    ] {
        let state = wait_for_run(&runner, "acceptance-run", Duration::from_secs(20), |run| {
            node(run, boundary).is_some() && node(run, absent).is_none()
        })?;
        ensure(
            next.requests.load(Ordering::SeqCst) == 0,
            "dependent model entered before accepted review and release",
        )?;
        let source_attempt = required_text(
            node(&state, "model").ok_or("original model absent")?,
            &["latest_attempt_id"],
        )?;
        let source = runner.success(&["attempt", "inspect", "acceptance-run", &source_attempt])?;
        ensure(
            source["value"]["terminal"] == "succeeded"
                && source["value"]["model_generation"]["finish_reason"] == "length",
            "acceptance changed or lost the successful exhausted invocation",
        )?;
        let mut decision_before_restart = None;
        if boundary == "remediation" || boundary == "before-continuation" {
            let prefix = if boundary == "remediation" {
                "first"
            } else {
                "second"
            };
            let attempt = required_text(
                node(&state, &format!("{prefix}-acceptance")).ok_or("acceptance node absent")?,
                &["latest_attempt_id"],
            )?;
            let inspected = runner.success(&["attempt", "inspect", "acceptance-run", &attempt])?;
            ensure(
                node(&state, &format!("{prefix}-acceptance")).ok_or("acceptance node absent")?["attempt_count"]
                    == 1,
                "acceptance evaluation repeated",
            )?;
            decision_before_restart = Some((attempt, inspected["value"].clone()));
            let result = inspected["value"]["result_acceptance"].clone();
            ensure(
                result["accepted"] == (boundary == "before-continuation"),
                "acceptance result contradicted its workflow route",
            )?;
            if boundary == "remediation" {
                ensure(
                    result["reason"] == "output_allowance_exhausted",
                    "empty exhausted review lost its rejection reason",
                )?;
                rejected = Some(result);
            } else {
                accepted = Some(result);
            }
        }
        daemon.terminate()?;
        daemon = start_daemon(&arguments.daemon, &config)?;
        wait_for_readiness(&runner, &mut daemon)?;
        let reopened = runner.success(&["run", "show", "acceptance-run"])?;
        ensure(
            node(&reopened, absent).is_none() && next.requests.load(Ordering::SeqCst) == 0,
            "restart opened an unreleased acceptance boundary",
        )?;
        if let Some((attempt, before)) = decision_before_restart {
            let after = runner.success(&["attempt", "inspect", "acceptance-run", &attempt])?;
            ensure(
                before == after["value"],
                "restart altered the retained acceptance decision",
            )?;
        }
        let command = format!("release-{boundary}");
        let sequence = super::required_u64(&reopened, &["value", "sequence"])?.to_string();
        let args = [
            "--command-id",
            &command,
            "--expected-sequence",
            &sequence,
            "run",
            "signal",
            "acceptance-run",
            "--signal-id",
            &command,
            "--signal-type",
            signal,
            "--payload",
            r#"{"authorized":true}"#,
        ];
        let mut first = runner.success(&args)?;
        let replay = runner.success(&args)?;
        ensure(
            replay["value"]["replayed"] == true,
            "release did not return its retained receipt",
        )?;
        first["value"]["replayed"] = serde_json::Value::Bool(true);
        ensure(
            first["value"] == replay["value"] && first["ok"] == replay["ok"],
            "exact authorized release replay changed its durable result",
        )?;
    }
    let completed = wait_for_run(&runner, "acceptance-run", Duration::from_secs(20), |run| {
        run["value"]["terminal"] == "succeeded"
    })?;
    ensure(
        node(&completed, "forbidden-dependent").is_none(),
        "rejected source opened the original dependent path",
    )?;
    for (endpoint, label) in [
        (&mut empty, "original"),
        (&mut review, "remediation"),
        (&mut next, "dependent"),
    ] {
        endpoint.join()?;
        ensure(
            endpoint.requests.load(Ordering::SeqCst) == 1,
            &format!("{label} entry count was not exactly one"),
        )?;
    }
    daemon.terminate()?;
    Ok(
        json!({"run":"acceptance-run","rejected":rejected,"accepted":accepted,"dependent_entries_before_acceptance":0,"dependent_entries_after_release":1,"restart_boundaries":3,"exact_release_replay":true}),
    )
}

pub(super) fn path(
    prefix: &str,
    source: &str,
    pass: &str,
    fail: &str,
) -> EvidenceResult<Vec<Mutation>> {
    let acceptance = format!("{prefix}-acceptance");
    let gate = format!("{prefix}-gate");
    let schema = SchemaRef::new(SchemaId::new("milkdrift.artifact-reference")?, 1)?;
    Ok(vec![
        Mutation::AddNode {
            node: result_acceptance_task(
                NodeId::new(&acceptance)?,
                ResultAcceptanceContract::new(ResultRequirement::ModelProse, BTreeSet::new())?,
                schema.clone(),
            )?,
        },
        Mutation::AddNode {
            node: result_acceptance_gate(NodeId::new(&gate)?, NodeId::new(&acceptance)?, schema)?,
        },
        link(
            &format!("{prefix}-response"),
            EdgeKind::Data,
            source,
            "model_response",
            &acceptance,
            "result",
        )?,
        link(
            &format!("{prefix}-assessed"),
            EdgeKind::Control,
            &acceptance,
            "out",
            &gate,
            "in",
        )?,
        link(
            &format!("{prefix}-accepted"),
            EdgeKind::Data,
            &acceptance,
            ACCEPTED_RESULT_OUTPUT,
            &gate,
            ACCEPTED_RESULT_OUTPUT,
        )?,
        link(
            &format!("{prefix}-pass"),
            EdgeKind::Control,
            &gate,
            "pass",
            pass,
            "in",
        )?,
        link(
            &format!("{prefix}-fail"),
            EdgeKind::Control,
            &gate,
            "fail",
            fail,
            "in",
        )?,
    ])
}

pub(super) fn link(
    id: &str,
    kind: EdgeKind,
    source: &str,
    source_port: &str,
    target: &str,
    target_port: &str,
) -> EvidenceResult<Mutation> {
    Ok(Mutation::AddEdge {
        edge: Edge::new(
            EdgeId::new(id)?,
            kind,
            NodeId::new(source)?,
            PortId::new(source_port)?,
            NodeId::new(target)?,
            PortId::new(target_port)?,
        ),
    })
}

pub(super) fn terminal(id: &str, outcome: TerminalOutcome) -> EvidenceResult<Mutation> {
    Ok(Mutation::AddNode {
        node: Node::new(NodeId::new(id)?, NodeKind::Terminal { outcome })?
            .with_control_input(PortId::new("in")?)?,
    })
}
