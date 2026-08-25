use milkdrift_capability::SideEffectClass;
use milkdrift_persistence::{NodeOutcome, RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::helpers::invalid_at;
use super::node::{
    AttemptState, NodeExecutionProjection, NodeExecutionState, RetryState,
    TimerCancellationProjection, TimerProjection, TimerPurpose, TimerState,
};
use super::run::RunProjection;
use super::structured::WaitProjection;

impl RunProjection {
    pub(super) fn apply_timer_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::TimerRegistered {
                timer,
                execution,
                fire_at,
            } => {
                if self.timers.contains_key(timer) || *fire_at < event.occurred_at() {
                    return Err(invalid_at(
                        event,
                        "timer identity is duplicate or deadline is in the past",
                    ));
                }
                if let Some(execution) = execution {
                    self.execution(execution, event)?;
                }
                self.timers.insert(
                    timer.clone(),
                    TimerProjection {
                        timer: timer.clone(),
                        purpose: TimerPurpose::Wait {
                            execution: execution.clone(),
                        },
                        fire_at: *fire_at,
                        state: TimerState::Pending,
                        cancellation: None,
                    },
                );
                self.pending_timer_ids.insert(timer.clone());
                if let Some(execution) = execution {
                    self.pending_timers_by_execution
                        .entry(execution.clone())
                        .or_default()
                        .insert(timer.clone());
                }
            }
            RunEventKind::TimerFired { timer, observed_at } => {
                let timer_view = self
                    .timers
                    .get_mut(timer)
                    .ok_or_else(|| invalid_at(event, "timer firing references an unknown timer"))?;
                if !timer_view.is_pending() || *observed_at < timer_view.fire_at {
                    return Err(invalid_at(
                        event,
                        "timer fired twice or before its deadline",
                    ));
                }
                let purpose = timer_view.purpose.clone();
                timer_view.state = TimerState::Fired {
                    observed_at: *observed_at,
                };
                self.pending_timer_ids.remove(timer);
                if let Some(retry) = self.retries.get_mut(timer) {
                    retry.state = RetryState::Ready;
                    self.attempts
                        .get_mut(&retry.next_attempt)
                        .ok_or_else(|| invalid_at(event, "retry timer has no reserved attempt"))?
                        .state = AttemptState::ReadyToSchedule;
                }
                self.remove_pending_timer_owner(timer, &purpose, event)?;
            }
            RunEventKind::TimerCancelled { timer, reason } => {
                let timer_view = self.timers.get(timer).ok_or_else(|| {
                    invalid_at(event, "timer cancellation references an unknown timer")
                })?;
                if !timer_view.is_pending() {
                    return Err(invalid_at(event, "only a pending timer may be cancelled"));
                }
                let purpose = timer_view.purpose.clone();
                let authorized = match &purpose {
                    TimerPurpose::Wait {
                        execution: Some(execution),
                    } => {
                        self.waits
                            .get(execution)
                            .and_then(WaitProjection::cancellation)
                            .is_some()
                            || self
                                .node_executions
                                .get(execution)
                                .and_then(NodeExecutionProjection::cancellation)
                                .is_some()
                            || self.has_execution_cancellation_source(execution)
                    }
                    TimerPurpose::Wait { execution: None } => self.cancellation.is_some(),
                    TimerPurpose::Retry { attempt } => {
                        let attempt_view = self.attempt(attempt, event)?;
                        attempt_view.state == AttemptState::AwaitingRetryTimer
                            && self.has_execution_cancellation_source(&attempt_view.execution)
                    }
                };
                if !authorized {
                    return Err(invalid_at(
                        event,
                        "timer cancellation lacks a structured owner cancellation fact",
                    ));
                }
                let timer_view = self
                    .timers
                    .get_mut(timer)
                    .ok_or_else(|| invalid_at(event, "unknown timer"))?;
                timer_view.state = TimerState::Cancelled;
                timer_view.cancellation = Some(TimerCancellationProjection {
                    reason: reason.clone(),
                    sequence,
                });
                self.pending_timer_ids.remove(timer);
                self.remove_pending_timer_owner(timer, &purpose, event)?;
                if let TimerPurpose::Retry { attempt } = purpose {
                    let retry = self
                        .retries
                        .get(timer)
                        .ok_or_else(|| invalid_at(event, "retry timer has no retry decision"))?;
                    if retry.next_attempt != attempt || retry.state != RetryState::Waiting {
                        return Err(invalid_at(
                            event,
                            "retry timer cancellation contradicts its retry decision",
                        ));
                    }
                    let retry_execution = retry.execution.clone();
                    let harmless_uncertain = self
                        .node_executions
                        .get(&retry_execution)
                        .into_iter()
                        .flat_map(|execution| execution.attempts.iter())
                        .filter_map(|candidate| {
                            let prior = self.attempts.get(candidate)?;
                            (prior.state == AttemptState::Uncertain
                                && prior.side_effect.as_ref().is_some_and(|facts| {
                                    matches!(
                                        facts.side_effect,
                                        SideEffectClass::None | SideEffectClass::ReadOnly
                                    )
                                }))
                            .then(|| candidate.clone())
                        })
                        .collect::<Vec<_>>();
                    self.retries
                        .get_mut(timer)
                        .ok_or_else(|| invalid_at(event, "retry timer has no retry decision"))?
                        .state = RetryState::Cancelled;
                    self.attempts
                        .get_mut(&attempt)
                        .ok_or_else(|| invalid_at(event, "retry timer has no reserved attempt"))?
                        .state = AttemptState::CancelledBeforeDispatch;
                    self.active_attempt_ids.remove(&attempt);
                    self.node_executions
                        .get_mut(&retry_execution)
                        .ok_or_else(|| invalid_at(event, "retry has no owning execution"))?
                        .state = NodeExecutionState::Terminal(NodeOutcome::Cancelled);
                    self.deactivate_execution(&retry_execution, event)?;
                    for prior in harmless_uncertain {
                        self.attempts
                            .get_mut(&prior)
                            .ok_or_else(|| invalid_at(event, "uncertain prior attempt is missing"))?
                            .state = AttemptState::UncertainAbandonedByCancellation {
                            cancelled_retry: attempt.clone(),
                        };
                        self.active_attempt_ids.remove(&prior);
                        self.complete_attempt_leases(&prior);
                    }
                }
            }
            _ => unreachable!("structured dispatch owns timer ownership routing"),
        }
        Ok(())
    }
}
