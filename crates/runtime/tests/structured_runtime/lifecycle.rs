//! Lifecycle integration scenarios.

use super::*;

#[test]
fn multiline_progress_remains_bounded_durable_and_replayable() -> TestResult {
    let harness = Harness::new("multiline-progress")?;
    let revision = task_revision("workflow-multiline-progress")?;
    let run = RunId::new("run-multiline-progress")?;
    let maximum_message = format!("{}\tx", "é".repeat(2_047));
    assert_eq!(maximum_message.len(), 4_096);
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Progress {
                message: "first\nsecond\r\n\t\u{1b}終".to_owned(),
                completed_units: Some(1),
                total_units: Some(2),
            },
            InvocationEventKind::Progress {
                message: maximum_message,
                completed_units: Some(2),
                total_units: Some(2),
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(runtime_tick(&harness.runtime)?.completed, 1);
    let history = harness.runtime.history(&run)?;
    let progress = history
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::NodeProgressRecorded {
                report_sequence,
                detail,
                completed_units,
                total_units,
                ..
            } => Some((
                *report_sequence,
                detail.as_str(),
                *completed_units,
                *total_units,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    let maximum_detail = format!("{} x", "é".repeat(2_047));
    assert_eq!(
        progress,
        vec![
            (1, "first second    終", Some(1), Some(2)),
            (2, maximum_detail.as_str(), Some(2), Some(2))
        ]
    );
    assert!(
        !history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { .. }))
    );
    let mut replay = milkdrift_runtime::RunProjection::default();
    for event in &history {
        replay.apply(event)?;
    }
    assert_eq!(replay, harness.runtime.projection(&run)?);
    assert_eq!(
        replay.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    Ok(())
}

#[test]
fn external_terminal_text_cannot_change_failure_or_uncertainty_classification() -> TestResult {
    for (label, status) in [
        ("failure", TerminalStatus::Failure),
        ("rejected", TerminalStatus::Rejected),
        ("uncertain", TerminalStatus::Uncertain),
        ("empty-uncertain", TerminalStatus::Uncertain),
    ] {
        let harness = Harness::new(&format!("terminal-text-{label}"))?;
        let revision = task_revision(&format!("workflow-terminal-text-{label}"))?;
        let run = RunId::new(format!("run-terminal-text-{label}"))?;
        let message = if label == "empty-uncertain" {
            String::new()
        } else {
            let message = format!("provider\n\t{}", "é".repeat(2_043));
            assert_eq!(message.len(), 4_096);
            message
        };
        harness.executor.set_script(
            OperationId::new("model.generate")?,
            vec![InvocationEventKind::Terminal {
                terminal: InvocationTerminal::new(
                    status,
                    Vec::new(),
                    Some(InvocationFailure::new(
                        ErrorClass::Provider,
                        false,
                        "provider_failure",
                        message,
                        None,
                    )?),
                    None,
                    SideEffectClass::None,
                )?,
            }],
        )?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        runtime_tick(&harness.runtime)?;
        let history = harness.runtime.history(&run)?;
        if status == TerminalStatus::Uncertain {
            let reasons = history
                .iter()
                .filter_map(|event| match event.kind() {
                    RunEventKind::ExternalOutcomeUncertain { reason, .. } => Some(reason.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let expected = if label == "empty-uncertain" {
                "external effect boundary returned without terminal evidence".to_owned()
            } else {
                format!("provider  {}", "é".repeat(995))
            };
            assert_eq!(reasons, vec![expected]);
        } else {
            let expected = if status == TerminalStatus::Failure {
                NodeOutcome::Failed
            } else {
                NodeOutcome::Rejected
            };
            let terminals = history
                .iter()
                .filter_map(|event| match event.kind() {
                    RunEventKind::NodeTerminal {
                        outcome,
                        error_class,
                        detail,
                        ..
                    } => Some((
                        *outcome,
                        *error_class,
                        detail.as_ref().map(|detail| detail.as_str().to_owned()),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                terminals,
                vec![(
                    expected,
                    Some(ErrorClass::Provider),
                    Some(format!("provider  {}", "é".repeat(2_043)))
                )]
            );
            assert!(!history.iter().any(|event| matches!(
                event.kind(),
                RunEventKind::ExternalOutcomeUncertain { .. }
            )));
        }
    }
    Ok(())
}

#[test]
fn claimed_effect_execution_reports_durable_entry_transition() -> TestResult {
    let harness = Harness::new("effect-tick-claimed")?;
    let revision = task_revision("workflow-effect-tick-claimed")?;
    let run = RunId::new("run-effect-tick-claimed")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    assert_eq!(harness.runtime.scheduler_tick()?.dispatched, 1);
    let actions = harness.runtime.claim_effects(PageSize::new(1)?)?;
    assert_eq!(actions.len(), 1);
    let result = harness.runtime.execute_effect(
        actions
            .into_iter()
            .next()
            .ok_or("claimed effect is absent")?,
    )?;
    assert!(matches!(
        result,
        EffectExecutionResult::Completed { observations } if observations > 0
    ));
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(
        harness
            .runtime
            .history(&run)?
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn terminal_observation_precedes_a_later_worker_boundary_failure() -> TestResult {
    let directory = TempDir::new()?;
    let executor = Arc::new(TerminalThenFailingExecutor::new(test_descriptor()?));
    executor.set_script(
        OperationId::new("model.generate")?,
        vec![InvocationEventKind::Terminal {
            terminal: successful_terminal()?,
        }],
    )?;
    let (store, _clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "terminal-before-worker-failure",
        "terminal-before-worker-failure",
        NOW,
        8,
        executor.clone(),
    )?;
    let revision = task_revision("workflow-terminal-before-worker-failure")?;
    let run = RunId::new("run-terminal-before-worker-failure")?;
    store.put_revision(&revision)?;
    submit_command(
        &runtime,
        store.as_ref(),
        &run,
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(
                run.clone(),
                ScopeId::new("scope-terminal-before-worker-failure")?,
            ),
            workspace_budget: generous_budget()?,
            inputs: Vec::new(),
        },
    )?;
    submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?;

    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    let action = runtime
        .claim_effects(PageSize::new(1)?)?
        .into_iter()
        .next()
        .ok_or("terminal precedence effect was not claimable")?;
    assert!(matches!(
        runtime.execute_effect(action)?,
        EffectExecutionResult::Completed { observations: 1 }
    ));
    assert_eq!(executor.dispatches(), 1);
    assert_eq!(
        runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    let history = runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::NodeTerminal { .. }))
            .count(),
        1
    );
    assert!(
        !history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { .. }))
    );
    Ok(())
}

#[test]
fn denied_capability_entry_is_recorded_and_terminates_without_dispatch() -> TestResult {
    let harness = Harness::with_entry_denied("effect-entry-denied")?;
    let revision = task_revision("workflow-effect-entry-denied")?;
    let run = RunId::new("run-effect-entry-denied")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    assert_eq!(harness.runtime.scheduler_tick()?.dispatched, 1);
    assert!(harness.runtime.claim_effects(PageSize::new(1)?)?.is_empty());
    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert!(
        projection
            .settled_node_executions()
            .values()
            .any(|execution| {
                execution.state() == &NodeExecutionState::Terminal(NodeOutcome::Rejected)
            })
    );
    let history = harness.runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(
                event.kind(),
                RunEventKind::CapabilityEntryDecisionRecorded { authorization, .. }
                    if !authorization.is_allowed()
            ))
            .count(),
        1
    );
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(
                event.kind(),
                RunEventKind::NodeTerminal {
                    outcome: NodeOutcome::Rejected,
                    error_class: Some(ErrorClass::Authorization),
                    ..
                }
            ))
            .count(),
        1
    );
    Ok(())
}

macro_rules! forward_store_methods {
    (
        $(
            fn $name:ident(
                &self
                $(, $argument:ident: $argument_type:ty)*
                $(,)?
            ) -> $return_type:ty;
        )+
    ) => {
        $(
            fn $name(&self $(, $argument: $argument_type)*) -> $return_type {
                self.inner.$name($($argument),*)
            }
        )+
    };
}

#[test]
fn scheduler_commits_dispatch_without_entering_long_running_executor() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new_with_authority(
        store.clone(),
        executor.clone(),
        test_authority(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("nonblocking-scheduler", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-nonblocking-scheduler")?,
            ActorRef::new("controller:nonblocking-scheduler")?,
            30_000,
            8,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?);
    let revision = task_revision("workflow-nonblocking-scheduler")?;
    let run = RunId::new("run-nonblocking-scheduler")?;
    store.put_revision(&revision)?;
    submit_command(
        &runtime,
        &store,
        &run,
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(
                run.clone(),
                ScopeId::new("scope-nonblocking-scheduler")?,
            ),
            workspace_budget: generous_budget()?,
            inputs: Vec::new(),
        },
    )?;
    submit_command(&runtime, &store, &run, RunCommand::StartRun)?;
    block_first_runnable_operation(&store, &runtime, &run, &executor)?;

    let tick = runtime.scheduler_tick()?;
    assert_eq!(tick.dispatched, 1);
    assert!(!executor.has_entered()?);

    let actions = runtime.claim_effects(PageSize::new(1)?)?;
    assert_eq!(actions.len(), 1);
    let action = actions
        .into_iter()
        .next()
        .ok_or("claimed effect is absent")?;
    let effect_runtime = runtime.clone();
    let effect = std::thread::spawn(move || {
        effect_runtime
            .execute_effect(action)
            .map_err(|error| error.to_string())
    });
    executor.wait_until_entered()?;

    let projection = runtime.projection(&run)?;
    assert!(
        projection
            .attempts()
            .values()
            .any(|attempt| { attempt.state() == &AttemptState::Running })
    );
    assert_eq!(runtime.scheduler_tick()?.dispatched, 0);

    executor.release()?;
    let effect_result = effect
        .join()
        .map_err(|_| "effect worker panicked")?
        .map_err(|error| format!("effect execution failed: {error}"))?;
    assert!(matches!(
        effect_result,
        EffectExecutionResult::Completed { .. }
    ));
    let completed = runtime.projection(&run)?;
    assert_eq!(
        completed.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert!(completed.attempts().is_empty());
    assert!(
        completed
            .settled_node_executions()
            .values()
            .any(|execution| {
                execution.state() == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
            })
    );
    Ok(())
}

#[test]
fn late_worker_reports_cannot_be_forged_through_external_commands() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        Arc::new(DeterministicExecutor::new(test_descriptor()?)),
        test_authority(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("late-terminal", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-late-terminal")?,
            ActorRef::new("controller:late-terminal")?,
            100,
            8,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?;
    let revision = task_revision("workflow-late-terminal")?;
    let run = RunId::new("run-late-terminal")?;
    store.put_revision(&revision)?;
    submit_command(
        &runtime,
        store.as_ref(),
        &run,
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-late-terminal")?),
            workspace_budget: generous_budget()?,
            inputs: Vec::new(),
        },
    )?;
    submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?;
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    let action = runtime
        .claim_effects(PageSize::new(1)?)?
        .into_iter()
        .next()
        .ok_or("expected one claimed effect")?;
    let dispatch = match action {
        EffectAction::Execute(dispatch) => dispatch,
        EffectAction::Cancel(_) => return Err("expected execution effect".into()),
    };

    clock.advance(101)?;
    let recovery = runtime.recover()?;
    assert_eq!(recovery.expired_leases, 1);
    let uncertain = runtime.projection(&run)?;
    let attempt = uncertain
        .attempts()
        .get(dispatch.attempt())
        .ok_or("claimed attempt is absent")?;
    assert_eq!(attempt.state(), &AttemptState::Uncertain);
    let report_sequence = attempt
        .obligation()
        .ok_or("uncertainty obligation is absent")?
        .report_sequence();

    let terminal = InvocationTerminal::new(
        TerminalStatus::Success,
        Vec::new(),
        None,
        None,
        SideEffectClass::None,
    )?;
    let report = InvocationEvent::new(
        dispatch.request().invocation().clone(),
        report_sequence,
        InvocationEventKind::Terminal { terminal },
    )?;
    let command = runtime.command(
        run.clone(),
        ActorRef::new("controller:late-terminal")?,
        store.head(&run)?,
        Reason::new("late executor terminal evidence")?,
        Vec::new(),
        RunCommand::WorkerReport {
            worker: WorkerId::new("worker-late-terminal")?,
            report: WorkerReport::Invocation {
                attempt: dispatch.attempt().clone(),
                report,
            },
        },
    )?;
    let head = store.head(&run)?;
    assert!(matches!(
        runtime.handle_authorized_command(&command, &test_authority_claim()?),
        Err(RuntimeError::InvalidCommand(detail))
            if detail.contains("worker reports cannot be submitted")
    ));
    assert_eq!(store.head(&run)?, head);
    let projection = runtime.projection(&run)?;
    assert_eq!(
        projection
            .attempts()
            .get(dispatch.attempt())
            .ok_or("claimed attempt is absent after rejected report")?
            .state(),
        &AttemptState::Uncertain
    );
    Ok(())
}

#[test]
fn expired_leases_cannot_cross_or_reenter_the_external_execution_boundary() -> TestResult {
    for (claim_before_expiry, elapsed_ms) in [
        (false, 30_000),
        (true, 30_000),
        (false, 30_001),
        (true, 30_001),
    ] {
        let directory = TempDir::new()?;
        let identity = if claim_before_expiry {
            format!("expired-claimed-ticket-{elapsed_ms}")
        } else {
            format!("expired-unclaimed-lease-{elapsed_ms}")
        };
        let executor = Arc::new(DeterministicExecutor::new(test_descriptor()?));
        let (store, clock, runtime) = runtime_with_executor_at(
            directory.path(),
            &identity,
            &identity,
            NOW,
            8,
            executor.clone(),
        )?;
        let revision = task_revision(&format!("workflow-{identity}"))?;
        let run = RunId::new(format!("run-{identity}"))?;
        store.put_revision(&revision)?;
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new(format!("scope-{identity}"))?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?;
        submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?;
        assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
        let action = if claim_before_expiry {
            Some(
                runtime
                    .claim_effects(PageSize::new(1)?)?
                    .into_iter()
                    .next()
                    .ok_or("fresh lease was not claimable")?,
            )
        } else {
            None
        };
        clock.advance(elapsed_ms)?;
        if let Some(action) = action {
            assert!(matches!(
                runtime.execute_effect(action),
                Err(RuntimeError::InvalidTransition(_))
            ));
        } else {
            assert!(runtime.claim_effects(PageSize::new(1)?)?.is_empty());
        }
        assert_eq!(executor.entry_count(), 0);
        let starts = runtime
            .history(&run)?
            .into_iter()
            .filter(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
            .count();
        assert_eq!(starts, usize::from(claim_before_expiry));
    }
    Ok(())
}

#[test]
fn consumed_non_idempotent_ticket_is_not_reissued_after_deterministic_failure() -> TestResult {
    let directory = TempDir::new()?;
    let executor = Arc::new(InvalidReportsCountingExecutor::new(
        descriptor_with_model_side_effect("non_idempotent_write")?,
    ));
    let (store, _clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "one-shot-effect-ticket",
        "one-shot-effect-ticket",
        NOW,
        8,
        executor.clone(),
    )?;
    let revision = task_revision("workflow-one-shot-effect-ticket")?;
    let run = RunId::new("run-one-shot-effect-ticket")?;
    store.put_revision(&revision)?;
    submit_command(
        &runtime,
        store.as_ref(),
        &run,
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(
                run.clone(),
                ScopeId::new("scope-one-shot-effect-ticket")?,
            ),
            workspace_budget: generous_budget()?,
            inputs: Vec::new(),
        },
    )?;
    submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?;
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    let action = runtime
        .claim_effects(PageSize::new(1)?)?
        .into_iter()
        .next()
        .ok_or("effect ticket was not claimed")?;
    let rejected = runtime.execute_effect(action);
    assert!(
        matches!(
            rejected,
            Err(RuntimeError::Executor(ExecutorError::InvalidReports(_)))
        ),
        "invalid post-entry report returned {rejected:?}"
    );
    assert_eq!(executor.dispatches(), 1);
    assert!(runtime.claim_effects(PageSize::new(1)?)?.is_empty());
    assert_eq!(executor.dispatches(), 1);
    let history = runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
            .count(),
        1
    );
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::ExternalOutcomeUncertain { reason, .. }
            if reason.as_str().contains("report rejected after adapter entry")
    )));
    Ok(())
}

#[test]
#[ignore = "expensive durable-storage boundary regression; run explicitly"]
fn historical_execution_frontier_stays_bounded_across_index_limit() -> TestResult {
    let harness = Harness::new("bounded-operational-frontier")?;
    let revision = signal_revision("workflow-bounded-operational-frontier")?;
    let run = RunId::new("run-bounded-operational-frontier")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    let initial = harness.runtime.projection(&run)?;
    let root_scope = initial
        .root_scope()
        .ok_or("bounded-frontier run has no root scope")?
        .reference()
        .clone();
    let budget = initial
        .workspace_budget()
        .ok_or("bounded-frontier run has no workspace budget")?
        .clone();
    let usage = harness.store.workspace_usage(&run)?;
    let historical_count = MAX_INDEX_MUTATIONS_PER_COMMIT + 1;
    let mut created = 0_usize;
    let mut batch_number = 0_usize;

    // Seed validated, immutable terminal execution facts in bounded journal commits.
    // The final runtime command is the behavior under test: the old implementation
    // generated one index tombstone for every seeded historical identity and failed
    // the atomic index-mutation bound before it could pause the run.
    while created < historical_count {
        let expected = harness.store.head(&run)?;
        // Each occurrence emits three events; keep the command result below its
        // independent 512-event-identity document bound.
        let batch_size = historical_count.saturating_sub(created).min(160);
        let mut sequence = expected;
        let mut events = Vec::with_capacity(batch_size.saturating_mul(3));
        for offset in 0..batch_size {
            let number = created.saturating_add(offset);
            let execution = NodeExecutionId::new(format!("historical-execution-{number:04}"))?;
            sequence = sequence.next()?;
            events.push(RunEventEnvelope::new(
                EventId::new(format!("historical-eligible-{number:04}"))?,
                run.clone(),
                sequence,
                TimestampMillis::new(NOW),
                RunEventKind::NodeBecameEligible {
                    node: NodeId::new("done")?,
                    execution: execution.clone(),
                    scope: root_scope.clone(),
                    mode: NodeExecutionMode::Runtime,
                },
            )?);
            sequence = sequence.next()?;
            events.push(RunEventEnvelope::new(
                EventId::new(format!("historical-terminal-{number:04}"))?,
                run.clone(),
                sequence,
                TimestampMillis::new(NOW),
                RunEventKind::DeterministicNodeTerminal {
                    execution: execution.clone(),
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            )?);
            sequence = sequence.next()?;
            events.push(RunEventEnvelope::new(
                EventId::new(format!("historical-successor-scan-{number:04}"))?,
                run.clone(),
                sequence,
                TimestampMillis::new(NOW),
                RunEventKind::StructuredSuccessorScanCompleted { execution },
            )?);
        }
        let command = CommandId::new(format!("seed-index-history-{batch_number:02}"))?;
        let receipt = CommandReceipt::new(
            command.clone(),
            run.clone(),
            ActorRef::new("controller:bounded-operational-frontier")?,
            expected,
            TimestampMillis::new(NOW),
            format!(r#"{{"batch":{batch_number},"schema_version":1,"type":"seed_index_history"}}"#)
                .into_bytes(),
        )?;
        let event_ids = events
            .iter()
            .map(|event| event.event_id().clone())
            .collect();
        let result = CommandResultDocument::new(
            command,
            run.clone(),
            receipt.fingerprint().clone(),
            CommandDisposition::Accepted,
            sequence,
            event_ids,
            BoundedJson::new(json!({"accepted": true}))?,
        )?;
        harness.store.commit_command(&AtomicRunCommitRequest::new(
            receipt,
            events,
            Vec::new(),
            Some(WorkspaceAccounting {
                budget: budget.clone(),
                expected_usage: usage,
                resulting_usage: usage,
            }),
            Vec::new(),
            Vec::new(),
            None,
            result,
            RunIndexUpdate::new(
                Some(RunSummaryIndex {
                    run: run.clone(),
                    workflow: revision.semantic().workflow().clone(),
                    revision: revision.id().clone(),
                    state: IndexedRunState::Waiting,
                    through_sequence: sequence,
                    updated_at: TimestampMillis::new(NOW),
                }),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        )?)?;
        created = created.saturating_add(batch_size);
        batch_number = batch_number.saturating_add(1);
    }

    let projection = harness.runtime.projection(&run)?;
    assert!(projection.waits().values().any(|wait| wait.is_pending()));
    assert!(
        projection.node_executions().len() <= 2,
        "active frontier retained {} full executions",
        projection.node_executions().len()
    );
    assert!(projection.settled_node_executions().len() <= 2);
    assert_eq!(
        projection
            .executions_for_node(&NodeId::new("done")?)
            .count(),
        1
    );
    let before_pause = harness.store.head(&run)?;
    assert_eq!(
        harness.command(&run, RunCommand::PauseRun)?,
        CommandDisposition::Accepted
    );
    assert_eq!(harness.store.head(&run)?, before_pause.next()?);
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Paused
    );

    let mut cursor = None;
    let mut eligible = 0_usize;
    let mut terminal = 0_usize;
    let mut scanned = 0_usize;
    loop {
        let page = harness.runtime.history_page(&EventPageQuery::new(
            run.clone(),
            cursor,
            PageSize::new(MAX_PAGE_SIZE)?,
        )?)?;
        for event in page.events {
            match event.kind() {
                RunEventKind::NodeBecameEligible { node, .. } if node.as_str() == "done" => {
                    eligible = eligible.saturating_add(1);
                }
                RunEventKind::DeterministicNodeTerminal { .. } => {
                    terminal = terminal.saturating_add(1);
                }
                RunEventKind::StructuredSuccessorScanCompleted { .. } => {
                    scanned = scanned.saturating_add(1);
                }
                _ => {}
            }
        }
        let Some(next) = page.next else {
            break;
        };
        cursor = Some(next);
    }
    assert_eq!(eligible, historical_count);
    assert_eq!(terminal, historical_count);
    assert_eq!(scanned, historical_count);
    eprintln!(
        "historical_occurrences={historical_count} active_executions={} settled_summaries={} pause_events=1 eligible_events={eligible} terminal_events={terminal} successor_scan_events={scanned}",
        projection.node_executions().len(),
        projection.settled_node_executions().len(),
    );
    Ok(())
}

#[path = "lifecycle/startup.rs"]
mod startup;

#[path = "lifecycle/cancellation.rs"]
mod cancellation;
