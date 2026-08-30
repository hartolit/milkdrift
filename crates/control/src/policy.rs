use std::collections::BTreeSet;

use milkdrift_blueprint::{BlueprintRevision, Mutation, NodeId, NodeKind};
use milkdrift_capability::{CapabilityCategory, IdempotencyBehavior, SideEffectClass};
use milkdrift_runtime::{NodeExecutionState, RunProjection};
use serde::{Deserialize, Serialize};

use crate::{RequestedRunAction, WorkflowProposal};

/// Stable deterministic risk-policy identity.
pub const CONTROL_RISK_POLICY_ID: &str = "milkdrift.control-risk";
/// Current deterministic risk-policy version.
pub const CONTROL_RISK_POLICY_VERSION_V1: u32 = 1;

/// Coarse deterministic policy classification; authority remains the final decision.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    /// Prospective and bounded; may auto-apply only with exact apply authority.
    Low,
    /// Requires an explicit recorded approval plus exact authority.
    ApprovalRequired,
    /// Cannot be applied at this boundary without changing the closed control contract.
    Forbidden,
}

/// Stable classifier evidence explaining why a class was selected.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskConstraint {
    /// A future-only pure/read-only task was added.
    FutureReadOnlyWorkAdded,
    /// Bounded metadata/reporting content was changed.
    ReportingOrMetadataChanged,
    /// Pausing is the only requested live action.
    PauseRequested,
    /// Retry is idempotent and remains inside the existing run budget.
    IdempotentRetryRequested,
    /// Existing node configuration was replaced.
    ExistingWorkChanged,
    /// Existing work was removed.
    ExistingWorkRemoved,
    /// Dependencies were changed.
    DependencyChanged,
    /// Active/running work is affected.
    RunningWorkAffected,
    /// A started descendant can observe changed dependencies.
    StartedDescendantAffected,
    /// A task can write or has unknown effects.
    ExternalWriteOrUnknownEffect,
    /// A new provider profile is referenced.
    NewProviderProfile,
    /// A process or peer capability class is introduced.
    PrivilegedCapabilityClass,
    /// Repeat/controller ceilings or termination policy changed.
    RepeatOrControllerBoundsChanged,
    /// A workflow terminal condition changed.
    TerminalConditionChanged,
    /// The workflow interface or pinned subworkflow contract changed.
    InterfaceOrSubworkflowChanged,
    /// Cancellation/termination was requested.
    TerminationRequested,
    /// Compensation or remediation was requested.
    CompensationRequested,
    /// Resume or another state-expanding action was requested.
    StateExpandingActionRequested,
    /// An explicit merge attempts to add another lineage parent.
    MergeLineageChanged,
    /// The proposal contains no recognized low-risk prospective change.
    UnclassifiedChange,
}

/// Complete deterministic policy evidence persisted or returned with a decision.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyClassification {
    /// Stable policy lineage.
    pub policy: String,
    /// Exact policy version.
    pub policy_version: u32,
    /// Coarse result.
    pub risk: RiskClass,
    /// Canonically sorted distinct evidence constraints.
    pub constraints: Vec<RiskConstraint>,
}

/// Classifies a fully validated old/new revision pair at an optional live boundary.
#[must_use]
pub fn classify_proposal(
    old: &BlueprintRevision,
    new: &BlueprintRevision,
    proposal: &WorkflowProposal,
    projection: Option<&RunProjection>,
) -> PolicyClassification {
    let mut constraints = BTreeSet::new();
    let mut risk = RiskClass::Low;

    for mutation in proposal.mutation().operations() {
        match mutation {
            Mutation::AddNode { node } | Mutation::InstantiateSubworkflow { node } => {
                classify_added_node(old, node.kind(), &mut constraints, &mut risk);
            }
            Mutation::RemoveNode { node } => {
                elevate(&mut risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::ExistingWorkRemoved);
                classify_live_node(node, projection, &mut constraints, &mut risk);
                if old
                    .semantic()
                    .nodes()
                    .get(node)
                    .is_some_and(|value| matches!(value.kind(), NodeKind::Terminal { .. }))
                {
                    constraints.insert(RiskConstraint::TerminalConditionChanged);
                }
            }
            Mutation::ReplaceNode { node } => {
                elevate(&mut risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::ExistingWorkChanged);
                classify_live_node(node.id(), projection, &mut constraints, &mut risk);
                classify_replacement(old, node.id(), node.kind(), &mut constraints);
            }
            Mutation::AddEdge { edge } => {
                constraints.insert(RiskConstraint::DependencyChanged);
                if !node_is_future_only(edge.target_node(), projection) {
                    elevate(&mut risk, RiskClass::ApprovalRequired);
                    classify_live_node(edge.target_node(), projection, &mut constraints, &mut risk);
                }
            }
            Mutation::ReplaceEdge { edge } => {
                elevate(&mut risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::DependencyChanged);
                classify_live_node(edge.target_node(), projection, &mut constraints, &mut risk);
            }
            Mutation::RemoveEdge { edge } => {
                constraints.insert(RiskConstraint::DependencyChanged);
                if let Some(edge) = old.semantic().edges().get(edge)
                    && !node_is_future_only(edge.target_node(), projection)
                {
                    elevate(&mut risk, RiskClass::ApprovalRequired);
                    classify_live_node(edge.target_node(), projection, &mut constraints, &mut risk);
                }
            }
            Mutation::UpgradeSubworkflow { node, .. } => {
                elevate(&mut risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::InterfaceOrSubworkflowChanged);
                classify_live_node(node, projection, &mut constraints, &mut risk);
            }
            Mutation::SetInterface { .. } => {
                elevate(&mut risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::InterfaceOrSubworkflowChanged);
            }
            Mutation::SetMetadata { .. } => {
                constraints.insert(RiskConstraint::ReportingOrMetadataChanged);
            }
            Mutation::SetMergeParents { .. } => {
                elevate(&mut risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::MergeLineageChanged);
            }
        }
    }

    if old.semantic().interface() != new.semantic().interface() {
        elevate(&mut risk, RiskClass::ApprovalRequired);
        constraints.insert(RiskConstraint::InterfaceOrSubworkflowChanged);
    }
    classify_action(
        proposal.requested_action(),
        projection,
        &mut constraints,
        &mut risk,
    );
    if constraints.is_empty() {
        elevate(&mut risk, RiskClass::ApprovalRequired);
        constraints.insert(RiskConstraint::UnclassifiedChange);
    }

    PolicyClassification {
        policy: CONTROL_RISK_POLICY_ID.to_owned(),
        policy_version: CONTROL_RISK_POLICY_VERSION_V1,
        risk,
        constraints: constraints.into_iter().collect(),
    }
}

fn classify_added_node(
    old: &BlueprintRevision,
    kind: &NodeKind,
    constraints: &mut BTreeSet<RiskConstraint>,
    risk: &mut RiskClass,
) {
    match kind {
        NodeKind::Task { config } => {
            let requirement = config.requirement();
            if requirement.maximum_side_effect_class() <= SideEffectClass::ReadOnly {
                constraints.insert(RiskConstraint::FutureReadOnlyWorkAdded);
            } else {
                elevate(risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::ExternalWriteOrUnknownEffect);
            }
            if let Some(profile) = requirement.provider_profile_ref() {
                let existed = old.semantic().nodes().values().any(|node| {
                    matches!(node.kind(), NodeKind::Task { config } if config.requirement().provider_profile_ref() == Some(profile))
                });
                if !existed {
                    elevate(risk, RiskClass::ApprovalRequired);
                    constraints.insert(RiskConstraint::NewProviderProfile);
                }
            }
            if requirement.categories().iter().any(|category| {
                matches!(
                    category,
                    CapabilityCategory::Process | CapabilityCategory::Peer
                )
            }) {
                elevate(risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::PrivilegedCapabilityClass);
            }
        }
        NodeKind::Reducer { config } => match config.strategy() {
            milkdrift_blueprint::ReducerStrategy::Capability(_) => {
                elevate(risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::ExternalWriteOrUnknownEffect);
                constraints.insert(RiskConstraint::PrivilegedCapabilityClass);
            }
            milkdrift_blueprint::ReducerStrategy::Collect
            | milkdrift_blueprint::ReducerStrategy::First => {
                constraints.insert(RiskConstraint::FutureReadOnlyWorkAdded);
            }
        },
        NodeKind::Wait { .. }
        | NodeKind::SignalWait { .. }
        | NodeKind::Branch { .. }
        | NodeKind::Fork { .. }
        | NodeKind::Join { .. } => {
            constraints.insert(RiskConstraint::FutureReadOnlyWorkAdded);
        }
        NodeKind::Repeat { .. } => {
            elevate(risk, RiskClass::ApprovalRequired);
            constraints.insert(RiskConstraint::RepeatOrControllerBoundsChanged);
        }
        NodeKind::Subworkflow { .. } => {
            elevate(risk, RiskClass::ApprovalRequired);
            constraints.insert(RiskConstraint::InterfaceOrSubworkflowChanged);
        }
        NodeKind::Terminal { .. } => {
            elevate(risk, RiskClass::ApprovalRequired);
            constraints.insert(RiskConstraint::TerminalConditionChanged);
        }
    }
}

fn classify_replacement(
    old: &BlueprintRevision,
    node: &NodeId,
    new_kind: &NodeKind,
    constraints: &mut BTreeSet<RiskConstraint>,
) {
    let old_kind = old.semantic().nodes().get(node).map(|value| value.kind());
    if matches!(old_kind, Some(NodeKind::Terminal { .. }))
        || matches!(new_kind, NodeKind::Terminal { .. })
    {
        constraints.insert(RiskConstraint::TerminalConditionChanged);
    }
    if matches!(old_kind, Some(NodeKind::Repeat { .. }))
        || matches!(new_kind, NodeKind::Repeat { .. })
    {
        constraints.insert(RiskConstraint::RepeatOrControllerBoundsChanged);
    }
    if matches!(old_kind, Some(NodeKind::Subworkflow { .. }))
        || matches!(new_kind, NodeKind::Subworkflow { .. })
    {
        constraints.insert(RiskConstraint::InterfaceOrSubworkflowChanged);
    }
    if let NodeKind::Task { config } = new_kind
        && config.requirement().maximum_side_effect_class() > SideEffectClass::ReadOnly
    {
        constraints.insert(RiskConstraint::ExternalWriteOrUnknownEffect);
    }
    if matches!(
        new_kind,
        NodeKind::Reducer {
            config
        } if matches!(
            config.strategy(),
            milkdrift_blueprint::ReducerStrategy::Capability(_)
        )
    ) {
        constraints.insert(RiskConstraint::ExternalWriteOrUnknownEffect);
        constraints.insert(RiskConstraint::PrivilegedCapabilityClass);
    }
}

fn classify_live_node(
    node: &NodeId,
    projection: Option<&RunProjection>,
    constraints: &mut BTreeSet<RiskConstraint>,
    risk: &mut RiskClass,
) {
    let Some(projection) = projection else {
        return;
    };
    for execution in projection.executions_for_node(node) {
        match execution.state() {
            NodeExecutionState::Running(_) | NodeExecutionState::Uncertain(_) => {
                elevate(risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::RunningWorkAffected);
            }
            NodeExecutionState::Scheduled(_)
            | NodeExecutionState::RetryPending(_)
            | NodeExecutionState::Terminal(_)
            | NodeExecutionState::CancelledBeforeDispatch
            | NodeExecutionState::RemovedProspectively(_) => {
                elevate(risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::StartedDescendantAffected);
            }
            NodeExecutionState::Eligible => {}
        }
    }
}

fn node_is_future_only(node: &NodeId, projection: Option<&RunProjection>) -> bool {
    let Some(projection) = projection else {
        return false;
    };
    let executions = projection.executions_for_node(node).collect::<Vec<_>>();
    executions.is_empty()
        || executions
            .iter()
            .all(|execution| matches!(execution.state(), NodeExecutionState::Eligible))
}

fn classify_action(
    action: Option<&RequestedRunAction>,
    projection: Option<&RunProjection>,
    constraints: &mut BTreeSet<RiskConstraint>,
    risk: &mut RiskClass,
) {
    match action {
        None => {}
        Some(RequestedRunAction::Pause) => {
            constraints.insert(RiskConstraint::PauseRequested);
        }
        Some(RequestedRunAction::RetryExternalWork { attempt }) => {
            let idempotent = projection
                .and_then(|value| value.attempts().get(attempt))
                .and_then(|value| value.side_effect())
                .is_some_and(|value| {
                    value.side_effect() <= SideEffectClass::IdempotentWrite
                        && value.idempotency() != IdempotencyBehavior::Unsupported
                });
            if idempotent {
                constraints.insert(RiskConstraint::IdempotentRetryRequested);
            } else {
                elevate(risk, RiskClass::ApprovalRequired);
                constraints.insert(RiskConstraint::ExternalWriteOrUnknownEffect);
            }
        }
        Some(RequestedRunAction::RequestCancellation) => {
            elevate(risk, RiskClass::ApprovalRequired);
            constraints.insert(RiskConstraint::TerminationRequested);
        }
        Some(RequestedRunAction::Resume | RequestedRunAction::Signal { .. }) => {
            elevate(risk, RiskClass::ApprovalRequired);
            constraints.insert(RiskConstraint::StateExpandingActionRequested);
        }
    }
}

fn elevate(current: &mut RiskClass, candidate: RiskClass) {
    *current = (*current).max(candidate);
}
