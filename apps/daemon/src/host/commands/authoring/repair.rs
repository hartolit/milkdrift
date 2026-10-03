//! Prepare a proposal without changing the run. Submission remains the control service's job.

use super::{
    ActorSession, BuildResult, Owner, PublicFailure, failure,
    graph::{ModelWorkflow, add_edge, artifact},
};
use milkdrift_authority::AuthorityOperation;
use milkdrift_blueprint::{
    AuthorRef, BindingSource, BlueprintRevision, BlueprintRevisionDocument, DataPort, Edge,
    EdgeKind, Mutation, MutationBatch, NodeId, PathSelector, PortId,
};
use milkdrift_control::{
    ClaimedStopCondition, ProposalApplicationPolicy, ProposalId, ProposalProvenance,
    WorkflowProposal, WorkflowProposalDocument,
};
use milkdrift_control_protocol::{BlueprintEdit, CommandAccepted, CommandRequest, ModelRepair};
use milkdrift_persistence::{EvidenceId, EvidenceKind, EvidenceReference, RunSequence};
use milkdrift_runtime::{NodeExecutionState, RunLifecycle};
use milkdrift_workspace::RunId;
use serde_json::json;

pub(in crate::host::commands) fn prepare(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    run: &str,
    proposal: &str,
    repair: &ModelRepair,
) -> Result<CommandAccepted, PublicFailure> {
    owner.authorize_run_read(
        session,
        run,
        AuthorityOperation::Propose,
        "prepare:model-repair",
    )?;
    let state = owner.run_read(session, run)?;
    if request.expected_sequence != Some(state.sequence)
        || request.expected_revision != state.revision_id
    {
        return Err(super::super::super::read_model::conflict(
            "repair requires the exact current sequence and revision",
        ));
    }
    if state.terminal.is_some() {
        return Err(super::super::super::invalid(
            "this run has ended; start an explicitly linked new run",
        ));
    }
    let run_id = RunId::new(run).map_err(failure)?;
    let projection = owner
        .workflow()?
        .runtime
        .projection(&run_id)
        .map_err(failure)?;
    let hold = format!("author.{}.hold", repair.failed_step);
    if projection.lifecycle() != RunLifecycle::Paused
        || !projection.node_executions().values().any(|node| {
            node.node().as_str() == hold && node.state() == &NodeExecutionState::Eligible
        })
    {
        return Err(super::super::super::invalid(
            "pause at the selected failed-result review hold before preparing repair",
        ));
    }
    let accept = format!("author.{}.accept", repair.failed_step);
    let attempt = state
        .nodes
        .iter()
        .find(|node| node.node_id == accept)
        .and_then(|node| node.latest_attempt_id.as_ref())
        .ok_or_else(|| failure("failed completeness check is unavailable"))?;
    if owner
        .attempt_read(session, run, attempt)?
        .result_acceptance
        .is_none_or(|result| result.accepted)
    {
        return Err(failure(
            "repair requires an authorized rejected completeness decision",
        ));
    }
    let base_id = state
        .revision_id
        .as_deref()
        .ok_or_else(|| failure("run revision absent"))?;
    let base_read = owner.revision(session, base_id)?;
    let (_, base) = BlueprintRevisionDocument::from_json(
        &serde_json::to_vec(&base_read.document).map_err(failure)?,
    )
    .map_err(failure)?;
    let mutation = build(owner, session, &base, repair).map_err(failure)?;
    let proposal = WorkflowProposal::new(
        ProposalId::new(proposal).map_err(failure)?,
        session.actor.clone(),
        ProposalProvenance::Direct,
        base.semantic().workflow().clone(),
        Some(run_id),
        base.id().clone(),
        base.content_digest().clone(),
        Some(RunSequence::new(state.sequence)),
        mutation,
        request.reason.clone(),
        None,
        vec!["Completed work and the failed result check remain unchanged".into()],
        vec!["The final failed-result hold has not been signalled".into()],
        vec![EvidenceReference {
            id: EvidenceId::new(attempt).map_err(failure)?,
            kind: EvidenceKind::WorkerObservation,
        }],
        vec![],
        ProposalApplicationPolicy::RequireApproval,
        None,
        ClaimedStopCondition::Continue,
    )
    .map_err(failure)?;
    Ok(CommandAccepted {
        command_id: request.command_id.clone(),
        replayed: false,
        resulting_sequence: None,
        result_type: "model_repair_prepared".into(),
        value: json!({
            "document": serde_json::from_slice::<serde_json::Value>(&WorkflowProposalDocument::new(proposal).to_canonical_json().map_err(failure)?).map_err(failure)?,
            "base_revision":base_id, "observed_sequence":state.sequence,
            "summary":format!("Insert {} after {}; preserve the failed check and require a fresh completeness check", repair.repair_step, hold),
            "signal_type":"workflow.reviewed"
        }),
    })
}

fn build(
    owner: &Owner,
    session: &ActorSession,
    base: &BlueprintRevision,
    repair: &ModelRepair,
) -> BuildResult<MutationBatch> {
    let original = ModelWorkflow::read(base)?;
    let failed = original
        .steps
        .last()
        .filter(|step| step.id == repair.failed_step)
        .ok_or("repair convenience supports the final model step only")?;
    let (_, output) = original
        .output
        .as_ref()
        .filter(|(step, _)| step == &failed.id)
        .ok_or("final output must belong to the held model step")?;
    let mut replacement = ModelWorkflow::empty(&original.name);
    replacement.inputs = original.inputs.clone();
    replacement.edit(
        owner,
        session,
        &BlueprintEdit::AddModel {
            step: repair.repair_step.clone(),
            capability: repair.capability.clone(),
            prompt: repair.prompt.clone(),
            maximum_output_units: repair.maximum_output_units,
        },
    )?;
    replacement.steps[0].inputs = failed
        .inputs
        .iter()
        .filter(|(_, source)| matches!(source, BindingSource::WorkflowInput { .. }))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    replacement.output = Some((repair.repair_step.clone(), output.clone()));
    replacement.complete()?;
    let generated = replacement.build(
        base.semantic().workflow().clone(),
        AuthorRef::new(session.actor.as_str())?,
        "prepare future model repair",
    )?;
    let hold = format!("author.{}.hold", failed.id);
    let failed_terminal = format!("author.{}.failed", failed.id);
    let done = NodeId::new(format!("author.{}.done", repair.repair_step))?;
    let mut operations = vec![];
    for edge in base
        .semantic()
        .edges()
        .values()
        .filter(|edge| edge.source_node().as_str() == hold)
    {
        operations.push(Mutation::RemoveEdge {
            edge: edge.id().clone(),
        });
    }
    operations.push(Mutation::RemoveNode {
        node: NodeId::new(failed_terminal)?,
    });
    for node in generated.semantic().nodes().values() {
        let mut node = node.clone();
        if node.id().as_str() == "author.done" {
            let mut terminal = milkdrift_blueprint::Node::new(done.clone(), node.kind().clone())?;
            for port in node.control_inputs() {
                terminal = terminal.with_control_input(port.clone())?;
            }
            for (name, port) in node.data_inputs() {
                terminal = terminal.with_data_input(name.clone(), port.clone())?;
            }
            node = terminal;
        } else if node.id().as_str() == repair.repair_step {
            node = node
                .with_control_input(PortId::new("in")?)?
                .with_data_input(
                    PortId::new("failed_result")?,
                    DataPort::input(
                        artifact()?,
                        true,
                        Some(BindingSource::NodeOutput {
                            node: NodeId::new(&failed.id)?,
                            port: PortId::new("model_response")?,
                            path: PathSelector::new(vec![])?,
                        }),
                    )?,
                )?;
        }
        if base.semantic().nodes().contains_key(node.id()) {
            return Err("repair step identity already exists".into());
        }
        operations.push(Mutation::AddNode { node });
    }
    let mut edges = vec![];
    for edge in generated.semantic().edges().values() {
        edges.push(Edge::new(
            edge.id().clone(),
            edge.kind(),
            edge.source_node().clone(),
            edge.source_port().clone(),
            if edge.target_node().as_str() == "author.done" {
                done.clone()
            } else {
                edge.target_node().clone()
            },
            edge.target_port().clone(),
        ));
    }
    add_edge(
        &mut edges,
        EdgeKind::Control,
        &hold,
        "out",
        &repair.repair_step,
        "in",
    )?;
    add_edge(
        &mut edges,
        EdgeKind::Data,
        &failed.id,
        "model_response",
        &repair.repair_step,
        "failed_result",
    )?;
    operations.extend(edges.into_iter().map(|edge| Mutation::AddEdge { edge }));
    let mutation = MutationBatch::new(operations)?;
    base.revise(
        base.id(),
        mutation.clone(),
        AuthorRef::new(session.actor.as_str())?,
        "validate repair",
    )?;
    // Rich governed definitions refuse the editor recognizer; ordinary proposal admission still
    // owns authority delta, protected agreements, risk and the prospective plan.
    Ok(mutation)
}
