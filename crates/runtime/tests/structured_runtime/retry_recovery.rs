//! Retry recovery integration scenarios.

use super::*;

#[test]
fn idempotent_boundary_error_retries_exact_request_and_keeps_first_attempt_truthful() -> TestResult
{
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BoundaryFailingExecutor::new(
        descriptor_with_model_side_effect("idempotent_write")?,
        1,
    ));
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        executor.clone(),
        test_authority(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("idempotent-boundary-retry", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-idempotent-boundary-retry")?,
            ActorRef::new("controller:idempotent-boundary-retry")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(2, vec![ErrorClass::Adapter], 1, 1_000, 0)?,
        )?,
    )?;
    let revision = task_revision("workflow-idempotent-boundary-retry")?;
    let run = RunId::new("run-idempotent-boundary-retry")?;
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
                    ScopeId::new("scope-idempotent-boundary-retry")?,
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

    let first_tick = runtime_tick(&runtime)?;
    assert_eq!(first_tick.dispatched, 1);
    assert_eq!(first_tick.completed, 0);
    assert_eq!(first_tick.uncertain, 1);
    let pending = runtime.projection(&run)?;
    assert_eq!(pending.unresolved_attempts().count(), 1);
    assert!(
        pending
            .attempts()
            .values()
            .all(|attempt| attempt.terminal().is_none())
    );
    assert!(
        runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { .. }))
    );
    assert!(!runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeTerminal {
            outcome: NodeOutcome::Failed,
            ..
        }
    )));

    clock.advance(1)?;
    let retry_tick = runtime_tick(&runtime)?;
    assert_eq!(retry_tick.dispatched, 1);
    assert_eq!(retry_tick.completed, 1);
    let projection = runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    let second_attempt = projection
        .settled_node_executions()
        .values()
        .find_map(|execution| execution.latest_attempt().cloned())
        .ok_or("second idempotent attempt anchor is absent")?;
    assert!(projection.attempts().is_empty());
    assert!(
        projection
            .settled_node_executions()
            .values()
            .any(|execution| {
                execution.state() == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                    && execution.attempt_count() == 2
            })
    );
    assert_eq!(projection.unresolved_attempts().count(), 0);
    let mut history = Vec::new();
    let mut cursor = None;
    let mut page_count = 0_usize;
    loop {
        let page = runtime.history_page(&EventPageQuery::new(
            run.clone(),
            cursor,
            PageSize::new(3)?,
        )?)?;
        page_count = page_count.saturating_add(1);
        history.extend(page.events);
        let Some(next) = page.next else {
            break;
        };
        cursor = Some(next);
    }
    assert!(
        page_count > 1,
        "historical evidence did not cross a page cursor"
    );
    let (first_attempt, covering_attempt) = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::NodeRetryScheduled {
                previous_attempt,
                next_attempt,
                ..
            } => Some((previous_attempt, next_attempt)),
            _ => None,
        })
        .ok_or("durable retry provenance is absent")?;
    assert_eq!(covering_attempt, &second_attempt);
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::ExternalOutcomeUncertain { attempt, .. } if attempt == first_attempt
    )));

    let dispatches = executor.dispatches()?;
    assert_eq!(dispatches.len(), 2);
    assert_eq!(
        dispatches[0].request().idempotency_key(),
        dispatches[1].request().idempotency_key()
    );
    assert!(dispatches[0].request().idempotency_key().is_some());
    assert_eq!(dispatches[0].resolution(), dispatches[1].resolution());
    assert_eq!(
        dispatches[0].request().capability(),
        dispatches[1].request().capability()
    );
    assert_eq!(
        dispatches[0].request().operation(),
        dispatches[1].request().operation()
    );
    assert_eq!(
        dispatches[0].request().inputs(),
        dispatches[1].request().inputs()
    );
    assert_eq!(
        dispatches[0].request().extensions(),
        dispatches[1].request().extensions()
    );
    Ok(())
}

#[test]
fn uncertainty_and_retry_share_one_boundary_clock_observation() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(AdvancingClock::new(NOW));
    let executor = Arc::new(BoundaryFailingExecutor::new(
        descriptor_with_model_side_effect("idempotent_write")?,
        1,
    ));
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        executor,
        test_authority(),
        clock,
        Arc::new(SequentialIdGenerator::new(
            "advancing-uncertainty-clock",
            1,
        )?),
        RuntimeConfig::new(
            WorkerId::new("worker-advancing-uncertainty-clock")?,
            ActorRef::new("controller:advancing-uncertainty-clock")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(2, vec![ErrorClass::Adapter], 1, 1_000, 0)?,
        )?,
    )?;
    let revision = task_revision("workflow-advancing-uncertainty-clock")?;
    let run = RunId::new("run-advancing-uncertainty-clock")?;
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
                ScopeId::new("scope-advancing-uncertainty-clock")?,
            ),
            workspace_budget: generous_budget()?,
            inputs: Vec::new(),
        },
    )?;
    submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?;
    let tick = runtime_tick(&runtime)?;
    assert_eq!(tick.uncertain, 1);
    let history = runtime.history(&run)?;
    assert!(
        history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { .. }))
    );
    assert!(
        history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeRetryScheduled { .. }))
    );
    Ok(())
}

#[test]
fn uncertainty_survives_transient_retry_id_failure_and_recovery_retries_later() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BoundaryFailingExecutor::new(test_descriptor()?, 1));
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        executor,
        test_authority(),
        clock.clone(),
        Arc::new(TransientAttemptIdGenerator::new("transient-retry-id", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-transient-retry-id")?,
            ActorRef::new("controller:transient-retry-id")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(
                2,
                vec![ErrorClass::Adapter, ErrorClass::Transport],
                1,
                1_000,
                0,
            )?,
        )?,
    )?;
    let revision = task_revision("workflow-transient-retry-id")?;
    let run = RunId::new("run-transient-retry-id")?;
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
                    ScopeId::new("scope-transient-retry-id")?,
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

    let first_tick = runtime_tick(&runtime)?;
    assert_eq!(first_tick.uncertain, 1);
    let uncertain = runtime.projection(&run)?;
    assert_eq!(uncertain.unresolved_attempts().count(), 1);
    assert!(uncertain.retries().is_empty());
    let history = runtime.history(&run)?;
    assert!(
        history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { .. }))
    );
    assert!(
        !history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeRetryScheduled { .. }))
    );

    let recovered = runtime.recover()?;
    assert_eq!(recovered.retryable, 1);
    let pending = runtime.projection(&run)?;
    assert_eq!(pending.retries().len(), 1);
    assert_eq!(pending.unresolved_attempts().count(), 1);
    clock.advance(1)?;
    assert_eq!(runtime_tick(&runtime)?.completed, 1);
    let completed = runtime.projection(&run)?;
    assert_eq!(
        completed.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(completed.unresolved_attempts().count(), 0);
    Ok(())
}

#[test]
fn uncertainty_is_committed_when_retry_deadline_overflows() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(u64::MAX - 10));
    let executor = Arc::new(BoundaryFailingExecutor::new(test_descriptor()?, 1));
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        executor,
        test_authority(),
        clock,
        Arc::new(SequentialIdGenerator::new("retry-time-overflow", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-retry-time-overflow")?,
            ActorRef::new("controller:retry-time-overflow")?,
            1,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(2, vec![ErrorClass::Adapter], 100, 100, 0)?,
        )?,
    )?;
    let revision = task_revision("workflow-retry-time-overflow")?;
    let run = RunId::new("run-retry-time-overflow")?;
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
                    ScopeId::new("scope-retry-time-overflow")?,
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

    assert_eq!(runtime_tick(&runtime)?.uncertain, 1);
    let projection = runtime.projection(&run)?;
    assert_eq!(projection.unresolved_attempts().count(), 1);
    assert!(projection.retries().is_empty());
    let history = runtime.history(&run)?;
    assert!(
        history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { .. }))
    );
    assert!(
        !history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeRetryScheduled { .. }))
    );
    Ok(())
}

#[test]
fn concurrent_runtime_services_cannot_oversubscribe_one_global_lease_slot() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(AdmissionRaceExecutor::new(test_descriptor()?));
    let make_runtime = |suffix: &str| -> TestResult<Arc<RuntimeService>> {
        Ok(Arc::new(RuntimeService::new_with_authority(
            store.clone(),
            executor.clone(),
            test_authority(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new(
                format!("cross-service-{suffix}"),
                1,
            )?),
            RuntimeConfig::new(
                WorkerId::new(format!("worker-cross-service-{suffix}"))?,
                ActorRef::new(format!("controller:cross-service-{suffix}"))?,
                30_000,
                1,
                SchedulerLimits::new(1, 1, 1, 1)?,
                RetryPolicy::new(1, Vec::new(), 1, 1_000, 0)?,
            )?,
        )?))
    };
    let first_runtime = make_runtime("first")?;
    let second_runtime = make_runtime("second")?;
    let revision = task_revision("workflow-cross-service-admission")?;
    store.put_revision(&revision)?;

    let first_run = RunId::new("run-z-cross-service")?;
    assert_eq!(
        submit_command(
            first_runtime.as_ref(),
            store.as_ref(),
            &first_run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    first_run.clone(),
                    ScopeId::new("scope-z-cross-service")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(
            first_runtime.as_ref(),
            store.as_ref(),
            &first_run,
            RunCommand::StartRun,
        )?,
        CommandDisposition::Accepted
    );
    let first_tick_runtime = first_runtime.clone();
    let first_tick = std::thread::spawn(move || {
        runtime_tick(&first_tick_runtime)
            .map_err(|error| format!("first cross-service tick failed: {error}"))
    });
    executor.wait_for_resolvers(1)?;

    let second_run = RunId::new("run-a-cross-service")?;
    assert_eq!(
        submit_command(
            second_runtime.as_ref(),
            store.as_ref(),
            &second_run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    second_run.clone(),
                    ScopeId::new("scope-a-cross-service")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(
            second_runtime.as_ref(),
            store.as_ref(),
            &second_run,
            RunCommand::StartRun,
        )?,
        CommandDisposition::Accepted
    );
    let second_tick_runtime = second_runtime.clone();
    let second_tick = std::thread::spawn(move || {
        runtime_tick(&second_tick_runtime)
            .map_err(|error| format!("second cross-service tick failed: {error}"))
    });
    executor.wait_for_resolvers(2)?;
    executor.wait_for_execute(1)?;
    assert_eq!(
        store.active_leases(PageSize::new(2)?)?.entries.len(),
        1,
        "two runtime services both committed against one global slot"
    );
    executor.release()?;
    let first_result = first_tick
        .join()
        .map_err(|_| "first cross-service scheduler thread panicked")??;
    let second_result = second_tick
        .join()
        .map_err(|_| "second cross-service scheduler thread panicked")??;
    assert_eq!(
        first_result.dispatched + second_result.dispatched,
        1,
        "stale admission witness allowed two dispatches"
    );
    assert_eq!(first_result.completed + second_result.completed, 1);
    assert_eq!(first_result.deferred + second_result.deferred, 1);
    let granted = first_runtime
        .history(&first_run)?
        .into_iter()
        .chain(second_runtime.history(&second_run)?)
        .filter(|event| matches!(event.kind(), RunEventKind::LeaseGranted { .. }))
        .count();
    assert_eq!(granted, 1);

    for _ in 0..4 {
        if first_runtime.projection(&first_run)?.is_completed()
            && first_runtime.projection(&second_run)?.is_completed()
        {
            break;
        }
        runtime_tick(&first_runtime)?;
    }
    assert!(first_runtime.projection(&first_run)?.is_completed());
    assert!(first_runtime.projection(&second_run)?.is_completed());
    Ok(())
}

#[test]
fn harmless_uncertain_attempt_is_covered_by_exact_terminal_failure_retry() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BoundaryFailingExecutor::new(test_descriptor()?, 1));
    executor.set_script(
        OperationId::new("model.generate")?,
        vec![InvocationEventKind::Terminal {
            terminal: failed_terminal()?,
        }],
    )?;
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        executor,
        test_authority(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("harmless-failure-retry", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-harmless-failure-retry")?,
            ActorRef::new("controller:harmless-failure-retry")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(2, vec![ErrorClass::Adapter], 1, 1_000, 0)?,
        )?,
    )?;
    let revision = task_revision("workflow-harmless-failure-retry")?;
    let run = RunId::new("run-harmless-failure-retry")?;
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
                    ScopeId::new("scope-harmless-failure-retry")?,
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
    runtime_tick(&runtime)?;
    clock.advance(1)?;
    runtime_tick(&runtime)?;
    let projection = runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    let retry_attempt = projection
        .settled_node_executions()
        .values()
        .find_map(|execution| execution.latest_attempt().cloned())
        .ok_or("terminal failure retry anchor is absent")?;
    assert!(projection.attempts().is_empty());
    assert!(
        projection
            .settled_node_executions()
            .values()
            .any(|execution| {
                execution.state() == &NodeExecutionState::Terminal(NodeOutcome::Failed)
                    && execution.attempt_count() == 2
            })
    );
    assert_eq!(projection.unresolved_attempts().count(), 0);
    let history = runtime.history(&run)?;
    let first_attempt = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::NodeRetryScheduled {
                previous_attempt,
                next_attempt,
                ..
            } if next_attempt == &retry_attempt => Some(previous_attempt),
            _ => None,
        })
        .ok_or("durable harmless retry provenance is absent")?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::ExternalOutcomeUncertain { attempt, .. } if attempt == first_attempt
    )));
    Ok(())
}

#[test]
fn exhausted_idempotent_boundary_retries_remain_uncertain_without_fabricated_failure() -> TestResult
{
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BoundaryFailingExecutor::new(
        descriptor_with_model_side_effect("idempotent_write")?,
        2,
    ));
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        executor,
        test_authority(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new(
            "idempotent-boundary-exhausted",
            1,
        )?),
        RuntimeConfig::new(
            WorkerId::new("worker-idempotent-boundary-exhausted")?,
            ActorRef::new("controller:idempotent-boundary-exhausted")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(2, vec![ErrorClass::Adapter], 1, 1_000, 0)?,
        )?,
    )?;
    let revision = task_revision("workflow-idempotent-boundary-exhausted")?;
    let run = RunId::new("run-idempotent-boundary-exhausted")?;
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
                    ScopeId::new("scope-idempotent-boundary-exhausted")?,
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
    runtime_tick(&runtime)?;
    clock.advance(1)?;
    runtime_tick(&runtime)?;
    let projection = runtime.projection(&run)?;
    assert_eq!(projection.lifecycle(), RunLifecycle::Running);
    assert_eq!(projection.attempts().len(), 2);
    assert_eq!(projection.unresolved_attempts().count(), 2);
    assert!(
        projection
            .attempts()
            .values()
            .all(|attempt| attempt.state() == &AttemptState::Uncertain)
    );
    assert!(
        !runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeTerminal { .. }))
    );
    Ok(())
}

#[test]
fn active_retry_cancellation_only_closes_harmless_prior_uncertainty() -> TestResult {
    for (suffix, side_effect, closes) in [
        ("none", "none", true),
        ("idempotent", "idempotent_write", false),
    ] {
        let directory = TempDir::new()?;
        let store = Arc::new(RedbStore::open(directory.path())?);
        let clock = Arc::new(ManualClock::new(NOW));
        let executor = Arc::new(BoundaryThenBlockingExecutor::new(
            descriptor_with_model_side_effect(side_effect)?,
        ));
        let runtime = Arc::new(RuntimeService::new_with_authority(
            store.clone(),
            executor.clone(),
            test_authority(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new(
                format!("active-retry-cancel-{suffix}"),
                1,
            )?),
            RuntimeConfig::new(
                WorkerId::new(format!("worker-active-retry-cancel-{suffix}"))?,
                ActorRef::new(format!("controller:active-retry-cancel-{suffix}"))?,
                30_000,
                32,
                SchedulerLimits::new(8, 4, 2, 4)?,
                RetryPolicy::new(2, vec![ErrorClass::Adapter], 1, 1_000, 0)?,
            )?,
        )?);
        let revision = task_revision(&format!("workflow-active-retry-cancel-{suffix}"))?;
        let run = RunId::new(format!("run-active-retry-cancel-{suffix}"))?;
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
                        ScopeId::new(format!("scope-active-retry-cancel-{suffix}"))?,
                    ),
                    workspace_budget: generous_budget()?,
                    inputs: Vec::new(),
                },
            )?,
            CommandDisposition::Accepted
        );
        assert_eq!(
            submit_command(runtime.as_ref(), store.as_ref(), &run, RunCommand::StartRun,)?,
            CommandDisposition::Accepted
        );
        runtime_tick(&runtime)?;
        clock.advance(1)?;
        let retry_runtime = runtime.clone();
        let retry = std::thread::spawn(move || {
            runtime_tick(&retry_runtime)
                .map_err(|error| format!("active retry tick failed: {error}"))
        });
        executor.wait_until_entered()?;
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
        retry
            .join()
            .map_err(|_| "active retry dispatch thread panicked")??;
        for _ in 0..4 {
            if runtime.projection(&run)?.is_completed() {
                break;
            }
            runtime_tick(&runtime)?;
        }
        let projection = runtime.projection(&run)?;
        let history = runtime.history(&run)?;
        let retry_id = history
            .iter()
            .find_map(|event| match event.kind() {
                RunEventKind::NodeRetryScheduled { next_attempt, .. } => Some(next_attempt.clone()),
                _ => None,
            })
            .ok_or("cancelled retry identity is absent from history")?;
        if closes {
            assert!(projection.attempts().is_empty());
            assert_eq!(projection.unresolved_attempts().count(), 0);
            assert_eq!(
                projection.lifecycle(),
                RunLifecycle::Terminal(RunOutcome::Cancelled)
            );
        } else {
            let first = projection
                .attempts()
                .values()
                .find(|attempt| attempt.attempt_number() == 1)
                .ok_or("unresolved idempotent first attempt is absent")?;
            assert_eq!(first.state(), &AttemptState::Uncertain);
            assert!(first.terminal().is_none());
            assert!(first.obligation().is_some());
            assert_eq!(projection.unresolved_attempts().count(), 1);
            assert_eq!(projection.lifecycle(), RunLifecycle::Cancelling);
        }
        assert!(history.iter().any(|event| matches!(
            event.kind(),
            RunEventKind::NodeRetryScheduled { next_attempt, .. } if next_attempt == &retry_id
        )));
    }
    Ok(())
}

#[path = "retry_recovery/restart.rs"]
mod restart;
