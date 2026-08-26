//! Lifecycle integration scenarios.

use super::*;

#[derive(Clone, Copy)]
enum StartupDiscoveryFault {
    None,
    FailOnce,
    Stall,
}

struct StartupProbeStore {
    inner: Arc<RedbStore>,
    discovery_fault: StartupDiscoveryFault,
    discovery_calls: AtomicUsize,
    integrity_scan_calls: AtomicUsize,
    artifact_verification_requests: AtomicUsize,
}

impl StartupProbeStore {
    fn new(inner: Arc<RedbStore>, discovery_fault: StartupDiscoveryFault) -> Self {
        Self {
            inner,
            discovery_fault,
            discovery_calls: AtomicUsize::new(0),
            integrity_scan_calls: AtomicUsize::new(0),
            artifact_verification_requests: AtomicUsize::new(0),
        }
    }

    fn discovery_calls(&self) -> usize {
        self.discovery_calls.load(Ordering::SeqCst)
    }

    fn integrity_scan_calls(&self) -> usize {
        self.integrity_scan_calls.load(Ordering::SeqCst)
    }

    fn artifact_verification_requests(&self) -> usize {
        self.artifact_verification_requests.load(Ordering::SeqCst)
    }
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

impl RevisionStore for StartupProbeStore {
    forward_store_methods! {
        fn put_revision(
            &self,
            revision: &BlueprintRevision,
        ) -> PersistenceResult<milkdrift_persistence::ImmutableRevisionPut>;
        fn revision(
            &self,
            revision: &milkdrift_blueprint::RevisionId,
        ) -> PersistenceResult<Option<BlueprintRevision>>;
        fn revision_summary(
            &self,
            revision: &milkdrift_blueprint::RevisionId,
        ) -> PersistenceResult<Option<milkdrift_persistence::RevisionSummary>>;
        fn revisions_by_content(
            &self,
            digest: &milkdrift_blueprint::ContentDigest,
            limit: PageSize,
        ) -> PersistenceResult<Vec<milkdrift_persistence::RevisionSummary>>;
    }
}

impl RunJournal for StartupProbeStore {
    forward_store_methods! {
        fn commit_command(
            &self,
            request: &AtomicRunCommitRequest,
        ) -> PersistenceResult<milkdrift_persistence::AtomicRunCommitOutcome>;
        fn head(
            &self,
            run: &RunId,
        ) -> PersistenceResult<milkdrift_persistence::RunSequence>;
        fn command_result(
            &self,
            run: &RunId,
            command: &CommandId,
        ) -> PersistenceResult<Option<CommandResultDocument>>;
    }
}

impl RunQueryStore for StartupProbeStore {
    forward_store_methods! {
        fn events(
            &self,
            query: &milkdrift_persistence::EventPageQuery,
        ) -> PersistenceResult<milkdrift_persistence::EventPage>;
        fn signal_receipt(
            &self,
            run: &RunId,
            signal: &SignalId,
        ) -> PersistenceResult<Option<RunEventEnvelope>>;
        fn run_summary(
            &self,
            run: &RunId,
        ) -> PersistenceResult<Option<RunSummaryIndex>>;
        fn run_summaries(
            &self,
            query: &milkdrift_persistence::RunSummaryPageQuery,
        ) -> PersistenceResult<milkdrift_persistence::RunSummaryPage>;
        fn runnable_page(
            &self,
            eligible_through: TimestampMillis,
            cursor: Option<&milkdrift_persistence::RunnableCursor>,
            limit: PageSize,
        ) -> PersistenceResult<milkdrift_persistence::RunnablePage>;
        fn active_leases(
            &self,
            limit: PageSize,
        ) -> PersistenceResult<milkdrift_persistence::ActiveLeaseSnapshot>;
        fn due_timers(
            &self,
            due_through: TimestampMillis,
            limit: PageSize,
        ) -> PersistenceResult<Vec<milkdrift_persistence::TimerIndexEntry>>;
        fn expired_leases(
            &self,
            expired_through: TimestampMillis,
            limit: PageSize,
        ) -> PersistenceResult<Vec<milkdrift_persistence::LeaseIndexEntry>>;
    }

    fn nonterminal_run_page(
        &self,
        cursor: Option<&milkdrift_persistence::RunSummaryCursor>,
        limit: PageSize,
    ) -> PersistenceResult<milkdrift_persistence::RunSummaryPage> {
        let call = self.discovery_calls.fetch_add(1, Ordering::SeqCst);
        match self.discovery_fault {
            StartupDiscoveryFault::FailOnce if call == 0 => {
                Err(milkdrift_persistence::PersistenceError::Storage {
                    class: milkdrift_persistence::StorageFailureClass::Unavailable,
                    message: "scripted transient nonterminal discovery failure".to_owned(),
                })
            }
            StartupDiscoveryFault::Stall => {
                let anchor = RunId::new("startup-stalled-cursor-anchor").map_err(|error| {
                    milkdrift_persistence::PersistenceError::InvalidCursor(error.to_string())
                })?;
                let next = cursor.cloned().unwrap_or_else(|| {
                    milkdrift_persistence::RunSummaryCursor::for_nonterminal(anchor)
                });
                Ok(milkdrift_persistence::RunSummaryPage {
                    runs: Vec::new(),
                    next: Some(next),
                })
            }
            StartupDiscoveryFault::None | StartupDiscoveryFault::FailOnce => {
                self.inner.nonterminal_run_page(cursor, limit)
            }
        }
    }
}

impl milkdrift_persistence::RunDiscoveryIntegrityStore for StartupProbeStore {
    fn validate_run_discovery(
        &self,
        run: &RunId,
        through_sequence: milkdrift_persistence::RunSequence,
        runnable: &[milkdrift_persistence::RunnableIndexEntry],
        timers: &[milkdrift_persistence::TimerIndexEntry],
        leases: &[milkdrift_persistence::LeaseIndexEntry],
    ) -> PersistenceResult<()> {
        milkdrift_persistence::RunDiscoveryIntegrityStore::validate_run_discovery(
            self.inner.as_ref(),
            run,
            through_sequence,
            runnable,
            timers,
            leases,
        )
    }
}

impl WorkspaceStore for StartupProbeStore {
    forward_store_methods! {
        fn workspace_usage(
            &self,
            run: &RunId,
        ) -> PersistenceResult<WorkspaceUsage>;
        fn scope(
            &self,
            run: &RunId,
            scope: &ScopeId,
        ) -> PersistenceResult<Option<WorkspaceScope>>;
        fn value(
            &self,
            reference: &WorkspaceValueReference,
        ) -> PersistenceResult<Option<WorkspaceValueEntry>>;
        fn latest_value(
            &self,
            scope: &ScopeReference,
            key: &ValueKey,
        ) -> PersistenceResult<Option<WorkspaceValueEntry>>;
        fn scope_lineage(
            &self,
            leaf: &ScopeReference,
        ) -> PersistenceResult<Vec<WorkspaceScope>>;
    }
}

impl SnapshotStore for StartupProbeStore {
    forward_store_methods! {
        fn history_digest(
            &self,
            run: &RunId,
            through: milkdrift_persistence::RunSequence,
        ) -> PersistenceResult<milkdrift_persistence::IntegrityDigest>;
        fn put_snapshot(
            &self,
            snapshot: &milkdrift_persistence::SnapshotDocument,
        ) -> PersistenceResult<()>;
        fn latest_snapshot(
            &self,
            run: &RunId,
        ) -> PersistenceResult<milkdrift_persistence::SnapshotLoad>;
        fn discard_snapshot(
            &self,
            run: &RunId,
            snapshot: &milkdrift_persistence::SnapshotId,
        ) -> PersistenceResult<()>;
    }
}

impl ArtifactStore for StartupProbeStore {
    forward_store_methods! {
        fn begin_publication(
            &self,
            request: &BeginArtifactPublication,
        ) -> PersistenceResult<milkdrift_persistence::BeginArtifactOutcome>;
        fn write_chunk(
            &self,
            publication: &ArtifactPublicationId,
            offset: u64,
            bytes: &[u8],
        ) -> PersistenceResult<milkdrift_persistence::ArtifactWriteProgress>;
        fn commit_publication(
            &self,
            publication: &ArtifactPublicationId,
        ) -> PersistenceResult<milkdrift_persistence::CommitArtifactOutcome>;
        fn abort_publication(
            &self,
            publication: &ArtifactPublicationId,
        ) -> PersistenceResult<()>;
        fn metadata(
            &self,
            artifact: &ArtifactId,
        ) -> PersistenceResult<Option<ArtifactMetadata>>;
        fn is_referenced_by_run(
            &self,
            run: &RunId,
            reference: &milkdrift_workspace::ArtifactReference,
        ) -> PersistenceResult<bool>;
        fn read_chunk(
            &self,
            request: &milkdrift_persistence::ArtifactReadRequest,
        ) -> PersistenceResult<milkdrift_persistence::ArtifactReadChunk>;
        fn cleanup_orphans(
            &self,
            request: milkdrift_persistence::OrphanCleanupRequest,
        ) -> PersistenceResult<milkdrift_persistence::OrphanCleanupResult>;
    }

    fn is_committed(
        &self,
        reference: &milkdrift_workspace::ArtifactReference,
    ) -> PersistenceResult<bool> {
        self.artifact_verification_requests
            .fetch_add(1, Ordering::SeqCst);
        self.inner.is_committed(reference)
    }
}

impl StorageAdmin for StartupProbeStore {
    forward_store_methods! {
        fn schema_info(
            &self,
        ) -> PersistenceResult<milkdrift_persistence::StorageSchemaInfo>;
        fn health(
            &self,
            observed_at: TimestampMillis,
        ) -> PersistenceResult<milkdrift_persistence::StorageHealth>;
    }

    fn scan_integrity(
        &self,
        request: IntegrityScanRequest,
    ) -> PersistenceResult<milkdrift_persistence::IntegrityScanResult> {
        self.integrity_scan_calls.fetch_add(1, Ordering::SeqCst);
        if request.verify_artifact_content {
            self.artifact_verification_requests
                .fetch_add(1, Ordering::SeqCst);
        }
        self.inner.scan_integrity(request)
    }
}

fn runtime_for_startup_probe(
    store: Arc<StartupProbeStore>,
    prefix: &str,
) -> TestResult<RuntimeService> {
    Ok(RuntimeService::open_closed_with_authority(
        store,
        Arc::new(DeterministicExecutor::new(test_descriptor()?)),
        test_authority(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new(prefix, 1)?),
        RuntimeConfig::new(
            WorkerId::new(format!("worker-{prefix}"))?,
            ActorRef::new(format!("controller:{prefix}"))?,
            30_000,
            8,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?)
}

#[test]
fn startup_does_not_invoke_the_administrative_integrity_scanner() -> TestResult {
    let directory = TempDir::new()?;
    let probe = Arc::new(StartupProbeStore::new(
        Arc::new(RedbStore::open(directory.path())?),
        StartupDiscoveryFault::None,
    ));
    let runtime = runtime_for_startup_probe(probe.clone(), "startup-no-scrub")?;

    assert_eq!(probe.integrity_scan_calls(), 0);
    assert_eq!(probe.artifact_verification_requests(), 0);
    runtime.initialize_startup()?;
    assert_eq!(
        runtime.startup_state(),
        RuntimeStartupState::RecoveryCompleted
    );
    assert!(runtime.is_accepting_admission());
    assert!(probe.discovery_calls() > 0);
    assert_eq!(probe.integrity_scan_calls(), 0);
    assert_eq!(probe.artifact_verification_requests(), 0);
    Ok(())
}

#[test]
fn startup_retry_is_idempotent_after_a_transient_discovery_failure() -> TestResult {
    let directory = TempDir::new()?;
    let probe = Arc::new(StartupProbeStore::new(
        Arc::new(RedbStore::open(directory.path())?),
        StartupDiscoveryFault::FailOnce,
    ));
    let runtime = runtime_for_startup_probe(probe.clone(), "startup-transient-retry")?;

    let Err(error) = runtime.initialize_startup() else {
        return Err("scripted transient startup failure was not surfaced".into());
    };
    assert!(matches!(
        error,
        RuntimeError::Persistence(milkdrift_persistence::PersistenceError::Storage {
            class: milkdrift_persistence::StorageFailureClass::Unavailable,
            ..
        })
    ));
    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());

    runtime.initialize_startup()?;
    assert_eq!(
        runtime.startup_state(),
        RuntimeStartupState::RecoveryCompleted
    );
    assert!(runtime.is_accepting_admission());
    assert_eq!(probe.discovery_calls(), 2);
    assert_eq!(probe.integrity_scan_calls(), 0);
    Ok(())
}

#[test]
fn startup_rejects_a_nonadvancing_active_recovery_cursor() -> TestResult {
    let directory = TempDir::new()?;
    let probe = Arc::new(StartupProbeStore::new(
        Arc::new(RedbStore::open(directory.path())?),
        StartupDiscoveryFault::Stall,
    ));
    let runtime = runtime_for_startup_probe(probe.clone(), "startup-stalled-cursor")?;

    let Err(error) = runtime.initialize_startup() else {
        return Err("startup accepted a nonadvancing active-recovery cursor".into());
    };
    assert!(matches!(&error, RuntimeError::Scheduling(_)));
    assert!(error.to_string().contains("no bounded progress"));
    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());
    assert!(probe.discovery_calls() >= 2);
    assert_eq!(probe.integrity_scan_calls(), 0);
    Ok(())
}

#[test]
fn startup_rejects_symmetric_runnable_index_loss_after_authoritative_replay() -> TestResult {
    let directory = TempDir::new()?;
    let run = RunId::new("run-startup-runnable-symmetric-loss")?;
    {
        let store = Arc::new(RedbStore::open(directory.path())?);
        let runtime = RuntimeService::new_with_authority(
            store.clone(),
            Arc::new(DeterministicExecutor::new(test_descriptor()?)),
            test_authority(),
            Arc::new(ManualClock::new(NOW)),
            Arc::new(SequentialIdGenerator::new("startup-runnable-loss", 1)?),
            RuntimeConfig::new(
                WorkerId::new("worker-startup-runnable-loss")?,
                ActorRef::new("controller:startup-runnable-loss")?,
                30_000,
                8,
                SchedulerLimits::new(8, 4, 2, 4)?,
                RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
            )?,
        )?;
        let revision = task_revision("workflow-startup-runnable-symmetric-loss")?;
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
                    ScopeId::new("scope-startup-runnable-symmetric-loss")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?;
        submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?;
        assert_eq!(
            store
                .runnable_page(TimestampMillis::new(NOW), None, PageSize::new(8)?)?
                .entries
                .len(),
            1
        );
    }

    storage_fault::remove_run_runnable_discovery(directory.path(), &run)?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let runtime = RuntimeService::open_closed_with_authority(
        store,
        Arc::new(DeterministicExecutor::new(test_descriptor()?)),
        test_authority(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new(
            "startup-runnable-loss-reopen",
            1,
        )?),
        RuntimeConfig::new(
            WorkerId::new("worker-startup-runnable-loss-reopen")?,
            ActorRef::new("controller:startup-runnable-loss-reopen")?,
            30_000,
            8,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?;
    let Err(error) = runtime.initialize_startup() else {
        return Err("startup accepted symmetric runnable-index loss".into());
    };
    assert!(error.to_string().contains("runnable discovery"));
    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());
    Ok(())
}

#[test]
fn startup_keeps_admission_closed_until_active_recovery_completes() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let runtime = RuntimeService::open_closed_with_authority(
        store,
        Arc::new(DeterministicExecutor::new(test_descriptor()?)),
        test_authority(),
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
    for claim_before_expiry in [false, true] {
        let directory = TempDir::new()?;
        let identity = if claim_before_expiry {
            "expired-claimed-ticket"
        } else {
            "expired-unclaimed-lease"
        };
        let executor = Arc::new(DispatchCountingExecutor::new(test_descriptor()?));
        let (store, clock, runtime) = runtime_with_executor_at(
            directory.path(),
            identity,
            identity,
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
        clock.advance(30_001)?;
        if let Some(action) = action {
            assert!(matches!(
                runtime.execute_effect(action),
                Err(RuntimeError::InvalidTransition(_))
            ));
        } else {
            assert!(runtime.claim_effects(PageSize::new(1)?)?.is_empty());
        }
        assert_eq!(executor.dispatches(), 0);
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
    assert!(matches!(
        runtime.execute_effect(action),
        Err(RuntimeError::Executor(ExecutorError::InvalidReports(_)))
    ));
    assert_eq!(executor.dispatches(), 1);
    assert!(runtime.claim_effects(PageSize::new(1)?)?.is_empty());
    assert_eq!(executor.dispatches(), 1);
    assert_eq!(
        runtime
            .history(&run)?
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
            .count(),
        1
    );
    Ok(())
}

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
        std::thread::spawn(move || tick_runtime.tick().map_err(|error| error.to_string()));
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

    runtime.tick()?;
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
fn unsupported_v1_snapshot_is_rejected_and_startup_replays_authoritative_history() -> TestResult {
    let harness = Harness::new("snapshot-v1-fallback")?;
    let revision = task_revision("workflow-snapshot-v1-fallback")?;
    let run = RunId::new("run-snapshot-v1-fallback")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    for _ in 0..128 {
        if matches!(
            harness.store.latest_snapshot(&run)?,
            milkdrift_persistence::SnapshotLoad::Verified(_)
        ) {
            break;
        }
        let command = match harness.runtime.projection(&run)?.lifecycle() {
            RunLifecycle::Running => RunCommand::PauseRun,
            RunLifecycle::Paused => RunCommand::ResumeRun,
            lifecycle => {
                return Err(format!(
                    "snapshot fallback fixture reached unexpected lifecycle {lifecycle:?}"
                )
                .into());
            }
        };
        assert_eq!(
            harness.command(&run, command)?,
            CommandDisposition::Accepted
        );
    }
    assert!(matches!(
        harness.store.latest_snapshot(&run)?,
        milkdrift_persistence::SnapshotLoad::Verified(_)
    ));
    let expected = harness.runtime.projection(&run)?;
    let directory = harness.close();

    storage_fault::replace_latest_snapshot_document(
        directory.path(),
        &run,
        include_bytes!(
            "../fixtures/unsupported-projection-snapshot-envelope-v1-projection-v3-wire.json"
        ),
    )?;
    {
        let store = RedbStore::open(directory.path())?;
        assert!(matches!(
            store.latest_snapshot(&run)?,
            milkdrift_persistence::SnapshotLoad::Rejected {
                snapshot: Some(_),
                ..
            }
        ));
    }

    let (store, _clock, runtime) = runtime_at(directory.path(), "snapshot-v1-replay", NOW, 64)?;
    assert_eq!(runtime.projection(&run)?, expected);
    assert_eq!(
        store.latest_snapshot(&run)?,
        milkdrift_persistence::SnapshotLoad::Absent
    );
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
