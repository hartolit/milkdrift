//! Live, generation-safe capability adapter hosting above the durable runtime.
//!
//! Descriptors remain immutable semantic facts. Health, admission, draining, ownership,
//! adapter handles, and secret resolution remain bounded live host state.

mod adapter;
mod materialization;
mod registry;
mod secret;
mod worker;

pub use adapter::{
    AdapterError, AdapterExecutionContext, AdapterFailureKind, AdapterInvocation, AdapterReporter,
    CapabilityAdapter, HostAdapterContractError,
};
pub use materialization::{
    InputMaterialization, InvocationDataAccess, InvocationDataError,
    MATERIALIZATION_SCHEMA_VERSION_V1, MaterializationLimits, MaterializedExecution,
    StoreInvocationDataAccess,
};
pub use registry::{
    CapabilityHost, CapabilitySelectionPolicy, CatalogGenerationView, GenerationHealth,
    GenerationView, HostConfig, HostError, RegistrationOutcome, ShutdownReport,
};
#[cfg(any(test, feature = "test-support"))]
pub use secret::InMemorySecretResolver;
pub use secret::{SecretResolver, SecretResolverError};
pub use worker::{
    EffectPollReport, EffectShutdownMode, EffectWorkerConfig, EffectWorkerError,
    EffectWorkerHealth, EffectWorkerHost, EffectWorkerShutdown,
};
