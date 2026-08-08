//! Single ownership of immutable model, source, and canonical snapshot identity.

use std::fs;
use std::path::{Path, PathBuf};

use application_runtime::{
    ApplicationEngine, ApplicationModelFormat, ApplicationScalarType, ApplicationSource,
    ChatCompatibility, ModelSelection, PromptCompatibilityProfile, ResolvedModel,
};
use domain_contracts::{ScalarType, ScalarTypeSet};

use crate::error::{BenchmarkError, BenchmarkResult};

use super::{MODEL_REPOSITORY, MODEL_REVISION};

pub(super) const EXPECTED_VOCABULARY_SIZE: u32 = 32_000;
pub(super) const EXPECTED_CONTEXT_TOKENS: u32 = 2_048;
pub(super) const MODEL_CONFIGURATION_DECLARED_SCALAR: Option<ApplicationScalarType> =
    Some(ApplicationScalarType::Bf16);
pub(super) const MODEL_DOMAIN_CONFIGURATION_DECLARED_SCALAR: Option<ScalarType> =
    Some(ScalarType::Bf16);
pub(super) const MODEL_OBSERVED_TENSOR_SCALARS: ScalarTypeSet =
    ScalarTypeSet::from_scalar(ScalarType::Bf16);

const CONFIG_FILE: &str = "config.json";
const WEIGHT_FILE: &str = "model.safetensors";

pub(super) struct SnapshotArtifacts {
    pub(super) config_path: PathBuf,
    pub(super) weight_path: PathBuf,
}

pub(super) fn validate_exact_selection(selection: &ModelSelection) -> BenchmarkResult {
    if selection.repository() != MODEL_REPOSITORY || selection.revision() != MODEL_REVISION {
        return Err(BenchmarkError::new(format!(
            "external model selection must be exactly {MODEL_REPOSITORY}@{MODEL_REVISION}, received {}@{}",
            selection.repository(),
            selection.revision()
        )));
    }
    Ok(())
}

pub(super) fn validate_resolved_facts(
    model: &ResolvedModel,
    selection: &ModelSelection,
) -> BenchmarkResult<Option<ApplicationScalarType>> {
    validate_exact_selection(selection)?;
    let configuration_declared_scalar_type = model.configuration_declared_scalar_type();
    if model.selection() != selection
        || model.identity().repository() != MODEL_REPOSITORY
        || model.identity().commit() != MODEL_REVISION
        || model.engine() != ApplicationEngine::Candle
        || model.source() != ApplicationSource::HuggingFaceHub
        || model.format() != ApplicationModelFormat::Safetensors
        || configuration_declared_scalar_type != MODEL_CONFIGURATION_DECLARED_SCALAR
        || model.vocabulary_size() != EXPECTED_VOCABULARY_SIZE
        || model.chat_compatibility()
            != ChatCompatibility::Supported(PromptCompatibilityProfile::TinyLlamaChatV1)
    {
        return Err(BenchmarkError::new(format!(
            "resolved model did not retain the exact immutable TinyLlama Candle/Hub/Safetensors/optional-BF16-declaration/chat facts: {model:?}"
        )));
    }
    Ok(configuration_declared_scalar_type)
}

pub(super) fn canonical_snapshot_artifacts(
    cache_directory: &Path,
) -> BenchmarkResult<SnapshotArtifacts> {
    let canonical_cache = cache_directory.canonicalize().map_err(|error| {
        BenchmarkError::new(format!(
            "could not canonicalize explicit cache directory {}: {error}",
            cache_directory.display()
        ))
    })?;
    if !fs::metadata(&canonical_cache)
        .map_err(|error| {
            BenchmarkError::new(format!(
                "could not inspect canonical cache directory {}: {error}",
                canonical_cache.display()
            ))
        })?
        .is_dir()
    {
        return Err(BenchmarkError::new(format!(
            "canonical cache path {} is not a directory",
            canonical_cache.display()
        )));
    }

    let (config_candidate, weight_candidate) = snapshot_artifact_paths(&canonical_cache);
    let (config_path, config_bytes) =
        canonical_regular_file(&canonical_cache, &config_candidate, CONFIG_FILE)?;
    let (weight_path, weight_bytes) =
        canonical_regular_file(&canonical_cache, &weight_candidate, WEIGHT_FILE)?;
    if config_path == weight_path {
        return Err(BenchmarkError::new(
            "fixed config and Safetensors snapshot entries resolved to the same file",
        ));
    }
    if config_bytes == 0 || weight_bytes == 0 {
        return Err(BenchmarkError::new(
            "fixed config and Safetensors snapshot artifacts must both be nonempty",
        ));
    }

    Ok(SnapshotArtifacts {
        config_path,
        weight_path,
    })
}

fn snapshot_artifact_paths(cache_directory: &Path) -> (PathBuf, PathBuf) {
    let repository_directory = format!("models--{}", MODEL_REPOSITORY.replace('/', "--"));
    let snapshot = cache_directory
        .join(repository_directory)
        .join("snapshots")
        .join(MODEL_REVISION);
    (snapshot.join(CONFIG_FILE), snapshot.join(WEIGHT_FILE))
}

fn canonical_regular_file(
    canonical_cache: &Path,
    candidate: &Path,
    label: &str,
) -> BenchmarkResult<(PathBuf, u64)> {
    let canonical = candidate.canonicalize().map_err(|error| {
        BenchmarkError::new(format!(
            "fixed snapshot artifact {} could not be canonicalized: {error}",
            candidate.display()
        ))
    })?;
    if !canonical.starts_with(canonical_cache) {
        return Err(BenchmarkError::new(format!(
            "fixed snapshot artifact {label} resolves outside canonical cache {}: {}",
            canonical_cache.display(),
            canonical.display()
        )));
    }
    let metadata = fs::metadata(&canonical).map_err(|error| {
        BenchmarkError::new(format!(
            "could not inspect canonical snapshot artifact {}: {error}",
            canonical.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(BenchmarkError::new(format!(
            "fixed snapshot artifact {label} is not a regular file: {}",
            canonical.display()
        )));
    }
    Ok((canonical, metadata.len()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        MODEL_REPOSITORY, MODEL_REVISION, canonical_snapshot_artifacts, snapshot_artifact_paths,
    };
    use crate::external::model::MODEL_ARCHITECTURE;

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn fixed_identity_and_linux_snapshot_path_are_exact() -> Result<(), String> {
        assert_eq!(MODEL_REPOSITORY, "TinyLlama/TinyLlama-1.1B-Chat-v1.0");
        assert_eq!(MODEL_REVISION, "fe8a4ea1ffedaf415f4da2f062534de366a451e6");
        assert_eq!(MODEL_ARCHITECTURE, "Llama");

        let cache = PathBuf::from("/tmp/fixed-hf-cache");
        let (config, weights) = snapshot_artifact_paths(&cache);
        let snapshot = cache
            .join("models--TinyLlama--TinyLlama-1.1B-Chat-v1.0")
            .join("snapshots")
            .join(MODEL_REVISION);
        if config != snapshot.join("config.json") || weights != snapshot.join("model.safetensors") {
            return Err("fixed snapshot paths changed".to_owned());
        }
        Ok(())
    }

    #[test]
    fn snapshot_artifacts_are_canonical_regular_files_under_the_cache() -> Result<(), String> {
        let fixture = CacheFixture::create()?;
        let (config, weights) = fixture.write_regular_artifacts()?;
        let artifacts =
            canonical_snapshot_artifacts(&fixture.cache).map_err(|error| error.to_string())?;
        assert_eq!(
            artifacts.config_path,
            config.canonicalize().map_err(|error| error.to_string())?
        );
        assert_eq!(
            artifacts.weight_path,
            weights.canonicalize().map_err(|error| error.to_string())?
        );
        assert_eq!(
            fs::metadata(&artifacts.weight_path)
                .map_err(|error| error.to_string())?
                .len(),
            8
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn snapshot_artifact_symlink_cannot_escape_the_canonical_cache() -> Result<(), String> {
        use std::os::unix::fs::symlink;

        let fixture = CacheFixture::create()?;
        let (config, weights) = snapshot_artifact_paths(&fixture.cache);
        let outside = fixture.root.join("outside-config.json");
        fs::write(&outside, b"outside").map_err(|error| error.to_string())?;
        symlink(&outside, &config).map_err(|error| error.to_string())?;
        fs::write(weights, b"weights").map_err(|error| error.to_string())?;

        let error = canonical_snapshot_artifacts(&fixture.cache)
            .err()
            .ok_or_else(|| "escaping snapshot symlink unexpectedly succeeded".to_owned())?;
        assert!(error.to_string().contains("outside canonical cache"));
        Ok(())
    }

    #[test]
    fn snapshot_artifacts_must_be_regular_files() -> Result<(), String> {
        let fixture = CacheFixture::create()?;
        let (config, weights) = snapshot_artifact_paths(&fixture.cache);
        fs::create_dir_all(config).map_err(|error| error.to_string())?;
        fs::write(weights, b"weights").map_err(|error| error.to_string())?;

        let error = canonical_snapshot_artifacts(&fixture.cache)
            .err()
            .ok_or_else(|| "directory artifact unexpectedly succeeded".to_owned())?;
        assert!(error.to_string().contains("not a regular file"));
        Ok(())
    }

    struct CacheFixture {
        root: PathBuf,
        cache: PathBuf,
    }

    impl CacheFixture {
        fn create() -> Result<Self, String> {
            let root = unique_test_directory();
            let cache = root.join("cache");
            let (config, _) = snapshot_artifact_paths(&cache);
            let snapshot = config
                .parent()
                .ok_or_else(|| "snapshot fixture had no parent".to_owned())?;
            fs::create_dir_all(snapshot).map_err(|error| error.to_string())?;
            Ok(Self { root, cache })
        }

        fn write_regular_artifacts(&self) -> Result<(PathBuf, PathBuf), String> {
            let (config, weights) = snapshot_artifact_paths(&self.cache);
            fs::write(&config, b"{\"ok\":1}").map_err(|error| error.to_string())?;
            fs::write(&weights, b"weights!").map_err(|error| error.to_string())?;
            Ok((config, weights))
        }
    }

    impl Drop for CacheFixture {
        fn drop(&mut self) {
            let _cleanup = fs::remove_dir_all(&self.root);
        }
    }

    fn unique_test_directory() -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let identifier = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "milkdrift-external-model-{}-{timestamp}-{identifier}",
            std::process::id()
        ))
    }
}
