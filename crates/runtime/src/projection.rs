//! Pure, deterministic projection of one run's authoritative event history.
//!
//! This module deliberately has no persistence, clock, ID-generation, registry, or
//! executor dependency. It accepts already-decoded event envelopes and either
//! projects every fact or rejects the history at the first point where doing so
//! would require guessing.

mod apply_core;
mod apply_reconciliation;
mod apply_structured;
mod helpers;
mod node;
mod reconciliation;
mod replay;
mod run;
mod structured;

pub use node::{
    AttemptState, AttemptTerminal, CapabilityResolution, DeterministicNodeTerminalProjection,
    ExternalOutcomeObligation, LeaseProjection, LeaseState, NodeAttemptProjection,
    NodeExecutionCancellationProjection, NodeExecutionProjection, NodeExecutionState,
    ProgressObservation, PublishedNodeOutput, RetainedExternalOutcome, RetryProjection, RetryState,
    SideEffectClassification, TimerCancellationProjection, TimerProjection, TimerPurpose,
    TimerState,
};
pub use reconciliation::{
    ReconciliationCancellationProjection, ReconciliationDecision, ReconciliationPlanProjection,
    ReconciliationProjection, ReconciliationRemediationProjection, ReconciliationRequestProjection,
    ReconciliationRequestState, RecoveryDecision, RecoveryObservation, RecoveryProjection,
    RemediationProjection,
};
pub use run::{
    ResourceUsage, RevisionPin, RunCancellation, RunLifecycle, RunProjection,
    RunTerminalProjection, RunTerminationIntent,
};
pub use structured::{
    BranchProjection, BranchState, IterationProjection, IterationState, JoinProjection,
    RepeatContinuationDecisionProjection, RepeatContinuationProjection,
    RepeatContinuationRequestProjection, RepeatTermination, SignalProjection,
    SubworkflowOutputImport, SubworkflowProjection, SubworkflowState, WaitCancellationProjection,
    WaitProjection,
};

#[cfg(test)]
mod tests;
