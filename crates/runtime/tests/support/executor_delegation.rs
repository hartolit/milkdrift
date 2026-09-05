//! Forward only unchanged test port methods; injected entry behavior stays explicit.

macro_rules! delegate_resolve {
    ($field:ident) => {
        fn resolve(
            &self,
            requirement: &CapabilityRequirement,
            observed_at_unix_ms: u64,
        ) -> Result<ResolvedCapability, ExecutorError> {
            self.$field.resolve(requirement, observed_at_unix_ms)
        }
    };
}
pub(crate) use delegate_resolve;

macro_rules! delegate_cancel {
    ($field:ident) => {
        fn cancel(
            &self,
            request: &CancellationRequest,
        ) -> Result<CancellationAcknowledgement, ExecutorError> {
            self.$field.cancel(request)
        }
    };
}
pub(crate) use delegate_cancel;
