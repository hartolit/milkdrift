//! Shared bounded plan types and pure orchestration helpers.

mod names;
mod scans;

pub(super) use names::{command_kind_name, event_kind_name};
pub(super) use scans::{
    bounded_projection_map_keys, bounded_projection_set, bounded_projection_sweep_set,
};

use crate::RuntimeError;
use crate::projection::{
    BranchState, CurrentNodeExecution, NodeExecutionState, RunLifecycle, RunProjection,
};
use crate::reconciliation::{HistoricalExecutionState, NodeHistory};
use milkdrift_blueprint::{
    BlueprintRevision, EdgeKind, Node, NodeId, NodeKind, PathSegment, ReducerStrategy,
};
use milkdrift_capability::{
    BoundedJson, ErrorClass, IdempotencyKey, InvocationValueReference, SideEffectClass,
};
use milkdrift_persistence::{
    ControllerAccountAction, ControllerAccountId, IntegrityDigest, MAX_RECONCILIATION_PLAN_ITEMS,
    NodeExecutionId, NodeExecutionMode, NodeOutcome, Reason, RecoveryClassification,
    RepeatContinuationCause, RunEventEnvelope, RunEventKind, RunOutcome, TimestampMillis,
    WaitCondition, WorkspaceMutation,
};
use milkdrift_workspace::{
    ArtifactReference, BranchId, RunId, ScopeKind, ScopeReference, WorkspaceScope, WorkspaceUsage,
    WorkspaceValue, WorkspaceValueReference,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DispatchOutcome {
    Dispatched,
    Deferred,
    PreDispatchFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RepeatBudgetStatus {
    Within,
    Exhausted(RepeatContinuationCause),
    AccountingOverflow,
}

#[derive(Clone, Debug)]
pub(super) enum ResolvedInputValue {
    Inline {
        value: BoundedJson,
        source: Option<WorkspaceValueReference>,
    },
    Workspace(WorkspaceValueReference),
    Artifact(ArtifactReference),
}

#[derive(Default)]
pub(super) struct CommandPlan {
    pub(super) events: Vec<RunEventKind>,
    pub(super) workspace: Vec<WorkspaceMutation>,
    pub(super) creation_usage:
        Option<(WorkspaceUsage, WorkspaceUsage, BTreeSet<ArtifactReference>)>,
    pub(super) required_artifacts: BTreeSet<ArtifactReference>,
    pub(super) expected_lease_revision: Option<IntegrityDigest>,
    pub(super) controller_actions: Vec<ControllerAccountAction>,
    pub(super) expected_controller_revision: Option<(ControllerAccountId, IntegrityDigest)>,
}

impl CommandPlan {
    pub(super) fn one(event: RunEventKind) -> Self {
        Self {
            events: vec![event],
            ..Self::default()
        }
    }
}

pub(super) fn require_lifecycle(
    projection: &RunProjection,
    required: RunLifecycle,
    transition: &str,
) -> Result<(), RuntimeError> {
    if projection.lifecycle() == required {
        Ok(())
    } else {
        Err(RuntimeError::InvalidTransition(format!(
            "cannot {transition} from lifecycle {:?}",
            projection.lifecycle()
        )))
    }
}

pub(super) fn durable_rejection(error: &RuntimeError) -> bool {
    matches!(
        error,
        RuntimeError::AuthorizationDenied { .. }
            | RuntimeError::InvalidCommand(_)
            | RuntimeError::InvalidTransition(_)
            | RuntimeError::Scheduling(_)
            | RuntimeError::Reconciliation(_)
            | RuntimeError::Executor(_)
    )
}

pub(super) const fn node_outcome(outcome: RunOutcome) -> NodeOutcome {
    match outcome {
        RunOutcome::Succeeded => NodeOutcome::Succeeded,
        RunOutcome::Failed => NodeOutcome::Failed,
        RunOutcome::Cancelled => NodeOutcome::Cancelled,
    }
}

pub(super) fn checked_timestamp_add(
    timestamp: TimestampMillis,
    duration_ms: u64,
) -> Result<TimestampMillis, RuntimeError> {
    timestamp
        .get()
        .checked_add(duration_ms)
        .map(TimestampMillis::new)
        .ok_or_else(|| RuntimeError::Scheduling("timestamp overflow".to_owned()))
}

pub(super) const fn node_execution_mode(node: &Node) -> NodeExecutionMode {
    match node.kind() {
        NodeKind::Task { .. } => NodeExecutionMode::Executor,
        NodeKind::Reducer { config }
            if matches!(config.strategy(), ReducerStrategy::Capability(_)) =>
        {
            NodeExecutionMode::Executor
        }
        NodeKind::Branch { .. }
        | NodeKind::Fork { .. }
        | NodeKind::Join { .. }
        | NodeKind::Reducer { .. }
        | NodeKind::Repeat { .. }
        | NodeKind::Wait { .. }
        | NodeKind::SignalWait { .. }
        | NodeKind::Subworkflow { .. }
        | NodeKind::Terminal { .. } => NodeExecutionMode::Runtime,
    }
}

pub(super) fn entry_nodes(revision: &BlueprintRevision) -> Vec<&NodeId> {
    let targeted: BTreeSet<_> = revision
        .semantic()
        .edges()
        .values()
        .map(|edge| edge.target_node())
        .collect();
    revision
        .semantic()
        .nodes()
        .keys()
        .filter(|node| !targeted.contains(node))
        .collect()
}

pub(super) fn control_nodes_before_join(
    revision: &BlueprintRevision,
    start: &NodeId,
    join: &NodeId,
) -> BTreeSet<NodeId> {
    let mut result = BTreeSet::new();
    let mut pending = VecDeque::from([start.clone()]);
    while let Some(node) = pending.pop_front() {
        if &node == join || !result.insert(node.clone()) {
            continue;
        }
        pending.extend(
            revision
                .semantic()
                .edges()
                .values()
                .filter(|edge| edge.kind() == EdgeKind::Control && edge.source_node() == &node)
                .map(|edge| edge.target_node().clone()),
        );
    }
    result
}

pub(super) fn wait_signal_matches(
    condition: &WaitCondition,
    signal_type: &milkdrift_persistence::SignalTypeId,
    correlation: Option<&milkdrift_persistence::CorrelationKey>,
) -> bool {
    match condition {
        WaitCondition::Signal {
            signal_type: expected,
            correlation: expected_correlation,
        }
        | WaitCondition::SignalOrTimer {
            signal_type: expected,
            correlation: expected_correlation,
            ..
        } => expected == signal_type && expected_correlation.as_ref() == correlation,
        WaitCondition::Timer { .. } => false,
    }
}

pub(super) fn predecessors_ready(
    revision: &BlueprintRevision,
    projection: &RunProjection,
    target: &Node,
    target_scope: &ScopeReference,
) -> bool {
    if let NodeKind::Join { config } = target.kind() {
        return projection
            .executions_for_node(config.fork())
            .any(|execution| {
                execution.scope() == target_scope
                    && execution_is_in_current_node_epoch(projection, execution)
            });
    }
    let control_ready = revision
        .semantic()
        .edges()
        .values()
        .filter(|edge| edge.kind() == EdgeKind::Control && edge.target_node() == target.id())
        .all(|edge| {
            projection
                .executions_for_node(edge.source_node())
                .any(|execution| {
                    execution_is_in_current_node_epoch(projection, execution)
                        && execution.scope() == target_scope
                        && execution.state()
                            == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                        && projection
                            .route_for_execution(execution.execution())
                            .is_none_or(|port| port == edge.source_port())
                })
        });
    let data_ready = revision
        .semantic()
        .edges()
        .values()
        .filter(|edge| {
            edge.kind() == EdgeKind::Data
                && edge.target_node() == target.id()
                && target
                    .data_inputs()
                    .get(edge.target_port())
                    .is_some_and(milkdrift_blueprint::DataPort::is_required)
        })
        .all(|edge| {
            projection
                .executions_for_node(edge.source_node())
                .filter(|execution| {
                    execution_is_in_current_node_epoch(projection, *execution)
                        && execution_scope_related(projection, execution.scope(), target_scope)
                        && execution.state()
                            == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                })
                .any(|execution| {
                    execution
                        .outputs()
                        .iter()
                        .any(|output| output.value().key().as_str() == edge.source_port().as_str())
                })
        });
    control_ready && data_ready
}

pub(super) fn execution_scope_related(
    projection: &RunProjection,
    source_scope: &ScopeReference,
    target_scope: &ScopeReference,
) -> bool {
    if source_scope == target_scope {
        return true;
    }
    let mut cursor = projection
        .scopes()
        .get(target_scope)
        .and_then(WorkspaceScope::parent);
    for _ in 0..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let Some(scope) = cursor else {
            break;
        };
        if scope == source_scope {
            return true;
        }
        cursor = projection
            .scopes()
            .get(scope)
            .and_then(WorkspaceScope::parent);
    }
    let Some(ScopeKind::Branch { branch }) = projection
        .scopes()
        .get(source_scope)
        .map(WorkspaceScope::kind)
    else {
        return false;
    };
    projection
        .branches()
        .get(branch)
        .and_then(|branch| projection.current_node_execution(branch.fork_execution()))
        .is_some_and(|fork| fork.scope() == target_scope)
}

pub(super) fn execution_branch_state(
    projection: &RunProjection,
    execution: &NodeExecutionId,
) -> Option<BranchState> {
    let execution = projection.node_executions().get(execution)?;
    let mut cursor = Some(execution.scope());
    let mut active_branch = None;
    for _ in 0..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let Some(scope) = cursor else {
            break;
        };
        let scope_view = projection.scopes().get(scope)?;
        if let ScopeKind::Branch { branch } = scope_view.kind() {
            let state = projection.branches().get(branch)?.state();
            if state != BranchState::Active {
                return Some(state);
            }
            active_branch = Some(state);
        }
        cursor = scope_view.parent();
    }
    active_branch
}

pub(super) fn scope_has_inactive_branch(
    projection: &RunProjection,
    scope: &ScopeReference,
) -> bool {
    let mut cursor = Some(scope);
    for _ in 0..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let Some(reference) = cursor else {
            break;
        };
        let Some(scope_view) = projection.scopes().get(reference) else {
            return true;
        };
        if let ScopeKind::Branch { branch } = scope_view.kind()
            && projection
                .branches()
                .get(branch)
                .is_none_or(|branch| branch.state() != BranchState::Active)
        {
            return true;
        }
        cursor = scope_view.parent();
    }
    false
}

pub(super) fn run_drain_reason(projection: &RunProjection) -> Option<&Reason> {
    projection
        .cancellation()
        .map(|cancellation| cancellation.reason())
        .or_else(|| {
            projection
                .termination()
                .map(|termination| termination.reason())
        })
}

pub(super) fn cancellation_reason_for_branch(
    projection: &RunProjection,
    branch: &BranchId,
    run_reason: Option<&Reason>,
) -> Option<Reason> {
    if let Some(reason) = run_reason {
        return Some(reason.clone());
    }
    let branch = projection.branches().get(branch)?;
    let mut cursor = branch.scope().parent();
    for _ in 0..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let Some(scope) = cursor else {
            break;
        };
        let scope_view = projection.scopes().get(scope)?;
        if let ScopeKind::Branch { branch } = scope_view.kind() {
            let ancestor = projection.branches().get(branch)?;
            if ancestor.state() == BranchState::Cancelling {
                return ancestor.cancellation_reason().cloned();
            }
        }
        cursor = scope_view.parent();
    }
    None
}

pub(super) fn cancellation_reason_for_execution(
    projection: &RunProjection,
    execution: &NodeExecutionId,
    run_reason: Option<&Reason>,
) -> Option<Reason> {
    if let Some(reason) = run_reason {
        return Some(reason.clone());
    }
    if let Some(cancellation) = projection.reconciliation_cancellations().get(execution) {
        return Some(cancellation.reason().clone());
    }
    let execution = projection.node_executions().get(execution)?;
    let mut cursor = Some(execution.scope());
    for _ in 0..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let Some(scope) = cursor else {
            break;
        };
        let scope_view = projection.scopes().get(scope)?;
        if let ScopeKind::Branch { branch } = scope_view.kind() {
            let branch = projection.branches().get(branch)?;
            if branch.state() == BranchState::Cancelling {
                return branch.cancellation_reason().cloned();
            }
        }
        cursor = scope_view.parent();
    }
    None
}

pub(super) fn reconciliation_history(
    projection: &RunProjection,
    revision: &BlueprintRevision,
    target_revision: &BlueprintRevision,
) -> Result<BTreeMap<NodeId, Vec<NodeHistory>>, RuntimeError> {
    let mut result: BTreeMap<NodeId, Vec<NodeHistory>> = BTreeMap::new();
    let mut retained = 0_usize;
    for execution in projection.current_node_executions() {
        if !revision.semantic().nodes().contains_key(execution.node())
            && !target_revision
                .semantic()
                .nodes()
                .contains_key(execution.node())
        {
            continue;
        }
        retained = retained.saturating_add(1);
        if retained > MAX_RECONCILIATION_PLAN_ITEMS {
            return Err(RuntimeError::Reconciliation(format!(
                "reconciliation history exceeds {MAX_RECONCILIATION_PLAN_ITEMS} relevant executions"
            )));
        }
        let attempt = execution
            .attempts()
            .last()
            .and_then(|attempt| projection.attempts().get(attempt));
        let side_effect = match execution {
            CurrentNodeExecution::Active(_) => attempt
                .and_then(|attempt| attempt.side_effect())
                .map_or(SideEffectClass::None, |classification| {
                    classification.side_effect()
                }),
            CurrentNodeExecution::Settled(summary) => summary.side_effect(),
        };
        let structured_active = projection
            .execution_has_active_structured_ownership(execution.execution())
            || revision
                .semantic()
                .nodes()
                .get(execution.node())
                .is_some_and(|node| match node.kind() {
                    NodeKind::Join { config } => projection
                        .executions_for_node(config.fork())
                        .filter(|fork| fork.scope() == execution.scope())
                        .any(|fork| {
                            projection.branches().values().any(|branch| {
                                branch.is_active() && branch.fork_execution() == fork.execution()
                            })
                        }),
                    NodeKind::Branch { .. }
                    | NodeKind::Fork { .. }
                    | NodeKind::Repeat { .. }
                    | NodeKind::Wait { .. }
                    | NodeKind::SignalWait { .. }
                    | NodeKind::Subworkflow { .. }
                    | NodeKind::Terminal { .. }
                    | NodeKind::Task { .. }
                    | NodeKind::Reducer { .. } => false,
                });
        let state = match execution.state() {
            NodeExecutionState::Eligible if structured_active => HistoricalExecutionState::Active {
                side_effect,
                // Structured cancellation has its own durable protocol; the
                // attempt-local cancel-and-restart action is not enactable.
                cancellation_safe: false,
            },
            NodeExecutionState::Eligible => HistoricalExecutionState::Pending,
            NodeExecutionState::RetryPending(_) => HistoricalExecutionState::Active {
                side_effect,
                cancellation_safe: false,
            },
            NodeExecutionState::Scheduled(_) | NodeExecutionState::Running(_) => {
                HistoricalExecutionState::Active {
                    side_effect,
                    cancellation_safe: matches!(
                        side_effect,
                        SideEffectClass::None | SideEffectClass::ReadOnly
                    ),
                }
            }
            NodeExecutionState::Uncertain(_) => HistoricalExecutionState::Uncertain { side_effect },
            NodeExecutionState::CancelledBeforeDispatch => HistoricalExecutionState::Completed {
                side_effect: SideEffectClass::None,
            },
            NodeExecutionState::RemovedProspectively(_) => HistoricalExecutionState::Pending,
            NodeExecutionState::Terminal(_) => HistoricalExecutionState::Completed { side_effect },
        };
        result
            .entry(execution.node().clone())
            .or_default()
            .push(NodeHistory::new(
                execution.execution().clone(),
                execution.scope().clone(),
                execution.created_sequence(),
                state,
            ));
    }
    for histories in result.values_mut() {
        histories.sort_by(|left, right| {
            left.created_sequence()
                .cmp(&right.created_sequence())
                .then_with(|| left.execution().cmp(right.execution()))
                .then_with(|| left.scope().cmp(right.scope()))
        });
    }
    Ok(result)
}

pub(super) fn node_occurrence_exists_for_current_pin(
    projection: &RunProjection,
    node: &NodeId,
    scope: &ScopeReference,
) -> bool {
    projection
        .executions_for_node(node)
        .any(|execution| execution.scope() == scope && execution.is_current_epoch())
}

pub(super) fn execution_is_in_current_node_epoch(
    _projection: &RunProjection,
    execution: CurrentNodeExecution<'_>,
) -> bool {
    execution.is_current_epoch()
}

pub(super) fn source_execution_is_valid_for_occurrence(
    projection: &RunProjection,
    source: CurrentNodeExecution<'_>,
    target_node: &NodeId,
    target_scope: &ScopeReference,
) -> bool {
    let target_created = projection
        .executions_for_node(target_node)
        .filter(|target| target.scope() == target_scope)
        .filter(|target| target.is_current_epoch())
        .map(CurrentNodeExecution::created_sequence)
        .max();
    let Some(target_created) = target_created else {
        return false;
    };
    source.created_sequence() <= target_created
        && source
            .epoch_retired_sequence()
            .is_none_or(|retired| retired > target_created)
}

pub(super) fn recovery_classification(
    attempt: &crate::projection::NodeAttemptProjection,
) -> RecoveryClassification {
    let Some(side_effect) = attempt.side_effect() else {
        return RecoveryClassification::Uncertain;
    };
    match side_effect.side_effect() {
        SideEffectClass::None | SideEffectClass::ReadOnly => RecoveryClassification::Retryable,
        // Projection validation already requires every idempotent-write
        // classification to carry a supported, stable idempotency key.
        SideEffectClass::IdempotentWrite => RecoveryClassification::Retryable,
        SideEffectClass::NonIdempotentWrite | SideEffectClass::Unknown => {
            RecoveryClassification::Uncertain
        }
    }
}

pub(super) fn unresolved_retry_error_class(
    attempt: &crate::projection::NodeAttemptProjection,
) -> ErrorClass {
    if attempt
        .recovery()
        .iter()
        .any(|observation| observation.lease().is_some())
    {
        ErrorClass::Transport
    } else {
        ErrorClass::Adapter
    }
}

pub(super) const fn recovery_reason(classification: RecoveryClassification) -> &'static str {
    match classification {
        RecoveryClassification::NotStarted => {
            "no executor start was observed and the expired lease may be safely reassigned"
        }
        RecoveryClassification::Retryable => {
            "the expired work is read-only, side-effect-free, or protected by durable idempotency"
        }
        RecoveryClassification::LeaseStillValid => {
            "an unexpired durable lease still owns this invocation"
        }
        RecoveryClassification::Uncertain => {
            "the expired invocation may have externally visible effects"
        }
        RecoveryClassification::TerminalObserved => {
            "a durable terminal outcome was already observed"
        }
    }
}

pub(super) fn collect_required_artifacts(
    events: &[RunEventEnvelope],
    workspace: &[WorkspaceMutation],
) -> Result<BTreeSet<ArtifactReference>, RuntimeError> {
    let mut required = BTreeSet::new();
    for mutation in workspace {
        if let WorkspaceMutation::PutValue { entry } = mutation
            && let Some(artifact) = entry.value().as_artifact()
        {
            required.insert(artifact.clone());
        }
    }
    for event in events {
        required.extend(event.kind().required_artifacts()?);
    }
    Ok(required)
}

pub(super) fn stable_idempotency_key(
    run: &RunId,
    execution: &NodeExecutionId,
) -> Result<IdempotencyKey, RuntimeError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.runtime.idempotency-key.v1\0");
    hasher.update(run.as_str().as_bytes());
    // Durable identities exclude NUL, so these separators make the tuple framing
    // unambiguous without length conversion or concatenating its bounded inputs.
    hasher.update(b"\0");
    hasher.update(execution.as_str().as_bytes());
    IdempotencyKey::new(format!("milkdrift-v1-{}", hasher.finalize().to_hex()))
        .map_err(|error| RuntimeError::Scheduling(error.to_string()))
}

fn invocation_workspace_reference(
    reference: &WorkspaceValueReference,
) -> Result<InvocationValueReference, RuntimeError> {
    let identity = serde_json::to_string(reference)?;
    Ok(InvocationValueReference::WorkspaceValue {
        identity,
        version: reference.version().get().to_string(),
    })
}

pub(super) fn invocation_value_reference(
    value: ResolvedInputValue,
) -> Result<InvocationValueReference, RuntimeError> {
    match value {
        ResolvedInputValue::Inline { value, .. } => Ok(InvocationValueReference::Inline { value }),
        ResolvedInputValue::Workspace(reference) => invocation_workspace_reference(&reference),
        ResolvedInputValue::Artifact(reference) => {
            let capability_reference = milkdrift_capability::ArtifactReference::new(
                reference.artifact().as_str().to_owned(),
                reference.digest().to_hex(),
                Some(reference.media_type().as_str().to_owned()),
                Some(reference.size_bytes()),
            )
            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            Ok(InvocationValueReference::Artifact {
                reference: capability_reference,
            })
        }
    }
}

pub(super) fn select_json_path(
    value: &BoundedJson,
    segments: &[PathSegment],
) -> Result<BoundedJson, RuntimeError> {
    let mut selected = value.value();
    for segment in segments {
        selected = match segment {
            PathSegment::Field(field) => selected.get(field.as_str()).ok_or_else(|| {
                RuntimeError::Scheduling(format!("structured input path field {field} is absent"))
            })?,
            PathSegment::Index(index) => selected.get(usize::from(*index)).ok_or_else(|| {
                RuntimeError::Scheduling(format!("structured input path index {index} is absent"))
            })?,
        };
    }
    BoundedJson::new(selected.clone()).map_err(|error| RuntimeError::Scheduling(error.to_string()))
}

pub(super) fn workspace_value_as_bounded(
    value: &WorkspaceValue,
) -> Result<BoundedJson, RuntimeError> {
    match value {
        WorkspaceValue::Json(value) => Ok(value.clone()),
        WorkspaceValue::Artifact(reference) => artifact_reference_as_bounded(reference),
    }
}

pub(super) fn artifact_reference_as_bounded(
    reference: &ArtifactReference,
) -> Result<BoundedJson, RuntimeError> {
    BoundedJson::new(serde_json::to_value(reference)?)
        .map_err(|error| RuntimeError::Scheduling(error.to_string()))
}

pub(super) fn checked_increment<K: Ord>(
    map: &mut BTreeMap<K, u32>,
    key: K,
) -> Result<(), RuntimeError> {
    let value = map.entry(key).or_insert(0);
    *value = value
        .checked_add(1)
        .ok_or_else(|| RuntimeError::Scheduling("admission count overflow".to_owned()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
