use milkdrift_persistence::{
    NodeOutcome, ReconciliationAction, ReconciliationClassification, RunEventEnvelope,
    RunEventKind, RunOutcome,
};

use crate::RuntimeError;

use super::helpers::{ensure_unique, invalid_at};
use super::node::NodeExecutionState;
use super::run::{
    RevisionPin, RunCancellation, RunLifecycle, RunProjection, RunTerminalProjection,
    RunTerminationIntent,
};

impl RunProjection {
    pub(super) fn apply_lifecycle_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::RunCreated {
                workflow,
                revision,
                revision_digest,
                root_scope,
                workspace_budget,
                inputs,
            } => {
                if self.lifecycle != RunLifecycle::Uncreated {
                    return Err(invalid_at(event, "run creation may occur exactly once"));
                }
                if root_scope.reference().run() != event.run_id()
                    || !root_scope.kind().is_run_root()
                    || root_scope.parent().is_some()
                {
                    return Err(invalid_at(
                        event,
                        "run creation requires a parentless root scope owned by the envelope run",
                    ));
                }
                ensure_unique(inputs, event, "run input reference")?;
                if u64::try_from(inputs.len())
                    .map_err(|_| invalid_at(event, "input count overflow"))?
                    > workspace_budget.max_value_versions()
                {
                    return Err(invalid_at(
                        event,
                        "run inputs exceed the workspace value budget",
                    ));
                }
                for input in inputs {
                    if input.scope() != root_scope.reference() {
                        return Err(invalid_at(
                            event,
                            "every run input must belong to the root scope",
                        ));
                    }
                }
                self.run_id = Some(event.run_id().clone());
                self.lifecycle = RunLifecycle::Created;
                self.workflow = Some(workflow.clone());
                self.revision = Some(revision.clone());
                self.revision_digest = Some(revision_digest.clone());
                self.pins.clear();
                self.pins.push(RevisionPin {
                    revision: revision.clone(),
                    digest: revision_digest.clone(),
                    effective_sequence: sequence,
                    plan: None,
                });
                self.root_scope = Some(root_scope.clone());
                self.workspace_budget = Some(workspace_budget.clone());
                self.inputs = inputs.clone();
                self.scopes
                    .insert(root_scope.reference().clone(), root_scope.clone());
                for input in inputs {
                    self.record_workspace_value(input, event)?;
                }
            }
            RunEventKind::RevisionPinned {
                previous,
                revision,
                revision_digest,
                plan,
            } => {
                if self.pending_pin.as_ref() != Some(plan)
                    || self.revision.as_ref() != Some(previous)
                    || previous == revision
                {
                    return Err(invalid_at(
                        event,
                        "revision pin does not match the immediately preceding applied plan",
                    ));
                }
                let recorded =
                    self.reconciliation.plans.get(plan).ok_or_else(|| {
                        invalid_at(event, "revision pin references an unknown plan")
                    })?;
                if recorded.from_revision != *previous || recorded.to_revision != *revision {
                    return Err(invalid_at(
                        event,
                        "revision pin differs from its immutable plan",
                    ));
                }
                let retired_epoch_nodes: Vec<_> = recorded
                    .items
                    .iter()
                    .filter(|item| {
                        matches!(
                            item.classification,
                            ReconciliationClassification::Added
                                | ReconciliationClassification::ChangedPending
                        ) && item.action == ReconciliationAction::UseNewOnNextInvocation
                    })
                    .filter_map(|item| item.node.clone())
                    .collect();
                let completed_to_reconsider: Vec<_> = recorded
                    .items
                    .iter()
                    .filter_map(|item| item.execution.as_ref())
                    .filter(|execution| {
                        self.node_executions
                            .get(*execution)
                            .is_some_and(|execution| {
                                execution.state
                                    == NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                            })
                    })
                    .cloned()
                    .collect();
                self.revision = Some(revision.clone());
                self.revision_digest = Some(revision_digest.clone());
                self.pins.clear();
                self.pins.push(RevisionPin {
                    revision: revision.clone(),
                    digest: revision_digest.clone(),
                    effective_sequence: sequence,
                    plan: Some(plan.clone()),
                });
                for execution in self.node_executions.values_mut() {
                    if retired_epoch_nodes.contains(&execution.node)
                        && execution.epoch_retired_sequence.is_none()
                    {
                        execution.epoch_retired_sequence = Some(sequence);
                    }
                }
                for execution in self.settled_node_executions.values_mut() {
                    if retired_epoch_nodes.contains(&execution.node)
                        && execution.epoch_retired_sequence.is_none()
                    {
                        execution.epoch_retired_sequence = Some(sequence);
                    }
                }
                self.pending_successor_executions
                    .extend(completed_to_reconsider);
                self.pending_pin = None;
            }
            RunEventKind::RunStarted => {
                if self.lifecycle != RunLifecycle::Created {
                    return Err(invalid_at(event, "only a created run may start"));
                }
                self.lifecycle = RunLifecycle::Running;
            }
            RunEventKind::RunPaused { .. } => {
                if self.lifecycle != RunLifecycle::Running {
                    return Err(invalid_at(event, "only a running run may pause"));
                }
                self.lifecycle = RunLifecycle::Paused;
            }
            RunEventKind::RunResumed { .. } => {
                if self.lifecycle != RunLifecycle::Paused {
                    return Err(invalid_at(event, "only a paused run may resume"));
                }
                self.lifecycle = RunLifecycle::Running;
            }
            RunEventKind::RunCancellationRequested { reason, evidence } => {
                if !matches!(
                    self.lifecycle,
                    RunLifecycle::Created | RunLifecycle::Running | RunLifecycle::Paused
                ) || self.cancellation.is_some()
                {
                    return Err(invalid_at(
                        event,
                        "cancellation intent is duplicate or out of state",
                    ));
                }
                self.cancellation = Some(RunCancellation {
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    sequence,
                });
                self.lifecycle = RunLifecycle::Cancelling;
            }
            RunEventKind::RunTerminationRequested { outcome, reason } => {
                if self.lifecycle != RunLifecycle::Running
                    || self.cancellation.is_some()
                    || self.termination.is_some()
                    || *outcome != RunOutcome::Failed
                {
                    return Err(invalid_at(
                        event,
                        "run termination intent must be one first explicit failed drain on a running run",
                    ));
                }
                self.termination = Some(RunTerminationIntent {
                    outcome: *outcome,
                    reason: reason.clone(),
                    sequence,
                });
            }
            RunEventKind::RunTerminal {
                outcome,
                outputs,
                artifacts,
                reason,
            } => {
                if !matches!(
                    self.lifecycle,
                    RunLifecycle::Running
                        | RunLifecycle::Paused
                        | RunLifecycle::Cancelling
                        | RunLifecycle::Created
                ) {
                    return Err(invalid_at(event, "run terminal fact is out of state"));
                }
                if *outcome == RunOutcome::Cancelled && self.cancellation.is_none() {
                    return Err(invalid_at(
                        event,
                        "cancelled outcome requires durable cancellation intent",
                    ));
                }
                if self.lifecycle == RunLifecycle::Cancelling && *outcome != RunOutcome::Cancelled {
                    return Err(invalid_at(
                        event,
                        "durable cancellation intent requires a cancelled run outcome",
                    ));
                }
                if self.cancellation.is_none()
                    && self.termination.as_ref().is_some_and(|termination| {
                        *outcome != termination.outcome
                            || reason.as_ref() != Some(&termination.reason)
                    })
                {
                    return Err(invalid_at(
                        event,
                        "run terminal outcome contradicts its durable explicit-terminal drain",
                    ));
                }
                if self.lifecycle == RunLifecycle::Created && *outcome != RunOutcome::Cancelled {
                    return Err(invalid_at(
                        event,
                        "an unstarted run may only terminate as cancelled",
                    ));
                }
                self.ensure_terminal_quiescent(event)?;
                ensure_unique(outputs, event, "terminal output reference")?;
                ensure_unique(artifacts, event, "terminal artifact reference")?;
                for output in outputs {
                    self.validate_known_workspace_value(output, event)?;
                }
                for artifact in artifacts {
                    self.validate_published_artifact(artifact, event)?;
                }
                self.terminal = Some(RunTerminalProjection {
                    outcome: *outcome,
                    outputs: outputs.clone(),
                    artifacts: artifacts.clone(),
                    reason: reason.clone(),
                    sequence,
                });
                self.lifecycle = RunLifecycle::Terminal(*outcome);
            }
            _ => unreachable!("central projection dispatch owns lifecycle routing"),
        }
        Ok(())
    }
}
