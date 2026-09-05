//! Durable run command handling, deterministic projection, scheduling, execution,
//! recovery, and prospective revision reconciliation for Milkdrift.
//!
//! The crate owns workflow state transitions but no database, provider, operating
//! system process, network, asynchronous runtime, HTTP API, or UI. Every accepted
//! transition is expressed as a persistence-owned append-only event. Executor
//! adapters report bounded observations through [`TaskExecutor`]; they cannot mutate
//! projections or append history.

mod boundary;
mod command;
mod context;
mod controller;
mod engine;
mod error;
mod executor;
mod projection;
mod query;
mod reconciliation;
mod scheduler;

#[cfg(any(test, feature = "test-support"))]
pub use boundary::ManualClock;
pub use boundary::{BoundaryClock, IdGenerator, SequentialIdGenerator, SystemBoundaryClock};
pub use command::{
    AUTHORIZED_RUN_COMMAND_SCHEMA_VERSION_V1, CommandAuthorityClaim, ExternalWorkAction,
    MAX_COMMAND_ITEMS, RUN_COMMAND_SCHEMA_VERSION_V1, RunCommand, RunCommandDocument,
    SystemTransition, WorkerReport,
};
pub use context::{
    CausalContextBuilder, ContextBuildError, ContextBuildIdentity, ContextBuildRequest,
    ContextCandidate, ContextCandidateArtifactFacts, ContextCandidateAvailability,
    ContextCandidateSource, ContextSourceRequest, DurableContextCandidateSource,
    materialize_selected_context, persist_context_manifest, read_context_manifest,
};
pub use controller::{
    CONTROLLER_POLICY_EXTENSION_KEY, ControllerAssessment, ControllerAssessmentContext,
    ControllerLifecycle,
};
pub use engine::{
    CommandExecution, EffectExecutionResult, RecoveryResult, RuntimeConfig, RuntimeHealth,
    RuntimeService, RuntimeStartupState, RuntimeStore, SchedulerTickResult,
};
pub use error::RuntimeError;
#[cfg(any(test, feature = "test-support"))]
pub use executor::DeterministicExecutor;
pub use executor::{
    CancellationDispatch, CapabilityResolutionContext, EffectAction, ExecutionDispatch,
    ExecutionReporter, ExecutorError, MAX_REPORTS_PER_DISPATCH, ObservationDisposition,
    PreparedExecution, ResolvedCapability, TaskExecutor,
};
pub use projection::{
    AttemptState, AttemptTerminal, BranchProjection, BranchState, CapabilityResolution,
    ControllerAssessmentProjection, DeterministicNodeTerminalProjection, ExternalOutcomeObligation,
    IterationProjection, IterationState, JoinProjection, LateTerminalEvidence, LeaseProjection,
    LeaseState, NodeAttemptProjection, NodeExecutionCancellationProjection,
    NodeExecutionProjection, NodeExecutionState, ProgressObservation, PublishedNodeOutput,
    ReconciliationCancellationProjection, ReconciliationDecision, ReconciliationPlanProjection,
    ReconciliationProjection, ReconciliationRemediationProjection, ReconciliationRequestProjection,
    ReconciliationRequestState, RecoveryDecision, RecoveryObservation, RecoveryProjection,
    RemediationProjection, RepeatContinuationDecisionProjection, RepeatContinuationProjection,
    RepeatContinuationRequestProjection, RepeatTermination, ResourceUsage, RetainedExternalOutcome,
    RetryProjection, RetryState, RevisionPin, RunCancellation, RunLifecycle, RunProjection,
    RunTerminalProjection, RunTerminationIntent, SideEffectClassification, SignalProjection,
    SubworkflowOutputImport, SubworkflowProjection, SubworkflowState, TimerCancellationProjection,
    TimerProjection, TimerPurpose, TimerState, WaitCancellationProjection, WaitProjection,
};
pub use scheduler::{RetryPolicy, SchedulerLimits};
