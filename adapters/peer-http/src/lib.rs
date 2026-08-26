#![forbid(unsafe_code)]

//! Authenticated HTTPS peer transport and ordinary capability-host adapter.

mod artifact;
mod auth;
mod client;
mod config;
mod http;
mod remote;
mod service;
mod store;

pub use artifact::{
    FilePeerArtifactStore, PeerArtifactError, PeerArtifactFaultInjector, PeerArtifactFaultPoint,
    PeerArtifactStore,
};
pub use auth::{PeerAuthenticator, PeerCredentialSource, StaticPeerCredential};
pub use client::PeerHttpClient;
pub use config::{InsecureLoopbackMode, PeerClientConfig, PeerRelationship, PeerServerConfig};
pub use http::peer_router;
pub use remote::{PeerRegistry, PeerRegistryStatus, RemoteCapabilityProvenance};
pub use service::{PeerClock, PeerService, SystemPeerClock};
pub use store::{
    FilePeerExecutionStore, PeerExecutionStore, PeerStoreError, PeerStoreFaultInjector,
    PeerStoreFaultPoint, StoreAcceptance, StoredExecution,
};

use thiserror::Error;

/// Stable peer transport/application failure with redacted messages.
#[derive(Debug, Error)]
pub enum PeerHttpError {
    /// Configuration violates transport safety or bounds.
    #[error("invalid peer HTTP configuration: {0}")]
    Configuration(String),
    /// No configured credential authenticated the request.
    #[error("valid peer authentication is required")]
    Unauthenticated,
    /// The authenticated peer lacks the requested action/scope.
    #[error("peer authorization denied: {0}")]
    Unauthorized(String),
    /// Bounded protocol decoding or semantic validation failed.
    #[error("peer protocol error: {0}")]
    Protocol(String),
    /// Remote transport failed without proving an execution outcome.
    #[error("peer transport unavailable: {0}")]
    Transport(String),
    /// A requested peer-owned record does not exist.
    #[error("peer record not found: {0}")]
    NotFound(String),
    /// Peer quota or concurrency admission rejected before acceptance.
    #[error("peer overloaded: {0}")]
    Overloaded(String),
    /// Durable peer adapter state could not be read or committed.
    #[error("peer persistence unavailable: {0}")]
    Persistence(String),
    /// Local adapter/registry service is unavailable.
    #[error("peer service unavailable: {0}")]
    Unavailable(String),
}
