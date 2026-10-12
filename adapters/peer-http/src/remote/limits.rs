//! Narrow a configured relationship to this exact external capability.
use super::RemoteCapabilityAdapter;
use milkdrift_capability_host::AdapterError;

impl RemoteCapabilityAdapter {
    pub(super) fn request_limits(
        &self,
    ) -> Result<milkdrift_peer_protocol::ExecutionLimits, AdapterError> {
        let mut limits = self.relationship.execution_limits.clone();
        if self.remote_descriptor.category() == &milkdrift_capability::CapabilityCategory::Process {
            // Native process admission does not qualify model billing or logical tokens.
            // Forbid those dimensions in the exact delegated call; serving preparation
            // must confirm they are inapplicable before any process can enter.
            limits.input_units = None;
            limits.output_units = None;
            limits.cost_micros = 0;
            limits.cost_currency = None;
        }
        limits.nested_invocations = None;
        Ok(limits)
    }
}
