//! Local-first Milkdrift daemon host and versioned HTTP control plane.
//!
//! The async reactor owns sockets only. A dedicated bounded runtime-owner thread owns
//! durable control/query calls, while caller-owned effect workers enter external adapter
//! boundaries on their own fixed threads.

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
