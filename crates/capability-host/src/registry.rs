use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use milkdrift_authority::{CapabilityExecutionRequirements, PeerId};
use milkdrift_capability::{
    CapabilityCategory, CapabilityDescriptor, CapabilityId, CapabilityObservation,
    ExecutionTrustClass, InvocationId, Locality, OperationId, ProviderProfileRef, TrustZone,
};
use thiserror::Error;

mod execution;
mod lifecycle;
mod selection;

use crate::{AdapterError, CapabilityAdapter};

/// Bounded host configuration; no hidden queue is implemented in this pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostConfig {
    /// Maximum exact registrations across every identity.
    pub max_registrations: usize,
    /// Maximum retained revisions for one capability identity.
    pub max_generations_per_capability: usize,
    /// Host cap applied in addition to descriptor-advertised concurrency.
    pub max_concurrent_per_generation: u32,
    /// Maximum age of a live observation used for new resolution.
    pub observation_stale_after_ms: u64,
}

impl HostConfig {
    /// Validates nonzero defensive bounds.
    pub fn validate(self) -> Result<Self, HostError> {
        if self.max_registrations == 0
            || self.max_generations_per_capability == 0
            || self.max_concurrent_per_generation == 0
            || self.observation_stale_after_ms == 0
        {
            return Err(HostError::InvalidConfig);
        }
        Ok(self)
    }
}

/// Authority and deterministic priority facts applied during live resolution.
#[derive(Clone, Debug)]
pub struct CapabilitySelectionPolicy {
    priorities: BTreeMap<CapabilityId, i32>,
}

impl CapabilitySelectionPolicy {
    /// Constructs deterministic host priorities without any host-wide authority policy.
    #[must_use]
    pub fn priorities(priorities: BTreeMap<CapabilityId, i32>) -> Self {
        Self { priorities }
    }
}

/// Result of registering immutable descriptor facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistrationOutcome {
    /// A new exact generation became visible.
    Registered,
    /// Byte-identical descriptor facts were already registered.
    Idempotent,
}

/// Derived health state in an immutable registry read model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationHealth {
    /// No observation has been recorded.
    Unknown,
    /// The latest observation is available and current.
    Healthy,
    /// The latest observation explicitly reports unavailable.
    Unhealthy,
    /// The latest observation is older than the configured policy.
    Stale,
}

/// Immutable bounded generation view for future daemon clients.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationView {
    /// Capability identity.
    pub capability: CapabilityId,
    /// Exact descriptor revision.
    pub descriptor_revision: u64,
    /// Canonical descriptor digest.
    pub descriptor_digest: String,
    /// Discovery category used during scoped selection.
    pub category: CapabilityCategory,
    /// Exact advertised operation identities.
    pub operations: BTreeSet<OperationId>,
    /// Optional provider/model profile identity.
    pub provider_profile: Option<ProviderProfileRef>,
    /// Local, peer, remote-provider, or unspecified placement.
    pub locality: Locality,
    /// Authenticated owning peer for peer-local generations.
    pub peer: Option<PeerId>,
    /// Complete advertised trust-zone set.
    pub trust_zones: BTreeSet<TrustZone>,
    /// Exact execution-isolation/trust class.
    pub execution_trust: ExecutionTrustClass,
    /// Whether this is the current resolution generation.
    pub current: bool,
    /// Whether new unpinned resolution is closed.
    pub draining: bool,
    /// Derived current health.
    pub health: GenerationHealth,
    /// Latest observation time, when present.
    pub observed_at_unix_ms: Option<u64>,
    /// Latest adapter availability, when present.
    pub available: Option<bool>,
    /// Actual held permits.
    pub active_permits: u32,
    /// Enforced concurrent permit limit.
    pub permit_limit: u32,
    /// Bounded last failure summary.
    pub last_failure: Option<String>,
}

/// Immutable descriptor plus live state used by an external catalog adapter.
///
/// This is an observation of adapter-host state, never durable workflow truth.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogGenerationView {
    /// Exact immutable descriptor generation.
    pub descriptor: CapabilityDescriptor,
    /// Immutable filesystem, network, secret, and budget facts declared by the adapter.
    pub authority_requirements: CapabilityExecutionRequirements,
    /// Latest live health observation, when recorded.
    pub observation: Option<CapabilityObservation>,
    /// Whether this generation is selected for unpinned local resolution.
    pub current: bool,
    /// Whether new resolution is closed while exact owners drain.
    pub draining: bool,
}

/// Honest result of forced shutdown when exact invocations remained unresolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShutdownReport {
    /// Invocation identities still owned when adapters were forcibly removed.
    pub unresolved_invocations: Vec<InvocationId>,
}

/// Typed live-host lifecycle and registry failure.
#[derive(Debug, Error)]
pub enum HostError {
    /// Host bounds are zero or inconsistent.
    #[error("invalid capability-host configuration")]
    InvalidConfig,
    /// Registry lock was poisoned by a prior failure.
    #[error("capability registry is unavailable")]
    RegistryUnavailable,
    /// Registration count or retained-generation bound was reached.
    #[error("capability registry bound reached: {0}")]
    RegistryBound(&'static str),
    /// The same identity/revision was reused with different descriptor bytes.
    #[error("conflicting capability descriptor revision {capability}/{descriptor_revision}")]
    ConflictingRevision {
        /// Capability identity.
        capability: CapabilityId,
        /// Reused revision.
        descriptor_revision: u64,
    },
    /// Exact generation is not registered.
    #[error("capability generation {capability}/{descriptor_revision} is unavailable")]
    GenerationUnavailable {
        /// Capability identity.
        capability: CapabilityId,
        /// Descriptor revision.
        descriptor_revision: u64,
    },
    /// Descriptor and observation identities disagree.
    #[error("capability observation does not match its descriptor generation")]
    ObservationMismatch,
    /// Observation time moved backwards.
    #[error("capability observation time moved backwards")]
    ObservationRegressed,
    /// Generation still owns work and cannot be removed gracefully.
    #[error("capability generation still owns {0} in-flight invocation(s)")]
    InFlight(u32),
    /// Adapter lifecycle hook returned a bounded failure.
    #[error(transparent)]
    Adapter(#[from] AdapterError),
    /// Adapter lifecycle hook panicked.
    #[error("adapter lifecycle hook panicked")]
    AdapterPanicked,
    /// Capability descriptor canonicalization failed.
    #[error("invalid capability descriptor: {0}")]
    Descriptor(String),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GenerationKey {
    capability: CapabilityId,
    revision: u64,
}

struct Generation {
    descriptor: CapabilityDescriptor,
    descriptor_digest: String,
    adapter: Arc<dyn CapabilityAdapter>,
    authority_requirements: CapabilityExecutionRequirements,
    observation: Option<CapabilityObservation>,
    draining: bool,
    active: u32,
    permit_limit: u32,
    last_failure: Option<String>,
}

struct RegistryState {
    admission_open: bool,
    shutdown: bool,
    generations: BTreeMap<GenerationKey, Generation>,
    current: BTreeMap<CapabilityId, u64>,
    in_flight: BTreeMap<InvocationId, GenerationKey>,
}

struct HostCore {
    config: HostConfig,
    policy: CapabilitySelectionPolicy,
    state: Mutex<RegistryState>,
}

/// Embeddable live registry and runtime `TaskExecutor` bridge.
#[derive(Clone)]
pub struct CapabilityHost {
    core: Arc<HostCore>,
}
