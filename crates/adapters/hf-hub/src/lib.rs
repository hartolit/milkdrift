//! Synchronous Hugging Face Hub adapter for resolving cached Llama artifacts.

#![forbid(unsafe_code)]

mod bounded;
mod configuration;
mod content;
mod discovery;
mod identity;
mod weights;

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::path::{Path, PathBuf};

use configuration::parse_configuration_declared_scalar_type;
use content::{read_verified_content_bytes, resolve_content_artifact};
use discovery::resolve_commit_and_files;
use hf_hub::{HFClient, HFClientSync, HFError, HFRepositorySync, RepoTypeModel, split_id};
use identity::{resolve_weight_shard, selected_content_metadata, selected_weight_metadata};
use weights::{direct_weights, indexed_weights, validate_artifact_path};

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
    /// Cached model configuration paired with its exact bounded content identity.
    pub config: ResolvedContentArtifact,
    /// Cached serialized tokenizer paired with its exact bounded content identity.
    pub tokenizer: ResolvedContentArtifact,
    /// Ordered cached Safetensors shards with reusable whole-file identities.
    pub weight_shards: Vec<ResolvedSafetensorsShard>,
}

/// One bounded cached JSON artifact and its accepted exact content identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedContentArtifact {
    /// Cached local artifact path.
    pub path: PathBuf,
    /// Accepted exact whole-file content identity.
    pub content_identity: ArtifactContentIdentity,
    /// Bounded artifact role, which determines the reviewed byte ceiling.
    pub kind: ArtifactContentKind,
}

impl ResolvedContentArtifact {
    /// Returns the cached local artifact path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the accepted exact whole-file content identity.
    #[must_use]
    pub const fn content_identity(&self) -> ArtifactContentIdentity {
        self.content_identity
    }

    /// Opens the cached path once, retains its bounded exact bytes, and validates their
    /// length and SHA-256 against the accepted identity before returning those same bytes.
    ///
    /// # Errors
    ///
    /// Returns a [`HubError`] if the path cannot be read, is not a regular file, exceeds
    /// its reviewed bound, or no longer matches the accepted content identity.
    pub fn read_verified_bytes(&self) -> Result<Vec<u8>, HubError> {
        read_verified_content_bytes(self)
    }
}

/// Role of one bounded cached JSON artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactContentKind {
    /// Hugging Face model `config.json`.
    Configuration,
    /// Hugging Face `tokenizer.json`.
    Tokenizer,
    /// Hugging Face Safetensors shard index.
    WeightIndex,
}

impl ArtifactContentKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Configuration => "model configuration",
            Self::Tokenizer => "tokenizer",
            Self::WeightIndex => "Safetensors weight index",
        }
    }
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
    /// Bytes matched the Git blob object ID at the resolved Hub commit, then received this SHA-256.
    HuggingFaceGitBlob,
    /// SHA-256 was established by streaming a local file without provider-bound identity.
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
    /// Serialized tokenizer JSON bytes.
    TokenizerBytes,
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
            Self::TokenizerBytes => "serialized tokenizer JSON bytes",
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

/// Stable, allocation-free classification of Hub resolution failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HubErrorKind {
    /// Repository or revision selection is invalid.
    InvalidSelection,
    /// Hub transport, authentication, cache I/O, or required metadata is unavailable.
    Unavailable,
    /// A required immutable artifact is absent.
    MissingArtifact,
    /// The repository does not provide the supported weight layout.
    UnsupportedLayout,
    /// A bounded artifact structure exceeded its accepted limit.
    StructuralLimit,
    /// Configuration JSON or a declaration field is malformed.
    MalformedConfiguration,
    /// A present scalar declaration is not recognized.
    UnsupportedScalarDeclaration,
    /// Modern and legacy scalar declarations contradict one another.
    ConflictingScalarDeclarations,
    /// Resolved artifact structure, path, or content identity is invalid.
    InvalidArtifact,
}

/// Hub adapter failure with private vendor detail and stable classification.
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
    /// Exact config/tokenizer metadata was absent, repeated, inconsistent, or unexpected.
    InvalidContentMetadata,

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
    /// A cached serialized tokenizer could not be read.
    ReadTokenizer(io::Error),
    /// A bounded config/tokenizer path did not identify a regular file.
    InvalidContentFile(ArtifactContentKind),
    /// Bounded config/tokenizer bytes did not match the accepted exact identity.
    ContentIdentityMismatch(ArtifactContentKind),
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
    /// Git LFS metadata contained a missing or malformed SHA-256 value.
    InvalidLfsContentIdentity,
    /// Hub Git metadata contained a malformed blob object ID.
    InvalidGitContentIdentity,
    /// Hub-reported, LFS-reported, cached, or streamed shard lengths disagreed.
    ShardLengthMismatch,
    /// A non-LFS shard did not match its Git blob object ID at the resolved commit.
    ShardContentIdentityMismatch,
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

impl HubError {
    /// Returns the stable category used by application boundaries.
    #[must_use]
    pub const fn kind(&self) -> HubErrorKind {
        match self {
            Self::InvalidRepository | Self::InvalidRevision => HubErrorKind::InvalidSelection,
            Self::Client(_)
            | Self::RepositoryInfo(_)
            | Self::ArtifactMetadata(_)
            | Self::ReadConfiguration(_)
            | Self::ReadTokenizer(_)
            | Self::ReadIndex(_)
            | Self::ReadWeight(_)
            | Self::Download { .. } => HubErrorKind::Unavailable,
            Self::MissingArtifact(_) => HubErrorKind::MissingArtifact,
            Self::UnsupportedWeightLayout => HubErrorKind::UnsupportedLayout,
            Self::StructuralLimitExceeded(_) => HubErrorKind::StructuralLimit,
            Self::InvalidConfiguration => HubErrorKind::MalformedConfiguration,
            Self::UnsupportedScalarDeclaration => HubErrorKind::UnsupportedScalarDeclaration,
            Self::ConflictingScalarDeclarations => HubErrorKind::ConflictingScalarDeclarations,
            Self::InvalidContentMetadata
            | Self::InvalidCommit
            | Self::DuplicateRepositoryFilePath
            | Self::UnsafeArtifactPath(_)
            | Self::InvalidContentFile(_)
            | Self::ContentIdentityMismatch(_)
            | Self::InvalidIndex
            | Self::MissingShardMetadata
            | Self::DuplicateShardMetadata
            | Self::UnexpectedShardMetadata
            | Self::InvalidLfsContentIdentity
            | Self::InvalidGitContentIdentity
            | Self::ShardLengthMismatch
            | Self::ShardContentIdentityMismatch
            | Self::InvalidWeightFile => HubErrorKind::InvalidArtifact,
        }
    }
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
            Self::InvalidContentMetadata => {
                formatter.write_str("selected configuration, tokenizer, or index metadata is invalid")
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
            Self::ReadTokenizer(error) => {
                write!(formatter, "failed to read serialized tokenizer: {error}")
            }
            Self::InvalidContentFile(kind) => {
                write!(formatter, "cached {} is not a regular file", kind.label())
            }
            Self::ContentIdentityMismatch(kind) => write!(
                formatter,
                "cached {} does not match its accepted content identity",
                kind.label()
            ),
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
                formatter.write_str("Hub LFS metadata lacks a valid SHA-256 identity")
            }
            Self::InvalidGitContentIdentity => {
                formatter.write_str("Hub Git metadata contains an invalid blob object identity")
            }
            Self::ShardLengthMismatch => formatter.write_str(
                "Hub-reported, LFS-reported, cached, or streamed shard lengths disagree",
            ),
            Self::ShardContentIdentityMismatch => formatter.write_str(
                "cached Safetensors shard does not match its Git blob identity at the resolved commit",
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
            Self::ReadConfiguration(error)
            | Self::ReadTokenizer(error)
            | Self::ReadIndex(error)
            | Self::ReadWeight(error) => Some(error),
            Self::Download { source, .. } => Some(source),
            Self::InvalidRepository
            | Self::InvalidRevision
            | Self::InvalidContentMetadata
            | Self::InvalidCommit
            | Self::DuplicateRepositoryFilePath
            | Self::MissingArtifact(_)
            | Self::UnsupportedWeightLayout
            | Self::UnsafeArtifactPath(_)
            | Self::StructuralLimitExceeded(_)
            | Self::InvalidContentFile(_)
            | Self::ContentIdentityMismatch(_)
            | Self::InvalidConfiguration
            | Self::UnsupportedScalarDeclaration
            | Self::ConflictingScalarDeclarations
            | Self::InvalidIndex
            | Self::MissingShardMetadata
            | Self::DuplicateShardMetadata
            | Self::UnexpectedShardMetadata
            | Self::InvalidLfsContentIdentity
            | Self::InvalidGitContentIdentity
            | Self::ShardLengthMismatch
            | Self::ShardContentIdentityMismatch
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
        let has_weight_index = filenames.contains(WEIGHT_INDEX_FILE);
        let content_metadata =
            selected_content_metadata(&repository, commit.as_str(), has_weight_index)?;
        let weight_filenames = if has_weight_index {
            let index_metadata = content_metadata
                .weight_index
                .ok_or(HubError::InvalidContentMetadata)?;
            let index_path = resolve_file(&repository, commit.as_str(), WEIGHT_INDEX_FILE)?;
            let (_, bytes) = resolve_content_artifact(
                index_path,
                index_metadata,
                ArtifactContentKind::WeightIndex,
            )?;
            indexed_weights(bytes.as_slice(), &filenames)?
        } else {
            direct_weights(&filenames)?
        };
        let mut weight_metadata =
            selected_weight_metadata(&repository, commit.as_str(), weight_filenames.as_slice())?;

        let config_path = resolve_file(&repository, commit.as_str(), CONFIG_FILE)?;
        let (config, config_bytes) = resolve_content_artifact(
            config_path,
            content_metadata.configuration,
            ArtifactContentKind::Configuration,
        )?;
        let configuration_declared_scalar_type =
            parse_configuration_declared_scalar_type(config_bytes.as_slice())?;
        let tokenizer_path = resolve_file(&repository, commit.as_str(), TOKENIZER_FILE)?;
        let (tokenizer, _) = resolve_content_artifact(
            tokenizer_path,
            content_metadata.tokenizer,
            ArtifactContentKind::Tokenizer,
        )?;
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
            config,
            tokenizer,
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
    use super::{HubClientConfiguration, HubError, HubErrorKind, HubModelReference};

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
        assert_eq!(
            HubError::InvalidRepository.kind(),
            HubErrorKind::InvalidSelection
        );
        assert!(matches!(
            HubModelReference::new("owner/model", " "),
            Err(HubError::InvalidRevision)
        ));
        Ok(())
    }

    #[test]
    fn stable_error_kinds_distinguish_declarations_and_artifact_failures() {
        assert_eq!(
            HubError::InvalidConfiguration.kind(),
            HubErrorKind::MalformedConfiguration
        );
        assert_eq!(
            HubError::UnsupportedScalarDeclaration.kind(),
            HubErrorKind::UnsupportedScalarDeclaration
        );
        assert_eq!(
            HubError::ConflictingScalarDeclarations.kind(),
            HubErrorKind::ConflictingScalarDeclarations
        );
        assert_eq!(
            HubError::MissingArtifact("config.json").kind(),
            HubErrorKind::MissingArtifact
        );
        assert_eq!(
            HubError::InvalidLfsContentIdentity.kind(),
            HubErrorKind::InvalidArtifact
        );
    }
}
