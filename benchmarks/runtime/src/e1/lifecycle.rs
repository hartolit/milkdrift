//! Repeated download-free `ApplicationRuntime` start and bounded shutdown cycles.

use std::path::Path;
use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationRuntime, ApplicationRuntimeConfiguration,
};

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::memory::process_memory;
use crate::report::{ApplicationLifecycleCycle, CycleSet, duration_ns};
use crate::workspace::TemporaryWorkspace;

use super::shutdown_for_cleanup;

const POLL_INTERVAL: Duration = Duration::from_millis(1);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn run_lifecycle_cycles(
    warmup_cycles: u32,
    sample_cycles: u32,
) -> BenchmarkResult<CycleSet<ApplicationLifecycleCycle>> {
    let mut workspace = TemporaryWorkspace::create("application-lifecycle")?;
    let result = run_in_workspace(&workspace, warmup_cycles, sample_cycles);
    let cleanup = workspace.cleanup();
    match result {
        Ok(cycles) => {
            cleanup?;
            Ok(cycles)
        }
        Err(error) => Err(error.with_cleanup(cleanup)),
    }
}

fn run_in_workspace(
    workspace: &TemporaryWorkspace,
    warmup_cycles: u32,
    sample_cycles: u32,
) -> BenchmarkResult<CycleSet<ApplicationLifecycleCycle>> {
    let cache = workspace.empty_cache_directory()?;
    let mut warmups = Vec::new();
    let mut samples = Vec::new();
    warmups
        .try_reserve_exact(usize_from_u32(warmup_cycles)?)
        .map_err(|error| {
            BenchmarkError::new(format!("lifecycle warmup allocation failed: {error}"))
        })?;
    samples
        .try_reserve_exact(usize_from_u32(sample_cycles)?)
        .map_err(|error| {
            BenchmarkError::new(format!("lifecycle sample allocation failed: {error}"))
        })?;
    for ordinal in 1..=warmup_cycles {
        warmups.push(run_cycle(
            workspace.database_path("warmup", ordinal),
            &cache,
        )?);
    }
    for ordinal in 1..=sample_cycles {
        samples.push(run_cycle(
            workspace.database_path("sample", ordinal),
            &cache,
        )?);
    }
    Ok(CycleSet { warmups, samples })
}

fn run_cycle(
    database_path: std::path::PathBuf,
    cache_directory: &Path,
) -> BenchmarkResult<ApplicationLifecycleCycle> {
    let rss_before_start = process_memory()?;
    let configuration = application_configuration(database_path, cache_directory);
    let started = Instant::now();
    let mut runtime = ApplicationRuntime::start(configuration).map_err(|error| {
        BenchmarkError::new(format!(
            "download-free ApplicationRuntime start failed: {error}"
        ))
    })?;
    let start_elapsed = started.elapsed();
    let result = finish_cycle(&mut runtime, start_elapsed, rss_before_start);
    match result {
        Ok(cycle) => Ok(cycle),
        Err(error) => match shutdown_for_cleanup(&mut runtime) {
            Ok(()) => Err(error),
            Err(cleanup_error) => {
                let cleanup_error = BenchmarkError::new(format!(
                    "{cleanup_error}; failed ApplicationRuntime owner retained until process exit"
                ));
                let combined = error.with_cleanup(Err(cleanup_error));
                retain_failed_runtime_until_process_exit(runtime);
                Err(combined)
            }
        },
    }
}

fn retain_failed_runtime_until_process_exit(runtime: ApplicationRuntime) {
    std::mem::forget(runtime);
}

fn finish_cycle(
    runtime: &mut ApplicationRuntime,
    start_elapsed: Duration,
    rss_before_start: crate::memory::ProcessMemory,
) -> BenchmarkResult<ApplicationLifecycleCycle> {
    validate_started(runtime)?;
    let rss_after_start = process_memory()?;
    let shutdown_started = Instant::now();
    runtime.shutdown().map_err(|error| {
        BenchmarkError::new(format!(
            "download-free ApplicationRuntime shutdown failed: {error}"
        ))
    })?;
    let shutdown_elapsed = shutdown_started.elapsed();
    validate_stopped(runtime)?;
    let rss_after_shutdown = process_memory()?;
    Ok(ApplicationLifecycleCycle {
        start_ns: duration_ns(start_elapsed),
        shutdown_ns: duration_ns(shutdown_elapsed),
        rss_before_start,
        rss_after_start,
        rss_after_shutdown,
    })
}

fn application_configuration(
    database_path: std::path::PathBuf,
    cache_directory: &Path,
) -> ApplicationRuntimeConfiguration {
    let mut configuration = ApplicationRuntimeConfiguration::desktop(database_path);
    configuration.hub.cache_directory = Some(cache_directory.to_path_buf());
    configuration.hub.maximum_retries = 0;
    configuration.timing.runtime_poll = POLL_INTERVAL;
    configuration.timing.hub_worker_poll = POLL_INTERVAL;
    configuration.timing.hub_event_send_timeout = Duration::from_secs(1);
    configuration.timing.hub_command_shutdown_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.runtime_shutdown_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.runtime_shutdown_event_poll = POLL_INTERVAL;
    configuration.timing.runtime_join_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.runtime_join_poll = POLL_INTERVAL;
    configuration.timing.hub_shutdown_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.hub_shutdown_poll = POLL_INTERVAL;
    configuration
}

fn validate_started(runtime: &ApplicationRuntime) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || !state.hub_available()
        || !state.inference_available()
        || state.resolved().is_some()
        || state.loaded().is_some()
        || state.active_generation().is_some()
        || state.last_generation().is_some()
    {
        return Err(BenchmarkError::new(
            "download-free ApplicationRuntime start returned non-clean initial state",
        ));
    }
    Ok(())
}

fn validate_stopped(runtime: &ApplicationRuntime) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::ShuttingDown
        || state.hub_available()
        || state.inference_available()
        || state.loaded().is_some()
        || state.active_generation().is_some()
    {
        return Err(BenchmarkError::new(
            "ApplicationRuntime shutdown returned without terminal worker state",
        ));
    }
    Ok(())
}

fn usize_from_u32(value: u32) -> BenchmarkResult<usize> {
    usize::try_from(value)
        .map_err(|_| BenchmarkError::new("cycle count conversion to usize failed"))
}
