use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use milkdrift_blueprint::{PortId, RevisionId};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    ActorRef, BranchResultReference, CommandId, CorrelationKey, EvidenceReference, JoinRule,
    NodeExecutionId, Reason, RepeatContinuationCause, RepeatContinuationDecision, RepeatDecisionId,
    RepeatTerminationReason, RunOutcome, RunSequence, SignalDeliveryMode, SignalId, SignalTypeId,
    SubworkflowOwnership, WaitCondition, WaitSatisfaction,
};
use milkdrift_workspace::{
    BranchId, IterationId, RunId, SubworkflowId, WorkspaceScope, WorkspaceValueReference,
};

/// Current state of a structured branch scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct BranchProjection {
    pub(super) branch: BranchId,
    pub(super) fork_execution: NodeExecutionId,
    pub(super) port: PortId,
    pub(super) scope: WorkspaceScope,
    pub(super) children: BTreeSet<NodeExecutionId>,
    pub(super) state: BranchState,
    pub(super) cancellation_reason: Option<Reason>,
    pub(super) outputs: Vec<WorkspaceValueReference>,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct JoinProjection {
    pub(super) execution: NodeExecutionId,
    pub(super) rule: JoinRule,
    pub(super) branches: Vec<BranchResultReference>,
    pub(super) retained_branches: Vec<BranchId>,
    pub(super) sequence: RunSequence,
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum IterationState {
    /// Body/condition may still progress.
    Active,
    /// Frozen condition result awaits the next iteration or repeat termination.
    ConditionRecorded(bool),
    /// A later iteration or repeat termination closed this iteration.
    Completed(bool),
}

/// Isolated repeat-iteration scope read model.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct IterationProjection {
    pub(super) iteration: IterationId,
    pub(super) repeat_execution: NodeExecutionId,
    pub(super) iteration_number: u32,
    pub(super) scope: WorkspaceScope,
    pub(super) state: IterationState,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RepeatContinuationRequestProjection {
    pub(super) frontier_iteration: IterationId,
    pub(super) initial_iteration_limit: u32,
    pub(super) effective_iteration_limit: u32,
    pub(super) cause: RepeatContinuationCause,
    pub(super) sequence: RunSequence,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RepeatContinuationDecisionProjection {
    pub(super) decision: RepeatDecisionId,
    pub(super) actor: ActorRef,
    pub(super) outcome: RepeatContinuationDecision,
    pub(super) approved_additional_iterations: Option<u32>,
    pub(super) reason: Reason,
    pub(super) evidence: Vec<EvidenceReference>,
    pub(super) sequence: RunSequence,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RepeatContinuationProjection {
    pub(super) repeat_execution: NodeExecutionId,
    pub(super) initial_iteration_limit: u32,
    pub(super) effective_iteration_limit: u32,
    pub(super) budget_override_iteration_limit: Option<u32>,
    pub(super) pending_approval: bool,
    pub(super) rejected: bool,
    pub(super) requests: Vec<RepeatContinuationRequestProjection>,
    pub(super) decisions: Vec<RepeatContinuationDecisionProjection>,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RepeatTermination {
    pub(super) repeat_execution: NodeExecutionId,
    pub(super) termination: RepeatTerminationReason,
    pub(super) last_iteration: Option<IterationId>,
    pub(super) sequence: RunSequence,
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum SubworkflowState {
    /// Child remains live.
    Active,
    /// Structured cancellation was propagated to an attached child.
    Cancelling,
    /// Parent observed the child terminal outcome.
    Terminal(RunOutcome),
}

/// One explicit immutable child-output import into the parent run.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SubworkflowOutputImport {
    pub(super) child_value: WorkspaceValueReference,
    pub(super) parent_value: WorkspaceValueReference,
    pub(super) sequence: RunSequence,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SubworkflowProjection {
    pub(super) subworkflow: SubworkflowId,
    pub(super) parent_execution: NodeExecutionId,
    pub(super) child_run: RunId,
    pub(super) child_revision: RevisionId,
    pub(super) scope: WorkspaceScope,
    pub(super) ownership: SubworkflowOwnership,
    pub(super) inputs: Vec<WorkspaceValueReference>,
    pub(super) state: SubworkflowState,
    pub(super) cancellation_reason: Option<Reason>,
    pub(super) outputs: Vec<WorkspaceValueReference>,
    pub(super) imports: Vec<SubworkflowOutputImport>,
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
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SignalProjection {
    pub(super) signal: SignalId,
    pub(super) signal_type: SignalTypeId,
    pub(super) correlation: Option<CorrelationKey>,
    pub(super) mode: SignalDeliveryMode,
    pub(super) payload: BoundedJson,
    pub(super) received_sequence: RunSequence,
    pub(super) consumed_by: BTreeSet<NodeExecutionId>,
    pub(super) broadcast_scan_through: Option<NodeExecutionId>,
    pub(super) broadcast_scan_complete: bool,
    pub(super) duplicate_commands: Vec<CommandId>,
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WaitProjection {
    pub(super) execution: NodeExecutionId,
    pub(super) condition: WaitCondition,
    pub(super) registered_sequence: RunSequence,
    pub(super) satisfaction: Option<WaitSatisfaction>,
    pub(super) cancellation: Option<WaitCancellationProjection>,
}

/// Durable cancellation fact for a wait.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WaitCancellationProjection {
    pub(super) reason: Reason,
    pub(super) sequence: RunSequence,
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
