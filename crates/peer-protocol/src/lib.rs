#![forbid(unsafe_code)]

//! Describe remote acceptance and observations while each host retains its own durable truth.
//!
//! Begin with [`HandshakeRequest`] and [`CatalogSnapshot`] to find an advertised generation.
//! [`PeerInvocationRequest`] binds the exact selection and delegated scope; its request identity
//! survives lost replies through [`InvocationAcceptance`] and [`InvocationLookup`]. Follow a
//! known execution with [`ObservationPage`], which distinguishes retained rows from archival.
//!
//! A transport authenticates the peer and calls [`decode_envelope`] with bounded limits. Call
//! payload `validate`/`validate_for` methods for semantic and request-binding checks as applicable.
//! These contracts perform no authentication or I/O; `milkdrift-peer-http` supplies the current
//! transport, worker lifecycle, and core artifact bridge.

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
    ArchivedExecutionSummary, CancellationDisposition, DelegatedAuthorization, ExecutionLimits,
    InvocationAcceptance, InvocationLookup, ObservationCategory, ObservationHistory,
    ObservationPage, PeerCancellationAcknowledgement, PeerCancellationRequest,
    PeerExecutionProvenance, PeerInvocationRequest, PeerObservation, RemoteExecutionStatus,
};
pub use identity::{
    CatalogDigest, DelegationRef, PeerExecutionId, PeerRequestId, SessionId, TransferId,
};
pub use session::{
    DrainState, FeatureSet, HandshakeRequest, HandshakeResponse, HardLimits, HeartbeatLease,
    PROTOCOL_MAJOR_V1, PROTOCOL_MINOR_V1, PeerAction, PeerAuthority, ProtocolVersion,
    ProtocolVersionRange, SessionIdentity,
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
