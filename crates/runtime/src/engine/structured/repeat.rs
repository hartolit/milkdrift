//! Deterministic structured-node execution, reducers, repeats, and subworkflow intent creation.

use super::super::RuntimeService;
use super::super::support::{
    RepeatBudgetStatus, cancellation_reason_for_execution, run_drain_reason,
};
use crate::projection::{IterationState, RunProjection, SubworkflowState};
use crate::{
    CONTROLLER_POLICY_EXTENSION_KEY, ControllerAssessmentContext, RuntimeError, evaluate_condition,
};
use milkdrift_blueprint::{Node, RepeatTermination};
use milkdrift_persistence::{
    BoundedDetail, ControllerAssessmentBoundary, ControllerAssessmentOutcome, CurrencyCode,
    MAX_REPEAT_CONTINUATION_DECISIONS, NodeExecutionId, NodeOutcome, Reason,
    RepeatContinuationCause, RepeatTerminationReason, RunEventEnvelope, RunEventKind, RunOutcome,
    TimestampMillis, WorkspaceMutation,
};
use milkdrift_workspace::{IterationId, RunId, ScopeReference, WorkspaceScope};

impl RuntimeService {
    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    pub(super) fn drive_repeat_intent(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: &NodeExecutionId,
        scope_reference: &ScopeReference,
        config: &milkdrift_blueprint::RepeatConfig,
    ) -> Result<(), RuntimeError> {
        let latest = projection
            .iterations()
            .values()
            .filter(|iteration| iteration.repeat_execution() == execution)
            .max_by_key(|iteration| iteration.iteration_number())
            .map(|iteration| {
                (
                    iteration.iteration().clone(),
                    iteration.iteration_number(),
                    iteration.state(),
                )
            });
        let children: Vec<_> = projection
            .subworkflows()
            .values()
            .filter(|child| child.parent_execution() == execution)
            .map(|child| child.state())
            .collect();
        let latest_child_state = latest.as_ref().and_then(|(iteration, _, _)| {
            let iteration_scope = projection.iterations().get(iteration)?.scope().reference();
            projection
                .subworkflows()
                .values()
                .find(|child| {
                    child.parent_execution() == execution
                        && child.scope().parent() == Some(iteration_scope)
                })
                .map(|child| child.state())
        });
        let structurally_cancelling =
            cancellation_reason_for_execution(projection, execution, run_drain_reason(projection))
                .is_some();
        if structurally_cancelling {
            if children.iter().any(|state| {
                matches!(
                    state,
                    SubworkflowState::Active | SubworkflowState::Cancelling
                )
            }) {
                return Ok(());
            }
            if let Some((iteration, _, IterationState::Active)) = latest.as_ref() {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::RepeatConditionRecorded {
                        iteration: iteration.clone(),
                        result: false,
                    },
                )?;
            }
            let last_iteration = latest.as_ref().map(|(iteration, _, _)| iteration.clone());
            if !projection.repeat_terminations().contains_key(execution) {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::RepeatTerminated {
                        repeat_execution: execution.clone(),
                        termination: RepeatTerminationReason::Cancelled,
                        last_iteration,
                    },
                )?;
            }
            return self.complete_deterministic_with_outcome(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
                NodeOutcome::Cancelled,
                None,
            );
        }
        if let Some((iteration, _, IterationState::ConditionRecorded(true))) = latest.as_ref()
            && config.termination() == RepeatTermination::AwaitApproval
            && let Some(continuation) = projection.repeat_continuations().get(execution)
        {
            if continuation.is_rejected() {
                let termination = continuation.requests().last().map_or(
                    RepeatTerminationReason::MaximumIterations,
                    |request| match request.cause() {
                        RepeatContinuationCause::IterationLimit => {
                            RepeatTerminationReason::MaximumIterations
                        }
                        RepeatContinuationCause::DurationBudget { .. }
                        | RepeatContinuationCause::CostBudget { .. } => {
                            RepeatTerminationReason::BudgetExhausted
                        }
                        RepeatContinuationCause::ControllerCheckpoint { .. } => {
                            RepeatTerminationReason::MaximumIterations
                        }
                    },
                );
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::RepeatTerminated {
                        repeat_execution: execution.clone(),
                        termination,
                        last_iteration: Some(iteration.clone()),
                    },
                )?;
                return self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "repeat continuation was rejected by authority",
                    )?),
                );
            }
            if continuation.is_pending_approval() {
                return Ok(());
            }
        }
        let authority_budget_override = latest.as_ref().is_some_and(|(_, number, state)| {
            projection
                .repeat_continuations()
                .get(execution)
                .is_some_and(|continuation| {
                    !continuation.is_pending_approval()
                        && !continuation.is_rejected()
                        && continuation
                            .budget_override_iteration_limit()
                            .is_some_and(|limit| match state {
                                IterationState::Active => *number <= limit,
                                IterationState::ConditionRecorded(true) => *number < limit,
                                IterationState::ConditionRecorded(false)
                                | IterationState::Completed(_) => false,
                            })
                })
        });

        let budget_status = if authority_budget_override {
            RepeatBudgetStatus::Within
        } else {
            self.repeat_budget_exhaustion(config, projection, execution, occurred_at)?
        };
        if budget_status != RepeatBudgetStatus::Within {
            return self.drive_exhausted_repeat_budget(
                run,
                occurred_at,
                projection,
                events,
                workspace,
                node,
                execution,
                scope_reference,
                config,
                &latest,
                latest_child_state,
                budget_status,
            );
        }

        if config.termination() == RepeatTermination::AwaitApproval
            && let Some((iteration, iteration_number, IterationState::ConditionRecorded(true))) =
                latest.as_ref()
        {
            let effective_limit = projection.repeat_continuations().get(execution).map_or(
                config.maximum_iterations(),
                |continuation| {
                    continuation
                        .budget_override_iteration_limit()
                        .unwrap_or(continuation.effective_iteration_limit())
                },
            );
            if *iteration_number < effective_limit {
                return self.create_repeat_iteration(
                    run,
                    occurred_at,
                    projection,
                    events,
                    workspace,
                    node,
                    execution,
                    scope_reference,
                    config,
                    iteration_number.checked_add(1).ok_or_else(|| {
                        RuntimeError::Scheduling("repeat iteration number overflow".to_owned())
                    })?,
                );
            }
            let cause = projection
                .repeat_continuations()
                .get(execution)
                .and_then(|continuation| {
                    continuation
                        .budget_override_iteration_limit()
                        .filter(|limit| *iteration_number >= *limit)
                        .and_then(|_| continuation.requests().last())
                })
                .map_or(RepeatContinuationCause::IterationLimit, |request| {
                    request.cause().clone()
                });
            return self.request_repeat_continuation(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
                iteration,
                config,
                cause,
            );
        }
        self.drive_completed_repeat_body(
            run,
            occurred_at,
            projection,
            events,
            workspace,
            node,
            execution,
            scope_reference,
            config,
            latest,
            latest_child_state,
        )
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn drive_exhausted_repeat_budget(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: &NodeExecutionId,
        scope_reference: &ScopeReference,
        config: &milkdrift_blueprint::RepeatConfig,
        latest: &Option<(IterationId, u32, IterationState)>,
        latest_child_state: Option<SubworkflowState>,
        budget_status: RepeatBudgetStatus,
    ) -> Result<(), RuntimeError> {
        let accounting_overflow = budget_status == RepeatBudgetStatus::AccountingOverflow;
        let active_children: Vec<_> = projection
            .subworkflows()
            .values()
            .filter(|child| {
                child.parent_execution() == execution
                    && matches!(
                        child.state(),
                        SubworkflowState::Active | SubworkflowState::Cancelling
                    )
            })
            .map(|child| {
                (
                    child.subworkflow().clone(),
                    child.child_run().clone(),
                    child.state(),
                )
            })
            .collect();
        for (subworkflow, child_run, state) in &active_children {
            if *state == SubworkflowState::Active {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::SubworkflowCancellationRequested {
                        subworkflow: subworkflow.clone(),
                        child_run: child_run.clone(),
                        reason: Reason::new("repeat budget was exhausted")?,
                    },
                )?;
            }
        }
        if !active_children.is_empty() {
            return Ok(());
        }
        if let Some((iteration, _, IterationState::Active)) = latest.as_ref() {
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatConditionRecorded {
                    iteration: iteration.clone(),
                    result: config.termination() == RepeatTermination::AwaitApproval
                        && !accounting_overflow,
                },
            )?;
        }
        if config.termination() == RepeatTermination::AwaitApproval
            && !accounting_overflow
            && let Some((iteration, _, _)) = latest.as_ref()
        {
            let RepeatBudgetStatus::Exhausted(cause) = budget_status else {
                return Err(RuntimeError::InvalidHistory(
                    "repeat budget exhaustion has no typed continuation cause".to_owned(),
                ));
            };
            return self.request_repeat_continuation(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
                iteration,
                config,
                cause,
            );
        }
        let last_iteration = latest.as_ref().map(|(iteration, _, _)| iteration.clone());
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::RepeatTerminated {
                repeat_execution: execution.clone(),
                termination: RepeatTerminationReason::BudgetExhausted,
                last_iteration,
            },
        )?;
        let has_success =
            latest_child_state == Some(SubworkflowState::Terminal(RunOutcome::Succeeded));
        let outcome = match (accounting_overflow, config.termination()) {
            (true, _) => NodeOutcome::Failed,
            (false, RepeatTermination::SucceedWithLatest) if has_success => NodeOutcome::Succeeded,
            (false, RepeatTermination::SucceedWithLatest | RepeatTermination::Fail) => {
                NodeOutcome::Failed
            }
            (false, RepeatTermination::AwaitApproval) => {
                return Err(RuntimeError::InvalidHistory(
                    "await-approval repeat reached an unreachable terminal branch".to_owned(),
                ));
            }
        };
        if outcome == NodeOutcome::Succeeded
            && let Some(iteration) = latest.as_ref().map(|(iteration, _, _)| iteration)
        {
            self.publish_repeat_latest_outputs(
                run,
                occurred_at,
                projection,
                events,
                workspace,
                execution,
                scope_reference,
                iteration,
            )?;
        }
        self.complete_deterministic_with_outcome(
            run,
            occurred_at,
            projection,
            events,
            node,
            execution,
            outcome,
            if accounting_overflow {
                Some(BoundedDetail::new(
                    "repeat cost accounting exceeded its durable numeric range",
                )?)
            } else {
                (outcome != NodeOutcome::Succeeded)
                    .then(|| BoundedDetail::new("repeat budget was exhausted"))
                    .transpose()?
            },
        )
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn drive_completed_repeat_body(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: &NodeExecutionId,
        scope_reference: &ScopeReference,
        config: &milkdrift_blueprint::RepeatConfig,
        latest: Option<(IterationId, u32, IterationState)>,
        latest_child_state: Option<SubworkflowState>,
    ) -> Result<(), RuntimeError> {
        let Some((iteration, iteration_number, state)) = latest else {
            return self.create_repeat_iteration(
                run,
                occurred_at,
                projection,
                events,
                workspace,
                node,
                execution,
                scope_reference,
                config,
                1,
            );
        };
        if state != IterationState::Active
            || latest_child_state.is_none()
            || latest_child_state.is_some_and(|state| {
                matches!(
                    state,
                    SubworkflowState::Active | SubworkflowState::Cancelling
                )
            })
        {
            return Ok(());
        }
        let body_failed = matches!(
            latest_child_state,
            Some(SubworkflowState::Terminal(
                RunOutcome::Failed | RunOutcome::Cancelled
            ))
        );
        if body_failed {
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatConditionRecorded {
                    iteration: iteration.clone(),
                    result: false,
                },
            )?;
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatTerminated {
                    repeat_execution: execution.clone(),
                    termination: RepeatTerminationReason::BodyFailure,
                    last_iteration: Some(iteration.clone()),
                },
            )?;
            return self.complete_deterministic_with_outcome(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
                NodeOutcome::Failed,
                Some(BoundedDetail::new("the pinned repeat body failed")?),
            );
        }

        let context = match self.evaluation_context(node, projection, scope_reference, workspace) {
            Ok(context) => context,
            Err(RuntimeError::Scheduling(_)) => {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::RepeatTerminated {
                        repeat_execution: execution.clone(),
                        termination: RepeatTerminationReason::ConditionEvaluationFailed,
                        last_iteration: Some(iteration.clone()),
                    },
                )?;
                return self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "repeat condition inputs could not be resolved deterministically",
                    )?),
                );
            }
            Err(error) => return Err(error),
        };
        let result = match evaluate_condition(config.condition(), &context) {
            Ok(result) => result,
            Err(RuntimeError::Scheduling(_)) => {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::RepeatTerminated {
                        repeat_execution: execution.clone(),
                        termination: RepeatTerminationReason::ConditionEvaluationFailed,
                        last_iteration: Some(iteration.clone()),
                    },
                )?;
                return self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "repeat condition could not be evaluated against immutable inputs",
                    )?),
                );
            }
            Err(error) => return Err(error),
        };
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::RepeatConditionRecorded {
                iteration: iteration.clone(),
                result,
            },
        )?;
        if !result {
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatTerminated {
                    repeat_execution: execution.clone(),
                    termination: RepeatTerminationReason::ConditionFalse,
                    last_iteration: Some(iteration.clone()),
                },
            )?;
            self.publish_repeat_latest_outputs(
                run,
                occurred_at,
                projection,
                events,
                workspace,
                execution,
                scope_reference,
                &iteration,
            )?;
            return self.complete_deterministic(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
            );
        }
        let effective_limit = projection.repeat_continuations().get(execution).map_or(
            config.maximum_iterations(),
            |continuation| {
                continuation
                    .budget_override_iteration_limit()
                    .unwrap_or(continuation.effective_iteration_limit())
            },
        );
        if iteration_number >= effective_limit {
            if config.termination() == RepeatTermination::AwaitApproval {
                let cause = projection
                    .repeat_continuations()
                    .get(execution)
                    .and_then(|continuation| {
                        continuation
                            .budget_override_iteration_limit()
                            .filter(|limit| iteration_number >= *limit)
                            .and_then(|_| continuation.requests().last())
                    })
                    .map_or(RepeatContinuationCause::IterationLimit, |request| {
                        request.cause().clone()
                    });
                return self.request_repeat_continuation(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    execution,
                    &iteration,
                    config,
                    cause,
                );
            }
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatTerminated {
                    repeat_execution: execution.clone(),
                    termination: RepeatTerminationReason::MaximumIterations,
                    last_iteration: Some(iteration.clone()),
                },
            )?;
            let (outcome, detail) = match config.termination() {
                RepeatTermination::SucceedWithLatest => (NodeOutcome::Succeeded, None),
                RepeatTermination::Fail => (
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "repeat reached its maximum iteration bound",
                    )?),
                ),
                RepeatTermination::AwaitApproval => {
                    return Err(RuntimeError::InvalidHistory(
                        "await-approval repeat reached an unreachable terminal branch".to_owned(),
                    ));
                }
            };
            if outcome == NodeOutcome::Succeeded {
                self.publish_repeat_latest_outputs(
                    run,
                    occurred_at,
                    projection,
                    events,
                    workspace,
                    execution,
                    scope_reference,
                    &iteration,
                )?;
            }
            return self.complete_deterministic_with_outcome(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
                outcome,
                detail,
            );
        }
        self.create_repeat_iteration(
            run,
            occurred_at,
            projection,
            events,
            workspace,
            node,
            execution,
            scope_reference,
            config,
            iteration_number.checked_add(1).ok_or_else(|| {
                RuntimeError::Scheduling("repeat iteration number overflow".to_owned())
            })?,
        )
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn request_repeat_continuation(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        node: &Node,
        execution: &NodeExecutionId,
        frontier_iteration: &IterationId,
        config: &milkdrift_blueprint::RepeatConfig,
        cause: RepeatContinuationCause,
    ) -> Result<(), RuntimeError> {
        let continuation = projection.repeat_continuations().get(execution);
        if continuation.is_some_and(|value| value.is_pending_approval()) {
            return Ok(());
        }
        let (initial_iteration_limit, effective_iteration_limit, request_count) = continuation
            .map_or(
                (config.maximum_iterations(), config.maximum_iterations(), 0),
                |value| {
                    (
                        value.initial_iteration_limit(),
                        value.effective_iteration_limit(),
                        value.requests().len(),
                    )
                },
            );
        if request_count >= MAX_REPEAT_CONTINUATION_DECISIONS {
            let termination = match cause {
                RepeatContinuationCause::IterationLimit => {
                    RepeatTerminationReason::MaximumIterations
                }
                RepeatContinuationCause::DurationBudget { .. }
                | RepeatContinuationCause::CostBudget { .. } => {
                    RepeatTerminationReason::BudgetExhausted
                }
                RepeatContinuationCause::ControllerCheckpoint { .. } => {
                    RepeatTerminationReason::MaximumIterations
                }
            };
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatTerminated {
                    repeat_execution: execution.clone(),
                    termination,
                    last_iteration: Some(frontier_iteration.clone()),
                },
            )?;
            return self.complete_deterministic_with_outcome(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
                NodeOutcome::Failed,
                Some(BoundedDetail::new(
                    "repeat continuation reached its hard authority-cycle bound",
                )?),
            );
        }
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: execution.clone(),
                frontier_iteration: frontier_iteration.clone(),
                initial_iteration_limit,
                effective_iteration_limit,
                cause,
            },
        )
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn create_repeat_iteration(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: &NodeExecutionId,
        scope_reference: &ScopeReference,
        config: &milkdrift_blueprint::RepeatConfig,
        iteration_number: u32,
    ) -> Result<(), RuntimeError> {
        let cycle = ControllerCycleRequest {
            run,
            occurred_at,
            node,
            execution,
            iteration_number,
        };
        match self.assess_controller_cycle(&cycle, projection, events)? {
            ControllerCycleGate::Continue => {}
            ControllerCycleGate::HumanCheckpoint { checkpoint_id } => {
                let frontier = projection
                    .iterations()
                    .values()
                    .filter(|iteration| iteration.repeat_execution() == execution)
                    .max_by_key(|iteration| iteration.iteration_number())
                    .filter(|iteration| {
                        iteration.state() == IterationState::ConditionRecorded(true)
                    })
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "controller checkpoint has no completed true-condition frontier"
                                .to_owned(),
                        )
                    })?
                    .iteration()
                    .clone();
                return self.request_repeat_continuation(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    execution,
                    &frontier,
                    config,
                    RepeatContinuationCause::ControllerCheckpoint {
                        checkpoint_id,
                        completed_cycles: iteration_number.saturating_sub(1),
                    },
                );
            }
            ControllerCycleGate::BoundReached { bound } => {
                let last_iteration = projection
                    .iterations()
                    .values()
                    .filter(|iteration| iteration.repeat_execution() == execution)
                    .max_by_key(|iteration| iteration.iteration_number())
                    .map(|iteration| iteration.iteration().clone());
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::RepeatTerminated {
                        repeat_execution: execution.clone(),
                        termination: RepeatTerminationReason::BudgetExhausted,
                        last_iteration,
                    },
                )?;
                return self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(format!(
                        "controller policy reached immutable {bound} bound"
                    ))?),
                );
            }
        }
        let parent = projection.scopes().get(scope_reference).ok_or_else(|| {
            RuntimeError::InvalidHistory("repeat execution scope is absent".to_owned())
        })?;
        let iteration = self.next_iteration_id()?;
        let scope = WorkspaceScope::iteration(self.next_scope_id()?, parent, iteration.clone())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let iteration_scope = scope.reference().clone();
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: execution.clone(),
                iteration,
                iteration_number,
                scope: scope.clone(),
            },
        )?;
        workspace.push(WorkspaceMutation::CreateScope { scope });
        self.create_subworkflow_intent(
            run,
            occurred_at,
            projection,
            events,
            workspace,
            node,
            execution,
            scope_reference,
            &iteration_scope,
            config.body(),
        )
    }

    fn assess_controller_cycle(
        &self,
        request: &ControllerCycleRequest<'_>,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
    ) -> Result<ControllerCycleGate, RuntimeError> {
        let revision = self.revision_for_execution(projection, request.execution)?;
        let marked = revision
            .semantic()
            .metadata()
            .extensions()
            .keys()
            .any(|key| key.as_str() == CONTROLLER_POLICY_EXTENSION_KEY);
        let lifecycle = self.controller_lifecycle()?;
        if !marked && lifecycle.is_none() {
            return Ok(ControllerCycleGate::Continue);
        }
        let Some(lifecycle) = lifecycle else {
            return Err(RuntimeError::Scheduling(
                "controller policy is marked but no lifecycle owner is installed".to_owned(),
            ));
        };
        let boundary = if request.iteration_number == 1
            && projection
                .controller_assessment(request.execution)
                .is_none()
        {
            ControllerAssessmentBoundary::Activation
        } else {
            ControllerAssessmentBoundary::CycleEntry
        };
        let assessment = {
            let context = ControllerAssessmentContext {
                run: request.run,
                revision: &revision,
                node: request.node,
                execution: request.execution,
                projection,
                observed_at: request.occurred_at,
                boundary,
                next_cycle: Some(request.iteration_number),
            };
            lifecycle.assess(&context)?.map(|assessment| {
                let outcome = assessment.outcome.clone();
                (assessment.into_event(&context), outcome)
            })
        };
        let Some((event, outcome)) = assessment else {
            if marked {
                return Err(RuntimeError::InvalidHistory(
                    "marked controller policy was not recognized by its lifecycle owner".to_owned(),
                ));
            }
            return Ok(ControllerCycleGate::Continue);
        };
        self.push_projected_event(request.run, request.occurred_at, projection, events, event)?;
        Ok(match outcome {
            ControllerAssessmentOutcome::Continue => ControllerCycleGate::Continue,
            ControllerAssessmentOutcome::HumanCheckpoint { checkpoint_id } => {
                ControllerCycleGate::HumanCheckpoint { checkpoint_id }
            }
            ControllerAssessmentOutcome::BoundReached { bound, .. } => {
                ControllerCycleGate::BoundReached { bound }
            }
        })
    }

    fn repeat_budget_exhaustion(
        &self,
        config: &milkdrift_blueprint::RepeatConfig,
        projection: &RunProjection,
        execution: &NodeExecutionId,
        observed_at: TimestampMillis,
    ) -> Result<RepeatBudgetStatus, RuntimeError> {
        if let Some(maximum) = config.budget().max_duration_ms {
            let created_at = projection
                .node_executions()
                .get(execution)
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory("repeat execution is absent".to_owned())
                })?
                .created_at();
            let observed = observed_at.get().saturating_sub(created_at.get());
            if observed >= maximum {
                return Ok(RepeatBudgetStatus::Exhausted(
                    RepeatContinuationCause::DurationBudget {
                        maximum_ms: maximum,
                        observed_ms: observed,
                    },
                ));
            }
        }
        let Some(maximum_cost) = config.budget().max_cost_micros else {
            return Ok(RepeatBudgetStatus::Within);
        };
        let configured_currency = config.budget().max_cost_currency.as_ref().ok_or_else(|| {
            RuntimeError::InvalidHistory("repeat cost budget has no configured currency".to_owned())
        })?;
        let currency = CurrencyCode::new(configured_currency.as_str().to_owned())?;
        let usage = projection.subworkflow_usage_for_execution(execution);
        if usage.is_some_and(|usage| usage.overflowed()) {
            return Ok(RepeatBudgetStatus::AccountingOverflow);
        }
        let observed_cost = usage
            .and_then(|usage| usage.cost_micros().get(&currency))
            .copied()
            .unwrap_or(0);
        if observed_cost >= maximum_cost {
            Ok(RepeatBudgetStatus::Exhausted(
                RepeatContinuationCause::CostBudget {
                    maximum_micros: maximum_cost,
                    observed_micros: observed_cost,
                    currency,
                },
            ))
        } else {
            Ok(RepeatBudgetStatus::Within)
        }
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn publish_repeat_latest_outputs(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        execution: &NodeExecutionId,
        execution_scope: &ScopeReference,
        iteration: &IterationId,
    ) -> Result<(), RuntimeError> {
        let iteration_scope = projection
            .iterations()
            .get(iteration)
            .ok_or_else(|| RuntimeError::InvalidHistory("repeat iteration is absent".to_owned()))?
            .scope()
            .reference()
            .clone();
        let imports: Vec<_> = projection
            .subworkflows()
            .values()
            .find(|child| {
                child.parent_execution() == execution
                    && child.scope().parent() == Some(&iteration_scope)
                    && child.state() == SubworkflowState::Terminal(RunOutcome::Succeeded)
            })
            .map(|child| {
                child
                    .imports()
                    .iter()
                    .map(|import| import.parent_value().clone())
                    .collect()
            })
            .unwrap_or_default();
        for imported in imports {
            let source = self.projected_workspace_value(projection, &imported, workspace)?;
            let output = self.projected_output_entry(
                projection,
                execution_scope,
                source.reference().key().clone(),
                source.value().clone(),
                workspace,
            )?;
            let reference = output.reference().clone();
            workspace.push(WorkspaceMutation::PutValue { entry: output });
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::DeterministicOutputPublished {
                    execution: execution.clone(),
                    value: reference,
                    artifact: None,
                },
            )?;
        }
        Ok(())
    }
}

enum ControllerCycleGate {
    Continue,
    HumanCheckpoint { checkpoint_id: String },
    BoundReached { bound: String },
}

struct ControllerCycleRequest<'a> {
    run: &'a RunId,
    occurred_at: TimestampMillis,
    node: &'a Node,
    execution: &'a NodeExecutionId,
    iteration_number: u32,
}
