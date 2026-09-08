use milkdrift_workspace::RunId;
use thiserror::Error;

use crate::{CommandId, IntegrityDigest, RunSequence};

/// Classifies an adapter failure without exposing a database-specific error type.
///
/// These classes describe the failure, not whether a write committed or an external
/// effect happened. In particular, `Unavailable` is not permission to retry work with
/// a new identity. Follow the affected port's recovery contract, such as
/// [`crate::RunJournal`]'s saved-result lookup and command redelivery.
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

/// A document could not be constructed/read, or a storage operation could not return success.
///
/// This is distinct from a saved command result whose disposition is
/// [`crate::CommandDisposition::Rejected`]. Constructors have no storage effects;
/// errors from a write port need that port's recovery rules. [`crate::RunJournal`]
/// explains how to recover a result when an error may have arrived after commit.
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
    /// A reader does not support the supplied document or storage schema version.
    /// Some families accept several versions; consult the owning reader for that set.
    #[error("unsupported {document} schema version {found}; supported version is {supported}")]
    UnsupportedVersion {
        /// Durable document family.
        document: &'static str,
        /// Version read from storage.
        found: u32,
        /// Version reported by the reader as its supported target, not a list of all readable forms.
        supported: u32,
    },
    /// Stored data failed its checksum or another integrity check.
    #[error("durable data corruption: {0}")]
    Corruption(String),
    /// A new command's expected sequence differs from the current journal head.
    /// The runtime must reread history and replan; changing only the guard could admit
    /// a transition calculated from stale state. Exact replay precedes this check.
    #[error("run {run} sequence conflict: expected {expected}, actual {actual}")]
    SequenceConflict {
        /// Conflicting run.
        run: RunId,
        /// Caller's optimistic guard.
        expected: RunSequence,
        /// Authoritative journal sequence.
        actual: RunSequence,
    },
    /// A saved `(run, command)` has a different intent fingerprint.
    /// Preserve the original result and resolve which request was intended; do not
    /// choose a fresh key merely to bypass the conflict after losing a response.
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
    /// An actor-scoped external command identity was reused with different canonical content.
    #[error(
        "external command {command} idempotency conflict for actor {actor}: existing digest {existing}, supplied {supplied}"
    )]
    ExternalCommandIdempotencyConflict {
        /// Authenticated actor whose command-key namespace was reused.
        actor: milkdrift_authority::ActorRef,
        /// Reused client command identity.
        command: CommandId,
        /// Digest stored by the first durable result.
        existing: IntegrityDigest,
        /// Digest supplied by the conflicting delivery.
        supplied: IntegrityDigest,
    },
    /// Receipt archival raced a different successful archival generation.
    #[error(
        "application receipt archive generation conflict: expected {expected}, actual {actual}"
    )]
    ApplicationReceiptArchiveGenerationConflict {
        /// Generation observed by the caller.
        expected: u64,
        /// Authoritative generation inside the archival transaction.
        actual: u64,
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
    /// A required artifact is not durably committed, so this append cannot reference it.
    /// Resolve publication through [`crate::ArtifactStore`] before planning the append again.
    #[error("artifact is not durably committed: {0}")]
    ArtifactNotCommitted(String),
    /// Stored workspace accounting differs from the proposed commit's expected usage.
    /// Recompute charges from current usage before attempting a new commit.
    #[error("workspace usage conflict for run {run}")]
    WorkspaceUsageConflict {
        /// Conflicting workspace/run accounting domain.
        run: RunId,
    },
    /// The active-lease set changed after runtime admission was calculated.
    /// Recheck capacity from a fresh lease snapshot before planning another grant.
    #[error("lease revision conflict: expected {expected}, actual {actual}")]
    LeaseRevisionConflict {
        /// Opaque lease-set revision observed by the runtime.
        expected: IntegrityDigest,
        /// Authoritative lease-set revision inside the lease-grant transaction.
        actual: IntegrityDigest,
    },
    /// A controller account changed after runtime planned a guarded transition.
    #[error("controller account {account} revision conflict: expected {expected}, actual {actual}")]
    ControllerAccountRevisionConflict {
        /// Conflicting controller account.
        account: crate::ControllerAccountId,
        /// Opaque revision digest observed by runtime.
        expected: IntegrityDigest,
        /// Authoritative revision digest inside the transaction.
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
