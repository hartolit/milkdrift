//! Cold-path source description for one unquantized Llama model.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use domain_contracts::ScalarType;

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

/// Files and configuration-declared metadata for one unquantized Llama model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandleLlamaSource {
    config_path: PathBuf,
    weight_paths: Vec<PathBuf>,
    configuration_declared_scalar_type: Option<ScalarType>,
}

impl CandleLlamaSource {
    /// Creates a source from a Hugging Face Llama config and Safetensors shards.
    ///
    /// `configuration_declared_scalar_type` is optional producer metadata. It is
    /// retained independently from scalar types observed later in tensor headers
    /// and never proves that serialized tensors are homogeneous.
    ///
    /// Weight paths are sorted deterministically before being retained.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::MissingWeights`] when `weight_paths` is empty.
    pub fn new(
        config_path: impl Into<PathBuf>,
        mut weight_paths: Vec<PathBuf>,
        configuration_declared_scalar_type: Option<ScalarType>,
    ) -> Result<Self, SourceError> {
        if weight_paths.is_empty() {
            return Err(SourceError::MissingWeights);
        }
        weight_paths.sort();

        Ok(Self {
            config_path: config_path.into(),
            weight_paths,
            configuration_declared_scalar_type,
        })
    }

    /// Returns the Hugging Face model configuration path.
    #[must_use]
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Returns the deterministically sorted Safetensors shard paths.
    #[must_use]
    pub fn weight_paths(&self) -> &[PathBuf] {
        &self.weight_paths
    }

    /// Returns optional configuration-declared scalar metadata.
    ///
    /// This declaration is producer intent only. Observed tensor scalar types
    /// are read independently from every selected Safetensors header.
    #[must_use]
    pub const fn configuration_declared_scalar_type(&self) -> Option<ScalarType> {
        self.configuration_declared_scalar_type
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use domain_contracts::ScalarType;

    use super::CandleLlamaSource;

    #[test]
    fn source_retains_optional_declaration_and_sorts_shards() -> Result<(), super::SourceError> {
        let source = CandleLlamaSource::new(
            "config.json",
            vec![
                PathBuf::from("z.safetensors"),
                PathBuf::from("a.safetensors"),
            ],
            Some(ScalarType::Bf16),
        )?;

        assert_eq!(
            source.weight_paths(),
            [
                PathBuf::from("a.safetensors"),
                PathBuf::from("z.safetensors")
            ]
        );
        assert_eq!(
            source.configuration_declared_scalar_type(),
            Some(ScalarType::Bf16)
        );
        Ok(())
    }

    #[test]
    fn absent_configuration_declaration_is_retained() -> Result<(), super::SourceError> {
        let source = CandleLlamaSource::new(
            "config.json",
            vec![PathBuf::from("model.safetensors")],
            None,
        )?;

        assert_eq!(source.configuration_declared_scalar_type(), None);
        Ok(())
    }
}
