//! Exhaustive stable names for command and event families.

use crate::RunCommand;
use milkdrift_persistence::RunEventKind;

pub(in crate::engine) fn command_kind_name(command: &RunCommand) -> &'static str {
    match command {
        RunCommand::CreateRun { .. } => "create_run",
        RunCommand::StartRun => "start_run",
        RunCommand::PauseRun => "pause_run",
        RunCommand::ResumeRun => "resume_run",
        RunCommand::RequestCancellation => "request_cancellation",
        RunCommand::DeliverSignal { .. } => "deliver_signal",
        RunCommand::FireTimer { .. } => "fire_timer",
        RunCommand::RequestRevisionAdoption { .. } => "request_revision_adoption",
        RunCommand::DecideReconciliation { .. } => "decide_reconciliation",
        RunCommand::ApplyReconciliation { .. } => "apply_reconciliation",
        RunCommand::DecideRepeatContinuation { .. } => "decide_repeat_continuation",
        RunCommand::ResolveExternalWork { .. } => "resolve_external_work",
        RunCommand::SystemTransition { transition } => transition.label(),
        RunCommand::WorkerReport { .. } => "worker_report",
    }
}

pub(in crate::engine) fn event_kind_name(event: &RunEventKind) -> &'static str {
    match event {
        RunEventKind::RunCreated { .. } => "run_created",
        RunEventKind::ExecutionAuthorityEstablished { .. } => "execution_authority_established",
        RunEventKind::RevisionPinned { .. } => "revision_pinned",
        RunEventKind::RunStarted => "run_started",
        RunEventKind::RunPaused { .. } => "run_paused",
        RunEventKind::RunResumed { .. } => "run_resumed",
        RunEventKind::RunCancellationRequested { .. } => "run_cancellation_requested",
        RunEventKind::RunTerminationRequested { .. } => "run_termination_requested",
        RunEventKind::RunTerminal { .. } => "run_terminal",
        RunEventKind::NodeBecameEligible { .. } => "node_became_eligible",
        RunEventKind::NodeExecutionCancelledBeforeDispatch { .. } => {
            "node_execution_cancelled_before_dispatch"
        }
        RunEventKind::NodeExecutionCancellationRequested { .. } => {
            "node_execution_cancellation_requested"
        }
        RunEventKind::NodeScheduled { .. } => "node_scheduled",
        RunEventKind::CapabilityResolved { .. } => "capability_resolved",
        RunEventKind::CapabilityResolutionDecisionRecorded { .. } => {
            "capability_resolution_decision_recorded"
        }
        RunEventKind::SideEffectClassified { .. } => "side_effect_classified",
        RunEventKind::LeaseGranted { .. } => "lease_granted",
        RunEventKind::CapabilityEntryDecisionRecorded { .. } => {
            "capability_entry_decision_recorded"
        }
        RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. } => {
            "capability_adapter_entry_decision_recorded"
        }
        RunEventKind::LeaseHeartbeatRecorded { .. } => "lease_heartbeat_recorded",
        RunEventKind::LeaseExpired { .. } => "lease_expired",
        RunEventKind::NodeReLeased { .. } => "node_re_leased",
        RunEventKind::NodeStarted { .. } => "node_started",
        RunEventKind::NodeProgressRecorded { .. } => "node_progress_recorded",
        RunEventKind::AttemptUsageRecorded { .. } => "attempt_usage_recorded",
        RunEventKind::InvocationCancellationAcknowledged { .. } => {
            "invocation_cancellation_acknowledged"
        }
        RunEventKind::NodeOutputPublished { .. } => "node_output_published",
        RunEventKind::DeterministicOutputPublished { .. } => "deterministic_output_published",
        RunEventKind::DeterministicNodeTerminal { .. } => "deterministic_node_terminal",
        RunEventKind::NodePreDispatchFailed { .. } => "node_pre_dispatch_failed",
        RunEventKind::CapabilityResolutionDenied { .. } => "capability_resolution_denied",
        RunEventKind::StructuredSuccessorScanCompleted { .. } => {
            "structured_successor_scan_completed"
        }
        RunEventKind::NodeTerminal { .. } => "node_terminal",
        RunEventKind::NodeRetryScheduled { .. } => "node_retry_scheduled",
        RunEventKind::ExternalOutcomeUncertain { .. } => "external_outcome_uncertain",
        RunEventKind::LateTerminalEvidenceRecorded { .. } => "late_terminal_evidence_recorded",
        RunEventKind::ExternalOutcomeRetained { .. } => "external_outcome_retained",
        RunEventKind::ArtifactPublished { .. } => "artifact_published",
        RunEventKind::BranchScopeCreated { .. } => "branch_scope_created",
        RunEventKind::BranchRouteSelected { .. } => "branch_route_selected",
        RunEventKind::BranchChildAdded { .. } => "branch_child_added",
        RunEventKind::BranchCancellationRequested { .. } => "branch_cancellation_requested",
        RunEventKind::BranchTerminal { .. } => "branch_terminal",
        RunEventKind::JoinSatisfied { .. } => "join_satisfied",
        RunEventKind::ControllerAssessmentRecorded { .. } => "controller_assessment_recorded",
        RunEventKind::RepeatIterationCreated { .. } => "repeat_iteration_created",
        RunEventKind::RepeatConditionRecorded { .. } => "repeat_condition_recorded",
        RunEventKind::RepeatContinuationRequested { .. } => "repeat_continuation_requested",
        RunEventKind::RepeatContinuationDecided { .. } => "repeat_continuation_decided",
        RunEventKind::RepeatTerminated { .. } => "repeat_terminated",
        RunEventKind::TimerRegistered { .. } => "timer_registered",
        RunEventKind::TimerFired { .. } => "timer_fired",
        RunEventKind::TimerCancelled { .. } => "timer_cancelled",
        RunEventKind::WaitRegistered { .. } => "wait_registered",
        RunEventKind::WaitSatisfied { .. } => "wait_satisfied",
        RunEventKind::WaitCancelled { .. } => "wait_cancelled",
        RunEventKind::SignalReceived { .. } => "signal_received",
        RunEventKind::SignalBroadcastScanAdvanced { .. } => "signal_broadcast_scan_advanced",
        RunEventKind::SignalDeduplicated { .. } => "signal_deduplicated",
        RunEventKind::SignalConsumed { .. } => "signal_consumed",
        RunEventKind::SubworkflowCreated { .. } => "subworkflow_created",
        RunEventKind::SubworkflowTerminal { .. } => "subworkflow_terminal",
        RunEventKind::SubworkflowOutputImported { .. } => "subworkflow_output_imported",
        RunEventKind::SubworkflowCancellationRequested { .. } => {
            "subworkflow_cancellation_requested"
        }
        RunEventKind::RevisionAdoptionRequested { .. } => "revision_adoption_requested",
        RunEventKind::ReconciliationPlanRecorded { .. } => "reconciliation_plan_recorded",
        RunEventKind::ReconciliationDecisionRecorded { .. } => "reconciliation_decision_recorded",
        RunEventKind::ReconciliationApplied { .. } => "reconciliation_applied",
        RunEventKind::ReconciliationExecutionRemoved { .. } => "reconciliation_execution_removed",
        RunEventKind::ReconciliationCancellationRequested { .. } => {
            "reconciliation_cancellation_requested"
        }
        RunEventKind::ReconciliationRemediationCreated { .. } => {
            "reconciliation_remediation_created"
        }
        RunEventKind::RecoveryStarted { .. } => "recovery_started",
        RunEventKind::RecoveryClassified { .. } => "recovery_classified",
        RunEventKind::RecoveryDecisionRecorded { .. } => "recovery_decision_recorded",
        RunEventKind::RemediationWorkCreated { .. } => "remediation_work_created",
    }
}
