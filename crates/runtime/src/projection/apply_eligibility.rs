use milkdrift_persistence::{RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::helpers::invalid_at;
use super::node::{
    AttemptState, NodeExecutionCancellationProjection, NodeExecutionProjection, NodeExecutionState,
};
use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_eligibility_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::NodeBecameEligible {
                node,
                execution,
                scope,
                mode,
            } => {
                self.validate_scope_reference(scope, event)?;
                if self.node_executions.contains_key(execution)
                    || self.settled_node_executions.contains_key(execution)
                {
                    return Err(invalid_at(
                        event,
                        "node execution identity was already created",
                    ));
                }
                self.reserved_executions.remove(execution);
                self.node_executions.insert(
                    execution.clone(),
                    NodeExecutionProjection {
                        execution: execution.clone(),
                        node: node.clone(),
                        scope: scope.clone(),
                        mode: *mode,
                        revision: self
                            .revision
                            .clone()
                            .ok_or_else(|| invalid_at(event, "node execution has no revision"))?,
                        epoch_retired_sequence: None,
                        created_sequence: sequence,
                        created_at: event.occurred_at(),
                        attempts: Vec::new(),
                        attempt_count: 0,
                        state: NodeExecutionState::Eligible,
                        cancellation: None,
                        deterministic_terminal: None,
                        outputs: Vec::new(),
                    },
                );
                self.execution_ids_by_node
                    .entry(node.clone())
                    .or_default()
                    .insert(execution.clone());
                self.eligible_executions.insert(execution.clone());
                self.activate_execution(execution, event)?;
                if self
                    .pending_reconciliation_restarts
                    .remove(&(node.clone(), scope.clone()))
                    .is_some()
                {
                    self.adjust_scope_ownership(scope, false, event)?;
                }
            }
            RunEventKind::NodeExecutionCancelledBeforeDispatch { execution, reason } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.state != NodeExecutionState::Eligible
                    || !execution_view.attempts.is_empty()
                    || execution_view.cancellation.is_some()
                    || !self.has_execution_cancellation_source(execution)
                {
                    return Err(invalid_at(
                        event,
                        "pre-dispatch cancellation requires an eligible, attempt-free execution and a structured cancellation source",
                    ));
                }
                let execution_view = self
                    .node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?;
                execution_view.cancellation = Some(NodeExecutionCancellationProjection {
                    attempt: None,
                    reason: reason.clone(),
                    sequence,
                });
                execution_view.state = NodeExecutionState::CancelledBeforeDispatch;
                self.eligible_executions.remove(execution);
                self.deactivate_execution(execution, event)?;
            }
            RunEventKind::NodeExecutionCancellationRequested {
                execution,
                attempt,
                reason,
            } => {
                let execution_view = self.execution(execution, event)?;
                let attempt_view = self.attempt(attempt, event)?;
                let before_dispatch = matches!(
                    attempt_view.state,
                    AttemptState::Scheduled | AttemptState::Leased
                );
                if execution_view.attempts.last() != Some(attempt)
                    || execution_view.cancellation.is_some()
                    || attempt_view.execution != *execution
                    || !matches!(
                        attempt_view.state,
                        AttemptState::Scheduled | AttemptState::Leased | AttemptState::Running
                    )
                    || !matches!(
                        execution_view.state,
                        NodeExecutionState::Scheduled(ref active)
                            | NodeExecutionState::Running(ref active)
                            if active == attempt
                    )
                    || !self.has_execution_cancellation_source(execution)
                {
                    return Err(invalid_at(
                        event,
                        "attempt cancellation must target the latest scheduled, leased, or running attempt with structured authority",
                    ));
                }
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .cancellation = Some(NodeExecutionCancellationProjection {
                    attempt: Some(attempt.clone()),
                    reason: reason.clone(),
                    sequence,
                });
                if before_dispatch {
                    self.complete_attempt_leases(attempt);
                    self.active_attempt_ids.remove(attempt);
                    self.attempts
                        .get_mut(attempt)
                        .ok_or_else(|| invalid_at(event, "unknown cancellation attempt"))?
                        .state = AttemptState::CancelledBeforeDispatch;
                    self.node_executions
                        .get_mut(execution)
                        .ok_or_else(|| invalid_at(event, "unknown cancellation execution"))?
                        .state = NodeExecutionState::CancelledBeforeDispatch;
                    self.deactivate_execution(execution, event)?;
                }
            }
            _ => unreachable!(
                "central projection dispatch owns execution eligibility and cancellation routing"
            ),
        }
        Ok(())
    }
}
