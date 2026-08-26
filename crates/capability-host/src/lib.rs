//! Live, generation-safe capability adapter hosting above the durable runtime.
//!
//! Descriptors remain immutable semantic facts. Health, admission, draining, ownership,
//! adapter handles, and secret resolution remain bounded live host state.

mod adapter;
mod registry;
mod secret;

pub use adapter::{
    AdapterError, AdapterFailureKind, AdapterInvocation, AdapterReporter, CapabilityAdapter,
    HostAdapterContractError,
};
pub use registry::{
    CapabilityHost, CapabilitySelectionPolicy, GenerationHealth, GenerationView, HostConfig,
    HostError, RegistrationOutcome, ShutdownReport,
};
pub use secret::{InMemorySecretResolver, SecretResolver, SecretResolverError};
