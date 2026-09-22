//! Preparation shared by serving acceptance and the runtime executor bridge.

use milkdrift_capability::{IdempotencyKey, InvocationEvent, InvocationId, InvocationRequest};
use milkdrift_persistence::{PeerExecutionPhase, PeerExecutionRecord};
use milkdrift_runtime::ExecutorError;

use crate::registry::PreparedHostInvocation;
use crate::{AdapterError, AdapterExecutionContext, AdapterReporter, CapabilityHost};

/// Holds an exact generation while its serving owner commits the final entry decision.
/// Dropping it before entry releases the permit and all prepared data without an effect.
pub(crate) struct PreparedServingExecution {
    accepted: PeerExecutionRecord,
    context: AdapterExecutionContext,
    prepared: PreparedHostInvocation,
}

impl CapabilityHost {
    /// Prepares a claimed operation without crossing its external boundary.
    /// The caller must recheck current authorization and commit entry after this returns.
    pub(crate) fn prepare_serving_execution(
        &self,
        accepted: &PeerExecutionRecord,
        context: AdapterExecutionContext,
    ) -> Result<PreparedServingExecution, ExecutorError> {
        if !matches!(accepted.phase, PeerExecutionPhase::DispatchClaimed { .. }) {
            return Err(ExecutorError::InvalidDispatch(
                "serving preparation requires an exact pre-entry claim".to_owned(),
            ));
        }
        let request = serving_request(accepted)?;
        let prepared =
            self.prepare_invocation(&accepted.request.selection, &request, Some(&context))?;
        let input_bytes = accepted
            .request
            .input_artifact_bytes()
            .map_err(|error| ExecutorError::InvalidDispatch(error.to_string()))?;
        if !accepted
            .request
            .limits
            .permits_prepared(prepared.envelope(), input_bytes)
        {
            return Err(ExecutorError::InvalidDispatch(
                "prepared resources are unknown or exceed the accepted serving allowance"
                    .to_owned(),
            ));
        }
        Ok(PreparedServingExecution {
            accepted: accepted.clone(),
            context,
            prepared,
        })
    }
}

impl PreparedServingExecution {
    /// Consumes prepared bytes only for the exact claim's committed entry record.
    pub(crate) fn enter(
        self,
        entered: &PeerExecutionRecord,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), ExecutorError> {
        if entered.caller != self.accepted.caller
            || entered.execution != self.accepted.execution
            || entered.request != self.accepted.request
            || entered.phase.claim() != self.accepted.phase.claim()
            || !matches!(entered.phase, PeerExecutionPhase::Entered { .. })
        {
            return Err(ExecutorError::InvalidDispatch(
                "serving entry does not match its prepared acceptance and claim".to_owned(),
            ));
        }
        let context = self
            .context
            .with_serving_execution(entered)
            .map_err(|error| ExecutorError::InvalidDispatch(error.to_string()))?;
        let bridge = ServingReporter {
            local: serving_invocation(entered)?,
            original: entered.request.request.invocation(),
            reporter,
        };
        self.prepared.enter(Some(&context), &bridge)
    }
}

/// Caller-controlled invocation names never enter the local registry or adapter namespace.
/// The persisted acceptance remains unchanged; the same acceptance recreates this identity.
pub(crate) fn serving_invocation(
    record: &PeerExecutionRecord,
) -> Result<InvocationId, ExecutorError> {
    record
        .managed_invocation()
        .map_err(|error| ExecutorError::InvalidDispatch(error.to_string()))
}

fn serving_request(record: &PeerExecutionRecord) -> Result<InvocationRequest, ExecutorError> {
    let original = &record.request.request;
    let identity = serving_invocation(record)?;
    let key = original
        .idempotency_key()
        .map(|_| IdempotencyKey::new(identity.as_str()))
        .transpose()
        .map_err(|error| ExecutorError::InvalidDispatch(error.to_string()))?;
    let mut request = InvocationRequest::new(
        identity,
        original.capability().clone(),
        original.operation().clone(),
        original.provider_profile().cloned(),
        key,
        original.inputs().to_vec(),
        original.extensions().clone(),
    )
    .map_err(|error| ExecutorError::InvalidDispatch(error.to_string()))?;
    if let Some(manifest) = original.context_manifest() {
        request = request
            .with_context_manifest(manifest.clone())
            .map_err(|error| ExecutorError::InvalidDispatch(error.to_string()))?;
    }
    Ok(request)
}

struct ServingReporter<'a> {
    local: InvocationId,
    original: &'a InvocationId,
    reporter: &'a dyn AdapterReporter,
}

impl AdapterReporter for ServingReporter<'_> {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        if event.invocation() != &self.local {
            return Err(AdapterError::external_failure(
                "adapter reported another serving invocation",
            ));
        }
        let event = InvocationEvent::new(
            self.original.clone(),
            event.sequence(),
            event.kind().clone(),
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        self.reporter.invocation(event)
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        self.reporter.heartbeat()
    }
}
