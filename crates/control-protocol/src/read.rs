use super::{Cursor, Deserialize, ProtocolVersion, Serialize};

/// Services constructed by one daemon installation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostRole {
    /// Capability serving without workflow runtime, control, or scheduler workers.
    ExecutionOnly,
    /// Capability serving together with workflow ownership.
    WorkflowEnabled,
}

/// Daemon liveness/readiness/draining state.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthRead {
    /// Configured services available on this host.
    pub role: HostRole,
    /// Stable lifecycle state.
    pub state: DaemonState,
    /// Process is alive.
    pub live: bool,
    /// Recovery completed and commands may be considered.
    pub ready: bool,
    /// New mutations are refused.
    pub draining: bool,
    /// Current request queue occupancy.
    pub queued_requests: u32,
    /// Fixed request queue bound.
    pub request_queue_capacity: u32,
    /// Current effect workers executing external work.
    pub active_effects: u32,
    /// Redacted last startup/worker failure.
    pub last_failure: Option<String>,
    /// Serving-peer active/hot/tombstone lifecycle facts without per-peer identities.
    pub peer_executions: PeerExecutionHealthRead,
}

/// Redacted serving-peer execution retention and archival health.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerExecutionHealthRead {
    /// Whether serving-peer execution is enabled.
    pub enabled: bool,
    /// Accepted nonterminal execution count.
    pub active_count: u32,
    /// Configured global active ceiling.
    pub active_bound: u32,
    /// Active pre-entry dispatch queue count.
    pub dispatch_queued: u32,
    /// Configured durable dispatch queue ceiling.
    pub dispatch_bound: u32,
    /// Complete terminal/uncertain records retaining detailed observations.
    pub hot_terminal_count: u64,
    /// Configured hot terminal ceiling, including capacity reserved by active work.
    pub hot_terminal_bound: u64,
    /// Compact immutable archived request identities.
    pub tombstone_count: u64,
    /// Maximum records compacted in one transaction.
    pub archive_batch_size: u32,
    /// Minimum terminal age before detailed observations are compacted.
    pub observation_hot_retention_ms: u64,
    /// Monotonic successful archive generation.
    pub archive_generation: u64,
    /// Most recent successful archival boundary.
    pub last_archived_at_unix_ms: Option<u64>,
    /// Whether the last archival/status operation failed.
    pub archival_degraded: bool,
    /// Redacted archival failure classification.
    pub last_archival_failure: Option<String>,
}

/// Stable daemon lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonState {
    /// Configuration and resources are being initialized.
    Starting,
    /// Recovery and adapter initialization completed.
    Ready,
    /// Authenticated recovery controls are available; execution is permanently disabled.
    Recovery,
    /// Shutdown has closed admission.
    Draining,
    /// Host has stopped its owned services.
    Stopped,
    /// Startup failed before readiness.
    Failed,
}

/// Redacted provider-neutral contract for one capability operation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityOperationRead {
    /// Exact namespaced operation identity.
    pub operation: String,
    /// Declared maximum external side-effect class.
    pub side_effect: String,
    /// Advertised provider idempotency behavior.
    pub idempotency: String,
    /// Advertised cancellation behavior.
    pub cancellation: String,
    /// Complete advertised streaming modes.
    pub streaming: Vec<String>,
}

/// Capability generation and health observation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRead {
    /// Capability identity.
    pub capability_id: String,
    /// Descriptor revision.
    pub generation: u64,
    /// Descriptor digest.
    pub descriptor_digest: String,
    /// Stable discovery category.
    pub category: String,
    /// Exact advertised operation identities.
    pub operations: Vec<String>,
    /// Exact provider-neutral contract for every advertised operation.
    pub operation_contracts: Vec<CapabilityOperationRead>,
    /// Optional provider/model profile identity.
    pub provider_profile: Option<String>,
    /// Stable execution-locality label.
    pub locality: String,
    /// Authenticated remote peer identity, when peer-owned.
    pub peer_id: Option<String>,
    /// Complete advertised trust-zone labels.
    pub trust_zones: Vec<String>,
    /// Exact execution-isolation/trust class.
    pub execution_trust: String,
    /// Whether this generation is selected for new work.
    pub current: bool,
    /// Whether new work is refused.
    pub draining: bool,
    /// Stable health label.
    pub health: String,
    /// Last known availability.
    pub available: Option<bool>,
    /// Active/maximum permits.
    pub active_permits: u32,
    /// Admission permit bound.
    pub permit_limit: u32,
}

/// Safe authenticated peer/catalog status without credentials or transport internals.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerRead {
    /// Configured authenticated remote peer identity.
    pub peer_id: String,
    /// Whether an authenticated session and live verified catalog are current.
    pub connected: bool,
    /// Stable redacted health classification.
    pub health: String,
    /// Current authenticated remote daemon session identity.
    pub session_id: Option<String>,
    /// Current verified catalog generation.
    pub catalog_generation: Option<u64>,
    /// Current verified catalog digest.
    pub catalog_digest: Option<String>,
    /// Current ordinary capability-host registration count.
    pub registered_capabilities: usize,
    /// Current catalog hard expiry, when connected.
    pub catalog_expires_at_unix_ms: Option<u64>,
    /// Whether the live relationship was administratively revoked.
    pub revoked: bool,
}

/// Safe artifact metadata without a filesystem path.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMetadataRead {
    /// Artifact identity.
    pub artifact_id: String,
    /// Content digest.
    pub digest: String,
    /// Exact byte size.
    pub size: u64,
    /// Declared media type.
    pub content_type: String,
    /// Safe suggested download name.
    pub disposition_name: Option<String>,
    /// Stable sensitivity label.
    pub sensitivity: String,
}

/// Public actor/grant observation returned only under inspect authority.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityRead {
    /// Stable serving installation identity, independent of its current transport address.
    pub host: String,
    /// Server-owned actor identity.
    pub actor: String,
    /// Exact configured grant identity.
    pub grant_id: String,
    /// Immutable grant revision.
    pub grant_revision: u64,
    /// Digest of the exact immutable grant, for explicit service publication relationships.
    pub grant_digest: String,
    /// Current revocation generation.
    pub revocation_generation: u64,
    /// Stable operation names allowed by configuration.
    pub operations: Vec<String>,
}

/// A resumable, externally projected stream item.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationEnvelope {
    /// Protocol version.
    pub protocol: ProtocolVersion,
    /// Feed-bound monotonically ordered cursor.
    pub cursor: Cursor,
    /// Observation timestamp, independent from heartbeats.
    pub observed_at_ms: u64,
    /// Exact filtered feed identity.
    pub feed: String,
    /// External read-model payload.
    pub observation: Observation,
}

/// Closed stream observation vocabulary.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "value",
    deny_unknown_fields
)]
pub enum Observation {
    /// Complete current authorized capability catalogue. Replace the previous set, including
    /// on an empty snapshot; identity and generation together distinguish retained generations.
    CapabilitySnapshot(Vec<CapabilityRead>),
    /// Daemon lifecycle/health change.
    DaemonHealth(HealthRead),
    /// Server is shutting down this stream.
    StreamClosing {
        /// Redacted explanation of why this stream is closing.
        reason: String,
    },
    /// Subscriber fell outside retained stream history.
    ResyncRequired {
        /// Redacted explanation of why the client must reload bounded current state.
        reason: String,
    },
}
