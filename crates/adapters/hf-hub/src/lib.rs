//! Synchronous Hugging Face Hub adapter for resolving cached Llama artifacts.

#![forbid(unsafe_code)]

mod bounded;
mod configuration;
mod discovery;
mod identity;
mod weights;

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::path::PathBuf;

use configuration::read_configuration_declared_scalar_type;
use discovery::resolve_commit_and_files;
use hf_hub::{HFClient, HFClientSync, HFError, HFRepositorySync, RepoTypeModel, split_id};
use identity::{resolve_weight_shard, selected_weight_metadata};
use weights::{direct_weights, indexed_weights, read_index, validate_artifact_path};

const CONFIG_FILE: &str = "config.json";
const TOKENIZER_FILE: &str = "tokenizer.json";
const WEIGHT_INDEX_FILE: &str = "model.safetensors.index.json";
const SINGLE_WEIGHT_FILE: &str = "model.safetensors";

/// Explicit Hugging Face Hub client configuration.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct HubClientConfiguration {
    /// Optional cache root overriding `HF_HOME` resolution.
    pub cache_directory: Option<PathBuf>,
    /// Optional access token. `None` preserves anonymous or environment-derived access.
    pub access_token: Option<String>,
    /// Number of download retries after the initial attempt.
    pub maximum_retries: usize,
}

impl fmt::Debug for HubClientConfiguration {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HubClientConfiguration")
            .field("cache_directory", &self.cache_directory)
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "<redacted>"),
            )
            .field("maximum_retries", &self.maximum_retries)
            .finish()
    }
}

/// Immutable repository and revision selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubModelReference {
    repository: String,
    revision: String,
}

impl HubModelReference {
    /// Creates a validated model reference.
    ///
    /// # Errors
    ///
    /// Returns [`HubError::InvalidRepository`] if `repository` is empty after trimming, or
    /// [`HubError::InvalidRevision`] if `revision` is empty after trimming.
    pub fn new(
        repository: impl Into<String>,
        revision: impl Into<String>,
    ) -> Result<Self, HubError> {
        let repository = repository.into().trim().to_owned();
        let revision = revision.into().trim().to_owned();
        if repository.is_empty() {
            return Err(HubError::InvalidRepository);
        }
        if revision.is_empty() {
            return Err(HubError::InvalidRevision);
        }
        Ok(Self {
            repository,
            revision,
        })
    }

    /// Returns the Hub repository identifier.
    #[must_use]
    pub const fn repository(&self) -> &str {
        self.repository.as_str()
    }

    /// Returns the requested branch, tag, reference, or commit.
    #[must_use]
    pub const fn revision(&self) -> &str {
        self.revision.as_str()
    }
}

/// Local artifact paths and content identity resolved from one immutable Hub commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSafetensorsLlamaArtifacts {
    /// Requested repository.
    pub repository: String,
    /// Requested revision.
    pub revision: String,
    /// Immutable Hub commit returned by repository inspection.
    pub commit: String,
    /// Optional scalar metadata declared by immutable model configuration.
    ///
    /// This is producer-intent evidence only. It does not describe tensor-header
    /// homogeneity or the scalar selected for backend execution.
    pub configuration_declared_scalar_type: Option<ArtifactScalarType>,
    /// Cached model configuration.
    pub config_path: PathBuf,
    /// Cached serialized tokenizer.
    pub tokenizer_path: PathBuf,
    /// Ordered cached Safetensors shards with reusable whole-file identities.
    pub weight_shards: Vec<ResolvedSafetensorsShard>,
}

/// One cached Safetensors shard and the whole-file identity Candle must verify while reading it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSafetensorsShard {
    /// Cached local shard path.
    pub path: PathBuf,
    /// Expected whole-file content identity.
    pub content_identity: ArtifactContentIdentity,
}

/// Whole-file SHA-256 identity and exact byte length for one artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactContentIdentity {
    /// Exact whole-file byte length.
    pub byte_length: u64,
    /// Raw whole-file SHA-256 digest.
    pub sha256: [u8; 32],
    /// Authority by which the identity was established.
    pub authority: ArtifactContentIdentityAuthority,
}

/// Authority for an artifact content identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactContentIdentityAuthority {
    /// SHA-256 came from Git LFS metadata at the resolved Hub commit.
    HuggingFaceLfs,
    /// SHA-256 was established by streaming the downloaded local file.
    ProjectEstablished,
}

/// Scalar type declared by a Hugging Face model configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactScalarType {
    /// IEEE-754 32-bit floating point.
    F32,
    /// IEEE-754 16-bit floating point.
    F16,
    /// Brain floating point.
    Bf16,
}

/// Bounded artifact structure whose configured limit was exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubStructuralLimit {
    /// Model configuration bytes.
    ConfigurationBytes,
    /// Safetensors weight-index bytes.
    WeightIndexBytes,
    /// Weight-map entries in one Safetensors index.
    WeightIndexEntries,
    /// Bytes in one weight-map tensor name.
    WeightIndexTensorNameBytes,
    /// Bytes in one repository-relative artifact path.
    RepositoryPathBytes,
    /// Number of entries returned by recursive repository discovery.
    RepositoryEntries,
    /// Bytes in one repository-tree entry path.
    RepositoryEntryPathBytes,
    /// Aggregate bytes across repository-tree entry paths.
    RepositoryAggregatePathBytes,
    /// Number of selected Safetensors shards.
    SelectedWeightShards,
}

impl HubStructuralLimit {
    const fn label(self) -> &'static str {
        match self {
            Self::ConfigurationBytes => "model configuration bytes",
            Self::WeightIndexBytes => "Safetensors weight-index bytes",
            Self::WeightIndexEntries => "Safetensors weight-index entries",
            Self::WeightIndexTensorNameBytes => "Safetensors weight-index tensor-name bytes",
            Self::RepositoryPathBytes => "repository artifact-path bytes",
            Self::RepositoryEntries => "repository tree entries",
            Self::RepositoryEntryPathBytes => "repository tree entry-path bytes",
            Self::RepositoryAggregatePathBytes => "aggregate repository tree path bytes",
            Self::SelectedWeightShards => "selected Safetensors shards",
        }
    }
}

/// Stable Hub adapter failures.
#[derive(Debug)]
pub enum HubError {
    /// Repository identifier was empty.
    InvalidRepository,
    /// Revision identifier was empty.
    InvalidRevision,
    /// The synchronous Hub client could not be built.
    Client(HFError),
    /// Repository metadata could not be inspected.
    RepositoryInfo(HFError),
    /// Exact selected-file metadata could not be inspected.
    ArtifactMetadata(HFError),
    /// Repository metadata omitted the immutable commit identifier.
    MissingCommit,
    /// Repository metadata omitted its file listing.
    MissingFileListing,
    /// Repository metadata returned a non-canonical immutable commit identifier.
    InvalidCommit,
    /// Recursive repository metadata repeated one file path.
    DuplicateRepositoryFilePath,
    /// A required file is absent from the selected revision.
    MissingArtifact(&'static str),
    /// The repository does not provide supported unquantized Safetensors weights.
    UnsupportedWeightLayout,
    /// A Hub filename attempted to escape the repository namespace.
    UnsafeArtifactPath(String),
    /// A configured bounded artifact structure exceeded its limit.
    StructuralLimitExceeded(HubStructuralLimit),
    /// A cached model configuration could not be read.
    ReadConfiguration(io::Error),
    /// The model configuration JSON or declaration field type was malformed.
    InvalidConfiguration,
    /// A present scalar declaration was not recognized by this adapter.
    UnsupportedScalarDeclaration,
    /// Modern and legacy scalar declarations were both recognized but disagreed.
    ConflictingScalarDeclarations,
    /// The weight index could not be read.
    ReadIndex(io::Error),
    /// The weight index JSON or required value shape was malformed.
    InvalidIndex,
    /// Selected-file metadata omitted one requested shard.
    MissingShardMetadata,
    /// Selected-file metadata repeated one requested shard.
    DuplicateShardMetadata,
    /// Selected-file metadata returned a path or entry type that was not requested.
    UnexpectedShardMetadata,
    /// Git LFS metadata contained a malformed SHA-256 value.
    InvalidLfsContentIdentity,
    /// Hub-reported, LFS-reported, cached, or streamed shard lengths disagreed.
    ShardLengthMismatch,
    /// A cached weight shard was not a regular file.
    InvalidWeightFile,
    /// A cached weight shard could not be inspected or streamed.
    ReadWeight(io::Error),
    /// A required cached artifact could not be resolved or downloaded.
    Download {
        /// Repository-relative filename.
        filename: String,
        /// Upstream Hub failure.
        source: HFError,
    },
}

impl Display for HubError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRepository => formatter.write_str("repository identifier is empty"),
            Self::InvalidRevision => formatter.write_str("revision identifier is empty"),
            Self::Client(error) => write!(formatter, "failed to build Hub client: {error}"),
            Self::RepositoryInfo(error) => {
                write!(formatter, "failed to inspect Hub repository: {error}")
            }
            Self::ArtifactMetadata(error) => {
                write!(
                    formatter,
                    "failed to inspect selected Hub artifacts: {error}"
                )
            }
            Self::MissingCommit => {
                formatter.write_str("Hub repository metadata omitted the commit identifier")
            }
            Self::MissingFileListing => {
                formatter.write_str("Hub repository metadata omitted the file listing")
            }
            Self::InvalidCommit => {
                formatter.write_str("Hub repository metadata returned an invalid commit identifier")
            }
            Self::DuplicateRepositoryFilePath => {
                formatter.write_str("Hub repository metadata repeated a file path")
            }
            Self::MissingArtifact(filename) => {
                write!(formatter, "required Hub artifact is missing: {filename}")
            }
            Self::UnsupportedWeightLayout => {
                formatter.write_str("repository has no supported model.safetensors layout")
            }
            Self::UnsafeArtifactPath(filename) => {
                write!(formatter, "unsafe repository artifact path: {filename}")
            }
            Self::StructuralLimitExceeded(limit) => {
                write!(
                    formatter,
                    "artifact structural limit exceeded: {}",
                    limit.label()
                )
            }
            Self::ReadConfiguration(error) => {
                write!(formatter, "failed to read model configuration: {error}")
            }
            Self::InvalidConfiguration => formatter.write_str(
                "model configuration JSON or scalar declaration field type is malformed",
            ),
            Self::UnsupportedScalarDeclaration => formatter
                .write_str("model configuration contains an unsupported scalar declaration"),
            Self::ConflictingScalarDeclarations => formatter.write_str(
                "model configuration contains conflicting modern and legacy scalar declarations",
            ),
            Self::ReadIndex(error) => write!(formatter, "failed to read weight index: {error}"),
            Self::InvalidIndex => formatter.write_str("invalid Safetensors weight index"),
            Self::MissingShardMetadata => {
                formatter.write_str("selected Hub metadata omitted a Safetensors shard")
            }
            Self::DuplicateShardMetadata => {
                formatter.write_str("selected Hub metadata repeated a Safetensors shard")
            }
            Self::UnexpectedShardMetadata => {
                formatter.write_str("selected Hub metadata returned an unexpected artifact entry")
            }
            Self::InvalidLfsContentIdentity => {
                formatter.write_str("Hub LFS metadata contains an invalid SHA-256 identity")
            }
            Self::ShardLengthMismatch => formatter.write_str(
                "Hub-reported, LFS-reported, cached, or streamed shard lengths disagree",
            ),
            Self::InvalidWeightFile => {
                formatter.write_str("cached Safetensors shard is not a regular file")
            }
            Self::ReadWeight(error) => {
                write!(
                    formatter,
                    "failed to inspect or stream cached Safetensors shard: {error}"
                )
            }
            Self::Download { filename, source } => {
                write!(formatter, "failed to resolve {filename}: {source}")
            }
        }
    }
}

impl Error for HubError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Client(error) | Self::RepositoryInfo(error) | Self::ArtifactMetadata(error) => {
                Some(error)
            }
            Self::ReadConfiguration(error) | Self::ReadIndex(error) | Self::ReadWeight(error) => {
                Some(error)
            }
            Self::Download { source, .. } => Some(source),
            Self::InvalidRepository
            | Self::InvalidRevision
            | Self::MissingCommit
            | Self::MissingFileListing
            | Self::InvalidCommit
            | Self::DuplicateRepositoryFilePath
            | Self::MissingArtifact(_)
            | Self::UnsupportedWeightLayout
            | Self::UnsafeArtifactPath(_)
            | Self::StructuralLimitExceeded(_)
            | Self::InvalidConfiguration
            | Self::UnsupportedScalarDeclaration
            | Self::ConflictingScalarDeclarations
            | Self::InvalidIndex
            | Self::MissingShardMetadata
            | Self::DuplicateShardMetadata
            | Self::UnexpectedShardMetadata
            | Self::InvalidLfsContentIdentity
            | Self::ShardLengthMismatch
            | Self::InvalidWeightFile => None,
        }
    }
}

/// Blocking Hub client intended for a dedicated cold-path host worker.
pub struct HubClient {
    client: HFClientSync,
}

impl HubClient {
    /// Builds a client from environment defaults plus explicit overrides.
    ///
    /// # Errors
    ///
    /// Returns [`HubError::Client`] if the synchronous Hugging Face Hub client cannot be built.
    pub fn new(configuration: HubClientConfiguration) -> Result<Self, HubError> {
        let mut builder = HFClient::builder().retry_max_attempts(configuration.maximum_retries);
        if let Some(access_token) = configuration.access_token {
            builder = builder.token(access_token);
        }
        if let Some(cache_directory) = configuration.cache_directory {
            builder = builder.cache_dir(cache_directory);
        }
        let client = builder.build_sync().map_err(HubError::Client)?;
        Ok(Self { client })
    }

    /// Inspects and resolves all files required by the Candle Llama adapter.
    ///
    /// Artifacts are downloaded from the immutable commit reported for the requested revision.
    ///
    /// # Errors
    ///
    /// Returns a [`HubError`] if repository inspection fails, metadata or required artifacts are
    /// missing, the weight layout or an artifact path is invalid, configuration or index data
    /// cannot be read or parsed, shard identity cannot be established, or an artifact cannot be
    /// downloaded.
    pub fn resolve_safetensors_llama(
        &self,
        reference: &HubModelReference,
    ) -> Result<ResolvedSafetensorsLlamaArtifacts, HubError> {
        let (owner, name) = split_id(reference.repository.as_str());
        let repository = self.client.model(owner, name);
        let (commit, filenames) =
            resolve_commit_and_files(&repository, reference.revision.as_str())?;

        require_file(&filenames, CONFIG_FILE)?;
        require_file(&filenames, TOKENIZER_FILE)?;
        let weight_filenames = if filenames.contains(WEIGHT_INDEX_FILE) {
            let index_path = resolve_file(&repository, commit.as_str(), WEIGHT_INDEX_FILE)?;
            let bytes = read_index(index_path.as_path())?;
            indexed_weights(bytes.as_slice(), &filenames)?
        } else {
            direct_weights(&filenames)?
        };
        let mut weight_metadata =
            selected_weight_metadata(&repository, commit.as_str(), weight_filenames.as_slice())?;

        let config_path = resolve_file(&repository, commit.as_str(), CONFIG_FILE)?;
        let configuration_declared_scalar_type =
            read_configuration_declared_scalar_type(config_path.as_path())?;
        let tokenizer_path = resolve_file(&repository, commit.as_str(), TOKENIZER_FILE)?;
        let mut weight_shards = Vec::with_capacity(weight_filenames.len());
        for filename in weight_filenames {
            let metadata = weight_metadata
                .remove(filename.as_str())
                .ok_or(HubError::MissingShardMetadata)?;
            let path = resolve_file(&repository, commit.as_str(), filename.as_str())?;
            weight_shards.push(resolve_weight_shard(path, metadata)?);
        }
        if !weight_metadata.is_empty() {
            return Err(HubError::UnexpectedShardMetadata);
        }

        Ok(ResolvedSafetensorsLlamaArtifacts {
            repository: reference.repository.clone(),
            revision: reference.revision.clone(),
            commit,
            configuration_declared_scalar_type,
            config_path,
            tokenizer_path,
            weight_shards,
        })
    }
}

fn require_file(available: &BTreeSet<String>, filename: &'static str) -> Result<(), HubError> {
    if available.contains(filename) {
        Ok(())
    } else {
        Err(HubError::MissingArtifact(filename))
    }
}

fn resolve_file(
    repository: &HFRepositorySync<RepoTypeModel>,
    revision: &str,
    filename: &str,
) -> Result<PathBuf, HubError> {
    validate_artifact_path(filename)?;
    repository
        .download_file()
        .filename(filename)
        .revision(revision)
        .send()
        .map_err(|source| HubError::Download {
            filename: filename.to_owned(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::{HubClientConfiguration, HubError, HubModelReference};

    #[test]
    fn client_configuration_debug_redacts_access_tokens() {
        let configuration = HubClientConfiguration {
            cache_directory: None,
            access_token: Some("secret-token".to_owned()),
            maximum_retries: 2,
        };

        let debug = format!("{configuration:?}");
        assert!(!debug.contains("secret-token"));
        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("maximum_retries: 2"));
    }

    #[test]
    fn model_reference_validation_is_explicit() -> Result<(), HubError> {
        let reference = HubModelReference::new(" owner/model ", " revision ")?;
        assert_eq!(reference.repository(), "owner/model");
        assert_eq!(reference.revision(), "revision");
        assert!(matches!(
            HubModelReference::new(" ", "main"),
            Err(HubError::InvalidRepository)
        ));
        assert!(matches!(
            HubModelReference::new("owner/model", " "),
            Err(HubError::InvalidRevision)
        ));
        Ok(())
    }
}
