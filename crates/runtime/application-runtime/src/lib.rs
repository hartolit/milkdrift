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
mod unload;

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
pub use unload::ModelUnloadBehavior;
