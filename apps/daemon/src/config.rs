//! Compile operator configuration once, then give each host component only its owned plan.
mod compile;
mod redaction;
mod wire;
pub use wire::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, ControllerActivation, DaemonConfig, ModelProfileConfig, PeerHostConfig,
    PeerRelationshipConfig, PeerServingConfig, PeerSideEffectConfig, RuntimeHostConfig,
    SecretSourceConfig, ShutdownConfig, ShutdownEffectPolicy,
};

use std::{collections::BTreeMap, net::SocketAddr, path::PathBuf};

use milkdrift_authority::SecretRef;
use milkdrift_contracts::JsonLimits;
use milkdrift_control_protocol::MAX_DOCUMENT_BYTES;
use milkdrift_local_secret::LocalSecretSource;
use thiserror::Error;

/// Current daemon configuration document version.
pub const DAEMON_CONFIG_SCHEMA_VERSION: u32 = 9;

/// Configuration load or deterministic validation failure.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Configuration bytes could not be read.
    #[error("daemon configuration could not be read: {0}")]
    Read(String),
    /// Configuration TOML is malformed or contains duplicate keys.
    #[error("invalid daemon configuration TOML: {0}")]
    Toml(String),
    /// The schema version is unsupported.
    #[error("unsupported daemon configuration version {0}; supported version is 9")]
    UnsupportedVersion(u32),
    /// A host-safety invariant is invalid.
    #[error("invalid daemon configuration: {0}")]
    Invalid(String),
}

const CONFIG_DIGEST_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: 64,
    maximum_string_bytes: MAX_DOCUMENT_BYTES,
    maximum_key_bytes: 512,
    maximum_container_items: 4_096,
};

/// Validated configuration ready for [`crate::DaemonHost::start`].
///
/// Created by [`DaemonConfig::load`] or [`DaemonConfig::validate`], with relative paths resolved
/// against the configuration base. Startup consumes its storage, authentication, adapter, and
/// worker plans; editing the source TOML afterward does not update a running host.
#[derive(Clone, Debug)]
pub struct DaemonPlan {
    bind: SocketAddr,
    storage: StoragePlan,
    authentication: AuthenticationPlan,
    runtime: RuntimeHostConfig,
    adapters: AdapterConfig,
    peers: PeerHostConfig,
    shutdown: ShutdownConfig,
    redacted_toml: String,
    normalized_digest: String,
}

#[derive(Clone, Debug)]
pub(crate) struct StoragePlan {
    pub(crate) data_root: PathBuf,
    pub(crate) application_receipts: ApplicationReceiptConfig,
    pub(crate) security_audit_record_bound: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthenticationPlan {
    pub(crate) secret_sources: BTreeMap<SecretRef, LocalSecretSource>,
    pub(crate) actors: Vec<ActorBindingConfig>,
}

pub(crate) struct DaemonPlanParts {
    pub(crate) storage: StoragePlan,
    pub(crate) authentication: AuthenticationPlan,
    pub(crate) runtime: RuntimeHostConfig,
    pub(crate) adapters: AdapterConfig,
    pub(crate) peers: PeerHostConfig,
    pub(crate) shutdown: ShutdownConfig,
}

impl DaemonPlan {
    /// Local validated listener address.
    #[must_use]
    pub const fn bind(&self) -> SocketAddr {
        self.bind
    }

    /// Redacted normalized effective configuration rendered as TOML.
    #[must_use]
    pub fn redacted_toml(&self) -> &str {
        &self.redacted_toml
    }

    /// Digest of the normalized effective configuration, excluding source formatting.
    #[must_use]
    pub fn normalized_digest(&self) -> &str {
        &self.normalized_digest
    }

    pub(crate) fn into_parts(self) -> DaemonPlanParts {
        DaemonPlanParts {
            storage: self.storage,
            authentication: self.authentication,
            runtime: self.runtime,
            adapters: self.adapters,
            peers: self.peers,
            shutdown: self.shutdown,
        }
    }
}

#[cfg(test)]
mod tests;
