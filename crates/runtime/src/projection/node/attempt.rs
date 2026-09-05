//! Immutable attempt facts, observations, terminal evidence, and obligations.

use milkdrift_authority::AuthorityDecisionSnapshot;
use milkdrift_capability::{
    CancellationAcknowledgement, ErrorClass, IdempotencyBehavior, IdempotencyKey, InvocationId,
    InvocationRequest, InvocationTerminal, ResolvedCapabilitySnapshot, SideEffectClass,
};
use milkdrift_persistence::{
    AttemptId, AttemptUsage, BoundedDetail, EvidenceReference, LeaseId, NodeExecutionId,
    NodeOutcome, Reason, ReconciliationDecisionId, RunSequence, WorkerId,
};
use milkdrift_workspace::{ArtifactReference, WorkspaceValueReference};
use serde::{Deserialize, Serialize};

use super::super::reconciliation::{RecoveryDecision, RecoveryObservation};

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
    pub(in crate::projection) requirement: milkdrift_capability::CapabilityRequirement,
    pub(in crate::projection) snapshot: ResolvedCapabilitySnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::projection) authorization: Option<AuthorityDecisionSnapshot>,
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

    /// Exact canonical decision that allowed this generation to be selected.
    #[must_use]
    pub const fn authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.authorization.as_ref()
    }
}

/// Side-effect and external-idempotency facts frozen before dispatch.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SideEffectClassification {
    pub(in crate::projection) side_effect: SideEffectClass,
    pub(in crate::projection) idempotency: IdempotencyBehavior,
    pub(in crate::projection) idempotency_key: Option<IdempotencyKey>,
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
    pub(in crate::projection) report_sequence: u64,
    pub(in crate::projection) detail: BoundedDetail,
    pub(in crate::projection) completed_units: Option<u64>,
    pub(in crate::projection) total_units: Option<u64>,
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
    pub(in crate::projection) report_sequence: Option<u64>,
    pub(in crate::projection) value: WorkspaceValueReference,
    pub(in crate::projection) artifact: Option<ArtifactReference>,
    pub(in crate::projection) sequence: RunSequence,
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
    pub(in crate::projection) report_sequence: u64,
    pub(in crate::projection) outcome: NodeOutcome,
    pub(in crate::projection) error_class: Option<ErrorClass>,
    pub(in crate::projection) detail: Option<BoundedDetail>,
    pub(in crate::projection) sequence: RunSequence,
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
    pub(in crate::projection) report_sequence: u64,
    pub(in crate::projection) terminal: InvocationTerminal,
    pub(in crate::projection) worker: WorkerId,
    pub(in crate::projection) sequence: RunSequence,
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
    pub(in crate::projection) report_sequence: u64,
    pub(in crate::projection) side_effect: SideEffectClass,
    pub(in crate::projection) reason: Reason,
    pub(in crate::projection) evidence: Vec<EvidenceReference>,
    pub(in crate::projection) uncertain_sequence: RunSequence,
    pub(in crate::projection) retained: Option<RetainedExternalOutcome>,
    pub(in crate::projection) decisions: Vec<RecoveryDecision>,
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
    pub(in crate::projection) decision: ReconciliationDecisionId,
    pub(in crate::projection) reason: Reason,
    pub(in crate::projection) sequence: RunSequence,
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

/// Active/latest operational read model for one immutable attempt.
///
/// Settled high-frequency history is queried from the journal.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct NodeAttemptProjection {
    pub(in crate::projection) attempt: AttemptId,
    pub(in crate::projection) execution: NodeExecutionId,
    pub(in crate::projection) attempt_number: u32,
    pub(in crate::projection) invocation: Option<InvocationId>,
    pub(in crate::projection) idempotency_key: Option<IdempotencyKey>,
    pub(in crate::projection) request: Option<InvocationRequest>,
    pub(in crate::projection) scheduled_sequence: Option<RunSequence>,
    pub(in crate::projection) state: AttemptState,
    pub(in crate::projection) capability: Option<CapabilityResolution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::projection) resolution_authorization: Option<AuthorityDecisionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::projection) entry_authorization: Option<AuthorityDecisionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::projection) adapter_entry_authorization: Option<AuthorityDecisionSnapshot>,
    pub(in crate::projection) side_effect: Option<SideEffectClassification>,
    pub(in crate::projection) leases: Vec<LeaseId>,
    /// The latest worker that durably crossed `NodeStarted` for this attempt.
    /// Pre-start lease churn is deliberately excluded from this bounded summary.
    pub(in crate::projection) lease_workers: std::collections::BTreeSet<WorkerId>,
    pub(in crate::projection) progress: Vec<ProgressObservation>,
    pub(in crate::projection) last_report_sequence: Option<u64>,
    pub(in crate::projection) usage: Option<AttemptUsage>,
    pub(in crate::projection) cancellation_acknowledgements: Vec<CancellationAcknowledgement>,
    pub(in crate::projection) outputs: Vec<PublishedNodeOutput>,
    pub(in crate::projection) terminal: Option<AttemptTerminal>,
    pub(in crate::projection) late_terminal_evidence: Option<LateTerminalEvidence>,
    pub(in crate::projection) obligation: Option<ExternalOutcomeObligation>,
    pub(in crate::projection) recovery: Vec<RecoveryObservation>,
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
    /// scheduled. The owning execution carries the immutable workflow revision
    /// governing this attempt.
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

    /// Candidate-set authority decision recorded before capability resolution was frozen.
    #[must_use]
    pub const fn resolution_authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.resolution_authorization.as_ref()
    }

    /// Fresh exact-candidate decision recorded when the leased effect was claimed.
    #[must_use]
    pub const fn entry_authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.entry_authorization.as_ref()
    }

    /// Final decision recorded directly before adapter code was entered or denied.
    #[must_use]
    pub const fn adapter_entry_authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.adapter_entry_authorization.as_ref()
    }

    /// Frozen side-effect and idempotency classification.
    #[must_use]
    pub const fn side_effect(&self) -> Option<&SideEffectClassification> {
        self.side_effect.as_ref()
    }

    /// Lease identities still required by active recovery transitions.
    #[must_use]
    pub fn leases(&self) -> &[LeaseId] {
        &self.leases
    }

    /// Latest worker that durably crossed the attempt's start boundary.
    #[must_use]
    pub const fn lease_workers(&self) -> &std::collections::BTreeSet<WorkerId> {
        &self.lease_workers
    }

    /// Latest monotonic progress report, when present.
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

    /// Latest executor cancellation acknowledgement, when present.
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

    /// Latest durable recovery classification for this attempt.
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

    pub(in crate::projection) fn expects_report_sequence(&self, report_sequence: u64) -> bool {
        self.last_report_sequence
            .map_or(report_sequence == 1, |last| {
                last.checked_add(1) == Some(report_sequence)
            })
    }
}
