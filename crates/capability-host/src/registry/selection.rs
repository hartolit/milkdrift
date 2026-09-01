use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use milkdrift_authority::{AuthorityDecisionSnapshot, AuthorityEvaluator};
use milkdrift_capability::{
    CapabilityDescriptor, CapabilityDescriptorDocument, CapabilityId, CapabilityObservation,
    CapabilityRequirement, ResolvedCapabilitySnapshot,
};
use milkdrift_runtime::{CapabilityResolutionContext, ExecutorError, ResolvedCapability};

use super::{
    CapabilityHost, CapabilitySelectionPolicy, Generation, GenerationKey, HostConfig, HostCore,
    HostError, RegistrationOutcome, RegistryState,
    execution::{lifecycle_call, observation_available, update_current},
};
use crate::CapabilityAdapter;

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
        let authority_requirements = adapter.authority_requirements();
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
                authority_requirements,
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
    pub fn resolve_authorized_at(
        &self,
        requirement: &CapabilityRequirement,
        authority: &CapabilityResolutionContext,
        evaluator: &dyn AuthorityEvaluator,
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
        let mut denied_decision: Option<AuthorityDecisionSnapshot> = None;
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
            let Some(_contract) = generation.descriptor.operation(requirement.operation()) else {
                continue;
            };
            let request = authority.candidate_request(
                &generation.descriptor,
                requirement.operation(),
                &generation.authority_requirements,
                observed_at_unix_ms,
                "resolve",
            )?;
            let decision = evaluator
                .evaluate(&request)
                .map_err(|error| ExecutorError::BoundaryBeforeEntry(error.to_string()))?;
            if !decision.is_allowed() {
                denied_decision.get_or_insert(decision);
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
                decision,
            ));
        }
        if candidates.is_empty() {
            if !semantic_match {
                return Err(ExecutorError::ResolutionMismatch {
                    reasons: mismatch_reasons.into_iter().collect(),
                });
            }
            if !authority_match {
                let decision = denied_decision.ok_or_else(|| {
                    ExecutorError::BoundaryBeforeEntry(
                        "authority denial had no exact candidate decision".to_owned(),
                    )
                })?;
                return Err(ExecutorError::AuthorityDenied {
                    reasons: decision.reason_codes().to_vec(),
                    decision: Box::new(decision),
                });
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
        let (_priority, _key, descriptor, authorization) = candidates
            .into_iter()
            .next()
            .ok_or_else(|| ExecutorError::BoundaryBeforeEntry("candidate vanished".to_owned()))?;
        let snapshot =
            ResolvedCapabilitySnapshot::from_descriptor(&descriptor, requirement.operation())?;
        ResolvedCapability::new_authorized(descriptor, snapshot, authorization)
    }

    /// Resolves semantic/health/capacity state for registry inspection only.
    ///
    /// The returned value has no authority decision and cannot be dispatched by the runtime.
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
        let mut mismatch_reasons = BTreeSet::new();
        let mut semantic_match = false;
        let mut availability_match = false;
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
            if !availability_match {
                return Err(ExecutorError::Unavailable(
                    "matching generations are unavailable or stale".to_owned(),
                ));
            }
            return Err(ExecutorError::Overloaded(
                "matching generations have no immediate permit".to_owned(),
            ));
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
}
