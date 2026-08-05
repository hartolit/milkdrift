//! Cold-path source description for one unquantized Llama model.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use candle_core::DType;
use domain_contracts::{DeviceKind, ScalarType};

/// Configuration-declared source scalar for an unquantized Llama model.
///
/// This metadata is used by the current homogeneous-tensor compatibility
/// policy. It is not an observation of each Safetensors entry and does not
/// prove that every tensor has this dtype.
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

    pub(crate) const fn execution_dtype(
        self,
        device_kind: DeviceKind,
        supports_bf16: bool,
    ) -> Option<DType> {
        match (self, device_kind) {
            // Candle 0.11 CPU matmul does not support BF16 operands. Source
            // metadata remains BF16 while CPU execution expands to F32.
            (Self::F32, DeviceKind::Cpu | DeviceKind::Cuda) | (Self::Bf16, DeviceKind::Cpu) => {
                Some(DType::F32)
            }
            (Self::F16, DeviceKind::Cpu | DeviceKind::Cuda) => Some(DType::F16),
            (Self::Bf16, DeviceKind::Cuda) if supports_bf16 => Some(DType::BF16),
            _ => None,
        }
    }

    pub(crate) const fn execution_scalar_type(dtype: DType) -> Option<ScalarType> {
        match dtype {
            DType::F32 => Some(ScalarType::F32),
            DType::F16 => Some(ScalarType::F16),
            DType::BF16 => Some(ScalarType::Bf16),
            _ => None,
        }
    }

    pub(crate) const fn source_scalar_type(self) -> ScalarType {
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

    /// Returns the configuration-declared source scalar.
    ///
    /// The current loader checks each observed Safetensors tensor dtype against
    /// this metadata; the declaration itself does not prove tensor homogeneity.
    #[must_use]
    pub const fn scalar_type(&self) -> CandleScalarType {
        self.scalar_type
    }
}

#[cfg(test)]
mod tests {
    use super::CandleScalarType;
    use candle_core::DType;
    use domain_contracts::{DeviceKind, ScalarType};

    #[test]
    fn execution_scalar_policy_is_device_aware() {
        assert_eq!(CandleScalarType::F32.source_scalar_type(), ScalarType::F32);
        assert_eq!(CandleScalarType::F16.source_scalar_type(), ScalarType::F16);
        assert_eq!(
            CandleScalarType::Bf16.source_scalar_type(),
            ScalarType::Bf16
        );
        assert_eq!(
            CandleScalarType::execution_scalar_type(DType::F32),
            Some(ScalarType::F32)
        );
        assert_eq!(
            CandleScalarType::execution_scalar_type(DType::F16),
            Some(ScalarType::F16)
        );
        assert_eq!(
            CandleScalarType::execution_scalar_type(DType::BF16),
            Some(ScalarType::Bf16)
        );

        for kind in [DeviceKind::Cpu, DeviceKind::Cuda] {
            assert_eq!(
                CandleScalarType::F32.execution_dtype(kind, false),
                Some(DType::F32)
            );
            assert_eq!(
                CandleScalarType::F16.execution_dtype(kind, false),
                Some(DType::F16)
            );
        }
        assert_eq!(
            CandleScalarType::Bf16.execution_dtype(DeviceKind::Cpu, false),
            Some(DType::F32)
        );
        assert_eq!(
            CandleScalarType::Bf16.execution_dtype(DeviceKind::Cuda, true),
            Some(DType::BF16)
        );
        assert_eq!(
            CandleScalarType::Bf16.execution_dtype(DeviceKind::Cuda, false),
            None
        );
    }
}
