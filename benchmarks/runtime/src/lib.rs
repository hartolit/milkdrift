//! Controlled runtime baselines and shared Criterion support.
//!
//! Each normal runner writes exactly one typed JSON document to stdout. Human
//! progress and summaries use stderr so callers may redirect stdout directly.
//! The synthetic runner is download-free; the separate external runner requires
//! explicit network authorization, an explicit cache path, and an exact CPU or
//! feature-gated CUDA device selection.

#![forbid(unsafe_code)]
#![expect(
    clippy::large_types_passed_by_value,
    reason = "the observer preserves E0's bounded Copy outcome evidence without changing production representation"
)]

mod cli;
/// Benchmark-only hosted-E0 support shared with the separate Criterion crate.
#[doc(hidden)]
pub mod e0;
mod e1;
mod error;
mod evidence;
mod external;
mod fixture;
mod load_observation;
mod memory;
mod metadata;
mod report;
mod support;
mod workspace;

use std::ffi::OsString;
use std::io::{self, Write};

use cli::{Action, Configuration};
use e0::run_cycles as run_e0_cycles;
use e1::run_lifecycle_cycles;
pub use error::{BenchmarkError, BenchmarkResult};
use fixture::VerifiedFixture;
use report::{
    BaselineReport, BaselineResults, RunMetadata, SCHEMA_VERSION, SyntheticFixtureMetadata,
    WorkloadMetadata,
};
use serde::Serialize;
use workspace::repository_root;

/// Runs the bounded baseline command line, writing JSON only to stdout.
///
/// # Errors
///
/// Returns an actionable error when configuration, environment collection,
/// lifecycle execution, accounting validation, cleanup, or serialization fails.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<(), BenchmarkError> {
    match cli::parse(arguments)? {
        Action::Help => write_help(cli::HELP),
        Action::Run(configuration) => run_configuration(configuration),
    }
}

/// Runs the explicit external CPU/CUDA baseline, writing JSON only to stdout.
///
/// # Errors
///
/// Returns an actionable error when opt-in/cache/device policy, environment
/// collection, exact model identity, lifecycle execution, cleanup, or
/// serialization fails.
pub fn run_external_baseline(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<(), BenchmarkError> {
    match external::parse(arguments)? {
        external::Action::Help => write_help(external::HELP),
        external::Action::Run(configuration) => {
            let report = external::run_configuration(&configuration)?;
            external::print_human_summary(&report);
            write_json(&report)
        }
    }
}

fn write_help(help: &str) -> BenchmarkResult {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(help.as_bytes())
        .map_err(|error| BenchmarkError::new(format!("could not write help: {error}")))
}

fn run_configuration(configuration: Configuration) -> BenchmarkResult {
    let repository_root = repository_root()?;
    let environment = metadata::collect(&repository_root)?;
    eprintln!(
        "running download-free runtime baseline: {} warmup + {} sample cycles",
        configuration.warmup_cycles, configuration.sample_cycles
    );
    let fixture = VerifiedFixture::verify()?;
    let synthetic_e0 = run_e0_cycles(
        &fixture,
        configuration.warmup_cycles,
        configuration.sample_cycles,
    )?;
    let application_lifecycle =
        run_lifecycle_cycles(configuration.warmup_cycles, configuration.sample_cycles)?;
    let report = BaselineReport {
        schema_version: SCHEMA_VERSION,
        metadata: RunMetadata {
            git: environment.git,
            toolchain: environment.toolchain,
            system: environment.system,
            fixture: SyntheticFixtureMetadata {
                identity: fixture.identity,
                backend: "Candle",
                architecture: "Llama",
                format: "Safetensors",
                vocabulary_size: fixture::VOCABULARY_SIZE,
                context_capacity: fixture::CONTEXT_CAPACITY,
            },
            workload: WorkloadMetadata {
                warmup_cycles: configuration.warmup_cycles,
                sample_cycles: configuration.sample_cycles,
                checked_prefill_prompt_tokens: e0::CHECKED_PREFILL_TOKEN_COUNT,
                generation_prompt_tokens: e0::GENERATION_PROMPT_TOKEN_COUNT,
                first_token_generation_limit: e0::FIRST_TOKEN_GENERATION_LIMIT,
                post_first_token_window: e0::POST_FIRST_TOKEN_WINDOW,
                backpressure_generation_limit: e0::BACKPRESSURE_GENERATION_LIMIT,
                backpressure_hold_milliseconds: e0::BACKPRESSURE_HOLD_MILLISECONDS,
                cancellation_generation_limit: e0::CANCELLATION_GENERATION_LIMIT,
                cancellation_hold_milliseconds: e0::CANCELLATION_HOLD_MILLISECONDS,
                sampling_strategy: "greedy",
            },
        },
        results: BaselineResults {
            synthetic_e0,
            application_lifecycle,
        },
    };
    print_human_summary(&report)?;
    write_json(&report)
}

fn print_human_summary(report: &BaselineReport) -> BenchmarkResult {
    let e0 = &report.results.synthetic_e0.samples;
    let e1 = &report.results.application_lifecycle.samples;
    eprintln!(
        "runtime baseline complete: {} E0 and {} E1 lifecycle samples; median load={} ns, first-token pull={} ns, post-first proxy={} ns, cancellation Terminal={} ns, unload={} ns, application start={} ns, application shutdown={} ns",
        e0.len(),
        e1.len(),
        metric_median(e0.iter().map(|sample| sample.model_load_ns))?,
        metric_median(e0.iter().map(|sample| sample.first_token_ns))?,
        metric_median(e0.iter().map(|sample| sample.post_first_token_proxy_ns))?,
        metric_median(e0.iter().map(|sample| sample.cancellation.terminal_ns))?,
        metric_median(e0.iter().map(|sample| sample.model_unload_ns))?,
        metric_median(e1.iter().map(|sample| sample.start_ns))?,
        metric_median(e1.iter().map(|sample| sample.shutdown_ns))?,
    );
    Ok(())
}

fn metric_median(values: impl IntoIterator<Item = u64>) -> BenchmarkResult<u64> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        return Err(BenchmarkError::new(
            "cannot summarize an empty normal-sample set",
        ));
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        let lower = values
            .get(middle.saturating_sub(1))
            .copied()
            .ok_or_else(|| BenchmarkError::new("summary lower median disappeared"))?;
        let upper = values
            .get(middle)
            .copied()
            .ok_or_else(|| BenchmarkError::new("summary upper median disappeared"))?;
        Ok(lower + (upper - lower) / 2)
    } else {
        values
            .get(middle)
            .copied()
            .ok_or_else(|| BenchmarkError::new("summary median disappeared"))
    }
}

fn write_json(report: &(impl Serialize + ?Sized)) -> BenchmarkResult {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer(&mut lock, report).map_err(|error| {
        BenchmarkError::new(format!("could not serialize JSON report: {error}"))
    })?;
    lock.write_all(b"\n")
        .map_err(|error| BenchmarkError::new(format!("could not finish JSON report: {error}")))
}

#[cfg(test)]
mod tests {
    use super::metric_median;

    #[test]
    fn metric_median_uses_an_exact_overflow_safe_even_average() -> Result<(), String> {
        assert_eq!(
            metric_median([u64::MAX - 1, u64::MAX]).map_err(|error| error.to_string())?,
            u64::MAX - 1
        );
        assert_eq!(
            metric_median([0, u64::MAX]).map_err(|error| error.to_string())?,
            u64::MAX / 2
        );
        Ok(())
    }
}
