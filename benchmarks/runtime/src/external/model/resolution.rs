//! Public E1 resolution of the exact immutable model identity.

use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationEvent, ApplicationRuntime, ApplicationScalarType,
    ModelSelection, ResolvedModel,
};

use crate::error::{BenchmarkError, BenchmarkResult};

use super::identity::{validate_exact_selection, validate_resolved_facts};
use super::{MODEL_REPOSITORY, MODEL_REVISION, checked_deadline, wait_for_next_poll};

const RESOLUTION_TIMEOUT: Duration = Duration::from_mins(30);

pub(super) fn resolve_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
) -> BenchmarkResult<(ResolvedModel, Duration)> {
    validate_exact_selection(selection)?;
    validate_resolution_ready(runtime, selection)?;

    let started_at = Instant::now();
    runtime.resolve_model(selection.clone()).map_err(|error| {
        BenchmarkError::new(format!(
            "exact Hub resolution could not be submitted for {MODEL_REPOSITORY}@{MODEL_REVISION}: {error}"
        ))
    })?;
    let deadline = checked_deadline(RESOLUTION_TIMEOUT, "immutable Hub resolution")?;

    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelResolved {
                    model,
                    persistence_warning: None,
                } => {
                    validate_resolved_state(runtime, &model, selection)?;
                    return Ok((model, started_at.elapsed()));
                }
                ApplicationEvent::ModelResolved {
                    persistence_warning: Some(warning),
                    ..
                } => {
                    return Err(BenchmarkError::new(format!(
                        "Hub resolution succeeded but immutable catalogue persistence reported a warning: {warning}"
                    )));
                }
                ApplicationEvent::ModelResolutionFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "exact Hub resolution failed for {MODEL_REPOSITORY}@{MODEL_REVISION}: {failure}"
                    )));
                }
                ApplicationEvent::ModelCleanupPending { .. } => {
                    let cleanup = runtime.state().retained_model().ok_or_else(|| {
                        BenchmarkError::new("cleanup event omitted durable retained state")
                    })?;
                    return Err(BenchmarkError::new(format!(
                        "model cleanup retained E0 ownership during immutable resolution: disposition={:?}, primary_failure={}, cleanup_failure={:?}",
                        cleanup.cleanup(),
                        cleanup.primary_failure(),
                        cleanup.cleanup_failure()
                    )));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected during exact immutable resolution",
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during exact immutable resolution",
                    ));
                }
                unexpected => {
                    return Err(BenchmarkError::new(format!(
                        "unexpected application event during immutable resolution: {unexpected:?}"
                    )));
                }
            }
        }
        wait_for_next_poll(deadline, "immutable Hub resolution")?;
    }
}

pub(super) fn validate_resolved_state(
    runtime: &ApplicationRuntime,
    model: &ResolvedModel,
    selection: &ModelSelection,
) -> BenchmarkResult<Option<ApplicationScalarType>> {
    let configuration_declared_scalar_type = validate_resolved_facts(model, selection)?;
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.resolved() != Some(model)
        || state.loaded().is_some()
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
    {
        return Err(BenchmarkError::new(
            "public E1 state did not retain the clean exact immutable resolution",
        ));
    }
    Ok(configuration_declared_scalar_type)
}

fn validate_resolution_ready(
    runtime: &ApplicationRuntime,
    selection: &ModelSelection,
) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || !state.hub_available()
        || !state.inference_available()
        || state.loaded().is_some()
        || state.active_generation().is_some()
        || !state.can_resolve(selection)
    {
        return Err(BenchmarkError::new(
            "exact immutable resolution requires connected, idle E1 state without loaded or active ownership",
        ));
    }
    Ok(())
}
