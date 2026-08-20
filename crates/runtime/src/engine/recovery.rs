//! Restart recovery, graceful cancellation propagation, and admission accounting.

use super::support::{
    CommandPlan, bounded_projection_set, cancellation_reason_for_branch,
    cancellation_reason_for_execution, checked_increment, recovery_classification, recovery_reason,
    run_drain_reason, unresolved_retry_error_class,
};
use super::{RecoveryResult, RuntimeService, STRUCTURED_EVENT_SOFT_LIMIT};
use crate::projection::{BranchState, NodeExecutionState, SubworkflowState};
use crate::{AdmissionUsage, RunCommand, RunCommandDocument, RuntimeError, WorkerReport};
use milkdrift_capability::{CancellationRequest, ErrorClass, SideEffectClass};
use milkdrift_persistence::{
    IntegrityDigest, PageSize, Reason, RecoveryClassification, RunEventKind, SubworkflowOwnership,
    TimestampMillis,
};
use milkdrift_workspace::{ScopeKind, WorkspaceScope};
use std::collections::BTreeMap;
use std::sync::TryLockError;
use std::sync::atomic::Ordering;
use tracing::{info_span, warn};

impl RuntimeService {
    /// Replays and classifies a bounded page of nonterminal runs. Expired dispatches
    /// become truthful uncertainty obligations; only work whose frozen side-effect
    /// and idempotency facts permit exact replay receives a bounded retry timer.
    #[allow(clippy::too_many_lines)]
    pub fn recover(&self) -> Result<RecoveryResult, RuntimeError> {
        let now = self.clock.now()?;
        let span = info_span!(
            "runtime.recovery",
            controller = %self.config.worker,
            observed_at = now.get(),
        );
        let _entered = span.enter();
        let _scheduler_guard = match self.scheduler_gate.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                return Err(RuntimeError::Scheduling(
                    "runtime scheduler or recovery pass is already active".to_owned(),
                ));
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(RuntimeError::Scheduling(
                    "runtime scheduler coordination lock is poisoned".to_owned(),
                ));
            }
        };
        let limit = PageSize::new(u32::from(self.config.maximum_tick_items))?;
        let mut result = RecoveryResult::default();
        let mut remaining = usize::from(self.config.maximum_tick_items);
        for summary in self.next_nonterminal_page(&self.recovery_cursor, limit, "recovery")? {
            if remaining == 0 {
                break;
            }
            let projection = self.projection(&summary.run)?;
            result.runs_examined = result.runs_examined.saturating_add(1);
            let scanned = bounded_projection_set(
                &summary.run,
                projection.active_attempt_ids(),
                &self.recovery_attempt_cursors,
                &mut remaining,
                "recovery attempt scan cursor",
            )?;
            let actionable: Vec<_> = scanned
                .iter()
                .filter_map(|attempt| projection.attempts().get(attempt))
                .filter(|attempt| {
                    if attempt.is_active() {
                        return projection
                            .active_lease_for_attempt(attempt.attempt())
                            .is_none_or(|lease| lease.expires_at() <= now);
                    }
                    if !attempt.is_unresolved()
                        || recovery_classification(attempt) != RecoveryClassification::Retryable
                    {
                        return false;
                    }
                    let Some(side_effect) = attempt.side_effect() else {
                        return false;
                    };
                    self.config.retry_policy.permits_automatic_retry(
                        attempt.attempt_number(),
                        unresolved_retry_error_class(attempt),
                        true,
                        side_effect.side_effect(),
                        side_effect.idempotency(),
                        side_effect.idempotency_key(),
                    )
                })
                .collect();
            if actionable.is_empty() {
                continue;
            }
            let mut plan = CommandPlan::one(RunEventKind::RecoveryStarted {
                controller: self.config.worker.clone(),
                through_sequence: projection.sequence(),
            });
            for attempt in actionable {
                if plan.events.len()
                    > milkdrift_persistence::MAX_EVENTS_PER_COMMIT.saturating_sub(4)
                {
                    break;
                }
                let active_lease = projection.active_lease_for_attempt(attempt.attempt());
                let classification = if attempt.is_completed() {
                    RecoveryClassification::TerminalObserved
                } else if let Some(lease) = active_lease {
                    if lease.expires_at() > now {
                        RecoveryClassification::LeaseStillValid
                    } else {
                        recovery_classification(attempt)
                    }
                } else if attempt.is_unresolved() {
                    recovery_classification(attempt)
                } else {
                    RecoveryClassification::NotStarted
                };
                if let Some(lease) = active_lease
                    && lease.expires_at() <= now
                {
                    plan.events.push(RunEventKind::LeaseExpired {
                        lease: lease.lease().clone(),
                        classification,
                    });
                    result.expired_leases = result.expired_leases.saturating_add(1);
                }
                plan.events.push(RunEventKind::RecoveryClassified {
                    attempt: attempt.attempt().clone(),
                    lease: active_lease.map(|lease| lease.lease().clone()),
                    classification,
                    reason: Reason::new(recovery_reason(classification))?,
                });
                match classification {
                    RecoveryClassification::Retryable => {
                        if !attempt.is_unresolved() {
                            plan.events.push(RunEventKind::ExternalOutcomeUncertain {
                                attempt: attempt.attempt().clone(),
                                report_sequence: self
                                    .next_report_sequence(&projection, attempt.attempt())?,
                                side_effect: attempt
                                    .side_effect()
                                    .map_or(SideEffectClass::Unknown, |classification| {
                                        classification.side_effect()
                                    }),
                                reason: Reason::new(
                                    "lease expired before an external outcome was observed",
                                )?,
                                evidence: Vec::new(),
                            });
                            result.uncertain = result.uncertain.saturating_add(1);
                        }
                        let side_effect = attempt.side_effect();
                        let retry_error = if active_lease.is_some() {
                            ErrorClass::Transport
                        } else {
                            unresolved_retry_error_class(attempt)
                        };
                        let permit = side_effect.is_some_and(|classification| {
                            self.config.retry_policy.permits_automatic_retry(
                                attempt.attempt_number(),
                                retry_error,
                                true,
                                classification.side_effect(),
                                classification.idempotency(),
                                classification.idempotency_key(),
                            )
                        });
                        if permit {
                            match self.build_retry_event(
                                attempt.execution(),
                                attempt.attempt(),
                                attempt.attempt_number(),
                                now,
                                retry_error,
                                None,
                                "recovery admitted a safe bounded retry after lease expiry",
                            ) {
                                Ok(retry) => {
                                    plan.events.push(retry);
                                    result.retryable = result.retryable.saturating_add(1);
                                }
                                Err(error) => warn!(
                                    attempt = %attempt.attempt(),
                                    reason = %error,
                                    "recovery uncertainty retained without an unavailable retry timer"
                                ),
                            }
                        }
                    }
                    RecoveryClassification::Uncertain if !attempt.is_unresolved() => {
                        let side_effect = attempt
                            .side_effect()
                            .map_or(SideEffectClass::Unknown, |value| value.side_effect());
                        plan.events.push(RunEventKind::ExternalOutcomeUncertain {
                            attempt: attempt.attempt().clone(),
                            report_sequence: self
                                .next_report_sequence(&projection, attempt.attempt())?,
                            side_effect,
                            reason: Reason::new(
                                "lease expired and external side effects cannot be established",
                            )?,
                            evidence: Vec::new(),
                        });
                        result.uncertain = result.uncertain.saturating_add(1);
                    }
                    RecoveryClassification::NotStarted
                    | RecoveryClassification::LeaseStillValid
                    | RecoveryClassification::TerminalObserved
                    | RecoveryClassification::Uncertain => {}
                }
            }
            let marker = projection.attempts().keys().next().cloned();
            let _ = self.commit_internal_plan(
                &summary.run,
                now,
                "recover_nonterminal_run",
                marker.as_ref(),
                plan,
            )?;
        }
        Ok(result)
    }

    /// Alias for hosts that name restart orchestration explicitly.
    pub fn recover_nonterminal_runs(&self) -> Result<RecoveryResult, RuntimeError> {
        self.recover()
    }

    pub(super) fn propagate_cancellation(
        &self,
        now: TimestampMillis,
        limit: PageSize,
    ) -> Result<(), RuntimeError> {
        for summary in
            self.next_nonterminal_page(&self.cancellation_cursor, limit, "cancellation")?
        {
            if self.structured_scan_budget.load(Ordering::Acquire) == 0 {
                break;
            }
            let projection = self.projection(&summary.run)?;
            let run_reason = run_drain_reason(&projection).cloned();
            let has_branch_cancellation = !projection.cancelling_branch_ids().is_empty();
            if run_reason.is_none()
                && !has_branch_cancellation
                && projection.reconciliation_cancellations().is_empty()
            {
                continue;
            }
            let mut propagation = CommandPlan::default();
            let event_limit = STRUCTURED_EVENT_SOFT_LIMIT;
            let claimed = self.claim_structured_scan_visits(projection.active_branch_ids().len());
            let mut allowance = claimed;
            let branch_ids = bounded_projection_set(
                &summary.run,
                projection.active_branch_ids(),
                &self.cancellation_branch_cursors,
                &mut allowance,
                "cancellation branch scan cursor",
            )?;
            for branch_id in branch_ids {
                if propagation.events.len() == event_limit {
                    break;
                }
                let branch = projection.branches().get(&branch_id).ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "active cancellation branch identity is absent".to_owned(),
                    )
                })?;
                if branch.state() != BranchState::Active {
                    continue;
                }
                let Some(reason) = cancellation_reason_for_branch(
                    &projection,
                    branch.branch(),
                    run_reason.as_ref(),
                ) else {
                    continue;
                };
                propagation
                    .events
                    .push(RunEventKind::BranchCancellationRequested {
                        branch: branch.branch().clone(),
                        reason,
                    });
            }
            let claimed =
                self.claim_structured_scan_visits(projection.active_subworkflow_ids().len());
            let mut allowance = claimed;
            let child_ids = bounded_projection_set(
                &summary.run,
                projection.active_subworkflow_ids(),
                &self.cancellation_subworkflow_cursors,
                &mut allowance,
                "cancellation child scan cursor",
            )?;
            for child_id in child_ids {
                if propagation.events.len() == event_limit {
                    break;
                }
                let child = projection.subworkflows().get(&child_id).ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "active cancellation child identity is absent".to_owned(),
                    )
                })?;
                let reason = cancellation_reason_for_execution(
                    &projection,
                    child.parent_execution(),
                    run_reason.as_ref(),
                );
                if child.state() == SubworkflowState::Active
                    && child.ownership() == SubworkflowOwnership::Attached
                    && let Some(reason) = reason
                {
                    propagation
                        .events
                        .push(RunEventKind::SubworkflowCancellationRequested {
                            subworkflow: child.subworkflow().clone(),
                            child_run: child.child_run().clone(),
                            reason,
                        });
                }
            }
            let claimed =
                self.claim_structured_scan_visits(projection.active_execution_ids().len());
            let mut allowance = claimed;
            let execution_ids = bounded_projection_set(
                &summary.run,
                projection.active_execution_ids(),
                &self.cancellation_execution_cursors,
                &mut allowance,
                "cancellation execution scan cursor",
            )?;
            for execution_id in execution_ids {
                if propagation.events.len() == event_limit {
                    break;
                }
                let execution =
                    projection
                        .node_executions()
                        .get(&execution_id)
                        .ok_or_else(|| {
                            RuntimeError::InvalidHistory(
                                "active cancellation execution identity is absent".to_owned(),
                            )
                        })?;
                let Some(reason) = cancellation_reason_for_execution(
                    &projection,
                    execution.execution(),
                    run_reason.as_ref(),
                ) else {
                    continue;
                };
                match execution.state() {
                    NodeExecutionState::Eligible | NodeExecutionState::RetryPending(_) => {
                        if projection.execution_has_active_child_ownership(execution.execution()) {
                            continue;
                        }
                        for timer in projection.pending_timers_for_execution(execution.execution())
                        {
                            if propagation.events.len() == event_limit {
                                break;
                            }
                            propagation.events.push(RunEventKind::TimerCancelled {
                                timer: timer.clone(),
                                reason: reason.clone(),
                            });
                        }
                        if projection
                            .waits()
                            .get(execution.execution())
                            .is_some_and(|wait| wait.is_pending())
                            && propagation.events.len() < event_limit
                        {
                            propagation.events.push(RunEventKind::WaitCancelled {
                                execution: execution.execution().clone(),
                                reason: reason.clone(),
                            });
                        }
                        // Cancelling a retry timer atomically terminalizes the
                        // reserved attempt and its execution. A first-attempt
                        // eligible execution has no such timer-owned transition.
                        if execution.state() == &NodeExecutionState::Eligible
                            && propagation.events.len() < event_limit
                        {
                            propagation.events.push(
                                RunEventKind::NodeExecutionCancelledBeforeDispatch {
                                    execution: execution.execution().clone(),
                                    reason,
                                },
                            );
                        }
                    }
                    NodeExecutionState::Scheduled(attempt)
                    | NodeExecutionState::Running(attempt) => {
                        if execution.cancellation().is_none()
                            && !projection
                                .reconciliation_cancellations()
                                .contains_key(execution.execution())
                            && propagation.events.len() < event_limit
                        {
                            propagation.events.push(
                                RunEventKind::NodeExecutionCancellationRequested {
                                    execution: execution.execution().clone(),
                                    attempt: attempt.clone(),
                                    reason,
                                },
                            );
                        }
                    }
                    NodeExecutionState::Uncertain(_)
                    | NodeExecutionState::CancelledBeforeDispatch
                    | NodeExecutionState::RemovedProspectively(_)
                    | NodeExecutionState::Terminal(_) => {}
                }
            }
            if !propagation.events.is_empty() {
                let marker = projection.attempts().keys().next().cloned();
                let _ = self.commit_internal_plan(
                    &summary.run,
                    now,
                    "propagate_structured_cancellation",
                    marker.as_ref(),
                    propagation,
                )?;
            }
            if self.structured_scan_budget.load(Ordering::Acquire) == 0 {
                continue;
            }
            let projection = self.projection(&summary.run)?;
            let claimed = self.claim_structured_scan_visits(projection.active_attempt_ids().len());
            let mut allowance = claimed;
            let attempt_ids = bounded_projection_set(
                &summary.run,
                projection.active_attempt_ids(),
                &self.cancellation_attempt_cursors,
                &mut allowance,
                "cancellation attempt scan cursor",
            )?;
            let active: Vec<_> = attempt_ids
                .iter()
                .filter_map(|attempt| projection.attempts().get(attempt))
                .filter(|attempt| attempt.is_active())
                .filter(|attempt| attempt.cancellation_acknowledgements().is_empty())
                .filter(|attempt| {
                    cancellation_reason_for_execution(
                        &projection,
                        attempt.execution(),
                        run_drain_reason(&projection),
                    )
                    .is_some()
                })
                .filter(|attempt| {
                    projection
                        .active_lease_for_attempt(attempt.attempt())
                        .is_some_and(|lease| lease.worker() == &self.config.worker)
                })
                .filter_map(|attempt| {
                    let reason = cancellation_reason_for_execution(
                        &projection,
                        attempt.execution(),
                        run_drain_reason(&projection),
                    )?;
                    attempt
                        .invocation()
                        .map(|invocation| (attempt.attempt().clone(), invocation.clone(), reason))
                })
                .collect();
            for (attempt, invocation, reason) in active {
                let projection = self.projection(&summary.run)?;
                let attempt_view = projection.attempts().get(&attempt).ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "active cancellation attempt disappeared".to_owned(),
                    )
                })?;
                let request_sequence = attempt_view
                    .cancellation_acknowledgements()
                    .last()
                    .map_or(1, |acknowledgement| {
                        acknowledgement.request_sequence().saturating_add(1)
                    });
                let request = CancellationRequest::new(
                    invocation,
                    request_sequence,
                    reason.as_str().to_owned(),
                )
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
                let acknowledgement = match self.executor.cancel(&request) {
                    Ok(acknowledgement) => acknowledgement,
                    Err(error) => {
                        warn!(
                            run = %summary.run,
                            attempt = %attempt,
                            reason = %error,
                            "executor cancellation boundary failed; lease remains recoverable"
                        );
                        continue;
                    }
                };
                let command = RunCommandDocument::new(
                    self.next_command_id()?,
                    summary.run.clone(),
                    self.config.internal_actor.clone(),
                    self.store.head(&summary.run)?,
                    now,
                    Reason::new("executor acknowledged durable cancellation intent")?,
                    Vec::new(),
                    RunCommand::WorkerReport {
                        worker: self.config.worker.clone(),
                        report: WorkerReport::Cancellation {
                            attempt,
                            acknowledgement,
                        },
                    },
                )?;
                let _ = self.handle_command(&command)?;
            }
        }
        Ok(())
    }

    pub(super) fn admission_usage(
        &self,
    ) -> Result<(AdmissionUsage, IntegrityDigest), RuntimeError> {
        let mut usage = AdmissionUsage::default();
        let global_limit = self.config.scheduler_limits.global();
        let snapshot = self.store.active_leases(PageSize::new(global_limit)?)?;
        if snapshot.entries.len()
            == usize::try_from(global_limit).map_err(|_error| {
                RuntimeError::Scheduling("global admission limit does not fit usize".to_owned())
            })?
        {
            // The queried bound is the hard global limit. Reaching it is sufficient
            // to decline every new dispatch without projecting unrelated aggregates.
            usage.global = global_limit;
            return Ok((usage, snapshot.witness));
        }

        let mut projections = BTreeMap::new();
        for indexed in &snapshot.entries {
            if !projections.contains_key(&indexed.run) {
                projections.insert(indexed.run.clone(), self.projection(&indexed.run)?);
            }
        }
        for indexed in snapshot.entries {
            let projection = projections.get(&indexed.run).ok_or_else(|| {
                RuntimeError::InvalidHistory("active lease run projection is absent".to_owned())
            })?;
            let lease = projection.leases().get(&indexed.lease).ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "active lease index references an absent lease".to_owned(),
                )
            })?;
            if !lease.is_active()
                || lease.attempt() != &indexed.attempt
                || lease.worker() != &indexed.worker
                || lease.expires_at() != indexed.expires_at
                || projection.sequence() < indexed.through_sequence
            {
                return Err(RuntimeError::InvalidHistory(
                    "active lease index disagrees with authoritative run history".to_owned(),
                ));
            }
            let attempt = projection.attempts().get(lease.attempt()).ok_or_else(|| {
                RuntimeError::InvalidHistory("active lease has no attempt".to_owned())
            })?;
            let capability = attempt.capability().ok_or_else(|| {
                RuntimeError::InvalidHistory("active lease has no capability resolution".to_owned())
            })?;
            usage.global = usage.global.checked_add(1).ok_or_else(|| {
                RuntimeError::Scheduling("global admission count overflow".to_owned())
            })?;
            checked_increment(&mut usage.runs, indexed.run.clone())?;
            checked_increment(
                &mut usage.capability_classes,
                capability.snapshot().operation().clone(),
            )?;
            let execution = projection
                .node_executions()
                .get(attempt.execution())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory("attempt execution is absent".to_owned())
                })?;
            if let Some(ScopeKind::Branch { branch }) = projection
                .scopes()
                .get(execution.scope())
                .map(WorkspaceScope::kind)
            {
                checked_increment(&mut usage.branches, (indexed.run.clone(), branch.clone()))?;
            }
        }
        Ok((usage, snapshot.witness))
    }
}
