//! Translate one immutable published allowance into the exact delegated request ceiling.
use super::RemoteCapabilityAdapter;
use milkdrift_capability::InvocationAdmissionEnvelope;
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
        if let Some(value) = self
            .remote_descriptor
            .extensions()
            .iter()
            .find(|(key, _)| key.as_str() == milkdrift_capability::PUBLISHED_ALLOWANCE_EXTENSION)
            .map(|(_, value)| value)
        {
            let envelope: InvocationAdmissionEnvelope =
                serde_json::from_value(value.value().clone())
                    .map_err(|error| AdapterError::rejected(error.to_string()))?;
            limits.input_units = envelope.input_units().bounded().copied();
            limits.output_units = envelope.output_units().bounded().copied();
            limits.nested_invocations = envelope.nested_invocations();
            if let Some(money) = envelope.monetary_cost().bounded() {
                limits.cost_micros = money.maximum_micros();
                limits.cost_currency = Some(money.currency().to_owned());
            } else {
                limits.cost_micros = 0;
                limits.cost_currency = None;
            }
            if limits.nested_invocations.is_none()
                || !self.relationship.execution_limits.contains(&limits)
                || !limits.permits_prepared(&envelope, 0)
            {
                return Err(AdapterError::rejected(
                    "published allowance exceeds the configured peer relationship",
                ));
            }
        } else {
            limits.nested_invocations = None;
        }
        Ok(limits)
    }
}
