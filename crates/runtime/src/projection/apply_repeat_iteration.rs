use milkdrift_persistence::{
    MAX_REPEAT_CONTINUATION_CYCLES, MAX_REPEAT_EFFECTIVE_ITERATIONS, RepeatContinuationCause,
    RunEventEnvelope, RunEventKind,
};
use milkdrift_workspace::ScopeKind;

use crate::RuntimeError;

use super::helpers::invalid_at;
use super::run::RunProjection;
use super::structured::{
    IterationProjection, IterationState, RepeatContinuationProjection,
    RepeatContinuationRequestProjection,
};

impl RunProjection {
    pub(super) fn apply_repeat_iteration_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::RepeatIterationCreated {
                repeat_execution,
                iteration,
                iteration_number,
                scope,
            } => {
                let parent_scope = self.execution(repeat_execution, event)?.scope.clone();
                if self.iterations.contains_key(iteration)
                    || self.repeat_terminations.contains_key(repeat_execution)
                    || !matches!(scope.kind(), ScopeKind::Iteration { iteration: identity } if identity == iteration)
                    || scope.parent() != Some(&parent_scope)
                {
                    return Err(invalid_at(
                        event,
                        "repeat iteration identity, kind, parent, or state is invalid",
                    ));
                }
                let expected = self
                    .latest_iteration
                    .get(repeat_execution)
                    .and_then(|previous| self.iterations.get(previous))
                    .map_or(Some(1), |previous| previous.iteration_number.checked_add(1))
                    .ok_or_else(|| invalid_at(event, "repeat iteration number overflow"))?;
                if *iteration_number != expected {
                    return Err(invalid_at(
                        event,
                        "repeat iteration numbers must be contiguous and one-based",
                    ));
                }
                if self
                    .repeat_continuations
                    .get(repeat_execution)
                    .is_some_and(|continuation| {
                        continuation.pending_approval
                            || continuation.rejected
                            || *iteration_number > continuation.effective_iteration_limit
                    })
                {
                    return Err(invalid_at(
                        event,
                        "repeat iteration exceeds or bypasses its continuation authority",
                    ));
                }
                if let Some(previous) = self.latest_iteration.get(repeat_execution).cloned() {
                    let previous_view = self
                        .iterations
                        .get_mut(&previous)
                        .ok_or_else(|| invalid_at(event, "repeat frontier is missing"))?;
                    let IterationState::ConditionRecorded(result) = previous_view.state else {
                        return Err(invalid_at(
                            event,
                            "a new iteration requires the prior frozen condition",
                        ));
                    };
                    if !result {
                        return Err(invalid_at(
                            event,
                            "a false condition cannot create another iteration",
                        ));
                    }
                    previous_view.state = IterationState::Completed(result);
                    self.active_iteration_ids.remove(&previous);
                    self.adjust_structured_child_count(repeat_execution, false, event)?;
                }
                self.register_child_scope(scope, event)?;
                self.iterations.insert(
                    iteration.clone(),
                    IterationProjection {
                        iteration: iteration.clone(),
                        repeat_execution: repeat_execution.clone(),
                        iteration_number: *iteration_number,
                        scope: scope.clone(),
                        state: IterationState::Active,
                    },
                );
                self.active_iteration_ids.insert(iteration.clone());
                self.adjust_structured_child_count(repeat_execution, true, event)?;
                self.latest_iteration
                    .insert(repeat_execution.clone(), iteration.clone());
            }
            RunEventKind::RepeatConditionRecorded { iteration, result } => {
                let iteration_view = self.iterations.get(iteration).ok_or_else(|| {
                    invalid_at(event, "condition references an unknown iteration")
                })?;
                if iteration_view.state != IterationState::Active {
                    return Err(invalid_at(event, "repeat condition was already frozen"));
                }
                self.iterations
                    .get_mut(iteration)
                    .ok_or_else(|| invalid_at(event, "unknown iteration"))?
                    .state = IterationState::ConditionRecorded(*result);
            }
            RunEventKind::RepeatContinuationRequested {
                repeat_execution,
                frontier_iteration,
                initial_iteration_limit,
                effective_iteration_limit,
                cause,
            } => {
                let execution_view = self.execution(repeat_execution, event)?;
                let frontier = self.iterations.get(frontier_iteration).ok_or_else(|| {
                    invalid_at(event, "repeat continuation request has an unknown frontier")
                })?;
                let frontier_is_latest =
                    self.latest_iteration.get(repeat_execution) == Some(frontier_iteration);
                let cause_matches_frontier = match cause {
                    RepeatContinuationCause::IterationLimit => {
                        frontier.iteration_number == *effective_iteration_limit
                    }
                    RepeatContinuationCause::DurationBudget { .. }
                    | RepeatContinuationCause::CostBudget { .. } => {
                        frontier.iteration_number <= *effective_iteration_limit
                    }
                    RepeatContinuationCause::ControllerCheckpoint {
                        completed_cycles, ..
                    } => {
                        frontier.iteration_number == *completed_cycles
                            && frontier.iteration_number < *effective_iteration_limit
                    }
                };
                if execution_view.is_completed()
                    || self.repeat_terminations.contains_key(repeat_execution)
                    || frontier.repeat_execution != *repeat_execution
                    || frontier.state != IterationState::ConditionRecorded(true)
                    || !frontier_is_latest
                    || *initial_iteration_limit == 0
                    || *initial_iteration_limit > *effective_iteration_limit
                    || *effective_iteration_limit > MAX_REPEAT_EFFECTIVE_ITERATIONS
                    || !cause_matches_frontier
                {
                    return Err(invalid_at(
                        event,
                        "repeat continuation request contradicts its exact true-condition frontier, limits, or cause",
                    ));
                }
                let request = RepeatContinuationRequestProjection {
                    frontier_iteration: frontier_iteration.clone(),
                    initial_iteration_limit: *initial_iteration_limit,
                    effective_iteration_limit: *effective_iteration_limit,
                    cause: cause.clone(),
                    sequence,
                };
                if let Some(continuation) = self.repeat_continuations.get_mut(repeat_execution) {
                    if continuation.pending_approval
                        || continuation.rejected
                        || continuation.request_count
                            >= u32::try_from(MAX_REPEAT_CONTINUATION_CYCLES).unwrap_or(u32::MAX)
                        || continuation.request_count != continuation.decision_count
                        || continuation.initial_iteration_limit != *initial_iteration_limit
                        || continuation.effective_iteration_limit != *effective_iteration_limit
                        || continuation
                            .requests
                            .last()
                            .is_some_and(|prior| prior.frontier_iteration == *frontier_iteration)
                    {
                        return Err(invalid_at(
                            event,
                            "repeat continuation request is duplicate or disagrees with prior authority",
                        ));
                    }
                    continuation.budget_override_iteration_limit = None;
                    continuation.pending_approval = true;
                    continuation.request_count = continuation
                        .request_count
                        .checked_add(1)
                        .ok_or_else(|| invalid_at(event, "repeat request count overflow"))?;
                    continuation.requests.push(request);
                } else {
                    if initial_iteration_limit != effective_iteration_limit {
                        return Err(invalid_at(
                            event,
                            "the first repeat continuation request must record its original effective limit",
                        ));
                    }
                    self.repeat_continuations.insert(
                        repeat_execution.clone(),
                        RepeatContinuationProjection {
                            repeat_execution: repeat_execution.clone(),
                            initial_iteration_limit: *initial_iteration_limit,
                            effective_iteration_limit: *effective_iteration_limit,
                            budget_override_iteration_limit: None,
                            pending_approval: true,
                            rejected: false,
                            request_count: 1,
                            decision_count: 0,
                            requests: vec![request],
                            decisions: Vec::new(),
                        },
                    );
                }
            }
            _ => unreachable!(
                "structured dispatch owns repeat iteration and continuation request routing"
            ),
        }
        Ok(())
    }
}
