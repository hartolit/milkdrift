//! Application-owned selection and model-reporting vocabulary.

/// Local execution engine reported by E1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationEngine {
    /// Candle local execution.
    Candle,
}

/// Model artifact source reported by E1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationSource {
    /// Immutable artifacts resolved through Hugging Face Hub.
    HuggingFaceHub,
}

/// Local execution device reported by E1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationDevice {
    /// Host CPU execution.
    Cpu,
}

/// Model serialization format reported by E1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationModelFormat {
    /// Safetensors shards plus model configuration.
    Safetensors,
}

/// User-visible Hugging Face model selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSelection {
    repository: String,
    revision: String,
}

impl ModelSelection {
    /// Creates a normalized Hugging Face repository and revision selection.
    #[must_use]
    pub fn new(repository: impl Into<String>, revision: impl Into<String>) -> Self {
        Self {
            repository: repository.into().trim().to_owned(),
            revision: revision.into().trim().to_owned(),
        }
    }

    /// Returns the normalized Hugging Face repository.
    #[must_use]
    pub const fn repository(&self) -> &str {
        self.repository.as_str()
    }

    /// Returns the normalized requested revision.
    #[must_use]
    pub const fn revision(&self) -> &str {
        self.revision.as_str()
    }

    pub(crate) fn into_parts(self) -> (String, String) {
        (self.repository, self.revision)
    }
}

/// Scalar representation supported by the Candle/Safetensors application path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationScalarType {
    /// IEEE-754 32-bit floating point.
    F32,
    /// IEEE-754 16-bit floating point.
    F16,
    /// Brain floating point.
    Bf16,
}

/// Immutable Hugging Face artifact identity retained across resolution and loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImmutableModelIdentity {
    repository: String,
    commit: String,
}

impl ImmutableModelIdentity {
    pub(crate) fn new(repository: impl Into<String>, commit: impl Into<String>) -> Self {
        Self {
            repository: repository.into().trim().to_owned(),
            commit: commit.into().trim().to_owned(),
        }
    }

    /// Returns the repository whose immutable commit was resolved.
    #[must_use]
    pub const fn repository(&self) -> &str {
        self.repository.as_str()
    }

    /// Returns the immutable Hub commit identifier.
    #[must_use]
    pub const fn commit(&self) -> &str {
        self.commit.as_str()
    }
}
