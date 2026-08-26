use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::SocketAddr,
    path::{Component, Path, PathBuf},
};

use milkdrift_control_protocol::MAX_DOCUMENT_BYTES;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Current daemon configuration document version.
pub const DAEMON_CONFIG_SCHEMA_VERSION: u32 = 1;

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
    #[error("unsupported daemon configuration version {0}; supported version is 1")]
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

impl AuthorityPresetConfig {
    /// Stable operation labels disclosed by the authority read model.
    #[must_use]
    pub fn operations(self) -> Vec<String> {
        let values: &[&str] = match self {
            Self::Observer => &["inspect"],
            Self::Advisor => &["inspect", "propose"],
            Self::Supervisor => &[
                "inspect",
                "propose",
                "approve",
                "apply",
                "pause",
                "resume",
                "cancel",
                "retry",
                "deliver_signal",
            ],
            Self::Controller => &[
                "inspect",
                "propose",
                "approve",
                "apply",
                "create_run",
                "start_run",
                "pause",
                "resume",
                "cancel",
                "retry",
                "deliver_signal",
                "terminate",
                "artifact_read",
                "layout_write",
                "blueprint_import",
            ],
        };
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    /// Whether this preset may import immutable semantic revisions.
    #[must_use]
    pub const fn may_import_blueprint(self) -> bool {
        matches!(self, Self::Controller)
    }

    /// Whether this preset may update presentation-only layout state.
    #[must_use]
    pub const fn may_write_layout(self) -> bool {
        matches!(self, Self::Supervisor | Self::Controller)
    }

    /// Whether this preset supplies explicit protected-artifact read authority.
    #[must_use]
    pub const fn may_read_protected_artifact(self) -> bool {
        matches!(self, Self::Supervisor | Self::Controller)
    }
}

/// One credential-reference to immutable actor/grant mapping.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
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
    /// False revokes authentication immediately without removing audit configuration.
    pub enabled: bool,
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

/// Configured adapter profile sources containing no secret values.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterConfig {
    /// Versioned local-process profile documents.
    pub process_profiles: Vec<PathBuf>,
    /// Model capability identity/profile sources.
    pub model_profiles: Vec<ModelProfileConfig>,
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

/// Complete bounded version-one daemon host configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonConfig {
    /// Exact configuration schema version.
    pub schema_version: u32,
    /// Owned redb/artifact/layout/command-ledger root.
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
    /// Ordered shutdown policy.
    #[serde(default)]
    pub shutdown: ShutdownConfig,
    /// Maximum durable external command-idempotency records.
    #[serde(default = "default_command_ledger_bound")]
    pub command_ledger_bound: u32,
}

const fn default_command_ledger_bound() -> u32 {
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
        if self.command_ledger_bound == 0 || self.command_ledger_bound > 1_000_000 {
            return Err(ConfigError::Invalid(
                "command ledger bound must be in 1..=1000000".to_owned(),
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
        }
        for path in &mut self.adapters.process_profiles {
            *path = normalize_existing_file(&base, path)?;
        }
        for model in &mut self.adapters.model_profiles {
            validate_safe_identity("model capability", &model.capability_id)?;
            model.profile = normalize_existing_file(&base, &model.profile)?;
        }
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

    #[test]
    fn non_loopback_plaintext_is_rejected_before_storage() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let secret = directory.path().join("token");
        fs::write(&secret, "secret")?;
        let config = DaemonConfig {
            schema_version: 1,
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
                enabled: true,
            }],
            runtime: RuntimeHostConfig::default(),
            adapters: AdapterConfig::default(),
            shutdown: ShutdownConfig::default(),
            command_ledger_bound: 100,
        };
        assert!(config.validate(directory.path()).is_err());
        assert!(!directory.path().join("data").exists());
        Ok(())
    }
}
