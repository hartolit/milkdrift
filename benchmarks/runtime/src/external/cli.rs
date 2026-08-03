//! Explicit opt-in and cache-boundary policy for the external product baseline.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{BenchmarkError, BenchmarkResult};

pub(crate) const HELP: &str = "runtime-benchmarks external baseline\n\nUsage:\n  external-baseline --allow-network --cache-dir PATH --device cpu|cuda:0\n\nThe model repository and immutable revision are built in and cannot be\noverridden. PATH must already exist and resolve beneath the repository-root\ntarget/ directory or outside the repository. The cuda:0 device requires\nruntime-benchmarks to be built with --features cuda. The command may contact\nHugging Face only for the pinned model and writes one JSON report to stdout.\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RequestedDevice {
    Cpu,
    Cuda0,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Configuration {
    pub(crate) cache_directory: PathBuf,
    pub(crate) device: RequestedDevice,
}

pub(crate) enum Action {
    Help,
    Run(Configuration),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CacheLocation {
    RepositoryTarget,
    External,
}

impl CacheLocation {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::RepositoryTarget => "repository_root_target",
            Self::External => "outside_repository",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedConfiguration {
    pub(crate) cache_directory: PathBuf,
    pub(crate) cache_location: CacheLocation,
    pub(crate) device: RequestedDevice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CacheState {
    Empty,
    Populated,
}

impl CacheState {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Populated => "populated",
        }
    }
}

pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> BenchmarkResult<Action> {
    let mut arguments = arguments.into_iter();
    let _program = arguments.next();
    let mut allow_network = false;
    let mut cache_directory = None;
    let mut device = None;

    while let Some(argument) = arguments.next() {
        let flag = argument
            .to_str()
            .ok_or_else(|| BenchmarkError::new("command-line flags must be valid Unicode"))?;
        match flag {
            "-h" | "--help" => return Ok(Action::Help),
            "--allow-network" => {
                if allow_network {
                    return Err(BenchmarkError::new(
                        "--allow-network may be supplied only once",
                    ));
                }
                allow_network = true;
            }
            "--cache-dir" => {
                if cache_directory.is_some() {
                    return Err(BenchmarkError::new("--cache-dir may be supplied only once"));
                }
                cache_directory = Some(next_value(&mut arguments, "--cache-dir")?.into());
            }
            "--device" => {
                if device.is_some() {
                    return Err(BenchmarkError::new("--device may be supplied only once"));
                }
                device = Some(parse_device(&next_value(&mut arguments, "--device")?)?);
            }
            "--repository" | "--revision" | "--model" | "--repo" => {
                return Err(BenchmarkError::new(format!(
                    "{flag} is not supported; the external model and immutable revision are built in"
                )));
            }
            _ => {
                return Err(BenchmarkError::new(format!(
                    "unknown argument {flag:?}; use --help for the bounded interface"
                )));
            }
        }
    }

    if !allow_network {
        return Err(BenchmarkError::new(
            "--allow-network is required before external runtime startup",
        ));
    }
    let cache_directory =
        cache_directory.ok_or_else(|| BenchmarkError::new("--cache-dir PATH is required"))?;
    let device = device.ok_or_else(|| BenchmarkError::new("--device cpu|cuda:0 is required"))?;
    Ok(Action::Run(Configuration {
        cache_directory,
        device,
    }))
}

pub(crate) fn validate_cache_directory(
    configuration: &Configuration,
    repository_root: &Path,
) -> BenchmarkResult<ValidatedConfiguration> {
    let repository_root = repository_root.canonicalize().map_err(|error| {
        BenchmarkError::new(format!(
            "could not canonicalize repository root {}: {error}",
            repository_root.display()
        ))
    })?;
    let cache_directory = configuration
        .cache_directory
        .canonicalize()
        .map_err(|error| {
            BenchmarkError::new(format!(
                "--cache-dir {} must already exist and be canonicalizable: {error}",
                configuration.cache_directory.display()
            ))
        })?;
    if !cache_directory.is_dir() {
        return Err(BenchmarkError::new(format!(
            "--cache-dir {} is not a directory",
            cache_directory.display()
        )));
    }

    let cache_location = if cache_directory.starts_with(&repository_root) {
        let target_directory = repository_root.join("target");
        let target_directory = target_directory.canonicalize().map_err(|error| {
            BenchmarkError::new(format!(
                "could not canonicalize repository target directory {}: {error}",
                target_directory.display()
            ))
        })?;
        if cache_directory.starts_with(&target_directory) && cache_directory != target_directory {
            CacheLocation::RepositoryTarget
        } else {
            return Err(BenchmarkError::new(format!(
                "--cache-dir {} resolves inside the source tree but not beneath the repository-root target/ directory",
                cache_directory.display()
            )));
        }
    } else {
        CacheLocation::External
    };

    Ok(ValidatedConfiguration {
        cache_directory,
        cache_location,
        device: configuration.device,
    })
}

pub(crate) fn inspect_cache_state(cache_directory: &Path) -> BenchmarkResult<CacheState> {
    let mut entries = fs::read_dir(cache_directory).map_err(|error| {
        BenchmarkError::new(format!(
            "could not inspect explicit cache directory {}: {error}",
            cache_directory.display()
        ))
    })?;
    match entries.next() {
        None => Ok(CacheState::Empty),
        Some(Ok(_)) => Ok(CacheState::Populated),
        Some(Err(error)) => Err(BenchmarkError::new(format!(
            "could not inspect an entry in explicit cache directory {}: {error}",
            cache_directory.display()
        ))),
    }
}

fn parse_device(value: &std::ffi::OsStr) -> BenchmarkResult<RequestedDevice> {
    let value = value
        .to_str()
        .ok_or_else(|| BenchmarkError::new("--device value must be valid Unicode"))?;
    match value {
        "cpu" => Ok(RequestedDevice::Cpu),
        "cuda:0" => {
            if cfg!(feature = "cuda") {
                Ok(RequestedDevice::Cuda0)
            } else {
                Err(BenchmarkError::new(
                    "--device cuda:0 requires the runtime-benchmarks `cuda` feature; rebuild with `--features cuda`",
                ))
            }
        }
        _ => Err(BenchmarkError::new(format!(
            "unsupported --device value {value:?}; expected exactly cpu or cuda:0"
        ))),
    }
}

fn next_value(
    arguments: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> BenchmarkResult<OsString> {
    arguments
        .next()
        .ok_or_else(|| BenchmarkError::new(format!("{flag} requires a value")))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        Action, CacheLocation, Configuration, RequestedDevice, inspect_cache_state, parse,
        validate_cache_directory,
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn execution_arguments(device: &str) -> Vec<OsString> {
        arguments(&[
            "external-baseline",
            "--allow-network",
            "--cache-dir",
            "target/cache",
            "--device",
            device,
        ])
    }

    #[test]
    fn external_cli_help_is_side_effect_free() -> Result<(), String> {
        let action = parse(arguments(&[
            "external-baseline",
            "--help",
            "--cache-dir",
            "path-that-need-not-exist",
            "--device",
            "cuda:0",
        ]))
        .map_err(|error| error.to_string())?;
        let Action::Help = action else {
            return Err("--help unexpectedly requested execution".to_owned());
        };
        Ok(())
    }

    #[test]
    fn external_cli_requires_explicit_network_authorization() -> Result<(), String> {
        let Err(error) = parse(arguments(&[
            "external-baseline",
            "--cache-dir",
            "target/cache",
            "--device",
            "cpu",
        ])) else {
            return Err("missing opt-in unexpectedly succeeded".to_owned());
        };
        assert!(error.to_string().contains("--allow-network"));
        Ok(())
    }

    #[test]
    fn external_cli_requires_an_explicit_cache_path() -> Result<(), String> {
        let Err(error) = parse(arguments(&[
            "external-baseline",
            "--allow-network",
            "--device",
            "cpu",
        ])) else {
            return Err("missing cache unexpectedly succeeded".to_owned());
        };
        assert!(error.to_string().contains("--cache-dir"));
        Ok(())
    }

    #[test]
    fn external_cli_requires_an_explicit_device() -> Result<(), String> {
        let Err(error) = parse(arguments(&[
            "external-baseline",
            "--allow-network",
            "--cache-dir",
            "target/cache",
        ])) else {
            return Err("missing device unexpectedly succeeded".to_owned());
        };
        assert!(error.to_string().contains("--device cpu|cuda:0"));
        Ok(())
    }

    #[test]
    fn external_cli_rejects_a_device_without_a_value() -> Result<(), String> {
        let Err(error) = parse(arguments(&[
            "external-baseline",
            "--allow-network",
            "--cache-dir",
            "target/cache",
            "--device",
        ])) else {
            return Err("valueless device unexpectedly succeeded".to_owned());
        };
        assert!(error.to_string().contains("--device requires a value"));
        Ok(())
    }

    #[test]
    fn external_cli_rejects_duplicate_devices() -> Result<(), String> {
        let Err(error) = parse(arguments(&[
            "external-baseline",
            "--allow-network",
            "--cache-dir",
            "target/cache",
            "--device",
            "cpu",
            "--device",
            "cpu",
        ])) else {
            return Err("duplicate device unexpectedly succeeded".to_owned());
        };
        assert!(
            error
                .to_string()
                .contains("--device may be supplied only once")
        );
        Ok(())
    }

    #[test]
    fn external_cli_rejects_near_miss_device_values() -> Result<(), String> {
        for device in ["CPU", "cpu:0", "cuda", "cuda:00", "cuda:1", "gpu"] {
            let Err(error) = parse(execution_arguments(device)) else {
                return Err(format!(
                    "near-miss device {device:?} unexpectedly succeeded"
                ));
            };
            assert!(
                error.to_string().contains("expected exactly cpu or cuda:0"),
                "{device:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn external_cli_rejects_unknown_and_identity_substitution_flags() {
        for values in [
            &[
                "external-baseline",
                "--allow-network",
                "--cache-dir",
                "target/cache",
                "--device",
                "cpu",
                "--unknown",
            ][..],
            &[
                "external-baseline",
                "--allow-network",
                "--cache-dir",
                "target/cache",
                "--device",
                "cpu",
                "--repository",
                "other/model",
            ][..],
            &[
                "external-baseline",
                "--allow-network",
                "--cache-dir",
                "target/cache",
                "--device",
                "cpu",
                "--revision",
                "main",
            ][..],
            &[
                "external-baseline",
                "--allow-network",
                "--cache-dir",
                "target/cache",
                "--device",
                "cpu",
                "--model",
                "other/model",
            ][..],
            &[
                "external-baseline",
                "--allow-network",
                "--cache-dir",
                "target/cache",
                "--device",
                "cpu",
                "--repo",
                "other/model",
            ][..],
        ] {
            assert!(parse(arguments(values)).is_err(), "{values:?}");
        }
    }

    #[test]
    fn external_cli_always_accepts_the_exact_cpu_device() -> Result<(), String> {
        let action = parse(execution_arguments("cpu")).map_err(|error| error.to_string())?;
        let Action::Run(configuration) = action else {
            return Err("execution arguments unexpectedly requested help".to_owned());
        };
        assert_eq!(configuration.cache_directory, PathBuf::from("target/cache"));
        assert_eq!(configuration.device, RequestedDevice::Cpu);
        Ok(())
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn external_cli_accepts_cuda_zero_when_the_feature_is_enabled() -> Result<(), String> {
        let action = parse(execution_arguments("cuda:0")).map_err(|error| error.to_string())?;
        let Action::Run(configuration) = action else {
            return Err("CUDA execution arguments unexpectedly requested help".to_owned());
        };
        assert_eq!(configuration.device, RequestedDevice::Cuda0);
        Ok(())
    }

    #[cfg(not(feature = "cuda"))]
    #[test]
    fn external_cli_rejects_cuda_zero_when_the_feature_is_disabled() -> Result<(), String> {
        let Err(error) = parse(execution_arguments("cuda:0")) else {
            return Err("CUDA device unexpectedly succeeded without the cuda feature".to_owned());
        };
        let error = error.to_string();
        assert!(
            error.contains("requires the runtime-benchmarks `cuda` feature"),
            "{error}"
        );
        assert!(error.contains("--features cuda"), "{error}");
        Ok(())
    }

    #[test]
    fn cache_policy_rejects_source_paths_and_accepts_target_or_external_paths() -> Result<(), String>
    {
        let fixture = DirectoryFixture::create()?;
        let rejected = validate_cache_directory(
            &Configuration {
                cache_directory: fixture.source_cache.clone(),
                device: RequestedDevice::Cpu,
            },
            &fixture.repository,
        );
        assert!(rejected.is_err());

        let target = validate_cache_directory(
            &Configuration {
                cache_directory: fixture.target_cache.clone(),
                device: RequestedDevice::Cpu,
            },
            &fixture.repository,
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(target.cache_location, CacheLocation::RepositoryTarget);
        assert_eq!(target.device, RequestedDevice::Cpu);

        fs::remove_dir_all(fixture.repository.join("target")).map_err(|error| error.to_string())?;
        let external = validate_cache_directory(
            &Configuration {
                cache_directory: fixture.external_cache.clone(),
                device: RequestedDevice::Cpu,
            },
            &fixture.repository,
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(external.cache_location, CacheLocation::External);
        assert_eq!(external.device, RequestedDevice::Cpu);
        Ok(())
    }

    #[test]
    fn cache_state_distinguishes_empty_and_populated_directories() -> Result<(), String> {
        let fixture = DirectoryFixture::create()?;
        assert_eq!(
            inspect_cache_state(&fixture.target_cache).map_err(|error| error.to_string())?,
            super::CacheState::Empty
        );
        fs::write(fixture.target_cache.join("marker"), b"present")
            .map_err(|error| error.to_string())?;
        assert_eq!(
            inspect_cache_state(&fixture.target_cache).map_err(|error| error.to_string())?,
            super::CacheState::Populated
        );
        Ok(())
    }

    struct DirectoryFixture {
        root: PathBuf,
        repository: PathBuf,
        source_cache: PathBuf,
        target_cache: PathBuf,
        external_cache: PathBuf,
    }

    impl DirectoryFixture {
        fn create() -> Result<Self, String> {
            let root = unique_test_directory();
            let repository = root.join("repository");
            let source_cache = repository.join("benchmarks/cache");
            let target_cache = repository.join("target/cache");
            let external_cache = root.join("external-cache");
            for directory in [
                &source_cache,
                &target_cache,
                &external_cache,
                &repository.join("target"),
            ] {
                fs::create_dir_all(directory).map_err(|error| error.to_string())?;
            }
            Ok(Self {
                root,
                repository,
                source_cache,
                target_cache,
                external_cache,
            })
        }
    }

    impl Drop for DirectoryFixture {
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
            "milkdrift-external-cli-{}-{timestamp}-{identifier}",
            std::process::id()
        ))
    }
}
