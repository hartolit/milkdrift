//! Exact model identity, resolution, independent planning, loading, and unloading.

mod identity;
mod lifecycle;
mod planning;
mod resolution;

use std::path::Path;
use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationRuntime, ApplicationScalarType, LoadedModel, ModelSelection, ResolvedModel,
};
use domain_contracts::ScalarTypeSet;

use super::cli::RequestedDevice;
use super::observation::DeviceObserver;
use super::report::{PreparedLoadEvidence, UnloadResult};
use crate::error::{BenchmarkError, BenchmarkResult};

/// Exact external-product repository identity.
pub(super) const MODEL_REPOSITORY: &str = "TinyLlama/TinyLlama-1.1B-Chat-v1.0";
/// Exact immutable external-product revision and resolved commit.
pub(super) const MODEL_REVISION: &str = "fe8a4ea1ffedaf415f4da2f062534de366a451e6";
/// Exact external-product architecture label.
pub(super) const MODEL_ARCHITECTURE: &str = "Llama";

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Observer-owned prepared-load evidence retained until exact E1 load acceptance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlannedModelEvidence {
    /// Exact final and loading-peak quantities from an unmaterialized `prepare_load` transaction.
    pub(super) prepared_load: PreparedLoadEvidence,
    /// Optional configuration declaration matched across E1 resolution and adapter preparation.
    pub(super) configuration_declared_scalar_type: Option<ApplicationScalarType>,
    /// Scalar categories observed by adapter preparation in every selected tensor header.
    pub(super) observed_tensor_scalar_types: ScalarTypeSet,
    /// Execution scalar selected by `LoadPlan::execution_scalar_type`.
    pub(super) planned_execution_scalar_type: ApplicationScalarType,
    requested_device: RequestedDevice,
}

/// Resolves and validates the built-in immutable model selection through public E1 events.
pub(super) fn resolve_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
) -> BenchmarkResult<(ResolvedModel, Duration)> {
    resolution::resolve_model(runtime, selection)
}

/// Produces independent public Candle plan evidence without loading a second model.
pub(super) fn plan_resolved_model(
    cache_directory: &Path,
    runtime: &ApplicationRuntime,
    requested_device: RequestedDevice,
) -> BenchmarkResult<PlannedModelEvidence> {
    planning::plan_resolved_model(cache_directory, runtime, requested_device)
}

/// Loads through E1 and accepts the independent plan only after exact event/state validation.
pub(super) fn load_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
    planned: &mut PlannedModelEvidence,
    observer: &DeviceObserver,
) -> BenchmarkResult<(LoadedModel, Duration)> {
    lifecycle::load_model(runtime, selection, planned, observer)
}

/// Validates that observer preparation remains coherent after E1 load acceptance.
pub(super) fn validate_verified_plan(
    planned: &PlannedModelEvidence,
    observer: &DeviceObserver,
) -> BenchmarkResult {
    planning::validate_verified_plan(planned, observer)
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
