use super::{
    AttemptId, BoundedDetail, BoundedJson, CapabilityId, CapabilityRequirement, ErrorClass,
    IdempotencyBehavior, IdempotencyKey, InvocationId, InvocationRequest, LeaseId,
    NodeExecutionCancellationProjection, NodeExecutionId, NodeExecutionMode,
    NodeExecutionProjection, NodeExecutionState, NodeId, NodeOutcome, OperationId,
    ProviderProfileRef, Reason, ResolvedCapabilitySnapshotDocument, RunEventKind, RunLifecycle,
    RunOutcome, RunProjection, RunSequence, RuntimeError, SideEffectClass, TestResult, TimerId,
    TimestampMillis, WaitCondition, WorkerId, WorkspaceValueReference, created, eligible, envelope,
    fixture, invocation_request, resolved_snapshot_at, resolved_snapshot_with_side_effect,
    runtime_eligible,
};

#[test]
fn replay_equals_incremental_apply() -> TestResult {
    let fixture = fixture("deterministic")?;
    let execution = NodeExecutionId::new("execution-entry")?;
    let events = vec![
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        envelope(
            3,
            &fixture.run,
            RunEventKind::NodeBecameEligible {
                node: NodeId::new("entry")?,
                execution,
                scope: fixture.root.reference().clone(),
                mode: NodeExecutionMode::Executor,
            },
        )?,
    ];

    let replayed = RunProjection::replay(&events)?;
    let mut incremental = RunProjection::new();
    for event in &events {
        incremental.apply(event)?;
    }
    assert_eq!(replayed, incremental);
    assert_eq!(replayed.sequence(), RunSequence::new(3));
    assert!(replayed.is_active());
    Ok(())
}

#[test]
fn cancelling_lifecycle_rejects_non_cancelled_run_terminal_facts() -> TestResult {
    let fixture = fixture("cancellation-terminal-guard")?;
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        envelope(
            3,
            &fixture.run,
            RunEventKind::RunCancellationRequested {
                reason: Reason::new("operator cancelled")?,
                evidence: Vec::new(),
            },
        )?,
    ])?;
    for outcome in [RunOutcome::Succeeded, RunOutcome::Failed] {
        let before = projection.clone();
        assert!(
            projection
                .apply(&envelope(
                    4,
                    &fixture.run,
                    RunEventKind::RunTerminal {
                        outcome,
                        outputs: Vec::new(),
                        artifacts: Vec::new(),
                        reason: None,
                    },
                )?)
                .is_err()
        );
        assert_eq!(projection, before);
    }
    projection.apply(&envelope(
        4,
        &fixture.run,
        RunEventKind::RunTerminal {
            outcome: RunOutcome::Cancelled,
            outputs: Vec::new(),
            artifacts: Vec::new(),
            reason: Some(Reason::new("operator cancelled")?),
        },
    )?)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Cancelled)
    );
    Ok(())
}

#[test]
fn rejects_gaps_wrong_runs_and_illegal_transitions_atomically() -> TestResult {
    let primary = fixture("invalid")?;
    let other = fixture("other")?;

    let gap = vec![
        created(&primary, 1)?,
        envelope(3, &primary.run, RunEventKind::RunStarted)?,
    ];
    assert!(matches!(
        RunProjection::replay(&gap),
        Err(RuntimeError::InvalidHistory(_))
    ));

    let wrong_run = vec![
        created(&primary, 1)?,
        envelope(2, &other.run, RunEventKind::RunStarted)?,
    ];
    assert!(matches!(
        RunProjection::replay(&wrong_run),
        Err(RuntimeError::InvalidHistory(_))
    ));

    let mut projection = RunProjection::replay(&[created(&primary, 1)?])?;
    let before = projection.clone();
    let illegal = envelope(
        2,
        &primary.run,
        RunEventKind::RunPaused {
            reason: Reason::new("not running")?,
            evidence: Vec::new(),
        },
    )?;
    assert!(matches!(
        projection.apply(&illegal),
        Err(RuntimeError::InvalidHistory(_))
    ));
    assert_eq!(projection, before);
    Ok(())
}

#[test]
fn projects_attempt_free_deterministic_terminal_and_rejects_fabricated_attempts() -> TestResult {
    let fixture = fixture("deterministic-terminal")?;
    let direct = NodeExecutionId::new("execution-direct")?;
    let scheduled = NodeExecutionId::new("execution-scheduled")?;
    let runtime_open = NodeExecutionId::new("execution-runtime-open")?;
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        runtime_eligible(3, &fixture, "direct", &direct, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::DeterministicNodeTerminal {
                execution: direct.clone(),
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?,
        eligible(
            5,
            &fixture,
            "scheduled",
            &scheduled,
            fixture.root.reference(),
        )?,
        runtime_eligible(
            6,
            &fixture,
            "runtime-open",
            &runtime_open,
            fixture.root.reference(),
        )?,
    ])?;

    let terminal = projection
        .node_executions()
        .get(&direct)
        .and_then(NodeExecutionProjection::deterministic_terminal)
        .ok_or("deterministic terminal missing")?;
    assert_eq!(terminal.outcome(), NodeOutcome::Succeeded);
    assert!(projection.node_executions()[&direct].attempts().is_empty());
    assert_eq!(
        projection.node_executions()[&scheduled].mode(),
        NodeExecutionMode::Executor
    );
    assert_eq!(
        projection.node_executions()[&runtime_open].mode(),
        NodeExecutionMode::Runtime
    );

    let before = projection.clone();
    assert!(
        projection
            .apply(&envelope(
                7,
                &fixture.run,
                RunEventKind::DeterministicNodeTerminal {
                    execution: scheduled.clone(),
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            )?)
            .is_err()
    );
    assert_eq!(projection, before);

    let value = WorkspaceValueReference::new(
        fixture.root.reference().clone(),
        milkdrift_workspace::ValueKey::new("deterministic-output")?,
        milkdrift_workspace::ValueVersion::FIRST,
    );
    assert!(
        projection
            .apply(&envelope(
                7,
                &fixture.run,
                RunEventKind::DeterministicOutputPublished {
                    execution: scheduled.clone(),
                    value,
                    artifact: None,
                },
            )?)
            .is_err()
    );
    assert!(
        projection
            .apply(&envelope(
                7,
                &fixture.run,
                RunEventKind::NodeScheduled {
                    node: NodeId::new("runtime-open")?,
                    execution: runtime_open,
                    attempt: AttemptId::new("attempt-runtime-open")?,
                    invocation: InvocationId::new("invocation-runtime-open")?,
                    idempotency_key: None,
                    request: invocation_request(
                        &InvocationId::new("invocation-runtime-open")?,
                        None,
                    )?,
                },
            )?)
            .is_err()
    );
    projection.apply(&envelope(
        7,
        &fixture.run,
        RunEventKind::NodeScheduled {
            node: NodeId::new("scheduled")?,
            execution: scheduled,
            attempt: AttemptId::new("attempt-scheduled")?,
            invocation: InvocationId::new("invocation-scheduled")?,
            idempotency_key: None,
            request: invocation_request(&InvocationId::new("invocation-scheduled")?, None)?,
        },
    )?)?;
    Ok(())
}

#[test]
fn persisted_invocation_request_must_match_frozen_capability_resolution() -> TestResult {
    let fixture = fixture("request-provenance")?;
    let execution = NodeExecutionId::new("execution-task")?;
    let attempt = AttemptId::new("attempt-task")?;
    let invocation = InvocationId::new("invocation-task")?;
    let request = InvocationRequest::new(
        invocation.clone(),
        CapabilityId::new("different-capability")?,
        OperationId::new("tool.publish")?,
        Some(ProviderProfileRef::new("publisher-prod")?),
        None,
        Vec::new(),
        std::collections::BTreeMap::new(),
    )?;
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        eligible(3, &fixture, "task", &execution, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("task")?,
                execution: execution.clone(),
                attempt: attempt.clone(),
                invocation: invocation.clone(),
                idempotency_key: None,
                request,
            },
        )?,
    ])?;
    assert_eq!(
        projection.attempts()[&attempt]
            .request()
            .map(InvocationRequest::invocation),
        Some(&invocation)
    );
    assert_eq!(
        projection.attempts()[&attempt].scheduled_sequence(),
        Some(RunSequence::new(4))
    );
    assert_eq!(
        projection.revision_for_attempt(&attempt),
        Some(&fixture.revision)
    );
    let snapshot_document = ResolvedCapabilitySnapshotDocument::from_json(include_bytes!(
        "../../../../capability/tests/fixtures/resolved-capability-snapshot-v1.json"
    ))?;
    let mismatch = envelope(
        5,
        &fixture.run,
        RunEventKind::CapabilityResolved {
            execution,
            attempt,
            requirement: CapabilityRequirement::new(OperationId::new("tool.publish")?)
                .provider_profile(ProviderProfileRef::new("publisher-prod")?),
            snapshot: snapshot_document.body().clone(),
        },
    )?;
    assert!(projection.apply(&mismatch).is_err());
    assert_eq!(projection.sequence(), RunSequence::new(4));
    Ok(())
}

#[test]
fn idempotent_retries_cannot_rotate_stable_keys_or_resolved_snapshots() -> TestResult {
    let fixture = fixture("retry-stable-dispatch")?;
    let execution = NodeExecutionId::new("execution-task")?;
    let first_attempt = AttemptId::new("attempt-1")?;
    let second_attempt = AttemptId::new("attempt-2")?;
    let first_invocation = InvocationId::new("invocation-1")?;
    let second_invocation = InvocationId::new("invocation-2")?;
    let stable_key = IdempotencyKey::new("stable-retry-key")?;
    let rotated_key = IdempotencyKey::new("rotated-retry-key")?;
    let snapshot = resolved_snapshot_at(7)?;
    let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
        .provider_profile(ProviderProfileRef::new("publisher-prod")?);
    let timer = TimerId::new("retry-timer")?;
    let projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        eligible(3, &fixture, "task", &execution, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("task")?,
                execution: execution.clone(),
                attempt: first_attempt.clone(),
                invocation: first_invocation.clone(),
                idempotency_key: Some(stable_key.clone()),
                request: invocation_request(&first_invocation, Some(stable_key.clone()))?,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::CapabilityResolved {
                execution: execution.clone(),
                attempt: first_attempt.clone(),
                requirement: requirement.clone(),
                snapshot: snapshot.clone(),
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::SideEffectClassified {
                attempt: first_attempt.clone(),
                side_effect: SideEffectClass::IdempotentWrite,
                idempotency: IdempotencyBehavior::ProviderProfileScoped,
                idempotency_key: Some(stable_key.clone()),
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::LeaseGranted {
                lease: LeaseId::new("lease-first")?,
                execution: execution.clone(),
                attempt: first_attempt.clone(),
                worker: WorkerId::new("worker-first")?,
                expires_at: TimestampMillis::new(10_000),
            },
        )?,
        envelope(
            8,
            &fixture.run,
            RunEventKind::NodeTerminal {
                execution: execution.clone(),
                attempt: first_attempt.clone(),
                report_sequence: 1,
                outcome: NodeOutcome::Failed,
                error_class: Some(ErrorClass::Provider),
                detail: Some(BoundedDetail::new("provider failure")?),
            },
        )?,
        envelope(
            9,
            &fixture.run,
            RunEventKind::NodeRetryScheduled {
                execution: execution.clone(),
                previous_attempt: first_attempt,
                next_attempt: second_attempt.clone(),
                attempt_number: 2,
                timer: timer.clone(),
                fire_at: TimestampMillis::new(900),
                error_class: ErrorClass::Provider,
                reason: Reason::new("retry idempotent provider failure")?,
            },
        )?,
        envelope(
            10,
            &fixture.run,
            RunEventKind::TimerFired {
                timer,
                observed_at: TimestampMillis::new(900),
            },
        )?,
    ])?;

    let mut mutated_request_projection = projection.clone();
    let mutated_request = InvocationRequest::new(
        second_invocation.clone(),
        CapabilityId::new("publisher-primary")?,
        OperationId::new("tool.publish")?,
        Some(ProviderProfileRef::new("publisher-prod")?),
        Some(stable_key.clone()),
        Vec::new(),
        std::collections::BTreeMap::from([(
            milkdrift_capability::ExtensionKey::new("org.milkdrift/retry-mutation")?,
            BoundedJson::new(serde_json::json!({"changed": true}))?,
        )]),
    )?;
    let mutated_request = envelope(
        11,
        &fixture.run,
        RunEventKind::NodeScheduled {
            node: NodeId::new("task")?,
            execution: execution.clone(),
            attempt: second_attempt.clone(),
            invocation: second_invocation.clone(),
            idempotency_key: Some(stable_key.clone()),
            request: mutated_request,
        },
    )?;
    assert!(mutated_request_projection.apply(&mutated_request).is_err());
    assert_eq!(mutated_request_projection.sequence(), RunSequence::new(10));

    let mut rotated_snapshot_projection = projection.clone();
    rotated_snapshot_projection.apply(&envelope(
        11,
        &fixture.run,
        RunEventKind::NodeScheduled {
            node: NodeId::new("task")?,
            execution: execution.clone(),
            attempt: second_attempt.clone(),
            invocation: second_invocation.clone(),
            idempotency_key: Some(stable_key.clone()),
            request: invocation_request(&second_invocation, Some(stable_key.clone()))?,
        },
    )?)?;
    let rotated_snapshot = envelope(
        12,
        &fixture.run,
        RunEventKind::CapabilityResolved {
            execution: execution.clone(),
            attempt: second_attempt.clone(),
            requirement: requirement.clone(),
            snapshot: resolved_snapshot_at(8)?,
        },
    )?;
    assert!(
        rotated_snapshot_projection
            .apply(&rotated_snapshot)
            .is_err()
    );

    let mut rotated_key_projection = projection;
    rotated_key_projection.apply(&envelope(
        11,
        &fixture.run,
        RunEventKind::NodeScheduled {
            node: NodeId::new("task")?,
            execution: execution.clone(),
            attempt: second_attempt.clone(),
            invocation: second_invocation.clone(),
            idempotency_key: Some(rotated_key.clone()),
            request: invocation_request(&second_invocation, Some(rotated_key.clone()))?,
        },
    )?)?;
    rotated_key_projection.apply(&envelope(
        12,
        &fixture.run,
        RunEventKind::CapabilityResolved {
            execution,
            attempt: second_attempt.clone(),
            requirement,
            snapshot,
        },
    )?)?;
    let rotated_classification = envelope(
        13,
        &fixture.run,
        RunEventKind::SideEffectClassified {
            attempt: second_attempt,
            side_effect: SideEffectClass::IdempotentWrite,
            idempotency: IdempotencyBehavior::ProviderProfileScoped,
            idempotency_key: Some(rotated_key),
        },
    )?;
    assert!(
        rotated_key_projection
            .apply(&rotated_classification)
            .is_err()
    );
    assert_eq!(rotated_key_projection.sequence(), RunSequence::new(12));
    Ok(())
}

#[test]
fn automatic_retries_require_safe_side_effects_and_the_exact_failure_class() -> TestResult {
    let safe_fixture = fixture("retry-error-class")?;
    let safe_execution = NodeExecutionId::new("execution-safe")?;
    let safe_attempt = AttemptId::new("attempt-safe")?;
    let safe_invocation = InvocationId::new("invocation-safe")?;
    let stable_key = IdempotencyKey::new("stable-safe-key")?;
    let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
        .provider_profile(ProviderProfileRef::new("publisher-prod")?);
    let mut safe_projection = RunProjection::replay(&[
        created(&safe_fixture, 1)?,
        envelope(2, &safe_fixture.run, RunEventKind::RunStarted)?,
        eligible(
            3,
            &safe_fixture,
            "safe",
            &safe_execution,
            safe_fixture.root.reference(),
        )?,
        envelope(
            4,
            &safe_fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("safe")?,
                execution: safe_execution.clone(),
                attempt: safe_attempt.clone(),
                invocation: safe_invocation.clone(),
                idempotency_key: Some(stable_key.clone()),
                request: invocation_request(&safe_invocation, Some(stable_key.clone()))?,
            },
        )?,
        envelope(
            5,
            &safe_fixture.run,
            RunEventKind::CapabilityResolved {
                execution: safe_execution.clone(),
                attempt: safe_attempt.clone(),
                requirement: requirement.clone(),
                snapshot: resolved_snapshot_at(7)?,
            },
        )?,
        envelope(
            6,
            &safe_fixture.run,
            RunEventKind::SideEffectClassified {
                attempt: safe_attempt.clone(),
                side_effect: SideEffectClass::IdempotentWrite,
                idempotency: IdempotencyBehavior::ProviderProfileScoped,
                idempotency_key: Some(stable_key),
            },
        )?,
        envelope(
            7,
            &safe_fixture.run,
            RunEventKind::LeaseGranted {
                lease: LeaseId::new("lease-safe")?,
                execution: safe_execution.clone(),
                attempt: safe_attempt.clone(),
                worker: WorkerId::new("worker-safe")?,
                expires_at: TimestampMillis::new(10_000),
            },
        )?,
        envelope(
            8,
            &safe_fixture.run,
            RunEventKind::NodeTerminal {
                execution: safe_execution.clone(),
                attempt: safe_attempt.clone(),
                report_sequence: 1,
                outcome: NodeOutcome::Failed,
                error_class: Some(ErrorClass::Provider),
                detail: None,
            },
        )?,
    ])?;
    let substituted_class = envelope(
        9,
        &safe_fixture.run,
        RunEventKind::NodeRetryScheduled {
            execution: safe_execution,
            previous_attempt: safe_attempt,
            next_attempt: AttemptId::new("attempt-safe-next")?,
            attempt_number: 2,
            timer: TimerId::new("timer-safe-next")?,
            fire_at: TimestampMillis::new(900),
            error_class: ErrorClass::Transport,
            reason: Reason::new("must not substitute the durable failure class")?,
        },
    )?;
    assert!(safe_projection.apply(&substituted_class).is_err());
    assert_eq!(safe_projection.sequence(), RunSequence::new(8));

    let unsafe_fixture = fixture("retry-unsafe-write")?;
    let unsafe_execution = NodeExecutionId::new("execution-unsafe")?;
    let unsafe_attempt = AttemptId::new("attempt-unsafe")?;
    let unsafe_invocation = InvocationId::new("invocation-unsafe")?;
    let unsafe_snapshot = resolved_snapshot_with_side_effect(
        8,
        SideEffectClass::NonIdempotentWrite,
        IdempotencyBehavior::Unsupported,
    )?;
    let mut unsafe_projection = RunProjection::replay(&[
        created(&unsafe_fixture, 1)?,
        envelope(2, &unsafe_fixture.run, RunEventKind::RunStarted)?,
        eligible(
            3,
            &unsafe_fixture,
            "unsafe",
            &unsafe_execution,
            unsafe_fixture.root.reference(),
        )?,
        envelope(
            4,
            &unsafe_fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("unsafe")?,
                execution: unsafe_execution.clone(),
                attempt: unsafe_attempt.clone(),
                invocation: unsafe_invocation.clone(),
                idempotency_key: None,
                request: invocation_request(&unsafe_invocation, None)?,
            },
        )?,
        envelope(
            5,
            &unsafe_fixture.run,
            RunEventKind::CapabilityResolved {
                execution: unsafe_execution.clone(),
                attempt: unsafe_attempt.clone(),
                requirement,
                snapshot: unsafe_snapshot,
            },
        )?,
        envelope(
            6,
            &unsafe_fixture.run,
            RunEventKind::SideEffectClassified {
                attempt: unsafe_attempt.clone(),
                side_effect: SideEffectClass::NonIdempotentWrite,
                idempotency: IdempotencyBehavior::Unsupported,
                idempotency_key: None,
            },
        )?,
        envelope(
            7,
            &unsafe_fixture.run,
            RunEventKind::LeaseGranted {
                lease: LeaseId::new("lease-unsafe")?,
                execution: unsafe_execution.clone(),
                attempt: unsafe_attempt.clone(),
                worker: WorkerId::new("worker-unsafe")?,
                expires_at: TimestampMillis::new(10_000),
            },
        )?,
        envelope(
            8,
            &unsafe_fixture.run,
            RunEventKind::NodeTerminal {
                execution: unsafe_execution.clone(),
                attempt: unsafe_attempt.clone(),
                report_sequence: 1,
                outcome: NodeOutcome::Failed,
                error_class: Some(ErrorClass::Provider),
                detail: None,
            },
        )?,
    ])?;
    let unsafe_retry = envelope(
        9,
        &unsafe_fixture.run,
        RunEventKind::NodeRetryScheduled {
            execution: unsafe_execution,
            previous_attempt: unsafe_attempt,
            next_attempt: AttemptId::new("attempt-unsafe-next")?,
            attempt_number: 2,
            timer: TimerId::new("timer-unsafe-next")?,
            fire_at: TimestampMillis::new(900),
            error_class: ErrorClass::Provider,
            reason: Reason::new("unsafe writes require recovery authority")?,
        },
    )?;
    assert!(unsafe_projection.apply(&unsafe_retry).is_err());
    assert_eq!(unsafe_projection.sequence(), RunSequence::new(8));
    Ok(())
}

#[test]
fn cancellation_facts_close_attempt_free_wait_and_timer_ownership() -> TestResult {
    let fixture = fixture("wait-cancel")?;
    let execution = NodeExecutionId::new("execution-wait")?;
    let timer = TimerId::new("timer-wait")?;
    let events = vec![
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        runtime_eligible(3, &fixture, "wait", &execution, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::TimerRegistered {
                timer: timer.clone(),
                execution: Some(execution.clone()),
                fire_at: TimestampMillis::new(10_000),
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::WaitRegistered {
                execution: execution.clone(),
                condition: WaitCondition::Timer {
                    timer: timer.clone(),
                },
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::RunCancellationRequested {
                reason: Reason::new("stop")?,
                evidence: Vec::new(),
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::WaitCancelled {
                execution: execution.clone(),
                reason: Reason::new("owner cancelled")?,
            },
        )?,
        envelope(
            8,
            &fixture.run,
            RunEventKind::TimerCancelled {
                timer: timer.clone(),
                reason: Reason::new("wait cancelled")?,
            },
        )?,
        envelope(
            9,
            &fixture.run,
            RunEventKind::NodeExecutionCancelledBeforeDispatch {
                execution: execution.clone(),
                reason: Reason::new("never dispatched")?,
            },
        )?,
        envelope(
            10,
            &fixture.run,
            RunEventKind::RunTerminal {
                outcome: RunOutcome::Cancelled,
                outputs: Vec::new(),
                artifacts: Vec::new(),
                reason: Some(Reason::new("cancelled cleanly")?),
            },
        )?,
    ];
    let projection = RunProjection::replay(&events)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Cancelled)
    );
    assert_eq!(
        projection
            .current_node_execution(&execution)
            .ok_or("cancelled execution summary is absent")?
            .state(),
        &NodeExecutionState::CancelledBeforeDispatch
    );
    assert!(!projection.waits().contains_key(&execution));
    assert!(!projection.timers().contains_key(&timer));
    Ok(())
}

#[test]
fn attempt_cancellation_targets_only_the_latest_active_attempt() -> TestResult {
    let fixture = fixture("attempt-cancel")?;
    let execution = NodeExecutionId::new("execution-task")?;
    let attempt = AttemptId::new("attempt-task")?;
    let events = vec![
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        eligible(3, &fixture, "task", &execution, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("task")?,
                execution: execution.clone(),
                attempt: attempt.clone(),
                invocation: InvocationId::new("invocation-task")?,
                idempotency_key: None,
                request: invocation_request(&InvocationId::new("invocation-task")?, None)?,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::RunCancellationRequested {
                reason: Reason::new("stop")?,
                evidence: Vec::new(),
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::NodeExecutionCancellationRequested {
                execution: execution.clone(),
                attempt: attempt.clone(),
                reason: Reason::new("cancel active attempt")?,
            },
        )?,
    ];
    let mut projection = RunProjection::replay(&events)?;
    assert_eq!(
        projection.node_executions()[&execution]
            .cancellation()
            .and_then(NodeExecutionCancellationProjection::attempt),
        Some(&attempt)
    );
    assert!(
        projection
            .apply(&envelope(
                7,
                &fixture.run,
                RunEventKind::NodeExecutionCancellationRequested {
                    execution,
                    attempt,
                    reason: Reason::new("duplicate")?,
                },
            )?)
            .is_err()
    );
    Ok(())
}

#[test]
fn executor_terminal_reports_must_be_contiguous_and_stop_at_terminal() -> TestResult {
    let fixture = fixture("report-sequence")?;
    let execution = NodeExecutionId::new("execution-task")?;
    let attempt = AttemptId::new("attempt-task")?;
    let dispatch_facts = [
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        eligible(3, &fixture, "task", &execution, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("task")?,
                execution: execution.clone(),
                attempt: attempt.clone(),
                invocation: InvocationId::new("invocation-task")?,
                idempotency_key: None,
                request: invocation_request(&InvocationId::new("invocation-task")?, None)?,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::CapabilityResolved {
                execution: execution.clone(),
                attempt: attempt.clone(),
                requirement: CapabilityRequirement::new(OperationId::new("tool.publish")?)
                    .provider_profile(ProviderProfileRef::new("publisher-prod")?),
                snapshot: resolved_snapshot_with_side_effect(
                    7,
                    SideEffectClass::None,
                    IdempotencyBehavior::Unsupported,
                )?,
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
                lease: LeaseId::new("lease-task")?,
                execution: execution.clone(),
                attempt: attempt.clone(),
                worker: WorkerId::new("worker-task")?,
                expires_at: TimestampMillis::new(10_000),
            },
        )?,
    ];
    let mut projection = RunProjection::replay(&dispatch_facts[..4])?;
    let bare_terminal = envelope(
        5,
        &fixture.run,
        RunEventKind::NodeTerminal {
            execution: execution.clone(),
            attempt: attempt.clone(),
            report_sequence: 1,
            outcome: NodeOutcome::Succeeded,
            error_class: None,
            detail: None,
        },
    )?;
    assert!(projection.apply(&bare_terminal).is_err());
    assert_eq!(projection.sequence(), RunSequence::new(4));
    for fact in &dispatch_facts[4..] {
        projection.apply(fact)?;
    }
    assert!(
        projection
            .apply(&envelope(
                8,
                &fixture.run,
                RunEventKind::NodeTerminal {
                    execution: execution.clone(),
                    attempt: attempt.clone(),
                    report_sequence: 2,
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            )?)
            .is_err()
    );
    projection.apply(&envelope(
        8,
        &fixture.run,
        RunEventKind::NodeTerminal {
            execution: execution.clone(),
            attempt: attempt.clone(),
            report_sequence: 1,
            outcome: NodeOutcome::Succeeded,
            error_class: None,
            detail: None,
        },
    )?)?;
    assert_eq!(
        projection.attempts()[&attempt].last_report_sequence(),
        Some(1)
    );
    assert!(
        projection
            .apply(&envelope(
                9,
                &fixture.run,
                RunEventKind::NodeTerminal {
                    execution,
                    attempt,
                    report_sequence: 2,
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            )?)
            .is_err()
    );
    Ok(())
}
