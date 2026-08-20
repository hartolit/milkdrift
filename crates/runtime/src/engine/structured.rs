//! Deterministic structured-node execution, reducers, repeats, and subworkflow intent creation.

use super::support::{
    RepeatBudgetStatus, cancellation_reason_for_execution, checked_timestamp_add, entry_nodes,
    execution_branch_state, node_execution_mode, node_occurrence_exists_for_current_pin,
    node_outcome, run_drain_reason, wait_signal_matches,
};
use super::{RuntimeService, STRUCTURED_EVENT_SOFT_LIMIT};
use crate::projection::{
    BranchState, IterationState, NodeExecutionState, RunLifecycle, RunProjection, SubworkflowState,
    TimerPurpose,
};
use crate::{RuntimeError, evaluate_condition};
use milkdrift_blueprint::{
    BlueprintRevision, EdgeKind, Node, NodeKind, PortId, ReducerStrategy, RepeatTermination,
    TerminalOutcome,
};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    BoundedDetail, CurrencyCode, MAX_REPEAT_CONTINUATION_DECISIONS, NodeExecutionId,
    NodeExecutionMode, NodeOutcome, Reason, RepeatContinuationCause, RepeatTerminationReason,
    RunEventEnvelope, RunEventKind, RunOutcome, RunSequence, SignalDeliveryMode,
    SubworkflowOwnership, TimestampMillis, WaitCondition, WaitSatisfaction, WorkspaceMutation,
};
use milkdrift_workspace::{
    IterationId, RunId, ScopeReference, ValueKey, WorkspaceScope, WorkspaceValue,
};

impl RuntimeService {
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
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
        // External facts (including worker reports, signals, and timer firings) are
        // committed while paused, but they must not advance deterministic work or
        // materialize new eligibility until an explicit resume transitions the
        // projected lifecycle back to Running. Cancellation changes the lifecycle
        // to Cancelling and therefore continues to drain already-owned work.
        if projection.lifecycle() == RunLifecycle::Paused {
            return Ok(());
        }
        let received_signal_in_current_commit = events
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::SignalReceived { .. }));
        if !received_signal_in_current_commit && run_drain_reason(projection).is_none() {
            self.drain_broadcast_signals(run, occurred_at, projection, events, workspace)?;
        }
        if projection.lifecycle() == RunLifecycle::Running && projection.termination().is_none() {
            let root_scope = projection
                .root_scope()
                .ok_or_else(|| RuntimeError::InvalidHistory("run root scope is absent".to_owned()))?
                .reference()
                .clone();
            for node_id in entry_nodes(revision) {
                if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
                if node_occurrence_exists_for_current_pin(projection, node_id, &root_scope) {
                    continue;
                }
                let node = revision.semantic().nodes().get(node_id).ok_or_else(|| {
                    RuntimeError::InvalidHistory("current revision entry node is absent".to_owned())
                })?;
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::NodeBecameEligible {
                        node: node_id.clone(),
                        execution: self.next_execution_id()?,
                        scope: root_scope.clone(),
                        mode: node_execution_mode(node),
                    },
                )?;
            }
        }
        if let Some(reason) = run_drain_reason(projection).cloned() {
            let active_branches: Vec<_> = self
                .scan_branch_ids(run, projection, &mut branch_scan_remaining)?
                .into_iter()
                .filter(|branch| {
                    projection
                        .branches()
                        .get(branch)
                        .is_some_and(|branch| branch.state() == BranchState::Active)
                })
                .collect();
            for branch in active_branches {
                if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::BranchCancellationRequested {
                        branch,
                        reason: reason.clone(),
                    },
                )?;
            }
        }
        for _ in 0..MAX_DRIVER_PASSES {
            if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            let before = events.len();
            let eligible: Vec<_> = self
                .scan_eligible_execution_ids(run, projection, &mut eligible_scan_remaining)?
                .into_iter()
                .filter_map(|execution| {
                    let execution = projection.node_executions().get(&execution)?;
                    (execution.state() == &NodeExecutionState::Eligible
                        && (execution.mode() == NodeExecutionMode::Runtime
                            || run_drain_reason(projection).is_some()
                            || execution_branch_state(projection, execution.execution())
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
            for (execution, node_id, scope_reference) in eligible {
                if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
                let execution_revision = self.revision_for_execution(projection, &execution)?;
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
                let structurally_cancelling = run_drain_reason(projection).is_some()
                    || execution_branch_state(projection, &execution)
                        == Some(BranchState::Cancelling);
                if structurally_cancelling
                    && !matches!(
                        node.kind(),
                        NodeKind::Repeat { .. } | NodeKind::Subworkflow { .. }
                    )
                {
                    let timers: Vec<_> = projection
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
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::TimerCancelled {
                                timer,
                                reason: Reason::new(
                                    "structured cancellation released a pending timer",
                                )?,
                            },
                        )?;
                    }
                    if projection
                        .waits()
                        .get(&execution)
                        .is_some_and(|wait| wait.is_pending())
                    {
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::WaitCancelled {
                                execution: execution.clone(),
                                reason: Reason::new(
                                    "structured cancellation released a pending wait",
                                )?,
                            },
                        )?;
                    }
                    self.push_projected_event(
                        run,
                        occurred_at,
                        projection,
                        events,
                        RunEventKind::NodeExecutionCancelledBeforeDispatch {
                            execution: execution.clone(),
                            reason: Reason::new(
                                "execution was cancelled before an external dispatch boundary",
                            )?,
                        },
                    )?;
                    continue;
                }
                match node.kind() {
                    NodeKind::Task { .. } => {}
                    NodeKind::Terminal { outcome } => match outcome {
                        TerminalOutcome::Success => {
                            match self.materialize_success_terminal_outputs(
                                run,
                                occurred_at,
                                &execution_revision,
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
                    },
                    NodeKind::Wait { duration_ms } => {
                        if !projection.waits().contains_key(&execution) {
                            let timer = self.next_timer_id()?;
                            let fire_at = checked_timestamp_add(occurred_at, *duration_ms)?;
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
                                WaitCondition::Timer { timer }
                                | WaitCondition::SignalOrTimer { timer, .. }
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
                            self.complete_deterministic(
                                run,
                                occurred_at,
                                projection,
                                events,
                                node,
                                &execution,
                            )?;
                        }
                    }
                    NodeKind::SignalWait { signal } => {
                        if !projection.waits().contains_key(&execution) {
                            let signal_type = milkdrift_persistence::SignalTypeId::new(
                                signal.as_str().to_owned(),
                            )?;
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
                                .map(|candidate| {
                                    (candidate.signal().clone(), candidate.payload().clone())
                                });
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
                                    let key = ValueKey::new(port.as_str().to_owned()).map_err(
                                        |error| RuntimeError::Scheduling(error.to_string()),
                                    )?;
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
                            self.complete_deterministic(
                                run,
                                occurred_at,
                                projection,
                                events,
                                node,
                                &execution,
                            )?;
                        }
                    }
                    NodeKind::Branch { config } => {
                        if !projection.branch_routes().contains_key(&execution) {
                            let mut selected = None;
                            let context = match self.evaluation_context(
                                node,
                                projection,
                                &scope_reference,
                                workspace,
                            ) {
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
                                    continue;
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
                                continue;
                            }
                            let Some(selected) = selected.or_else(|| config.fallback().cloned())
                            else {
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
                                continue;
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
                            self.complete_deterministic(
                                run,
                                occurred_at,
                                projection,
                                events,
                                node,
                                &execution,
                            )?;
                        }
                    }
                    NodeKind::Fork { config } => {
                        let parent = projection
                            .scopes()
                            .get(&scope_reference)
                            .ok_or_else(|| {
                                RuntimeError::InvalidHistory(
                                    "fork execution scope is absent".to_owned(),
                                )
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
                            let scope = WorkspaceScope::branch(
                                self.next_scope_id()?,
                                &parent,
                                branch.clone(),
                            )
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
                        let expansion_complete = config.branches().iter().all(|port| {
                            projection.branch_for_fork_port(&execution, port).is_some()
                        });
                        if expansion_complete {
                            self.complete_deterministic(
                                run,
                                occurred_at,
                                projection,
                                events,
                                node,
                                &execution,
                            )?;
                        }
                    }
                    NodeKind::Reducer { config } => self.drive_reducer(
                        run,
                        occurred_at,
                        projection,
                        events,
                        workspace,
                        node,
                        &execution,
                        &scope_reference,
                        config,
                        &execution_revision,
                    )?,
                    NodeKind::Repeat { config } => self.drive_repeat_intent(
                        run,
                        occurred_at,
                        projection,
                        events,
                        workspace,
                        node,
                        &execution,
                        &scope_reference,
                        config,
                    )?,
                    NodeKind::Subworkflow { reference } => {
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
                    }
                    NodeKind::Join { config } => {
                        if !projection.joins().contains_key(&execution) {
                            self.try_satisfy_join(
                                run,
                                occurred_at,
                                &execution_revision,
                                projection,
                                events,
                                node,
                                &execution,
                                config,
                            )?;
                        }
                    }
                }
                if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
            }

            self.close_finished_branches(
                run,
                occurred_at,
                revision,
                projection,
                events,
                &mut branch_scan_remaining,
            )?;
            self.add_ready_successors(
                run,
                occurred_at,
                revision,
                projection,
                events,
                &mut successor_scan_remaining,
            )?;
            self.try_finalize_run(run, occurred_at, revision, projection, events, workspace)?;
            if events.len() == before || projection.is_completed() {
                return Ok(());
            }
            if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
        }
        Err(RuntimeError::Scheduling(
            "structured driver did not converge within its bounded pass count".to_owned(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn drive_reducer(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: &NodeExecutionId,
        scope_reference: &ScopeReference,
        config: &milkdrift_blueprint::ReducerConfig,
        revision: &BlueprintRevision,
    ) -> Result<(), RuntimeError> {
        if matches!(config.strategy(), ReducerStrategy::Capability(_)) {
            return Ok(());
        }
        if !projection
            .node_executions()
            .get(execution)
            .is_some_and(|value| value.outputs().is_empty())
        {
            return self.complete_deterministic(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
            );
        }
        let values = self.ordered_reducer_references(
            revision,
            projection,
            node,
            config.input_port(),
            scope_reference,
            workspace,
        )?;
        if values.len() < usize::from(config.minimum_items()) {
            return Ok(());
        }
        let output_port = node.data_outputs().keys().next().ok_or_else(|| {
            RuntimeError::Scheduling(format!("reducer node {} has no output port", node.id()))
        })?;
        let key = ValueKey::new(output_port.as_str().to_owned())
            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
        let (value, artifact) = match config.strategy() {
            ReducerStrategy::Collect => {
                let Ok(json_value) = serde_json::to_value(&values) else {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "deterministic reducer result could not be serialized",
                        )?),
                    );
                };
                let Ok(collected) = BoundedJson::new(json_value) else {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "deterministic reducer result exceeds the bounded JSON contract",
                        )?),
                    );
                };
                (WorkspaceValue::Json(collected), None)
            }
            ReducerStrategy::First => {
                let reference = values.first().ok_or_else(|| {
                    RuntimeError::Scheduling("first reducer has no input".to_owned())
                })?;
                let entry = self.projected_workspace_value(projection, reference, workspace)?;
                let artifact = entry.value().as_artifact().cloned();
                (entry.value().clone(), artifact)
            }
            ReducerStrategy::Capability(_) => return Ok(()),
        };
        let entry =
            self.projected_output_entry(projection, scope_reference, key, value, workspace)?;
        let reference = entry.reference().clone();
        workspace.push(WorkspaceMutation::PutValue { entry });
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::DeterministicOutputPublished {
                execution: execution.clone(),
                value: reference,
                artifact,
            },
        )?;
        self.complete_deterministic(run, occurred_at, projection, events, node, execution)
    }

    #[allow(clippy::too_many_arguments)]
    fn create_subworkflow_intent(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        parent_execution: &NodeExecutionId,
        occurrence_scope: &ScopeReference,
        parent_scope: &ScopeReference,
        reference: &milkdrift_blueprint::PinnedSubworkflow,
    ) -> Result<(), RuntimeError> {
        let child_revision =
            self.load_validated_revision(reference.revision(), Some(reference.workflow()))?;
        let parent_revision = self.revision_for_execution(projection, parent_execution)?;
        let mut resolved_inputs = Vec::new();
        for (field, interface_field) in child_revision.semantic().interface().inputs() {
            let port = PortId::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let Some(parent_declaration) = node.data_inputs().get(&port) else {
                if interface_field.is_required() {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        parent_execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "required subworkflow input has no parent node data port",
                        )?),
                    );
                }
                continue;
            };
            let resolved = match self.resolve_node_port_inputs(
                &parent_revision,
                projection,
                node,
                &port,
                occurrence_scope,
                workspace,
            ) {
                Ok(resolved) => resolved,
                Err(RuntimeError::Scheduling(_)) => {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        parent_execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "subworkflow inputs could not be resolved from immutable parent data",
                        )?),
                    );
                }
                Err(error) => return Err(error),
            };
            if resolved.is_empty() {
                if interface_field.is_required() || parent_declaration.is_required() {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        parent_execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "required subworkflow input is absent from immutable parent data",
                        )?),
                    );
                }
                continue;
            }
            if resolved.len() != 1 {
                return self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    parent_execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "subworkflow input resolved to more than one immutable value",
                    )?),
                );
            }
            let key = ValueKey::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let resolved_value = resolved.into_iter().next().ok_or_else(|| {
                RuntimeError::InvalidHistory("resolved subworkflow input disappeared".to_owned())
            })?;
            resolved_inputs.push((key, resolved_value));
        }
        let parent = projection.scopes().get(parent_scope).ok_or_else(|| {
            RuntimeError::InvalidHistory("subworkflow parent scope is absent".to_owned())
        })?;
        let subworkflow = self.next_subworkflow_id()?;
        let scope = WorkspaceScope::subworkflow(self.next_scope_id()?, parent, subworkflow.clone())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let scope_reference = scope.reference().clone();
        workspace.push(WorkspaceMutation::CreateScope {
            scope: scope.clone(),
        });
        let mut inputs = Vec::new();
        for (key, resolved_value) in resolved_inputs {
            let entry = self.materialize_subworkflow_input(
                projection,
                workspace,
                &scope_reference,
                parent_scope,
                key,
                resolved_value,
            )?;
            inputs.push(entry.reference().clone());
            workspace.push(WorkspaceMutation::PutValue { entry });
        }
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::SubworkflowCreated {
                subworkflow,
                parent_execution: parent_execution.clone(),
                child_run: self.next_run_id()?,
                child_revision: reference.revision().clone(),
                scope: scope.clone(),
                ownership: SubworkflowOwnership::Attached,
                inputs,
            },
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn drive_repeat_intent(
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
                (false, RepeatTermination::SucceedWithLatest) if has_success => {
                    NodeOutcome::Succeeded
                }
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
            return self.complete_deterministic_with_outcome(
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

    #[allow(clippy::too_many_arguments)]
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

    #[allow(clippy::too_many_arguments)]
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
        let mut observed_cost = 0_u64;
        for child in projection
            .subworkflows()
            .values()
            .filter(|child| child.parent_execution() == execution)
        {
            if self.store.head(child.child_run())? == RunSequence::ZERO {
                continue;
            }
            let child_projection = self.projection(child.child_run())?;
            if let Some(cost) = child_projection
                .resource_usage()
                .cost_micros()
                .get(&currency)
            {
                let Some(total) = observed_cost.checked_add(*cost) else {
                    return Ok(RepeatBudgetStatus::AccountingOverflow);
                };
                observed_cost = total;
            }
        }
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

    #[allow(clippy::too_many_arguments)]
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
