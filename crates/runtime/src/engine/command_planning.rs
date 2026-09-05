//! Command validation, planning, worker-report incorporation, and atomic command commits.

mod admission;
mod commit;
mod reporting;

use super::RuntimeService;
use super::support::{CommandPlan, require_lifecycle};
use crate::projection::{IterationState, RunLifecycle, RunProjection};
use crate::{
    CONTROLLER_POLICY_EXTENSION_KEY, ControllerAssessmentContext, RunCommand, RunCommandDocument,
    RuntimeError,
};
use milkdrift_blueprint::{NodeKind, RepeatTermination};
use milkdrift_persistence::{
    ControllerAssessmentBoundary, ControllerAssessmentOutcome, RunEventKind,
};

impl RuntimeService {
    pub(super) fn plan_command(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
    ) -> Result<CommandPlan, RuntimeError> {
        match document.command() {
            RunCommand::CreateRun { .. } => self.plan_create_run(document, projection),
            RunCommand::StartRun => self.plan_start_run(projection),
            RunCommand::PauseRun => {
                require_lifecycle(projection, RunLifecycle::Running, "pause")?;
                Ok(CommandPlan::one(RunEventKind::RunPaused {
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                }))
            }
            RunCommand::ResumeRun => {
                require_lifecycle(projection, RunLifecycle::Paused, "resume")?;
                Ok(CommandPlan::one(RunEventKind::RunResumed {
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                }))
            }
            RunCommand::RequestCancellation => {
                if !matches!(
                    projection.lifecycle(),
                    RunLifecycle::Created | RunLifecycle::Running | RunLifecycle::Paused
                ) {
                    return Err(RuntimeError::InvalidTransition(
                        "only a created, running, or paused run can be cancelled".to_owned(),
                    ));
                }
                Ok(CommandPlan::one(RunEventKind::RunCancellationRequested {
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                }))
            }
            RunCommand::DeliverSignal { .. } => self.plan_signal(document, projection),
            RunCommand::FireTimer { .. } => self.plan_timer(document, projection),
            RunCommand::RequestRevisionAdoption {
                reconciliation,
                revision,
                policy,
            } => self.plan_revision_adoption(
                projection,
                document.actor(),
                reconciliation,
                revision,
                *policy,
            ),
            RunCommand::DecideReconciliation {
                plan,
                decision,
                outcome,
            } => self.plan_reconciliation_decision(document, projection, plan, decision, *outcome),
            RunCommand::ApplyReconciliation { plan } => {
                self.plan_reconciliation_application(document.run_id(), projection, plan)
            }
            RunCommand::DecideRepeatContinuation {
                repeat_execution,
                decision,
                outcome,
                approved_additional_iterations,
            } => {
                let execution = projection
                    .node_executions()
                    .get(repeat_execution)
                    .ok_or_else(|| {
                        RuntimeError::InvalidTransition(
                            "repeat continuation references an unknown execution".to_owned(),
                        )
                    })?;
                let revision = self.revision_for_execution(projection, repeat_execution)?;
                let node = revision
                    .semantic()
                    .nodes()
                    .get(execution.node())
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "repeat continuation node is absent from the revision".to_owned(),
                        )
                    })?;
                let NodeKind::Repeat { config } = node.kind() else {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat execution is not configured to await approval".to_owned(),
                    ));
                };
                if config.termination() != RepeatTermination::AwaitApproval {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat execution is not configured to await approval".to_owned(),
                    ));
                }
                let frontier = projection
                    .iterations()
                    .values()
                    .filter(|iteration| iteration.repeat_execution() == repeat_execution)
                    .max_by_key(|iteration| iteration.iteration_number())
                    .ok_or_else(|| {
                        RuntimeError::InvalidTransition(
                            "repeat continuation has no iteration frontier".to_owned(),
                        )
                    })?;
                if frontier.state() != IterationState::ConditionRecorded(true) {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat continuation requires a true-condition frontier".to_owned(),
                    ));
                }
                let continuation = projection
                    .repeat_continuations()
                    .get(repeat_execution)
                    .ok_or_else(|| {
                        RuntimeError::InvalidTransition(
                            "repeat continuation has no durable authority request".to_owned(),
                        )
                    })?;
                let pending_request = continuation.pending_request().ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "repeat continuation has no pending durable authority request".to_owned(),
                    )
                })?;
                if continuation.is_rejected()
                    || pending_request.frontier_iteration() != frontier.iteration()
                {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat continuation decision is outside its exact authority boundary"
                            .to_owned(),
                    ));
                }
                let decision_event = RunEventKind::RepeatContinuationDecided {
                    repeat_execution: repeat_execution.clone(),
                    decision: decision.clone(),
                    actor: document.actor().clone(),
                    outcome: *outcome,
                    approved_additional_iterations: *approved_additional_iterations,
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                };
                if !matches!(
                    pending_request.cause(),
                    milkdrift_persistence::RepeatContinuationCause::ControllerCheckpoint { .. }
                ) {
                    return Ok(CommandPlan::one(decision_event));
                }
                let marked = revision
                    .semantic()
                    .metadata()
                    .extensions()
                    .keys()
                    .any(|key| key.as_str() == CONTROLLER_POLICY_EXTENSION_KEY);
                let lifecycle = self.controller_lifecycle()?;
                let Some(lifecycle) = lifecycle else {
                    return Err(RuntimeError::Scheduling(
                        "controller checkpoint has no installed lifecycle owner".to_owned(),
                    ));
                };
                let account = self.controller_account_for_run(document.run_id())?;
                let assessment = {
                    let context = ControllerAssessmentContext {
                        run: document.run_id(),
                        revision: &revision,
                        node,
                        execution: repeat_execution,
                        projection,
                        account: account.as_ref(),
                        observed_at: document.issued_at(),
                        boundary: ControllerAssessmentBoundary::CheckpointContinuation,
                        next_cycle: None,
                    };
                    lifecycle.assess(&context)?.map(|assessment| {
                        let outcome = assessment.outcome.clone();
                        (assessment.into_event(&context), outcome)
                    })
                };
                let Some((assessment_event, assessment_outcome)) = assessment else {
                    if marked {
                        return Err(RuntimeError::InvalidHistory(
                            "marked controller checkpoint was not recognized".to_owned(),
                        ));
                    }
                    return Ok(CommandPlan::one(decision_event));
                };
                let mut plan = CommandPlan::one(assessment_event);
                if *outcome == milkdrift_persistence::RepeatContinuationDecision::Rejected
                    || assessment_outcome == ControllerAssessmentOutcome::Continue
                {
                    plan.events.push(decision_event);
                }
                Ok(plan)
            }
            RunCommand::ResolveExternalWork {
                attempt,
                decision,
                action,
                remediation_node,
            } => self.plan_external_resolution(
                document,
                projection,
                attempt,
                decision,
                *action,
                remediation_node.as_ref(),
            ),
            RunCommand::SystemTransition { .. } => Err(RuntimeError::InvalidCommand(
                "system transitions are runtime-owned and cannot be submitted externally"
                    .to_owned(),
            )),
            RunCommand::WorkerReport { worker, report } => {
                self.plan_worker_report(document, projection, worker, report)
            }
        }
    }
}
