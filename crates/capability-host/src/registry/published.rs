use super::{CapabilityHost, GenerationKey, HostError};
use crate::PublishedWorkflowContinuation;
use milkdrift_capability::InvocationEvent;
use milkdrift_persistence::published::PublishedInvocationPlan;
use milkdrift_runtime::ExecutorError;
use std::sync::{Arc, Weak};
impl CapabilityHost {
    pub(crate) fn release_published_serving_owner(&self, owner: &crate::PeerService) {
        if let Ok(mut slot) = self.core.published_serving.lock()
            && slot
                .as_ref()
                .is_some_and(|current| std::ptr::eq(current.as_ptr(), owner))
        {
            *slot = None;
        }
    }

    pub(crate) fn install_published_serving_owner(
        &self,
        owner: &Arc<crate::PeerService>,
    ) -> Result<(), HostError> {
        let mut slot = self
            .core
            .published_serving
            .lock()
            .map_err(|_| HostError::RegistryUnavailable)?;
        if slot.as_ref().and_then(Weak::upgrade).is_some() {
            return Err(HostError::RegistryUnavailable);
        }
        *slot = Some(Arc::downgrade(owner));
        Ok(())
    }

    /// Install the workflow-enabled composition's owner before accepting published calls.
    pub fn install_published_continuation(
        &self,
        owner: &Arc<dyn PublishedWorkflowContinuation>,
    ) -> Result<(), HostError> {
        let mut slot = self
            .core
            .published
            .lock()
            .map_err(|_| HostError::RegistryUnavailable)?;
        if slot.as_ref().and_then(Weak::upgrade).is_some() {
            return Err(HostError::RegistryUnavailable);
        }
        *slot = Some(Arc::downgrade(owner));
        Ok(())
    }

    pub(crate) fn continue_published_invocation(
        &self,
        plan: &PublishedInvocationPlan,
        cancel: bool,
        next_sequence: u64,
    ) -> Result<Option<InvocationEvent>, ExecutorError> {
        let owner = self
            .core
            .published
            .lock()
            .map_err(|_| {
                ExecutorError::Unavailable("published continuation lock unavailable".to_owned())
            })?
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or_else(|| {
                ExecutorError::Unavailable(
                    "workflow-enabled publication owner is unavailable".to_owned(),
                )
            })?;
        self.retain_published_invocation(plan)?;
        owner.continue_invocation(plan, cancel, next_sequence)
    }
    /// Restore a saved invocation's lifetime pin before reopening admission. The publication's
    /// immutable pending bound is independent of live worker/entry capacity.
    pub fn retain_published_invocation(
        &self,
        plan: &PublishedInvocationPlan,
    ) -> Result<(), ExecutorError> {
        self.retain_published_pin(plan, true)
    }

    fn retain_published_pin(
        &self,
        plan: &PublishedInvocationPlan,
        restore: bool,
    ) -> Result<(), ExecutorError> {
        let mut state = self
            .lock_state()
            .map_err(|e| ExecutorError::Unavailable(e.to_string()))?;
        let key = GenerationKey {
            capability: plan.capability.clone(),
            revision: plan.generation,
        };
        if let Some(existing) = state.pending.get(&plan.invocation) {
            return if existing == &key {
                Ok(())
            } else {
                Err(ExecutorError::InvalidDispatch(
                    "pending invocation belongs to another generation".to_owned(),
                ))
            };
        }
        if state
            .in_flight
            .get(&plan.invocation)
            .is_some_and(|existing| existing != &key)
        {
            return Err(ExecutorError::InvalidDispatch(
                "entry invocation belongs to another generation".to_owned(),
            ));
        }
        let entered = state.in_flight.contains_key(&plan.invocation);
        if !restore && !entered {
            return Ok(());
        }
        let generation = state.generations.get_mut(&key).ok_or_else(|| {
            ExecutorError::UnavailableGeneration {
                capability: plan.capability.clone(),
                descriptor_revision: plan.generation,
            }
        })?;
        if generation
            .pending_limit
            .is_none_or(|limit| generation.pending >= limit)
        {
            return Err(ExecutorError::Overloaded(
                "durable workflow continuation bound reached".to_owned(),
            ));
        }
        generation.pending += 1;
        if entered {
            generation.active = generation.active.checked_sub(1).ok_or_else(|| {
                ExecutorError::Boundary("entry permit count underflow".to_owned())
            })?;
            state.in_flight.remove(&plan.invocation);
        }
        state.pending.insert(plan.invocation.clone(), key);
        Ok(())
    }

    pub(crate) fn transfer_published_invocation(
        &self,
        plan: &PublishedInvocationPlan,
    ) -> Result<(), ExecutorError> {
        // The observer may already have transferred and settled this exact entry while its
        // originating worker was returning. An absent entry must not resurrect a completed pin.
        self.retain_published_pin(plan, false)
    }

    pub(crate) fn release_published_invocation(
        &self,
        plan: &PublishedInvocationPlan,
    ) -> Result<(), ExecutorError> {
        let mut state = self
            .lock_state()
            .map_err(|e| ExecutorError::Unavailable(e.to_string()))?;
        let key = GenerationKey {
            capability: plan.capability.clone(),
            revision: plan.generation,
        };
        match state.pending.get(&plan.invocation) {
            None => return Ok(()),
            Some(existing) if existing == &key => {}
            Some(_) => {
                return Err(ExecutorError::InvalidDispatch(
                    "terminal invocation belongs to another generation".to_owned(),
                ));
            }
        }
        state.pending.remove(&plan.invocation);
        if let Some(generation) = state.generations.get_mut(&key) {
            generation.pending = generation.pending.checked_sub(1).ok_or_else(|| {
                ExecutorError::Boundary("pending ownership count underflow".to_owned())
            })?;
        }
        Ok(())
    }
}
