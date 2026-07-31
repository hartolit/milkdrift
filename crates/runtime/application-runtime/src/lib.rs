//! Frontend-neutral orchestration for local model acquisition, lifecycle, and generation.

#![forbid(unsafe_code)]

mod chat;
mod configuration;
mod conversation;
mod error;
mod event;
mod generation;
mod hub_worker;
mod local;
mod runtime;
mod selection;
mod shutdown;
mod state;
mod support;
mod unload;

pub use chat::{ChatCompatibility, ContextDiagnostics, PromptCompatibilityProfile};
pub use configuration::{
    ApplicationHubConfiguration, ApplicationPreferences, ApplicationRuntimeConfiguration,
    ApplicationTiming,
};
pub use conversation::{
    ConversationProvenance, ConversationRecord, ConversationRecordId, ConversationRetention,
    ConversationRole, ConversationTokenEstimate, ResponseAttempt, ResponseAttemptId,
    ResponseAttemptState,
};

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
pub use selection::{
    ApplicationDevice, ApplicationEngine, ApplicationModelFormat, ApplicationScalarType,
    ApplicationSource, ImmutableModelIdentity, ModelSelection,
};
pub use state::{
    ApplicationActivity, ApplicationState, GenerationPhase, GenerationSummary, GenerationTerminal,
    GenerationTerminalOutcome, LoadedModel, ResolvedModel,
};
pub use unload::ModelUnloadBehavior;
