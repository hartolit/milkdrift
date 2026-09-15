//! Real separate run/actor and sibling-scope sources for continuation refusals.
use super::*;

pub(super) fn select_evidence(mut operations: Vec<Mutation>) -> TestResult<Vec<Mutation>> {
    let bytes = b"PRIOR_SELECTED_EVIDENCE";
    let reference = milkdrift_workspace::ArtifactReference::new(
        ArtifactId::new("unrelated-response")?,
        ContentDigest::for_bytes(bytes),
        milkdrift_workspace::MediaType::new("text/plain")?,
        bytes.len() as u64,
    );
    for operation in &mut operations {
        if let Mutation::AddNode { node } = operation
            && node.id().as_str() == "model"
        {
            let NodeKind::Task { config } = node.kind() else {
                return Err("model is not a task".into());
            };
            let policy = config.context_policy().clone().with_exact_sources(
                BTreeSet::new(),
                BTreeSet::new(),
                BTreeSet::from([serde_json::to_string(&ContextSource::Artifact {
                    reference: reference.clone(),
                })?]),
            )?;
            let mut replacement = Node::new(
                node.id().clone(),
                NodeKind::Task {
                    config: milkdrift_blueprint::TaskConfig::new(
                        config.requirement().clone(),
                        policy,
                    )?,
                },
            )?
            .with_control_output(PortId::new("out")?)?;
            for (port, input) in node.data_inputs() {
                replacement = replacement.with_data_input(port.clone(), input.clone())?;
            }
            for (port, output) in node.data_outputs() {
                replacement = replacement.with_data_output(port.clone(), output.clone())?;
            }
            *node = replacement;
        }
    }
    Ok(operations)
}

pub(super) fn foreign_history(
    fixture: &ModelFixture,
) -> TestResult<Vec<milkdrift_persistence::RunEventEnvelope>> {
    use milkdrift_workspace::{ScopeId, WorkspaceScope};
    let projection = fixture.runtime.projection(&fixture.run)?;
    let revision = fixture
        .store
        .revision(projection.revision().ok_or("revision missing")?)?
        .ok_or("revision missing")?;
    let run = RunId::new("other-actor-run")?;
    let actor = ActorRef::new("human:other")?;
    let claim = CommandAuthorityClaim::new(
        GrantId::new("grant:other")?,
        1,
        GrantDigest::new(format!("b3_{}", "1".repeat(64)))?,
        0,
    )?;
    for command in [
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(run.clone(), ScopeId::new("other-root")?),
            workspace_budget: projection
                .workspace_budget()
                .ok_or("budget missing")?
                .clone(),
            inputs: Vec::new(),
        },
        RunCommand::StartRun,
    ] {
        let document = fixture.runtime.command(
            run.clone(),
            actor.clone(),
            fixture.store.head(&run)?,
            Reason::new("separate actor work")?,
            Vec::new(),
            command,
        )?;
        let result = fixture
            .runtime
            .handle_authorized_command(&document, &claim)?;
        assert_eq!(
            result.result().disposition(),
            milkdrift_persistence::CommandDisposition::Accepted
        );
    }
    fixture.runtime.scheduler_tick()?;
    for effect in fixture.runtime.claim_execution_effects(PageSize::new(8)?)? {
        fixture.runtime.execute_effect(effect)?;
    }
    Ok(fixture.runtime.history(&run)?)
}

pub(super) fn separate_branches(mut operations: Vec<Mutation>) -> TestResult<Vec<Mutation>> {
    for operation in &mut operations {
        if let Mutation::AddNode { node } = operation
            && node.id().as_str() == "model"
        {
            *node = node.clone().with_control_input(PortId::new("in")?)?;
        }
    }
    operations.retain(|operation| !matches!(operation, Mutation::AddEdge { edge } if edge.id().as_str() == "model-hold"));
    operations.push(Mutation::AddNode {
        node: Node::new(
            NodeId::new("fork")?,
            NodeKind::Fork {
                config: milkdrift_blueprint::ForkConfig::new(BTreeSet::from([
                    PortId::new("left")?,
                    PortId::new("right")?,
                ]))?,
            },
        )?
        .with_control_output(PortId::new("left")?)?
        .with_control_output(PortId::new("right")?)?,
    });
    operations.push(Mutation::AddNode {
        node: Node::new(
            NodeId::new("left-wait")?,
            NodeKind::SignalWait {
                signal: OperationId::new("private.hold")?,
            },
        )?
        .with_control_input(PortId::new("in")?)?
        .with_control_output(PortId::new("out")?)?,
    });
    operations.push(Mutation::AddNode {
        node: Node::new(
            NodeId::new("left-end")?,
            NodeKind::Terminal {
                outcome: TerminalOutcome::Success,
            },
        )?
        .with_control_input(PortId::new("in")?)?,
    });
    for (id, source, port, target) in [
        ("fork-model", "fork", "left", "model"),
        ("fork-hold", "fork", "right", "hold"),
        ("model-left-wait", "model", "out", "left-wait"),
        ("left-wait-end", "left-wait", "out", "left-end"),
    ] {
        operations.push(Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new(id)?,
                EdgeKind::Control,
                NodeId::new(source)?,
                PortId::new(port)?,
                NodeId::new(target)?,
                PortId::new("in")?,
            ),
        });
    }
    Ok(operations)
}
