//! Cold-path source description for one unquantized Llama model.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

/// Exact whole-file content identity that Candle must observe while materializing a shard.
///
/// This value is an expectation supplied by the caller. It deliberately carries
/// no claim about which provider, repository, or local process established that
/// expectation. Candle verifies the exact length and SHA-256 before publishing a
/// model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CandleExpectedContentIdentity {
    byte_length: u64,
    sha256: [u8; 32],
}

impl CandleExpectedContentIdentity {
    /// Creates an exact whole-file content expectation.
    #[must_use]
    pub const fn new(byte_length: u64, sha256: [u8; 32]) -> Self {
        Self {
            byte_length,
            sha256,
        }
    }

    /// Returns the expected exact whole-file byte length.
    #[must_use]
    pub const fn byte_length(self) -> u64 {
        self.byte_length
    }

    /// Returns the expected whole-file SHA-256 digest.
    #[must_use]
    pub const fn sha256(self) -> [u8; 32] {
        self.sha256
    }
}

/// One Safetensors path paired with an optional exact content expectation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandleWeightShard {
    path: PathBuf,
    expected_content: Option<CandleExpectedContentIdentity>,
}

impl CandleWeightShard {
    /// Pairs a Safetensors path with the exact content Candle must observe.
    #[must_use]
    pub fn with_expected_content(
        path: impl Into<PathBuf>,
        expected_content: CandleExpectedContentIdentity,
    ) -> Self {
        Self {
            path: path.into(),
            expected_content: Some(expected_content),
        }
    }

    /// Creates an unverified local shard whose baseline Candle must establish.
    #[must_use]
    pub fn unverified_local(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            expected_content: None,
        }
    }

    /// Returns the local Safetensors path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the caller-supplied exact content expectation, when available.
    #[must_use]
    pub const fn expected_content(&self) -> Option<CandleExpectedContentIdentity> {
        self.expected_content
    }
}

/// Invalid construction of a Candle model source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// At least one Safetensors weight path is required.
    MissingWeights,
    /// Host allocation for the compact source inventory failed.
    Allocation,
}

impl Display for SourceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingWeights => formatter.write_str("at least one weight file is required"),
            Self::Allocation => formatter.write_str("weight source inventory allocation failed"),
        }
    }
}

impl Error for SourceError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CandleConfigurationSource {
    Path(PathBuf),
    Bytes(Vec<u8>),
}

/// Configuration and content-bound Safetensors shards for one Llama model.
///
/// The scalar declaration is deliberately not caller supplied. Candle derives
/// it from the exact bounded `config.json` bytes used for Candle configuration
/// decoding. Shards are sorted deterministically as complete path/expectation
/// pairs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandleLlamaSource {
    configuration: CandleConfigurationSource,
    weight_shards: Vec<CandleWeightShard>,
}

impl CandleLlamaSource {
    /// Creates a source from a Hugging Face Llama config and content-bound
    /// Safetensors shards.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::MissingWeights`] when `weight_shards` is empty.
    pub fn new(
        config_path: impl Into<PathBuf>,
        weight_shards: Vec<CandleWeightShard>,
    ) -> Result<Self, SourceError> {
        Self::with_configuration(
            CandleConfigurationSource::Path(config_path.into()),
            weight_shards,
        )
    }

    /// Creates a source from owned, already selected configuration bytes and
    /// content-bound Safetensors shards.
    ///
    /// The loader parses this exact byte vector and never reopens a configuration
    /// path, allowing callers to preserve a previously verified content binding.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::MissingWeights`] when `weight_shards` is empty.
    pub fn from_config_bytes(
        config_bytes: Vec<u8>,
        weight_shards: Vec<CandleWeightShard>,
    ) -> Result<Self, SourceError> {
        Self::with_configuration(
            CandleConfigurationSource::Bytes(config_bytes),
            weight_shards,
        )
    }

    /// Creates a source from mutable or otherwise unverified local files.
    ///
    /// Candle performs one bounded-buffer whole-file SHA-256 baseline pass on
    /// each retained open shard before device admission.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::MissingWeights`] when `weight_paths` is empty or
    /// [`SourceError::Allocation`] when the compact shard inventory cannot be
    /// allocated.
    pub fn from_local_files(
        config_path: impl Into<PathBuf>,
        weight_paths: Vec<PathBuf>,
    ) -> Result<Self, SourceError> {
        if weight_paths.is_empty() {
            return Err(SourceError::MissingWeights);
        }
        let mut weight_shards = Vec::new();
        weight_shards
            .try_reserve_exact(weight_paths.len())
            .map_err(|_| SourceError::Allocation)?;
        weight_shards.extend(
            weight_paths
                .into_iter()
                .map(CandleWeightShard::unverified_local),
        );
        Self::new(config_path, weight_shards)
    }

    /// Returns the late-bound Hugging Face model configuration path, when this
    /// source was constructed from an arbitrary local path.
    #[must_use]
    pub fn config_path(&self) -> Option<&Path> {
        match &self.configuration {
            CandleConfigurationSource::Path(path) => Some(path),
            CandleConfigurationSource::Bytes(_) => None,
        }
    }

    /// Returns the exact owned configuration bytes, when this source was
    /// constructed from previously selected bytes.
    #[must_use]
    pub fn config_bytes(&self) -> Option<&[u8]> {
        match &self.configuration {
            CandleConfigurationSource::Path(_) => None,
            CandleConfigurationSource::Bytes(bytes) => Some(bytes.as_slice()),
        }
    }

    pub(crate) const fn configuration(&self) -> &CandleConfigurationSource {
        &self.configuration
    }

    /// Returns deterministically sorted path/identity shard pairs.
    #[must_use]
    pub fn weight_shards(&self) -> &[CandleWeightShard] {
        &self.weight_shards
    }

    fn with_configuration(
        configuration: CandleConfigurationSource,
        mut weight_shards: Vec<CandleWeightShard>,
    ) -> Result<Self, SourceError> {
        if weight_shards.is_empty() {
            return Err(SourceError::MissingWeights);
        }
        weight_shards.sort_unstable_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.expected_content.cmp(&right.expected_content))
        });

        Ok(Self {
            configuration,
            weight_shards,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{CandleExpectedContentIdentity, CandleLlamaSource, CandleWeightShard, SourceError};

    #[test]
    fn source_sorts_complete_path_expectation_pairs() -> Result<(), SourceError> {
        let first_digest = [1_u8; 32];
        let second_digest = [2_u8; 32];
        let source = CandleLlamaSource::new(
            "config.json",
            vec![
                CandleWeightShard::with_expected_content(
                    "z.safetensors",
                    CandleExpectedContentIdentity::new(7, second_digest),
                ),
                CandleWeightShard::with_expected_content(
                    "a.safetensors",
                    CandleExpectedContentIdentity::new(5, first_digest),
                ),
            ],
        )?;

        let expected = [
            CandleWeightShard::with_expected_content(
                "a.safetensors",
                CandleExpectedContentIdentity::new(5, first_digest),
            ),
            CandleWeightShard::with_expected_content(
                "z.safetensors",
                CandleExpectedContentIdentity::new(7, second_digest),
            ),
        ];
        assert_eq!(source.weight_shards(), expected.as_slice());
        Ok(())
    }

    #[test]
    fn local_convenience_marks_every_shard_unverified() -> Result<(), SourceError> {
        let source = CandleLlamaSource::from_local_files(
            "config.json",
            vec![
                PathBuf::from("z.safetensors"),
                PathBuf::from("a.safetensors"),
            ],
        )?;
        let expected = [
            CandleWeightShard::unverified_local("a.safetensors"),
            CandleWeightShard::unverified_local("z.safetensors"),
        ];
        assert_eq!(source.weight_shards(), expected.as_slice());
        assert_eq!(
            source.config_path(),
            Some(std::path::Path::new("config.json"))
        );
        assert_eq!(source.config_bytes(), None);
        Ok(())
    }

    #[test]
    fn owned_configuration_bytes_are_retained_without_a_path() -> Result<(), SourceError> {
        let source = CandleLlamaSource::from_config_bytes(
            br#"{"model_type":"llama"}"#.to_vec(),
            vec![CandleWeightShard::unverified_local("model.safetensors")],
        )?;
        assert_eq!(
            source.config_bytes(),
            Some(br#"{"model_type":"llama"}"#.as_slice())
        );
        assert_eq!(source.config_path(), None);
        Ok(())
    }

    #[test]
    fn source_requires_at_least_one_weight() {
        assert_eq!(
            CandleLlamaSource::new("config.json", Vec::new()),
            Err(SourceError::MissingWeights)
        );
        assert_eq!(
            CandleLlamaSource::from_local_files("config.json", Vec::new()),
            Err(SourceError::MissingWeights)
        );
        assert_eq!(
            CandleLlamaSource::from_config_bytes(Vec::new(), Vec::new()),
            Err(SourceError::MissingWeights)
        );
    }
}
