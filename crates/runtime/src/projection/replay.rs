use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::{ContentDigest, NodeId, PortId, RevisionId, WorkflowId};
use milkdrift_persistence::{
    AttemptId, AuthorityDecision, LeaseId, NodeExecutionId, RunEventEnvelope, RunEventKind,
    RunSequence, SignalId, SubworkflowOwnership, TimerId,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, BranchId, IterationId, RunId, ScopeReference, SubworkflowId,
    WorkspaceBudget, WorkspaceScope, WorkspaceValueReference,
};

use crate::RuntimeError;

use super::helpers::{invalid, invalid_at};
use super::node::{
    CurrentNodeExecution, LeaseProjection, NodeAttemptProjection, NodeExecutionProjection,
    RetryProjection, SettledNodeExecutionProjection, TimerProjection,
};
use super::reconciliation::{
    ReconciliationCancellationProjection, ReconciliationProjection,
    ReconciliationRemediationProjection, ReconciliationRequestState, RecoveryProjection,
    RemediationProjection,
};
use super::run::{
    ResourceUsage, RevisionPin, RunCancellation, RunLifecycle, RunProjection,
    RunTerminalProjection, RunTerminationIntent,
};
use super::structured::{
    BranchProjection, ControllerAssessmentProjection, IterationProjection, JoinProjection,
    RepeatContinuationProjection, RepeatTermination, SignalProjection, SubworkflowProjection,
    SubworkflowUsageSummary, WaitProjection,
};

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

    /// Last sequence whose settled historical detail was compacted into operational state.
    ///
    /// The append-only journal remains the source of truth for timeline/audit queries.
    /// Active high-frequency collections are never a promise of complete history.
    #[must_use]
    pub const fn history_compacted_through(&self) -> RunSequence {
        self.history_compacted_through
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

    /// Current revision pin required by active planning.
    ///
    /// Earlier pins are historical journal facts.
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

    /// Frozen execution authority established when this run was allowed to start.
    #[must_use]
    pub const fn execution_authority(
        &self,
    ) -> Option<&milkdrift_authority::ExecutionAuthorityBasis> {
        self.execution_authority.as_ref()
    }

    /// Workspace scopes still referenced by active state or retained values.
    #[must_use]
    pub const fn scopes(&self) -> &BTreeMap<ScopeReference, WorkspaceScope> {
        &self.scopes
    }

    /// Immutable workspace value references still retained by active context/output policy.
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

    /// Operationally live logical executions keyed by identity.
    ///
    /// Complete settled history is intentionally absent; use journal pages for
    /// timeline/audit access and [`Self::settled_node_executions`] for bounded
    /// current terminal facts.
    #[must_use]
    pub const fn node_executions(&self) -> &BTreeMap<NodeExecutionId, NodeExecutionProjection> {
        &self.node_executions
    }

    /// Bounded current terminal summaries keyed by their journal occurrence identity.
    #[must_use]
    pub const fn settled_node_executions(
        &self,
    ) -> &BTreeMap<NodeExecutionId, SettledNodeExecutionProjection> {
        &self.settled_node_executions
    }

    /// Looks up one occurrence only when it remains in the operational frontier.
    #[must_use]
    pub fn current_node_execution(
        &self,
        execution: &NodeExecutionId,
    ) -> Option<CurrentNodeExecution<'_>> {
        self.node_executions
            .get(execution)
            .map(CurrentNodeExecution::Active)
            .or_else(|| {
                self.settled_node_executions
                    .get(execution)
                    .map(CurrentNodeExecution::Settled)
            })
    }

    /// Every live or compact current occurrence in deterministic projection order.
    pub(crate) fn current_node_executions(&self) -> impl Iterator<Item = CurrentNodeExecution<'_>> {
        self.node_executions
            .values()
            .map(CurrentNodeExecution::Active)
            .chain(
                self.settled_node_executions
                    .values()
                    .map(CurrentNodeExecution::Settled),
            )
    }

    /// Compact terminal-child usage required by one live structured parent.
    #[must_use]
    pub fn subworkflow_usage_for_execution(
        &self,
        execution: &NodeExecutionId,
    ) -> Option<&SubworkflowUsageSummary> {
        self.subworkflow_usage_by_execution.get(execution)
    }

    /// Latest durable controller assessment for one logical controller occurrence.
    #[must_use]
    pub fn controller_assessment(
        &self,
        execution: &NodeExecutionId,
    ) -> Option<&ControllerAssessmentProjection> {
        self.controller_assessments.get(execution)
    }

    /// Latest assessments for independently identified controllers in this run.
    #[must_use]
    pub const fn controller_assessments(
        &self,
    ) -> &BTreeMap<NodeExecutionId, ControllerAssessmentProjection> {
        &self.controller_assessments
    }

    /// Logical executions that have not reached a closed terminal/removed state.
    #[must_use]
    pub(crate) const fn active_execution_ids(&self) -> &BTreeSet<NodeExecutionId> {
        &self.active_execution_ids
    }

    /// Direct structured branch that owns an execution, when present.
    #[must_use]
    pub fn branch_for_execution(&self, execution: &NodeExecutionId) -> Option<&BranchProjection> {
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
    pub(crate) fn active_lease_for_attempt(&self, attempt: &AttemptId) -> Option<&LeaseProjection> {
        self.active_lease_by_attempt
            .get(attempt)
            .and_then(|lease| self.leases.get(lease))
    }

    /// Current live/settled occurrences of one semantic node in stable identity order.
    ///
    /// This is an operational frontier, not complete execution history.
    pub fn executions_for_node<'a>(
        &'a self,
        node: &'a NodeId,
    ) -> impl Iterator<Item = CurrentNodeExecution<'a>> + 'a {
        self.execution_ids_by_node
            .get(node)
            .into_iter()
            .flat_map(BTreeSet::iter)
            .filter_map(|execution| self.current_node_execution(execution))
    }

    /// Latest execution of a node in one scope or any descendant scope.
    #[must_use]
    pub(crate) fn latest_descendant_execution(
        &self,
        scope: &ScopeReference,
        node: &NodeId,
    ) -> Option<CurrentNodeExecution<'_>> {
        self.latest_descendant_execution_by_scope_node
            .get(&(scope.clone(), node.clone()))
            .and_then(|execution| self.current_node_execution(execution))
    }

    /// Current/latest attempts and unresolved safety obligations keyed by identity.
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
            .and_then(|attempt| self.node_executions.get(attempt.execution()))
            .map(NodeExecutionProjection::revision)
    }

    /// Attempts with uncertain or explicitly retained external truth.
    pub fn unresolved_attempts(&self) -> impl Iterator<Item = &NodeAttemptProjection> {
        self.attempts
            .values()
            .filter(|attempt| attempt.is_unresolved())
    }

    /// Active leases and the latest lease still required by recovery or late evidence.
    #[must_use]
    pub const fn leases(&self) -> &BTreeMap<LeaseId, LeaseProjection> {
        &self.leases
    }

    /// Pending timers and fired/cancelled timers still referenced by an active wait.
    #[must_use]
    pub const fn timers(&self) -> &BTreeMap<TimerId, TimerProjection> {
        &self.timers
    }

    /// Retry decisions whose admission remains pending.
    #[must_use]
    pub const fn retries(&self) -> &BTreeMap<TimerId, RetryProjection> {
        &self.retries
    }

    /// Structured branches still needed by active ownership or current join facts.
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

    /// Frozen route for a live or compact current occurrence.
    #[must_use]
    pub(crate) fn route_for_execution(&self, execution: &NodeExecutionId) -> Option<&PortId> {
        self.branch_routes.get(execution).or_else(|| {
            self.settled_node_executions
                .get(execution)
                .and_then(SettledNodeExecutionProjection::route)
        })
    }

    /// Current satisfied joins still needed by their execution frontier.
    #[must_use]
    pub const fn joins(&self) -> &BTreeMap<NodeExecutionId, JoinProjection> {
        &self.joins
    }

    /// Active/latest repeat iteration frontiers; older iterations are journal history.
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

    /// Current terminal repeat summaries keyed by retained repeat execution.
    #[must_use]
    pub const fn repeat_terminations(&self) -> &BTreeMap<NodeExecutionId, RepeatTermination> {
        &self.repeat_terminations
    }

    /// Unconsumed deliveries and broadcasts whose bounded scan remains incomplete.
    #[must_use]
    pub const fn signals(&self) -> &BTreeMap<SignalId, SignalProjection> {
        &self.signals
    }

    /// Ordered broadcast receipts whose bounded wait-catalog scan is incomplete.
    pub(crate) const fn pending_broadcast_signals(&self) -> &BTreeSet<(RunSequence, SignalId)> {
        &self.pending_broadcast_signals
    }

    /// Pending waits and compact completed waits still needed by their owning execution.
    #[must_use]
    pub const fn waits(&self) -> &BTreeMap<NodeExecutionId, WaitProjection> {
        &self.waits
    }

    /// Active/latest parent links and output-import obligations.
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
    pub(crate) fn execution_has_active_child_ownership(&self, execution: &NodeExecutionId) -> bool {
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

    /// Current/latest prospective revision-reconciliation state.
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

    /// Latest recovery-controller pass; older passes are paged from the journal.
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

    /// Prospective revision requests attributed to the frozen run actor.
    #[must_use]
    pub const fn run_actor_revision_requests(&self) -> u64 {
        self.run_actor_revision_requests
    }

    /// Rejected prospective revision requests attributed to the frozen run actor.
    #[must_use]
    pub const fn run_actor_rejections(&self) -> u64 {
        self.run_actor_rejections
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
        self.sequence = event.sequence();
        self.compact_settled_state()?;
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
            if let Some(request) = self.reconciliation.requests.get_mut(&reconciliation)
                && request.state == ReconciliationRequestState::Planned
            {
                request.state = ReconciliationRequestState::Stale;
            }
        }
    }
}
