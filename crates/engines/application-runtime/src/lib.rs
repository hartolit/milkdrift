//! Frontend-neutral orchestration for local model acquisition, lifecycle, and generation.

#![forbid(unsafe_code)]

mod configuration;
mod error;
mod event;
mod generation;
mod hub_worker;
mod runtime;
mod shutdown;
mod state;
mod support;

pub mod workflow;

pub use configuration::{
    ApplicationHubConfiguration, ApplicationPreferences, ApplicationRuntimeConfiguration,
    ApplicationTiming,
};
pub use domain_contracts::ScalarType;
pub use error::{
    ApplicationConfigurationField, ApplicationError, ApplicationFailure, ApplicationFailureKind,
    ApplicationWorker, GenerationSettingsField,
};
pub use event::ApplicationEvent;
pub use generation::{
    ApplicationOutputBatch, ApplicationOutputRecord, ApplicationOutputRecordKind,
    ApplicationOutputState, ApplicationTextRange, GenerationSeed, GenerationSettings,
    GenerationTerminalKind,
};
pub use runtime::ApplicationRuntime;
pub use state::{
    ApplicationActivity, ApplicationBackend, ApplicationState, GenerationPhase, GenerationSummary,
    GenerationTerminal, GenerationTerminalOutcome, LoadedModel, ResolvedModel,
};
pub use workflow::{
    Artifact, ArtifactContent, ArtifactContentKind, ArtifactId, ArtifactInputs, ArtifactKind,
    ArtifactReference, ArtifactRole, ArtifactStore, CorrectiveWorkflowConfiguration,
    CorrectiveWorkflowExecutor, Diagnostic, DiagnosticLocation, DiagnosticSeverity, ModelPolicy,
    ModelTaskExecutor, ModelTaskRequest, NormalizedValidationReport, RawDiagnostic, TaskAttempt,
    TaskBudget, TaskGraphError, TaskId, TaskKind, ValidationReport, ValidationTaskExecutor,
    ValidationTaskRequest, ValidationVerdict, WorkflowArtifactLimits, WorkflowBudgetClass,
    WorkflowConfigurationError, WorkflowError, WorkflowEvent, WorkflowExecutorLimitError,
    WorkflowExecutorLimits, WorkflowId, WorkflowIdentifierKind, WorkflowOutcome, WorkflowStage,
    WorkflowStatus, normalize_validation_report,
};
