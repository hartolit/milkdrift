//! Cold-path source description for one unquantized Llama model.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

/// Whole-shard identity authority supplied with one Safetensors path.
///
/// Both identity-bearing variants provide an exact byte length and SHA-256.
/// They remain distinct so callers and diagnostics do not conflate identity
/// proven by an immutable artifact source with identity computed from a mutable
/// path. Candle revalidates project-established identity before admission and
/// verifies either digest while materializing the retained file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandleShardIdentity {
    /// Identity obtained from a source whose content-addressing and immutability
    /// semantics were independently verified.
    VerifiedImmutable {
        /// Exact whole-file byte length.
        byte_length: u64,
        /// Whole-file SHA-256 digest.
        sha256: [u8; 32],
    },
    /// Identity established by project code by hashing the complete shard,
    /// without claiming that the original path itself is immutable. Candle
    /// rehashes the retained file against this identity before device admission.
    ProjectEstablished {
        /// Exact whole-file byte length.
        byte_length: u64,
        /// Whole-file SHA-256 digest.
        sha256: [u8; 32],
    },
    /// No reusable whole-file identity is available. Candle establishes one
    /// from its retained open file before device admission.
    Unverified,
}

impl CandleShardIdentity {
    pub(crate) const fn supplied(self) -> Option<(u64, [u8; 32])> {
        match self {
            Self::VerifiedImmutable {
                byte_length,
                sha256,
            }
            | Self::ProjectEstablished {
                byte_length,
                sha256,
            } => Some((byte_length, sha256)),
            Self::Unverified => None,
        }
    }
}

/// One Safetensors path paired with its whole-file identity authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandleWeightShard {
    path: PathBuf,
    identity: CandleShardIdentity,
}

impl CandleWeightShard {
    /// Pairs a local Safetensors path with its identity authority.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, identity: CandleShardIdentity) -> Self {
        Self {
            path: path.into(),
            identity,
        }
    }

    /// Creates an unverified local shard.
    #[must_use]
    pub fn unverified(path: impl Into<PathBuf>) -> Self {
        Self::new(path, CandleShardIdentity::Unverified)
    }

    /// Returns the local Safetensors path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the whole-file identity authority paired with the path.
    #[must_use]
    pub const fn identity(&self) -> CandleShardIdentity {
        self.identity
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

/// Configuration and identity-bearing Safetensors shards for one Llama model.
///
/// The scalar declaration is deliberately not caller supplied. Candle derives
/// it from the exact bounded `config.json` bytes used for Candle configuration
/// decoding. Shards are sorted deterministically as complete path/identity
/// pairs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandleLlamaSource {
    config_path: PathBuf,
    weight_shards: Vec<CandleWeightShard>,
}

impl CandleLlamaSource {
    /// Creates a source from a Hugging Face Llama config and identity-bearing
    /// Safetensors shards.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::MissingWeights`] when `weight_shards` is empty.
    pub fn new(
        config_path: impl Into<PathBuf>,
        mut weight_shards: Vec<CandleWeightShard>,
    ) -> Result<Self, SourceError> {
        if weight_shards.is_empty() {
            return Err(SourceError::MissingWeights);
        }
        weight_shards.sort_unstable_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.identity.cmp(&right.identity))
        });

        Ok(Self {
            config_path: config_path.into(),
            weight_shards,
        })
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
        weight_shards.extend(weight_paths.into_iter().map(CandleWeightShard::unverified));
        Self::new(config_path, weight_shards)
    }

    /// Returns the Hugging Face model configuration path.
    #[must_use]
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Returns deterministically sorted path/identity shard pairs.
    #[must_use]
    pub fn weight_shards(&self) -> &[CandleWeightShard] {
        &self.weight_shards
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{CandleLlamaSource, CandleShardIdentity, CandleWeightShard, SourceError};

    #[test]
    fn source_sorts_complete_path_identity_pairs() -> Result<(), SourceError> {
        let first_digest = [1_u8; 32];
        let second_digest = [2_u8; 32];
        let source = CandleLlamaSource::new(
            "config.json",
            vec![
                CandleWeightShard::new(
                    "z.safetensors",
                    CandleShardIdentity::VerifiedImmutable {
                        byte_length: 7,
                        sha256: second_digest,
                    },
                ),
                CandleWeightShard::new(
                    "a.safetensors",
                    CandleShardIdentity::ProjectEstablished {
                        byte_length: 5,
                        sha256: first_digest,
                    },
                ),
            ],
        )?;

        let expected = [
            CandleWeightShard::new(
                "a.safetensors",
                CandleShardIdentity::ProjectEstablished {
                    byte_length: 5,
                    sha256: first_digest,
                },
            ),
            CandleWeightShard::new(
                "z.safetensors",
                CandleShardIdentity::VerifiedImmutable {
                    byte_length: 7,
                    sha256: second_digest,
                },
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
            CandleWeightShard::unverified("a.safetensors"),
            CandleWeightShard::unverified("z.safetensors"),
        ];
        assert_eq!(source.weight_shards(), expected.as_slice());
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
    }
}
