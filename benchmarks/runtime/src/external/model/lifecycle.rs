//! Exact E1 load acceptance and synchronized public unload lifecycle.

use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationEvent, ApplicationRuntime, LoadedModel, ModelSelection,
    ModelUnloadBehavior, ResolvedModel,
};

use super::super::observation::DeviceObserver;
use super::super::report::UnloadResult;
use crate::error::{BenchmarkError, BenchmarkResult};

use super::identity::{
    EXPECTED_CONTEXT_TOKENS, EXPECTED_VOCABULARY_SIZE, validate_exact_selection,
    validate_resolved_facts,
};
use super::resolution::validate_resolved_state;
use super::{checked_deadline, wait_for_next_poll};

const LOAD_TIMEOUT: Duration = Duration::from_mins(10);
const UNLOAD_TIMEOUT: Duration = Duration::from_mins(2);

pub(super) fn load_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
    observer: &DeviceObserver,
) -> BenchmarkResult<(LoadedModel, Duration)> {
    validate_exact_selection(selection)?;
    let resolved = runtime
        .state()
        .resolved()
        .cloned()
        .ok_or_else(|| BenchmarkError::new("model load requires an E1-resolved model"))?;
    validate_resolved_state(runtime, &resolved, selection)?;
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
                    validate_loaded_state(runtime, &model, &resolved, selection, observer)?;
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
    observer: &DeviceObserver,
) -> BenchmarkResult {
    validate_resolved_facts(resolved, selection)?;
    observer.validate_selected_e1(runtime)?;
    observer.validate_actual_loaded(loaded.device())?;

    if loaded.selection() != selection
        || loaded.identity() != resolved.identity()
        || loaded.vocabulary_size() != EXPECTED_VOCABULARY_SIZE
        || loaded.vocabulary_size() != resolved.vocabulary_size()
        || loaded.maximum_context_tokens() != EXPECTED_CONTEXT_TOKENS
        || loaded.maximum_prefill_batch() != EXPECTED_CONTEXT_TOKENS
    {
        return Err(BenchmarkError::new(format!(
            "loaded model did not retain the exact resolved TinyLlama selection, identity, vocabulary, and capacities: {loaded:?}"
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
