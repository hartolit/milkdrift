use milkdrift_workspace::RunId;
use thiserror::Error;

use crate::{CommandId, IntegrityDigest, RunSequence};

/// Stable classification for failures reported by a storage adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageFailureClass {
    /// Stored bytes or indexes failed integrity validation.
    Corruption,
    /// The backing store is temporarily unavailable.
    Unavailable,
    /// The configured data owner is already held elsewhere.
    OwnerBusy,
    /// The adapter cannot safely complete a migration.
    Migration,
    /// An operation exceeded a configured resource bound.
    ResourceExhausted,
    /// A lower-level failure did not fit another stable class.
    Internal,
}

/// Failure returned by a durable persistence port.
#[derive(Debug, Error)]
pub enum PersistenceError {
    /// A persistence-owned typed identity was malformed.
    #[error("invalid {kind}: {reason}")]
    InvalidIdentity {
        /// Identity kind.
        kind: &'static str,
        /// Bounded validation detail.
        reason: String,
    },
    /// A content or integrity digest was malformed.
    #[error("invalid integrity digest: {0}")]
    InvalidDigest(String),
    /// A bounded durable field exceeded its contract.
    #[error("persistence bound exceeded at {location}: {reason}")]
    Bounds {
        /// Stable field location.
        location: &'static str,
        /// Validation detail.
        reason: String,
    },
    /// A storage document did not satisfy its semantic invariants.
    #[error("invalid durable document: {0}")]
    InvalidDocument(String),
    /// JSON was malformed or did not match the closed schema.
    #[error("invalid durable JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// A document or storage schema is newer than this binary understands.
    #[error("unsupported {document} schema version {found}; maximum supported is {supported}")]
    UnsupportedVersion {
        /// Durable document family.
        document: &'static str,
        /// Version read from storage.
        found: u32,
        /// Latest supported version.
        supported: u32,
    },
    /// Stored data failed its checksum or another integrity check.
    #[error("durable data corruption: {0}")]
    Corruption(String),
    /// The event sequence changed since the caller read the aggregate.
    #[error("run {run} sequence conflict: expected {expected}, actual {actual}")]
    SequenceConflict {
        /// Conflicting run.
        run: RunId,
        /// Caller's optimistic guard.
        expected: RunSequence,
        /// Authoritative journal sequence.
        actual: RunSequence,
    },
    /// An idempotency identity was reused for different command content.
    #[error(
        "command {command} idempotency conflict for run {run}: existing fingerprint {existing}, supplied {supplied}"
    )]
    IdempotencyConflict {
        /// Aggregate on which the command identity was first observed.
        run: RunId,
        /// Reused command identity.
        command: CommandId,
        /// Fingerprint recorded by the first durable result.
        existing: IntegrityDigest,
        /// Fingerprint supplied by the conflicting delivery.
        supplied: IntegrityDigest,
    },
    /// An immutable identity already exists with different bytes.
    #[error("immutable {entity} conflict for identity {identity}")]
    ImmutableConflict {
        /// Durable entity family.
        entity: &'static str,
        /// Stable identity text.
        identity: String,
    },
    /// A required durable entity was absent.
    #[error("{entity} not found: {identity}")]
    NotFound {
        /// Durable entity family.
        entity: &'static str,
        /// Stable identity text.
        identity: String,
    },
    /// A cursor is malformed, belongs to another query, or is no longer resumable.
    #[error("invalid page cursor: {0}")]
    InvalidCursor(String),
    /// A required artifact is not durably committed, so an event may not reference it.
    #[error("artifact is not durably committed: {0}")]
    ArtifactNotCommitted(String),
    /// Stored workspace accounting changed since the caller read it.
    #[error("workspace usage conflict for run {run}")]
    WorkspaceUsageConflict {
        /// Conflicting workspace/run accounting domain.
        run: RunId,
    },
    /// The active-lease catalog changed after runtime admission was calculated.
    #[error("lease catalog conflict: expected {expected}, actual {actual}")]
    LeaseCatalogConflict {
        /// Opaque catalog root observed by the runtime.
        expected: IntegrityDigest,
        /// Authoritative catalog root inside the lease-grant transaction.
        actual: IntegrityDigest,
    },
    /// Sensitive artifact content was requested without an explicit authority proof.
    #[error("artifact access denied: {0}")]
    ArtifactAccessDenied(String),
    /// Appending would overflow the run sequence authority.
    #[error("run sequence overflow")]
    SequenceOverflow,
    /// A migration is required before ordinary access can continue.
    #[error("storage schema {found} requires migration to {target}")]
    MigrationRequired {
        /// Existing supported schema.
        found: u32,
        /// Current schema.
        target: u32,
    },
    /// Adapter-specific detail classified without leaking implementation types.
    #[error("storage {class:?}: {message}")]
    Storage {
        /// Portable classification.
        class: StorageFailureClass,
        /// Redacted bounded message.
        message: String,
    },
}
