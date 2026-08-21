use serde::{Deserialize, Serialize};

use milkdrift_blueprint::NodeId;
use milkdrift_capability::{
    CancellationAcknowledgement, ErrorClass, IdempotencyBehavior, IdempotencyKey, InvocationId,
    InvocationRequest, InvocationTerminal, ResolvedCapabilitySnapshot, SideEffectClass,
};
use milkdrift_persistence::{
    AttemptId, AttemptUsage, BoundedDetail, EvidenceReference, LeaseId, NodeExecutionId,
    NodeExecutionMode, NodeOutcome, Reason, ReconciliationDecisionId, ReconciliationPlanId,
    RecoveryClassification, RunSequence, TimerId, TimestampMillis, WorkerId,
};
use milkdrift_workspace::{ArtifactReference, ScopeReference, WorkspaceValueReference};

use super::reconciliation::{RecoveryDecision, RecoveryObservation};

/// Dynamic state of one logical node execution.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct NodeExecutionCancellationProjection {
    pub(super) attempt: Option<AttemptId>,
    pub(super) reason: Reason,
    pub(super) sequence: RunSequence,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct DeterministicNodeTerminalProjection {
    pub(super) outcome: NodeOutcome,
    pub(super) error_class: Option<ErrorClass>,
    pub(super) detail: Option<BoundedDetail>,
    pub(super) sequence: RunSequence,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct NodeExecutionProjection {
    pub(super) execution: NodeExecutionId,
    pub(super) node: NodeId,
    pub(super) scope: ScopeReference,
    pub(super) mode: NodeExecutionMode,
    pub(super) created_sequence: RunSequence,
    pub(super) created_at: TimestampMillis,
    pub(super) attempts: Vec<AttemptId>,
    pub(super) state: NodeExecutionState,
    pub(super) cancellation: Option<NodeExecutionCancellationProjection>,
    pub(super) deterministic_terminal: Option<DeterministicNodeTerminalProjection>,
    pub(super) outputs: Vec<PublishedNodeOutput>,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
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
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CapabilityResolution {
    pub(super) requirement: milkdrift_capability::CapabilityRequirement,
    pub(super) snapshot: ResolvedCapabilitySnapshot,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SideEffectClassification {
    pub(super) side_effect: SideEffectClass,
    pub(super) idempotency: IdempotencyBehavior,
    pub(super) idempotency_key: Option<IdempotencyKey>,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ProgressObservation {
    pub(super) report_sequence: u64,
    pub(super) detail: BoundedDetail,
    pub(super) completed_units: Option<u64>,
    pub(super) total_units: Option<u64>,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PublishedNodeOutput {
    pub(super) report_sequence: Option<u64>,
    pub(super) value: WorkspaceValueReference,
    pub(super) artifact: Option<ArtifactReference>,
    pub(super) sequence: RunSequence,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct AttemptTerminal {
    pub(super) report_sequence: u64,
    pub(super) outcome: NodeOutcome,
    pub(super) error_class: Option<ErrorClass>,
    pub(super) detail: Option<BoundedDetail>,
    pub(super) sequence: RunSequence,
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

/// Known terminal evidence received after active worker ownership ended.
///
/// The observation remains separate from [`AttemptTerminal`] so replay never
/// rewrites an earlier uncertainty classification or a later retry result.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct LateTerminalEvidence {
    pub(super) report_sequence: u64,
    pub(super) terminal: InvocationTerminal,
    pub(super) worker: WorkerId,
    pub(super) sequence: RunSequence,
}

impl LateTerminalEvidence {
    /// Original executor-local report sequence.
    #[must_use]
    pub const fn report_sequence(&self) -> u64 {
        self.report_sequence
    }

    /// Provider-neutral terminal observation.
    #[must_use]
    pub const fn terminal(&self) -> &InvocationTerminal {
        &self.terminal
    }

    /// Worker that historically owned the attempt.
    #[must_use]
    pub const fn worker(&self) -> &WorkerId {
        &self.worker
    }

    /// Durable event sequence at which the evidence was recorded.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Current unresolved external-outcome obligation for an attempt.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ExternalOutcomeObligation {
    pub(super) report_sequence: u64,
    pub(super) side_effect: SideEffectClass,
    pub(super) reason: Reason,
    pub(super) evidence: Vec<EvidenceReference>,
    pub(super) uncertain_sequence: RunSequence,
    pub(super) retained: Option<RetainedExternalOutcome>,
    pub(super) decisions: Vec<RecoveryDecision>,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RetainedExternalOutcome {
    pub(super) decision: ReconciliationDecisionId,
    pub(super) reason: Reason,
    pub(super) sequence: RunSequence,
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
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct NodeAttemptProjection {
    pub(super) attempt: AttemptId,
    pub(super) execution: NodeExecutionId,
    pub(super) attempt_number: u32,
    pub(super) invocation: Option<InvocationId>,
    pub(super) idempotency_key: Option<IdempotencyKey>,
    pub(super) request: Option<InvocationRequest>,
    pub(super) scheduled_sequence: Option<RunSequence>,
    pub(super) state: AttemptState,
    pub(super) capability: Option<CapabilityResolution>,
    pub(super) side_effect: Option<SideEffectClassification>,
    pub(super) leases: Vec<LeaseId>,
    pub(super) progress: Vec<ProgressObservation>,
    pub(super) last_report_sequence: Option<u64>,
    pub(super) usage: Option<AttemptUsage>,
    pub(super) cancellation_acknowledgements: Vec<CancellationAcknowledgement>,
    pub(super) outputs: Vec<PublishedNodeOutput>,
    pub(super) terminal: Option<AttemptTerminal>,
    pub(super) late_terminal_evidence: Option<LateTerminalEvidence>,
    pub(super) obligation: Option<ExternalOutcomeObligation>,
    pub(super) recovery: Vec<RecoveryObservation>,
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
    /// scheduled. Combined with [`crate::projection::RunProjection::revision_at`], it identifies
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

    /// Terminal evidence received only after active lease ownership ended.
    #[must_use]
    pub const fn late_terminal_evidence(&self) -> Option<&LateTerminalEvidence> {
        self.late_terminal_evidence.as_ref()
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

    pub(super) fn expects_report_sequence(&self, report_sequence: u64) -> bool {
        self.last_report_sequence
            .map_or(report_sequence == 1, |last| {
                last.checked_add(1) == Some(report_sequence)
            })
    }
}

/// State of a durable lease.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct LeaseProjection {
    pub(super) lease: LeaseId,
    pub(super) execution: NodeExecutionId,
    pub(super) attempt: AttemptId,
    pub(super) worker: WorkerId,
    pub(super) expires_at: TimestampMillis,
    pub(super) state: LeaseState,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TimerCancellationProjection {
    pub(super) reason: Reason,
    pub(super) sequence: RunSequence,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TimerProjection {
    pub(super) timer: TimerId,
    pub(super) purpose: TimerPurpose,
    pub(super) fire_at: TimestampMillis,
    pub(super) state: TimerState,
    pub(super) cancellation: Option<TimerCancellationProjection>,
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RetryProjection {
    pub(super) execution: NodeExecutionId,
    pub(super) previous_attempt: AttemptId,
    pub(super) next_attempt: AttemptId,
    pub(super) attempt_number: u32,
    pub(super) timer: TimerId,
    pub(super) fire_at: TimestampMillis,
    pub(super) error_class: ErrorClass,
    pub(super) reason: Reason,
    pub(super) state: RetryState,
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
