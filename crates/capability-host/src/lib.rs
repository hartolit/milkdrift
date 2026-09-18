//! Connect authorized host invocations and durable workflow work to external adapters.
//!
//! [`CapabilityHost`] implements runtime's `TaskExecutor`: it selects authorized generations
//! and holds an exact-generation permit during entry. Implement [`CapabilityAdapter`] to supply
//! an external mechanism, and use [`AdapterReporter`] to return durable observations rather
//! than changing workflow state directly.
//! [`PeerService`] owns durable serving acceptance, prepared entry, recovery and observations
//! for authenticated direct clients and delegated peers, without constructing a workflow runtime.
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
mod serving;
mod worker;

pub use adapter::{
    AdapterError, AdapterExecutionContext, AdapterFailureKind, AdapterInputSelection,
    AdapterInvocation, AdapterReporter, CapabilityAdapter, DirectInputSelection,
    HostAdapterContractError, PreparedAdapterExecution, WorkflowExecutionContext,
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
#[cfg(any(test, feature = "test-support"))]
pub use serving::SystemPeerClock;
pub use serving::{
    CorePeerArtifactStore, PeerArtifactError, PeerArtifactStore, PeerArtifactTransferFacts,
    PeerAuthenticator, PeerClock, PeerClockError, PeerRelationship, PeerServerConfig, PeerService,
    PeerWorkerConfig, PeerWorkerShutdownReport, ServingClientPolicy, ServingError,
};
pub use worker::{
    EffectPollReport, EffectShutdownMode, EffectWorkerConfig, EffectWorkerError,
    EffectWorkerHealth, EffectWorkerHost, EffectWorkerShutdown,
};
