use milkdrift_persistence::{
    AuthorityDecision, ReconciliationAction, ReconciliationClassification, RunEventEnvelope,
    RunEventKind,
};

use crate::RuntimeError;

use super::helpers::invalid_at;
use super::node::{
    AttemptState, NodeExecutionCancellationProjection, NodeExecutionProjection, NodeExecutionState,
};
use super::reconciliation::{
    ReconciliationCancellationProjection, ReconciliationRemediationProjection,
    ReconciliationRequestState,
};
use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_reconciliation_action_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
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
                let authorized = self
                    .reconciliation
                    .plans
                    .get(plan)
                    .ok_or_else(|| invalid_at(event, "cancellation references an unknown plan"))
                    .map(|plan_view| {
                        plan_view.stale_sequence.is_none()
                            && plan_view.applied_sequence.is_none()
                            && plan_view.items.iter().any(|item| {
                                item.execution.as_ref() == Some(execution)
                                    && item.action == ReconciliationAction::CancelAndRestart
                            })
                    })?;
                let execution_view = self.execution(execution, event)?;
                let attempt_view = self.attempt(attempt, event)?;
                let before_dispatch = matches!(
                    attempt_view.state,
                    AttemptState::Scheduled | AttemptState::Leased
                );
                if !authorized
                    || !self.reconciliation_cancellation_is_safe(execution)
                    || self.reconciliation_cancellations.contains_key(execution)
                    || execution_view.cancellation.is_some()
                    || attempt_view.execution != *execution
                    || execution_view.attempts.last() != Some(attempt)
                    || !matches!(
                        attempt_view.state,
                        AttemptState::Scheduled | AttemptState::Leased | AttemptState::Running
                    )
                    || !matches!(
                        execution_view.state,
                        NodeExecutionState::Scheduled(ref active)
                            | NodeExecutionState::Running(ref active)
                            if active == attempt
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
                if before_dispatch {
                    self.complete_attempt_leases(attempt);
                    self.active_attempt_ids.remove(attempt);
                    self.attempts
                        .get_mut(attempt)
                        .ok_or_else(|| invalid_at(event, "unknown reconciliation attempt"))?
                        .state = AttemptState::CancelledBeforeDispatch;
                    self.node_executions
                        .get_mut(execution)
                        .ok_or_else(|| invalid_at(event, "unknown reconciliation execution"))?
                        .state = NodeExecutionState::CancelledBeforeDispatch;
                    self.deactivate_execution(execution, event)?;
                }
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
                let target_revision = plan_view.to_revision.clone();
                let source_revision = plan_view.from_revision.clone();
                let source = self
                    .current_node_execution(source_execution)
                    .ok_or_else(|| {
                        invalid_at(event, "remediation source is outside the current frontier")
                    })?;
                if !authorized
                    || self.revision.as_ref() != Some(&source_revision)
                    || self.node_executions.contains_key(execution)
                    || self.settled_node_executions.contains_key(execution)
                    || self.reserved_executions.contains(execution)
                    || self.reconciliation_remediations.contains_key(execution)
                    || source_attempt.as_ref().is_some_and(|attempt| {
                        !source.attempts().contains(attempt)
                            || self
                                .attempts
                                .get(attempt)
                                .is_some_and(|attempt| attempt.execution != *source_execution)
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
                        revision: target_revision,
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
                            self.current_node_execution(execution)
                                .is_some_and(|execution| {
                                    execution.state()
                                        == &NodeExecutionState::RemovedProspectively(plan.clone())
                                })
                        })
                    }
                    ReconciliationAction::UseNewOnNextInvocation
                        if item.classification == ReconciliationClassification::ChangedPending =>
                    {
                        item.execution.as_ref().is_none_or(|execution| {
                            self.current_node_execution(execution)
                                .is_some_and(|execution| {
                                    execution.state()
                                        == &NodeExecutionState::RemovedProspectively(plan.clone())
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
            _ => {
                unreachable!("reconciliation dispatch owns reconciliation action enactment routing")
            }
        }
        Ok(())
    }
}
