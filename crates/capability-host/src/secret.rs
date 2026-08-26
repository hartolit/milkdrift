use std::{collections::BTreeMap, fmt, sync::Mutex};

use milkdrift_authority::{SecretRef, SensitiveSecret};
use thiserror::Error;

/// Redacted failure at the opaque secret-resolution port.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SecretResolverError {
    /// The opaque reference is not configured.
    #[error("secret reference is unavailable")]
    Unavailable,
    /// Deterministic test resolver synchronization failed.
    #[error("secret resolver is unavailable")]
    ResolverUnavailable,
}

/// Narrow host/adapter secret port; resolved values never enter serializable contracts.
pub trait SecretResolver: Send + Sync {
    /// Resolves one authorized opaque reference into explicitly sensitive bytes.
    fn resolve(&self, reference: &SecretRef) -> Result<SensitiveSecret, SecretResolverError>;
}

/// Deterministic in-memory resolver for tests; it performs no file or environment I/O.
#[derive(Default)]
pub struct InMemorySecretResolver {
    values: Mutex<BTreeMap<SecretRef, Vec<u8>>>,
}

impl InMemorySecretResolver {
    /// Creates an empty resolver.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs test-only secret bytes under an opaque reference.
    pub fn insert(&self, reference: SecretRef, value: Vec<u8>) -> Result<(), SecretResolverError> {
        self.values
            .lock()
            .map_err(|_error| SecretResolverError::ResolverUnavailable)?
            .insert(reference, value);
        Ok(())
    }
}

impl SecretResolver for InMemorySecretResolver {
    fn resolve(&self, reference: &SecretRef) -> Result<SensitiveSecret, SecretResolverError> {
        let values = self
            .values
            .lock()
            .map_err(|_error| SecretResolverError::ResolverUnavailable)?;
        values
            .get(reference)
            .map(|value| SensitiveSecret::new(value.clone()))
            .ok_or(SecretResolverError::Unavailable)
    }
}

impl fmt::Debug for InMemorySecretResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InMemorySecretResolver([redacted])")
    }
}
