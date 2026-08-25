use milkdrift_capability::{IdempotencyBehavior, SideEffectClass};
use milkdrift_persistence::{AuthorityDecision, NodeOutcome, RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::helpers::{invalid_at, new_attempt};
use super::node::{
    AttemptState, ExternalOutcomeObligation, LateTerminalEvidence, NodeExecutionState,
    RetainedExternalOutcome, RetryProjection, RetryState, TimerProjection, TimerPurpose,
    TimerState,
};
use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_retry_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::NodeRetryScheduled {
                execution,
                previous_attempt,
                next_attempt,
                attempt_number,
                timer,
                fire_at,
                error_class,
                reason,
            } => {
                if self.attempts.contains_key(next_attempt)
                    || self.timers.contains_key(timer)
                    || *fire_at < event.occurred_at()
                {
                    return Err(invalid_at(
                        event,
                        "retry identities are duplicate or deadline is in the past",
                    ));
                }
                let previous = self.attempt(previous_attempt, event)?;
                let retry_safe = previous.side_effect.as_ref().is_some_and(|classification| {
                    matches!(
                        classification.side_effect,
                        SideEffectClass::None | SideEffectClass::ReadOnly
                    ) || (classification.side_effect == SideEffectClass::IdempotentWrite
                        && classification.idempotency != IdempotencyBehavior::Unsupported
                        && classification.idempotency_key.is_some())
                });
                let retryable_terminal = matches!(
                    previous.state,
                    AttemptState::Terminal(NodeOutcome::Failed | NodeOutcome::Rejected)
                ) && previous
                    .terminal
                    .as_ref()
                    .is_some_and(|terminal| terminal.error_class == Some(*error_class))
                    && retry_safe;
                let retryable_uncertain = previous.state == AttemptState::Uncertain
                    && previous.obligation.as_ref().is_some_and(|obligation| {
                        obligation.side_effect
                            == previous
                                .side_effect
                                .as_ref()
                                .map_or(SideEffectClass::Unknown, |facts| facts.side_effect)
                    })
                    && retry_safe;
                let authority_retry = previous.obligation.as_ref().is_some_and(|obligation| {
                    obligation
                        .decisions
                        .last()
                        .is_some_and(|decision| decision.outcome == AuthorityDecision::Retry)
                }) && retry_safe;
                let execution_view = self.execution(execution, event)?;
                let expected_number = execution_view.attempt_count.checked_add(1);
                if previous.execution != *execution
                    || execution_view.attempts.last() != Some(previous_attempt)
                    || expected_number != Some(*attempt_number)
                    || *attempt_number > crate::scheduler::MAX_RETRY_ATTEMPTS
                    || (!retryable_terminal && !retryable_uncertain && !authority_retry)
                {
                    return Err(invalid_at(
                        event,
                        "retry does not follow the latest retryable attempt",
                    ));
                }
                self.attempts.insert(
                    next_attempt.clone(),
                    new_attempt(
                        next_attempt.clone(),
                        execution.clone(),
                        *attempt_number,
                        AttemptState::AwaitingRetryTimer,
                    ),
                );
                self.active_attempt_ids.insert(next_attempt.clone());
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .attempts
                    .push(next_attempt.clone());
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .attempt_count = *attempt_number;
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::RetryPending(next_attempt.clone());
                if !self.active_execution_ids.contains(execution) {
                    self.activate_execution(execution, event)?;
                }
                self.timers.insert(
                    timer.clone(),
                    TimerProjection {
                        timer: timer.clone(),
                        purpose: TimerPurpose::Retry {
                            attempt: next_attempt.clone(),
                        },
                        fire_at: *fire_at,
                        state: TimerState::Pending,
                        cancellation: None,
                    },
                );
                self.pending_timer_ids.insert(timer.clone());
                self.pending_timers_by_execution
                    .entry(execution.clone())
                    .or_default()
                    .insert(timer.clone());
                self.retries.insert(
                    timer.clone(),
                    RetryProjection {
                        execution: execution.clone(),
                        previous_attempt: previous_attempt.clone(),
                        next_attempt: next_attempt.clone(),
                        attempt_number: *attempt_number,
                        timer: timer.clone(),
                        fire_at: *fire_at,
                        error_class: *error_class,
                        reason: reason.clone(),
                        state: RetryState::Waiting,
                    },
                );
                self.retry_by_attempt
                    .insert(next_attempt.clone(), timer.clone());
                if retryable_uncertain {
                    self.complete_attempt_leases(previous_attempt);
                }
            }
            RunEventKind::ExternalOutcomeUncertain {
                attempt,
                report_sequence,
                side_effect,
                reason,
                evidence,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let classified = attempt_view.side_effect.as_ref().ok_or_else(|| {
                    invalid_at(event, "uncertainty lacks frozen side-effect facts")
                })?;
                if !matches!(
                    attempt_view.state,
                    AttemptState::Leased | AttemptState::Running
                ) || attempt_view.obligation.is_some()
                    || !attempt_view.expects_report_sequence(*report_sequence)
                    || classified.side_effect != *side_effect
                {
                    return Err(invalid_at(
                        event,
                        "uncertain outcome is duplicate or contradicts dispatch facts",
                    ));
                }
                let execution = attempt_view.execution.clone();
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.state = AttemptState::Uncertain;
                attempt_view.last_report_sequence = Some(*report_sequence);
                attempt_view.obligation = Some(ExternalOutcomeObligation {
                    report_sequence: *report_sequence,
                    side_effect: *side_effect,
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    uncertain_sequence: sequence,
                    retained: None,
                    decisions: Vec::new(),
                });
                self.node_executions
                    .get_mut(&execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::Uncertain(attempt.clone());
                self.complete_attempt_leases(attempt);
            }
            RunEventKind::LateTerminalEvidenceRecorded {
                attempt,
                worker,
                report_sequence,
                terminal,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let obligation = attempt_view.obligation.as_ref().ok_or_else(|| {
                    invalid_at(
                        event,
                        "late terminal evidence requires an uncertainty obligation",
                    )
                })?;
                let classified = attempt_view.side_effect.as_ref().ok_or_else(|| {
                    invalid_at(
                        event,
                        "late terminal evidence lacks side-effect classification",
                    )
                })?;
                let historically_owned = attempt_view.lease_workers.contains(worker);
                if attempt_view.terminal.is_some()
                    || attempt_view.late_terminal_evidence.is_some()
                    || *report_sequence < obligation.report_sequence
                    || terminal.side_effect() > classified.side_effect
                    || !historically_owned
                {
                    return Err(invalid_at(
                        event,
                        "late terminal evidence contradicts attempt ownership or existing terminal facts",
                    ));
                }
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .late_terminal_evidence = Some(LateTerminalEvidence {
                    report_sequence: *report_sequence,
                    terminal: terminal.clone(),
                    worker: worker.clone(),
                    sequence,
                });
            }
            RunEventKind::ExternalOutcomeRetained {
                attempt,
                decision,
                reason,
            } => {
                let decision_view = self.recovery_decisions.get(decision);
                if decision_view != Some(&(attempt.clone(), AuthorityDecision::Retain)) {
                    return Err(invalid_at(
                        event,
                        "retention lacks its prior matching authority decision",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "retention references an unknown attempt"))?;
                let obligation = attempt_view.obligation.as_mut().ok_or_else(|| {
                    invalid_at(event, "retention requires an uncertain obligation")
                })?;
                if obligation.retained.is_some() {
                    return Err(invalid_at(event, "external outcome was already retained"));
                }
                obligation.retained = Some(RetainedExternalOutcome {
                    decision: decision.clone(),
                    reason: reason.clone(),
                    sequence,
                });
                attempt_view.state = AttemptState::Retained;
                self.complete_attempt_leases(attempt);
            }
            _ => unreachable!("central projection dispatch owns retry and uncertainty routing"),
        }
        Ok(())
    }
}
