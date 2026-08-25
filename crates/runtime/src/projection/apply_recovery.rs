use milkdrift_capability::{IdempotencyBehavior, SideEffectClass};
use milkdrift_persistence::{
    AuthorityDecision, NodeOutcome, RecoveryClassification, RunEventEnvelope, RunEventKind,
};

use crate::RuntimeError;

use super::helpers::invalid_at;
use super::node::{AttemptState, LeaseState, NodeExecutionProjection, NodeExecutionState};
use super::reconciliation::{
    RecoveryDecision, RecoveryObservation, RecoveryProjection, RemediationProjection,
};
use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_recovery_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::RecoveryStarted {
                controller,
                through_sequence,
            } => {
                if *through_sequence != self.sequence {
                    return Err(invalid_at(
                        event,
                        "recovery must name the exact journal head examined",
                    ));
                }
                self.recovery.clear();
                self.recovery.push(RecoveryProjection {
                    controller: controller.clone(),
                    through_sequence: *through_sequence,
                    started_sequence: sequence,
                    classifications: Vec::new(),
                });
                self.current_recovery = self.recovery.len().checked_sub(1);
            }
            RunEventKind::RecoveryClassified {
                attempt,
                lease,
                classification,
                reason,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let lease_view = lease
                    .as_ref()
                    .map(|lease| {
                        self.leases.get(lease).ok_or_else(|| {
                            invalid_at(event, "recovery references an unknown lease")
                        })
                    })
                    .transpose()?;
                if lease_view.is_some_and(|lease| lease.attempt != *attempt) {
                    return Err(invalid_at(
                        event,
                        "recovery lease belongs to another attempt",
                    ));
                }
                let recovery_index = self.current_recovery.ok_or_else(|| {
                    invalid_at(event, "classification has no preceding recovery start")
                })?;
                if self.recovery.get(recovery_index).is_none_or(|recovery| {
                    recovery
                        .classifications
                        .iter()
                        .any(|(classified, _)| classified == attempt)
                }) {
                    return Err(invalid_at(
                        event,
                        "attempt was already classified in this recovery pass",
                    ));
                }
                let retry_safe = attempt_view
                    .side_effect
                    .as_ref()
                    .is_some_and(|side_effect| {
                        matches!(
                            side_effect.side_effect,
                            SideEffectClass::None | SideEffectClass::ReadOnly
                        ) || (side_effect.side_effect == SideEffectClass::IdempotentWrite
                            && side_effect.idempotency != IdempotencyBehavior::Unsupported
                            && side_effect.idempotency_key.is_some())
                    });
                let classification_valid = match classification {
                    RecoveryClassification::LeaseStillValid => lease_view.is_some_and(|lease| {
                        lease.is_active()
                            && lease.expires_at > event.occurred_at()
                            && matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            )
                    }),
                    RecoveryClassification::TerminalObserved => attempt_view.is_completed(),
                    RecoveryClassification::NotStarted => {
                        matches!(
                            attempt_view.state,
                            AttemptState::AwaitingRetryTimer
                                | AttemptState::ReadyToSchedule
                                | AttemptState::Scheduled
                                | AttemptState::Leased
                        ) && lease_view.is_none_or(|lease| {
                            matches!(
                                lease.state,
                                LeaseState::Expired(RecoveryClassification::NotStarted)
                            )
                        })
                    }
                    RecoveryClassification::Retryable => {
                        retry_safe
                            && (attempt_view.is_unresolved() && lease_view.is_none()
                                || matches!(
                                    attempt_view.state,
                                    AttemptState::Leased | AttemptState::Running
                                ) && lease_view.is_some_and(|lease| {
                                    matches!(
                                        lease.state,
                                        LeaseState::Expired(RecoveryClassification::Retryable)
                                    )
                                }))
                    }
                    RecoveryClassification::Uncertain => {
                        attempt_view.is_unresolved()
                            || (matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            ) && lease_view.is_some_and(|lease| {
                                matches!(
                                    lease.state,
                                    LeaseState::Expired(RecoveryClassification::Uncertain)
                                )
                            }) && !retry_safe)
                    }
                };
                if !classification_valid {
                    return Err(invalid_at(
                        event,
                        "recovery classification contradicts projected attempt state",
                    ));
                }
                let observation = RecoveryObservation {
                    lease: lease.clone(),
                    classification: *classification,
                    reason: reason.clone(),
                    sequence,
                };
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .recovery
                    .push(observation.clone());
                self.recovery
                    .get_mut(recovery_index)
                    .ok_or_else(|| invalid_at(event, "current recovery pass is missing"))?
                    .classifications
                    .push((attempt.clone(), observation));
            }
            RunEventKind::RecoveryDecisionRecorded {
                attempt,
                decision,
                actor,
                outcome,
                reason,
                evidence,
            } => {
                if !matches!(
                    outcome,
                    AuthorityDecision::Retain
                        | AuthorityDecision::Query
                        | AuthorityDecision::Retry
                        | AuthorityDecision::Compensate
                        | AuthorityDecision::ResolveSucceeded
                        | AuthorityDecision::ResolveFailed
                ) || matches!(
                    outcome,
                    AuthorityDecision::ResolveSucceeded | AuthorityDecision::ResolveFailed
                ) && evidence.is_empty()
                    || self.recovery_decisions.contains_key(decision)
                    || self.attempts.values().any(|attempt| {
                        attempt.obligation.as_ref().is_some_and(|obligation| {
                            obligation
                                .decisions
                                .iter()
                                .any(|recorded| recorded.decision == *decision)
                        })
                    })
                {
                    return Err(invalid_at(
                        event,
                        "recovery decision outcome or identity is invalid",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "decision references an unknown attempt"))?;
                let obligation = attempt_view.obligation.as_mut().ok_or_else(|| {
                    invalid_at(event, "decision requires uncertain or retained work")
                })?;
                obligation.decisions.push(RecoveryDecision {
                    decision: decision.clone(),
                    actor: actor.clone(),
                    outcome: *outcome,
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    sequence,
                });
                let execution = attempt_view.execution.clone();
                let resolved = match outcome {
                    AuthorityDecision::ResolveSucceeded => Some(NodeOutcome::Succeeded),
                    AuthorityDecision::ResolveFailed => Some(NodeOutcome::Failed),
                    AuthorityDecision::Retain
                    | AuthorityDecision::Query
                    | AuthorityDecision::Retry
                    | AuthorityDecision::Compensate => None,
                    AuthorityDecision::Approve | AuthorityDecision::Reject => None,
                };
                if let Some(outcome) = resolved {
                    attempt_view.state = AttemptState::Resolved(outcome);
                    attempt_view.obligation = None;
                    self.active_attempt_ids.remove(attempt);
                    self.node_executions
                        .get_mut(&execution)
                        .ok_or_else(|| invalid_at(event, "unknown execution"))?
                        .state = NodeExecutionState::Terminal(outcome);
                    self.deactivate_execution(&execution, event)?;
                    if outcome == NodeOutcome::Succeeded {
                        self.pending_successor_executions.insert(execution.clone());
                    }
                    self.complete_attempt_leases(attempt);
                }
                self.recovery_decisions
                    .insert(decision.clone(), (attempt.clone(), *outcome));
            }
            RunEventKind::RemediationWorkCreated {
                source_attempt,
                execution,
                node,
                scope,
                mode,
                decision,
                reason,
            } => {
                let source = self.attempt(source_attempt, event)?;
                let source_has_obligation = source.obligation.is_some();
                let source_execution = source.execution.clone();
                let source_scope = self.execution(&source_execution, event)?.scope.clone();
                if !source_has_obligation
                    || self.recovery_decisions.get(decision)
                        != Some(&(source_attempt.clone(), AuthorityDecision::Compensate))
                    || self.remediations.contains_key(execution)
                    || self.reserved_executions.contains(execution)
                    || self.node_executions.contains_key(execution)
                    || self.settled_node_executions.contains_key(execution)
                    || *scope != source_scope
                {
                    return Err(invalid_at(
                        event,
                        "remediation lacks authority or reuses an execution identity",
                    ));
                }
                self.validate_scope_reference(scope, event)?;
                self.node_executions.insert(
                    execution.clone(),
                    NodeExecutionProjection {
                        execution: execution.clone(),
                        node: node.clone(),
                        scope: scope.clone(),
                        mode: *mode,
                        revision: self
                            .revision
                            .clone()
                            .ok_or_else(|| invalid_at(event, "remediation has no revision"))?,
                        epoch_retired_sequence: None,
                        created_sequence: sequence,
                        created_at: event.occurred_at(),
                        attempts: Vec::new(),
                        attempt_count: 0,
                        state: NodeExecutionState::Eligible,
                        cancellation: None,
                        deterministic_terminal: None,
                        outputs: Vec::new(),
                    },
                );
                self.execution_ids_by_node
                    .entry(node.clone())
                    .or_default()
                    .insert(execution.clone());
                self.eligible_executions.insert(execution.clone());
                self.activate_execution(execution, event)?;
                self.remediations.insert(
                    execution.clone(),
                    RemediationProjection {
                        source_attempt: source_attempt.clone(),
                        execution: execution.clone(),
                        node: node.clone(),
                        scope: scope.clone(),
                        mode: *mode,
                        decision: decision.clone(),
                        reason: reason.clone(),
                        sequence,
                    },
                );
            }
            _ => unreachable!("reconciliation dispatch owns recovery and remediation routing"),
        }
        Ok(())
    }
}
