//! Bounded command-line parsing without a runtime dependency.

use std::ffi::OsString;

use crate::error::{BenchmarkError, BenchmarkResult};

const DEFAULT_WARMUP_CYCLES: u32 = 1;
const DEFAULT_SAMPLE_CYCLES: u32 = 3;
const MAXIMUM_WARMUP_CYCLES: u32 = 10;
const MAXIMUM_SAMPLE_CYCLES: u32 = 20;

pub(crate) const HELP: &str = "runtime-benchmarks download-free baseline runner\n\nUsage:\n  baseline [--mode synthetic] [--warmup N] [--cycles N]\n\nThe only supported mode exercises the deterministic hosted-E0 fixture and\nseparate download-free ApplicationRuntime startup/shutdown cycles. It performs\nno network access, model resolution, or product-model execution.\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Configuration {
    pub(crate) warmup_cycles: u32,
    pub(crate) sample_cycles: u32,
}

pub(crate) enum Action {
    Help,
    Run(Configuration),
}

pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> BenchmarkResult<Action> {
    let mut arguments = arguments.into_iter();
    let _program = arguments.next();
    let mut mode_supplied = false;
    let mut warmup_cycles = DEFAULT_WARMUP_CYCLES;
    let mut sample_cycles = DEFAULT_SAMPLE_CYCLES;

    while let Some(argument) = arguments.next() {
        let flag = argument
            .to_str()
            .ok_or_else(|| BenchmarkError::new("command-line flags must be valid Unicode"))?;
        match flag {
            "-h" | "--help" => return Ok(Action::Help),
            "--mode" => {
                if mode_supplied {
                    return Err(BenchmarkError::new("--mode may be supplied only once"));
                }
                let value = unicode_value(&mut arguments, "--mode")?;
                if value != "synthetic" {
                    return Err(BenchmarkError::new(format!(
                        "unsupported mode {value:?}; only synthetic is available"
                    )));
                }
                mode_supplied = true;
            }
            "--warmup" => {
                let value = unicode_value(&mut arguments, "--warmup")?;
                warmup_cycles = bounded_count(&value, "--warmup", MAXIMUM_WARMUP_CYCLES)?;
            }
            "--cycles" => {
                let value = unicode_value(&mut arguments, "--cycles")?;
                sample_cycles = bounded_count(&value, "--cycles", MAXIMUM_SAMPLE_CYCLES)?;
            }
            _ => {
                return Err(BenchmarkError::new(format!(
                    "unknown argument {flag:?}; use --help for the bounded interface"
                )));
            }
        }
    }

    Ok(Action::Run(Configuration {
        warmup_cycles,
        sample_cycles,
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
    next_value(arguments, flag)?
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
    use super::{Action, parse};
    use std::ffi::OsString;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn defaults_are_bounded_and_download_free() -> Result<(), String> {
        let action = parse(arguments(&["baseline"])).map_err(|error| error.to_string())?;
        let Action::Run(configuration) = action else {
            return Err("default invocation unexpectedly requested help".to_owned());
        };
        assert_eq!(configuration.warmup_cycles, 1);
        assert_eq!(configuration.sample_cycles, 3);
        Ok(())
    }

    #[test]
    fn explicit_synthetic_mode_is_accepted() -> Result<(), String> {
        let action = parse(arguments(&["baseline", "--mode", "synthetic"]))
            .map_err(|error| error.to_string())?;
        assert!(matches!(action, Action::Run(_)));
        Ok(())
    }

    #[test]
    fn cycle_counts_are_bounded() {
        assert!(parse(arguments(&["baseline", "--cycles", "0"])).is_err());
        assert!(parse(arguments(&["baseline", "--warmup", "11"])).is_err());
        assert!(parse(arguments(&["baseline", "--cycles", "not-a-number"])).is_err());
    }

    #[test]
    fn removed_product_options_are_not_accepted() {
        for arguments in [
            &["baseline", "--mode", "real-product"][..],
            &["baseline", "--cache-dir", "target/cache"][..],
            &["baseline", "--allow-network"][..],
        ] {
            assert!(parse(self::arguments(arguments)).is_err());
        }
    }
}
