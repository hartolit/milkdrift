use milkdrift_capability::{IdempotencyBehavior, SideEffectClass};
use milkdrift_persistence::{NodeExecutionMode, NodeOutcome, RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::helpers::{invalid_at, same_logical_invocation_request};
use super::node::{
    AttemptState, AttemptTerminal, DeterministicNodeTerminalProjection,
    NodeExecutionCancellationProjection, NodeExecutionState,
};
use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_terminal_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::DeterministicNodeTerminal {
                execution,
                outcome,
                error_class,
                detail,
            } => {
                let execution_view = self.execution(execution, event)?;
                let failure_shape = matches!(outcome, NodeOutcome::Failed | NodeOutcome::Rejected);
                if execution_view.state != NodeExecutionState::Eligible
                    || execution_view.mode != NodeExecutionMode::Runtime
                    || !execution_view.attempts.is_empty()
                    || execution_view.cancellation.is_some()
                    || execution_view.deterministic_terminal.is_some()
                    || *outcome == NodeOutcome::Cancelled
                    || failure_shape != error_class.is_some()
                {
                    return Err(invalid_at(
                        event,
                        "deterministic terminal fact requires an attempt-free eligible execution and a valid non-cancellation outcome",
                    ));
                }
                let terminal = DeterministicNodeTerminalProjection {
                    outcome: *outcome,
                    error_class: *error_class,
                    detail: detail.clone(),
                    sequence,
                };
                let execution_view = self
                    .node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?;
                execution_view.deterministic_terminal = Some(terminal);
                execution_view.state = NodeExecutionState::Terminal(*outcome);
                self.eligible_executions.remove(execution);
                self.deactivate_execution(execution, event)?;
                if *outcome == NodeOutcome::Succeeded {
                    self.pending_successor_executions.insert(execution.clone());
                }
            }
            RunEventKind::NodePreDispatchFailed {
                execution,
                error_class,
                detail,
            } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.state != NodeExecutionState::Eligible
                    || execution_view.mode != NodeExecutionMode::Executor
                    || !execution_view.attempts.is_empty()
                    || execution_view.cancellation.is_some()
                    || execution_view.deterministic_terminal.is_some()
                {
                    return Err(invalid_at(
                        event,
                        "pre-dispatch failure requires an attempt-free eligible executor execution",
                    ));
                }
                let terminal = DeterministicNodeTerminalProjection {
                    outcome: NodeOutcome::Failed,
                    error_class: Some(*error_class),
                    detail: detail.clone(),
                    sequence,
                };
                let execution_view = self
                    .node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?;
                execution_view.deterministic_terminal = Some(terminal);
                execution_view.state = NodeExecutionState::Terminal(NodeOutcome::Failed);
                self.eligible_executions.remove(execution);
                self.deactivate_execution(execution, event)?;
            }
            RunEventKind::StructuredSuccessorScanCompleted { execution } => {
                if self.node_executions.get(execution).is_none_or(|execution| {
                    execution.state != NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                }) || !self.pending_successor_executions.remove(execution)
                {
                    return Err(invalid_at(
                        event,
                        "successor scan marker must consume one pending successful execution",
                    ));
                }
            }
            RunEventKind::NodeTerminal {
                execution,
                attempt,
                report_sequence,
                outcome,
                error_class,
                detail,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let failure_shape = matches!(outcome, NodeOutcome::Failed | NodeOutcome::Rejected);
                let cancellation_matches = self
                    .node_executions
                    .get(execution)
                    .and_then(|execution| execution.cancellation.as_ref())
                    .and_then(NodeExecutionCancellationProjection::attempt)
                    == Some(attempt);
                if attempt_view.execution != *execution
                    || !matches!(
                        attempt_view.state,
                        AttemptState::Leased | AttemptState::Running
                    )
                    || attempt_view.capability.is_none()
                    || attempt_view.side_effect.is_none()
                    || attempt_view.leases.is_empty()
                    || !attempt_view.expects_report_sequence(*report_sequence)
                    || failure_shape != error_class.is_some()
                    || (*outcome == NodeOutcome::Cancelled && !cancellation_matches)
                {
                    return Err(invalid_at(
                        event,
                        "node terminal fact is duplicate, mismatched, or malformed",
                    ));
                }
                let safely_covered_uncertain = {
                    let current_request = attempt_view.request.as_ref();
                    let current_capability = attempt_view.capability.as_ref();
                    let current_side_effect = attempt_view.side_effect.as_ref();
                    self.node_executions
                        .get(execution)
                        .into_iter()
                        .flat_map(|execution| execution.attempts.iter())
                        .take_while(|candidate| *candidate != attempt)
                        .filter_map(|candidate| {
                            let prior = self.attempts.get(candidate)?;
                            let prior_side_effect = prior.side_effect.as_ref()?;
                            let retry_safe = matches!(
                                prior_side_effect.side_effect,
                                SideEffectClass::None | SideEffectClass::ReadOnly
                            ) || (prior_side_effect.side_effect
                                == SideEffectClass::IdempotentWrite
                                && prior_side_effect.idempotency
                                    != IdempotencyBehavior::Unsupported
                                && prior_side_effect.idempotency_key.is_some());
                            let terminal_covers = *outcome == NodeOutcome::Succeeded
                                || matches!(
                                    prior_side_effect.side_effect,
                                    SideEffectClass::None | SideEffectClass::ReadOnly
                                );
                            (prior.state == AttemptState::Uncertain
                                && prior.obligation.is_some()
                                && retry_safe
                                && terminal_covers
                                && prior_side_effect == current_side_effect?
                                && prior.request.as_ref().zip(current_request).is_some_and(
                                    |(prior, current)| {
                                        same_logical_invocation_request(prior, current)
                                    },
                                )
                                && prior.idempotency_key == attempt_view.idempotency_key
                                && prior
                                    .capability
                                    .as_ref()
                                    .zip(current_capability)
                                    .is_some_and(|(prior, current)| {
                                        prior.snapshot == current.snapshot
                                    }))
                            .then(|| candidate.clone())
                        })
                        .collect::<Vec<_>>()
                };
                let terminal = AttemptTerminal {
                    report_sequence: *report_sequence,
                    outcome: *outcome,
                    error_class: *error_class,
                    detail: detail.clone(),
                    sequence,
                };
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.state = AttemptState::Terminal(*outcome);
                attempt_view.last_report_sequence = Some(*report_sequence);
                attempt_view.terminal = Some(terminal);
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::Terminal(*outcome);
                self.active_attempt_ids.remove(attempt);
                self.deactivate_execution(execution, event)?;
                if *outcome == NodeOutcome::Succeeded {
                    self.pending_successor_executions.insert(execution.clone());
                }
                self.complete_attempt_leases(attempt);
                for covered in safely_covered_uncertain {
                    let covered_view = self
                        .attempts
                        .get_mut(&covered)
                        .ok_or_else(|| invalid_at(event, "superseded attempt is missing"))?;
                    covered_view.state = if *outcome == NodeOutcome::Cancelled {
                        AttemptState::UncertainAbandonedByCancellation {
                            cancelled_retry: attempt.clone(),
                        }
                    } else {
                        AttemptState::UncertainSupersededByRetry {
                            covering_attempt: attempt.clone(),
                        }
                    };
                    self.active_attempt_ids.remove(&covered);
                    self.complete_attempt_leases(&covered);
                }
            }
            _ => unreachable!("central projection dispatch owns node terminal outcomes routing"),
        }
        Ok(())
    }
}
