//! Retry recovery integration scenarios.

use super::*;

#[test]
fn crash_after_durable_lease_recovers_only_after_expiry_and_retries_once() -> TestResult {
    let directory = TempDir::new()?;
    let revision = task_revision("workflow-crash-after-lease")?;
    let run = RunId::new("run-crash-after-lease")?;

    let (original_attempt, original_lease) = {
        let store = Arc::new(RedbStore::open(directory.path())?);
        store.put_revision(&revision)?;
        let runtime = recovery_service(
            store.clone(),
            Arc::new(ManualClock::new(NOW)),
            Arc::new(DeterministicExecutor::new(test_descriptor()?)),
            "crash-after-lease",
        )?;
        let root_scope =
            WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-crash-after-lease")?);
        let create = runtime.command(
            run.clone(),
            ActorRef::new("human:structured-runtime-test")?,
            store.head(&run)?,
            Reason::new("create crash-boundary run")?,
            Vec::new(),
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope,
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
            Reason::new("start crash-boundary run")?,
            Vec::new(),
            RunCommand::StartRun,
        )?;
        runtime.handle_command(&start)?;

        let scheduled = runtime.scheduler_tick()?;
        assert_eq!(scheduled.dispatched, 1);
        let stranded = runtime.projection(&run)?;
        assert_eq!(stranded.attempts().len(), 1);
        assert_eq!(stranded.leases().len(), 1);
        let attempt = stranded
            .attempts()
            .values()
            .next()
            .ok_or("leased crash-boundary attempt is absent")?;
        let lease = stranded
            .leases()
            .values()
            .next()
            .ok_or("leased crash-boundary ownership is absent")?;
        assert_eq!(attempt.state(), &AttemptState::Leased);
        assert_eq!(lease.attempt(), attempt.attempt());
        assert!(lease.is_active());
        let history = runtime.history(&run)?;
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::NodeScheduled { .. }))
                .count(),
            1
        );
        assert!(
            !history
                .iter()
                .any(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
        );
        (attempt.attempt().clone(), lease.lease().clone())
    };

    {
        let store = Arc::new(RedbStore::open(directory.path())?);
        let runtime = recovery_service(
            store,
            Arc::new(ManualClock::new(NOW + 50)),
            Arc::new(DeterministicExecutor::new(test_descriptor()?)),
            "recover-before-expiry",
        )?;
        let preserved = runtime.projection(&run)?;
        assert_eq!(preserved.attempts().len(), 1);
        let attempt = preserved
            .attempts()
            .get(&original_attempt)
            .ok_or("unexpired leased attempt is absent")?;
        assert_eq!(attempt.state(), &AttemptState::Leased);
        assert!(attempt.recovery().is_empty());
        assert_eq!(
            preserved
                .leases()
                .get(&original_lease)
                .map(|lease| lease.state()),
            Some(&LeaseState::Active)
        );
        let history = runtime.history(&run)?;
        assert!(!history.iter().any(|event| matches!(
            event.kind(),
            RunEventKind::LeaseExpired { .. }
                | RunEventKind::RecoveryClassified { .. }
                | RunEventKind::NodeReLeased { .. }
        )));
        let scheduled = runtime.scheduler_tick()?;
        assert_eq!(scheduled.dispatched, 0);
        assert_eq!(scheduled.completed, 0);
        assert!(runtime.claim_effects(PageSize::new(1)?)?.is_empty());
    }

    {
        let store = Arc::new(RedbStore::open(directory.path())?);
        let runtime = recovery_service(
            store,
            Arc::new(ManualClock::new(NOW + 101)),
            Arc::new(DeterministicExecutor::new(test_descriptor()?)),
            "recover-after-expiry",
        )?;
        let recovered = runtime.projection(&run)?;
        assert_eq!(recovered.attempts().len(), 1);
        let attempt = recovered
            .attempts()
            .get(&original_attempt)
            .ok_or("recovered leased attempt is absent")?;
        assert_eq!(attempt.state(), &AttemptState::Leased);
        assert_eq!(attempt.attempt_number(), 1);
        assert!(attempt.terminal().is_none());
        assert!(attempt.obligation().is_none());
        assert_eq!(attempt.recovery().len(), 1);
        assert_eq!(attempt.recovery()[0].lease(), Some(&original_lease));
        assert_eq!(
            attempt.recovery()[0].classification(),
            RecoveryClassification::NotStarted
        );
        assert_eq!(attempt.leases().len(), 2);
        let replacement_lease = attempt
            .leases()
            .last()
            .ok_or("replacement lease is absent")?
            .clone();
        assert_ne!(replacement_lease, original_lease);
        assert_eq!(
            recovered
                .leases()
                .get(&original_lease)
                .map(|lease| lease.state()),
            Some(&LeaseState::Superseded(replacement_lease.clone()))
        );
        assert_eq!(
            recovered
                .leases()
                .get(&replacement_lease)
                .map(|lease| lease.state()),
            Some(&LeaseState::Active)
        );

        let history = runtime.history(&run)?;
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    RunEventKind::LeaseExpired {
                        lease,
                        classification: RecoveryClassification::NotStarted,
                    } if lease == &original_lease
                ))
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    RunEventKind::RecoveryClassified {
                        attempt,
                        lease: Some(lease),
                        classification: RecoveryClassification::NotStarted,
                        ..
                    } if attempt == &original_attempt && lease == &original_lease
                ))
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    RunEventKind::NodeReLeased {
                        previous_lease,
                        lease,
                        attempt,
                        ..
                    } if previous_lease == &original_lease
                        && lease == &replacement_lease
                        && attempt == &original_attempt
                ))
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::NodeScheduled { .. }))
                .count(),
            1
        );
        assert!(!history.iter().any(|event| matches!(
            event.kind(),
            RunEventKind::NodeStarted { .. }
                | RunEventKind::NodeRetryScheduled { .. }
                | RunEventKind::ExternalOutcomeUncertain { .. }
        )));

        let scheduled = runtime.scheduler_tick()?;
        assert_eq!(scheduled.dispatched, 0);
        assert_eq!(scheduled.completed, 0);
        let actions = runtime.claim_effects(PageSize::new(1)?)?;
        assert_eq!(actions.len(), 1);
        let dispatch = match &actions[0] {
            EffectAction::Execute(dispatch) => dispatch,
            EffectAction::Cancel(_) => {
                return Err("recovered unstarted attempt claimed cancellation".into());
            }
        };
        assert_eq!(dispatch.attempt(), &original_attempt);
        assert_eq!(dispatch.lease(), &replacement_lease);
        assert_eq!(
            runtime.execute_effect(&actions[0])?,
            EffectExecutionResult::Completed { observations: 1 }
        );

        let completed = runtime.projection(&run)?;
        assert_eq!(
            completed.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Succeeded)
        );
        assert!(completed.attempts().is_empty());
        let execution = completed
            .settled_node_executions()
            .values()
            .find(|execution| execution.latest_attempt() == Some(&original_attempt))
            .ok_or("completed reassigned execution summary is absent")?;
        assert_eq!(
            execution.state(),
            &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
        );
        assert!(
            !completed.leases().contains_key(&replacement_lease),
            "completed lease detail belongs to journal history, not the active projection"
        );
        let history = runtime.history(&run)?;
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::NodeScheduled { .. }))
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    RunEventKind::NodeStarted { attempt, .. } if attempt == &original_attempt
                ))
                .count(),
            1
        );
        assert!(
            !history
                .iter()
                .any(|event| matches!(event.kind(), RunEventKind::NodeRetryScheduled { .. }))
        );
    }
    Ok(())
}

#[test]
fn crash_after_durable_start_recovers_as_uncertain_without_duplicate_dispatch_history() -> TestResult
{
    let directory = TempDir::new()?;
    let revision = task_revision("workflow-crash-after-start")?;
    let run = RunId::new("run-crash-after-start")?;
    let descriptor = descriptor_with_model_side_effect("non_idempotent_write")?;

    let (original_attempt, original_lease) = {
        let store = Arc::new(RedbStore::open(directory.path())?);
        store.put_revision(&revision)?;
        let runtime = recovery_service(
            store.clone(),
            Arc::new(ManualClock::new(NOW)),
            Arc::new(PanickingExecutor {
                resolver: DeterministicExecutor::new(descriptor.clone()),
            }),
            "crash-after-start",
        )?;
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-crash-after-start")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?;
        submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?;

        assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
        let actions = runtime.claim_effects(PageSize::new(1)?)?;
        assert_eq!(actions.len(), 1);
        let action = actions
            .into_iter()
            .next()
            .ok_or("started crash fixture did not claim its invocation")?;
        let (attempt, lease) = match &action {
            EffectAction::Execute(dispatch) => {
                (dispatch.attempt().clone(), dispatch.lease().clone())
            }
            EffectAction::Cancel(_dispatch) => {
                return Err(
                    "started crash fixture claimed cancellation instead of execution".into(),
                );
            }
        };

        let started = runtime.projection(&run)?;
        assert_eq!(
            started
                .attempts()
                .get(&attempt)
                .map(|attempt| attempt.state()),
            Some(&AttemptState::Running)
        );
        let history = runtime.history(&run)?;
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::NodeScheduled { .. }))
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
                .count(),
            1
        );

        let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.execute_effect(&action)
        }));
        assert!(
            crash.is_err(),
            "panicking executor did not fail after durable invocation start"
        );
        let stranded = runtime.projection(&run)?;
        assert_eq!(
            stranded
                .attempts()
                .get(&attempt)
                .map(|attempt| attempt.state()),
            Some(&AttemptState::Running)
        );
        assert!(
            !runtime
                .history(&run)?
                .iter()
                .any(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { .. })),
            "process loss before boundary classification must not fabricate uncertainty in-process"
        );
        (attempt, lease)
    };

    {
        let store = Arc::new(RedbStore::open(directory.path())?);
        let runtime = recovery_service(
            store,
            Arc::new(ManualClock::new(NOW + 101)),
            Arc::new(DeterministicExecutor::new(descriptor)),
            "recover-after-start",
        )?;

        let recovered = runtime.projection(&run)?;
        assert_eq!(recovered.attempts().len(), 1);
        let attempt = recovered
            .attempts()
            .get(&original_attempt)
            .ok_or("started attempt was not retained across recovery")?;
        assert_eq!(attempt.state(), &AttemptState::Uncertain);
        assert!(attempt.terminal().is_none());
        assert!(attempt.obligation().is_some());
        assert_eq!(attempt.recovery().len(), 1);
        assert_eq!(attempt.recovery()[0].lease(), Some(&original_lease));
        assert_eq!(
            attempt.recovery()[0].classification(),
            RecoveryClassification::Uncertain
        );
        assert_eq!(
            recovered
                .leases()
                .get(&original_lease)
                .map(|lease| lease.state()),
            Some(&LeaseState::Expired(RecoveryClassification::Uncertain))
        );

        let history = runtime.history(&run)?;
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::NodeScheduled { .. }))
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    RunEventKind::LeaseExpired {
                        lease,
                        classification: RecoveryClassification::Uncertain,
                    } if lease == &original_lease
                ))
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    RunEventKind::RecoveryClassified {
                        attempt,
                        lease: Some(lease),
                        classification: RecoveryClassification::Uncertain,
                        ..
                    } if attempt == &original_attempt && lease == &original_lease
                ))
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    RunEventKind::ExternalOutcomeUncertain { attempt, .. }
                        if attempt == &original_attempt
                ))
                .count(),
            1
        );
        assert!(!history.iter().any(|event| matches!(
            event.kind(),
            RunEventKind::NodeRetryScheduled { .. }
                | RunEventKind::NodeReLeased { .. }
                | RunEventKind::NodeTerminal { .. }
        )));
        assert_eq!(runtime.scheduler_tick()?.dispatched, 0);
        assert!(runtime.claim_effects(PageSize::new(1)?)?.is_empty());
    }
    Ok(())
}

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
    let runtime = RuntimeService::new(
        store.clone(),
        executor.clone(),
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

    let first_tick = runtime.tick()?;
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
    let retry_tick = runtime.tick()?;
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
fn uncertainty_survives_transient_retry_id_failure_and_recovery_retries_later() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BoundaryFailingExecutor::new(test_descriptor()?, 1));
    let runtime = RuntimeService::new(
        store.clone(),
        executor,
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

    let first_tick = runtime.tick()?;
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
    assert_eq!(runtime.tick()?.completed, 1);
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
    let runtime = RuntimeService::new(
        store.clone(),
        executor,
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

    assert_eq!(runtime.tick()?.uncertain, 1);
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
        Ok(Arc::new(RuntimeService::new(
            store.clone(),
            executor.clone(),
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
        first_tick_runtime
            .tick()
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
        second_tick_runtime
            .tick()
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
        first_runtime.tick()?;
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
    let runtime = RuntimeService::new(
        store.clone(),
        executor,
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
    runtime.tick()?;
    clock.advance(1)?;
    runtime.tick()?;
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
    let runtime = RuntimeService::new(
        store.clone(),
        executor,
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
    runtime.tick()?;
    clock.advance(1)?;
    runtime.tick()?;
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
        let runtime = Arc::new(RuntimeService::new(
            store.clone(),
            executor.clone(),
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
        runtime.tick()?;
        clock.advance(1)?;
        let retry_runtime = runtime.clone();
        let retry = std::thread::spawn(move || {
            retry_runtime
                .tick()
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
        runtime.tick()?;
        assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
        executor.release()?;
        retry
            .join()
            .map_err(|_| "active retry dispatch thread panicked")??;
        for _ in 0..4 {
            if runtime.projection(&run)?.is_completed() {
                break;
            }
            runtime.tick()?;
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
