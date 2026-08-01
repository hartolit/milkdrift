//! Bounded command-line parsing without a runtime dependency.

use std::ffi::OsString;
use std::path::PathBuf;

use serde::Serialize;

use crate::error::{BenchmarkError, BenchmarkResult};

const DEFAULT_WARMUP_CYCLES: u32 = 1;
const DEFAULT_SAMPLE_CYCLES: u32 = 3;
const MAXIMUM_WARMUP_CYCLES: u32 = 10;
const MAXIMUM_SAMPLE_CYCLES: u32 = 20;

pub(crate) const HELP: &str = "runtime-benchmarks bounded baseline runner\n\nUsage:\n  baseline [--mode synthetic] [--warmup N] [--cycles N]\n  baseline --mode real-product --cache-dir PATH --allow-network [--warmup N] [--cycles N]\n\nModes:\n  synthetic     Download-free deterministic E0 baseline (default).\n  real-product  Pinned public E1 Hugging Face/Candle product path.\n\nNetwork contract:\n  the production E1 resolver performs immutable Hub metadata resolution, so\n  real-product requires the unmistakable --allow-network opt-in. Repository\n  and revision are fixed; HF_HUB_OFFLINE=1 is rejected as contradictory.\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Mode {
    Synthetic,
    RealProduct,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Configuration {
    pub(crate) mode: Mode,
    pub(crate) warmup_cycles: u32,
    pub(crate) sample_cycles: u32,
    pub(crate) cache_directory: Option<PathBuf>,
    pub(crate) allow_network: bool,
}

pub(crate) enum Action {
    Help,
    Run(Configuration),
}

pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> BenchmarkResult<Action> {
    let mut arguments = arguments.into_iter();
    let _program = arguments.next();
    let mut mode = None;
    let mut warmup_cycles = DEFAULT_WARMUP_CYCLES;
    let mut sample_cycles = DEFAULT_SAMPLE_CYCLES;
    let mut cache_directory = None;
    let mut allow_network = false;

    while let Some(argument) = arguments.next() {
        let flag = argument
            .to_str()
            .ok_or_else(|| BenchmarkError::new("command-line flags must be valid Unicode"))?;
        match flag {
            "-h" | "--help" => return Ok(Action::Help),
            "--mode" => {
                if mode.is_some() {
                    return Err(BenchmarkError::new("--mode may be supplied only once"));
                }
                let value = unicode_value(&mut arguments, "--mode")?;
                mode = Some(match value.as_str() {
                    "synthetic" => Mode::Synthetic,
                    "real-product" => Mode::RealProduct,
                    _ => {
                        return Err(BenchmarkError::new(format!(
                            "unsupported mode {value:?}; expected synthetic or real-product"
                        )));
                    }
                });
            }
            "--warmup" => {
                let value = unicode_value(&mut arguments, "--warmup")?;
                warmup_cycles = bounded_count(&value, "--warmup", MAXIMUM_WARMUP_CYCLES)?;
            }
            "--cycles" => {
                let value = unicode_value(&mut arguments, "--cycles")?;
                sample_cycles = bounded_count(&value, "--cycles", MAXIMUM_SAMPLE_CYCLES)?;
            }
            "--cache-dir" => {
                if cache_directory.is_some() {
                    return Err(BenchmarkError::new("--cache-dir may be supplied only once"));
                }
                cache_directory = Some(PathBuf::from(next_value(&mut arguments, "--cache-dir")?));
            }
            "--allow-network" => {
                if allow_network {
                    return Err(BenchmarkError::new(
                        "--allow-network may be supplied only once",
                    ));
                }
                allow_network = true;
            }
            _ => {
                return Err(BenchmarkError::new(format!(
                    "unknown argument {flag:?}; use --help for the bounded interface"
                )));
            }
        }
    }

    let mode = mode.unwrap_or(Mode::Synthetic);
    match mode {
        Mode::Synthetic if cache_directory.is_some() => {
            return Err(BenchmarkError::new(
                "--cache-dir is valid only with --mode real-product",
            ));
        }
        Mode::Synthetic if allow_network => {
            return Err(BenchmarkError::new(
                "--allow-network is invalid in download-free synthetic mode",
            ));
        }
        Mode::RealProduct if cache_directory.is_none() => {
            return Err(BenchmarkError::new(
                "--mode real-product requires an explicit --cache-dir PATH",
            ));
        }
        Mode::RealProduct if !allow_network => {
            return Err(BenchmarkError::new(
                "--mode real-product requires the unmistakable --allow-network opt-in because public E1 resolution contacts the Hub",
            ));
        }
        Mode::Synthetic | Mode::RealProduct => {}
    }

    Ok(Action::Run(Configuration {
        mode,
        warmup_cycles,
        sample_cycles,
        cache_directory,
        allow_network,
    }))
}

fn next_value(
    arguments: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> BenchmarkResult<OsString> {
    arguments
        .next()
        .ok_or_else(|| BenchmarkError::new(format!("{flag} requires a value")))
}

fn unicode_value(
    arguments: &mut impl Iterator<Item = OsString>,
    flag: &str,
) -> BenchmarkResult<String> {
    let value = next_value(arguments, flag)?;
    value
        .into_string()
        .map_err(|_| BenchmarkError::new(format!("{flag} value must be valid Unicode")))
}

fn bounded_count(value: &str, flag: &str, maximum: u32) -> BenchmarkResult<u32> {
    let parsed = value.parse::<u32>().map_err(|error| {
        BenchmarkError::new(format!("{flag} requires a base-10 integer: {error}"))
    })?;
    if parsed == 0 || parsed > maximum {
        return Err(BenchmarkError::new(format!(
            "{flag} must be between 1 and {maximum}, inclusive"
        )));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::{Action, Mode, parse};
    use std::ffi::OsString;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn defaults_are_bounded_download_free_synthetic() -> Result<(), String> {
        let action = parse(arguments(&["baseline"])).map_err(|error| error.to_string())?;
        let Action::Run(configuration) = action else {
            return Err("default invocation unexpectedly requested help".to_owned());
        };
        assert_eq!(configuration.mode, Mode::Synthetic);
        assert_eq!(configuration.warmup_cycles, 1);
        assert_eq!(configuration.sample_cycles, 3);
        assert!(configuration.cache_directory.is_none());
        assert!(!configuration.allow_network);
        Ok(())
    }

    #[test]
    fn synthetic_counts_are_validated() {
        let zero = parse(arguments(&["baseline", "--cycles", "0"]));
        assert!(zero.is_err());
        let excessive = parse(arguments(&["baseline", "--warmup", "11"]));
        assert!(excessive.is_err());
    }

    #[test]
    fn synthetic_rejects_network_options() {
        let network = parse(arguments(&["baseline", "--allow-network"]));
        assert!(network.is_err());
        let cache = parse(arguments(&[
            "baseline",
            "--cache-dir",
            "target/runtime-benchmarks/cache",
        ]));
        assert!(cache.is_err());
    }

    #[test]
    fn real_product_requires_cache_and_explicit_network_opt_in() -> Result<(), String> {
        let no_cache = parse(arguments(&[
            "baseline",
            "--mode",
            "real-product",
            "--allow-network",
        ]));
        assert!(no_cache.is_err());
        let no_network = parse(arguments(&[
            "baseline",
            "--mode",
            "real-product",
            "--cache-dir",
            "target/runtime-benchmarks/cache",
        ]));
        assert!(no_network.is_err());
        let action = parse(arguments(&[
            "baseline",
            "--mode",
            "real-product",
            "--cache-dir",
            "target/runtime-benchmarks/cache",
            "--allow-network",
        ]))
        .map_err(|error| error.to_string())?;
        let Action::Run(configuration) = action else {
            return Err("real-product invocation unexpectedly requested help".to_owned());
        };
        assert_eq!(configuration.mode, Mode::RealProduct);
        assert!(configuration.allow_network);
        Ok(())
    }
}
