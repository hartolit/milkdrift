//! Runtime projections grouped by execution, attempt, and ownership invariants.

mod attempt;
mod execution;
mod lease;

pub use attempt::{
    AttemptState, AttemptTerminal, CapabilityResolution, ExternalOutcomeObligation,
    LateTerminalEvidence, NodeAttemptProjection, ProgressObservation, PublishedNodeOutput,
    RetainedExternalOutcome, SideEffectClassification,
};
pub use execution::{
    CurrentNodeExecution, DeterministicNodeTerminalProjection, NodeExecutionCancellationProjection,
    NodeExecutionProjection, NodeExecutionState, SettledNodeExecutionProjection,
};
pub use lease::{
    LeaseProjection, LeaseState, RetryProjection, RetryState, TimerCancellationProjection,
    TimerProjection, TimerPurpose, TimerState,
};
