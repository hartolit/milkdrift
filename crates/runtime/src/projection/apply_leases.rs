use milkdrift_capability::{IdempotencyBehavior, SideEffectClass};
use milkdrift_persistence::{RecoveryClassification, RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::helpers::invalid_at;
use super::node::{AttemptState, LeaseProjection, LeaseState, NodeExecutionState};
use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_lease_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let _sequence = event.sequence();
        match event.kind() {
            RunEventKind::LeaseGranted {
                lease,
                execution,
                attempt,
                worker,
                expires_at,
            } => {
                if self.leases.contains_key(lease) || *expires_at <= event.occurred_at() {
                    return Err(invalid_at(
                        event,
                        "lease identity is duplicate or expiration is not future",
                    ));
                }
                let attempt_view = self.attempt(attempt, event)?;
                if attempt_view.execution != *execution
                    || attempt_view.state != AttemptState::Scheduled
                    || attempt_view.capability.is_none()
                    || attempt_view.side_effect.is_none()
                    || self.active_lease_for_attempt(attempt).is_some()
                {
                    return Err(invalid_at(
                        event,
                        "lease grant is out of state or lacks dispatch facts",
                    ));
                }
                self.leases.insert(
                    lease.clone(),
                    LeaseProjection {
                        lease: lease.clone(),
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        worker: worker.clone(),
                        expires_at: *expires_at,
                        state: LeaseState::Active,
                    },
                );
                if self
                    .active_lease_by_attempt
                    .insert(attempt.clone(), lease.clone())
                    .is_some()
                {
                    return Err(invalid_at(
                        event,
                        "lease grant replaced an active attempt lease",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.leases.push(lease.clone());
                attempt_view.state = AttemptState::Leased;
            }
            RunEventKind::LeaseHeartbeatRecorded { lease, expires_at } => {
                let lease_view = self
                    .leases
                    .get_mut(lease)
                    .ok_or_else(|| invalid_at(event, "heartbeat references an unknown lease"))?;
                if !lease_view.is_active()
                    || event.occurred_at() >= lease_view.expires_at
                    || *expires_at <= lease_view.expires_at
                    || *expires_at <= event.occurred_at()
                {
                    return Err(invalid_at(
                        event,
                        "heartbeat requires a still-valid active lease and later expiration",
                    ));
                }
                lease_view.expires_at = *expires_at;
            }
            RunEventKind::LeaseExpired {
                lease,
                classification,
            } => {
                let lease_view = self
                    .leases
                    .get(lease)
                    .ok_or_else(|| invalid_at(event, "expiry references an unknown lease"))?;
                let lease_attempt = lease_view.attempt.clone();
                let attempt_view = self.attempt(&lease_attempt, event)?;
                let retry_safe = attempt_view
                    .side_effect
                    .as_ref()
                    .is_some_and(|classification| {
                        matches!(
                            classification.side_effect,
                            SideEffectClass::None | SideEffectClass::ReadOnly
                        ) || (classification.side_effect == SideEffectClass::IdempotentWrite
                            && classification.idempotency != IdempotencyBehavior::Unsupported
                            && classification.idempotency_key.is_some())
                    });
                let classification_valid = match classification {
                    RecoveryClassification::NotStarted => {
                        attempt_view.state == AttemptState::Leased
                    }
                    RecoveryClassification::Retryable => {
                        retry_safe
                            && matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            )
                    }
                    RecoveryClassification::Uncertain => {
                        !retry_safe
                            && matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            )
                    }
                    RecoveryClassification::LeaseStillValid
                    | RecoveryClassification::TerminalObserved => false,
                };
                if !lease_view.is_active()
                    || event.occurred_at() < lease_view.expires_at
                    || !classification_valid
                {
                    return Err(invalid_at(
                        event,
                        "lease expiry is early, duplicate, or contradicts immutable attempt facts",
                    ));
                }
                self.leases
                    .get_mut(lease)
                    .ok_or_else(|| invalid_at(event, "unknown lease"))?
                    .state = LeaseState::Expired(*classification);
                if self.active_lease_by_attempt.remove(&lease_attempt).as_ref() != Some(lease) {
                    return Err(invalid_at(
                        event,
                        "lease expiry disagrees with the active attempt lease",
                    ));
                }
            }
            RunEventKind::NodeReLeased {
                previous_lease,
                lease,
                attempt,
                worker,
                expires_at,
            } => {
                if self.leases.contains_key(lease) || *expires_at <= event.occurred_at() {
                    return Err(invalid_at(
                        event,
                        "replacement lease is duplicate or already expired",
                    ));
                }
                let prior = self.leases.get(previous_lease).ok_or_else(|| {
                    invalid_at(event, "replacement references an unknown prior lease")
                })?;
                let classification = match prior.state {
                    LeaseState::Expired(classification) => classification,
                    LeaseState::Active | LeaseState::Superseded(_) | LeaseState::Completed => {
                        return Err(invalid_at(event, "only an expired lease may be superseded"));
                    }
                };
                let execution = prior.execution.clone();
                let attempt_view = self.attempt(attempt, event)?;
                let execution_view = self.execution(&execution, event)?;
                let retry_safe = attempt_view
                    .side_effect
                    .as_ref()
                    .is_some_and(|classification| {
                        matches!(
                            classification.side_effect,
                            SideEffectClass::None | SideEffectClass::ReadOnly
                        ) || (classification.side_effect == SideEffectClass::IdempotentWrite
                            && classification.idempotency != IdempotencyBehavior::Unsupported
                            && classification.idempotency_key.is_some())
                    });
                let state_is_releasable = match classification {
                    RecoveryClassification::NotStarted => {
                        attempt_view.state == AttemptState::Leased
                    }
                    RecoveryClassification::Retryable => {
                        retry_safe
                            && matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            )
                    }
                    RecoveryClassification::LeaseStillValid
                    | RecoveryClassification::Uncertain
                    | RecoveryClassification::TerminalObserved => false,
                };
                let exact_recovery = attempt_view.recovery.last().is_some_and(|observation| {
                    observation.lease.as_ref() == Some(previous_lease)
                        && observation.classification == classification
                });
                if prior.attempt != *attempt
                    || attempt_view.leases.last() != Some(previous_lease)
                    || execution_view.attempts.last() != Some(attempt)
                    || execution_view.cancellation.is_some()
                    || !matches!(
                        execution_view.state,
                        NodeExecutionState::Scheduled(ref active)
                            | NodeExecutionState::Running(ref active)
                            if active == attempt
                    )
                    || !state_is_releasable
                    || !exact_recovery
                    || self.active_lease_for_attempt(attempt).is_some()
                {
                    return Err(invalid_at(
                        event,
                        "attempt is not safely eligible for re-lease",
                    ));
                }
                self.leases
                    .get_mut(previous_lease)
                    .ok_or_else(|| invalid_at(event, "unknown prior lease"))?
                    .state = LeaseState::Superseded(lease.clone());
                self.leases.insert(
                    lease.clone(),
                    LeaseProjection {
                        lease: lease.clone(),
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        worker: worker.clone(),
                        expires_at: *expires_at,
                        state: LeaseState::Active,
                    },
                );
                if self
                    .active_lease_by_attempt
                    .insert(attempt.clone(), lease.clone())
                    .is_some()
                {
                    return Err(invalid_at(
                        event,
                        "replacement lease displaced an active attempt lease",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.leases.push(lease.clone());
                attempt_view.state = AttemptState::Leased;
                self.node_executions
                    .get_mut(&execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::Scheduled(attempt.clone());
            }
            _ => unreachable!("central projection dispatch owns lease ownership routing"),
        }
        Ok(())
    }
}
