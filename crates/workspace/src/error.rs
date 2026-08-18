use thiserror::Error;

/// Error returned when a workspace contract violates a durable invariant.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// A typed identity failed its length or character rules.
    #[error("invalid {type_name}: {reason}")]
    InvalidIdentity {
        /// Identity type being validated.
        type_name: &'static str,
        /// Stable diagnostic detail.
        reason: String,
    },
    /// A content digest was not a canonical BLAKE3 digest.
    #[error("invalid BLAKE3 content digest: {0}")]
    InvalidDigest(String),
    /// A media type was empty, oversized, or syntactically unsafe.
    #[error("invalid artifact media type: {0}")]
    InvalidMediaType(String),
    /// A scope or scope lineage violated structured parent semantics.
    #[error("invalid workspace scope: {0}")]
    InvalidScope(String),
    /// A value reference, origin, or version transition was inconsistent.
    #[error("invalid workspace value: {0}")]
    InvalidValue(String),
    /// Artifact metadata or provenance was inconsistent or exceeded its bounds.
    #[error("invalid artifact metadata: {0}")]
    InvalidArtifact(String),
    /// A configured budget was internally inconsistent.
    #[error("invalid workspace budget: {0}")]
    InvalidBudget(String),
    /// Admitting a value or artifact would exceed a configured budget.
    #[error("workspace budget exceeded for {resource}: limit {limit}, attempted {attempted}")]
    BudgetExceeded {
        /// Name of the exhausted resource.
        resource: &'static str,
        /// Configured inclusive limit.
        limit: u64,
        /// Usage that the operation would produce.
        attempted: u64,
    },
    /// Integer accounting overflowed rather than being allowed to wrap.
    #[error("workspace accounting overflow for {0}")]
    AccountingOverflow(&'static str),
    /// JSON serialization failed while measuring a bounded inline value.
    #[error("cannot measure bounded workspace value: {0}")]
    Json(#[from] serde_json::Error),
}
