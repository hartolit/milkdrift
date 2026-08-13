//! Public E1 resolution of the exact immutable model identity.

use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationEvent, ApplicationRuntime, ApplicationScalarType,
    ModelSelection, ResolvedModel,
};

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::support::application_wait::{
    ApplicationWaitStage, WaitStatus, drive_application_wait, unexpected_event,
};

use super::identity::{validate_exact_selection, validate_resolved_facts};
use super::{MODEL_REPOSITORY, MODEL_REVISION};

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
    let model = drive_application_wait(runtime, RESOLUTION_TIMEOUT, ResolutionStage { selection })?;
    Ok((model, started_at.elapsed()))
}

struct ResolutionStage<'a> {
    selection: &'a ModelSelection,
}

impl ApplicationWaitStage<ApplicationRuntime> for ResolutionStage<'_> {
    type Output = ResolvedModel;

    fn name(&self) -> &'static str {
        "immutable Hub resolution"
    }

    fn observe_event(
        &mut self,
        runtime: &mut ApplicationRuntime,
        event: ApplicationEvent,
        _observed_at: Instant,
    ) -> BenchmarkResult<WaitStatus<Self::Output>> {
        match event {
            ApplicationEvent::ModelResolved {
                model,
                persistence_warning: None,
            } => {
                validate_resolved_state(runtime, &model, self.selection)?;
                Ok(WaitStatus::Complete(model))
            }
            ApplicationEvent::ModelResolved {
                persistence_warning: Some(warning),
                ..
            } => Err(BenchmarkError::new(format!(
                "Hub resolution succeeded but immutable catalogue persistence reported a warning: {warning}"
            ))),
            ApplicationEvent::ModelResolutionFailed { failure } => {
                Err(BenchmarkError::new(format!(
                    "exact Hub resolution failed for {MODEL_REPOSITORY}@{MODEL_REVISION}: {failure}"
                )))
            }
            unexpected => Err(unexpected_event(self.name(), &unexpected)),
        }
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
