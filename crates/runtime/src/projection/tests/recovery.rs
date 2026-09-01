use super::{
    ActorRef, AttemptId, AttemptState, AuthorityDecision, CapabilityRequirement, EvidenceId,
    EvidenceKind, EvidenceReference, IdempotencyBehavior, IdempotencyKey, InvocationId, LeaseId,
    NodeAttemptProjection, NodeExecutionId, NodeExecutionMode, NodeExecutionState, NodeId,
    NodeOutcome, OperationId, ProviderProfileRef, Reason, ReconciliationAction,
    ReconciliationClassification, ReconciliationDecisionId, ReconciliationId, ReconciliationItem,
    ReconciliationPlanId, ReconciliationPolicy, RecoveryClassification,
    ResolvedCapabilitySnapshotDocument, RunEventKind, RunLifecycle, RunProjection, RunSequence,
    RuntimeError, SideEffectClass, TestResult, TimerId, TimestampMillis, WaitCondition, WorkerId,
    created, digest, eligible, envelope, fixture, invocation_request,
    resolved_snapshot_with_side_effect, revision, runtime_eligible,
};

#[test]
fn keeps_uncertain_retained_work_visible_through_cancellation_and_recovery() -> TestResult {
    let fixture = fixture("recovery")?;
    let execution = NodeExecutionId::new("execution-side-effect")?;
    let attempt = AttemptId::new("attempt-1")?;
    let invocation = InvocationId::new("invocation-1")?;
    let key = IdempotencyKey::new("idempotency-1")?;
    let lease = LeaseId::new("lease-1")?;
    let decision = ReconciliationDecisionId::new("decision-retain")?;
    let snapshot_document = ResolvedCapabilitySnapshotDocument::from_json(include_bytes!(
        "../../../../capability/tests/fixtures/resolved-capability-snapshot-v2.json"
    ))?;
    let snapshot = snapshot_document.body().clone();
    let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
        .provider_profile(ProviderProfileRef::new("publisher-prod")?);
    let events = vec![
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        eligible(
            3,
            &fixture,
            "side-effect",
            &execution,
            fixture.root.reference(),
        )?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("side-effect")?,
                execution: execution.clone(),
                attempt: attempt.clone(),
                invocation: invocation.clone(),
                idempotency_key: Some(key.clone()),
                request: invocation_request(&invocation, Some(key.clone()))?,
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
                side_effect: SideEffectClass::IdempotentWrite,
                idempotency: IdempotencyBehavior::ProviderProfileScoped,
                idempotency_key: Some(key),
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::LeaseGranted {
                lease: lease.clone(),
                execution: execution.clone(),
                attempt: attempt.clone(),
                worker: WorkerId::new("worker-1")?,
                expires_at: TimestampMillis::new(10_000),
            },
        )?,
        envelope(
            8,
            &fixture.run,
            RunEventKind::NodeStarted {
                execution,
                attempt: attempt.clone(),
                invocation,
            },
        )?,
        envelope(
            9,
            &fixture.run,
            RunEventKind::ExternalOutcomeUncertain {
                attempt: attempt.clone(),
                report_sequence: 1,
                side_effect: SideEffectClass::IdempotentWrite,
                reason: Reason::new("worker disconnected after dispatch")?,
                evidence: Vec::new(),
            },
        )?,
        envelope(
            10,
            &fixture.run,
            RunEventKind::RunCancellationRequested {
                reason: Reason::new("operator stopped the run")?,
                evidence: Vec::new(),
            },
        )?,
        envelope(
            11,
            &fixture.run,
            RunEventKind::RecoveryStarted {
                controller: WorkerId::new("recovery-controller")?,
                through_sequence: RunSequence::new(10),
            },
        )?,
        envelope(
            12,
            &fixture.run,
            RunEventKind::RecoveryClassified {
                attempt: attempt.clone(),
                lease: Some(lease),
                classification: RecoveryClassification::Uncertain,
                reason: Reason::new("external receipt is unavailable")?,
            },
        )?,
        envelope(
            13,
            &fixture.run,
            RunEventKind::RecoveryDecisionRecorded {
                attempt: attempt.clone(),
                decision: decision.clone(),
                actor: ActorRef::new("operator")?,
                outcome: AuthorityDecision::Retain,
                reason: Reason::new("retain for later investigation")?,
                evidence: Vec::new(),
            },
        )?,
        envelope(
            14,
            &fixture.run,
            RunEventKind::ExternalOutcomeRetained {
                attempt: attempt.clone(),
                decision,
                reason: Reason::new("investigation remains open")?,
            },
        )?,
    ];

    let projection = RunProjection::replay(&events)?;
    assert_eq!(projection.lifecycle(), RunLifecycle::Cancelling);
    assert_eq!(projection.unresolved_attempts().count(), 1);
    assert_eq!(
        projection
            .attempts()
            .get(&attempt)
            .map(NodeAttemptProjection::state),
        Some(&AttemptState::Retained)
    );
    Ok(())
}

#[test]
fn explicit_external_resolution_releases_the_execution_after_successor_closure() -> TestResult {
    let fixture = fixture("resolved-uncertainty-compaction")?;
    let execution = NodeExecutionId::new("execution-resolved")?;
    let attempt = AttemptId::new("attempt-resolved")?;
    let invocation = InvocationId::new("invocation-resolved")?;
    let snapshot = resolved_snapshot_with_side_effect(
        1,
        SideEffectClass::None,
        IdempotencyBehavior::Unsupported,
    )?;
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        eligible(
            3,
            &fixture,
            "resolved",
            &execution,
            fixture.root.reference(),
        )?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("resolved")?,
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
                requirement: CapabilityRequirement::new(OperationId::new("tool.publish")?)
                    .provider_profile(ProviderProfileRef::new("publisher-prod")?),
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
                lease: LeaseId::new("lease-resolved")?,
                execution: execution.clone(),
                attempt: attempt.clone(),
                worker: WorkerId::new("worker-resolved")?,
                expires_at: TimestampMillis::new(10_000),
            },
        )?,
        envelope(
            8,
            &fixture.run,
            RunEventKind::NodeStarted {
                execution: execution.clone(),
                attempt: attempt.clone(),
                invocation,
            },
        )?,
        envelope(
            9,
            &fixture.run,
            RunEventKind::ExternalOutcomeUncertain {
                attempt: attempt.clone(),
                report_sequence: 1,
                side_effect: SideEffectClass::None,
                reason: Reason::new("worker disconnected after dispatch")?,
                evidence: Vec::new(),
            },
        )?,
    ])?;
    assert_eq!(projection.unresolved_attempts().count(), 1);
    assert!(projection.node_executions().contains_key(&execution));
    assert!(projection.settled_node_executions().is_empty());

    projection.apply_replayed(&envelope(
        10,
        &fixture.run,
        RunEventKind::RecoveryDecisionRecorded {
            attempt: attempt.clone(),
            decision: ReconciliationDecisionId::new("decision-resolved")?,
            actor: ActorRef::new("operator")?,
            outcome: AuthorityDecision::ResolveSucceeded,
            reason: Reason::new("external receipt confirms success")?,
            evidence: vec![EvidenceReference {
                id: EvidenceId::new("receipt-resolved")?,
                kind: EvidenceKind::ExternalReceipt,
            }],
        },
    )?)?;
    assert_eq!(projection.unresolved_attempts().count(), 0);
    assert!(projection.node_executions().contains_key(&execution));
    assert!(
        projection
            .pending_successor_execution_ids()
            .contains(&execution)
    );

    projection.apply_replayed(&envelope(
        11,
        &fixture.run,
        RunEventKind::StructuredSuccessorScanCompleted {
            execution: execution.clone(),
        },
    )?)?;
    assert!(projection.node_executions().is_empty());
    assert!(projection.attempts().is_empty());
    assert_eq!(projection.settled_node_executions().len(), 1);
    assert_eq!(
        projection.settled_node_executions()[&execution].side_effect(),
        SideEffectClass::None
    );
    Ok(())
}

#[test]
fn recovery_query_preserves_obligation_and_remediation_creates_real_work() -> TestResult {
    let fixture = fixture("recovery-query-remediation")?;
    let source_execution = NodeExecutionId::new("execution-source")?;
    let source_attempt = AttemptId::new("attempt-source")?;
    let invocation = InvocationId::new("invocation-source")?;
    let key = IdempotencyKey::new("idempotency-source")?;
    let lease = LeaseId::new("lease-source")?;
    let query = ReconciliationDecisionId::new("decision-query")?;
    let compensate = ReconciliationDecisionId::new("decision-compensate")?;
    let remediation = NodeExecutionId::new("execution-remediation")?;
    let snapshot_document = ResolvedCapabilitySnapshotDocument::from_json(include_bytes!(
        "../../../../capability/tests/fixtures/resolved-capability-snapshot-v2.json"
    ))?;
    let snapshot = snapshot_document.body().clone();
    let requirement = CapabilityRequirement::new(OperationId::new("tool.publish")?)
        .provider_profile(ProviderProfileRef::new("publisher-prod")?);
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        eligible(
            3,
            &fixture,
            "source",
            &source_execution,
            fixture.root.reference(),
        )?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::NodeScheduled {
                node: NodeId::new("source")?,
                execution: source_execution.clone(),
                attempt: source_attempt.clone(),
                invocation: invocation.clone(),
                idempotency_key: Some(key.clone()),
                request: invocation_request(&invocation, Some(key.clone()))?,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::CapabilityResolved {
                execution: source_execution.clone(),
                attempt: source_attempt.clone(),
                requirement,
                snapshot,
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::SideEffectClassified {
                attempt: source_attempt.clone(),
                side_effect: SideEffectClass::IdempotentWrite,
                idempotency: IdempotencyBehavior::ProviderProfileScoped,
                idempotency_key: Some(key),
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::LeaseGranted {
                lease,
                execution: source_execution.clone(),
                attempt: source_attempt.clone(),
                worker: WorkerId::new("worker-source")?,
                expires_at: TimestampMillis::new(10_000),
            },
        )?,
        envelope(
            8,
            &fixture.run,
            RunEventKind::NodeStarted {
                execution: source_execution.clone(),
                attempt: source_attempt.clone(),
                invocation,
            },
        )?,
        envelope(
            9,
            &fixture.run,
            RunEventKind::ExternalOutcomeUncertain {
                attempt: source_attempt.clone(),
                report_sequence: 1,
                side_effect: SideEffectClass::IdempotentWrite,
                reason: Reason::new("external result needs investigation")?,
                evidence: Vec::new(),
            },
        )?,
        envelope(
            10,
            &fixture.run,
            RunEventKind::RecoveryDecisionRecorded {
                attempt: source_attempt.clone(),
                decision: query,
                actor: ActorRef::new("operator")?,
                outcome: AuthorityDecision::Query,
                reason: Reason::new("query status without resolving truth")?,
                evidence: Vec::new(),
            },
        )?,
    ])?;
    let source = &projection.attempts()[&source_attempt];
    assert_eq!(source.state(), &AttemptState::Uncertain);
    let obligation = source.obligation().ok_or("missing obligation")?;
    assert!(obligation.retained().is_none());
    assert_eq!(obligation.decisions().len(), 1);
    assert_eq!(
        obligation.decisions()[0].outcome(),
        AuthorityDecision::Query
    );

    projection.apply(&envelope(
        11,
        &fixture.run,
        RunEventKind::RecoveryDecisionRecorded {
            attempt: source_attempt.clone(),
            decision: compensate.clone(),
            actor: ActorRef::new("operator")?,
            outcome: AuthorityDecision::Compensate,
            reason: Reason::new("create explicit remediation")?,
            evidence: Vec::new(),
        },
    )?)?;
    projection.apply(&envelope(
        12,
        &fixture.run,
        RunEventKind::RemediationWorkCreated {
            source_attempt: source_attempt.clone(),
            execution: remediation.clone(),
            node: NodeId::new("remediation")?,
            scope: fixture.root.reference().clone(),
            mode: NodeExecutionMode::Runtime,
            decision: compensate,
            reason: Reason::new("runtime-owned remediation")?,
        },
    )?)?;
    assert_eq!(
        projection.node_executions()[&remediation].state(),
        &NodeExecutionState::Eligible
    );
    assert_eq!(
        projection.node_executions()[&remediation].mode(),
        NodeExecutionMode::Runtime
    );
    assert_eq!(
        projection.remediations()[&remediation].scope(),
        fixture.root.reference()
    );
    assert!(
        projection.attempts()[&source_attempt]
            .obligation()
            .is_some()
    );
    projection.apply(&envelope(
        13,
        &fixture.run,
        RunEventKind::DeterministicNodeTerminal {
            execution: remediation,
            outcome: NodeOutcome::Succeeded,
            error_class: None,
            detail: None,
        },
    )?)?;
    Ok(())
}

#[test]
fn rejects_stale_reconciliation_and_requires_immediate_exact_repin() -> TestResult {
    let fixture = fixture("reconciliation")?;
    let reconciliation = ReconciliationId::new("reconciliation-1")?;
    let plan = ReconciliationPlanId::new("plan-1")?;
    let next_revision = revision('b')?;
    let next_digest = digest('2')?;
    let mut projection = RunProjection::new();
    for event in [
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        envelope(
            3,
            &fixture.run,
            RunEventKind::RevisionAdoptionRequested {
                reconciliation: reconciliation.clone(),
                requested_by: Some(ActorRef::new("human:test-reconciliation")?),
                from_revision: fixture.revision.clone(),
                to_revision: next_revision.clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::ReconciliationPlanRecorded {
                reconciliation,
                plan: plan.clone(),
                from_revision: fixture.revision.clone(),
                to_revision: next_revision.clone(),
                based_on_sequence: RunSequence::new(2),
                items: vec![ReconciliationItem {
                    node: Some(NodeId::new("future-node")?),
                    execution: None,
                    classification: ReconciliationClassification::Added,
                    action: ReconciliationAction::UseNewOnNextInvocation,
                    reason: Reason::new("new prospective node")?,
                }],
            },
        )?,
    ] {
        projection.apply(&event)?;
    }

    let before = projection.clone();
    let stale = envelope(
        5,
        &fixture.run,
        RunEventKind::ReconciliationApplied {
            plan: plan.clone(),
            from_revision: fixture.revision.clone(),
            to_revision: next_revision.clone(),
            based_on_sequence: RunSequence::new(3),
        },
    )?;
    assert!(matches!(
        projection.apply(&stale),
        Err(RuntimeError::InvalidHistory(_))
    ));
    assert_eq!(projection, before);

    projection.apply(&envelope(
        5,
        &fixture.run,
        RunEventKind::ReconciliationApplied {
            plan: plan.clone(),
            from_revision: fixture.revision.clone(),
            to_revision: next_revision.clone(),
            based_on_sequence: RunSequence::new(4),
        },
    )?)?;
    projection.apply(&envelope(
        6,
        &fixture.run,
        RunEventKind::RevisionPinned {
            previous: fixture.revision,
            revision: next_revision.clone(),
            revision_digest: next_digest,
            plan,
        },
    )?)?;
    assert_eq!(projection.revision(), Some(&next_revision));
    assert_eq!(projection.sequence(), RunSequence::new(6));
    Ok(())
}

#[test]
fn reconciliation_removal_without_a_created_execution_is_an_enacted_noop() -> TestResult {
    let fixture = fixture("reconciliation-uncreated-removal")?;
    let reconciliation = ReconciliationId::new("reconciliation-remove")?;
    let plan = ReconciliationPlanId::new("plan-remove")?;
    let removed_node = NodeId::new("removed-before-eligibility")?;
    let next_revision = revision('b')?;
    let next_digest = digest('2')?;
    let projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        envelope(
            3,
            &fixture.run,
            RunEventKind::RevisionAdoptionRequested {
                reconciliation: reconciliation.clone(),
                requested_by: Some(ActorRef::new("human:test-reconciliation")?),
                from_revision: fixture.revision.clone(),
                to_revision: next_revision.clone(),
                policy: ReconciliationPolicy::RemoveUnstartedOnly,
            },
        )?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::ReconciliationPlanRecorded {
                reconciliation,
                plan: plan.clone(),
                from_revision: fixture.revision.clone(),
                to_revision: next_revision.clone(),
                based_on_sequence: RunSequence::new(2),
                items: vec![ReconciliationItem {
                    node: Some(removed_node),
                    execution: None,
                    classification: ReconciliationClassification::RemovedPending,
                    action: ReconciliationAction::RemoveUnstarted,
                    reason: Reason::new("node never became eligible")?,
                }],
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::ReconciliationApplied {
                plan: plan.clone(),
                from_revision: fixture.revision.clone(),
                to_revision: next_revision.clone(),
                based_on_sequence: RunSequence::new(4),
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::RevisionPinned {
                previous: fixture.revision,
                revision: next_revision.clone(),
                revision_digest: next_digest,
                plan,
            },
        )?,
    ])?;

    assert_eq!(projection.revision(), Some(&next_revision));
    assert!(projection.node_executions().is_empty());
    Ok(())
}

#[test]
fn reconciliation_removal_rejects_eligible_execution_with_live_wait_ownership() -> TestResult {
    let fixture = fixture("reconciliation-live-wait-removal")?;
    let execution = NodeExecutionId::new("execution-live-wait")?;
    let timer = TimerId::new("timer-live-wait")?;
    let reconciliation = ReconciliationId::new("reconciliation-live-wait")?;
    let plan = ReconciliationPlanId::new("plan-live-wait")?;
    let next_revision = revision('b')?;
    let mut projection = RunProjection::replay(&[
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
                condition: WaitCondition::Timer { timer },
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::RevisionAdoptionRequested {
                reconciliation: reconciliation.clone(),
                requested_by: Some(ActorRef::new("human:test-reconciliation")?),
                from_revision: fixture.revision.clone(),
                to_revision: next_revision.clone(),
                policy: ReconciliationPolicy::RemoveUnstartedOnly,
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::ReconciliationPlanRecorded {
                reconciliation,
                plan: plan.clone(),
                from_revision: fixture.revision.clone(),
                to_revision: next_revision,
                based_on_sequence: RunSequence::new(5),
                items: vec![ReconciliationItem {
                    node: Some(NodeId::new("wait")?),
                    execution: Some(execution.clone()),
                    classification: ReconciliationClassification::RemovedPending,
                    action: ReconciliationAction::RemoveUnstarted,
                    reason: Reason::new("maliciously treated a live wait as unstarted")?,
                }],
            },
        )?,
    ])?;
    assert!(projection.execution_has_active_structured_ownership(&execution));
    let before = projection.clone();
    assert!(
        projection
            .apply(&envelope(
                8,
                &fixture.run,
                RunEventKind::ReconciliationExecutionRemoved { plan, execution },
            )?)
            .is_err()
    );
    assert_eq!(projection, before);
    Ok(())
}

#[test]
fn reconciliation_cancellation_is_execution_cancellation_authority() -> TestResult {
    let fixture = fixture("reconciliation-cancellation")?;
    let execution = NodeExecutionId::new("execution-active")?;
    let attempt = AttemptId::new("attempt-active")?;
    let invocation = InvocationId::new("invocation-active")?;
    let reconciliation = ReconciliationId::new("reconciliation-cancel")?;
    let plan = ReconciliationPlanId::new("plan-cancel")?;
    let next_revision = revision('b')?;
    let next_digest = digest('2')?;
    let reason = Reason::new("cancel safely for prospective restart")?;
    let events = [
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
                request: invocation_request(&invocation, None)?,
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
                lease: LeaseId::new("lease-active")?,
                execution: execution.clone(),
                attempt: attempt.clone(),
                worker: WorkerId::new("worker-active")?,
                expires_at: TimestampMillis::new(10_000),
            },
        )?,
        envelope(
            8,
            &fixture.run,
            RunEventKind::RevisionAdoptionRequested {
                reconciliation: reconciliation.clone(),
                requested_by: Some(ActorRef::new("human:test-reconciliation")?),
                from_revision: fixture.revision.clone(),
                to_revision: next_revision.clone(),
                policy: ReconciliationPolicy::CancelAndRestartSafeWork,
            },
        )?,
        envelope(
            9,
            &fixture.run,
            RunEventKind::ReconciliationPlanRecorded {
                reconciliation,
                plan: plan.clone(),
                from_revision: fixture.revision.clone(),
                to_revision: next_revision.clone(),
                based_on_sequence: RunSequence::new(7),
                items: vec![ReconciliationItem {
                    node: Some(NodeId::new("task")?),
                    execution: Some(execution.clone()),
                    classification: ReconciliationClassification::ChangedActive,
                    action: ReconciliationAction::CancelAndRestart,
                    reason: reason.clone(),
                }],
            },
        )?,
        envelope(
            10,
            &fixture.run,
            RunEventKind::ReconciliationCancellationRequested {
                plan: plan.clone(),
                execution: execution.clone(),
                attempt: attempt.clone(),
                reason: reason.clone(),
            },
        )?,
    ];
    let mut unsafe_events = events.clone();
    unsafe_events[4] = envelope(
        5,
        &fixture.run,
        RunEventKind::CapabilityResolved {
            execution: execution.clone(),
            attempt: attempt.clone(),
            requirement: CapabilityRequirement::new(OperationId::new("tool.publish")?)
                .provider_profile(ProviderProfileRef::new("publisher-prod")?),
            snapshot: resolved_snapshot_with_side_effect(
                7,
                SideEffectClass::NonIdempotentWrite,
                IdempotencyBehavior::Unsupported,
            )?,
        },
    )?;
    unsafe_events[5] = envelope(
        6,
        &fixture.run,
        RunEventKind::SideEffectClassified {
            attempt: attempt.clone(),
            side_effect: SideEffectClass::NonIdempotentWrite,
            idempotency: IdempotencyBehavior::Unsupported,
            idempotency_key: None,
        },
    )?;
    assert!(
        RunProjection::replay(&unsafe_events).is_err(),
        "hostile cancel-and-restart plans must not authorize non-idempotent active work"
    );
    let mut projection = RunProjection::replay(&events)?;

    assert_eq!(
        projection
            .current_node_execution(&execution)
            .map(super::super::node::CurrentNodeExecution::state),
        Some(&NodeExecutionState::CancelledBeforeDispatch)
    );
    assert!(!projection.active_execution_ids().contains(&execution));
    assert!(!projection.active_attempt_ids().contains(&attempt));
    assert!(projection.active_lease_for_attempt(&attempt).is_none());

    projection.apply(&envelope(
        11,
        &fixture.run,
        RunEventKind::ReconciliationApplied {
            plan: plan.clone(),
            from_revision: fixture.revision.clone(),
            to_revision: next_revision.clone(),
            based_on_sequence: RunSequence::new(10),
        },
    )?)?;
    projection.apply(&envelope(
        12,
        &fixture.run,
        RunEventKind::RevisionPinned {
            previous: fixture.revision.clone(),
            revision: next_revision,
            revision_digest: next_digest,
            plan,
        },
    )?)?;

    assert!(
        projection
            .apply(&envelope(
                13,
                &fixture.run,
                RunEventKind::NodeTerminal {
                    execution,
                    attempt,
                    report_sequence: 1,
                    outcome: NodeOutcome::Cancelled,
                    error_class: None,
                    detail: None,
                },
            )?)
            .is_err(),
        "terminal evidence must not revive work cancelled before external start"
    );
    Ok(())
}
