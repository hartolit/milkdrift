use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
};

use milkdrift_authority::{AuthorityBudget, CapabilityAuthorityScope};
use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor,
    CapabilityDescriptorDocument, CapabilityId, CapabilityObservation, CapabilityRequirement,
    InvocationEvent, InvocationId, InvocationRequest, ResolvedCapabilitySnapshot, SideEffectClass,
};
use milkdrift_runtime::{
    ExecutionDispatch, ExecutionReporter, ExecutorError, ResolvedCapability, TaskExecutor,
};
use thiserror::Error;

use crate::{
    AdapterError, AdapterFailureKind, AdapterInvocation, AdapterReporter, CapabilityAdapter,
};

/// Bounded host configuration; no hidden queue is implemented in this pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostConfig {
    /// Maximum exact registrations across every identity.
    pub max_registrations: usize,
    /// Maximum retained revisions for one capability identity.
    pub max_generations_per_capability: usize,
    /// Host cap applied in addition to descriptor-advertised concurrency.
    pub max_concurrent_per_generation: u32,
    /// Maximum age of a live observation used for new resolution.
    pub observation_stale_after_ms: u64,
}

impl HostConfig {
    /// Validates nonzero defensive bounds.
    pub fn validate(self) -> Result<Self, HostError> {
        if self.max_registrations == 0
            || self.max_generations_per_capability == 0
            || self.max_concurrent_per_generation == 0
            || self.observation_stale_after_ms == 0
        {
            return Err(HostError::InvalidConfig);
        }
        Ok(self)
    }
}

/// Authority and deterministic priority facts applied during live resolution.
#[derive(Clone, Debug)]
pub struct CapabilitySelectionPolicy {
    authority: CapabilityAuthorityScope,
    budget: AuthorityBudget,
    priorities: BTreeMap<CapabilityId, i32>,
}

impl CapabilitySelectionPolicy {
    /// Constructs a policy; absent priorities are zero.
    #[must_use]
    pub fn new(
        authority: CapabilityAuthorityScope,
        budget: AuthorityBudget,
        priorities: BTreeMap<CapabilityId, i32>,
    ) -> Self {
        Self {
            authority,
            budget,
            priorities,
        }
    }

    /// Capability authority constraints.
    #[must_use]
    pub const fn authority(&self) -> &CapabilityAuthorityScope {
        &self.authority
    }
}

/// Result of registering immutable descriptor facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistrationOutcome {
    /// A new exact generation became visible.
    Registered,
    /// Byte-identical descriptor facts were already registered.
    Idempotent,
}

/// Derived health state in an immutable registry read model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationHealth {
    /// No observation has been recorded.
    Unknown,
    /// The latest observation is available and current.
    Healthy,
    /// The latest observation explicitly reports unavailable.
    Unhealthy,
    /// The latest observation is older than the configured policy.
    Stale,
}

/// Immutable bounded generation view for future daemon clients.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationView {
    /// Capability identity.
    pub capability: CapabilityId,
    /// Exact descriptor revision.
    pub descriptor_revision: u64,
    /// Canonical descriptor digest.
    pub descriptor_digest: String,
    /// Whether this is the current resolution generation.
    pub current: bool,
    /// Whether new unpinned resolution is closed.
    pub draining: bool,
    /// Derived current health.
    pub health: GenerationHealth,
    /// Latest observation time, when present.
    pub observed_at_unix_ms: Option<u64>,
    /// Latest adapter availability, when present.
    pub available: Option<bool>,
    /// Actual held permits.
    pub active_permits: u32,
    /// Enforced concurrent permit limit.
    pub permit_limit: u32,
    /// Bounded last failure summary.
    pub last_failure: Option<String>,
}

/// Honest result of forced shutdown when exact invocations remained unresolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShutdownReport {
    /// Invocation identities still owned when adapters were forcibly removed.
    pub unresolved_invocations: Vec<InvocationId>,
}

/// Typed live-host lifecycle and registry failure.
#[derive(Debug, Error)]
pub enum HostError {
    /// Host bounds are zero or inconsistent.
    #[error("invalid capability-host configuration")]
    InvalidConfig,
    /// Registry lock was poisoned by a prior failure.
    #[error("capability registry is unavailable")]
    RegistryUnavailable,
    /// Registration count or retained-generation bound was reached.
    #[error("capability registry bound reached: {0}")]
    RegistryBound(&'static str),
    /// The same identity/revision was reused with different descriptor bytes.
    #[error("conflicting capability descriptor revision {capability}/{descriptor_revision}")]
    ConflictingRevision {
        /// Capability identity.
        capability: CapabilityId,
        /// Reused revision.
        descriptor_revision: u64,
    },
    /// Exact generation is not registered.
    #[error("capability generation {capability}/{descriptor_revision} is unavailable")]
    GenerationUnavailable {
        /// Capability identity.
        capability: CapabilityId,
        /// Descriptor revision.
        descriptor_revision: u64,
    },
    /// Descriptor and observation identities disagree.
    #[error("capability observation does not match its descriptor generation")]
    ObservationMismatch,
    /// Observation time moved backwards.
    #[error("capability observation time moved backwards")]
    ObservationRegressed,
    /// Generation still owns work and cannot be removed gracefully.
    #[error("capability generation still owns {0} in-flight invocation(s)")]
    InFlight(u32),
    /// Adapter lifecycle hook returned a bounded failure.
    #[error(transparent)]
    Adapter(#[from] AdapterError),
    /// Adapter lifecycle hook panicked.
    #[error("adapter lifecycle hook panicked")]
    AdapterPanicked,
    /// Capability descriptor canonicalization failed.
    #[error("invalid capability descriptor: {0}")]
    Descriptor(String),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GenerationKey {
    capability: CapabilityId,
    revision: u64,
}

struct Generation {
    descriptor: CapabilityDescriptor,
    descriptor_digest: String,
    adapter: Arc<dyn CapabilityAdapter>,
    observation: Option<CapabilityObservation>,
    draining: bool,
    active: u32,
    permit_limit: u32,
    last_failure: Option<String>,
}

struct RegistryState {
    admission_open: bool,
    shutdown: bool,
    generations: BTreeMap<GenerationKey, Generation>,
    current: BTreeMap<CapabilityId, u64>,
    in_flight: BTreeMap<InvocationId, GenerationKey>,
}

struct HostCore {
    config: HostConfig,
    policy: CapabilitySelectionPolicy,
    state: Mutex<RegistryState>,
}

/// Embeddable live registry and runtime `TaskExecutor` bridge.
#[derive(Clone)]
pub struct CapabilityHost {
    core: Arc<HostCore>,
}

impl CapabilityHost {
    /// Constructs an empty host with admission open.
    pub fn new(config: HostConfig, policy: CapabilitySelectionPolicy) -> Result<Self, HostError> {
        let config = config.validate()?;
        Ok(Self {
            core: Arc::new(HostCore {
                config,
                policy,
                state: Mutex::new(RegistryState {
                    admission_open: true,
                    shutdown: false,
                    generations: BTreeMap::new(),
                    current: BTreeMap::new(),
                    in_flight: BTreeMap::new(),
                }),
            }),
        })
    }

    /// Registers one immutable descriptor generation and starts its adapter.
    pub fn register(
        &self,
        descriptor: CapabilityDescriptor,
        adapter: Arc<dyn CapabilityAdapter>,
        observation: Option<CapabilityObservation>,
    ) -> Result<RegistrationOutcome, HostError> {
        if observation
            .as_ref()
            .is_some_and(|value| value.capability() != descriptor.identity())
        {
            return Err(HostError::ObservationMismatch);
        }
        let key = GenerationKey {
            capability: descriptor.identity().clone(),
            revision: descriptor.descriptor_revision(),
        };
        {
            let state = self.lock_state()?;
            if let Some(existing) = state.generations.get(&key) {
                return if existing.descriptor == descriptor {
                    Ok(RegistrationOutcome::Idempotent)
                } else {
                    Err(HostError::ConflictingRevision {
                        capability: key.capability,
                        descriptor_revision: key.revision,
                    })
                };
            }
            self.validate_registration_capacity(&state, &key)?;
        }
        let bytes = CapabilityDescriptorDocument::new(descriptor.clone())
            .to_canonical_json()
            .map_err(|error| HostError::Descriptor(error.to_string()))?;
        let descriptor_digest = format!("b3_{}", blake3::hash(&bytes));
        let permit_limit = descriptor
            .admission()
            .max_concurrent()
            .min(self.core.config.max_concurrent_per_generation);
        if let Err(error) = lifecycle_call(|| adapter.start()) {
            let _ = lifecycle_call(|| adapter.shutdown());
            return Err(error);
        }
        let mut state = match self.lock_state() {
            Ok(state) => state,
            Err(error) => {
                let _ = lifecycle_call(|| adapter.shutdown());
                return Err(error);
            }
        };
        if let Some(existing) = state.generations.get(&key) {
            let _ = lifecycle_call(|| adapter.shutdown());
            return if existing.descriptor == descriptor {
                Ok(RegistrationOutcome::Idempotent)
            } else {
                Err(HostError::ConflictingRevision {
                    capability: key.capability,
                    descriptor_revision: key.revision,
                })
            };
        }
        if let Err(error) = self.validate_registration_capacity(&state, &key) {
            drop(state);
            let _ = lifecycle_call(|| adapter.shutdown());
            return Err(error);
        }
        state.generations.insert(
            key.clone(),
            Generation {
                descriptor,
                descriptor_digest,
                adapter,
                observation,
                draining: false,
                active: 0,
                permit_limit,
                last_failure: None,
            },
        );
        update_current(&mut state, &key.capability);
        Ok(RegistrationOutcome::Registered)
    }

    /// Replaces the live observation for one exact generation.
    pub fn update_observation(
        &self,
        capability: &CapabilityId,
        descriptor_revision: u64,
        observation: CapabilityObservation,
    ) -> Result<(), HostError> {
        if observation.capability() != capability {
            return Err(HostError::ObservationMismatch);
        }
        let mut state = self.lock_state()?;
        let generation = state
            .generations
            .get_mut(&GenerationKey {
                capability: capability.clone(),
                revision: descriptor_revision,
            })
            .ok_or_else(|| HostError::GenerationUnavailable {
                capability: capability.clone(),
                descriptor_revision,
            })?;
        if generation
            .observation
            .as_ref()
            .is_some_and(|prior| prior.observed_at_unix_ms() > observation.observed_at_unix_ms())
        {
            return Err(HostError::ObservationRegressed);
        }
        generation.observation = Some(observation);
        Ok(())
    }

    /// Pulls one bounded health observation from the exact adapter generation.
    pub fn refresh_health(
        &self,
        capability: &CapabilityId,
        descriptor_revision: u64,
        observed_at_unix_ms: u64,
    ) -> Result<(), HostError> {
        let adapter = {
            let state = self.lock_state()?;
            state
                .generations
                .get(&GenerationKey {
                    capability: capability.clone(),
                    revision: descriptor_revision,
                })
                .ok_or_else(|| HostError::GenerationUnavailable {
                    capability: capability.clone(),
                    descriptor_revision,
                })?
                .adapter
                .clone()
        };
        let observation = lifecycle_call(|| adapter.health(observed_at_unix_ms))?;
        self.update_observation(capability, descriptor_revision, observation)
    }

    /// Resolves against one mutex-protected registry snapshot and explicit boundary time.
    pub fn resolve_at(
        &self,
        requirement: &CapabilityRequirement,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        let state = self.core.state.lock().map_err(|_error| {
            ExecutorError::BoundaryBeforeEntry("registry unavailable".to_owned())
        })?;
        if !state.admission_open || state.shutdown {
            return Err(ExecutorError::AdmissionClosed);
        }
        let mut candidates = Vec::new();
        let mut semantic_match = false;
        let mut authority_match = false;
        let mut availability_match = false;
        let mut capacity_match = false;
        let mut mismatch_reasons = BTreeSet::new();
        for (key, generation) in &state.generations {
            if state.current.get(&key.capability) != Some(&key.revision) || generation.draining {
                continue;
            }
            let matched = generation.descriptor.matches(requirement);
            if !matched.is_match() {
                mismatch_reasons.extend(matched.mismatch_reasons().iter().cloned());
                continue;
            }
            semantic_match = true;
            let Some(contract) = generation.descriptor.operation(requirement.operation()) else {
                continue;
            };
            if !policy_allows(
                &self.core.policy,
                &generation.descriptor,
                requirement.operation(),
                contract.side_effect(),
            ) {
                continue;
            }
            authority_match = true;
            if !observation_available(
                generation.observation.as_ref(),
                observed_at_unix_ms,
                self.core.config.observation_stale_after_ms,
            ) {
                continue;
            }
            availability_match = true;
            if generation.active >= generation.permit_limit {
                continue;
            }
            capacity_match = true;
            candidates.push((
                self.core
                    .policy
                    .priorities
                    .get(&key.capability)
                    .copied()
                    .unwrap_or(0),
                key.clone(),
                generation.descriptor.clone(),
            ));
        }
        if candidates.is_empty() {
            if !semantic_match {
                return Err(ExecutorError::ResolutionMismatch {
                    reasons: mismatch_reasons.into_iter().collect(),
                });
            }
            if !authority_match {
                return Err(ExecutorError::AuthorityDenied(
                    "no semantically matching descriptor is within the configured grant scope"
                        .to_owned(),
                ));
            }
            if !availability_match {
                return Err(ExecutorError::Unavailable(
                    "matching generations are unavailable or stale".to_owned(),
                ));
            }
            if !capacity_match {
                return Err(ExecutorError::Overloaded(
                    "matching generations have no immediate permit".to_owned(),
                ));
            }
        }
        candidates.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.capability.cmp(&right.1.capability))
                .then_with(|| right.1.revision.cmp(&left.1.revision))
        });
        let (_priority, _key, descriptor) = candidates
            .into_iter()
            .next()
            .ok_or_else(|| ExecutorError::BoundaryBeforeEntry("candidate vanished".to_owned()))?;
        let snapshot =
            ResolvedCapabilitySnapshot::from_descriptor(&descriptor, requirement.operation())?;
        ResolvedCapability::new(descriptor, snapshot)
    }

    /// Marks an exact generation draining and removes it from new resolution.
    pub fn begin_drain(
        &self,
        capability: &CapabilityId,
        descriptor_revision: u64,
    ) -> Result<(), HostError> {
        let key = GenerationKey {
            capability: capability.clone(),
            revision: descriptor_revision,
        };
        let adapter = {
            let mut state = self.lock_state()?;
            let generation = state.generations.get_mut(&key).ok_or_else(|| {
                HostError::GenerationUnavailable {
                    capability: capability.clone(),
                    descriptor_revision,
                }
            })?;
            generation.draining = true;
            let adapter = generation.adapter.clone();
            update_current(&mut state, capability);
            adapter
        };
        lifecycle_call(|| adapter.begin_drain())
    }

    /// Removes a drained generation only after every exact owner released its permit.
    pub fn finish_drain(
        &self,
        capability: &CapabilityId,
        descriptor_revision: u64,
    ) -> Result<(), HostError> {
        let key = GenerationKey {
            capability: capability.clone(),
            revision: descriptor_revision,
        };
        let adapter =
            {
                let mut state = self.lock_state()?;
                let generation = state.generations.get(&key).ok_or_else(|| {
                    HostError::GenerationUnavailable {
                        capability: capability.clone(),
                        descriptor_revision,
                    }
                })?;
                if !generation.draining {
                    return Err(HostError::Descriptor(
                        "generation must be draining before removal".to_owned(),
                    ));
                }
                if generation.active != 0 {
                    return Err(HostError::InFlight(generation.active));
                }
                let generation = state.generations.remove(&key).ok_or_else(|| {
                    HostError::GenerationUnavailable {
                        capability: capability.clone(),
                        descriptor_revision,
                    }
                })?;
                update_current(&mut state, capability);
                generation.adapter
            };
        lifecycle_call(|| adapter.shutdown())
    }

    /// Forcibly removes an exact generation and leaves later exact dispatch unavailable.
    pub fn force_remove(
        &self,
        capability: &CapabilityId,
        descriptor_revision: u64,
    ) -> Result<ShutdownReport, HostError> {
        let key = GenerationKey {
            capability: capability.clone(),
            revision: descriptor_revision,
        };
        let (adapter, unresolved) =
            {
                let mut state = self.lock_state()?;
                let generation = state.generations.remove(&key).ok_or_else(|| {
                    HostError::GenerationUnavailable {
                        capability: capability.clone(),
                        descriptor_revision,
                    }
                })?;
                let unresolved = state
                    .in_flight
                    .iter()
                    .filter(|(_invocation, owner)| *owner == &key)
                    .map(|(invocation, _owner)| invocation.clone())
                    .collect::<Vec<_>>();
                state.in_flight.retain(|_invocation, owner| owner != &key);
                update_current(&mut state, capability);
                (generation.adapter, unresolved)
            };
        lifecycle_call(|| adapter.shutdown())?;
        Ok(ShutdownReport {
            unresolved_invocations: unresolved,
        })
    }

    /// Closes admission and marks every generation draining.
    pub fn begin_shutdown(&self) -> Result<(), HostError> {
        let adapters = {
            let mut state = self.lock_state()?;
            if state.shutdown {
                return Ok(());
            }
            state.admission_open = false;
            state.shutdown = true;
            state.current.clear();
            state
                .generations
                .values_mut()
                .map(|generation| {
                    generation.draining = true;
                    generation.adapter.clone()
                })
                .collect::<Vec<_>>()
        };
        for adapter in adapters {
            lifecycle_call(|| adapter.begin_drain())?;
        }
        Ok(())
    }

    /// Gracefully shuts down only after every permit is released.
    pub fn shutdown(&self) -> Result<ShutdownReport, HostError> {
        self.begin_shutdown()?;
        let adapters = {
            let mut state = self.lock_state()?;
            let active = state
                .generations
                .values()
                .map(|generation| generation.active)
                .sum::<u32>();
            if active != 0 {
                return Err(HostError::InFlight(active));
            }
            state.in_flight.clear();
            std::mem::take(&mut state.generations)
                .into_values()
                .map(|generation| generation.adapter)
                .collect::<Vec<_>>()
        };
        for adapter in adapters {
            lifecycle_call(|| adapter.shutdown())?;
        }
        Ok(ShutdownReport {
            unresolved_invocations: Vec::new(),
        })
    }

    /// Returns a stable sorted generation view, filtered by authority scope.
    pub fn generations(
        &self,
        visible: &CapabilityAuthorityScope,
        observed_at_unix_ms: u64,
    ) -> Result<Vec<GenerationView>, HostError> {
        let state = self.lock_state()?;
        Ok(state
            .generations
            .iter()
            .filter(|(_key, generation)| {
                scope_allows(visible, &generation.descriptor, SideEffectClass::None)
            })
            .map(|(key, generation)| GenerationView {
                capability: key.capability.clone(),
                descriptor_revision: key.revision,
                descriptor_digest: generation.descriptor_digest.clone(),
                current: state.current.get(&key.capability) == Some(&key.revision),
                draining: generation.draining,
                health: generation_health(
                    generation.observation.as_ref(),
                    observed_at_unix_ms,
                    self.core.config.observation_stale_after_ms,
                ),
                observed_at_unix_ms: generation
                    .observation
                    .as_ref()
                    .map(CapabilityObservation::observed_at_unix_ms),
                available: generation
                    .observation
                    .as_ref()
                    .map(CapabilityObservation::available),
                active_permits: generation.active,
                permit_limit: generation.permit_limit,
                last_failure: generation.last_failure.clone(),
            })
            .collect())
    }

    fn validate_registration_capacity(
        &self,
        state: &RegistryState,
        key: &GenerationKey,
    ) -> Result<(), HostError> {
        if state.generations.len() >= self.core.config.max_registrations {
            return Err(HostError::RegistryBound("total registrations"));
        }
        let generations = state
            .generations
            .keys()
            .filter(|existing| existing.capability == key.capability)
            .count();
        if generations >= self.core.config.max_generations_per_capability {
            return Err(HostError::RegistryBound("retained generations"));
        }
        Ok(())
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, RegistryState>, HostError> {
        self.core
            .state
            .lock()
            .map_err(|_error| HostError::RegistryUnavailable)
    }

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

    fn acquire(
        &self,
        snapshot: &ResolvedCapabilitySnapshot,
        request: &InvocationRequest,
    ) -> Result<(Arc<dyn CapabilityAdapter>, Permit), ExecutorError> {
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
        requirement: &CapabilityRequirement,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolve_at(requirement, observed_at_unix_ms)
    }

    fn execute_streaming(
        &self,
        dispatch: &ExecutionDispatch,
        reporter: &dyn ExecutionReporter,
    ) -> Result<(), ExecutorError> {
        let bridge = ReporterBridge { reporter };
        self.execute_exact(dispatch.resolution(), dispatch.request(), &bridge)
    }

    fn cancel(
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
            Ok(Ok(acknowledgement)) => Ok(acknowledgement),
            Ok(Err(error)) => Err(executor_error_from_adapter(&error)),
            Err(_panic) => Err(ExecutorError::AdapterPanicked { after_entry: true }),
        }
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

fn update_current(state: &mut RegistryState, capability: &CapabilityId) {
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

fn lifecycle_call<T>(call: impl FnOnce() -> Result<T, AdapterError>) -> Result<T, HostError> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(result) => result.map_err(HostError::Adapter),
        Err(_panic) => Err(HostError::AdapterPanicked),
    }
}

fn generation_health(
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

fn observation_available(
    observation: Option<&CapabilityObservation>,
    now: u64,
    stale_after: u64,
) -> bool {
    matches!(
        generation_health(observation, now, stale_after),
        GenerationHealth::Healthy
    )
}

fn policy_allows(
    policy: &CapabilitySelectionPolicy,
    descriptor: &CapabilityDescriptor,
    operation: &milkdrift_capability::OperationId,
    side_effect: SideEffectClass,
) -> bool {
    if !scope_allows(&policy.authority, descriptor, side_effect) {
        return false;
    }
    if !policy.authority.operations().is_empty()
        && !policy.authority.operations().contains(operation)
    {
        return false;
    }
    let observations = descriptor.resource_observations();
    if let Some(ceiling) = policy.budget.duration_ms
        && observations
            .and_then(|value| value.estimated_duration_ms())
            .is_some_and(|estimate| estimate > ceiling)
    {
        return false;
    }
    if let Some(ceiling_minor) = policy.budget.cost_minor {
        let ceiling_micros = ceiling_minor.saturating_mul(10_000);
        if observations
            .and_then(|value| value.estimated_cost_micros())
            .is_some_and(|estimate| estimate > ceiling_micros)
        {
            return false;
        }
    }
    if let Some(concurrency) = policy.budget.concurrency
        && descriptor.admission().max_concurrent() > concurrency
    {
        return false;
    }
    true
}

fn scope_allows(
    scope: &CapabilityAuthorityScope,
    descriptor: &CapabilityDescriptor,
    side_effect: SideEffectClass,
) -> bool {
    (scope.identities().is_empty() || scope.identities().contains(descriptor.identity()))
        && (scope.categories().is_empty() || scope.categories().contains(descriptor.category()))
        && (scope.operations().is_empty()
            || descriptor
                .operations()
                .keys()
                .any(|operation| scope.operations().contains(operation)))
        && (scope.provider_profiles().is_empty()
            || descriptor
                .provider_profile()
                .is_some_and(|profile| scope.provider_profiles().contains(profile)))
        && (scope.localities().is_empty() || scope.localities().contains(&descriptor.locality()))
        && (scope.trust_zones().is_empty()
            || descriptor
                .trust_zones()
                .iter()
                .any(|zone| scope.trust_zones().contains(zone)))
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
    if value.len() <= 512 {
        return value.to_owned();
    }
    let mut end = 512;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
