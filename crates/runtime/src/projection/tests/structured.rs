use super::{
    ActorRef, BoundedJson, BranchId, BranchResultReference, CommandId, CurrencyCode, IterationId,
    JoinRule, MAX_REPEAT_CONTINUATION_CYCLES, NodeExecutionId, NodeOutcome, PortId, Reason,
    RepeatContinuationCause, RepeatContinuationDecision, RepeatContinuationRequestProjection,
    RepeatDecisionId, RepeatTerminationReason, RunEventKind, RunId, RunOutcome, RunProjection,
    RunSequence, ScopeId, SignalDeliveryMode, SignalId, SignalTypeId, SubworkflowId,
    SubworkflowOwnership, SubworkflowProjection, TestResult, TimerId, TimestampMillis,
    WaitCondition, WaitProjection, WaitSatisfaction, WorkspaceScope, WorkspaceValueReference,
    created, envelope, fixture, revision, runtime_eligible,
};

#[test]
fn projects_structured_scopes_waits_signals_and_subworkflows() -> TestResult {
    let fixture = fixture("structured")?;
    let fork = NodeExecutionId::new("execution-fork")?;
    let child = NodeExecutionId::new("execution-branch-child")?;
    let join = NodeExecutionId::new("execution-join")?;
    let repeat = NodeExecutionId::new("execution-repeat")?;
    let wait_timer = NodeExecutionId::new("execution-wait-timer")?;
    let wait_signal = NodeExecutionId::new("execution-wait-signal")?;
    let parent = NodeExecutionId::new("execution-subworkflow")?;
    let branch = BranchId::new("branch-a")?;
    let branch_scope =
        WorkspaceScope::branch(ScopeId::new("branch-scope")?, &fixture.root, branch.clone())?;
    let branch_output = WorkspaceValueReference::new(
        branch_scope.reference().clone(),
        milkdrift_workspace::ValueKey::new("result")?,
        milkdrift_workspace::ValueVersion::FIRST,
    );
    let iteration = IterationId::new("iteration-1")?;
    let iteration_scope = WorkspaceScope::iteration(
        ScopeId::new("iteration-scope")?,
        &fixture.root,
        iteration.clone(),
    )?;
    let timer = TimerId::new("timer-wait")?;
    let signal = SignalId::new("signal-ready")?;
    let signal_type = SignalTypeId::new("example.ready")?;
    let subworkflow = SubworkflowId::new("subworkflow-child")?;
    let child_scope = WorkspaceScope::subworkflow(
        ScopeId::new("subworkflow-scope")?,
        &fixture.root,
        subworkflow.clone(),
    )?;
    let child_run = RunId::new("run-structured-child")?;
    let events = vec![
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        runtime_eligible(3, &fixture, "fork", &fork, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::BranchScopeCreated {
                fork_execution: fork,
                port: PortId::new("branch-a")?,
                branch: branch.clone(),
                scope: branch_scope.clone(),
            },
        )?,
        runtime_eligible(
            5,
            &fixture,
            "branch-child",
            &child,
            branch_scope.reference(),
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::BranchChildAdded {
                branch: branch.clone(),
                execution: child,
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::DeterministicOutputPublished {
                execution: NodeExecutionId::new("execution-branch-child")?,
                value: branch_output,
                artifact: None,
            },
        )?,
        envelope(
            8,
            &fixture.run,
            RunEventKind::DeterministicNodeTerminal {
                execution: NodeExecutionId::new("execution-branch-child")?,
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?,
        envelope(
            9,
            &fixture.run,
            RunEventKind::BranchTerminal {
                branch: branch.clone(),
                outcome: RunOutcome::Succeeded,
                outputs: Vec::new(),
            },
        )?,
        runtime_eligible(10, &fixture, "join", &join, fixture.root.reference())?,
        envelope(
            11,
            &fixture.run,
            RunEventKind::JoinSatisfied {
                execution: join,
                rule: JoinRule::All,
                branches: vec![BranchResultReference {
                    branch: branch.clone(),
                    scope: branch_scope.reference().clone(),
                    outcome: RunOutcome::Succeeded,
                    outputs: Vec::new(),
                }],
                retained_branches: Vec::new(),
            },
        )?,
        runtime_eligible(12, &fixture, "repeat", &repeat, fixture.root.reference())?,
        envelope(
            13,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: iteration.clone(),
                iteration_number: 1,
                scope: iteration_scope,
            },
        )?,
        envelope(
            14,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: iteration.clone(),
                result: false,
            },
        )?,
        envelope(
            15,
            &fixture.run,
            RunEventKind::RepeatTerminated {
                repeat_execution: repeat,
                termination: RepeatTerminationReason::ConditionFalse,
                last_iteration: Some(iteration),
            },
        )?,
        runtime_eligible(
            16,
            &fixture,
            "wait-timer",
            &wait_timer,
            fixture.root.reference(),
        )?,
        envelope(
            17,
            &fixture.run,
            RunEventKind::TimerRegistered {
                timer: timer.clone(),
                execution: Some(wait_timer.clone()),
                fire_at: TimestampMillis::new(1_800),
            },
        )?,
        envelope(
            18,
            &fixture.run,
            RunEventKind::WaitRegistered {
                execution: wait_timer.clone(),
                condition: WaitCondition::Timer {
                    timer: timer.clone(),
                },
            },
        )?,
        envelope(
            19,
            &fixture.run,
            RunEventKind::TimerFired {
                timer: timer.clone(),
                observed_at: TimestampMillis::new(1_900),
            },
        )?,
        envelope(
            20,
            &fixture.run,
            RunEventKind::WaitSatisfied {
                execution: wait_timer,
                cause: WaitSatisfaction::Timer { timer },
            },
        )?,
        runtime_eligible(
            21,
            &fixture,
            "wait-signal",
            &wait_signal,
            fixture.root.reference(),
        )?,
        envelope(
            22,
            &fixture.run,
            RunEventKind::WaitRegistered {
                execution: wait_signal.clone(),
                condition: WaitCondition::Signal {
                    signal_type: signal_type.clone(),
                    correlation: None,
                },
            },
        )?,
        envelope(
            23,
            &fixture.run,
            RunEventKind::SignalReceived {
                signal: signal.clone(),
                signal_type,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(serde_json::json!({"ready": true}))?,
            },
        )?,
        envelope(
            24,
            &fixture.run,
            RunEventKind::SignalConsumed {
                signal: signal.clone(),
                execution: wait_signal.clone(),
            },
        )?,
        envelope(
            25,
            &fixture.run,
            RunEventKind::WaitSatisfied {
                execution: wait_signal,
                cause: WaitSatisfaction::Signal { signal },
            },
        )?,
        runtime_eligible(
            26,
            &fixture,
            "subworkflow",
            &parent,
            fixture.root.reference(),
        )?,
        envelope(
            27,
            &fixture.run,
            RunEventKind::SubworkflowCreated {
                subworkflow: subworkflow.clone(),
                parent_execution: parent,
                child_run: child_run.clone(),
                child_revision: revision('b')?,
                scope: child_scope,
                ownership: SubworkflowOwnership::Attached,
                inputs: Vec::new(),
            },
        )?,
        envelope(
            28,
            &fixture.run,
            RunEventKind::SubworkflowTerminal {
                subworkflow,
                child_run,
                outcome: RunOutcome::Succeeded,
                outputs: Vec::new(),
            },
        )?,
    ];

    let projection = RunProjection::replay(&events)?;
    assert_eq!(projection.branches().len(), 1);
    assert_eq!(projection.iterations().len(), 1);
    assert_eq!(projection.waits().len(), 2);
    assert!(
        projection
            .waits()
            .values()
            .all(WaitProjection::is_completed)
    );
    assert!(
        projection
            .subworkflows()
            .values()
            .all(SubworkflowProjection::is_completed)
    );
    Ok(())
}

#[test]
fn signals_support_queued_one_shot_and_preexisting_broadcast_waiters() -> TestResult {
    let fixture = fixture("signal-delivery")?;
    let signal_type = SignalTypeId::new("example.ready")?;
    let queued_signal = SignalId::new("signal-queued")?;
    let queued_wait = NodeExecutionId::new("execution-queued-wait")?;
    let broadcast_signal = SignalId::new("signal-broadcast")?;
    let broadcast_wait = NodeExecutionId::new("execution-broadcast-wait")?;
    let late_wait = NodeExecutionId::new("execution-late-wait")?;
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        envelope(
            3,
            &fixture.run,
            RunEventKind::SignalReceived {
                signal: queued_signal.clone(),
                signal_type: signal_type.clone(),
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(serde_json::json!({"queued": true}))?,
            },
        )?,
        runtime_eligible(
            4,
            &fixture,
            "queued-wait",
            &queued_wait,
            fixture.root.reference(),
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::WaitRegistered {
                execution: queued_wait.clone(),
                condition: WaitCondition::Signal {
                    signal_type: signal_type.clone(),
                    correlation: None,
                },
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::SignalConsumed {
                signal: queued_signal.clone(),
                execution: queued_wait.clone(),
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::WaitSatisfied {
                execution: queued_wait.clone(),
                cause: WaitSatisfaction::Signal {
                    signal: queued_signal.clone(),
                },
            },
        )?,
        runtime_eligible(
            8,
            &fixture,
            "broadcast-wait",
            &broadcast_wait,
            fixture.root.reference(),
        )?,
        envelope(
            9,
            &fixture.run,
            RunEventKind::WaitRegistered {
                execution: broadcast_wait.clone(),
                condition: WaitCondition::Signal {
                    signal_type: signal_type.clone(),
                    correlation: None,
                },
            },
        )?,
        envelope(
            10,
            &fixture.run,
            RunEventKind::SignalReceived {
                signal: broadcast_signal.clone(),
                signal_type: signal_type.clone(),
                correlation: None,
                mode: SignalDeliveryMode::Broadcast,
                payload: BoundedJson::new(serde_json::json!({"broadcast": true}))?,
            },
        )?,
    ])?;
    assert!(projection.signals()[&queued_signal].is_completed());
    assert!(projection.waits()[&queued_wait].is_completed());
    assert!(projection.signals()[&broadcast_signal].is_completed());
    assert!(!projection.signals()[&broadcast_signal].is_pending());
    assert!(
        projection.signals()[&broadcast_signal]
            .consumed_by()
            .is_empty()
    );

    projection.apply(&envelope(
        11,
        &fixture.run,
        RunEventKind::SignalConsumed {
            signal: broadcast_signal.clone(),
            execution: broadcast_wait.clone(),
        },
    )?)?;
    projection.apply(&envelope(
        12,
        &fixture.run,
        RunEventKind::WaitSatisfied {
            execution: broadcast_wait,
            cause: WaitSatisfaction::Signal {
                signal: broadcast_signal.clone(),
            },
        },
    )?)?;
    projection.apply(&runtime_eligible(
        13,
        &fixture,
        "late-wait",
        &late_wait,
        fixture.root.reference(),
    )?)?;
    projection.apply(&envelope(
        14,
        &fixture.run,
        RunEventKind::WaitRegistered {
            execution: late_wait.clone(),
            condition: WaitCondition::Signal {
                signal_type,
                correlation: None,
            },
        },
    )?)?;
    let late_broadcast_consumption = envelope(
        15,
        &fixture.run,
        RunEventKind::SignalConsumed {
            signal: broadcast_signal,
            execution: late_wait,
        },
    )?;
    assert!(projection.apply(&late_broadcast_consumption).is_err());
    assert_eq!(projection.sequence(), RunSequence::new(14));
    Ok(())
}

#[test]
fn signal_deduplication_command_identity_cannot_name_two_signals() -> TestResult {
    let fixture = fixture("signal-dedup-command")?;
    let first = SignalId::new("signal-first")?;
    let second = SignalId::new("signal-second")?;
    let duplicate_command = CommandId::new("command-duplicate-delivery")?;
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        envelope(
            3,
            &fixture.run,
            RunEventKind::SignalReceived {
                signal: first.clone(),
                signal_type: SignalTypeId::new("example.first")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(serde_json::json!({"value": 1}))?,
            },
        )?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::SignalReceived {
                signal: second.clone(),
                signal_type: SignalTypeId::new("example.second")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(serde_json::json!({"value": 2}))?,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::SignalDeduplicated {
                signal: first,
                duplicate_command: duplicate_command.clone(),
            },
        )?,
    ])?;

    let before = projection.clone();
    let contradictory = envelope(
        6,
        &fixture.run,
        RunEventKind::SignalDeduplicated {
            signal: second,
            duplicate_command,
        },
    )?;
    assert!(projection.apply(&contradictory).is_err());
    assert_eq!(projection, before);
    Ok(())
}

#[test]
fn repeat_continuation_decisions_are_bounded_and_preserve_authority_history() -> TestResult {
    let fixture = fixture("repeat-approval")?;
    let repeat = NodeExecutionId::new("execution-repeat")?;
    let first = IterationId::new("iteration-1")?;
    let second = IterationId::new("iteration-2")?;
    let third = IterationId::new("iteration-3")?;
    let iteration_scope = |number: u32, iteration: &IterationId| {
        WorkspaceScope::iteration(
            ScopeId::new(format!("iteration-scope-{number}"))?,
            &fixture.root,
            iteration.clone(),
        )
    };
    let events = vec![
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: first.clone(),
                iteration_number: 1,
                scope: iteration_scope(1, &first)?,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: first.clone(),
                result: true,
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: repeat.clone(),
                frontier_iteration: first.clone(),
                initial_iteration_limit: 1,
                effective_iteration_limit: 1,
                cause: RepeatContinuationCause::IterationLimit,
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::RepeatContinuationDecided {
                repeat_execution: repeat.clone(),
                decision: RepeatDecisionId::new("decision-approve")?,
                actor: ActorRef::new("operator")?,
                outcome: RepeatContinuationDecision::Approved,
                approved_additional_iterations: Some(2),
                reason: Reason::new("allow two more")?,
                evidence: Vec::new(),
            },
        )?,
        envelope(
            8,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: second.clone(),
                iteration_number: 2,
                scope: iteration_scope(2, &second)?,
            },
        )?,
        envelope(
            9,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: second,
                result: true,
            },
        )?,
        envelope(
            10,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: third.clone(),
                iteration_number: 3,
                scope: iteration_scope(3, &third)?,
            },
        )?,
        envelope(
            11,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: third.clone(),
                result: true,
            },
        )?,
        envelope(
            12,
            &fixture.run,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: repeat.clone(),
                frontier_iteration: third.clone(),
                initial_iteration_limit: 1,
                effective_iteration_limit: 3,
                cause: RepeatContinuationCause::IterationLimit,
            },
        )?,
        envelope(
            13,
            &fixture.run,
            RunEventKind::RepeatContinuationDecided {
                repeat_execution: repeat.clone(),
                decision: RepeatDecisionId::new("decision-reject")?,
                actor: ActorRef::new("operator")?,
                outcome: RepeatContinuationDecision::Rejected,
                approved_additional_iterations: None,
                reason: Reason::new("stop at boundary")?,
                evidence: Vec::new(),
            },
        )?,
        envelope(
            14,
            &fixture.run,
            RunEventKind::RepeatTerminated {
                repeat_execution: repeat.clone(),
                termination: RepeatTerminationReason::MaximumIterations,
                last_iteration: Some(third),
            },
        )?,
    ];
    let pending = RunProjection::replay(&events[..6])?;
    let pending_continuation = &pending.repeat_continuations()[&repeat];
    assert!(pending_continuation.is_pending_approval());
    assert_eq!(
        pending_continuation
            .pending_request()
            .map(RepeatContinuationRequestProjection::frontier_iteration),
        Some(&first)
    );
    let projection = RunProjection::replay(&events)?;
    let continuation = &projection.repeat_continuations()[&repeat];
    assert_eq!(continuation.initial_iteration_limit(), 1);
    assert_eq!(continuation.effective_iteration_limit(), 3);
    assert!(continuation.is_rejected());
    assert!(!continuation.is_pending_approval());
    assert_eq!(continuation.requests().len(), 2);
    assert_eq!(continuation.decisions().len(), 2);
    Ok(())
}

#[test]
fn repeat_continuation_requires_an_exact_pending_request() -> TestResult {
    let fixture = fixture("repeat-request")?;
    let repeat = NodeExecutionId::new("execution-repeat")?;
    let frontier = IterationId::new("iteration-frontier")?;
    let scope = WorkspaceScope::iteration(
        ScopeId::new("iteration-frontier-scope")?,
        &fixture.root,
        frontier.clone(),
    )?;
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: frontier.clone(),
                iteration_number: 1,
                scope,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: frontier.clone(),
                result: true,
            },
        )?,
    ])?;
    assert!(projection.repeat_continuations().is_empty());

    let decision_without_request = envelope(
        6,
        &fixture.run,
        RunEventKind::RepeatContinuationDecided {
            repeat_execution: repeat.clone(),
            decision: RepeatDecisionId::new("decision-without-request")?,
            actor: ActorRef::new("operator")?,
            outcome: RepeatContinuationDecision::Approved,
            approved_additional_iterations: Some(1),
            reason: Reason::new("cannot authorize an implicit boundary")?,
            evidence: Vec::new(),
        },
    )?;
    assert!(projection.apply(&decision_without_request).is_err());
    assert_eq!(projection.sequence(), RunSequence::new(5));

    let wrong_limit = envelope(
        6,
        &fixture.run,
        RunEventKind::RepeatContinuationRequested {
            repeat_execution: repeat.clone(),
            frontier_iteration: frontier.clone(),
            initial_iteration_limit: 2,
            effective_iteration_limit: 2,
            cause: RepeatContinuationCause::IterationLimit,
        },
    )?;
    assert!(projection.apply(&wrong_limit).is_err());

    let request = envelope(
        6,
        &fixture.run,
        RunEventKind::RepeatContinuationRequested {
            repeat_execution: repeat.clone(),
            frontier_iteration: frontier,
            initial_iteration_limit: 1,
            effective_iteration_limit: 1,
            cause: RepeatContinuationCause::IterationLimit,
        },
    )?;
    projection.apply(&request)?;
    let continuation = &projection.repeat_continuations()[&repeat];
    assert!(continuation.is_pending_approval());
    assert_eq!(continuation.requests().len(), 1);

    let duplicate = envelope(
        7,
        &fixture.run,
        RunEventKind::RepeatContinuationRequested {
            repeat_execution: repeat,
            frontier_iteration: IterationId::new("iteration-frontier")?,
            initial_iteration_limit: 1,
            effective_iteration_limit: 1,
            cause: RepeatContinuationCause::IterationLimit,
        },
    )?;
    assert!(projection.apply(&duplicate).is_err());
    assert_eq!(projection.sequence(), RunSequence::new(6));
    Ok(())
}

#[test]
fn repeat_budget_rejection_preserves_its_currency_specific_cause() -> TestResult {
    let fixture = fixture("repeat-budget-request")?;
    let repeat = NodeExecutionId::new("execution-repeat")?;
    let frontier = IterationId::new("iteration-frontier")?;
    let scope = WorkspaceScope::iteration(
        ScopeId::new("iteration-frontier-scope")?,
        &fixture.root,
        frontier.clone(),
    )?;
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: frontier.clone(),
                iteration_number: 1,
                scope,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: frontier.clone(),
                result: true,
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: repeat.clone(),
                frontier_iteration: frontier.clone(),
                initial_iteration_limit: 10,
                effective_iteration_limit: 10,
                cause: RepeatContinuationCause::CostBudget {
                    maximum_micros: 100,
                    observed_micros: 125,
                    currency: CurrencyCode::new("EUR")?,
                },
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::RepeatContinuationDecided {
                repeat_execution: repeat.clone(),
                decision: RepeatDecisionId::new("decision-reject-budget")?,
                actor: ActorRef::new("operator")?,
                outcome: RepeatContinuationDecision::Rejected,
                approved_additional_iterations: None,
                reason: Reason::new("budget remains binding")?,
                evidence: Vec::new(),
            },
        )?,
    ])?;
    let request = projection.repeat_continuations()[&repeat]
        .requests()
        .last()
        .ok_or("missing request")?;
    assert_eq!(
        request.cause(),
        &RepeatContinuationCause::CostBudget {
            maximum_micros: 100,
            observed_micros: 125,
            currency: CurrencyCode::new("EUR")?,
        }
    );
    let wrong_termination = envelope(
        8,
        &fixture.run,
        RunEventKind::RepeatTerminated {
            repeat_execution: repeat.clone(),
            termination: RepeatTerminationReason::MaximumIterations,
            last_iteration: Some(frontier.clone()),
        },
    )?;
    assert!(projection.apply(&wrong_termination).is_err());
    projection.apply(&envelope(
        8,
        &fixture.run,
        RunEventKind::RepeatTerminated {
            repeat_execution: repeat,
            termination: RepeatTerminationReason::BudgetExhausted,
            last_iteration: Some(frontier),
        },
    )?)?;
    Ok(())
}

#[test]
fn repeat_budget_approval_has_a_frontier_local_override_cap() -> TestResult {
    let fixture = fixture("repeat-budget-override")?;
    let repeat = NodeExecutionId::new("execution-repeat")?;
    let first = IterationId::new("iteration-1")?;
    let second = IterationId::new("iteration-2")?;
    let third = IterationId::new("iteration-3")?;
    let iteration_scope = |number: u32, iteration: &IterationId| {
        WorkspaceScope::iteration(
            ScopeId::new(format!("iteration-scope-{number}"))?,
            &fixture.root,
            iteration.clone(),
        )
    };
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: first.clone(),
                iteration_number: 1,
                scope: iteration_scope(1, &first)?,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: first,
                result: true,
            },
        )?,
        envelope(
            6,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: second.clone(),
                iteration_number: 2,
                scope: iteration_scope(2, &second)?,
            },
        )?,
        envelope(
            7,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: second.clone(),
                result: true,
            },
        )?,
        envelope(
            8,
            &fixture.run,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: repeat.clone(),
                frontier_iteration: second,
                initial_iteration_limit: 10,
                effective_iteration_limit: 10,
                cause: RepeatContinuationCause::DurationBudget {
                    maximum_ms: 100,
                    observed_ms: 125,
                },
            },
        )?,
        envelope(
            9,
            &fixture.run,
            RunEventKind::RepeatContinuationDecided {
                repeat_execution: repeat.clone(),
                decision: RepeatDecisionId::new("decision-approve-budget")?,
                actor: ActorRef::new("operator")?,
                outcome: RepeatContinuationDecision::Approved,
                approved_additional_iterations: Some(1),
                reason: Reason::new("authorize exactly one post-budget iteration")?,
                evidence: Vec::new(),
            },
        )?,
    ])?;
    let continuation = &projection.repeat_continuations()[&repeat];
    assert_eq!(continuation.effective_iteration_limit(), 11);
    assert_eq!(continuation.budget_override_iteration_limit(), Some(3));

    projection.apply(&envelope(
        10,
        &fixture.run,
        RunEventKind::RepeatIterationCreated {
            repeat_execution: repeat.clone(),
            iteration: third.clone(),
            iteration_number: 3,
            scope: iteration_scope(3, &third)?,
        },
    )?)?;
    projection.apply(&envelope(
        11,
        &fixture.run,
        RunEventKind::RepeatConditionRecorded {
            iteration: third.clone(),
            result: true,
        },
    )?)?;
    projection.apply(&envelope(
        12,
        &fixture.run,
        RunEventKind::RepeatContinuationRequested {
            repeat_execution: repeat.clone(),
            frontier_iteration: third,
            initial_iteration_limit: 10,
            effective_iteration_limit: 11,
            cause: RepeatContinuationCause::DurationBudget {
                maximum_ms: 100,
                observed_ms: 150,
            },
        },
    )?)?;
    let continuation = &projection.repeat_continuations()[&repeat];
    assert!(continuation.is_pending_approval());
    assert_eq!(continuation.budget_override_iteration_limit(), None);
    Ok(())
}

#[test]
fn repeat_continuation_request_cycles_are_hard_capped() -> TestResult {
    let fixture = fixture("repeat-request-cap")?;
    let repeat = NodeExecutionId::new("execution-repeat")?;
    let mut events = vec![
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
    ];
    let mut sequence = 4_u64;
    for number in 1..=MAX_REPEAT_CONTINUATION_CYCLES as u32 {
        let iteration = IterationId::new(format!("iteration-{number}"))?;
        let scope = WorkspaceScope::iteration(
            ScopeId::new(format!("iteration-scope-{number}"))?,
            &fixture.root,
            iteration.clone(),
        )?;
        events.push(envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration: iteration.clone(),
                iteration_number: number,
                scope,
            },
        )?);
        sequence += 1;
        events.push(envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatConditionRecorded {
                iteration: iteration.clone(),
                result: true,
            },
        )?);
        sequence += 1;
        events.push(envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: repeat.clone(),
                frontier_iteration: iteration,
                initial_iteration_limit: 1,
                effective_iteration_limit: number,
                cause: RepeatContinuationCause::IterationLimit,
            },
        )?);
        sequence += 1;
        events.push(envelope(
            sequence,
            &fixture.run,
            RunEventKind::RepeatContinuationDecided {
                repeat_execution: repeat.clone(),
                decision: RepeatDecisionId::new(format!("decision-{number}"))?,
                actor: ActorRef::new("operator")?,
                outcome: RepeatContinuationDecision::Approved,
                approved_additional_iterations: Some(1),
                reason: Reason::new("bounded continuation")?,
                evidence: Vec::new(),
            },
        )?);
        sequence += 1;
    }
    let mut projection = RunProjection::replay(&events)?;
    let continuation = &projection.repeat_continuations()[&repeat];
    assert_eq!(
        continuation.requests().len(),
        MAX_REPEAT_CONTINUATION_CYCLES
    );
    assert_eq!(
        continuation.decisions().len(),
        MAX_REPEAT_CONTINUATION_CYCLES
    );
    assert_eq!(continuation.effective_iteration_limit(), 65);

    let frontier = IterationId::new("iteration-over-cap")?;
    projection.apply(&envelope(
        sequence,
        &fixture.run,
        RunEventKind::RepeatIterationCreated {
            repeat_execution: repeat.clone(),
            iteration: frontier.clone(),
            iteration_number: 65,
            scope: WorkspaceScope::iteration(
                ScopeId::new("iteration-scope-over-cap")?,
                &fixture.root,
                frontier.clone(),
            )?,
        },
    )?)?;
    sequence += 1;
    projection.apply(&envelope(
        sequence,
        &fixture.run,
        RunEventKind::RepeatConditionRecorded {
            iteration: frontier.clone(),
            result: true,
        },
    )?)?;
    sequence += 1;
    let over_cap = envelope(
        sequence,
        &fixture.run,
        RunEventKind::RepeatContinuationRequested {
            repeat_execution: repeat,
            frontier_iteration: frontier,
            initial_iteration_limit: 1,
            effective_iteration_limit: 65,
            cause: RepeatContinuationCause::IterationLimit,
        },
    )?;
    assert!(projection.apply(&over_cap).is_err());
    Ok(())
}

#[test]
fn repeat_body_subworkflow_may_be_nested_under_the_active_iteration_scope() -> TestResult {
    let fixture = fixture("repeat-subworkflow")?;
    let repeat = NodeExecutionId::new("execution-repeat")?;
    let iteration = IterationId::new("iteration-1")?;
    let iteration_scope = WorkspaceScope::iteration(
        ScopeId::new("iteration-scope")?,
        &fixture.root,
        iteration.clone(),
    )?;
    let subworkflow = SubworkflowId::new("repeat-body")?;
    let child_scope = WorkspaceScope::subworkflow(
        ScopeId::new("repeat-body-scope")?,
        &iteration_scope,
        subworkflow.clone(),
    )?;
    let projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        runtime_eligible(3, &fixture, "repeat", &repeat, fixture.root.reference())?,
        envelope(
            4,
            &fixture.run,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: repeat.clone(),
                iteration,
                iteration_number: 1,
                scope: iteration_scope,
            },
        )?,
        envelope(
            5,
            &fixture.run,
            RunEventKind::SubworkflowCreated {
                subworkflow: subworkflow.clone(),
                parent_execution: repeat,
                child_run: RunId::new("run-repeat-child")?,
                child_revision: revision('b')?,
                scope: child_scope,
                ownership: SubworkflowOwnership::Attached,
                inputs: Vec::new(),
            },
        )?,
    ])?;
    assert!(projection.subworkflows().contains_key(&subworkflow));
    Ok(())
}

#[test]
fn subworkflow_creation_materializes_atomic_child_scope_inputs() -> TestResult {
    let fixture = fixture("subworkflow-inputs")?;
    let parent = NodeExecutionId::new("execution-subworkflow")?;
    let subworkflow = SubworkflowId::new("subworkflow-child")?;
    let child_scope = WorkspaceScope::subworkflow(
        ScopeId::new("subworkflow-child-scope")?,
        &fixture.root,
        subworkflow.clone(),
    )?;
    let child_input = WorkspaceValueReference::new(
        child_scope.reference().clone(),
        milkdrift_workspace::ValueKey::new("request")?,
        milkdrift_workspace::ValueVersion::FIRST,
    );
    let mut projection = RunProjection::replay(&[
        created(&fixture, 1)?,
        envelope(2, &fixture.run, RunEventKind::RunStarted)?,
        runtime_eligible(
            3,
            &fixture,
            "subworkflow",
            &parent,
            fixture.root.reference(),
        )?,
    ])?;

    let unknown_ancestor = WorkspaceValueReference::new(
        fixture.root.reference().clone(),
        milkdrift_workspace::ValueKey::new("unknown")?,
        milkdrift_workspace::ValueVersion::FIRST,
    );
    let malformed = envelope(
        4,
        &fixture.run,
        RunEventKind::SubworkflowCreated {
            subworkflow: subworkflow.clone(),
            parent_execution: parent.clone(),
            child_run: RunId::new("run-subworkflow-child")?,
            child_revision: revision('b')?,
            scope: child_scope.clone(),
            ownership: SubworkflowOwnership::Attached,
            inputs: vec![unknown_ancestor],
        },
    )?;
    assert!(projection.apply(&malformed).is_err());
    assert_eq!(projection.sequence(), RunSequence::new(3));

    projection.apply(&envelope(
        4,
        &fixture.run,
        RunEventKind::SubworkflowCreated {
            subworkflow: subworkflow.clone(),
            parent_execution: parent,
            child_run: RunId::new("run-subworkflow-child")?,
            child_revision: revision('b')?,
            scope: child_scope,
            ownership: SubworkflowOwnership::Attached,
            inputs: vec![child_input.clone()],
        },
    )?)?;
    assert!(projection.workspace_values.contains(&child_input));
    assert_eq!(
        projection.subworkflows()[&subworkflow].inputs(),
        &[child_input]
    );
    Ok(())
}
