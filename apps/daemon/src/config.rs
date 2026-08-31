use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::SocketAddr,
    path::{Component, Path, PathBuf},
};

use milkdrift_authority::{
    ArtifactAuthorityScope, AuthorityBudget, BoundaryTimeMillis, CapabilityAuthorityScope,
    DaemonAuthorityScope, FilesystemScope, LayoutAuthorityScope, NetworkProfileRef, NetworkScope,
    PeerAuthorityScope, ResourceScope, SecretRef, WorkflowRunScope, WorkspaceAuthorityScope,
};
use milkdrift_capability::SideEffectClass;
use milkdrift_control_protocol::MAX_DOCUMENT_BYTES;
use milkdrift_peer_protocol::PeerAction;
use milkdrift_workspace::ArtifactSensitivity;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Current daemon configuration document version.
pub const DAEMON_CONFIG_SCHEMA_VERSION: u32 = 5;

/// Configuration load or deterministic validation failure.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Configuration bytes could not be read.
    #[error("daemon configuration could not be read: {0}")]
    Read(String),
    /// Configuration JSON is malformed or contains duplicate keys.
    #[error("invalid daemon configuration JSON: {0}")]
    Json(String),
    /// The schema version is unsupported.
    #[error("unsupported daemon configuration version {0}; supported version is 5")]
    UnsupportedVersion(u32),
    /// A host-safety invariant is invalid.
    #[error("invalid daemon configuration: {0}")]
    Invalid(String),
}

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

/// Coarse configured authority preset expanded into an immutable ordinary grant.
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
    /// False revokes authentication immediately without removing audit configuration.
    pub enabled: bool,
}

/// Explicit schema-v5 grant facts; preset names choose operations only.
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
        Self {
            resources: ResourceScope {
                workflow_run: WorkflowRunScope::Any,
                capability: CapabilityAuthorityScope::any(SideEffectClass::Unknown),
                filesystem: vec![milkdrift_authority::FilesystemScope::dangerous_all_access_root()],
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

/// Bounded hot lifecycle for exact application-command receipts.
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

/// Default-disabled peer listener/client configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerHostConfig {
    /// Enables the separate `/peer/v1` authentication realm.
    #[serde(default)]
    pub enabled: bool,
    /// Stable identity of this daemon when peer support is enabled.
    pub local_peer_id: Option<String>,
    /// Explicit operator-configured relationships. Empty exposes nothing.
    #[serde(default)]
    pub relationships: Vec<PeerRelationshipConfig>,
}

/// One operator-configured authenticated remote peer relationship.
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
    /// Allowed protocol minimum minor on major version one.
    #[serde(default)]
    pub minimum_minor: u16,
    /// Allowed protocol maximum minor on major version one.
    #[serde(default)]
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

/// Ordered daemon shutdown policy.
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

/// Complete bounded exact-current daemon host configuration.
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

/// Path-normalized, safety-checked configuration used to open the host.
#[derive(Clone, Debug)]
pub struct ValidatedDaemonConfig {
    /// Original validated document with normalized paths.
    pub document: DaemonConfig,
    /// Directory against which relative sources were resolved.
    pub configuration_directory: PathBuf,
}

impl DaemonConfig {
    /// Loads duplicate-safe bounded JSON and validates before storage is opened.
    pub fn load(path: &Path) -> Result<ValidatedDaemonConfig, ConfigError> {
        let bytes = fs::read(path).map_err(|error| ConfigError::Read(error.kind().to_string()))?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(ConfigError::Invalid(format!(
                "configuration exceeds {MAX_DOCUMENT_BYTES} bytes"
            )));
        }
        let value = milkdrift_contracts::parse_json_without_duplicates(&bytes)
            .map_err(|error| ConfigError::Json(error.to_string()))?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| ConfigError::Json("missing numeric schema_version".to_owned()))?;
        if version != DAEMON_CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedVersion(version));
        }
        let config: Self =
            serde_json::from_value(value).map_err(|error| ConfigError::Json(error.to_string()))?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let parent = parent
            .canonicalize()
            .map_err(|error| ConfigError::Read(error.kind().to_string()))?;
        config.validate(&parent)
    }

    /// Deterministically validates and normalizes a programmatically built config.
    pub fn validate(mut self, base: &Path) -> Result<ValidatedDaemonConfig, ConfigError> {
        if self.schema_version != DAEMON_CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedVersion(self.schema_version));
        }
        if !self.bind.ip().is_loopback() {
            return Err(ConfigError::Invalid(
                "plaintext HTTP bind must be loopback".to_owned(),
            ));
        }
        if self.actors.is_empty() || self.actors.len() > 256 {
            return Err(ConfigError::Invalid(
                "authentication requires 1..=256 actor bindings".to_owned(),
            ));
        }
        if self.secret_sources.is_empty() || self.secret_sources.len() > 512 {
            return Err(ConfigError::Invalid(
                "secret source count must be in 1..=512".to_owned(),
            ));
        }
        validate_runtime(&self.runtime)?;
        if self.shutdown.deadline_ms == 0 || self.shutdown.deadline_ms > 300_000 {
            return Err(ConfigError::Invalid(
                "shutdown deadline must be in 1..=300000 milliseconds".to_owned(),
            ));
        }
        if self.application_receipts.hot_receipt_bound == 0
            || self.application_receipts.hot_receipt_bound > 1_000_000
            || self.application_receipts.archive_batch_size == 0
            || self.application_receipts.archive_batch_size
                > self.application_receipts.hot_receipt_bound
        {
            return Err(ConfigError::Invalid(
                "application hot receipt bound must be in 1..=1000000 and archive batch must be in 1..=hot bound"
                    .to_owned(),
            ));
        }
        if self.security_audit_record_bound == 0 || self.security_audit_record_bound > 1_000_000 {
            return Err(ConfigError::Invalid(
                "security audit record bound must be in 1..=1000000".to_owned(),
            ));
        }
        let base = base
            .canonicalize()
            .map_err(|error| ConfigError::Read(error.kind().to_string()))?;
        self.data_root = normalize_owned_path(&base, &self.data_root)?;
        for source in self.secret_sources.values_mut() {
            match source {
                SecretSourceConfig::Environment { variable } => {
                    if variable.is_empty()
                        || variable.len() > 128
                        || !variable.is_ascii()
                        || !variable
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    {
                        return Err(ConfigError::Invalid(
                            "environment secret source name is invalid".to_owned(),
                        ));
                    }
                }
                SecretSourceConfig::File { path } => {
                    *path = normalize_existing_file(&base, path)?;
                }
            }
        }
        let mut actors = BTreeSet::new();
        let mut grants = BTreeSet::new();
        let mut credential_refs = BTreeSet::new();
        for actor in &self.actors {
            validate_safe_identity("credential_ref", &actor.credential_ref)?;
            validate_safe_identity("actor", &actor.actor)?;
            validate_safe_identity("grant_id", &actor.grant_id)?;
            if actor.grant_revision == 0
                || !actors.insert(&actor.actor)
                || !grants.insert((&actor.grant_id, actor.grant_revision))
                || !credential_refs.insert(&actor.credential_ref)
                || !self.secret_sources.contains_key(&actor.credential_ref)
            {
                return Err(ConfigError::Invalid(
                    "actor, grant, credential reference, or revision mapping is invalid/duplicate"
                        .to_owned(),
                ));
            }
            validate_actor_authority(&actor.authority)?;
        }
        for path in &mut self.adapters.process_profiles {
            *path = normalize_existing_file(&base, path)?;
        }
        for model in &mut self.adapters.model_profiles {
            validate_safe_identity("model capability", &model.capability_id)?;
            model.profile = normalize_existing_file(&base, &model.profile)?;
        }
        validate_peers(&self.peers, &self.secret_sources)?;
        Ok(ValidatedDaemonConfig {
            document: self,
            configuration_directory: base,
        })
    }

    /// Returns redacted effective JSON without any resolved values.
    pub fn redacted_json(&self) -> Result<serde_json::Value, ConfigError> {
        let mut value =
            serde_json::to_value(self).map_err(|error| ConfigError::Json(error.to_string()))?;
        if let Some(sources) = value
            .as_object_mut()
            .and_then(|object| object.get_mut("secret_sources"))
        {
            *sources = serde_json::json!({"configured_references": self.secret_sources.len(), "values": "[redacted]"});
        }
        Ok(value)
    }
}

fn validate_actor_authority(authority: &ActorGrantConfig) -> Result<(), ConfigError> {
    CapabilityAuthorityScope::new(
        authority.resources.capability.identities().clone(),
        authority.resources.capability.categories().clone(),
        authority.resources.capability.operations().clone(),
        authority.resources.capability.provider_profiles().clone(),
        authority.resources.capability.trust_zones().clone(),
        authority.resources.capability.localities().clone(),
        authority.resources.capability.maximum_side_effect(),
    )
    .and_then(|scope| scope.with_peers(authority.resources.capability.peers().clone()))
    .and_then(|scope| {
        scope.with_execution_trust_classes(
            authority
                .resources
                .capability
                .execution_trust_classes()
                .clone(),
        )
    })
    .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    NetworkScope::new(
        authority.resources.network.profiles().clone(),
        authority.resources.network.destinations().clone(),
    )
    .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    for filesystem in &authority.resources.filesystem {
        milkdrift_authority::FilesystemScope::new(
            filesystem.root().to_owned(),
            filesystem.access().clone(),
        )
        .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    }
    ArtifactAuthorityScope::new(
        authority.resources.artifacts.identities().clone(),
        authority.resources.artifacts.sensitivities().clone(),
    )
    .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    LayoutAuthorityScope::new(
        authority.resources.layouts.revisions().clone(),
        authority.resources.layouts.actors().clone(),
        authority.resources.layouts.shared(),
    )
    .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    PeerAuthorityScope::new(
        authority.resources.peers.identities().clone(),
        authority.resources.peers.allows_any(),
    )
    .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    WorkspaceAuthorityScope::new(
        authority.resources.workspace.scopes().clone(),
        authority.resources.workspace.allows_any_in_run(),
    )
    .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    if authority.valid_from > authority.valid_until {
        return Err(ConfigError::Invalid(
            "actor authority validity interval is inverted".to_owned(),
        ));
    }
    if authority.dangerous_allow_broad_authority {
        return Ok(());
    }
    let capability = &authority.resources.capability;
    let broad_workflow = matches!(authority.resources.workflow_run, WorkflowRunScope::Any);
    let broad_capability = !capability.denies_all()
        && ((capability.identities().is_empty() && capability.categories().is_empty())
            || capability.operations().is_empty()
            || capability.maximum_side_effect() == SideEffectClass::Unknown);
    let unbounded_budget = authority.budget.cost_minor.is_none()
        || authority.budget.duration_ms.is_none()
        || authority.budget.invocations.is_none()
        || authority.budget.artifact_bytes.is_none()
        || authority.budget.units.is_none()
        || authority.budget.concurrency.is_none();
    let effectively_unbounded = authority.valid_until.get() == u64::MAX;
    let broad_reads = (authority.resources.artifacts.identities().is_empty()
        && !authority.resources.artifacts.sensitivities().is_empty())
        || authority.resources.peers.allows_any()
        || authority.resources.workspace.allows_any_in_run()
        || (authority.resources.layouts.shared()
            && authority.resources.layouts.revisions().is_empty());
    if broad_workflow
        || broad_capability
        || broad_reads
        || unbounded_budget
        || effectively_unbounded
    {
        return Err(ConfigError::Invalid(
            "broad workflow/capability/read scope, unknown side effects, omitted ceilings, or infinite validity requires dangerous_allow_broad_authority=true"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_peers(
    peers: &PeerHostConfig,
    secrets: &BTreeMap<String, SecretSourceConfig>,
) -> Result<(), ConfigError> {
    if !peers.enabled {
        if peers.local_peer_id.is_some() || !peers.relationships.is_empty() {
            return Err(ConfigError::Invalid(
                "peer relationships require peers.enabled=true".to_owned(),
            ));
        }
        return Ok(());
    }
    let local = peers.local_peer_id.as_deref().ok_or_else(|| {
        ConfigError::Invalid("enabled peer support requires local_peer_id".to_owned())
    })?;
    validate_safe_identity("local_peer_id", local)?;
    if peers.relationships.len() > 256 {
        return Err(ConfigError::Invalid(
            "peer relationship count must not exceed 256".to_owned(),
        ));
    }
    let mut identities = BTreeSet::new();
    for relationship in &peers.relationships {
        validate_safe_identity("peer_id", &relationship.peer_id)?;
        validate_safe_identity("peer credential_ref", &relationship.credential_ref)?;
        validate_safe_identity("peer trust_zone", &relationship.trust_zone)?;
        validate_safe_identity("peer delegation_ref", &relationship.delegation_ref)?;
        if relationship.peer_id == local
            || !identities.insert(&relationship.peer_id)
            || !secrets.contains_key(&relationship.credential_ref)
            || relationship.maximum_minor < relationship.minimum_minor
            || relationship.maximum_concurrent == 0
            || relationship.maximum_requests_per_minute == 0
            || relationship.maximum_requests_per_minute > 100_000
            || relationship.maximum_artifact_bytes == 0
            || relationship.maximum_duration_ms == 0
            || relationship.maximum_observations == 0
            || relationship.catalog_ttl_ms == 0
            || relationship.catalog_ttl_ms > 300_000
            || relationship.expires_at_unix_ms == 0
        {
            return Err(ConfigError::Invalid(
                "peer identity, credential, version, quota, TTL, or expiry is invalid".to_owned(),
            ));
        }
        for capability in relationship
            .capability_allow
            .iter()
            .chain(&relationship.capability_deny)
        {
            validate_safe_identity("peer capability filter", capability)?;
        }
        for operation in &relationship.operation_allow {
            validate_safe_identity("peer operation filter", operation)?;
        }
        for filesystem in &relationship.execution_filesystem {
            FilesystemScope::new(filesystem.root(), filesystem.access().clone())
                .map_err(|error| ConfigError::Invalid(error.to_string()))?;
        }
        NetworkScope::new(
            relationship.execution_network_profiles.clone(),
            relationship.execution_network_destinations.clone(),
        )
        .map_err(|error| ConfigError::Invalid(error.to_string()))?;
        if relationship
            .execution_secrets
            .iter()
            .any(|secret| !secrets.contains_key(secret.as_str()))
        {
            return Err(ConfigError::Invalid(
                "peer execution authority references an unknown secret source".to_owned(),
            ));
        }
        let endpoint = url::Url::parse(&relationship.endpoint)
            .map_err(|error| ConfigError::Invalid(format!("invalid peer endpoint: {error}")))?;
        let plaintext_loopback = endpoint.scheme() == "http"
            && relationship.insecure_loopback_development
            && matches!(
                endpoint.host(),
                Some(url::Host::Ipv4(address)) if address.is_loopback()
            )
            || endpoint.scheme() == "http"
                && relationship.insecure_loopback_development
                && matches!(
                    endpoint.host(),
                    Some(url::Host::Ipv6(address)) if address.is_loopback()
                )
            || endpoint.scheme() == "http"
                && relationship.insecure_loopback_development
                && endpoint
                    .host_str()
                    .is_some_and(|host| host.eq_ignore_ascii_case("localhost"));
        if endpoint.scheme() != "https" && !plaintext_loopback {
            return Err(ConfigError::Invalid(
                "peer endpoint must use HTTPS unless insecure loopback development mode is explicit"
                    .to_owned(),
            ));
        }
        if !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(ConfigError::Invalid(
                "peer endpoint must not contain credentials or fragments".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_runtime(config: &RuntimeHostConfig) -> Result<(), ConfigError> {
    if config.request_queue == 0
        || config.request_queue > 65_536
        || config.maintenance_interval_ms == 0
        || config.maintenance_interval_ms > 60_000
        || config.maximum_tick_items == 0
        || config.maximum_tick_items > 1_000
        || config.global_concurrency == 0
        || config.per_run_concurrency == 0
        || config.per_run_concurrency > config.global_concurrency
        || config.per_branch_concurrency == 0
        || config.per_branch_concurrency > config.per_run_concurrency
        || config.per_capability_concurrency == 0
        || config.effect_threads == 0
        || config.effect_threads > 256
        || config.effect_queue == 0
        || config.cancellation_queue == 0
        || config.maximum_effect_claim == 0
        || config.lease_duration_ms == 0
    {
        return Err(ConfigError::Invalid(
            "runtime scheduler, queue, lease, or worker bounds are invalid".to_owned(),
        ));
    }
    Ok(())
}

fn normalize_owned_path(base: &Path, path: &Path) -> Result<PathBuf, ConfigError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(ConfigError::Invalid(
            "configured paths must not contain parent traversal".to_owned(),
        ));
    }
    Ok(path)
}

fn normalize_existing_file(base: &Path, path: &Path) -> Result<PathBuf, ConfigError> {
    let path = normalize_owned_path(base, path)?;
    let canonical = path
        .canonicalize()
        .map_err(|error| ConfigError::Read(error.kind().to_string()))?;
    if !canonical.is_file() {
        return Err(ConfigError::Invalid(
            "configured source path is not a regular file".to_owned(),
        ));
    }
    Ok(canonical)
}

fn validate_safe_identity(location: &str, value: &str) -> Result<(), ConfigError> {
    if value.is_empty()
        || value.len() > 192
        || !value.is_ascii()
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(ConfigError::Invalid(format!(
            "{location} is not a safe bounded identity"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/daemon-config-v5.json")
    }

    #[test]
    fn schema_v5_fixture_is_explicit_safe_and_round_trips() -> Result<(), Box<dyn std::error::Error>>
    {
        let validated = DaemonConfig::load(&fixture_path())?;
        let actor = &validated.document.actors[0];
        assert_eq!(
            validated.document.schema_version,
            DAEMON_CONFIG_SCHEMA_VERSION
        );
        assert!(!actor.authority.dangerous_allow_broad_authority);
        assert!(matches!(
            actor.authority.resources.workflow_run,
            WorkflowRunScope::Workflow { .. }
        ));
        assert_eq!(actor.authority.resources.capability.identities().len(), 1);
        assert_eq!(actor.authority.resources.capability.operations().len(), 1);
        assert_eq!(actor.authority.budget.concurrency, Some(4));
        let bytes = serde_json::to_vec(&validated.document)?;
        let decoded: DaemonConfig = serde_json::from_slice(&bytes)?;
        decoded.validate(&validated.configuration_directory)?;
        Ok(())
    }

    #[test]
    fn old_and_future_config_versions_are_rejected_truthfully()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = fs::read(fixture_path())?;
        for unsupported in [1_u32, 2_u32, 3_u32, 4_u32, 6_u32] {
            let directory = tempfile::tempdir()?;
            let mut value: serde_json::Value = serde_json::from_slice(&source)?;
            value["schema_version"] = serde_json::json!(unsupported);
            let path = directory.path().join("daemon.json");
            fs::write(&path, serde_json::to_vec(&value)?)?;
            assert!(matches!(
                DaemonConfig::load(&path),
                Err(ConfigError::UnsupportedVersion(found)) if found == unsupported
            ));
        }
        Ok(())
    }

    #[test]
    fn wildcard_or_unbounded_authority_requires_the_dangerous_flag()
    -> Result<(), Box<dyn std::error::Error>> {
        let value: serde_json::Value = serde_json::from_slice(&fs::read(fixture_path())?)?;
        let mut config: DaemonConfig = serde_json::from_value(value)?;
        config.actors[0].authority.resources.workflow_run = WorkflowRunScope::Any;
        config.actors[0].authority.budget.duration_ms = None;
        config.actors[0].authority.valid_until = BoundaryTimeMillis::new(u64::MAX);
        config.actors[0].authority.dangerous_allow_broad_authority = false;
        let directory = tempfile::tempdir()?;
        assert!(matches!(
            config.validate(directory.path()),
            Err(ConfigError::Invalid(message))
                if message.contains("dangerous_allow_broad_authority=true")
        ));
        Ok(())
    }

    #[test]
    fn non_loopback_plaintext_is_rejected_before_storage() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let secret = directory.path().join("token");
        fs::write(&secret, "secret")?;
        let config = DaemonConfig {
            schema_version: 5,
            data_root: directory.path().join("data"),
            bind: "0.0.0.0:9734".parse()?,
            secret_sources: BTreeMap::from([(
                "credential:operator".to_owned(),
                SecretSourceConfig::File { path: secret },
            )]),
            actors: vec![ActorBindingConfig {
                credential_ref: "credential:operator".to_owned(),
                actor: "human:operator".to_owned(),
                grant_id: "grant:operator".to_owned(),
                grant_revision: 1,
                revocation_generation: 0,
                preset: AuthorityPresetConfig::Controller,
                authority: ActorGrantConfig::dangerous_administrator(),
                enabled: true,
            }],
            runtime: RuntimeHostConfig::default(),
            adapters: AdapterConfig::default(),
            peers: PeerHostConfig::default(),
            shutdown: ShutdownConfig::default(),
            application_receipts: ApplicationReceiptConfig {
                hot_receipt_bound: 100,
                archive_batch_size: 10,
            },
            security_audit_record_bound: 100,
        };
        assert!(config.validate(directory.path()).is_err());
        assert!(!directory.path().join("data").exists());
        Ok(())
    }
}
