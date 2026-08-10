//! Exact model identity, public E1 resolution, loading, and unloading.

mod identity;
mod lifecycle;
mod resolution;

use std::time::{Duration, Instant};

use application_runtime::{ApplicationRuntime, LoadedModel, ModelSelection, ResolvedModel};

use super::observation::DeviceObserver;
use super::report::UnloadResult;
use crate::error::{BenchmarkError, BenchmarkResult};

/// Exact external-product repository identity.
pub(super) const MODEL_REPOSITORY: &str = "TinyLlama/TinyLlama-1.1B-Chat-v1.0";
/// Exact immutable external-product revision and resolved commit.
pub(super) const MODEL_REVISION: &str = "fe8a4ea1ffedaf415f4da2f062534de366a451e6";
/// Exact external-product architecture label.
pub(super) const MODEL_ARCHITECTURE: &str = "Llama";

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Resolves and validates the built-in immutable model selection through public E1 events.
pub(super) fn resolve_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
) -> BenchmarkResult<(ResolvedModel, Duration)> {
    resolution::resolve_model(runtime, selection)
}

/// Loads through E1 and validates the actual public loaded-model facts.
pub(super) fn load_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
    observer: &DeviceObserver,
) -> BenchmarkResult<(LoadedModel, Duration)> {
    lifecycle::load_model(runtime, selection, observer)
}

/// Unloads the released model and validates the terminal public E1 contract.
pub(super) fn unload_model(
    runtime: &mut ApplicationRuntime,
    loaded: &LoadedModel,
) -> BenchmarkResult<UnloadResult> {
    lifecycle::unload_model(runtime, loaded)
}

fn checked_deadline(timeout: Duration, operation: &'static str) -> BenchmarkResult<Instant> {
    Instant::now().checked_add(timeout).ok_or_else(|| {
        BenchmarkError::new(format!(
            "deadline overflow while preparing to wait for {operation}"
        ))
    })
}

fn wait_for_next_poll(deadline: Instant, operation: &'static str) -> BenchmarkResult {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| BenchmarkError::new(format!("timed out waiting for {operation}")))?;
    std::thread::sleep(POLL_INTERVAL.min(remaining));
    Ok(())
}
