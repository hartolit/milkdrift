//! Exact E1 load acceptance and synchronized public unload lifecycle.

use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationDevice, ApplicationEvent, ApplicationRuntime,
    ApplicationScalarType, LoadedModel, ModelSelection, ModelUnloadBehavior, ResolvedModel,
};

use super::super::observation::DeviceObserver;
use super::super::report::UnloadResult;
use crate::error::{BenchmarkError, BenchmarkResult};

use super::identity::{
    EXPECTED_CONTEXT_TOKENS, EXPECTED_VOCABULARY_SIZE, validate_exact_selection,
    validate_resolved_facts,
};
use super::planning::validate_unverified_plan;
use super::resolution::validate_resolved_state;
use super::{PlannedModelEvidence, checked_deadline, wait_for_next_poll};

const LOAD_TIMEOUT: Duration = Duration::from_mins(10);
const UNLOAD_TIMEOUT: Duration = Duration::from_mins(2);

pub(super) fn load_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
    planned: &mut PlannedModelEvidence,
    observer: &DeviceObserver,
) -> BenchmarkResult<(LoadedModel, Duration)> {
    validate_exact_selection(selection)?;
    let resolved = runtime
        .state()
        .resolved()
        .cloned()
        .ok_or_else(|| BenchmarkError::new("model load requires an E1-resolved model"))?;
    validate_resolved_state(runtime, &resolved, selection)?;
    validate_unverified_plan(planned, observer)?;
    observer.validate_selected_e1(runtime)?;
    if !runtime.state().can_load(selection) {
        return Err(BenchmarkError::new(
            "public E1 state did not admit the exact resolved model for loading",
        ));
    }

    let started_at = Instant::now();
    runtime.load_model(selection).map_err(|error| {
        BenchmarkError::new(format!(
            "exact Candle model load could not be submitted: {error}"
        ))
    })?;
    let deadline = checked_deadline(LOAD_TIMEOUT, "Candle model load")?;

    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelLoaded { model } => {
                    validate_loaded_state(
                        runtime, &model, &resolved, selection, planned, observer,
                    )?;
                    // This transition follows complete resolved declaration, prepared layout,
                    // planned/actual execution, device, identity, and capacity validation.
                    planned.record_e1_load_acceptance()?;
                    return Ok((model, started_at.elapsed()));
                }
                ApplicationEvent::ModelLoadFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "exact Candle model load failed: {failure}"
                    )));
                }
                ApplicationEvent::ModelCleanupPending { cleanup } => {
                    return Err(BenchmarkError::new(format!(
                        "exact Candle model load retained E0 cleanup ownership: disposition={:?}, primary_failure={}, cleanup_failure={:?}",
                        cleanup.cleanup(),
                        cleanup.primary_failure(),
                        cleanup.cleanup_failure()
                    )));
                }
                ApplicationEvent::ModelCompatibilityFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "resolved and loaded model compatibility failed: {failure}"
                    )));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected while the exact model was loading",
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during exact model load",
                    ));
                }
                unexpected => {
                    return Err(BenchmarkError::new(format!(
                        "unexpected application event during exact model load: {unexpected:?}"
                    )));
                }
            }
        }
        wait_for_next_poll(deadline, "Candle model load")?;
    }
}

pub(super) fn unload_model(
    runtime: &mut ApplicationRuntime,
    loaded: &LoadedModel,
) -> BenchmarkResult<UnloadResult> {
    validate_unload_admission(runtime, loaded)?;

    let started_at = Instant::now();
    runtime
        .unload_model_with_behavior(ModelUnloadBehavior::RejectIfBusy)
        .map_err(|error| {
            BenchmarkError::new(format!(
                "RejectIfBusy model unload could not be submitted after release: {error}"
            ))
        })?;
    let deadline = checked_deadline(UNLOAD_TIMEOUT, "model unload")?;

    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelUnloaded {
                    handle,
                    cancelled_requests,
                } => {
                    if handle != loaded.handle() || cancelled_requests != 0 {
                        return Err(BenchmarkError::new(format!(
                            "model unload receipt did not match the loaded handle with zero cancellations: handle={handle:?}, cancelled_requests={cancelled_requests}"
                        )));
                    }
                    let state = runtime.state();
                    if state.activity() != ApplicationActivity::Idle
                        || state.loaded().is_some()
                        || state.active_generation().is_some()
                        || !state.hub_available()
                        || !state.inference_available()
                    {
                        return Err(BenchmarkError::new(
                            "public E1 state retained loaded/active ownership or disconnected after synchronized model removal",
                        ));
                    }
                    return Ok(UnloadResult {
                        duration_ns: super::super::generation::duration_ns(
                            started_at.elapsed(),
                            "synchronized E1 model unload",
                        )?,
                        cancelled_requests,
                        loaded_model_absent: true,
                        active_generation_absent: true,
                        runtime_connected: true,
                        backend_release_synchronized: true,
                    });
                }
                ApplicationEvent::ModelDraining { handle } => {
                    return Err(BenchmarkError::new(format!(
                        "RejectIfBusy unload entered draining for handle {handle:?} after all requests were already released"
                    )));
                }
                ApplicationEvent::ModelUnloadFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "RejectIfBusy model unload failed: {failure}"
                    )));
                }
                ApplicationEvent::ModelCleanupPending { cleanup } => {
                    return Err(BenchmarkError::new(format!(
                        "model cleanup retained E0 ownership during unload: disposition={:?}, primary_failure={}, cleanup_failure={:?}",
                        cleanup.cleanup(),
                        cleanup.primary_failure(),
                        cleanup.cleanup_failure()
                    )));
                }
                ApplicationEvent::GenerationCleanupPending {
                    request_id,
                    exhausted,
                    failure,
                } => {
                    return Err(BenchmarkError::new(format!(
                        "generation cleanup remained pending for request {} during unload (exhausted={exhausted}): {failure}",
                        request_id.get()
                    )));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected before explicit shutdown",
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during model unload",
                    ));
                }
                unexpected => {
                    return Err(BenchmarkError::new(format!(
                        "unexpected application event during model unload: {unexpected:?}"
                    )));
                }
            }
        }
        wait_for_next_poll(deadline, "model unload")?;
    }
}

fn validate_unload_admission(
    runtime: &ApplicationRuntime,
    loaded: &LoadedModel,
) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.loaded() != Some(loaded)
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
        || !state.can_unload()
    {
        return Err(BenchmarkError::new(
            "RejectIfBusy unload requires the exact loaded handle in connected, idle E1 state after every request release",
        ));
    }
    Ok(())
}

fn validate_loaded_state(
    runtime: &ApplicationRuntime,
    loaded: &LoadedModel,
    resolved: &ResolvedModel,
    selection: &ModelSelection,
    planned: &PlannedModelEvidence,
    observer: &DeviceObserver,
) -> BenchmarkResult {
    let resolved_declaration = validate_resolved_facts(resolved, selection)?;
    validate_unverified_plan(planned, observer)?;
    observer.validate_selected_e1(runtime)?;
    observer.validate_actual_loaded(loaded.device())?;

    validate_loaded_scalar_facts(
        resolved_declaration,
        planned.configuration_declared_scalar_type,
        planned.planned_execution_scalar_type,
        loaded.execution_scalar_type(),
    )?;

    if loaded.selection() != selection
        || loaded.identity() != resolved.identity()
        || loaded.device() != requested_application_device(planned.requested_device)
        || loaded.vocabulary_size() != EXPECTED_VOCABULARY_SIZE
        || loaded.vocabulary_size() != resolved.vocabulary_size()
        || loaded.maximum_context_tokens() != EXPECTED_CONTEXT_TOKENS
        || loaded.maximum_prefill_batch() != EXPECTED_CONTEXT_TOKENS
    {
        return Err(BenchmarkError::new(format!(
            "loaded model did not retain the exact resolved TinyLlama selection, identity, actual device, execution declaration, and capacities: {loaded:?}"
        )));
    }

    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.resolved() != Some(resolved)
        || state.loaded() != Some(loaded)
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
        || !state.can_start_generation()
        || !runtime.can_submit_chat_message()
        || !runtime.conversation().is_empty()
        || runtime.context_diagnostics().is_some()
    {
        return Err(BenchmarkError::new(
            "public E1 state did not retain the exact loaded model with clean direct/chat admission",
        ));
    }
    Ok(())
}

fn validate_loaded_scalar_facts(
    resolved_declaration: Option<ApplicationScalarType>,
    prepared_declaration: Option<ApplicationScalarType>,
    planned_execution: ApplicationScalarType,
    actual_execution: ApplicationScalarType,
) -> BenchmarkResult {
    if prepared_declaration != resolved_declaration {
        return Err(BenchmarkError::new(format!(
            "prepared configuration declaration did not match optional E1 resolution metadata: resolved={resolved_declaration:?}, prepared={prepared_declaration:?}"
        )));
    }
    if actual_execution != planned_execution {
        return Err(BenchmarkError::new(format!(
            "actual E1 execution scalar {actual_execution:?} did not match prepare_load execution scalar {planned_execution:?}"
        )));
    }
    Ok(())
}

const fn requested_application_device(
    requested: super::super::cli::RequestedDevice,
) -> ApplicationDevice {
    match requested {
        super::super::cli::RequestedDevice::Cpu => ApplicationDevice::Cpu,
        super::super::cli::RequestedDevice::Cuda0 => ApplicationDevice::Cuda { ordinal: 0 },
    }
}

#[cfg(test)]
mod tests {
    use application_runtime::ApplicationScalarType;

    use super::validate_loaded_scalar_facts;

    #[test]
    fn loaded_facts_match_optional_resolution_and_prepared_execution() -> Result<(), String> {
        validate_loaded_scalar_facts(
            Some(ApplicationScalarType::Bf16),
            Some(ApplicationScalarType::Bf16),
            ApplicationScalarType::F32,
            ApplicationScalarType::F32,
        )
        .map_err(|error| error.to_string())?;
        validate_loaded_scalar_facts(
            None,
            None,
            ApplicationScalarType::Bf16,
            ApplicationScalarType::Bf16,
        )
        .map_err(|error| error.to_string())
    }

    #[test]
    fn loaded_execution_scalar_mismatch_is_rejected_before_receipt_verification() {
        assert!(
            validate_loaded_scalar_facts(
                Some(ApplicationScalarType::Bf16),
                Some(ApplicationScalarType::Bf16),
                ApplicationScalarType::Bf16,
                ApplicationScalarType::F32,
            )
            .is_err()
        );
        assert!(
            validate_loaded_scalar_facts(
                Some(ApplicationScalarType::Bf16),
                Some(ApplicationScalarType::F32),
                ApplicationScalarType::F32,
                ApplicationScalarType::F32,
            )
            .is_err()
        );
    }
}
