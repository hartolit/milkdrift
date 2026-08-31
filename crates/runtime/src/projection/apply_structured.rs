//! Exhaustive dispatch for structured-control event families.

use milkdrift_persistence::{RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::helpers::invalid_at;
use super::run::RunProjection;
use super::structured::ControllerAssessmentProjection;

impl RunProjection {
    pub(super) fn apply_structured_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        match event.kind() {
            RunEventKind::BranchScopeCreated { .. }
            | RunEventKind::BranchRouteSelected { .. }
            | RunEventKind::BranchChildAdded { .. }
            | RunEventKind::BranchCancellationRequested { .. }
            | RunEventKind::BranchTerminal { .. } => self.apply_branch_kind(event),

            RunEventKind::JoinSatisfied { .. } => self.apply_join_kind(event),

            RunEventKind::ControllerAssessmentRecorded { .. } => {
                self.apply_controller_assessment_kind(event)
            }

            RunEventKind::RepeatIterationCreated { .. }
            | RunEventKind::RepeatConditionRecorded { .. }
            | RunEventKind::RepeatContinuationRequested { .. } => {
                self.apply_repeat_iteration_kind(event)
            }

            RunEventKind::RepeatContinuationDecided { .. }
            | RunEventKind::RepeatTerminated { .. } => self.apply_repeat_decision_kind(event),

            RunEventKind::TimerRegistered { .. }
            | RunEventKind::TimerFired { .. }
            | RunEventKind::TimerCancelled { .. } => self.apply_timer_kind(event),

            RunEventKind::WaitRegistered { .. }
            | RunEventKind::WaitSatisfied { .. }
            | RunEventKind::WaitCancelled { .. }
            | RunEventKind::SignalReceived { .. }
            | RunEventKind::SignalBroadcastScanAdvanced { .. }
            | RunEventKind::SignalDeduplicated { .. }
            | RunEventKind::SignalConsumed { .. } => self.apply_wait_signal_kind(event),

            RunEventKind::SubworkflowCreated { .. }
            | RunEventKind::SubworkflowTerminal { .. }
            | RunEventKind::SubworkflowOutputImported { .. }
            | RunEventKind::SubworkflowCancellationRequested { .. } => {
                self.apply_subworkflow_kind(event)
            }
            _ => unreachable!("central projection dispatch owns structured routing"),
        }
    }

    fn apply_controller_assessment_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let RunEventKind::ControllerAssessmentRecorded {
            controller_id,
            policy_digest,
            governing_revision,
            controller_node,
            controller_execution,
            assessment_id,
            cycle_id,
            boundary,
            through_sequence,
            progress,
            outcome,
        } = event.kind()
        else {
            unreachable!("controller assessment reducer received another event family");
        };
        let execution = self
            .node_executions
            .get(controller_execution)
            .ok_or_else(|| invalid_at(event, "controller assessment execution is absent"))?;
        let execution_started_at = execution.created_at();
        if execution.node() != controller_node
            || execution.revision() != governing_revision
            || *through_sequence
                != milkdrift_persistence::RunSequence::new(event.sequence().get().saturating_sub(1))
            || self
                .controller_assessments
                .get(controller_execution)
                .is_some_and(|prior| prior.assessment_id == *assessment_id)
        {
            return Err(invalid_at(
                event,
                "controller assessment does not bind the exact next controller boundary",
            ));
        }
        let started_at = self
            .controller_assessments
            .get(controller_execution)
            .map_or(execution_started_at, |prior| prior.started_at);
        self.controller_assessments.insert(
            controller_execution.clone(),
            ControllerAssessmentProjection {
                controller_id: controller_id.clone(),
                policy_digest: policy_digest.clone(),
                governing_revision: governing_revision.clone(),
                controller_node: controller_node.clone(),
                controller_execution: controller_execution.clone(),
                assessment_id: assessment_id.clone(),
                cycle_id: cycle_id.clone(),
                boundary: *boundary,
                through_sequence: *through_sequence,
                progress: progress.clone(),
                outcome: outcome.clone(),
                started_at,
                recorded_sequence: event.sequence(),
                recorded_at: event.occurred_at(),
            },
        );
        Ok(())
    }
}
