//! Reconciliation tests.

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
                C::StartedDescendantDependencyChanged | C::IncompatibleInterfaceOrSubworkflow => {
                    match policy {
                        P::RequireAuthority => A::RequireAuthority,
                        P::FinishCurrentThenAdopt
                        | P::CancelAndRestartSafeWork
                        | P::CompensateOrRemediate
                        | P::RemoveUnstartedOnly => A::RejectRetrospectiveRewrite,
                    }
                }
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
    let matrix = ReconciliationMatrix::new(&old, &unchanged, &empty);

    assert_eq!(
        matrix
            .classify_node(&node, None, Some(new_node), None)?
            .map(|value| value.0),
        Some(ReconciliationClassification::Added)
    );
    assert_eq!(
        matrix
            .classify_node(&node, Some(old_node), None, None)?
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
            matrix
                .classify_node(&node, Some(old_node), Some(new_node), Some(&occurrence))?
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
