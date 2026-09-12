//! Existing repeat, process, acceptance, and workflow-control nodes used by the binary scenario.
use std::collections::BTreeSet;

use milkdrift_blueprint::{
    BindingSource, Condition, ForkConfig, JoinConfig, JoinPolicy, PinnedSubworkflow,
    WorkflowInterface,
};
use milkdrift_capability::{BoundedJson, SideEffectClass};
use milkdrift_control::{
    ControlCommandDocument, ControllerBlueprintSpec, ControllerLimits, ResultAcceptanceContract,
    ResultRequirement, build_controller_blueprint, result_acceptance_gate, result_acceptance_task,
};

use super::super::{
    AuthorRef, BlueprintRevision, CapabilityId, CapabilityRequirement, DataPort, Edge, EdgeId,
    EdgeKind, EvidenceResult, Mutation, MutationBatch, Node, NodeId, NodeKind, OperationId, PortId,
    SchemaId, SchemaRef, TerminalOutcome, WorkflowId, control_edge, terminal_node,
};

pub(super) fn schema() -> EvidenceResult<SchemaRef> {
    Ok(SchemaRef::new(SchemaId::new("evidence.stdout")?, 1)?)
}

pub(super) fn process(
    id: &str,
    capability: &str,
    predecessor: bool,
    verifier: bool,
) -> EvidenceResult<Node> {
    let mut node = Node::new(
        NodeId::new(id)?,
        NodeKind::task_direct_inputs(
            CapabilityRequirement::new(OperationId::new("process.execute")?)
                .exact(CapabilityId::new(capability)?)
                .maximum_side_effect(SideEffectClass::ReadOnly),
        )?,
    )?
    .with_control_output(PortId::new("next")?)?
    .with_data_output(PortId::new("stdout")?, DataPort::output(schema()?))?;
    if predecessor {
        node = node.with_control_input(PortId::new("in")?)?;
    }
    if verifier {
        node = node.with_data_input(
            PortId::new("work")?,
            DataPort::input(schema()?, true, None)?,
        )?;
    }
    Ok(node)
}

pub(super) fn wait(id: &str, input: bool) -> EvidenceResult<Node> {
    let node = Node::new(
        NodeId::new(id)?,
        NodeKind::SignalWait {
            signal: OperationId::new(format!("controller.{id}"))?,
        },
    )?
    .with_control_output(PortId::new("next")?)?;
    Ok(if input {
        node.with_control_input(PortId::new("in")?)?
    } else {
        node
    })
}

fn data(id: &str, from: &str, output: &str, to: &str, input: &str) -> EvidenceResult<Mutation> {
    Ok(Mutation::AddEdge {
        edge: Edge::new(
            EdgeId::new(id)?,
            EdgeKind::Data,
            NodeId::new(from)?,
            PortId::new(output)?,
            NodeId::new(to)?,
            PortId::new(input)?,
        ),
    })
}

pub(super) fn body() -> EvidenceResult<BlueprintRevision> {
    let mut nodes = vec![
        process("work", "controller-work", false, false)?,
        process("verify", "controller-verify", true, true)?,
        wait("repair-release", true)?,
        process("repair", "controller-work", true, false)?,
        process("reverify", "controller-verify", true, true)?,
        wait("failed-again", true)?,
        terminal_node("done", TerminalOutcome::Success, true)?,
        terminal_node("unexpected", TerminalOutcome::Failure, true)?,
        terminal_node("failed", TerminalOutcome::Failure, true)?,
    ];
    let mut operations = Vec::new();
    for (prefix, source, pass, fail) in [
        ("first", "verify", "unexpected", "repair-release"),
        ("second", "reverify", "done", "failed-again"),
    ] {
        let acceptance = format!("{prefix}-acceptance");
        let gate = format!("{prefix}-gate");
        nodes.push(result_acceptance_task(
            NodeId::new(&acceptance)?,
            ResultAcceptanceContract::new(
                ResultRequirement::Verification {
                    checks: BTreeSet::from(["correct".to_owned()]),
                },
                BTreeSet::from(["work".to_owned()]),
            )?,
            schema()?,
        )?);
        nodes.push(result_acceptance_gate(
            NodeId::new(&gate)?,
            NodeId::new(&acceptance)?,
            schema()?,
        )?);
        operations.extend([
            control_edge(
                &format!("{prefix}-evaluate"),
                source,
                "next",
                &acceptance,
                "in",
            )?,
            control_edge(&format!("{prefix}-branch"), &acceptance, "out", &gate, "in")?,
            control_edge(&format!("{prefix}-pass"), &gate, "pass", pass, "in")?,
            control_edge(&format!("{prefix}-fail"), &gate, "fail", fail, "in")?,
            data(
                &format!("{prefix}-result"),
                source,
                "stdout",
                &acceptance,
                "result",
            )?,
            data(
                &format!("{prefix}-evidence"),
                if prefix == "first" { "work" } else { "repair" },
                "stdout",
                &acceptance,
                "work",
            )?,
            data(
                &format!("{prefix}-accepted"),
                &acceptance,
                "accepted_result",
                &gate,
                "accepted_result",
            )?,
        ]);
    }
    operations.extend([
        control_edge("failed-again-stop", "failed-again", "next", "failed", "in")?,
        control_edge("work-verify", "work", "next", "verify", "in")?,
        data("work-input", "work", "stdout", "verify", "work")?,
        control_edge("release-repair", "repair-release", "next", "repair", "in")?,
        control_edge("repair-reverify", "repair", "next", "reverify", "in")?,
        data("repair-input", "repair", "stdout", "reverify", "work")?,
    ]);
    let mut node_operations = nodes
        .into_iter()
        .map(|node| Mutation::AddNode { node })
        .collect::<Vec<_>>();
    node_operations.extend(operations);
    Ok(BlueprintRevision::genesis(
        WorkflowId::new("controller-evidence-body")?,
        MutationBatch::new(node_operations)?,
        AuthorRef::new(super::HUMAN)?,
        "Independent verification and prospective repair",
    )?)
}

pub(super) fn wrapper(
    body: &BlueprintRevision,
    identity: &str,
    processes: u32,
) -> EvidenceResult<BlueprintRevision> {
    Ok(build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new(identity)?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits: ControllerLimits::new(
            8, 8, 16, 8, 300_000, 1_000_000, 32_768, 32_768, 32_000_000, processes, 8, 2, 4, 2, 2,
            None,
        )?,
        author: AuthorRef::new(super::HUMAN)?,
    })?)
}

pub(super) fn root(body: &BlueprintRevision) -> EvidenceResult<BlueprintRevision> {
    let wrapper = wrapper(body, "controller-evidence", 4)?;
    let repeat = wrapper
        .semantic()
        .nodes()
        .get(&NodeId::new("controller-repeat")?)
        .ok_or("controller repeat absent")?
        .clone()
        .with_control_input(PortId::new("in")?)?;
    let fork = Node::new(
        NodeId::new("fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(BTreeSet::from([
                PortId::new("loop")?,
                PortId::new("decision")?,
            ]))?,
        },
    )?
    .with_control_output(PortId::new("loop")?)?
    .with_control_output(PortId::new("decision")?)?;
    let join = Node::new(
        NodeId::new("join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("fork")?, JoinPolicy::All),
        },
    )?
    .with_control_input(PortId::new("loop")?)?
    .with_control_input(PortId::new("decision")?)?
    .with_control_output(PortId::new("out")?)?;
    Ok(wrapper.revise(
        wrapper.id(),
        MutationBatch::new(vec![
            Mutation::ReplaceNode { node: repeat },
            Mutation::ReplaceNode {
                node: terminal_node("controller-complete", TerminalOutcome::Failure, true)?,
            },
            Mutation::AddNode { node: fork },
            Mutation::AddNode { node: join },
            Mutation::RemoveEdge {
                edge: EdgeId::new("controller-finished")?,
            },
            Mutation::AddNode {
                node: wait("authorize-proposer", true)?,
            },
            Mutation::AddNode {
                node: wait("proposer", true)?,
            },
            Mutation::AddNode {
                node: wait("proposer-finished", true)?,
            },
            control_edge("fork-loop", "fork", "loop", "controller-repeat", "in")?,
            control_edge(
                "fork-decision",
                "fork",
                "decision",
                "authorize-proposer",
                "in",
            )?,
            control_edge("loop-join", "controller-repeat", "out", "join", "loop")?,
            control_edge(
                "decision-join",
                "proposer-finished",
                "next",
                "join",
                "decision",
            )?,
            control_edge("join-complete", "join", "out", "controller-complete", "in")?,
            control_edge(
                "release-proposer",
                "authorize-proposer",
                "next",
                "proposer",
                "in",
            )?,
            control_edge(
                "proposer-finished",
                "proposer",
                "next",
                "proposer-finished",
                "in",
            )?,
        ])?,
        AuthorRef::new(super::HUMAN)?,
        "Explicit controller decision boundary",
    )?)
}

pub(super) fn prelude_root(
    body: &BlueprintRevision,
) -> EvidenceResult<(BlueprintRevision, BlueprintRevision)> {
    let base = wrapper(body, "controller-unaccounted-prelude", 4)?;
    let repeat = base.semantic().nodes()[&NodeId::new("controller-repeat")?]
        .clone()
        .with_control_input(PortId::new("in")?)?;
    let revised = base.revise(
        base.id(),
        MutationBatch::new(vec![
            Mutation::ReplaceNode { node: repeat },
            Mutation::AddNode {
                node: Node::new(
                    NodeId::new("prelude")?,
                    NodeKind::Subworkflow {
                        reference: PinnedSubworkflow::new(
                            body.semantic().workflow().clone(),
                            body.id().clone(),
                            WorkflowInterface::new([], [])?,
                        ),
                    },
                )?
                .with_control_output(PortId::new("out")?)?,
            },
            control_edge(
                "prelude-repeat",
                "prelude",
                "out",
                "controller-repeat",
                "in",
            )?,
        ])?,
        AuthorRef::new(super::HUMAN)?,
        "A child before activation must not escape cumulative accounting",
    )?;
    Ok((base, revised))
}

pub(super) fn proposer(command: &ControlCommandDocument) -> EvidenceResult<Node> {
    Ok(Node::new(
        NodeId::new("proposer")?,
        NodeKind::task_direct_inputs(
            CapabilityRequirement::new(OperationId::new(
                milkdrift_control::WORKFLOW_PROPOSE_OPERATION,
            )?)
            .exact(CapabilityId::new("milkdrift-workflow-control")?)
            .maximum_side_effect(SideEffectClass::IdempotentWrite),
        )?,
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("next")?)?
    .with_data_input(
        PortId::new("milkdrift.control_request")?,
        DataPort::input(
            schema()?,
            true,
            Some(BindingSource::Literal {
                value: BoundedJson::new(serde_json::to_value(command)?)?,
            }),
        )?,
    )?
    .with_data_output(PortId::new("control_result")?, DataPort::output(schema()?))?)
}
