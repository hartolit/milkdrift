//! Revision adoption, reconciliation decisions, and explicit external-work resolution.

use super::RuntimeService;
use super::support::{
    CommandPlan, node_execution_mode, reconciliation_history, unresolved_retry_error_class,
};
use crate::projection::RunProjection;
use crate::query::fold_history_from;
use crate::reconciliation::plan_reconciliation;
use crate::{ExternalWorkAction, RunCommandDocument, RuntimeError};
use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_persistence::{
    AttemptId, AuthorityDecision, ReconciliationAction, ReconciliationClassification,
    ReconciliationDecisionId, ReconciliationPlanId, RunEventKind,
};
use milkdrift_workspace::RunId;

impl RuntimeService {
    pub(super) fn plan_revision_adoption(
        &self,
        projection: &RunProjection,
        requested_by: &ActorRef,
        reconciliation: &milkdrift_persistence::ReconciliationId,
        requested_revision: &RevisionId,
        policy: milkdrift_persistence::ReconciliationPolicy,
    ) -> Result<CommandPlan, RuntimeError> {
        if !projection.lifecycle().is_active()
            || projection.termination().is_some()
            || projection.reconciliation().is_active()
        {
            return Err(RuntimeError::InvalidTransition(
                "revision adoption requires an active non-draining run with no active reconciliation"
                    .to_owned(),
            ));
        }
        let old = self.current_revision(projection)?;
        let workflow = projection
            .workflow()
            .ok_or_else(|| RuntimeError::InvalidHistory("run has no workflow".to_owned()))?;
        let new = self.load_validated_revision(requested_revision, Some(workflow))?;
        let history = reconciliation_history(projection, &old, &new)?;
        let plan_id = self.next_plan_id()?;
        let plan = plan_reconciliation(
            reconciliation.clone(),
            plan_id,
            &old,
            &new,
            projection.sequence(),
            &history,
            policy,
        )?;
        let mut result = CommandPlan::one(RunEventKind::RevisionAdoptionRequested {
            reconciliation: reconciliation.clone(),
            requested_by: Some(requested_by.clone()),
            from_revision: old.id().clone(),
            to_revision: new.id().clone(),
            policy,
        });
        result.events.push(plan.recorded_event());
        Ok(result)
    }

    pub(super) fn plan_reconciliation_decision(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        plan: &ReconciliationPlanId,
        decision: &ReconciliationDecisionId,
        outcome: AuthorityDecision,
    ) -> Result<CommandPlan, RuntimeError> {
        let plan_view = projection
            .reconciliation()
            .plans()
            .get(plan)
            .ok_or_else(|| RuntimeError::Reconciliation(format!("unknown plan {plan}")))?;
        if !plan_view.is_pending()
            || !matches!(
                outcome,
                AuthorityDecision::Approve | AuthorityDecision::Reject
            )
        {
            return Err(RuntimeError::Reconciliation(
                "only a pending plan can receive an approve/reject decision".to_owned(),
            ));
        }
        if projection
            .reconciliation()
            .plans()
            .values()
            .flat_map(|candidate| candidate.decisions())
            .any(|existing| existing.decision() == decision)
        {
            return Err(RuntimeError::Reconciliation(
                "reconciliation decision identity was already used".to_owned(),
            ));
        }
        Ok(CommandPlan::one(
            RunEventKind::ReconciliationDecisionRecorded {
                plan: plan.clone(),
                decision: decision.clone(),
                actor: document.actor().clone(),
                outcome,
                reason: document.reason().clone(),
                evidence: document.evidence().to_vec(),
            },
        ))
    }

    pub(super) fn plan_reconciliation_application(
        &self,
        run: &RunId,
        projection: &RunProjection,
        plan: &ReconciliationPlanId,
    ) -> Result<CommandPlan, RuntimeError> {
        let plan_view = projection
            .reconciliation()
            .plans()
            .get(plan)
            .ok_or_else(|| RuntimeError::Reconciliation(format!("unknown plan {plan}")))?;
        if !plan_view.is_pending() {
            return Err(RuntimeError::Reconciliation(
                "reconciliation plan was already applied".to_owned(),
            ));
        }
        if plan_view
            .items()
            .iter()
            .any(|item| item.action == ReconciliationAction::RejectRetrospectiveRewrite)
        {
            return Err(RuntimeError::Reconciliation(
                "plan contains a retrospective rewrite and cannot be applied".to_owned(),
            ));
        }
        let needs_authority = plan_view
            .items()
            .iter()
            .any(|item| item.action == ReconciliationAction::RequireAuthority);
        let last_decision = plan_view.decisions().last().map(|value| value.outcome());
        if last_decision == Some(AuthorityDecision::Reject)
            || (needs_authority && last_decision != Some(AuthorityDecision::Approve))
        {
            return Err(RuntimeError::Reconciliation(
                "plan lacks a final approving authority decision".to_owned(),
            ));
        }
        if projection.revision() != Some(plan_view.from_revision()) {
            return Err(RuntimeError::Reconciliation(
                "revision pin moved after the plan was created".to_owned(),
            ));
        }
        fold_history_from(
            self.store.as_ref(),
            run,
            plan_view.based_on_sequence(),
            (),
            |_unit, event| {
                let allowed = match event.kind() {
                    RunEventKind::RevisionAdoptionRequested { reconciliation, .. }
                    | RunEventKind::ReconciliationPlanRecorded { reconciliation, .. } => {
                        reconciliation == plan_view.reconciliation()
                    }
                    RunEventKind::ReconciliationDecisionRecorded {
                        plan: event_plan, ..
                    } => event_plan == plan,
                    _ => false,
                };
                if allowed {
                    Ok(())
                } else {
                    Err(RuntimeError::Reconciliation(format!(
                        "plan became stale at event {} sequence {}",
                        event.event_id(),
                        event.sequence()
                    )))
                }
            },
        )?;
        let next = self.load_validated_revision(plan_view.to_revision(), projection.workflow())?;
        let mut result = CommandPlan::default();
        for item in plan_view.items() {
            match item.action {
                ReconciliationAction::RemoveUnstarted => {
                    if let Some(execution) = &item.execution {
                        result
                            .events
                            .push(RunEventKind::ReconciliationExecutionRemoved {
                                plan: plan.clone(),
                                execution: execution.clone(),
                            });
                    }
                }
                ReconciliationAction::CancelAndRestart => {
                    let execution = item.execution.as_ref().ok_or_else(|| {
                        RuntimeError::Reconciliation(
                            "cancel-and-restart item has no exact execution".to_owned(),
                        )
                    })?;
                    let execution_view =
                        projection.node_executions().get(execution).ok_or_else(|| {
                            RuntimeError::Reconciliation(
                                "cancel-and-restart execution is absent".to_owned(),
                            )
                        })?;
                    let attempt = execution_view.attempts().last().ok_or_else(|| {
                        RuntimeError::Reconciliation(
                            "cancel-and-restart execution has no active attempt".to_owned(),
                        )
                    })?;
                    result
                        .events
                        .push(RunEventKind::ReconciliationCancellationRequested {
                            plan: plan.clone(),
                            execution: execution.clone(),
                            attempt: attempt.clone(),
                            reason: item.reason.clone(),
                        });
                }
                ReconciliationAction::CompensateOrRemediate => {
                    let source_execution = item.execution.as_ref().ok_or_else(|| {
                        RuntimeError::Reconciliation(
                            "remediation item has no source execution".to_owned(),
                        )
                    })?;
                    let node = item.node.as_ref().ok_or_else(|| {
                        RuntimeError::Reconciliation(
                            "remediation item has no target node".to_owned(),
                        )
                    })?;
                    if !next.semantic().nodes().contains_key(node) {
                        return Err(RuntimeError::Reconciliation(
                            "remediation target is absent from the adopted revision".to_owned(),
                        ));
                    }
                    let source = projection
                        .current_node_execution(source_execution)
                        .ok_or_else(|| {
                            RuntimeError::Reconciliation(
                                "remediation source execution is absent".to_owned(),
                            )
                        })?;
                    result
                        .events
                        .push(RunEventKind::ReconciliationRemediationCreated {
                            plan: plan.clone(),
                            source_execution: source_execution.clone(),
                            source_attempt: source.attempts().last().cloned(),
                            execution: self.next_execution_id()?,
                            node: node.clone(),
                            scope: source.scope().clone(),
                            mode: node_execution_mode(
                                next.semantic().nodes().get(node).ok_or_else(|| {
                                    RuntimeError::InvalidHistory(
                                        "reconciliation remediation node is absent".to_owned(),
                                    )
                                })?,
                            ),
                            reason: item.reason.clone(),
                        });
                }
                ReconciliationAction::UseNewOnNextInvocation
                    if item.classification == ReconciliationClassification::ChangedPending =>
                {
                    if let Some(execution) = &item.execution {
                        result
                            .events
                            .push(RunEventKind::ReconciliationExecutionRemoved {
                                plan: plan.clone(),
                                execution: execution.clone(),
                            });
                    }
                }
                ReconciliationAction::Preserve
                | ReconciliationAction::UseNewOnNextInvocation
                | ReconciliationAction::RequireAuthority => {}
                ReconciliationAction::RejectRetrospectiveRewrite => {
                    return Err(RuntimeError::Reconciliation(
                        "rejected retrospective rewrite cannot be enacted".to_owned(),
                    ));
                }
            }
        }
        let mut application_boundary = projection.sequence();
        for _ in &result.events {
            application_boundary = application_boundary.next()?;
        }
        result.events.push(RunEventKind::ReconciliationApplied {
            plan: plan.clone(),
            from_revision: plan_view.from_revision().clone(),
            to_revision: plan_view.to_revision().clone(),
            based_on_sequence: application_boundary,
        });
        result.events.push(RunEventKind::RevisionPinned {
            previous: plan_view.from_revision().clone(),
            revision: plan_view.to_revision().clone(),
            revision_digest: next.content_digest().clone(),
            plan: plan.clone(),
        });
        Ok(result)
    }

    pub(super) fn plan_external_resolution(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        attempt: &AttemptId,
        decision: &ReconciliationDecisionId,
        action: ExternalWorkAction,
        remediation_node: Option<&NodeId>,
    ) -> Result<CommandPlan, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        if !attempt_view.is_unresolved() {
            return Err(RuntimeError::InvalidTransition(
                "external-work resolution requires an uncertain or retained attempt".to_owned(),
            ));
        }
        let retry_event = if action == ExternalWorkAction::Retry {
            let classified = attempt_view.side_effect().ok_or_else(|| {
                RuntimeError::InvalidTransition(
                    "manual retry requires a durable side-effect classification".to_owned(),
                )
            })?;
            let error_class = attempt_view
                .terminal()
                .and_then(crate::projection::AttemptTerminal::error_class)
                .unwrap_or_else(|| unresolved_retry_error_class(attempt_view));
            if !self.config.retry_policy.permits_automatic_retry(
                attempt_view.attempt_number(),
                error_class,
                true,
                classified.side_effect(),
                classified.idempotency(),
                classified.idempotency_key(),
            ) {
                return Err(RuntimeError::InvalidTransition(
                    "manual retry exceeds the bounded retry policy or is unsafe for the durable side-effect/idempotency facts"
                        .to_owned(),
                ));
            }
            Some(self.build_retry_event(
                attempt_view.execution(),
                attempt,
                attempt_view.attempt_number(),
                document.issued_at(),
                error_class,
                None,
                "bounded authority retry admitted by durable side-effect and idempotency policy",
            )?)
        } else {
            None
        };
        let authority = match action {
            ExternalWorkAction::Retain => AuthorityDecision::Retain,
            ExternalWorkAction::Query => AuthorityDecision::Query,
            ExternalWorkAction::Retry => AuthorityDecision::Retry,
            ExternalWorkAction::Compensate => AuthorityDecision::Compensate,
            ExternalWorkAction::ResolveSucceeded => AuthorityDecision::ResolveSucceeded,
            ExternalWorkAction::ResolveFailed => AuthorityDecision::ResolveFailed,
        };
        let mut plan = CommandPlan::one(RunEventKind::RecoveryDecisionRecorded {
            attempt: attempt.clone(),
            decision: decision.clone(),
            actor: document.actor().clone(),
            outcome: authority,
            reason: document.reason().clone(),
            evidence: document.evidence().to_vec(),
        });
        match action {
            ExternalWorkAction::Retain => {
                plan.events.push(RunEventKind::ExternalOutcomeRetained {
                    attempt: attempt.clone(),
                    decision: decision.clone(),
                    reason: document.reason().clone(),
                });
            }
            ExternalWorkAction::Query => {}
            ExternalWorkAction::Retry => {
                plan.events.push(retry_event.ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "validated manual retry event disappeared".to_owned(),
                    )
                })?);
            }
            ExternalWorkAction::Compensate => {
                let node = remediation_node.ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "compensation requires an exact remediation target node".to_owned(),
                    )
                })?;
                let revision = self.current_revision(projection)?;
                let target = revision.semantic().nodes().get(node).ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "remediation target is absent from the pinned revision".to_owned(),
                    )
                })?;
                let source_execution = projection
                    .node_executions()
                    .get(attempt_view.execution())
                    .ok_or_else(|| {
                    RuntimeError::InvalidHistory("uncertain source execution is absent".to_owned())
                })?;
                plan.events.push(RunEventKind::RemediationWorkCreated {
                    source_attempt: attempt.clone(),
                    execution: self.next_execution_id()?,
                    node: node.clone(),
                    scope: source_execution.scope().clone(),
                    mode: node_execution_mode(target),
                    decision: decision.clone(),
                    reason: document.reason().clone(),
                });
            }
            ExternalWorkAction::ResolveSucceeded | ExternalWorkAction::ResolveFailed => {}
        }
        Ok(plan)
    }
}
