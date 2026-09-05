//! Final adapter-entry revalidation, durable decision, and boundary execution.

use milkdrift_authority::{BoundaryTimeMillis, DecisionId};
use milkdrift_capability::ErrorClass;
use milkdrift_persistence::{
    ControllerAccountAction, ControllerAdmissionOutcome, ControllerReservationId, PersistenceError,
    RunEventKind, TimestampMillis,
};
use tracing::warn;

use super::super::RuntimeService;
use super::super::support::{CommandPlan, cancellation_reason_for_execution, run_drain_reason};
use super::reporter::{DurableExecutionReporter, adapter_entry_decision_is_new};
use super::{EffectExecutionResult, MAX_OBSERVATION_COMMIT_RETRIES};
use crate::projection::{AttemptState, NodeExecutionState};
use crate::{ExecutionDispatch, ExecutorError, PreparedExecution, RuntimeError, SystemTransition};

struct FinalEntry<'a> {
    prepared: PreparedExecution<'a>,
    dispatch: ExecutionDispatch,
    controller_admission: ControllerAdmissionOutcome,
    next_sequence: u64,
}

impl RuntimeService {
    fn prepare_final_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
        now: TimestampMillis,
    ) -> Result<Option<FinalEntry<'a>>, RuntimeError> {
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
            RuntimeError::InvalidTransition("effect ticket lease is no longer active".to_owned())
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
        } else if let ControllerAdmissionOutcome::Denied { reason, .. } = &controller_admission {
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
        let prepared = prepared.ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "allowed adapter entry lost its prepared handle".to_owned(),
            )
        })?;
        let dispatch = adapter_dispatch.ok_or_else(|| {
            RuntimeError::InvalidHistory("allowed adapter entry lost its exact dispatch".to_owned())
        })?;
        Ok(Some(FinalEntry {
            prepared,
            dispatch,
            controller_admission,
            next_sequence,
        }))
    }

    pub(super) fn execute_invocation_effect(
        &self,
        dispatch: &ExecutionDispatch,
    ) -> Result<EffectExecutionResult, RuntimeError> {
        let Some(FinalEntry {
            prepared,
            dispatch: adapter_dispatch,
            controller_admission,
            next_sequence,
        }) = retry_final_entry(self.clock.as_ref(), |now| {
            self.prepare_final_entry(dispatch, now)
        })?
        else {
            return Ok(EffectExecutionResult::Completed { observations: 0 });
        };
        let reporter = DurableExecutionReporter::new(self, &adapter_dispatch, next_sequence);
        let boundary = prepared.enter_with_controller_reservation(
            &adapter_dispatch,
            controller_admission.reservation(),
            &reporter,
        );
        let mut outcome = reporter.finish()?;
        if outcome.terminal_seen() {
            if let Some(failure) = outcome.take_failure() {
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
                observations: outcome.observations(),
            });
        }
        if let Some(failure) = outcome.take_failure() {
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
                    observations: outcome.observations(),
                })
            }
        }
    }
}

pub(super) fn retry_final_entry<T>(
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
