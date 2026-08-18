use milkdrift_persistence::PersistenceError;
use thiserror::Error;

use crate::ExecutorError;

/// Failure returned by deterministic runtime operations.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// A command violates its versioned bounds or state-independent contract.
    #[error("invalid run command: {0}")]
    InvalidCommand(String),
    /// A future command schema cannot be interpreted safely.
    #[error("unsupported run command schema {found}; supported schema is {supported}")]
    UnsupportedCommandVersion {
        /// Version supplied by the caller.
        found: u32,
        /// Latest version understood by this runtime.
        supported: u32,
    },
    /// Ordered history cannot be projected without guessing.
    #[error("invalid run history: {0}")]
    InvalidHistory(String),
    /// A requested transition is not valid from the exact projection.
    #[error("invalid run transition: {0}")]
    InvalidTransition(String),
    /// Pure eligibility or condition evaluation failed.
    #[error("deterministic scheduling failed: {0}")]
    Scheduling(String),
    /// A reconciliation plan is incompatible, unauthorized, or stale.
    #[error("revision reconciliation failed: {0}")]
    Reconciliation(String),
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
