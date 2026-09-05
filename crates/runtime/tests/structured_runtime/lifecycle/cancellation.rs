use super::*;

#[test]
fn explicit_terminal_waits_for_an_already_dispatched_any_join_loser() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new_with_authority(
        store.clone(),
        executor.clone(),
        test_authority(),
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
    runtime.handle_authorized_command(&create, &test_authority_claim()?)?;
    let start = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("start terminal deferral run")?,
        Vec::new(),
        RunCommand::StartRun,
    )?;
    runtime.handle_authorized_command(&start, &test_authority_claim()?)?;

    let first_runtime = runtime.clone();
    let first_tick =
        std::thread::spawn(move || runtime_tick(&first_runtime).map_err(|error| error.to_string()));
    executor.wait_until_entered()?;
    let second_tick = runtime_tick(&runtime)?;
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

    runtime_tick(&runtime)?;
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
        let runtime = Arc::new(RuntimeService::new_with_authority(
            store.clone(),
            executor.clone(),
            test_authority(),
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
            runtime_tick(&blocked_runtime)
                .map_err(|error| format!("terminal cancellation blocked tick failed: {error}"))
        });
        executor.wait_until_entered()?;
        assert_eq!(runtime_tick(&runtime)?.dispatched, 1);
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
        runtime_tick(&runtime)?;
        assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
        executor.release()?;
        blocked
            .join()
            .map_err(|_| "terminal cancellation dispatch thread panicked")??;
        for _ in 0..4 {
            if runtime.projection(&run)?.is_completed() {
                break;
            }
            runtime_tick(&runtime)?;
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
    let runtime = Arc::new(RuntimeService::new_with_authority(
        store.clone(),
        executor.clone(),
        test_authority(),
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
        runtime_tick(&blocked_runtime)
            .map_err(|error| format!("explicit failure blocked tick failed: {error}"))
    });
    executor.wait_until_entered()?;
    assert_eq!(runtime_tick(&runtime)?.dispatched, 1);

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

    runtime_tick(&runtime)?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    executor.release()?;
    blocked
        .join()
        .map_err(|_| "explicit failure dispatch thread panicked")??;
    for _ in 0..4 {
        if runtime.projection(&run)?.is_completed() {
            break;
        }
        runtime_tick(&runtime)?;
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
    let runtime = Arc::new(RuntimeService::new_with_authority(
        store.clone(),
        executor.clone(),
        test_authority(),
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
        runtime
            .handle_authorized_command(&create, &test_authority_claim()?)?
            .result()
            .disposition(),
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
    runtime.handle_authorized_command(&start, &test_authority_claim()?)?;

    let tick_runtime = runtime.clone();
    let dispatch =
        std::thread::spawn(move || runtime_tick(&tick_runtime).map_err(|error| error.to_string()));
    executor.wait_until_entered()?;
    let invocation = runtime
        .projection(&run)?
        .attempts()
        .values()
        .find(|attempt| attempt.state() == &AttemptState::Running)
        .and_then(|attempt| attempt.invocation())
        .cloned()
        .ok_or("active cancellation fixture has no running invocation")?;
    let cancel = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("cancel an invocation after durable dispatch")?,
        Vec::new(),
        RunCommand::RequestCancellation,
    )?;
    assert_eq!(
        runtime
            .handle_authorized_command(&cancel, &test_authority_claim()?)?
            .result()
            .disposition(),
        CommandDisposition::Accepted
    );

    runtime_tick(&runtime)?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        executor.cancellation_request_sequence(&invocation)?,
        Some(1)
    );
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
    assert!(projection.attempts().is_empty());
    assert!(
        projection
            .settled_node_executions()
            .values()
            .any(|execution| {
                execution.state() == &NodeExecutionState::Terminal(NodeOutcome::Cancelled)
            })
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
    assert_eq!(runtime_tick(&harness.runtime)?.deferred, 1);
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
