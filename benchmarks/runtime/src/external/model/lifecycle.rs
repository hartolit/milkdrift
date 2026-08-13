//! Exact E1 load acceptance and synchronized public unload lifecycle.

use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationEvent, ApplicationRuntime, LoadedModel, ModelSelection,
    ModelUnloadBehavior, ResolvedModel,
};

use super::super::observation::DeviceObserver;
use super::super::report::UnloadResult;
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::support::application_wait::{
    ApplicationWaitStage, WaitStatus, drive_application_wait, unexpected_event,
};

use super::identity::{
    EXPECTED_CONTEXT_TOKENS, EXPECTED_VOCABULARY_SIZE, validate_exact_selection,
    validate_resolved_facts,
};
use super::resolution::validate_resolved_state;

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
    let model = drive_application_wait(
        runtime,
        LOAD_TIMEOUT,
        LoadStage {
            selection,
            resolved: &resolved,
            observer,
        },
    )?;
    Ok((model, started_at.elapsed()))
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
    let (cancelled_requests, observed_at) =
        drive_application_wait(runtime, UNLOAD_TIMEOUT, UnloadStage { loaded })?;
    let duration = observed_at
        .checked_duration_since(started_at)
        .ok_or_else(|| {
            BenchmarkError::new("synchronized model unload observation preceded its submission")
        })?;
    Ok(UnloadResult {
        duration_ns: super::super::generation::duration_ns(
            duration,
            "synchronized E1 model unload",
        )?,
        cancelled_requests,
    })
}

struct LoadStage<'a> {
    selection: &'a ModelSelection,
    resolved: &'a ResolvedModel,
    observer: &'a DeviceObserver,
}

impl ApplicationWaitStage<ApplicationRuntime> for LoadStage<'_> {
    type Output = LoadedModel;

    fn name(&self) -> &'static str {
        "Candle model load"
    }

    fn observe_event(
        &mut self,
        runtime: &mut ApplicationRuntime,
        event: ApplicationEvent,
        _observed_at: Instant,
    ) -> BenchmarkResult<WaitStatus<Self::Output>> {
        match event {
            ApplicationEvent::ModelLoaded { model } => {
                validate_loaded_state(
                    runtime,
                    &model,
                    self.resolved,
                    self.selection,
                    self.observer,
                )?;
                Ok(WaitStatus::Complete(model))
            }
            ApplicationEvent::ModelLoadFailed { failure } => Err(BenchmarkError::new(format!(
                "exact Candle model load failed: {failure}"
            ))),
            ApplicationEvent::ModelCompatibilityFailed { failure } => Err(BenchmarkError::new(
                format!("resolved and loaded model compatibility failed: {failure}"),
            )),
            unexpected => Err(unexpected_event(self.name(), &unexpected)),
        }
    }
}

struct UnloadStage<'a> {
    loaded: &'a LoadedModel,
}

impl ApplicationWaitStage<ApplicationRuntime> for UnloadStage<'_> {
    type Output = (u32, Instant);

    fn name(&self) -> &'static str {
        "model unload"
    }

    fn observe_event(
        &mut self,
        runtime: &mut ApplicationRuntime,
        event: ApplicationEvent,
        observed_at: Instant,
    ) -> BenchmarkResult<WaitStatus<Self::Output>> {
        match event {
            ApplicationEvent::ModelUnloaded {
                handle,
                cancelled_requests,
            } => {
                if handle != self.loaded.handle() || cancelled_requests != 0 {
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
                Ok(WaitStatus::Complete((cancelled_requests, observed_at)))
            }
            ApplicationEvent::ModelDraining { handle } => Err(BenchmarkError::new(format!(
                "RejectIfBusy unload entered draining for handle {handle:?} after all requests were already released"
            ))),
            ApplicationEvent::ModelUnloadFailed { failure } => Err(BenchmarkError::new(format!(
                "RejectIfBusy model unload failed: {failure}"
            ))),
            unexpected => Err(unexpected_event(self.name(), &unexpected)),
        }
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
