use milkdrift_authority::{AuthorityError, DecisionReasonCode};
use milkdrift_persistence::PersistenceError;
use milkdrift_runtime::RuntimeError;
use thiserror::Error;

/// A checked request failed without bypassing authority or durable admission.
#[derive(Debug, Error)]
pub enum ControlError {
    /// The supplied control facts violate a bounded contract.
    #[error("invalid control contract: {0}")]
    InvalidContract(String),
    /// Canonical request encoding failed.
    #[error("invalid control JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Current immutable grants deny the requested operation.
    #[error("authority denied the control request: {reasons:?}")]
    AuthorizationDenied {
        /// Stable reasons that do not reveal hidden resource contents.
        reasons: Vec<DecisionReasonCode>,
        /// Exact decision identity when evaluation completed.
        decision_digest: Option<String>,
    },
    /// Authority construction or evaluation failed.
    #[error(transparent)]
    Authority(#[from] AuthorityError),
    /// Durable storage refused the operation.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    /// Task admission, transition or execution failed.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
}
