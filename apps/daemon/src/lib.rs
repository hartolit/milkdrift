//! Run a durable Milkdrift host and expose it through the authenticated control API.
//!
//! Load operator TOML with [`DaemonConfig::load`] to obtain a validated [`DaemonPlan`].
//! [`DaemonHost::start`] recovers storage and starts adapters/workers before returning ready;
//! [`serve`] connects that host to a listener and a caller-supplied shutdown future.
//!
//! HTTP handlers authenticate and frame requests. A bounded owner thread serializes durable
//! command and read calls, while fixed effect workers enter external capabilities. A command
//! reply reports acceptance; subsequent run and attempt reads report execution. Callers must
//! complete [`DaemonHost::shutdown`] when hosting without [`serve`] so workers can finish
//! their durable writes before storage ownership ends.

mod auth;
mod config;
mod host;
mod http;

pub use config::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, DAEMON_CONFIG_SCHEMA_VERSION, DaemonConfig, DaemonPlan,
    ModelProfileConfig, PeerHostConfig, PeerRelationshipConfig, PeerServingConfig,
    PeerSideEffectConfig, RuntimeHostConfig, SecretSourceConfig, ShutdownConfig,
    ShutdownEffectPolicy,
};
pub use host::{DaemonHost, HostError};
pub use http::serve;
