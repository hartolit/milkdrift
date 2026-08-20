use std::collections::BTreeMap;

use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_persistence::{
    ActorRef, AttemptId, AuthorityDecision, EvidenceReference, LeaseId, NodeExecutionId,
    NodeExecutionMode, Reason, ReconciliationDecisionId, ReconciliationId, ReconciliationItem,
    ReconciliationPlanId, ReconciliationPolicy, RecoveryClassification, RunSequence, WorkerId,
};
use milkdrift_workspace::ScopeReference;

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
    pub(super) reconciliation: ReconciliationId,
    pub(super) from_revision: RevisionId,
    pub(super) to_revision: RevisionId,
    pub(super) policy: ReconciliationPolicy,
    pub(super) sequence: RunSequence,
    pub(super) plan: Option<ReconciliationPlanId>,
    pub(super) state: ReconciliationRequestState,
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
    pub(super) decision: ReconciliationDecisionId,
    pub(super) actor: ActorRef,
    pub(super) outcome: AuthorityDecision,
    pub(super) reason: Reason,
    pub(super) evidence: Vec<EvidenceReference>,
    pub(super) sequence: RunSequence,
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
    pub(super) reconciliation: ReconciliationId,
    pub(super) plan: ReconciliationPlanId,
    pub(super) from_revision: RevisionId,
    pub(super) to_revision: RevisionId,
    pub(super) based_on_sequence: RunSequence,
    pub(super) items: Vec<ReconciliationItem>,
    pub(super) decisions: Vec<ReconciliationDecision>,
    pub(super) applied_sequence: Option<RunSequence>,
    pub(super) stale_sequence: Option<RunSequence>,
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
    pub(super) requests: BTreeMap<ReconciliationId, ReconciliationRequestProjection>,
    pub(super) plans: BTreeMap<ReconciliationPlanId, ReconciliationPlanProjection>,
    pub(super) current_request: Option<ReconciliationId>,
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
    pub(super) lease: Option<LeaseId>,
    pub(super) classification: RecoveryClassification,
    pub(super) reason: Reason,
    pub(super) sequence: RunSequence,
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
    pub(super) decision: ReconciliationDecisionId,
    pub(super) actor: ActorRef,
    pub(super) outcome: AuthorityDecision,
    pub(super) reason: Reason,
    pub(super) evidence: Vec<EvidenceReference>,
    pub(super) sequence: RunSequence,
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
    pub(super) controller: WorkerId,
    pub(super) through_sequence: RunSequence,
    pub(super) started_sequence: RunSequence,
    pub(super) classifications: Vec<(AttemptId, RecoveryObservation)>,
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
    pub(super) plan: ReconciliationPlanId,
    pub(super) execution: NodeExecutionId,
    pub(super) attempt: AttemptId,
    pub(super) reason: Reason,
    pub(super) sequence: RunSequence,
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
    pub(super) plan: ReconciliationPlanId,
    pub(super) source_execution: NodeExecutionId,
    pub(super) source_attempt: Option<AttemptId>,
    pub(super) execution: NodeExecutionId,
    pub(super) node: NodeId,
    pub(super) scope: ScopeReference,
    pub(super) reason: Reason,
    pub(super) sequence: RunSequence,
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
    pub(super) source_attempt: AttemptId,
    pub(super) execution: NodeExecutionId,
    pub(super) node: NodeId,
    pub(super) scope: ScopeReference,
    pub(super) mode: NodeExecutionMode,
    pub(super) decision: ReconciliationDecisionId,
    pub(super) reason: Reason,
    pub(super) sequence: RunSequence,
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
