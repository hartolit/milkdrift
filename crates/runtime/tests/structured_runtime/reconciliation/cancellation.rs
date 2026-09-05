use super::*;

#[test]
fn run_cancellation_after_lease_prevents_external_start() -> TestResult {
    let directory = TempDir::new()?;
    let executor = Arc::new(OperationCountingExecutor::new(test_descriptor()?));
    let (store, _clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "cancel-after-lease",
        "cancel-after-lease",
        NOW,
        64,
        executor.clone(),
    )?;
    let revision = task_revision("workflow-cancel-after-lease")?;
    let run = RunId::new("run-cancel-after-lease")?;
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
                    ScopeId::new("scope-cancel-after-lease")?,
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
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::RequestCancellation,
        )?,
        CommandDisposition::Accepted
    );
    assert!(runtime.claim_effects(PageSize::new(1)?)?.is_empty());
    assert_eq!(executor.calls("model.generate")?, 0);
    assert!(
        !runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
    );
    Ok(())
}

#[test]
fn run_cancellation_after_effect_claim_prevents_final_adapter_entry() -> TestResult {
    let directory = TempDir::new()?;
    let executor = Arc::new(OperationCountingExecutor::new(test_descriptor()?));
    let (store, _clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "cancel-after-claim",
        "cancel-after-claim",
        NOW,
        64,
        executor.clone(),
    )?;
    let revision = task_revision("workflow-cancel-after-claim")?;
    let run = RunId::new("run-cancel-after-claim")?;
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
                    ScopeId::new("scope-cancel-after-claim")?,
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
    let mut actions = runtime.claim_execution_effects(PageSize::new(1)?)?;
    assert_eq!(actions.len(), 1);
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::RequestCancellation,
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        runtime.execute_effect(actions.remove(0))?,
        EffectExecutionResult::Completed { observations: 0 }
    );
    assert_eq!(executor.calls("model.generate")?, 0);
    assert!(!runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. }
    )));
    Ok(())
}

#[test]
fn reconciliation_cancellation_after_effect_claim_prevents_the_old_external_start() -> TestResult {
    let directory = TempDir::new()?;
    let executor = Arc::new(OperationCountingExecutor::new(test_descriptor()?));
    let (store, _clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "reconcile-cancel-after-lease",
        "reconcile-cancel-after-lease",
        NOW,
        64,
        executor.clone(),
    )?;
    let old = task_revision("workflow-reconcile-cancel-after-lease")?;
    let new = revised_task_revision(&old, "model.fail")?;
    let run = RunId::new("run-reconcile-cancel-after-lease")?;
    store.put_revision(&old)?;
    store.put_revision(&new)?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: old.semantic().workflow().clone(),
                revision: old.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-reconcile-cancel-after-lease")?,
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
    let mut actions = runtime.claim_execution_effects(PageSize::new(1)?)?;
    assert_eq!(actions.len(), 1);
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconcile-cancel-after-lease")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::CancelAndRestartSafeWork,
            },
        )?,
        CommandDisposition::Accepted
    );
    let plan = runtime
        .projection(&run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("cancel-after-lease reconciliation plan is absent")?
        .plan()
        .clone();
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::ApplyReconciliation { plan },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        runtime.execute_effect(actions.remove(0))?,
        EffectExecutionResult::Completed { observations: 0 }
    );
    assert_eq!(executor.calls("model.generate")?, 0);
    assert!(!runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. }
    )));
    Ok(())
}

#[test]
fn cancel_and_restart_adoption_creates_one_replacement_after_confirmed_cancellation() -> TestResult
{
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new_with_authority(
        store.clone(),
        executor.clone(),
        test_authority(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("cancel-restart-adoption", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-cancel-restart-adoption")?,
            ActorRef::new("controller:cancel-restart-adoption")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?);
    let old = task_revision("workflow-cancel-restart-adoption")?;
    let new = revised_task_revision(&old, "model.fail")?;
    let run = RunId::new("run-cancel-restart-adoption")?;
    store.put_revision(&old)?;
    store.put_revision(&new)?;
    let create = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("create cancel-and-restart adoption run")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: old.semantic().workflow().clone(),
            revision: old.id().clone(),
            root_scope: WorkspaceScope::run_root(
                run.clone(),
                ScopeId::new("scope-cancel-restart-adoption")?,
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
        Reason::new("start cancel-and-restart adoption run")?,
        Vec::new(),
        RunCommand::StartRun,
    )?;
    runtime.handle_authorized_command(&start, &test_authority_claim()?)?;

    let dispatch_runtime = runtime.clone();
    let dispatch = std::thread::spawn(move || {
        runtime_tick(&dispatch_runtime).map_err(|error| error.to_string())
    });
    executor.wait_until_entered()?;
    let request = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("adopt a changed active safe task")?,
        Vec::new(),
        RunCommand::RequestRevisionAdoption {
            reconciliation: ReconciliationId::new("reconciliation-cancel-restart")?,
            revision: new.id().clone(),
            policy: ReconciliationPolicy::CancelAndRestartSafeWork,
        },
    )?;
    assert_eq!(
        runtime
            .handle_authorized_command(&request, &test_authority_claim()?)?
            .result()
            .disposition(),
        CommandDisposition::Accepted
    );
    let plan = runtime
        .projection(&run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("cancel-and-restart plan is absent")?
        .plan()
        .clone();
    let apply = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("apply cancel-and-restart plan")?,
        Vec::new(),
        RunCommand::ApplyReconciliation { plan },
    )?;
    assert_eq!(
        runtime
            .handle_authorized_command(&apply, &test_authority_claim()?)?
            .result()
            .disposition(),
        CommandDisposition::Accepted
    );

    runtime_tick(&runtime)?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    executor.release()?;
    dispatch
        .join()
        .map_err(|_| "cancel-and-restart dispatch thread panicked")?
        .map_err(|error| format!("cancel-and-restart dispatch failed: {error}"))?;
    for _ in 0..4 {
        if runtime.projection(&run)?.is_completed() {
            break;
        }
        runtime_tick(&runtime)?;
    }

    let projection = runtime.projection(&run)?;
    assert_eq!(projection.revision(), Some(new.id()));
    let work = NodeId::new("work")?;
    let executions: Vec<_> = projection.executions_for_node(&work).collect();
    assert_eq!(executions.len(), 1);
    assert_eq!(
        executions[0].state(),
        &NodeExecutionState::Terminal(milkdrift_persistence::NodeOutcome::Succeeded)
    );
    let history = runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeTerminal {
            outcome: milkdrift_persistence::NodeOutcome::Cancelled,
            ..
        }
    )));
    assert_eq!(
        runtime
            .history(&run)?
            .iter()
            .filter(|event| matches!(
                event.kind(),
                RunEventKind::ReconciliationCancellationRequested { .. }
            ))
            .count(),
        1
    );
    Ok(())
}
