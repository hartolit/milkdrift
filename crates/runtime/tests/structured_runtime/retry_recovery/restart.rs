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
            Reason::new("start crash-boundary run")?,
            Vec::new(),
            RunCommand::StartRun,
        )?;
        runtime.handle_authorized_command(&start, &test_authority_claim()?)?;

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
        let action = actions
            .into_iter()
            .next()
            .ok_or("recovered unstarted attempt action is absent")?;
        let dispatch = match &action {
            EffectAction::Execute(dispatch) => dispatch,
            EffectAction::Cancel(_) => {
                return Err("recovered unstarted attempt claimed cancellation".into());
            }
        };
        assert_eq!(dispatch.attempt(), &original_attempt);
        assert_eq!(dispatch.lease(), &replacement_lease);
        assert_eq!(
            runtime.execute_effect(action)?,
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
            runtime.execute_effect(action)
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
fn crash_after_idempotent_start_recovers_as_retryable_with_stable_key() -> TestResult {
    let directory = TempDir::new()?;
    let revision = task_revision("workflow-crash-idempotent-start")?;
    let run = RunId::new("run-crash-idempotent-start")?;
    let descriptor = descriptor_with_model_side_effect("idempotent_write")?;

    let (original_attempt, original_lease) = {
        let store = Arc::new(RedbStore::open(directory.path())?);
        store.put_revision(&revision)?;
        let runtime = recovery_service(
            store.clone(),
            Arc::new(ManualClock::new(NOW)),
            Arc::new(PanickingExecutor {
                resolver: DeterministicExecutor::new(descriptor.clone()),
            }),
            "crash-idempotent-start",
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
                    ScopeId::new("scope-crash-idempotent-start")?,
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
            .ok_or("idempotent crash fixture did not claim its invocation")?;
        let dispatch = match &action {
            EffectAction::Execute(dispatch) => dispatch,
            EffectAction::Cancel(_) => return Err("expected idempotent execution effect".into()),
        };
        assert!(dispatch.request().idempotency_key().is_some());
        let attempt = dispatch.attempt().clone();
        let lease = dispatch.lease().clone();
        let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.execute_effect(action)
        }));
        assert!(crash.is_err(), "idempotent crash fixture did not panic");
        (attempt, lease)
    };

    let store = Arc::new(RedbStore::open(directory.path())?);
    let runtime = recovery_service(
        store,
        Arc::new(ManualClock::new(NOW + 101)),
        Arc::new(DeterministicExecutor::new(descriptor)),
        "recover-idempotent-start",
    )?;
    let history = runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(
                event.kind(),
                RunEventKind::LeaseExpired {
                    lease,
                    classification: RecoveryClassification::Retryable,
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
                    classification: RecoveryClassification::Retryable,
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
                RunEventKind::NodeRetryScheduled { previous_attempt, .. }
                    if previous_attempt == &original_attempt
            ))
            .count(),
        1
    );
    assert!(!history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::RecoveryClassified {
            attempt,
            classification: RecoveryClassification::Uncertain,
            ..
        } if attempt == &original_attempt
    )));
    Ok(())
}
