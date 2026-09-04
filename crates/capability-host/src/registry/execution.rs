use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use milkdrift_authority::{AuthorityEvaluator, CapabilityAuthorityScope};
use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor, CapabilityId,
    CapabilityObservation, CapabilityRequirement, InvocationEvent, InvocationId, InvocationRequest,
    OperationId, ResolvedCapabilitySnapshot, SideEffectClass,
};
use milkdrift_runtime::{
    CapabilityResolutionContext, ExecutionDispatch, ExecutionReporter, ExecutorError,
    PreparedExecution, ResolvedCapability, TaskExecutor,
};

use super::{CapabilityHost, GenerationHealth, GenerationKey, HostCore, HostError, RegistryState};
use crate::{
    AdapterError, AdapterExecutionContext, AdapterFailureKind, AdapterInvocation, AdapterReporter,
    CapabilityAdapter,
};

impl CapabilityHost {
    /// Executes one already-persisted exact snapshot without re-resolution or fallback.
    pub fn execute_exact(
        &self,
        snapshot: &ResolvedCapabilitySnapshot,
        request: &InvocationRequest,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), ExecutorError> {
        let (adapter, mut permit) = self.acquire(snapshot, request)?;
        let invocation = AdapterInvocation::new(snapshot, request);
        match catch_unwind(AssertUnwindSafe(|| adapter.execute(&invocation, reporter))) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                permit.failure = Some(error.summary().to_owned());
                Err(executor_error_from_adapter(&error))
            }
            Err(_panic) => {
                permit.failure = Some("adapter panicked".to_owned());
                Err(ExecutorError::AdapterPanicked { after_entry: true })
            }
        }
    }

    /// Executes one already-persisted exact snapshot with explicit durable provenance.
    pub fn execute_exact_with_context(
        &self,
        snapshot: &ResolvedCapabilitySnapshot,
        request: &InvocationRequest,
        context: &AdapterExecutionContext,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), ExecutorError> {
        let (adapter, mut permit) = self.acquire(snapshot, request)?;
        let invocation = AdapterInvocation::with_context(snapshot, request, context);
        match catch_unwind(AssertUnwindSafe(|| adapter.execute(&invocation, reporter))) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                permit.failure = Some(error.summary().to_owned());
                Err(executor_error_from_adapter(&error))
            }
            Err(_panic) => {
                permit.failure = Some("adapter panicked".to_owned());
                Err(ExecutorError::AdapterPanicked { after_entry: true })
            }
        }
    }

    /// Routes cancellation to the exact registered generation currently owning an invocation.
    ///
    /// This inherent boundary lets capability adapters such as peer transports reuse host
    /// ownership semantics without depending on the runtime trait that also exposes it.
    pub fn cancel_exact(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        let adapter = {
            let state = self.core.state.lock().map_err(|_error| {
                ExecutorError::BoundaryBeforeEntry("registry unavailable".to_owned())
            })?;
            let key = state.in_flight.get(request.invocation()).ok_or_else(|| {
                ExecutorError::Unavailable(
                    "no exact generation owns the cancellation invocation".to_owned(),
                )
            })?;
            state
                .generations
                .get(key)
                .ok_or_else(|| ExecutorError::UnavailableGeneration {
                    capability: key.capability.clone(),
                    descriptor_revision: key.revision,
                })?
                .adapter
                .clone()
        };
        match catch_unwind(AssertUnwindSafe(|| adapter.cancel(request))) {
            Ok(Ok(acknowledgement)) => {
                if acknowledgement.invocation() != request.invocation()
                    || acknowledgement.request_sequence() != request.request_sequence()
                {
                    return Err(ExecutorError::InvalidReports(
                        "cancellation acknowledgement does not match the exact request".to_owned(),
                    ));
                }
                Ok(acknowledgement)
            }
            Ok(Err(error)) => Err(executor_error_from_adapter(&error)),
            Err(_panic) => Err(ExecutorError::AdapterPanicked { after_entry: true }),
        }
    }

    fn acquire(
        &self,
        snapshot: &ResolvedCapabilitySnapshot,
        request: &InvocationRequest,
    ) -> Result<(Arc<dyn CapabilityAdapter>, Permit), ExecutorError> {
        snapshot
            .validate_request(request)
            .map_err(|error| ExecutorError::InvalidDispatch(error.to_string()))?;
        let key = GenerationKey {
            capability: snapshot.capability().clone(),
            revision: snapshot.descriptor_revision(),
        };
        let mut state = self.core.state.lock().map_err(|_error| {
            ExecutorError::BoundaryBeforeEntry("registry unavailable".to_owned())
        })?;
        if !state.admission_open || state.shutdown {
            return Err(ExecutorError::AdmissionClosed);
        }
        if state.in_flight.contains_key(request.invocation()) {
            return Err(ExecutorError::Overloaded(
                "invocation already owns a generation permit".to_owned(),
            ));
        }
        let generation = state.generations.get_mut(&key).ok_or_else(|| {
            ExecutorError::UnavailableGeneration {
                capability: key.capability.clone(),
                descriptor_revision: key.revision,
            }
        })?;
        snapshot.validate_against(&generation.descriptor)?;
        if generation.active >= generation.permit_limit {
            return Err(ExecutorError::Overloaded(format!(
                "{}/{}",
                key.capability, key.revision
            )));
        }
        let adapter = generation.adapter.clone();
        generation.active = generation.active.saturating_add(1);
        state
            .in_flight
            .insert(request.invocation().clone(), key.clone());
        Ok((
            adapter,
            Permit {
                core: self.core.clone(),
                key,
                invocation: request.invocation().clone(),
                failure: None,
            },
        ))
    }
}

impl TaskExecutor for CapabilityHost {
    fn resolve(
        &self,
        _requirement: &CapabilityRequirement,
        _observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        Err(ExecutorError::BoundaryBeforeEntry(
            "capability host resolution requires an execution-authority context".to_owned(),
        ))
    }

    fn resolve_authorized(
        &self,
        requirement: &CapabilityRequirement,
        authority: &CapabilityResolutionContext,
        evaluator: &dyn AuthorityEvaluator,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolve_authorized_at(requirement, authority, evaluator, observed_at_unix_ms)
    }

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        let (adapter, mut permit) = self.acquire(dispatch.resolution(), dispatch.request())?;
        let context = AdapterExecutionContext::from_dispatch(dispatch, None);
        let invocation =
            AdapterInvocation::with_context(dispatch.resolution(), dispatch.request(), &context);
        let envelope =
            match catch_unwind(AssertUnwindSafe(|| adapter.admission_envelope(&invocation))) {
                Ok(Ok(envelope)) => envelope,
                Ok(Err(error)) => {
                    permit.mark_failure(error.summary());
                    return Err(executor_error_from_adapter(&error));
                }
                Err(_panic) => {
                    permit.mark_failure("adapter panicked while deriving admission envelope");
                    return Err(ExecutorError::AdapterPanicked { after_entry: false });
                }
            };
        Ok(PreparedExecution::new_with_controller_reservation(
            dispatch,
            envelope,
            move |dispatch, reservation, reporter| {
                let bridge = ReporterBridge { reporter };
                let context = AdapterExecutionContext::from_dispatch(dispatch, reservation);
                let invocation = AdapterInvocation::with_context(
                    dispatch.resolution(),
                    dispatch.request(),
                    &context,
                );
                match catch_unwind(AssertUnwindSafe(|| adapter.execute(&invocation, &bridge))) {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(error)) => {
                        permit.mark_failure(error.summary());
                        Err(executor_error_from_adapter(&error))
                    }
                    Err(_panic) => {
                        permit.mark_failure("adapter panicked");
                        Err(ExecutorError::AdapterPanicked { after_entry: true })
                    }
                }
            },
        ))
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.cancel_exact(request)
    }
}

struct ReporterBridge<'a> {
    reporter: &'a dyn ExecutionReporter,
}

impl AdapterReporter for ReporterBridge<'_> {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        self.reporter
            .invocation(event)
            .map(|_disposition| ())
            .map_err(|error| AdapterError::reporter_failure(bounded_summary(&error.to_string())))
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        self.reporter
            .heartbeat()
            .map(|_disposition| ())
            .map_err(|error| AdapterError::reporter_failure(bounded_summary(&error.to_string())))
    }
}

struct Permit {
    core: Arc<HostCore>,
    key: GenerationKey,
    invocation: InvocationId,
    failure: Option<String>,
}

impl Permit {
    fn mark_failure(&mut self, summary: &str) {
        self.failure = Some(summary.to_owned());
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let Ok(mut state) = self.core.state.lock() else {
            return;
        };
        state.in_flight.remove(&self.invocation);
        if let Some(generation) = state.generations.get_mut(&self.key) {
            generation.active = generation.active.saturating_sub(1);
            if let Some(failure) = self.failure.take() {
                generation.last_failure = Some(bounded_summary(&failure));
            }
        }
    }
}

pub(super) fn update_current(state: &mut RegistryState, capability: &CapabilityId) {
    let current = state
        .generations
        .iter()
        .filter(|(key, generation)| key.capability == *capability && !generation.draining)
        .map(|(key, _generation)| key.revision)
        .max();
    match current {
        Some(revision) => {
            state.current.insert(capability.clone(), revision);
        }
        None => {
            state.current.remove(capability);
        }
    }
}

pub(super) fn lifecycle_call<T>(
    call: impl FnOnce() -> Result<T, AdapterError>,
) -> Result<T, HostError> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(result) => result.map_err(HostError::Adapter),
        Err(_panic) => Err(HostError::AdapterPanicked),
    }
}

pub(super) fn generation_health(
    observation: Option<&CapabilityObservation>,
    now: u64,
    stale_after: u64,
) -> GenerationHealth {
    let Some(observation) = observation else {
        return GenerationHealth::Unknown;
    };
    if now.saturating_sub(observation.observed_at_unix_ms()) > stale_after {
        GenerationHealth::Stale
    } else if observation.available() {
        GenerationHealth::Healthy
    } else {
        GenerationHealth::Unhealthy
    }
}

pub(super) fn observation_available(
    observation: Option<&CapabilityObservation>,
    now: u64,
    stale_after: u64,
) -> bool {
    matches!(
        generation_health(observation, now, stale_after),
        GenerationHealth::Healthy
    )
}

pub(super) fn scope_allows_operation(
    scope: &CapabilityAuthorityScope,
    descriptor: &CapabilityDescriptor,
    operation: &OperationId,
    side_effect: SideEffectClass,
) -> bool {
    !scope.denies_all()
        && scope
            .identity_selection()
            .is_some_and(|selection| selection.matches(descriptor.identity()))
        && scope
            .category_selection()
            .is_some_and(|selection| selection.matches(descriptor.category()))
        && scope
            .operation_selection()
            .is_some_and(|selection| selection.matches(operation))
        && scope.provider_profile_selection().is_some_and(|selection| {
            selection.is_any()
                || descriptor
                    .provider_profile()
                    .is_some_and(|profile| selection.matches(profile))
        })
        && scope
            .locality_selection()
            .is_some_and(|selection| selection.matches(&descriptor.locality()))
        && scope.peer_selection().is_some_and(|selection| {
            selection.is_any()
                || descriptor
                    .peer()
                    .is_some_and(|peer| selection.matches(peer))
        })
        && scope.trust_zone_selection().is_some_and(|selection| {
            selection.is_any()
                || descriptor
                    .trust_zones()
                    .iter()
                    .any(|zone| selection.matches(zone))
        })
        && scope
            .execution_trust_class_selection()
            .is_some_and(|selection| selection.matches(&descriptor.execution_trust()))
        && side_effect <= scope.maximum_side_effect()
}

fn executor_error_from_adapter(error: &AdapterError) -> ExecutorError {
    match error.kind() {
        AdapterFailureKind::Rejected | AdapterFailureKind::Unavailable => {
            ExecutorError::BoundaryBeforeEntry(error.summary().to_owned())
        }
        AdapterFailureKind::ExternalFailure => {
            ExecutorError::BoundaryAfterEntry(error.summary().to_owned())
        }
    }
}

fn bounded_summary(value: &str) -> String {
    milkdrift_contracts::truncate_utf8(value, 512).to_owned()
}
