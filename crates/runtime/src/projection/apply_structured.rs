//! Exhaustive dispatch for structured-control event families.

use milkdrift_persistence::{RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::run::RunProjection;

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
}
