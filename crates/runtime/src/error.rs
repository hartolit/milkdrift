use milkdrift_authority::AuthorityError;
use milkdrift_authority::DecisionReasonCode;
use milkdrift_persistence::PersistenceError;
use thiserror::Error;

use crate::ExecutorError;

/// Failure returned by deterministic runtime operations.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Pure authority contract construction or evaluation failed.
    #[error(transparent)]
    Authority(#[from] AuthorityError),
    /// The exact supplied grant revision denied an external command.
    #[error("authorization denied by decision {decision}: {reasons:?}")]
    AuthorizationDenied {
        /// Deterministic decision digest.
        decision: String,
        /// Stable denial classifications.
        reasons: Vec<DecisionReasonCode>,
    },
    /// A command violates its versioned bounds or state-independent contract.
    #[error("invalid workflow command: {0}")]
    InvalidCommand(String),
    /// Ordered history cannot be projected without guessing.
    #[error("invalid workflow history: {0}")]
    InvalidHistory(String),
    /// A requested transition is not valid from the exact projection.
    #[error("invalid task transition: {0}")]
    InvalidTransition(String),
    /// Pure eligibility or condition evaluation failed.
    #[error("deterministic scheduling failed: {0}")]
    Scheduling(String),
    /// JSON encoding/decoding failed.
    #[error("invalid runtime JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Durable port failed explicitly.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    /// Capability execution boundary failed.
    #[error(transparent)]
    Executor(#[from] ExecutorError),
}
