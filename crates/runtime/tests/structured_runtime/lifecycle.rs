//! Lifecycle integration scenarios.

use super::*;

#[test]
fn startup_keeps_admission_closed_until_integrity_and_recovery_complete() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let runtime = RuntimeService::open_closed(
        store,
        Arc::new(DeterministicExecutor::new(test_descriptor()?)),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("startup-gate", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-startup-gate")?,
            ActorRef::new("controller:startup-gate")?,
            30_000,
            8,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?;

    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());
    assert!(runtime.resume_admission().is_err());

    runtime.initialize_startup()?;
    assert_eq!(
        runtime.startup_state(),
        RuntimeStartupState::RecoveryCompleted
    );
    assert!(runtime.is_accepting_admission());

    runtime.begin_shutdown();
    assert!(!runtime.is_accepting_admission());
    runtime.resume_admission()?;
    assert!(runtime.is_accepting_admission());
    Ok(())
}

#[test]
fn scheduler_commits_dispatch_without_entering_long_running_executor() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new(
        store.clone(),
        executor.clone(),
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
            .execute_effect(&action)
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
    effect
        .join()
        .map_err(|_| "effect worker panicked")?
        .map_err(|error| format!("effect execution failed: {error}"))?;
    assert!(runtime.projection(&run)?.is_completed());
    Ok(())
}

#[test]
fn terminal_observed_after_lease_expiry_is_preserved_as_late_evidence() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let runtime = RuntimeService::new(
        store.clone(),
        Arc::new(DeterministicExecutor::new(test_descriptor()?)),
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
    assert_eq!(
        runtime.handle_command(&command)?.result().disposition(),
        CommandDisposition::Accepted
    );

    let projection = runtime.projection(&run)?;
    let attempt = projection
        .attempts()
        .get(dispatch.attempt())
        .ok_or("claimed attempt is absent after evidence")?;
    assert_eq!(attempt.state(), &AttemptState::Uncertain);
    assert!(attempt.terminal().is_none());
    let evidence = attempt
        .late_terminal_evidence()
        .ok_or("late terminal evidence was not retained")?;
    assert_eq!(evidence.report_sequence(), report_sequence);
    assert_eq!(evidence.terminal().status(), TerminalStatus::Success);
    assert!(runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::LateTerminalEvidenceRecorded { .. }
    )));
    Ok(())
}

#[test]
fn explicit_terminal_waits_for_an_already_dispatched_any_join_loser() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new(
        store.clone(),
        executor.clone(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("terminal-deferral", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-terminal-deferral")?,
            ActorRef::new("controller:terminal-deferral")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?);
    let revision = fork_revision("workflow-terminal-deferral", JoinPolicy::Any, "model.fail")?;
    let run = RunId::new("run-terminal-deferral")?;
    store.put_revision(&revision)?;
    let create = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("create terminal deferral run")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(
                run.clone(),
                ScopeId::new("scope-terminal-deferral")?,
            ),
            workspace_budget: generous_budget()?,
            inputs: Vec::new(),
        },
    )?;
    runtime.handle_command(&create)?;
    let start = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("start terminal deferral run")?,
        Vec::new(),
        RunCommand::StartRun,
    )?;
    runtime.handle_command(&start)?;

    let first_runtime = runtime.clone();
    let first_tick =
        std::thread::spawn(move || first_runtime.tick().map_err(|error| error.to_string()));
    executor.wait_until_entered()?;
    let second_tick = runtime.tick()?;
    assert_eq!(second_tick.dispatched, 1);

    let midway = runtime.projection(&run)?;
    assert_eq!(midway.lifecycle(), RunLifecycle::Running);
    assert_eq!(midway.joins().len(), 1);
    let done_id = NodeId::new("done")?;
    assert_eq!(
        midway
            .executions_for_node(&done_id)
            .next()
            .map(|execution| execution.state()),
        Some(&NodeExecutionState::Terminal(
            milkdrift_persistence::NodeOutcome::Succeeded
        ))
    );
    assert!(
        midway
            .attempts()
            .values()
            .any(|attempt| attempt.is_active())
    );
    assert!(
        !runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::RunTerminal { .. }))
    );

    runtime.tick()?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.projection(&run)?.lifecycle(), RunLifecycle::Running);
    executor.release()?;
    first_tick
        .join()
        .map_err(|_| "first scheduler thread panicked")?
        .map_err(|error| format!("first scheduler tick failed: {error}"))?;

    assert_eq!(
        runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert!(
        runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::RunTerminal { .. }))
    );
    Ok(())
}

#[test]
fn later_cancellation_dominates_completed_explicit_success_and_failure_terminals() -> TestResult {
    for (suffix, terminal_outcome, node_outcome) in [
        ("success", TerminalOutcome::Success, NodeOutcome::Succeeded),
        ("failure", TerminalOutcome::Failure, NodeOutcome::Failed),
    ] {
        let directory = TempDir::new()?;
        let store = Arc::new(RedbStore::open(directory.path())?);
        let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
        let runtime = Arc::new(RuntimeService::new(
            store.clone(),
            executor.clone(),
            Arc::new(ManualClock::new(NOW)),
            Arc::new(SequentialIdGenerator::new(
                format!("terminal-cancellation-{suffix}"),
                1,
            )?),
            RuntimeConfig::new(
                WorkerId::new(format!("worker-terminal-cancellation-{suffix}"))?,
                ActorRef::new(format!("controller:terminal-cancellation-{suffix}"))?,
                30_000,
                32,
                SchedulerLimits::new(8, 4, 2, 4)?,
                RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
            )?,
        )?);
        let revision = fork_revision_with_terminal(
            &format!("workflow-terminal-cancellation-{suffix}"),
            JoinPolicy::Any,
            "model.fail",
            terminal_outcome,
        )?;
        let run = RunId::new(format!("run-terminal-cancellation-{suffix}"))?;
        store.put_revision(&revision)?;
        assert_eq!(
            submit_command(
                runtime.as_ref(),
                store.as_ref(),
                &run,
                RunCommand::CreateRun {
                    workflow: revision.semantic().workflow().clone(),
                    revision: revision.id().clone(),
                    root_scope: WorkspaceScope::run_root(
                        run.clone(),
                        ScopeId::new(format!("scope-terminal-cancellation-{suffix}"))?,
                    ),
                    workspace_budget: generous_budget()?,
                    inputs: Vec::new(),
                },
            )?,
            CommandDisposition::Accepted
        );
        assert_eq!(
            submit_command(runtime.as_ref(), store.as_ref(), &run, RunCommand::StartRun)?,
            CommandDisposition::Accepted
        );
        block_first_runnable_operation(store.as_ref(), runtime.as_ref(), &run, executor.as_ref())?;

        let blocked_runtime = runtime.clone();
        let blocked = std::thread::spawn(move || {
            blocked_runtime
                .tick()
                .map_err(|error| format!("terminal cancellation blocked tick failed: {error}"))
        });
        executor.wait_until_entered()?;
        assert_eq!(runtime.tick()?.dispatched, 1);
        let midway = runtime.projection(&run)?;
        let done = NodeId::new("done")?;
        assert_eq!(
            midway
                .executions_for_node(&done)
                .next()
                .map(|execution| execution.state()),
            Some(&NodeExecutionState::Terminal(node_outcome))
        );
        assert_eq!(midway.lifecycle(), RunLifecycle::Running);
        if terminal_outcome == TerminalOutcome::Failure {
            let history = runtime.history(&run)?;
            let failure_sequence = history
                .iter()
                .find_map(|event| match event.kind() {
                    RunEventKind::DeterministicNodeTerminal {
                        execution,
                        outcome: NodeOutcome::Failed,
                        ..
                    } if midway
                        .node_executions()
                        .get(execution)
                        .is_some_and(|execution| execution.node() == &done) =>
                    {
                        Some(event.sequence())
                    }
                    _ => None,
                })
                .ok_or("explicit failure terminal event is absent")?;
            let termination = midway
                .termination()
                .ok_or("failure drain termination intent is absent")?;
            assert_eq!(termination.outcome(), RunOutcome::Failed);
            assert!(termination.sequence() > failure_sequence);
            assert!(midway.cancellation().is_none());
        }
        assert_eq!(
            submit_command(
                runtime.as_ref(),
                store.as_ref(),
                &run,
                RunCommand::RequestCancellation,
            )?,
            CommandDisposition::Accepted
        );
        runtime.tick()?;
        assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
        executor.release()?;
        blocked
            .join()
            .map_err(|_| "terminal cancellation dispatch thread panicked")??;
        for _ in 0..4 {
            if runtime.projection(&run)?.is_completed() {
                break;
            }
            runtime.tick()?;
        }
        let completed = runtime.projection(&run)?;
        assert_eq!(
            completed.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Cancelled),
            "explicit {suffix} terminal overrode later cancellation intent"
        );
        let terminal_outcomes: Vec<_> = runtime
            .history(&run)?
            .iter()
            .filter_map(|event| match event.kind() {
                RunEventKind::RunTerminal { outcome, .. } => Some(*outcome),
                _ => None,
            })
            .collect();
        assert_eq!(terminal_outcomes, vec![RunOutcome::Cancelled]);
    }
    Ok(())
}

#[test]
fn explicit_failure_terminal_drains_owned_work_and_finishes_failed() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new(
        store.clone(),
        executor.clone(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("explicit-failure-drain", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-explicit-failure-drain")?,
            ActorRef::new("controller:explicit-failure-drain")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?);
    let revision = fork_revision_with_terminal(
        "workflow-explicit-failure-drain",
        JoinPolicy::Any,
        "model.fail",
        TerminalOutcome::Failure,
    )?;
    let run = RunId::new("run-explicit-failure-drain")?;
    store.put_revision(&revision)?;
    assert_eq!(
        submit_command(
            runtime.as_ref(),
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-explicit-failure-drain")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(runtime.as_ref(), store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    block_first_runnable_operation(store.as_ref(), runtime.as_ref(), &run, executor.as_ref())?;

    let blocked_runtime = runtime.clone();
    let blocked = std::thread::spawn(move || {
        blocked_runtime
            .tick()
            .map_err(|error| format!("explicit failure blocked tick failed: {error}"))
    });
    executor.wait_until_entered()?;
    assert_eq!(runtime.tick()?.dispatched, 1);

    let draining = runtime.projection(&run)?;
    assert_eq!(draining.lifecycle(), RunLifecycle::Running);
    assert_eq!(
        draining
            .termination()
            .ok_or("explicit failure drain intent is absent")?
            .outcome(),
        RunOutcome::Failed
    );
    assert!(draining.cancellation().is_none());
    assert!(
        !runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::RunCancellationRequested { .. }))
    );

    runtime.tick()?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    executor.release()?;
    blocked
        .join()
        .map_err(|_| "explicit failure dispatch thread panicked")??;
    for _ in 0..4 {
        if runtime.projection(&run)?.is_completed() {
            break;
        }
        runtime.tick()?;
    }
    let completed = runtime.projection(&run)?;
    assert_eq!(
        completed.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert!(completed.cancellation().is_none());
    let terminal_outcomes: Vec<_> = runtime
        .history(&run)?
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::RunTerminal { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .collect();
    assert_eq!(terminal_outcomes, vec![RunOutcome::Failed]);
    Ok(())
}

#[test]
fn active_invocation_cancellation_reaches_the_executor_and_is_acknowledged_durably() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let config = RuntimeConfig::new(
        WorkerId::new("worker-active-cancel")?,
        ActorRef::new("controller:active-cancel")?,
        30_000,
        32,
        SchedulerLimits::new(8, 4, 2, 4)?,
        RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
    )?;
    let runtime = Arc::new(RuntimeService::new(
        store.clone(),
        executor.clone(),
        clock,
        Arc::new(SequentialIdGenerator::new("active-cancel", 1)?),
        config,
    )?);
    let revision = task_revision("workflow-active-cancel")?;
    let run = RunId::new("run-active-cancel")?;
    store.put_revision(&revision)?;

    let create = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("create active cancellation run")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-active-cancel")?),
            workspace_budget: generous_budget()?,
            inputs: Vec::new(),
        },
    )?;
    assert_eq!(
        runtime.handle_command(&create)?.result().disposition(),
        CommandDisposition::Accepted
    );
    let start = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("start active cancellation run")?,
        Vec::new(),
        RunCommand::StartRun,
    )?;
    runtime.handle_command(&start)?;

    let tick_runtime = runtime.clone();
    let dispatch =
        std::thread::spawn(move || tick_runtime.tick().map_err(|error| error.to_string()));
    executor.wait_until_entered()?;
    let cancel = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("cancel an invocation after durable dispatch")?,
        Vec::new(),
        RunCommand::RequestCancellation,
    )?;
    assert_eq!(
        runtime.handle_command(&cancel)?.result().disposition(),
        CommandDisposition::Accepted
    );

    runtime.tick()?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    executor.release()?;
    dispatch
        .join()
        .map_err(|_| "dispatch thread panicked")?
        .map_err(|error| format!("dispatch failed: {error}"))?;

    let projection = runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Cancelled)
    );
    let history = runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeExecutionCancellationRequested { .. }
    )));
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::InvocationCancellationAcknowledged { .. }
    )));
    Ok(())
}

#[test]
fn shutdown_closes_admission_and_cancellation_explicitly_drains_wait_ownership() -> TestResult {
    let harness = Harness::new("cancel")?;
    let revision = wait_revision("workflow-cancel", 60_000)?;
    let run = RunId::new("run-cancel")?;
    harness.put_revision(&revision)?;
    harness.create(&run, &revision)?;

    harness.runtime.begin_shutdown();
    assert!(!harness.runtime.is_accepting_admission());
    assert_eq!(harness.runtime.tick()?.deferred, 1);
    assert_eq!(
        harness.command(&run, RunCommand::StartRun)?,
        CommandDisposition::Rejected
    );
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Created
    );

    harness.runtime.resume_admission()?;
    assert_eq!(
        harness.command(&run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(&run, RunCommand::RequestCancellation)?,
        CommandDisposition::Accepted
    );
    harness.drive(&run, 4)?;

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Cancelled)
    );
    assert!(
        projection
            .timers()
            .values()
            .all(|timer| !timer.is_pending())
    );
    assert!(projection.waits().values().all(|wait| !wait.is_pending()));
    let history = harness.runtime.history(&run)?;
    assert!(
        history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::TimerCancelled { .. }))
    );
    assert!(
        history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::WaitCancelled { .. }))
    );
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeExecutionCancelledBeforeDispatch { .. }
    )));
    Ok(())
}

#[test]
fn startup_recovery_finishes_with_an_unexpired_active_lease() -> TestResult {
    let directory = TempDir::new()?;
    let identity = "startup-valid-active-lease";
    let (store, clock, runtime) = runtime_with_executor_at(
        directory.path(),
        identity,
        identity,
        NOW,
        8,
        Arc::new(DeterministicExecutor::new(test_descriptor()?)),
    )?;
    let revision = task_revision("workflow-startup-valid-active-lease")?;
    let run = RunId::new("run-startup-valid-active-lease")?;
    store.put_revision(&revision)?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-startup-valid-active-lease")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    let actions = runtime.claim_effects(PageSize::new(1)?)?;
    assert_eq!(actions.len(), 1);
    assert!(matches!(actions.first(), Some(EffectAction::Execute(_))));
    let projection = runtime.projection(&run)?;
    assert!(projection.attempts().values().any(|attempt| {
        attempt.state() == &AttemptState::Running
            && projection.leases().values().any(|lease| {
                lease.is_active()
                    && lease.attempt() == attempt.attempt()
                    && lease.expires_at() > TimestampMillis::new(NOW)
            })
    }));
    drop(projection);
    drop(actions);
    drop(runtime);
    drop(clock);
    drop(store);

    let (_store, _clock, reopened) = runtime_with_executor_at(
        directory.path(),
        "startup-valid-active-lease-reopen",
        identity,
        NOW,
        8,
        Arc::new(DeterministicExecutor::new(test_descriptor()?)),
    )?;
    assert_eq!(
        reopened.startup_state(),
        RuntimeStartupState::RecoveryCompleted
    );
    assert!(reopened.is_accepting_admission());
    Ok(())
}

#[test]
#[ignore = "expensive authenticated-storage boundary regression; run explicitly"]
fn more_than_index_mutation_limit_inactive_identities_do_not_block_commits() -> TestResult {
    let harness = Harness::new("large-inactive-index-history")?;
    let revision = signal_revision("workflow-large-inactive-index-history")?;
    let run = RunId::new("run-large-inactive-index-history")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    let initial = harness.runtime.projection(&run)?;
    let root_scope = initial
        .root_scope()
        .ok_or("large-history run has no root scope")?
        .reference()
        .clone();
    let budget = initial
        .workspace_budget()
        .ok_or("large-history run has no workspace budget")?
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
        let batch_size = historical_count.saturating_sub(created).min(256);
        let mut sequence = expected;
        let mut events = Vec::with_capacity(batch_size.saturating_mul(2));
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
                    execution,
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            )?);
        }
        let command = CommandId::new(format!("seed-index-history-{batch_number:02}"))?;
        let receipt = CommandReceipt::new(
            command.clone(),
            run.clone(),
            ActorRef::new("controller:large-inactive-index-history")?,
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
        projection.node_executions().len() > MAX_INDEX_MUTATIONS_PER_COMMIT,
        "fixture accumulated only {} execution identities",
        projection.node_executions().len()
    );
    assert_eq!(
        harness.command(&run, RunCommand::PauseRun)?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Paused
    );
    Ok(())
}
