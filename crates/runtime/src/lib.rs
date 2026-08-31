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

pub use boundary::{
    BoundaryClock, IdGenerator, ManualClock, SequentialIdGenerator, SystemBoundaryClock,
};
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
    CommandExecution, EffectExecutionResult, EffectTickResult, RecoveryResult, RuntimeConfig,
    RuntimeHealth, RuntimeService, RuntimeStartupState, RuntimeStore, SchedulerTickResult,
};
pub use error::RuntimeError;
pub use executor::{
    CancellationDispatch, CapabilityResolutionContext, DeterministicExecutor, EffectAction,
    ExecutionDispatch, ExecutionReportBatch, ExecutionReporter, ExecutorError,
    MAX_REPORTS_PER_DISPATCH, ObservationDisposition, ResolvedCapability, TaskExecutor,
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
pub use reconciliation::{
    HistoricalExecutionState, NodeHistory, ReconciliationPlan, plan_reconciliation,
    validate_plan_is_fresh,
};
pub use scheduler::{
    AdmissionRequest, AdmissionUsage, EvaluationContext, RetryPolicy, SchedulerLimits,
    evaluate_condition, select_fair_runnable,
};
