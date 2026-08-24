use super::{
    AttemptId, CapabilityRequirement, ErrorClass, IdempotencyBehavior, InvocationId, LeaseId,
    NodeExecutionId, NodeId, NodeOutcome, OperationId, ProviderProfileRef, Reason,
    ReconciliationAction, ReconciliationClassification, ReconciliationId, ReconciliationItem,
    ReconciliationPlanId, ReconciliationPolicy, RunEventKind, RunProjection, RunSequence,
    SideEffectClass, SignalDeliveryMode, SignalId, SignalTypeId, TestResult, TimerId,
    TimestampMillis, WorkerId, WorkspaceScope, created, eligible, envelope, fixture,
    invocation_request, resolved_snapshot_with_side_effect, runtime_eligible,
};
use crate::query::encode_projection_snapshot;
use milkdrift_workspace::{IterationId, ScopeId};

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
fn one_thousand_settled_revision_plans_keep_only_the_current_summary() -> TestResult {
    let fixture = fixture("bounded-revisions")?;
    let mut projection = RunProjection::new();
    projection.apply_replayed(&created(&fixture, 1)?)?;
    projection.apply_replayed(&envelope(2, &fixture.run, RunEventKind::RunStarted)?)?;
    let mut sequence = 3_u64;
    let mut current = fixture.revision.clone();
    let mut size_at_100 = 0_usize;
    for number in 1..=1_000_u32 {
        let execution = NodeExecutionId::new(format!("revision-pending-{number}"))?;
        projection.apply_replayed(&runtime_eligible(
            sequence,
            &fixture,
            "work",
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
                    node: Some(NodeId::new("work")?),
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
        if number == 100 {
            size_at_100 = encode_projection_snapshot(&projection)?.len();
        }
    }
    assert_eq!(projection.pins().len(), 1);
    assert_eq!(projection.reconciliation().requests().len(), 1);
    assert_eq!(projection.reconciliation().plans().len(), 1);
    let final_size = encode_projection_snapshot(&projection)?.len();
    assert!(final_size < size_at_100.saturating_mul(2));
    assert!(final_size.abs_diff(size_at_100) < 1_024);
    Ok(())
}
