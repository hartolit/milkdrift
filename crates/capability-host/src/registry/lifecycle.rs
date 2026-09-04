use milkdrift_authority::CapabilityAuthorityScope;
use milkdrift_capability::{CapabilityId, CapabilityObservation};

use std::sync::Arc;

use super::{
    CapabilityHost, CatalogGenerationView, GenerationKey, GenerationView, HostError, RegistryState,
    ShutdownReport,
    execution::{generation_health, lifecycle_call, scope_allows_operation, update_current},
};

impl CapabilityHost {
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
        call_every_adapter(adapters, |adapter| adapter.begin_drain())
    }

    /// Gracefully shuts down only after every permit is released.
    pub fn shutdown(&self) -> Result<ShutdownReport, HostError> {
        self.begin_shutdown()?;
        let adapters = {
            let mut state = self.lock_state()?;
            if !state.starting.is_empty() {
                return Err(HostError::RegistrationInProgress(state.starting.len()));
            }
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
        call_every_adapter(adapters, |adapter| adapter.shutdown())?;
        Ok(ShutdownReport {
            unresolved_invocations: Vec::new(),
        })
    }

    /// Forcibly removes every generation and asks each adapter to release live resources.
    ///
    /// Returned invocation identities remain unresolved unless their executing worker
    /// subsequently records terminal evidence. This operation never invents completion.
    pub fn force_shutdown(&self) -> Result<ShutdownReport, HostError> {
        self.begin_shutdown()?;
        let (adapters, unresolved_invocations) = {
            let mut state = self.lock_state()?;
            if !state.starting.is_empty() {
                return Err(HostError::RegistrationInProgress(state.starting.len()));
            }
            let unresolved_invocations = state.in_flight.keys().cloned().collect::<Vec<_>>();
            state.in_flight.clear();
            state.current.clear();
            let adapters = std::mem::take(&mut state.generations)
                .into_values()
                .map(|generation| generation.adapter)
                .collect::<Vec<_>>();
            (adapters, unresolved_invocations)
        };
        call_every_adapter(adapters, |adapter| adapter.shutdown())?;
        Ok(ShutdownReport {
            unresolved_invocations,
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
                generation
                    .descriptor
                    .operations()
                    .iter()
                    .any(|(identity, operation)| {
                        scope_allows_operation(
                            visible,
                            &generation.descriptor,
                            identity,
                            operation.side_effect(),
                        )
                    })
            })
            .map(|(key, generation)| GenerationView {
                capability: key.capability.clone(),
                descriptor_revision: key.revision,
                descriptor_digest: generation.descriptor_digest.clone(),
                category: generation.descriptor.category().clone(),
                operation_contracts: generation
                    .descriptor
                    .operations()
                    .iter()
                    .filter(|(identity, operation)| {
                        scope_allows_operation(
                            visible,
                            &generation.descriptor,
                            identity,
                            operation.side_effect(),
                        )
                    })
                    .map(|(identity, operation)| (identity.clone(), operation.clone()))
                    .collect(),
                provider_profile: generation.descriptor.provider_profile().cloned(),
                locality: generation.descriptor.locality(),
                peer: generation.descriptor.peer().cloned(),
                trust_zones: generation.descriptor.trust_zones().clone(),
                execution_trust: generation.descriptor.execution_trust(),
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

    /// Returns exact descriptor/observation pairs filtered by an explicit authority scope.
    ///
    /// Peer transports use this narrow snapshot to derive a further relationship-filtered,
    /// expiring advertisement. The returned values do not expose adapter handles.
    pub fn catalog_generations(
        &self,
        visible: &CapabilityAuthorityScope,
    ) -> Result<Vec<CatalogGenerationView>, HostError> {
        let state = self.lock_state()?;
        Ok(state
            .generations
            .iter()
            .filter(|(_key, generation)| {
                generation
                    .descriptor
                    .operations()
                    .iter()
                    .any(|(identity, operation)| {
                        scope_allows_operation(
                            visible,
                            &generation.descriptor,
                            identity,
                            operation.side_effect(),
                        )
                    })
            })
            .map(|(key, generation)| CatalogGenerationView {
                descriptor: generation.descriptor.clone(),
                authority_requirements: generation.authority_requirements.clone(),
                observation: generation.observation.clone(),
                current: state.current.get(&key.capability) == Some(&key.revision),
                draining: generation.draining,
            })
            .collect())
    }

    pub(super) fn validate_registration_capacity(
        &self,
        state: &RegistryState,
        key: &GenerationKey,
    ) -> Result<(), HostError> {
        if state.generations.len().saturating_add(state.starting.len())
            >= self.core.config.max_registrations
        {
            return Err(HostError::RegistryBound("total registrations"));
        }
        let generations = state
            .generations
            .keys()
            .chain(state.starting.iter())
            .filter(|existing| existing.capability == key.capability)
            .count();
        if generations >= self.core.config.max_generations_per_capability {
            return Err(HostError::RegistryBound("retained generations"));
        }
        Ok(())
    }

    pub(super) fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, RegistryState>, HostError> {
        self.core
            .state
            .lock()
            .map_err(|_error| HostError::RegistryUnavailable)
    }
}

fn call_every_adapter(
    adapters: Vec<Arc<dyn crate::CapabilityAdapter>>,
    call: impl Fn(&Arc<dyn crate::CapabilityAdapter>) -> Result<(), crate::AdapterError>,
) -> Result<(), HostError> {
    let mut first_failure = None;
    for adapter in adapters {
        if let Err(error) = lifecycle_call(|| call(&adapter)) {
            first_failure.get_or_insert(error);
        }
    }
    first_failure.map_or(Ok(()), Err)
}
