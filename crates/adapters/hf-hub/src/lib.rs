//! Synchronous Hugging Face Hub adapter for resolving cached Llama artifacts.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Component, Path, PathBuf};

use hf_hub::{HFClient, HFClientSync, HFError, HFRepositorySync, RepoTypeModel, split_id};
use serde::Deserialize;

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

/// Local immutable artifact paths resolved from one Hub revision.
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
    /// Ordered cached Safetensors shards.
    pub weight_paths: Vec<PathBuf>,
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
    /// Repository metadata omitted the immutable commit identifier.
    MissingCommit,
    /// Repository metadata omitted its file listing.
    MissingFileListing,
    /// A required file is absent from the selected revision.
    MissingArtifact(&'static str),
    /// The repository does not provide supported unquantized Safetensors weights.
    UnsupportedWeightLayout,
    /// A Hub filename attempted to escape the repository namespace.
    UnsafeArtifactPath(String),
    /// A cached model configuration could not be read.
    ReadConfiguration(std::io::Error),
    /// The model configuration JSON was malformed.
    InvalidConfiguration(serde_json::Error),
    /// The weight index could not be read.
    ReadIndex(std::io::Error),
    /// The weight index JSON was malformed.
    InvalidIndex(serde_json::Error),
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
            Self::MissingCommit => {
                formatter.write_str("Hub repository metadata omitted the commit identifier")
            }
            Self::MissingFileListing => {
                formatter.write_str("Hub repository metadata omitted the file listing")
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
            Self::ReadConfiguration(error) => {
                write!(formatter, "failed to read model configuration: {error}")
            }
            Self::InvalidConfiguration(error) => {
                write!(formatter, "invalid model configuration: {error}")
            }
            Self::ReadIndex(error) => write!(formatter, "failed to read weight index: {error}"),
            Self::InvalidIndex(error) => write!(formatter, "invalid weight index: {error}"),
            Self::Download { filename, source } => {
                write!(formatter, "failed to resolve {filename}: {source}")
            }
        }
    }
}

impl Error for HubError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Client(error) | Self::RepositoryInfo(error) => Some(error),
            Self::ReadConfiguration(error) | Self::ReadIndex(error) => Some(error),
            Self::InvalidConfiguration(error) | Self::InvalidIndex(error) => Some(error),
            Self::Download { source, .. } => Some(source),
            Self::InvalidRepository
            | Self::InvalidRevision
            | Self::MissingCommit
            | Self::MissingFileListing
            | Self::MissingArtifact(_)
            | Self::UnsupportedWeightLayout
            | Self::UnsafeArtifactPath(_) => None,
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
    /// cannot be read or parsed, or an artifact cannot be downloaded.
    pub fn resolve_safetensors_llama(
        &self,
        reference: &HubModelReference,
    ) -> Result<ResolvedSafetensorsLlamaArtifacts, HubError> {
        let (owner, name) = split_id(reference.repository.as_str());
        let repository = self.client.model(owner, name);
        let information = repository
            .info()
            .revision(reference.revision.clone())
            .send()
            .map_err(HubError::RepositoryInfo)?;
        let filenames: BTreeSet<String> = information
            .siblings
            .ok_or(HubError::MissingFileListing)?
            .into_iter()
            .map(|sibling| sibling.rfilename)
            .collect();

        require_file(&filenames, CONFIG_FILE)?;
        require_file(&filenames, TOKENIZER_FILE)?;

        let commit = information.sha.ok_or(HubError::MissingCommit)?;
        let weight_filenames = if filenames.contains(WEIGHT_INDEX_FILE) {
            let index_path = resolve_file(&repository, commit.as_str(), WEIGHT_INDEX_FILE)?;
            let bytes = fs::read(index_path).map_err(HubError::ReadIndex)?;
            indexed_weights(bytes.as_slice(), &filenames)?
        } else {
            direct_weights(&filenames)?
        };

        let config_path = resolve_file(&repository, commit.as_str(), CONFIG_FILE)?;
        let configuration_declared_scalar_type =
            read_configuration_declared_scalar_type(config_path.as_path())?;
        let tokenizer_path = resolve_file(&repository, commit.as_str(), TOKENIZER_FILE)?;
        let mut weight_paths = Vec::with_capacity(weight_filenames.len());
        for filename in weight_filenames {
            weight_paths.push(resolve_file(
                &repository,
                commit.as_str(),
                filename.as_str(),
            )?);
        }

        Ok(ResolvedSafetensorsLlamaArtifacts {
            repository: reference.repository.clone(),
            revision: reference.revision.clone(),
            commit,
            configuration_declared_scalar_type,
            config_path,
            tokenizer_path,
            weight_paths,
        })
    }
}

#[derive(Deserialize)]
struct SafetensorsIndex {
    weight_map: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ModelConfiguration {
    #[serde(default)]
    dtype: Option<String>,
    #[serde(default)]
    torch_dtype: Option<String>,
}

fn read_configuration_declared_scalar_type(
    path: &Path,
) -> Result<Option<ArtifactScalarType>, HubError> {
    let bytes = fs::read(path).map_err(HubError::ReadConfiguration)?;
    parse_configuration_declared_scalar_type(bytes.as_slice())
        .map_err(HubError::InvalidConfiguration)
}

fn parse_configuration_declared_scalar_type(
    bytes: &[u8],
) -> Result<Option<ArtifactScalarType>, serde_json::Error> {
    let configuration: ModelConfiguration = serde_json::from_slice(bytes)?;
    Ok(configuration
        .dtype
        .as_deref()
        .and_then(parse_scalar_type)
        .or_else(|| {
            configuration
                .torch_dtype
                .as_deref()
                .and_then(parse_scalar_type)
        }))
}

fn parse_scalar_type(value: &str) -> Option<ArtifactScalarType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "float32" | "f32" => Some(ArtifactScalarType::F32),
        "float16" | "half" | "f16" => Some(ArtifactScalarType::F16),
        "bfloat16" | "bf16" => Some(ArtifactScalarType::Bf16),
        _ => None,
    }
}

fn indexed_weights(bytes: &[u8], available: &BTreeSet<String>) -> Result<Vec<String>, HubError> {
    let index: SafetensorsIndex = serde_json::from_slice(bytes).map_err(HubError::InvalidIndex)?;
    let mut weights = BTreeSet::new();
    for filename in index.weight_map.into_values() {
        validate_artifact_path(filename.as_str())?;
        if !filename.ends_with(".safetensors") || !available.contains(&filename) {
            return Err(HubError::UnsupportedWeightLayout);
        }
        weights.insert(filename);
    }
    if weights.is_empty() {
        return Err(HubError::UnsupportedWeightLayout);
    }
    Ok(weights.into_iter().collect())
}

fn direct_weights(available: &BTreeSet<String>) -> Result<Vec<String>, HubError> {
    if available.contains(SINGLE_WEIGHT_FILE) {
        return Ok(vec![SINGLE_WEIGHT_FILE.to_owned()]);
    }

    let mut shards = Vec::new();
    for filename in available {
        if let Some((index, total)) = parse_standard_shard(filename) {
            shards.push((index, total, filename.clone()));
        }
    }
    let Some(expected_total) = shards.first().map(|(_, total, _)| *total) else {
        return Err(HubError::UnsupportedWeightLayout);
    };
    if expected_total == 0
        || shards.len() != expected_total
        || shards.iter().any(|(_, total, _)| *total != expected_total)
    {
        return Err(HubError::UnsupportedWeightLayout);
    }
    shards.sort_unstable_by_key(|(index, _, _)| *index);
    if shards
        .iter()
        .enumerate()
        .any(|(offset, (index, _, _))| *index != offset + 1)
    {
        return Err(HubError::UnsupportedWeightLayout);
    }
    Ok(shards
        .into_iter()
        .map(|(_, _, filename)| filename)
        .collect())
}

fn parse_standard_shard(filename: &str) -> Option<(usize, usize)> {
    let stem = filename
        .strip_prefix("model-")?
        .strip_suffix(".safetensors")?;
    let (index, total) = stem.split_once("-of-")?;
    if index.len() != 5
        || total.len() != 5
        || !index.bytes().all(|byte| byte.is_ascii_digit())
        || !total.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some((index.parse().ok()?, total.parse().ok()?))
}

fn require_file(available: &BTreeSet<String>, filename: &'static str) -> Result<(), HubError> {
    if available.contains(filename) {
        Ok(())
    } else {
        Err(HubError::MissingArtifact(filename))
    }
}

fn validate_artifact_path(filename: &str) -> Result<(), HubError> {
    let path = Path::new(filename);
    if filename.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(HubError::UnsafeArtifactPath(filename.to_owned()));
    }
    Ok(())
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
    use super::{
        ArtifactScalarType, HubClientConfiguration, direct_weights, indexed_weights,
        parse_configuration_declared_scalar_type, parse_scalar_type,
    };
    use std::collections::BTreeSet;

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
    fn index_deduplicates_and_orders_shards() {
        let available = BTreeSet::from([
            "model-00001-of-00002.safetensors".to_owned(),
            "model-00002-of-00002.safetensors".to_owned(),
        ]);
        let index = br#"{
            "weight_map": {
                "layer.1": "model-00002-of-00002.safetensors",
                "layer.0": "model-00001-of-00002.safetensors",
                "layer.2": "model-00002-of-00002.safetensors"
            }
        }"#;
        let result = indexed_weights(index, &available);
        assert!(result.is_ok());
        assert_eq!(
            result.ok(),
            Some(vec![
                "model-00001-of-00002.safetensors".to_owned(),
                "model-00002-of-00002.safetensors".to_owned(),
            ])
        );
    }

    #[test]
    fn direct_layout_rejects_unrelated_safetensors() {
        let available = BTreeSet::from(["adapter_model.safetensors".to_owned()]);
        assert!(direct_weights(&available).is_err());
    }

    #[test]
    fn direct_layout_rejects_incomplete_shards() {
        let available = BTreeSet::from([
            "model-00001-of-00003.safetensors".to_owned(),
            "model-00003-of-00003.safetensors".to_owned(),
        ]);
        assert!(direct_weights(&available).is_err());
    }

    #[test]
    fn scalar_type_parser_is_explicit() {
        assert_eq!(parse_scalar_type("float32"), Some(ArtifactScalarType::F32));
        assert_eq!(parse_scalar_type("HALF"), Some(ArtifactScalarType::F16));
        assert_eq!(parse_scalar_type(" bf16 "), Some(ArtifactScalarType::Bf16));
        assert_eq!(parse_scalar_type("float8_e4m3fn"), None);
    }

    #[test]
    fn configuration_parser_recognizes_modern_and_legacy_declarations()
    -> Result<(), serde_json::Error> {
        assert_eq!(
            parse_configuration_declared_scalar_type(br#"{"dtype":"bfloat16"}"#)?,
            Some(ArtifactScalarType::Bf16)
        );
        assert_eq!(
            parse_configuration_declared_scalar_type(br#"{"torch_dtype":"float16"}"#)?,
            Some(ArtifactScalarType::F16)
        );
        Ok(())
    }

    #[test]
    fn configuration_parser_retains_absent_declaration() -> Result<(), serde_json::Error> {
        assert_eq!(parse_configuration_declared_scalar_type(br"{}")?, None);
        assert_eq!(
            parse_configuration_declared_scalar_type(br#"{"dtype":null,"torch_dtype":null}"#)?,
            None
        );
        Ok(())
    }

    #[test]
    fn configuration_parser_prefers_recognized_dtype_then_legacy_fallback()
    -> Result<(), serde_json::Error> {
        assert_eq!(
            parse_configuration_declared_scalar_type(
                br#"{"dtype":"bfloat16","torch_dtype":"float16"}"#
            )?,
            Some(ArtifactScalarType::Bf16)
        );
        assert_eq!(
            parse_configuration_declared_scalar_type(
                br#"{"dtype":"float8_e4m3fn","torch_dtype":"float16"}"#
            )?,
            Some(ArtifactScalarType::F16)
        );
        Ok(())
    }

    #[test]
    fn unsafe_artifact_paths_are_rejected() {
        assert!(super::validate_artifact_path("../model.safetensors").is_err());
        assert!(super::validate_artifact_path("/tmp/model.safetensors").is_err());
        assert!(super::validate_artifact_path("weights/model.safetensors").is_ok());
    }
}
