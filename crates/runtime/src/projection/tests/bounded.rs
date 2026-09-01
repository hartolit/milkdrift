use super::{
    ActorRef, AttemptId, CapabilityRequirement, ErrorClass, IdempotencyBehavior, InvocationId,
    LeaseId, NodeExecutionId, NodeId, NodeOutcome, OperationId, ProviderProfileRef, Reason,
    ReconciliationAction, ReconciliationClassification, ReconciliationId, ReconciliationItem,
    ReconciliationPlanId, ReconciliationPolicy, RecoveryClassification, RunEventKind,
    RunProjection, RunSequence, SideEffectClass, SignalDeliveryMode, SignalId, SignalTypeId,
    TestResult, TimerId, TimestampMillis, WorkerId, WorkspaceScope, created, eligible, envelope,
    fixture, invocation_request, resolved_snapshot_with_side_effect, runtime_eligible,
};
use crate::query::encode_projection_snapshot;
use milkdrift_workspace::{IterationId, ScopeId};

#[test]
fn ten_thousand_closed_task_occurrences_keep_one_terminal_frontier() -> TestResult {
    let fixture = fixture("bounded-task-occurrences")?;
    let snapshot = resolved_snapshot_with_side_effect(
        1,
        SideEffectClass::None,
        IdempotencyBehavior::Unsupported,
    )?;
    let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
        .provider_profile(ProviderProfileRef::new("publisher-prod")?);
    let mut projection = RunProjection::new();
    projection.apply_replayed(&created(&fixture, 1)?)?;
    projection.apply_replayed(&envelope(2, &fixture.run, RunEventKind::RunStarted)?)?;
    let mut sequence = 3_u64;
    let mut size_at_100 = 0_usize;

    for number in 1..=10_000_u32 {
        let execution = NodeExecutionId::new(format!("execution-{number}"))?;
        let attempt = AttemptId::new(format!("attempt-{number}"))?;
        let invocation = InvocationId::new(format!("invocation-{number}"))?;
        projection.apply_replayed(&eligible(
            sequence,
            &fixture,
            "work",
            &execution,
            fixture.root.reference(),
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("work")?,
                execution: execution.clone(),
                attempt: attempt.clone(),
                invocation: invocation.clone(),
                idempotency_key: None,
                request: invocation_request(&invocation, None)?,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::CapabilityResolved {
                execution: execution.clone(),
                attempt: attempt.clone(),
                requirement: requirement.clone(),
                snapshot: snapshot.clone(),
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::SideEffectClassified {
                attempt: attempt.clone(),
                side_effect: SideEffectClass::None,
                idempotency: IdempotencyBehavior::Unsupported,
                idempotency_key: None,
            },
        )?)?;
        sequence += 1;
        let lease = LeaseId::new(format!("lease-{number}"))?;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::LeaseGranted {
                lease,
                execution: execution.clone(),
                attempt: attempt.clone(),
                worker: WorkerId::new("worker")?,
                expires_at: TimestampMillis::new(sequence.saturating_add(10).saturating_mul(100)),
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::NodeStarted {
                execution: execution.clone(),
                attempt: attempt.clone(),
                invocation,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::NodeTerminal {
                execution: execution.clone(),
                attempt,
                report_sequence: 1,
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::StructuredSuccessorScanCompleted { execution },
        )?)?;
        sequence += 1;
        if number == 100 {
            size_at_100 = encode_projection_snapshot(&projection)?.len();
        }
    }

    assert!(projection.node_executions().is_empty());
    assert_eq!(projection.settled_node_executions().len(), 1);
    assert_eq!(
        projection
            .executions_for_node(&NodeId::new("work")?)
            .count(),
        1
    );
    assert!(projection.attempts().is_empty());
    assert!(projection.invocations.is_empty());
    assert!(projection.leases().is_empty());
    assert_eq!(projection.scopes().len(), 1);
    assert_eq!(projection.execution_ids_by_node.len(), 1);
    assert_eq!(
        projection.latest_descendant_execution_by_scope_node.len(),
        1
    );
    let final_size = encode_projection_snapshot(&projection)?.len();
    assert!(final_size < size_at_100.saturating_mul(2));
    assert!(final_size.abs_diff(size_at_100) < 1_024);
    eprintln!(
        "ordinary_occurrences=10000 active_executions={} settled_summaries={} attempts={} invocations={} leases={} scopes={} node_indexes={} descendant_indexes={} snapshot_at_100_bytes={size_at_100} snapshot_at_10000_bytes={final_size}",
        projection.node_executions().len(),
        projection.settled_node_executions().len(),
        projection.attempts().len(),
        projection.invocations.len(),
        projection.leases().len(),
        projection.scopes().len(),
        projection.execution_ids_by_node.len(),
        projection.latest_descendant_execution_by_scope_node.len(),
    );
    Ok(())
}

#[test]
fn incomplete_and_failed_lower_risk_replacements_preserve_maximum_side_effect_risk() -> TestResult {
    let fixture = fixture("bounded-summary-side-effect")?;
    let first = NodeExecutionId::new("execution-side-effect-first")?;
    let second = NodeExecutionId::new("execution-side-effect-second")?;
    let attempt = AttemptId::new("attempt-side-effect-first")?;
    let invocation = InvocationId::new("invocation-side-effect-first")?;
    let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
        .provider_profile(ProviderProfileRef::new("publisher-prod")?);
    let snapshot = resolved_snapshot_with_side_effect(
        1,
        SideEffectClass::NonIdempotentWrite,
        IdempotencyBehavior::Unsupported,
    )?;
    let events = [
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        eligible(3, &fixture, "work", &first, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("work")?,
                execution: first.clone(),
                attempt: attempt.clone(),
                invocation: invocation.clone(),
                idempotency_key: None,
                request: invocation_request(&invocation, None)?,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::CapabilityResolved {
                execution: first.clone(),
                attempt: attempt.clone(),
                requirement,
                snapshot,
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::SideEffectClassified {
                attempt: attempt.clone(),
                side_effect: SideEffectClass::NonIdempotentWrite,
                idempotency: IdempotencyBehavior::Unsupported,
                idempotency_key: None,
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::LeaseGranted {
                lease: LeaseId::new("lease-side-effect-first")?,
                execution: first.clone(),
                attempt: attempt.clone(),
                worker: WorkerId::new("worker-side-effect-first")?,
                expires_at: TimestampMillis::new(10_000),
            },
        )?,
        envelope(
            8,
            &fixture.run,
            RunEventKind::NodeStarted {
                execution: first.clone(),
                attempt: attempt.clone(),
                invocation,
            },
        )?,
        envelope(
            9,
            &fixture.run,
            RunEventKind::NodeTerminal {
                execution: first.clone(),
                attempt,
                report_sequence: 1,
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?,
        envelope(
            10,
            &fixture.run,
            RunEventKind::StructuredSuccessorScanCompleted {
                execution: first.clone(),
            },
        )?,
        runtime_eligible(11, &fixture, "work", &second, fixture.root.reference())?,
        envelope(
            12,
            &fixture.run,
            RunEventKind::DeterministicNodeTerminal {
                execution: second.clone(),
                outcome: NodeOutcome::Failed,
                error_class: Some(ErrorClass::Provider),
                detail: None,
            },
        )?,
        envelope(
            13,
            &fixture.run,
            RunEventKind::RunTerminal {
                outcome: milkdrift_persistence::RunOutcome::Failed,
                outputs: Vec::new(),
                artifacts: Vec::new(),
                reason: Some(Reason::new("lower-risk replacement failed")?),
            },
        )?,
    ];

    let incomplete = RunProjection::replay(&events[..11])?;
    assert!(incomplete.node_executions().contains_key(&second));
    assert_eq!(
        incomplete.settled_node_executions()[&first].side_effect(),
        SideEffectClass::NonIdempotentWrite
    );

    let projection = RunProjection::replay(&events)?;

    assert_eq!(projection.settled_node_executions().len(), 1);
    assert_eq!(
        projection.settled_node_executions()[&second].side_effect(),
        SideEffectClass::NonIdempotentWrite
    );
    Ok(())
}

#[test]
fn ten_thousand_closed_fork_join_cycles_retire_structured_ownership() -> TestResult {
    let fixture = fixture("bounded-fork-join")?;
    let repeat = NodeExecutionId::new("repeat-execution")?;
    let mut projection = RunProjection::new();
    projection.apply_replayed(&created(&fixture, 1)?)?;
    projection.apply_replayed(&envelope(2, &fixture.run, RunEventKind::RunStarted)?)?;
    projection.apply_replayed(&runtime_eligible(
        3,
        &fixture,
        "repeat",
        &repeat,
        fixture.root.reference(),
    )?)?;
    let mut sequence = 4_u64;
    let mut size_at_100 = 0_usize;
    let mut size_at_10_000 = None;
    for number in 1..=10_000_u32 {
        let iteration = IterationId::new(format!("iteration-{number}"))?;
        let iteration_scope = WorkspaceScope::iteration(
            ScopeId::new(format!("iteration-scope-{number}"))?,
            &fixture.root,
            iteration.clone(),
        )?;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: iteration.clone(),
                iteration_number: number,
                scope: iteration_scope.clone(),
            },
        )?)?;
        sequence += 1;
        let fork = NodeExecutionId::new(format!("fork-{number}"))?;
        projection.apply_replayed(&runtime_eligible(
            sequence,
            &fixture,
            "fork",
            &fork,
            iteration_scope.reference(),
        )?)?;
        sequence += 1;

        let mut results = Vec::new();
        for port_name in ["a", "b"] {
            let port = milkdrift_blueprint::PortId::new(port_name)?;
            let branch =
                milkdrift_workspace::BranchId::new(format!("branch-{number}-{port_name}"))?;
            let branch_scope = WorkspaceScope::branch(
                ScopeId::new(format!("branch-scope-{number}-{port_name}"))?,
                &iteration_scope,
                branch.clone(),
            )?;
            projection.apply_replayed(&envelope(
                sequence,
                &fixture.run,
                RunEventKind::BranchScopeCreated {
                    fork_execution: fork.clone(),
                    port,
                    branch: branch.clone(),
                    scope: branch_scope.clone(),
                },
            )?)?;
            sequence += 1;
            results.push(milkdrift_persistence::BranchResultReference {
                branch,
                scope: branch_scope.reference().clone(),
                outcome: milkdrift_persistence::RunOutcome::Succeeded,
                outputs: Vec::new(),
            });
        }
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::DeterministicNodeTerminal {
                execution: fork.clone(),
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::StructuredSuccessorScanCompleted {
                execution: fork.clone(),
            },
        )?)?;
        sequence += 1;

        for result in &results {
            let child = NodeExecutionId::new(format!("child-{number}-{}", result.branch.as_str()))?;
            projection.apply_replayed(&runtime_eligible(
                sequence,
                &fixture,
                "branch-task",
                &child,
                &result.scope,
            )?)?;
            sequence += 1;
            projection.apply_replayed(&envelope(
                sequence,
                &fixture.run,
                RunEventKind::BranchChildAdded {
                    branch: result.branch.clone(),
                    execution: child.clone(),
                },
            )?)?;
            sequence += 1;
            projection.apply_replayed(&envelope(
                sequence,
                &fixture.run,
                RunEventKind::DeterministicNodeTerminal {
                    execution: child.clone(),
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            )?)?;
            sequence += 1;
            projection.apply_replayed(&envelope(
                sequence,
                &fixture.run,
                RunEventKind::StructuredSuccessorScanCompleted { execution: child },
            )?)?;
            sequence += 1;
            projection.apply_replayed(&envelope(
                sequence,
                &fixture.run,
                RunEventKind::BranchTerminal {
                    branch: result.branch.clone(),
                    outcome: milkdrift_persistence::RunOutcome::Succeeded,
                    outputs: Vec::new(),
                },
            )?)?;
            sequence += 1;
        }

        let join = NodeExecutionId::new(format!("join-{number}"))?;
        projection.apply_replayed(&runtime_eligible(
            sequence,
            &fixture,
            "join",
            &join,
            iteration_scope.reference(),
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::JoinSatisfied {
                execution: join.clone(),
                rule: milkdrift_persistence::JoinRule::All,
                branches: results,
                retained_branches: Vec::new(),
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::DeterministicNodeTerminal {
                execution: join.clone(),
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::StructuredSuccessorScanCompleted { execution: join },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: iteration.clone(),
                result: number != 10_000,
            },
        )?)?;
        sequence += 1;
        if number == 100 {
            size_at_100 = encode_projection_snapshot(&projection)?.len();
        }
        if number == 10_000 {
            size_at_10_000 = Some(encode_projection_snapshot(&projection)?.len());
            projection.apply_replayed(&envelope(
                sequence,
                &fixture.run,
                RunEventKind::RepeatTerminated {
                    repeat_execution: repeat.clone(),
                    termination: milkdrift_persistence::RepeatTerminationReason::ConditionFalse,
                    last_iteration: Some(iteration),
                },
            )?)?;
            sequence += 1;
        }
    }
    projection.apply_replayed(&envelope(
        sequence,
        &fixture.run,
        RunEventKind::DeterministicNodeTerminal {
            execution: repeat.clone(),
            outcome: NodeOutcome::Succeeded,
            error_class: None,
            detail: None,
        },
    )?)?;
    sequence += 1;
    projection.apply_replayed(&envelope(
        sequence,
        &fixture.run,
        RunEventKind::StructuredSuccessorScanCompleted { execution: repeat },
    )?)?;

    assert!(projection.node_executions().is_empty());
    assert_eq!(projection.settled_node_executions().len(), 1);
    assert!(projection.branches().is_empty());
    assert!(projection.branch_by_fork_port.is_empty());
    assert!(projection.branch_ids_by_fork_execution.is_empty());
    assert!(projection.branch_owner.is_empty());
    assert!(projection.branch_routes().is_empty());
    assert!(projection.joins().is_empty());
    assert!(projection.iterations().is_empty());
    assert!(projection.active_scope_ownership.is_empty());
    assert!(
        projection
            .active_structured_children_by_execution
            .is_empty()
    );
    assert_eq!(projection.scopes().len(), 1);
    let size_at_10_000 = size_at_10_000.ok_or("missing 10,000-cycle snapshot evidence")?;
    let final_size = encode_projection_snapshot(&projection)?.len();
    assert!(final_size < size_at_100.saturating_mul(2));
    assert!(size_at_10_000 < size_at_100.saturating_mul(2));
    assert!(size_at_10_000.abs_diff(size_at_100) < 1_024);
    eprintln!(
        "fork_join_cycles=10000 branches={} joins={} routes={} owners={} child_sets={} branch_scopes={} snapshot_at_100_bytes={size_at_100} snapshot_at_10000_bytes={size_at_10_000} closed_snapshot_bytes={final_size}",
        projection.branches().len(),
        projection.joins().len(),
        projection.branch_routes().len(),
        projection.branch_owner.len(),
        projection.active_structured_children_by_execution.len(),
        projection.scopes().len().saturating_sub(1),
    );
    Ok(())
}

#[test]
fn ten_thousand_completed_repeat_children_keep_only_import_frontier() -> TestResult {
    let fixture = fixture("bounded-subworkflows")?;
    let repeat = NodeExecutionId::new("repeat-execution")?;
    let mut projection = RunProjection::new();
    projection.apply_replayed(&created(&fixture, 1)?)?;
    projection.apply_replayed(&envelope(2, &fixture.run, RunEventKind::RunStarted)?)?;
    projection.apply_replayed(&runtime_eligible(
        3,
        &fixture,
        "repeat",
        &repeat,
        fixture.root.reference(),
    )?)?;
    let mut sequence = 4_u64;
    let mut size_at_100 = 0_usize;
    let mut size_at_9_999 = 0_usize;

    for number in 1..=10_000_u32 {
        let iteration = IterationId::new(format!("iteration-{number}"))?;
        let iteration_scope = WorkspaceScope::iteration(
            ScopeId::new(format!("iteration-scope-{number}"))?,
            &fixture.root,
            iteration.clone(),
        )?;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: iteration.clone(),
                iteration_number: number,
                scope: iteration_scope.clone(),
            },
        )?)?;
        sequence += 1;
        let subworkflow = milkdrift_workspace::SubworkflowId::new(format!("child-{number}"))?;
        let child_run = milkdrift_workspace::RunId::new(format!("child-run-{number}"))?;
        let child_scope = WorkspaceScope::subworkflow(
            ScopeId::new(format!("child-scope-{number}"))?,
            &iteration_scope,
            subworkflow.clone(),
        )?;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::SubworkflowCreated {
                subworkflow: subworkflow.clone(),
                parent_execution: repeat.clone(),
                child_run: child_run.clone(),
                child_revision: super::revision('b')?,
                scope: child_scope,
                ownership: milkdrift_persistence::SubworkflowOwnership::Attached,
                inputs: Vec::new(),
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::SubworkflowTerminal {
                subworkflow,
                child_run,
                outcome: milkdrift_persistence::RunOutcome::Succeeded,
                outputs: Vec::new(),
                cost_micros: std::collections::BTreeMap::new(),
                usage: Default::default(),
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: iteration.clone(),
                result: number != 10_000,
            },
        )?)?;
        sequence += 1;
        if number == 100 {
            size_at_100 = encode_projection_snapshot(&projection)?.len();
        } else if number == 9_999 {
            size_at_9_999 = encode_projection_snapshot(&projection)?.len();
        }
        assert!(projection.subworkflows().len() <= 1);
        assert!(projection.child_runs.len() <= 1);
        if number == 10_000 {
            projection.apply_replayed(&envelope(
                sequence,
                &fixture.run,
                RunEventKind::RepeatTerminated {
                    repeat_execution: repeat.clone(),
                    termination: milkdrift_persistence::RepeatTerminationReason::ConditionFalse,
                    last_iteration: Some(iteration),
                },
            )?)?;
            sequence += 1;
        }
    }
    projection.apply_replayed(&envelope(
        sequence,
        &fixture.run,
        RunEventKind::DeterministicNodeTerminal {
            execution: repeat.clone(),
            outcome: NodeOutcome::Succeeded,
            error_class: None,
            detail: None,
        },
    )?)?;
    sequence += 1;
    projection.apply_replayed(&envelope(
        sequence,
        &fixture.run,
        RunEventKind::StructuredSuccessorScanCompleted { execution: repeat },
    )?)?;

    assert!(projection.subworkflows().is_empty());
    assert!(projection.active_subworkflow_ids.is_empty());
    assert!(projection.active_attached_subworkflow_ids.is_empty());
    assert!(projection.child_runs.is_empty());
    assert!(projection.subworkflow_usage_by_execution.is_empty());
    assert!(projection.iterations().is_empty());
    assert_eq!(projection.scopes().len(), 1);
    let final_size = encode_projection_snapshot(&projection)?.len();
    assert!(size_at_100 > 0);
    assert!(size_at_9_999 > 0);
    assert!(size_at_9_999 < size_at_100.saturating_mul(2));
    assert!(size_at_9_999.abs_diff(size_at_100) < 1_024);
    eprintln!(
        "completed_subworkflows=10000 children={} child_runs={} usage_summaries={} iterations={} child_scopes={} snapshot_at_100_bytes={size_at_100} snapshot_at_9999_bytes={size_at_9_999} closed_snapshot_bytes={final_size}",
        projection.subworkflows().len(),
        projection.child_runs.len(),
        projection.subworkflow_usage_by_execution.len(),
        projection.iterations().len(),
        projection.scopes().len().saturating_sub(1),
    );
    Ok(())
}

#[test]
fn settled_output_reference_and_provenance_anchor_survive_full_retirement() -> TestResult {
    let fixture = fixture("retained-output-summary")?;
    let execution = NodeExecutionId::new("output-execution")?;
    let output = milkdrift_workspace::WorkspaceValueReference::new(
        fixture.root.reference().clone(),
        milkdrift_workspace::ValueKey::new("result")?,
        milkdrift_workspace::ValueVersion::FIRST,
    );
    let projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        runtime_eligible(
            3,
            &fixture,
            "producer",
            &execution,
            fixture.root.reference(),
        )?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::DeterministicOutputPublished {
                execution: execution.clone(),
                value: output.clone(),
                artifact: None,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::DeterministicNodeTerminal {
                execution: execution.clone(),
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::StructuredSuccessorScanCompleted {
                execution: execution.clone(),
            },
        )?,
    ])?;

    assert!(projection.node_executions().is_empty());
    let summary = projection
        .settled_node_executions()
        .get(&execution)
        .ok_or("settled output summary is absent")?;
    assert_eq!(summary.revision(), &fixture.revision);
    assert_eq!(summary.created_sequence(), RunSequence::new(3));
    assert_eq!(summary.outputs().len(), 1);
    assert_eq!(summary.outputs()[0].value(), &output);
    assert!(projection.workspace_values().contains(&output));
    Ok(())
}

#[test]
fn one_thousand_settled_retries_keep_one_current_attempt_and_a_stable_snapshot() -> TestResult {
    let fixture = fixture("bounded-retries")?;
    let execution = NodeExecutionId::new("execution-retries")?;
    let snapshot = resolved_snapshot_with_side_effect(
        1,
        SideEffectClass::None,
        IdempotencyBehavior::Unsupported,
    )?;
    let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
        .provider_profile(ProviderProfileRef::new("publisher-prod")?);
    let mut projection = RunProjection::new();
    let mut sequence = 1_u64;
    projection.apply_replayed(&created(&fixture, sequence)?)?;
    sequence += 1;
    projection.apply_replayed(&envelope(sequence, &fixture.run, RunEventKind::RunStarted)?)?;
    sequence += 1;
    projection.apply_replayed(&eligible(
        sequence,
        &fixture,
        "retry-task",
        &execution,
        fixture.root.reference(),
    )?)?;
    sequence += 1;

    let mut previous: Option<AttemptId> = None;
    let mut size_at_100 = 0_usize;
    for number in 1..=1_000_u32 {
        let attempt = AttemptId::new(format!("attempt-{number}"))?;
        let invocation = InvocationId::new(format!("invocation-{number}"))?;
        if let Some(prior) = previous.as_ref() {
            let timer = TimerId::new(format!("retry-timer-{number}"))?;
            projection.apply_replayed(&envelope(
                sequence,
                &fixture.run,
                RunEventKind::NodeRetryScheduled {
                    execution: execution.clone(),
                    previous_attempt: prior.clone(),
                    next_attempt: attempt.clone(),
                    attempt_number: number,
                    timer: timer.clone(),
                    fire_at: TimestampMillis::new(sequence.saturating_add(1).saturating_mul(100)),
                    error_class: ErrorClass::Transport,
                    reason: Reason::new("synthetic bounded retry")?,
                },
            )?)?;
            sequence += 1;
            projection.apply_replayed(&envelope(
                sequence,
                &fixture.run,
                RunEventKind::TimerFired {
                    timer,
                    observed_at: TimestampMillis::new(sequence.saturating_mul(100)),
                },
            )?)?;
            sequence += 1;
        }
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: milkdrift_blueprint::NodeId::new("retry-task")?,
                execution: execution.clone(),
                attempt: attempt.clone(),
                invocation: invocation.clone(),
                idempotency_key: None,
                request: invocation_request(&invocation, None)?,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::CapabilityResolved {
                execution: execution.clone(),
                attempt: attempt.clone(),
                requirement: requirement.clone(),
                snapshot: snapshot.clone(),
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::SideEffectClassified {
                attempt: attempt.clone(),
                side_effect: SideEffectClass::None,
                idempotency: IdempotencyBehavior::Unsupported,
                idempotency_key: None,
            },
        )?)?;
        sequence += 1;
        let lease = LeaseId::new(format!("lease-{number}"))?;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::LeaseGranted {
                lease,
                execution: execution.clone(),
                attempt: attempt.clone(),
                worker: WorkerId::new("worker")?,
                expires_at: TimestampMillis::new(sequence.saturating_add(10).saturating_mul(100)),
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::NodeStarted {
                execution: execution.clone(),
                attempt: attempt.clone(),
                invocation,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::NodeTerminal {
                execution: execution.clone(),
                attempt: attempt.clone(),
                report_sequence: 1,
                outcome: NodeOutcome::Failed,
                error_class: Some(ErrorClass::Transport),
                detail: None,
            },
        )?)?;
        sequence += 1;
        previous = Some(attempt);
        if number == 100 {
            size_at_100 = encode_projection_snapshot(&projection)?.len();
        }
    }

    let execution_view = &projection.node_executions()[&execution];
    assert_eq!(execution_view.attempt_count(), 1_000);
    assert_eq!(execution_view.attempts().len(), 1);
    assert_eq!(projection.attempts().len(), 1);
    assert!(projection.leases().is_empty());
    assert!(projection.timers().is_empty());
    assert!(projection.retries().is_empty());
    let final_size = encode_projection_snapshot(&projection)?.len();
    assert!(final_size < size_at_100.saturating_mul(2));
    assert!(final_size.abs_diff(size_at_100) < 1_024);
    Ok(())
}

#[test]
fn ten_thousand_repeat_iterations_retain_only_the_exact_latest_frontier() -> TestResult {
    let fixture = fixture("bounded-repeat")?;
    let repeat = NodeExecutionId::new("execution-repeat")?;
    let mut projection = RunProjection::new();
    projection.apply_replayed(&created(&fixture, 1)?)?;
    projection.apply_replayed(&envelope(2, &fixture.run, RunEventKind::RunStarted)?)?;
    projection.apply_replayed(&runtime_eligible(
        3,
        &fixture,
        "repeat",
        &repeat,
        fixture.root.reference(),
    )?)?;
    let mut sequence = 4_u64;
    let mut size_at_100 = 0_usize;
    for number in 1..=10_000_u32 {
        let iteration = IterationId::new(format!("iteration-{number}"))?;
        let scope = WorkspaceScope::iteration(
            ScopeId::new(format!("iteration-scope-{number}"))?,
            &fixture.root,
            iteration.clone(),
        )?;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: iteration.clone(),
                iteration_number: number,
                scope,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration,
                result: number != 10_000,
            },
        )?)?;
        sequence += 1;
        if number == 100 {
            size_at_100 = encode_projection_snapshot(&projection)?.len();
        }
    }

    assert_eq!(projection.iterations().len(), 1);
    assert_eq!(projection.scopes().len(), 2);
    let latest = projection
        .iterations()
        .values()
        .next()
        .ok_or("missing repeat frontier")?;
    assert_eq!(latest.iteration_number(), 10_000);
    assert_eq!(
        latest.state(),
        super::super::IterationState::ConditionRecorded(false)
    );
    let final_size = encode_projection_snapshot(&projection)?.len();
    assert!(final_size < size_at_100.saturating_mul(2));
    assert!(final_size.abs_diff(size_at_100) < 1_024);
    Ok(())
}

#[test]
fn settled_signals_timers_and_recovery_passes_do_not_accumulate() -> TestResult {
    let fixture = fixture("bounded-obligations")?;
    let mut projection = RunProjection::new();
    projection.apply_replayed(&created(&fixture, 1)?)?;
    projection.apply_replayed(&envelope(2, &fixture.run, RunEventKind::RunStarted)?)?;
    let mut sequence = 3_u64;
    for number in 1..=10_000_u32 {
        let timer = TimerId::new(format!("timer-{number}"))?;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::TimerRegistered {
                timer: timer.clone(),
                execution: None,
                fire_at: TimestampMillis::new(sequence.saturating_add(1).saturating_mul(100)),
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::TimerFired {
                timer,
                observed_at: TimestampMillis::new(sequence.saturating_mul(100)),
            },
        )?)?;
        sequence += 1;
        let signal = SignalId::new(format!("signal-{number}"))?;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::SignalReceived {
                signal: signal.clone(),
                signal_type: SignalTypeId::new("test.signal")?,
                correlation: None,
                mode: SignalDeliveryMode::Broadcast,
                payload: milkdrift_capability::BoundedJson::new(serde_json::json!({}))?,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::SignalBroadcastScanAdvanced {
                signal,
                through_execution: None,
                complete: true,
            },
        )?)?;
        sequence += 1;
    }
    for _ in 0..1_000 {
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RecoveryStarted {
                controller: WorkerId::new("controller")?,
                through_sequence: RunSequence::new(sequence - 1),
            },
        )?)?;
        sequence += 1;
    }
    assert!(projection.timers().is_empty());
    assert!(projection.signals().is_empty());
    assert_eq!(projection.recovery().len(), 1);
    assert!(projection.recovery()[0].classifications().is_empty());
    Ok(())
}

#[test]
fn ten_thousand_unmatched_signals_cannot_exceed_the_pending_budget() -> TestResult {
    let fixture = fixture("bounded-pending-signals")?;
    let mut events = vec![
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
    ];
    for number in 1..=10_000_u32 {
        events.push(envelope(
            u64::from(number).saturating_add(2),
            &fixture.run,
            RunEventKind::SignalReceived {
                signal: SignalId::new(format!("pending-signal-{number}"))?,
                signal_type: SignalTypeId::new("test.pending")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: milkdrift_capability::BoundedJson::new(serde_json::json!({}))?,
            },
        )?);
    }
    assert!(RunProjection::replay(&events).is_err());

    let accepted = super::super::MAX_PENDING_SIGNAL_COUNT;
    let projection = RunProjection::replay(&events[..accepted.saturating_add(2)])?;
    assert_eq!(projection.signals().len(), accepted);
    let payload_bytes = projection
        .signals()
        .values()
        .try_fold(0_usize, |total, signal| {
            serde_json::to_vec(signal.payload()).map(|payload| total.saturating_add(payload.len()))
        })?;
    assert!(payload_bytes <= super::super::MAX_PENDING_SIGNAL_PAYLOAD_BYTES);
    let snapshot_bytes = encode_projection_snapshot(&projection)?.len();
    eprintln!(
        "unmatched_signals_attempted=10000 retained={} retained_payload_bytes={payload_bytes} snapshot_bytes={snapshot_bytes}",
        projection.signals().len(),
    );
    Ok(())
}

#[test]
fn aggregate_pending_signal_payload_bytes_are_hard_bounded() -> TestResult {
    let fixture = fixture("bounded-pending-signal-bytes")?;
    let payload = milkdrift_capability::BoundedJson::new(serde_json::json!("x".repeat(32_768)))?;
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
    ])?;
    let mut accepted = 0_usize;
    for number in 1..=100_u64 {
        let event = envelope(
            number.saturating_add(2),
            &fixture.run,
            RunEventKind::SignalReceived {
                signal: SignalId::new(format!("large-pending-signal-{number}"))?,
                signal_type: SignalTypeId::new("test.pending.large")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: payload.clone(),
            },
        )?;
        if projection.apply(&event).is_err() {
            break;
        }
        accepted = accepted.saturating_add(1);
    }
    let one_payload_bytes = serde_json::to_vec(&payload)?.len();
    let retained_payload_bytes = accepted.saturating_mul(one_payload_bytes);
    assert!(accepted < super::super::MAX_PENDING_SIGNAL_COUNT);
    assert!(retained_payload_bytes <= super::super::MAX_PENDING_SIGNAL_PAYLOAD_BYTES);
    assert!(
        retained_payload_bytes.saturating_add(one_payload_bytes)
            > super::super::MAX_PENDING_SIGNAL_PAYLOAD_BYTES
    );
    Ok(())
}

#[test]
fn ten_thousand_pre_start_releases_retain_no_worker_history() -> TestResult {
    let fixture = fixture("bounded-pre-start-releases")?;
    let execution = NodeExecutionId::new("execution-pre-start-releases")?;
    let attempt = AttemptId::new("attempt-pre-start-releases")?;
    let invocation = InvocationId::new("invocation-pre-start-releases")?;
    let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
        .provider_profile(ProviderProfileRef::new("publisher-prod")?);
    let snapshot = resolved_snapshot_with_side_effect(
        1,
        SideEffectClass::None,
        IdempotencyBehavior::Unsupported,
    )?;
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        eligible(3, &fixture, "work", &execution, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("work")?,
                execution: execution.clone(),
                attempt: attempt.clone(),
                invocation: invocation.clone(),
                idempotency_key: None,
                request: invocation_request(&invocation, None)?,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::CapabilityResolved {
                execution: execution.clone(),
                attempt: attempt.clone(),
                requirement,
                snapshot,
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::SideEffectClassified {
                attempt: attempt.clone(),
                side_effect: SideEffectClass::None,
                idempotency: IdempotencyBehavior::Unsupported,
                idempotency_key: None,
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::LeaseGranted {
                lease: LeaseId::new("lease-0")?,
                execution: execution.clone(),
                attempt: attempt.clone(),
                worker: WorkerId::new("worker-0")?,
                expires_at: TimestampMillis::new(800),
            },
        )?,
    ])?;
    let mut sequence = 8_u64;
    let mut previous_lease = LeaseId::new("lease-0")?;
    let mut size_at_100 = 0_usize;

    let started = envelope(
        sequence,
        &fixture.run,
        RunEventKind::NodeStarted {
            execution: execution.clone(),
            attempt: attempt.clone(),
            invocation: invocation.clone(),
        },
    )?;
    let mut malformed_state = projection.clone();
    malformed_state
        .node_executions
        .get_mut(&execution)
        .ok_or("hostile start fixture execution is absent")?
        .state = super::super::NodeExecutionState::Eligible;
    assert!(malformed_state.apply(&started).is_err());

    let mut cancelling = projection.clone();
    cancelling.apply(&envelope(
        sequence,
        &fixture.run,
        RunEventKind::RunCancellationRequested {
            reason: Reason::new("cancel before worker start")?,
            evidence: Vec::new(),
        },
    )?)?;
    assert!(
        cancelling
            .apply(&envelope(
                sequence + 1,
                &fixture.run,
                RunEventKind::NodeStarted {
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                },
            )?)
            .is_err()
    );

    for number in 1..=10_000_u32 {
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::LeaseExpired {
                lease: previous_lease.clone(),
                classification: RecoveryClassification::NotStarted,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RecoveryStarted {
                controller: WorkerId::new("controller")?,
                through_sequence: RunSequence::new(sequence - 1),
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RecoveryClassified {
                attempt: attempt.clone(),
                lease: Some(previous_lease.clone()),
                classification: RecoveryClassification::NotStarted,
                reason: Reason::new("worker never crossed start")?,
            },
        )?)?;
        sequence += 1;
        let next_lease = LeaseId::new(format!("lease-{number}"))?;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::NodeReLeased {
                previous_lease,
                lease: next_lease.clone(),
                attempt: attempt.clone(),
                worker: WorkerId::new(format!("worker-{number}"))?,
                expires_at: TimestampMillis::new(sequence.saturating_add(1).saturating_mul(100)),
            },
        )?)?;
        sequence += 1;
        previous_lease = next_lease;
        assert!(projection.attempts()[&attempt].lease_workers().is_empty());
        assert!(projection.attempts()[&attempt].leases().len() <= 2);
        assert!(projection.leases().len() <= 2);
        if number == 100 {
            size_at_100 = encode_projection_snapshot(&projection)?.len();
        }
    }
    let final_size = encode_projection_snapshot(&projection)?.len();
    assert!(final_size < size_at_100.saturating_mul(2));
    assert!(final_size.abs_diff(size_at_100) < 1_024);
    eprintln!(
        "pre_start_releases=10000 retained_workers={} retained_attempt_leases={} retained_leases={} snapshot_at_100_bytes={size_at_100} snapshot_at_10000_bytes={final_size}",
        projection.attempts()[&attempt].lease_workers().len(),
        projection.attempts()[&attempt].leases().len(),
        projection.leases().len(),
    );
    Ok(())
}

#[test]
fn ten_thousand_revision_node_churn_drops_removed_root_summaries() -> TestResult {
    let fixture = fixture("bounded-revisions")?;
    let mut projection = RunProjection::new();
    projection.apply_replayed(&created(&fixture, 1)?)?;
    projection.apply_replayed(&envelope(2, &fixture.run, RunEventKind::RunStarted)?)?;
    let mut sequence = 3_u64;
    let mut current = fixture.revision.clone();
    let mut size_at_100 = 0_usize;
    for number in 1..=10_000_u32 {
        let node_name = format!("work-{number}");
        let execution = NodeExecutionId::new(format!("revision-pending-{number}"))?;
        projection.apply_replayed(&runtime_eligible(
            sequence,
            &fixture,
            &node_name,
            &execution,
            fixture.root.reference(),
        )?)?;
        sequence += 1;
        let next: milkdrift_blueprint::RevisionId =
            serde_json::from_str(&format!("\"rev_{number:064x}\""))?;
        let next_digest: milkdrift_blueprint::ContentDigest =
            serde_json::from_str(&format!("\"b3_{number:064x}\""))?;
        let reconciliation = ReconciliationId::new(format!("reconciliation-{number}"))?;
        let plan = ReconciliationPlanId::new(format!("plan-{number}"))?;
        let based_on = RunSequence::new(sequence - 1);
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RevisionAdoptionRequested {
                reconciliation: reconciliation.clone(),
                requested_by: Some(ActorRef::new("human:test-reconciliation")?),
                from_revision: current.clone(),
                to_revision: next.clone(),
                policy: ReconciliationPolicy::RemoveUnstartedOnly,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::ReconciliationPlanRecorded {
                reconciliation,
                plan: plan.clone(),
                from_revision: current.clone(),
                to_revision: next.clone(),
                based_on_sequence: based_on,
                items: vec![ReconciliationItem {
                    node: Some(NodeId::new(node_name)?),
                    execution: Some(execution.clone()),
                    classification: ReconciliationClassification::ChangedPending,
                    action: ReconciliationAction::UseNewOnNextInvocation,
                    reason: Reason::new("replace pending work prospectively")?,
                }],
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::ReconciliationExecutionRemoved {
                plan: plan.clone(),
                execution,
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::ReconciliationApplied {
                plan: plan.clone(),
                from_revision: current.clone(),
                to_revision: next.clone(),
                based_on_sequence: RunSequence::new(sequence - 1),
            },
        )?)?;
        sequence += 1;
        projection.apply_replayed(&envelope(
            sequence,
            &fixture.run,
            RunEventKind::RevisionPinned {
                previous: current,
                revision: next.clone(),
                revision_digest: next_digest,
                plan,
            },
        )?)?;
        sequence += 1;
        current = next;
        assert!(projection.node_executions().is_empty());
        assert!(projection.settled_node_executions().is_empty());
        if number == 100 {
            size_at_100 = encode_projection_snapshot(&projection)?.len();
        }
    }
    assert_eq!(projection.pins().len(), 1);
    assert_eq!(projection.reconciliation().requests().len(), 1);
    assert_eq!(projection.reconciliation().plans().len(), 1);
    assert!(projection.execution_ids_by_node.is_empty());
    assert!(projection.settled_execution_by_scope_node.is_empty());
    assert!(
        projection
            .latest_descendant_execution_by_scope_node
            .is_empty()
    );
    let final_size = encode_projection_snapshot(&projection)?.len();
    assert!(final_size < size_at_100.saturating_mul(2));
    assert!(final_size.abs_diff(size_at_100) < 1_024);
    eprintln!(
        "revision_node_churn=10000 settled_summaries={} node_indexes={} scope_node_indexes={} descendant_indexes={} snapshot_at_100_bytes={size_at_100} snapshot_at_10000_bytes={final_size}",
        projection.settled_node_executions().len(),
        projection.execution_ids_by_node.len(),
        projection.settled_execution_by_scope_node.len(),
        projection.latest_descendant_execution_by_scope_node.len(),
    );
    Ok(())
}
