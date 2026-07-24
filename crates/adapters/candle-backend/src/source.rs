//! Cold-path source description for one unquantized Llama model.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use candle_core::DType;
use domain_contracts::ScalarType;

/// Scalar type stored in the source weight tensors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandleScalarType {
    /// IEEE-754 32-bit floating point.
    F32,
    /// IEEE-754 16-bit floating point.
    F16,
    /// Brain floating point.
    Bf16,
}

impl CandleScalarType {
    pub(crate) const fn weight_dtype(self) -> DType {
        match self {
            Self::F32 => DType::F32,
            Self::F16 => DType::F16,
            Self::Bf16 => DType::BF16,
        }
    }

    pub(crate) const fn execution_dtype(self) -> DType {
        // Candle 0.11 CPU matmul does not support BF16 operands. Preserve BF16
        // as source metadata while executing BF16-sourced CPU models in F32.
        match self {
            Self::F32 | Self::Bf16 => DType::F32,
            Self::F16 => DType::F16,
        }
    }

    pub(crate) const fn domain_type(self) -> ScalarType {
        match self {
            Self::F32 => ScalarType::F32,
            Self::F16 => ScalarType::F16,
            Self::Bf16 => ScalarType::Bf16,
        }
    }

    pub(crate) const fn weight_bytes_per_element(self) -> u64 {
        match self {
            Self::F32 => 4,
            Self::F16 | Self::Bf16 => 2,
        }
    }

    pub(crate) const fn execution_bytes_per_element(self) -> u64 {
        match self {
            Self::F32 | Self::Bf16 => 4,
            Self::F16 => 2,
        }
    }
}

/// Invalid construction of a Candle model source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// At least one Safetensors weight path is required.
    MissingWeights,
}

impl Display for SourceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingWeights => formatter.write_str("at least one weight file is required"),
        }
    }
}

impl Error for SourceError {}

/// Files required to inspect and load one unquantized Llama model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandleLlamaSource {
    config_path: PathBuf,
    weight_paths: Vec<PathBuf>,
    scalar_type: CandleScalarType,
}

impl CandleLlamaSource {
    /// Creates a source from a Hugging Face Llama config and Safetensors shards.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::MissingWeights`] when `weight_paths` is empty.
    pub fn new(
        config_path: impl Into<PathBuf>,
        weight_paths: Vec<PathBuf>,
        scalar_type: CandleScalarType,
    ) -> Result<Self, SourceError> {
        if weight_paths.is_empty() {
            return Err(SourceError::MissingWeights);
        }

        Ok(Self {
            config_path: config_path.into(),
            weight_paths,
            scalar_type,
        })
    }

    /// Returns the Hugging Face model configuration path.
    #[must_use]
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Returns the ordered Safetensors shard paths.
    #[must_use]
    pub fn weight_paths(&self) -> &[PathBuf] {
        &self.weight_paths
    }

    /// Returns the scalar type stored in the source weight tensors.
    #[must_use]
    pub const fn scalar_type(&self) -> CandleScalarType {
        self.scalar_type
    }
}
