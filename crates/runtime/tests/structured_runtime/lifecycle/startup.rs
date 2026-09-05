use super::*;

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
            "../../fixtures/unsupported-projection-snapshot-envelope-v1-projection-v3-wire.json"
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
        fn revisions(
            &self,
            query: &milkdrift_persistence::RevisionPageQuery,
        ) -> PersistenceResult<milkdrift_persistence::RevisionPage>;
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

impl milkdrift_persistence::ControllerAccountStore for StartupProbeStore {
    fn controller_account_binding(
        &self,
        run: &RunId,
    ) -> PersistenceResult<Option<milkdrift_persistence::ControllerAccountId>> {
        self.inner.controller_account_binding(run)
    }

    fn controller_account(
        &self,
        account: &milkdrift_persistence::ControllerAccountId,
    ) -> PersistenceResult<Option<milkdrift_persistence::ControllerAccountState>> {
        self.inner.controller_account(account)
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
