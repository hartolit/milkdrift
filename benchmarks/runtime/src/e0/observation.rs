//! E0 snapshot capture, exact accounting validation, and report conversion.

use domain_contracts::{MemoryFootprint, ModelHandle, ModelLifecycleState};
use inference_runtime::{RuntimeCommand, RuntimeEvent, RuntimeSnapshot};

use super::harness::HostedE0Harness;
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::memory::process_memory;
use crate::report::{
    MemoryFootprintRecord, ModelAccounting, RuntimeAccounting, SnapshotCheckpoint,
};

pub(super) struct CapturedSnapshot {
    pub(super) raw: RuntimeSnapshot,
    pub(super) models: Vec<inference_runtime::ModelSnapshot>,
    pub(super) record: SnapshotCheckpoint,
}

pub(super) fn capture_snapshot(
    harness: &mut HostedE0Harness,
    checkpoint: &'static str,
) -> BenchmarkResult<CapturedSnapshot> {
    let ticket = harness.ticket()?;
    harness.submit(RuntimeCommand::Snapshot { ticket }, "runtime snapshot")?;
    let event = harness.receive(ticket, "runtime snapshot")?;
    let RuntimeEvent::Snapshot {
        runtime, models, ..
    } = event
    else {
        return Err(BenchmarkError::new(
            "runtime snapshot command returned a non-snapshot event",
        ));
    };
    validate_no_cleanup_failure(&runtime, &models, checkpoint)?;
    let model_records = models.iter().map(model_accounting).collect();
    let record = SnapshotCheckpoint {
        checkpoint,
        process_memory: process_memory()?,
        runtime: runtime_accounting(&runtime),
        models: model_records,
    };
    Ok(CapturedSnapshot {
        raw: runtime,
        models,
        record,
    })
}

fn validate_no_cleanup_failure(
    runtime: &RuntimeSnapshot,
    models: &[inference_runtime::ModelSnapshot],
    checkpoint: &str,
) -> BenchmarkResult {
    if runtime.unverified_ownership.is_some()
        || runtime.admission_blocked
        || runtime.pending_cleanup_models != 0
        || runtime.pending_cleanup_sequences != 0
        || runtime.exhausted_cleanup_models != 0
        || runtime.exhausted_cleanup_sequences != 0
        || runtime.maintenance_error.is_some()
        || models.iter().any(|model| {
            model.pending_cleanup_sequences != 0
                || model.exhausted_cleanup_sequences != 0
                || model.degraded
        })
    {
        return Err(BenchmarkError::new(format!(
            "{checkpoint} snapshot contains pending, exhausted, degraded, or failed cleanup accounting"
        )));
    }
    Ok(())
}

pub(super) fn validate_empty_snapshot(
    snapshot: &CapturedSnapshot,
    checkpoint: &str,
) -> BenchmarkResult {
    let runtime = snapshot.raw;
    if runtime.loaded_models != 0
        || runtime.active_requests != 0
        || runtime.reserved_footprint != MemoryFootprint::default()
        || runtime.unverified_ownership.is_some()
        || runtime.admission_blocked
        || runtime.generation_workspaces != 0
        || runtime.reserved_generation_workspace != MemoryFootprint::default()
        || runtime.pending_cleanup_models != 0
        || runtime.pending_cleanup_sequences != 0
        || runtime.exhausted_cleanup_models != 0
        || runtime.exhausted_cleanup_sequences != 0
        || runtime.last_cleanup.is_some()
        || runtime.maintenance_error.is_some()
        || runtime.shutting_down
        || !snapshot.models.is_empty()
    {
        return Err(BenchmarkError::new(format!(
            "{checkpoint} did not have exact empty E0 accounting"
        )));
    }
    Ok(())
}

pub(super) fn validate_loaded_idle_snapshot(
    snapshot: &CapturedSnapshot,
    handle: ModelHandle,
    expected_footprint: MemoryFootprint,
    checkpoint: &str,
) -> BenchmarkResult {
    let runtime = snapshot.raw;
    let model = only_model(snapshot, checkpoint)?;
    if runtime.loaded_models != 1
        || runtime.active_requests != 0
        || runtime.reserved_footprint != expected_footprint
        || runtime.unverified_ownership.is_some()
        || runtime.admission_blocked
        || runtime.generation_workspaces != 0
        || runtime.reserved_generation_workspace != MemoryFootprint::default()
        || runtime.last_cleanup.is_some()
        || runtime.shutting_down
        || model.handle != handle
        || model.lifecycle != ModelLifecycleState::Ready
        || model.reserved_footprint != expected_footprint
        || model.active_requests != 0
        || model.pending_cleanup_sequences != 0
        || model.exhausted_cleanup_sequences != 0
        || model.degraded
    {
        return Err(BenchmarkError::new(format!(
            "{checkpoint} did not have exact loaded-idle E0 accounting"
        )));
    }
    Ok(())
}

pub(super) fn validate_active_snapshot(
    snapshot: &CapturedSnapshot,
    handle: ModelHandle,
    loaded_footprint: MemoryFootprint,
    request_footprint: MemoryFootprint,
    checkpoint: &str,
) -> BenchmarkResult {
    let runtime = snapshot.raw;
    let model = only_model(snapshot, checkpoint)?;
    let expected_footprint =
        checked_add_footprints(loaded_footprint, request_footprint, checkpoint)?;
    let generation_workspace = runtime.reserved_generation_workspace;
    let exact_generation_workspace = runtime.generation_workspaces == 1
        && generation_workspace.host_weight_bytes == 0
        && generation_workspace.device_weight_bytes == 0
        && generation_workspace.host_working_bytes != 0
        && generation_workspace.device_working_bytes == 0
        && footprint_contains(request_footprint, generation_workspace);
    if runtime.loaded_models != 1
        || runtime.active_requests != 1
        || runtime.reserved_footprint != expected_footprint
        || runtime.unverified_ownership.is_some()
        || runtime.admission_blocked
        || !exact_generation_workspace
        || runtime.last_cleanup.is_some()
        || runtime.shutting_down
        || model.handle != handle
        || !matches!(
            model.lifecycle,
            ModelLifecycleState::Active { active_requests: 1 }
        )
        || model.reserved_footprint != expected_footprint
        || model.active_requests != 1
        || model.pending_cleanup_sequences != 0
        || model.exhausted_cleanup_sequences != 0
        || model.degraded
    {
        return Err(BenchmarkError::new(format!(
            "{checkpoint} did not have exact active-request, footprint, lifecycle, and generation-workspace accounting"
        )));
    }
    Ok(())
}

fn checked_add_footprints(
    left: MemoryFootprint,
    right: MemoryFootprint,
    checkpoint: &str,
) -> BenchmarkResult<MemoryFootprint> {
    left.checked_add(right)
        .ok_or_else(|| footprint_overflow(checkpoint))
}

fn footprint_overflow(checkpoint: &str) -> BenchmarkError {
    BenchmarkError::new(format!("{checkpoint} active footprint addition overflowed"))
}

const fn footprint_contains(available: MemoryFootprint, required: MemoryFootprint) -> bool {
    available.contains_components(required)
}

fn only_model<'a>(
    snapshot: &'a CapturedSnapshot,
    checkpoint: &str,
) -> BenchmarkResult<&'a inference_runtime::ModelSnapshot> {
    if snapshot.models.len() != 1 {
        return Err(BenchmarkError::new(format!(
            "{checkpoint} expected exactly one model snapshot"
        )));
    }
    snapshot
        .models
        .first()
        .ok_or_else(|| BenchmarkError::new(format!("{checkpoint} model snapshot disappeared")))
}

fn runtime_accounting(snapshot: &RuntimeSnapshot) -> RuntimeAccounting {
    RuntimeAccounting {
        loaded_models: snapshot.loaded_models,
        active_requests: snapshot.active_requests,
        reserved_footprint: footprint(snapshot.reserved_footprint),
        generation_workspaces: snapshot.generation_workspaces,
        reserved_generation_workspace: footprint(snapshot.reserved_generation_workspace),
        pending_cleanup_models: snapshot.pending_cleanup_models,
        pending_cleanup_sequences: snapshot.pending_cleanup_sequences,
        exhausted_cleanup_models: snapshot.exhausted_cleanup_models,
        exhausted_cleanup_sequences: snapshot.exhausted_cleanup_sequences,
        last_cleanup_present: snapshot.last_cleanup.is_some(),
        maintenance_error_present: snapshot.maintenance_error.is_some(),
        shutting_down: snapshot.shutting_down,
    }
}

fn model_accounting(snapshot: &inference_runtime::ModelSnapshot) -> ModelAccounting {
    ModelAccounting {
        model_id: snapshot.handle.id.get(),
        generation: snapshot.handle.generation.get(),
        lifecycle: lifecycle_label(snapshot.lifecycle),
        reserved_footprint: footprint(snapshot.reserved_footprint),
        active_requests: snapshot.active_requests,
        pending_cleanup_sequences: snapshot.pending_cleanup_sequences,
        exhausted_cleanup_sequences: snapshot.exhausted_cleanup_sequences,
        degraded: snapshot.degraded,
    }
}

const fn lifecycle_label(state: ModelLifecycleState) -> &'static str {
    match state {
        ModelLifecycleState::Absent => "absent",
        ModelLifecycleState::Loading => "loading",
        ModelLifecycleState::Ready => "ready",
        ModelLifecycleState::Active { .. } => "active",
        ModelLifecycleState::Draining { .. } => "draining",
        ModelLifecycleState::Cancelling { .. } => "cancelling",
        ModelLifecycleState::Unloading => "unloading",
        ModelLifecycleState::Failed { .. } => "failed",
    }
}

const fn footprint(value: MemoryFootprint) -> MemoryFootprintRecord {
    MemoryFootprintRecord {
        host_weight_bytes: value.host_weight_bytes,
        device_weight_bytes: value.device_weight_bytes,
        host_working_bytes: value.host_working_bytes,
        device_working_bytes: value.device_working_bytes,
    }
}
