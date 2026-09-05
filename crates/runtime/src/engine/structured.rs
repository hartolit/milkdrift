//! Deterministic structured-control coordination.

mod reducer;
mod repeat;
mod subworkflow;

use super::support::{
    checked_timestamp_add, entry_nodes, execution_branch_state, node_execution_mode,
    node_occurrence_exists_for_current_pin, node_outcome, run_drain_reason, wait_signal_matches,
};
use super::transition::PlanTransition;
use super::{RuntimeService, STRUCTURED_EVENT_SOFT_LIMIT};
use crate::RuntimeError;
use crate::projection::{
    BranchState, NodeExecutionState, RunLifecycle, RunProjection, SubworkflowState, TimerPurpose,
};
use crate::scheduler::evaluate_condition;
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
        let mut transition =
            PlanTransition::new(self, run, occurred_at, projection, events, workspace);
        self.prepare_structured_frontier(revision, &mut transition, &mut branch_scan_remaining)?;
        for _ in 0..MAX_DRIVER_PASSES {
            if transition.event_count() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            let before = transition.event_count();
            self.drive_eligible_frontier(&mut transition, &mut eligible_scan_remaining)?;
            self.close_finished_branches(revision, &mut transition, &mut branch_scan_remaining)?;
            self.add_ready_successors(revision, &mut transition, &mut successor_scan_remaining)?;
            self.try_finalize_run(revision, &mut transition)?;
            if transition.event_count() == before || transition.projection().is_completed() {
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
        transition: &mut PlanTransition<'_>,
        branch_scan_remaining: &mut usize,
    ) -> Result<(), RuntimeError> {
        let received_signal_in_current_commit = transition
            .events()
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::SignalReceived { .. }));
        if !received_signal_in_current_commit && run_drain_reason(transition.projection()).is_none()
        {
            self.drain_broadcast_signals(transition)?;
        }
        if transition.projection().lifecycle() == RunLifecycle::Running
            && transition.projection().termination().is_none()
        {
            let root_scope = transition
                .projection()
                .root_scope()
                .ok_or_else(|| RuntimeError::InvalidHistory("run root scope is absent".to_owned()))?
                .reference()
                .clone();
            for node_id in entry_nodes(revision) {
                if transition.event_count() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
                if node_occurrence_exists_for_current_pin(
                    transition.projection(),
                    node_id,
                    &root_scope,
                ) {
                    continue;
                }
                let node = revision.semantic().nodes().get(node_id).ok_or_else(|| {
                    RuntimeError::InvalidHistory("current revision entry node is absent".to_owned())
                })?;
                transition.push_event(RunEventKind::NodeBecameEligible {
                    node: node_id.clone(),
                    execution: self.next_execution_id()?,
                    scope: root_scope.clone(),
                    mode: node_execution_mode(node),
                })?;
            }
        }
        if let Some(reason) = run_drain_reason(transition.projection()).cloned() {
            let active_branches: Vec<_> = self
                .scan_branch_ids(
                    transition.run(),
                    transition.projection(),
                    branch_scan_remaining,
                )?
                .into_iter()
                .filter(|branch| {
                    transition
                        .projection()
                        .branches()
                        .get(branch)
                        .is_some_and(|branch| branch.state() == BranchState::Active)
                })
                .collect();
            for branch in active_branches {
                if transition.event_count() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
                transition.push_event(RunEventKind::BranchCancellationRequested {
                    branch,
                    reason: reason.clone(),
                })?;
            }
        }
        Ok(())
    }

    fn drive_eligible_frontier(
        &self,
        transition: &mut PlanTransition<'_>,
        eligible_scan_remaining: &mut usize,
    ) -> Result<(), RuntimeError> {
        let eligible: Vec<_> = self
            .scan_eligible_execution_ids(
                transition.run(),
                transition.projection(),
                eligible_scan_remaining,
            )?
            .into_iter()
            .filter_map(|execution| {
                let execution = transition.projection().node_executions().get(&execution)?;
                (execution.state() == &NodeExecutionState::Eligible
                    && (execution.mode() == NodeExecutionMode::Runtime
                        || run_drain_reason(transition.projection()).is_some()
                        || execution_branch_state(transition.projection(), execution.execution())
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
            if transition.event_count() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            self.drive_eligible_execution(transition, execution, node, scope)?;
        }
        Ok(())
    }

    fn drive_eligible_execution(
        &self,
        transition: &mut PlanTransition<'_>,
        execution: NodeExecutionId,
        node_id: NodeId,
        scope_reference: ScopeReference,
    ) -> Result<(), RuntimeError> {
        let execution_revision =
            self.revision_for_execution(transition.projection(), &execution)?;
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
        let structurally_cancelling = run_drain_reason(transition.projection()).is_some()
            || execution_branch_state(transition.projection(), &execution)
                == Some(BranchState::Cancelling);
        if structurally_cancelling
            && !matches!(
                node.kind(),
                NodeKind::Repeat { .. } | NodeKind::Subworkflow { .. }
            )
        {
            let timers: Vec<_> = transition
                .projection()
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
                transition.push_event(RunEventKind::TimerCancelled {
                    timer,
                    reason: Reason::new("structured cancellation released a pending timer")?,
                })?;
            }
            if transition
                .projection()
                .waits()
                .get(&execution)
                .is_some_and(|wait| wait.is_pending())
            {
                transition.push_event(RunEventKind::WaitCancelled {
                    execution: execution.clone(),
                    reason: Reason::new("structured cancellation released a pending wait")?,
                })?;
            }
            transition.push_event(RunEventKind::NodeExecutionCancelledBeforeDispatch {
                execution: execution.clone(),
                reason: Reason::new(
                    "execution was cancelled before an external dispatch boundary",
                )?,
            })?;
            return Ok(());
        }
        match node.kind() {
            NodeKind::Task { .. } => {}
            NodeKind::Terminal { outcome } => self.drive_terminal_node(
                transition,
                &execution_revision,
                node,
                execution,
                scope_reference,
                outcome,
            )?,
            NodeKind::Wait { duration_ms } => {
                self.drive_timer_wait_node(transition, node, execution, *duration_ms)?
            }
            NodeKind::SignalWait { signal } => {
                self.drive_signal_wait_node(transition, node, execution, scope_reference, signal)?
            }
            NodeKind::Branch { config } => {
                self.drive_branch_node(transition, node, execution, scope_reference, config)?
            }
            NodeKind::Fork { config } => self.drive_fork_node(
                transition,
                &execution_revision,
                node,
                execution,
                scope_reference,
                config,
            )?,
            NodeKind::Reducer { config } => self.drive_reducer(
                transition,
                node,
                &execution,
                &scope_reference,
                config,
                &execution_revision,
            )?,
            NodeKind::Repeat { config } => {
                self.drive_repeat_intent(transition, node, &execution, &scope_reference, config)?
            }
            NodeKind::Subworkflow { reference } => self.drive_subworkflow_node(
                transition,
                node,
                execution,
                scope_reference,
                reference,
            )?,
            NodeKind::Join { config } => {
                self.drive_join_node(transition, &execution_revision, node, execution, config)?
            }
        }
        Ok(())
    }

    fn drive_terminal_node(
        &self,
        transition: &mut PlanTransition<'_>,
        execution_revision: &BlueprintRevision,
        node: &Node,
        execution: NodeExecutionId,
        scope_reference: ScopeReference,
        outcome: &TerminalOutcome,
    ) -> Result<(), RuntimeError> {
        match outcome {
            TerminalOutcome::Success => {
                match self.materialize_success_terminal_outputs(
                    transition,
                    execution_revision,
                    node,
                    &execution,
                    &scope_reference,
                ) {
                    Ok(true) => self.complete_deterministic(transition, node, &execution)?,
                    Ok(false) => {}
                    Err(RuntimeError::Scheduling(_)) => {
                        self.complete_deterministic_with_outcome(
                            transition,
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
                    transition,
                    node,
                    &execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "the explicit workflow terminal selected failure",
                    )?),
                )?;
                if execution_branch_state(transition.projection(), &execution).is_none()
                    && transition.projection().cancellation().is_none()
                    && transition.projection().termination().is_none()
                {
                    transition.push_event(RunEventKind::RunTerminationRequested {
                        outcome: RunOutcome::Failed,
                        reason: Reason::new("explicit failure terminal is draining owned work")?,
                    })?;
                }
            }
            TerminalOutcome::Cancelled => {
                let branch = transition
                    .projection()
                    .branches()
                    .values()
                    .find(|branch| {
                        branch.state() == BranchState::Active
                            && branch.children().contains(&execution)
                    })
                    .map(|branch| branch.branch().clone());
                if let Some(branch) = branch {
                    transition.push_event(RunEventKind::BranchCancellationRequested {
                        branch,
                        reason: Reason::new("explicit cancelled terminal ended its fork branch")?,
                    })?;
                } else if transition.projection().cancellation().is_none() {
                    transition.push_event(RunEventKind::RunCancellationRequested {
                        reason: Reason::new("explicit cancelled terminal is draining owned work")?,
                        evidence: Vec::new(),
                    })?;
                }
                self.complete_deterministic_with_outcome(
                    transition,
                    node,
                    &execution,
                    NodeOutcome::Cancelled,
                    None,
                )?;
            }
        }
        Ok(())
    }

    fn drive_timer_wait_node(
        &self,
        transition: &mut PlanTransition<'_>,
        node: &Node,
        execution: NodeExecutionId,
        duration_ms: u64,
    ) -> Result<(), RuntimeError> {
        if !transition.projection().waits().contains_key(&execution) {
            let timer = self.next_timer_id()?;
            let fire_at = checked_timestamp_add(transition.occurred_at(), duration_ms)?;
            transition.push_event(RunEventKind::TimerRegistered {
                timer: timer.clone(),
                execution: Some(execution.clone()),
                fire_at,
            })?;
            transition.push_event(RunEventKind::WaitRegistered {
                execution: execution.clone(),
                condition: WaitCondition::Timer { timer },
            })?;
        } else if let Some(timer) = transition
            .projection()
            .waits()
            .get(&execution)
            .filter(|wait| wait.is_pending())
            .and_then(|wait| match wait.condition() {
                WaitCondition::Timer { timer } | WaitCondition::SignalOrTimer { timer, .. }
                    if transition
                        .projection()
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
            transition.push_event(RunEventKind::WaitSatisfied {
                execution: execution.clone(),
                cause: WaitSatisfaction::Timer { timer },
            })?;
        } else if transition
            .projection()
            .waits()
            .get(&execution)
            .is_some_and(|wait| wait.is_completed())
        {
            self.complete_deterministic(transition, node, &execution)?;
        }
        Ok(())
    }

    fn drive_signal_wait_node(
        &self,
        transition: &mut PlanTransition<'_>,
        node: &Node,
        execution: NodeExecutionId,
        scope_reference: ScopeReference,
        signal: &OperationId,
    ) -> Result<(), RuntimeError> {
        if !transition.projection().waits().contains_key(&execution) {
            let signal_type = milkdrift_persistence::SignalTypeId::new(signal.as_str().to_owned())?;
            transition.push_event(RunEventKind::WaitRegistered {
                execution: execution.clone(),
                condition: WaitCondition::Signal {
                    signal_type,
                    correlation: None,
                },
            })?;
        }
        if let Some(registered_condition) = transition
            .projection()
            .waits()
            .get(&execution)
            .filter(|wait| wait.is_pending())
            .map(|wait| wait.condition().clone())
        {
            let queued = transition
                .projection()
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
                transition.push_event(RunEventKind::SignalConsumed {
                    signal: queued_signal.clone(),
                    execution: execution.clone(),
                })?;
                for port in node.data_outputs().keys() {
                    let key = ValueKey::new(port.as_str().to_owned())
                        .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
                    let entry = self.projected_output_entry(
                        transition.projection(),
                        &scope_reference,
                        key,
                        WorkspaceValue::Json(payload.clone()),
                        transition.workspace(),
                    )?;
                    let value = entry.reference().clone();
                    transition.push_workspace(WorkspaceMutation::PutValue { entry })?;
                    transition.push_event(RunEventKind::DeterministicOutputPublished {
                        execution: execution.clone(),
                        value,
                        artifact: None,
                    })?;
                }
                transition.push_event(RunEventKind::WaitSatisfied {
                    execution: execution.clone(),
                    cause: WaitSatisfaction::Signal {
                        signal: queued_signal,
                    },
                })?;
            }
        }
        if transition
            .projection()
            .waits()
            .get(&execution)
            .is_some_and(|wait| wait.is_completed())
        {
            self.complete_deterministic(transition, node, &execution)?;
        }
        Ok(())
    }

    fn drive_branch_node(
        &self,
        transition: &mut PlanTransition<'_>,
        node: &Node,
        execution: NodeExecutionId,
        scope_reference: ScopeReference,
        config: &BranchConfig,
    ) -> Result<(), RuntimeError> {
        if !transition
            .projection()
            .branch_routes()
            .contains_key(&execution)
        {
            let mut selected = None;
            let context = match self.evaluation_context(
                node,
                transition.projection(),
                &scope_reference,
                transition.workspace(),
            ) {
                Ok(context) => context,
                Err(RuntimeError::Scheduling(_)) => {
                    self.complete_deterministic_with_outcome(
                        transition,
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
                    transition,
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
                    transition,
                    node,
                    &execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "branch selected no route and declared no fallback",
                    )?),
                )?;
                return Ok(());
            };
            transition.push_event(RunEventKind::BranchRouteSelected {
                execution: execution.clone(),
                selected_port: selected,
            })?;
            self.complete_deterministic(transition, node, &execution)?;
        }
        Ok(())
    }

    fn drive_fork_node(
        &self,
        transition: &mut PlanTransition<'_>,
        execution_revision: &BlueprintRevision,
        node: &Node,
        execution: NodeExecutionId,
        scope_reference: ScopeReference,
        config: &ForkConfig,
    ) -> Result<(), RuntimeError> {
        let parent = transition
            .projection()
            .scopes()
            .get(&scope_reference)
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("fork execution scope is absent".to_owned())
            })?
            .clone();
        for port in config.branches() {
            if transition
                .projection()
                .branch_for_fork_port(&execution, port)
                .is_some()
            {
                continue;
            }
            // BranchScopeCreated, NodeBecameEligible, and
            // BranchChildAdded are one atomic expansion unit.
            if !transition.has_event_capacity(3, STRUCTURED_EVENT_SOFT_LIMIT) {
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
                        && edge.source_node() == node.id()
                        && edge.source_port() == port
                })
                .map(|edge| edge.target_node().clone())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "fork branch has no exact control target".to_owned(),
                    )
                })?;
            transition.push_event(RunEventKind::BranchScopeCreated {
                fork_execution: execution.clone(),
                port: port.clone(),
                branch: branch.clone(),
                scope: scope.clone(),
            })?;
            transition.push_workspace(WorkspaceMutation::CreateScope {
                scope: scope.clone(),
            })?;
            let child_execution = self.next_execution_id()?;
            transition.push_event(RunEventKind::NodeBecameEligible {
                mode: node_execution_mode(
                    execution_revision
                        .semantic()
                        .nodes()
                        .get(&target)
                        .ok_or_else(|| {
                            RuntimeError::InvalidHistory("branch target node is absent".to_owned())
                        })?,
                ),
                node: target,
                execution: child_execution.clone(),
                scope: scope.reference().clone(),
            })?;
            transition.push_event(RunEventKind::BranchChildAdded {
                branch,
                execution: child_execution,
            })?;
        }
        let expansion_complete = config.branches().iter().all(|port| {
            transition
                .projection()
                .branch_for_fork_port(&execution, port)
                .is_some()
        });
        if expansion_complete {
            self.complete_deterministic(transition, node, &execution)?;
        }
        Ok(())
    }

    fn drive_subworkflow_node(
        &self,
        transition: &mut PlanTransition<'_>,
        node: &Node,
        execution: NodeExecutionId,
        scope_reference: ScopeReference,
        reference: &milkdrift_blueprint::PinnedSubworkflow,
    ) -> Result<(), RuntimeError> {
        let child = transition
            .projection()
            .subworkflows()
            .values()
            .find(|child| child.parent_execution() == &execution);
        if let Some(child) = child {
            if let SubworkflowState::Terminal(outcome) = child.state() {
                self.complete_deterministic_with_outcome(
                    transition,
                    node,
                    &execution,
                    node_outcome(outcome),
                    None,
                )?;
            }
        } else {
            self.create_subworkflow_intent(
                transition,
                node,
                &execution,
                &scope_reference,
                &scope_reference,
                reference,
            )?;
        }
        Ok(())
    }

    fn drive_join_node(
        &self,
        transition: &mut PlanTransition<'_>,
        execution_revision: &BlueprintRevision,
        node: &Node,
        execution: NodeExecutionId,
        config: &JoinConfig,
    ) -> Result<(), RuntimeError> {
        if !transition.projection().joins().contains_key(&execution) {
            self.try_satisfy_join(transition, execution_revision, node, &execution, config)?;
        }
        Ok(())
    }
}
