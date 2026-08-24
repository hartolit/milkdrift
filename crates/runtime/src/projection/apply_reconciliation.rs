use milkdrift_capability::{IdempotencyBehavior, SideEffectClass};
use milkdrift_persistence::{
    AuthorityDecision, NodeOutcome, ReconciliationAction, ReconciliationClassification,
    RecoveryClassification, RunEventEnvelope, RunEventKind,
};

use crate::RuntimeError;

use super::helpers::invalid_at;
use super::node::{
    AttemptState, LeaseState, NodeExecutionCancellationProjection, NodeExecutionProjection,
    NodeExecutionState,
};
use super::reconciliation::{
    ReconciliationCancellationProjection, ReconciliationDecision, ReconciliationPlanProjection,
    ReconciliationRemediationProjection, ReconciliationRequestProjection,
    ReconciliationRequestState, RecoveryDecision, RecoveryObservation, RecoveryProjection,
    RemediationProjection,
};
use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_reconciliation_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::RevisionAdoptionRequested {
                reconciliation,
                from_revision,
                to_revision,
                policy,
            } => {
                if self.reconciliation.requests.contains_key(reconciliation)
                    || self.revision.as_ref() != Some(from_revision)
                    || from_revision == to_revision
                    || self.reconciliation.is_active()
                {
                    return Err(invalid_at(
                        event,
                        "revision adoption request is duplicate, stale, or conflicts with an active request",
                    ));
                }
                self.reconciliation.requests.insert(
                    reconciliation.clone(),
                    ReconciliationRequestProjection {
                        reconciliation: reconciliation.clone(),
                        from_revision: from_revision.clone(),
                        to_revision: to_revision.clone(),
                        policy: *policy,
                        sequence,
                        plan: None,
                        state: ReconciliationRequestState::Requested,
                    },
                );
                self.reconciliation.current_request = Some(reconciliation.clone());
            }
            RunEventKind::ReconciliationPlanRecorded {
                reconciliation,
                plan,
                from_revision,
                to_revision,
                based_on_sequence,
                items,
            } => {
                let request = self
                    .reconciliation
                    .requests
                    .get(reconciliation)
                    .ok_or_else(|| {
                        invalid_at(event, "plan references an unknown adoption request")
                    })?;
                if request.state != ReconciliationRequestState::Requested
                    || request.from_revision != *from_revision
                    || request.to_revision != *to_revision
                    || self.reconciliation.plans.contains_key(plan)
                    || request.sequence.get().checked_sub(1) != Some(based_on_sequence.get())
                    || self.sequence != request.sequence
                    || self.revision_at(*based_on_sequence) != Some(from_revision)
                {
                    return Err(invalid_at(
                        event,
                        "reconciliation plan is duplicate, stale, or differs from its request",
                    ));
                }
                for item in items {
                    if !crate::reconciliation::reconciliation_action_is_valid(
                        item.classification,
                        item.action,
                        request.policy,
                    ) {
                        return Err(invalid_at(
                            event,
                            "reconciliation action contradicts its classification or requested policy",
                        ));
                    }
                    if item.node.is_none()
                        && item.execution.is_none()
                        && item.classification
                            != ReconciliationClassification::IncompatibleInterfaceOrSubworkflow
                    {
                        return Err(invalid_at(
                            event,
                            "reconciliation item must name a node or execution",
                        ));
                    }
                    if let Some(execution) = &item.execution {
                        let execution_view = self.execution(execution, event)?;
                        if item
                            .node
                            .as_ref()
                            .is_some_and(|node| node != &execution_view.node)
                        {
                            return Err(invalid_at(
                                event,
                                "reconciliation item node and execution disagree",
                            ));
                        }
                    }
                }
                self.reconciliation.plans.insert(
                    plan.clone(),
                    ReconciliationPlanProjection {
                        reconciliation: reconciliation.clone(),
                        plan: plan.clone(),
                        from_revision: from_revision.clone(),
                        to_revision: to_revision.clone(),
                        based_on_sequence: *based_on_sequence,
                        items: items.clone(),
                        decisions: Vec::new(),
                        applied_sequence: None,
                        stale_sequence: None,
                    },
                );
                let request = self
                    .reconciliation
                    .requests
                    .get_mut(reconciliation)
                    .ok_or_else(|| invalid_at(event, "unknown reconciliation request"))?;
                request.plan = Some(plan.clone());
                request.state = ReconciliationRequestState::Planned;
            }
            RunEventKind::ReconciliationDecisionRecorded {
                plan,
                decision,
                actor,
                outcome,
                reason,
                evidence,
            } => {
                if !matches!(
                    outcome,
                    AuthorityDecision::Approve | AuthorityDecision::Reject
                ) || self
                    .reconciliation
                    .plans
                    .values()
                    .flat_map(|plan| plan.decisions.iter())
                    .any(|recorded| recorded.decision == *decision)
                {
                    return Err(invalid_at(
                        event,
                        "reconciliation decision outcome or identity is invalid",
                    ));
                }
                let plan_view = self
                    .reconciliation
                    .plans
                    .get_mut(plan)
                    .ok_or_else(|| invalid_at(event, "decision references an unknown plan"))?;
                if plan_view.applied_sequence.is_some()
                    || plan_view.stale_sequence.is_some()
                    || !plan_view.decisions.is_empty()
                {
                    return Err(invalid_at(
                        event,
                        "decision follows application/staleness or contradicts a prior decision",
                    ));
                }
                plan_view.decisions.push(ReconciliationDecision {
                    decision: decision.clone(),
                    actor: actor.clone(),
                    outcome: *outcome,
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    sequence,
                });
                if *outcome == AuthorityDecision::Reject {
                    self.reconciliation
                        .requests
                        .get_mut(&plan_view.reconciliation)
                        .ok_or_else(|| invalid_at(event, "plan request is missing"))?
                        .state = ReconciliationRequestState::Rejected;
                }
            }
            RunEventKind::ReconciliationExecutionRemoved { plan, execution } => {
                let plan_view = self
                    .reconciliation
                    .plans
                    .get(plan)
                    .ok_or_else(|| invalid_at(event, "removal references an unknown plan"))?;
                let authorized = plan_view.stale_sequence.is_none()
                    && plan_view.applied_sequence.is_none()
                    && plan_view.items.iter().any(|item| {
                        item.execution.as_ref() == Some(execution)
                            && (item.action == ReconciliationAction::RemoveUnstarted
                                || item.action == ReconciliationAction::UseNewOnNextInvocation
                                    && item.classification
                                        == ReconciliationClassification::ChangedPending)
                    });
                let execution_view = self.execution(execution, event)?;
                if !authorized
                    || execution_view.state != NodeExecutionState::Eligible
                    || !execution_view.attempts.is_empty()
                    || self.execution_has_active_structured_ownership(execution)
                {
                    return Err(invalid_at(
                        event,
                        "prospective removal is unauthorized or the execution already started",
                    ));
                }
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::RemovedProspectively(plan.clone());
                self.eligible_executions.remove(execution);
                self.deactivate_execution(execution, event)?;
            }
            RunEventKind::ReconciliationCancellationRequested {
                plan,
                execution,
                attempt,
                reason,
            } => {
                let plan_view =
                    self.reconciliation.plans.get(plan).ok_or_else(|| {
                        invalid_at(event, "cancellation references an unknown plan")
                    })?;
                let authorized = plan_view.stale_sequence.is_none()
                    && plan_view.applied_sequence.is_none()
                    && plan_view.items.iter().any(|item| {
                        item.execution.as_ref() == Some(execution)
                            && item.action == ReconciliationAction::CancelAndRestart
                    });
                let execution_view = self.execution(execution, event)?;
                let attempt_view = self.attempt(attempt, event)?;
                if !authorized
                    || self.reconciliation_cancellations.contains_key(execution)
                    || execution_view.cancellation.is_some()
                    || attempt_view.execution != *execution
                    || execution_view.attempts.last() != Some(attempt)
                    || !matches!(
                        attempt_view.state,
                        AttemptState::Scheduled | AttemptState::Leased | AttemptState::Running
                    )
                {
                    return Err(invalid_at(
                        event,
                        "reconciliation cancellation is duplicate, unauthorized, or not active",
                    ));
                }
                self.reconciliation_cancellations.insert(
                    execution.clone(),
                    ReconciliationCancellationProjection {
                        plan: plan.clone(),
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        reason: reason.clone(),
                        sequence,
                    },
                );
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown reconciliation execution"))?
                    .cancellation = Some(NodeExecutionCancellationProjection {
                    attempt: Some(attempt.clone()),
                    reason: reason.clone(),
                    sequence,
                });
                let source = self.execution(execution, event)?;
                let restart_scope = source.scope.clone();
                let restart_key = (source.node.clone(), restart_scope.clone());
                if self
                    .pending_reconciliation_restarts
                    .insert(restart_key, execution.clone())
                    .is_some()
                {
                    return Err(invalid_at(
                        event,
                        "reconciliation restart token is duplicate",
                    ));
                }
                self.adjust_scope_ownership(&restart_scope, true, event)?;
            }
            RunEventKind::ReconciliationRemediationCreated {
                plan,
                source_execution,
                source_attempt,
                execution,
                node,
                scope,
                mode,
                reason,
            } => {
                let plan_view =
                    self.reconciliation.plans.get(plan).ok_or_else(|| {
                        invalid_at(event, "remediation references an unknown plan")
                    })?;
                let authorized = plan_view.stale_sequence.is_none()
                    && plan_view.applied_sequence.is_none()
                    && plan_view.items.iter().any(|item| {
                        item.execution.as_ref() == Some(source_execution)
                            && item.node.as_ref() == Some(node)
                            && item.action == ReconciliationAction::CompensateOrRemediate
                    });
                let source = self.execution(source_execution, event)?;
                if !authorized
                    || self.node_executions.contains_key(execution)
                    || self.reserved_executions.contains(execution)
                    || self.reconciliation_remediations.contains_key(execution)
                    || source_attempt.as_ref().is_some_and(|attempt| {
                        self.attempts
                            .get(attempt)
                            .is_none_or(|attempt| attempt.execution != *source_execution)
                            || !source.attempts.contains(attempt)
                    })
                {
                    return Err(invalid_at(
                        event,
                        "reconciliation remediation is duplicate, unauthorized, or mismatched",
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
                self.reconciliation_remediations.insert(
                    execution.clone(),
                    ReconciliationRemediationProjection {
                        plan: plan.clone(),
                        source_execution: source_execution.clone(),
                        source_attempt: source_attempt.clone(),
                        execution: execution.clone(),
                        node: node.clone(),
                        scope: scope.clone(),
                        reason: reason.clone(),
                        sequence,
                    },
                );
            }
            RunEventKind::ReconciliationApplied {
                plan,
                from_revision,
                to_revision,
                based_on_sequence,
            } => {
                if *based_on_sequence != self.sequence
                    || self.revision.as_ref() != Some(from_revision)
                {
                    return Err(invalid_at(event, "reconciliation application is stale"));
                }
                let plan_view =
                    self.reconciliation.plans.get(plan).ok_or_else(|| {
                        invalid_at(event, "application references an unknown plan")
                    })?;
                let request_state = self
                    .reconciliation
                    .requests
                    .get(&plan_view.reconciliation)
                    .map(|request| request.state);
                let needs_authority = plan_view
                    .items
                    .iter()
                    .any(|item| item.action == ReconciliationAction::RequireAuthority);
                let approved = plan_view
                    .decisions
                    .last()
                    .is_some_and(|decision| decision.outcome == AuthorityDecision::Approve);
                let rejected_action = plan_view
                    .items
                    .iter()
                    .any(|item| item.action == ReconciliationAction::RejectRetrospectiveRewrite);
                let actions_enacted = plan_view.items.iter().all(|item| match item.action {
                    ReconciliationAction::RemoveUnstarted => {
                        item.execution.as_ref().is_none_or(|execution| {
                            self.node_executions
                                .get(execution)
                                .is_some_and(|execution| {
                                    execution.state
                                        == NodeExecutionState::RemovedProspectively(plan.clone())
                                })
                        })
                    }
                    ReconciliationAction::UseNewOnNextInvocation
                        if item.classification == ReconciliationClassification::ChangedPending =>
                    {
                        item.execution.as_ref().is_none_or(|execution| {
                            self.node_executions
                                .get(execution)
                                .is_some_and(|execution| {
                                    execution.state
                                        == NodeExecutionState::RemovedProspectively(plan.clone())
                                })
                        })
                    }
                    ReconciliationAction::CancelAndRestart => {
                        item.execution.as_ref().is_some_and(|execution| {
                            self.reconciliation_cancellations
                                .get(execution)
                                .is_some_and(|cancellation| cancellation.plan == *plan)
                        })
                    }
                    ReconciliationAction::CompensateOrRemediate => {
                        item.execution.as_ref().is_some_and(|source| {
                            self.reconciliation_remediations
                                .values()
                                .any(|remediation| {
                                    remediation.plan == *plan
                                        && remediation.source_execution == *source
                                })
                        })
                    }
                    ReconciliationAction::Preserve
                    | ReconciliationAction::UseNewOnNextInvocation
                    | ReconciliationAction::RequireAuthority
                    | ReconciliationAction::RejectRetrospectiveRewrite => true,
                });
                if plan_view.from_revision != *from_revision
                    || plan_view.to_revision != *to_revision
                    || plan_view.applied_sequence.is_some()
                    || plan_view.stale_sequence.is_some()
                    || self.reconciliation.current_request.as_ref()
                        != Some(&plan_view.reconciliation)
                    || request_state != Some(ReconciliationRequestState::Planned)
                    || (needs_authority && !approved)
                    || !actions_enacted
                    || rejected_action
                    || plan_view
                        .decisions
                        .last()
                        .is_some_and(|decision| decision.outcome == AuthorityDecision::Reject)
                {
                    return Err(invalid_at(
                        event,
                        "plan is mismatched, already applied, or lacks authority",
                    ));
                }
                let reconciliation = plan_view.reconciliation.clone();
                self.reconciliation
                    .plans
                    .get_mut(plan)
                    .ok_or_else(|| invalid_at(event, "unknown plan"))?
                    .applied_sequence = Some(sequence);
                self.reconciliation
                    .requests
                    .get_mut(&reconciliation)
                    .ok_or_else(|| invalid_at(event, "plan request is missing"))?
                    .state = ReconciliationRequestState::Applied;
                self.pending_pin = Some(plan.clone());
            }
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
            _ => {
                return Err(invalid_at(
                    event,
                    "internal reconciliation event routing failure",
                ));
            }
        }
        Ok(())
    }
}
