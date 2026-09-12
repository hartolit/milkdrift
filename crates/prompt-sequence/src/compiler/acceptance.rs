//! Insert purpose-specific review gates in initial and prospective stage graphs.

use super::{
    APPROVED, BTreeSet, CONTROL_IN, CONTROL_OUT, Edge, EdgeKind, FAIL, Mutation, NodeId, PASS,
    PromptSequenceError, add_edge, approval_wait_node, artifact_schema, compilation, control_edge,
    failure_terminal, gate_node, node_id, port,
};
pub(super) fn install_review_acceptance(
    operations: &mut Vec<Mutation>,
    reviewer: &NodeId,
) -> Result<(), PromptSequenceError> {
    let acceptance = node_id(format!("{reviewer}-acceptance"))?;
    let gate = node_id(format!("{reviewer}-gate"))?;
    let rejected = node_id(format!("{reviewer}-rejected"))?;
    let stopped = node_id(format!("{reviewer}-rejected-stop"))?;
    for operation in operations.iter_mut() {
        if let Mutation::AddEdge { edge } = operation
            && edge.kind() == EdgeKind::Control
            && edge.source_node() == reviewer
        {
            *edge = Edge::new(
                edge.id().clone(),
                EdgeKind::Control,
                gate.clone(),
                port(PASS)?,
                edge.target_node().clone(),
                edge.target_port().clone(),
            );
        }
    }
    operations.extend([
        Mutation::AddNode {
            node: milkdrift_control::result_acceptance_task(
                acceptance.clone(),
                milkdrift_control::ResultAcceptanceContract::new(
                    milkdrift_control::ResultRequirement::ReviewProse,
                    BTreeSet::new(),
                )
                .map_err(|error| compilation(error.to_string()))?,
                artifact_schema()?,
            )
            .map_err(|error| compilation(error.to_string()))?,
        },
        Mutation::AddNode {
            node: gate_node(&acceptance, &gate)?,
        },
        Mutation::AddNode {
            node: approval_wait_node(&rejected)?,
        },
        Mutation::AddNode {
            node: failure_terminal(&stopped)?,
        },
        control_edge(
            &format!("{reviewer}-accept"),
            reviewer,
            CONTROL_OUT,
            &acceptance,
            CONTROL_IN,
        )?,
        control_edge(
            &format!("{reviewer}-accept-gate"),
            &acceptance,
            CONTROL_OUT,
            &gate,
            CONTROL_IN,
        )?,
        control_edge(
            &format!("{reviewer}-accept-failed"),
            &gate,
            FAIL,
            &rejected,
            CONTROL_IN,
        )?,
        control_edge(
            &format!("{reviewer}-rejected-stop"),
            &rejected,
            APPROVED,
            &stopped,
            CONTROL_IN,
        )?,
        add_edge(
            &format!("{reviewer}-result-acceptance"),
            EdgeKind::Data,
            reviewer,
            "review",
            &acceptance,
            "result",
        )?,
        add_edge(
            &format!("{reviewer}-accepted-gate"),
            EdgeKind::Data,
            &acceptance,
            milkdrift_control::ACCEPTED_RESULT_OUTPUT,
            &gate,
            milkdrift_control::ACCEPTED_RESULT_OUTPUT,
        )?,
    ]);
    Ok(())
}
