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
    AuthorityPresetConfig, DaemonConfig, ModelProfileConfig, PeerHostConfig,
    PeerRelationshipConfig, PeerServingConfig, PeerSideEffectConfig, RuntimeHostConfig,
    SecretSourceConfig, ShutdownConfig, ShutdownEffectPolicy, ValidatedDaemonConfig,
};
pub use host::{DaemonHost, HostError};
pub use http::{router, serve};
