//! Restart recovery, graceful cancellation propagation, and admission accounting.

use super::support::{
    CommandPlan, bounded_projection_set, bounded_projection_sweep_set,
    cancellation_reason_for_branch, cancellation_reason_for_execution, checked_increment,
    checked_timestamp_add, recovery_classification, recovery_reason, run_drain_reason,
    unresolved_retry_error_class,
};
use super::{RecoveryResult, RuntimeService, STRUCTURED_EVENT_SOFT_LIMIT};
use crate::projection::{
    AttemptState, BranchState, NodeExecutionState, RunProjection, SubworkflowState,
};
use crate::{AdmissionUsage, RuntimeError, SystemTransition};
use milkdrift_capability::{ErrorClass, InvocationValueReference, SideEffectClass};
use milkdrift_persistence::{
    IndexedRunState, IntegrityDigest, PageSize, PersistenceError, Reason, RecoveryClassification,
    RunEventKind, RunSummaryIndex, StorageFailureClass, SubworkflowOwnership, TimestampMillis,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactReference, RunId, ScopeKind, WorkspaceScope, WorkspaceUsage,
    WorkspaceValueReference,
};
use std::collections::BTreeMap;
use std::sync::TryLockError;
use std::sync::atomic::Ordering;
use tracing::{info_span, warn};

impl RuntimeService {
    /// Replays and classifies a bounded page of nonterminal runs. An expired lease
    /// whose executor start was never observed is safely reassigned to the same
    /// immutable attempt. Started work becomes a truthful uncertainty obligation;
    /// only work whose frozen side-effect and idempotency facts permit exact replay
    /// receives a bounded retry timer.
    #[expect(
        clippy::too_many_lines,
        reason = "recovery is one ordered replay-and-repair pass with shared progress invariants"
    )]
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
        let cursor_before = self
            .recovery_cursor
            .lock()
            .map_err(|_error| {
                RuntimeError::Scheduling(
                    "runtime recovery pagination cursor lock is poisoned".to_owned(),
                )
            })?
            .clone();
        let attempt_cursors_before = self
            .recovery_attempt_cursors
            .lock()
            .map_err(|_error| {
                RuntimeError::Scheduling(
                    "runtime recovery attempt cursor lock is poisoned".to_owned(),
                )
            })?
            .clone();

        let outcome = (|| -> Result<RecoveryResult, RuntimeError> {
            let limit = PageSize::new(u32::from(self.config.maximum_tick_items))?;
            let mut result = RecoveryResult::default();
            let mut remaining = usize::from(self.config.maximum_tick_items);
            let summaries = self.next_nonterminal_page(&self.recovery_cursor, limit, "recovery")?;
            let mut validated = Vec::with_capacity(summaries.len());
            for summary in summaries {
                let projection = self.active_recovery_component(
                    &summary.run,
                    "authoritative history",
                    self.projection(&summary.run),
                )?;
                self.validate_active_recovery_state(&summary, &projection)?;
                validated.push((summary, projection));
            }

            // Validate every active aggregate in the fetched page before recovery appends
            // anything. Corruption in one active run therefore cannot be mistaken for an
            // empty input or allow repairs to be appended after the contradiction is known.
            let summary_count = validated.len();
            for (summary_index, (summary, projection)) in validated.into_iter().enumerate() {
                if remaining == 0 {
                    return Err(RuntimeError::Scheduling(
                        "recovery page exhausted its visit budget before every returned run was examined"
                            .to_owned(),
                    ));
                }
                result.runs_examined = result.runs_examined.saturating_add(1);
                // Reserve one attempt visit for every later run in the already-advanced
                // persistence page. Without this reservation, one run can consume the
                // whole attempt budget while the page cursor skips unvisited summaries.
                let later_runs = summary_count.saturating_sub(summary_index.saturating_add(1));
                let reserved_for_later = later_runs.min(remaining.saturating_sub(1));
                let scan_limit = remaining.saturating_sub(reserved_for_later);
                let mut scan_remaining = scan_limit;
                let scanned = bounded_projection_sweep_set(
                    &summary.run,
                    projection.active_attempt_ids(),
                    &self.recovery_attempt_cursors,
                    &mut scan_remaining,
                    "recovery attempt sweep cursor",
                )?;
                remaining = remaining.saturating_sub(scan_limit.saturating_sub(scan_remaining));
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
                        } else if attempt.state() == &AttemptState::Leased {
                            RecoveryClassification::NotStarted
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
                        RecoveryClassification::NotStarted => {
                            if let Some(previous_lease) = active_lease {
                                // No executor start crossed the current lease boundary. Preserve the
                                // immutable attempt/invocation and rotate only durable ownership.
                                if plan.expected_lease_revision.is_none() {
                                    let (_usage, revision) = self.admission_usage()?;
                                    plan.expected_lease_revision = Some(revision);
                                }
                                plan.events.push(RunEventKind::NodeReLeased {
                                    previous_lease: previous_lease.lease().clone(),
                                    lease: self.next_lease_id()?,
                                    attempt: attempt.attempt().clone(),
                                    worker: self.config.worker.clone(),
                                    expires_at: checked_timestamp_add(
                                        now,
                                        self.config.lease_duration_ms,
                                    )?,
                                });
                                result.retryable = result.retryable.saturating_add(1);
                            }
                        }
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
                        RecoveryClassification::LeaseStillValid
                        | RecoveryClassification::TerminalObserved
                        | RecoveryClassification::Uncertain => {}
                    }
                }
                let _ = self.commit_internal_plan(
                    &summary.run,
                    now,
                    SystemTransition::RecoverNonterminalRun,
                    plan,
                )?;
            }
            Ok(result)
        })();

        if outcome.is_err() {
            *self.recovery_cursor.lock().map_err(|_error| {
                RuntimeError::Scheduling(
                    "runtime recovery pagination cursor lock is poisoned".to_owned(),
                )
            })? = cursor_before;
            *self.recovery_attempt_cursors.lock().map_err(|_error| {
                RuntimeError::Scheduling(
                    "runtime recovery attempt cursor lock is poisoned".to_owned(),
                )
            })? = attempt_cursors_before;
        }
        outcome
    }

    fn validate_active_recovery_state(
        &self,
        summary: &RunSummaryIndex,
        projection: &RunProjection,
    ) -> Result<(), RuntimeError> {
        let run = &summary.run;
        if projection.run_id() != Some(run) {
            return Err(Self::active_recovery_invalid(
                run,
                "run identity",
                "projected aggregate identity does not match the nonterminal index",
            ));
        }
        if projection.lifecycle().is_completed() || summary.state == IndexedRunState::Terminal {
            return Err(Self::active_recovery_invalid(
                run,
                "run lifecycle",
                "nonterminal discovery points at terminal state",
            ));
        }
        if summary.through_sequence != projection.sequence() {
            return Err(Self::active_recovery_invalid(
                run,
                "run summary",
                format!(
                    "summary sequence {} contradicts projected sequence {}",
                    summary.through_sequence,
                    projection.sequence()
                ),
            ));
        }
        if projection.workflow() != Some(&summary.workflow) {
            return Err(Self::active_recovery_invalid(
                run,
                "run summary",
                "summary workflow contradicts authoritative history",
            ));
        }
        if projection.revision() != Some(&summary.revision) {
            return Err(Self::active_recovery_invalid(
                run,
                "run summary",
                "summary revision contradicts authoritative history",
            ));
        }

        let head = self.active_recovery_component(
            run,
            "journal head",
            self.store.head(run).map_err(RuntimeError::from),
        )?;
        if head != projection.sequence() {
            return Err(Self::active_recovery_invalid(
                run,
                "journal head",
                format!(
                    "authoritative head {head} contradicts projected sequence {}",
                    projection.sequence()
                ),
            ));
        }

        let (runnable, timers, leases) = self.active_recovery_component(
            run,
            "derived discovery indexes",
            self.discovery_expectations(run, projection),
        )?;
        self.active_recovery_component(
            run,
            "derived discovery indexes",
            self.store
                .validate_run_discovery(run, projection.sequence(), &runnable, &timers, &leases)
                .map_err(RuntimeError::from),
        )?;

        let revision = self.active_recovery_component(
            run,
            "pinned revision",
            self.current_revision(projection),
        )?;
        if projection.revision_digest() != Some(revision.content_digest()) {
            return Err(Self::active_recovery_invalid(
                run,
                "pinned revision",
                "revision content digest contradicts the active projection",
            ));
        }

        let budget = projection.workspace_budget().ok_or_else(|| {
            Self::active_recovery_invalid(
                run,
                "workspace budget",
                "active history has no pinned workspace budget",
            )
        })?;
        let durable_usage = self.active_recovery_component(
            run,
            "workspace accounting",
            self.store.workspace_usage(run).map_err(RuntimeError::from),
        )?;
        self.active_recovery_component(
            run,
            "workspace accounting",
            budget
                .validate_usage(&durable_usage)
                .map_err(|error| RuntimeError::InvalidHistory(error.to_string())),
        )?;

        let root_scope = projection.root_scope().ok_or_else(|| {
            Self::active_recovery_invalid(
                run,
                "root workspace scope",
                "active history has no root scope",
            )
        })?;
        self.active_recovery_component(
            run,
            "root workspace scope",
            self.validate_projected_scope(projection, root_scope.reference(), &[]),
        )?;
        for reference in projection.scopes().keys() {
            if reference == root_scope.reference() {
                continue;
            }
            self.active_recovery_component(
                run,
                "workspace scope",
                self.validate_projected_scope(projection, reference, &[]),
            )?;
        }

        let mut projected_value_usage = WorkspaceUsage::EMPTY;
        for reference in projection.inputs() {
            let entry = self.active_recovery_component(
                run,
                "supplied run input",
                self.projected_workspace_value(projection, reference, &[]),
            )?;
            self.validate_active_workspace_artifact(run, entry.value().as_artifact())?;
        }
        for reference in projection.workspace_values() {
            let entry = self.active_recovery_component(
                run,
                "workspace value",
                self.projected_workspace_value(projection, reference, &[]),
            )?;
            projected_value_usage = self.active_recovery_component(
                run,
                "workspace accounting",
                budget
                    .admit_value(&projected_value_usage, entry.value())
                    .map_err(|error| RuntimeError::InvalidHistory(error.to_string())),
            )?;
            self.validate_active_workspace_artifact(run, entry.value().as_artifact())?;
        }
        if durable_usage.value_versions() != projected_value_usage.value_versions()
            || durable_usage.inline_bytes() != projected_value_usage.inline_bytes()
        {
            return Err(Self::active_recovery_invalid(
                run,
                "workspace accounting",
                format!(
                    "persisted value usage ({}, {} bytes) contradicts projected value usage ({}, {} bytes)",
                    durable_usage.value_versions(),
                    durable_usage.inline_bytes(),
                    projected_value_usage.value_versions(),
                    projected_value_usage.inline_bytes()
                ),
            ));
        }
        if durable_usage.artifacts() < projection.resource_usage().artifacts()
            || durable_usage.artifact_bytes() < projection.resource_usage().artifact_bytes()
        {
            return Err(Self::active_recovery_invalid(
                run,
                "workspace accounting",
                "persisted artifact usage is lower than authoritative published-artifact facts",
            ));
        }

        for (artifact, expected) in projection.artifacts() {
            let durable = self.active_recovery_component(
                run,
                "artifact metadata",
                self.store.metadata(artifact).map_err(RuntimeError::from),
            )?;
            if durable.as_ref() != Some(expected) {
                return Err(Self::active_recovery_invalid(
                    run,
                    "artifact metadata",
                    format!("artifact {artifact} is absent or contradicts active history"),
                ));
            }
        }

        for attempt in projection.attempts().values() {
            let Some(request) = attempt.request() else {
                continue;
            };
            for input in request.inputs() {
                match input.value() {
                    InvocationValueReference::WorkspaceValue { identity, version } => {
                        let reference: WorkspaceValueReference = serde_json::from_str(identity)
                            .map_err(|error| {
                                Self::active_recovery_invalid(
                                    run,
                                    "frozen invocation input",
                                    format!(
                                        "invocation {} contains an invalid workspace identity: {error}",
                                        request.invocation()
                                    ),
                                )
                            })?;
                        if version != &reference.version().get().to_string() {
                            return Err(Self::active_recovery_invalid(
                                run,
                                "frozen invocation input",
                                format!(
                                    "invocation {} workspace version contradicts its encoded identity",
                                    request.invocation()
                                ),
                            ));
                        }
                        self.active_recovery_component(
                            run,
                            "frozen invocation input",
                            self.projected_workspace_value(projection, &reference, &[]),
                        )?;
                    }
                    InvocationValueReference::Artifact { reference } => {
                        self.validate_active_invocation_artifact(run, reference)?;
                    }
                    InvocationValueReference::Inline { .. } => {}
                }
            }
        }
        Ok(())
    }

    fn validate_active_workspace_artifact(
        &self,
        run: &RunId,
        reference: Option<&ArtifactReference>,
    ) -> Result<(), RuntimeError> {
        let Some(reference) = reference else {
            return Ok(());
        };
        let durable = self.active_recovery_component(
            run,
            "workspace artifact metadata",
            self.store
                .metadata(reference.artifact())
                .map_err(RuntimeError::from),
        )?;
        if durable
            .as_ref()
            .is_none_or(|metadata| metadata.reference() != reference)
        {
            return Err(Self::active_recovery_invalid(
                run,
                "workspace artifact metadata",
                format!(
                    "artifact {} is absent or contradicts the workspace reference",
                    reference.artifact()
                ),
            ));
        }
        Ok(())
    }

    fn validate_active_invocation_artifact(
        &self,
        run: &RunId,
        reference: &milkdrift_capability::ArtifactReference,
    ) -> Result<(), RuntimeError> {
        let artifact = ArtifactId::new(reference.identity()).map_err(|error| {
            Self::active_recovery_invalid(run, "frozen invocation artifact", error.to_string())
        })?;
        let durable = self.active_recovery_component(
            run,
            "frozen invocation artifact",
            self.store.metadata(&artifact).map_err(RuntimeError::from),
        )?;
        let Some(metadata) = durable else {
            return Err(Self::active_recovery_invalid(
                run,
                "frozen invocation artifact",
                format!("artifact {artifact} is absent"),
            ));
        };
        let expected = metadata.reference();
        let expected_digest = expected.digest().to_hex();
        if reference.digest() != expected_digest.as_str()
            || reference.media_type() != Some(expected.media_type().as_str())
            || reference.size_bytes() != Some(expected.size_bytes())
        {
            return Err(Self::active_recovery_invalid(
                run,
                "frozen invocation artifact",
                format!("artifact {artifact} metadata contradicts the frozen request"),
            ));
        }
        Ok(())
    }

    fn active_recovery_component<T>(
        &self,
        run: &RunId,
        component: &'static str,
        result: Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        result.map_err(|error| Self::classify_active_recovery_error(run, component, error))
    }

    fn classify_active_recovery_error(
        run: &RunId,
        component: &'static str,
        error: RuntimeError,
    ) -> RuntimeError {
        let retryable_boundary = matches!(
            &error,
            RuntimeError::Persistence(PersistenceError::Storage {
                class: StorageFailureClass::Unavailable
                    | StorageFailureClass::OwnerBusy
                    | StorageFailureClass::ResourceExhausted,
                ..
            })
        );
        if retryable_boundary {
            error
        } else {
            Self::active_recovery_invalid(run, component, error.to_string())
        }
    }

    fn active_recovery_invalid(
        run: &RunId,
        component: &'static str,
        detail: impl std::fmt::Display,
    ) -> RuntimeError {
        RuntimeError::InvalidHistory(format!(
            "active recovery for run {run} found corrupt {component}: {detail}"
        ))
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
                let _ = self.commit_internal_plan(
                    &summary.run,
                    now,
                    SystemTransition::PropagateStructuredCancellation,
                    propagation,
                )?;
            }
            // Adapter cancellation is an external effect. Durable cancellation intent above
            // is claimed later by `claim_effects`; recovery never enters an adapter boundary.
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
            return Ok((usage, snapshot.revision));
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
        Ok((usage, snapshot.revision))
    }
}
