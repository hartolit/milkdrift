//! Connect durable runtime work to live process, model, control, and peer adapters.
//!
//! [`CapabilityHost`] implements runtime's `TaskExecutor`: it selects authorized generations
//! and holds an exact-generation permit during entry. Implement [`CapabilityAdapter`] to supply
//! an external mechanism, and use [`AdapterReporter`] to return durable observations rather
//! than changing workflow state directly.
//!
//! [`EffectWorkerHost`] supplies explicitly polled, bounded execution and cancellation queues.
//! [`InvocationDataAccess`] and [`SecretResolver`] provide the bytes an authorized adapter needs.
//! The embedding daemon owns startup, health refresh, and shutdown ordering; the package README
//! follows that consumer through a complete invocation.

mod adapter;
#[cfg(any(test, feature = "test-support"))]
pub mod conformance;
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
