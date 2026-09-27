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
    /// The bounded registry is inspected, not the lifetime publication history. The registry's
    /// final check preserves both entry permits and pending workflow pins during concurrent work.
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
            self.host
                .begin_drain(&generation.capability, generation.descriptor_revision)
                .map_err(failure)?;
            match self
                .host
                .finish_drain(&generation.capability, generation.descriptor_revision)
            {
                Ok(()) | Err(HostError::InFlight(_)) => {}
                Err(error) => return Err(failure(error)),
            }
        }
        Ok(())
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
