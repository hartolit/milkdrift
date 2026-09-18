use thiserror::Error;

/// Stable serving execution failure with redacted messages.
#[derive(Debug, Error)]
pub enum ServingError {
    /// Configuration violates transport safety or bounds.
    #[error("invalid serving configuration: {0}")]
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
