//! Logical execution identity, lifecycle, and settled summaries.

use milkdrift_blueprint::{NodeId, PortId, RevisionId};
use milkdrift_capability::{ErrorClass, SideEffectClass};
use milkdrift_persistence::{
    AttemptId, BoundedDetail, NodeExecutionId, NodeExecutionMode, NodeOutcome, Reason,
    ReconciliationPlanId, RunSequence, TimestampMillis,
};
use milkdrift_workspace::ScopeReference;
use serde::{Deserialize, Serialize};

use super::PublishedNodeOutput;

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
    pub(in crate::projection) attempt: Option<AttemptId>,
    pub(in crate::projection) reason: Reason,
    pub(in crate::projection) sequence: RunSequence,
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
    pub(in crate::projection) outcome: NodeOutcome,
    pub(in crate::projection) error_class: Option<ErrorClass>,
    pub(in crate::projection) detail: Option<BoundedDetail>,
    pub(in crate::projection) sequence: RunSequence,
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
    pub(in crate::projection) execution: NodeExecutionId,
    pub(in crate::projection) node: NodeId,
    pub(in crate::projection) scope: ScopeReference,
    pub(in crate::projection) mode: NodeExecutionMode,
    /// Immutable revision governing this occurrence. Keeping it on the active
    /// occurrence avoids retaining the run's lifetime revision-pin timeline.
    pub(in crate::projection) revision: RevisionId,
    /// Revision-pin sequence that retired this occurrence from its node epoch.
    /// Unchanged occurrences remain current across repins.
    pub(in crate::projection) epoch_retired_sequence: Option<RunSequence>,
    pub(in crate::projection) created_sequence: RunSequence,
    pub(in crate::projection) created_at: TimestampMillis,
    pub(in crate::projection) attempts: Vec<AttemptId>,
    /// Total attempts admitted for this occurrence, including compacted attempts.
    pub(in crate::projection) attempt_count: u32,
    pub(in crate::projection) state: NodeExecutionState,
    pub(in crate::projection) cancellation: Option<NodeExecutionCancellationProjection>,
    pub(in crate::projection) deterministic_terminal: Option<DeterministicNodeTerminalProjection>,
    pub(in crate::projection) outputs: Vec<PublishedNodeOutput>,
}

/// Bounded current terminal fact for a settled execution occurrence.
///
/// Full attempt, cancellation, dispatch, and deterministic-terminal detail is
/// journal history after every transition consumer has closed. One summary is
/// retained per live scope/node frontier so current scheduling, output
/// selection, and prospective reconciliation never need a lifetime execution
/// catalog.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SettledNodeExecutionProjection {
    pub(in crate::projection) execution: NodeExecutionId,
    pub(in crate::projection) node: NodeId,
    pub(in crate::projection) scope: ScopeReference,
    pub(in crate::projection) mode: NodeExecutionMode,
    pub(in crate::projection) revision: RevisionId,
    pub(in crate::projection) epoch_retired_sequence: Option<RunSequence>,
    pub(in crate::projection) created_sequence: RunSequence,
    pub(in crate::projection) attempt_count: u32,
    pub(in crate::projection) attempts: Vec<AttemptId>,
    pub(in crate::projection) state: NodeExecutionState,
    /// Exact terminal event retained as a bounded causal-context provenance anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::projection) terminal_sequence: Option<RunSequence>,
    pub(in crate::projection) side_effect: SideEffectClass,
    pub(in crate::projection) route: Option<PortId>,
    pub(in crate::projection) outputs: Vec<PublishedNodeOutput>,
}

impl SettledNodeExecutionProjection {
    /// Stable occurrence identity used to locate exact journal evidence.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Stable semantic node identity.
    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }

    /// Scope whose current semantic frontier owns this summary.
    #[must_use]
    pub const fn scope(&self) -> &ScopeReference {
        &self.scope
    }

    /// Immutable revision governing the settled occurrence.
    #[must_use]
    pub const fn revision(&self) -> &RevisionId {
        &self.revision
    }

    /// Whether the summary remains in its node's current revision epoch.
    #[must_use]
    pub const fn is_current_epoch(&self) -> bool {
        self.epoch_retired_sequence.is_none()
    }

    /// Sequence at which the occurrence became eligible.
    #[must_use]
    pub const fn created_sequence(&self) -> RunSequence {
        self.created_sequence
    }

    /// Terminal occurrence state.
    #[must_use]
    pub const fn state(&self) -> &NodeExecutionState {
        &self.state
    }

    /// Exact terminal event sequence retained for bounded context/audit lookup.
    #[must_use]
    pub const fn terminal_sequence(&self) -> Option<RunSequence> {
        self.terminal_sequence
    }

    /// Total attempts admitted for the occurrence.
    #[must_use]
    pub const fn attempt_count(&self) -> u32 {
        self.attempt_count
    }

    /// Latest attempt anchor for journal provenance, if the occurrence dispatched.
    #[must_use]
    pub fn latest_attempt(&self) -> Option<&AttemptId> {
        self.attempts.last()
    }

    /// Latest attempt anchor only; earlier identities are journal history.
    #[must_use]
    pub fn attempts(&self) -> &[AttemptId] {
        &self.attempts
    }

    /// Most conservative frozen side-effect classification for the occurrence.
    #[must_use]
    pub const fn side_effect(&self) -> SideEffectClass {
        self.side_effect
    }

    /// Frozen branch/router selection needed by the current downstream frontier.
    #[must_use]
    pub const fn route(&self) -> Option<&PortId> {
        self.route.as_ref()
    }

    /// Explicit output references still retained for current downstream work.
    #[must_use]
    pub fn outputs(&self) -> &[PublishedNodeOutput] {
        &self.outputs
    }
}

/// One occurrence in the current operational node/scope frontier.
///
/// The active variant carries complete transition state. The settled variant is
/// a compact terminal fact; neither variant is a historical timeline query.
#[derive(Clone, Copy, Debug)]
pub enum CurrentNodeExecution<'a> {
    /// Operationally live occurrence with full transition state.
    Active(&'a NodeExecutionProjection),
    /// Closed current occurrence represented by a bounded summary.
    Settled(&'a SettledNodeExecutionProjection),
}

impl<'a> CurrentNodeExecution<'a> {
    /// Stable occurrence identity.
    #[must_use]
    pub const fn execution(self) -> &'a NodeExecutionId {
        match self {
            Self::Active(value) => value.execution(),
            Self::Settled(value) => value.execution(),
        }
    }

    /// Stable semantic node identity.
    #[must_use]
    pub const fn node(self) -> &'a NodeId {
        match self {
            Self::Active(value) => value.node(),
            Self::Settled(value) => value.node(),
        }
    }

    /// Exact occurrence scope.
    #[must_use]
    pub const fn scope(self) -> &'a ScopeReference {
        match self {
            Self::Active(value) => value.scope(),
            Self::Settled(value) => value.scope(),
        }
    }

    /// Immutable governing revision.
    #[must_use]
    pub const fn revision(self) -> &'a RevisionId {
        match self {
            Self::Active(value) => value.revision(),
            Self::Settled(value) => value.revision(),
        }
    }

    /// Whether this occurrence remains in its node's current revision epoch.
    #[must_use]
    pub const fn is_current_epoch(self) -> bool {
        match self {
            Self::Active(value) => value.is_current_epoch(),
            Self::Settled(value) => value.is_current_epoch(),
        }
    }

    /// Revision-pin sequence that retired the occurrence epoch, if any.
    #[must_use]
    pub const fn epoch_retired_sequence(self) -> Option<RunSequence> {
        match self {
            Self::Active(value) => value.epoch_retired_sequence(),
            Self::Settled(value) => value.epoch_retired_sequence,
        }
    }

    /// Sequence at which the occurrence became eligible.
    #[must_use]
    pub const fn created_sequence(self) -> RunSequence {
        match self {
            Self::Active(value) => value.created_sequence(),
            Self::Settled(value) => value.created_sequence(),
        }
    }

    /// Current or terminal state.
    #[must_use]
    pub const fn state(self) -> &'a NodeExecutionState {
        match self {
            Self::Active(value) => value.state(),
            Self::Settled(value) => value.state(),
        }
    }

    /// Explicit output references retained for the current frontier.
    #[must_use]
    pub fn outputs(self) -> &'a [PublishedNodeOutput] {
        match self {
            Self::Active(value) => value.outputs(),
            Self::Settled(value) => value.outputs(),
        }
    }

    /// Latest attempt anchor only; settled earlier attempts are journal history.
    #[must_use]
    pub fn attempts(self) -> &'a [AttemptId] {
        match self {
            Self::Active(value) => value.attempts(),
            Self::Settled(value) => &value.attempts,
        }
    }
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

    /// Immutable revision governing this execution occurrence.
    #[must_use]
    pub const fn revision(&self) -> &RevisionId {
        &self.revision
    }

    /// Revision-pin sequence that retired this occurrence from its node epoch.
    #[must_use]
    pub const fn epoch_retired_sequence(&self) -> Option<RunSequence> {
        self.epoch_retired_sequence
    }

    /// Whether this occurrence still belongs to its node's current epoch.
    #[must_use]
    pub const fn is_current_epoch(&self) -> bool {
        self.epoch_retired_sequence.is_none()
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

    /// Total attempts admitted, including settled attempts available only in the journal.
    #[must_use]
    pub const fn attempt_count(&self) -> u32 {
        self.attempt_count
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
