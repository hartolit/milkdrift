//! Installed-daemon evidence for racing process admissions and unknown model metering.
use std::{collections::BTreeSet, path::Path, sync::atomic::Ordering, time::Duration};

use milkdrift_authority::{NetworkProfileRef, NetworkScope};
use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, ForkConfig, JoinConfig, JoinPolicy, Mutation, MutationBatch,
    Node, NodeId, NodeKind, PortId, TerminalOutcome, WorkflowId,
};
use milkdrift_daemon::{DaemonConfig, ModelProfileConfig};
use milkdrift_evidence::{
    EvidenceResult,
    application::{CliRunner, ensure, required_text, wait_for_run},
};
use serde_json::{Value, json};

use super::{fs, import, wait_for_child, workflow};

pub(super) fn prelude(
    runner: &CliRunner,
    directory: &Path,
    body: &BlueprintRevision,
    mode: &str,
) -> EvidenceResult {
    let (base, root) = workflow::prelude_root(body)?;
    import(runner, directory, &format!("prelude-base-{mode}"), &base)?;
    import(runner, directory, &format!("prelude-root-{mode}"), &root)?;
    let run = format!("run-prelude-{mode}");
    let before = super::children(runner, body.semantic().workflow().as_str())?;
    let result = runner.run(
        &[
            "run",
            "start",
            &run,
            root.semantic().workflow().as_str(),
            root.id().as_str(),
        ],
        None,
    )?;
    ensure(
        !result.status.success(),
        "unaccounted prelude start was accepted",
    )?;
    let read = runner.success(&["run", "show", &run])?;
    ensure(
        read["value"]["lifecycle"] == "created"
            && read["value"]["controller_accounting"]["state"] == "inactive"
            && super::children(runner, body.semantic().workflow().as_str())? == before,
        "refused prelude created or entered an unaccounted child",
    )?;
    fs::write(
        directory.join(format!("prelude-refusal-{mode}.json")),
        serde_json::to_vec_pretty(&read)?,
    )?;
    Ok(())
}

pub(super) struct Models {
    mock: super::super::setup::MockModel,
    profiles: Vec<Value>,
}

impl Models {
    pub(super) fn configure(
        arguments: &super::super::Arguments,
        directory: &Path,
        config: &mut DaemonConfig,
    ) -> EvidenceResult<Self> {
        let mock = super::super::setup::MockModel::start()?;
        let mut fixture: Value = serde_json::from_slice(&fs::read(
            arguments
                .examples
                .join("../local-model/openai-compatible-loopback.example.json"),
        )?)?;
        fixture["identity"] = json!("controller-metering-fixture");
        fixture["base_url"] = json!(format!("http://{}", mock.address));
        fixture["model"] = json!("operator-model");
        let mut profiles = vec![fixture];
        for path in &arguments.controller_model_profile {
            profiles.push(serde_json::from_slice(&fs::read(path)?)?);
        }
        ensure(
            profiles.len() <= 4,
            "model refusal fixture is bounded to four profiles",
        )?;
        let mut identities = BTreeSet::new();
        let mut destinations = BTreeSet::new();
        for (index, profile) in profiles.iter().enumerate() {
            let identity = required_text(profile, &["identity"])?;
            let endpoint = url::Url::parse(&required_text(profile, &["base_url"])?)?;
            identities.insert(NetworkProfileRef::new(identity)?);
            destinations.insert(format!(
                "{}:{}",
                endpoint.host_str().ok_or("model host absent")?,
                endpoint
                    .port_or_known_default()
                    .ok_or("model port absent")?
            ));
            let path = directory.join(format!("refused-model-profile-{index}.json"));
            fs::write(&path, serde_json::to_vec_pretty(profile)?)?;
            config.adapters.model_profiles.push(ModelProfileConfig {
                capability_id: format!("controller-model-{index}"),
                profile: path,
            });
        }
        config.actors[0].authority.resources.network = NetworkScope::new(identities, destinations)?;
        Ok(Self { mock, profiles })
    }

    pub(super) fn exercise(
        &self,
        arguments: &super::super::Arguments,
        runner: &CliRunner,
        directory: &Path,
    ) -> EvidenceResult {
        let template: Value =
            serde_json::from_slice(&fs::read(arguments.examples.join("model.json"))?)?;
        for (index, profile) in self.profiles.iter().enumerate() {
            let identity = format!("controller-unknown-model-{index}");
            let mut task = template["revision"]["semantic"]["nodes"]["model"].clone();
            task["kind"]["config"]["requirement"]["exact_capability"] =
                json!(format!("controller-model-{index}"));
            task["kind"]["config"]["requirement"]["provider_profile"] = profile["identity"].clone();
            let body = BlueprintRevision::genesis(
                WorkflowId::new(format!("{identity}-body"))?,
                MutationBatch::new(vec![
                    Mutation::AddNode {
                        node: serde_json::from_value(task)?,
                    },
                    Mutation::AddNode {
                        node: super::super::terminal_node("done", TerminalOutcome::Success, true)?,
                    },
                    super::super::control_edge("model-done", "model", "next", "done", "in")?,
                ])?,
                AuthorRef::new(super::HUMAN)?,
                "Unknown metering must refuse before transport entry",
            )?;
            let root = workflow::wrapper(&body, &identity, 4)?;
            import(runner, directory, &format!("{identity}-body"), &body)?;
            import(runner, directory, &identity, &root)?;
            runner.success(&[
                "run",
                "start",
                &identity,
                root.semantic().workflow().as_str(),
                root.id().as_str(),
            ])?;
            let child = wait_for_child(runner, body.semantic().workflow().as_str())?;
            let read = wait_for_run(runner, &child, Duration::from_secs(20), |read| {
                read["value"]["terminal"] == "failed"
            })?;
            let inspected = super::inspect_node(runner, &child, &read, "model")?;
            let attempt = &inspected["value"];
            let detail = required_text(attempt, &["terminal_detail"])?;
            ensure(
                detail.contains("Unknown") && detail.contains("input_units"),
                "model did not refuse unknown hard usage bounds",
            )?;
            let accounting = &read["value"]["controller_accounting"];
            ensure(
                accounting["committed"]["model_admissions"] == 0
                    && accounting["account"]["reservations"]
                        .as_object()
                        .is_some_and(|v| v.is_empty())
                    && attempt["uncertain"] == false,
                "refused model entered or reserved external usage",
            )?;
            let report = json!({"qualifying":false, "negative_evidence":"unknown hard input/output/cost bounds refuse before transport entry", "profile":profile,
                "requested_output_units":64, "effective_server_settings":"unknown", "attempt":attempt, "accounting":accounting});
            fs::write(
                directory.join(format!("model-refusal-{index}.json")),
                serde_json::to_vec_pretty(&report)?,
            )?;
        }
        ensure(
            self.mock.invocations.load(Ordering::SeqCst) == 0,
            "metering refusal contacted the deterministic model endpoint",
        )?;
        Ok(())
    }
}

pub(super) fn race(runner: &CliRunner, directory: &Path) -> EvidenceResult {
    let ports = ["one", "two", "three"];
    let fork = Node::new(
        NodeId::new("fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(
                ports
                    .iter()
                    .map(|id| PortId::new(*id))
                    .collect::<Result<_, _>>()?,
            )?,
        },
    )?;
    let join = Node::new(
        NodeId::new("join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("fork")?, JoinPolicy::All),
        },
    )?
    .with_control_output(PortId::new("out")?)?;
    let (mut fork, mut join) = (fork, join);
    let mut changes = Vec::new();
    for id in ports {
        fork = fork.with_control_output(PortId::new(id)?)?;
        join = join.with_control_input(PortId::new(id)?)?;
        changes.extend([
            Mutation::AddNode {
                node: workflow::process(id, "controller-race", true, false)?,
            },
            super::super::control_edge(&format!("fork-{id}"), "fork", id, id, "in")?,
            super::super::control_edge(&format!("join-{id}"), id, "next", "join", id)?,
        ]);
    }
    changes.extend([
        Mutation::AddNode { node: fork },
        Mutation::AddNode { node: join },
        Mutation::AddNode {
            node: super::super::terminal_node("done", TerminalOutcome::Success, true)?,
        },
        super::super::control_edge("join-done", "join", "out", "done", "in")?,
    ]);
    let body = BlueprintRevision::genesis(
        WorkflowId::new("controller-race-body")?,
        MutationBatch::new(changes)?,
        AuthorRef::new(super::HUMAN)?,
        "Three concurrent descendants compete for two process admissions",
    )?;
    let root = workflow::wrapper(&body, "controller-race", 2)?;
    import(runner, directory, "race-body", &body)?;
    import(runner, directory, "race-root", &root)?;
    runner.success(&[
        "run",
        "start",
        "run-controller-race",
        root.semantic().workflow().as_str(),
        root.id().as_str(),
    ])?;
    let child = wait_for_child(runner, body.semantic().workflow().as_str())?;
    let reserved = wait_for_run(runner, &child, Duration::from_secs(40), |read| {
        read["value"]["controller_accounting"]["committed"]["process_admissions"] == 2
            && read["value"]["controller_accounting"]["account"]["reservations"]
                .as_object()
                .is_some_and(|values| values.len() == 2)
    });
    fs::write(
        directory.join("race-health.json"),
        serde_json::to_vec_pretty(&runner.success(&["daemon", "health"])?)?,
    )?;
    let reserved = reserved?;
    ensure(
        reserved["value"]["controller_accounting"]["remaining"]["process_admissions"] == 0,
        "racing reservations did not consume the last allowance",
    )?;
    wait_for_run(
        runner,
        "run-controller-race",
        Duration::from_secs(40),
        |read| read["value"]["terminal"] == "failed",
    )?;
    let mut admitted = 0;
    let mut denied = 0;
    for name in ports {
        let attempt = super::inspect_node(runner, &child, &reserved, name)?;
        let value = &attempt["value"];
        if value["terminal_detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("process_admissions"))
        {
            denied += 1;
        }
        if value["outputs"]
            .as_array()
            .is_some_and(|outputs| !outputs.is_empty())
        {
            admitted += 1;
        }
    }
    ensure(
        admitted == 2 && denied == 1,
        "racing workers did not produce exactly two entries and one budget denial",
    )?;
    let settled = runner.success(&["run", "show", &child])?;
    ensure(
        settled["value"]["controller_accounting"]["committed"]["process_admissions"] == 2,
        "settlement reset or duplicated the racing allowance",
    )?;
    fs::write(
        directory.join("concurrent-admission.json"),
        serde_json::to_vec_pretty(
            &json!({"reserved":reserved, "settled":settled, "admitted":admitted, "denied":denied}),
        )?,
    )?;
    Ok(())
}

pub(super) fn entered_process(runner: &CliRunner, directory: &Path) -> EvidenceResult<Value> {
    let mut work =
        serde_json::to_value(workflow::process("work", "controller-crash", false, false)?)?;
    work["kind"]["config"]["requirement"]["maximum_side_effect"] = json!("non_idempotent_write");
    let body = BlueprintRevision::genesis(
        WorkflowId::new("controller-crash-body")?,
        MutationBatch::new(vec![
            Mutation::AddNode {
                node: serde_json::from_value(work)?,
            },
            Mutation::AddNode {
                node: super::super::terminal_node("done", TerminalOutcome::Success, true)?,
            },
            super::super::control_edge("work-done", "work", "next", "done", "in")?,
        ])?,
        AuthorRef::new(super::HUMAN)?,
        "Crash an entered process while its artifact obligation is reserved",
    )?;
    let root = workflow::wrapper(&body, "controller-crash", 1)?;
    import(runner, directory, "crash-body", &body)?;
    import(runner, directory, "crash-root", &root)?;
    runner.success(&[
        "run",
        "start",
        "run-controller-crash",
        root.semantic().workflow().as_str(),
        root.id().as_str(),
    ])?;
    let child = wait_for_child(runner, body.semantic().workflow().as_str())?;
    wait_for_run(runner, &child, Duration::from_secs(40), |read| {
        read["value"]["controller_accounting"]["committed"]["process_admissions"] == 1
            && super::node(read, "work").is_some_and(|node| {
                node["latest_attempt"]["entry_authorization"]["allowed"] == true
                    && node["latest_attempt"]["progress_observations"]
                        .as_u64()
                        .is_some_and(|count| count > 0)
                    && node["latest_attempt"]["terminal"].is_null()
            })
    })
}
