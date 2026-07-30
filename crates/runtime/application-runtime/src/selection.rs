//! Closed application-owned vocabulary for supported local model products.

use std::path::{Path, PathBuf};

/// One of the two local CPU model products supported by E1.
///
/// Backend, source, device, and format are derived from this enum so callers
/// cannot construct unsupported cross-product combinations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalModelProduct {
    /// Hugging Face Hub artifacts executed by Candle from Safetensors weights.
    HuggingFaceCandleSafetensors,
    /// A local GGUF file executed by llama.cpp.
    LocalLlamaCppGguf,
}

impl LocalModelProduct {
    /// Returns the concrete local inference backend.
    #[must_use]
    pub const fn backend(self) -> ApplicationBackend {
        match self {
            Self::HuggingFaceCandleSafetensors => ApplicationBackend::Candle,
            Self::LocalLlamaCppGguf => ApplicationBackend::LlamaCpp,
        }
    }

    /// Returns the artifact source category.
    #[must_use]
    pub const fn source(self) -> ApplicationSource {
        match self {
            Self::HuggingFaceCandleSafetensors => ApplicationSource::HuggingFaceHub,
            Self::LocalLlamaCppGguf => ApplicationSource::LocalFile,
        }
    }

    /// Returns the execution device category.
    #[must_use]
    pub const fn device(self) -> ApplicationDevice {
        ApplicationDevice::Cpu
    }

    /// Returns the model serialization format.
    #[must_use]
    pub const fn format(self) -> ApplicationModelFormat {
        match self {
            Self::HuggingFaceCandleSafetensors => ApplicationModelFormat::Safetensors,
            Self::LocalLlamaCppGguf => ApplicationModelFormat::Gguf,
        }
    }
}

/// Application-supported local inference backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationBackend {
    /// Candle Llama execution.
    Candle,
    /// llama.cpp execution.
    LlamaCpp,
}

/// Application-supported model artifact source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationSource {
    /// Immutable artifacts resolved through Hugging Face Hub.
    HuggingFaceHub,
    /// A user-selected local file.
    LocalFile,
}

/// Application-supported execution device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationDevice {
    /// Host CPU execution.
    Cpu,
}

/// Application-supported model serialization format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationModelFormat {
    /// Unquantized Safetensors shards plus model configuration.
    Safetensors,
    /// A GGUF model containing weights, metadata, and vocabulary.
    Gguf,
}

/// One complete user-visible local model selection.
///
/// There are deliberately no hosted or peer variants. Each variant fixes the
/// source, backend, device, and format as one reviewed product combination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelSelection {
    /// Hugging Face Hub + Candle + Safetensors.
    HuggingFaceSafetensors {
        /// Hugging Face model repository.
        repository: String,
        /// Branch, tag, reference, or commit requested from the Hub.
        revision: String,
    },
    /// Local file + llama.cpp + GGUF.
    LocalGguf {
        /// User-selected local GGUF path.
        path: PathBuf,
    },
}

impl ModelSelection {
    /// Creates a normalized Hugging Face/Candle/Safetensors selection.
    #[must_use]
    pub fn hugging_face_safetensors(
        repository: impl Into<String>,
        revision: impl Into<String>,
    ) -> Self {
        Self::HuggingFaceSafetensors {
            repository: repository.into().trim().to_owned(),
            revision: revision.into().trim().to_owned(),
        }
    }

    /// Creates a local llama.cpp/GGUF selection.
    #[must_use]
    pub fn local_gguf(path: impl Into<PathBuf>) -> Self {
        Self::LocalGguf { path: path.into() }
    }

    /// Returns the fixed local product combination represented by this selection.
    #[must_use]
    pub const fn product(&self) -> LocalModelProduct {
        match self {
            Self::HuggingFaceSafetensors { .. } => LocalModelProduct::HuggingFaceCandleSafetensors,
            Self::LocalGguf { .. } => LocalModelProduct::LocalLlamaCppGguf,
        }
    }

    /// Returns the Hugging Face repository and revision, when selected.
    #[must_use]
    pub const fn hugging_face_reference(&self) -> Option<(&str, &str)> {
        match self {
            Self::HuggingFaceSafetensors {
                repository,
                revision,
            } => Some((repository.as_str(), revision.as_str())),
            Self::LocalGguf { .. } => None,
        }
    }

    /// Returns the local GGUF path, when selected.
    #[must_use]
    pub fn local_path(&self) -> Option<&Path> {
        match self {
            Self::LocalGguf { path } => Some(path.as_path()),
            Self::HuggingFaceSafetensors { .. } => None,
        }
    }
}

/// Application-owned scalar representation used in model summaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationScalarType {
    /// IEEE-754 32-bit floating point.
    F32,
    /// IEEE-754 16-bit floating point.
    F16,
    /// Brain floating point.
    Bf16,
    /// Signed 8-bit integer.
    I8,
    /// Unsigned 8-bit integer.
    U8,
    /// Backend-defined scalar representation.
    Other(u16),
}

/// Application-owned quantization representation used in model summaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationQuantization {
    /// Model weights are not quantized.
    None,
    /// Generic signed 8-bit quantization.
    Int8,
    /// Generic signed 4-bit quantization.
    Int4,
    /// GGUF-defined file type code.
    Gguf(u16),
    /// Backend-defined quantization code.
    Other(u16),
}

/// Product-specific scalar and quantization compatibility evidence.
///
/// Safetensors cannot be paired with a GGUF quantization code, and GGUF always
/// carries an inspected scalar and quantization value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelCompatibility {
    /// Candle/Safetensors compatibility from the immutable model configuration.
    CandleSafetensors {
        /// Declared scalar type, or `None` when the configuration did not expose
        /// a Candle-supported scalar.
        scalar_type: Option<ApplicationScalarType>,
    },
    /// llama.cpp/GGUF compatibility from the inspected GGUF header.
    LlamaCppGguf {
        /// Inspected tensor scalar representation.
        scalar_type: ApplicationScalarType,
        /// Inspected GGUF quantization description.
        quantization: ApplicationQuantization,
    },
}

impl ModelCompatibility {
    /// Returns the recognized scalar type, when one is available.
    #[must_use]
    pub const fn scalar_type(self) -> Option<ApplicationScalarType> {
        match self {
            Self::CandleSafetensors { scalar_type } => scalar_type,
            Self::LlamaCppGguf { scalar_type, .. } => Some(scalar_type),
        }
    }

    /// Returns the effective quantization description.
    #[must_use]
    pub const fn quantization(self) -> ApplicationQuantization {
        match self {
            Self::CandleSafetensors { .. } => ApplicationQuantization::None,
            Self::LlamaCppGguf { quantization, .. } => quantization,
        }
    }

    /// Returns whether the compatibility evidence is sufficient for loading.
    #[must_use]
    pub const fn is_loadable(self) -> bool {
        match self {
            Self::CandleSafetensors { scalar_type } => scalar_type.is_some(),
            Self::LlamaCppGguf { .. } => true,
        }
    }
}

/// Immutable artifact identity retained across resolution and loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImmutableModelIdentity {
    /// Immutable Hugging Face repository commit.
    HuggingFaceCommit {
        /// Repository whose immutable commit was resolved.
        repository: String,
        /// Immutable Hub commit identifier.
        commit: String,
    },
    /// SHA-256 identity of exact local GGUF bytes.
    GgufSha256 {
        /// Lowercase hexadecimal SHA-256 digest.
        digest: String,
    },
}

/// Backward-compatible short name for the application-owned scalar vocabulary.
pub type ScalarType = ApplicationScalarType;
