use std::sync::Arc;

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
    /// Adapter rejected local validation or state; the execution stage determines entry proof.
    Rejected,
    /// Adapter's external dependency was unavailable.
    Unavailable,
    /// Adapter entered its external boundary and could not prove an outcome.
    ExternalFailure,
    /// A complete provider response was parsed, but local publication/reporting failed.
    ResponseObservedFailure,
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

    /// Describes a bounded local rejection. Runtime treats this as no-entry proof only
    /// when it comes from preparation before durable entry intent, never from error text alone.
    #[must_use]
    pub fn rejected(summary: impl Into<String>) -> Self {
        Self::bounded(
            AdapterFailureKind::Rejected,
            summary,
            "adapter rejected request",
        )
    }

    /// Describes a bounded unavailable dependency. A post-intent return still requires
    /// conservative recovery when no terminal observation became durable.
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

    /// Preserves response completion when subsequent local work loses durable reporting.
    /// This is not terminal proof and must never authorize replay of the provider request.
    #[must_use]
    pub fn response_observed_failure(summary: impl Into<String>) -> Self {
        Self::bounded(
            AdapterFailureKind::ResponseObservedFailure,
            summary,
            "local failure after complete provider response",
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
        let boundary = milkdrift_contracts::truncate_utf8(&summary, 512).len();
        summary.truncate(boundary);
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
///
/// The host supplies a sink scoped to exactly one invocation. Adapters must propagate every sink
/// failure, preserve contiguous invocation sequence numbers (one for the first observation),
/// and emit at most one terminal observation with nothing after it. A successful
/// return from [`AdapterReporter::invocation`] means the observation crossed the owning durable
/// boundary; it is not merely queued in adapter memory.
pub trait AdapterReporter: Send + Sync {
    /// Submits one bounded sequenced invocation observation durably.
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError>;

    /// Requests a runtime-chosen lease extension and waits for durability.
    fn heartbeat(&self) -> Result<(), AdapterError>;
}

/// Object-safe boundary implemented by process, model, peer, or human adapters.
///
/// One implementation instance represents one immutable descriptor generation. The capability
/// host calls adapter code without holding its registry lock, contains panics at every hook, and
/// owns registration visibility and exact-generation permits. Implementations own only their
/// external mechanism and live resources: they never re-resolve, fall back, mutate workflow state,
/// or treat cancellation receipt as terminal evidence.
///
/// Lifecycle hooks deliberately have no defaults. Stateless implementations must spell out their
/// idempotent no-resource behavior; resource-owning implementations must make replay, drain, and
/// shutdown behavior explicit in their own state machine.
pub trait CapabilityAdapter: Send + Sync + 'static {
    /// Prepares local request data without contacting an external capability.
    ///
    /// The host holds the exact generation permit while this runs. Local reads must be
    /// bounded and authorized. The returned closure owns the prepared bytes until the
    /// final authority/account transaction permits its single entry. The default only
    /// freezes admission facts; adapters with local request validation should override it.
    fn prepare(
        self: Arc<Self>,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<PreparedAdapterExecution, AdapterError> {
        let envelope = self.admission_envelope(invocation)?;
        Ok(PreparedAdapterExecution::new(
            envelope,
            move |invocation, reporter| self.execute(invocation, reporter),
        ))
    }

    /// Derives enforceable bounds for this exact immutable request and generation.
    /// This hook runs before durable entry intent; it must not start external work. Return
    /// unknown resource dimensions honestly so runtime can refuse unsupported reservations.
    fn admission_envelope(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError>;

    /// Returns immutable filesystem/network/secret and budget facts for canonical evaluation.
    ///
    /// The result must be deterministic for this generation and must not consult mutable ambient
    /// authority. The host snapshots it before registration becomes visible.
    fn authority_requirements(&self) -> CapabilityExecutionRequirements;

    /// Starts adapter-owned live resources before registration becomes visible.
    ///
    /// Repeated calls must either replay idempotently or return a stable typed lifecycle conflict.
    /// A failed call must remain safe for one cleanup call to [`CapabilityAdapter::shutdown`].
    fn start(&self) -> Result<(), AdapterError>;

    /// Executes exactly the supplied immutable selection with no fallback.
    /// Emit observations through the reporter and propagate its failures. A successful method
    /// return without durable terminal evidence does not establish completion. Resource-owning
    /// implementations must also arrange cleanup when reporting fails after external entry.
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

    /// Stops adapter-owned admission while already selected exact work and active work can finish.
    ///
    /// The host separately removes this generation from new resolution before invoking the hook.
    /// Repeated drain calls must be idempotent.
    fn begin_drain(&self) -> Result<(), AdapterError>;

    /// Releases adapter-owned live resources after admission and in-flight work close.
    ///
    /// Repeated calls must be idempotent. Resource-owning implementations must join or release
    /// everything they own, while stateless implementations explicitly return the no-resource
    /// outcome.
    fn shutdown(&self) -> Result<(), AdapterError>;
}

type AdapterEntry = Box<
    dyn FnOnce(&AdapterInvocation<'_>, &dyn AdapterReporter) -> Result<(), AdapterError> + Send,
>;

/// Adapter-owned request preparation retained by the host's exact one-shot dispatch.
///
/// Only the host can consume this handle. Its runtime dispatch binds request, context,
/// generation, and authority; the closure receives the final context including any committed
/// controller reservation. Dropping an unentered handle releases its ephemeral data.
pub struct PreparedAdapterExecution {
    envelope: InvocationAdmissionEnvelope,
    entry: AdapterEntry,
}

impl PreparedAdapterExecution {
    /// Captures bounded local preparation and the only operation that may submit it.
    pub fn new(
        envelope: InvocationAdmissionEnvelope,
        entry: impl FnOnce(&AdapterInvocation<'_>, &dyn AdapterReporter) -> Result<(), AdapterError>
        + Send
        + 'static,
    ) -> Self {
        Self {
            envelope,
            entry: Box::new(entry),
        }
    }

    pub(crate) fn envelope(&self) -> &InvocationAdmissionEnvelope {
        &self.envelope
    }

    pub(crate) fn enter(
        self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        (self.entry)(invocation, reporter)
    }
}
