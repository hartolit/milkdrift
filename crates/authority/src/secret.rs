use std::fmt;

/// Explicitly sensitive resolved bytes with redacted formatting and no serialization or clone.
pub struct SensitiveSecret(Vec<u8>);

impl SensitiveSecret {
    /// Takes ownership of resolver-produced secret bytes.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Exposes the bytes only for the duration of one adapter-owned closure.
    pub fn expose<R>(&self, use_secret: impl FnOnce(&[u8]) -> R) -> R {
        use_secret(&self.0)
    }

    /// Returns the byte count without exposing content.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the resolved value is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SensitiveSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitiveSecret([redacted])")
    }
}

impl fmt::Display for SensitiveSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

impl Drop for SensitiveSecret {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}
