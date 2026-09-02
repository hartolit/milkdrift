use milkdrift_authority::{
    AuthorityDecisionSnapshot, CapabilityExecutionRequirements, ExecutionAuthorityBasis,
};
use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, CapabilityObservation,
    InvocationAdmissionEnvelope, InvocationEvent, InvocationRequest, ResolvedCapabilitySnapshot,
};
use milkdrift_persistence::{AttemptId, ControllerReservationId, NodeExecutionId};
use milkdrift_workspace::RunId;
use thiserror::Error;

/// Stable class of a bounded adapter failure summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterFailureKind {
    /// Adapter rejected the exact request before starting provider work.
    Rejected,
    /// Adapter's external dependency was unavailable.
    Unavailable,
    /// Adapter entered its external boundary and could not prove an outcome.
    ExternalFailure,
}

/// Bounded adapter-owned error that contains no database or provider-client type.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("adapter {kind:?}: {summary}")]
pub struct AdapterError {
    kind: AdapterFailureKind,
    summary: String,
}

impl AdapterError {
    /// Constructs a failure with a summary limited to 512 bytes.
    pub fn new(
        kind: AdapterFailureKind,
        summary: impl Into<String>,
    ) -> Result<Self, HostAdapterContractError> {
        let summary = summary.into();
        if summary.is_empty() || summary.len() > 512 {
            return Err(HostAdapterContractError);
        }
        Ok(Self { kind, summary })
    }

    /// Stable failure class.
    #[must_use]
    pub const fn kind(&self) -> AdapterFailureKind {
        self.kind
    }

    /// Bounded non-secret summary.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Constructs a bounded deterministic pre-provider rejection.
    #[must_use]
    pub fn rejected(summary: impl Into<String>) -> Self {
        Self::bounded(
            AdapterFailureKind::Rejected,
            summary,
            "adapter rejected request",
        )
    }

    /// Constructs a bounded failure proving provider work was not entered.
    #[must_use]
    pub fn unavailable(summary: impl Into<String>) -> Self {
        Self::bounded(
            AdapterFailureKind::Unavailable,
            summary,
            "adapter dependency unavailable",
        )
    }

    /// Constructs a bounded post-entry external failure, truncating only the summary.
    #[must_use]
    pub fn external_failure(summary: impl Into<String>) -> Self {
        Self::bounded(
            AdapterFailureKind::ExternalFailure,
            summary,
            "adapter external failure",
        )
    }

    fn bounded(
        kind: AdapterFailureKind,
        summary: impl Into<String>,
        fallback: &'static str,
    ) -> Self {
        let mut summary = summary.into();
        if summary.is_empty() {
            summary = fallback.to_owned();
        }
        if summary.len() > 512 {
            let mut end = 512;
            while !summary.is_char_boundary(end) {
                end -= 1;
            }
            summary.truncate(end);
        }
        Self { kind, summary }
    }

    pub(crate) fn reporter_failure(summary: String) -> Self {
        Self {
            kind: AdapterFailureKind::ExternalFailure,
            summary,
        }
    }
}

/// Failure to construct a bounded adapter error.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("adapter failure summary must contain 1..=512 bytes")]
pub struct HostAdapterContractError;

/// Exact durable execution provenance supplied to materializing adapters.
#[derive(Clone, Debug, PartialEq)]
pub struct AdapterExecutionContext {
    run: RunId,
    revision: RevisionId,
    node: NodeId,
    execution: NodeExecutionId,
    attempt: AttemptId,
    authority: Option<ExecutionAuthorityBasis>,
    resolution_authorization: Option<AuthorityDecisionSnapshot>,
    entry_authorization: Option<AuthorityDecisionSnapshot>,
    controller_reservation: Option<ControllerReservationId>,
}

impl AdapterExecutionContext {
    /// Constructs exact durable provenance for an already validated execution dispatch.
    #[must_use]
    pub const fn new(
        run: RunId,
        revision: RevisionId,
        node: NodeId,
        execution: NodeExecutionId,
        attempt: AttemptId,
    ) -> Self {
        Self {
            run,
            revision,
            node,
            execution,
            attempt,
            authority: None,
            resolution_authorization: None,
            entry_authorization: None,
            controller_reservation: None,
        }
    }

    pub(crate) fn from_dispatch(
        dispatch: &milkdrift_runtime::ExecutionDispatch,
        controller_reservation: Option<&ControllerReservationId>,
    ) -> Self {
        Self {
            run: dispatch.run().clone(),
            revision: dispatch.revision().clone(),
            node: dispatch.node().clone(),
            execution: dispatch.execution().clone(),
            attempt: dispatch.attempt().clone(),
            authority: Some(dispatch.execution_authority().clone()),
            resolution_authorization: Some(dispatch.resolution_authorization().clone()),
            entry_authorization: Some(dispatch.entry_authorization().clone()),
            controller_reservation: controller_reservation.cloned(),
        }
    }

    /// Owning durable run.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }

    /// Exact immutable workflow revision.
    #[must_use]
    pub const fn revision(&self) -> &RevisionId {
        &self.revision
    }

    /// Stable semantic node identity.
    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }

    /// Logical node execution identity.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Immutable execution-attempt identity.
    #[must_use]
    pub const fn attempt(&self) -> &AttemptId {
        &self.attempt
    }

    /// Exact controller reservation committed before this adapter entry, when controlled.
    #[must_use]
    pub const fn controller_reservation(&self) -> Option<&ControllerReservationId> {
        self.controller_reservation.as_ref()
    }

    /// Frozen actor/grant/policy basis inherited from run acceptance.
    #[must_use]
    pub const fn authority(&self) -> Option<&ExecutionAuthorityBasis> {
        self.authority.as_ref()
    }

    /// Exact decision that allowed this capability generation to be selected.
    #[must_use]
    pub const fn resolution_authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.resolution_authorization.as_ref()
    }

    /// Fresh decision committed immediately before adapter entry.
    #[must_use]
    pub const fn entry_authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.entry_authorization.as_ref()
    }
}

/// Narrow immutable invocation view supplied to a concrete adapter.
pub struct AdapterInvocation<'a> {
    resolution: &'a ResolvedCapabilitySnapshot,
    request: &'a InvocationRequest,
    context: Option<&'a AdapterExecutionContext>,
}

impl<'a> AdapterInvocation<'a> {
    /// Constructs an immutable invocation view without durable execution provenance.
    #[must_use]
    pub const fn new(
        resolution: &'a ResolvedCapabilitySnapshot,
        request: &'a InvocationRequest,
    ) -> Self {
        Self {
            resolution,
            request,
            context: None,
        }
    }

    /// Constructs an immutable invocation view with exact durable execution provenance.
    #[must_use]
    pub const fn with_context(
        resolution: &'a ResolvedCapabilitySnapshot,
        request: &'a InvocationRequest,
        context: &'a AdapterExecutionContext,
    ) -> Self {
        Self {
            resolution,
            request,
            context: Some(context),
        }
    }

    /// Exact persisted descriptor/operation selection.
    #[must_use]
    pub const fn resolution(&self) -> &ResolvedCapabilitySnapshot {
        self.resolution
    }

    /// Provider-neutral immutable invocation request.
    #[must_use]
    pub const fn request(&self) -> &InvocationRequest {
        self.request
    }

    /// Exact durable execution provenance when invoked through `RuntimeService`.
    #[must_use]
    pub const fn context(&self) -> Option<&AdapterExecutionContext> {
        self.context
    }
}

/// Durable observation sink exposed without runtime state-mutation APIs.
pub trait AdapterReporter: Send + Sync {
    /// Submits one bounded sequenced invocation observation durably.
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError>;

    /// Requests a runtime-chosen lease extension and waits for durability.
    fn heartbeat(&self) -> Result<(), AdapterError>;
}

/// Object-safe boundary implemented by later process, model, peer, or human adapters.
pub trait CapabilityAdapter: Send + Sync {
    /// Derives enforceable bounds for this exact immutable request and generation.
    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(InvocationAdmissionEnvelope::unknown())
    }

    /// Returns immutable filesystem/network/secret and budget facts for canonical evaluation.
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        CapabilityExecutionRequirements::default()
    }

    /// Starts adapter-owned live resources before registration becomes visible.
    fn start(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    /// Executes exactly the supplied immutable selection with no fallback.
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError>;

    /// Routes cancellation to the adapter generation owning the invocation.
    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError>;

    /// Returns one bounded observation at an explicitly supplied boundary time.
    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError>;

    /// Stops accepting newly resolved work while owned work can finish.
    fn begin_drain(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    /// Releases adapter-owned live resources after admission and in-flight work close.
    fn shutdown(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}
