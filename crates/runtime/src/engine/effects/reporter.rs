//! Durable ingestion of incremental executor observations.

use std::sync::Mutex;

use milkdrift_capability::InvocationEvent;
use milkdrift_persistence::{AttemptId, CommandDisposition, CommandId, LeaseId, Reason};
use milkdrift_workspace::RunId;

use super::super::RuntimeService;
use super::super::support::checked_timestamp_add;
use crate::{
    ExecutionDispatch, ExecutionReporter, ExecutorError, MAX_REPORTS_PER_DISPATCH,
    ObservationDisposition, RuntimeError, WorkerReport,
};

pub(super) enum ReportIngestionFailure {
    Runtime(RuntimeError),
    InvalidReports(String),
}

impl ReportIngestionFailure {
    pub(super) fn into_runtime_error(self) -> RuntimeError {
        match self {
            Self::Runtime(error) => error,
            Self::InvalidReports(reason) => {
                RuntimeError::Executor(ExecutorError::InvalidReports(reason))
            }
        }
    }
}

pub(super) struct ReporterOutcome {
    terminal_seen: bool,
    observations: u32,
    failure: Option<ReportIngestionFailure>,
}

impl ReporterOutcome {
    pub(super) fn terminal_seen(&self) -> bool {
        self.terminal_seen
    }

    pub(super) fn observations(&self) -> u32 {
        self.observations
    }

    pub(super) fn take_failure(&mut self) -> Option<ReportIngestionFailure> {
        self.failure.take()
    }
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

pub(super) struct DurableExecutionReporter<'a> {
    runtime: &'a RuntimeService,
    run: RunId,
    attempt: AttemptId,
    lease: LeaseId,
    invocation: milkdrift_capability::InvocationId,
    state: Mutex<ReporterState>,
}

impl<'a> DurableExecutionReporter<'a> {
    pub(super) fn new(
        runtime: &'a RuntimeService,
        dispatch: &ExecutionDispatch,
        next_sequence: u64,
    ) -> Self {
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

    pub(super) fn finish(&self) -> Result<ReporterOutcome, RuntimeError> {
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

pub(super) fn terminal_report_identity(report: &WorkerReport) -> Option<(AttemptId, u64)> {
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

pub(super) fn adapter_entry_decision_is_new(
    disposition: CommandDisposition,
    replayed: bool,
) -> bool {
    disposition == CommandDisposition::Accepted && !replayed
}

pub(super) fn stable_effect_command_id(
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
