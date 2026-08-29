#![forbid(unsafe_code)]

//! Transport-neutral, bounded protocol contracts between authenticated Milkdrift peers.
//!
//! This package intentionally owns no socket, async runtime, TLS, database, provider,
//! process, or workflow state. A transport authenticates a [`PeerId`] and then decodes
//! these messages under the limits negotiated by the two configured daemons.

mod artifact;
mod catalog;
mod document;
mod execution;
mod identity;
mod session;

pub use artifact::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision, ArtifactTransferDirection,
    MAX_ARTIFACT_CHUNK_BYTES,
};
pub use catalog::{CatalogEntry, CatalogSnapshot, CatalogUpdate, CatalogUpdateKind};
pub use document::{
    DecodeLimits, MAX_PEER_DOCUMENT_BYTES, ProtocolEnvelope, decode_envelope, encode_envelope,
};
pub use execution::{
    CancellationDisposition, DelegatedAuthorization, ExecutionLimits, InvocationAcceptance,
    InvocationLookup, ObservationCategory, ObservationPage, PeerCancellationAcknowledgement,
    PeerCancellationRequest, PeerExecutionProvenance, PeerInvocationRequest, PeerObservation,
    RemoteExecutionStatus,
};
pub use identity::{
    CatalogDigest, DelegationRef, PeerExecutionId, PeerRequestId, SessionId, TransferId,
};
pub use milkdrift_authority::PeerId;
pub use session::{
    DrainState, FeatureSet, HandshakeRequest, HandshakeResponse, HardLimits, HeartbeatLease,
    PeerAction, PeerAuthority, ProtocolVersion, ProtocolVersionRange, SessionIdentity,
};

use thiserror::Error;

/// Stable validation, negotiation, or bounded-codec failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PeerProtocolError {
    /// A protocol-owned identity is malformed.
    #[error("invalid {kind}: {reason}")]
    InvalidIdentity {
        /// Identity type.
        kind: &'static str,
        /// Stable explanation.
        reason: String,
    },
    /// Message facts contradict protocol semantics.
    #[error("invalid peer protocol contract: {0}")]
    InvalidContract(String),
    /// A defensive resource limit was exceeded.
    #[error("peer protocol bound exceeded at {location}: {reason}")]
    Bounds {
        /// Stable field location.
        location: &'static str,
        /// Stable explanation.
        reason: String,
    },
    /// No compatible protocol version exists.
    #[error("incompatible peer protocol version")]
    IncompatibleVersion,
    /// Canonical encoding or bounded decoding failed.
    #[error("invalid peer protocol JSON: {0}")]
    Json(String),
    /// A supplied canonical digest did not match the immutable facts.
    #[error("peer protocol digest mismatch for {0}")]
    DigestMismatch(&'static str),
}
