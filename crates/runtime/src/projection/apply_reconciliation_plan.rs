use milkdrift_capability::SideEffectClass;
use milkdrift_persistence::{
    AuthorityDecision, NodeExecutionId, ReconciliationAction, ReconciliationClassification,
    RunEventEnvelope, RunEventKind,
};

use crate::RuntimeError;

use super::helpers::invalid_at;
use super::reconciliation::{
    ReconciliationDecision, ReconciliationPlanProjection, ReconciliationRequestProjection,
    ReconciliationRequestState,
};
use super::run::RunProjection;

impl RunProjection {
    pub(super) fn reconciliation_cancellation_is_safe(&self, execution: &NodeExecutionId) -> bool {
        let Some(execution_view) = self.node_executions.get(execution) else {
            return false;
        };
        let attempt = match &execution_view.state {
            super::node::NodeExecutionState::Scheduled(attempt)
            | super::node::NodeExecutionState::Running(attempt) => attempt,
            _ => return false,
        };
        self.attempts.get(attempt).is_some_and(|attempt_view| {
            attempt_view.execution == *execution
                && matches!(
                    attempt_view.state,
                    super::node::AttemptState::Scheduled
                        | super::node::AttemptState::Leased
                        | super::node::AttemptState::Running
                )
                && attempt_view.side_effect.as_ref().is_some_and(|facts| {
                    matches!(
                        facts.side_effect,
                        SideEffectClass::None | SideEffectClass::ReadOnly
                    )
                })
        }) && !self.execution_has_active_structured_ownership(execution)
    }

    pub(super) fn apply_reconciliation_plan_kind(
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
                    || self.revision.as_ref() != Some(from_revision)
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
                        let execution_view =
                            self.current_node_execution(execution).ok_or_else(|| {
                                invalid_at(event, "reconciliation item references retired history")
                            })?;
                        if item
                            .node
                            .as_ref()
                            .is_some_and(|node| node != execution_view.node())
                        {
                            return Err(invalid_at(
                                event,
                                "reconciliation item node and execution disagree",
                            ));
                        }
                        if item.action == ReconciliationAction::CancelAndRestart
                            && !self.reconciliation_cancellation_is_safe(execution)
                        {
                            return Err(invalid_at(
                                event,
                                "cancel-and-restart requires active none/read-only work with no structured ownership",
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
            _ => {
                unreachable!("reconciliation dispatch owns reconciliation request and plan routing")
            }
        }
        Ok(())
    }
}
