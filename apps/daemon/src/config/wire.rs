//! Strict schema-9 TOML input and owner-specific configuration choices.
use milkdrift_authority::{
    ArtifactAuthorityScope, AuthorityBudget, BoundaryTimeMillis, CapabilityAuthorityScope,
    DaemonAuthorityScope, FilesystemScope, LayoutAuthorityScope, NetworkProfileRef, NetworkScope,
    PeerAuthorityScope, ResourceScope, SecretRef, WorkflowRunScope, WorkspaceAuthorityScope,
};
use milkdrift_capability::SideEffectClass;
use milkdrift_peer_protocol::{PROTOCOL_MINOR_V1, PeerAction};
use milkdrift_workspace::ArtifactSensitivity;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, collections::BTreeSet, net::SocketAddr, path::PathBuf};

/// Named non-inline secret source.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum SecretSourceConfig {
    /// Resolve the exact named process environment variable at each use.
    Environment {
        /// Explicit environment variable name; ambient enumeration is never used.
        variable: String,
    },
    /// Read a restricted local file at each use, permitting rotation without restart.
    File {
        /// Config-relative or absolute credential file.
        path: PathBuf,
    },
}

/// Chooses the operation set used when compiling an actor's grant.
///
/// Resource selectors, budgets, validity, and revocation come from the actor binding separately.
/// A controller preset therefore cannot authorize a resource omitted from that binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityPresetConfig {
    /// Read-only inspection.
    Observer,
    /// Inspection plus prospective proposal submission.
    Advisor,
    /// Operational run controls and approvals.
    Supervisor,
    /// Full local workflow control within configured host bounds.
    Controller,
}

/// One credential-reference to immutable actor/grant mapping.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorBindingConfig {
    /// Key into [`DaemonConfig::secret_sources`].
    pub credential_ref: String,
    /// Server-owned actor identity.
    pub actor: String,
    /// Immutable grant identity.
    pub grant_id: String,
    /// Immutable grant revision.
    pub grant_revision: u64,
    /// Revocation generation required by every command.
    pub revocation_generation: u64,
    /// Expanded authority preset.
    pub preset: AuthorityPresetConfig,
    /// Explicit ordinary resource scope, numeric ceilings, and validity interval.
    pub authority: ActorGrantConfig,
    /// False disables this binding when the configuration is loaded at startup.
    pub enabled: bool,
}

/// Resource scope, limits, and validity accompanying an actor's operation preset.
///
/// Keep these facts explicit even for a read-only preset. Advancing a grant's content requires
/// a new `grant_revision`; a running daemon adopts that configuration on restart.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorGrantConfig {
    /// Exact workflow/run, capability, filesystem, network, and secret scope.
    pub resources: ResourceScope,
    /// Explicit numeric ceilings. `None` means the resource is not granted.
    pub budget: AuthorityBudget,
    /// Inclusive grant validity start.
    pub valid_from: BoundaryTimeMillis,
    /// Inclusive finite validity end.
    pub valid_until: BoundaryTimeMillis,
    /// Required visual acknowledgement for wildcard/unknown/unbounded administration.
    #[serde(default)]
    pub dangerous_allow_broad_authority: bool,
}

impl ActorGrantConfig {
    /// Deliberately constructs visually broad administration for migration/tests.
    #[must_use]
    pub fn dangerous_administrator() -> Self {
        #[cfg(unix)]
        let filesystem = vec![FilesystemScope::dangerous_all_access_unix_root()];
        #[cfg(windows)]
        let filesystem = FilesystemScope::dangerous_all_access_windows_drives();
        #[cfg(not(any(unix, windows)))]
        let filesystem = Vec::new();
        Self {
            resources: ResourceScope {
                workflow_run: WorkflowRunScope::Any,
                capability: CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
                filesystem,
                network: NetworkScope::empty(),
                secrets: BTreeSet::new(),
                artifacts: ArtifactAuthorityScope::dangerous_all(),
                layouts: LayoutAuthorityScope::dangerous_all(),
                peers: PeerAuthorityScope::dangerous_all(),
                daemon: DaemonAuthorityScope::dangerous_all(),
                workspace: WorkspaceAuthorityScope::dangerous_all_in_run(),
            },
            budget: AuthorityBudget {
                cost_minor: Some(u64::MAX),
                duration_ms: Some(u64::MAX),
                invocations: Some(u64::MAX),
                artifact_bytes: Some(u64::MAX),
                units: Some(u64::MAX),
                concurrency: Some(u32::MAX),
            },
            valid_from: BoundaryTimeMillis::new(0),
            valid_until: BoundaryTimeMillis::new(u64::MAX),
            dangerous_allow_broad_authority: true,
        }
    }
}

/// Fixed runtime-owner, scheduler, and effect-worker bounds.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHostConfig {
    /// Bounded synchronous owner queue.
    pub request_queue: u32,
    /// Scheduler/effect notification maintenance maximum interval.
    pub maintenance_interval_ms: u64,
    /// Runtime scheduler page bound.
    pub maximum_tick_items: u16,
    /// Global active lease bound.
    pub global_concurrency: u32,
    /// Per-run active lease bound.
    pub per_run_concurrency: u32,
    /// Per-branch active lease bound.
    pub per_branch_concurrency: u32,
    /// Default operation-class active lease bound.
    pub per_capability_concurrency: u32,
    /// External invocation worker threads.
    pub effect_threads: u16,
    /// Bounded external invocation queue.
    pub effect_queue: u16,
    /// Bounded cancellation queue.
    pub cancellation_queue: u16,
    /// Durable effect claims per notification.
    pub maximum_effect_claim: u16,
    /// Lease duration recorded by runtime.
    pub lease_duration_ms: u64,
}

impl Default for RuntimeHostConfig {
    fn default() -> Self {
        Self {
            request_queue: 128,
            maintenance_interval_ms: 100,
            maximum_tick_items: 128,
            global_concurrency: 32,
            per_run_concurrency: 8,
            per_branch_concurrency: 4,
            per_capability_concurrency: 8,
            effect_threads: 4,
            effect_queue: 64,
            cancellation_queue: 32,
            maximum_effect_claim: 32,
            lease_duration_ms: 30_000,
        }
    }
}

/// Limits recent receipt storage while preserving old command results for exact replay.
///
/// Archival moves complete receipts to cold storage; these settings do not bound total history
/// or expire command IDs. Monitor disk capacity as cold receipts accumulate.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationReceiptConfig {
    /// Maximum recent receipt documents retained in the hot operational tier.
    pub hot_receipt_bound: u32,
    /// Maximum oldest receipts moved atomically by one archival transaction.
    pub archive_batch_size: u32,
}

impl Default for ApplicationReceiptConfig {
    fn default() -> Self {
        Self {
            hot_receipt_bound: 10_000,
            archive_batch_size: 256,
        }
    }
}

/// Configured adapter profile sources containing no secret values.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterConfig {
    /// Versioned local-process profile documents.
    pub process_profiles: Vec<PathBuf>,
    /// Model capability identity/profile sources.
    pub model_profiles: Vec<ModelProfileConfig>,
}

/// Explicit peer-host deployment state.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "mode")]
pub enum PeerHostConfig {
    /// No peer authentication realm, relationships, workers, or remote registrations.
    #[default]
    Disabled,
    /// One exact local identity with explicit relationships and serving policy.
    Enabled {
        /// Stable identity of this daemon.
        local_peer_id: String,
        /// Explicit operator-configured relationships. Empty exposes nothing.
        #[serde(default)]
        relationships: Vec<PeerRelationshipConfig>,
        /// Independent serving-peer worker, capacity, recovery, and observation-retention policy.
        #[serde(default)]
        serving: PeerServingConfig,
    },
}

/// Worker, admission, and history limits for work this daemon accepts from peers.
///
/// Active work reserves room for its eventual terminal record. Old eligible terminal detail
/// can become a compact tombstone, retaining request replay/conflict facts after progress rows
/// expire. This retention policy is independent of local application receipts and artifacts.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerServingConfig {
    /// Fixed serving worker thread count.
    pub worker_threads: u16,
    /// Global accepted nonterminal ceiling.
    pub maximum_global_active: u32,
    /// Durable pre-entry dispatch queue ceiling.
    pub maximum_dispatch_queue: u32,
    /// Complete terminal/uncertain records retaining detailed observations.
    pub maximum_hot_terminal_records: u64,
    /// Maximum records compacted in one atomic archival transaction.
    pub archive_batch_size: u32,
    /// Minimum terminal age before detailed observation rows are compacted.
    pub observation_hot_retention_ms: u64,
    /// Maximum prior-owner claims recovered in one transaction.
    pub recovery_page: u16,
    /// Idle durable dispatch poll interval.
    pub poll_interval_ms: u64,
}

impl Default for PeerServingConfig {
    fn default() -> Self {
        Self {
            worker_threads: 4,
            maximum_global_active: 256,
            maximum_dispatch_queue: 256,
            maximum_hot_terminal_records: 10_000,
            archive_batch_size: 256,
            observation_hot_retention_ms: 86_400_000,
            recovery_page: 128,
            poll_interval_ms: 100,
        }
    }
}

/// Connects a known peer identity and endpoint to explicit operations, resources, and quotas.
///
/// Both daemons need corresponding relationship configuration. Catalog reload refreshes remote
/// registrations; changing these configured authority facts requires a validated restart.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerRelationshipConfig {
    /// Exact authenticated remote peer identity.
    pub peer_id: String,
    /// Fixed endpoint; workflow/model input cannot replace it.
    pub endpoint: String,
    /// Key into [`DaemonConfig::secret_sources`].
    pub credential_ref: String,
    /// Explicit development-only plaintext loopback exception.
    #[serde(default)]
    pub insecure_loopback_development: bool,
    /// Protocol minimum minor compatibility assertion; must equal the current minor.
    #[serde(default = "default_peer_protocol_minor")]
    pub minimum_minor: u16,
    /// Protocol maximum minor compatibility assertion; must equal the current minor.
    #[serde(default = "default_peer_protocol_minor")]
    pub maximum_minor: u16,
    /// Exact allowed protocol action families; empty denies all.
    #[serde(default)]
    pub actions: BTreeSet<PeerAction>,
    /// Exact capability allowlist; empty advertises and invokes nothing.
    #[serde(default)]
    pub capability_allow: BTreeSet<String>,
    /// Exact capability denylist applied after allow matching.
    #[serde(default)]
    pub capability_deny: BTreeSet<String>,
    /// Exact operation allowlist; empty advertises and invokes nothing.
    #[serde(default)]
    pub operation_allow: BTreeSet<String>,
    /// Maximum side-effect class accepted from this peer.
    #[serde(default)]
    pub maximum_side_effect: PeerSideEffectConfig,
    /// Explicit host filesystem authority available to allowed remote capabilities.
    #[serde(default)]
    pub execution_filesystem: Vec<FilesystemScope>,
    /// Explicit credential-free network profiles available to allowed remote capabilities.
    #[serde(default)]
    pub execution_network_profiles: BTreeSet<NetworkProfileRef>,
    /// Explicit network destinations available to allowed remote capabilities.
    #[serde(default)]
    pub execution_network_destinations: BTreeSet<String>,
    /// Explicit daemon secret references available to allowed remote capabilities.
    #[serde(default)]
    pub execution_secrets: BTreeSet<SecretRef>,
    /// Maximum simultaneous accepted remote executions.
    #[serde(default = "default_peer_concurrency")]
    pub maximum_concurrent: u16,
    /// Maximum authenticated requests per minute for each action/operation bucket.
    #[serde(default = "default_peer_requests_per_minute")]
    pub maximum_requests_per_minute: u32,
    /// Maximum artifact bytes per execution.
    #[serde(default = "default_peer_artifact_bytes")]
    pub maximum_artifact_bytes: u64,
    /// Explicit transferable artifact sensitivity classes; empty denies artifact transfer.
    pub artifact_sensitivities: BTreeSet<ArtifactSensitivity>,
    /// Maximum execution duration.
    #[serde(default = "default_peer_duration_ms")]
    pub maximum_duration_ms: u64,
    /// Maximum observed cost in millionths.
    #[serde(default)]
    pub maximum_cost_micros: u64,
    /// Maximum semantic observations retained for one execution.
    #[serde(default = "default_peer_observations")]
    pub maximum_observations: u32,
    /// Expiring catalog TTL.
    #[serde(default = "default_peer_catalog_ttl_ms")]
    pub catalog_ttl_ms: u64,
    /// Policy trust zone added to local remote adapter registrations.
    pub trust_zone: String,
    /// Opaque configured server-side delegation reference.
    pub delegation_ref: String,
    /// Relationship revocation generation.
    #[serde(default)]
    pub revocation_generation: u64,
    /// Hard relationship expiration in Unix epoch milliseconds.
    pub expires_at_unix_ms: u64,
    /// False revokes authentication while retaining audit configuration.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Configuration representation of the maximum permitted side effect.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerSideEffectConfig {
    /// Pure operation only.
    #[default]
    None,
    /// Protected or external reads.
    ReadOnly,
    /// Keyed idempotent writes.
    IdempotentWrite,
    /// Potentially non-idempotent writes.
    NonIdempotentWrite,
    /// Unknown side effects.
    Unknown,
}

const fn default_peer_concurrency() -> u16 {
    4
}

const fn default_peer_protocol_minor() -> u16 {
    PROTOCOL_MINOR_V1
}

const fn default_peer_requests_per_minute() -> u32 {
    600
}

const fn default_peer_artifact_bytes() -> u64 {
    64 * 1_048_576
}

const fn default_peer_duration_ms() -> u64 {
    300_000
}

const fn default_peer_observations() -> u32 {
    10_000
}

const fn default_peer_catalog_ttl_ms() -> u64 {
    30_000
}

const fn default_true() -> bool {
    true
}

/// One model capability and provider-neutral endpoint profile source.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProfileConfig {
    /// Capability identity advertised to workflows.
    pub capability_id: String,
    /// Versioned non-secret endpoint profile document.
    pub profile: PathBuf,
}

/// How long shutdown allows owned work to drain and what to do with outstanding effects.
///
/// A deadline or cancellation request does not establish an external terminal outcome. The host
/// reports retained or unresolved work when it cannot finish the chosen policy cleanly.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownConfig {
    /// Total drain deadline.
    pub deadline_ms: u64,
    /// Drain, cancel, or retain already claimed external effects.
    pub effect_policy: ShutdownEffectPolicy,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            deadline_ms: 10_000,
            effect_policy: ShutdownEffectPolicy::Drain,
        }
    }
}

/// Effect disposition at orderly shutdown.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownEffectPolicy {
    /// Finish queued/running effects until the deadline.
    Drain,
    /// Request adapter cancellation and retain unresolved identities.
    Cancel,
    /// Do not enter queued effects; recover them on restart.
    Retain,
}

/// Operator choices for one daemon, normally read from TOML with [`Self::load`].
///
/// Rust callers may assemble the public fields and call [`Self::validate`] with an explicit
/// base directory. Both routes produce the same immutable [`super::DaemonPlan`]; profile paths
/// name separately owned documents, and secret sources contain references rather than values.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonConfig {
    /// Exact configuration schema version.
    pub schema_version: u32,
    /// Owned redb/artifact/application-state root.
    pub data_root: PathBuf,
    /// Local plaintext HTTP listener.
    pub bind: SocketAddr,
    /// Explicit secret reference sources; values are never present here.
    pub secret_sources: BTreeMap<String, SecretSourceConfig>,
    /// Credential-to-actor mappings.
    pub actors: Vec<ActorBindingConfig>,
    /// Fixed runtime/worker bounds.
    #[serde(default)]
    pub runtime: RuntimeHostConfig,
    /// Explicit adapter sources.
    #[serde(default)]
    pub adapters: AdapterConfig,
    /// Explicit default-disabled authenticated peer relationships.
    #[serde(default)]
    pub peers: PeerHostConfig,
    /// Ordered shutdown policy.
    #[serde(default)]
    pub shutdown: ShutdownConfig,
    /// Bounded hot lifecycle; cold exact replay grows until physical storage is exhausted.
    #[serde(default)]
    pub application_receipts: ApplicationReceiptConfig,
    /// Independently retained security-audit prefix bound.
    #[serde(default = "default_security_audit_record_bound")]
    pub security_audit_record_bound: u32,
}

const fn default_security_audit_record_bound() -> u32 {
    10_000
}
