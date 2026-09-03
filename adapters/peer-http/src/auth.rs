use std::sync::Arc;

use milkdrift_authority::SensitiveSecret;
use milkdrift_capability::PeerId;

use crate::PeerHttpError;

/// Request-time secret source supporting operator-owned credential rotation.
pub trait PeerCredentialSource: Send + Sync {
    /// Resolves the current secret value for exactly one outbound HTTP request.
    fn resolve(&self) -> Result<SensitiveSecret, PeerHttpError>;
}

/// Static redacted credential source used by direct library callers and tests.
pub struct StaticPeerCredential {
    value: Arc<SensitiveSecret>,
}

impl StaticPeerCredential {
    /// Wraps an already resolved credential.
    #[must_use]
    pub const fn new(value: Arc<SensitiveSecret>) -> Self {
        Self { value }
    }
}

impl PeerCredentialSource for StaticPeerCredential {
    fn resolve(&self) -> Result<SensitiveSecret, PeerHttpError> {
        Ok(self
            .value
            .expose(|bytes| SensitiveSecret::new(bytes.to_vec())))
    }
}

/// Server authentication boundary mapping current credential bytes to configured identity.
pub trait PeerAuthenticator: Send + Sync {
    /// Authenticates at a boundary time. Payload claims cannot influence the result.
    fn authenticate(&self, supplied: &[u8], now_unix_ms: u64) -> Option<PeerId>;
}
