//! Durable effect claiming and incremental observation ingestion.
//!
//! Scheduler calls only create immutable requests and leases. External work is claimed
//! explicitly and may be executed on any caller-owned bounded thread/task. The runtime
//! starts no hidden worker, and every successful reporter call is already durable.

mod entry;
mod reporter;

use reporter::{stable_effect_command_id, terminal_report_identity};

use milkdrift_authority::{BoundaryTimeMillis, DecisionId};
use milkdrift_capability::{CancellationAcknowledgement, CancellationRequest, ErrorClass};
use milkdrift_persistence::{
    AttemptId, CommandDisposition, CommandId, LeaseId, PageSize, PersistenceError, Reason,
    RunEventKind, TimestampMillis,
};
use milkdrift_workspace::RunId;
use tracing::warn;

use super::support::{CommandPlan, cancellation_reason_for_execution, run_drain_reason};
use super::{CommandExecution, RuntimeService};
use crate::projection::{AttemptState, NodeExecutionState};
use crate::{
    CancellationDispatch, EffectAction, ExecutionDispatch, ExecutorError, ObservationDisposition,
    RunCommand, RunCommandDocument, RuntimeError, SystemTransition, WorkerReport,
};

const MAX_OBSERVATION_COMMIT_RETRIES: usize = 16;

/// Result of executing one claimed effect on the caller's thread/task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectExecutionResult {
    /// A terminal invocation observation is durable.
    Completed {
        /// Invocation/heartbeat observations accepted or replayed.
        observations: u32,
    },
    /// The external boundary returned without a provable terminal outcome.
    Uncertain {
        /// Observations made durable before uncertainty was recorded.
        observations: u32,
    },
    /// A cancellation acknowledgement is durable.
    CancellationAcknowledged,
    /// The cancellation boundary failed without changing durable invocation truth.
    CancellationDeferred,
}

impl RuntimeService {
    /// Claims at most `maximum` already-durable external effects.
    ///
    /// Claiming an invocation atomically records `NodeStarted` before returning the
    /// dispatch. Only the call that first commits that transition receives an action;
    /// idempotent command replay never authorizes duplicate execution.
    pub fn claim_effects(&self, maximum: PageSize) -> Result<Vec<EffectAction>, RuntimeError> {
        self.claim_effects_filtered(maximum, true, true)
    }

    /// Claims only invocation-entry effects for a bounded execution-worker queue.
    ///
    /// This split lets a caller reserve independent cancellation capacity even while
    /// every process-execution worker is occupied.
    pub fn claim_execution_effects(
        &self,
        maximum: PageSize,
    ) -> Result<Vec<EffectAction>, RuntimeError> {
        self.claim_effects_filtered(maximum, true, false)
    }

    /// Claims only cancellation effects for a bounded control-worker queue.
    pub fn claim_cancellation_effects(
        &self,
        maximum: PageSize,
    ) -> Result<Vec<EffectAction>, RuntimeError> {
        self.claim_effects_filtered(maximum, false, true)
    }

    fn claim_effects_filtered(
        &self,
        maximum: PageSize,
        executions: bool,
        cancellations: bool,
    ) -> Result<Vec<EffectAction>, RuntimeError> {
        let _claim_guard = self.effect_claim_gate.lock().map_err(|_error| {
            RuntimeError::Scheduling("effect claim coordination lock is poisoned".to_owned())
        })?;
        // The scheduler enforces that the complete active set fits in the configured
        // global bound. Read that complete set before filtering by worker, otherwise a
        // small claim page could be permanently filled by another worker's leases.
        let snapshot = self
            .store
            .active_leases(PageSize::new(self.config.scheduler_limits.global())?)?;
        let maximum = usize::try_from(maximum.get()).map_err(|_error| {
            RuntimeError::Scheduling("effect claim page size does not fit usize".to_owned())
        })?;
        let now = self.clock.now()?;
        let mut actions = Vec::with_capacity(maximum);
        for indexed in snapshot.entries {
            if actions.len() == maximum || indexed.worker != self.config.worker {
                continue;
            }
            let projection = self.projection(&indexed.run)?;
            let lease = projection.leases().get(&indexed.lease).ok_or_else(|| {
                RuntimeError::InvalidHistory("active effect index names an absent lease".to_owned())
            })?;
            if !lease.is_active()
                || lease.attempt() != &indexed.attempt
                || lease.worker() != &indexed.worker
                || lease.expires_at() != indexed.expires_at
                || projection.sequence() < indexed.through_sequence
            {
                return Err(RuntimeError::InvalidHistory(
                    "active effect index disagrees with authoritative lease state".to_owned(),
                ));
            }
            if lease.expires_at() <= now {
                continue;
            }
            let attempt = projection.attempts().get(&indexed.attempt).ok_or_else(|| {
                RuntimeError::InvalidHistory("active lease names an absent attempt".to_owned())
            })?;
            match attempt.state() {
                AttemptState::Leased if executions => {
                    if let Some(dispatch) =
                        self.claim_invocation(&indexed.run, &indexed.lease, &indexed.attempt, now)?
                    {
                        actions.push(EffectAction::Execute(Box::new(dispatch)));
                    }
                }
                AttemptState::Running if cancellations => {
                    if let Some(cancellation) =
                        self.cancellation_dispatch(&indexed.run, &projection, &indexed.attempt)?
                    {
                        actions.push(EffectAction::Cancel(cancellation));
                    }
                }
                AttemptState::Leased
                | AttemptState::Running
                | AttemptState::AwaitingRetryTimer
                | AttemptState::ReadyToSchedule
                | AttemptState::Scheduled
                | AttemptState::Terminal(_)
                | AttemptState::Uncertain
                | AttemptState::UncertainSupersededByRetry { .. }
                | AttemptState::UncertainAbandonedByCancellation { .. }
                | AttemptState::Retained
                | AttemptState::Resolved(_)
                | AttemptState::CancelledBeforeDispatch => {}
            }
        }
        Ok(actions)
    }

    /// Executes one previously claimed effect on the caller's thread/task.
    ///
    /// This may block for the complete external invocation and therefore must not be
    /// called on the scheduler/recovery call stack.
    pub fn execute_effect(
        &self,
        action: EffectAction,
    ) -> Result<EffectExecutionResult, RuntimeError> {
        match action {
            EffectAction::Execute(dispatch) => self.execute_invocation_effect(&dispatch),
            EffectAction::Cancel(dispatch) => self.execute_cancellation_effect(&dispatch),
        }
    }

    fn claim_invocation(
        &self,
        run: &RunId,
        lease: &LeaseId,
        attempt: &AttemptId,
        now: TimestampMillis,
    ) -> Result<Option<ExecutionDispatch>, RuntimeError> {
        let projection = self.projection(run)?;
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidHistory("claimed attempt is absent".to_owned()))?;
        if attempt_view.state() != &AttemptState::Leased {
            return Ok(None);
        }
        let lease_view = projection
            .leases()
            .get(lease)
            .ok_or_else(|| RuntimeError::InvalidHistory("claimed lease is absent".to_owned()))?;
        if !lease_view.is_active()
            || lease_view.attempt() != attempt
            || lease_view.worker() != &self.config.worker
            || lease_view.expires_at() <= now
        {
            return Ok(None);
        }
        let execution = projection
            .node_executions()
            .get(attempt_view.execution())
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("claimed attempt execution is absent".to_owned())
            })?;
        if execution.cancellation().is_some()
            || !matches!(
                execution.state(),
                NodeExecutionState::Scheduled(active) if active == attempt
            )
            || cancellation_reason_for_execution(
                &projection,
                execution.execution(),
                run_drain_reason(&projection),
            )
            .is_some()
        {
            return Ok(None);
        }
        let revision = projection.revision_for_attempt(attempt).ok_or_else(|| {
            RuntimeError::InvalidHistory("claimed attempt has no governing revision".to_owned())
        })?;
        let request = attempt_view.request().ok_or_else(|| {
            RuntimeError::InvalidHistory("claimed attempt has no immutable request".to_owned())
        })?;
        let capability = attempt_view.capability().ok_or_else(|| {
            RuntimeError::InvalidHistory("claimed attempt has no capability snapshot".to_owned())
        })?;
        let resolution_authorization = capability.authorization().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "claimed attempt has no durable resolution authorization".to_owned(),
            )
        })?;
        let basis = projection.execution_authority().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "leased attempt has no frozen execution-authority basis".to_owned(),
            )
        })?;
        let mut entry_request = resolution_authorization.request().clone();
        let entry_identity = format!("{}:entry", resolution_authorization.digest());
        entry_request.decision = DecisionId::new(format!(
            "decision:{}",
            blake3::hash(entry_identity.as_bytes())
        ))?;
        entry_request.evaluated_at = BoundaryTimeMillis::new(now.get());
        let entry_authorization = self.authority.evaluate(&entry_request)?;
        if !entry_authorization.is_allowed() {
            let detail = milkdrift_persistence::BoundedDetail::new(format!(
                "authority decision {} denied capability entry",
                entry_authorization.digest(),
            ))?;
            let plan = CommandPlan {
                events: vec![
                    RunEventKind::CapabilityEntryDecisionRecorded {
                        attempt: attempt.clone(),
                        authorization: entry_authorization,
                    },
                    RunEventKind::NodeTerminal {
                        execution: execution.execution().clone(),
                        attempt: attempt.clone(),
                        report_sequence: 1,
                        outcome: milkdrift_persistence::NodeOutcome::Rejected,
                        error_class: Some(ErrorClass::Authorization),
                        detail: Some(detail),
                    },
                ],
                ..CommandPlan::default()
            };
            let _ = self.commit_internal_plan(
                run,
                now,
                SystemTransition::DenyCapabilityEntry {
                    attempt: attempt.clone(),
                },
                plan,
            )?;
            return Ok(None);
        }
        let report = WorkerReport::LeaseAccepted {
            lease: lease.clone(),
            attempt: attempt.clone(),
            authorization: Box::new(entry_authorization.clone()),
        };
        let (command_execution, _rejection) = self.commit_worker_report(
            run,
            stable_effect_command_id(run, attempt, &report)?,
            now,
            Reason::new("effect host accepted a durable dispatch lease")?,
            report,
        )?;
        if command_execution.result().disposition() != CommandDisposition::Accepted
            || command_execution.replayed()
        {
            return Ok(None);
        }
        Ok(Some(ExecutionDispatch::from_snapshot(
            run.clone(),
            revision.clone(),
            execution.node().clone(),
            execution.execution().clone(),
            attempt.clone(),
            lease.clone(),
            lease_view.expires_at(),
            capability.snapshot().clone(),
            basis.clone(),
            resolution_authorization.clone(),
            entry_authorization,
            request.clone(),
        )?))
    }

    fn cancellation_dispatch(
        &self,
        run: &RunId,
        projection: &crate::RunProjection,
        attempt: &AttemptId,
    ) -> Result<Option<CancellationDispatch>, RuntimeError> {
        let attempt_view = projection.attempts().get(attempt).ok_or_else(|| {
            RuntimeError::InvalidHistory("cancellation attempt is absent".to_owned())
        })?;
        let execution = projection
            .node_executions()
            .get(attempt_view.execution())
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("cancellation execution is absent".to_owned())
            })?;
        if !matches!(execution.state(), NodeExecutionState::Running(active) if active == attempt) {
            return Ok(None);
        }
        let Some(reason) = cancellation_reason_for_execution(
            projection,
            execution.execution(),
            run_drain_reason(projection),
        ) else {
            return Ok(None);
        };
        if attempt_view
            .cancellation_acknowledgements()
            .last()
            .is_some_and(CancellationAcknowledgement::accepted)
        {
            return Ok(None);
        }
        let invocation = attempt_view.invocation().ok_or_else(|| {
            RuntimeError::InvalidHistory("cancellation attempt has no invocation".to_owned())
        })?;
        let request_sequence = attempt_view
            .cancellation_acknowledgements()
            .last()
            .map_or(1, |acknowledgement| {
                acknowledgement.request_sequence().saturating_add(1)
            });
        let request = CancellationRequest::new(
            invocation.clone(),
            request_sequence,
            reason.as_str().to_owned(),
        )
        .map_err(ExecutorError::from)?;
        Ok(Some(CancellationDispatch::new(
            run.clone(),
            attempt.clone(),
            request,
        )))
    }

    fn execute_cancellation_effect(
        &self,
        dispatch: &CancellationDispatch,
    ) -> Result<EffectExecutionResult, RuntimeError> {
        let acknowledgement = match self.executor.cancel(dispatch.request()) {
            Ok(acknowledgement) => acknowledgement,
            Err(error) => {
                warn!(
                    run = %dispatch.run(),
                    attempt = %dispatch.attempt(),
                    reason = %error,
                    "cancellation boundary failed; durable request remains eligible for redelivery"
                );
                return Ok(EffectExecutionResult::CancellationDeferred);
            }
        };
        if acknowledgement.invocation() != dispatch.request().invocation()
            || acknowledgement.request_sequence() != dispatch.request().request_sequence()
        {
            return Err(RuntimeError::Executor(ExecutorError::InvalidReports(
                "cancellation acknowledgement does not match its durable request".to_owned(),
            )));
        }
        let report = WorkerReport::Cancellation {
            attempt: dispatch.attempt().clone(),
            acknowledgement,
        };
        let (execution, rejection) = self.commit_worker_report(
            dispatch.run(),
            stable_effect_command_id(dispatch.run(), dispatch.attempt(), &report)?,
            self.clock.now()?,
            Reason::new("executor acknowledged durable cancellation intent")?,
            report,
        )?;
        if let Some(error) = rejection {
            return Err(error);
        }
        if execution.result().disposition() != CommandDisposition::Accepted {
            return Err(RuntimeError::Executor(ExecutorError::InvalidReports(
                "cancellation acknowledgement was durably rejected".to_owned(),
            )));
        }
        Ok(EffectExecutionResult::CancellationAcknowledged)
    }

    fn submit_effect_observation(
        &self,
        run: &RunId,
        command: CommandId,
        observed_at: TimestampMillis,
        reason: Reason,
        report: WorkerReport,
    ) -> Result<ObservationDisposition, RuntimeError> {
        let terminal_identity = terminal_report_identity(&report);
        let (execution, rejection) =
            self.commit_worker_report(run, command, observed_at, reason, report)?;
        if let Some(error) = rejection {
            return Err(error);
        }
        if execution.result().disposition() != CommandDisposition::Accepted {
            return Err(RuntimeError::Executor(ExecutorError::InvalidReports(
                "executor observation was durably rejected".to_owned(),
            )));
        }
        if execution.replayed() {
            return Ok(ObservationDisposition::Replayed);
        }
        if let Some((attempt, report_sequence)) = terminal_identity {
            let projection = self.projection(run)?;
            if projection
                .attempts()
                .get(&attempt)
                .and_then(crate::projection::NodeAttemptProjection::late_terminal_evidence)
                .is_some_and(|evidence| evidence.report_sequence() == report_sequence)
            {
                return Ok(ObservationDisposition::LateEvidence);
            }
        }
        Ok(ObservationDisposition::Applied)
    }

    fn commit_worker_report(
        &self,
        run: &RunId,
        command: CommandId,
        observed_at: TimestampMillis,
        reason: Reason,
        report: WorkerReport,
    ) -> Result<(CommandExecution, Option<RuntimeError>), RuntimeError> {
        for _ in 0..MAX_OBSERVATION_COMMIT_RETRIES {
            let projection = self.projection(run)?;
            let document = RunCommandDocument::new(
                command.clone(),
                run.clone(),
                self.config.internal_actor.clone(),
                projection.sequence(),
                observed_at,
                reason.clone(),
                Vec::new(),
                RunCommand::WorkerReport {
                    worker: self.config.worker.clone(),
                    report: report.clone(),
                },
            )?;
            match self.handle_internal_command_preserving_rejection(&document) {
                Ok(outcome) => return Ok(outcome),
                Err(RuntimeError::Persistence(
                    PersistenceError::SequenceConflict { .. }
                    | PersistenceError::ControllerAccountRevisionConflict { .. },
                )) => {}
                Err(error) => return Err(error),
            }
        }
        Err(RuntimeError::Scheduling(
            "executor observation could not obtain a stable aggregate head".to_owned(),
        ))
    }

    fn record_effect_uncertainty(
        &self,
        run: &RunId,
        attempt: &AttemptId,
        detail: &str,
    ) -> Result<(), RuntimeError> {
        let projection = self.projection(run)?;
        let Some(attempt_view) = projection.attempts().get(attempt) else {
            // A concurrent terminal cancellation can compact the attempt before the adapter's
            // owned execution call returns. That accepted terminal fact is authoritative.
            return Ok(());
        };
        if !matches!(
            attempt_view.state(),
            AttemptState::Leased | AttemptState::Running
        ) {
            return Ok(());
        }
        let side_effect = attempt_view.side_effect().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "uncertain effect has no side-effect classification".to_owned(),
            )
        })?;
        let report_sequence = attempt_view
            .last_report_sequence()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| {
                RuntimeError::InvalidTransition("report sequence overflow".to_owned())
            })?;
        let now = self.clock.now()?;
        let mut plan = CommandPlan::one(RunEventKind::ExternalOutcomeUncertain {
            attempt: attempt.clone(),
            report_sequence,
            side_effect: side_effect.side_effect(),
            reason: bounded_uncertainty_reason(detail)?,
            evidence: Vec::new(),
        });
        if self.config.retry_policy.permits_automatic_retry(
            attempt_view.attempt_number(),
            ErrorClass::Adapter,
            true,
            side_effect.side_effect(),
            side_effect.idempotency(),
            attempt_view.idempotency_key(),
        ) {
            match self.build_retry_event(
                attempt_view.execution(),
                attempt,
                attempt_view.attempt_number(),
                now,
                ErrorClass::Adapter,
                None,
                "automatic retry admitted after a lost external effect boundary",
            ) {
                Ok(retry) => plan.events.push(retry),
                Err(error) => warn!(
                    run = %run,
                    attempt = %attempt,
                    reason = %error,
                    "effect uncertainty retained without an available retry timer"
                ),
            }
        }
        let _ = self.commit_internal_plan(
            run,
            now,
            SystemTransition::DispatchOutcomeUncertain {
                attempt: attempt.clone(),
            },
            plan,
        )?;
        Ok(())
    }
}

fn bounded_uncertainty_reason(detail: &str) -> Result<Reason, PersistenceError> {
    let mut value = detail
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    if value.is_empty() {
        value.push_str("external effect boundary returned without terminal evidence");
    }
    let boundary = milkdrift_contracts::truncate_utf8(&value, 2_000).len();
    value.truncate(boundary);
    Reason::new(value)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::entry::retry_final_entry;
    use super::reporter::adapter_entry_decision_is_new;
    use crate::{BoundaryClock, RuntimeError};
    use milkdrift_persistence::{
        CommandDisposition, PersistenceError, RunSequence, TimestampMillis,
    };
    use milkdrift_workspace::RunId;

    struct AdvancingClock(AtomicU64);

    impl BoundaryClock for AdvancingClock {
        fn now(&self) -> Result<TimestampMillis, RuntimeError> {
            Ok(TimestampMillis::new(self.0.fetch_add(1, Ordering::SeqCst)))
        }
    }

    #[test]
    fn adapter_entry_requires_both_acceptance_and_a_new_commit() {
        assert!(adapter_entry_decision_is_new(
            CommandDisposition::Accepted,
            false,
        ));
        assert!(!adapter_entry_decision_is_new(
            CommandDisposition::Accepted,
            true,
        ));
        assert!(!adapter_entry_decision_is_new(
            CommandDisposition::Rejected,
            false,
        ));
        assert!(!adapter_entry_decision_is_new(
            CommandDisposition::Rejected,
            true,
        ));
    }

    #[test]
    fn final_entry_retry_observes_fresh_boundary_time() -> Result<(), RuntimeError> {
        let clock = AdvancingClock(AtomicU64::new(41));
        let run = RunId::new("run-final-entry-fresh-time")
            .map_err(|error| RuntimeError::InvalidCommand(error.to_string()))?;
        let mut observed = Vec::new();
        let committed = retry_final_entry(&clock, |now| {
            observed.push(now);
            if observed.len() < 3 {
                return Err(RuntimeError::Persistence(
                    PersistenceError::SequenceConflict {
                        run: run.clone(),
                        expected: RunSequence::ZERO,
                        actual: RunSequence::FIRST,
                    },
                ));
            }
            Ok(now)
        })?;
        assert_eq!(
            observed,
            [
                TimestampMillis::new(41),
                TimestampMillis::new(42),
                TimestampMillis::new(43)
            ]
        );
        assert_eq!(committed, TimestampMillis::new(43));
        Ok(())
    }
}
