use std::collections::{BTreeMap, BTreeSet, VecDeque};

use milkdrift_blueprint::{
    BlueprintRevision, EdgeKind, NodeId, NodeKind, node_configuration_fingerprint,
    node_dependency_fingerprint,
};
use milkdrift_capability::SideEffectClass;
use milkdrift_persistence::{
    MAX_RECONCILIATION_PLAN_ITEMS, NodeExecutionId, Reason, ReconciliationAction,
    ReconciliationClassification, ReconciliationId, ReconciliationItem, ReconciliationPlanId,
    ReconciliationPolicy, RunEventEnvelope, RunEventKind, RunSequence,
};
use milkdrift_workspace::ScopeReference;

use crate::RuntimeError;

/// Historical execution state considered by prospective reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HistoricalExecutionState {
    /// Execution has not crossed its scheduling/dispatch boundary.
    Pending,
    /// An attempt is scheduled, leased, or running.
    Active {
        /// Most conservative known side-effect class.
        side_effect: SideEffectClass,
        /// Whether a confirmed cancellation boundary permits restart.
        cancellation_safe: bool,
    },
    /// Execution completed and remains immutable history.
    Completed {
        /// Most conservative attempted/observed side-effect class.
        side_effect: SideEffectClass,
    },
    /// Outcome or externally visible effects remain uncertain.
    Uncertain {
        /// Most conservative side-effect class.
        side_effect: SideEffectClass,
    },
}

impl HistoricalExecutionState {
    fn has_started(&self) -> bool {
        !matches!(self, Self::Pending)
    }

    fn completed_or_uncertain_effect(&self) -> bool {
        match self {
            Self::Completed { side_effect } | Self::Uncertain { side_effect } => !matches!(
                side_effect,
                SideEffectClass::None | SideEffectClass::ReadOnly
            ),
            Self::Pending | Self::Active { .. } => false,
        }
    }
}

/// Exact history for one scoped logical execution supplied by the run projection.
///
/// One semantic node may execute repeatedly in independent branch, iteration, and
/// subworkflow scopes. Reconciliation therefore retains each execution separately.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeHistory {
    execution: NodeExecutionId,
    scope: ScopeReference,
    created_sequence: RunSequence,
    state: HistoricalExecutionState,
}

impl NodeHistory {
    /// Constructs one exact scoped execution history.
    #[must_use]
    pub const fn new(
        execution: NodeExecutionId,
        scope: ScopeReference,
        created_sequence: RunSequence,
        state: HistoricalExecutionState,
    ) -> Self {
        Self {
            execution,
            scope,
            created_sequence,
            state,
        }
    }

    /// Stable logical execution identity.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Exact branch, iteration, subworkflow, or run-root scope.
    #[must_use]
    pub const fn scope(&self) -> &ScopeReference {
        &self.scope
    }

    /// Sequence at which this execution became eligible.
    #[must_use]
    pub const fn created_sequence(&self) -> RunSequence {
        self.created_sequence
    }

    /// Historical commitment state of this execution.
    #[must_use]
    pub const fn state(&self) -> &HistoricalExecutionState {
        &self.state
    }
}

/// Immutable persisted prospective reconciliation plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationPlan {
    reconciliation: ReconciliationId,
    plan: ReconciliationPlanId,
    from_revision: milkdrift_blueprint::RevisionId,
    to_revision: milkdrift_blueprint::RevisionId,
    based_on_sequence: RunSequence,
    policy: ReconciliationPolicy,
    items: Vec<ReconciliationItem>,
}

impl ReconciliationPlan {
    /// Owning adoption request.
    #[must_use]
    pub const fn reconciliation(&self) -> &ReconciliationId {
        &self.reconciliation
    }

    /// Immutable plan identity.
    #[must_use]
    pub const fn plan(&self) -> &ReconciliationPlanId {
        &self.plan
    }

    /// Exact old revision.
    #[must_use]
    pub const fn from_revision(&self) -> &milkdrift_blueprint::RevisionId {
        &self.from_revision
    }

    /// Exact requested new revision.
    #[must_use]
    pub const fn to_revision(&self) -> &milkdrift_blueprint::RevisionId {
        &self.to_revision
    }

    /// Exact run sequence whose projection was compared.
    #[must_use]
    pub const fn based_on_sequence(&self) -> RunSequence {
        self.based_on_sequence
    }

    /// Requested prospective policy.
    #[must_use]
    pub const fn policy(&self) -> ReconciliationPolicy {
        self.policy
    }

    /// Closed classifications and prospective actions.
    #[must_use]
    pub fn items(&self) -> &[ReconciliationItem] {
        &self.items
    }

    /// Event fact persisted before any plan application.
    #[must_use]
    pub fn recorded_event(&self) -> RunEventKind {
        RunEventKind::ReconciliationPlanRecorded {
            reconciliation: self.reconciliation.clone(),
            plan: self.plan.clone(),
            from_revision: self.from_revision.clone(),
            to_revision: self.to_revision.clone(),
            based_on_sequence: self.based_on_sequence,
            items: self.items.clone(),
        }
    }

    /// Returns whether at least one item requires an authority decision.
    #[must_use]
    pub fn requires_authority(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.action == ReconciliationAction::RequireAuthority)
    }

    /// Returns whether the plan contains an impossible retrospective rewrite.
    #[must_use]
    pub fn is_rejected(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.action == ReconciliationAction::RejectRetrospectiveRewrite)
    }
}

/// Pure deterministic planner over exact immutable revisions and one event projection.
pub fn plan_reconciliation(
    reconciliation: ReconciliationId,
    plan: ReconciliationPlanId,
    old: &BlueprintRevision,
    new: &BlueprintRevision,
    based_on_sequence: RunSequence,
    history: &BTreeMap<NodeId, Vec<NodeHistory>>,
    policy: ReconciliationPolicy,
) -> Result<ReconciliationPlan, RuntimeError> {
    if old.semantic().workflow() != new.semantic().workflow() {
        return Err(RuntimeError::Reconciliation(
            "prospective adoption cannot cross workflow lineages".to_owned(),
        ));
    }
    if old.id() == new.id() {
        return Err(RuntimeError::Reconciliation(
            "prospective adoption requires a different immutable revision".to_owned(),
        ));
    }

    let mut items = Vec::new();
    if old.semantic().interface() != new.semantic().interface() {
        items.push(ReconciliationItem {
            node: None,
            execution: None,
            classification: ReconciliationClassification::IncompatibleInterfaceOrSubworkflow,
            action: action_for(
                ReconciliationClassification::IncompatibleInterfaceOrSubworkflow,
                None,
                policy,
                false,
            ),
            reason: Reason::new(
                "workflow interface changed and cannot reinterpret this run's existing inputs or outputs",
            )?,
        });
    }

    let identities: BTreeSet<_> = old
        .semantic()
        .nodes()
        .keys()
        .chain(new.semantic().nodes().keys())
        .cloned()
        .collect();
    let mut historical_execution_ids = BTreeSet::new();
    for node in identities {
        let old_node = old.semantic().nodes().get(&node);
        let new_node = new.semantic().nodes().get(&node);
        // A node added to the immediately prospective revision is a new occurrence
        // even when an older, already removed revision used the same stable NodeId.
        let histories = if old_node.is_none() && new_node.is_some() {
            &[][..]
        } else {
            history.get(&node).map_or(&[][..], Vec::as_slice)
        };
        let additional_items = histories.len().max(1);
        if items.len().saturating_add(additional_items) > MAX_RECONCILIATION_PLAN_ITEMS {
            return Err(RuntimeError::Reconciliation(format!(
                "reconciliation plan exceeds {MAX_RECONCILIATION_PLAN_ITEMS} items"
            )));
        }
        let mut executions = histories.to_vec();
        executions.sort_by(|left, right| {
            left.created_sequence
                .cmp(&right.created_sequence)
                .then_with(|| left.execution.cmp(&right.execution))
                .then_with(|| left.scope.cmp(&right.scope))
        });
        if executions.iter().any(|item| {
            item.created_sequence == RunSequence::ZERO
                || item.created_sequence > based_on_sequence
                || !historical_execution_ids.insert(item.execution.clone())
        }) {
            return Err(RuntimeError::Reconciliation(format!(
                "node '{node}' contains duplicate, zero-sequence, or future execution history"
            )));
        }

        if executions.is_empty() {
            let Some((classification, rationale)) =
                classify_node(&node, old_node, new_node, old, new, history, None)?
            else {
                continue;
            };
            items.push(ReconciliationItem {
                node: Some(node),
                execution: None,
                classification,
                action: action_for(classification, None, policy, new_node.is_some()),
                reason: Reason::new(rationale)?,
            });
        } else {
            for execution in &executions {
                let Some((classification, rationale)) = classify_node(
                    &node,
                    old_node,
                    new_node,
                    old,
                    new,
                    history,
                    Some(execution),
                )?
                else {
                    continue;
                };
                items.push(ReconciliationItem {
                    node: Some(node.clone()),
                    execution: Some(execution.execution.clone()),
                    classification,
                    action: action_for(classification, Some(execution), policy, new_node.is_some()),
                    reason: Reason::new(rationale)?,
                });
            }
        }
    }

    if items.len() > MAX_RECONCILIATION_PLAN_ITEMS {
        return Err(RuntimeError::Reconciliation(format!(
            "reconciliation plan exceeds {MAX_RECONCILIATION_PLAN_ITEMS} items"
        )));
    }

    Ok(ReconciliationPlan {
        reconciliation,
        plan,
        from_revision: old.id().clone(),
        to_revision: new.id().clone(),
        based_on_sequence,
        policy,
        items,
    })
}

#[allow(clippy::too_many_arguments)] // One reconciliation classification compares the complete old/new/history fact set.
fn classify_node(
    node: &NodeId,
    old_node: Option<&milkdrift_blueprint::Node>,
    new_node: Option<&milkdrift_blueprint::Node>,
    old: &BlueprintRevision,
    new: &BlueprintRevision,
    history: &BTreeMap<NodeId, Vec<NodeHistory>>,
    execution: Option<&NodeHistory>,
) -> Result<Option<(ReconciliationClassification, &'static str)>, RuntimeError> {
    let classification = match (old_node, new_node) {
        (None, Some(_)) => (
            ReconciliationClassification::Added,
            "node exists only in the prospective revision",
        ),
        (Some(_), None) => classify_removed(execution),
        (Some(old_node), Some(new_node)) => {
            let same_configuration = node_configuration_fingerprint(old_node)
                .map_err(|error| RuntimeError::Reconciliation(error.to_string()))?
                == node_configuration_fingerprint(new_node)
                    .map_err(|error| RuntimeError::Reconciliation(error.to_string()))?;
            let same_dependencies = node_dependency_fingerprint(old.semantic(), node)
                .map_err(|error| RuntimeError::Reconciliation(error.to_string()))?
                == node_dependency_fingerprint(new.semantic(), node)
                    .map_err(|error| RuntimeError::Reconciliation(error.to_string()))?;
            if incompatible_subworkflow(old_node.kind(), new_node.kind()) {
                (
                    ReconciliationClassification::IncompatibleInterfaceOrSubworkflow,
                    "pinned subworkflow lineage or interface changed incompatibly",
                )
            } else if !same_dependencies && started_descendant(node, old, new, history) {
                (
                    ReconciliationClassification::StartedDescendantDependencyChanged,
                    "dependency edit affects a descendant that already started",
                )
            } else if same_configuration && same_dependencies {
                classify_unchanged(execution)
            } else {
                classify_changed(execution)
            }
        }
        (None, None) => return Ok(None),
    };
    Ok(Some(classification))
}

fn classify_unchanged(
    history: Option<&NodeHistory>,
) -> (ReconciliationClassification, &'static str) {
    match history.map(|value| &value.state) {
        Some(HistoricalExecutionState::Completed { .. })
        | Some(HistoricalExecutionState::Uncertain { .. }) => (
            ReconciliationClassification::UnchangedCompleted,
            "completed definition and dependencies are unchanged",
        ),
        Some(HistoricalExecutionState::Active { .. }) => (
            ReconciliationClassification::UnchangedActive,
            "active definition and dependencies are unchanged",
        ),
        Some(HistoricalExecutionState::Pending) | None => (
            ReconciliationClassification::UnchangedPending,
            "pending definition and dependencies are unchanged",
        ),
    }
}

fn classify_changed(history: Option<&NodeHistory>) -> (ReconciliationClassification, &'static str) {
    match history.map(|value| &value.state) {
        Some(state) if state.completed_or_uncertain_effect() => (
            ReconciliationClassification::CompletedOrUncertainSideEffects,
            "changed work has completed or uncertain external side effects",
        ),
        Some(HistoricalExecutionState::Completed { .. })
        | Some(HistoricalExecutionState::Uncertain { .. }) => (
            ReconciliationClassification::ChangedCompleted,
            "completed work changed but its prior history remains immutable",
        ),
        Some(HistoricalExecutionState::Active { .. }) => (
            ReconciliationClassification::ChangedActive,
            "active work changed after its execution boundary",
        ),
        Some(HistoricalExecutionState::Pending) | None => (
            ReconciliationClassification::ChangedPending,
            "pending work changed before execution",
        ),
    }
}

fn classify_removed(history: Option<&NodeHistory>) -> (ReconciliationClassification, &'static str) {
    match history.map(|value| &value.state) {
        Some(state) if state.completed_or_uncertain_effect() => (
            ReconciliationClassification::CompletedOrUncertainSideEffects,
            "removed work has completed or uncertain external side effects",
        ),
        Some(HistoricalExecutionState::Completed { .. })
        | Some(HistoricalExecutionState::Uncertain { .. }) => (
            ReconciliationClassification::ChangedCompleted,
            "removed completed work remains immutable history",
        ),
        Some(HistoricalExecutionState::Active { .. }) => (
            ReconciliationClassification::ChangedActive,
            "removed work is already active",
        ),
        Some(HistoricalExecutionState::Pending) | None => (
            ReconciliationClassification::RemovedPending,
            "removed work has never started",
        ),
    }
}

fn action_for(
    classification: ReconciliationClassification,
    history: Option<&NodeHistory>,
    policy: ReconciliationPolicy,
    target_exists: bool,
) -> ReconciliationAction {
    use ReconciliationAction as Action;
    use ReconciliationClassification as Classification;
    match classification {
        Classification::UnchangedCompleted
        | Classification::UnchangedActive
        | Classification::UnchangedPending => Action::Preserve,
        Classification::Added
        | Classification::ChangedPending
        | Classification::ChangedCompleted => Action::UseNewOnNextInvocation,
        Classification::RemovedPending => Action::RemoveUnstarted,
        Classification::ChangedActive => match policy {
            ReconciliationPolicy::FinishCurrentThenAdopt => Action::UseNewOnNextInvocation,
            ReconciliationPolicy::CancelAndRestartSafeWork
                if target_exists
                    && history.is_some_and(|history| {
                        matches!(
                            history.state,
                            HistoricalExecutionState::Active {
                                cancellation_safe: true,
                                side_effect: SideEffectClass::None | SideEffectClass::ReadOnly,
                            }
                        )
                    }) =>
            {
                Action::CancelAndRestart
            }
            ReconciliationPolicy::CompensateOrRemediate if target_exists => {
                Action::CompensateOrRemediate
            }
            ReconciliationPolicy::RequireAuthority => Action::RequireAuthority,
            ReconciliationPolicy::CancelAndRestartSafeWork
            | ReconciliationPolicy::CompensateOrRemediate
            | ReconciliationPolicy::RemoveUnstartedOnly => Action::RejectRetrospectiveRewrite,
        },
        Classification::CompletedOrUncertainSideEffects => match policy {
            ReconciliationPolicy::CompensateOrRemediate if target_exists => {
                Action::CompensateOrRemediate
            }
            ReconciliationPolicy::RequireAuthority => Action::RequireAuthority,
            ReconciliationPolicy::FinishCurrentThenAdopt
            | ReconciliationPolicy::CancelAndRestartSafeWork
            | ReconciliationPolicy::CompensateOrRemediate
            | ReconciliationPolicy::RemoveUnstartedOnly => Action::RejectRetrospectiveRewrite,
        },
        Classification::StartedDescendantDependencyChanged
        | Classification::IncompatibleInterfaceOrSubworkflow => match policy {
            ReconciliationPolicy::RequireAuthority => Action::RequireAuthority,
            ReconciliationPolicy::FinishCurrentThenAdopt
            | ReconciliationPolicy::CancelAndRestartSafeWork
            | ReconciliationPolicy::CompensateOrRemediate
            | ReconciliationPolicy::RemoveUnstartedOnly => Action::RejectRetrospectiveRewrite,
        },
        Classification::RequiresAuthority => Action::RequireAuthority,
    }
}

pub(crate) const fn reconciliation_action_is_valid(
    classification: ReconciliationClassification,
    action: ReconciliationAction,
    policy: ReconciliationPolicy,
) -> bool {
    use ReconciliationAction as Action;
    use ReconciliationClassification as Classification;
    match classification {
        Classification::UnchangedCompleted
        | Classification::UnchangedActive
        | Classification::UnchangedPending => matches!(action, Action::Preserve),
        Classification::Added
        | Classification::ChangedPending
        | Classification::ChangedCompleted => {
            matches!(action, Action::UseNewOnNextInvocation)
        }
        Classification::RemovedPending => matches!(action, Action::RemoveUnstarted),
        Classification::ChangedActive => match policy {
            ReconciliationPolicy::FinishCurrentThenAdopt => {
                matches!(action, Action::UseNewOnNextInvocation)
            }
            ReconciliationPolicy::CancelAndRestartSafeWork => {
                matches!(
                    action,
                    Action::CancelAndRestart | Action::RejectRetrospectiveRewrite
                )
            }
            ReconciliationPolicy::CompensateOrRemediate => {
                matches!(
                    action,
                    Action::CompensateOrRemediate | Action::RejectRetrospectiveRewrite
                )
            }
            ReconciliationPolicy::RemoveUnstartedOnly => {
                matches!(action, Action::RejectRetrospectiveRewrite)
            }
            ReconciliationPolicy::RequireAuthority => {
                matches!(action, Action::RequireAuthority)
            }
        },
        Classification::CompletedOrUncertainSideEffects => match policy {
            ReconciliationPolicy::CompensateOrRemediate => {
                matches!(
                    action,
                    Action::CompensateOrRemediate | Action::RejectRetrospectiveRewrite
                )
            }
            ReconciliationPolicy::RequireAuthority => {
                matches!(action, Action::RequireAuthority)
            }
            ReconciliationPolicy::FinishCurrentThenAdopt
            | ReconciliationPolicy::CancelAndRestartSafeWork
            | ReconciliationPolicy::RemoveUnstartedOnly => {
                matches!(action, Action::RejectRetrospectiveRewrite)
            }
        },
        Classification::StartedDescendantDependencyChanged
        | Classification::IncompatibleInterfaceOrSubworkflow => match policy {
            ReconciliationPolicy::RequireAuthority => {
                matches!(action, Action::RequireAuthority)
            }
            ReconciliationPolicy::FinishCurrentThenAdopt
            | ReconciliationPolicy::CancelAndRestartSafeWork
            | ReconciliationPolicy::CompensateOrRemediate
            | ReconciliationPolicy::RemoveUnstartedOnly => {
                matches!(action, Action::RejectRetrospectiveRewrite)
            }
        },
        Classification::RequiresAuthority => matches!(action, Action::RequireAuthority),
    }
}

fn incompatible_subworkflow(old: &NodeKind, new: &NodeKind) -> bool {
    match (old, new) {
        (NodeKind::Subworkflow { reference: old }, NodeKind::Subworkflow { reference: new }) => {
            old.workflow() != new.workflow() || old.interface() != new.interface()
        }
        (NodeKind::Subworkflow { .. }, _) | (_, NodeKind::Subworkflow { .. }) => true,
        _ => false,
    }
}

fn started_descendant(
    node: &NodeId,
    old: &BlueprintRevision,
    new: &BlueprintRevision,
    history: &BTreeMap<NodeId, Vec<NodeHistory>>,
) -> bool {
    descendants(node, old)
        .into_iter()
        .chain(descendants(node, new))
        .any(|descendant| {
            history
                .get(&descendant)
                .is_some_and(|executions| executions.iter().any(|value| value.state.has_started()))
        })
}

fn descendants(node: &NodeId, revision: &BlueprintRevision) -> BTreeSet<NodeId> {
    let mut adjacency: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for edge in revision.semantic().edges().values() {
        if matches!(edge.kind(), EdgeKind::Control | EdgeKind::Data) {
            adjacency
                .entry(edge.source_node().clone())
                .or_default()
                .push(edge.target_node().clone());
        }
    }
    let mut queue = VecDeque::from([node.clone()]);
    let mut seen = BTreeSet::new();
    while let Some(current) = queue.pop_front() {
        if let Some(children) = adjacency.get(&current) {
            for child in children {
                if child != node && seen.insert(child.clone()) {
                    queue.push_back(child.clone());
                }
            }
        }
    }
    seen
}

/// Verifies that only the plan's own request/plan/decision facts followed its base sequence.
pub fn validate_plan_is_fresh(
    plan: &ReconciliationPlan,
    current_revision: &milkdrift_blueprint::RevisionId,
    events_after_base: &[RunEventEnvelope],
) -> Result<(), RuntimeError> {
    if current_revision != plan.from_revision() {
        return Err(RuntimeError::Reconciliation(
            "reconciliation plan is stale because the revision pin moved".to_owned(),
        ));
    }
    for event in events_after_base {
        if event.sequence() <= plan.based_on_sequence() {
            return Err(RuntimeError::Reconciliation(
                "stale-plan validation received an event at or before the plan base".to_owned(),
            ));
        }
        let allowed = match event.kind() {
            RunEventKind::RevisionAdoptionRequested { reconciliation, .. } => {
                reconciliation == plan.reconciliation()
            }
            RunEventKind::ReconciliationPlanRecorded {
                reconciliation,
                plan: event_plan,
                ..
            } => reconciliation == plan.reconciliation() && event_plan == plan.plan(),
            RunEventKind::ReconciliationDecisionRecorded {
                plan: event_plan, ..
            } => event_plan == plan.plan(),
            _ => false,
        };
        if !allowed {
            return Err(RuntimeError::Reconciliation(format!(
                "plan {} is stale after run event {} at sequence {}",
                plan.plan(),
                event.event_id(),
                event.sequence()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use milkdrift_blueprint::{
        AuthorRef, Edge, EdgeId, FieldId, InterfaceField, Mutation, MutationBatch, Node,
        PinnedSubworkflow, PortId, SchemaRef, TerminalOutcome, WorkflowId, WorkflowInterface,
    };
    use milkdrift_capability::{CapabilityRequirement, OperationId, SchemaId};
    use milkdrift_persistence::{EventId, RepeatTerminationReason, TimestampMillis};
    use milkdrift_workspace::{RunId, ScopeId};

    use super::*;

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    fn task(name: &str, operation: &str) -> TestResult<Node> {
        Ok(Node::new(
            NodeId::new(name)?,
            NodeKind::task_direct_inputs(CapabilityRequirement::new(OperationId::new(operation)?))?,
        )?)
    }

    fn terminal(name: &str) -> TestResult<Node> {
        Ok(Node::new(
            NodeId::new(name)?,
            NodeKind::Terminal {
                outcome: TerminalOutcome::Success,
            },
        )?)
    }

    fn empty_interface() -> TestResult<WorkflowInterface> {
        Ok(WorkflowInterface::new([], [])?)
    }

    fn linear_revision(workflow: &str, operation: &str) -> TestResult<BlueprintRevision> {
        let work = task("work", operation)?.with_control_output(PortId::new("next")?)?;
        let done = terminal("done")?.with_control_input(PortId::new("in")?)?;
        Ok(BlueprintRevision::genesis(
            WorkflowId::new(workflow)?,
            MutationBatch::new(vec![
                Mutation::SetInterface {
                    interface: empty_interface()?,
                },
                Mutation::AddNode { node: work },
                Mutation::AddNode { node: done },
                Mutation::AddEdge {
                    edge: Edge::new(
                        EdgeId::new("work-done")?,
                        EdgeKind::Control,
                        NodeId::new("work")?,
                        PortId::new("next")?,
                        NodeId::new("done")?,
                        PortId::new("in")?,
                    ),
                },
            ])?,
            AuthorRef::new("test")?,
            "genesis",
        )?)
    }

    fn revise_task(old: &BlueprintRevision, operation: &str) -> TestResult<BlueprintRevision> {
        let replacement = task("work", operation)?.with_control_output(PortId::new("next")?)?;
        Ok(old.revise(
            old.id(),
            MutationBatch::new(vec![Mutation::ReplaceNode { node: replacement }])?,
            AuthorRef::new("test")?,
            "replace work",
        )?)
    }

    fn revise_without_semantic_change(old: &BlueprintRevision) -> TestResult<BlueprintRevision> {
        Ok(old.revise(
            old.id(),
            MutationBatch::new(vec![Mutation::SetInterface {
                interface: old.semantic().interface().clone(),
            }])?,
            AuthorRef::new("test")?,
            "republish",
        )?)
    }

    fn scope(name: &str) -> TestResult<ScopeReference> {
        Ok(ScopeReference::new(
            RunId::new("run-reconcile")?,
            ScopeId::new(name)?,
        ))
    }

    fn history(
        execution: &str,
        scope_name: &str,
        sequence: u64,
        state: HistoricalExecutionState,
    ) -> TestResult<NodeHistory> {
        Ok(NodeHistory::new(
            NodeExecutionId::new(execution)?,
            scope(scope_name)?,
            RunSequence::new(sequence),
            state,
        ))
    }

    fn plan_ids() -> TestResult<(ReconciliationId, ReconciliationPlanId)> {
        Ok((
            ReconciliationId::new("reconciliation")?,
            ReconciliationPlanId::new("plan")?,
        ))
    }

    fn envelope(sequence: u64, kind: RunEventKind) -> TestResult<RunEventEnvelope> {
        Ok(RunEventEnvelope::new(
            EventId::new(format!("event-{sequence}"))?,
            RunId::new("run-reconcile")?,
            RunSequence::new(sequence),
            TimestampMillis::new(sequence),
            kind,
        )?)
    }

    #[test]
    fn action_matrix_is_closed_for_every_classification_and_policy() -> TestResult {
        use ReconciliationAction as A;
        use ReconciliationClassification as C;
        use ReconciliationPolicy as P;

        let policies = [
            P::FinishCurrentThenAdopt,
            P::CancelAndRestartSafeWork,
            P::CompensateOrRemediate,
            P::RemoveUnstartedOnly,
            P::RequireAuthority,
        ];
        let classifications = [
            C::UnchangedCompleted,
            C::ChangedCompleted,
            C::UnchangedActive,
            C::UnchangedPending,
            C::ChangedActive,
            C::ChangedPending,
            C::Added,
            C::RemovedPending,
            C::CompletedOrUncertainSideEffects,
            C::StartedDescendantDependencyChanged,
            C::IncompatibleInterfaceOrSubworkflow,
            C::RequiresAuthority,
        ];
        let safe_active = history(
            "execution-active",
            "scope-active",
            1,
            HistoricalExecutionState::Active {
                side_effect: SideEffectClass::ReadOnly,
                cancellation_safe: true,
            },
        )?;

        for classification in classifications {
            for policy in policies {
                let expected = match classification {
                    C::UnchangedCompleted | C::UnchangedActive | C::UnchangedPending => A::Preserve,
                    C::ChangedCompleted | C::ChangedPending | C::Added => A::UseNewOnNextInvocation,
                    C::RemovedPending => A::RemoveUnstarted,
                    C::ChangedActive => match policy {
                        P::FinishCurrentThenAdopt => A::UseNewOnNextInvocation,
                        P::CancelAndRestartSafeWork => A::CancelAndRestart,
                        P::CompensateOrRemediate => A::CompensateOrRemediate,
                        P::RemoveUnstartedOnly => A::RejectRetrospectiveRewrite,
                        P::RequireAuthority => A::RequireAuthority,
                    },
                    C::CompletedOrUncertainSideEffects => match policy {
                        P::CompensateOrRemediate => A::CompensateOrRemediate,
                        P::RequireAuthority => A::RequireAuthority,
                        P::FinishCurrentThenAdopt
                        | P::CancelAndRestartSafeWork
                        | P::RemoveUnstartedOnly => A::RejectRetrospectiveRewrite,
                    },
                    C::StartedDescendantDependencyChanged
                    | C::IncompatibleInterfaceOrSubworkflow => match policy {
                        P::RequireAuthority => A::RequireAuthority,
                        P::FinishCurrentThenAdopt
                        | P::CancelAndRestartSafeWork
                        | P::CompensateOrRemediate
                        | P::RemoveUnstartedOnly => A::RejectRetrospectiveRewrite,
                    },
                    C::RequiresAuthority => A::RequireAuthority,
                };
                let actual = action_for(classification, Some(&safe_active), policy, true);
                assert_eq!(actual, expected, "{classification:?} under {policy:?}");
                assert!(reconciliation_action_is_valid(
                    classification,
                    actual,
                    policy
                ));
            }
        }

        let unsafe_active = history(
            "execution-unsafe",
            "scope-unsafe",
            2,
            HistoricalExecutionState::Active {
                side_effect: SideEffectClass::NonIdempotentWrite,
                cancellation_safe: false,
            },
        )?;
        assert_eq!(
            action_for(
                C::ChangedActive,
                Some(&unsafe_active),
                P::CancelAndRestartSafeWork,
                true,
            ),
            A::RejectRetrospectiveRewrite
        );
        assert_eq!(
            action_for(
                C::ChangedActive,
                Some(&safe_active),
                P::CancelAndRestartSafeWork,
                false,
            ),
            A::RejectRetrospectiveRewrite
        );
        for classification in [C::ChangedActive, C::CompletedOrUncertainSideEffects] {
            assert_eq!(
                action_for(
                    classification,
                    Some(&safe_active),
                    P::CompensateOrRemediate,
                    false,
                ),
                A::RejectRetrospectiveRewrite
            );
            assert_eq!(
                action_for(
                    classification,
                    Some(&safe_active),
                    P::RequireAuthority,
                    false,
                ),
                A::RequireAuthority
            );
        }
        assert!(!reconciliation_action_is_valid(
            C::RemovedPending,
            A::Preserve,
            P::FinishCurrentThenAdopt
        ));
        Ok(())
    }

    #[test]
    fn planner_keeps_every_scoped_occurrence_in_deterministic_execution_order() -> TestResult {
        let old = linear_revision("multi-occurrence", "tool.old")?;
        let new = revise_task(&old, "tool.new")?;
        let node = NodeId::new("work")?;
        let histories = vec![
            history(
                "execution-effect",
                "scope-effect",
                40,
                HistoricalExecutionState::Completed {
                    side_effect: SideEffectClass::NonIdempotentWrite,
                },
            )?,
            history(
                "execution-pending",
                "scope-pending",
                10,
                HistoricalExecutionState::Pending,
            )?,
            history(
                "execution-completed",
                "scope-completed",
                30,
                HistoricalExecutionState::Completed {
                    side_effect: SideEffectClass::ReadOnly,
                },
            )?,
            history(
                "execution-active",
                "scope-active",
                20,
                HistoricalExecutionState::Active {
                    side_effect: SideEffectClass::ReadOnly,
                    cancellation_safe: true,
                },
            )?,
            history(
                "execution-uncertain",
                "scope-uncertain",
                50,
                HistoricalExecutionState::Uncertain {
                    side_effect: SideEffectClass::Unknown,
                },
            )?,
        ];
        let (reconciliation, plan) = plan_ids()?;
        let planned = plan_reconciliation(
            reconciliation,
            plan,
            &old,
            &new,
            RunSequence::new(50),
            &BTreeMap::from([(node.clone(), histories)]),
            ReconciliationPolicy::CancelAndRestartSafeWork,
        )?;
        let work_items: Vec<_> = planned
            .items()
            .iter()
            .filter(|item| item.node.as_ref() == Some(&node))
            .collect();
        assert_eq!(work_items.len(), 5);
        assert_eq!(
            work_items
                .iter()
                .map(|item| item.execution.as_ref().map(NodeExecutionId::as_str))
                .collect::<Vec<_>>(),
            vec![
                Some("execution-pending"),
                Some("execution-active"),
                Some("execution-completed"),
                Some("execution-effect"),
                Some("execution-uncertain"),
            ]
        );
        assert_eq!(
            work_items
                .iter()
                .map(|item| item.classification)
                .collect::<Vec<_>>(),
            vec![
                ReconciliationClassification::ChangedPending,
                ReconciliationClassification::ChangedActive,
                ReconciliationClassification::ChangedCompleted,
                ReconciliationClassification::CompletedOrUncertainSideEffects,
                ReconciliationClassification::CompletedOrUncertainSideEffects,
            ]
        );
        assert_eq!(work_items[1].action, ReconciliationAction::CancelAndRestart);
        assert_eq!(
            work_items[3].action,
            ReconciliationAction::RejectRetrospectiveRewrite
        );
        assert!(planned.is_rejected());
        Ok(())
    }

    #[test]
    fn planner_rejects_histories_whose_actions_cannot_fit_one_atomic_commit() -> TestResult {
        let old = linear_revision("plan-bound", "tool.old")?;
        let new = revise_task(&old, "tool.new")?;
        let node = NodeId::new("work")?;
        let occurrences = (0..=MAX_RECONCILIATION_PLAN_ITEMS)
            .map(|index| {
                history(
                    &format!("execution-{index}"),
                    &format!("scope-{index}"),
                    u64::try_from(index)?.saturating_add(1),
                    HistoricalExecutionState::Pending,
                )
            })
            .collect::<TestResult<Vec<_>>>()?;
        let (reconciliation, plan) = plan_ids()?;
        let result = plan_reconciliation(
            reconciliation,
            plan,
            &old,
            &new,
            RunSequence::new(1_000),
            &BTreeMap::from([(node, occurrences)]),
            ReconciliationPolicy::FinishCurrentThenAdopt,
        );
        assert!(matches!(
            result,
            Err(RuntimeError::Reconciliation(reason))
                if reason.contains("exceeds 510 items")
        ));
        Ok(())
    }

    #[test]
    fn planner_classifies_added_removed_and_all_unchanged_states() -> TestResult {
        let old = linear_revision("basic-classifications", "tool.same")?;
        let unchanged = revise_without_semantic_change(&old)?;
        let node = NodeId::new("work")?;
        let old_node = old
            .semantic()
            .nodes()
            .get(&node)
            .ok_or("old node missing")?;
        let new_node = unchanged
            .semantic()
            .nodes()
            .get(&node)
            .ok_or("new node missing")?;
        let empty = BTreeMap::new();

        assert_eq!(
            classify_node(&node, None, Some(new_node), &old, &unchanged, &empty, None)?
                .map(|value| value.0),
            Some(ReconciliationClassification::Added)
        );
        assert_eq!(
            classify_node(&node, Some(old_node), None, &old, &unchanged, &empty, None)?
                .map(|value| value.0),
            Some(ReconciliationClassification::RemovedPending)
        );
        for (state, expected) in [
            (
                HistoricalExecutionState::Pending,
                ReconciliationClassification::UnchangedPending,
            ),
            (
                HistoricalExecutionState::Active {
                    side_effect: SideEffectClass::None,
                    cancellation_safe: true,
                },
                ReconciliationClassification::UnchangedActive,
            ),
            (
                HistoricalExecutionState::Completed {
                    side_effect: SideEffectClass::ReadOnly,
                },
                ReconciliationClassification::UnchangedCompleted,
            ),
        ] {
            let occurrence = history("execution", "scope", 1, state)?;
            assert_eq!(
                classify_node(
                    &node,
                    Some(old_node),
                    Some(new_node),
                    &old,
                    &unchanged,
                    &empty,
                    Some(&occurrence),
                )?
                .map(|value| value.0),
                Some(expected)
            );
        }
        Ok(())
    }

    #[test]
    fn dependency_edits_detect_started_descendants() -> TestResult {
        let source = task("source", "tool.source")?.with_control_output(PortId::new("next")?)?;
        let child = task("child", "tool.child")?
            .with_control_input(PortId::new("in")?)?
            .with_control_output(PortId::new("next")?)?;
        let done = terminal("done")?.with_control_input(PortId::new("in")?)?;
        let old = BlueprintRevision::genesis(
            WorkflowId::new("dependency")?,
            MutationBatch::new(vec![
                Mutation::SetInterface {
                    interface: empty_interface()?,
                },
                Mutation::AddNode { node: source },
                Mutation::AddNode { node: child },
                Mutation::AddNode { node: done },
                Mutation::AddEdge {
                    edge: Edge::new(
                        EdgeId::new("source-child")?,
                        EdgeKind::Control,
                        NodeId::new("source")?,
                        PortId::new("next")?,
                        NodeId::new("child")?,
                        PortId::new("in")?,
                    ),
                },
                Mutation::AddEdge {
                    edge: Edge::new(
                        EdgeId::new("child-done")?,
                        EdgeKind::Control,
                        NodeId::new("child")?,
                        PortId::new("next")?,
                        NodeId::new("done")?,
                        PortId::new("in")?,
                    ),
                },
            ])?,
            AuthorRef::new("test")?,
            "dependency genesis",
        )?;
        let new = old.revise(
            old.id(),
            MutationBatch::new(vec![
                Mutation::RemoveEdge {
                    edge: EdgeId::new("source-child")?,
                },
                Mutation::AddEdge {
                    edge: Edge::new(
                        EdgeId::new("source-child-v2")?,
                        EdgeKind::Control,
                        NodeId::new("source")?,
                        PortId::new("next")?,
                        NodeId::new("child")?,
                        PortId::new("in")?,
                    ),
                },
            ])?,
            AuthorRef::new("test")?,
            "replace dependency fact",
        )?;
        let child_history = history(
            "execution-child",
            "scope-child",
            2,
            HistoricalExecutionState::Active {
                side_effect: SideEffectClass::ReadOnly,
                cancellation_safe: true,
            },
        )?;
        let (reconciliation, plan) = plan_ids()?;
        let planned = plan_reconciliation(
            reconciliation,
            plan,
            &old,
            &new,
            RunSequence::new(2),
            &BTreeMap::from([(NodeId::new("child")?, vec![child_history])]),
            ReconciliationPolicy::FinishCurrentThenAdopt,
        )?;
        assert!(planned.items().iter().any(|item| {
            item.node
                .as_ref()
                .is_some_and(|node| node.as_str() == "source")
                && item.classification
                    == ReconciliationClassification::StartedDescendantDependencyChanged
                && item.action == ReconciliationAction::RejectRetrospectiveRewrite
        }));
        Ok(())
    }

    #[test]
    fn interface_and_subworkflow_changes_are_explicitly_incompatible() -> TestResult {
        let old = linear_revision("interface", "tool.same")?;
        let schema = SchemaRef::new(SchemaId::new("schema.input")?, 1)?;
        let interface_changed = old.revise(
            old.id(),
            MutationBatch::new(vec![Mutation::SetInterface {
                interface: WorkflowInterface::new(
                    [(FieldId::new("input")?, InterfaceField::required(schema))],
                    [],
                )?,
            }])?,
            AuthorRef::new("test")?,
            "change interface",
        )?;
        let (reconciliation, plan) = plan_ids()?;
        let interface_plan = plan_reconciliation(
            reconciliation,
            plan,
            &old,
            &interface_changed,
            RunSequence::new(1),
            &BTreeMap::new(),
            ReconciliationPolicy::RequireAuthority,
        )?;
        assert!(interface_plan.items().iter().any(|item| {
            item.node.is_none()
                && item.execution.is_none()
                && item.classification
                    == ReconciliationClassification::IncompatibleInterfaceOrSubworkflow
                && item.action == ReconciliationAction::RequireAuthority
        }));
        assert!(interface_plan.requires_authority());

        let body_v1 = linear_revision("body", "tool.body")?;
        let body_v2 = revise_without_semantic_change(&body_v1)?;
        let interface = empty_interface()?;
        let call = Node::new(
            NodeId::new("call")?,
            NodeKind::Subworkflow {
                reference: PinnedSubworkflow::new(
                    WorkflowId::new("body")?,
                    body_v1.id().clone(),
                    interface.clone(),
                ),
            },
        )?
        .with_control_output(PortId::new("next")?)?;
        let outer_done = terminal("done")?.with_control_input(PortId::new("in")?)?;
        let outer = BlueprintRevision::genesis(
            WorkflowId::new("outer")?,
            MutationBatch::new(vec![
                Mutation::SetInterface {
                    interface: interface.clone(),
                },
                Mutation::InstantiateSubworkflow { node: call },
                Mutation::AddNode { node: outer_done },
                Mutation::AddEdge {
                    edge: Edge::new(
                        EdgeId::new("call-done")?,
                        EdgeKind::Control,
                        NodeId::new("call")?,
                        PortId::new("next")?,
                        NodeId::new("done")?,
                        PortId::new("in")?,
                    ),
                },
            ])?,
            AuthorRef::new("test")?,
            "outer genesis",
        )?;
        let upgraded = outer.revise(
            outer.id(),
            MutationBatch::new(vec![Mutation::UpgradeSubworkflow {
                node: NodeId::new("call")?,
                expected_revision: body_v1.id().clone(),
                replacement: PinnedSubworkflow::new(
                    WorkflowId::new("other-body")?,
                    body_v2.id().clone(),
                    interface,
                ),
            }])?,
            AuthorRef::new("test")?,
            "upgrade child",
        )?;
        let (reconciliation, plan) = (
            ReconciliationId::new("reconciliation-subworkflow")?,
            ReconciliationPlanId::new("plan-subworkflow")?,
        );
        let subworkflow_plan = plan_reconciliation(
            reconciliation,
            plan,
            &outer,
            &upgraded,
            RunSequence::new(1),
            &BTreeMap::new(),
            ReconciliationPolicy::FinishCurrentThenAdopt,
        )?;
        assert!(subworkflow_plan.items().iter().any(|item| {
            item.node
                .as_ref()
                .is_some_and(|node| node.as_str() == "call")
                && item.classification
                    == ReconciliationClassification::IncompatibleInterfaceOrSubworkflow
                && item.action == ReconciliationAction::RejectRetrospectiveRewrite
        }));
        Ok(())
    }

    #[test]
    fn stale_plans_and_impossible_history_are_rejected() -> TestResult {
        let old = linear_revision("stale", "tool.same")?;
        let new = revise_without_semantic_change(&old)?;
        let (reconciliation, plan_id) = plan_ids()?;
        let plan = plan_reconciliation(
            reconciliation.clone(),
            plan_id.clone(),
            &old,
            &new,
            RunSequence::new(2),
            &BTreeMap::new(),
            ReconciliationPolicy::FinishCurrentThenAdopt,
        )?;
        let own_events = vec![
            envelope(
                3,
                RunEventKind::RevisionAdoptionRequested {
                    reconciliation: reconciliation.clone(),
                    requested_by: Some(milkdrift_authority::ActorRef::new(
                        "human:test-reconciliation",
                    )?),
                    from_revision: old.id().clone(),
                    to_revision: new.id().clone(),
                    policy: ReconciliationPolicy::FinishCurrentThenAdopt,
                },
            )?,
            envelope(4, plan.recorded_event())?,
        ];
        validate_plan_is_fresh(&plan, old.id(), &own_events)?;
        let mut stale = own_events;
        stale.push(envelope(
            5,
            RunEventKind::RunPaused {
                reason: Reason::new("state moved")?,
                evidence: Vec::new(),
            },
        )?);
        assert!(validate_plan_is_fresh(&plan, old.id(), &stale).is_err());
        assert!(validate_plan_is_fresh(&plan, new.id(), &[]).is_err());

        let duplicate = history(
            "execution-duplicate",
            "scope-one",
            1,
            HistoricalExecutionState::Pending,
        )?;
        let same_identity = history(
            "execution-duplicate",
            "scope-two",
            2,
            HistoricalExecutionState::Pending,
        )?;
        let (reconciliation, plan_id) = (
            ReconciliationId::new("reconciliation-invalid")?,
            ReconciliationPlanId::new("plan-invalid")?,
        );
        assert!(
            plan_reconciliation(
                reconciliation,
                plan_id,
                &old,
                &new,
                RunSequence::new(2),
                &BTreeMap::from([(NodeId::new("work")?, vec![duplicate, same_identity])]),
                ReconciliationPolicy::FinishCurrentThenAdopt,
            )
            .is_err()
        );

        // Keep every closed reason variant referenced in this contract test so future
        // additions cannot silently become unhandled by reconciliation callers.
        let _ = RepeatTerminationReason::MaximumIterations;
        Ok(())
    }
}
