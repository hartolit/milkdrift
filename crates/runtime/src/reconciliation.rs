use std::collections::{BTreeMap, BTreeSet, VecDeque};

use milkdrift_blueprint::{
    BlueprintRevision, EdgeKind, NodeId, NodeKind, node_configuration_fingerprint,
    node_dependency_fingerprint,
};
use milkdrift_capability::SideEffectClass;
#[cfg(test)]
use milkdrift_persistence::RunEventEnvelope;
use milkdrift_persistence::{
    MAX_RECONCILIATION_PLAN_ITEMS, NodeExecutionId, Reason, ReconciliationAction,
    ReconciliationClassification, ReconciliationId, ReconciliationItem, ReconciliationPlanId,
    ReconciliationPolicy, RunEventKind, RunSequence,
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
}

/// Immutable persisted prospective reconciliation plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationPlan {
    reconciliation: ReconciliationId,
    plan: ReconciliationPlanId,
    from_revision: milkdrift_blueprint::RevisionId,
    to_revision: milkdrift_blueprint::RevisionId,
    based_on_sequence: RunSequence,
    items: Vec<ReconciliationItem>,
}

/// One closed comparison owner for node/history classification across two revisions.
struct ReconciliationMatrix<'a> {
    old: &'a BlueprintRevision,
    new: &'a BlueprintRevision,
    history: &'a BTreeMap<NodeId, Vec<NodeHistory>>,
}

impl<'a> ReconciliationMatrix<'a> {
    const fn new(
        old: &'a BlueprintRevision,
        new: &'a BlueprintRevision,
        history: &'a BTreeMap<NodeId, Vec<NodeHistory>>,
    ) -> Self {
        Self { old, new, history }
    }

    fn classify_node(
        &self,
        node: &NodeId,
        old_node: Option<&milkdrift_blueprint::Node>,
        new_node: Option<&milkdrift_blueprint::Node>,
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
                let same_dependencies = node_dependency_fingerprint(self.old.semantic(), node)
                    .map_err(|error| RuntimeError::Reconciliation(error.to_string()))?
                    == node_dependency_fingerprint(self.new.semantic(), node)
                        .map_err(|error| RuntimeError::Reconciliation(error.to_string()))?;
                if incompatible_subworkflow(old_node.kind(), new_node.kind()) {
                    (
                        ReconciliationClassification::IncompatibleInterfaceOrSubworkflow,
                        "pinned subworkflow lineage or interface changed incompatibly",
                    )
                } else if !same_dependencies
                    && started_descendant(node, self.old, self.new, self.history)
                {
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
}

impl ReconciliationPlan {
    /// Closed classifications and prospective actions.
    #[cfg(test)]
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
    #[cfg(test)]
    #[must_use]
    pub fn requires_authority(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.action == ReconciliationAction::RequireAuthority)
    }

    /// Returns whether the plan contains an impossible retrospective rewrite.
    #[cfg(test)]
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

    let matrix = ReconciliationMatrix::new(old, new, history);
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
                matrix.classify_node(&node, old_node, new_node, None)?
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
                let Some((classification, rationale)) =
                    matrix.classify_node(&node, old_node, new_node, Some(execution))?
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
        items,
    })
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

fn allowed_actions(
    classification: ReconciliationClassification,
    policy: ReconciliationPolicy,
) -> &'static [ReconciliationAction] {
    use ReconciliationAction as Action;
    use ReconciliationClassification as Classification;

    const PRESERVE: &[Action] = &[Action::Preserve];
    const USE_NEW: &[Action] = &[Action::UseNewOnNextInvocation];
    const REMOVE: &[Action] = &[Action::RemoveUnstarted];
    const REJECT: &[Action] = &[Action::RejectRetrospectiveRewrite];
    const AUTHORITY: &[Action] = &[Action::RequireAuthority];
    const CANCEL_OR_REJECT: &[Action] =
        &[Action::CancelAndRestart, Action::RejectRetrospectiveRewrite];
    const REMEDIATE_OR_REJECT: &[Action] = &[
        Action::CompensateOrRemediate,
        Action::RejectRetrospectiveRewrite,
    ];

    match classification {
        Classification::UnchangedCompleted
        | Classification::UnchangedActive
        | Classification::UnchangedPending => PRESERVE,
        Classification::Added
        | Classification::ChangedPending
        | Classification::ChangedCompleted => USE_NEW,
        Classification::RemovedPending => REMOVE,
        Classification::ChangedActive => match policy {
            ReconciliationPolicy::FinishCurrentThenAdopt => USE_NEW,
            ReconciliationPolicy::CancelAndRestartSafeWork => CANCEL_OR_REJECT,
            ReconciliationPolicy::CompensateOrRemediate => REMEDIATE_OR_REJECT,
            ReconciliationPolicy::RemoveUnstartedOnly => REJECT,
            ReconciliationPolicy::RequireAuthority => AUTHORITY,
        },
        Classification::CompletedOrUncertainSideEffects => match policy {
            ReconciliationPolicy::CompensateOrRemediate => REMEDIATE_OR_REJECT,
            ReconciliationPolicy::RequireAuthority => AUTHORITY,
            ReconciliationPolicy::FinishCurrentThenAdopt
            | ReconciliationPolicy::CancelAndRestartSafeWork
            | ReconciliationPolicy::RemoveUnstartedOnly => REJECT,
        },
        Classification::StartedDescendantDependencyChanged
        | Classification::IncompatibleInterfaceOrSubworkflow => match policy {
            ReconciliationPolicy::RequireAuthority => AUTHORITY,
            ReconciliationPolicy::FinishCurrentThenAdopt
            | ReconciliationPolicy::CancelAndRestartSafeWork
            | ReconciliationPolicy::CompensateOrRemediate
            | ReconciliationPolicy::RemoveUnstartedOnly => REJECT,
        },
        Classification::RequiresAuthority => AUTHORITY,
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

    match (classification, policy) {
        (Classification::ChangedActive, ReconciliationPolicy::CancelAndRestartSafeWork) => {
            if target_exists
                && history.is_some_and(|history| {
                    matches!(
                        history.state,
                        HistoricalExecutionState::Active {
                            cancellation_safe: true,
                            side_effect: SideEffectClass::None | SideEffectClass::ReadOnly,
                        }
                    )
                })
            {
                Action::CancelAndRestart
            } else {
                Action::RejectRetrospectiveRewrite
            }
        }
        (Classification::ChangedActive, ReconciliationPolicy::CompensateOrRemediate)
        | (
            Classification::CompletedOrUncertainSideEffects,
            ReconciliationPolicy::CompensateOrRemediate,
        ) => {
            if target_exists {
                Action::CompensateOrRemediate
            } else {
                Action::RejectRetrospectiveRewrite
            }
        }
        _ => allowed_actions(classification, policy)[0],
    }
}

pub(crate) fn reconciliation_action_is_valid(
    classification: ReconciliationClassification,
    action: ReconciliationAction,
    policy: ReconciliationPolicy,
) -> bool {
    allowed_actions(classification, policy).contains(&action)
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
#[cfg(test)]
fn validate_plan_is_fresh(
    plan: &ReconciliationPlan,
    current_revision: &milkdrift_blueprint::RevisionId,
    events_after_base: &[RunEventEnvelope],
) -> Result<(), RuntimeError> {
    if current_revision != &plan.from_revision {
        return Err(RuntimeError::Reconciliation(
            "reconciliation plan is stale because the revision pin moved".to_owned(),
        ));
    }
    for event in events_after_base {
        if event.sequence() <= plan.based_on_sequence {
            return Err(RuntimeError::Reconciliation(
                "stale-plan validation received an event at or before the plan base".to_owned(),
            ));
        }
        let allowed = match event.kind() {
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
            } => event_plan == &plan.plan,
            _ => false,
        };
        if !allowed {
            return Err(RuntimeError::Reconciliation(format!(
                "plan {} is stale after run event {} at sequence {}",
                plan.plan,
                event.event_id(),
                event.sequence()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
