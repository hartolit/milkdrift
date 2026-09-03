//! Durable effect claiming and incremental observation ingestion.
//!
//! Scheduler calls only create immutable requests and leases. External work is claimed
//! explicitly and may be executed on any caller-owned bounded thread/task. The runtime
//! starts no hidden worker, and every successful reporter call is already durable.

use std::sync::Mutex;

use milkdrift_authority::{BoundaryTimeMillis, DecisionId};
use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, ErrorClass, InvocationEvent,
};
use milkdrift_persistence::{
    AttemptId, CommandDisposition, CommandId, ControllerAccountAction, ControllerAdmissionOutcome,
    ControllerReservationId, LeaseId, PageSize, PersistenceError, Reason, RunEventKind,
    TimestampMillis,
};
use milkdrift_workspace::RunId;
use tracing::warn;

use super::support::{
    CommandPlan, cancellation_reason_for_execution, checked_timestamp_add, run_drain_reason,
};
use super::{CommandExecution, RuntimeService};
use crate::projection::{AttemptState, NodeExecutionState};
use crate::{
    CancellationDispatch, EffectAction, ExecutionDispatch, ExecutionReporter, ExecutorError,
    MAX_REPORTS_PER_DISPATCH, ObservationDisposition, RunCommand, RunCommandDocument, RuntimeError,
    SystemTransition, WorkerReport,
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

    fn execute_invocation_effect(
        &self,
        dispatch: &ExecutionDispatch,
    ) -> Result<EffectExecutionResult, RuntimeError> {
        let prepared_entry = retry_final_entry(self.clock.as_ref(), |now| {
            let projection = self.projection(dispatch.run())?;
            let attempt = projection
                .attempts()
                .get(dispatch.attempt())
                .ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "effect ticket attempt is no longer active".to_owned(),
                    )
                })?;
            let execution = projection
                .node_executions()
                .get(dispatch.execution())
                .ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "effect ticket execution is no longer active".to_owned(),
                    )
                })?;
            let lease = projection.leases().get(dispatch.lease()).ok_or_else(|| {
                RuntimeError::InvalidTransition(
                    "effect ticket lease is no longer active".to_owned(),
                )
            })?;
            if execution.cancellation().is_some()
                || cancellation_reason_for_execution(
                    &projection,
                    execution.execution(),
                    run_drain_reason(&projection),
                )
                .is_some()
            {
                return Ok(None);
            }
            let exact_ticket_coordinates = [
                attempt.state() == &AttemptState::Running,
                attempt.execution() == dispatch.execution(),
                attempt.request() == Some(dispatch.request()),
                attempt.capability().map(|value| value.snapshot()) == Some(dispatch.resolution()),
                attempt.capability().and_then(|value| value.authorization())
                    == Some(dispatch.resolution_authorization()),
                attempt.entry_authorization() == Some(dispatch.entry_authorization()),
                projection.execution_authority() == Some(dispatch.execution_authority()),
                matches!(execution.state(), NodeExecutionState::Running(active) if active == dispatch.attempt()),
                execution.node() == dispatch.node(),
                projection.revision_for_attempt(dispatch.attempt()) == Some(dispatch.revision()),
                lease.is_active(),
                lease.attempt() == dispatch.attempt(),
                lease.execution() == dispatch.execution(),
                lease.worker() == &self.config.worker,
                lease.expires_at() == dispatch.lease_expires_at(),
                lease.expires_at() > now,
            ];
            if exact_ticket_coordinates.contains(&false) {
                return Err(RuntimeError::InvalidTransition(
                    "effect ticket no longer matches the exact active attempt and lease".to_owned(),
                ));
            }
            let next_sequence = attempt
                .last_report_sequence()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| {
                    RuntimeError::InvalidTransition("report sequence overflow".to_owned())
                })?;
            let mut request = dispatch.resolution_authorization().request().clone();
            let identity = format!("{}:adapter-entry", dispatch.entry_authorization().digest());
            request.decision =
                DecisionId::new(format!("decision:{}", blake3::hash(identity.as_bytes())))?;
            request.evaluated_at = BoundaryTimeMillis::new(now.get());
            let authorization = self.authority.evaluate(&request)?;
            let adapter_dispatch = authorization
                .is_allowed()
                .then(|| dispatch.with_entry_authorization(authorization.clone()))
                .transpose()?;
            let prepared = adapter_dispatch
                .as_ref()
                .map(|exact| self.executor.prepare_exact_entry(exact))
                .transpose()?;
            let mut controller_actions = Vec::new();
            let mut expected_controller_revision = None;
            let controller_admission = if let (Some(prepared), Some(account)) = (
                prepared.as_ref(),
                self.controller_account_for_run(dispatch.run())?,
            ) {
                let category = dispatch.resolution().category().cloned().ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "current resolved capability snapshot has no frozen category".to_owned(),
                    )
                })?;
                let reservation = ControllerReservationId::for_attempt(
                    account.declaration().account(),
                    dispatch.attempt(),
                )?;
                let mut candidate = account.clone();
                let outcome = candidate.admit(
                    reservation.clone(),
                    dispatch.attempt().clone(),
                    category.clone(),
                    prepared.admission_envelope(),
                )?;
                expected_controller_revision = Some((
                    account.declaration().account().clone(),
                    account.revision_digest().clone(),
                ));
                controller_actions.push(ControllerAccountAction::AdmitEntry {
                    account: account.declaration().account().clone(),
                    reservation,
                    attempt: dispatch.attempt().clone(),
                    category,
                    envelope: prepared.admission_envelope().clone(),
                    expected_outcome: outcome.clone(),
                });
                outcome
            } else {
                ControllerAdmissionOutcome::NotControlled
            };
            let mut events = vec![RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: dispatch.attempt().clone(),
                authorization: authorization.clone(),
                controller_admission: controller_admission.clone(),
            }];
            if !authorization.is_allowed() {
                events.push(RunEventKind::NodeTerminal {
                    execution: dispatch.execution().clone(),
                    attempt: dispatch.attempt().clone(),
                    report_sequence: next_sequence,
                    outcome: milkdrift_persistence::NodeOutcome::Rejected,
                    error_class: Some(ErrorClass::Authorization),
                    detail: Some(milkdrift_persistence::BoundedDetail::new(format!(
                        "authority decision {} denied final adapter entry",
                        authorization.digest(),
                    ))?),
                });
            } else if let ControllerAdmissionOutcome::Denied { reason, .. } = &controller_admission
            {
                events.push(RunEventKind::NodeTerminal {
                    execution: dispatch.execution().clone(),
                    attempt: dispatch.attempt().clone(),
                    report_sequence: next_sequence,
                    outcome: milkdrift_persistence::NodeOutcome::Rejected,
                    error_class: Some(ErrorClass::RateLimit),
                    detail: Some(milkdrift_persistence::BoundedDetail::new(format!(
                        "controller resource admission denied: {reason:?}"
                    ))?),
                });
            }
            let decision_commit = self.commit_internal_plan(
                dispatch.run(),
                now,
                SystemTransition::DecideCapabilityAdapterEntry {
                    attempt: dispatch.attempt().clone(),
                },
                CommandPlan {
                    events,
                    controller_actions,
                    expected_controller_revision,
                    ..CommandPlan::default()
                },
            )?;
            if !adapter_entry_decision_is_new(
                decision_commit.result().disposition(),
                decision_commit.replayed(),
            ) {
                return Err(RuntimeError::InvalidTransition(
                    "final adapter-entry decision was not newly committed".to_owned(),
                ));
            }
            if !authorization.is_allowed()
                || matches!(
                    controller_admission,
                    ControllerAdmissionOutcome::Denied { .. }
                )
            {
                return Ok(None);
            }
            Ok(Some((
                prepared.ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "allowed adapter entry lost its prepared handle".to_owned(),
                    )
                })?,
                adapter_dispatch.ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "allowed adapter entry lost its exact dispatch".to_owned(),
                    )
                })?,
                controller_admission,
                next_sequence,
            )))
        })?;
        let Some((prepared, adapter_dispatch, controller_admission, next_sequence)) =
            prepared_entry
        else {
            return Ok(EffectExecutionResult::Completed { observations: 0 });
        };
        let reporter = DurableExecutionReporter::new(self, &adapter_dispatch, next_sequence);
        let boundary = prepared.enter_with_controller_reservation(
            &adapter_dispatch,
            controller_admission.reservation(),
            &reporter,
        );
        let outcome = reporter.finish()?;
        if outcome.terminal_seen {
            if let Some(failure) = outcome.failure {
                let error = failure.into_runtime_error();
                warn!(
                    run = %adapter_dispatch.run(),
                    attempt = %adapter_dispatch.attempt(),
                    reason = %error,
                    "executor reported an error after a terminal observation was already durable"
                );
            } else if let Err(error) = boundary {
                warn!(
                    run = %adapter_dispatch.run(),
                    attempt = %adapter_dispatch.attempt(),
                    reason = %error,
                    "executor returned an error after a terminal observation was already durable"
                );
            }
            return Ok(EffectExecutionResult::Completed {
                observations: outcome.observations,
            });
        }
        if let Some(failure) = outcome.failure {
            let error = failure.into_runtime_error();
            self.record_effect_uncertainty(
                adapter_dispatch.run(),
                adapter_dispatch.attempt(),
                &format!("executor report rejected after adapter entry: {error}"),
            )?;
            return Err(error);
        }
        match boundary {
            Ok(()) => {
                let error = RuntimeError::Executor(ExecutorError::InvalidReports(
                    "executor returned without a terminal observation".to_owned(),
                ));
                self.record_effect_uncertainty(
                    adapter_dispatch.run(),
                    adapter_dispatch.attempt(),
                    &error.to_string(),
                )?;
                Err(error)
            }
            Err(error @ ExecutorError::InvalidReports(_)) => {
                self.record_effect_uncertainty(
                    adapter_dispatch.run(),
                    adapter_dispatch.attempt(),
                    &format!("executor report rejected after adapter entry: {error}"),
                )?;
                Err(RuntimeError::Executor(error))
            }
            Err(error) => {
                warn!(
                    run = %adapter_dispatch.run(),
                    attempt = %adapter_dispatch.attempt(),
                    reason = %error,
                    "external effect boundary returned without a terminal observation"
                );
                self.record_effect_uncertainty(
                    adapter_dispatch.run(),
                    adapter_dispatch.attempt(),
                    &format!(
                        "external effect boundary returned without terminal evidence: {error}"
                    ),
                )?;
                Ok(EffectExecutionResult::Uncertain {
                    observations: outcome.observations,
                })
            }
        }
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

fn retry_final_entry<T>(
    clock: &dyn crate::BoundaryClock,
    mut attempt: impl FnMut(TimestampMillis) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    for _ in 0..MAX_OBSERVATION_COMMIT_RETRIES {
        // A retry is a new final-entry boundary. Its lease and authority checks must not reuse
        // time observed by a prior transaction attempt.
        let now = clock.now()?;
        match attempt(now) {
            Err(RuntimeError::Persistence(
                PersistenceError::SequenceConflict { .. }
                | PersistenceError::ControllerAccountRevisionConflict { .. },
            )) => {}
            result => return result,
        }
    }
    Err(RuntimeError::Scheduling(
        "final adapter entry could not obtain a stable account revision".to_owned(),
    ))
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

enum ReportIngestionFailure {
    Runtime(RuntimeError),
    InvalidReports(String),
}

impl ReportIngestionFailure {
    fn into_runtime_error(self) -> RuntimeError {
        match self {
            Self::Runtime(error) => error,
            Self::InvalidReports(reason) => {
                RuntimeError::Executor(ExecutorError::InvalidReports(reason))
            }
        }
    }
}

struct ReporterOutcome {
    terminal_seen: bool,
    observations: u32,
    failure: Option<ReportIngestionFailure>,
}

struct ReporterState {
    next_sequence: u64,
    terminal_seen: bool,
    observations: u32,
    failure: Option<ReportIngestionFailure>,
}

impl ReporterState {
    fn prior_failure(&self) -> Option<ExecutorError> {
        self.failure.as_ref().map(|failure| match failure {
            ReportIngestionFailure::Runtime(_error) => ExecutorError::Boundary(
                "runtime rejected an executor observation; typed failure retained internally"
                    .to_owned(),
            ),
            ReportIngestionFailure::InvalidReports(reason) => {
                ExecutorError::InvalidReports(reason.clone())
            }
        })
    }

    fn reject_runtime(&mut self, error: RuntimeError) -> ExecutorError {
        if self.failure.is_none() {
            self.failure = Some(ReportIngestionFailure::Runtime(error));
        }
        ExecutorError::Boundary(
            "runtime rejected an executor observation; typed failure retained internally"
                .to_owned(),
        )
    }

    fn reject_invalid_reports(&mut self, reason: impl Into<String>) -> ExecutorError {
        let reason = reason.into();
        if self.failure.is_none() {
            self.failure = Some(ReportIngestionFailure::InvalidReports(reason.clone()));
        }
        ExecutorError::InvalidReports(reason)
    }

    fn next_observation_count(&mut self) -> Result<u32, ExecutorError> {
        let maximum = match u32::try_from(MAX_REPORTS_PER_DISPATCH) {
            Ok(maximum) => maximum,
            Err(_error) => {
                return Err(self.reject_invalid_reports(
                    "maximum report count cannot be represented by the reporter",
                ));
            }
        };
        if self.observations >= maximum {
            return Err(self.reject_invalid_reports(format!(
                "an executor may submit at most {MAX_REPORTS_PER_DISPATCH} observations per dispatch"
            )));
        }
        match self.observations.checked_add(1) {
            Some(next) => Ok(next),
            None => Err(self.reject_invalid_reports("executor observation count overflow")),
        }
    }
}

struct DurableExecutionReporter<'a> {
    runtime: &'a RuntimeService,
    run: RunId,
    attempt: AttemptId,
    lease: LeaseId,
    invocation: milkdrift_capability::InvocationId,
    state: Mutex<ReporterState>,
}

impl<'a> DurableExecutionReporter<'a> {
    fn new(runtime: &'a RuntimeService, dispatch: &ExecutionDispatch, next_sequence: u64) -> Self {
        Self {
            runtime,
            run: dispatch.run().clone(),
            attempt: dispatch.attempt().clone(),
            lease: dispatch.lease().clone(),
            invocation: dispatch.request().invocation().clone(),
            state: Mutex::new(ReporterState {
                next_sequence,
                terminal_seen: false,
                observations: 0,
                failure: None,
            }),
        }
    }

    fn finish(&self) -> Result<ReporterOutcome, RuntimeError> {
        let mut state = self.state.lock().map_err(|_error| {
            RuntimeError::Scheduling("execution reporter state lock is poisoned".to_owned())
        })?;
        Ok(ReporterOutcome {
            terminal_seen: state.terminal_seen,
            observations: state.observations,
            failure: state.failure.take(),
        })
    }
}

impl ExecutionReporter for DurableExecutionReporter<'_> {
    fn invocation(&self, report: InvocationEvent) -> Result<ObservationDisposition, ExecutorError> {
        let mut state = self.state.lock().map_err(|_error| {
            ExecutorError::Boundary("execution reporter state lock is poisoned".to_owned())
        })?;
        if let Some(error) = state.prior_failure() {
            return Err(error);
        }
        if state.terminal_seen {
            return Err(state.reject_invalid_reports("no report may follow a terminal observation"));
        }
        let expected_sequence = state.next_sequence;
        if report.invocation() != &self.invocation || report.sequence() != expected_sequence {
            return Err(state.reject_invalid_reports(format!(
                "invocation observations must be contiguous; expected sequence {expected_sequence}"
            )));
        }
        let terminal = report.kind().terminal().is_some();
        let next_sequence = if terminal {
            None
        } else {
            match state.next_sequence.checked_add(1) {
                Some(next_sequence) => Some(next_sequence),
                None => {
                    return Err(state.reject_invalid_reports("invocation report sequence overflow"));
                }
            }
        };
        let next_observations = state.next_observation_count()?;
        let worker_report = WorkerReport::Invocation {
            attempt: self.attempt.clone(),
            report,
        };
        let command = match stable_effect_command_id(&self.run, &self.attempt, &worker_report) {
            Ok(command) => command,
            Err(error) => return Err(state.reject_runtime(error)),
        };
        let observed_at = match self.runtime.clock.now() {
            Ok(observed_at) => observed_at,
            Err(error) => return Err(state.reject_runtime(error)),
        };
        let reason = match Reason::new("executor supplied an incremental invocation observation") {
            Ok(reason) => reason,
            Err(error) => return Err(state.reject_runtime(error.into())),
        };
        let disposition = match self.runtime.submit_effect_observation(
            &self.run,
            command,
            observed_at,
            reason,
            worker_report,
        ) {
            Ok(disposition) => disposition,
            Err(error) => return Err(state.reject_runtime(error)),
        };
        if let Some(next_sequence) = next_sequence {
            state.next_sequence = next_sequence;
        }
        state.terminal_seen = terminal;
        state.observations = next_observations;
        Ok(disposition)
    }

    fn heartbeat(&self) -> Result<ObservationDisposition, ExecutorError> {
        let mut state = self.state.lock().map_err(|_error| {
            ExecutorError::Boundary("execution reporter state lock is poisoned".to_owned())
        })?;
        if let Some(error) = state.prior_failure() {
            return Err(error);
        }
        if state.terminal_seen {
            return Err(
                state.reject_invalid_reports("a terminal invocation cannot renew its lease")
            );
        }
        let next_observations = state.next_observation_count()?;
        let observed_at = match self.runtime.clock.now() {
            Ok(observed_at) => observed_at,
            Err(error) => return Err(state.reject_runtime(error)),
        };
        let expires_at =
            match checked_timestamp_add(observed_at, self.runtime.config.lease_duration_ms) {
                Ok(expires_at) => expires_at,
                Err(error) => return Err(state.reject_runtime(error)),
            };
        let report = WorkerReport::Heartbeat {
            lease: self.lease.clone(),
            expires_at,
        };
        let command = match stable_effect_command_id(&self.run, &self.attempt, &report) {
            Ok(command) => command,
            Err(error) => return Err(state.reject_runtime(error)),
        };
        let reason = match Reason::new("executor renewed its durable lease") {
            Ok(reason) => reason,
            Err(error) => return Err(state.reject_runtime(error.into())),
        };
        let disposition = match self.runtime.submit_effect_observation(
            &self.run,
            command,
            observed_at,
            reason,
            report,
        ) {
            Ok(disposition) => disposition,
            Err(error) => return Err(state.reject_runtime(error)),
        };
        state.observations = next_observations;
        Ok(disposition)
    }
}

fn terminal_report_identity(report: &WorkerReport) -> Option<(AttemptId, u64)> {
    match report {
        WorkerReport::Invocation { attempt, report } => report
            .kind()
            .terminal()
            .map(|_terminal| (attempt.clone(), report.sequence())),
        WorkerReport::Terminal {
            attempt,
            report_sequence,
            ..
        } => Some((attempt.clone(), *report_sequence)),
        WorkerReport::LeaseAccepted { .. }
        | WorkerReport::Heartbeat { .. }
        | WorkerReport::Started { .. }
        | WorkerReport::Cancellation { .. } => None,
    }
}

fn adapter_entry_decision_is_new(disposition: CommandDisposition, replayed: bool) -> bool {
    disposition == CommandDisposition::Accepted && !replayed
}

fn stable_effect_command_id(
    run: &RunId,
    attempt: &AttemptId,
    report: &WorkerReport,
) -> Result<CommandId, RuntimeError> {
    let bytes = serde_json::to_vec(report)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.effect-observation-command.v1\0");
    for component in [
        run.as_str().as_bytes(),
        attempt.as_str().as_bytes(),
        bytes.as_slice(),
    ] {
        let length = u64::try_from(component.len()).map_err(|_error| {
            RuntimeError::InvalidCommand(
                "effect observation identity input is too large".to_owned(),
            )
        })?;
        hasher.update(&length.to_be_bytes());
        hasher.update(component);
    }
    CommandId::new(format!("effect:{}", hasher.finalize().to_hex())).map_err(RuntimeError::from)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{adapter_entry_decision_is_new, retry_final_entry};
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
