use std::collections::BTreeSet;

use milkdrift_capability::SideEffectClass;
use milkdrift_persistence::{AttemptId, LeaseId, NodeExecutionId, TimerId, WaitCondition};
use milkdrift_workspace::{IterationId, ScopeKind, ScopeReference, SubworkflowId};

use crate::RuntimeError;

use super::{
    AttemptState, NodeExecutionState, RunProjection, SettledNodeExecutionProjection,
    SignalProjection,
};

impl RunProjection {
    /// Deterministically retires state whose last semantic consumer was closed by
    /// an authoritative event. This runs after every fold transition; snapshots
    /// therefore serialize the same bounded state used by command planning.
    pub(super) fn compact_settled_state(&mut self) -> Result<(), RuntimeError> {
        self.history_compacted_through = self.sequence;
        self.compact_observations();
        self.compact_recovery_passes();
        self.compact_wait_state();
        self.compact_repeat_state();
        self.compact_subworkflows();
        self.compact_reconciliation_history();
        self.compact_execution_frontier()?;
        self.compact_retry_and_lease_state();
        self.compact_identity_summaries();
        self.compact_completed_structured_state();
        self.compact_subworkflows();
        self.compact_settled_execution_summaries();
        self.compact_scopes();
        self.validate_compacted_state()
    }

    fn compact_observations(&mut self) {
        for attempt in self.attempts.values_mut() {
            keep_latest(&mut attempt.progress);
            keep_latest(&mut attempt.cancellation_acknowledgements);
            keep_latest(&mut attempt.recovery);
            if let Some(obligation) = attempt.obligation.as_mut() {
                keep_latest(&mut obligation.decisions);
            }
        }
        for signal in self.signals.values_mut() {
            keep_latest(&mut signal.duplicate_commands);
        }
        for continuation in self.repeat_continuations.values_mut() {
            keep_latest(&mut continuation.requests);
            keep_latest(&mut continuation.decisions);
        }
    }

    fn compact_recovery_passes(&mut self) {
        if self.recovery.len() > 1 {
            let latest = self.recovery.pop();
            self.recovery.clear();
            self.recovery.extend(latest);
        }
        self.current_recovery = (!self.recovery.is_empty()).then_some(0);
    }

    fn compact_retry_and_lease_state(&mut self) {
        self.retries.retain(|_, retry| retry.is_pending());
        self.retry_by_attempt
            .retain(|_, timer| self.retries.contains_key(timer));

        let mut retained_timers: BTreeSet<TimerId> = self.pending_timer_ids.clone();
        retained_timers.extend(self.retries.keys().cloned());
        for wait in self.waits.values().filter(|wait| wait.is_pending()) {
            match wait.condition() {
                WaitCondition::Timer { timer } | WaitCondition::SignalOrTimer { timer, .. } => {
                    retained_timers.insert(timer.clone());
                }
                WaitCondition::Signal { .. } => {}
            }
        }
        self.timers
            .retain(|timer, _| retained_timers.contains(timer));
        self.pending_timers_by_execution.retain(|_, timers| {
            timers.retain(|timer| self.timers.contains_key(timer));
            !timers.is_empty()
        });

        let mut retained_leases: BTreeSet<LeaseId> =
            self.active_lease_by_attempt.values().cloned().collect();
        for attempt in self.attempts.values() {
            if matches!(attempt.state, AttemptState::Leased | AttemptState::Running)
                || attempt.is_unresolved()
            {
                retained_leases.extend(attempt.leases.last().cloned());
            }
            if let Some(recovery) = attempt.recovery.last() {
                retained_leases.extend(recovery.lease.clone());
            }
        }
        self.leases
            .retain(|lease, _| retained_leases.contains(lease));
        for attempt in self.attempts.values_mut() {
            attempt
                .leases
                .retain(|lease| self.leases.contains_key(lease));
        }

        let mut retained_attempts: BTreeSet<AttemptId> = self.active_attempt_ids.clone();
        for execution in self.node_executions.values() {
            retained_attempts.extend(execution.attempts.last().cloned());
            let latest_is_not_fully_classified = execution
                .attempts
                .last()
                .and_then(|attempt| self.attempts.get(attempt))
                .is_some_and(|attempt| attempt.side_effect.is_none());
            if latest_is_not_fully_classified {
                retained_attempts.extend(execution.attempts.iter().rev().nth(1).cloned());
            }
        }
        for retry in self.retries.values() {
            retained_attempts.insert(retry.previous_attempt.clone());
            retained_attempts.insert(retry.next_attempt.clone());
        }
        for cancellation in self.reconciliation_cancellations.values() {
            retained_attempts.insert(cancellation.attempt.clone());
        }
        for remediation in self.reconciliation_remediations.values() {
            retained_attempts.extend(remediation.source_attempt.clone());
        }
        for attempt in self.attempts.values() {
            if attempt.is_unresolved() {
                retained_attempts.insert(attempt.attempt.clone());
            }
        }
        self.attempts
            .retain(|attempt, _| retained_attempts.contains(attempt));
        for execution in self.node_executions.values_mut() {
            execution
                .attempts
                .retain(|attempt| self.attempts.contains_key(attempt));
        }
    }

    fn compact_wait_state(&mut self) {
        let completed_signals: BTreeSet<_> = self
            .signals
            .iter()
            .filter(|(_, signal)| {
                signal_is_settled(signal)
                    && !signal.consumed_by.iter().any(|execution| {
                        self.waits
                            .get(execution)
                            .is_some_and(|wait| wait.is_pending())
                    })
            })
            .map(|(signal, _)| signal.clone())
            .collect();
        self.signals
            .retain(|signal, _| !completed_signals.contains(signal));
        self.pending_broadcast_signals
            .retain(|(_, signal)| self.signals.contains_key(signal));

        self.waits.retain(|execution, wait| {
            wait.is_pending()
                || self
                    .node_executions
                    .get(execution)
                    .is_some_and(|owner| !owner.is_completed())
        });
        self.pending_wait_execution_ids.retain(|execution| {
            self.waits
                .get(execution)
                .is_some_and(|wait| wait.is_pending())
        });
    }

    fn compact_repeat_state(&mut self) {
        let retained_iterations: BTreeSet<IterationId> = self
            .active_iteration_ids
            .iter()
            .cloned()
            .chain(self.latest_iteration.values().cloned())
            .collect();
        self.iterations
            .retain(|iteration, _| retained_iterations.contains(iteration));
        self.active_iteration_ids
            .retain(|iteration| self.iterations.contains_key(iteration));
        self.latest_iteration
            .retain(|_, iteration| self.iterations.contains_key(iteration));
    }

    fn compact_subworkflows(&mut self) {
        let retained_iterations: BTreeSet<_> = self
            .iterations
            .values()
            .map(|iteration| iteration.scope.reference().clone())
            .collect();
        let retained: BTreeSet<SubworkflowId> = self
            .subworkflows
            .iter()
            .filter(|(_, child)| {
                let parent_is_iteration = child
                    .scope
                    .parent()
                    .and_then(|scope| self.scopes.get(scope))
                    .is_some_and(|scope| matches!(scope.kind(), ScopeKind::Iteration { .. }));
                child.is_active()
                    || !child.outputs_fully_imported()
                    || child
                        .scope
                        .parent()
                        .is_some_and(|scope| retained_iterations.contains(scope))
                    || (!parent_is_iteration
                        && self
                            .node_executions
                            .get(&child.parent_execution)
                            .is_some_and(|parent| !parent.is_completed()))
            })
            .map(|(subworkflow, _)| subworkflow.clone())
            .collect();
        self.subworkflows
            .retain(|subworkflow, _| retained.contains(subworkflow));
        self.active_subworkflow_ids
            .retain(|subworkflow| self.subworkflows.contains_key(subworkflow));
        self.active_attached_subworkflow_ids
            .retain(|subworkflow| self.subworkflows.contains_key(subworkflow));
        self.child_runs = self
            .subworkflows
            .values()
            .map(|child| child.child_run.clone())
            .collect();
        self.subworkflow_usage_by_execution.retain(|execution, _| {
            self.node_executions
                .get(execution)
                .is_some_and(|parent| !parent.is_completed())
                || self.controller_assessments.contains_key(execution)
        });
    }

    fn compact_reconciliation_history(&mut self) {
        if let Some(current) = self.reconciliation.current_request.clone() {
            self.reconciliation
                .requests
                .retain(|identity, _| identity == &current);
            let current_plan = self
                .reconciliation
                .requests
                .get(&current)
                .and_then(|request| request.plan.clone());
            self.reconciliation
                .plans
                .retain(|identity, _| Some(identity) == current_plan.as_ref());
        } else {
            self.reconciliation.requests.clear();
            self.reconciliation.plans.clear();
        }
        self.reconciliation_cancellations
            .retain(|execution, cancellation| {
                self.node_executions
                    .get(execution)
                    .is_some_and(|execution| {
                        !execution.is_completed()
                            || self
                                .reconciliation
                                .plans
                                .get(cancellation.plan())
                                .is_some_and(|plan| plan.is_pending())
                    })
            });
        self.reconciliation_remediations.retain(|execution, _| {
            self.node_executions
                .get(execution)
                .is_some_and(|execution| !execution.is_completed())
        });
        self.remediations.retain(|execution, _| {
            self.node_executions
                .get(execution)
                .is_some_and(|execution| !execution.is_completed())
        });
    }

    fn compact_execution_frontier(&mut self) -> Result<(), RuntimeError> {
        let retired: BTreeSet<_> = self
            .node_executions
            .iter()
            .filter(|(identity, execution)| {
                execution.is_completed()
                    && !self.active_execution_ids.contains(*identity)
                    && !self.pending_successor_executions.contains(*identity)
                    && !self.reserved_executions.contains(*identity)
                    && !self.eligible_executions.contains(*identity)
                    && self
                        .waits
                        .get(*identity)
                        .is_none_or(|wait| !wait.is_pending())
                    && !self
                        .active_structured_children_by_execution
                        .contains_key(*identity)
                    && !self.reconciliation_cancellations.contains_key(*identity)
                    && !self.reconciliation_remediations.contains_key(*identity)
                    && !self.remediations.contains_key(*identity)
                    && !self.subworkflows.values().any(|child| {
                        child.parent_execution == **identity
                            && (child.is_active() || !child.outputs_fully_imported())
                    })
                    && self.execution_scope_is_closed(execution)
            })
            .map(|(identity, _)| identity.clone())
            .collect();
        if retired.is_empty() {
            return Ok(());
        }
        for identity in &retired {
            let execution = self.node_executions.remove(identity).ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "retired execution disappeared during deterministic compaction".to_owned(),
                )
            })?;
            let latest_attempt = execution.attempts.last().cloned();
            let terminal_sequence = execution
                .deterministic_terminal
                .as_ref()
                .map(|terminal| terminal.sequence())
                .or_else(|| {
                    latest_attempt
                        .as_ref()
                        .and_then(|attempt| self.attempts.get(attempt))
                        .and_then(|attempt| attempt.terminal.as_ref())
                        .map(|terminal| terminal.sequence())
                });
            let mut side_effect = execution
                .attempts
                .iter()
                .filter_map(|attempt| self.attempts.get(attempt))
                .filter_map(|attempt| attempt.side_effect.as_ref())
                .map(|classification| classification.side_effect)
                .max_by_key(|effect| side_effect_rank(*effect))
                .unwrap_or(SideEffectClass::None);
            let key = (execution.scope.clone(), execution.node.clone());
            if let Some(previous) = self
                .settled_execution_by_scope_node
                .get(&key)
                .and_then(|previous| self.settled_node_executions.get(previous))
                && side_effect_rank(previous.side_effect()) > side_effect_rank(side_effect)
            {
                side_effect = previous.side_effect();
            }
            let is_latest_frontier = self
                .settled_execution_by_scope_node
                .get(&key)
                .and_then(|previous| self.settled_node_executions.get(previous))
                .is_none_or(|previous| previous.created_sequence() <= execution.created_sequence);
            let route = self.branch_routes.remove(identity);
            if !is_latest_frontier {
                if let Some(index) = self.execution_ids_by_node.get_mut(&execution.node) {
                    index.remove(identity);
                }
                self.branch_owner.remove(identity);
                for branch in self.branches.values_mut() {
                    branch.children.remove(identity);
                }
                continue;
            }
            if let Some(previous) = self
                .settled_execution_by_scope_node
                .insert(key, identity.clone())
            {
                self.settled_node_executions.remove(&previous);
                if let Some(index) = self.execution_ids_by_node.get_mut(&execution.node) {
                    index.remove(&previous);
                }
                self.branch_owner.remove(&previous);
                for branch in self.branches.values_mut() {
                    branch.children.remove(&previous);
                }
            }
            self.settled_node_executions.insert(
                identity.clone(),
                SettledNodeExecutionProjection {
                    execution: identity.clone(),
                    node: execution.node,
                    scope: execution.scope,
                    mode: execution.mode,
                    revision: execution.revision,
                    epoch_retired_sequence: execution.epoch_retired_sequence,
                    created_sequence: execution.created_sequence,
                    attempt_count: execution.attempt_count,
                    attempts: latest_attempt.into_iter().collect(),
                    state: execution.state,
                    terminal_sequence,
                    side_effect,
                    route,
                    outputs: execution.outputs,
                },
            );
        }
        self.execution_ids_by_node.retain(|_, executions| {
            executions.retain(|identity| {
                self.node_executions.contains_key(identity)
                    || self.settled_node_executions.contains_key(identity)
            });
            !executions.is_empty()
        });
        self.latest_descendant_execution_by_scope_node
            .retain(|_, identity| {
                self.node_executions.contains_key(identity)
                    || self.settled_node_executions.contains_key(identity)
            });
        self.eligible_executions
            .retain(|identity| !retired.contains(identity));
        self.subworkflow_usage_by_execution.retain(|identity, _| {
            !retired.contains(identity) || self.controller_assessments.contains_key(identity)
        });
        Ok(())
    }

    fn execution_scope_is_closed(&self, execution: &super::NodeExecutionProjection) -> bool {
        let Some(scope) = self.scopes.get(&execution.scope) else {
            return false;
        };
        match scope.kind() {
            ScopeKind::RunRoot => {
                matches!(
                    execution.state,
                    NodeExecutionState::Terminal(milkdrift_persistence::NodeOutcome::Succeeded)
                ) || self.lifecycle.is_completed()
                    || matches!(execution.state, NodeExecutionState::RemovedProspectively(_))
            }
            ScopeKind::Branch { branch } => self
                .branches
                .get(branch)
                .is_some_and(|branch| branch.is_completed()),
            ScopeKind::Iteration { iteration } => self
                .iterations
                .get(iteration)
                .is_some_and(|iteration| !matches!(iteration.state, super::IterationState::Active)),
            ScopeKind::Subworkflow { subworkflow } => self
                .subworkflows
                .get(subworkflow)
                .is_none_or(|child| child.is_completed()),
        }
    }

    fn compact_identity_summaries(&mut self) {
        let retained_invocations: BTreeSet<_> = self
            .attempts
            .values()
            .filter_map(|attempt| attempt.invocation.clone())
            .collect();
        self.invocations
            .retain(|invocation| retained_invocations.contains(invocation));
        self.recovery_decisions
            .retain(|decision, (attempt, outcome)| {
                self.attempts.get(attempt).is_some_and(|attempt| {
                    attempt
                        .obligation
                        .as_ref()
                        .is_some_and(|obligation| match outcome {
                            milkdrift_persistence::AuthorityDecision::Retain => {
                                obligation.retained.is_none()
                            }
                            milkdrift_persistence::AuthorityDecision::Compensate => !self
                                .remediations
                                .values()
                                .any(|remediation| remediation.decision == *decision),
                            milkdrift_persistence::AuthorityDecision::Approve
                            | milkdrift_persistence::AuthorityDecision::Reject
                            | milkdrift_persistence::AuthorityDecision::Query
                            | milkdrift_persistence::AuthorityDecision::Retry
                            | milkdrift_persistence::AuthorityDecision::ResolveSucceeded
                            | milkdrift_persistence::AuthorityDecision::ResolveFailed => false,
                        })
                })
            });
    }

    fn compact_completed_structured_state(&mut self) {
        let closed_repeats: BTreeSet<_> = self
            .repeat_terminations
            .keys()
            .filter(|execution| {
                self.settled_node_executions.contains_key(*execution)
                    && !self.pending_successor_executions.contains(*execution)
            })
            .cloned()
            .collect();
        for repeat in &closed_repeats {
            if let Some(iteration) = self.latest_iteration.remove(repeat) {
                self.active_iteration_ids.remove(&iteration);
                self.iterations.remove(&iteration);
            }
            self.repeat_continuations.remove(repeat);
            self.repeat_terminations.remove(repeat);
        }

        let removable_joins: BTreeSet<_> = self
            .joins
            .iter()
            .filter(|(execution, _)| {
                self.settled_node_executions
                    .get(*execution)
                    .is_some_and(|summary| self.scope_frontier_is_closed(summary.scope()))
            })
            .map(|(execution, _)| execution.clone())
            .collect();
        let joined_branches: BTreeSet<_> = removable_joins
            .iter()
            .filter_map(|execution| self.joins.get(execution))
            .flat_map(|join| {
                join.branches
                    .iter()
                    .map(|result| result.branch.clone())
                    .chain(join.retained_branches.iter().cloned())
            })
            .collect();
        self.joins
            .retain(|execution, _| !removable_joins.contains(execution));

        let removable_branches: BTreeSet<_> = self
            .branches
            .iter()
            .filter(|(branch, value)| {
                value.is_completed()
                    && (joined_branches.contains(*branch)
                        || self.scope_frontier_is_closed(value.scope.reference()))
            })
            .map(|(branch, _)| branch.clone())
            .collect();
        let removed_branch_executions: BTreeSet<NodeExecutionId> = self
            .branch_owner
            .iter()
            .filter(|(_, branch)| removable_branches.contains(*branch))
            .map(|(execution, _)| execution.clone())
            .collect();
        self.branches
            .retain(|branch, _| !removable_branches.contains(branch));
        self.branch_by_fork_port
            .retain(|_, branch| !removable_branches.contains(branch));
        self.branch_ids_by_fork_execution.retain(|_, branches| {
            branches.retain(|branch| !removable_branches.contains(branch));
            !branches.is_empty()
        });
        self.active_branch_ids
            .retain(|branch| !removable_branches.contains(branch));
        self.cancelling_branch_ids
            .retain(|branch| !removable_branches.contains(branch));
        self.branch_owner.retain(|execution, branch| {
            !removed_branch_executions.contains(execution) && !removable_branches.contains(branch)
        });
    }

    fn scope_frontier_is_closed(&self, scope: &ScopeReference) -> bool {
        if self.lifecycle.is_completed() {
            return true;
        }
        let mut cursor = Some(scope);
        for _ in 0..milkdrift_workspace::MAX_SCOPE_DEPTH {
            let Some(reference) = cursor else {
                break;
            };
            let Some(scope) = self.scopes.get(reference) else {
                return true;
            };
            match scope.kind() {
                ScopeKind::Iteration { iteration } => {
                    return !self.iterations.contains_key(iteration);
                }
                ScopeKind::Branch { branch } if !self.branches.contains_key(branch) => {
                    return true;
                }
                ScopeKind::RunRoot | ScopeKind::Branch { .. } | ScopeKind::Subworkflow { .. } => {}
            }
            cursor = scope.parent();
        }
        false
    }

    fn compact_settled_execution_summaries(&mut self) {
        let retained: BTreeSet<_> = self
            .settled_node_executions
            .iter()
            .filter(|(_, summary)| {
                self.scopes
                    .get(summary.scope())
                    .is_some_and(|scope| match scope.kind() {
                        ScopeKind::RunRoot => {
                            !matches!(
                                summary.state(),
                                NodeExecutionState::RemovedProspectively(plan)
                                    if self.reconciliation.plans.get(plan).is_none_or(|plan| !plan.is_pending())
                            )
                        }
                        ScopeKind::Branch { branch } => self.branches.contains_key(branch),
                        ScopeKind::Iteration { iteration } => {
                            self.iterations.contains_key(iteration)
                        }
                        ScopeKind::Subworkflow { subworkflow } => {
                            self.subworkflows.contains_key(subworkflow)
                        }
                    })
            })
            .map(|(execution, _)| execution.clone())
            .collect();
        self.settled_node_executions
            .retain(|execution, _| retained.contains(execution));
        self.settled_execution_by_scope_node
            .retain(|_, execution| retained.contains(execution));
        self.execution_ids_by_node.retain(|_, executions| {
            executions.retain(|execution| {
                self.node_executions.contains_key(execution)
                    || self.settled_node_executions.contains_key(execution)
            });
            !executions.is_empty()
        });
        self.latest_descendant_execution_by_scope_node
            .retain(|_, execution| {
                self.node_executions.contains_key(execution)
                    || self.settled_node_executions.contains_key(execution)
            });
        self.controller_assessments.retain(|execution, _| {
            self.node_executions.contains_key(execution)
                || self.settled_node_executions.contains_key(execution)
        });
        self.subworkflow_usage_by_execution.retain(|execution, _| {
            self.node_executions.contains_key(execution)
                || self.controller_assessments.contains_key(execution)
        });
    }

    fn compact_scopes(&mut self) {
        let mut retained: BTreeSet<ScopeReference> = BTreeSet::new();
        retained.extend(
            self.root_scope
                .as_ref()
                .map(|scope| scope.reference().clone()),
        );
        retained.extend(
            self.node_executions
                .values()
                .map(|execution| execution.scope.clone()),
        );
        retained.extend(
            self.settled_node_executions
                .values()
                .map(|execution| execution.scope.clone()),
        );
        retained.extend(
            self.branches
                .values()
                .map(|branch| branch.scope.reference().clone()),
        );
        retained.extend(
            self.iterations
                .values()
                .map(|iteration| iteration.scope.reference().clone()),
        );
        retained.extend(
            self.subworkflows
                .values()
                .map(|child| child.scope.reference().clone()),
        );
        retained.extend(
            self.workspace_values
                .iter()
                .map(|value| value.scope().clone()),
        );
        retained.extend(self.active_scope_ownership.keys().cloned());
        retained.extend(
            self.pending_reconciliation_restarts
                .keys()
                .map(|(_, scope)| scope.clone()),
        );

        let mut frontier: Vec<_> = retained.iter().cloned().collect();
        while let Some(scope) = frontier.pop() {
            let Some(parent) = self
                .scopes
                .get(&scope)
                .and_then(|scope| scope.parent())
                .cloned()
            else {
                continue;
            };
            if retained.insert(parent.clone()) {
                frontier.push(parent);
            }
        }
        self.scopes.retain(|scope, _| retained.contains(scope));
        self.latest_descendant_execution_by_scope_node
            .retain(|(scope, _), execution| {
                self.scopes.contains_key(scope)
                    && (self.node_executions.contains_key(execution)
                        || self.settled_node_executions.contains_key(execution))
            });
    }

    pub(crate) fn validate_compacted_state(&self) -> Result<(), RuntimeError> {
        let invalid = |reason: &str| RuntimeError::InvalidHistory(reason.to_owned());
        let retained_signal_payload_bytes = self.signals.values().try_fold(
            0_usize,
            |total, signal| -> Result<usize, RuntimeError> {
                let payload = serde_json::to_vec(signal.payload()).map_err(|_| {
                    invalid("retained signal payload could not be deterministically serialized")
                })?;
                total
                    .checked_add(payload.len())
                    .ok_or_else(|| invalid("retained signal payload byte count overflowed"))
            },
        )?;
        if self.signals.len() > super::MAX_PENDING_SIGNAL_COUNT
            || retained_signal_payload_bytes > super::MAX_PENDING_SIGNAL_PAYLOAD_BYTES
        {
            return Err(invalid(
                "retained signal count or aggregate payload bytes exceed the pending budget",
            ));
        }
        if self
            .attempts
            .values()
            .any(|attempt| attempt.lease_workers.len() > 1)
        {
            return Err(invalid(
                "attempt retains more than one worker that crossed the start boundary",
            ));
        }
        if self.settled_node_executions.values().any(|summary| {
            self.scopes
                .get(summary.scope())
                .is_some_and(|scope| matches!(scope.kind(), ScopeKind::RunRoot))
                && matches!(
                    summary.state(),
                    NodeExecutionState::RemovedProspectively(plan)
                        if self.reconciliation.plans.get(plan).is_none_or(|plan| !plan.is_pending())
                )
        }) {
            return Err(invalid(
                "applied prospective removal remains in the root execution frontier",
            ));
        }
        if self
            .active_attempt_ids
            .iter()
            .any(|attempt| !self.attempts.contains_key(attempt))
        {
            return Err(invalid("active attempt references compacted state"));
        }
        if self.active_lease_by_attempt.iter().any(|(attempt, lease)| {
            !self.attempts.contains_key(attempt) || !self.leases.contains_key(lease)
        }) {
            return Err(invalid("active lease references compacted state"));
        }
        if self
            .pending_timer_ids
            .iter()
            .any(|timer| !self.timers.contains_key(timer))
        {
            return Err(invalid("pending timer references compacted state"));
        }
        if self
            .active_iteration_ids
            .iter()
            .any(|iteration| !self.iterations.contains_key(iteration))
        {
            return Err(invalid("active iteration references compacted state"));
        }
        if self.active_subworkflow_ids.iter().any(|child| {
            self.subworkflows
                .get(child)
                .is_none_or(|child| !child.is_active())
        }) {
            return Err(invalid("active subworkflow references compacted state"));
        }
        if self.subworkflows.values().any(|child| {
            child.imports.iter().enumerate().any(|(index, import)| {
                !child.outputs.contains(&import.child_value)
                    || child.imports[..index].iter().any(|prior| {
                        prior.child_value == import.child_value
                            || prior.parent_value == import.parent_value
                    })
                    || !self.workspace_values.contains(&import.parent_value)
            })
        }) {
            return Err(invalid(
                "subworkflow imports do not exactly reference unique terminal child outputs",
            ));
        }
        if self.node_executions.values().any(|execution| {
            matches!(
                &execution.state,
                NodeExecutionState::Scheduled(attempt)
                    | NodeExecutionState::Running(attempt)
                    | NodeExecutionState::RetryPending(attempt)
                    | NodeExecutionState::Uncertain(attempt)
                    if !self.attempts.contains_key(attempt)
            )
        }) {
            return Err(invalid("execution frontier references compacted attempt"));
        }
        if self.execution_ids_by_node.iter().any(|(node, executions)| {
            executions.iter().any(|execution| {
                self.current_node_execution(execution)
                    .is_none_or(|execution| execution.node() != node)
            })
        }) {
            return Err(invalid(
                "node execution index references compacted or mismatched state",
            ));
        }
        if self
            .settled_execution_by_scope_node
            .iter()
            .any(|((scope, node), execution)| {
                self.settled_node_executions
                    .get(execution)
                    .is_none_or(|summary| summary.scope() != scope || summary.node() != node)
            })
        {
            return Err(invalid(
                "settled execution index references mismatched state",
            ));
        }
        if self.branch_owner.iter().any(|(execution, branch)| {
            self.current_node_execution(execution).is_none() || !self.branches.contains_key(branch)
        }) {
            return Err(invalid("branch owner index references compacted state"));
        }
        if self.node_executions.iter().any(|(identity, execution)| {
            self.execution_ids_by_node
                .get(&execution.node)
                .is_none_or(|executions| !executions.contains(identity))
        }) || self
            .settled_node_executions
            .iter()
            .any(|(identity, execution)| {
                self.execution_ids_by_node
                    .get(execution.node())
                    .is_none_or(|executions| !executions.contains(identity))
                    || self
                        .settled_execution_by_scope_node
                        .get(&(execution.scope().clone(), execution.node().clone()))
                        != Some(identity)
            })
        {
            return Err(invalid(
                "execution owner is unreachable from its current indexes",
            ));
        }
        if self
            .branch_by_fork_port
            .iter()
            .any(|((fork, port), branch)| {
                self.current_node_execution(fork).is_none()
                    || self
                        .branches
                        .get(branch)
                        .is_none_or(|value| value.fork_execution != *fork || value.port != *port)
            })
            || self
                .branch_ids_by_fork_execution
                .iter()
                .any(|(fork, branches)| {
                    self.current_node_execution(fork).is_none()
                        || branches.iter().any(|branch| {
                            self.branches
                                .get(branch)
                                .is_none_or(|value| value.fork_execution != *fork)
                        })
                })
        {
            return Err(invalid(
                "branch index references compacted or mismatched ownership",
            ));
        }
        if self.branches.iter().any(|(identity, branch)| {
            self.current_node_execution(&branch.fork_execution)
                .is_none()
                || self
                    .branch_by_fork_port
                    .get(&(branch.fork_execution.clone(), branch.port.clone()))
                    != Some(identity)
                || self
                    .branch_ids_by_fork_execution
                    .get(&branch.fork_execution)
                    .is_none_or(|branches| !branches.contains(identity))
                || branch
                    .children
                    .iter()
                    .any(|child| self.current_node_execution(child).is_none())
        }) {
            return Err(invalid(
                "branch owner is unreachable from its current indexes",
            ));
        }
        if self.joins.values().any(|join| {
            self.current_node_execution(&join.execution).is_none()
                || join
                    .branches
                    .iter()
                    .any(|result| !self.branches.contains_key(&result.branch))
                || join
                    .retained_branches
                    .iter()
                    .any(|branch| !self.branches.contains_key(branch))
        }) {
            return Err(invalid(
                "join references compacted execution or branch state",
            ));
        }
        Ok(())
    }
}

fn keep_latest<T>(values: &mut Vec<T>) {
    if values.len() > 1 {
        let latest = values.pop();
        values.clear();
        values.extend(latest);
    }
}

fn signal_is_settled(signal: &SignalProjection) -> bool {
    !signal.is_pending()
        && (signal.mode != milkdrift_persistence::SignalDeliveryMode::Broadcast
            || signal.broadcast_scan_complete)
}

const fn side_effect_rank(effect: SideEffectClass) -> u8 {
    match effect {
        SideEffectClass::None => 0,
        SideEffectClass::ReadOnly => 1,
        SideEffectClass::IdempotentWrite => 2,
        SideEffectClass::NonIdempotentWrite => 3,
        SideEffectClass::Unknown => 4,
    }
}
