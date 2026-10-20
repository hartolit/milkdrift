use milkdrift_workspace::RunId;
use thiserror::Error;

/// Classifies an adapter failure without exposing a database-specific error type.
///
/// These classes describe the failure, not whether a write committed or an external
/// effect happened. In particular, `Unavailable` is not permission to retry work with
/// a new identity. Follow the affected port's recovery contract, such as
/// [`crate::WorkflowStore`]'s saved-result lookup and command redelivery.
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
/// a task command rejection. Constructors have no storage effects;
/// errors from a write port need that port's recovery rules. [`crate::WorkflowStore`]
/// explains how to recover a result when an error may have arrived after commit.
#[derive(Debug, Error)]
pub enum PersistenceError {
    /// Project settings changed before the guarded registration or task acceptance.
    #[error("project generation conflict: expected {expected}, actual {actual}")]
    ProjectGenerationConflict {
        /// Generation reviewed by the caller.
        expected: u64,
        /// Current accepted project generation.
        actual: u64,
    },
    /// A project command key already binds different canonical request bytes.
    #[error("project command {key} idempotency conflict for actor {actor}")]
    ProjectIdempotencyConflict {
        /// Authenticated receipt owner.
        actor: milkdrift_authority::ActorRef,
        /// Conflicting actor-local key.
        key: String,
    },
    /// Another accepted rename changed the daemon's name.
    #[error("daemon name generation conflict: expected {expected}, actual {actual}")]
    DaemonGenerationConflict {
        /// Generation observed by the caller.
        expected: u64,
        /// Current accepted generation.
        actual: u64,
    },
    /// A rename request key already identifies different canonical request bytes.
    #[error("daemon rename {key} idempotency conflict for actor {actor}")]
    DaemonIdempotencyConflict {
        /// Authenticated actor owning the request.
        actor: milkdrift_authority::ActorRef,
        /// Conflicting actor-local request key.
        key: String,
    },
    /// A task workflow changed after its transition was planned.
    #[error("workflow {workflow} generation conflict: expected {expected:?}, actual {actual:?}")]
    WorkflowGenerationConflict {
        /// Workflow whose state changed.
        workflow: milkdrift_blueprint::WorkflowId,
        /// Caller observation, or absence for creation.
        expected: Option<u64>,
        /// Durable state, or absence.
        actual: Option<u64>,
    },
    /// Exact task request recovery refuses reuse with different canonical bytes.
    #[error("workflow command {key} idempotency conflict for actor {actor}")]
    WorkflowIdempotencyConflict {
        /// Authenticated actor owning the key.
        actor: String,
        /// Conflicting request key.
        key: String,
    },
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
    /// A required artifact is not durably committed, so the transition cannot reference it.
    /// Resolve publication through [`crate::ArtifactStore`] before planning the transition again.
    #[error("artifact is not durably committed: {0}")]
    ArtifactNotCommitted(String),
    /// Stored workspace accounting differs from the proposed commit's expected usage.
    /// Recompute charges from current usage before attempting a new commit.
    #[error("workspace usage conflict for run {run}")]
    WorkspaceUsageConflict {
        /// Conflicting workspace/run accounting domain.
        run: RunId,
    },
    /// Sensitive artifact content was requested without an explicit authority proof.
    #[error("artifact access denied: {0}")]
    ArtifactAccessDenied(String),
    /// Advancing a durable evidence sequence would overflow its numeric range.
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
