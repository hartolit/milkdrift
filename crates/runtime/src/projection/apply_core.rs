//! Exhaustive event-family dispatch for the authoritative run projection reducer.

use milkdrift_persistence::{RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_kind(&mut self, event: &RunEventEnvelope) -> Result<(), RuntimeError> {
        match event.kind() {
            RunEventKind::RunCreated { .. }
            | RunEventKind::ExecutionAuthorityEstablished { .. }
            | RunEventKind::RevisionPinned { .. }
            | RunEventKind::RunStarted
            | RunEventKind::RunPaused { .. }
            | RunEventKind::RunResumed { .. }
            | RunEventKind::RunCancellationRequested { .. }
            | RunEventKind::RunTerminationRequested { .. }
            | RunEventKind::RunTerminal { .. } => self.apply_lifecycle_kind(event),

            RunEventKind::NodeBecameEligible { .. }
            | RunEventKind::NodeExecutionCancelledBeforeDispatch { .. }
            | RunEventKind::NodeExecutionCancellationRequested { .. } => {
                self.apply_eligibility_kind(event)
            }

            RunEventKind::NodeScheduled { .. }
            | RunEventKind::CapabilityResolutionDecisionRecorded { .. }
            | RunEventKind::CapabilityResolved { .. }
            | RunEventKind::SideEffectClassified { .. } => self.apply_execution_kind(event),

            RunEventKind::LeaseGranted { .. }
            | RunEventKind::CapabilityEntryDecisionRecorded { .. }
            | RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. }
            | RunEventKind::LeaseHeartbeatRecorded { .. }
            | RunEventKind::LeaseExpired { .. }
            | RunEventKind::NodeReLeased { .. } => self.apply_lease_kind(event),

            RunEventKind::NodeStarted { .. }
            | RunEventKind::NodeProgressRecorded { .. }
            | RunEventKind::AttemptUsageRecorded { .. }
            | RunEventKind::InvocationCancellationAcknowledged { .. }
            | RunEventKind::NodeOutputPublished { .. }
            | RunEventKind::DeterministicOutputPublished { .. } => {
                self.apply_observation_kind(event)
            }

            RunEventKind::DeterministicNodeTerminal { .. }
            | RunEventKind::NodePreDispatchFailed { .. }
            | RunEventKind::CapabilityResolutionDenied { .. }
            | RunEventKind::StructuredSuccessorScanCompleted { .. }
            | RunEventKind::NodeTerminal { .. } => self.apply_terminal_kind(event),

            RunEventKind::NodeRetryScheduled { .. }
            | RunEventKind::ExternalOutcomeUncertain { .. }
            | RunEventKind::LateTerminalEvidenceRecorded { .. }
            | RunEventKind::ExternalOutcomeRetained { .. } => self.apply_retry_kind(event),

            RunEventKind::ArtifactPublished { .. } => self.apply_artifact_kind(event),

            RunEventKind::BranchScopeCreated { .. }
            | RunEventKind::BranchRouteSelected { .. }
            | RunEventKind::BranchChildAdded { .. }
            | RunEventKind::BranchCancellationRequested { .. }
            | RunEventKind::BranchTerminal { .. }
            | RunEventKind::JoinSatisfied { .. }
            | RunEventKind::RepeatIterationCreated { .. }
            | RunEventKind::RepeatConditionRecorded { .. }
            | RunEventKind::RepeatContinuationRequested { .. }
            | RunEventKind::RepeatContinuationDecided { .. }
            | RunEventKind::RepeatTerminated { .. }
            | RunEventKind::TimerRegistered { .. }
            | RunEventKind::TimerFired { .. }
            | RunEventKind::TimerCancelled { .. }
            | RunEventKind::WaitRegistered { .. }
            | RunEventKind::WaitSatisfied { .. }
            | RunEventKind::WaitCancelled { .. }
            | RunEventKind::SignalReceived { .. }
            | RunEventKind::SignalBroadcastScanAdvanced { .. }
            | RunEventKind::SignalDeduplicated { .. }
            | RunEventKind::SignalConsumed { .. }
            | RunEventKind::SubworkflowCreated { .. }
            | RunEventKind::SubworkflowTerminal { .. }
            | RunEventKind::SubworkflowOutputImported { .. }
            | RunEventKind::SubworkflowCancellationRequested { .. } => {
                self.apply_structured_kind(event)
            }

            RunEventKind::RevisionAdoptionRequested { .. }
            | RunEventKind::ReconciliationPlanRecorded { .. }
            | RunEventKind::ReconciliationDecisionRecorded { .. }
            | RunEventKind::ReconciliationApplied { .. }
            | RunEventKind::ReconciliationExecutionRemoved { .. }
            | RunEventKind::ReconciliationCancellationRequested { .. }
            | RunEventKind::ReconciliationRemediationCreated { .. }
            | RunEventKind::RecoveryStarted { .. }
            | RunEventKind::RecoveryClassified { .. }
            | RunEventKind::RecoveryDecisionRecorded { .. }
            | RunEventKind::RemediationWorkCreated { .. } => self.apply_reconciliation_kind(event),
        }
    }
}
