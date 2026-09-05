//! Worker-report validation and transition planning.

use super::super::RuntimeService;
use super::super::support::{
    CommandPlan, cancellation_reason_for_execution, checked_timestamp_add, run_drain_reason,
};
use crate::projection::{AttemptState, RunProjection};
use crate::{RunCommandDocument, RuntimeError, WorkerReport};
use milkdrift_blueprint::PortId;
use milkdrift_capability::{
    ErrorClass, InvocationEvent, InvocationEventKind, InvocationTerminal, TerminalStatus,
};
use milkdrift_persistence::{
    AttemptId, AttemptUsage, BoundedDetail, ControllerAccountAction, CurrencyCode, NodeExecutionId,
    NodeOutcome, Reason, RunEventKind, TimestampMillis, WorkerId, WorkspaceMutation,
};
use milkdrift_workspace::{ArtifactId, ArtifactReference, ValueKey, WorkspaceValue};
use std::collections::BTreeSet;
use tracing::warn;

impl RuntimeService {
    pub(super) fn plan_worker_report(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        worker: &WorkerId,
        report: &WorkerReport,
    ) -> Result<CommandPlan, RuntimeError> {
        if document.actor() != &self.config.internal_actor || worker != &self.config.worker {
            return Err(RuntimeError::InvalidTransition(
                "worker reports require the configured worker and trusted internal actor boundary"
                    .to_owned(),
            ));
        }
        match report {
            WorkerReport::LeaseAccepted {
                lease,
                attempt,
                authorization,
            } => {
                let lease_view = projection.leases().get(lease).ok_or_else(|| {
                    RuntimeError::InvalidTransition(format!("unknown lease {lease}"))
                })?;
                if lease_view.worker() != worker
                    || lease_view.attempt() != attempt
                    || !lease_view.is_active()
                    || lease_view.expires_at() <= document.issued_at()
                {
                    return Err(RuntimeError::InvalidTransition(
                        "lease acceptance does not match active worker ownership".to_owned(),
                    ));
                }
                let attempt_view = projection.attempts().get(attempt).ok_or_else(|| {
                    RuntimeError::InvalidHistory("lease attempt is absent".to_owned())
                })?;
                let execution = projection
                    .node_executions()
                    .get(attempt_view.execution())
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory("lease attempt execution is absent".to_owned())
                    })?;
                if execution.cancellation().is_some()
                    || cancellation_reason_for_execution(
                        projection,
                        execution.execution(),
                        run_drain_reason(projection),
                    )
                    .is_some()
                {
                    return Err(RuntimeError::InvalidTransition(
                        "a cancelled execution cannot cross the external start boundary".to_owned(),
                    ));
                }
                let invocation = attempt_view.invocation().ok_or_else(|| {
                    RuntimeError::InvalidHistory("leased attempt has no invocation".to_owned())
                })?;
                let resolution_authorization = attempt_view
                    .capability()
                    .and_then(crate::projection::CapabilityResolution::authorization)
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "leased attempt has no resolution authorization".to_owned(),
                        )
                    })?;
                let mut expected = resolution_authorization.request().clone();
                expected.decision = authorization.request().decision.clone();
                expected.evaluated_at = authorization.request().evaluated_at;
                if !authorization.is_allowed() || authorization.request() != &expected {
                    return Err(RuntimeError::InvalidTransition(
                        "lease acceptance authorization does not re-evaluate the exact resolved generation"
                            .to_owned(),
                    ));
                }
                Ok(CommandPlan {
                    events: vec![
                        RunEventKind::CapabilityEntryDecisionRecorded {
                            attempt: attempt.clone(),
                            authorization: authorization.as_ref().clone(),
                        },
                        RunEventKind::NodeStarted {
                            execution: attempt_view.execution().clone(),
                            attempt: attempt.clone(),
                            invocation: invocation.clone(),
                        },
                    ],
                    ..CommandPlan::default()
                })
            }
            WorkerReport::Heartbeat { lease, expires_at } => {
                let lease_view = projection.leases().get(lease).ok_or_else(|| {
                    RuntimeError::InvalidTransition(format!("unknown lease {lease}"))
                })?;
                if lease_view.worker() != worker || !lease_view.is_active() {
                    return Err(RuntimeError::InvalidTransition(
                        "heartbeat does not match active worker ownership".to_owned(),
                    ));
                }
                if *expires_at <= lease_view.expires_at() || *expires_at <= document.issued_at() {
                    return Err(RuntimeError::InvalidTransition(
                        "heartbeat expiration must advance the active lease into the future"
                            .to_owned(),
                    ));
                }
                Ok(CommandPlan::one(RunEventKind::LeaseHeartbeatRecorded {
                    lease: lease.clone(),
                    expires_at: *expires_at,
                }))
            }
            WorkerReport::Started { attempt } => {
                let attempt_view = self.worker_attempt(projection, worker, attempt)?;
                let lease_is_current = projection.leases().values().any(|lease| {
                    lease.attempt() == attempt
                        && lease.worker() == worker
                        && lease.is_active()
                        && lease.expires_at() > document.issued_at()
                });
                if !lease_is_current {
                    return Err(RuntimeError::InvalidTransition(
                        "worker start requires an unexpired active lease".to_owned(),
                    ));
                }
                if attempt_view
                    .entry_authorization()
                    .is_none_or(|decision| !decision.is_allowed())
                {
                    return Err(RuntimeError::InvalidTransition(
                        "worker start requires a durable allowed capability-entry decision"
                            .to_owned(),
                    ));
                }
                let invocation = attempt_view.invocation().ok_or_else(|| {
                    RuntimeError::InvalidHistory("scheduled attempt has no invocation".to_owned())
                })?;
                Ok(CommandPlan::one(RunEventKind::NodeStarted {
                    execution: attempt_view.execution().clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                }))
            }
            WorkerReport::Invocation { attempt, report } => {
                let attempt_view = self.historically_owned_attempt(projection, worker, attempt)?;
                if attempt_view.invocation() != Some(report.invocation()) {
                    return Err(RuntimeError::InvalidTransition(
                        "invocation report correlation does not match the attempt".to_owned(),
                    ));
                }
                if let InvocationEventKind::Terminal { terminal } = report.kind()
                    && !self.worker_owns_active_lease(projection, worker, attempt)
                {
                    return self.plan_late_terminal_report(
                        projection,
                        worker,
                        attempt,
                        report.sequence(),
                        terminal,
                    );
                }
                let _ = self.worker_attempt(projection, worker, attempt)?;
                self.plan_invocation_report(document, projection, attempt, report)
            }
            WorkerReport::Cancellation {
                attempt,
                acknowledgement,
            } => {
                let attempt_view = self.worker_attempt(projection, worker, attempt)?;
                if attempt_view.invocation() != Some(acknowledgement.invocation()) {
                    return Err(RuntimeError::InvalidTransition(
                        "cancellation acknowledgement names another invocation".to_owned(),
                    ));
                }
                let mut plan = CommandPlan::one(RunEventKind::InvocationCancellationAcknowledged {
                    attempt: attempt.clone(),
                    acknowledgement: acknowledgement.clone(),
                });
                if acknowledgement.accepted() && acknowledgement.terminal_boundary() {
                    if let Some(account) = self.controller_account_for_run(document.run_id())? {
                        let reservation =
                            milkdrift_persistence::ControllerReservationId::for_attempt(
                                account.declaration().account(),
                                attempt,
                            )?;
                        if !account.reservations().contains_key(&reservation) {
                            return Err(RuntimeError::InvalidHistory(
                                "controlled terminal cancellation has no exact outstanding reservation"
                                    .to_owned(),
                            ));
                        }
                        plan.expected_controller_revision = Some((
                            account.declaration().account().clone(),
                            account.revision_digest().clone(),
                        ));
                        plan.controller_actions
                            .push(ControllerAccountAction::SettleTerminal {
                                account: account.declaration().account().clone(),
                                reservation,
                                usage: None,
                            });
                    }
                    plan.events.push(RunEventKind::NodeTerminal {
                        execution: attempt_view.execution().clone(),
                        attempt: attempt.clone(),
                        report_sequence: self.next_report_sequence(projection, attempt)?,
                        outcome: NodeOutcome::Cancelled,
                        error_class: None,
                        detail: acknowledgement
                            .detail()
                            .map(|detail| BoundedDetail::new(detail.to_owned()))
                            .transpose()?,
                    });
                }
                Ok(plan)
            }
            WorkerReport::Terminal {
                attempt,
                report_sequence,
                terminal,
            } => {
                let _ = self.historically_owned_attempt(projection, worker, attempt)?;
                if !self.worker_owns_active_lease(projection, worker, attempt) {
                    return self.plan_late_terminal_report(
                        projection,
                        worker,
                        attempt,
                        *report_sequence,
                        terminal,
                    );
                }
                self.plan_terminal_report(document, projection, attempt, *report_sequence, terminal)
            }
        }
    }

    fn worker_owns_active_lease(
        &self,
        projection: &RunProjection,
        worker: &WorkerId,
        attempt: &AttemptId,
    ) -> bool {
        projection.leases().values().any(|lease| {
            lease.attempt() == attempt && lease.worker() == worker && lease.is_active()
        })
    }

    fn historically_owned_attempt<'a>(
        &self,
        projection: &'a RunProjection,
        worker: &WorkerId,
        attempt: &AttemptId,
    ) -> Result<&'a crate::projection::NodeAttemptProjection, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        let historically_owned = attempt_view.lease_workers().contains(worker);
        if !historically_owned {
            return Err(RuntimeError::InvalidTransition(
                "worker never owned a durable lease for the attempt".to_owned(),
            ));
        }
        Ok(attempt_view)
    }

    fn worker_attempt<'a>(
        &self,
        projection: &'a RunProjection,
        worker: &WorkerId,
        attempt: &AttemptId,
    ) -> Result<&'a crate::projection::NodeAttemptProjection, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        if !self.worker_owns_active_lease(projection, worker, attempt) {
            return Err(RuntimeError::InvalidTransition(
                "worker does not own an active lease for the attempt".to_owned(),
            ));
        }
        Ok(attempt_view)
    }

    fn plan_invocation_report(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        attempt: &AttemptId,
        report: &InvocationEvent,
    ) -> Result<CommandPlan, RuntimeError> {
        match report.kind() {
            InvocationEventKind::Progress {
                message,
                completed_units,
                total_units,
            } => {
                let expected = self.next_report_sequence(projection, attempt)?;
                if report.sequence() != expected {
                    return Err(RuntimeError::InvalidTransition(format!(
                        "progress report sequence must be exactly {expected}"
                    )));
                }
                Ok(CommandPlan::one(RunEventKind::NodeProgressRecorded {
                    attempt: attempt.clone(),
                    report_sequence: report.sequence(),
                    detail: BoundedDetail::new(message.clone())?,
                    completed_units: *completed_units,
                    total_units: *total_units,
                }))
            }
            InvocationEventKind::Output { name, reference } => {
                self.plan_output_report(projection, attempt, report.sequence(), name, reference)
            }
            InvocationEventKind::Terminal { terminal } => self.plan_terminal_report(
                document,
                projection,
                attempt,
                report.sequence(),
                terminal,
            ),
        }
    }

    fn plan_output_report(
        &self,
        projection: &RunProjection,
        attempt: &AttemptId,
        report_sequence: u64,
        name: &str,
        reference: &milkdrift_capability::ArtifactReference,
    ) -> Result<CommandPlan, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        if !matches!(attempt_view.state(), AttemptState::Running) {
            return Err(RuntimeError::InvalidTransition(
                "output report requires a running attempt".to_owned(),
            ));
        }
        let expected = self.next_report_sequence(projection, attempt)?;
        if report_sequence != expected {
            return Err(RuntimeError::InvalidTransition(format!(
                "output report sequence must be exactly {expected}"
            )));
        }
        let execution = projection
            .node_executions()
            .get(attempt_view.execution())
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("attempt execution is absent".to_owned())
            })?;
        let scheduled_revision = projection.revision_for_attempt(attempt).ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "scheduled attempt has no governing revision pin".to_owned(),
            )
        })?;
        let revision = self.load_validated_revision(scheduled_revision, projection.workflow())?;
        let node = revision
            .semantic()
            .nodes()
            .get(execution.node())
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "scheduled attempt node is absent from its governing revision".to_owned(),
                )
            })?;
        let output_port = PortId::new(name.to_owned())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        if !node.data_outputs().contains_key(&output_port) {
            return Err(RuntimeError::InvalidTransition(format!(
                "executor output {name} is not a declared data output of node {}",
                node.id()
            )));
        }
        let (metadata, artifact) = self.resolve_executor_artifact(reference)?;
        let key = ValueKey::new(output_port.as_str().to_owned())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let entry = self.projected_output_entry(
            projection,
            execution.scope(),
            key,
            WorkspaceValue::Artifact(artifact.clone()),
            &[],
        )?;
        let mut plan = CommandPlan::default();
        if !projection.artifacts().contains_key(artifact.artifact()) {
            plan.events
                .push(RunEventKind::ArtifactPublished { metadata });
        }
        plan.events.push(RunEventKind::NodeOutputPublished {
            execution: attempt_view.execution().clone(),
            attempt: attempt.clone(),
            report_sequence,
            value: entry.reference().clone(),
            artifact: Some(artifact.clone()),
        });
        plan.workspace.push(WorkspaceMutation::PutValue { entry });
        plan.required_artifacts.insert(artifact);
        Ok(plan)
    }

    fn plan_late_terminal_report(
        &self,
        projection: &RunProjection,
        worker: &WorkerId,
        attempt: &AttemptId,
        report_sequence: u64,
        terminal: &InvocationTerminal,
    ) -> Result<CommandPlan, RuntimeError> {
        let attempt_view = self.historically_owned_attempt(projection, worker, attempt)?;
        let obligation = attempt_view.obligation().ok_or_else(|| {
            RuntimeError::InvalidTransition(
                "late terminal evidence is accepted only for an uncertain external outcome"
                    .to_owned(),
            )
        })?;
        if attempt_view.terminal().is_some() || attempt_view.late_terminal_evidence().is_some() {
            return Err(RuntimeError::InvalidTransition(
                "attempt already has terminal evidence".to_owned(),
            ));
        }
        if report_sequence < obligation.report_sequence() {
            return Err(RuntimeError::InvalidTransition(format!(
                "late terminal report sequence must be at least {}",
                obligation.report_sequence()
            )));
        }
        if terminal.status() == TerminalStatus::Uncertain {
            return Err(RuntimeError::InvalidTransition(
                "late evidence must establish a known terminal observation".to_owned(),
            ));
        }
        let classified = attempt_view.side_effect().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "late terminal attempt has no side-effect classification".to_owned(),
            )
        })?;
        if terminal.side_effect() > classified.side_effect() {
            return Err(RuntimeError::InvalidTransition(
                "late terminal observation exceeds the frozen side-effect classification"
                    .to_owned(),
            ));
        }
        let mut plan = CommandPlan::one(RunEventKind::LateTerminalEvidenceRecorded {
            attempt: attempt.clone(),
            worker: worker.clone(),
            report_sequence,
            terminal: terminal.clone(),
        });
        let run = projection.run_id().ok_or_else(|| {
            RuntimeError::InvalidHistory("late terminal projection has no run identity".to_owned())
        })?;
        if let Some(account) = self.controller_account_for_run(run)? {
            let reservation = milkdrift_persistence::ControllerReservationId::for_attempt(
                account.declaration().account(),
                attempt,
            )?;
            if !account.reservations().contains_key(&reservation) {
                return Err(RuntimeError::InvalidHistory(
                    "controlled late terminal evidence has no exact outstanding reservation"
                        .to_owned(),
                ));
            }
            let usage = terminal
                .usage()
                .map(|usage| {
                    let cost = match usage.cost_micros().zip(usage.currency()) {
                        Some((micros, currency)) => Some(milkdrift_persistence::MonetaryUsage {
                            micros,
                            currency: CurrencyCode::new(currency.to_owned())?,
                        }),
                        None => None,
                    };
                    Ok::<_, RuntimeError>(AttemptUsage {
                        input_units: usage.input_units(),
                        output_units: usage.output_units(),
                        duration_ms: usage.duration_ms(),
                        cost,
                    })
                })
                .transpose()?;
            plan.expected_controller_revision = Some((
                account.declaration().account().clone(),
                account.revision_digest().clone(),
            ));
            plan.controller_actions
                .push(ControllerAccountAction::SettleTerminal {
                    account: account.declaration().account().clone(),
                    reservation,
                    usage,
                });
        }
        Ok(plan)
    }

    fn plan_terminal_report(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        attempt: &AttemptId,
        report_sequence: u64,
        terminal: &InvocationTerminal,
    ) -> Result<CommandPlan, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        let expected = self.next_report_sequence(projection, attempt)?;
        if report_sequence != expected || attempt_view.is_completed() {
            return Err(RuntimeError::InvalidTransition(format!(
                "terminal report sequence must be exactly {expected} and attempt must be active"
            )));
        }
        let classified = attempt_view.side_effect().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "terminal attempt has no side-effect classification".to_owned(),
            )
        })?;
        if terminal.side_effect() > classified.side_effect() {
            return Err(RuntimeError::InvalidTransition(
                "terminal observation exceeds the frozen side-effect classification".to_owned(),
            ));
        }
        let published: BTreeSet<_> = attempt_view
            .outputs()
            .iter()
            .filter_map(|output| output.artifact())
            .map(|artifact| artifact.digest().to_hex())
            .collect();
        if terminal
            .outputs()
            .iter()
            .any(|output| !published.contains(output.digest()))
        {
            return Err(RuntimeError::InvalidTransition(
                "terminal output was not first published as a durable workspace artifact"
                    .to_owned(),
            ));
        }
        let mut plan = CommandPlan::default();
        let terminal_usage = terminal
            .usage()
            .map(|usage| {
                let cost = match usage.cost_micros().zip(usage.currency()) {
                    Some((micros, currency)) => Some(milkdrift_persistence::MonetaryUsage {
                        micros,
                        currency: CurrencyCode::new(currency.to_owned())?,
                    }),
                    None => None,
                };
                Ok::<_, RuntimeError>(AttemptUsage {
                    input_units: usage.input_units(),
                    output_units: usage.output_units(),
                    duration_ms: usage.duration_ms(),
                    cost,
                })
            })
            .transpose()?;
        if let Some(usage) = terminal_usage.as_ref() {
            plan.events.push(RunEventKind::AttemptUsageRecorded {
                attempt: attempt.clone(),
                usage: usage.clone(),
            });
        }
        if terminal.status() == TerminalStatus::Uncertain {
            plan.events.push(RunEventKind::ExternalOutcomeUncertain {
                attempt: attempt.clone(),
                report_sequence,
                side_effect: classified.side_effect(),
                reason: Reason::new(terminal.failure().map_or(
                    "executor reported an uncertain external outcome",
                    |failure| failure.message(),
                ))?,
                evidence: document.evidence().to_vec(),
            });
            return Ok(plan);
        }
        if let Some(account) = self.controller_account_for_run(document.run_id())? {
            let reservation = milkdrift_persistence::ControllerReservationId::for_attempt(
                account.declaration().account(),
                attempt,
            )?;
            if !account.reservations().contains_key(&reservation) {
                return Err(RuntimeError::InvalidHistory(
                    "controlled terminal observation has no exact outstanding reservation"
                        .to_owned(),
                ));
            }
            plan.expected_controller_revision = Some((
                account.declaration().account().clone(),
                account.revision_digest().clone(),
            ));
            plan.controller_actions
                .push(ControllerAccountAction::SettleTerminal {
                    account: account.declaration().account().clone(),
                    reservation,
                    usage: terminal_usage.clone(),
                });
        }
        let (outcome, error_class, detail) = match terminal.status() {
            TerminalStatus::Success => (NodeOutcome::Succeeded, None, None),
            TerminalStatus::Cancelled => (NodeOutcome::Cancelled, None, None),
            TerminalStatus::Failure | TerminalStatus::Rejected => {
                let failure = terminal.failure().ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "failed terminal report lacks details".to_owned(),
                    )
                })?;
                (
                    if terminal.status() == TerminalStatus::Rejected {
                        NodeOutcome::Rejected
                    } else {
                        NodeOutcome::Failed
                    },
                    Some(failure.class()),
                    Some(BoundedDetail::new(failure.message().to_owned())?),
                )
            }
            TerminalStatus::Uncertain => {
                return Err(RuntimeError::InvalidHistory(
                    "uncertain terminal routing failure".to_owned(),
                ));
            }
        };
        plan.events.push(RunEventKind::NodeTerminal {
            execution: attempt_view.execution().clone(),
            attempt: attempt.clone(),
            report_sequence,
            outcome,
            error_class,
            detail,
        });
        if let Some(failure) = terminal.failure()
            && self.config.retry_policy.permits_automatic_retry(
                attempt_view.attempt_number(),
                failure.class(),
                failure.retryable(),
                classified.side_effect(),
                classified.idempotency(),
                classified.idempotency_key(),
            )
        {
            match self.build_retry_event(
                attempt_view.execution(),
                attempt,
                attempt_view.attempt_number(),
                document.issued_at(),
                failure.class(),
                failure.retry_after_ms(),
                "bounded automatic retry policy admitted another attempt",
            ) {
                Ok(retry) => plan.events.push(retry),
                Err(error) => warn!(
                    attempt = %attempt,
                    reason = %error,
                    "truthful terminal report retained without an out-of-policy retry timer"
                ),
            }
        }
        Ok(plan)
    }

    #[allow(clippy::too_many_arguments)] // Execution identity, prior attempt/number, observation time, failure class, retry hint, and rationale are independently durable retry-policy inputs.
    pub(in crate::engine) fn build_retry_event(
        &self,
        execution: &NodeExecutionId,
        previous_attempt: &AttemptId,
        previous_attempt_number: u32,
        observed_at: TimestampMillis,
        error_class: ErrorClass,
        retry_after_ms: Option<u64>,
        rationale: &'static str,
    ) -> Result<RunEventKind, RuntimeError> {
        let attempt_number = previous_attempt_number
            .checked_add(1)
            .ok_or_else(|| RuntimeError::Scheduling("attempt number overflow".to_owned()))?;
        let delay = self
            .config
            .retry_policy
            .retry_delay_ms(attempt_number, 0, retry_after_ms)?;
        let fire_at = checked_timestamp_add(observed_at, delay)?;
        let reason = Reason::new(rationale)?;
        let next_attempt = self.next_attempt_id()?;
        let timer = self.next_timer_id()?;
        Ok(RunEventKind::NodeRetryScheduled {
            execution: execution.clone(),
            previous_attempt: previous_attempt.clone(),
            next_attempt,
            attempt_number,
            timer,
            fire_at,
            error_class,
            reason,
        })
    }

    fn resolve_executor_artifact(
        &self,
        reference: &milkdrift_capability::ArtifactReference,
    ) -> Result<(milkdrift_workspace::ArtifactMetadata, ArtifactReference), RuntimeError> {
        let artifact_id = ArtifactId::new(reference.identity().to_owned())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let metadata = self.store.metadata(&artifact_id)?.ok_or_else(|| {
            RuntimeError::InvalidTransition(format!(
                "executor artifact {} has no committed metadata",
                reference.identity()
            ))
        })?;
        let durable = metadata.reference().clone();
        let media_matches = reference
            .media_type()
            .is_none_or(|media| media == durable.media_type().as_str());
        let size_matches = reference
            .size_bytes()
            .is_none_or(|size| size == durable.size_bytes());
        if reference.digest() != durable.digest().to_hex()
            || !media_matches
            || !size_matches
            || !self.store.is_committed(&durable)?
        {
            return Err(RuntimeError::InvalidTransition(
                "executor artifact reference differs from committed content metadata".to_owned(),
            ));
        }
        Ok((metadata, durable))
    }

    pub(in crate::engine) fn next_report_sequence(
        &self,
        projection: &RunProjection,
        attempt: &AttemptId,
    ) -> Result<u64, RuntimeError> {
        projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidHistory("attempt is absent".to_owned()))?
            .last_report_sequence()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| RuntimeError::InvalidTransition("report sequence overflow".to_owned()))
    }
}
