//! Registry residency follows executable obligations; definitions remain in durable storage.
use super::{PublishedWorkflowService, failure, rejected};
use milkdrift_authority::CapabilityAuthorityScope;
use milkdrift_capability::SideEffectClass;
use milkdrift_capability_host::HostError;
use milkdrift_persistence::published::PublishedInvocationPlan;
use milkdrift_runtime::ExecutorError;
use std::sync::Arc;

impl PublishedWorkflowService {
    /// Release idle retired adapters. Call after continuation maintenance and before publishing.
    /// Inspect the bounded registry and hot serving queue, not lifetime publication history.
    /// Durable retirement closes new serving acceptance before the queue check; the registry's
    /// final check also preserves entry permits and pending workflow pins during concurrent work.
    ///
    /// # Errors
    /// Returns clock, storage, or host-drain failures. Generations with serving obligations or live
    /// permits remain retained; a failed call does not prove that a generation was removed.
    pub fn maintain_retirement(&self) -> Result<(), ExecutorError> {
        let now = self.clock.now().map_err(failure)?.get();
        for generation in self
            .host
            .generations(
                &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
                now,
            )
            .map_err(failure)?
        {
            if !self
                .store
                .published_method(&generation.capability, generation.descriptor_revision)
                .map_err(failure)?
                .is_some_and(|record| record.retired)
            {
                continue;
            }
            match self
                .host
                .begin_drain(&generation.capability, generation.descriptor_revision)
            {
                Ok(()) => {}
                Err(HostError::GenerationUnavailable { .. }) => continue,
                Err(error) => return Err(failure(error)),
            }
            if self
                .has_serving_obligation(&generation.capability, generation.descriptor_revision)?
            {
                continue;
            }
            match self
                .host
                .finish_drain(&generation.capability, generation.descriptor_revision)
            {
                Ok(()) | Err(HostError::InFlight(_) | HostError::GenerationUnavailable { .. }) => {}
                Err(error) => return Err(failure(error)),
            }
        }
        Ok(())
    }

    fn has_serving_obligation(
        &self,
        capability: &milkdrift_capability::CapabilityId,
        generation: u64,
    ) -> Result<bool, ExecutorError> {
        let mut cursor = None;
        loop {
            let (records, next) = self
                .store
                .active_serving_page(
                    cursor.as_ref(),
                    milkdrift_persistence::PageSize::new(128).map_err(failure)?,
                )
                .map_err(failure)?;
            if records.iter().any(|record| {
                record.request.selection.capability() == capability
                    && record.request.selection.descriptor_revision() == generation
            }) {
                return Ok(true);
            }
            cursor = next;
            if cursor.is_none() {
                return Ok(false);
            }
        }
    }

    pub(super) fn restore_invocation(
        self: &Arc<Self>,
        plan: &PublishedInvocationPlan,
    ) -> Result<(), ExecutorError> {
        let record = self
            .store
            .published_method(&plan.capability, plan.generation)
            .map_err(failure)?
            .ok_or_else(|| rejected("pending publication definition is absent"))?;
        self.register(&self.host, &record)?;
        self.host.retain_published_invocation(plan)
    }
}
