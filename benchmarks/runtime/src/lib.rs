//! Controlled runtime baselines and reusable benchmark support.
//!
//! The normal runner writes exactly one typed JSON document to stdout. Human
//! progress and summaries use stderr so callers may redirect stdout directly.

#![forbid(unsafe_code)]

mod cli;
mod e0;
mod e1;
mod error;
mod fixture;
mod memory;
mod metadata;
mod report;
mod workspace;

use std::ffi::OsString;
use std::io::{self, Write};

use cli::{Action, Configuration, Mode};
use e0::{SyntheticCycles, run_cycles as run_e0_cycles};
use e1::{
    REAL_GENERATION_TOKEN_COUNT, REAL_POST_FIRST_TOKEN_WINDOW, REAL_PRODUCT_REPOSITORY,
    REAL_PRODUCT_REVISION, RealCycles, run_real_cycles, run_startup_cycles,
};
pub use error::BenchmarkError;
use error::BenchmarkResult;
use fixture::{CONFIG_SHA256, VerifiedFixture, WEIGHTS_SHA256};
use report::{
    BaselineReport, ExecutionPolicy, ModelMetadata, RealProductSummary, Results, RunMetadata,
    SCHEMA_VERSION, SamplingMetadata, SyntheticSummary, WorkloadMetadata, metric_summary,
};
use workspace::{canonical_external_cache_directory, repository_root};

const SYNTHETIC_PREFILL_PROMPT_TOKENS: u32 = 4;
const SYNTHETIC_GENERATION_PROMPT_TOKENS: u64 = 2;
const SYNTHETIC_GENERATION_SEED: u64 = 17;
const REAL_GENERATION_SEED: u64 = 39;

/// Verifies the exact committed synthetic fixture and returns its Candle source.
///
/// The normal runner and Criterion target share this setup boundary so both
/// reject size, hash, or parsed-configuration drift before timing.
///
/// # Errors
///
/// Returns an actionable error when either fixture file is missing, changed, or
/// no longer forms a valid public Candle Llama source.
pub fn synthetic_fixture_source() -> Result<candle_backend::CandleLlamaSource, BenchmarkError> {
    VerifiedFixture::verify()?.source()
}

/// Runs the bounded baseline command line, writing JSON only to stdout.
///
/// # Errors
///
/// Returns an actionable error when configuration, environment collection,
/// lifecycle execution, accounting validation, cleanup, or serialization fails.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<(), BenchmarkError> {
    match cli::parse(arguments)? {
        Action::Help => write_help(),
        Action::Run(configuration) => run_configuration(&configuration),
    }
}

fn write_help() -> BenchmarkResult {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(cli::HELP.as_bytes())
        .map_err(|error| BenchmarkError::new(format!("could not write help: {error}")))
}

fn run_configuration(configuration: &Configuration) -> BenchmarkResult {
    let repository_root = repository_root()?;
    let environment = metadata::collect(&repository_root)?;
    let report = match configuration.mode {
        Mode::Synthetic => run_synthetic(configuration, environment)?,
        Mode::RealProduct => run_real_product(configuration, environment, &repository_root)?,
    };
    print_human_summary(&report);
    write_json(&report)
}

fn run_synthetic(
    configuration: &Configuration,
    environment: metadata::EnvironmentMetadata,
) -> BenchmarkResult<BaselineReport> {
    eprintln!(
        "running download-free synthetic runtime baseline: {} warmup + {} sample cycles",
        configuration.warmup_cycles, configuration.sample_cycles
    );
    let fixture = VerifiedFixture::verify()?;
    let SyntheticCycles { warmups, samples } = run_e0_cycles(
        &fixture,
        configuration.warmup_cycles,
        configuration.sample_cycles,
    )?;
    let application_runtime_startup =
        run_startup_cycles(configuration.warmup_cycles, configuration.sample_cycles)?;
    let summary = synthetic_summary(&samples, &application_runtime_startup.samples)?;
    let revision = format!("config-sha256:{CONFIG_SHA256};weights-sha256:{WEIGHTS_SHA256}");
    let metadata = RunMetadata {
        git: environment.git,
        toolchain: environment.toolchain,
        system: environment.system,
        model: ModelMetadata {
            identity: "milkdrift-project-authored-candle-llama-synthetic-fixture".to_owned(),
            repository: None,
            revision,
            architecture: "Llama",
            format: "Safetensors",
            scalar_type: "F32",
            vocabulary_size: fixture::VOCABULARY_SIZE,
            context_capacity: fixture::CONTEXT_CAPACITY,
            fixture: Some(fixture.identity.clone()),
        },
        workload: WorkloadMetadata {
            mode: Mode::Synthetic,
            warmup_cycles: configuration.warmup_cycles,
            sample_cycles: configuration.sample_cycles,
            application_runtime_startup_warmup_cycles: configuration.warmup_cycles,
            application_runtime_startup_sample_cycles: configuration.sample_cycles,
            prompt_token_count: SYNTHETIC_GENERATION_PROMPT_TOKENS,
            checked_prefill_prompt_token_count: Some(SYNTHETIC_PREFILL_PROMPT_TOKENS),
            generation_token_count: e0::GENERATION_TOKEN_COUNT,
            post_first_token_window: e0::POST_FIRST_TOKEN_WINDOW,
            sampling: SamplingMetadata {
                policy: "greedy",
                temperature: 1.0,
                top_k: 1,
                top_p: 1.0,
                min_p: 0.0,
                repetition_penalty: 1.0,
                repetition_window: 0,
                seed: SYNTHETIC_GENERATION_SEED,
                eos_token_count: 0,
                stop_sequence_count: 0,
            },
        },
        execution_policy: ExecutionPolicy {
            network_allowed: false,
            hugging_face_offline: offline_environment_is_one(),
            cache_directory: None,
            operational_timeouts_are_thresholds: false,
            result_files_written_by_runner: false,
        },
    };
    Ok(BaselineReport {
        schema_version: SCHEMA_VERSION,
        metadata,
        results: Results::Synthetic {
            warmups,
            samples,
            application_runtime_startup,
            summary: Box::new(summary),
        },
    })
}

fn run_real_product(
    configuration: &Configuration,
    environment: metadata::EnvironmentMetadata,
    repository_root: &std::path::Path,
) -> BenchmarkResult<BaselineReport> {
    if !configuration.allow_network {
        return Err(BenchmarkError::new(
            "real-product mode refuses network by default and requires explicit --allow-network because public E1 resolution contacts the Hub",
        ));
    }
    if hugging_face_offline()? {
        return Err(BenchmarkError::new(
            "HF_HUB_OFFLINE=1 conflicts with real-product mode: the current public E1 resolver still performs immutable Hub metadata resolution; unset it and pass --allow-network explicitly",
        ));
    }
    let cache = configuration
        .cache_directory
        .as_ref()
        .ok_or_else(|| BenchmarkError::new("real-product cache directory disappeared"))?;
    let cache = canonical_external_cache_directory(cache, repository_root)?;
    eprintln!(
        "running pinned real-product runtime baseline: {}@{}; {} warmup + {} sample cycles; network=explicitly allowed",
        REAL_PRODUCT_REPOSITORY,
        REAL_PRODUCT_REVISION,
        configuration.warmup_cycles,
        configuration.sample_cycles
    );
    let RealCycles { warmups, samples } = run_real_cycles(
        &cache,
        configuration.warmup_cycles,
        configuration.sample_cycles,
    )?;
    let reference = samples
        .first()
        .ok_or_else(|| BenchmarkError::new("real-product normal sample set is empty"))?;
    let summary = real_product_summary(&samples)?;
    let metadata = RunMetadata {
        git: environment.git,
        toolchain: environment.toolchain,
        system: environment.system,
        model: ModelMetadata {
            identity: format!("{REAL_PRODUCT_REPOSITORY}@{REAL_PRODUCT_REVISION}"),
            repository: Some(REAL_PRODUCT_REPOSITORY.to_owned()),
            revision: REAL_PRODUCT_REVISION.to_owned(),
            architecture: "Llama",
            format: "Safetensors",
            scalar_type: "F32",
            vocabulary_size: reference.model.vocabulary_size,
            context_capacity: reference.model.maximum_context_tokens,
            fixture: None,
        },
        workload: WorkloadMetadata {
            mode: Mode::RealProduct,
            warmup_cycles: configuration.warmup_cycles,
            sample_cycles: configuration.sample_cycles,
            application_runtime_startup_warmup_cycles: configuration.warmup_cycles,
            application_runtime_startup_sample_cycles: configuration.sample_cycles,
            prompt_token_count: reference.generation.terminal_usage.prompt_tokens,
            checked_prefill_prompt_token_count: None,
            generation_token_count: REAL_GENERATION_TOKEN_COUNT,
            post_first_token_window: REAL_POST_FIRST_TOKEN_WINDOW,
            sampling: SamplingMetadata {
                policy: "top-k-1 deterministic product sampling",
                temperature: 1.0,
                top_k: 1,
                top_p: 1.0,
                min_p: 0.0,
                repetition_penalty: 1.0,
                repetition_window: 0,
                seed: REAL_GENERATION_SEED,
                eos_token_count: 0,
                stop_sequence_count: 0,
            },
        },
        execution_policy: ExecutionPolicy {
            network_allowed: true,
            hugging_face_offline: false,
            cache_directory: Some(path_string(&cache)),
            operational_timeouts_are_thresholds: false,
            result_files_written_by_runner: false,
        },
    };
    Ok(BaselineReport {
        schema_version: SCHEMA_VERSION,
        metadata,
        results: Results::RealProduct {
            warmups,
            samples,
            summary: Box::new(summary),
        },
    })
}

fn hugging_face_offline() -> BenchmarkResult<bool> {
    match std::env::var("HF_HUB_OFFLINE") {
        Ok(value) => Ok(value == "1"),
        Err(std::env::VarError::NotPresent) => Ok(false),
        Err(std::env::VarError::NotUnicode(_)) => Err(BenchmarkError::new(
            "HF_HUB_OFFLINE is not valid Unicode; real-product mode requires it to be unset or a Unicode value other than 1",
        )),
    }
}

fn synthetic_summary(
    samples: &[report::SyntheticCycle],
    startup_samples: &[report::ApplicationStartupCycle],
) -> BenchmarkResult<SyntheticSummary> {
    Ok(SyntheticSummary {
        model_load_ns: metric_summary(samples.iter().map(|sample| sample.model_load_ns))?,
        checked_prefill_ns: metric_summary(
            samples
                .iter()
                .map(|sample| sample.checked_prefill.duration_ns),
        )?,
        first_token_ns: metric_summary(samples.iter().map(|sample| sample.first_token_ns))?,
        post_first_token_proxy_ns: metric_summary(
            samples
                .iter()
                .map(|sample| sample.post_first_token_proxy.duration_ns),
        )?,
        backpressure_recovery_ns: metric_summary(
            samples
                .iter()
                .map(|sample| sample.backpressure.recovery_to_next_token_ns),
        )?,
        cancellation_terminal_ns: metric_summary(
            samples.iter().map(|sample| sample.cancellation.terminal_ns),
        )?,
        model_unload_ns: metric_summary(samples.iter().map(|sample| sample.model_unload_ns))?,
        e0_shutdown_total_ns: metric_summary(
            samples.iter().map(|sample| sample.shutdown.total_ns),
        )?,
        application_start_ns: metric_summary(startup_samples.iter().map(|sample| sample.start_ns))?,
        application_shutdown_ns: metric_summary(
            startup_samples.iter().map(|sample| sample.shutdown_ns),
        )?,
    })
}

fn real_product_summary(
    samples: &[report::RealProductCycle],
) -> BenchmarkResult<RealProductSummary> {
    Ok(RealProductSummary {
        application_start_ns: metric_summary(
            samples.iter().map(|sample| sample.application_start_ns),
        )?,
        resolution_or_download_ns: metric_summary(
            samples
                .iter()
                .map(|sample| sample.resolution_or_download_ns),
        )?,
        model_load_ns: metric_summary(samples.iter().map(|sample| sample.model_load_ns))?,
        first_decoded_output_ns: metric_summary(
            samples
                .iter()
                .map(|sample| sample.first_decoded_output.duration_ns),
        )?,
        model_unload_ns: metric_summary(samples.iter().map(|sample| sample.model_unload_ns))?,
        application_shutdown_ns: metric_summary(
            samples.iter().map(|sample| sample.application_shutdown_ns),
        )?,
    })
}

fn print_human_summary(report: &BaselineReport) {
    match &report.results {
        Results::Synthetic {
            samples, summary, ..
        } => {
            eprintln!(
                "synthetic baseline complete: {} samples; median load={} ns, first-token pull={} ns, post-first proxy={} ns, cancellation Terminal={} ns, unload={} ns",
                samples.len(),
                summary.model_load_ns.median,
                summary.first_token_ns.median,
                summary.post_first_token_proxy_ns.median,
                summary.cancellation_terminal_ns.median,
                summary.model_unload_ns.median
            );
        }
        Results::RealProduct {
            samples, summary, ..
        } => {
            eprintln!(
                "real-product baseline complete: {} samples; median resolve/download={} ns, load={} ns, first decoded output={} ns, unload={} ns",
                samples.len(),
                summary.resolution_or_download_ns.median,
                summary.model_load_ns.median,
                summary.first_decoded_output_ns.median,
                summary.model_unload_ns.median
            );
        }
    }
}

fn write_json(report: &BaselineReport) -> BenchmarkResult {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer(&mut lock, report).map_err(|error| {
        BenchmarkError::new(format!("could not serialize JSON report: {error}"))
    })?;
    lock.write_all(b"\n")
        .map_err(|error| BenchmarkError::new(format!("could not finish JSON report: {error}")))
}

fn path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

fn offline_environment_is_one() -> bool {
    std::env::var("HF_HUB_OFFLINE").is_ok_and(|value| value == "1")
}
