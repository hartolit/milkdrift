use std::collections::BTreeSet;

use milkdrift_persistence::{AttemptId, LeaseId, TimerId, WaitCondition};
use milkdrift_workspace::{IterationId, ScopeReference, SubworkflowId};

use crate::RuntimeError;

use super::{AttemptState, NodeExecutionState, RunProjection, SignalProjection};

impl RunProjection {
    /// Deterministically retires state whose last semantic consumer was closed by
    /// an authoritative event. This runs after every fold transition; snapshots
    /// therefore serialize the same bounded state used by command planning.
    pub(super) fn compact_settled_state(&mut self) -> Result<(), RuntimeError> {
        self.history_compacted_through = self.sequence;
        self.compact_observations();
        self.compact_recovery_passes();
        self.compact_retry_and_lease_state();
        self.compact_wait_state();
        self.compact_repeat_state();
        self.compact_subworkflows();
        self.compact_reconciliation_history();
        self.compact_retired_pending_executions();
        self.compact_identity_summaries();
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
        let latest_completed_by_parent: BTreeSet<SubworkflowId> = self
            .subworkflows
            .values()
            .filter(|child| child.is_completed())
            .fold(std::collections::BTreeMap::new(), |mut latest, child| {
                latest
                    .entry(child.parent_execution.clone())
                    .and_modify(
                        |current: &mut (milkdrift_persistence::RunSequence, SubworkflowId)| {
                            if child.created_sequence > current.0 {
                                *current = (child.created_sequence, child.subworkflow.clone());
                            }
                        },
                    )
                    .or_insert_with(|| (child.created_sequence, child.subworkflow.clone()));
                latest
            })
            .into_values()
            .map(|(_, subworkflow)| subworkflow)
            .collect();
        let retained: BTreeSet<SubworkflowId> = self
            .subworkflows
            .iter()
            .filter(|(_, child)| {
                child.is_active()
                    || child.outputs.len() != child.imports.len()
                    || child
                        .scope
                        .parent()
                        .is_some_and(|scope| retained_iterations.contains(scope))
                    || latest_completed_by_parent.contains(&child.subworkflow)
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
        self.reconciliation_cancellations.retain(|execution, _| {
            self.node_executions
                .get(execution)
                .is_some_and(|execution| !execution.is_completed())
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

    fn compact_retired_pending_executions(&mut self) {
        let retired: BTreeSet<_> = self
            .node_executions
            .iter()
            .filter(|(identity, execution)| {
                execution.epoch_retired_sequence.is_some()
                    && matches!(
                        execution.state,
                        NodeExecutionState::RemovedProspectively(_)
                            | NodeExecutionState::CancelledBeforeDispatch
                    )
                    && execution.attempts.is_empty()
                    && !self.active_execution_ids.contains(*identity)
                    && !self.pending_successor_executions.contains(*identity)
                    && !self.reserved_executions.contains(*identity)
                    && !self.waits.contains_key(*identity)
                    && !self.joins.contains_key(*identity)
                    && !self.latest_iteration.contains_key(*identity)
                    && !self.repeat_continuations.contains_key(*identity)
                    && !self.repeat_terminations.contains_key(*identity)
                    && !self
                        .subworkflows
                        .values()
                        .any(|child| child.parent_execution == **identity)
            })
            .map(|(identity, _)| identity.clone())
            .collect();
        if retired.is_empty() {
            return;
        }
        self.node_executions
            .retain(|identity, _| !retired.contains(identity));
        self.execution_ids_by_node.retain(|_, executions| {
            executions.retain(|identity| !retired.contains(identity));
            !executions.is_empty()
        });
        self.latest_descendant_execution_by_scope_node
            .retain(|_, identity| !retired.contains(identity));
        self.eligible_executions
            .retain(|identity| !retired.contains(identity));
        self.branch_routes
            .retain(|identity, _| !retired.contains(identity));
        self.subworkflow_usage_by_execution
            .retain(|identity, _| !retired.contains(identity));
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
                self.scopes.contains_key(scope) && self.node_executions.contains_key(execution)
            });
    }

    pub(crate) fn validate_compacted_state(&self) -> Result<(), RuntimeError> {
        let invalid = |reason: &str| RuntimeError::InvalidHistory(reason.to_owned());
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
