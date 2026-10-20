//! Derive protected operator paths from the same concrete inputs used to compose this daemon.
use super::{ConfigError, DaemonConfig, SecretSourceConfig};
use milkdrift_managed_linux::{LinuxRecipe, ModelService, ProtectedServiceRecipe};
use std::{collections::BTreeSet, io::Read, path::PathBuf};

pub(super) fn configured(config: &DaemonConfig) -> Result<Vec<PathBuf>, ConfigError> {
    let mut paths = BTreeSet::new();
    for source in config.secret_sources.values() {
        if let SecretSourceConfig::File { path } = source {
            paths.insert(path.clone());
        }
    }
    paths.extend(config.adapters.process_profiles.iter().cloned());
    paths.extend(
        config
            .adapters
            .model_profiles
            .iter()
            .map(|model| model.profile.clone()),
    );
    if let Some(managed) = &config.adapters.managed_linux {
        paths.extend([
            managed.state_root.clone(),
            managed.quadlet_directory.clone(),
            managed.systemd_directory.clone(),
        ]);
        for path in &managed.recipes {
            paths.insert(path.clone());
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .and_then(|file| file.take(65_537).read_to_end(&mut bytes))
                .map_err(|error| ConfigError::Read(error.kind().to_string()))?;
            if let Ok(recipe) = ProtectedServiceRecipe::from_json(&bytes) {
                for input in [
                    recipe.token_file,
                    recipe.clock_file,
                    recipe.verifier_executable,
                ] {
                    paths.insert(
                        input
                            .canonicalize()
                            .map_err(|error| ConfigError::Read(error.kind().to_string()))?,
                    );
                }
            } else if let ModelService::Owned { model, .. } = LinuxRecipe::from_json(&bytes)
                .map_err(|error| ConfigError::Invalid(error.to_string()))?
                .model_service
            {
                paths.insert(
                    model
                        .canonicalize()
                        .map_err(|error| ConfigError::Read(error.kind().to_string()))?,
                );
            }
        }
    }
    Ok(paths.into_iter().collect())
}
