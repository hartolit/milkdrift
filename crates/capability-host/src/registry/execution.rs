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
    CapabilityAdapter, PreparedAdapterExecution,
};

impl CapabilityHost {
    pub(crate) fn verify_current_project(
        &self,
        binding: &milkdrift_blueprint::TaskProjectBinding,
    ) -> Result<(), ExecutorError> {
        match self.core.native_paths.get() {
            Some(claims) => claims.verify_project(binding),
            None => crate::verify_project_binding(binding),
        }
    }

    pub(crate) fn claim_serving_paths(
        &self,
        record: &milkdrift_persistence::PeerExecutionRecord,
        paths: &[milkdrift_blueprint::TaskPathAccess],
    ) -> Result<Option<String>, ExecutorError> {
        if paths.is_empty() {
            return Ok(None);
        }
        self.core
            .native_paths
            .get()
            .ok_or_else(|| {
                ExecutorError::BoundaryBeforeEntry(
                    "native path claim owner is not configured for serving execution".into(),
                )
            })?
            .acquire_serving(record, paths)
    }

    pub(crate) fn revalidate_native_owner(&self, owner: &str) -> Result<(), ExecutorError> {
        self.core
            .native_paths
            .get()
            .ok_or_else(|| {
                ExecutorError::BoundaryBeforeEntry(
                    "native path claim owner disappeared before entry".into(),
                )
            })?
            .revalidate(owner)
    }

    pub(crate) fn model_path_roots(
        &self,
        context: &AdapterExecutionContext,
    ) -> Result<Vec<milkdrift_persistence::NativePathRoot>, AdapterError> {
        if let Some(binding) = context.project_binding() {
            self.verify_current_project(binding)
                .map_err(|error| AdapterError::rejected(error.to_string()))?;
        }
        if context.task_paths.is_empty() {
            return Ok(Vec::new());
        }
        let attempt = context
            .workflow()
            .ok_or_else(|| AdapterError::rejected("model path claims require task provenance"))?
            .attempt();
        let roots = self
            .core
            .native_paths
            .get()
            .ok_or_else(|| AdapterError::rejected("native path owner missing"))?
            .roots(attempt.as_str())
            .map_err(|error| AdapterError::rejected(error.to_string()))?;
        // Configured process directories also hold claims, but never become file-tool permission.
        Ok(roots
            .into_iter()
            .filter_map(|mut root| {
                let path = context.task_paths.iter().find(|path| {
                    root.requested == std::path::Path::new(&path.path)
                        && (!path.write || root.write)
                })?;
                root.write = path.write;
                Some(root)
            })
            .collect())
    }

    pub(crate) fn revalidate_model_paths(
        &self,
        context: &AdapterExecutionContext,
    ) -> Result<(), AdapterError> {
        if let Some(binding) = context.project_binding() {
            self.verify_current_project(binding)
                .map_err(|error| AdapterError::rejected(error.to_string()))?;
        }
        if !context.task_paths.is_empty()
            || context
                .task_tools
                .iter()
                .any(|tool| matches!(tool, milkdrift_blueprint::TaskTool::Process { .. }))
        {
            let attempt = context
                .workflow()
                .ok_or_else(|| AdapterError::rejected("model path claims require task provenance"))?
                .attempt();
            let claims = self
                .core
                .native_paths
                .get()
                .ok_or_else(|| AdapterError::rejected("native path owner missing"))?;
            if context.task_paths.is_empty() {
                claims.revalidate_if_present(attempt.as_str())
            } else {
                claims.revalidate(attempt.as_str())
            }
            .map_err(|error| AdapterError::rejected(error.to_string()))?;
        }
        Ok(())
    }

    pub(crate) fn declared_native_paths(
        &self,
        snapshot: &ResolvedCapabilitySnapshot,
        request: &InvocationRequest,
        context: &AdapterExecutionContext,
    ) -> Result<Vec<milkdrift_blueprint::TaskPathAccess>, ExecutorError> {
        snapshot
            .validate_request(request)
            .map_err(|error| ExecutorError::InvalidDispatch(error.to_string()))?;
        let adapter = {
            let state = self.core.state.lock().map_err(|_| {
                ExecutorError::BoundaryBeforeEntry("host registry unavailable".into())
            })?;
            if !state.admission_open || state.shutdown {
                return Err(ExecutorError::AdmissionClosed);
            }
            let generation = state
                .generations
                .get(&GenerationKey {
                    capability: snapshot.capability().clone(),
                    revision: snapshot.descriptor_revision(),
                })
                .ok_or_else(|| ExecutorError::UnavailableGeneration {
                    capability: snapshot.capability().clone(),
                    descriptor_revision: snapshot.descriptor_revision(),
                })?;
            snapshot.validate_against(&generation.descriptor)?;
            generation.adapter.clone()
        };
        let invocation = invocation_view(snapshot, request, Some(context));
        match catch_unwind(AssertUnwindSafe(|| adapter.native_path_claims(&invocation))) {
            Ok(Ok(paths)) if paths.len() <= 16 => paths
                .into_iter()
                .map(|scope| {
                    let path = scope.path.to_str().ok_or_else(|| {
                        ExecutorError::BoundaryBeforeEntry(
                            "configured native scope must be a UTF-8 path".into(),
                        )
                    })?;
                    Ok(milkdrift_blueprint::TaskPathAccess {
                        path: path.to_owned(),
                        write: scope.write,
                    })
                })
                .collect(),
            Ok(Ok(_)) => Err(ExecutorError::BoundaryBeforeEntry(
                "configured native scope exceeds sixteen directories".into(),
            )),
            Ok(Err(error)) => Err(executor_error_from_adapter(&error)),
            Err(_) => Err(ExecutorError::AdapterPanicked { after_entry: false }),
        }
    }

    pub(crate) fn verify_process_tool_claims(
        &self,
        snapshot: &ResolvedCapabilitySnapshot,
        request: &InvocationRequest,
        context: &AdapterExecutionContext,
    ) -> Result<(), ExecutorError> {
        let paths = self.declared_native_paths(snapshot, request, context)?;
        if paths.is_empty() {
            return Ok(());
        }
        let owner = context
            .workflow()
            .ok_or_else(|| {
                ExecutorError::BoundaryBeforeEntry(
                    "configured process tools require a durable parent attempt".into(),
                )
            })?
            .attempt();
        self.core
            .native_paths
            .get()
            .ok_or_else(|| {
                ExecutorError::BoundaryBeforeEntry(
                    "configured process tool native claim owner is unavailable".into(),
                )
            })?
            .verify_configured(owner.as_str(), &paths)
    }

    pub(crate) fn loop_adapter(
        &self,
        snapshot: &ResolvedCapabilitySnapshot,
    ) -> Result<Arc<dyn CapabilityAdapter>, AdapterError> {
        let state = self
            .core
            .state
            .lock()
            .map_err(|_| AdapterError::unavailable("host registry unavailable"))?;
        let generation = state
            .generations
            .get(&GenerationKey {
                capability: snapshot.capability().clone(),
                revision: snapshot.descriptor_revision(),
            })
            .ok_or_else(|| AdapterError::rejected("selected tool generation is unavailable"))?;
        snapshot
            .validate_against(&generation.descriptor)
            .map_err(|error| AdapterError::rejected(error.to_string()))?;
        Ok(generation.adapter.clone())
    }

    /// Exercises prepared adapter entry without a durable owner in conformance tests.
    #[cfg(any(test, feature = "test-support"))]
    ///
    /// # Errors
    /// Rejects invalid snapshots, closed admission, unavailable generations, and exhausted
    /// permits; propagates adapter preparation, entry, panic, and reporting failures.
    pub fn execute_exact(
        &self,
        snapshot: &ResolvedCapabilitySnapshot,
        request: &InvocationRequest,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), ExecutorError> {
        self.prepare_invocation(snapshot, request, None)?
            .enter(None, reporter)
    }

    /// Exercises contextual prepared entry without a durable owner in adapter tests.
    #[cfg(any(test, feature = "test-support"))]
    ///
    /// # Errors
    /// Rejects invalid exact dispatch or unavailable permits, and propagates adapter preparation,
    /// entry, panic, and reporting failures. Context does not replace durable admission.
    pub fn execute_exact_with_context(
        &self,
        snapshot: &ResolvedCapabilitySnapshot,
        request: &InvocationRequest,
        context: &AdapterExecutionContext,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), ExecutorError> {
        self.prepare_invocation(snapshot, request, Some(context))?
            .enter(Some(context), reporter)
    }

    pub(crate) fn prepare_invocation(
        &self,
        snapshot: &ResolvedCapabilitySnapshot,
        request: &InvocationRequest,
        context: Option<&AdapterExecutionContext>,
    ) -> Result<PreparedHostInvocation, ExecutorError> {
        if let Some(binding) = context.and_then(AdapterExecutionContext::project_binding) {
            self.verify_current_project(binding)?;
        }
        let (adapter, mut permit) = self.acquire(snapshot, request)?;
        let invocation = invocation_view(snapshot, request, context);
        let prepared = match catch_unwind(AssertUnwindSafe(|| adapter.prepare(&invocation))) {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(error)) => {
                permit.mark_failure(error.summary());
                return Err(executor_error_from_adapter(&error));
            }
            Err(_panic) => {
                permit.mark_failure("adapter panicked during local preparation");
                return Err(ExecutorError::AdapterPanicked { after_entry: false });
            }
        };
        Ok(PreparedHostInvocation {
            snapshot: snapshot.clone(),
            request: request.clone(),
            prepared,
            permit,
        })
    }

    /// Routes cancellation to the exact registered generation currently owning an invocation.
    ///
    /// This inherent boundary lets capability adapters such as peer transports reuse host
    /// ownership semantics without depending on the runtime trait that also exposes it.
    ///
    /// # Errors
    /// Returns an error if no retained generation owns the invocation, the registry is unavailable,
    /// the adapter fails or panics, or its acknowledgement names another request.
    pub fn cancel_exact(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        let adapter = {
            let state = self.core.state.lock().map_err(|_error| {
                ExecutorError::BoundaryBeforeEntry("registry unavailable".to_owned())
            })?;
            let key = state
                .in_flight
                .get(request.invocation())
                .or_else(|| state.pending.get(request.invocation()))
                .ok_or_else(|| {
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
        if state.in_flight.contains_key(request.invocation())
            || state.pending.contains_key(request.invocation())
        {
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
        if generation.active >= generation.permit_limit
            || generation
                .pending_limit
                .is_some_and(|limit| generation.active.saturating_add(generation.pending) >= limit)
        {
            return Err(ExecutorError::CapacityUnavailable(format!(
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
                transferred: false,
            },
        ))
    }
}

impl TaskExecutor for CapabilityHost {
    fn task_authoring(
        &self,
        requirement: &CapabilityRequirement,
    ) -> Result<
        Option<(
            milkdrift_capability::CapabilityDescriptor,
            milkdrift_capability::TaskAuthoringContract,
        )>,
        ExecutorError,
    > {
        let Some(capability) = requirement.exact_capability() else {
            return Ok(None);
        };
        let state = self
            .lock_state()
            .map_err(|error| ExecutorError::BoundaryBeforeEntry(error.to_string()))?;
        let Some(revision) = state.current.get(capability) else {
            return Ok(None);
        };
        let key = super::GenerationKey {
            capability: capability.clone(),
            revision: *revision,
        };
        Ok(state.generations.get(&key).and_then(|generation| {
            generation
                .task_authoring
                .get(requirement.operation())
                .map(|contract| (generation.descriptor.clone(), contract.clone()))
        }))
    }
    fn pin_project_paths(
        &self,
        binding: &milkdrift_blueprint::TaskProjectBinding,
    ) -> Result<Vec<milkdrift_blueprint::TaskProjectFolder>, ExecutorError> {
        self.verify_current_project(binding)?;
        crate::native_paths::pin_project_paths(binding)
    }
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
        if let Some(binding) = dispatch.project_binding() {
            self.verify_current_project(binding)?;
        }
        let claims = self.core.native_paths.get();
        let context = AdapterExecutionContext::from_dispatch(dispatch)
            .map_err(|error| ExecutorError::InvalidDispatch(error.to_string()))?;
        let mut native_paths = dispatch.native_path_claims().to_vec();
        if dispatch.is_task() {
            for configured in
                self.declared_native_paths(dispatch.resolution(), dispatch.request(), &context)?
            {
                if let Some(existing) = native_paths
                    .iter_mut()
                    .find(|path| path.path == configured.path)
                {
                    existing.write |= configured.write;
                } else {
                    native_paths.push(configured);
                }
            }
        }
        if dispatch.is_task() && !native_paths.is_empty() {
            claims
                .ok_or_else(|| {
                    ExecutorError::BoundaryBeforeEntry(
                        "native project path owner is not configured".to_owned(),
                    )
                })?
                .acquire(dispatch.attempt().as_str(), &native_paths)?;
        }
        let prepared =
            self.prepare_invocation(dispatch.resolution(), dispatch.request(), Some(&context))?;
        Ok(PreparedExecution::new(
            dispatch,
            prepared.envelope().clone(),
            move |dispatch, reporter| {
                if let Some(binding) = dispatch.project_binding() {
                    self.verify_current_project(binding)?;
                }
                if dispatch.is_task()
                    && !native_paths.is_empty()
                    && let Some(claims) = claims
                {
                    claims.revalidate(dispatch.attempt().as_str())?;
                }
                let bridge = ReporterBridge { reporter };
                let context = AdapterExecutionContext::from_dispatch(dispatch)
                    .map_err(|error| ExecutorError::InvalidDispatch(error.to_string()))?;
                prepared.enter(Some(&context), &bridge)
            },
        ))
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.cancel_exact(request)
    }

    fn settle(&self, dispatch: &ExecutionDispatch) -> Result<(), ExecutorError> {
        if dispatch.is_task()
            && let Some(claims) = self.core.native_paths.get()
        {
            claims.release(dispatch.attempt().as_str())?;
        }
        Ok(())
    }

    fn defer_before_entry(&self, dispatch: &ExecutionDispatch) -> Result<(), ExecutorError> {
        {
            let state = self.core.state.lock().map_err(|_| {
                ExecutorError::BoundaryBeforeEntry(
                    "host registry unavailable during deferral".into(),
                )
            })?;
            let invocation = dispatch.request().invocation();
            if state.in_flight.contains_key(invocation) || state.pending.contains_key(invocation) {
                return Err(ExecutorError::BoundaryBeforeEntry(
                    "prepared or entered invocation ownership remains during deferral".into(),
                ));
            }
        }
        // Runtime owns the exact unentered dispatch until this returns. Its prepared handle
        // has already dropped, releasing adapter resources and the generation permit.
        if dispatch.is_task()
            && let Some(claims) = self.core.native_paths.get()
        {
            claims.release(dispatch.attempt().as_str())?;
        }
        Ok(())
    }
}

pub(crate) struct PreparedHostInvocation {
    snapshot: ResolvedCapabilitySnapshot,
    request: InvocationRequest,
    prepared: PreparedAdapterExecution,
    permit: Permit,
}

impl PreparedHostInvocation {
    pub(crate) fn envelope(&self) -> &milkdrift_capability::InvocationAdmissionEnvelope {
        self.prepared.envelope()
    }

    pub(crate) fn enter(
        mut self,
        context: Option<&AdapterExecutionContext>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), ExecutorError> {
        let invocation = invocation_view(&self.snapshot, &self.request, context);
        match catch_unwind(AssertUnwindSafe(|| {
            self.prepared.enter(&invocation, reporter)
        })) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.permit.mark_failure(error.summary());
                Err(executor_error_from_adapter(&error))
            }
            Err(_panic) => {
                self.permit.mark_failure("adapter panicked");
                Err(ExecutorError::AdapterPanicked { after_entry: true })
            }
        }
    }
}

fn invocation_view<'a>(
    snapshot: &'a ResolvedCapabilitySnapshot,
    request: &'a InvocationRequest,
    context: Option<&'a AdapterExecutionContext>,
) -> AdapterInvocation<'a> {
    match context {
        Some(context) => AdapterInvocation::with_context(snapshot, request, context),
        None => AdapterInvocation::new(snapshot, request),
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

// Live capacity lasts through the prepared closure as well as adapter execution. Dropping an
// unentered preparation after a denied or replayed durable decision must release the same slot.
struct Permit {
    core: Arc<HostCore>,
    key: GenerationKey,
    invocation: InvocationId,
    failure: Option<String>,
    transferred: bool,
}

impl Permit {
    fn mark_failure(&mut self, summary: &str) {
        self.failure = Some(summary.to_owned());
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        if self.transferred {
            return;
        }
        let Ok(mut state) = self.core.state.lock() else {
            return;
        };
        if state.in_flight.remove(&self.invocation).as_ref() != Some(&self.key) {
            return;
        }
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
        AdapterFailureKind::ResponseObservedFailure => {
            ExecutorError::BoundaryAfterResponse(error.summary().to_owned())
        }
    }
}

fn bounded_summary(value: &str) -> String {
    milkdrift_contracts::truncate_utf8(value, 512).to_owned()
}
