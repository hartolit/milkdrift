//! Deterministic structured-control coordination.

mod reducer;
mod repeat;
mod subworkflow;

use super::support::{
    checked_timestamp_add, entry_nodes, execution_branch_state, node_execution_mode,
    node_occurrence_exists_for_current_pin, node_outcome, run_drain_reason, wait_signal_matches,
};
use super::{RuntimeService, STRUCTURED_EVENT_SOFT_LIMIT};
use crate::projection::{
    BranchState, NodeExecutionState, RunLifecycle, RunProjection, SubworkflowState, TimerPurpose,
};
use crate::{RuntimeError, evaluate_condition};
use milkdrift_blueprint::{
    BlueprintRevision, BranchConfig, EdgeKind, ForkConfig, JoinConfig, Node, NodeId, NodeKind,
    TerminalOutcome,
};
use milkdrift_capability::OperationId;
use milkdrift_persistence::{
    BoundedDetail, NodeExecutionId, NodeExecutionMode, NodeOutcome, Reason, RunEventEnvelope,
    RunEventKind, RunOutcome, SignalDeliveryMode, TimestampMillis, WaitCondition, WaitSatisfaction,
    WorkspaceMutation,
};
use milkdrift_workspace::{RunId, ScopeReference, ValueKey, WorkspaceScope, WorkspaceValue};

struct StructuredPlanContext<'a> {
    run: &'a RunId,
    occurred_at: TimestampMillis,
    projection: &'a mut RunProjection,
    events: &'a mut Vec<RunEventEnvelope>,
    workspace: &'a mut Vec<WorkspaceMutation>,
}

impl RuntimeService {
    pub(super) fn extend_structured_progress(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
    ) -> Result<(), RuntimeError> {
        const MAX_DRIVER_PASSES: usize = 512;
        let scan_limit = usize::from(self.config.maximum_tick_items);
        let mut eligible_scan_remaining = scan_limit;
        let mut successor_scan_remaining = scan_limit;
        let mut branch_scan_remaining = scan_limit;
        if projection.lifecycle() == RunLifecycle::Paused {
            return Ok(());
        }
        let mut context = StructuredPlanContext {
            run,
            occurred_at,
            projection,
            events,
            workspace,
        };
        self.prepare_structured_frontier(revision, &mut context, &mut branch_scan_remaining)?;
        for _ in 0..MAX_DRIVER_PASSES {
            if context.events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            let before = context.events.len();
            self.drive_eligible_frontier(&mut context, &mut eligible_scan_remaining)?;
            self.close_finished_branches(
                context.run,
                context.occurred_at,
                revision,
                context.projection,
                context.events,
                &mut branch_scan_remaining,
            )?;
            self.add_ready_successors(
                context.run,
                context.occurred_at,
                revision,
                context.projection,
                context.events,
                &mut successor_scan_remaining,
            )?;
            self.try_finalize_run(
                context.run,
                context.occurred_at,
                revision,
                context.projection,
                context.events,
                context.workspace,
            )?;
            if context.events.len() == before || context.projection.is_completed() {
                return Ok(());
            }
        }
        Err(RuntimeError::Scheduling(
            "structured driver did not converge within its bounded pass count".to_owned(),
        ))
    }

    fn prepare_structured_frontier(
        &self,
        revision: &BlueprintRevision,
        context: &mut StructuredPlanContext<'_>,
        branch_scan_remaining: &mut usize,
    ) -> Result<(), RuntimeError> {
        let received_signal_in_current_commit = context
            .events
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::SignalReceived { .. }));
        if !received_signal_in_current_commit && run_drain_reason(context.projection).is_none() {
            self.drain_broadcast_signals(
                context.run,
                context.occurred_at,
                context.projection,
                context.events,
                context.workspace,
            )?;
        }
        if context.projection.lifecycle() == RunLifecycle::Running
            && context.projection.termination().is_none()
        {
            let root_scope = context
                .projection
                .root_scope()
                .ok_or_else(|| RuntimeError::InvalidHistory("run root scope is absent".to_owned()))?
                .reference()
                .clone();
            for node_id in entry_nodes(revision) {
                if context.events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
                if node_occurrence_exists_for_current_pin(context.projection, node_id, &root_scope)
                {
                    continue;
                }
                let node = revision.semantic().nodes().get(node_id).ok_or_else(|| {
                    RuntimeError::InvalidHistory("current revision entry node is absent".to_owned())
                })?;
                self.push_projected_event(
                    context.run,
                    context.occurred_at,
                    context.projection,
                    context.events,
                    RunEventKind::NodeBecameEligible {
                        node: node_id.clone(),
                        execution: self.next_execution_id()?,
                        scope: root_scope.clone(),
                        mode: node_execution_mode(node),
                    },
                )?;
            }
        }
        if let Some(reason) = run_drain_reason(context.projection).cloned() {
            let active_branches: Vec<_> = self
                .scan_branch_ids(context.run, context.projection, branch_scan_remaining)?
                .into_iter()
                .filter(|branch| {
                    context
                        .projection
                        .branches()
                        .get(branch)
                        .is_some_and(|branch| branch.state() == BranchState::Active)
                })
                .collect();
            for branch in active_branches {
                if context.events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
                self.push_projected_event(
                    context.run,
                    context.occurred_at,
                    context.projection,
                    context.events,
                    RunEventKind::BranchCancellationRequested {
                        branch,
                        reason: reason.clone(),
                    },
                )?;
            }
        }
        Ok(())
    }

    fn drive_eligible_frontier(
        &self,
        context: &mut StructuredPlanContext<'_>,
        eligible_scan_remaining: &mut usize,
    ) -> Result<(), RuntimeError> {
        let eligible: Vec<_> = self
            .scan_eligible_execution_ids(context.run, context.projection, eligible_scan_remaining)?
            .into_iter()
            .filter_map(|execution| {
                let execution = context.projection.node_executions().get(&execution)?;
                (execution.state() == &NodeExecutionState::Eligible
                    && (execution.mode() == NodeExecutionMode::Runtime
                        || run_drain_reason(context.projection).is_some()
                        || execution_branch_state(context.projection, execution.execution())
                            == Some(BranchState::Cancelling)))
                .then(|| {
                    (
                        execution.execution().clone(),
                        execution.node().clone(),
                        execution.scope().clone(),
                    )
                })
            })
            .collect();
        for (execution, node, scope) in eligible {
            if context.events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            self.drive_eligible_execution(context, execution, node, scope)?;
        }
        Ok(())
    }

    fn drive_eligible_execution(
        &self,
        context: &mut StructuredPlanContext<'_>,
        execution: NodeExecutionId,
        node_id: NodeId,
        scope_reference: ScopeReference,
    ) -> Result<(), RuntimeError> {
        let execution_revision = self.revision_for_execution(context.projection, &execution)?;
        let node = execution_revision
            .semantic()
            .nodes()
            .get(&node_id)
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(format!(
                    "eligible node {node_id} is absent from governing revision {}",
                    execution_revision.id()
                ))
            })?;
        let structurally_cancelling = run_drain_reason(context.projection).is_some()
            || execution_branch_state(context.projection, &execution)
                == Some(BranchState::Cancelling);
        if structurally_cancelling
            && !matches!(
                node.kind(),
                NodeKind::Repeat { .. } | NodeKind::Subworkflow { .. }
            )
        {
            let timers: Vec<_> = context
                .projection
                .timers()
                .values()
                .filter(|timer| {
                    timer.is_pending()
                        && matches!(
                            timer.purpose(),
                            TimerPurpose::Wait { execution: Some(owner) }
                                if owner == &execution
                        )
                })
                .map(|timer| timer.timer().clone())
                .collect();
            for timer in timers {
                self.push_projected_event(
                    context.run,
                    context.occurred_at,
                    context.projection,
                    context.events,
                    RunEventKind::TimerCancelled {
                        timer,
                        reason: Reason::new("structured cancellation released a pending timer")?,
                    },
                )?;
            }
            if context
                .projection
                .waits()
                .get(&execution)
                .is_some_and(|wait| wait.is_pending())
            {
                self.push_projected_event(
                    context.run,
                    context.occurred_at,
                    context.projection,
                    context.events,
                    RunEventKind::WaitCancelled {
                        execution: execution.clone(),
                        reason: Reason::new("structured cancellation released a pending wait")?,
                    },
                )?;
            }
            self.push_projected_event(
                context.run,
                context.occurred_at,
                context.projection,
                context.events,
                RunEventKind::NodeExecutionCancelledBeforeDispatch {
                    execution: execution.clone(),
                    reason: Reason::new(
                        "execution was cancelled before an external dispatch boundary",
                    )?,
                },
            )?;
            return Ok(());
        }
        match node.kind() {
            NodeKind::Task { .. } => {}
            NodeKind::Terminal { outcome } => self.drive_terminal_node(
                context.run,
                context.occurred_at,
                &execution_revision,
                context.projection,
                context.events,
                context.workspace,
                node,
                execution,
                scope_reference,
                outcome,
            )?,
            NodeKind::Wait { duration_ms } => self.drive_timer_wait_node(
                context.run,
                context.occurred_at,
                context.projection,
                context.events,
                node,
                execution,
                *duration_ms,
            )?,
            NodeKind::SignalWait { signal } => self.drive_signal_wait_node(
                context.run,
                context.occurred_at,
                context.projection,
                context.events,
                context.workspace,
                node,
                execution,
                scope_reference,
                signal,
            )?,
            NodeKind::Branch { config } => self.drive_branch_node(
                context.run,
                context.occurred_at,
                context.projection,
                context.events,
                context.workspace,
                node,
                execution,
                scope_reference,
                config,
            )?,
            NodeKind::Fork { config } => self.drive_fork_node(
                context.run,
                context.occurred_at,
                &execution_revision,
                context.projection,
                context.events,
                context.workspace,
                node,
                execution,
                node_id,
                scope_reference,
                config,
            )?,
            NodeKind::Reducer { config } => self.drive_reducer(
                context.run,
                context.occurred_at,
                context.projection,
                context.events,
                context.workspace,
                node,
                &execution,
                &scope_reference,
                config,
                &execution_revision,
            )?,
            NodeKind::Repeat { config } => self.drive_repeat_intent(
                context.run,
                context.occurred_at,
                context.projection,
                context.events,
                context.workspace,
                node,
                &execution,
                &scope_reference,
                config,
            )?,
            NodeKind::Subworkflow { reference } => self.drive_subworkflow_node(
                context.run,
                context.occurred_at,
                context.projection,
                context.events,
                context.workspace,
                node,
                execution,
                scope_reference,
                reference,
            )?,
            NodeKind::Join { config } => self.drive_join_node(
                context.run,
                context.occurred_at,
                &execution_revision,
                context.projection,
                context.events,
                node,
                execution,
                config,
            )?,
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn drive_terminal_node(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        execution_revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: NodeExecutionId,
        scope_reference: ScopeReference,
        outcome: &TerminalOutcome,
    ) -> Result<(), RuntimeError> {
        match outcome {
            TerminalOutcome::Success => {
                match self.materialize_success_terminal_outputs(
                    run,
                    occurred_at,
                    execution_revision,
                    projection,
                    events,
                    workspace,
                    node,
                    &execution,
                    &scope_reference,
                ) {
                    Ok(true) => self.complete_deterministic(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        &execution,
                    )?,
                    Ok(false) => {}
                    Err(RuntimeError::Scheduling(_)) => {
                        self.complete_deterministic_with_outcome(
                            run,
                            occurred_at,
                            projection,
                            events,
                            node,
                            &execution,
                            NodeOutcome::Failed,
                            Some(BoundedDetail::new(
                                "terminal outputs could not be resolved from immutable inputs",
                            )?),
                        )?;
                    }
                    Err(error) => return Err(error),
                }
            }
            TerminalOutcome::Failure => {
                self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    &execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "the explicit workflow terminal selected failure",
                    )?),
                )?;
                if execution_branch_state(projection, &execution).is_none()
                    && projection.cancellation().is_none()
                    && projection.termination().is_none()
                {
                    self.push_projected_event(
                        run,
                        occurred_at,
                        projection,
                        events,
                        RunEventKind::RunTerminationRequested {
                            outcome: RunOutcome::Failed,
                            reason: Reason::new(
                                "explicit failure terminal is draining owned work",
                            )?,
                        },
                    )?;
                }
            }
            TerminalOutcome::Cancelled => {
                let branch = projection
                    .branches()
                    .values()
                    .find(|branch| {
                        branch.state() == BranchState::Active
                            && branch.children().contains(&execution)
                    })
                    .map(|branch| branch.branch().clone());
                if let Some(branch) = branch {
                    self.push_projected_event(
                        run,
                        occurred_at,
                        projection,
                        events,
                        RunEventKind::BranchCancellationRequested {
                            branch,
                            reason: Reason::new(
                                "explicit cancelled terminal ended its fork branch",
                            )?,
                        },
                    )?;
                } else if projection.cancellation().is_none() {
                    self.push_projected_event(
                        run,
                        occurred_at,
                        projection,
                        events,
                        RunEventKind::RunCancellationRequested {
                            reason: Reason::new(
                                "explicit cancelled terminal is draining owned work",
                            )?,
                            evidence: Vec::new(),
                        },
                    )?;
                }
                self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    &execution,
                    NodeOutcome::Cancelled,
                    None,
                )?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn drive_timer_wait_node(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        node: &Node,
        execution: NodeExecutionId,
        duration_ms: u64,
    ) -> Result<(), RuntimeError> {
        if !projection.waits().contains_key(&execution) {
            let timer = self.next_timer_id()?;
            let fire_at = checked_timestamp_add(occurred_at, duration_ms)?;
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::TimerRegistered {
                    timer: timer.clone(),
                    execution: Some(execution.clone()),
                    fire_at,
                },
            )?;
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::WaitRegistered {
                    execution: execution.clone(),
                    condition: WaitCondition::Timer { timer },
                },
            )?;
        } else if let Some(timer) = projection
            .waits()
            .get(&execution)
            .filter(|wait| wait.is_pending())
            .and_then(|wait| match wait.condition() {
                WaitCondition::Timer { timer } | WaitCondition::SignalOrTimer { timer, .. }
                    if projection
                        .timers()
                        .get(timer)
                        .is_some_and(|timer| timer.is_completed()) =>
                {
                    Some(timer.clone())
                }
                WaitCondition::Timer { .. }
                | WaitCondition::Signal { .. }
                | WaitCondition::SignalOrTimer { .. } => None,
            })
        {
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::WaitSatisfied {
                    execution: execution.clone(),
                    cause: WaitSatisfaction::Timer { timer },
                },
            )?;
        } else if projection
            .waits()
            .get(&execution)
            .is_some_and(|wait| wait.is_completed())
        {
            self.complete_deterministic(run, occurred_at, projection, events, node, &execution)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn drive_signal_wait_node(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: NodeExecutionId,
        scope_reference: ScopeReference,
        signal: &OperationId,
    ) -> Result<(), RuntimeError> {
        if !projection.waits().contains_key(&execution) {
            let signal_type = milkdrift_persistence::SignalTypeId::new(signal.as_str().to_owned())?;
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::WaitRegistered {
                    execution: execution.clone(),
                    condition: WaitCondition::Signal {
                        signal_type,
                        correlation: None,
                    },
                },
            )?;
        }
        if let Some(registered_condition) = projection
            .waits()
            .get(&execution)
            .filter(|wait| wait.is_pending())
            .map(|wait| wait.condition().clone())
        {
            let queued = projection
                .signals()
                .values()
                .filter(|candidate| {
                    candidate.is_pending()
                        && candidate.mode() == SignalDeliveryMode::OneShot
                        && wait_signal_matches(
                            &registered_condition,
                            candidate.signal_type(),
                            candidate.correlation(),
                        )
                })
                .min_by_key(|candidate| candidate.received_sequence())
                .map(|candidate| (candidate.signal().clone(), candidate.payload().clone()));
            if let Some((queued_signal, payload)) = queued {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::SignalConsumed {
                        signal: queued_signal.clone(),
                        execution: execution.clone(),
                    },
                )?;
                for port in node.data_outputs().keys() {
                    let key = ValueKey::new(port.as_str().to_owned())
                        .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
                    let entry = self.projected_output_entry(
                        projection,
                        &scope_reference,
                        key,
                        WorkspaceValue::Json(payload.clone()),
                        workspace,
                    )?;
                    let value = entry.reference().clone();
                    workspace.push(WorkspaceMutation::PutValue { entry });
                    self.push_projected_event(
                        run,
                        occurred_at,
                        projection,
                        events,
                        RunEventKind::DeterministicOutputPublished {
                            execution: execution.clone(),
                            value,
                            artifact: None,
                        },
                    )?;
                }
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::WaitSatisfied {
                        execution: execution.clone(),
                        cause: WaitSatisfaction::Signal {
                            signal: queued_signal,
                        },
                    },
                )?;
            }
        }
        if projection
            .waits()
            .get(&execution)
            .is_some_and(|wait| wait.is_completed())
        {
            self.complete_deterministic(run, occurred_at, projection, events, node, &execution)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn drive_branch_node(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &[WorkspaceMutation],
        node: &Node,
        execution: NodeExecutionId,
        scope_reference: ScopeReference,
        config: &BranchConfig,
    ) -> Result<(), RuntimeError> {
        if !projection.branch_routes().contains_key(&execution) {
            let mut selected = None;
            let context =
                match self.evaluation_context(node, projection, &scope_reference, workspace) {
                    Ok(context) => context,
                    Err(RuntimeError::Scheduling(_)) => {
                        self.complete_deterministic_with_outcome(
                            run,
                            occurred_at,
                            projection,
                            events,
                            node,
                            &execution,
                            NodeOutcome::Failed,
                            Some(BoundedDetail::new(
                                "branch inputs could not be evaluated deterministically",
                            )?),
                        )?;
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                };
            let mut evaluation_failed = false;
            for (port, condition) in config.arms() {
                match evaluate_condition(condition, &context) {
                    Ok(true) => {
                        selected = Some(port.clone());
                        break;
                    }
                    Ok(false) => {}
                    Err(RuntimeError::Scheduling(_)) => {
                        evaluation_failed = true;
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            if evaluation_failed {
                self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    &execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "branch condition evaluation failed on immutable input data",
                    )?),
                )?;
                return Ok(());
            }
            let Some(selected) = selected.or_else(|| config.fallback().cloned()) else {
                self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    &execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "branch selected no route and declared no fallback",
                    )?),
                )?;
                return Ok(());
            };
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::BranchRouteSelected {
                    execution: execution.clone(),
                    selected_port: selected,
                },
            )?;
            self.complete_deterministic(run, occurred_at, projection, events, node, &execution)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn drive_fork_node(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        execution_revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: NodeExecutionId,
        node_id: NodeId,
        scope_reference: ScopeReference,
        config: &ForkConfig,
    ) -> Result<(), RuntimeError> {
        let parent = projection
            .scopes()
            .get(&scope_reference)
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("fork execution scope is absent".to_owned())
            })?
            .clone();
        for port in config.branches() {
            if projection.branch_for_fork_port(&execution, port).is_some() {
                continue;
            }
            // BranchScopeCreated, NodeBecameEligible, and
            // BranchChildAdded are one atomic expansion unit.
            if events.len().saturating_add(3) > STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            let branch = self.next_branch_id()?;
            let scope = WorkspaceScope::branch(self.next_scope_id()?, &parent, branch.clone())
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
            let target = execution_revision
                .semantic()
                .edges()
                .values()
                .find(|edge| {
                    edge.kind() == EdgeKind::Control
                        && edge.source_node() == &node_id
                        && edge.source_port() == port
                })
                .map(|edge| edge.target_node().clone())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "fork branch has no exact control target".to_owned(),
                    )
                })?;
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::BranchScopeCreated {
                    fork_execution: execution.clone(),
                    port: port.clone(),
                    branch: branch.clone(),
                    scope: scope.clone(),
                },
            )?;
            workspace.push(WorkspaceMutation::CreateScope {
                scope: scope.clone(),
            });
            let child_execution = self.next_execution_id()?;
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::NodeBecameEligible {
                    mode: node_execution_mode(
                        execution_revision
                            .semantic()
                            .nodes()
                            .get(&target)
                            .ok_or_else(|| {
                                RuntimeError::InvalidHistory(
                                    "branch target node is absent".to_owned(),
                                )
                            })?,
                    ),
                    node: target,
                    execution: child_execution.clone(),
                    scope: scope.reference().clone(),
                },
            )?;
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::BranchChildAdded {
                    branch,
                    execution: child_execution,
                },
            )?;
        }
        let expansion_complete = config
            .branches()
            .iter()
            .all(|port| projection.branch_for_fork_port(&execution, port).is_some());
        if expansion_complete {
            self.complete_deterministic(run, occurred_at, projection, events, node, &execution)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn drive_subworkflow_node(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: NodeExecutionId,
        scope_reference: ScopeReference,
        reference: &milkdrift_blueprint::PinnedSubworkflow,
    ) -> Result<(), RuntimeError> {
        let child = projection
            .subworkflows()
            .values()
            .find(|child| child.parent_execution() == &execution);
        if let Some(child) = child {
            if let SubworkflowState::Terminal(outcome) = child.state() {
                self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    &execution,
                    node_outcome(outcome),
                    None,
                )?;
            }
        } else {
            self.create_subworkflow_intent(
                run,
                occurred_at,
                projection,
                events,
                workspace,
                node,
                &execution,
                &scope_reference,
                &scope_reference,
                reference,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // One atomic runtime transition owns these borrowed state views and durable outputs.
    fn drive_join_node(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        execution_revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        node: &Node,
        execution: NodeExecutionId,
        config: &JoinConfig,
    ) -> Result<(), RuntimeError> {
        if !projection.joins().contains_key(&execution) {
            self.try_satisfy_join(
                run,
                occurred_at,
                execution_revision,
                projection,
                events,
                node,
                &execution,
                config,
            )?;
        }
        Ok(())
    }
}
