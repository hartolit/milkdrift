//! Pure, deterministic projection of one run's authoritative event history.
//!
//! This module deliberately has no persistence, clock, ID-generation, registry, or
//! executor dependency. It accepts already-decoded event envelopes and either
//! projects every fact or rejects the history at the first point where doing so
//! would require guessing.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::{ContentDigest, NodeId, PortId, RevisionId, WorkflowId};
use milkdrift_capability::{
    BoundedJson, CancellationAcknowledgement, ErrorClass, IdempotencyBehavior, IdempotencyKey,
    InvocationId, InvocationRequest, ResolvedCapabilitySnapshot, SideEffectClass,
};
use milkdrift_persistence::{
    ActorRef, AttemptId, AttemptUsage, AuthorityDecision, BoundedDetail, BranchResultReference,
    CommandId, CorrelationKey, CurrencyCode, EventId, EvidenceReference, JoinRule, LeaseId,
    MAX_PAGE_SIZE, MAX_REPEAT_CONTINUATION_CYCLES, MAX_REPEAT_EFFECTIVE_ITERATIONS,
    NodeExecutionId, NodeExecutionMode, NodeOutcome, Reason, ReconciliationAction,
    ReconciliationClassification, ReconciliationDecisionId, ReconciliationId,
    ReconciliationItem, ReconciliationPlanId, ReconciliationPolicy, RecoveryClassification,
    RepeatContinuationCause,
    RepeatContinuationDecision, RepeatDecisionId, RepeatTerminationReason, RunEventEnvelope,
    RunEventKind, RunOutcome, RunSequence, SignalDeliveryMode, SignalId, SignalTypeId,
    SubworkflowOwnership, TimerId, TimestampMillis, WaitCondition, WaitSatisfaction, WorkerId,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactReference, BranchId, CausalReference, IterationId, RunId,
    ScopeKind, ScopeReference, SubworkflowId, WorkspaceBudget, WorkspaceScope,
    WorkspaceValueReference,
};

use crate::RuntimeError;

/// Lifecycle derived exclusively from durable run facts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RunLifecycle {
    /// No creation fact has been applied.
    #[default]
    Uncreated,
    /// The aggregate exists but has not started.
    Created,
    /// The run is admitting and executing work.
    Running,
    /// New admission and dispatch are paused.
    Paused,
    /// Durable cancellation intent has been recorded.
    Cancelling,
    /// The run reached a truthful terminal boundary.
    Terminal(RunOutcome),
}

impl RunLifecycle {
    /// Returns whether the run exists but has not started.
    #[must_use]
    pub const fn is_pending(self) -> bool {
        matches!(self, Self::Created)
    }

    /// Returns whether the run is started and nonterminal.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::Cancelling)
    }

    /// Returns whether the run reached a terminal outcome.
    #[must_use]
    pub const fn is_completed(self) -> bool {
        matches!(self, Self::Terminal(_))
    }

    /// Returns the terminal outcome, when present.
    #[must_use]
    pub const fn outcome(self) -> Option<RunOutcome> {
        match self {
            Self::Terminal(outcome) => Some(outcome),
            Self::Uncreated | Self::Created | Self::Running | Self::Paused | Self::Cancelling => {
                None
            }
        }
    }
}

/// One exact revision pin and the sequence at which it became effective.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionPin {
    revision: RevisionId,
    digest: ContentDigest,
    effective_sequence: RunSequence,
    plan: Option<ReconciliationPlanId>,
}

impl RevisionPin {
    /// Exact immutable revision.
    #[must_use]
    pub const fn revision(&self) -> &RevisionId {
        &self.revision
    }

    /// Semantic content digest of the revision.
    #[must_use]
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    /// First event sequence governed by this pin.
    #[must_use]
    pub const fn effective_sequence(&self) -> RunSequence {
        self.effective_sequence
    }

    /// Reconciliation plan authorizing a prospective repin, if any.
    #[must_use]
    pub const fn plan(&self) -> Option<&ReconciliationPlanId> {
        self.plan.as_ref()
    }
}

/// Durable cancellation intent for the aggregate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunCancellation {
    reason: Reason,
    evidence: Vec<EvidenceReference>,
    sequence: RunSequence,
}

/// Durable internal drain intent selected by an explicit non-cancellation terminal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunTerminationIntent {
    outcome: RunOutcome,
    reason: Reason,
    sequence: RunSequence,
}

impl RunTerminationIntent {
    /// Outcome to record after all structured ownership becomes quiescent.
    #[must_use]
    pub const fn outcome(&self) -> RunOutcome {
        self.outcome
    }

    /// Bounded rationale for draining already-owned work.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Sequence at which the terminal selection became durable.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

impl RunCancellation {
    /// Recorded cancellation rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Supporting durable evidence references.
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceReference] {
        &self.evidence
    }

    /// Sequence at which cancellation intent became durable.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Dynamic state of one logical node execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NodeExecutionState {
    /// Eligible but not yet scheduled.
    Eligible,
    /// An attempt is scheduled but has not started.
    Scheduled(AttemptId),
    /// An attempt is actively executing.
    Running(AttemptId),
    /// A retry attempt exists and is waiting for its durable timer/dispatch.
    RetryPending(AttemptId),
    /// The latest attempt has an unresolved external outcome.
    Uncertain(AttemptId),
    /// The latest known attempt reached a terminal result.
    Terminal(NodeOutcome),
    /// Structured cancellation removed this execution before any executor attempt existed.
    CancelledBeforeDispatch,
    /// A prospective reconciliation plan removed work that never started.
    RemovedProspectively(ReconciliationPlanId),
}

/// Durable execution-local cancellation fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeExecutionCancellationProjection {
    attempt: Option<AttemptId>,
    reason: Reason,
    sequence: RunSequence,
}

impl NodeExecutionCancellationProjection {
    /// Exact executor attempt receiving cancellation, or `None` before first dispatch.
    #[must_use]
    pub const fn attempt(&self) -> Option<&AttemptId> {
        self.attempt.as_ref()
    }

    /// Bounded causal cancellation rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Sequence at which execution-local cancellation became durable.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Terminal fact for an attempt-free execution, including deterministic runtime
/// work and immutable executor request failures before dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeterministicNodeTerminalProjection {
    outcome: NodeOutcome,
    error_class: Option<ErrorClass>,
    detail: Option<BoundedDetail>,
    sequence: RunSequence,
}

impl DeterministicNodeTerminalProjection {
    /// Truthful deterministic outcome.
    #[must_use]
    pub const fn outcome(&self) -> NodeOutcome {
        self.outcome
    }

    /// Classified deterministic failure, when relevant.
    #[must_use]
    pub const fn error_class(&self) -> Option<ErrorClass> {
        self.error_class
    }

    /// Bounded deterministic result/failure detail.
    #[must_use]
    pub const fn detail(&self) -> Option<&BoundedDetail> {
        self.detail.as_ref()
    }

    /// Sequence at which the deterministic execution terminated.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Projected state of one stable logical node execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeExecutionProjection {
    execution: NodeExecutionId,
    node: NodeId,
    scope: ScopeReference,
    mode: NodeExecutionMode,
    created_sequence: RunSequence,
    created_at: TimestampMillis,
    attempts: Vec<AttemptId>,
    state: NodeExecutionState,
    cancellation: Option<NodeExecutionCancellationProjection>,
    deterministic_terminal: Option<DeterministicNodeTerminalProjection>,
    outputs: Vec<PublishedNodeOutput>,
}

impl NodeExecutionProjection {
    /// Stable logical execution identity.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Stable semantic blueprint node.
    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }

    /// Exact workspace scope used by this execution.
    #[must_use]
    pub const fn scope(&self) -> &ScopeReference {
        &self.scope
    }

    /// Closed dispatch ownership recorded when the execution became eligible.
    #[must_use]
    pub const fn mode(&self) -> NodeExecutionMode {
        self.mode
    }

    /// Sequence at which eligibility was recorded.
    #[must_use]
    pub const fn created_sequence(&self) -> RunSequence {
        self.created_sequence
    }

    /// Boundary-clock fact recorded when eligibility was created.
    #[must_use]
    pub const fn created_at(&self) -> TimestampMillis {
        self.created_at
    }

    /// Immutable attempt identities in one-based attempt order.
    #[must_use]
    pub fn attempts(&self) -> &[AttemptId] {
        &self.attempts
    }

    /// Current execution state.
    #[must_use]
    pub const fn state(&self) -> &NodeExecutionState {
        &self.state
    }

    /// Durable execution-local cancellation fact, when present.
    #[must_use]
    pub const fn cancellation(&self) -> Option<&NodeExecutionCancellationProjection> {
        self.cancellation.as_ref()
    }

    /// Direct attempt-free terminal fact, when present.
    #[must_use]
    pub const fn deterministic_terminal(&self) -> Option<&DeterministicNodeTerminalProjection> {
        self.deterministic_terminal.as_ref()
    }

    /// Immutable outputs published by all attempts of the execution.
    #[must_use]
    pub fn outputs(&self) -> &[PublishedNodeOutput] {
        &self.outputs
    }

    /// Returns whether this execution is eligible or waiting for a retry.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(
            self.state,
            NodeExecutionState::Eligible
                | NodeExecutionState::Scheduled(_)
                | NodeExecutionState::RetryPending(_)
        )
    }

    /// Returns whether an invocation is active or its outcome is unresolved.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(
            self.state,
            NodeExecutionState::Running(_) | NodeExecutionState::Uncertain(_)
        )
    }

    /// Returns whether the latest attempt is terminal and no retry is pending.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(
            self.state,
            NodeExecutionState::Terminal(_)
                | NodeExecutionState::CancelledBeforeDispatch
                | NodeExecutionState::RemovedProspectively(_)
        )
    }
}

/// Dynamic state of one immutable attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttemptState {
    /// Retry identity is reserved but its timer has not fired.
    AwaitingRetryTimer,
    /// Retry timer fired and the attempt is ready to be scheduled.
    ReadyToSchedule,
    /// Invocation was scheduled durably.
    Scheduled,
    /// A worker owns an active durable lease.
    Leased,
    /// Executor start was observed.
    Running,
    /// A known terminal result was observed.
    Terminal(NodeOutcome),
    /// External outcome cannot currently be established.
    Uncertain,
    /// The physical attempt remains uncertain, but a later exact safe replay
    /// established the logical invocation result.
    UncertainSupersededByRetry {
        /// Exact terminal retry whose immutable request covered this one.
        covering_attempt: AttemptId,
    },
    /// The physical attempt remains uncertain, but cancellation abandoned a
    /// harmless replay chain that cannot have external write effects.
    UncertainAbandonedByCancellation {
        /// Reserved retry whose cancellation closed the logical work.
        cancelled_retry: AttemptId,
    },
    /// External work is intentionally retained as an unresolved obligation.
    Retained,
    /// Evidence and authority resolved an uncertain outcome.
    Resolved(NodeOutcome),
    /// A retry attempt reserved in history was cancelled before it was dispatched.
    CancelledBeforeDispatch,
}

/// Exact capability facts frozen for an attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct CapabilityResolution {
    requirement: milkdrift_capability::CapabilityRequirement,
    snapshot: ResolvedCapabilitySnapshot,
}

impl CapabilityResolution {
    /// Blueprint-owned capability selection requirement.
    #[must_use]
    pub const fn requirement(&self) -> &milkdrift_capability::CapabilityRequirement {
        &self.requirement
    }

    /// Exact immutable capability/operation snapshot supplied to the executor.
    #[must_use]
    pub const fn snapshot(&self) -> &ResolvedCapabilitySnapshot {
        &self.snapshot
    }
}

/// Side-effect and external-idempotency facts frozen before dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SideEffectClassification {
    side_effect: SideEffectClass,
    idempotency: IdempotencyBehavior,
    idempotency_key: Option<IdempotencyKey>,
}

impl SideEffectClassification {
    /// Potential external side-effect class.
    #[must_use]
    pub const fn side_effect(&self) -> SideEffectClass {
        self.side_effect
    }

    /// Advertised external idempotency behavior.
    #[must_use]
    pub const fn idempotency(&self) -> IdempotencyBehavior {
        self.idempotency
    }

    /// Stable propagated idempotency key, if supported.
    #[must_use]
    pub const fn idempotency_key(&self) -> Option<&IdempotencyKey> {
        self.idempotency_key.as_ref()
    }
}

/// One bounded monotonic executor progress observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgressObservation {
    report_sequence: u64,
    detail: BoundedDetail,
    completed_units: Option<u64>,
    total_units: Option<u64>,
}

impl ProgressObservation {
    /// Monotonic executor report sequence.
    #[must_use]
    pub const fn report_sequence(&self) -> u64 {
        self.report_sequence
    }

    /// Bounded redacted progress detail.
    #[must_use]
    pub const fn detail(&self) -> &BoundedDetail {
        &self.detail
    }

    /// Provider-defined completed units.
    #[must_use]
    pub const fn completed_units(&self) -> Option<u64> {
        self.completed_units
    }

    /// Provider-defined total units.
    #[must_use]
    pub const fn total_units(&self) -> Option<u64> {
        self.total_units
    }
}

/// One immutable workspace output publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedNodeOutput {
    report_sequence: Option<u64>,
    value: WorkspaceValueReference,
    artifact: Option<ArtifactReference>,
    sequence: RunSequence,
}

impl PublishedNodeOutput {
    /// Executor-local report sequence.
    #[must_use]
    pub const fn report_sequence(&self) -> Option<u64> {
        self.report_sequence
    }

    /// Exact immutable workspace value reference.
    #[must_use]
    pub const fn value(&self) -> &WorkspaceValueReference {
        &self.value
    }

    /// Optional separately stored content-addressed artifact.
    #[must_use]
    pub const fn artifact(&self) -> Option<&ArtifactReference> {
        self.artifact.as_ref()
    }

    /// Publication event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Known terminal result of one immutable attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptTerminal {
    report_sequence: u64,
    outcome: NodeOutcome,
    error_class: Option<ErrorClass>,
    detail: Option<BoundedDetail>,
    sequence: RunSequence,
}

impl AttemptTerminal {
    /// Executor-local terminal report sequence.
    #[must_use]
    pub const fn report_sequence(&self) -> u64 {
        self.report_sequence
    }

    /// Truthful known node outcome.
    #[must_use]
    pub const fn outcome(&self) -> NodeOutcome {
        self.outcome
    }

    /// Classified failure, when relevant.
    #[must_use]
    pub const fn error_class(&self) -> Option<ErrorClass> {
        self.error_class
    }

    /// Bounded redacted terminal detail.
    #[must_use]
    pub const fn detail(&self) -> Option<&BoundedDetail> {
        self.detail.as_ref()
    }

    /// Terminal event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Current unresolved external-outcome obligation for an attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalOutcomeObligation {
    report_sequence: u64,
    side_effect: SideEffectClass,
    reason: Reason,
    evidence: Vec<EvidenceReference>,
    uncertain_sequence: RunSequence,
    retained: Option<RetainedExternalOutcome>,
    decisions: Vec<RecoveryDecision>,
}

impl ExternalOutcomeObligation {
    /// Executor-local terminal report sequence that established uncertainty.
    #[must_use]
    pub const fn report_sequence(&self) -> u64 {
        self.report_sequence
    }

    /// Side-effect class governing recovery safety.
    #[must_use]
    pub const fn side_effect(&self) -> SideEffectClass {
        self.side_effect
    }

    /// Why certainty was lost.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Evidence supporting the uncertainty classification.
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceReference] {
        &self.evidence
    }

    /// Sequence at which uncertainty became durable.
    #[must_use]
    pub const fn uncertain_sequence(&self) -> RunSequence {
        self.uncertain_sequence
    }

    /// Explicit retention fact, if one was recorded.
    #[must_use]
    pub const fn retained(&self) -> Option<&RetainedExternalOutcome> {
        self.retained.as_ref()
    }

    /// Operator/controller decisions recorded for the obligation.
    #[must_use]
    pub fn decisions(&self) -> &[RecoveryDecision] {
        &self.decisions
    }
}

/// Explicit fact retaining uncertain external work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedExternalOutcome {
    decision: ReconciliationDecisionId,
    reason: Reason,
    sequence: RunSequence,
}

impl RetainedExternalOutcome {
    /// Authority decision supporting retention.
    #[must_use]
    pub const fn decision(&self) -> &ReconciliationDecisionId {
        &self.decision
    }

    /// Why the external work remains retained.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Retention event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Complete read model for one immutable attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeAttemptProjection {
    attempt: AttemptId,
    execution: NodeExecutionId,
    attempt_number: u32,
    invocation: Option<InvocationId>,
    idempotency_key: Option<IdempotencyKey>,
    request: Option<InvocationRequest>,
    scheduled_sequence: Option<RunSequence>,
    state: AttemptState,
    capability: Option<CapabilityResolution>,
    side_effect: Option<SideEffectClassification>,
    leases: Vec<LeaseId>,
    progress: Vec<ProgressObservation>,
    last_report_sequence: Option<u64>,
    usage: Option<AttemptUsage>,
    cancellation_acknowledgements: Vec<CancellationAcknowledgement>,
    outputs: Vec<PublishedNodeOutput>,
    terminal: Option<AttemptTerminal>,
    obligation: Option<ExternalOutcomeObligation>,
    recovery: Vec<RecoveryObservation>,
}

impl NodeAttemptProjection {
    /// Immutable attempt identity.
    #[must_use]
    pub const fn attempt(&self) -> &AttemptId {
        &self.attempt
    }

    /// Owning logical execution.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// One-based attempt number within the execution.
    #[must_use]
    pub const fn attempt_number(&self) -> u32 {
        self.attempt_number
    }

    /// Stable executor-facing invocation identity after scheduling.
    #[must_use]
    pub const fn invocation(&self) -> Option<&InvocationId> {
        self.invocation.as_ref()
    }

    /// Stable propagated external idempotency key.
    #[must_use]
    pub const fn idempotency_key(&self) -> Option<&IdempotencyKey> {
        self.idempotency_key.as_ref()
    }

    /// Exact immutable request durably scheduled for executor delivery.
    #[must_use]
    pub const fn request(&self) -> Option<&InvocationRequest> {
        self.request.as_ref()
    }

    /// Sequence at which the exact executor request became durable.
    ///
    /// This is absent only while a retry identity is reserved but not yet
    /// scheduled. Combined with [`RunProjection::revision_at`], it identifies
    /// the immutable workflow revision governing this attempt.
    #[must_use]
    pub const fn scheduled_sequence(&self) -> Option<RunSequence> {
        self.scheduled_sequence
    }

    /// Current projected attempt state.
    #[must_use]
    pub const fn state(&self) -> &AttemptState {
        &self.state
    }

    /// Exact frozen capability resolution, when applicable.
    #[must_use]
    pub const fn capability(&self) -> Option<&CapabilityResolution> {
        self.capability.as_ref()
    }

    /// Frozen side-effect and idempotency classification.
    #[must_use]
    pub const fn side_effect(&self) -> Option<&SideEffectClassification> {
        self.side_effect.as_ref()
    }

    /// Durable leases granted to this same attempt over recovery cycles.
    #[must_use]
    pub fn leases(&self) -> &[LeaseId] {
        &self.leases
    }

    /// Ordered monotonic progress reports.
    #[must_use]
    pub fn progress(&self) -> &[ProgressObservation] {
        &self.progress
    }

    /// Latest durably projected executor report sequence.
    #[must_use]
    pub const fn last_report_sequence(&self) -> Option<u64> {
        self.last_report_sequence
    }

    /// Exact provider-neutral usage observation.
    #[must_use]
    pub const fn usage(&self) -> Option<&AttemptUsage> {
        self.usage.as_ref()
    }

    /// Ordered executor cancellation acknowledgements.
    #[must_use]
    pub fn cancellation_acknowledgements(&self) -> &[CancellationAcknowledgement] {
        &self.cancellation_acknowledgements
    }

    /// Immutable output publications from this attempt.
    #[must_use]
    pub fn outputs(&self) -> &[PublishedNodeOutput] {
        &self.outputs
    }

    /// Known terminal report, if one was observed directly.
    #[must_use]
    pub const fn terminal(&self) -> Option<&AttemptTerminal> {
        self.terminal.as_ref()
    }

    /// Durable external-outcome record, including one later covered by a safe retry.
    #[must_use]
    pub const fn obligation(&self) -> Option<&ExternalOutcomeObligation> {
        self.obligation.as_ref()
    }

    /// Durable recovery classifications for this attempt.
    #[must_use]
    pub fn recovery(&self) -> &[RecoveryObservation] {
        &self.recovery
    }

    /// Returns whether this attempt awaits timer, scheduling, or dispatch.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(
            self.state,
            AttemptState::AwaitingRetryTimer
                | AttemptState::ReadyToSchedule
                | AttemptState::Scheduled
        )
    }

    /// Returns whether this attempt currently has active executor ownership.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.state, AttemptState::Leased | AttemptState::Running)
    }

    /// Returns whether the attempt no longer owns outstanding logical work.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(
            self.state,
            AttemptState::Terminal(_)
                | AttemptState::Resolved(_)
                | AttemptState::UncertainSupersededByRetry { .. }
                | AttemptState::UncertainAbandonedByCancellation { .. }
                | AttemptState::CancelledBeforeDispatch
        )
    }

    /// Returns whether external truth remains unresolved or retained.
    #[must_use]
    pub const fn is_unresolved(&self) -> bool {
        self.obligation.is_some()
            && matches!(self.state, AttemptState::Uncertain | AttemptState::Retained)
    }

    fn expects_report_sequence(&self, report_sequence: u64) -> bool {
        self.last_report_sequence
            .map_or(report_sequence == 1, |last| {
                last.checked_add(1) == Some(report_sequence)
            })
    }
}

/// State of a durable lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaseState {
    /// Lease remains the active ownership fact.
    Active,
    /// Lease expired with the recorded recovery classification.
    Expired(RecoveryClassification),
    /// An expired lease was superseded by a new lease for the same attempt.
    Superseded(LeaseId),
    /// The attempt reached a terminal or evidence-resolved boundary.
    Completed,
}

/// Read model for one immutable worker lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaseProjection {
    lease: LeaseId,
    execution: NodeExecutionId,
    attempt: AttemptId,
    worker: WorkerId,
    expires_at: TimestampMillis,
    state: LeaseState,
}

impl LeaseProjection {
    /// Stable lease identity.
    #[must_use]
    pub const fn lease(&self) -> &LeaseId {
        &self.lease
    }

    /// Owning logical execution.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Owning immutable attempt.
    #[must_use]
    pub const fn attempt(&self) -> &AttemptId {
        &self.attempt
    }

    /// Worker/controller holding this lease.
    #[must_use]
    pub const fn worker(&self) -> &WorkerId {
        &self.worker
    }

    /// Latest recorded expiration fact.
    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }

    /// Current lease state.
    #[must_use]
    pub const fn state(&self) -> &LeaseState {
        &self.state
    }

    /// Returns whether this is the attempt's active lease.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.state, LeaseState::Active)
    }

    /// Returns whether the lease no longer owns work.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        !self.is_active()
    }
}

/// Origin and state of a durable timer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimerPurpose {
    /// General workflow wait, optionally attached to a node execution.
    Wait {
        /// Waiting execution when the timer is node-owned.
        execution: Option<NodeExecutionId>,
    },
    /// Retry backoff for one reserved next attempt.
    Retry {
        /// Reserved next attempt admitted by this timer.
        attempt: AttemptId,
    },
}

/// State of a durable timer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerState {
    /// Registered and not yet fired.
    Pending,
    /// Fired at the recorded boundary-clock observation.
    Fired {
        /// Boundary-clock observation proving the deadline elapsed.
        observed_at: TimestampMillis,
    },
    /// Explicitly cancelled before firing.
    Cancelled,
}

/// Durable cancellation fact for a timer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimerCancellationProjection {
    reason: Reason,
    sequence: RunSequence,
}

impl TimerCancellationProjection {
    /// Bounded causal cancellation rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Sequence at which cancellation became durable.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Durable timer read model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimerProjection {
    timer: TimerId,
    purpose: TimerPurpose,
    fire_at: TimestampMillis,
    state: TimerState,
    cancellation: Option<TimerCancellationProjection>,
}

impl TimerProjection {
    /// Stable timer identity.
    #[must_use]
    pub const fn timer(&self) -> &TimerId {
        &self.timer
    }

    /// Workflow wait or retry purpose.
    #[must_use]
    pub const fn purpose(&self) -> &TimerPurpose {
        &self.purpose
    }

    /// Exact registered deadline.
    #[must_use]
    pub const fn fire_at(&self) -> TimestampMillis {
        self.fire_at
    }

    /// Current timer state.
    #[must_use]
    pub const fn state(&self) -> TimerState {
        self.state
    }

    /// Explicit cancellation fact, when present.
    #[must_use]
    pub const fn cancellation(&self) -> Option<&TimerCancellationProjection> {
        self.cancellation.as_ref()
    }

    /// Returns whether the timer is still pending.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self.state, TimerState::Pending)
    }

    /// Returns whether the timer fired.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(self.state, TimerState::Fired { .. } | TimerState::Cancelled)
    }
}

/// Current retry admission state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryState {
    /// Backoff timer has not fired.
    Waiting,
    /// Backoff timer fired and the next attempt may be scheduled.
    Ready,
    /// The next attempt was scheduled.
    Scheduled,
    /// Structured cancellation prevented the reserved attempt from dispatching.
    Cancelled,
}

/// Immutable retry decision and its current admission state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetryProjection {
    execution: NodeExecutionId,
    previous_attempt: AttemptId,
    next_attempt: AttemptId,
    attempt_number: u32,
    timer: TimerId,
    fire_at: TimestampMillis,
    error_class: ErrorClass,
    reason: Reason,
    state: RetryState,
}

impl RetryProjection {
    /// Owning logical execution.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Completed or authority-released prior attempt.
    #[must_use]
    pub const fn previous_attempt(&self) -> &AttemptId {
        &self.previous_attempt
    }

    /// Reserved immutable next attempt.
    #[must_use]
    pub const fn next_attempt(&self) -> &AttemptId {
        &self.next_attempt
    }

    /// One-based number of the next attempt.
    #[must_use]
    pub const fn attempt_number(&self) -> u32 {
        self.attempt_number
    }

    /// Durable backoff timer.
    #[must_use]
    pub const fn timer(&self) -> &TimerId {
        &self.timer
    }

    /// Recorded deadline including deterministic or recorded jitter.
    #[must_use]
    pub const fn fire_at(&self) -> TimestampMillis {
        self.fire_at
    }

    /// Failure class selected by retry policy.
    #[must_use]
    pub const fn error_class(&self) -> ErrorClass {
        self.error_class
    }

    /// Bounded policy rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Current retry state.
    #[must_use]
    pub const fn state(&self) -> RetryState {
        self.state
    }

    /// Returns whether retry admission remains pending.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self.state, RetryState::Waiting | RetryState::Ready)
    }

    /// Returns whether the retry attempt was scheduled.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(self.state, RetryState::Scheduled | RetryState::Cancelled)
    }
}

/// Current state of a structured branch scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchState {
    /// Branch may still acquire or complete owned children.
    Active,
    /// Structured cancellation intent was propagated into the branch.
    Cancelling,
    /// A satisfied join recorded the branch's terminal result.
    Completed(RunOutcome),
    /// Early join satisfaction explicitly retained this branch.
    Retained,
}

/// Structured branch scope and ownership read model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchProjection {
    branch: BranchId,
    fork_execution: NodeExecutionId,
    port: PortId,
    scope: WorkspaceScope,
    children: BTreeSet<NodeExecutionId>,
    state: BranchState,
    cancellation_reason: Option<Reason>,
    outputs: Vec<WorkspaceValueReference>,
}

impl BranchProjection {
    /// Stable semantic branch identity.
    #[must_use]
    pub const fn branch(&self) -> &BranchId {
        &self.branch
    }

    /// Fork execution owning this structured branch.
    #[must_use]
    pub const fn fork_execution(&self) -> &NodeExecutionId {
        &self.fork_execution
    }

    /// Exact fork output port owning this branch.
    #[must_use]
    pub const fn port(&self) -> &PortId {
        &self.port
    }

    /// Exact branch-local workspace scope.
    #[must_use]
    pub const fn scope(&self) -> &WorkspaceScope {
        &self.scope
    }

    /// Child executions owned by the branch.
    #[must_use]
    pub const fn children(&self) -> &BTreeSet<NodeExecutionId> {
        &self.children
    }

    /// Current branch state.
    #[must_use]
    pub const fn state(&self) -> BranchState {
        self.state
    }

    /// Structured cancellation rationale, when requested.
    #[must_use]
    pub const fn cancellation_reason(&self) -> Option<&Reason> {
        self.cancellation_reason.as_ref()
    }

    /// Exact immutable branch-local terminal outputs.
    #[must_use]
    pub fn outputs(&self) -> &[WorkspaceValueReference] {
        &self.outputs
    }

    /// Returns whether branch work remains live.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.state, BranchState::Active | BranchState::Cancelling)
    }

    /// Returns whether a terminal result or explicit retention was recorded.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(
            self.state,
            BranchState::Completed(_) | BranchState::Retained
        )
    }
}

/// Immutable satisfied join result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JoinProjection {
    execution: NodeExecutionId,
    rule: JoinRule,
    branches: Vec<BranchResultReference>,
    retained_branches: Vec<BranchId>,
    sequence: RunSequence,
}

impl JoinProjection {
    /// Join node execution.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Exact synchronization rule.
    #[must_use]
    pub const fn rule(&self) -> JoinRule {
        self.rule
    }

    /// Immutable branch results supplied to downstream composition.
    #[must_use]
    pub fn branches(&self) -> &[BranchResultReference] {
        &self.branches
    }

    /// Branches retained after early satisfaction.
    #[must_use]
    pub fn retained_branches(&self) -> &[BranchId] {
        &self.retained_branches
    }

    /// Sequence at which the join became satisfied.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Current state of one isolated repeat iteration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IterationState {
    /// Body/condition may still progress.
    Active,
    /// Frozen condition result awaits the next iteration or repeat termination.
    ConditionRecorded(bool),
    /// A later iteration or repeat termination closed this iteration.
    Completed(bool),
}

/// Isolated repeat-iteration scope read model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IterationProjection {
    iteration: IterationId,
    repeat_execution: NodeExecutionId,
    iteration_number: u32,
    scope: WorkspaceScope,
    state: IterationState,
}

impl IterationProjection {
    /// Stable iteration identity.
    #[must_use]
    pub const fn iteration(&self) -> &IterationId {
        &self.iteration
    }

    /// Owning repeat execution.
    #[must_use]
    pub const fn repeat_execution(&self) -> &NodeExecutionId {
        &self.repeat_execution
    }

    /// One-based iteration number.
    #[must_use]
    pub const fn iteration_number(&self) -> u32 {
        self.iteration_number
    }

    /// Exact isolated child scope.
    #[must_use]
    pub const fn scope(&self) -> &WorkspaceScope {
        &self.scope
    }

    /// Current iteration state.
    #[must_use]
    pub const fn state(&self) -> IterationState {
        self.state
    }

    /// Returns whether this iteration remains the active repeat frontier.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !matches!(self.state, IterationState::Completed(_))
    }

    /// Returns whether a later durable fact closed the iteration.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(self.state, IterationState::Completed(_))
    }
}

/// One immutable request for authority at an exact repeat boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepeatContinuationRequestProjection {
    frontier_iteration: IterationId,
    initial_iteration_limit: u32,
    effective_iteration_limit: u32,
    cause: RepeatContinuationCause,
    sequence: RunSequence,
}

impl RepeatContinuationRequestProjection {
    /// Exact true-condition frontier requiring authority.
    #[must_use]
    pub const fn frontier_iteration(&self) -> &IterationId {
        &self.frontier_iteration
    }

    /// Original configured iteration limit recorded by the first request.
    #[must_use]
    pub const fn initial_iteration_limit(&self) -> u32 {
        self.initial_iteration_limit
    }

    /// Effective iteration limit at the time of this request.
    #[must_use]
    pub const fn effective_iteration_limit(&self) -> u32 {
        self.effective_iteration_limit
    }

    /// Exact iteration, duration, or currency-specific cost boundary.
    #[must_use]
    pub const fn cause(&self) -> &RepeatContinuationCause {
        &self.cause
    }

    /// Sequence at which the request became durable.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// One immutable authority decision at a repeat continuation boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepeatContinuationDecisionProjection {
    decision: RepeatDecisionId,
    actor: ActorRef,
    outcome: RepeatContinuationDecision,
    approved_additional_iterations: Option<u32>,
    reason: Reason,
    evidence: Vec<EvidenceReference>,
    sequence: RunSequence,
}

impl RepeatContinuationDecisionProjection {
    /// Stable decision identity.
    #[must_use]
    pub const fn decision(&self) -> &RepeatDecisionId {
        &self.decision
    }

    /// Actor exercising continuation authority.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        &self.actor
    }

    /// Closed approval/rejection outcome.
    #[must_use]
    pub const fn outcome(&self) -> RepeatContinuationDecision {
        self.outcome
    }

    /// Additional iterations authorized by an approval.
    #[must_use]
    pub const fn approved_additional_iterations(&self) -> Option<u32> {
        self.approved_additional_iterations
    }

    /// Bounded authority rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Supporting durable evidence references.
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceReference] {
        &self.evidence
    }

    /// Sequence at which the decision became durable.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Bounded continuation authority state for one repeat execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepeatContinuationProjection {
    repeat_execution: NodeExecutionId,
    initial_iteration_limit: u32,
    effective_iteration_limit: u32,
    budget_override_iteration_limit: Option<u32>,
    pending_approval: bool,
    rejected: bool,
    requests: Vec<RepeatContinuationRequestProjection>,
    decisions: Vec<RepeatContinuationDecisionProjection>,
}

impl RepeatContinuationProjection {
    /// Owning repeat execution.
    #[must_use]
    pub const fn repeat_execution(&self) -> &NodeExecutionId {
        &self.repeat_execution
    }

    /// Original configured iteration limit recorded by the first request.
    #[must_use]
    pub const fn initial_iteration_limit(&self) -> u32 {
        self.initial_iteration_limit
    }

    /// Absolute iteration limit after all recorded approvals.
    #[must_use]
    pub const fn effective_iteration_limit(&self) -> u32 {
        self.effective_iteration_limit
    }

    /// Exact frontier cap through which the latest budget approval permits progress.
    #[must_use]
    pub const fn budget_override_iteration_limit(&self) -> Option<u32> {
        self.budget_override_iteration_limit
    }

    /// Whether the current true-condition frontier needs another decision.
    #[must_use]
    pub const fn is_pending_approval(&self) -> bool {
        self.pending_approval
    }

    /// Whether authority rejected continuation and deterministic termination is pending.
    #[must_use]
    pub const fn is_rejected(&self) -> bool {
        self.rejected
    }

    /// Ordered immutable authority requests.
    #[must_use]
    pub fn requests(&self) -> &[RepeatContinuationRequestProjection] {
        &self.requests
    }

    /// Exact request currently awaiting authority, when present.
    #[must_use]
    pub fn pending_request(&self) -> Option<&RepeatContinuationRequestProjection> {
        if self.pending_approval {
            self.requests.last()
        } else {
            None
        }
    }

    /// Ordered immutable authority decisions.
    #[must_use]
    pub fn decisions(&self) -> &[RepeatContinuationDecisionProjection] {
        &self.decisions
    }
}

/// Terminal state of one explicit repeat execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepeatTermination {
    repeat_execution: NodeExecutionId,
    termination: RepeatTerminationReason,
    last_iteration: Option<IterationId>,
    sequence: RunSequence,
}

impl RepeatTermination {
    /// Owning repeat execution.
    #[must_use]
    pub const fn repeat_execution(&self) -> &NodeExecutionId {
        &self.repeat_execution
    }

    /// Deterministic termination classification.
    #[must_use]
    pub const fn termination(&self) -> RepeatTerminationReason {
        self.termination
    }

    /// Last created iteration, if any.
    #[must_use]
    pub const fn last_iteration(&self) -> Option<&IterationId> {
        self.last_iteration.as_ref()
    }

    /// Termination event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Current state of a parent-linked child subworkflow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubworkflowState {
    /// Child remains live.
    Active,
    /// Structured cancellation was propagated to an attached child.
    Cancelling,
    /// Parent observed the child terminal outcome.
    Terminal(RunOutcome),
}

/// One explicit immutable child-output import into the parent run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubworkflowOutputImport {
    child_value: WorkspaceValueReference,
    parent_value: WorkspaceValueReference,
    sequence: RunSequence,
}

impl SubworkflowOutputImport {
    /// Exact source value owned by the child run.
    #[must_use]
    pub const fn child_value(&self) -> &WorkspaceValueReference {
        &self.child_value
    }

    /// Exact immutable value introduced in the parent run.
    #[must_use]
    pub const fn parent_value(&self) -> &WorkspaceValueReference {
        &self.parent_value
    }

    /// Import event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Parent-local child-subworkflow read model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubworkflowProjection {
    subworkflow: SubworkflowId,
    parent_execution: NodeExecutionId,
    child_run: RunId,
    child_revision: RevisionId,
    scope: WorkspaceScope,
    ownership: SubworkflowOwnership,
    inputs: Vec<WorkspaceValueReference>,
    state: SubworkflowState,
    cancellation_reason: Option<Reason>,
    outputs: Vec<WorkspaceValueReference>,
    imports: Vec<SubworkflowOutputImport>,
}

impl SubworkflowProjection {
    /// Stable parent-local subworkflow identity.
    #[must_use]
    pub const fn subworkflow(&self) -> &SubworkflowId {
        &self.subworkflow
    }

    /// Parent node execution.
    #[must_use]
    pub const fn parent_execution(&self) -> &NodeExecutionId {
        &self.parent_execution
    }

    /// Exact child run aggregate.
    #[must_use]
    pub const fn child_run(&self) -> &RunId {
        &self.child_run
    }

    /// Exact pinned child revision.
    #[must_use]
    pub const fn child_revision(&self) -> &RevisionId {
        &self.child_revision
    }

    /// Child workspace scope declared by the parent.
    #[must_use]
    pub const fn scope(&self) -> &WorkspaceScope {
        &self.scope
    }

    /// Attached or explicitly detached ownership.
    #[must_use]
    pub const fn ownership(&self) -> SubworkflowOwnership {
        self.ownership
    }

    /// Exact child inputs.
    #[must_use]
    pub fn inputs(&self) -> &[WorkspaceValueReference] {
        &self.inputs
    }

    /// Current child state.
    #[must_use]
    pub const fn state(&self) -> SubworkflowState {
        self.state
    }

    /// Structured cancellation rationale, when present.
    #[must_use]
    pub const fn cancellation_reason(&self) -> Option<&Reason> {
        self.cancellation_reason.as_ref()
    }

    /// Exact bound outputs after child termination.
    #[must_use]
    pub fn outputs(&self) -> &[WorkspaceValueReference] {
        &self.outputs
    }

    /// Explicit child-to-parent immutable output imports.
    #[must_use]
    pub fn imports(&self) -> &[SubworkflowOutputImport] {
        &self.imports
    }

    /// Returns whether child liveness remains outstanding.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(
            self.state,
            SubworkflowState::Active | SubworkflowState::Cancelling
        )
    }

    /// Returns whether parent observed a terminal child result.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(self.state, SubworkflowState::Terminal(_))
    }
}

/// Durable signal and its exact consumption history.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalProjection {
    signal: SignalId,
    signal_type: SignalTypeId,
    correlation: Option<CorrelationKey>,
    mode: SignalDeliveryMode,
    payload: BoundedJson,
    received_sequence: RunSequence,
    consumed_by: BTreeSet<NodeExecutionId>,
    broadcast_scan_through: Option<NodeExecutionId>,
    broadcast_scan_complete: bool,
    duplicate_commands: Vec<CommandId>,
}

impl SignalProjection {
    /// Stable signal delivery identity.
    #[must_use]
    pub const fn signal(&self) -> &SignalId {
        &self.signal
    }

    /// Semantic payload type.
    #[must_use]
    pub const fn signal_type(&self) -> &SignalTypeId {
        &self.signal_type
    }

    /// Optional matching correlation identity.
    #[must_use]
    pub const fn correlation(&self) -> Option<&CorrelationKey> {
        self.correlation.as_ref()
    }

    /// Explicit delivery/consumption shape.
    #[must_use]
    pub const fn mode(&self) -> SignalDeliveryMode {
        self.mode
    }

    /// Bounded typed payload.
    #[must_use]
    pub const fn payload(&self) -> &BoundedJson {
        &self.payload
    }

    /// Sequence at which delivery became durable.
    #[must_use]
    pub const fn received_sequence(&self) -> RunSequence {
        self.received_sequence
    }

    /// Wait executions that consumed this signal exactly once.
    #[must_use]
    pub const fn consumed_by(&self) -> &BTreeSet<NodeExecutionId> {
        &self.consumed_by
    }

    /// Last ordered wait identity examined by the bounded broadcast drain.
    #[must_use]
    pub const fn broadcast_scan_through(&self) -> Option<&NodeExecutionId> {
        self.broadcast_scan_through.as_ref()
    }

    /// Returns whether every pre-receipt compatible wait has been examined.
    #[must_use]
    pub const fn broadcast_scan_complete(&self) -> bool {
        self.broadcast_scan_complete
    }

    /// Later command identities whose duplicate observations were recorded.
    #[must_use]
    pub fn duplicate_commands(&self) -> &[CommandId] {
        &self.duplicate_commands
    }

    /// Returns whether a one-shot delivery remains queued for one consumer.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.mode == SignalDeliveryMode::OneShot && self.consumed_by.is_empty()
    }

    /// Returns whether delivery is closed (immediately for broadcast).
    #[must_use]
    pub fn is_completed(&self) -> bool {
        !self.is_pending()
    }
}

/// Exact state of a durable wait node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaitProjection {
    execution: NodeExecutionId,
    condition: WaitCondition,
    registered_sequence: RunSequence,
    satisfaction: Option<WaitSatisfaction>,
    cancellation: Option<WaitCancellationProjection>,
}

/// Durable cancellation fact for a wait.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaitCancellationProjection {
    reason: Reason,
    sequence: RunSequence,
}

impl WaitCancellationProjection {
    /// Bounded causal cancellation rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Sequence at which cancellation became durable.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

impl WaitProjection {
    /// Waiting execution.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Frozen timer/signal condition.
    #[must_use]
    pub const fn condition(&self) -> &WaitCondition {
        &self.condition
    }

    /// Registration sequence.
    #[must_use]
    pub const fn registered_sequence(&self) -> RunSequence {
        self.registered_sequence
    }

    /// Exact recorded satisfaction cause, when completed.
    #[must_use]
    pub const fn satisfaction(&self) -> Option<&WaitSatisfaction> {
        self.satisfaction.as_ref()
    }

    /// Explicit cancellation fact, when present.
    #[must_use]
    pub const fn cancellation(&self) -> Option<&WaitCancellationProjection> {
        self.cancellation.as_ref()
    }

    /// Returns whether the wait remains pending.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.satisfaction.is_none() && self.cancellation.is_none()
    }

    /// Returns whether one cause satisfied the wait.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        self.satisfaction.is_some() || self.cancellation.is_some()
    }
}

/// State of one revision-adoption request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconciliationRequestState {
    /// Request exists but has no immutable plan yet.
    Requested,
    /// A plan was recorded.
    Planned,
    /// Its plan was applied and awaits or completed the exact repin fact.
    Applied,
    /// Authority rejected the plan.
    Rejected,
    /// Later run history invalidated the plan's exact projection boundary.
    Stale,
}

/// One prospective revision-adoption request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationRequestProjection {
    reconciliation: ReconciliationId,
    from_revision: RevisionId,
    to_revision: RevisionId,
    policy: ReconciliationPolicy,
    sequence: RunSequence,
    plan: Option<ReconciliationPlanId>,
    state: ReconciliationRequestState,
}

impl ReconciliationRequestProjection {
    /// Stable reconciliation request identity.
    #[must_use]
    pub const fn reconciliation(&self) -> &ReconciliationId {
        &self.reconciliation
    }

    /// Exact pin against which adoption was requested.
    #[must_use]
    pub const fn from_revision(&self) -> &RevisionId {
        &self.from_revision
    }

    /// Exact requested prospective revision.
    #[must_use]
    pub const fn to_revision(&self) -> &RevisionId {
        &self.to_revision
    }

    /// Explicit requested reconciliation policy.
    #[must_use]
    pub const fn policy(&self) -> ReconciliationPolicy {
        self.policy
    }

    /// Request event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }

    /// Immutable plan produced for this request, when present.
    #[must_use]
    pub const fn plan(&self) -> Option<&ReconciliationPlanId> {
        self.plan.as_ref()
    }

    /// Current request state.
    #[must_use]
    pub const fn state(&self) -> ReconciliationRequestState {
        self.state
    }

    /// Returns whether the request can still advance.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(
            self.state,
            ReconciliationRequestState::Requested | ReconciliationRequestState::Planned
        )
    }

    /// Returns whether the request was applied or rejected.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        matches!(
            self.state,
            ReconciliationRequestState::Applied
                | ReconciliationRequestState::Rejected
                | ReconciliationRequestState::Stale
        )
    }
}

/// One recorded authority decision over an immutable reconciliation plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationDecision {
    decision: ReconciliationDecisionId,
    actor: ActorRef,
    outcome: AuthorityDecision,
    reason: Reason,
    evidence: Vec<EvidenceReference>,
    sequence: RunSequence,
}

impl ReconciliationDecision {
    /// Stable decision identity.
    #[must_use]
    pub const fn decision(&self) -> &ReconciliationDecisionId {
        &self.decision
    }

    /// Actor authorizing the decision.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        &self.actor
    }

    /// Closed authority outcome.
    #[must_use]
    pub const fn outcome(&self) -> AuthorityDecision {
        self.outcome
    }

    /// Bounded rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Supporting evidence references.
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceReference] {
        &self.evidence
    }

    /// Decision event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Immutable prospective reconciliation plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationPlanProjection {
    reconciliation: ReconciliationId,
    plan: ReconciliationPlanId,
    from_revision: RevisionId,
    to_revision: RevisionId,
    based_on_sequence: RunSequence,
    items: Vec<ReconciliationItem>,
    decisions: Vec<ReconciliationDecision>,
    applied_sequence: Option<RunSequence>,
    stale_sequence: Option<RunSequence>,
}

impl ReconciliationPlanProjection {
    /// Owning adoption request.
    #[must_use]
    pub const fn reconciliation(&self) -> &ReconciliationId {
        &self.reconciliation
    }

    /// Stable immutable plan identity.
    #[must_use]
    pub const fn plan(&self) -> &ReconciliationPlanId {
        &self.plan
    }

    /// Exact old revision.
    #[must_use]
    pub const fn from_revision(&self) -> &RevisionId {
        &self.from_revision
    }

    /// Exact prospective revision.
    #[must_use]
    pub const fn to_revision(&self) -> &RevisionId {
        &self.to_revision
    }

    /// Historical sequence compared by the planner.
    #[must_use]
    pub const fn based_on_sequence(&self) -> RunSequence {
        self.based_on_sequence
    }

    /// Closed classifications and prospective actions.
    #[must_use]
    pub fn items(&self) -> &[ReconciliationItem] {
        &self.items
    }

    /// Authority decisions recorded over this plan.
    #[must_use]
    pub fn decisions(&self) -> &[ReconciliationDecision] {
        &self.decisions
    }

    /// Sequence at which the plan was applied, when present.
    #[must_use]
    pub const fn applied_sequence(&self) -> Option<RunSequence> {
        self.applied_sequence
    }

    /// First event sequence that invalidated the plan's exact base, when present.
    #[must_use]
    pub const fn stale_sequence(&self) -> Option<RunSequence> {
        self.stale_sequence
    }

    /// Returns whether the plan has not yet been applied.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.applied_sequence.is_none() && self.stale_sequence.is_none()
    }

    /// Returns whether the prospective application fact is durable.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        self.applied_sequence.is_some() || self.stale_sequence.is_some()
    }
}

/// Complete revision-reconciliation read model.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReconciliationProjection {
    requests: BTreeMap<ReconciliationId, ReconciliationRequestProjection>,
    plans: BTreeMap<ReconciliationPlanId, ReconciliationPlanProjection>,
    current_request: Option<ReconciliationId>,
}

impl ReconciliationProjection {
    /// All immutable adoption requests by identity.
    #[must_use]
    pub const fn requests(&self) -> &BTreeMap<ReconciliationId, ReconciliationRequestProjection> {
        &self.requests
    }

    /// All immutable plans by identity.
    #[must_use]
    pub const fn plans(&self) -> &BTreeMap<ReconciliationPlanId, ReconciliationPlanProjection> {
        &self.plans
    }

    /// Most recently active or completed request.
    #[must_use]
    pub fn current(&self) -> Option<&ReconciliationRequestProjection> {
        self.current_request
            .as_ref()
            .and_then(|identity| self.requests.get(identity))
    }

    /// Returns whether a reconciliation request or plan remains active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.current()
            .is_some_and(ReconciliationRequestProjection::is_active)
    }
}

/// One recovery classification attached to an attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryObservation {
    lease: Option<LeaseId>,
    classification: RecoveryClassification,
    reason: Reason,
    sequence: RunSequence,
}

impl RecoveryObservation {
    /// Lease classified by recovery, when one existed.
    #[must_use]
    pub const fn lease(&self) -> Option<&LeaseId> {
        self.lease.as_ref()
    }

    /// Truthful recorded recovery classification.
    #[must_use]
    pub const fn classification(&self) -> RecoveryClassification {
        self.classification
    }

    /// Bounded recovery rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Classification event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Operator/controller decision over uncertain or retained work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryDecision {
    decision: ReconciliationDecisionId,
    actor: ActorRef,
    outcome: AuthorityDecision,
    reason: Reason,
    evidence: Vec<EvidenceReference>,
    sequence: RunSequence,
}

impl RecoveryDecision {
    /// Stable decision identity.
    #[must_use]
    pub const fn decision(&self) -> &ReconciliationDecisionId {
        &self.decision
    }

    /// Actor authorizing the decision.
    #[must_use]
    pub const fn actor(&self) -> &ActorRef {
        &self.actor
    }

    /// Closed recovery outcome.
    #[must_use]
    pub const fn outcome(&self) -> AuthorityDecision {
        self.outcome
    }

    /// Bounded rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Supporting durable evidence references.
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceReference] {
        &self.evidence
    }

    /// Decision event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// One recovery-controller pass over an exact durable head.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryProjection {
    controller: WorkerId,
    through_sequence: RunSequence,
    started_sequence: RunSequence,
    classifications: Vec<(AttemptId, RecoveryObservation)>,
}

impl RecoveryProjection {
    /// Stable recovery controller identity.
    #[must_use]
    pub const fn controller(&self) -> &WorkerId {
        &self.controller
    }

    /// Exact journal head examined.
    #[must_use]
    pub const fn through_sequence(&self) -> RunSequence {
        self.through_sequence
    }

    /// Sequence of the recovery-start fact.
    #[must_use]
    pub const fn started_sequence(&self) -> RunSequence {
        self.started_sequence
    }

    /// Ordered attempt classifications in this recovery pass.
    #[must_use]
    pub fn classifications(&self) -> &[(AttemptId, RecoveryObservation)] {
        &self.classifications
    }
}

/// Attempt-local cancellation intent authorized by an immutable reconciliation plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationCancellationProjection {
    plan: ReconciliationPlanId,
    execution: NodeExecutionId,
    attempt: AttemptId,
    reason: Reason,
    sequence: RunSequence,
}

impl ReconciliationCancellationProjection {
    /// Authorizing immutable plan.
    #[must_use]
    pub const fn plan(&self) -> &ReconciliationPlanId {
        &self.plan
    }

    /// Logical execution receiving cancellation intent.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Exact active attempt receiving cancellation intent.
    #[must_use]
    pub const fn attempt(&self) -> &AttemptId {
        &self.attempt
    }

    /// Bounded deterministic rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Cancellation-intent event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Plan-created remediation execution preserving its exact source truth.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationRemediationProjection {
    plan: ReconciliationPlanId,
    source_execution: NodeExecutionId,
    source_attempt: Option<AttemptId>,
    execution: NodeExecutionId,
    node: NodeId,
    scope: ScopeReference,
    reason: Reason,
    sequence: RunSequence,
}

impl ReconciliationRemediationProjection {
    /// Authorizing immutable plan.
    #[must_use]
    pub const fn plan(&self) -> &ReconciliationPlanId {
        &self.plan
    }

    /// Existing execution whose truth is retained.
    #[must_use]
    pub const fn source_execution(&self) -> &NodeExecutionId {
        &self.source_execution
    }

    /// Exact source attempt, when one existed.
    #[must_use]
    pub const fn source_attempt(&self) -> Option<&AttemptId> {
        self.source_attempt.as_ref()
    }

    /// New independently scheduled remediation execution.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Target semantic node under the adopted revision.
    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }

    /// Durable workspace scope of the remediation execution.
    #[must_use]
    pub const fn scope(&self) -> &ScopeReference {
        &self.scope
    }

    /// Bounded plan rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Creation event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Authority-created remediation relationship preserving the source attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemediationProjection {
    source_attempt: AttemptId,
    execution: NodeExecutionId,
    node: NodeId,
    scope: ScopeReference,
    mode: NodeExecutionMode,
    decision: ReconciliationDecisionId,
    reason: Reason,
    sequence: RunSequence,
}

impl RemediationProjection {
    /// Prior attempt whose truth needs remediation.
    #[must_use]
    pub const fn source_attempt(&self) -> &AttemptId {
        &self.source_attempt
    }

    /// New logical execution created for remediation.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Exact target node for the independently created remediation execution.
    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }

    /// Durable workspace scope inherited from the source execution.
    #[must_use]
    pub const fn scope(&self) -> &ScopeReference {
        &self.scope
    }

    /// Closed runtime/executor ownership of the remediation execution.
    #[must_use]
    pub const fn mode(&self) -> NodeExecutionMode {
        self.mode
    }

    /// Authority decision permitting remediation.
    #[must_use]
    pub const fn decision(&self) -> &ReconciliationDecisionId {
        &self.decision
    }

    /// Bounded rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Remediation creation sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Aggregate resource and durable workspace-budget usage visible from event facts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceUsage {
    input_units: Option<u64>,
    output_units: Option<u64>,
    duration_ms: Option<u64>,
    cost_micros: BTreeMap<CurrencyCode, u64>,
    workspace_value_references: u64,
    artifacts: u64,
    artifact_bytes: u64,
}

impl ResourceUsage {
    /// Sum of observed provider-defined input units.
    #[must_use]
    pub const fn input_units(&self) -> Option<u64> {
        self.input_units
    }

    /// Sum of observed provider-defined output units.
    #[must_use]
    pub const fn output_units(&self) -> Option<u64> {
        self.output_units
    }

    /// Sum of observed executor durations.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// Exact observed cost totals grouped by currency.
    #[must_use]
    pub const fn cost_micros(&self) -> &BTreeMap<CurrencyCode, u64> {
        &self.cost_micros
    }

    /// Number of distinct workspace value references carried by history.
    #[must_use]
    pub const fn workspace_value_references(&self) -> u64 {
        self.workspace_value_references
    }

    /// Number of uniquely published artifact metadata records.
    #[must_use]
    pub const fn artifacts(&self) -> u64 {
        self.artifacts
    }

    /// Sum of exact bytes across uniquely published artifacts.
    #[must_use]
    pub const fn artifact_bytes(&self) -> u64 {
        self.artifact_bytes
    }
}

/// Truthful terminal output summary with references into published provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunTerminalProjection {
    outcome: RunOutcome,
    outputs: Vec<WorkspaceValueReference>,
    artifacts: Vec<ArtifactReference>,
    reason: Option<Reason>,
    sequence: RunSequence,
}

impl RunTerminalProjection {
    /// Semantic terminal outcome.
    #[must_use]
    pub const fn outcome(&self) -> RunOutcome {
        self.outcome
    }

    /// Exact terminal workspace values.
    #[must_use]
    pub fn outputs(&self) -> &[WorkspaceValueReference] {
        &self.outputs
    }

    /// Exact terminal content-addressed artifacts.
    #[must_use]
    pub fn artifacts(&self) -> &[ArtifactReference] {
        &self.artifacts
    }

    /// Bounded terminal rationale, when relevant.
    #[must_use]
    pub const fn reason(&self) -> Option<&Reason> {
        self.reason.as_ref()
    }

    /// Terminal event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Pure read model obtained by replaying one run's ordered authoritative facts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunProjection {
    sequence: RunSequence,
    run_id: Option<RunId>,
    lifecycle: RunLifecycle,
    workflow: Option<WorkflowId>,
    revision: Option<RevisionId>,
    revision_digest: Option<ContentDigest>,
    pins: Vec<RevisionPin>,
    root_scope: Option<WorkspaceScope>,
    workspace_budget: Option<WorkspaceBudget>,
    inputs: Vec<WorkspaceValueReference>,
    scopes: BTreeMap<ScopeReference, WorkspaceScope>,
    workspace_values: BTreeSet<WorkspaceValueReference>,
    event_ids: BTreeSet<EventId>,
    cancellation: Option<RunCancellation>,
    termination: Option<RunTerminationIntent>,
    node_executions: BTreeMap<NodeExecutionId, NodeExecutionProjection>,
    execution_ids_by_node: BTreeMap<NodeId, BTreeSet<NodeExecutionId>>,
    latest_descendant_execution_by_scope_node:
        BTreeMap<(ScopeReference, NodeId), NodeExecutionId>,
    active_execution_ids: BTreeSet<NodeExecutionId>,
    eligible_executions: BTreeSet<NodeExecutionId>,
    pending_successor_executions: BTreeSet<NodeExecutionId>,
    reserved_executions: BTreeSet<NodeExecutionId>,
    attempts: BTreeMap<AttemptId, NodeAttemptProjection>,
    active_attempt_ids: BTreeSet<AttemptId>,
    invocations: BTreeSet<InvocationId>,
    leases: BTreeMap<LeaseId, LeaseProjection>,
    active_lease_by_attempt: BTreeMap<AttemptId, LeaseId>,
    timers: BTreeMap<TimerId, TimerProjection>,
    pending_timer_ids: BTreeSet<TimerId>,
    pending_timers_by_execution: BTreeMap<NodeExecutionId, BTreeSet<TimerId>>,
    retries: BTreeMap<TimerId, RetryProjection>,
    retry_by_attempt: BTreeMap<AttemptId, TimerId>,
    branches: BTreeMap<BranchId, BranchProjection>,
    branch_by_fork_port: BTreeMap<(NodeExecutionId, PortId), BranchId>,
    branch_ids_by_fork_execution: BTreeMap<NodeExecutionId, BTreeSet<BranchId>>,
    active_branch_ids: BTreeSet<BranchId>,
    cancelling_branch_ids: BTreeSet<BranchId>,
    active_scope_ownership: BTreeMap<ScopeReference, u64>,
    active_structured_children_by_execution: BTreeMap<NodeExecutionId, u32>,
    branch_owner: BTreeMap<NodeExecutionId, BranchId>,
    branch_routes: BTreeMap<NodeExecutionId, PortId>,
    joins: BTreeMap<NodeExecutionId, JoinProjection>,
    iterations: BTreeMap<IterationId, IterationProjection>,
    active_iteration_ids: BTreeSet<IterationId>,
    latest_iteration: BTreeMap<NodeExecutionId, IterationId>,
    repeat_continuations: BTreeMap<NodeExecutionId, RepeatContinuationProjection>,
    repeat_decision_ids: BTreeSet<RepeatDecisionId>,
    repeat_terminations: BTreeMap<NodeExecutionId, RepeatTermination>,
    signals: BTreeMap<SignalId, SignalProjection>,
    pending_broadcast_signals: BTreeSet<(RunSequence, SignalId)>,
    pending_one_shot_signals:
        BTreeMap<(SignalTypeId, Option<CorrelationKey>), BTreeSet<(RunSequence, SignalId)>>,
    signal_duplicate_commands: BTreeSet<CommandId>,
    waits: BTreeMap<NodeExecutionId, WaitProjection>,
    pending_wait_execution_ids: BTreeSet<NodeExecutionId>,
    pending_signal_waits:
        BTreeMap<(SignalTypeId, Option<CorrelationKey>), BTreeSet<NodeExecutionId>>,
    subworkflows: BTreeMap<SubworkflowId, SubworkflowProjection>,
    active_subworkflow_ids: BTreeSet<SubworkflowId>,
    active_attached_subworkflow_ids: BTreeSet<SubworkflowId>,
    child_runs: BTreeSet<RunId>,
    artifacts: BTreeMap<ArtifactId, ArtifactMetadata>,
    reconciliation: ReconciliationProjection,
    pending_pin: Option<ReconciliationPlanId>,
    reconciliation_cancellations: BTreeMap<NodeExecutionId, ReconciliationCancellationProjection>,
    pending_reconciliation_restarts:
        BTreeMap<(NodeId, ScopeReference), NodeExecutionId>,
    reconciliation_remediations: BTreeMap<NodeExecutionId, ReconciliationRemediationProjection>,
    decision_ids: BTreeSet<ReconciliationDecisionId>,
    recovery_decisions: BTreeMap<ReconciliationDecisionId, (AttemptId, AuthorityDecision)>,
    recovery: Vec<RecoveryProjection>,
    current_recovery: Option<usize>,
    remediations: BTreeMap<NodeExecutionId, RemediationProjection>,
    resource_usage: ResourceUsage,
    terminal: Option<RunTerminalProjection>,
}

impl RunProjection {
    /// Creates an empty, uncreated projection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replays a complete ordered history without consulting any external state.
    pub fn replay(events: &[RunEventEnvelope]) -> Result<Self, RuntimeError> {
        let mut projection = Self::new();
        for event in events {
            projection.apply_in_place(event)?;
        }
        Ok(projection)
    }

    /// Applies one replay event without cloning an intermediate projection.
    ///
    /// This is crate-private because partial mutation on failure is suitable only
    /// while constructing a disposable projection in the paged history fold.
    pub(crate) fn apply_replayed(&mut self, event: &RunEventEnvelope) -> Result<(), RuntimeError> {
        self.apply_in_place(event)
    }

    /// Applies one next event atomically to this projection.
    ///
    /// On failure `self` remains unchanged, allowing callers to retain a previously
    /// verified journal prefix.
    pub fn apply(&mut self, event: &RunEventEnvelope) -> Result<(), RuntimeError> {
        let mut candidate = self.clone();
        candidate.apply_in_place(event)?;
        *self = candidate;
        Ok(())
    }

    /// Authoritative sequence covered by this projection.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }

    /// Aggregate identity, absent only for empty history.
    #[must_use]
    pub const fn run_id(&self) -> Option<&RunId> {
        self.run_id.as_ref()
    }

    /// Current derived run lifecycle.
    #[must_use]
    pub const fn lifecycle(&self) -> RunLifecycle {
        self.lifecycle
    }

    /// Workflow lineage, absent only for empty history.
    #[must_use]
    pub const fn workflow(&self) -> Option<&WorkflowId> {
        self.workflow.as_ref()
    }

    /// Current exact revision pin, absent only for empty history.
    #[must_use]
    pub const fn revision(&self) -> Option<&RevisionId> {
        self.revision.as_ref()
    }

    /// Current semantic revision digest.
    #[must_use]
    pub const fn revision_digest(&self) -> Option<&ContentDigest> {
        self.revision_digest.as_ref()
    }

    /// Complete immutable pin timeline.
    #[must_use]
    pub fn pins(&self) -> &[RevisionPin] {
        &self.pins
    }

    /// Root workspace scope, absent only for empty history.
    #[must_use]
    pub const fn root_scope(&self) -> Option<&WorkspaceScope> {
        self.root_scope.as_ref()
    }

    /// Immutable workspace budget pinned at run creation.
    #[must_use]
    pub const fn workspace_budget(&self) -> Option<&WorkspaceBudget> {
        self.workspace_budget.as_ref()
    }

    /// Exact bounded run input references.
    #[must_use]
    pub fn inputs(&self) -> &[WorkspaceValueReference] {
        &self.inputs
    }

    /// Every durable workspace scope declared by run history.
    #[must_use]
    pub const fn scopes(&self) -> &BTreeMap<ScopeReference, WorkspaceScope> {
        &self.scopes
    }

    /// Distinct immutable workspace value references carried by projected facts.
    #[must_use]
    pub const fn workspace_values(&self) -> &BTreeSet<WorkspaceValueReference> {
        &self.workspace_values
    }

    /// Durable aggregate cancellation intent, when present.
    #[must_use]
    pub const fn cancellation(&self) -> Option<&RunCancellation> {
        self.cancellation.as_ref()
    }

    /// Durable explicit-terminal drain intent, when present.
    #[must_use]
    pub const fn termination(&self) -> Option<&RunTerminationIntent> {
        self.termination.as_ref()
    }

    /// All logical node executions keyed by stable identity.
    #[must_use]
    pub const fn node_executions(&self) -> &BTreeMap<NodeExecutionId, NodeExecutionProjection> {
        &self.node_executions
    }

    /// Logical executions that have not reached a closed terminal/removed state.
    #[must_use]
    pub(crate) const fn active_execution_ids(&self) -> &BTreeSet<NodeExecutionId> {
        &self.active_execution_ids
    }

    /// Direct structured branch that owns an execution, when present.
    #[must_use]
    pub fn branch_for_execution(
        &self,
        execution: &NodeExecutionId,
    ) -> Option<&BranchProjection> {
        self.branch_owner
            .get(execution)
            .and_then(|branch| self.branches.get(branch))
    }

    /// Currently eligible logical executions, without historical terminal entries.
    #[must_use]
    pub const fn eligible_execution_ids(&self) -> &BTreeSet<NodeExecutionId> {
        &self.eligible_executions
    }

    /// Successful executions whose prospective successors have not been examined.
    #[must_use]
    pub const fn pending_successor_execution_ids(&self) -> &BTreeSet<NodeExecutionId> {
        &self.pending_successor_executions
    }

    /// Branch identities that still own active or cancelling structured work.
    #[must_use]
    pub const fn active_branch_ids(&self) -> &BTreeSet<BranchId> {
        &self.active_branch_ids
    }

    /// Active branch identities with durable cancellation intent.
    #[must_use]
    pub(crate) const fn cancelling_branch_ids(&self) -> &BTreeSet<BranchId> {
        &self.cancelling_branch_ids
    }

    /// Immutable attempts whose truth still owns scheduling or recovery work.
    #[must_use]
    pub(crate) const fn active_attempt_ids(&self) -> &BTreeSet<AttemptId> {
        &self.active_attempt_ids
    }

    /// Pending timers owned directly by one logical execution.
    #[must_use]
    pub(crate) fn pending_timers_for_execution(
        &self,
        execution: &NodeExecutionId,
    ) -> impl Iterator<Item = &TimerId> {
        self.pending_timers_by_execution
            .get(execution)
            .into_iter()
            .flat_map(BTreeSet::iter)
    }

    /// Attached/detached child records whose child terminal fact is not yet observed.
    #[must_use]
    pub(crate) const fn active_subworkflow_ids(&self) -> &BTreeSet<SubworkflowId> {
        &self.active_subworkflow_ids
    }

    /// Exact node/scope restart tokens keyed for bounded deterministic paging.
    #[must_use]
    pub(crate) const fn pending_reconciliation_restarts(
        &self,
    ) -> &BTreeMap<(NodeId, ScopeReference), NodeExecutionId> {
        &self.pending_reconciliation_restarts
    }

    /// Returns whether any durable run-owned work or required restart remains open.
    #[must_use]
    pub(crate) fn has_active_owned_work(&self) -> bool {
        !self.active_execution_ids.is_empty()
            || !self.active_attempt_ids.is_empty()
            || !self.active_lease_by_attempt.is_empty()
            || !self.active_branch_ids.is_empty()
            || !self.active_iteration_ids.is_empty()
            || !self.pending_timer_ids.is_empty()
            || !self.pending_wait_execution_ids.is_empty()
            || !self.active_attached_subworkflow_ids.is_empty()
            || self.reconciliation.is_active()
            || !self.pending_reconciliation_restarts.is_empty()
            || self.pending_pin.is_some()
            || !self.reserved_executions.is_empty()
    }

    /// Exact active lease for an attempt, without scanning historical leases.
    #[must_use]
    pub(crate) fn active_lease_for_attempt(
        &self,
        attempt: &AttemptId,
    ) -> Option<&LeaseProjection> {
        self.active_lease_by_attempt
            .get(attempt)
            .and_then(|lease| self.leases.get(lease))
    }

    /// Every execution of one stable semantic node in stable identity order.
    pub fn executions_for_node<'a>(
        &'a self,
        node: &'a NodeId,
    ) -> impl Iterator<Item = &'a NodeExecutionProjection> + 'a {
        self.execution_ids_by_node
            .get(node)
            .into_iter()
            .flat_map(BTreeSet::iter)
            .filter_map(|execution| self.node_executions.get(execution))
    }

    /// Latest execution of a node in one scope or any descendant scope.
    #[must_use]
    pub(crate) fn latest_descendant_execution(
        &self,
        scope: &ScopeReference,
        node: &NodeId,
    ) -> Option<&NodeExecutionProjection> {
        self.latest_descendant_execution_by_scope_node
            .get(&(scope.clone(), node.clone()))
            .and_then(|execution| self.node_executions.get(execution))
    }

    /// All immutable attempts keyed by identity.
    #[must_use]
    pub const fn attempts(&self) -> &BTreeMap<AttemptId, NodeAttemptProjection> {
        &self.attempts
    }

    /// Exact immutable workflow revision that governed one scheduled attempt.
    ///
    /// Returns `None` for an unknown attempt or a retry identity whose backoff
    /// timer has not yet admitted a durable executor request.
    #[must_use]
    pub fn revision_for_attempt(&self, attempt: &AttemptId) -> Option<&RevisionId> {
        self.attempts
            .get(attempt)
            .and_then(NodeAttemptProjection::scheduled_sequence)
            .and_then(|sequence| self.revision_at(sequence))
    }

    /// Attempts with uncertain or explicitly retained external truth.
    pub fn unresolved_attempts(&self) -> impl Iterator<Item = &NodeAttemptProjection> {
        self.attempts
            .values()
            .filter(|attempt| attempt.is_unresolved())
    }

    /// Every durable lease, including expired, superseded, and completed records.
    #[must_use]
    pub const fn leases(&self) -> &BTreeMap<LeaseId, LeaseProjection> {
        &self.leases
    }

    /// Every durable timer, including fired records.
    #[must_use]
    pub const fn timers(&self) -> &BTreeMap<TimerId, TimerProjection> {
        &self.timers
    }

    /// Every immutable retry decision keyed by its timer.
    #[must_use]
    pub const fn retries(&self) -> &BTreeMap<TimerId, RetryProjection> {
        &self.retries
    }

    /// All structured branches.
    #[must_use]
    pub const fn branches(&self) -> &BTreeMap<BranchId, BranchProjection> {
        &self.branches
    }

    /// Exact branch previously expanded for one fork execution/port pair.
    #[must_use]
    pub(crate) fn branch_for_fork_port(
        &self,
        fork: &NodeExecutionId,
        port: &PortId,
    ) -> Option<&BranchProjection> {
        self.branch_by_fork_port
            .get(&(fork.clone(), port.clone()))
            .and_then(|branch| self.branches.get(branch))
    }

    /// Branches owned by one immutable fork occurrence.
    pub(crate) fn branches_for_fork<'a>(
        &'a self,
        fork: &'a NodeExecutionId,
    ) -> impl Iterator<Item = &'a BranchProjection> + 'a {
        self.branch_ids_by_fork_execution
            .get(fork)
            .into_iter()
            .flat_map(BTreeSet::iter)
            .filter_map(|branch| self.branches.get(branch))
    }

    /// Frozen branch/router port selections keyed by routing execution.
    #[must_use]
    pub const fn branch_routes(&self) -> &BTreeMap<NodeExecutionId, PortId> {
        &self.branch_routes
    }

    /// All satisfied joins keyed by join execution.
    #[must_use]
    pub const fn joins(&self) -> &BTreeMap<NodeExecutionId, JoinProjection> {
        &self.joins
    }

    /// All isolated repeat iterations.
    #[must_use]
    pub const fn iterations(&self) -> &BTreeMap<IterationId, IterationProjection> {
        &self.iterations
    }

    /// Bounded continuation decisions keyed by repeat execution.
    #[must_use]
    pub const fn repeat_continuations(
        &self,
    ) -> &BTreeMap<NodeExecutionId, RepeatContinuationProjection> {
        &self.repeat_continuations
    }

    /// All terminal repeat facts keyed by repeat execution.
    #[must_use]
    pub const fn repeat_terminations(&self) -> &BTreeMap<NodeExecutionId, RepeatTermination> {
        &self.repeat_terminations
    }

    /// All received durable signals.
    #[must_use]
    pub const fn signals(&self) -> &BTreeMap<SignalId, SignalProjection> {
        &self.signals
    }

    /// Ordered broadcast receipts whose bounded wait-catalog scan is incomplete.
    pub(crate) const fn pending_broadcast_signals(
        &self,
    ) -> &BTreeSet<(RunSequence, SignalId)> {
        &self.pending_broadcast_signals
    }

    /// Earliest pending wait compatible with one exact signal identity.
    pub(crate) fn earliest_pending_signal_wait(
        &self,
        signal_type: &SignalTypeId,
        correlation: Option<&CorrelationKey>,
    ) -> Option<&NodeExecutionId> {
        self.pending_signal_waits
            .get(&(signal_type.clone(), correlation.cloned()))
            .and_then(BTreeSet::first)
    }

    /// Earliest durable queued one-shot signal compatible with one wait.
    pub(crate) fn earliest_pending_one_shot_signal(
        &self,
        condition: &WaitCondition,
    ) -> Option<&SignalId> {
        let key = signal_match_key(condition)?;
        self.pending_one_shot_signals
            .get(&key)
            .and_then(BTreeSet::first)
            .map(|(_, signal)| signal)
    }

    /// All registered wait conditions keyed by execution.
    #[must_use]
    pub const fn waits(&self) -> &BTreeMap<NodeExecutionId, WaitProjection> {
        &self.waits
    }

    /// All parent-linked child subworkflows.
    #[must_use]
    pub const fn subworkflows(&self) -> &BTreeMap<SubworkflowId, SubworkflowProjection> {
        &self.subworkflows
    }

    /// Returns whether an execution still owns live structured runtime state.
    ///
    /// Structured nodes deliberately remain `Eligible` while their durable
    /// wait, child, iteration, or branch frontier is active. Callers must not
    /// mistake that executor-oriented state for "never started".
    #[must_use]
    pub fn execution_has_active_structured_ownership(&self, execution: &NodeExecutionId) -> bool {
        self.waits
            .get(execution)
            .is_some_and(WaitProjection::is_pending)
            || self.pending_timers_by_execution.contains_key(execution)
            || self
                .active_structured_children_by_execution
                .contains_key(execution)
            || self.joins.contains_key(execution)
            || self
                .branch_owner
                .get(execution)
                .is_some_and(|branch| self.active_branch_ids.contains(branch))
    }

    /// Returns whether a structured child aggregate must drain before its owner.
    #[must_use]
    pub(crate) fn execution_has_active_child_ownership(
        &self,
        execution: &NodeExecutionId,
    ) -> bool {
        self.active_structured_children_by_execution
            .contains_key(execution)
    }

    /// Returns whether a branch scope still contains live nested structured ownership.
    ///
    /// Direct child completion is insufficient for a nested fork because the fork
    /// execution becomes terminal before its child branches and join frontier do.
    pub(crate) fn branch_has_active_descendant_ownership(&self, branch: &BranchId) -> bool {
        let Some(owner) = self.branches.get(branch) else {
            return false;
        };
        // The branch itself owns one token at its isolated scope. Every active
        // execution, nested branch, or pending reconciliation replacement below
        // that scope contributes another token propagated through its ancestors.
        self.active_scope_ownership
            .get(owner.scope.reference())
            .is_some_and(|count| *count > 1)
    }

    /// Published artifact metadata and causal provenance keyed by logical artifact ID.
    #[must_use]
    pub const fn artifacts(&self) -> &BTreeMap<ArtifactId, ArtifactMetadata> {
        &self.artifacts
    }

    /// Complete prospective revision-reconciliation read model.
    #[must_use]
    pub const fn reconciliation(&self) -> &ReconciliationProjection {
        &self.reconciliation
    }

    /// Attempt-local cancellation intents enacted by reconciliation plans.
    #[must_use]
    pub const fn reconciliation_cancellations(
        &self,
    ) -> &BTreeMap<NodeExecutionId, ReconciliationCancellationProjection> {
        &self.reconciliation_cancellations
    }

    /// Plan-created remediation executions preserving source history.
    #[must_use]
    pub const fn reconciliation_remediations(
        &self,
    ) -> &BTreeMap<NodeExecutionId, ReconciliationRemediationProjection> {
        &self.reconciliation_remediations
    }

    /// Recovery-controller passes over exact durable heads.
    #[must_use]
    pub fn recovery(&self) -> &[RecoveryProjection] {
        &self.recovery
    }

    /// Authority-created remediation relationships.
    #[must_use]
    pub const fn remediations(&self) -> &BTreeMap<NodeExecutionId, RemediationProjection> {
        &self.remediations
    }

    /// Current aggregate provider and workspace-budget usage visible from events.
    #[must_use]
    pub const fn resource_usage(&self) -> &ResourceUsage {
        &self.resource_usage
    }

    /// Truthful terminal output summary, when present.
    #[must_use]
    pub const fn terminal(&self) -> Option<&RunTerminalProjection> {
        self.terminal.as_ref()
    }

    /// Returns whether the run exists but is not started.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.lifecycle.is_pending()
    }

    /// Returns whether the run is started and nonterminal.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.lifecycle.is_active()
    }

    /// Returns whether the run reached a terminal boundary.
    #[must_use]
    pub const fn is_completed(&self) -> bool {
        self.lifecycle.is_completed()
    }

    fn apply_in_place(&mut self, event: &RunEventEnvelope) -> Result<(), RuntimeError> {
        let expected = self
            .sequence
            .get()
            .checked_add(1)
            .ok_or_else(|| invalid("run sequence overflow"))?;
        if event.sequence().get() != expected {
            return Err(invalid_at(
                event,
                format!(
                    "expected contiguous sequence {expected}, found {}",
                    event.sequence()
                ),
            ));
        }
        if self.event_ids.contains(event.event_id()) {
            return Err(invalid_at(event, "duplicate event identity"));
        }
        match &self.run_id {
            Some(run) if run != event.run_id() => {
                return Err(invalid_at(
                    event,
                    "history contains more than one run aggregate",
                ));
            }
            None if !matches!(event.kind(), RunEventKind::RunCreated { .. }) => {
                return Err(invalid_at(event, "the first event must create the run"));
            }
            Some(_) | None => {}
        }
        if self.pending_pin.is_some()
            && !matches!(event.kind(), RunEventKind::RevisionPinned { .. })
        {
            return Err(invalid_at(
                event,
                "reconciliation application must be followed immediately by its revision pin",
            ));
        }
        if self.lifecycle.is_completed() && !self.event_is_safe_after_terminal(event.kind()) {
            return Err(invalid_at(
                event,
                "event is not safe after the run terminal boundary",
            ));
        }

        self.apply_kind(event)?;
        self.mark_reconciliation_staleness(event);
        let inserted = self.event_ids.insert(event.event_id().clone());
        if !inserted {
            return Err(invalid_at(event, "duplicate event identity"));
        }
        self.sequence = event.sequence();
        Ok(())
    }

    fn event_is_safe_after_terminal(&self, kind: &RunEventKind) -> bool {
        match kind {
            RunEventKind::RecoveryStarted { .. }
            | RunEventKind::RecoveryClassified { .. }
            | RunEventKind::ExternalOutcomeRetained { .. } => true,
            RunEventKind::RecoveryDecisionRecorded { outcome, .. } => matches!(
                outcome,
                AuthorityDecision::Retain
                    | AuthorityDecision::ResolveSucceeded
                    | AuthorityDecision::ResolveFailed
            ),
            RunEventKind::SubworkflowTerminal { subworkflow, .. } => self
                .subworkflows
                .get(subworkflow)
                .is_some_and(|child| child.ownership == SubworkflowOwnership::Detached),
            _ => false,
        }
    }

    fn mark_reconciliation_staleness(&mut self, event: &RunEventEnvelope) {
        let mut stale_requests = Vec::new();
        for plan in self.reconciliation.plans.values_mut() {
            if plan.applied_sequence.is_some()
                || plan.stale_sequence.is_some()
                || event.sequence() <= plan.based_on_sequence
            {
                continue;
            }
            let belongs_to_plan = match event.kind() {
                RunEventKind::RevisionAdoptionRequested { reconciliation, .. } => {
                    reconciliation == &plan.reconciliation
                }
                RunEventKind::ReconciliationPlanRecorded {
                    reconciliation,
                    plan: event_plan,
                    ..
                } => reconciliation == &plan.reconciliation && event_plan == &plan.plan,
                RunEventKind::ReconciliationDecisionRecorded {
                    plan: event_plan, ..
                }
                | RunEventKind::ReconciliationApplied {
                    plan: event_plan, ..
                }
                | RunEventKind::ReconciliationExecutionRemoved {
                    plan: event_plan, ..
                }
                | RunEventKind::ReconciliationCancellationRequested {
                    plan: event_plan, ..
                }
                | RunEventKind::ReconciliationRemediationCreated {
                    plan: event_plan, ..
                } => event_plan == &plan.plan,
                _ => false,
            };
            if !belongs_to_plan {
                plan.stale_sequence = Some(event.sequence());
                stale_requests.push(plan.reconciliation.clone());
            }
        }
        for reconciliation in stale_requests {
            if let Some(request) = self.reconciliation.requests.get_mut(&reconciliation) {
                if request.state == ReconciliationRequestState::Planned {
                    request.state = ReconciliationRequestState::Stale;
                }
            }
        }
    }
}

impl RunProjection {
    #[allow(clippy::too_many_lines)]
    fn apply_kind(&mut self, event: &RunEventEnvelope) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::RunCreated {
                workflow,
                revision,
                revision_digest,
                root_scope,
                workspace_budget,
                inputs,
            } => {
                if self.lifecycle != RunLifecycle::Uncreated {
                    return Err(invalid_at(event, "run creation may occur exactly once"));
                }
                if root_scope.reference().run() != event.run_id()
                    || !root_scope.kind().is_run_root()
                    || root_scope.parent().is_some()
                {
                    return Err(invalid_at(
                        event,
                        "run creation requires a parentless root scope owned by the envelope run",
                    ));
                }
                ensure_unique(inputs, event, "run input reference")?;
                if u64::try_from(inputs.len())
                    .map_err(|_| invalid_at(event, "input count overflow"))?
                    > workspace_budget.max_value_versions()
                {
                    return Err(invalid_at(
                        event,
                        "run inputs exceed the workspace value budget",
                    ));
                }
                for input in inputs {
                    if input.scope() != root_scope.reference() {
                        return Err(invalid_at(
                            event,
                            "every run input must belong to the root scope",
                        ));
                    }
                }
                self.run_id = Some(event.run_id().clone());
                self.lifecycle = RunLifecycle::Created;
                self.workflow = Some(workflow.clone());
                self.revision = Some(revision.clone());
                self.revision_digest = Some(revision_digest.clone());
                self.pins.push(RevisionPin {
                    revision: revision.clone(),
                    digest: revision_digest.clone(),
                    effective_sequence: sequence,
                    plan: None,
                });
                self.root_scope = Some(root_scope.clone());
                self.workspace_budget = Some(workspace_budget.clone());
                self.inputs = inputs.clone();
                self.scopes
                    .insert(root_scope.reference().clone(), root_scope.clone());
                for input in inputs {
                    self.record_workspace_value(input, event)?;
                }
            }
            RunEventKind::RevisionPinned {
                previous,
                revision,
                revision_digest,
                plan,
            } => {
                if self.pending_pin.as_ref() != Some(plan)
                    || self.revision.as_ref() != Some(previous)
                    || previous == revision
                {
                    return Err(invalid_at(
                        event,
                        "revision pin does not match the immediately preceding applied plan",
                    ));
                }
                let recorded =
                    self.reconciliation.plans.get(plan).ok_or_else(|| {
                        invalid_at(event, "revision pin references an unknown plan")
                    })?;
                if recorded.from_revision != *previous || recorded.to_revision != *revision {
                    return Err(invalid_at(
                        event,
                        "revision pin differs from its immutable plan",
                    ));
                }
                let completed_to_reconsider: Vec<_> = recorded
                    .items
                    .iter()
                    .filter_map(|item| item.execution.as_ref())
                    .filter(|execution| {
                        self.node_executions.get(*execution).is_some_and(|execution| {
                            execution.state
                                == NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                        })
                    })
                    .cloned()
                    .collect();
                self.revision = Some(revision.clone());
                self.revision_digest = Some(revision_digest.clone());
                self.pins.push(RevisionPin {
                    revision: revision.clone(),
                    digest: revision_digest.clone(),
                    effective_sequence: sequence,
                    plan: Some(plan.clone()),
                });
                self.pending_successor_executions
                    .extend(completed_to_reconsider);
                self.pending_pin = None;
            }
            RunEventKind::RunStarted => {
                if self.lifecycle != RunLifecycle::Created {
                    return Err(invalid_at(event, "only a created run may start"));
                }
                self.lifecycle = RunLifecycle::Running;
            }
            RunEventKind::RunPaused { .. } => {
                if self.lifecycle != RunLifecycle::Running {
                    return Err(invalid_at(event, "only a running run may pause"));
                }
                self.lifecycle = RunLifecycle::Paused;
            }
            RunEventKind::RunResumed { .. } => {
                if self.lifecycle != RunLifecycle::Paused {
                    return Err(invalid_at(event, "only a paused run may resume"));
                }
                self.lifecycle = RunLifecycle::Running;
            }
            RunEventKind::RunCancellationRequested { reason, evidence } => {
                if !matches!(
                    self.lifecycle,
                    RunLifecycle::Created | RunLifecycle::Running | RunLifecycle::Paused
                ) || self.cancellation.is_some()
                {
                    return Err(invalid_at(
                        event,
                        "cancellation intent is duplicate or out of state",
                    ));
                }
                self.cancellation = Some(RunCancellation {
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    sequence,
                });
                self.lifecycle = RunLifecycle::Cancelling;
            }
            RunEventKind::RunTerminationRequested { outcome, reason } => {
                if self.lifecycle != RunLifecycle::Running
                    || self.cancellation.is_some()
                    || self.termination.is_some()
                    || *outcome != RunOutcome::Failed
                {
                    return Err(invalid_at(
                        event,
                        "run termination intent must be one first explicit failed drain on a running run",
                    ));
                }
                self.termination = Some(RunTerminationIntent {
                    outcome: *outcome,
                    reason: reason.clone(),
                    sequence,
                });
            }
            RunEventKind::RunTerminal {
                outcome,
                outputs,
                artifacts,
                reason,
            } => {
                if !matches!(
                    self.lifecycle,
                    RunLifecycle::Running
                        | RunLifecycle::Paused
                        | RunLifecycle::Cancelling
                        | RunLifecycle::Created
                ) {
                    return Err(invalid_at(event, "run terminal fact is out of state"));
                }
                if *outcome == RunOutcome::Cancelled && self.cancellation.is_none() {
                    return Err(invalid_at(
                        event,
                        "cancelled outcome requires durable cancellation intent",
                    ));
                }
                if self.lifecycle == RunLifecycle::Cancelling && *outcome != RunOutcome::Cancelled {
                    return Err(invalid_at(
                        event,
                        "durable cancellation intent requires a cancelled run outcome",
                    ));
                }
                if self.cancellation.is_none()
                    && self.termination.as_ref().is_some_and(|termination| {
                        *outcome != termination.outcome
                            || reason.as_ref() != Some(&termination.reason)
                    })
                {
                    return Err(invalid_at(
                        event,
                        "run terminal outcome contradicts its durable explicit-terminal drain",
                    ));
                }
                if self.lifecycle == RunLifecycle::Created && *outcome != RunOutcome::Cancelled {
                    return Err(invalid_at(
                        event,
                        "an unstarted run may only terminate as cancelled",
                    ));
                }
                self.ensure_terminal_quiescent(event)?;
                ensure_unique(outputs, event, "terminal output reference")?;
                ensure_unique(artifacts, event, "terminal artifact reference")?;
                for output in outputs {
                    self.validate_known_workspace_value(output, event)?;
                }
                for artifact in artifacts {
                    self.validate_published_artifact(artifact, event)?;
                }
                self.terminal = Some(RunTerminalProjection {
                    outcome: *outcome,
                    outputs: outputs.clone(),
                    artifacts: artifacts.clone(),
                    reason: reason.clone(),
                    sequence,
                });
                self.lifecycle = RunLifecycle::Terminal(*outcome);
            }
            RunEventKind::NodeBecameEligible {
                node,
                execution,
                scope,
                mode,
            } => {
                self.validate_scope_reference(scope, event)?;
                if self.node_executions.contains_key(execution) {
                    return Err(invalid_at(
                        event,
                        "node execution identity was already created",
                    ));
                }
                self.reserved_executions.remove(execution);
                self.node_executions.insert(
                    execution.clone(),
                    NodeExecutionProjection {
                        execution: execution.clone(),
                        node: node.clone(),
                        scope: scope.clone(),
                        mode: *mode,
                        created_sequence: sequence,
                        created_at: event.occurred_at(),
                        attempts: Vec::new(),
                        state: NodeExecutionState::Eligible,
                        cancellation: None,
                        deterministic_terminal: None,
                        outputs: Vec::new(),
                    },
                );
                self.execution_ids_by_node
                    .entry(node.clone())
                    .or_default()
                    .insert(execution.clone());
                self.eligible_executions.insert(execution.clone());
                self.activate_execution(execution, event)?;
                if self
                    .pending_reconciliation_restarts
                    .remove(&(node.clone(), scope.clone()))
                    .is_some()
                {
                    self.adjust_scope_ownership(scope, false, event)?;
                }
            }
            RunEventKind::NodeExecutionCancelledBeforeDispatch { execution, reason } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.state != NodeExecutionState::Eligible
                    || !execution_view.attempts.is_empty()
                    || execution_view.cancellation.is_some()
                    || !self.has_execution_cancellation_source(execution)
                {
                    return Err(invalid_at(
                        event,
                        "pre-dispatch cancellation requires an eligible, attempt-free execution and a structured cancellation source",
                    ));
                }
                let execution_view = self
                    .node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?;
                execution_view.cancellation = Some(NodeExecutionCancellationProjection {
                    attempt: None,
                    reason: reason.clone(),
                    sequence,
                });
                execution_view.state = NodeExecutionState::CancelledBeforeDispatch;
                self.eligible_executions.remove(execution);
                self.deactivate_execution(execution, event)?;
            }
            RunEventKind::NodeExecutionCancellationRequested {
                execution,
                attempt,
                reason,
            } => {
                let execution_view = self.execution(execution, event)?;
                let attempt_view = self.attempt(attempt, event)?;
                if execution_view.attempts.last() != Some(attempt)
                    || execution_view.cancellation.is_some()
                    || attempt_view.execution != *execution
                    || !matches!(
                        attempt_view.state,
                        AttemptState::Scheduled | AttemptState::Leased | AttemptState::Running
                    )
                    || !matches!(
                        execution_view.state,
                        NodeExecutionState::Scheduled(ref active)
                            | NodeExecutionState::Running(ref active)
                            if active == attempt
                    )
                    || !self.has_execution_cancellation_source(execution)
                {
                    return Err(invalid_at(
                        event,
                        "attempt cancellation must target the latest scheduled, leased, or running attempt with structured authority",
                    ));
                }
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .cancellation = Some(NodeExecutionCancellationProjection {
                    attempt: Some(attempt.clone()),
                    reason: reason.clone(),
                    sequence,
                });
            }
            RunEventKind::NodeScheduled {
                node,
                execution,
                attempt,
                invocation,
                idempotency_key,
                request,
            } => {
                if self.invocations.contains(invocation) {
                    return Err(invalid_at(
                        event,
                        "invocation identity was already scheduled",
                    ));
                }
                let execution_view = self.execution(execution, event)?;
                if execution_view.node != *node
                    || execution_view.mode != NodeExecutionMode::Executor
                    || request.invocation() != invocation
                    || request.idempotency_key() != idempotency_key.as_ref()
                {
                    return Err(invalid_at(
                        event,
                        "scheduled node differs from its execution or is runtime-owned",
                    ));
                }
                if execution_view.cancellation.is_some() {
                    return Err(invalid_at(
                        event,
                        "a cancelled execution cannot schedule another invocation",
                    ));
                }
                let is_first = execution_view.attempts.is_empty();
                if is_first {
                    if execution_view.state != NodeExecutionState::Eligible
                        || self.attempts.contains_key(attempt)
                    {
                        return Err(invalid_at(
                            event,
                            "first attempt is duplicate or out of state",
                        ));
                    }
                    self.attempts.insert(
                        attempt.clone(),
                        new_attempt(
                            attempt.clone(),
                            execution.clone(),
                            1,
                            AttemptState::Scheduled,
                        ),
                    );
                    self.node_executions
                        .get_mut(execution)
                        .ok_or_else(|| invalid_at(event, "unknown node execution"))?
                        .attempts
                        .push(attempt.clone());
                } else {
                    let projected_attempt = self
                        .attempts
                        .get(attempt)
                        .ok_or_else(|| invalid_at(event, "retry attempt was not reserved"))?;
                    if projected_attempt.execution != *execution
                        || projected_attempt.state != AttemptState::ReadyToSchedule
                        || execution_view.attempts.last() != Some(attempt)
                    {
                        return Err(invalid_at(
                            event,
                            "retry attempt is not ready for this execution",
                        ));
                    }
                    let previous_request = execution_view
                        .attempts
                        .iter()
                        .rev()
                        .nth(1)
                        .and_then(|previous| self.attempts.get(previous))
                        .and_then(NodeAttemptProjection::request)
                        .ok_or_else(|| {
                            invalid_at(event, "retry has no prior persisted invocation request")
                        })?;
                    if !same_logical_invocation_request(previous_request, request) {
                        return Err(invalid_at(
                            event,
                            "retry changed immutable capability, provider, input, or extension facts",
                        ));
                    }
                    let timer = self
                        .retry_by_attempt
                        .get(attempt)
                        .ok_or_else(|| invalid_at(event, "retry attempt has no retry decision"))?
                        .clone();
                    self.retries
                        .get_mut(&timer)
                        .ok_or_else(|| invalid_at(event, "retry decision is missing"))?
                        .state = RetryState::Scheduled;
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "attempt was not created"))?;
                attempt_view.invocation = Some(invocation.clone());
                attempt_view.idempotency_key = idempotency_key.clone();
                attempt_view.request = Some(request.clone());
                attempt_view.scheduled_sequence = Some(sequence);
                attempt_view.state = AttemptState::Scheduled;
                self.active_attempt_ids.insert(attempt.clone());
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown node execution"))?
                    .state = NodeExecutionState::Scheduled(attempt.clone());
                self.eligible_executions.remove(execution);
                self.invocations.insert(invocation.clone());
            }
            RunEventKind::CapabilityResolved {
                execution,
                attempt,
                requirement,
                snapshot,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let request = attempt_view.request.as_ref().ok_or_else(|| {
                    invalid_at(
                        event,
                        "capability resolution has no persisted invocation request",
                    )
                })?;
                let execution_attempts = self
                    .node_executions
                    .get(&attempt_view.execution)
                    .map(NodeExecutionProjection::attempts)
                    .ok_or_else(|| invalid_at(event, "attempt has no owning execution"))?;
                let attempt_position = execution_attempts
                    .iter()
                    .position(|candidate| candidate == attempt);
                let stable_retry_snapshot = attempt_position.is_some_and(|position| {
                    position.checked_sub(1).is_none_or(|previous_position| {
                        execution_attempts
                            .get(previous_position)
                            .and_then(|previous| self.attempts.get(previous))
                            .is_some_and(|previous| {
                                let requires_stable_snapshot = previous.state
                                    == AttemptState::Uncertain
                                    || previous.side_effect.as_ref().is_some_and(
                                        |classification| {
                                            classification.side_effect
                                                == SideEffectClass::IdempotentWrite
                                        },
                                    );
                                !requires_stable_snapshot
                                    || previous
                                        .capability
                                        .as_ref()
                                        .is_some_and(|capability| capability.snapshot == *snapshot)
                            })
                    })
                });
                if attempt_view.execution != *execution
                    || attempt_view.state != AttemptState::Scheduled
                    || attempt_view.capability.is_some()
                    || requirement.operation() != snapshot.operation()
                    || requirement
                        .exact_capability()
                        .is_some_and(|required| required != snapshot.capability())
                    || requirement
                        .provider_profile_ref()
                        .is_some_and(|required| snapshot.provider_profile() != Some(required))
                    || snapshot.operation_contract().side_effect()
                        > requirement.maximum_side_effect_class()
                    || request.capability() != snapshot.capability()
                    || request.operation() != snapshot.operation()
                    || request.provider_profile() != snapshot.provider_profile()
                    || !stable_retry_snapshot
                {
                    return Err(invalid_at(
                        event,
                        "capability resolution is duplicate or incompatible",
                    ));
                }
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .capability = Some(CapabilityResolution {
                    requirement: requirement.clone(),
                    snapshot: snapshot.clone(),
                });
            }
            RunEventKind::SideEffectClassified {
                attempt,
                side_effect,
                idempotency,
                idempotency_key,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let capability = attempt_view.capability.as_ref().ok_or_else(|| {
                    invalid_at(
                        event,
                        "side-effect classification precedes capability resolution",
                    )
                })?;
                let contract = capability.snapshot.operation_contract();
                let key_shape_valid = match idempotency {
                    IdempotencyBehavior::Unsupported => idempotency_key.is_none(),
                    IdempotencyBehavior::CapabilityScoped
                    | IdempotencyBehavior::ProviderProfileScoped => idempotency_key.is_some(),
                };
                let execution_attempts = self
                    .node_executions
                    .get(&attempt_view.execution)
                    .map(NodeExecutionProjection::attempts)
                    .ok_or_else(|| invalid_at(event, "attempt has no owning execution"))?;
                let attempt_position = execution_attempts
                    .iter()
                    .position(|candidate| candidate == attempt);
                let stable_retry_key = if *side_effect == SideEffectClass::IdempotentWrite {
                    *idempotency != IdempotencyBehavior::Unsupported
                        && idempotency_key.is_some()
                        && attempt_position.is_some_and(|position| {
                            execution_attempts[..position].iter().all(|prior| {
                                self.attempts
                                    .get(prior)
                                    .and_then(|attempt| attempt.side_effect.as_ref())
                                    .and_then(|classification| {
                                        classification.idempotency_key.as_ref()
                                    })
                                    == idempotency_key.as_ref()
                            })
                        })
                } else {
                    true
                };
                if attempt_view.state != AttemptState::Scheduled
                    || attempt_view.side_effect.is_some()
                    || contract.side_effect() != *side_effect
                    || contract.idempotency() != *idempotency
                    || attempt_view.idempotency_key.as_ref() != idempotency_key.as_ref()
                    || !key_shape_valid
                    || !stable_retry_key
                {
                    return Err(invalid_at(
                        event,
                        "side-effect classification contradicts frozen dispatch facts",
                    ));
                }
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .side_effect = Some(SideEffectClassification {
                    side_effect: *side_effect,
                    idempotency: *idempotency,
                    idempotency_key: idempotency_key.clone(),
                });
            }
            RunEventKind::LeaseGranted {
                lease,
                execution,
                attempt,
                worker,
                expires_at,
            } => {
                if self.leases.contains_key(lease) || *expires_at <= event.occurred_at() {
                    return Err(invalid_at(
                        event,
                        "lease identity is duplicate or expiration is not future",
                    ));
                }
                let attempt_view = self.attempt(attempt, event)?;
                if attempt_view.execution != *execution
                    || attempt_view.state != AttemptState::Scheduled
                    || attempt_view.capability.is_none()
                    || attempt_view.side_effect.is_none()
                    || self.active_lease_for_attempt(attempt).is_some()
                {
                    return Err(invalid_at(
                        event,
                        "lease grant is out of state or lacks dispatch facts",
                    ));
                }
                self.leases.insert(
                    lease.clone(),
                    LeaseProjection {
                        lease: lease.clone(),
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        worker: worker.clone(),
                        expires_at: *expires_at,
                        state: LeaseState::Active,
                    },
                );
                if self
                    .active_lease_by_attempt
                    .insert(attempt.clone(), lease.clone())
                    .is_some()
                {
                    return Err(invalid_at(
                        event,
                        "lease grant replaced an active attempt lease",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.leases.push(lease.clone());
                attempt_view.state = AttemptState::Leased;
            }
            RunEventKind::LeaseHeartbeatRecorded { lease, expires_at } => {
                let lease_view = self
                    .leases
                    .get_mut(lease)
                    .ok_or_else(|| invalid_at(event, "heartbeat references an unknown lease"))?;
                if !lease_view.is_active()
                    || event.occurred_at() >= lease_view.expires_at
                    || *expires_at <= lease_view.expires_at
                    || *expires_at <= event.occurred_at()
                {
                    return Err(invalid_at(
                        event,
                        "heartbeat requires a still-valid active lease and later expiration",
                    ));
                }
                lease_view.expires_at = *expires_at;
            }
            RunEventKind::LeaseExpired {
                lease,
                classification,
            } => {
                let lease_view = self
                    .leases
                    .get(lease)
                    .ok_or_else(|| invalid_at(event, "expiry references an unknown lease"))?;
                let lease_attempt = lease_view.attempt.clone();
                let attempt_view = self.attempt(&lease_attempt, event)?;
                let retry_safe = attempt_view
                    .side_effect
                    .as_ref()
                    .is_some_and(|classification| {
                        matches!(
                            classification.side_effect,
                            SideEffectClass::None | SideEffectClass::ReadOnly
                        ) || (classification.side_effect == SideEffectClass::IdempotentWrite
                            && classification.idempotency != IdempotencyBehavior::Unsupported
                            && classification.idempotency_key.is_some())
                    });
                let classification_valid = match classification {
                    RecoveryClassification::NotStarted => {
                        attempt_view.state == AttemptState::Leased
                    }
                    RecoveryClassification::Retryable => {
                        retry_safe
                            && matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            )
                    }
                    RecoveryClassification::Uncertain => {
                        !retry_safe
                            && matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            )
                    }
                    RecoveryClassification::LeaseStillValid
                    | RecoveryClassification::TerminalObserved => false,
                };
                if !lease_view.is_active()
                    || event.occurred_at() < lease_view.expires_at
                    || !classification_valid
                {
                    return Err(invalid_at(
                        event,
                        "lease expiry is early, duplicate, or contradicts immutable attempt facts",
                    ));
                }
                self.leases
                    .get_mut(lease)
                    .ok_or_else(|| invalid_at(event, "unknown lease"))?
                    .state = LeaseState::Expired(*classification);
                if self.active_lease_by_attempt.remove(&lease_attempt).as_ref()
                    != Some(lease)
                {
                    return Err(invalid_at(
                        event,
                        "lease expiry disagrees with the active attempt lease",
                    ));
                }
            }
            RunEventKind::NodeReLeased {
                previous_lease,
                lease,
                attempt,
                worker,
                expires_at,
            } => {
                if self.leases.contains_key(lease) || *expires_at <= event.occurred_at() {
                    return Err(invalid_at(
                        event,
                        "replacement lease is duplicate or already expired",
                    ));
                }
                let prior = self.leases.get(previous_lease).ok_or_else(|| {
                    invalid_at(event, "replacement references an unknown prior lease")
                })?;
                let classification = match prior.state {
                    LeaseState::Expired(classification) => classification,
                    LeaseState::Active | LeaseState::Superseded(_) | LeaseState::Completed => {
                        return Err(invalid_at(event, "only an expired lease may be superseded"));
                    }
                };
                let execution = prior.execution.clone();
                let attempt_view = self.attempt(attempt, event)?;
                let execution_view = self.execution(&execution, event)?;
                let retry_safe = attempt_view
                    .side_effect
                    .as_ref()
                    .is_some_and(|classification| {
                        matches!(
                            classification.side_effect,
                            SideEffectClass::None | SideEffectClass::ReadOnly
                        ) || (classification.side_effect == SideEffectClass::IdempotentWrite
                            && classification.idempotency != IdempotencyBehavior::Unsupported
                            && classification.idempotency_key.is_some())
                    });
                let state_is_releasable = match classification {
                    RecoveryClassification::NotStarted => {
                        attempt_view.state == AttemptState::Leased
                    }
                    RecoveryClassification::Retryable => {
                        retry_safe
                            && matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            )
                    }
                    RecoveryClassification::LeaseStillValid
                    | RecoveryClassification::Uncertain
                    | RecoveryClassification::TerminalObserved => false,
                };
                let exact_recovery = attempt_view.recovery.last().is_some_and(|observation| {
                    observation.lease.as_ref() == Some(previous_lease)
                        && observation.classification == classification
                });
                if prior.attempt != *attempt
                    || attempt_view.leases.last() != Some(previous_lease)
                    || execution_view.attempts.last() != Some(attempt)
                    || execution_view.cancellation.is_some()
                    || !matches!(
                        execution_view.state,
                        NodeExecutionState::Scheduled(ref active)
                            | NodeExecutionState::Running(ref active)
                            if active == attempt
                    )
                    || !state_is_releasable
                    || !exact_recovery
                    || self.active_lease_for_attempt(attempt).is_some()
                {
                    return Err(invalid_at(
                        event,
                        "attempt is not safely eligible for re-lease",
                    ));
                }
                self.leases
                    .get_mut(previous_lease)
                    .ok_or_else(|| invalid_at(event, "unknown prior lease"))?
                    .state = LeaseState::Superseded(lease.clone());
                self.leases.insert(
                    lease.clone(),
                    LeaseProjection {
                        lease: lease.clone(),
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        worker: worker.clone(),
                        expires_at: *expires_at,
                        state: LeaseState::Active,
                    },
                );
                if self
                    .active_lease_by_attempt
                    .insert(attempt.clone(), lease.clone())
                    .is_some()
                {
                    return Err(invalid_at(
                        event,
                        "replacement lease displaced an active attempt lease",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.leases.push(lease.clone());
                attempt_view.state = AttemptState::Leased;
                self.node_executions
                    .get_mut(&execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::Scheduled(attempt.clone());
            }
            RunEventKind::NodeStarted {
                execution,
                attempt,
                invocation,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                if attempt_view.execution != *execution
                    || attempt_view.invocation.as_ref() != Some(invocation)
                    || attempt_view.state != AttemptState::Leased
                    || self.active_lease_for_attempt(attempt).is_none()
                {
                    return Err(invalid_at(
                        event,
                        "node start does not match a leased scheduled invocation",
                    ));
                }
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .state = AttemptState::Running;
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::Running(attempt.clone());
            }
            RunEventKind::NodeProgressRecorded {
                attempt,
                report_sequence,
                detail,
                completed_units,
                total_units,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                if attempt_view.state != AttemptState::Running
                    || !attempt_view.expects_report_sequence(*report_sequence)
                {
                    return Err(invalid_at(
                        event,
                        "progress is out of state or not the exact next report",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.progress.push(ProgressObservation {
                    report_sequence: *report_sequence,
                    detail: detail.clone(),
                    completed_units: *completed_units,
                    total_units: *total_units,
                });
                attempt_view.last_report_sequence = Some(*report_sequence);
            }
            RunEventKind::AttemptUsageRecorded { attempt, usage } => {
                let attempt_view = self.attempt(attempt, event)?;
                if !matches!(
                    attempt_view.state,
                    AttemptState::Running | AttemptState::Terminal(_)
                ) || attempt_view.usage.is_some()
                {
                    return Err(invalid_at(
                        event,
                        "attempt usage is duplicate or out of state",
                    ));
                }
                self.accumulate_usage(usage, event)?;
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .usage = Some(usage.clone());
            }
            RunEventKind::InvocationCancellationAcknowledged {
                attempt,
                acknowledgement,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let cancellation_matches = self
                    .node_executions
                    .get(&attempt_view.execution)
                    .and_then(|execution| execution.cancellation.as_ref())
                    .and_then(NodeExecutionCancellationProjection::attempt)
                    == Some(attempt);
                if !cancellation_matches
                    || attempt_view.invocation.as_ref() != Some(acknowledgement.invocation())
                    || attempt_view.is_completed()
                    || attempt_view
                        .cancellation_acknowledgements
                        .last()
                        .is_some_and(|prior| {
                            acknowledgement.request_sequence() <= prior.request_sequence()
                        })
                {
                    return Err(invalid_at(
                        event,
                        "cancellation acknowledgement is stale or mismatched",
                    ));
                }
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .cancellation_acknowledgements
                    .push(acknowledgement.clone());
            }
            RunEventKind::NodeOutputPublished {
                execution,
                attempt,
                report_sequence,
                value,
                artifact,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let execution_view = self.execution(execution, event)?;
                if attempt_view.execution != *execution
                    || attempt_view.state != AttemptState::Running
                    || !attempt_view.expects_report_sequence(*report_sequence)
                    || value.scope() != &execution_view.scope
                    || self.workspace_values.contains(value)
                    || attempt_view
                        .outputs
                        .iter()
                        .any(|output| output.value == *value)
                {
                    return Err(invalid_at(
                        event,
                        "node output is duplicate, out of scope, or out of state",
                    ));
                }
                self.validate_workspace_value(value, event)?;
                if let Some(artifact) = artifact {
                    self.validate_published_artifact(artifact, event)?;
                }
                let output = PublishedNodeOutput {
                    report_sequence: Some(*report_sequence),
                    value: value.clone(),
                    artifact: artifact.clone(),
                    sequence,
                };
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.outputs.push(output.clone());
                attempt_view.last_report_sequence = Some(*report_sequence);
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .outputs
                    .push(output);
                self.record_workspace_value(value, event)?;
            }
            RunEventKind::DeterministicOutputPublished {
                execution,
                value,
                artifact,
            } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.state != NodeExecutionState::Eligible
                    || execution_view.mode != NodeExecutionMode::Runtime
                    || !execution_view.attempts.is_empty()
                    || value.scope() != &execution_view.scope
                    || self.workspace_values.contains(value)
                    || execution_view
                        .outputs
                        .iter()
                        .any(|output| output.value == *value)
                {
                    return Err(invalid_at(
                        event,
                        "deterministic output is duplicate, out of scope, or follows completion",
                    ));
                }
                self.validate_workspace_value(value, event)?;
                if let Some(artifact) = artifact {
                    self.validate_published_artifact(artifact, event)?;
                }
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .outputs
                    .push(PublishedNodeOutput {
                        report_sequence: None,
                        value: value.clone(),
                        artifact: artifact.clone(),
                        sequence,
                    });
                self.record_workspace_value(value, event)?;
            }
            RunEventKind::DeterministicNodeTerminal {
                execution,
                outcome,
                error_class,
                detail,
            } => {
                let execution_view = self.execution(execution, event)?;
                let failure_shape = matches!(outcome, NodeOutcome::Failed | NodeOutcome::Rejected);
                if execution_view.state != NodeExecutionState::Eligible
                    || execution_view.mode != NodeExecutionMode::Runtime
                    || !execution_view.attempts.is_empty()
                    || execution_view.cancellation.is_some()
                    || execution_view.deterministic_terminal.is_some()
                    || *outcome == NodeOutcome::Cancelled
                    || failure_shape != error_class.is_some()
                {
                    return Err(invalid_at(
                        event,
                        "deterministic terminal fact requires an attempt-free eligible execution and a valid non-cancellation outcome",
                    ));
                }
                let terminal = DeterministicNodeTerminalProjection {
                    outcome: *outcome,
                    error_class: *error_class,
                    detail: detail.clone(),
                    sequence,
                };
                let execution_view = self
                    .node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?;
                execution_view.deterministic_terminal = Some(terminal);
                execution_view.state = NodeExecutionState::Terminal(*outcome);
                self.eligible_executions.remove(execution);
                self.deactivate_execution(execution, event)?;
                if *outcome == NodeOutcome::Succeeded {
                    self.pending_successor_executions.insert(execution.clone());
                }
            }
            RunEventKind::NodePreDispatchFailed {
                execution,
                error_class,
                detail,
            } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.state != NodeExecutionState::Eligible
                    || execution_view.mode != NodeExecutionMode::Executor
                    || !execution_view.attempts.is_empty()
                    || execution_view.cancellation.is_some()
                    || execution_view.deterministic_terminal.is_some()
                {
                    return Err(invalid_at(
                        event,
                        "pre-dispatch failure requires an attempt-free eligible executor execution",
                    ));
                }
                let terminal = DeterministicNodeTerminalProjection {
                    outcome: NodeOutcome::Failed,
                    error_class: Some(*error_class),
                    detail: detail.clone(),
                    sequence,
                };
                let execution_view = self
                    .node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?;
                execution_view.deterministic_terminal = Some(terminal);
                execution_view.state = NodeExecutionState::Terminal(NodeOutcome::Failed);
                self.eligible_executions.remove(execution);
                self.deactivate_execution(execution, event)?;
            }
            RunEventKind::StructuredSuccessorScanCompleted { execution } => {
                if self
                    .node_executions
                    .get(execution)
                    .is_none_or(|execution| {
                        execution.state
                            != NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                    })
                    || !self.pending_successor_executions.remove(execution)
                {
                    return Err(invalid_at(
                        event,
                        "successor scan marker must consume one pending successful execution",
                    ));
                }
            }
            RunEventKind::NodeTerminal {
                execution,
                attempt,
                report_sequence,
                outcome,
                error_class,
                detail,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let failure_shape = matches!(outcome, NodeOutcome::Failed | NodeOutcome::Rejected);
                let cancellation_matches = self
                    .node_executions
                    .get(execution)
                    .and_then(|execution| execution.cancellation.as_ref())
                    .and_then(NodeExecutionCancellationProjection::attempt)
                    == Some(attempt);
                if attempt_view.execution != *execution
                    || !matches!(
                        attempt_view.state,
                        AttemptState::Leased | AttemptState::Running
                    )
                    || attempt_view.capability.is_none()
                    || attempt_view.side_effect.is_none()
                    || attempt_view.leases.is_empty()
                    || !attempt_view.expects_report_sequence(*report_sequence)
                    || failure_shape != error_class.is_some()
                    || (*outcome == NodeOutcome::Cancelled && !cancellation_matches)
                {
                    return Err(invalid_at(
                        event,
                        "node terminal fact is duplicate, mismatched, or malformed",
                    ));
                }
                let safely_covered_uncertain = {
                    let current_request = attempt_view.request.as_ref();
                    let current_capability = attempt_view.capability.as_ref();
                    let current_side_effect = attempt_view.side_effect.as_ref();
                    self.node_executions
                        .get(execution)
                        .into_iter()
                        .flat_map(|execution| execution.attempts.iter())
                        .take_while(|candidate| *candidate != attempt)
                        .filter_map(|candidate| {
                            let prior = self.attempts.get(candidate)?;
                            let prior_side_effect = prior.side_effect.as_ref()?;
                            let retry_safe = matches!(
                                prior_side_effect.side_effect,
                                SideEffectClass::None | SideEffectClass::ReadOnly
                            ) || (prior_side_effect.side_effect
                                == SideEffectClass::IdempotentWrite
                                && prior_side_effect.idempotency
                                    != IdempotencyBehavior::Unsupported
                                && prior_side_effect.idempotency_key.is_some());
                            let terminal_covers = *outcome == NodeOutcome::Succeeded
                                || matches!(
                                    prior_side_effect.side_effect,
                                    SideEffectClass::None | SideEffectClass::ReadOnly
                                );
                            (prior.state == AttemptState::Uncertain
                                && prior.obligation.is_some()
                                && retry_safe
                                && terminal_covers
                                && prior_side_effect == current_side_effect?
                                && prior.request.as_ref().zip(current_request).is_some_and(
                                    |(prior, current)| {
                                        same_logical_invocation_request(prior, current)
                                    },
                                )
                                && prior.idempotency_key == attempt_view.idempotency_key
                                && prior
                                    .capability
                                    .as_ref()
                                    .zip(current_capability)
                                    .is_some_and(|(prior, current)| {
                                        prior.snapshot == current.snapshot
                                    }))
                            .then(|| candidate.clone())
                        })
                        .collect::<Vec<_>>()
                };
                let terminal = AttemptTerminal {
                    report_sequence: *report_sequence,
                    outcome: *outcome,
                    error_class: *error_class,
                    detail: detail.clone(),
                    sequence,
                };
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.state = AttemptState::Terminal(*outcome);
                attempt_view.last_report_sequence = Some(*report_sequence);
                attempt_view.terminal = Some(terminal);
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::Terminal(*outcome);
                self.active_attempt_ids.remove(attempt);
                self.deactivate_execution(execution, event)?;
                if *outcome == NodeOutcome::Succeeded {
                    self.pending_successor_executions.insert(execution.clone());
                }
                self.complete_attempt_leases(attempt);
                for covered in safely_covered_uncertain {
                    let covered_view = self
                        .attempts
                        .get_mut(&covered)
                        .ok_or_else(|| invalid_at(event, "superseded attempt is missing"))?;
                    covered_view.state = if *outcome == NodeOutcome::Cancelled {
                        AttemptState::UncertainAbandonedByCancellation {
                            cancelled_retry: attempt.clone(),
                        }
                    } else {
                        AttemptState::UncertainSupersededByRetry {
                            covering_attempt: attempt.clone(),
                        }
                    };
                    self.active_attempt_ids.remove(&covered);
                    self.complete_attempt_leases(&covered);
                }
            }
            RunEventKind::NodeRetryScheduled {
                execution,
                previous_attempt,
                next_attempt,
                attempt_number,
                timer,
                fire_at,
                error_class,
                reason,
            } => {
                if self.attempts.contains_key(next_attempt)
                    || self.timers.contains_key(timer)
                    || *fire_at < event.occurred_at()
                {
                    return Err(invalid_at(
                        event,
                        "retry identities are duplicate or deadline is in the past",
                    ));
                }
                let previous = self.attempt(previous_attempt, event)?;
                let retry_safe = previous.side_effect.as_ref().is_some_and(|classification| {
                    matches!(
                        classification.side_effect,
                        SideEffectClass::None | SideEffectClass::ReadOnly
                    ) || (classification.side_effect == SideEffectClass::IdempotentWrite
                        && classification.idempotency != IdempotencyBehavior::Unsupported
                        && classification.idempotency_key.is_some())
                });
                let retryable_terminal = matches!(
                    previous.state,
                    AttemptState::Terminal(NodeOutcome::Failed | NodeOutcome::Rejected)
                ) && previous
                    .terminal
                    .as_ref()
                    .is_some_and(|terminal| terminal.error_class == Some(*error_class))
                    && retry_safe;
                let retryable_uncertain = previous.state == AttemptState::Uncertain
                    && previous.obligation.as_ref().is_some_and(|obligation| {
                        obligation.side_effect
                            == previous
                                .side_effect
                                .as_ref()
                                .map_or(SideEffectClass::Unknown, |facts| facts.side_effect)
                    })
                    && retry_safe;
                let authority_retry = previous.obligation.as_ref().is_some_and(|obligation| {
                    obligation
                        .decisions
                        .last()
                        .is_some_and(|decision| decision.outcome == AuthorityDecision::Retry)
                }) && retry_safe;
                let execution_view = self.execution(execution, event)?;
                let expected_number = u32::try_from(execution_view.attempts.len())
                    .ok()
                    .and_then(|count| count.checked_add(1));
                if previous.execution != *execution
                    || execution_view.attempts.last() != Some(previous_attempt)
                    || expected_number != Some(*attempt_number)
                    || *attempt_number > crate::scheduler::MAX_RETRY_ATTEMPTS
                    || (!retryable_terminal && !retryable_uncertain && !authority_retry)
                {
                    return Err(invalid_at(
                        event,
                        "retry does not follow the latest retryable attempt",
                    ));
                }
                self.attempts.insert(
                    next_attempt.clone(),
                    new_attempt(
                        next_attempt.clone(),
                        execution.clone(),
                        *attempt_number,
                        AttemptState::AwaitingRetryTimer,
                    ),
                );
                self.active_attempt_ids.insert(next_attempt.clone());
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .attempts
                    .push(next_attempt.clone());
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::RetryPending(next_attempt.clone());
                self.timers.insert(
                    timer.clone(),
                    TimerProjection {
                        timer: timer.clone(),
                        purpose: TimerPurpose::Retry {
                            attempt: next_attempt.clone(),
                        },
                        fire_at: *fire_at,
                        state: TimerState::Pending,
                        cancellation: None,
                    },
                );
                self.pending_timer_ids.insert(timer.clone());
                self.pending_timers_by_execution
                    .entry(execution.clone())
                    .or_default()
                    .insert(timer.clone());
                self.retries.insert(
                    timer.clone(),
                    RetryProjection {
                        execution: execution.clone(),
                        previous_attempt: previous_attempt.clone(),
                        next_attempt: next_attempt.clone(),
                        attempt_number: *attempt_number,
                        timer: timer.clone(),
                        fire_at: *fire_at,
                        error_class: *error_class,
                        reason: reason.clone(),
                        state: RetryState::Waiting,
                    },
                );
                self.retry_by_attempt
                    .insert(next_attempt.clone(), timer.clone());
                if retryable_uncertain {
                    self.complete_attempt_leases(previous_attempt);
                }
            }
            RunEventKind::ExternalOutcomeUncertain {
                attempt,
                report_sequence,
                side_effect,
                reason,
                evidence,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let classified = attempt_view.side_effect.as_ref().ok_or_else(|| {
                    invalid_at(event, "uncertainty lacks frozen side-effect facts")
                })?;
                if !matches!(
                    attempt_view.state,
                    AttemptState::Leased | AttemptState::Running
                ) || attempt_view.obligation.is_some()
                    || !attempt_view.expects_report_sequence(*report_sequence)
                    || classified.side_effect != *side_effect
                {
                    return Err(invalid_at(
                        event,
                        "uncertain outcome is duplicate or contradicts dispatch facts",
                    ));
                }
                let execution = attempt_view.execution.clone();
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.state = AttemptState::Uncertain;
                attempt_view.last_report_sequence = Some(*report_sequence);
                attempt_view.obligation = Some(ExternalOutcomeObligation {
                    report_sequence: *report_sequence,
                    side_effect: *side_effect,
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    uncertain_sequence: sequence,
                    retained: None,
                    decisions: Vec::new(),
                });
                self.node_executions
                    .get_mut(&execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::Uncertain(attempt.clone());
                self.complete_attempt_leases(attempt);
            }
            RunEventKind::ExternalOutcomeRetained {
                attempt,
                decision,
                reason,
            } => {
                let decision_view = self.recovery_decisions.get(decision);
                if decision_view != Some(&(attempt.clone(), AuthorityDecision::Retain)) {
                    return Err(invalid_at(
                        event,
                        "retention lacks its prior matching authority decision",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "retention references an unknown attempt"))?;
                let obligation = attempt_view.obligation.as_mut().ok_or_else(|| {
                    invalid_at(event, "retention requires an uncertain obligation")
                })?;
                if obligation.retained.is_some() {
                    return Err(invalid_at(event, "external outcome was already retained"));
                }
                obligation.retained = Some(RetainedExternalOutcome {
                    decision: decision.clone(),
                    reason: reason.clone(),
                    sequence,
                });
                attempt_view.state = AttemptState::Retained;
                self.complete_attempt_leases(attempt);
            }
            RunEventKind::ArtifactPublished { metadata } => {
                self.apply_artifact_publication(metadata, event)?;
            }
            _ => self.apply_structured_kind(event)?,
        }
        Ok(())
    }
}

impl RunProjection {
    #[allow(clippy::too_many_lines)]
    fn apply_structured_kind(&mut self, event: &RunEventEnvelope) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::BranchScopeCreated {
                fork_execution,
                port,
                branch,
                scope,
            } => {
                let owner_scope = self.execution(fork_execution, event)?.scope.clone();
                if self.branches.contains_key(branch)
                    || self
                        .branch_by_fork_port
                        .contains_key(&(fork_execution.clone(), port.clone()))
                    || !matches!(scope.kind(), ScopeKind::Branch { branch: identity } if identity == branch)
                    || scope.parent() != Some(&owner_scope)
                {
                    return Err(invalid_at(
                        event,
                        "branch scope identity, port, kind, or parent is invalid",
                    ));
                }
                self.register_child_scope(scope, event)?;
                self.branches.insert(
                    branch.clone(),
                    BranchProjection {
                        branch: branch.clone(),
                        fork_execution: fork_execution.clone(),
                        port: port.clone(),
                        scope: scope.clone(),
                        children: BTreeSet::new(),
                        state: BranchState::Active,
                        cancellation_reason: None,
                        outputs: Vec::new(),
                    },
                );
                self.branch_by_fork_port.insert(
                    (fork_execution.clone(), port.clone()),
                    branch.clone(),
                );
                self.branch_ids_by_fork_execution
                    .entry(fork_execution.clone())
                    .or_default()
                    .insert(branch.clone());
                self.active_branch_ids.insert(branch.clone());
                self.adjust_scope_ownership(scope.reference(), true, event)?;
                self.adjust_structured_child_count(fork_execution, true, event)?;
            }
            RunEventKind::BranchRouteSelected {
                execution,
                selected_port,
            } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.is_completed() || self.branch_routes.contains_key(execution) {
                    return Err(invalid_at(
                        event,
                        "branch route is duplicate or follows terminal execution",
                    ));
                }
                self.branch_routes
                    .insert(execution.clone(), selected_port.clone());
            }
            RunEventKind::BranchChildAdded { branch, execution } => {
                let child_scope = self.execution(execution, event)?.scope.clone();
                let branch_view = self.branches.get(branch).ok_or_else(|| {
                    invalid_at(event, "branch child references an unknown branch")
                })?;
                if !branch_view.is_active()
                    || self.branch_owner.contains_key(execution)
                    || !self.scope_descends_from(&child_scope, branch_view.scope.reference())
                {
                    return Err(invalid_at(
                        event,
                        "branch child is duplicate, out of state, or outside its scope",
                    ));
                }
                self.branches
                    .get_mut(branch)
                    .ok_or_else(|| invalid_at(event, "unknown branch"))?
                    .children
                    .insert(execution.clone());
                self.branch_owner.insert(execution.clone(), branch.clone());
            }
            RunEventKind::BranchCancellationRequested { branch, reason } => {
                let branch_view = self.branches.get_mut(branch).ok_or_else(|| {
                    invalid_at(event, "cancellation references an unknown branch")
                })?;
                if branch_view.state != BranchState::Active {
                    return Err(invalid_at(
                        event,
                        "branch cancellation is duplicate or terminal",
                    ));
                }
                branch_view.state = BranchState::Cancelling;
                branch_view.cancellation_reason = Some(reason.clone());
                self.cancelling_branch_ids.insert(branch.clone());
            }
            RunEventKind::BranchTerminal {
                branch,
                outcome,
                outputs,
            } => {
                let branch_view = self.branches.get(branch).ok_or_else(|| {
                    invalid_at(event, "terminal fact references an unknown branch")
                })?;
                if !branch_view.is_active()
                    || (*outcome == RunOutcome::Cancelled
                        && branch_view.state != BranchState::Cancelling)
                    || self.branch_has_active_descendant_ownership(branch)
                {
                    return Err(invalid_at(
                        event,
                        "branch terminal fact is duplicate, contradicts cancellation, or abandons a child",
                    ));
                }
                ensure_unique(outputs, event, "branch terminal output")?;
                for output in outputs {
                    self.validate_known_workspace_value(output, event)?;
                    if !self.scope_descends_from(output.scope(), branch_view.scope.reference()) {
                        return Err(invalid_at(
                            event,
                            "branch terminal output is outside its isolated scope",
                        ));
                    }
                }
                let branch_scope = branch_view.scope.reference().clone();
                let fork_execution = branch_view.fork_execution.clone();
                let branch_view = self
                    .branches
                    .get_mut(branch)
                    .ok_or_else(|| invalid_at(event, "unknown branch"))?;
                branch_view.state = BranchState::Completed(*outcome);
                branch_view.outputs = outputs.clone();
                self.active_branch_ids.remove(branch);
                self.cancelling_branch_ids.remove(branch);
                self.adjust_scope_ownership(&branch_scope, false, event)?;
                self.adjust_structured_child_count(&fork_execution, false, event)?;
            }
            RunEventKind::JoinSatisfied {
                execution,
                rule,
                branches,
                retained_branches,
            } => {
                self.execution(execution, event)?;
                if self.joins.contains_key(execution) {
                    return Err(invalid_at(event, "join was already satisfied"));
                }
                ensure_unique_by(
                    branches,
                    |result| result.branch.clone(),
                    event,
                    "join branch",
                )?;
                ensure_unique(retained_branches, event, "retained branch")?;
                let result_ids: BTreeSet<_> =
                    branches.iter().map(|result| &result.branch).collect();
                if retained_branches
                    .iter()
                    .any(|branch| result_ids.contains(branch))
                {
                    return Err(invalid_at(
                        event,
                        "a completed join result cannot also be retained",
                    ));
                }
                let fork_execution = branches
                    .first()
                    .and_then(|result| self.branches.get(&result.branch))
                    .map(|branch| branch.fork_execution.clone())
                    .ok_or_else(|| invalid_at(event, "join has no known owning fork"))?;
                let fork_scope = self.execution(&fork_execution, event)?.scope.clone();
                if self.execution(execution, event)?.scope != fork_scope {
                    return Err(invalid_at(
                        event,
                        "join execution and owning fork must share a structured scope",
                    ));
                }
                for result in branches {
                    let branch = self
                        .branches
                        .get(&result.branch)
                        .ok_or_else(|| invalid_at(event, "join references an unknown branch"))?;
                    if branch.state != BranchState::Completed(result.outcome)
                        || branch.fork_execution != fork_execution
                        || branch.scope.reference() != &result.scope
                        || branch.outputs != result.outputs
                    {
                        return Err(invalid_at(
                            event,
                            "join result disagrees with the branch's durable terminal fact",
                        ));
                    }
                    for output in &result.outputs {
                        self.validate_known_workspace_value(output, event)?;
                        if !self.scope_descends_from(output.scope(), &result.scope) {
                            return Err(invalid_at(
                                event,
                                "branch result output is outside its scope",
                            ));
                        }
                    }
                }
                for retained in retained_branches {
                    let branch = self
                        .branches
                        .get(retained)
                        .ok_or_else(|| invalid_at(event, "join retains an unknown branch"))?;
                    if branch.state != BranchState::Active
                        || branch.fork_execution != fork_execution
                    {
                        return Err(invalid_at(
                            event,
                            "join retains a terminal, cancelling, or foreign branch",
                        ));
                    }
                }
                let owned = self
                    .branch_ids_by_fork_execution
                    .get(&fork_execution)
                    .cloned()
                    .unwrap_or_default();
                let named: BTreeSet<_> = branches
                    .iter()
                    .map(|result| result.branch.clone())
                    .chain(retained_branches.iter().cloned())
                    .collect();
                let unnamed_are_cancelling = owned.difference(&named).all(|branch| {
                    self.branches
                        .get(branch)
                        .is_some_and(|branch| branch.state == BranchState::Cancelling)
                });
                let successes = branches
                    .iter()
                    .filter(|result| result.outcome == RunOutcome::Succeeded)
                    .count();
                let satisfied = match rule {
                    JoinRule::All => {
                        !branches.is_empty()
                            && retained_branches.is_empty()
                            && result_ids.len() == owned.len()
                            && owned.iter().all(|branch| result_ids.contains(branch))
                    }
                    JoinRule::AnyCompletion => !branches.is_empty() && unnamed_are_cancelling,
                    JoinRule::FirstSuccess => {
                        successes >= 1 && retained_branches.is_empty() && unnamed_are_cancelling
                    }
                    JoinRule::Quorum { required } => {
                        usize::try_from(*required).is_ok_and(|required| successes >= required)
                            && retained_branches.is_empty()
                            && unnamed_are_cancelling
                    }
                };
                if !satisfied {
                    return Err(invalid_at(
                        event,
                        "recorded branch results do not satisfy the join rule",
                    ));
                }
                for result in branches {
                    let branch = self
                        .branches
                        .get(&result.branch)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?;
                    if branch.state != BranchState::Completed(result.outcome) {
                        return Err(invalid_at(event, "branch terminal outcome changed at join"));
                    }
                }
                for retained in retained_branches {
                    let scope = self
                        .branches
                        .get(retained)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?
                        .scope
                        .reference()
                        .clone();
                    let fork_execution = self
                        .branches
                        .get(retained)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?
                        .fork_execution
                        .clone();
                    self.branches
                        .get_mut(retained)
                        .ok_or_else(|| invalid_at(event, "unknown branch"))?
                        .state = BranchState::Retained;
                    self.active_branch_ids.remove(retained);
                    self.cancelling_branch_ids.remove(retained);
                    self.adjust_scope_ownership(&scope, false, event)?;
                    self.adjust_structured_child_count(&fork_execution, false, event)?;
                }
                self.joins.insert(
                    execution.clone(),
                    JoinProjection {
                        execution: execution.clone(),
                        rule: *rule,
                        branches: branches.clone(),
                        retained_branches: retained_branches.clone(),
                        sequence,
                    },
                );
            }
            RunEventKind::RepeatIterationCreated {
                repeat_execution,
                iteration,
                iteration_number,
                scope,
            } => {
                let parent_scope = self.execution(repeat_execution, event)?.scope.clone();
                if self.iterations.contains_key(iteration)
                    || self.repeat_terminations.contains_key(repeat_execution)
                    || !matches!(scope.kind(), ScopeKind::Iteration { iteration: identity } if identity == iteration)
                    || scope.parent() != Some(&parent_scope)
                {
                    return Err(invalid_at(
                        event,
                        "repeat iteration identity, kind, parent, or state is invalid",
                    ));
                }
                let expected = self
                    .latest_iteration
                    .get(repeat_execution)
                    .and_then(|previous| self.iterations.get(previous))
                    .map_or(Some(1), |previous| previous.iteration_number.checked_add(1))
                    .ok_or_else(|| invalid_at(event, "repeat iteration number overflow"))?;
                if *iteration_number != expected {
                    return Err(invalid_at(
                        event,
                        "repeat iteration numbers must be contiguous and one-based",
                    ));
                }
                if self
                    .repeat_continuations
                    .get(repeat_execution)
                    .is_some_and(|continuation| {
                        continuation.pending_approval
                            || continuation.rejected
                            || *iteration_number > continuation.effective_iteration_limit
                    })
                {
                    return Err(invalid_at(
                        event,
                        "repeat iteration exceeds or bypasses its continuation authority",
                    ));
                }
                if let Some(previous) = self.latest_iteration.get(repeat_execution).cloned() {
                    let previous_view = self
                        .iterations
                        .get_mut(&previous)
                        .ok_or_else(|| invalid_at(event, "repeat frontier is missing"))?;
                    let IterationState::ConditionRecorded(result) = previous_view.state else {
                        return Err(invalid_at(
                            event,
                            "a new iteration requires the prior frozen condition",
                        ));
                    };
                    if !result {
                        return Err(invalid_at(
                            event,
                            "a false condition cannot create another iteration",
                        ));
                    }
                    previous_view.state = IterationState::Completed(result);
                    self.active_iteration_ids.remove(&previous);
                    self.adjust_structured_child_count(repeat_execution, false, event)?;
                }
                self.register_child_scope(scope, event)?;
                self.iterations.insert(
                    iteration.clone(),
                    IterationProjection {
                        iteration: iteration.clone(),
                        repeat_execution: repeat_execution.clone(),
                        iteration_number: *iteration_number,
                        scope: scope.clone(),
                        state: IterationState::Active,
                    },
                );
                self.active_iteration_ids.insert(iteration.clone());
                self.adjust_structured_child_count(repeat_execution, true, event)?;
                self.latest_iteration
                    .insert(repeat_execution.clone(), iteration.clone());
            }
            RunEventKind::RepeatConditionRecorded { iteration, result } => {
                let iteration_view = self.iterations.get(iteration).ok_or_else(|| {
                    invalid_at(event, "condition references an unknown iteration")
                })?;
                if iteration_view.state != IterationState::Active {
                    return Err(invalid_at(event, "repeat condition was already frozen"));
                }
                self.iterations
                    .get_mut(iteration)
                    .ok_or_else(|| invalid_at(event, "unknown iteration"))?
                    .state = IterationState::ConditionRecorded(*result);
            }
            RunEventKind::RepeatContinuationRequested {
                repeat_execution,
                frontier_iteration,
                initial_iteration_limit,
                effective_iteration_limit,
                cause,
            } => {
                let execution_view = self.execution(repeat_execution, event)?;
                let frontier = self.iterations.get(frontier_iteration).ok_or_else(|| {
                    invalid_at(event, "repeat continuation request has an unknown frontier")
                })?;
                let frontier_is_latest =
                    self.latest_iteration.get(repeat_execution) == Some(frontier_iteration);
                let cause_matches_frontier = match cause {
                    RepeatContinuationCause::IterationLimit => {
                        frontier.iteration_number == *effective_iteration_limit
                    }
                    RepeatContinuationCause::DurationBudget { .. }
                    | RepeatContinuationCause::CostBudget { .. } => {
                        frontier.iteration_number <= *effective_iteration_limit
                    }
                };
                if execution_view.is_completed()
                    || self.repeat_terminations.contains_key(repeat_execution)
                    || frontier.repeat_execution != *repeat_execution
                    || frontier.state != IterationState::ConditionRecorded(true)
                    || !frontier_is_latest
                    || *initial_iteration_limit == 0
                    || *initial_iteration_limit > *effective_iteration_limit
                    || *effective_iteration_limit > MAX_REPEAT_EFFECTIVE_ITERATIONS
                    || !cause_matches_frontier
                {
                    return Err(invalid_at(
                        event,
                        "repeat continuation request contradicts its exact true-condition frontier, limits, or cause",
                    ));
                }
                let request = RepeatContinuationRequestProjection {
                    frontier_iteration: frontier_iteration.clone(),
                    initial_iteration_limit: *initial_iteration_limit,
                    effective_iteration_limit: *effective_iteration_limit,
                    cause: cause.clone(),
                    sequence,
                };
                if let Some(continuation) = self.repeat_continuations.get_mut(repeat_execution) {
                    if continuation.pending_approval
                        || continuation.rejected
                        || continuation.requests.len() >= MAX_REPEAT_CONTINUATION_CYCLES
                        || continuation.requests.len() != continuation.decisions.len()
                        || continuation.initial_iteration_limit != *initial_iteration_limit
                        || continuation.effective_iteration_limit != *effective_iteration_limit
                        || continuation
                            .requests
                            .last()
                            .is_some_and(|prior| prior.frontier_iteration == *frontier_iteration)
                    {
                        return Err(invalid_at(
                            event,
                            "repeat continuation request is duplicate or disagrees with prior authority",
                        ));
                    }
                    continuation.budget_override_iteration_limit = None;
                    continuation.pending_approval = true;
                    continuation.requests.push(request);
                } else {
                    if initial_iteration_limit != effective_iteration_limit {
                        return Err(invalid_at(
                            event,
                            "the first repeat continuation request must record its original effective limit",
                        ));
                    }
                    self.repeat_continuations.insert(
                        repeat_execution.clone(),
                        RepeatContinuationProjection {
                            repeat_execution: repeat_execution.clone(),
                            initial_iteration_limit: *initial_iteration_limit,
                            effective_iteration_limit: *effective_iteration_limit,
                            budget_override_iteration_limit: None,
                            pending_approval: true,
                            rejected: false,
                            requests: vec![request],
                            decisions: Vec::new(),
                        },
                    );
                }
            }
            RunEventKind::RepeatContinuationDecided {
                repeat_execution,
                decision,
                actor,
                outcome,
                approved_additional_iterations,
                reason,
                evidence,
            } => {
                let execution_view = self.execution(repeat_execution, event)?;
                let shape_valid = match (outcome, approved_additional_iterations) {
                    (RepeatContinuationDecision::Approved, Some(additional)) => (1
                        ..=milkdrift_persistence::MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS)
                        .contains(additional),
                    (RepeatContinuationDecision::Rejected, None) => true,
                    (RepeatContinuationDecision::Approved, None)
                    | (RepeatContinuationDecision::Rejected, Some(_)) => false,
                };
                if execution_view.is_completed()
                    || self.repeat_terminations.contains_key(repeat_execution)
                    || !shape_valid
                    || self.repeat_decision_ids.contains(decision)
                {
                    return Err(invalid_at(
                        event,
                        "repeat continuation decision is duplicate, malformed, or follows completion",
                    ));
                }
                let continuation =
                    self.repeat_continuations
                        .get(repeat_execution)
                        .ok_or_else(|| {
                            invalid_at(event, "repeat decision has no durable continuation request")
                        })?;
                let pending_request = continuation.pending_request().ok_or_else(|| {
                    invalid_at(event, "repeat decision has no pending continuation request")
                })?;
                let frontier = self
                    .iterations
                    .get(&pending_request.frontier_iteration)
                    .ok_or_else(|| invalid_at(event, "pending repeat frontier is missing"))?;
                if continuation.rejected
                    || continuation.requests.len() != continuation.decisions.len() + 1
                    || self.latest_iteration.get(repeat_execution)
                        != Some(&pending_request.frontier_iteration)
                    || frontier.repeat_execution != *repeat_execution
                    || frontier.state != IterationState::ConditionRecorded(true)
                    || pending_request.effective_iteration_limit
                        != continuation.effective_iteration_limit
                {
                    return Err(invalid_at(
                        event,
                        "repeat decision does not consume the exact pending authority request",
                    ));
                }
                let budget_frontier = match pending_request.cause {
                    RepeatContinuationCause::DurationBudget { .. }
                    | RepeatContinuationCause::CostBudget { .. } => Some(frontier.iteration_number),
                    RepeatContinuationCause::IterationLimit => None,
                };
                let decision_projection = RepeatContinuationDecisionProjection {
                    decision: decision.clone(),
                    actor: actor.clone(),
                    outcome: *outcome,
                    approved_additional_iterations: *approved_additional_iterations,
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    sequence,
                };
                let continuation = self
                    .repeat_continuations
                    .get_mut(repeat_execution)
                    .ok_or_else(|| invalid_at(event, "unknown repeat continuation"))?;
                if let Some(additional) = approved_additional_iterations {
                    continuation.effective_iteration_limit = continuation
                        .effective_iteration_limit
                        .checked_add(*additional)
                        .filter(|limit| *limit <= MAX_REPEAT_EFFECTIVE_ITERATIONS)
                        .ok_or_else(|| {
                            invalid_at(event, "repeat effective iteration limit overflow")
                        })?;
                    continuation.budget_override_iteration_limit = budget_frontier
                        .map(|frontier| {
                            frontier
                                .checked_add(*additional)
                                .filter(|limit| *limit <= MAX_REPEAT_EFFECTIVE_ITERATIONS)
                                .ok_or_else(|| {
                                    invalid_at(event, "repeat budget override frontier overflow")
                                })
                        })
                        .transpose()?;
                } else {
                    continuation.rejected = true;
                    continuation.budget_override_iteration_limit = None;
                }
                continuation.pending_approval = false;
                continuation.decisions.push(decision_projection);
                self.repeat_decision_ids.insert(decision.clone());
            }
            RunEventKind::RepeatTerminated {
                repeat_execution,
                termination,
                last_iteration,
            } => {
                self.execution(repeat_execution, event)?;
                let continuation_conflict = self
                    .repeat_continuations
                    .get(repeat_execution)
                    .is_some_and(|continuation| {
                        if *termination == RepeatTerminationReason::Cancelled {
                            return false;
                        }
                        if continuation.pending_approval {
                            return true;
                        }
                        if !continuation.rejected {
                            return false;
                        }
                        continuation.requests.last().is_none_or(|request| {
                            let expected = match request.cause {
                                RepeatContinuationCause::IterationLimit => {
                                    RepeatTerminationReason::MaximumIterations
                                }
                                RepeatContinuationCause::DurationBudget { .. }
                                | RepeatContinuationCause::CostBudget { .. } => {
                                    RepeatTerminationReason::BudgetExhausted
                                }
                            };
                            *termination != expected
                        })
                    });
                if self.repeat_terminations.contains_key(repeat_execution)
                    || self.latest_iteration.get(repeat_execution) != last_iteration.as_ref()
                    || continuation_conflict
                {
                    return Err(invalid_at(
                        event,
                        "repeat termination is duplicate or names the wrong frontier",
                    ));
                }
                if let Some(iteration) = last_iteration {
                    let iteration_view = self.iterations.get_mut(iteration).ok_or_else(|| {
                        invalid_at(event, "repeat termination references an unknown iteration")
                    })?;
                    let result = match iteration_view.state {
                        IterationState::ConditionRecorded(result)
                            if *termination
                                != RepeatTerminationReason::ConditionEvaluationFailed =>
                        {
                            result
                        }
                        IterationState::Active
                            if *termination
                                == RepeatTerminationReason::ConditionEvaluationFailed =>
                        {
                            false
                        }
                        IterationState::Active | IterationState::Completed(_) => {
                            return Err(invalid_at(
                                event,
                                "repeat termination requires a frozen frontier condition",
                            ));
                        }
                        IterationState::ConditionRecorded(_) => {
                            return Err(invalid_at(
                                event,
                                "condition-evaluation failure cannot follow a recorded condition",
                            ));
                        }
                    };
                    if *termination == RepeatTerminationReason::ConditionFalse && result {
                        return Err(invalid_at(
                            event,
                            "condition-false termination contradicts a true condition",
                        ));
                    }
                    iteration_view.state = IterationState::Completed(result);
                    self.active_iteration_ids.remove(iteration);
                    self.adjust_structured_child_count(repeat_execution, false, event)?;
                } else if matches!(
                    termination,
                    RepeatTerminationReason::ConditionFalse
                        | RepeatTerminationReason::ConditionEvaluationFailed
                ) {
                    return Err(invalid_at(
                        event,
                        "condition termination requires an iteration",
                    ));
                }
                self.repeat_terminations.insert(
                    repeat_execution.clone(),
                    RepeatTermination {
                        repeat_execution: repeat_execution.clone(),
                        termination: *termination,
                        last_iteration: last_iteration.clone(),
                        sequence,
                    },
                );
                if let Some(continuation) = self.repeat_continuations.get_mut(repeat_execution) {
                    continuation.pending_approval = false;
                    continuation.budget_override_iteration_limit = None;
                }
            }
            RunEventKind::TimerRegistered {
                timer,
                execution,
                fire_at,
            } => {
                if self.timers.contains_key(timer) || *fire_at < event.occurred_at() {
                    return Err(invalid_at(
                        event,
                        "timer identity is duplicate or deadline is in the past",
                    ));
                }
                if let Some(execution) = execution {
                    self.execution(execution, event)?;
                }
                self.timers.insert(
                    timer.clone(),
                    TimerProjection {
                        timer: timer.clone(),
                        purpose: TimerPurpose::Wait {
                            execution: execution.clone(),
                        },
                        fire_at: *fire_at,
                        state: TimerState::Pending,
                        cancellation: None,
                    },
                );
                self.pending_timer_ids.insert(timer.clone());
                if let Some(execution) = execution {
                    self.pending_timers_by_execution
                        .entry(execution.clone())
                        .or_default()
                        .insert(timer.clone());
                }
            }
            RunEventKind::TimerFired { timer, observed_at } => {
                let timer_view = self
                    .timers
                    .get_mut(timer)
                    .ok_or_else(|| invalid_at(event, "timer firing references an unknown timer"))?;
                if !timer_view.is_pending() || *observed_at < timer_view.fire_at {
                    return Err(invalid_at(
                        event,
                        "timer fired twice or before its deadline",
                    ));
                }
                let purpose = timer_view.purpose.clone();
                timer_view.state = TimerState::Fired {
                    observed_at: *observed_at,
                };
                self.pending_timer_ids.remove(timer);
                if let Some(retry) = self.retries.get_mut(timer) {
                    retry.state = RetryState::Ready;
                    self.attempts
                        .get_mut(&retry.next_attempt)
                        .ok_or_else(|| invalid_at(event, "retry timer has no reserved attempt"))?
                        .state = AttemptState::ReadyToSchedule;
                }
                self.remove_pending_timer_owner(timer, &purpose, event)?;
            }
            RunEventKind::TimerCancelled { timer, reason } => {
                let timer_view = self.timers.get(timer).ok_or_else(|| {
                    invalid_at(event, "timer cancellation references an unknown timer")
                })?;
                if !timer_view.is_pending() {
                    return Err(invalid_at(event, "only a pending timer may be cancelled"));
                }
                let purpose = timer_view.purpose.clone();
                let authorized = match &purpose {
                    TimerPurpose::Wait {
                        execution: Some(execution),
                    } => {
                        self.waits
                            .get(execution)
                            .and_then(WaitProjection::cancellation)
                            .is_some()
                            || self
                                .node_executions
                                .get(execution)
                                .and_then(NodeExecutionProjection::cancellation)
                                .is_some()
                            || self.has_execution_cancellation_source(execution)
                    }
                    TimerPurpose::Wait { execution: None } => self.cancellation.is_some(),
                    TimerPurpose::Retry { attempt } => {
                        let attempt_view = self.attempt(attempt, event)?;
                        attempt_view.state == AttemptState::AwaitingRetryTimer
                            && self.has_execution_cancellation_source(&attempt_view.execution)
                    }
                };
                if !authorized {
                    return Err(invalid_at(
                        event,
                        "timer cancellation lacks a structured owner cancellation fact",
                    ));
                }
                let timer_view = self
                    .timers
                    .get_mut(timer)
                    .ok_or_else(|| invalid_at(event, "unknown timer"))?;
                timer_view.state = TimerState::Cancelled;
                timer_view.cancellation = Some(TimerCancellationProjection {
                    reason: reason.clone(),
                    sequence,
                });
                self.pending_timer_ids.remove(timer);
                self.remove_pending_timer_owner(timer, &purpose, event)?;
                if let TimerPurpose::Retry { attempt } = purpose {
                    let retry = self
                        .retries
                        .get(timer)
                        .ok_or_else(|| invalid_at(event, "retry timer has no retry decision"))?;
                    if retry.next_attempt != attempt || retry.state != RetryState::Waiting {
                        return Err(invalid_at(
                            event,
                            "retry timer cancellation contradicts its retry decision",
                        ));
                    }
                    let retry_execution = retry.execution.clone();
                    let harmless_uncertain = self
                        .node_executions
                        .get(&retry_execution)
                        .into_iter()
                        .flat_map(|execution| execution.attempts.iter())
                        .filter_map(|candidate| {
                            let prior = self.attempts.get(candidate)?;
                            (prior.state == AttemptState::Uncertain
                                && prior.side_effect.as_ref().is_some_and(|facts| {
                                    matches!(
                                        facts.side_effect,
                                        SideEffectClass::None | SideEffectClass::ReadOnly
                                    )
                                }))
                            .then(|| candidate.clone())
                        })
                        .collect::<Vec<_>>();
                    self.retries
                        .get_mut(timer)
                        .ok_or_else(|| invalid_at(event, "retry timer has no retry decision"))?
                        .state = RetryState::Cancelled;
                    self.attempts
                        .get_mut(&attempt)
                        .ok_or_else(|| invalid_at(event, "retry timer has no reserved attempt"))?
                        .state = AttemptState::CancelledBeforeDispatch;
                    self.active_attempt_ids.remove(&attempt);
                    self.node_executions
                        .get_mut(&retry_execution)
                        .ok_or_else(|| invalid_at(event, "retry has no owning execution"))?
                        .state = NodeExecutionState::Terminal(NodeOutcome::Cancelled);
                    self.deactivate_execution(&retry_execution, event)?;
                    for prior in harmless_uncertain {
                        self.attempts
                            .get_mut(&prior)
                            .ok_or_else(|| invalid_at(event, "uncertain prior attempt is missing"))?
                            .state = AttemptState::UncertainAbandonedByCancellation {
                            cancelled_retry: attempt.clone(),
                        };
                        self.active_attempt_ids.remove(&prior);
                        self.complete_attempt_leases(&prior);
                    }
                }
            }
            RunEventKind::WaitRegistered {
                execution,
                condition,
            } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.is_completed() || self.waits.contains_key(execution) {
                    return Err(invalid_at(
                        event,
                        "wait is duplicate or follows terminal execution",
                    ));
                }
                if let Some(timer) = wait_condition_timer(condition) {
                    let timer_view = self
                        .timers
                        .get(timer)
                        .ok_or_else(|| invalid_at(event, "wait references an unknown timer"))?;
                    if !matches!(
                        &timer_view.purpose,
                        TimerPurpose::Wait { execution: Some(owner) } if owner == execution
                    ) {
                        return Err(invalid_at(event, "wait timer belongs to another execution"));
                    }
                }
                self.waits.insert(
                    execution.clone(),
                    WaitProjection {
                        execution: execution.clone(),
                        condition: condition.clone(),
                        registered_sequence: sequence,
                        satisfaction: None,
                        cancellation: None,
                    },
                );
                self.pending_wait_execution_ids.insert(execution.clone());
            }
            RunEventKind::WaitSatisfied { execution, cause } => {
                let wait = self
                    .waits
                    .get(execution)
                    .ok_or_else(|| invalid_at(event, "satisfaction references an unknown wait"))?;
                if !wait.is_pending() || !self.wait_cause_matches(wait, cause) {
                    return Err(invalid_at(
                        event,
                        "wait cause is duplicate, incompatible, or not yet durable",
                    ));
                }
                self.waits
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown wait"))?
                    .satisfaction = Some(cause.clone());
                self.pending_wait_execution_ids.remove(execution);
            }
            RunEventKind::WaitCancelled { execution, reason } => {
                let wait = self.waits.get(execution).ok_or_else(|| {
                    invalid_at(event, "wait cancellation references an unknown wait")
                })?;
                let authorized = self
                    .node_executions
                    .get(execution)
                    .and_then(NodeExecutionProjection::cancellation)
                    .is_some()
                    || self.has_execution_cancellation_source(execution);
                if !wait.is_pending() || !authorized {
                    return Err(invalid_at(
                        event,
                        "wait cancellation requires a pending wait and structured owner cancellation",
                    ));
                }
                self.waits
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown wait"))?
                    .cancellation = Some(WaitCancellationProjection {
                        reason: reason.clone(),
                        sequence,
                    });
                self.pending_wait_execution_ids.remove(execution);
            }
            RunEventKind::SignalReceived {
                signal,
                signal_type,
                correlation,
                mode,
                payload,
            } => {
                if self.signals.contains_key(signal) {
                    return Err(invalid_at(event, "signal identity was already received"));
                }
                self.signals.insert(
                    signal.clone(),
                    SignalProjection {
                        signal: signal.clone(),
                        signal_type: signal_type.clone(),
                        correlation: correlation.clone(),
                        mode: *mode,
                        payload: payload.clone(),
                        received_sequence: sequence,
                        consumed_by: BTreeSet::new(),
                        broadcast_scan_through: None,
                        broadcast_scan_complete: false,
                        duplicate_commands: Vec::new(),
                    },
                );
                if *mode == SignalDeliveryMode::Broadcast {
                    self.pending_broadcast_signals
                        .insert((sequence, signal.clone()));
                }
            }
            RunEventKind::SignalBroadcastScanAdvanced {
                signal,
                through_execution,
                complete,
            } => {
                let signal_view = self.signals.get(signal).ok_or_else(|| {
                    invalid_at(event, "broadcast scan references an unknown signal")
                })?;
                if signal_view.mode != SignalDeliveryMode::Broadcast
                    || signal_view.broadcast_scan_complete
                {
                    return Err(invalid_at(
                        event,
                        "broadcast scan requires an incomplete broadcast signal",
                    ));
                }
                let previous = signal_view.broadcast_scan_through.as_ref();
                let cursor_valid = match (previous, through_execution.as_ref()) {
                    (None, None) => *complete,
                    (None, Some(next)) => self.waits.contains_key(next),
                    (Some(_), None) => false,
                    (Some(previous), Some(next)) => {
                        self.waits.contains_key(next)
                            && (next > previous || (*complete && next == previous))
                    }
                };
                if !cursor_valid {
                    return Err(invalid_at(
                        event,
                        "broadcast scan cursor did not advance monotonically through known waits",
                    ));
                }
                let lower = previous
                    .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
                let upper = if *complete {
                    std::ops::Bound::Unbounded
                } else {
                    std::ops::Bound::Included(through_execution.as_ref().ok_or_else(|| {
                        invalid_at(event, "incomplete broadcast scan has no cursor")
                    })?)
                };
                let mut scanned = 0_u32;
                for (_, wait) in self.waits.range((lower, upper)) {
                    scanned = scanned.saturating_add(1);
                    if scanned > MAX_PAGE_SIZE {
                        return Err(invalid_at(
                            event,
                            "one broadcast scan event exceeds the durable wait-page bound",
                        ));
                    }
                    let eligible = wait.is_pending()
                        && wait.registered_sequence() < signal_view.received_sequence
                        && !signal_view.consumed_by.contains(wait.execution())
                        && wait_signal_projection_matches(
                            wait.condition(),
                            &signal_view.signal_type,
                            signal_view.correlation.as_ref(),
                            &self.timers,
                        );
                    if eligible {
                        return Err(invalid_at(
                            event,
                            "broadcast scan cannot advance past an eligible unconsumed wait",
                        ));
                    }
                }
                let signal_view = self.signals.get_mut(signal).ok_or_else(|| {
                    invalid_at(event, "broadcast scan references an unknown signal")
                })?;
                signal_view.broadcast_scan_through = through_execution.clone();
                signal_view.broadcast_scan_complete = *complete;
                if *complete {
                    self.pending_broadcast_signals
                        .remove(&(signal_view.received_sequence, signal.clone()));
                }
            }
            RunEventKind::SignalDeduplicated {
                signal,
                duplicate_command,
            } => {
                if self
                    .signals
                    .values()
                    .any(|received| received.duplicate_commands.contains(duplicate_command))
                {
                    return Err(invalid_at(
                        event,
                        "duplicate signal command identity was already recorded",
                    ));
                }
                let signal_view = self.signals.get_mut(signal).ok_or_else(|| {
                    invalid_at(event, "deduplication references an unknown signal")
                })?;
                signal_view
                    .duplicate_commands
                    .push(duplicate_command.clone());
            }
            RunEventKind::SignalConsumed { signal, execution } => {
                let execution_view = self.execution(execution, event)?;
                let wait_view = self
                    .waits
                    .get(execution)
                    .ok_or_else(|| invalid_at(event, "signal consumer has no registered wait"))?;
                let signal_view = self
                    .signals
                    .get(signal)
                    .ok_or_else(|| invalid_at(event, "consumption references an unknown signal"))?;
                let compatible_wait = match wait_view.condition() {
                    WaitCondition::Signal {
                        signal_type,
                        correlation,
                    } => {
                        signal_view.signal_type == *signal_type
                            && signal_view.correlation == *correlation
                    }
                    WaitCondition::SignalOrTimer {
                        timer,
                        signal_type,
                        correlation,
                    } => {
                        signal_view.signal_type == *signal_type
                            && signal_view.correlation == *correlation
                            && self
                                .timers
                                .get(timer)
                                .is_some_and(TimerProjection::is_pending)
                    }
                    WaitCondition::Timer { .. } => false,
                };
                if execution_view.is_completed()
                    || wait_view.is_completed()
                    || !compatible_wait
                    || signal_view.consumed_by.contains(execution)
                    || (signal_view.mode == SignalDeliveryMode::OneShot
                        && !signal_view.consumed_by.is_empty())
                    || (signal_view.mode == SignalDeliveryMode::Broadcast
                        && wait_view.registered_sequence >= signal_view.received_sequence)
                {
                    return Err(invalid_at(
                        event,
                        "signal consumption is duplicate, incompatible, or violates delivery mode",
                    ));
                }
                self.signals
                    .get_mut(signal)
                    .ok_or_else(|| invalid_at(event, "unknown signal"))?
                    .consumed_by
                    .insert(execution.clone());
            }
            RunEventKind::SubworkflowCreated {
                subworkflow,
                parent_execution,
                child_run,
                child_revision,
                scope,
                ownership,
                inputs,
            } => {
                let parent_scope = self.execution(parent_execution, event)?.scope.clone();
                let valid_parent_scope = scope.parent() == Some(&parent_scope)
                    || self.iterations.values().any(|iteration| {
                        iteration.repeat_execution == *parent_execution
                            && iteration.state == IterationState::Active
                            && scope.parent() == Some(iteration.scope.reference())
                    });
                if self.subworkflows.contains_key(subworkflow)
                    || self.child_runs.contains(child_run)
                    || child_run == event.run_id()
                    || !matches!(scope.kind(), ScopeKind::Subworkflow { subworkflow: identity } if identity == subworkflow)
                    || !valid_parent_scope
                {
                    return Err(invalid_at(
                        event,
                        "subworkflow identity, child run, scope kind, or parent is invalid",
                    ));
                }
                ensure_unique(inputs, event, "subworkflow input")?;
                for input in inputs {
                    if input.scope() != scope.reference() {
                        self.validate_known_workspace_value(input, event)?;
                        let accessible_from_parent = scope.parent().is_some_and(|parent| {
                            input.scope() == parent
                                || self.scope_descends_from(parent, input.scope())
                        });
                        if !accessible_from_parent {
                            return Err(invalid_at(
                                event,
                                "pre-existing subworkflow input is not owned by an ancestor scope",
                            ));
                        }
                    }
                }
                self.register_child_scope(scope, event)?;
                for input in inputs {
                    if input.scope() == scope.reference() {
                        self.record_workspace_value(input, event)?;
                    }
                }
                self.child_runs.insert(child_run.clone());
                self.subworkflows.insert(
                    subworkflow.clone(),
                    SubworkflowProjection {
                        subworkflow: subworkflow.clone(),
                        parent_execution: parent_execution.clone(),
                        child_run: child_run.clone(),
                        child_revision: child_revision.clone(),
                        scope: scope.clone(),
                        ownership: *ownership,
                        inputs: inputs.clone(),
                        state: SubworkflowState::Active,
                        cancellation_reason: None,
                        outputs: Vec::new(),
                        imports: Vec::new(),
                    },
                );
                self.active_subworkflow_ids.insert(subworkflow.clone());
                if *ownership == SubworkflowOwnership::Attached {
                    self.active_attached_subworkflow_ids
                        .insert(subworkflow.clone());
                }
                self.adjust_structured_child_count(parent_execution, true, event)?;
            }
            RunEventKind::SubworkflowTerminal {
                subworkflow,
                child_run,
                outcome,
                outputs,
            } => {
                let child = self.subworkflows.get(subworkflow).ok_or_else(|| {
                    invalid_at(event, "child terminal references an unknown subworkflow")
                })?;
                if child.child_run != *child_run
                    || child.is_completed()
                    || (*outcome == RunOutcome::Cancelled
                        && child.state != SubworkflowState::Cancelling)
                {
                    return Err(invalid_at(
                        event,
                        "child terminal is duplicate or names the wrong run",
                    ));
                }
                ensure_unique(outputs, event, "subworkflow output")?;
                for output in outputs {
                    if output.scope().run() != child_run {
                        return Err(invalid_at(
                            event,
                            "subworkflow terminal output belongs to another run",
                        ));
                    }
                }
                let parent_execution = child.parent_execution.clone();
                let child = self
                    .subworkflows
                    .get_mut(subworkflow)
                    .ok_or_else(|| invalid_at(event, "unknown subworkflow"))?;
                child.state = SubworkflowState::Terminal(*outcome);
                child.outputs = outputs.clone();
                self.active_subworkflow_ids.remove(subworkflow);
                self.active_attached_subworkflow_ids.remove(subworkflow);
                self.adjust_structured_child_count(&parent_execution, false, event)?;
            }
            RunEventKind::SubworkflowOutputImported {
                subworkflow,
                child_value,
                parent_value,
            } => {
                let child = self.subworkflows.get(subworkflow).ok_or_else(|| {
                    invalid_at(event, "output import references an unknown subworkflow")
                })?;
                let parent_scope = self
                    .execution(&child.parent_execution, event)?
                    .scope
                    .clone();
                if !child.is_completed()
                    || child_value.scope().run() != &child.child_run
                    || !child.outputs.contains(child_value)
                    || child
                        .imports
                        .iter()
                        .any(|import| import.parent_value == *parent_value)
                    || self.workspace_values.contains(parent_value)
                {
                    return Err(invalid_at(
                        event,
                        "subworkflow import is duplicate or not backed by its terminal child output",
                    ));
                }
                self.validate_workspace_value(parent_value, event)?;
                if !self.scope_descends_from(parent_value.scope(), &parent_scope) {
                    return Err(invalid_at(
                        event,
                        "subworkflow import target is outside its parent execution scope",
                    ));
                }
                self.subworkflows
                    .get_mut(subworkflow)
                    .ok_or_else(|| invalid_at(event, "unknown subworkflow"))?
                    .imports
                    .push(SubworkflowOutputImport {
                        child_value: child_value.clone(),
                        parent_value: parent_value.clone(),
                        sequence,
                    });
                self.record_workspace_value(parent_value, event)?;
            }
            RunEventKind::SubworkflowCancellationRequested {
                subworkflow,
                child_run,
                reason,
            } => {
                let child = self.subworkflows.get_mut(subworkflow).ok_or_else(|| {
                    invalid_at(
                        event,
                        "child cancellation references an unknown subworkflow",
                    )
                })?;
                if child.child_run != *child_run
                    || child.ownership != SubworkflowOwnership::Attached
                    || child.state != SubworkflowState::Active
                {
                    return Err(invalid_at(
                        event,
                        "child cancellation is duplicate, detached, or mismatched",
                    ));
                }
                child.state = SubworkflowState::Cancelling;
                child.cancellation_reason = Some(reason.clone());
            }
            RunEventKind::RevisionAdoptionRequested {
                reconciliation,
                from_revision,
                to_revision,
                policy,
            } => {
                if self.reconciliation.requests.contains_key(reconciliation)
                    || self.revision.as_ref() != Some(from_revision)
                    || from_revision == to_revision
                    || self.reconciliation.is_active()
                {
                    return Err(invalid_at(
                        event,
                        "revision adoption request is duplicate, stale, or conflicts with an active request",
                    ));
                }
                self.reconciliation.requests.insert(
                    reconciliation.clone(),
                    ReconciliationRequestProjection {
                        reconciliation: reconciliation.clone(),
                        from_revision: from_revision.clone(),
                        to_revision: to_revision.clone(),
                        policy: *policy,
                        sequence,
                        plan: None,
                        state: ReconciliationRequestState::Requested,
                    },
                );
                self.reconciliation.current_request = Some(reconciliation.clone());
            }
            RunEventKind::ReconciliationPlanRecorded {
                reconciliation,
                plan,
                from_revision,
                to_revision,
                based_on_sequence,
                items,
            } => {
                let request = self
                    .reconciliation
                    .requests
                    .get(reconciliation)
                    .ok_or_else(|| {
                        invalid_at(event, "plan references an unknown adoption request")
                    })?;
                if request.state != ReconciliationRequestState::Requested
                    || request.from_revision != *from_revision
                    || request.to_revision != *to_revision
                    || self.reconciliation.plans.contains_key(plan)
                    || request.sequence.get().checked_sub(1) != Some(based_on_sequence.get())
                    || self.sequence != request.sequence
                    || self.revision_at(*based_on_sequence) != Some(from_revision)
                {
                    return Err(invalid_at(
                        event,
                        "reconciliation plan is duplicate, stale, or differs from its request",
                    ));
                }
                for item in items {
                    if !crate::reconciliation::reconciliation_action_is_valid(
                        item.classification,
                        item.action,
                        request.policy,
                    ) {
                        return Err(invalid_at(
                            event,
                            "reconciliation action contradicts its classification or requested policy",
                        ));
                    }
                    if item.node.is_none()
                        && item.execution.is_none()
                        && item.classification
                            != ReconciliationClassification::IncompatibleInterfaceOrSubworkflow
                    {
                        return Err(invalid_at(
                            event,
                            "reconciliation item must name a node or execution",
                        ));
                    }
                    if let Some(execution) = &item.execution {
                        let execution_view = self.execution(execution, event)?;
                        if item
                            .node
                            .as_ref()
                            .is_some_and(|node| node != &execution_view.node)
                        {
                            return Err(invalid_at(
                                event,
                                "reconciliation item node and execution disagree",
                            ));
                        }
                    }
                }
                self.reconciliation.plans.insert(
                    plan.clone(),
                    ReconciliationPlanProjection {
                        reconciliation: reconciliation.clone(),
                        plan: plan.clone(),
                        from_revision: from_revision.clone(),
                        to_revision: to_revision.clone(),
                        based_on_sequence: *based_on_sequence,
                        items: items.clone(),
                        decisions: Vec::new(),
                        applied_sequence: None,
                        stale_sequence: None,
                    },
                );
                let request = self
                    .reconciliation
                    .requests
                    .get_mut(reconciliation)
                    .ok_or_else(|| invalid_at(event, "unknown reconciliation request"))?;
                request.plan = Some(plan.clone());
                request.state = ReconciliationRequestState::Planned;
            }
            RunEventKind::ReconciliationDecisionRecorded {
                plan,
                decision,
                actor,
                outcome,
                reason,
                evidence,
            } => {
                if !matches!(
                    outcome,
                    AuthorityDecision::Approve | AuthorityDecision::Reject
                ) || !self.decision_ids.insert(decision.clone())
                {
                    return Err(invalid_at(
                        event,
                        "reconciliation decision outcome or identity is invalid",
                    ));
                }
                let plan_view = self
                    .reconciliation
                    .plans
                    .get_mut(plan)
                    .ok_or_else(|| invalid_at(event, "decision references an unknown plan"))?;
                if plan_view.applied_sequence.is_some()
                    || plan_view.stale_sequence.is_some()
                    || !plan_view.decisions.is_empty()
                {
                    return Err(invalid_at(
                        event,
                        "decision follows application/staleness or contradicts a prior decision",
                    ));
                }
                plan_view.decisions.push(ReconciliationDecision {
                    decision: decision.clone(),
                    actor: actor.clone(),
                    outcome: *outcome,
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    sequence,
                });
                if *outcome == AuthorityDecision::Reject {
                    self.reconciliation
                        .requests
                        .get_mut(&plan_view.reconciliation)
                        .ok_or_else(|| invalid_at(event, "plan request is missing"))?
                        .state = ReconciliationRequestState::Rejected;
                }
            }
            RunEventKind::ReconciliationExecutionRemoved { plan, execution } => {
                let plan_view = self
                    .reconciliation
                    .plans
                    .get(plan)
                    .ok_or_else(|| invalid_at(event, "removal references an unknown plan"))?;
                let authorized = plan_view.stale_sequence.is_none()
                    && plan_view.applied_sequence.is_none()
                    && plan_view.items.iter().any(|item| {
                        item.execution.as_ref() == Some(execution)
                            && (item.action == ReconciliationAction::RemoveUnstarted
                                || item.action == ReconciliationAction::UseNewOnNextInvocation
                                    && item.classification
                                        == ReconciliationClassification::ChangedPending)
                    });
                let execution_view = self.execution(execution, event)?;
                if !authorized
                    || execution_view.state != NodeExecutionState::Eligible
                    || !execution_view.attempts.is_empty()
                    || self.execution_has_active_structured_ownership(execution)
                {
                    return Err(invalid_at(
                        event,
                        "prospective removal is unauthorized or the execution already started",
                    ));
                }
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::RemovedProspectively(plan.clone());
                self.eligible_executions.remove(execution);
                self.deactivate_execution(execution, event)?;
            }
            RunEventKind::ReconciliationCancellationRequested {
                plan,
                execution,
                attempt,
                reason,
            } => {
                let plan_view =
                    self.reconciliation.plans.get(plan).ok_or_else(|| {
                        invalid_at(event, "cancellation references an unknown plan")
                    })?;
                let authorized = plan_view.stale_sequence.is_none()
                    && plan_view.applied_sequence.is_none()
                    && plan_view.items.iter().any(|item| {
                        item.execution.as_ref() == Some(execution)
                            && item.action == ReconciliationAction::CancelAndRestart
                    });
                let execution_view = self.execution(execution, event)?;
                let attempt_view = self.attempt(attempt, event)?;
                if !authorized
                    || self.reconciliation_cancellations.contains_key(execution)
                    || execution_view.cancellation.is_some()
                    || attempt_view.execution != *execution
                    || execution_view.attempts.last() != Some(attempt)
                    || !matches!(
                        attempt_view.state,
                        AttemptState::Scheduled | AttemptState::Leased | AttemptState::Running
                    )
                {
                    return Err(invalid_at(
                        event,
                        "reconciliation cancellation is duplicate, unauthorized, or not active",
                    ));
                }
                self.reconciliation_cancellations.insert(
                    execution.clone(),
                    ReconciliationCancellationProjection {
                        plan: plan.clone(),
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        reason: reason.clone(),
                        sequence,
                    },
                );
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown reconciliation execution"))?
                    .cancellation = Some(NodeExecutionCancellationProjection {
                    attempt: Some(attempt.clone()),
                    reason: reason.clone(),
                    sequence,
                });
                let source = self.execution(execution, event)?;
                let restart_scope = source.scope.clone();
                let restart_key = (source.node.clone(), restart_scope.clone());
                if self
                    .pending_reconciliation_restarts
                    .insert(restart_key, execution.clone())
                    .is_some()
                {
                    return Err(invalid_at(event, "reconciliation restart token is duplicate"));
                }
                self.adjust_scope_ownership(&restart_scope, true, event)?;
            }
            RunEventKind::ReconciliationRemediationCreated {
                plan,
                source_execution,
                source_attempt,
                execution,
                node,
                scope,
                mode,
                reason,
            } => {
                let plan_view =
                    self.reconciliation.plans.get(plan).ok_or_else(|| {
                        invalid_at(event, "remediation references an unknown plan")
                    })?;
                let authorized = plan_view.stale_sequence.is_none()
                    && plan_view.applied_sequence.is_none()
                    && plan_view.items.iter().any(|item| {
                        item.execution.as_ref() == Some(source_execution)
                            && item.node.as_ref() == Some(node)
                            && item.action == ReconciliationAction::CompensateOrRemediate
                    });
                let source = self.execution(source_execution, event)?;
                if !authorized
                    || self.node_executions.contains_key(execution)
                    || self.reserved_executions.contains(execution)
                    || self.reconciliation_remediations.contains_key(execution)
                    || source_attempt.as_ref().is_some_and(|attempt| {
                        self.attempts
                            .get(attempt)
                            .is_none_or(|attempt| attempt.execution != *source_execution)
                            || !source.attempts.contains(attempt)
                    })
                {
                    return Err(invalid_at(
                        event,
                        "reconciliation remediation is duplicate, unauthorized, or mismatched",
                    ));
                }
                self.validate_scope_reference(scope, event)?;
                self.node_executions.insert(
                    execution.clone(),
                    NodeExecutionProjection {
                        execution: execution.clone(),
                        node: node.clone(),
                        scope: scope.clone(),
                        mode: *mode,
                        created_sequence: sequence,
                        created_at: event.occurred_at(),
                        attempts: Vec::new(),
                        state: NodeExecutionState::Eligible,
                        cancellation: None,
                        deterministic_terminal: None,
                        outputs: Vec::new(),
                    },
                );
                self.execution_ids_by_node
                    .entry(node.clone())
                    .or_default()
                    .insert(execution.clone());
                self.eligible_executions.insert(execution.clone());
                self.activate_execution(execution, event)?;
                self.reconciliation_remediations.insert(
                    execution.clone(),
                    ReconciliationRemediationProjection {
                        plan: plan.clone(),
                        source_execution: source_execution.clone(),
                        source_attempt: source_attempt.clone(),
                        execution: execution.clone(),
                        node: node.clone(),
                        scope: scope.clone(),
                        reason: reason.clone(),
                        sequence,
                    },
                );
            }
            RunEventKind::ReconciliationApplied {
                plan,
                from_revision,
                to_revision,
                based_on_sequence,
            } => {
                if *based_on_sequence != self.sequence
                    || self.revision.as_ref() != Some(from_revision)
                {
                    return Err(invalid_at(event, "reconciliation application is stale"));
                }
                let plan_view =
                    self.reconciliation.plans.get(plan).ok_or_else(|| {
                        invalid_at(event, "application references an unknown plan")
                    })?;
                let request_state = self
                    .reconciliation
                    .requests
                    .get(&plan_view.reconciliation)
                    .map(|request| request.state);
                let needs_authority = plan_view
                    .items
                    .iter()
                    .any(|item| item.action == ReconciliationAction::RequireAuthority);
                let approved = plan_view
                    .decisions
                    .last()
                    .is_some_and(|decision| decision.outcome == AuthorityDecision::Approve);
                let rejected_action = plan_view
                    .items
                    .iter()
                    .any(|item| item.action == ReconciliationAction::RejectRetrospectiveRewrite);
                let actions_enacted = plan_view.items.iter().all(|item| match item.action {
                    ReconciliationAction::RemoveUnstarted => {
                        item.execution.as_ref().is_none_or(|execution| {
                            self.node_executions
                                .get(execution)
                                .is_some_and(|execution| {
                                    execution.state
                                        == NodeExecutionState::RemovedProspectively(plan.clone())
                                })
                        })
                    }
                    ReconciliationAction::UseNewOnNextInvocation
                        if item.classification
                            == ReconciliationClassification::ChangedPending =>
                    {
                        item.execution.as_ref().is_none_or(|execution| {
                            self.node_executions.get(execution).is_some_and(|execution| {
                                execution.state
                                    == NodeExecutionState::RemovedProspectively(plan.clone())
                            })
                        })
                    }
                    ReconciliationAction::CancelAndRestart => {
                        item.execution.as_ref().is_some_and(|execution| {
                            self.reconciliation_cancellations
                                .get(execution)
                                .is_some_and(|cancellation| cancellation.plan == *plan)
                        })
                    }
                    ReconciliationAction::CompensateOrRemediate => {
                        item.execution.as_ref().is_some_and(|source| {
                            self.reconciliation_remediations
                                .values()
                                .any(|remediation| {
                                    remediation.plan == *plan
                                        && remediation.source_execution == *source
                                })
                        })
                    }
                    ReconciliationAction::Preserve
                    | ReconciliationAction::UseNewOnNextInvocation
                    | ReconciliationAction::RequireAuthority
                    | ReconciliationAction::RejectRetrospectiveRewrite => true,
                });
                if plan_view.from_revision != *from_revision
                    || plan_view.to_revision != *to_revision
                    || plan_view.applied_sequence.is_some()
                    || plan_view.stale_sequence.is_some()
                    || self.reconciliation.current_request.as_ref()
                        != Some(&plan_view.reconciliation)
                    || request_state != Some(ReconciliationRequestState::Planned)
                    || (needs_authority && !approved)
                    || !actions_enacted
                    || rejected_action
                    || plan_view
                        .decisions
                        .last()
                        .is_some_and(|decision| decision.outcome == AuthorityDecision::Reject)
                {
                    return Err(invalid_at(
                        event,
                        "plan is mismatched, already applied, or lacks authority",
                    ));
                }
                let reconciliation = plan_view.reconciliation.clone();
                self.reconciliation
                    .plans
                    .get_mut(plan)
                    .ok_or_else(|| invalid_at(event, "unknown plan"))?
                    .applied_sequence = Some(sequence);
                self.reconciliation
                    .requests
                    .get_mut(&reconciliation)
                    .ok_or_else(|| invalid_at(event, "plan request is missing"))?
                    .state = ReconciliationRequestState::Applied;
                self.pending_pin = Some(plan.clone());
            }
            RunEventKind::RecoveryStarted {
                controller,
                through_sequence,
            } => {
                if *through_sequence != self.sequence {
                    return Err(invalid_at(
                        event,
                        "recovery must name the exact journal head examined",
                    ));
                }
                self.recovery.push(RecoveryProjection {
                    controller: controller.clone(),
                    through_sequence: *through_sequence,
                    started_sequence: sequence,
                    classifications: Vec::new(),
                });
                self.current_recovery = self.recovery.len().checked_sub(1);
            }
            RunEventKind::RecoveryClassified {
                attempt,
                lease,
                classification,
                reason,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let lease_view = lease
                    .as_ref()
                    .map(|lease| {
                        self.leases.get(lease).ok_or_else(|| {
                            invalid_at(event, "recovery references an unknown lease")
                        })
                    })
                    .transpose()?;
                if lease_view.is_some_and(|lease| lease.attempt != *attempt) {
                    return Err(invalid_at(
                        event,
                        "recovery lease belongs to another attempt",
                    ));
                }
                let recovery_index = self.current_recovery.ok_or_else(|| {
                    invalid_at(event, "classification has no preceding recovery start")
                })?;
                if self.recovery.get(recovery_index).is_none_or(|recovery| {
                    recovery
                        .classifications
                        .iter()
                        .any(|(classified, _)| classified == attempt)
                }) {
                    return Err(invalid_at(
                        event,
                        "attempt was already classified in this recovery pass",
                    ));
                }
                let retry_safe = attempt_view
                    .side_effect
                    .as_ref()
                    .is_some_and(|side_effect| {
                        matches!(
                            side_effect.side_effect,
                            SideEffectClass::None | SideEffectClass::ReadOnly
                        ) || (side_effect.side_effect == SideEffectClass::IdempotentWrite
                            && side_effect.idempotency != IdempotencyBehavior::Unsupported
                            && side_effect.idempotency_key.is_some())
                    });
                let classification_valid = match classification {
                    RecoveryClassification::LeaseStillValid => lease_view.is_some_and(|lease| {
                        lease.is_active()
                            && lease.expires_at > event.occurred_at()
                            && matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            )
                    }),
                    RecoveryClassification::TerminalObserved => attempt_view.is_completed(),
                    RecoveryClassification::NotStarted => {
                        matches!(
                            attempt_view.state,
                            AttemptState::AwaitingRetryTimer
                                | AttemptState::ReadyToSchedule
                                | AttemptState::Scheduled
                                | AttemptState::Leased
                        ) && lease_view.is_none_or(|lease| {
                            matches!(
                                lease.state,
                                LeaseState::Expired(RecoveryClassification::NotStarted)
                            )
                        })
                    }
                    RecoveryClassification::Retryable => {
                        retry_safe
                            && (attempt_view.is_unresolved()
                                && lease_view.is_none()
                                || matches!(
                                    attempt_view.state,
                                    AttemptState::Leased | AttemptState::Running
                                ) && lease_view.is_some_and(|lease| {
                                    matches!(
                                        lease.state,
                                        LeaseState::Expired(RecoveryClassification::Retryable)
                                    )
                                }))
                    }
                    RecoveryClassification::Uncertain => {
                        attempt_view.is_unresolved()
                            || (matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            ) && lease_view.is_some_and(|lease| {
                                matches!(
                                    lease.state,
                                    LeaseState::Expired(RecoveryClassification::Uncertain)
                                )
                            }) && !retry_safe)
                    }
                };
                if !classification_valid {
                    return Err(invalid_at(
                        event,
                        "recovery classification contradicts projected attempt state",
                    ));
                }
                let observation = RecoveryObservation {
                    lease: lease.clone(),
                    classification: *classification,
                    reason: reason.clone(),
                    sequence,
                };
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .recovery
                    .push(observation.clone());
                self.recovery
                    .get_mut(recovery_index)
                    .ok_or_else(|| invalid_at(event, "current recovery pass is missing"))?
                    .classifications
                    .push((attempt.clone(), observation));
            }
            RunEventKind::RecoveryDecisionRecorded {
                attempt,
                decision,
                actor,
                outcome,
                reason,
                evidence,
            } => {
                if !matches!(
                    outcome,
                    AuthorityDecision::Retain
                        | AuthorityDecision::Query
                        | AuthorityDecision::Retry
                        | AuthorityDecision::Compensate
                        | AuthorityDecision::ResolveSucceeded
                        | AuthorityDecision::ResolveFailed
                ) || matches!(
                    outcome,
                    AuthorityDecision::ResolveSucceeded | AuthorityDecision::ResolveFailed
                ) && evidence.is_empty()
                    || !self.decision_ids.insert(decision.clone())
                {
                    return Err(invalid_at(
                        event,
                        "recovery decision outcome or identity is invalid",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "decision references an unknown attempt"))?;
                let obligation = attempt_view.obligation.as_mut().ok_or_else(|| {
                    invalid_at(event, "decision requires uncertain or retained work")
                })?;
                obligation.decisions.push(RecoveryDecision {
                    decision: decision.clone(),
                    actor: actor.clone(),
                    outcome: *outcome,
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    sequence,
                });
                let execution = attempt_view.execution.clone();
                let resolved = match outcome {
                    AuthorityDecision::ResolveSucceeded => Some(NodeOutcome::Succeeded),
                    AuthorityDecision::ResolveFailed => Some(NodeOutcome::Failed),
                    AuthorityDecision::Retain
                    | AuthorityDecision::Query
                    | AuthorityDecision::Retry
                    | AuthorityDecision::Compensate => None,
                    AuthorityDecision::Approve | AuthorityDecision::Reject => None,
                };
                if let Some(outcome) = resolved {
                    attempt_view.state = AttemptState::Resolved(outcome);
                    attempt_view.obligation = None;
                    self.active_attempt_ids.remove(attempt);
                    self.node_executions
                        .get_mut(&execution)
                        .ok_or_else(|| invalid_at(event, "unknown execution"))?
                        .state = NodeExecutionState::Terminal(outcome);
                    self.deactivate_execution(&execution, event)?;
                    if outcome == NodeOutcome::Succeeded {
                        self.pending_successor_executions.insert(execution.clone());
                    }
                    self.complete_attempt_leases(attempt);
                }
                self.recovery_decisions
                    .insert(decision.clone(), (attempt.clone(), *outcome));
            }
            RunEventKind::RemediationWorkCreated {
                source_attempt,
                execution,
                node,
                scope,
                mode,
                decision,
                reason,
            } => {
                let source = self.attempt(source_attempt, event)?;
                let source_has_obligation = source.obligation.is_some();
                let source_execution = source.execution.clone();
                let source_scope = self.execution(&source_execution, event)?.scope.clone();
                if !source_has_obligation
                    || self.recovery_decisions.get(decision)
                        != Some(&(source_attempt.clone(), AuthorityDecision::Compensate))
                    || self.remediations.contains_key(execution)
                    || self.reserved_executions.contains(execution)
                    || self.node_executions.contains_key(execution)
                    || *scope != source_scope
                {
                    return Err(invalid_at(
                        event,
                        "remediation lacks authority or reuses an execution identity",
                    ));
                }
                self.validate_scope_reference(scope, event)?;
                self.node_executions.insert(
                    execution.clone(),
                    NodeExecutionProjection {
                        execution: execution.clone(),
                        node: node.clone(),
                        scope: scope.clone(),
                        mode: *mode,
                        created_sequence: sequence,
                        created_at: event.occurred_at(),
                        attempts: Vec::new(),
                        state: NodeExecutionState::Eligible,
                        cancellation: None,
                        deterministic_terminal: None,
                        outputs: Vec::new(),
                    },
                );
                self.execution_ids_by_node
                    .entry(node.clone())
                    .or_default()
                    .insert(execution.clone());
                self.eligible_executions.insert(execution.clone());
                self.activate_execution(execution, event)?;
                self.remediations.insert(
                    execution.clone(),
                    RemediationProjection {
                        source_attempt: source_attempt.clone(),
                        execution: execution.clone(),
                        node: node.clone(),
                        scope: scope.clone(),
                        mode: *mode,
                        decision: decision.clone(),
                        reason: reason.clone(),
                        sequence,
                    },
                );
            }
            RunEventKind::RunCreated { .. }
            | RunEventKind::RevisionPinned { .. }
            | RunEventKind::RunStarted
            | RunEventKind::RunPaused { .. }
            | RunEventKind::RunResumed { .. }
            | RunEventKind::RunCancellationRequested { .. }
            | RunEventKind::RunTerminationRequested { .. }
            | RunEventKind::RunTerminal { .. }
            | RunEventKind::NodeBecameEligible { .. }
            | RunEventKind::NodeExecutionCancelledBeforeDispatch { .. }
            | RunEventKind::NodeExecutionCancellationRequested { .. }
            | RunEventKind::NodeScheduled { .. }
            | RunEventKind::CapabilityResolved { .. }
            | RunEventKind::SideEffectClassified { .. }
            | RunEventKind::LeaseGranted { .. }
            | RunEventKind::LeaseHeartbeatRecorded { .. }
            | RunEventKind::LeaseExpired { .. }
            | RunEventKind::NodeReLeased { .. }
            | RunEventKind::NodeStarted { .. }
            | RunEventKind::NodeProgressRecorded { .. }
            | RunEventKind::AttemptUsageRecorded { .. }
            | RunEventKind::InvocationCancellationAcknowledged { .. }
            | RunEventKind::NodeOutputPublished { .. }
            | RunEventKind::DeterministicOutputPublished { .. }
            | RunEventKind::DeterministicNodeTerminal { .. }
            | RunEventKind::NodePreDispatchFailed { .. }
            | RunEventKind::StructuredSuccessorScanCompleted { .. }
            | RunEventKind::NodeTerminal { .. }
            | RunEventKind::NodeRetryScheduled { .. }
            | RunEventKind::ExternalOutcomeUncertain { .. }
            | RunEventKind::ExternalOutcomeRetained { .. }
            | RunEventKind::ArtifactPublished { .. } => {
                return Err(invalid_at(
                    event,
                    "internal projection event routing failure",
                ));
            }
        }
        Ok(())
    }
}

impl RunProjection {
    fn execution<'a>(
        &'a self,
        execution: &NodeExecutionId,
        event: &RunEventEnvelope,
    ) -> Result<&'a NodeExecutionProjection, RuntimeError> {
        self.node_executions
            .get(execution)
            .ok_or_else(|| invalid_at(event, format!("unknown node execution '{execution}'")))
    }

    fn attempt<'a>(
        &'a self,
        attempt: &AttemptId,
        event: &RunEventEnvelope,
    ) -> Result<&'a NodeAttemptProjection, RuntimeError> {
        self.attempts
            .get(attempt)
            .ok_or_else(|| invalid_at(event, format!("unknown attempt '{attempt}'")))
    }

    fn validate_scope_reference(
        &self,
        scope: &ScopeReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        if scope.run() != event.run_id() || !self.scopes.contains_key(scope) {
            return Err(invalid_at(
                event,
                "workspace scope is unknown or belongs to another run",
            ));
        }
        Ok(())
    }

    fn register_child_scope(
        &mut self,
        scope: &WorkspaceScope,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        if scope.reference().run() != event.run_id()
            || scope.kind().is_run_root()
            || scope
                .parent()
                .is_none_or(|parent| !self.scopes.contains_key(parent))
            || self.scopes.contains_key(scope.reference())
        {
            return Err(invalid_at(
                event,
                "child scope is duplicate, parentless, or has an unknown parent",
            ));
        }
        self.scopes.insert(scope.reference().clone(), scope.clone());
        Ok(())
    }

    pub(crate) fn scope_descends_from(
        &self,
        scope: &ScopeReference,
        ancestor: &ScopeReference,
    ) -> bool {
        let mut current = Some(scope);
        let mut remaining = self.scopes.len().saturating_add(1);
        while let Some(reference) = current {
            if reference == ancestor {
                return true;
            }
            if remaining == 0 {
                return false;
            }
            remaining -= 1;
            current = self.scopes.get(reference).and_then(WorkspaceScope::parent);
        }
        false
    }

    fn validate_workspace_value(
        &self,
        value: &WorkspaceValueReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        self.validate_scope_reference(value.scope(), event)
    }

    fn validate_known_workspace_value(
        &self,
        value: &WorkspaceValueReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        self.validate_workspace_value(value, event)?;
        if !self.workspace_values.contains(value) {
            return Err(invalid_at(
                event,
                "workspace value reference was not introduced by prior run history",
            ));
        }
        Ok(())
    }

    fn record_workspace_value(
        &mut self,
        value: &WorkspaceValueReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        self.validate_workspace_value(value, event)?;
        if self.workspace_values.insert(value.clone()) {
            let next = self
                .resource_usage
                .workspace_value_references
                .checked_add(1)
                .ok_or_else(|| invalid_at(event, "workspace value-reference count overflow"))?;
            if self
                .workspace_budget
                .as_ref()
                .is_some_and(|budget| next > budget.max_value_versions())
            {
                return Err(invalid_at(
                    event,
                    "workspace value references exceed the pinned budget",
                ));
            }
            self.resource_usage.workspace_value_references = next;
        }
        Ok(())
    }

    fn validate_published_artifact(
        &self,
        artifact: &ArtifactReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let metadata = self
            .artifacts
            .get(artifact.artifact())
            .ok_or_else(|| invalid_at(event, "artifact reference precedes publication metadata"))?;
        if metadata.reference() != artifact {
            return Err(invalid_at(
                event,
                "artifact reference contradicts published metadata",
            ));
        }
        Ok(())
    }

    fn apply_artifact_publication(
        &mut self,
        metadata: &ArtifactMetadata,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let reference = metadata.reference();
        if self.artifacts.contains_key(reference.artifact()) {
            return Err(invalid_at(event, "artifact identity was already published"));
        }
        self.validate_causal_reference(metadata.provenance().producer(), event)?;
        for cause in metadata.provenance().causes() {
            self.validate_causal_reference(cause, event)?;
        }
        let budget = self
            .workspace_budget
            .as_ref()
            .ok_or_else(|| invalid_at(event, "artifact publication precedes run creation"))?;
        let artifacts = self
            .resource_usage
            .artifacts
            .checked_add(1)
            .ok_or_else(|| invalid_at(event, "artifact count overflow"))?;
        let artifact_bytes = self
            .resource_usage
            .artifact_bytes
            .checked_add(reference.size_bytes())
            .ok_or_else(|| invalid_at(event, "artifact byte accounting overflow"))?;
        if reference.size_bytes() > budget.max_bytes_per_artifact()
            || artifacts > budget.max_artifacts()
            || artifact_bytes > budget.max_total_artifact_bytes()
        {
            return Err(invalid_at(
                event,
                "artifact publication exceeds the pinned workspace budget",
            ));
        }
        self.resource_usage.artifacts = artifacts;
        self.resource_usage.artifact_bytes = artifact_bytes;
        self.artifacts
            .insert(reference.artifact().clone(), metadata.clone());
        Ok(())
    }

    fn validate_causal_reference(
        &self,
        reference: &CausalReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        match reference {
            CausalReference::RunInput { run, .. } if run == event.run_id() => Ok(()),
            CausalReference::WorkspaceValue { reference } => {
                self.validate_known_workspace_value(reference, event)
            }
            CausalReference::Artifact { reference } => {
                self.validate_published_artifact(reference, event)
            }
            CausalReference::Invocation { invocation } if self.invocations.contains(invocation) => {
                Ok(())
            }
            CausalReference::External { .. } => Ok(()),
            CausalReference::RunInput { .. } | CausalReference::Invocation { .. } => {
                Err(invalid_at(
                    event,
                    "artifact provenance references an unknown or foreign fact",
                ))
            }
        }
    }

    fn has_execution_cancellation_source(&self, execution: &NodeExecutionId) -> bool {
        self.cancellation.is_some()
            || self.termination.is_some()
            || self
                .branch_owner
                .get(execution)
                .and_then(|branch| self.branches.get(branch))
                .and_then(BranchProjection::cancellation_reason)
                .is_some()
            || self.reconciliation_cancellations.contains_key(execution)
            || self.subworkflows.values().any(|child| {
                child.parent_execution == *execution && child.cancellation_reason.is_some()
            })
    }

    fn ensure_terminal_quiescent(&self, event: &RunEventEnvelope) -> Result<(), RuntimeError> {
        let outstanding = self.has_active_owned_work();
        if outstanding {
            return Err(invalid_at(
                event,
                "run terminal boundary would abandon active owned work or an unresolved obligation",
            ));
        }
        Ok(())
    }

    fn adjust_scope_ownership(
        &mut self,
        scope: &ScopeReference,
        add: bool,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let mut current = Some(scope.clone());
        while let Some(reference) = current {
            let parent = self
                .scopes
                .get(&reference)
                .and_then(WorkspaceScope::parent)
                .cloned();
            if add {
                let count = self
                    .active_scope_ownership
                    .entry(reference.clone())
                    .or_default();
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| invalid_at(event, "active scope ownership overflow"))?;
            } else {
                let count = self
                    .active_scope_ownership
                    .get_mut(&reference)
                    .ok_or_else(|| invalid_at(event, "active scope ownership underflow"))?;
                *count = count
                    .checked_sub(1)
                    .ok_or_else(|| invalid_at(event, "active scope ownership underflow"))?;
                if *count == 0 {
                    self.active_scope_ownership.remove(&reference);
                }
            }
            current = parent;
        }
        Ok(())
    }

    fn activate_execution(
        &mut self,
        execution: &NodeExecutionId,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let scope = self.execution(execution, event)?.scope.clone();
        if !self.active_execution_ids.insert(execution.clone()) {
            return Err(invalid_at(event, "execution was already active"));
        }
        let execution_view = self.execution(execution, event)?;
        let node = execution_view.node.clone();
        let mut current = Some(scope.clone());
        while let Some(reference) = current {
            let parent = self
                .scopes
                .get(&reference)
                .and_then(WorkspaceScope::parent)
                .cloned();
            self.latest_descendant_execution_by_scope_node
                .insert((reference, node.clone()), execution.clone());
            current = parent;
        }
        self.adjust_scope_ownership(&scope, true, event)
    }

    fn deactivate_execution(
        &mut self,
        execution: &NodeExecutionId,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let scope = self.execution(execution, event)?.scope.clone();
        if !self.active_execution_ids.remove(execution) {
            return Err(invalid_at(event, "execution was not active"));
        }
        self.adjust_scope_ownership(&scope, false, event)
    }

    fn adjust_structured_child_count(
        &mut self,
        execution: &NodeExecutionId,
        add: bool,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        if add {
            let count = self
                .active_structured_children_by_execution
                .entry(execution.clone())
                .or_default();
            *count = count
                .checked_add(1)
                .ok_or_else(|| invalid_at(event, "structured child ownership overflow"))?;
        } else {
            let count = self
                .active_structured_children_by_execution
                .get_mut(execution)
                .ok_or_else(|| invalid_at(event, "structured child ownership underflow"))?;
            *count = count
                .checked_sub(1)
                .ok_or_else(|| invalid_at(event, "structured child ownership underflow"))?;
            if *count == 0 {
                self.active_structured_children_by_execution
                    .remove(execution);
            }
        }
        Ok(())
    }

    fn remove_pending_timer_owner(
        &mut self,
        timer: &TimerId,
        purpose: &TimerPurpose,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let owner = match purpose {
            TimerPurpose::Wait {
                execution: Some(execution),
            } => Some(execution.clone()),
            TimerPurpose::Retry { attempt } => self
                .attempts
                .get(attempt)
                .map(|attempt| attempt.execution.clone()),
            TimerPurpose::Wait { execution: None } => None,
        };
        if let Some(owner) = owner {
            let timers = self
                .pending_timers_by_execution
                .get_mut(&owner)
                .ok_or_else(|| invalid_at(event, "pending timer owner index is absent"))?;
            if !timers.remove(timer) {
                return Err(invalid_at(event, "pending timer owner index disagrees"));
            }
            if timers.is_empty() {
                self.pending_timers_by_execution.remove(&owner);
            }
        }
        Ok(())
    }

    fn complete_attempt_leases(&mut self, attempt: &AttemptId) {
        if let Some(lease) = self.active_lease_by_attempt.remove(attempt) {
            if let Some(lease) = self.leases.get_mut(&lease) {
                lease.state = LeaseState::Completed;
            }
        }
    }

    fn accumulate_usage(
        &mut self,
        usage: &AttemptUsage,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        add_optional_usage(
            &mut self.resource_usage.input_units,
            usage.input_units,
            event,
            "input units",
        )?;
        add_optional_usage(
            &mut self.resource_usage.output_units,
            usage.output_units,
            event,
            "output units",
        )?;
        add_optional_usage(
            &mut self.resource_usage.duration_ms,
            usage.duration_ms,
            event,
            "duration",
        )?;
        if let Some(cost) = &usage.cost {
            let total = self
                .resource_usage
                .cost_micros
                .entry(cost.currency.clone())
                .or_default();
            *total = total
                .checked_add(cost.micros)
                .ok_or_else(|| invalid_at(event, "monetary usage overflow"))?;
        }
        Ok(())
    }

    /// Returns the exact immutable revision governing a durable event sequence.
    ///
    /// The value is absent before run creation or beyond the projected journal
    /// head. Attempt provenance uses its recorded scheduling sequence here rather
    /// than assuming the run's current prospective pin.
    #[must_use]
    pub fn revision_at(&self, sequence: RunSequence) -> Option<&RevisionId> {
        if sequence == RunSequence::ZERO || sequence > self.sequence {
            return None;
        }
        self.pins
            .iter()
            .rev()
            .find(|pin| pin.effective_sequence <= sequence)
            .map(|pin| &pin.revision)
    }

    fn wait_cause_matches(&self, wait: &WaitProjection, cause: &WaitSatisfaction) -> bool {
        match (wait.condition(), cause) {
            (WaitCondition::Timer { timer: expected }, WaitSatisfaction::Timer { timer })
            | (
                WaitCondition::SignalOrTimer {
                    timer: expected, ..
                },
                WaitSatisfaction::Timer { timer },
            ) if expected == timer => {
                self.timers
                    .get(timer)
                    .is_some_and(TimerProjection::is_completed)
                    && !self
                        .signals
                        .values()
                        .any(|signal| signal.consumed_by.contains(&wait.execution))
            }
            (
                WaitCondition::Signal {
                    signal_type,
                    correlation,
                }
                | WaitCondition::SignalOrTimer {
                    signal_type,
                    correlation,
                    ..
                },
                WaitSatisfaction::Signal { signal },
            ) => self.signals.get(signal).is_some_and(|received| {
                received.signal_type == *signal_type
                    && received.correlation == *correlation
                    && received.consumed_by.contains(&wait.execution)
            }),
            _ => false,
        }
    }
}

fn new_attempt(
    attempt: AttemptId,
    execution: NodeExecutionId,
    attempt_number: u32,
    state: AttemptState,
) -> NodeAttemptProjection {
    NodeAttemptProjection {
        attempt,
        execution,
        attempt_number,
        invocation: None,
        idempotency_key: None,
        request: None,
        scheduled_sequence: None,
        state,
        capability: None,
        side_effect: None,
        leases: Vec::new(),
        progress: Vec::new(),
        last_report_sequence: None,
        usage: None,
        cancellation_acknowledgements: Vec::new(),
        outputs: Vec::new(),
        terminal: None,
        obligation: None,
        recovery: Vec::new(),
    }
}

fn same_logical_invocation_request(left: &InvocationRequest, right: &InvocationRequest) -> bool {
    left.capability() == right.capability()
        && left.operation() == right.operation()
        && left.provider_profile() == right.provider_profile()
        && left.inputs() == right.inputs()
        && left.extensions() == right.extensions()
}

fn wait_condition_timer(condition: &WaitCondition) -> Option<&TimerId> {
    match condition {
        WaitCondition::Timer { timer } | WaitCondition::SignalOrTimer { timer, .. } => Some(timer),
        WaitCondition::Signal { .. } => None,
    }
}

fn wait_signal_projection_matches(
    condition: &WaitCondition,
    signal_type: &SignalTypeId,
    correlation: Option<&CorrelationKey>,
    timers: &BTreeMap<TimerId, TimerProjection>,
) -> bool {
    match condition {
        WaitCondition::Signal {
            signal_type: expected,
            correlation: expected_correlation,
        } => expected == signal_type && expected_correlation.as_ref() == correlation,
        WaitCondition::SignalOrTimer {
            timer,
            signal_type: expected,
            correlation: expected_correlation,
        } => {
            expected == signal_type
                && expected_correlation.as_ref() == correlation
                && timers.get(timer).is_some_and(TimerProjection::is_pending)
        }
        WaitCondition::Timer { .. } => false,
    }
}

fn ensure_unique<T: Ord>(
    items: &[T],
    event: &RunEventEnvelope,
    kind: &str,
) -> Result<(), RuntimeError> {
    let mut unique = BTreeSet::new();
    if items.iter().all(|item| unique.insert(item)) {
        Ok(())
    } else {
        Err(invalid_at(event, format!("duplicate {kind}")))
    }
}

fn ensure_unique_by<T, K: Ord, F: Fn(&T) -> K>(
    items: &[T],
    key: F,
    event: &RunEventEnvelope,
    kind: &str,
) -> Result<(), RuntimeError> {
    let mut unique = BTreeSet::new();
    if items.iter().all(|item| unique.insert(key(item))) {
        Ok(())
    } else {
        Err(invalid_at(event, format!("duplicate {kind}")))
    }
}

fn add_optional_usage(
    total: &mut Option<u64>,
    observation: Option<u64>,
    event: &RunEventEnvelope,
    resource: &str,
) -> Result<(), RuntimeError> {
    if let Some(observation) = observation {
        *total = Some(
            total
                .unwrap_or(0)
                .checked_add(observation)
                .ok_or_else(|| invalid_at(event, format!("{resource} usage overflow")))?,
        );
    }
    Ok(())
}

fn invalid(reason: impl Into<String>) -> RuntimeError {
    RuntimeError::InvalidHistory(reason.into())
}

fn invalid_at(event: &RunEventEnvelope, reason: impl AsRef<str>) -> RuntimeError {
    invalid(format!(
        "event {} at sequence {}: {}",
        event.event_id(),
        event.sequence(),
        reason.as_ref()
    ))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use milkdrift_capability::{
        AdmissionConstraints, CapabilityCategory, CapabilityId, CapabilityRequirement,
        DescriptorBuilder, IdempotencyKey, Locality, OperationContract, OperationId,
        ProviderProfileRef, ResolvedCapabilitySnapshotDocument,
    };
    use milkdrift_persistence::{EventId, ReconciliationClassification};
    use milkdrift_workspace::ScopeId;

    use super::*;

    type TestResult = Result<(), Box<dyn Error>>;

    struct Fixture {
        run: RunId,
        root: WorkspaceScope,
        workflow: WorkflowId,
        revision: RevisionId,
        digest: ContentDigest,
        budget: WorkspaceBudget,
    }

    fn fixture(name: &str) -> Result<Fixture, Box<dyn Error>> {
        let run = RunId::new(format!("run-{name}"))?;
        let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
        Ok(Fixture {
            run,
            root,
            workflow: WorkflowId::new(format!("workflow-{name}"))?,
            revision: revision('a')?,
            digest: digest('1')?,
            budget: WorkspaceBudget::new(100, 10_000, 100_000, 100, 100_000, 1_000_000)?,
        })
    }

    fn revision(character: char) -> Result<RevisionId, Box<dyn Error>> {
        Ok(serde_json::from_str(&format!(
            "\"rev_{}\"",
            character.to_string().repeat(64)
        ))?)
    }

    fn digest(character: char) -> Result<ContentDigest, Box<dyn Error>> {
        Ok(serde_json::from_str(&format!(
            "\"b3_{}\"",
            character.to_string().repeat(64)
        ))?)
    }

    fn envelope(
        sequence: u64,
        run: &RunId,
        kind: RunEventKind,
    ) -> Result<RunEventEnvelope, milkdrift_persistence::PersistenceError> {
        RunEventEnvelope::new(
            EventId::new(format!("event-{sequence}"))?,
            run.clone(),
            RunSequence::new(sequence),
            TimestampMillis::new(sequence.saturating_mul(100)),
            kind,
        )
    }

    fn created(
        fixture: &Fixture,
        sequence: u64,
    ) -> Result<RunEventEnvelope, milkdrift_persistence::PersistenceError> {
        envelope(
            sequence,
            &fixture.run,
            RunEventKind::RunCreated {
                workflow: fixture.workflow.clone(),
                revision: fixture.revision.clone(),
                revision_digest: fixture.digest.clone(),
                root_scope: fixture.root.clone(),
                workspace_budget: fixture.budget.clone(),
                inputs: Vec::new(),
            },
        )
    }

    #[test]
    fn replay_equals_incremental_apply() -> TestResult {
        let fixture = fixture("deterministic")?;
        let execution = NodeExecutionId::new("execution-entry")?;
        let events = vec![
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            envelope(
                3,
                &fixture.run,
                RunEventKind::NodeBecameEligible {
                    node: NodeId::new("entry")?,
                    execution,
                    scope: fixture.root.reference().clone(),
                    mode: NodeExecutionMode::Executor,
                },
            )?,
        ];

        let replayed = RunProjection::replay(&events)?;
        let mut incremental = RunProjection::new();
        for event in &events {
            incremental.apply(event)?;
        }
        assert_eq!(replayed, incremental);
        assert_eq!(replayed.sequence(), RunSequence::new(3));
        assert!(replayed.is_active());
        Ok(())
    }

    #[test]
    fn cancelling_lifecycle_rejects_non_cancelled_run_terminal_facts() -> TestResult {
        let fixture = fixture("cancellation-terminal-guard")?;
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            envelope(
                3,
                &fixture.run,
                RunEventKind::RunCancellationRequested {
                    reason: Reason::new("operator cancelled")?,
                    evidence: Vec::new(),
                },
            )?,
        ])?;
        for outcome in [RunOutcome::Succeeded, RunOutcome::Failed] {
            let before = projection.clone();
            assert!(
                projection
                    .apply(&envelope(
                        4,
                        &fixture.run,
                        RunEventKind::RunTerminal {
                            outcome,
                            outputs: Vec::new(),
                            artifacts: Vec::new(),
                            reason: None,
                        },
                    )?)
                    .is_err()
            );
            assert_eq!(projection, before);
        }
        projection.apply(&envelope(
            4,
            &fixture.run,
            RunEventKind::RunTerminal {
                outcome: RunOutcome::Cancelled,
                outputs: Vec::new(),
                artifacts: Vec::new(),
                reason: Some(Reason::new("operator cancelled")?),
            },
        )?)?;
        assert_eq!(
            projection.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Cancelled)
        );
        Ok(())
    }

    #[test]
    fn rejects_gaps_wrong_runs_and_illegal_transitions_atomically() -> TestResult {
        let primary = fixture("invalid")?;
        let other = fixture("other")?;

        let gap = vec![
            created(&primary, 1)?,
            envelope(3, &primary.run, RunEventKind::RunStarted)?,
        ];
        assert!(matches!(
            RunProjection::replay(&gap),
            Err(RuntimeError::InvalidHistory(_))
        ));

        let wrong_run = vec![
            created(&primary, 1)?,
            envelope(2, &other.run, RunEventKind::RunStarted)?,
        ];
        assert!(matches!(
            RunProjection::replay(&wrong_run),
            Err(RuntimeError::InvalidHistory(_))
        ));

        let mut projection = RunProjection::replay(&[created(&primary, 1)?])?;
        let before = projection.clone();
        let illegal = envelope(
            2,
            &primary.run,
            RunEventKind::RunPaused {
                reason: Reason::new("not running")?,
                evidence: Vec::new(),
            },
        )?;
        assert!(matches!(
            projection.apply(&illegal),
            Err(RuntimeError::InvalidHistory(_))
        ));
        assert_eq!(projection, before);
        Ok(())
    }

    #[test]
    fn projects_structured_scopes_waits_signals_and_subworkflows() -> TestResult {
        let fixture = fixture("structured")?;
        let fork = NodeExecutionId::new("execution-fork")?;
        let child = NodeExecutionId::new("execution-branch-child")?;
        let join = NodeExecutionId::new("execution-join")?;
        let repeat = NodeExecutionId::new("execution-repeat")?;
        let wait_timer = NodeExecutionId::new("execution-wait-timer")?;
        let wait_signal = NodeExecutionId::new("execution-wait-signal")?;
        let parent = NodeExecutionId::new("execution-subworkflow")?;
        let branch = BranchId::new("branch-a")?;
        let branch_scope =
            WorkspaceScope::branch(ScopeId::new("branch-scope")?, &fixture.root, branch.clone())?;
        let branch_output = WorkspaceValueReference::new(
            branch_scope.reference().clone(),
            milkdrift_workspace::ValueKey::new("result")?,
            milkdrift_workspace::ValueVersion::FIRST,
        );
        let iteration = IterationId::new("iteration-1")?;
        let iteration_scope = WorkspaceScope::iteration(
            ScopeId::new("iteration-scope")?,
            &fixture.root,
            iteration.clone(),
        )?;
        let timer = TimerId::new("timer-wait")?;
        let signal = SignalId::new("signal-ready")?;
        let signal_type = SignalTypeId::new("example.ready")?;
        let subworkflow = SubworkflowId::new("subworkflow-child")?;
        let child_scope = WorkspaceScope::subworkflow(
            ScopeId::new("subworkflow-scope")?,
            &fixture.root,
            subworkflow.clone(),
        )?;
        let child_run = RunId::new("run-structured-child")?;
        let events = vec![
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            runtime_eligible(3, &fixture, "fork", &fork, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::BranchScopeCreated {
                    fork_execution: fork,
                    port: PortId::new("branch-a")?,
                    branch: branch.clone(),
                    scope: branch_scope.clone(),
                },
            )?,
            runtime_eligible(
                5,
                &fixture,
                "branch-child",
                &child,
                branch_scope.reference(),
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::BranchChildAdded {
                    branch: branch.clone(),
                    execution: child,
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::DeterministicOutputPublished {
                    execution: NodeExecutionId::new("execution-branch-child")?,
                    value: branch_output,
                    artifact: None,
                },
            )?,
            envelope(
                8,
                &fixture.run,
                RunEventKind::DeterministicNodeTerminal {
                    execution: NodeExecutionId::new("execution-branch-child")?,
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            )?,
            envelope(
                9,
                &fixture.run,
                RunEventKind::BranchTerminal {
                    branch: branch.clone(),
                    outcome: RunOutcome::Succeeded,
                    outputs: Vec::new(),
                },
            )?,
            runtime_eligible(10, &fixture, "join", &join, fixture.root.reference())?,
            envelope(
                11,
                &fixture.run,
                RunEventKind::JoinSatisfied {
                    execution: join,
                    rule: JoinRule::All,
                    branches: vec![BranchResultReference {
                        branch: branch.clone(),
                        scope: branch_scope.reference().clone(),
                        outcome: RunOutcome::Succeeded,
                        outputs: Vec::new(),
                    }],
                    retained_branches: Vec::new(),
                },
            )?,
            runtime_eligible(12, &fixture, "repeat", &repeat, fixture.root.reference())?,
            envelope(
                13,
                &fixture.run,
                RunEventKind::RepeatIterationCreated {
                    repeat_execution: repeat.clone(),
                    iteration: iteration.clone(),
                    iteration_number: 1,
                    scope: iteration_scope,
                },
            )?,
            envelope(
                14,
                &fixture.run,
                RunEventKind::RepeatConditionRecorded {
                    iteration: iteration.clone(),
                    result: false,
                },
            )?,
            envelope(
                15,
                &fixture.run,
                RunEventKind::RepeatTerminated {
                    repeat_execution: repeat,
                    termination: RepeatTerminationReason::ConditionFalse,
                    last_iteration: Some(iteration),
                },
            )?,
            runtime_eligible(
                16,
                &fixture,
                "wait-timer",
                &wait_timer,
                fixture.root.reference(),
            )?,
            envelope(
                17,
                &fixture.run,
                RunEventKind::TimerRegistered {
                    timer: timer.clone(),
                    execution: Some(wait_timer.clone()),
                    fire_at: TimestampMillis::new(1_800),
                },
            )?,
            envelope(
                18,
                &fixture.run,
                RunEventKind::WaitRegistered {
                    execution: wait_timer.clone(),
                    condition: WaitCondition::Timer {
                        timer: timer.clone(),
                    },
                },
            )?,
            envelope(
                19,
                &fixture.run,
                RunEventKind::TimerFired {
                    timer: timer.clone(),
                    observed_at: TimestampMillis::new(1_900),
                },
            )?,
            envelope(
                20,
                &fixture.run,
                RunEventKind::WaitSatisfied {
                    execution: wait_timer,
                    cause: WaitSatisfaction::Timer { timer },
                },
            )?,
            runtime_eligible(
                21,
                &fixture,
                "wait-signal",
                &wait_signal,
                fixture.root.reference(),
            )?,
            envelope(
                22,
                &fixture.run,
                RunEventKind::WaitRegistered {
                    execution: wait_signal.clone(),
                    condition: WaitCondition::Signal {
                        signal_type: signal_type.clone(),
                        correlation: None,
                    },
                },
            )?,
            envelope(
                23,
                &fixture.run,
                RunEventKind::SignalReceived {
                    signal: signal.clone(),
                    signal_type,
                    correlation: None,
                    mode: SignalDeliveryMode::OneShot,
                    payload: BoundedJson::new(serde_json::json!({"ready": true}))?,
                },
            )?,
            envelope(
                24,
                &fixture.run,
                RunEventKind::SignalConsumed {
                    signal: signal.clone(),
                    execution: wait_signal.clone(),
                },
            )?,
            envelope(
                25,
                &fixture.run,
                RunEventKind::WaitSatisfied {
                    execution: wait_signal,
                    cause: WaitSatisfaction::Signal { signal },
                },
            )?,
            runtime_eligible(
                26,
                &fixture,
                "subworkflow",
                &parent,
                fixture.root.reference(),
            )?,
            envelope(
                27,
                &fixture.run,
                RunEventKind::SubworkflowCreated {
                    subworkflow: subworkflow.clone(),
                    parent_execution: parent,
                    child_run: child_run.clone(),
                    child_revision: revision('b')?,
                    scope: child_scope,
                    ownership: SubworkflowOwnership::Attached,
                    inputs: Vec::new(),
                },
            )?,
            envelope(
                28,
                &fixture.run,
                RunEventKind::SubworkflowTerminal {
                    subworkflow,
                    child_run,
                    outcome: RunOutcome::Succeeded,
                    outputs: Vec::new(),
                },
            )?,
        ];

        let projection = RunProjection::replay(&events)?;
        assert_eq!(projection.branches().len(), 1);
        assert_eq!(projection.iterations().len(), 1);
        assert_eq!(projection.waits().len(), 2);
        assert!(
            projection
                .waits()
                .values()
                .all(WaitProjection::is_completed)
        );
        assert!(
            projection
                .subworkflows()
                .values()
                .all(SubworkflowProjection::is_completed)
        );
        Ok(())
    }

    #[test]
    fn signals_support_queued_one_shot_and_preexisting_broadcast_waiters() -> TestResult {
        let fixture = fixture("signal-delivery")?;
        let signal_type = SignalTypeId::new("example.ready")?;
        let queued_signal = SignalId::new("signal-queued")?;
        let queued_wait = NodeExecutionId::new("execution-queued-wait")?;
        let broadcast_signal = SignalId::new("signal-broadcast")?;
        let broadcast_wait = NodeExecutionId::new("execution-broadcast-wait")?;
        let late_wait = NodeExecutionId::new("execution-late-wait")?;
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            envelope(
                3,
                &fixture.run,
                RunEventKind::SignalReceived {
                    signal: queued_signal.clone(),
                    signal_type: signal_type.clone(),
                    correlation: None,
                    mode: SignalDeliveryMode::OneShot,
                    payload: BoundedJson::new(serde_json::json!({"queued": true}))?,
                },
            )?,
            runtime_eligible(
                4,
                &fixture,
                "queued-wait",
                &queued_wait,
                fixture.root.reference(),
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::WaitRegistered {
                    execution: queued_wait.clone(),
                    condition: WaitCondition::Signal {
                        signal_type: signal_type.clone(),
                        correlation: None,
                    },
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::SignalConsumed {
                    signal: queued_signal.clone(),
                    execution: queued_wait.clone(),
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::WaitSatisfied {
                    execution: queued_wait.clone(),
                    cause: WaitSatisfaction::Signal {
                        signal: queued_signal.clone(),
                    },
                },
            )?,
            runtime_eligible(
                8,
                &fixture,
                "broadcast-wait",
                &broadcast_wait,
                fixture.root.reference(),
            )?,
            envelope(
                9,
                &fixture.run,
                RunEventKind::WaitRegistered {
                    execution: broadcast_wait.clone(),
                    condition: WaitCondition::Signal {
                        signal_type: signal_type.clone(),
                        correlation: None,
                    },
                },
            )?,
            envelope(
                10,
                &fixture.run,
                RunEventKind::SignalReceived {
                    signal: broadcast_signal.clone(),
                    signal_type: signal_type.clone(),
                    correlation: None,
                    mode: SignalDeliveryMode::Broadcast,
                    payload: BoundedJson::new(serde_json::json!({"broadcast": true}))?,
                },
            )?,
        ])?;
        assert!(projection.signals()[&queued_signal].is_completed());
        assert!(projection.waits()[&queued_wait].is_completed());
        assert!(projection.signals()[&broadcast_signal].is_completed());
        assert!(!projection.signals()[&broadcast_signal].is_pending());
        assert!(
            projection.signals()[&broadcast_signal]
                .consumed_by()
                .is_empty()
        );

        projection.apply(&envelope(
            11,
            &fixture.run,
            RunEventKind::SignalConsumed {
                signal: broadcast_signal.clone(),
                execution: broadcast_wait.clone(),
            },
        )?)?;
        projection.apply(&envelope(
            12,
            &fixture.run,
            RunEventKind::WaitSatisfied {
                execution: broadcast_wait,
                cause: WaitSatisfaction::Signal {
                    signal: broadcast_signal.clone(),
                },
            },
        )?)?;
        projection.apply(&runtime_eligible(
            13,
            &fixture,
            "late-wait",
            &late_wait,
            fixture.root.reference(),
        )?)?;
        projection.apply(&envelope(
            14,
            &fixture.run,
            RunEventKind::WaitRegistered {
                execution: late_wait.clone(),
                condition: WaitCondition::Signal {
                    signal_type,
                    correlation: None,
                },
            },
        )?)?;
        let late_broadcast_consumption = envelope(
            15,
            &fixture.run,
            RunEventKind::SignalConsumed {
                signal: broadcast_signal,
                execution: late_wait,
            },
        )?;
        assert!(projection.apply(&late_broadcast_consumption).is_err());
        assert_eq!(projection.sequence(), RunSequence::new(14));
        Ok(())
    }

    #[test]
    fn signal_deduplication_command_identity_cannot_name_two_signals() -> TestResult {
        let fixture = fixture("signal-dedup-command")?;
        let first = SignalId::new("signal-first")?;
        let second = SignalId::new("signal-second")?;
        let duplicate_command = CommandId::new("command-duplicate-delivery")?;
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            envelope(
                3,
                &fixture.run,
                RunEventKind::SignalReceived {
                    signal: first.clone(),
                    signal_type: SignalTypeId::new("example.first")?,
                    correlation: None,
                    mode: SignalDeliveryMode::OneShot,
                    payload: BoundedJson::new(serde_json::json!({"value": 1}))?,
                },
            )?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::SignalReceived {
                    signal: second.clone(),
                    signal_type: SignalTypeId::new("example.second")?,
                    correlation: None,
                    mode: SignalDeliveryMode::OneShot,
                    payload: BoundedJson::new(serde_json::json!({"value": 2}))?,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::SignalDeduplicated {
                    signal: first,
                    duplicate_command: duplicate_command.clone(),
                },
            )?,
        ])?;

        let before = projection.clone();
        let contradictory = envelope(
            6,
            &fixture.run,
            RunEventKind::SignalDeduplicated {
                signal: second,
                duplicate_command,
            },
        )?;
        assert!(projection.apply(&contradictory).is_err());
        assert_eq!(projection, before);
        Ok(())
    }

    fn eligible(
        sequence: u64,
        fixture: &Fixture,
        node: &str,
        execution: &NodeExecutionId,
        scope: &ScopeReference,
    ) -> Result<RunEventEnvelope, Box<dyn Error>> {
        Ok(envelope(
            sequence,
            &fixture.run,
            RunEventKind::NodeBecameEligible {
                node: NodeId::new(node)?,
                execution: execution.clone(),
                scope: scope.clone(),
                mode: NodeExecutionMode::Executor,
            },
        )?)
    }

    fn runtime_eligible(
        sequence: u64,
        fixture: &Fixture,
        node: &str,
        execution: &NodeExecutionId,
        scope: &ScopeReference,
    ) -> Result<RunEventEnvelope, Box<dyn Error>> {
        Ok(envelope(
            sequence,
            &fixture.run,
            RunEventKind::NodeBecameEligible {
                node: NodeId::new(node)?,
                execution: execution.clone(),
                scope: scope.clone(),
                mode: NodeExecutionMode::Runtime,
            },
        )?)
    }

    fn invocation_request(
        invocation: &InvocationId,
        idempotency_key: Option<IdempotencyKey>,
    ) -> Result<InvocationRequest, Box<dyn Error>> {
        Ok(InvocationRequest::new(
            invocation.clone(),
            CapabilityId::new("publisher-primary")?,
            OperationId::new("tool.publish")?,
            Some(ProviderProfileRef::new("publisher-prod")?),
            idempotency_key,
            Vec::new(),
            std::collections::BTreeMap::new(),
        )?)
    }

    fn resolved_snapshot_at(
        descriptor_revision: u64,
    ) -> Result<ResolvedCapabilitySnapshot, Box<dyn Error>> {
        let base = ResolvedCapabilitySnapshotDocument::from_json(include_bytes!(
            "../../capability/tests/fixtures/resolved-capability-snapshot-v1.json"
        ))?;
        let operation = base.body().operation().clone();
        let descriptor = DescriptorBuilder::new(
            base.body().capability().clone(),
            descriptor_revision,
            CapabilityCategory::Tool,
            AdmissionConstraints::new(1, 1)?,
            Locality::Remote,
        )
        .provider_profile(base.body().provider_profile().cloned())
        .operations(std::collections::BTreeMap::from([(
            operation.clone(),
            base.body().operation_contract().clone(),
        )]))
        .build()?;
        Ok(ResolvedCapabilitySnapshot::from_descriptor(
            &descriptor,
            &operation,
        )?)
    }

    fn resolved_snapshot_with_side_effect(
        descriptor_revision: u64,
        side_effect: SideEffectClass,
        idempotency: IdempotencyBehavior,
    ) -> Result<ResolvedCapabilitySnapshot, Box<dyn Error>> {
        let base = ResolvedCapabilitySnapshotDocument::from_json(include_bytes!(
            "../../capability/tests/fixtures/resolved-capability-snapshot-v1.json"
        ))?;
        let operation = base.body().operation().clone();
        let contract = base.body().operation_contract();
        let operation_contract = OperationContract::new(
            contract.input().clone(),
            contract.output().clone(),
            contract.streaming().clone(),
            contract.cancellation(),
            idempotency,
            side_effect,
            contract.features().clone(),
        )?;
        let descriptor = DescriptorBuilder::new(
            base.body().capability().clone(),
            descriptor_revision,
            CapabilityCategory::Tool,
            AdmissionConstraints::new(1, 1)?,
            Locality::Remote,
        )
        .provider_profile(base.body().provider_profile().cloned())
        .operations(std::collections::BTreeMap::from([(
            operation.clone(),
            operation_contract,
        )]))
        .build()?;
        Ok(ResolvedCapabilitySnapshot::from_descriptor(
            &descriptor,
            &operation,
        )?)
    }

    #[test]
    fn keeps_uncertain_retained_work_visible_through_cancellation_and_recovery() -> TestResult {
        let fixture = fixture("recovery")?;
        let execution = NodeExecutionId::new("execution-side-effect")?;
        let attempt = AttemptId::new("attempt-1")?;
        let invocation = InvocationId::new("invocation-1")?;
        let key = IdempotencyKey::new("idempotency-1")?;
        let lease = LeaseId::new("lease-1")?;
        let decision = ReconciliationDecisionId::new("decision-retain")?;
        let snapshot_document = ResolvedCapabilitySnapshotDocument::from_json(include_bytes!(
            "../../capability/tests/fixtures/resolved-capability-snapshot-v1.json"
        ))?;
        let snapshot = snapshot_document.body().clone();
        let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
            .provider_profile(ProviderProfileRef::new("publisher-prod")?);
        let events = vec![
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            eligible(
                3,
                &fixture,
                "side-effect",
                &execution,
                fixture.root.reference(),
            )?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::NodeScheduled {
                    node: NodeId::new("side-effect")?,
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                    idempotency_key: Some(key.clone()),
                    request: invocation_request(&invocation, Some(key.clone()))?,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::CapabilityResolved {
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    requirement,
                    snapshot,
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::SideEffectClassified {
                    attempt: attempt.clone(),
                    side_effect: SideEffectClass::IdempotentWrite,
                    idempotency: IdempotencyBehavior::ProviderProfileScoped,
                    idempotency_key: Some(key),
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::LeaseGranted {
                    lease: lease.clone(),
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    worker: WorkerId::new("worker-1")?,
                    expires_at: TimestampMillis::new(10_000),
                },
            )?,
            envelope(
                8,
                &fixture.run,
                RunEventKind::NodeStarted {
                    execution,
                    attempt: attempt.clone(),
                    invocation,
                },
            )?,
            envelope(
                9,
                &fixture.run,
                RunEventKind::ExternalOutcomeUncertain {
                    attempt: attempt.clone(),
                    report_sequence: 1,
                    side_effect: SideEffectClass::IdempotentWrite,
                    reason: Reason::new("worker disconnected after dispatch")?,
                    evidence: Vec::new(),
                },
            )?,
            envelope(
                10,
                &fixture.run,
                RunEventKind::RunCancellationRequested {
                    reason: Reason::new("operator stopped the run")?,
                    evidence: Vec::new(),
                },
            )?,
            envelope(
                11,
                &fixture.run,
                RunEventKind::RecoveryStarted {
                    controller: WorkerId::new("recovery-controller")?,
                    through_sequence: RunSequence::new(10),
                },
            )?,
            envelope(
                12,
                &fixture.run,
                RunEventKind::RecoveryClassified {
                    attempt: attempt.clone(),
                    lease: Some(lease),
                    classification: RecoveryClassification::Uncertain,
                    reason: Reason::new("external receipt is unavailable")?,
                },
            )?,
            envelope(
                13,
                &fixture.run,
                RunEventKind::RecoveryDecisionRecorded {
                    attempt: attempt.clone(),
                    decision: decision.clone(),
                    actor: ActorRef::new("operator")?,
                    outcome: AuthorityDecision::Retain,
                    reason: Reason::new("retain for later investigation")?,
                    evidence: Vec::new(),
                },
            )?,
            envelope(
                14,
                &fixture.run,
                RunEventKind::ExternalOutcomeRetained {
                    attempt: attempt.clone(),
                    decision,
                    reason: Reason::new("investigation remains open")?,
                },
            )?,
        ];

        let projection = RunProjection::replay(&events)?;
        assert_eq!(projection.lifecycle(), RunLifecycle::Cancelling);
        assert_eq!(projection.unresolved_attempts().count(), 1);
        assert_eq!(
            projection
                .attempts()
                .get(&attempt)
                .map(NodeAttemptProjection::state),
            Some(&AttemptState::Retained)
        );
        Ok(())
    }

    #[test]
    fn recovery_query_preserves_obligation_and_remediation_creates_real_work() -> TestResult {
        let fixture = fixture("recovery-query-remediation")?;
        let source_execution = NodeExecutionId::new("execution-source")?;
        let source_attempt = AttemptId::new("attempt-source")?;
        let invocation = InvocationId::new("invocation-source")?;
        let key = IdempotencyKey::new("idempotency-source")?;
        let lease = LeaseId::new("lease-source")?;
        let query = ReconciliationDecisionId::new("decision-query")?;
        let compensate = ReconciliationDecisionId::new("decision-compensate")?;
        let remediation = NodeExecutionId::new("execution-remediation")?;
        let snapshot_document = ResolvedCapabilitySnapshotDocument::from_json(include_bytes!(
            "../../capability/tests/fixtures/resolved-capability-snapshot-v1.json"
        ))?;
        let snapshot = snapshot_document.body().clone();
        let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
            .provider_profile(ProviderProfileRef::new("publisher-prod")?);
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            eligible(
                3,
                &fixture,
                "source",
                &source_execution,
                fixture.root.reference(),
            )?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::NodeScheduled {
                    node: NodeId::new("source")?,
                    execution: source_execution.clone(),
                    attempt: source_attempt.clone(),
                    invocation: invocation.clone(),
                    idempotency_key: Some(key.clone()),
                    request: invocation_request(&invocation, Some(key.clone()))?,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::CapabilityResolved {
                    execution: source_execution.clone(),
                    attempt: source_attempt.clone(),
                    requirement,
                    snapshot,
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::SideEffectClassified {
                    attempt: source_attempt.clone(),
                    side_effect: SideEffectClass::IdempotentWrite,
                    idempotency: IdempotencyBehavior::ProviderProfileScoped,
                    idempotency_key: Some(key),
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::LeaseGranted {
                    lease,
                    execution: source_execution.clone(),
                    attempt: source_attempt.clone(),
                    worker: WorkerId::new("worker-source")?,
                    expires_at: TimestampMillis::new(10_000),
                },
            )?,
            envelope(
                8,
                &fixture.run,
                RunEventKind::NodeStarted {
                    execution: source_execution.clone(),
                    attempt: source_attempt.clone(),
                    invocation,
                },
            )?,
            envelope(
                9,
                &fixture.run,
                RunEventKind::ExternalOutcomeUncertain {
                    attempt: source_attempt.clone(),
                    report_sequence: 1,
                    side_effect: SideEffectClass::IdempotentWrite,
                    reason: Reason::new("external result needs investigation")?,
                    evidence: Vec::new(),
                },
            )?,
            envelope(
                10,
                &fixture.run,
                RunEventKind::RecoveryDecisionRecorded {
                    attempt: source_attempt.clone(),
                    decision: query,
                    actor: ActorRef::new("operator")?,
                    outcome: AuthorityDecision::Query,
                    reason: Reason::new("query status without resolving truth")?,
                    evidence: Vec::new(),
                },
            )?,
        ])?;
        let source = &projection.attempts()[&source_attempt];
        assert_eq!(source.state(), &AttemptState::Uncertain);
        let obligation = source.obligation().ok_or("missing obligation")?;
        assert!(obligation.retained().is_none());
        assert_eq!(obligation.decisions().len(), 1);
        assert_eq!(
            obligation.decisions()[0].outcome(),
            AuthorityDecision::Query
        );

        projection.apply(&envelope(
            11,
            &fixture.run,
            RunEventKind::RecoveryDecisionRecorded {
                attempt: source_attempt.clone(),
                decision: compensate.clone(),
                actor: ActorRef::new("operator")?,
                outcome: AuthorityDecision::Compensate,
                reason: Reason::new("create explicit remediation")?,
                evidence: Vec::new(),
            },
        )?)?;
        projection.apply(&envelope(
            12,
            &fixture.run,
            RunEventKind::RemediationWorkCreated {
                source_attempt: source_attempt.clone(),
                execution: remediation.clone(),
                node: NodeId::new("remediation")?,
                scope: fixture.root.reference().clone(),
                mode: NodeExecutionMode::Runtime,
                decision: compensate,
                reason: Reason::new("runtime-owned remediation")?,
            },
        )?)?;
        assert_eq!(
            projection.node_executions()[&remediation].state(),
            &NodeExecutionState::Eligible
        );
        assert_eq!(
            projection.node_executions()[&remediation].mode(),
            NodeExecutionMode::Runtime
        );
        assert_eq!(
            projection.remediations()[&remediation].scope(),
            fixture.root.reference()
        );
        assert!(
            projection.attempts()[&source_attempt]
                .obligation()
                .is_some()
        );
        projection.apply(&envelope(
            13,
            &fixture.run,
            RunEventKind::DeterministicNodeTerminal {
                execution: remediation,
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?)?;
        Ok(())
    }

    #[test]
    fn rejects_stale_reconciliation_and_requires_immediate_exact_repin() -> TestResult {
        let fixture = fixture("reconciliation")?;
        let reconciliation = ReconciliationId::new("reconciliation-1")?;
        let plan = ReconciliationPlanId::new("plan-1")?;
        let next_revision = revision('b')?;
        let next_digest = digest('2')?;
        let mut projection = RunProjection::new();
        for event in [
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            envelope(
                3,
                &fixture.run,
                RunEventKind::RevisionAdoptionRequested {
                    reconciliation: reconciliation.clone(),
                    from_revision: fixture.revision.clone(),
                    to_revision: next_revision.clone(),
                    policy: ReconciliationPolicy::FinishCurrentThenAdopt,
                },
            )?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::ReconciliationPlanRecorded {
                    reconciliation,
                    plan: plan.clone(),
                    from_revision: fixture.revision.clone(),
                    to_revision: next_revision.clone(),
                    based_on_sequence: RunSequence::new(2),
                    items: vec![ReconciliationItem {
                        node: Some(NodeId::new("future-node")?),
                        execution: None,
                        classification: ReconciliationClassification::Added,
                        action: ReconciliationAction::UseNewOnNextInvocation,
                        reason: Reason::new("new prospective node")?,
                    }],
                },
            )?,
        ] {
            projection.apply(&event)?;
        }

        let before = projection.clone();
        let stale = envelope(
            5,
            &fixture.run,
            RunEventKind::ReconciliationApplied {
                plan: plan.clone(),
                from_revision: fixture.revision.clone(),
                to_revision: next_revision.clone(),
                based_on_sequence: RunSequence::new(3),
            },
        )?;
        assert!(matches!(
            projection.apply(&stale),
            Err(RuntimeError::InvalidHistory(_))
        ));
        assert_eq!(projection, before);

        projection.apply(&envelope(
            5,
            &fixture.run,
            RunEventKind::ReconciliationApplied {
                plan: plan.clone(),
                from_revision: fixture.revision.clone(),
                to_revision: next_revision.clone(),
                based_on_sequence: RunSequence::new(4),
            },
        )?)?;
        projection.apply(&envelope(
            6,
            &fixture.run,
            RunEventKind::RevisionPinned {
                previous: fixture.revision,
                revision: next_revision.clone(),
                revision_digest: next_digest,
                plan,
            },
        )?)?;
        assert_eq!(projection.revision(), Some(&next_revision));
        assert_eq!(projection.sequence(), RunSequence::new(6));
        Ok(())
    }

    #[test]
    fn reconciliation_removal_without_a_created_execution_is_an_enacted_noop() -> TestResult {
        let fixture = fixture("reconciliation-uncreated-removal")?;
        let reconciliation = ReconciliationId::new("reconciliation-remove")?;
        let plan = ReconciliationPlanId::new("plan-remove")?;
        let removed_node = NodeId::new("removed-before-eligibility")?;
        let next_revision = revision('b')?;
        let next_digest = digest('2')?;
        let projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            envelope(
                3,
                &fixture.run,
                RunEventKind::RevisionAdoptionRequested {
                    reconciliation: reconciliation.clone(),
                    from_revision: fixture.revision.clone(),
                    to_revision: next_revision.clone(),
                    policy: ReconciliationPolicy::RemoveUnstartedOnly,
                },
            )?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::ReconciliationPlanRecorded {
                    reconciliation,
                    plan: plan.clone(),
                    from_revision: fixture.revision.clone(),
                    to_revision: next_revision.clone(),
                    based_on_sequence: RunSequence::new(2),
                    items: vec![ReconciliationItem {
                        node: Some(removed_node),
                        execution: None,
                        classification: ReconciliationClassification::RemovedPending,
                        action: ReconciliationAction::RemoveUnstarted,
                        reason: Reason::new("node never became eligible")?,
                    }],
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::ReconciliationApplied {
                    plan: plan.clone(),
                    from_revision: fixture.revision.clone(),
                    to_revision: next_revision.clone(),
                    based_on_sequence: RunSequence::new(4),
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::RevisionPinned {
                    previous: fixture.revision,
                    revision: next_revision.clone(),
                    revision_digest: next_digest,
                    plan,
                },
            )?,
        ])?;

        assert_eq!(projection.revision(), Some(&next_revision));
        assert!(projection.node_executions().is_empty());
        Ok(())
    }

    #[test]
    fn reconciliation_removal_rejects_eligible_execution_with_live_wait_ownership() -> TestResult {
        let fixture = fixture("reconciliation-live-wait-removal")?;
        let execution = NodeExecutionId::new("execution-live-wait")?;
        let timer = TimerId::new("timer-live-wait")?;
        let reconciliation = ReconciliationId::new("reconciliation-live-wait")?;
        let plan = ReconciliationPlanId::new("plan-live-wait")?;
        let next_revision = revision('b')?;
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            runtime_eligible(3, &fixture, "wait", &execution, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::TimerRegistered {
                    timer: timer.clone(),
                    execution: Some(execution.clone()),
                    fire_at: TimestampMillis::new(10_000),
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::WaitRegistered {
                    execution: execution.clone(),
                    condition: WaitCondition::Timer { timer },
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::RevisionAdoptionRequested {
                    reconciliation: reconciliation.clone(),
                    from_revision: fixture.revision.clone(),
                    to_revision: next_revision.clone(),
                    policy: ReconciliationPolicy::RemoveUnstartedOnly,
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::ReconciliationPlanRecorded {
                    reconciliation,
                    plan: plan.clone(),
                    from_revision: fixture.revision.clone(),
                    to_revision: next_revision,
                    based_on_sequence: RunSequence::new(5),
                    items: vec![ReconciliationItem {
                        node: Some(NodeId::new("wait")?),
                        execution: Some(execution.clone()),
                        classification: ReconciliationClassification::RemovedPending,
                        action: ReconciliationAction::RemoveUnstarted,
                        reason: Reason::new("maliciously treated a live wait as unstarted")?,
                    }],
                },
            )?,
        ])?;
        assert!(projection.execution_has_active_structured_ownership(&execution));
        let before = projection.clone();
        assert!(
            projection
                .apply(&envelope(
                    8,
                    &fixture.run,
                    RunEventKind::ReconciliationExecutionRemoved { plan, execution },
                )?)
                .is_err()
        );
        assert_eq!(projection, before);
        Ok(())
    }

    #[test]
    fn reconciliation_cancellation_is_execution_cancellation_authority() -> TestResult {
        let fixture = fixture("reconciliation-cancellation")?;
        let execution = NodeExecutionId::new("execution-active")?;
        let attempt = AttemptId::new("attempt-active")?;
        let invocation = InvocationId::new("invocation-active")?;
        let reconciliation = ReconciliationId::new("reconciliation-cancel")?;
        let plan = ReconciliationPlanId::new("plan-cancel")?;
        let next_revision = revision('b')?;
        let next_digest = digest('2')?;
        let reason = Reason::new("cancel safely for prospective restart")?;
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            eligible(3, &fixture, "task", &execution, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::NodeScheduled {
                    node: NodeId::new("task")?,
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                    idempotency_key: None,
                    request: invocation_request(&invocation, None)?,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::CapabilityResolved {
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    requirement: CapabilityRequirement::new(OperationId::new("tool.publish")?)
                        .provider_profile(ProviderProfileRef::new("publisher-prod")?),
                    snapshot: resolved_snapshot_with_side_effect(
                        7,
                        SideEffectClass::None,
                        IdempotencyBehavior::Unsupported,
                    )?,
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::SideEffectClassified {
                    attempt: attempt.clone(),
                    side_effect: SideEffectClass::None,
                    idempotency: IdempotencyBehavior::Unsupported,
                    idempotency_key: None,
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::LeaseGranted {
                    lease: LeaseId::new("lease-active")?,
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    worker: WorkerId::new("worker-active")?,
                    expires_at: TimestampMillis::new(10_000),
                },
            )?,
            envelope(
                8,
                &fixture.run,
                RunEventKind::RevisionAdoptionRequested {
                    reconciliation: reconciliation.clone(),
                    from_revision: fixture.revision.clone(),
                    to_revision: next_revision.clone(),
                    policy: ReconciliationPolicy::CancelAndRestartSafeWork,
                },
            )?,
            envelope(
                9,
                &fixture.run,
                RunEventKind::ReconciliationPlanRecorded {
                    reconciliation,
                    plan: plan.clone(),
                    from_revision: fixture.revision.clone(),
                    to_revision: next_revision.clone(),
                    based_on_sequence: RunSequence::new(7),
                    items: vec![ReconciliationItem {
                        node: Some(NodeId::new("task")?),
                        execution: Some(execution.clone()),
                        classification: ReconciliationClassification::ChangedActive,
                        action: ReconciliationAction::CancelAndRestart,
                        reason: reason.clone(),
                    }],
                },
            )?,
            envelope(
                10,
                &fixture.run,
                RunEventKind::ReconciliationCancellationRequested {
                    plan: plan.clone(),
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    reason: reason.clone(),
                },
            )?,
            envelope(
                11,
                &fixture.run,
                RunEventKind::ReconciliationApplied {
                    plan: plan.clone(),
                    from_revision: fixture.revision.clone(),
                    to_revision: next_revision.clone(),
                    based_on_sequence: RunSequence::new(10),
                },
            )?,
            envelope(
                12,
                &fixture.run,
                RunEventKind::RevisionPinned {
                    previous: fixture.revision.clone(),
                    revision: next_revision,
                    revision_digest: next_digest,
                    plan,
                },
            )?,
        ])?;

        let cancellation = projection.node_executions()[&execution]
            .cancellation()
            .ok_or("reconciliation cancellation was not projected on the execution")?;
        assert_eq!(cancellation.attempt(), Some(&attempt));
        assert_eq!(cancellation.reason(), &reason);

        projection.apply(&envelope(
            13,
            &fixture.run,
            RunEventKind::NodeTerminal {
                execution: execution.clone(),
                attempt,
                report_sequence: 1,
                outcome: NodeOutcome::Cancelled,
                error_class: None,
                detail: None,
            },
        )?)?;
        assert_eq!(
            projection.node_executions()[&execution].state(),
            &NodeExecutionState::Terminal(NodeOutcome::Cancelled)
        );
        Ok(())
    }

    #[test]
    fn projects_attempt_free_deterministic_terminal_and_rejects_fabricated_attempts() -> TestResult
    {
        let fixture = fixture("deterministic-terminal")?;
        let direct = NodeExecutionId::new("execution-direct")?;
        let scheduled = NodeExecutionId::new("execution-scheduled")?;
        let runtime_open = NodeExecutionId::new("execution-runtime-open")?;
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            runtime_eligible(3, &fixture, "direct", &direct, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::DeterministicNodeTerminal {
                    execution: direct.clone(),
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            )?,
            eligible(
                5,
                &fixture,
                "scheduled",
                &scheduled,
                fixture.root.reference(),
            )?,
            runtime_eligible(
                6,
                &fixture,
                "runtime-open",
                &runtime_open,
                fixture.root.reference(),
            )?,
        ])?;

        let terminal = projection
            .node_executions()
            .get(&direct)
            .and_then(NodeExecutionProjection::deterministic_terminal)
            .ok_or("deterministic terminal missing")?;
        assert_eq!(terminal.outcome(), NodeOutcome::Succeeded);
        assert!(projection.node_executions()[&direct].attempts().is_empty());
        assert_eq!(
            projection.node_executions()[&scheduled].mode(),
            NodeExecutionMode::Executor
        );
        assert_eq!(
            projection.node_executions()[&runtime_open].mode(),
            NodeExecutionMode::Runtime
        );

        let before = projection.clone();
        assert!(
            projection
                .apply(&envelope(
                    7,
                    &fixture.run,
                    RunEventKind::DeterministicNodeTerminal {
                        execution: scheduled.clone(),
                        outcome: NodeOutcome::Succeeded,
                        error_class: None,
                        detail: None,
                    },
                )?)
                .is_err()
        );
        assert_eq!(projection, before);

        let value = WorkspaceValueReference::new(
            fixture.root.reference().clone(),
            milkdrift_workspace::ValueKey::new("deterministic-output")?,
            milkdrift_workspace::ValueVersion::FIRST,
        );
        assert!(
            projection
                .apply(&envelope(
                    7,
                    &fixture.run,
                    RunEventKind::DeterministicOutputPublished {
                        execution: scheduled.clone(),
                        value,
                        artifact: None,
                    },
                )?)
                .is_err()
        );
        assert!(
            projection
                .apply(&envelope(
                    7,
                    &fixture.run,
                    RunEventKind::NodeScheduled {
                        node: NodeId::new("runtime-open")?,
                        execution: runtime_open,
                        attempt: AttemptId::new("attempt-runtime-open")?,
                        invocation: InvocationId::new("invocation-runtime-open")?,
                        idempotency_key: None,
                        request: invocation_request(
                            &InvocationId::new("invocation-runtime-open")?,
                            None,
                        )?,
                    },
                )?)
                .is_err()
        );
        projection.apply(&envelope(
            7,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("scheduled")?,
                execution: scheduled,
                attempt: AttemptId::new("attempt-scheduled")?,
                invocation: InvocationId::new("invocation-scheduled")?,
                idempotency_key: None,
                request: invocation_request(&InvocationId::new("invocation-scheduled")?, None)?,
            },
        )?)?;
        Ok(())
    }

    #[test]
    fn persisted_invocation_request_must_match_frozen_capability_resolution() -> TestResult {
        let fixture = fixture("request-provenance")?;
        let execution = NodeExecutionId::new("execution-task")?;
        let attempt = AttemptId::new("attempt-task")?;
        let invocation = InvocationId::new("invocation-task")?;
        let request = InvocationRequest::new(
            invocation.clone(),
            CapabilityId::new("different-capability")?,
            OperationId::new("tool.publish")?,
            Some(ProviderProfileRef::new("publisher-prod")?),
            None,
            Vec::new(),
            std::collections::BTreeMap::new(),
        )?;
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            eligible(3, &fixture, "task", &execution, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::NodeScheduled {
                    node: NodeId::new("task")?,
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                    idempotency_key: None,
                    request,
                },
            )?,
        ])?;
        assert_eq!(
            projection.attempts()[&attempt]
                .request()
                .map(InvocationRequest::invocation),
            Some(&invocation)
        );
        assert_eq!(
            projection.attempts()[&attempt].scheduled_sequence(),
            Some(RunSequence::new(4))
        );
        assert_eq!(
            projection.revision_for_attempt(&attempt),
            Some(&fixture.revision)
        );
        assert_eq!(
            projection.revision_at(RunSequence::new(4)),
            Some(&fixture.revision)
        );
        assert_eq!(projection.revision_at(RunSequence::new(5)), None);
        let snapshot_document = ResolvedCapabilitySnapshotDocument::from_json(include_bytes!(
            "../../capability/tests/fixtures/resolved-capability-snapshot-v1.json"
        ))?;
        let mismatch = envelope(
            5,
            &fixture.run,
            RunEventKind::CapabilityResolved {
                execution,
                attempt,
                requirement: CapabilityRequirement::new(OperationId::new("tool.publish")?)
                    .provider_profile(ProviderProfileRef::new("publisher-prod")?),
                snapshot: snapshot_document.body().clone(),
            },
        )?;
        assert!(projection.apply(&mismatch).is_err());
        assert_eq!(projection.sequence(), RunSequence::new(4));
        Ok(())
    }

    #[test]
    fn idempotent_retries_cannot_rotate_stable_keys_or_resolved_snapshots() -> TestResult {
        let fixture = fixture("retry-stable-dispatch")?;
        let execution = NodeExecutionId::new("execution-task")?;
        let first_attempt = AttemptId::new("attempt-1")?;
        let second_attempt = AttemptId::new("attempt-2")?;
        let first_invocation = InvocationId::new("invocation-1")?;
        let second_invocation = InvocationId::new("invocation-2")?;
        let stable_key = IdempotencyKey::new("stable-retry-key")?;
        let rotated_key = IdempotencyKey::new("rotated-retry-key")?;
        let snapshot = resolved_snapshot_at(7)?;
        let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
            .provider_profile(ProviderProfileRef::new("publisher-prod")?);
        let timer = TimerId::new("retry-timer")?;
        let projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            eligible(3, &fixture, "task", &execution, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::NodeScheduled {
                    node: NodeId::new("task")?,
                    execution: execution.clone(),
                    attempt: first_attempt.clone(),
                    invocation: first_invocation.clone(),
                    idempotency_key: Some(stable_key.clone()),
                    request: invocation_request(&first_invocation, Some(stable_key.clone()))?,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::CapabilityResolved {
                    execution: execution.clone(),
                    attempt: first_attempt.clone(),
                    requirement: requirement.clone(),
                    snapshot: snapshot.clone(),
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::SideEffectClassified {
                    attempt: first_attempt.clone(),
                    side_effect: SideEffectClass::IdempotentWrite,
                    idempotency: IdempotencyBehavior::ProviderProfileScoped,
                    idempotency_key: Some(stable_key.clone()),
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::LeaseGranted {
                    lease: LeaseId::new("lease-first")?,
                    execution: execution.clone(),
                    attempt: first_attempt.clone(),
                    worker: WorkerId::new("worker-first")?,
                    expires_at: TimestampMillis::new(10_000),
                },
            )?,
            envelope(
                8,
                &fixture.run,
                RunEventKind::NodeTerminal {
                    execution: execution.clone(),
                    attempt: first_attempt.clone(),
                    report_sequence: 1,
                    outcome: NodeOutcome::Failed,
                    error_class: Some(ErrorClass::Provider),
                    detail: Some(BoundedDetail::new("provider failure")?),
                },
            )?,
            envelope(
                9,
                &fixture.run,
                RunEventKind::NodeRetryScheduled {
                    execution: execution.clone(),
                    previous_attempt: first_attempt,
                    next_attempt: second_attempt.clone(),
                    attempt_number: 2,
                    timer: timer.clone(),
                    fire_at: TimestampMillis::new(900),
                    error_class: ErrorClass::Provider,
                    reason: Reason::new("retry idempotent provider failure")?,
                },
            )?,
            envelope(
                10,
                &fixture.run,
                RunEventKind::TimerFired {
                    timer,
                    observed_at: TimestampMillis::new(900),
                },
            )?,
        ])?;

        let mut mutated_request_projection = projection.clone();
        let mutated_request = InvocationRequest::new(
            second_invocation.clone(),
            CapabilityId::new("publisher-primary")?,
            OperationId::new("tool.publish")?,
            Some(ProviderProfileRef::new("publisher-prod")?),
            Some(stable_key.clone()),
            Vec::new(),
            std::collections::BTreeMap::from([(
                milkdrift_capability::ExtensionKey::new("org.milkdrift/retry-mutation")?,
                BoundedJson::new(serde_json::json!({"changed": true}))?,
            )]),
        )?;
        let mutated_request = envelope(
            11,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("task")?,
                execution: execution.clone(),
                attempt: second_attempt.clone(),
                invocation: second_invocation.clone(),
                idempotency_key: Some(stable_key.clone()),
                request: mutated_request,
            },
        )?;
        assert!(mutated_request_projection.apply(&mutated_request).is_err());
        assert_eq!(mutated_request_projection.sequence(), RunSequence::new(10));

        let mut rotated_snapshot_projection = projection.clone();
        rotated_snapshot_projection.apply(&envelope(
            11,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("task")?,
                execution: execution.clone(),
                attempt: second_attempt.clone(),
                invocation: second_invocation.clone(),
                idempotency_key: Some(stable_key.clone()),
                request: invocation_request(&second_invocation, Some(stable_key.clone()))?,
            },
        )?)?;
        let rotated_snapshot = envelope(
            12,
            &fixture.run,
            RunEventKind::CapabilityResolved {
                execution: execution.clone(),
                attempt: second_attempt.clone(),
                requirement: requirement.clone(),
                snapshot: resolved_snapshot_at(8)?,
            },
        )?;
        assert!(
            rotated_snapshot_projection
                .apply(&rotated_snapshot)
                .is_err()
        );

        let mut rotated_key_projection = projection;
        rotated_key_projection.apply(&envelope(
            11,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("task")?,
                execution: execution.clone(),
                attempt: second_attempt.clone(),
                invocation: second_invocation.clone(),
                idempotency_key: Some(rotated_key.clone()),
                request: invocation_request(&second_invocation, Some(rotated_key.clone()))?,
            },
        )?)?;
        rotated_key_projection.apply(&envelope(
            12,
            &fixture.run,
            RunEventKind::CapabilityResolved {
                execution,
                attempt: second_attempt.clone(),
                requirement,
                snapshot,
            },
        )?)?;
        let rotated_classification = envelope(
            13,
            &fixture.run,
            RunEventKind::SideEffectClassified {
                attempt: second_attempt,
                side_effect: SideEffectClass::IdempotentWrite,
                idempotency: IdempotencyBehavior::ProviderProfileScoped,
                idempotency_key: Some(rotated_key),
            },
        )?;
        assert!(
            rotated_key_projection
                .apply(&rotated_classification)
                .is_err()
        );
        assert_eq!(rotated_key_projection.sequence(), RunSequence::new(12));
        Ok(())
    }

    #[test]
    fn automatic_retries_require_safe_side_effects_and_the_exact_failure_class() -> TestResult {
        let safe_fixture = fixture("retry-error-class")?;
        let safe_execution = NodeExecutionId::new("execution-safe")?;
        let safe_attempt = AttemptId::new("attempt-safe")?;
        let safe_invocation = InvocationId::new("invocation-safe")?;
        let stable_key = IdempotencyKey::new("stable-safe-key")?;
        let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
            .provider_profile(ProviderProfileRef::new("publisher-prod")?);
        let mut safe_projection = RunProjection::replay(&[
            created(&safe_fixture, 1)?,
            envelope(2, &safe_fixture.run, RunEventKind::RunStarted)?,
            eligible(
                3,
                &safe_fixture,
                "safe",
                &safe_execution,
                safe_fixture.root.reference(),
            )?,
            envelope(
                4,
                &safe_fixture.run,
                RunEventKind::NodeScheduled {
                    node: NodeId::new("safe")?,
                    execution: safe_execution.clone(),
                    attempt: safe_attempt.clone(),
                    invocation: safe_invocation.clone(),
                    idempotency_key: Some(stable_key.clone()),
                    request: invocation_request(&safe_invocation, Some(stable_key.clone()))?,
                },
            )?,
            envelope(
                5,
                &safe_fixture.run,
                RunEventKind::CapabilityResolved {
                    execution: safe_execution.clone(),
                    attempt: safe_attempt.clone(),
                    requirement: requirement.clone(),
                    snapshot: resolved_snapshot_at(7)?,
                },
            )?,
            envelope(
                6,
                &safe_fixture.run,
                RunEventKind::SideEffectClassified {
                    attempt: safe_attempt.clone(),
                    side_effect: SideEffectClass::IdempotentWrite,
                    idempotency: IdempotencyBehavior::ProviderProfileScoped,
                    idempotency_key: Some(stable_key),
                },
            )?,
            envelope(
                7,
                &safe_fixture.run,
                RunEventKind::LeaseGranted {
                    lease: LeaseId::new("lease-safe")?,
                    execution: safe_execution.clone(),
                    attempt: safe_attempt.clone(),
                    worker: WorkerId::new("worker-safe")?,
                    expires_at: TimestampMillis::new(10_000),
                },
            )?,
            envelope(
                8,
                &safe_fixture.run,
                RunEventKind::NodeTerminal {
                    execution: safe_execution.clone(),
                    attempt: safe_attempt.clone(),
                    report_sequence: 1,
                    outcome: NodeOutcome::Failed,
                    error_class: Some(ErrorClass::Provider),
                    detail: None,
                },
            )?,
        ])?;
        let substituted_class = envelope(
            9,
            &safe_fixture.run,
            RunEventKind::NodeRetryScheduled {
                execution: safe_execution,
                previous_attempt: safe_attempt,
                next_attempt: AttemptId::new("attempt-safe-next")?,
                attempt_number: 2,
                timer: TimerId::new("timer-safe-next")?,
                fire_at: TimestampMillis::new(900),
                error_class: ErrorClass::Transport,
                reason: Reason::new("must not substitute the durable failure class")?,
            },
        )?;
        assert!(safe_projection.apply(&substituted_class).is_err());
        assert_eq!(safe_projection.sequence(), RunSequence::new(8));

        let unsafe_fixture = fixture("retry-unsafe-write")?;
        let unsafe_execution = NodeExecutionId::new("execution-unsafe")?;
        let unsafe_attempt = AttemptId::new("attempt-unsafe")?;
        let unsafe_invocation = InvocationId::new("invocation-unsafe")?;
        let unsafe_snapshot = resolved_snapshot_with_side_effect(
            8,
            SideEffectClass::NonIdempotentWrite,
            IdempotencyBehavior::Unsupported,
        )?;
        let mut unsafe_projection = RunProjection::replay(&[
            created(&unsafe_fixture, 1)?,
            envelope(2, &unsafe_fixture.run, RunEventKind::RunStarted)?,
            eligible(
                3,
                &unsafe_fixture,
                "unsafe",
                &unsafe_execution,
                unsafe_fixture.root.reference(),
            )?,
            envelope(
                4,
                &unsafe_fixture.run,
                RunEventKind::NodeScheduled {
                    node: NodeId::new("unsafe")?,
                    execution: unsafe_execution.clone(),
                    attempt: unsafe_attempt.clone(),
                    invocation: unsafe_invocation.clone(),
                    idempotency_key: None,
                    request: invocation_request(&unsafe_invocation, None)?,
                },
            )?,
            envelope(
                5,
                &unsafe_fixture.run,
                RunEventKind::CapabilityResolved {
                    execution: unsafe_execution.clone(),
                    attempt: unsafe_attempt.clone(),
                    requirement,
                    snapshot: unsafe_snapshot,
                },
            )?,
            envelope(
                6,
                &unsafe_fixture.run,
                RunEventKind::SideEffectClassified {
                    attempt: unsafe_attempt.clone(),
                    side_effect: SideEffectClass::NonIdempotentWrite,
                    idempotency: IdempotencyBehavior::Unsupported,
                    idempotency_key: None,
                },
            )?,
            envelope(
                7,
                &unsafe_fixture.run,
                RunEventKind::LeaseGranted {
                    lease: LeaseId::new("lease-unsafe")?,
                    execution: unsafe_execution.clone(),
                    attempt: unsafe_attempt.clone(),
                    worker: WorkerId::new("worker-unsafe")?,
                    expires_at: TimestampMillis::new(10_000),
                },
            )?,
            envelope(
                8,
                &unsafe_fixture.run,
                RunEventKind::NodeTerminal {
                    execution: unsafe_execution.clone(),
                    attempt: unsafe_attempt.clone(),
                    report_sequence: 1,
                    outcome: NodeOutcome::Failed,
                    error_class: Some(ErrorClass::Provider),
                    detail: None,
                },
            )?,
        ])?;
        let unsafe_retry = envelope(
            9,
            &unsafe_fixture.run,
            RunEventKind::NodeRetryScheduled {
                execution: unsafe_execution,
                previous_attempt: unsafe_attempt,
                next_attempt: AttemptId::new("attempt-unsafe-next")?,
                attempt_number: 2,
                timer: TimerId::new("timer-unsafe-next")?,
                fire_at: TimestampMillis::new(900),
                error_class: ErrorClass::Provider,
                reason: Reason::new("unsafe writes require recovery authority")?,
            },
        )?;
        assert!(unsafe_projection.apply(&unsafe_retry).is_err());
        assert_eq!(unsafe_projection.sequence(), RunSequence::new(8));
        Ok(())
    }

    #[test]
    fn cancellation_facts_close_attempt_free_wait_and_timer_ownership() -> TestResult {
        let fixture = fixture("wait-cancel")?;
        let execution = NodeExecutionId::new("execution-wait")?;
        let timer = TimerId::new("timer-wait")?;
        let events = vec![
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            runtime_eligible(3, &fixture, "wait", &execution, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::TimerRegistered {
                    timer: timer.clone(),
                    execution: Some(execution.clone()),
                    fire_at: TimestampMillis::new(10_000),
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::WaitRegistered {
                    execution: execution.clone(),
                    condition: WaitCondition::Timer {
                        timer: timer.clone(),
                    },
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::RunCancellationRequested {
                    reason: Reason::new("stop")?,
                    evidence: Vec::new(),
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::WaitCancelled {
                    execution: execution.clone(),
                    reason: Reason::new("owner cancelled")?,
                },
            )?,
            envelope(
                8,
                &fixture.run,
                RunEventKind::TimerCancelled {
                    timer: timer.clone(),
                    reason: Reason::new("wait cancelled")?,
                },
            )?,
            envelope(
                9,
                &fixture.run,
                RunEventKind::NodeExecutionCancelledBeforeDispatch {
                    execution: execution.clone(),
                    reason: Reason::new("never dispatched")?,
                },
            )?,
            envelope(
                10,
                &fixture.run,
                RunEventKind::RunTerminal {
                    outcome: RunOutcome::Cancelled,
                    outputs: Vec::new(),
                    artifacts: Vec::new(),
                    reason: Some(Reason::new("cancelled cleanly")?),
                },
            )?,
        ];
        let projection = RunProjection::replay(&events)?;
        assert_eq!(
            projection.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Cancelled)
        );
        assert_eq!(
            projection.node_executions()[&execution].state(),
            &NodeExecutionState::CancelledBeforeDispatch
        );
        assert!(projection.waits()[&execution].cancellation().is_some());
        assert!(projection.timers()[&timer].cancellation().is_some());
        Ok(())
    }

    #[test]
    fn attempt_cancellation_targets_only_the_latest_active_attempt() -> TestResult {
        let fixture = fixture("attempt-cancel")?;
        let execution = NodeExecutionId::new("execution-task")?;
        let attempt = AttemptId::new("attempt-task")?;
        let events = vec![
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            eligible(3, &fixture, "task", &execution, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::NodeScheduled {
                    node: NodeId::new("task")?,
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    invocation: InvocationId::new("invocation-task")?,
                    idempotency_key: None,
                    request: invocation_request(&InvocationId::new("invocation-task")?, None)?,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::RunCancellationRequested {
                    reason: Reason::new("stop")?,
                    evidence: Vec::new(),
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::NodeExecutionCancellationRequested {
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    reason: Reason::new("cancel active attempt")?,
                },
            )?,
        ];
        let mut projection = RunProjection::replay(&events)?;
        assert_eq!(
            projection.node_executions()[&execution]
                .cancellation()
                .and_then(NodeExecutionCancellationProjection::attempt),
            Some(&attempt)
        );
        assert!(
            projection
                .apply(&envelope(
                    7,
                    &fixture.run,
                    RunEventKind::NodeExecutionCancellationRequested {
                        execution,
                        attempt,
                        reason: Reason::new("duplicate")?,
                    },
                )?)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn executor_terminal_reports_must_be_contiguous_and_stop_at_terminal() -> TestResult {
        let fixture = fixture("report-sequence")?;
        let execution = NodeExecutionId::new("execution-task")?;
        let attempt = AttemptId::new("attempt-task")?;
        let dispatch_facts = [
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            eligible(3, &fixture, "task", &execution, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::NodeScheduled {
                    node: NodeId::new("task")?,
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    invocation: InvocationId::new("invocation-task")?,
                    idempotency_key: None,
                    request: invocation_request(&InvocationId::new("invocation-task")?, None)?,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::CapabilityResolved {
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    requirement: CapabilityRequirement::new(OperationId::new("tool.publish")?)
                        .provider_profile(ProviderProfileRef::new("publisher-prod")?),
                    snapshot: resolved_snapshot_with_side_effect(
                        7,
                        SideEffectClass::None,
                        IdempotencyBehavior::Unsupported,
                    )?,
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::SideEffectClassified {
                    attempt: attempt.clone(),
                    side_effect: SideEffectClass::None,
                    idempotency: IdempotencyBehavior::Unsupported,
                    idempotency_key: None,
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::LeaseGranted {
                    lease: LeaseId::new("lease-task")?,
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    worker: WorkerId::new("worker-task")?,
                    expires_at: TimestampMillis::new(10_000),
                },
            )?,
        ];
        let mut projection = RunProjection::replay(&dispatch_facts[..4])?;
        let bare_terminal = envelope(
            5,
            &fixture.run,
            RunEventKind::NodeTerminal {
                execution: execution.clone(),
                attempt: attempt.clone(),
                report_sequence: 1,
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?;
        assert!(projection.apply(&bare_terminal).is_err());
        assert_eq!(projection.sequence(), RunSequence::new(4));
        for fact in &dispatch_facts[4..] {
            projection.apply(fact)?;
        }
        assert!(
            projection
                .apply(&envelope(
                    8,
                    &fixture.run,
                    RunEventKind::NodeTerminal {
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        report_sequence: 2,
                        outcome: NodeOutcome::Succeeded,
                        error_class: None,
                        detail: None,
                    },
                )?)
                .is_err()
        );
        projection.apply(&envelope(
            8,
            &fixture.run,
            RunEventKind::NodeTerminal {
                execution: execution.clone(),
                attempt: attempt.clone(),
                report_sequence: 1,
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?)?;
        assert_eq!(
            projection.attempts()[&attempt].last_report_sequence(),
            Some(1)
        );
        assert!(
            projection
                .apply(&envelope(
                    9,
                    &fixture.run,
                    RunEventKind::NodeTerminal {
                        execution,
                        attempt,
                        report_sequence: 2,
                        outcome: NodeOutcome::Succeeded,
                        error_class: None,
                        detail: None,
                    },
                )?)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn repeat_continuation_decisions_are_bounded_and_preserve_authority_history() -> TestResult {
        let fixture = fixture("repeat-approval")?;
        let repeat = NodeExecutionId::new("execution-repeat")?;
        let first = IterationId::new("iteration-1")?;
        let second = IterationId::new("iteration-2")?;
        let third = IterationId::new("iteration-3")?;
        let iteration_scope = |number: u32, iteration: &IterationId| {
            WorkspaceScope::iteration(
                ScopeId::new(format!("iteration-scope-{number}"))?,
                &fixture.root,
                iteration.clone(),
            )
        };
        let events = vec![
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::RepeatIterationCreated {
                    repeat_execution: repeat.clone(),
                    iteration: first.clone(),
                    iteration_number: 1,
                    scope: iteration_scope(1, &first)?,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::RepeatConditionRecorded {
                    iteration: first.clone(),
                    result: true,
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::RepeatContinuationRequested {
                    repeat_execution: repeat.clone(),
                    frontier_iteration: first.clone(),
                    initial_iteration_limit: 1,
                    effective_iteration_limit: 1,
                    cause: RepeatContinuationCause::IterationLimit,
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::RepeatContinuationDecided {
                    repeat_execution: repeat.clone(),
                    decision: RepeatDecisionId::new("decision-approve")?,
                    actor: ActorRef::new("operator")?,
                    outcome: RepeatContinuationDecision::Approved,
                    approved_additional_iterations: Some(2),
                    reason: Reason::new("allow two more")?,
                    evidence: Vec::new(),
                },
            )?,
            envelope(
                8,
                &fixture.run,
                RunEventKind::RepeatIterationCreated {
                    repeat_execution: repeat.clone(),
                    iteration: second.clone(),
                    iteration_number: 2,
                    scope: iteration_scope(2, &second)?,
                },
            )?,
            envelope(
                9,
                &fixture.run,
                RunEventKind::RepeatConditionRecorded {
                    iteration: second,
                    result: true,
                },
            )?,
            envelope(
                10,
                &fixture.run,
                RunEventKind::RepeatIterationCreated {
                    repeat_execution: repeat.clone(),
                    iteration: third.clone(),
                    iteration_number: 3,
                    scope: iteration_scope(3, &third)?,
                },
            )?,
            envelope(
                11,
                &fixture.run,
                RunEventKind::RepeatConditionRecorded {
                    iteration: third.clone(),
                    result: true,
                },
            )?,
            envelope(
                12,
                &fixture.run,
                RunEventKind::RepeatContinuationRequested {
                    repeat_execution: repeat.clone(),
                    frontier_iteration: third.clone(),
                    initial_iteration_limit: 1,
                    effective_iteration_limit: 3,
                    cause: RepeatContinuationCause::IterationLimit,
                },
            )?,
            envelope(
                13,
                &fixture.run,
                RunEventKind::RepeatContinuationDecided {
                    repeat_execution: repeat.clone(),
                    decision: RepeatDecisionId::new("decision-reject")?,
                    actor: ActorRef::new("operator")?,
                    outcome: RepeatContinuationDecision::Rejected,
                    approved_additional_iterations: None,
                    reason: Reason::new("stop at boundary")?,
                    evidence: Vec::new(),
                },
            )?,
            envelope(
                14,
                &fixture.run,
                RunEventKind::RepeatTerminated {
                    repeat_execution: repeat.clone(),
                    termination: RepeatTerminationReason::MaximumIterations,
                    last_iteration: Some(third),
                },
            )?,
        ];
        let pending = RunProjection::replay(&events[..6])?;
        let pending_continuation = &pending.repeat_continuations()[&repeat];
        assert!(pending_continuation.is_pending_approval());
        assert_eq!(
            pending_continuation
                .pending_request()
                .map(RepeatContinuationRequestProjection::frontier_iteration),
            Some(&first)
        );
        let projection = RunProjection::replay(&events)?;
        let continuation = &projection.repeat_continuations()[&repeat];
        assert_eq!(continuation.initial_iteration_limit(), 1);
        assert_eq!(continuation.effective_iteration_limit(), 3);
        assert!(continuation.is_rejected());
        assert!(!continuation.is_pending_approval());
        assert_eq!(continuation.requests().len(), 2);
        assert_eq!(continuation.decisions().len(), 2);
        Ok(())
    }

    #[test]
    fn repeat_continuation_requires_an_exact_pending_request() -> TestResult {
        let fixture = fixture("repeat-request")?;
        let repeat = NodeExecutionId::new("execution-repeat")?;
        let frontier = IterationId::new("iteration-frontier")?;
        let scope = WorkspaceScope::iteration(
            ScopeId::new("iteration-frontier-scope")?,
            &fixture.root,
            frontier.clone(),
        )?;
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::RepeatIterationCreated {
                    repeat_execution: repeat.clone(),
                    iteration: frontier.clone(),
                    iteration_number: 1,
                    scope,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::RepeatConditionRecorded {
                    iteration: frontier.clone(),
                    result: true,
                },
            )?,
        ])?;
        assert!(projection.repeat_continuations().is_empty());

        let decision_without_request = envelope(
            6,
            &fixture.run,
            RunEventKind::RepeatContinuationDecided {
                repeat_execution: repeat.clone(),
                decision: RepeatDecisionId::new("decision-without-request")?,
                actor: ActorRef::new("operator")?,
                outcome: RepeatContinuationDecision::Approved,
                approved_additional_iterations: Some(1),
                reason: Reason::new("cannot authorize an implicit boundary")?,
                evidence: Vec::new(),
            },
        )?;
        assert!(projection.apply(&decision_without_request).is_err());
        assert_eq!(projection.sequence(), RunSequence::new(5));

        let wrong_limit = envelope(
            6,
            &fixture.run,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: repeat.clone(),
                frontier_iteration: frontier.clone(),
                initial_iteration_limit: 2,
                effective_iteration_limit: 2,
                cause: RepeatContinuationCause::IterationLimit,
            },
        )?;
        assert!(projection.apply(&wrong_limit).is_err());

        let request = envelope(
            6,
            &fixture.run,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: repeat.clone(),
                frontier_iteration: frontier,
                initial_iteration_limit: 1,
                effective_iteration_limit: 1,
                cause: RepeatContinuationCause::IterationLimit,
            },
        )?;
        projection.apply(&request)?;
        let continuation = &projection.repeat_continuations()[&repeat];
        assert!(continuation.is_pending_approval());
        assert_eq!(continuation.requests().len(), 1);

        let duplicate = envelope(
            7,
            &fixture.run,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: repeat,
                frontier_iteration: IterationId::new("iteration-frontier")?,
                initial_iteration_limit: 1,
                effective_iteration_limit: 1,
                cause: RepeatContinuationCause::IterationLimit,
            },
        )?;
        assert!(projection.apply(&duplicate).is_err());
        assert_eq!(projection.sequence(), RunSequence::new(6));
        Ok(())
    }

    #[test]
    fn repeat_budget_rejection_preserves_its_currency_specific_cause() -> TestResult {
        let fixture = fixture("repeat-budget-request")?;
        let repeat = NodeExecutionId::new("execution-repeat")?;
        let frontier = IterationId::new("iteration-frontier")?;
        let scope = WorkspaceScope::iteration(
            ScopeId::new("iteration-frontier-scope")?,
            &fixture.root,
            frontier.clone(),
        )?;
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::RepeatIterationCreated {
                    repeat_execution: repeat.clone(),
                    iteration: frontier.clone(),
                    iteration_number: 1,
                    scope,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::RepeatConditionRecorded {
                    iteration: frontier.clone(),
                    result: true,
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::RepeatContinuationRequested {
                    repeat_execution: repeat.clone(),
                    frontier_iteration: frontier.clone(),
                    initial_iteration_limit: 10,
                    effective_iteration_limit: 10,
                    cause: RepeatContinuationCause::CostBudget {
                        maximum_micros: 100,
                        observed_micros: 125,
                        currency: CurrencyCode::new("EUR")?,
                    },
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::RepeatContinuationDecided {
                    repeat_execution: repeat.clone(),
                    decision: RepeatDecisionId::new("decision-reject-budget")?,
                    actor: ActorRef::new("operator")?,
                    outcome: RepeatContinuationDecision::Rejected,
                    approved_additional_iterations: None,
                    reason: Reason::new("budget remains binding")?,
                    evidence: Vec::new(),
                },
            )?,
        ])?;
        let request = projection.repeat_continuations()[&repeat]
            .requests()
            .last()
            .ok_or("missing request")?;
        assert_eq!(
            request.cause(),
            &RepeatContinuationCause::CostBudget {
                maximum_micros: 100,
                observed_micros: 125,
                currency: CurrencyCode::new("EUR")?,
            }
        );
        let wrong_termination = envelope(
            8,
            &fixture.run,
            RunEventKind::RepeatTerminated {
                repeat_execution: repeat.clone(),
                termination: RepeatTerminationReason::MaximumIterations,
                last_iteration: Some(frontier.clone()),
            },
        )?;
        assert!(projection.apply(&wrong_termination).is_err());
        projection.apply(&envelope(
            8,
            &fixture.run,
            RunEventKind::RepeatTerminated {
                repeat_execution: repeat,
                termination: RepeatTerminationReason::BudgetExhausted,
                last_iteration: Some(frontier),
            },
        )?)?;
        Ok(())
    }

    #[test]
    fn repeat_budget_approval_has_a_frontier_local_override_cap() -> TestResult {
        let fixture = fixture("repeat-budget-override")?;
        let repeat = NodeExecutionId::new("execution-repeat")?;
        let first = IterationId::new("iteration-1")?;
        let second = IterationId::new("iteration-2")?;
        let third = IterationId::new("iteration-3")?;
        let iteration_scope = |number: u32, iteration: &IterationId| {
            WorkspaceScope::iteration(
                ScopeId::new(format!("iteration-scope-{number}"))?,
                &fixture.root,
                iteration.clone(),
            )
        };
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::RepeatIterationCreated {
                    repeat_execution: repeat.clone(),
                    iteration: first.clone(),
                    iteration_number: 1,
                    scope: iteration_scope(1, &first)?,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::RepeatConditionRecorded {
                    iteration: first,
                    result: true,
                },
            )?,
            envelope(
                6,
                &fixture.run,
                RunEventKind::RepeatIterationCreated {
                    repeat_execution: repeat.clone(),
                    iteration: second.clone(),
                    iteration_number: 2,
                    scope: iteration_scope(2, &second)?,
                },
            )?,
            envelope(
                7,
                &fixture.run,
                RunEventKind::RepeatConditionRecorded {
                    iteration: second.clone(),
                    result: true,
                },
            )?,
            envelope(
                8,
                &fixture.run,
                RunEventKind::RepeatContinuationRequested {
                    repeat_execution: repeat.clone(),
                    frontier_iteration: second,
                    initial_iteration_limit: 10,
                    effective_iteration_limit: 10,
                    cause: RepeatContinuationCause::DurationBudget {
                        maximum_ms: 100,
                        observed_ms: 125,
                    },
                },
            )?,
            envelope(
                9,
                &fixture.run,
                RunEventKind::RepeatContinuationDecided {
                    repeat_execution: repeat.clone(),
                    decision: RepeatDecisionId::new("decision-approve-budget")?,
                    actor: ActorRef::new("operator")?,
                    outcome: RepeatContinuationDecision::Approved,
                    approved_additional_iterations: Some(1),
                    reason: Reason::new("authorize exactly one post-budget iteration")?,
                    evidence: Vec::new(),
                },
            )?,
        ])?;
        let continuation = &projection.repeat_continuations()[&repeat];
        assert_eq!(continuation.effective_iteration_limit(), 11);
        assert_eq!(continuation.budget_override_iteration_limit(), Some(3));

        projection.apply(&envelope(
            10,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: third.clone(),
                iteration_number: 3,
                scope: iteration_scope(3, &third)?,
            },
        )?)?;
        projection.apply(&envelope(
            11,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: third.clone(),
                result: true,
            },
        )?)?;
        projection.apply(&envelope(
            12,
            &fixture.run,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: repeat.clone(),
                frontier_iteration: third,
                initial_iteration_limit: 10,
                effective_iteration_limit: 11,
                cause: RepeatContinuationCause::DurationBudget {
                    maximum_ms: 100,
                    observed_ms: 150,
                },
            },
        )?)?;
        let continuation = &projection.repeat_continuations()[&repeat];
        assert!(continuation.is_pending_approval());
        assert_eq!(continuation.budget_override_iteration_limit(), None);
        Ok(())
    }

    #[test]
    fn repeat_continuation_request_cycles_are_hard_capped() -> TestResult {
        let fixture = fixture("repeat-request-cap")?;
        let repeat = NodeExecutionId::new("execution-repeat")?;
        let mut events = vec![
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
        ];
        let mut sequence = 4_u64;
        for number in 1..=MAX_REPEAT_CONTINUATION_CYCLES as u32 {
            let iteration = IterationId::new(format!("iteration-{number}"))?;
            let scope = WorkspaceScope::iteration(
                ScopeId::new(format!("iteration-scope-{number}"))?,
                &fixture.root,
                iteration.clone(),
            )?;
            events.push(envelope(
                sequence,
                &fixture.run,
                RunEventKind::RepeatIterationCreated {
                    repeat_execution: repeat.clone(),
                    iteration: iteration.clone(),
                    iteration_number: number,
                    scope,
                },
            )?);
            sequence += 1;
            events.push(envelope(
                sequence,
                &fixture.run,
                RunEventKind::RepeatConditionRecorded {
                    iteration: iteration.clone(),
                    result: true,
                },
            )?);
            sequence += 1;
            events.push(envelope(
                sequence,
                &fixture.run,
                RunEventKind::RepeatContinuationRequested {
                    repeat_execution: repeat.clone(),
                    frontier_iteration: iteration,
                    initial_iteration_limit: 1,
                    effective_iteration_limit: number,
                    cause: RepeatContinuationCause::IterationLimit,
                },
            )?);
            sequence += 1;
            events.push(envelope(
                sequence,
                &fixture.run,
                RunEventKind::RepeatContinuationDecided {
                    repeat_execution: repeat.clone(),
                    decision: RepeatDecisionId::new(format!("decision-{number}"))?,
                    actor: ActorRef::new("operator")?,
                    outcome: RepeatContinuationDecision::Approved,
                    approved_additional_iterations: Some(1),
                    reason: Reason::new("bounded continuation")?,
                    evidence: Vec::new(),
                },
            )?);
            sequence += 1;
        }
        let mut projection = RunProjection::replay(&events)?;
        let continuation = &projection.repeat_continuations()[&repeat];
        assert_eq!(
            continuation.requests().len(),
            MAX_REPEAT_CONTINUATION_CYCLES
        );
        assert_eq!(
            continuation.decisions().len(),
            MAX_REPEAT_CONTINUATION_CYCLES
        );
        assert_eq!(continuation.effective_iteration_limit(), 65);

        let frontier = IterationId::new("iteration-over-cap")?;
        projection.apply(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: frontier.clone(),
                iteration_number: 65,
                scope: WorkspaceScope::iteration(
                    ScopeId::new("iteration-scope-over-cap")?,
                    &fixture.root,
                    frontier.clone(),
                )?,
            },
        )?)?;
        sequence += 1;
        projection.apply(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: frontier.clone(),
                result: true,
            },
        )?)?;
        sequence += 1;
        let over_cap = envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: repeat,
                frontier_iteration: frontier,
                initial_iteration_limit: 1,
                effective_iteration_limit: 65,
                cause: RepeatContinuationCause::IterationLimit,
            },
        )?;
        assert!(projection.apply(&over_cap).is_err());
        Ok(())
    }

    #[test]
    fn repeat_body_subworkflow_may_be_nested_under_the_active_iteration_scope() -> TestResult {
        let fixture = fixture("repeat-subworkflow")?;
        let repeat = NodeExecutionId::new("execution-repeat")?;
        let iteration = IterationId::new("iteration-1")?;
        let iteration_scope = WorkspaceScope::iteration(
            ScopeId::new("iteration-scope")?,
            &fixture.root,
            iteration.clone(),
        )?;
        let subworkflow = SubworkflowId::new("repeat-body")?;
        let child_scope = WorkspaceScope::subworkflow(
            ScopeId::new("repeat-body-scope")?,
            &iteration_scope,
            subworkflow.clone(),
        )?;
        let projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
            envelope(
                4,
                &fixture.run,
                RunEventKind::RepeatIterationCreated {
                    repeat_execution: repeat.clone(),
                    iteration,
                    iteration_number: 1,
                    scope: iteration_scope,
                },
            )?,
            envelope(
                5,
                &fixture.run,
                RunEventKind::SubworkflowCreated {
                    subworkflow: subworkflow.clone(),
                    parent_execution: repeat,
                    child_run: RunId::new("run-repeat-child")?,
                    child_revision: revision('b')?,
                    scope: child_scope,
                    ownership: SubworkflowOwnership::Attached,
                    inputs: Vec::new(),
                },
            )?,
        ])?;
        assert!(projection.subworkflows().contains_key(&subworkflow));
        Ok(())
    }

    #[test]
    fn subworkflow_creation_materializes_atomic_child_scope_inputs() -> TestResult {
        let fixture = fixture("subworkflow-inputs")?;
        let parent = NodeExecutionId::new("execution-subworkflow")?;
        let subworkflow = SubworkflowId::new("subworkflow-child")?;
        let child_scope = WorkspaceScope::subworkflow(
            ScopeId::new("subworkflow-child-scope")?,
            &fixture.root,
            subworkflow.clone(),
        )?;
        let child_input = WorkspaceValueReference::new(
            child_scope.reference().clone(),
            milkdrift_workspace::ValueKey::new("request")?,
            milkdrift_workspace::ValueVersion::FIRST,
        );
        let mut projection = RunProjection::replay(&[
            created(&fixture, 1)?,
            envelope(2, &fixture.run, RunEventKind::RunStarted)?,
            runtime_eligible(
                3,
                &fixture,
                "subworkflow",
                &parent,
                fixture.root.reference(),
            )?,
        ])?;

        let unknown_ancestor = WorkspaceValueReference::new(
            fixture.root.reference().clone(),
            milkdrift_workspace::ValueKey::new("unknown")?,
            milkdrift_workspace::ValueVersion::FIRST,
        );
        let malformed = envelope(
            4,
            &fixture.run,
            RunEventKind::SubworkflowCreated {
                subworkflow: subworkflow.clone(),
                parent_execution: parent.clone(),
                child_run: RunId::new("run-subworkflow-child")?,
                child_revision: revision('b')?,
                scope: child_scope.clone(),
                ownership: SubworkflowOwnership::Attached,
                inputs: vec![unknown_ancestor],
            },
        )?;
        assert!(projection.apply(&malformed).is_err());
        assert_eq!(projection.sequence(), RunSequence::new(3));

        projection.apply(&envelope(
            4,
            &fixture.run,
            RunEventKind::SubworkflowCreated {
                subworkflow: subworkflow.clone(),
                parent_execution: parent,
                child_run: RunId::new("run-subworkflow-child")?,
                child_revision: revision('b')?,
                scope: child_scope,
                ownership: SubworkflowOwnership::Attached,
                inputs: vec![child_input.clone()],
            },
        )?)?;
        assert!(projection.workspace_values.contains(&child_input));
        assert_eq!(
            projection.subworkflows()[&subworkflow].inputs(),
            &[child_input]
        );
        Ok(())
    }
}
