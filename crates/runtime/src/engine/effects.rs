//! Durable effect claiming and incremental observation ingestion.
//!
//! Scheduler calls only create immutable requests and leases. External work is claimed
//! explicitly and may be executed on any caller-owned bounded thread/task. The runtime
//! starts no hidden worker, and every successful reporter call is already durable.

use std::sync::Mutex;

use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, ErrorClass, InvocationEvent,
};
use milkdrift_persistence::{
    AttemptId, CommandDisposition, CommandId, LeaseId, PageSize, PersistenceError, Reason,
    RunEventKind, TimestampMillis,
};
use milkdrift_workspace::RunId;
use tracing::warn;

use super::support::{
    CommandPlan, cancellation_reason_for_execution, checked_timestamp_add, run_drain_reason,
};
use super::{CommandExecution, EffectTickResult, RuntimeService};
use crate::projection::{AttemptState, NodeExecutionState};
use crate::{
    CancellationDispatch, EffectAction, ExecutionDispatch, ExecutionReporter, ExecutorError,
    ObservationDisposition, RunCommand, RunCommandDocument, RuntimeError, SystemTransition,
    WorkerReport,
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
            let attempt = projection.attempts().get(&indexed.attempt).ok_or_else(|| {
                RuntimeError::InvalidHistory("active lease names an absent attempt".to_owned())
            })?;
            match attempt.state() {
                AttemptState::Leased => {
                    if let Some(dispatch) =
                        self.claim_invocation(&indexed.run, &indexed.lease, &indexed.attempt)?
                    {
                        actions.push(EffectAction::Execute(Box::new(dispatch)));
                    }
                }
                AttemptState::Running => {
                    if let Some(cancellation) =
                        self.cancellation_dispatch(&indexed.run, &projection, &indexed.attempt)?
                    {
                        actions.push(EffectAction::Cancel(cancellation));
                    }
                }
                AttemptState::AwaitingRetryTimer
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
        action: &EffectAction,
    ) -> Result<EffectExecutionResult, RuntimeError> {
        match action {
            EffectAction::Execute(dispatch) => self.execute_invocation_effect(dispatch),
            EffectAction::Cancel(dispatch) => self.execute_cancellation_effect(dispatch),
        }
    }

    /// Blocking compatibility host for tests and simple embeddings.
    ///
    /// Production daemons should claim effects and execute them on caller-owned bounded
    /// workers so scheduling, cancellation intent, inspection, and heartbeats remain
    /// independently responsive.
    pub fn effect_tick(&self) -> Result<EffectTickResult, RuntimeError> {
        let limit = PageSize::new(u32::from(self.config.maximum_tick_items))?;
        let actions = self.claim_effects(limit)?;
        let mut result = EffectTickResult {
            claimed: u32::try_from(actions.len()).map_err(|_error| {
                RuntimeError::Scheduling("claimed effect count exceeds u32".to_owned())
            })?,
            ..EffectTickResult::default()
        };
        for action in &actions {
            match self.execute_effect(action)? {
                EffectExecutionResult::Completed { observations } => {
                    result.completed = result.completed.saturating_add(1);
                    result.observations = result.observations.saturating_add(observations);
                }
                EffectExecutionResult::Uncertain { observations } => {
                    result.uncertain = result.uncertain.saturating_add(1);
                    result.observations = result.observations.saturating_add(observations);
                }
                EffectExecutionResult::CancellationAcknowledged => {
                    result.cancellations = result.cancellations.saturating_add(1);
                    result.observations = result.observations.saturating_add(1);
                }
                EffectExecutionResult::CancellationDeferred => {
                    result.cancellation_deferred = result.cancellation_deferred.saturating_add(1);
                }
            }
        }
        Ok(result)
    }

    /// Explicit blocking compatibility driver: schedule once, then execute claimed effects.
    pub fn drive_once(&self) -> Result<super::SchedulerTickResult, RuntimeError> {
        let mut scheduled = self.scheduler_tick()?;
        let effects = self.effect_tick()?;
        scheduled.completed = scheduled.completed.saturating_add(effects.completed);
        scheduled.uncertain = scheduled.uncertain.saturating_add(effects.uncertain);
        Ok(scheduled)
    }

    fn claim_invocation(
        &self,
        run: &RunId,
        lease: &LeaseId,
        attempt: &AttemptId,
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
        {
            return Ok(None);
        }
        let execution = projection
            .node_executions()
            .get(attempt_view.execution())
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("claimed attempt execution is absent".to_owned())
            })?;
        let revision = projection.revision_for_attempt(attempt).ok_or_else(|| {
            RuntimeError::InvalidHistory("claimed attempt has no governing revision".to_owned())
        })?;
        let request = attempt_view.request().ok_or_else(|| {
            RuntimeError::InvalidHistory("claimed attempt has no immutable request".to_owned())
        })?;
        let capability = attempt_view.capability().ok_or_else(|| {
            RuntimeError::InvalidHistory("claimed attempt has no capability snapshot".to_owned())
        })?;
        let dispatch = ExecutionDispatch::from_snapshot(
            run.clone(),
            revision.clone(),
            execution.node().clone(),
            execution.execution().clone(),
            attempt.clone(),
            lease.clone(),
            lease_view.expires_at(),
            capability.snapshot().clone(),
            request.clone(),
        )?;
        let report = WorkerReport::LeaseAccepted {
            lease: lease.clone(),
            attempt: attempt.clone(),
        };
        let execution = self.commit_worker_report(
            run,
            stable_effect_command_id(run, attempt, &report)?,
            self.clock.now()?,
            Reason::new("effect host accepted a durable dispatch lease")?,
            report,
        )?;
        if execution.result().disposition() != CommandDisposition::Accepted || execution.replayed()
        {
            return Ok(None);
        }
        Ok(Some(dispatch))
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
        let next_sequence = self
            .projection(dispatch.run())?
            .attempts()
            .get(dispatch.attempt())
            .and_then(crate::projection::NodeAttemptProjection::last_report_sequence)
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| {
                RuntimeError::InvalidTransition("report sequence overflow".to_owned())
            })?;
        let reporter = DurableExecutionReporter::new(self, dispatch, next_sequence);
        let boundary = self.executor.execute_streaming(dispatch, &reporter);
        let terminal_seen = reporter.terminal_seen()?;
        let observations = reporter.observations()?;
        if terminal_seen {
            if let Err(error) = boundary {
                warn!(
                    run = %dispatch.run(),
                    attempt = %dispatch.attempt(),
                    reason = %error,
                    "executor returned an error after a terminal observation was already durable"
                );
            }
            return Ok(EffectExecutionResult::Completed { observations });
        }
        if let Err(error) = &boundary {
            warn!(
                run = %dispatch.run(),
                attempt = %dispatch.attempt(),
                reason = %error,
                "external effect boundary returned without a terminal observation"
            );
        }
        self.record_effect_uncertainty(dispatch.run(), dispatch.attempt())?;
        Ok(EffectExecutionResult::Uncertain { observations })
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
        let execution = self.commit_worker_report(
            dispatch.run(),
            stable_effect_command_id(dispatch.run(), dispatch.attempt(), &report)?,
            self.clock.now()?,
            Reason::new("executor acknowledged durable cancellation intent")?,
            report,
        )?;
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
        let execution = self.commit_worker_report(run, command, observed_at, reason, report)?;
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
    ) -> Result<CommandExecution, RuntimeError> {
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
            match self.handle_command(&document) {
                Ok(execution) => return Ok(execution),
                Err(RuntimeError::Persistence(PersistenceError::SequenceConflict { .. })) => {}
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
    ) -> Result<(), RuntimeError> {
        let projection = self.projection(run)?;
        let attempt_view = projection.attempts().get(attempt).ok_or_else(|| {
            RuntimeError::InvalidHistory("uncertain effect attempt is absent".to_owned())
        })?;
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
        let mut plan = CommandPlan::one(RunEventKind::ExternalOutcomeUncertain {
            attempt: attempt.clone(),
            report_sequence,
            side_effect: side_effect.side_effect(),
            reason: Reason::new(
                "external effect boundary returned without a terminal observation",
            )?,
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
                self.clock.now()?,
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
            self.clock.now()?,
            SystemTransition::DispatchOutcomeUncertain {
                attempt: attempt.clone(),
            },
            plan,
        )?;
        Ok(())
    }
}

struct ReporterState {
    next_sequence: u64,
    terminal_seen: bool,
    observations: u32,
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
            }),
        }
    }

    fn terminal_seen(&self) -> Result<bool, RuntimeError> {
        Ok(self
            .state
            .lock()
            .map_err(|_error| {
                RuntimeError::Scheduling("execution reporter state lock is poisoned".to_owned())
            })?
            .terminal_seen)
    }

    fn observations(&self) -> Result<u32, RuntimeError> {
        Ok(self
            .state
            .lock()
            .map_err(|_error| {
                RuntimeError::Scheduling("execution reporter state lock is poisoned".to_owned())
            })?
            .observations)
    }
}

impl ExecutionReporter for DurableExecutionReporter<'_> {
    fn invocation(&self, report: InvocationEvent) -> Result<ObservationDisposition, ExecutorError> {
        let mut state = self.state.lock().map_err(|_error| {
            ExecutorError::Boundary("execution reporter state lock is poisoned".to_owned())
        })?;
        if state.terminal_seen {
            return Err(ExecutorError::InvalidReports(
                "no report may follow a terminal observation".to_owned(),
            ));
        }
        if report.invocation() != &self.invocation || report.sequence() != state.next_sequence {
            return Err(ExecutorError::InvalidReports(format!(
                "invocation observations must be contiguous; expected sequence {}",
                state.next_sequence
            )));
        }
        let terminal = report.kind().terminal().is_some();
        let worker_report = WorkerReport::Invocation {
            attempt: self.attempt.clone(),
            report,
        };
        let disposition = self
            .runtime
            .submit_effect_observation(
                &self.run,
                stable_effect_command_id(&self.run, &self.attempt, &worker_report)
                    .map_err(|error| ExecutorError::Boundary(error.to_string()))?,
                self.runtime
                    .clock
                    .now()
                    .map_err(|error| ExecutorError::Boundary(error.to_string()))?,
                Reason::new("executor supplied an incremental invocation observation")
                    .map_err(|error| ExecutorError::Boundary(error.to_string()))?,
                worker_report,
            )
            .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        state.next_sequence = state.next_sequence.checked_add(1).ok_or_else(|| {
            ExecutorError::InvalidReports("invocation report sequence overflow".to_owned())
        })?;
        state.terminal_seen = terminal;
        state.observations = state.observations.saturating_add(1);
        Ok(disposition)
    }

    fn heartbeat(&self) -> Result<ObservationDisposition, ExecutorError> {
        let mut state = self.state.lock().map_err(|_error| {
            ExecutorError::Boundary("execution reporter state lock is poisoned".to_owned())
        })?;
        if state.terminal_seen {
            return Err(ExecutorError::InvalidReports(
                "a terminal invocation cannot renew its lease".to_owned(),
            ));
        }
        let observed_at = self
            .runtime
            .clock
            .now()
            .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        let expires_at = checked_timestamp_add(observed_at, self.runtime.config.lease_duration_ms)
            .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        let report = WorkerReport::Heartbeat {
            lease: self.lease.clone(),
            expires_at,
        };
        let disposition = self
            .runtime
            .submit_effect_observation(
                &self.run,
                stable_effect_command_id(&self.run, &self.attempt, &report)
                    .map_err(|error| ExecutorError::Boundary(error.to_string()))?,
                observed_at,
                Reason::new("executor renewed its durable lease")
                    .map_err(|error| ExecutorError::Boundary(error.to_string()))?,
                report,
            )
            .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        state.observations = state.observations.saturating_add(1);
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
