use milkdrift_persistence::{
    MAX_REPEAT_EFFECTIVE_ITERATIONS, RepeatContinuationCause, RepeatContinuationDecision,
    RepeatTerminationReason, RunEventEnvelope, RunEventKind,
};

use crate::RuntimeError;

use super::helpers::invalid_at;
use super::run::RunProjection;
use super::structured::{IterationState, RepeatContinuationDecisionProjection, RepeatTermination};

impl RunProjection {
    pub(super) fn apply_repeat_decision_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::RepeatContinuationDecided {
                repeat_execution,
                decision,
                actor,
                outcome,
                approved_additional_iterations,
                reason,
                evidence,
            } => {
                let execution_view = self.execution(repeat_execution, event)?;
                let shape_valid = match (outcome, approved_additional_iterations) {
                    (RepeatContinuationDecision::Approved, Some(additional)) => (1
                        ..=milkdrift_persistence::MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS)
                        .contains(additional),
                    (RepeatContinuationDecision::Rejected, None) => true,
                    (RepeatContinuationDecision::Approved, None)
                    | (RepeatContinuationDecision::Rejected, Some(_)) => false,
                };
                if execution_view.is_completed()
                    || self.repeat_terminations.contains_key(repeat_execution)
                    || !shape_valid
                    || self
                        .repeat_continuations
                        .values()
                        .flat_map(|continuation| continuation.decisions.iter())
                        .any(|recorded| recorded.decision == *decision)
                {
                    return Err(invalid_at(
                        event,
                        "repeat continuation decision is duplicate, malformed, or follows completion",
                    ));
                }
                let continuation =
                    self.repeat_continuations
                        .get(repeat_execution)
                        .ok_or_else(|| {
                            invalid_at(event, "repeat decision has no durable continuation request")
                        })?;
                let pending_request = continuation.pending_request().ok_or_else(|| {
                    invalid_at(event, "repeat decision has no pending continuation request")
                })?;
                let frontier = self
                    .iterations
                    .get(&pending_request.frontier_iteration)
                    .ok_or_else(|| invalid_at(event, "pending repeat frontier is missing"))?;
                if continuation.rejected
                    || continuation.request_count != continuation.decision_count + 1
                    || self.latest_iteration.get(repeat_execution)
                        != Some(&pending_request.frontier_iteration)
                    || frontier.repeat_execution != *repeat_execution
                    || frontier.state != IterationState::ConditionRecorded(true)
                    || pending_request.effective_iteration_limit
                        != continuation.effective_iteration_limit
                {
                    return Err(invalid_at(
                        event,
                        "repeat decision does not consume the exact pending authority request",
                    ));
                }
                let budget_frontier = match pending_request.cause {
                    RepeatContinuationCause::DurationBudget { .. }
                    | RepeatContinuationCause::CostBudget { .. } => Some(frontier.iteration_number),
                    RepeatContinuationCause::IterationLimit => None,
                };
                let decision_projection = RepeatContinuationDecisionProjection {
                    decision: decision.clone(),
                    actor: actor.clone(),
                    outcome: *outcome,
                    approved_additional_iterations: *approved_additional_iterations,
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    sequence,
                };
                let continuation = self
                    .repeat_continuations
                    .get_mut(repeat_execution)
                    .ok_or_else(|| invalid_at(event, "unknown repeat continuation"))?;
                if let Some(additional) = approved_additional_iterations {
                    continuation.effective_iteration_limit = continuation
                        .effective_iteration_limit
                        .checked_add(*additional)
                        .filter(|limit| *limit <= MAX_REPEAT_EFFECTIVE_ITERATIONS)
                        .ok_or_else(|| {
                            invalid_at(event, "repeat effective iteration limit overflow")
                        })?;
                    continuation.budget_override_iteration_limit = budget_frontier
                        .map(|frontier| {
                            frontier
                                .checked_add(*additional)
                                .filter(|limit| *limit <= MAX_REPEAT_EFFECTIVE_ITERATIONS)
                                .ok_or_else(|| {
                                    invalid_at(event, "repeat budget override frontier overflow")
                                })
                        })
                        .transpose()?;
                } else {
                    continuation.rejected = true;
                    continuation.budget_override_iteration_limit = None;
                }
                continuation.pending_approval = false;
                continuation.decision_count = continuation
                    .decision_count
                    .checked_add(1)
                    .ok_or_else(|| invalid_at(event, "repeat decision count overflow"))?;
                continuation.decisions.push(decision_projection);
            }
            RunEventKind::RepeatTerminated {
                repeat_execution,
                termination,
                last_iteration,
            } => {
                self.execution(repeat_execution, event)?;
                let continuation_conflict = self
                    .repeat_continuations
                    .get(repeat_execution)
                    .is_some_and(|continuation| {
                        if *termination == RepeatTerminationReason::Cancelled {
                            return false;
                        }
                        if continuation.pending_approval {
                            return true;
                        }
                        if !continuation.rejected {
                            return false;
                        }
                        continuation.requests.last().is_none_or(|request| {
                            let expected = match request.cause {
                                RepeatContinuationCause::IterationLimit => {
                                    RepeatTerminationReason::MaximumIterations
                                }
                                RepeatContinuationCause::DurationBudget { .. }
                                | RepeatContinuationCause::CostBudget { .. } => {
                                    RepeatTerminationReason::BudgetExhausted
                                }
                            };
                            *termination != expected
                        })
                    });
                if self.repeat_terminations.contains_key(repeat_execution)
                    || self.latest_iteration.get(repeat_execution) != last_iteration.as_ref()
                    || continuation_conflict
                {
                    return Err(invalid_at(
                        event,
                        "repeat termination is duplicate or names the wrong frontier",
                    ));
                }
                if let Some(iteration) = last_iteration {
                    let iteration_view = self.iterations.get_mut(iteration).ok_or_else(|| {
                        invalid_at(event, "repeat termination references an unknown iteration")
                    })?;
                    let result = match iteration_view.state {
                        IterationState::ConditionRecorded(result)
                            if *termination
                                != RepeatTerminationReason::ConditionEvaluationFailed =>
                        {
                            result
                        }
                        IterationState::Active
                            if *termination
                                == RepeatTerminationReason::ConditionEvaluationFailed =>
                        {
                            false
                        }
                        IterationState::Active | IterationState::Completed(_) => {
                            return Err(invalid_at(
                                event,
                                "repeat termination requires a frozen frontier condition",
                            ));
                        }
                        IterationState::ConditionRecorded(_) => {
                            return Err(invalid_at(
                                event,
                                "condition-evaluation failure cannot follow a recorded condition",
                            ));
                        }
                    };
                    if *termination == RepeatTerminationReason::ConditionFalse && result {
                        return Err(invalid_at(
                            event,
                            "condition-false termination contradicts a true condition",
                        ));
                    }
                    iteration_view.state = IterationState::Completed(result);
                    self.active_iteration_ids.remove(iteration);
                    self.adjust_structured_child_count(repeat_execution, false, event)?;
                } else if matches!(
                    termination,
                    RepeatTerminationReason::ConditionFalse
                        | RepeatTerminationReason::ConditionEvaluationFailed
                ) {
                    return Err(invalid_at(
                        event,
                        "condition termination requires an iteration",
                    ));
                }
                self.repeat_terminations.insert(
                    repeat_execution.clone(),
                    RepeatTermination {
                        repeat_execution: repeat_execution.clone(),
                        termination: *termination,
                        last_iteration: last_iteration.clone(),
                        sequence,
                    },
                );
                if let Some(continuation) = self.repeat_continuations.get_mut(repeat_execution) {
                    continuation.pending_approval = false;
                    continuation.budget_override_iteration_limit = None;
                }
            }
            _ => unreachable!(
                "structured dispatch owns repeat continuation authority and termination routing"
            ),
        }
        Ok(())
    }
}
