use std::collections::BTreeSet;

use milkdrift_capability::SideEffectClass;
use milkdrift_persistence::{
    JoinRule, MAX_PAGE_SIZE, MAX_REPEAT_CONTINUATION_CYCLES, MAX_REPEAT_EFFECTIVE_ITERATIONS,
    NodeOutcome, RepeatContinuationCause, RepeatContinuationDecision, RepeatTerminationReason,
    RunEventEnvelope, RunEventKind, RunOutcome, SignalDeliveryMode, SubworkflowOwnership,
    WaitCondition,
};
use milkdrift_workspace::ScopeKind;

use crate::RuntimeError;

use super::helpers::{
    ensure_unique, ensure_unique_by, invalid_at, wait_condition_timer,
    wait_signal_projection_matches,
};
use super::node::{
    AttemptState, NodeExecutionProjection, NodeExecutionState, RetryState,
    TimerCancellationProjection, TimerProjection, TimerPurpose, TimerState,
};
use super::run::RunProjection;
use super::structured::{
    BranchProjection, BranchState, IterationProjection, IterationState, JoinProjection,
    RepeatContinuationDecisionProjection, RepeatContinuationProjection,
    RepeatContinuationRequestProjection, RepeatTermination, SignalProjection,
    SubworkflowOutputImport, SubworkflowProjection, SubworkflowState, WaitCancellationProjection,
    WaitProjection,
};

impl RunProjection {
    #[allow(clippy::too_many_lines)]
    pub(super) fn apply_structured_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::BranchScopeCreated {
                fork_execution,
                port,
                branch,
                scope,
            } => {
                let owner_scope = self.execution(fork_execution, event)?.scope.clone();
                if self.branches.contains_key(branch)
                    || self
                        .branch_by_fork_port
                        .contains_key(&(fork_execution.clone(), port.clone()))
                    || !matches!(scope.kind(), ScopeKind::Branch { branch: identity } if identity == branch)
                    || scope.parent() != Some(&owner_scope)
                {
                    return Err(invalid_at(
                        event,
                        "branch scope identity, port, kind, or parent is invalid",
                    ));
                }
                self.register_child_scope(scope, event)?;
                self.branches.insert(
                    branch.clone(),
                    BranchProjection {
                        branch: branch.clone(),
                        fork_execution: fork_execution.clone(),
                        port: port.clone(),
                        scope: scope.clone(),
                        children: BTreeSet::new(),
                        state: BranchState::Active,
                        cancellation_reason: None,
                        outputs: Vec::new(),
                    },
                );
                self.branch_by_fork_port
                    .insert((fork_execution.clone(), port.clone()), branch.clone());
                self.branch_ids_by_fork_execution
                    .entry(fork_execution.clone())
                    .or_default()
                    .insert(branch.clone());
                self.active_branch_ids.insert(branch.clone());
                self.adjust_scope_ownership(scope.reference(), true, event)?;
                self.adjust_structured_child_count(fork_execution, true, event)?;
            }
            RunEventKind::BranchRouteSelected {
                execution,
                selected_port,
            } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.is_completed() || self.branch_routes.contains_key(execution) {
                    return Err(invalid_at(
                        event,
                        "branch route is duplicate or follows terminal execution",
                    ));
                }
                self.branch_routes
                    .insert(execution.clone(), selected_port.clone());
            }
            RunEventKind::BranchChildAdded { branch, execution } => {
                let child_scope = self.execution(execution, event)?.scope.clone();
                let branch_view = self.branches.get(branch).ok_or_else(|| {
                    invalid_at(event, "branch child references an unknown branch")
                })?;
                if !branch_view.is_active()
                    || self.branch_owner.contains_key(execution)
                    || !self.scope_descends_from(&child_scope, branch_view.scope.reference())
                {
                    return Err(invalid_at(
                        event,
                        "branch child is duplicate, out of state, or outside its scope",
                    ));
                }
                self.branches
                    .get_mut(branch)
                    .ok_or_else(|| invalid_at(event, "unknown branch"))?
                    .children
                    .insert(execution.clone());
                self.branch_owner.insert(execution.clone(), branch.clone());
            }
            RunEventKind::BranchCancellationRequested { branch, reason } => {
                let branch_view = self.branches.get_mut(branch).ok_or_else(|| {
                    invalid_at(event, "cancellation references an unknown branch")
                })?;
                if branch_view.state != BranchState::Active {
                    return Err(invalid_at(
                        event,
                        "branch cancellation is duplicate or terminal",
                    ));
                }
                branch_view.state = BranchState::Cancelling;
                branch_view.cancellation_reason = Some(reason.clone());
                self.cancelling_branch_ids.insert(branch.clone());
            }
            RunEventKind::BranchTerminal {
                branch,
                outcome,
                outputs,
            } => {
                let branch_view = self.branches.get(branch).ok_or_else(|| {
                    invalid_at(event, "terminal fact references an unknown branch")
                })?;
                if !branch_view.is_active()
                    || (*outcome == RunOutcome::Cancelled
                        && branch_view.state != BranchState::Cancelling)
                    || self.branch_has_active_descendant_ownership(branch)
                {
                    return Err(invalid_at(
                        event,
                        "branch terminal fact is duplicate, contradicts cancellation, or abandons a child",
                    ));
                }
                ensure_unique(outputs, event, "branch terminal output")?;
                for output in outputs {
                    self.validate_known_workspace_value(output, event)?;
                    if !self.scope_descends_from(output.scope(), branch_view.scope.reference()) {
                        return Err(invalid_at(
                            event,
                            "branch terminal output is outside its isolated scope",
                        ));
                    }
                }
                let branch_scope = branch_view.scope.reference().clone();
                let fork_execution = branch_view.fork_execution.clone();
                let branch_view = self
                    .branches
                    .get_mut(branch)
                    .ok_or_else(|| invalid_at(event, "unknown branch"))?;
                branch_view.state = BranchState::Completed(*outcome);
                branch_view.outputs = outputs.clone();
                self.active_branch_ids.remove(branch);
                self.cancelling_branch_ids.remove(branch);
                self.adjust_scope_ownership(&branch_scope, false, event)?;
                self.adjust_structured_child_count(&fork_execution, false, event)?;
            }
            RunEventKind::JoinSatisfied {
                execution,
                rule,
                branches,
                retained_branches,
            } => {
                self.execution(execution, event)?;
                if self.joins.contains_key(execution) {
                    return Err(invalid_at(event, "join was already satisfied"));
                }
                ensure_unique_by(
                    branches,
                    |result| result.branch.clone(),
                    event,
                    "join branch",
                )?;
                ensure_unique(retained_branches, event, "retained branch")?;
                let result_ids: BTreeSet<_> =
                    branches.iter().map(|result| &result.branch).collect();
                if retained_branches
                    .iter()
                    .any(|branch| result_ids.contains(branch))
                {
                    return Err(invalid_at(
                        event,
                        "a completed join result cannot also be retained",
                    ));
                }
                let fork_execution = branches
                    .first()
                    .and_then(|result| self.branches.get(&result.branch))
                    .map(|branch| branch.fork_execution.clone())
                    .ok_or_else(|| invalid_at(event, "join has no known owning fork"))?;
                let fork_scope = self
                    .current_node_execution(&fork_execution)
                    .ok_or_else(|| invalid_at(event, "join fork is outside the current frontier"))?
                    .scope()
                    .clone();
                if self.execution(execution, event)?.scope != fork_scope {
                    return Err(invalid_at(
                        event,
                        "join execution and owning fork must share a structured scope",
                    ));
                }
                for result in branches {
                    let branch = self
                        .branches
                        .get(&result.branch)
                        .ok_or_else(|| invalid_at(event, "join references an unknown branch"))?;
                    if branch.state != BranchState::Completed(result.outcome)
                        || branch.fork_execution != fork_execution
                        || branch.scope.reference() != &result.scope
                        || branch.outputs != result.outputs
                    {
                        return Err(invalid_at(
                            event,
                            "join result disagrees with the branch's durable terminal fact",
                        ));
                    }
                    for output in &result.outputs {
                        self.validate_known_workspace_value(output, event)?;
                        if !self.scope_descends_from(output.scope(), &result.scope) {
                            return Err(invalid_at(
                                event,
                                "branch result output is outside its scope",
                            ));
                        }
                    }
                }
                for retained in retained_branches {
                    let branch = self
                        .branches
                        .get(retained)
                        .ok_or_else(|| invalid_at(event, "join retains an unknown branch"))?;
                    if branch.state != BranchState::Active
                        || branch.fork_execution != fork_execution
                    {
                        return Err(invalid_at(
                            event,
                            "join retains a terminal, cancelling, or foreign branch",
                        ));
                    }
                }
                let owned = self
                    .branch_ids_by_fork_execution
                    .get(&fork_execution)
                    .cloned()
                    .unwrap_or_default();
                let named: BTreeSet<_> = branches
                    .iter()
                    .map(|result| result.branch.clone())
                    .chain(retained_branches.iter().cloned())
                    .collect();
                let unnamed_are_cancelling = owned.difference(&named).all(|branch| {
                    self.branches
                        .get(branch)
                        .is_some_and(|branch| branch.state == BranchState::Cancelling)
                });
                let successes = branches
                    .iter()
                    .filter(|result| result.outcome == RunOutcome::Succeeded)
                    .count();
                let satisfied = match rule {
                    JoinRule::All => {
                        !branches.is_empty()
                            && retained_branches.is_empty()
                            && result_ids.len() == owned.len()
                            && owned.iter().all(|branch| result_ids.contains(branch))
                    }
                    JoinRule::AnyCompletion => !branches.is_empty() && unnamed_are_cancelling,
                    JoinRule::FirstSuccess => {
                        successes >= 1 && retained_branches.is_empty() && unnamed_are_cancelling
                    }
                    JoinRule::Quorum { required } => {
                        usize::try_from(*required).is_ok_and(|required| successes >= required)
                            && retained_branches.is_empty()
                            && unnamed_are_cancelling
                    }
                };
                if !satisfied {
                    return Err(invalid_at(
                        event,
                        "recorded branch results do not satisfy the join rule",
                    ));
                }
                for result in branches {
                    let branch = self
                        .branches
                        .get(&result.branch)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?;
                    if branch.state != BranchState::Completed(result.outcome) {
                        return Err(invalid_at(event, "branch terminal outcome changed at join"));
                    }
                }
                for retained in retained_branches {
                    let scope = self
                        .branches
                        .get(retained)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?
                        .scope
                        .reference()
                        .clone();
                    let fork_execution = self
                        .branches
                        .get(retained)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?
                        .fork_execution
                        .clone();
                    self.branches
                        .get_mut(retained)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?
                        .state = BranchState::Retained;
                    self.active_branch_ids.remove(retained);
                    self.cancelling_branch_ids.remove(retained);
                    self.adjust_scope_ownership(&scope, false, event)?;
                    self.adjust_structured_child_count(&fork_execution, false, event)?;
                }
                self.joins.insert(
                    execution.clone(),
                    JoinProjection {
                        execution: execution.clone(),
                        rule: *rule,
                        branches: branches.clone(),
                        retained_branches: retained_branches.clone(),
                        sequence,
                    },
                );
            }
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
            RunEventKind::TimerRegistered {
                timer,
                execution,
                fire_at,
            } => {
                if self.timers.contains_key(timer) || *fire_at < event.occurred_at() {
                    return Err(invalid_at(
                        event,
                        "timer identity is duplicate or deadline is in the past",
                    ));
                }
                if let Some(execution) = execution {
                    self.execution(execution, event)?;
                }
                self.timers.insert(
                    timer.clone(),
                    TimerProjection {
                        timer: timer.clone(),
                        purpose: TimerPurpose::Wait {
                            execution: execution.clone(),
                        },
                        fire_at: *fire_at,
                        state: TimerState::Pending,
                        cancellation: None,
                    },
                );
                self.pending_timer_ids.insert(timer.clone());
                if let Some(execution) = execution {
                    self.pending_timers_by_execution
                        .entry(execution.clone())
                        .or_default()
                        .insert(timer.clone());
                }
            }
            RunEventKind::TimerFired { timer, observed_at } => {
                let timer_view = self
                    .timers
                    .get_mut(timer)
                    .ok_or_else(|| invalid_at(event, "timer firing references an unknown timer"))?;
                if !timer_view.is_pending() || *observed_at < timer_view.fire_at {
                    return Err(invalid_at(
                        event,
                        "timer fired twice or before its deadline",
                    ));
                }
                let purpose = timer_view.purpose.clone();
                timer_view.state = TimerState::Fired {
                    observed_at: *observed_at,
                };
                self.pending_timer_ids.remove(timer);
                if let Some(retry) = self.retries.get_mut(timer) {
                    retry.state = RetryState::Ready;
                    self.attempts
                        .get_mut(&retry.next_attempt)
                        .ok_or_else(|| invalid_at(event, "retry timer has no reserved attempt"))?
                        .state = AttemptState::ReadyToSchedule;
                }
                self.remove_pending_timer_owner(timer, &purpose, event)?;
            }
            RunEventKind::TimerCancelled { timer, reason } => {
                let timer_view = self.timers.get(timer).ok_or_else(|| {
                    invalid_at(event, "timer cancellation references an unknown timer")
                })?;
                if !timer_view.is_pending() {
                    return Err(invalid_at(event, "only a pending timer may be cancelled"));
                }
                let purpose = timer_view.purpose.clone();
                let authorized = match &purpose {
                    TimerPurpose::Wait {
                        execution: Some(execution),
                    } => {
                        self.waits
                            .get(execution)
                            .and_then(WaitProjection::cancellation)
                            .is_some()
                            || self
                                .node_executions
                                .get(execution)
                                .and_then(NodeExecutionProjection::cancellation)
                                .is_some()
                            || self.has_execution_cancellation_source(execution)
                    }
                    TimerPurpose::Wait { execution: None } => self.cancellation.is_some(),
                    TimerPurpose::Retry { attempt } => {
                        let attempt_view = self.attempt(attempt, event)?;
                        attempt_view.state == AttemptState::AwaitingRetryTimer
                            && self.has_execution_cancellation_source(&attempt_view.execution)
                    }
                };
                if !authorized {
                    return Err(invalid_at(
                        event,
                        "timer cancellation lacks a structured owner cancellation fact",
                    ));
                }
                let timer_view = self
                    .timers
                    .get_mut(timer)
                    .ok_or_else(|| invalid_at(event, "unknown timer"))?;
                timer_view.state = TimerState::Cancelled;
                timer_view.cancellation = Some(TimerCancellationProjection {
                    reason: reason.clone(),
                    sequence,
                });
                self.pending_timer_ids.remove(timer);
                self.remove_pending_timer_owner(timer, &purpose, event)?;
                if let TimerPurpose::Retry { attempt } = purpose {
                    let retry = self
                        .retries
                        .get(timer)
                        .ok_or_else(|| invalid_at(event, "retry timer has no retry decision"))?;
                    if retry.next_attempt != attempt || retry.state != RetryState::Waiting {
                        return Err(invalid_at(
                            event,
                            "retry timer cancellation contradicts its retry decision",
                        ));
                    }
                    let retry_execution = retry.execution.clone();
                    let harmless_uncertain = self
                        .node_executions
                        .get(&retry_execution)
                        .into_iter()
                        .flat_map(|execution| execution.attempts.iter())
                        .filter_map(|candidate| {
                            let prior = self.attempts.get(candidate)?;
                            (prior.state == AttemptState::Uncertain
                                && prior.side_effect.as_ref().is_some_and(|facts| {
                                    matches!(
                                        facts.side_effect,
                                        SideEffectClass::None | SideEffectClass::ReadOnly
                                    )
                                }))
                            .then(|| candidate.clone())
                        })
                        .collect::<Vec<_>>();
                    self.retries
                        .get_mut(timer)
                        .ok_or_else(|| invalid_at(event, "retry timer has no retry decision"))?
                        .state = RetryState::Cancelled;
                    self.attempts
                        .get_mut(&attempt)
                        .ok_or_else(|| invalid_at(event, "retry timer has no reserved attempt"))?
                        .state = AttemptState::CancelledBeforeDispatch;
                    self.active_attempt_ids.remove(&attempt);
                    self.node_executions
                        .get_mut(&retry_execution)
                        .ok_or_else(|| invalid_at(event, "retry has no owning execution"))?
                        .state = NodeExecutionState::Terminal(NodeOutcome::Cancelled);
                    self.deactivate_execution(&retry_execution, event)?;
                    for prior in harmless_uncertain {
                        self.attempts
                            .get_mut(&prior)
                            .ok_or_else(|| invalid_at(event, "uncertain prior attempt is missing"))?
                            .state = AttemptState::UncertainAbandonedByCancellation {
                            cancelled_retry: attempt.clone(),
                        };
                        self.active_attempt_ids.remove(&prior);
                        self.complete_attempt_leases(&prior);
                    }
                }
            }
            RunEventKind::WaitRegistered {
                execution,
                condition,
            } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.is_completed() || self.waits.contains_key(execution) {
                    return Err(invalid_at(
                        event,
                        "wait is duplicate or follows terminal execution",
                    ));
                }
                if let Some(timer) = wait_condition_timer(condition) {
                    let timer_view = self
                        .timers
                        .get(timer)
                        .ok_or_else(|| invalid_at(event, "wait references an unknown timer"))?;
                    if !matches!(
                        &timer_view.purpose,
                        TimerPurpose::Wait { execution: Some(owner) } if owner == execution
                    ) {
                        return Err(invalid_at(event, "wait timer belongs to another execution"));
                    }
                }
                self.waits.insert(
                    execution.clone(),
                    WaitProjection {
                        execution: execution.clone(),
                        condition: condition.clone(),
                        registered_sequence: sequence,
                        satisfaction: None,
                        cancellation: None,
                    },
                );
                self.pending_wait_execution_ids.insert(execution.clone());
            }
            RunEventKind::WaitSatisfied { execution, cause } => {
                let wait = self
                    .waits
                    .get(execution)
                    .ok_or_else(|| invalid_at(event, "satisfaction references an unknown wait"))?;
                if !wait.is_pending() || !self.wait_cause_matches(wait, cause) {
                    return Err(invalid_at(
                        event,
                        "wait cause is duplicate, incompatible, or not yet durable",
                    ));
                }
                self.waits
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown wait"))?
                    .satisfaction = Some(cause.clone());
                self.pending_wait_execution_ids.remove(execution);
            }
            RunEventKind::WaitCancelled { execution, reason } => {
                let wait = self.waits.get(execution).ok_or_else(|| {
                    invalid_at(event, "wait cancellation references an unknown wait")
                })?;
                let authorized = self
                    .node_executions
                    .get(execution)
                    .and_then(NodeExecutionProjection::cancellation)
                    .is_some()
                    || self.has_execution_cancellation_source(execution);
                if !wait.is_pending() || !authorized {
                    return Err(invalid_at(
                        event,
                        "wait cancellation requires a pending wait and structured owner cancellation",
                    ));
                }
                self.waits
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown wait"))?
                    .cancellation = Some(WaitCancellationProjection {
                    reason: reason.clone(),
                    sequence,
                });
                self.pending_wait_execution_ids.remove(execution);
            }
            RunEventKind::SignalReceived {
                signal,
                signal_type,
                correlation,
                mode,
                payload,
            } => {
                if self.signals.contains_key(signal) {
                    return Err(invalid_at(event, "signal identity was already received"));
                }
                self.signals.insert(
                    signal.clone(),
                    SignalProjection {
                        signal: signal.clone(),
                        signal_type: signal_type.clone(),
                        correlation: correlation.clone(),
                        mode: *mode,
                        payload: payload.clone(),
                        received_sequence: sequence,
                        consumed_by: BTreeSet::new(),
                        broadcast_scan_through: None,
                        broadcast_scan_complete: false,
                        duplicate_commands: Vec::new(),
                    },
                );
                if *mode == SignalDeliveryMode::Broadcast {
                    self.pending_broadcast_signals
                        .insert((sequence, signal.clone()));
                }
            }
            RunEventKind::SignalBroadcastScanAdvanced {
                signal,
                through_execution,
                complete,
            } => {
                let signal_view = self.signals.get(signal).ok_or_else(|| {
                    invalid_at(event, "broadcast scan references an unknown signal")
                })?;
                if signal_view.mode != SignalDeliveryMode::Broadcast
                    || signal_view.broadcast_scan_complete
                {
                    return Err(invalid_at(
                        event,
                        "broadcast scan requires an incomplete broadcast signal",
                    ));
                }
                let previous = signal_view.broadcast_scan_through.as_ref();
                let cursor_valid = match (previous, through_execution.as_ref()) {
                    (None, None) => *complete,
                    (None, Some(next)) => self.waits.contains_key(next),
                    (Some(_), None) => false,
                    (Some(previous), Some(next)) => {
                        self.waits.contains_key(next)
                            && (next > previous || (*complete && next == previous))
                    }
                };
                if !cursor_valid {
                    return Err(invalid_at(
                        event,
                        "broadcast scan cursor did not advance monotonically through known waits",
                    ));
                }
                let lower = previous.map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
                let upper = if *complete {
                    std::ops::Bound::Unbounded
                } else {
                    std::ops::Bound::Included(through_execution.as_ref().ok_or_else(|| {
                        invalid_at(event, "incomplete broadcast scan has no cursor")
                    })?)
                };
                let mut scanned = 0_u32;
                for (_, wait) in self.waits.range((lower, upper)) {
                    scanned = scanned.saturating_add(1);
                    if scanned > MAX_PAGE_SIZE {
                        return Err(invalid_at(
                            event,
                            "one broadcast scan event exceeds the durable wait-page bound",
                        ));
                    }
                    let eligible = wait.is_pending()
                        && wait.registered_sequence() < signal_view.received_sequence
                        && !signal_view.consumed_by.contains(wait.execution())
                        && wait_signal_projection_matches(
                            wait.condition(),
                            &signal_view.signal_type,
                            signal_view.correlation.as_ref(),
                            &self.timers,
                        );
                    if eligible {
                        return Err(invalid_at(
                            event,
                            "broadcast scan cannot advance past an eligible unconsumed wait",
                        ));
                    }
                }
                let signal_view = self.signals.get_mut(signal).ok_or_else(|| {
                    invalid_at(event, "broadcast scan references an unknown signal")
                })?;
                signal_view.broadcast_scan_through = through_execution.clone();
                signal_view.broadcast_scan_complete = *complete;
                if *complete {
                    self.pending_broadcast_signals
                        .remove(&(signal_view.received_sequence, signal.clone()));
                }
            }
            RunEventKind::SignalDeduplicated {
                signal,
                duplicate_command,
            } => {
                if self
                    .signals
                    .values()
                    .any(|received| received.duplicate_commands.contains(duplicate_command))
                {
                    return Err(invalid_at(
                        event,
                        "duplicate signal command identity was already recorded",
                    ));
                }
                if let Some(signal_view) = self.signals.get_mut(signal) {
                    signal_view
                        .duplicate_commands
                        .push(duplicate_command.clone());
                }
            }
            RunEventKind::SignalConsumed { signal, execution } => {
                let execution_view = self.execution(execution, event)?;
                let wait_view = self
                    .waits
                    .get(execution)
                    .ok_or_else(|| invalid_at(event, "signal consumer has no registered wait"))?;
                let signal_view = self
                    .signals
                    .get(signal)
                    .ok_or_else(|| invalid_at(event, "consumption references an unknown signal"))?;
                let compatible_wait = match wait_view.condition() {
                    WaitCondition::Signal {
                        signal_type,
                        correlation,
                    } => {
                        signal_view.signal_type == *signal_type
                            && signal_view.correlation == *correlation
                    }
                    WaitCondition::SignalOrTimer {
                        timer,
                        signal_type,
                        correlation,
                    } => {
                        signal_view.signal_type == *signal_type
                            && signal_view.correlation == *correlation
                            && self
                                .timers
                                .get(timer)
                                .is_some_and(TimerProjection::is_pending)
                    }
                    WaitCondition::Timer { .. } => false,
                };
                if execution_view.is_completed()
                    || wait_view.is_completed()
                    || !compatible_wait
                    || signal_view.consumed_by.contains(execution)
                    || (signal_view.mode == SignalDeliveryMode::OneShot
                        && !signal_view.consumed_by.is_empty())
                    || (signal_view.mode == SignalDeliveryMode::Broadcast
                        && wait_view.registered_sequence >= signal_view.received_sequence)
                {
                    return Err(invalid_at(
                        event,
                        "signal consumption is duplicate, incompatible, or violates delivery mode",
                    ));
                }
                self.signals
                    .get_mut(signal)
                    .ok_or_else(|| invalid_at(event, "unknown signal"))?
                    .consumed_by
                    .insert(execution.clone());
            }
            RunEventKind::SubworkflowCreated {
                subworkflow,
                parent_execution,
                child_run,
                child_revision,
                scope,
                ownership,
                inputs,
            } => {
                let parent_scope = self.execution(parent_execution, event)?.scope.clone();
                let valid_parent_scope = scope.parent() == Some(&parent_scope)
                    || self.iterations.values().any(|iteration| {
                        iteration.repeat_execution == *parent_execution
                            && iteration.state == IterationState::Active
                            && scope.parent() == Some(iteration.scope.reference())
                    });
                if self.subworkflows.contains_key(subworkflow)
                    || self.child_runs.contains(child_run)
                    || child_run == event.run_id()
                    || !matches!(scope.kind(), ScopeKind::Subworkflow { subworkflow: identity } if identity == subworkflow)
                    || !valid_parent_scope
                {
                    return Err(invalid_at(
                        event,
                        "subworkflow identity, child run, scope kind, or parent is invalid",
                    ));
                }
                ensure_unique(inputs, event, "subworkflow input")?;
                for input in inputs {
                    if input.scope() != scope.reference() {
                        self.validate_known_workspace_value(input, event)?;
                        let accessible_from_parent = scope.parent().is_some_and(|parent| {
                            input.scope() == parent
                                || self.scope_descends_from(parent, input.scope())
                        });
                        if !accessible_from_parent {
                            return Err(invalid_at(
                                event,
                                "pre-existing subworkflow input is not owned by an ancestor scope",
                            ));
                        }
                    }
                }
                self.register_child_scope(scope, event)?;
                for input in inputs {
                    if input.scope() == scope.reference() {
                        self.record_workspace_value(input, event)?;
                    }
                }
                self.child_runs.insert(child_run.clone());
                self.subworkflows.insert(
                    subworkflow.clone(),
                    SubworkflowProjection {
                        subworkflow: subworkflow.clone(),
                        parent_execution: parent_execution.clone(),
                        created_sequence: sequence,
                        child_run: child_run.clone(),
                        child_revision: child_revision.clone(),
                        scope: scope.clone(),
                        ownership: *ownership,
                        inputs: inputs.clone(),
                        state: SubworkflowState::Active,
                        cancellation_reason: None,
                        outputs: Vec::new(),
                        imports: Vec::new(),
                    },
                );
                self.active_subworkflow_ids.insert(subworkflow.clone());
                if *ownership == SubworkflowOwnership::Attached {
                    self.active_attached_subworkflow_ids
                        .insert(subworkflow.clone());
                }
                self.adjust_structured_child_count(parent_execution, true, event)?;
            }
            RunEventKind::SubworkflowTerminal {
                subworkflow,
                child_run,
                outcome,
                outputs,
                cost_micros,
            } => {
                let child = self.subworkflows.get(subworkflow).ok_or_else(|| {
                    invalid_at(event, "child terminal references an unknown subworkflow")
                })?;
                if child.child_run != *child_run
                    || child.is_completed()
                    || (*outcome == RunOutcome::Cancelled
                        && child.state != SubworkflowState::Cancelling)
                {
                    return Err(invalid_at(
                        event,
                        "child terminal is duplicate or names the wrong run",
                    ));
                }
                ensure_unique(outputs, event, "subworkflow output")?;
                for output in outputs {
                    if output.scope().run() != child_run {
                        return Err(invalid_at(
                            event,
                            "subworkflow terminal output belongs to another run",
                        ));
                    }
                }
                let parent_execution = child.parent_execution.clone();
                let child = self
                    .subworkflows
                    .get_mut(subworkflow)
                    .ok_or_else(|| invalid_at(event, "unknown subworkflow"))?;
                child.state = SubworkflowState::Terminal(*outcome);
                child.outputs = outputs.clone();
                let usage = self
                    .subworkflow_usage_by_execution
                    .entry(parent_execution.clone())
                    .or_default();
                usage.completed_children =
                    usage.completed_children.checked_add(1).unwrap_or_else(|| {
                        usage.overflowed = true;
                        usage.completed_children
                    });
                for (currency, cost) in cost_micros {
                    let total = usage.cost_micros.entry(currency.clone()).or_default();
                    if let Some(next) = total.checked_add(*cost) {
                        *total = next;
                    } else {
                        usage.overflowed = true;
                    }
                }
                self.active_subworkflow_ids.remove(subworkflow);
                self.active_attached_subworkflow_ids.remove(subworkflow);
                self.adjust_structured_child_count(&parent_execution, false, event)?;
            }
            RunEventKind::SubworkflowOutputImported {
                subworkflow,
                child_value,
                parent_value,
            } => {
                let child = self.subworkflows.get(subworkflow).ok_or_else(|| {
                    invalid_at(event, "output import references an unknown subworkflow")
                })?;
                let parent_scope = self
                    .execution(&child.parent_execution, event)?
                    .scope
                    .clone();
                if !child.is_completed()
                    || child_value.scope().run() != &child.child_run
                    || !child.outputs.contains(child_value)
                    || child
                        .imports
                        .iter()
                        .any(|import| import.parent_value == *parent_value)
                    || self.workspace_values.contains(parent_value)
                {
                    return Err(invalid_at(
                        event,
                        "subworkflow import is duplicate or not backed by its terminal child output",
                    ));
                }
                self.validate_workspace_value(parent_value, event)?;
                if !self.scope_descends_from(parent_value.scope(), &parent_scope) {
                    return Err(invalid_at(
                        event,
                        "subworkflow import target is outside its parent execution scope",
                    ));
                }
                self.subworkflows
                    .get_mut(subworkflow)
                    .ok_or_else(|| invalid_at(event, "unknown subworkflow"))?
                    .imports
                    .push(SubworkflowOutputImport {
                        child_value: child_value.clone(),
                        parent_value: parent_value.clone(),
                        sequence,
                    });
                self.record_workspace_value(parent_value, event)?;
            }
            RunEventKind::SubworkflowCancellationRequested {
                subworkflow,
                child_run,
                reason,
            } => {
                let child = self.subworkflows.get_mut(subworkflow).ok_or_else(|| {
                    invalid_at(
                        event,
                        "child cancellation references an unknown subworkflow",
                    )
                })?;
                if child.child_run != *child_run
                    || child.ownership != SubworkflowOwnership::Attached
                    || child.state != SubworkflowState::Active
                {
                    return Err(invalid_at(
                        event,
                        "child cancellation is duplicate, detached, or mismatched",
                    ));
                }
                child.state = SubworkflowState::Cancelling;
                child.cancellation_reason = Some(reason.clone());
            }
            _ => self.apply_reconciliation_kind(event)?,
        }
        Ok(())
    }
}
