//! Exhaustive dispatch for revision reconciliation and recovery facts.

use milkdrift_persistence::{RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_reconciliation_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        match event.kind() {
            RunEventKind::RevisionAdoptionRequested { .. }
            | RunEventKind::ReconciliationPlanRecorded { .. }
            | RunEventKind::ReconciliationDecisionRecorded { .. } => {
                self.apply_reconciliation_plan_kind(event)
            }

            RunEventKind::ReconciliationExecutionRemoved { .. }
            | RunEventKind::ReconciliationCancellationRequested { .. }
            | RunEventKind::ReconciliationRemediationCreated { .. }
            | RunEventKind::ReconciliationApplied { .. } => {
                self.apply_reconciliation_action_kind(event)
            }

            RunEventKind::RecoveryStarted { .. }
            | RunEventKind::RecoveryClassified { .. }
            | RunEventKind::RecoveryDecisionRecorded { .. }
            | RunEventKind::RemediationWorkCreated { .. } => self.apply_recovery_kind(event),
            _ => unreachable!("central projection dispatch owns reconciliation routing"),
        }
    }
}
