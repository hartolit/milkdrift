//! A busy artifact owner must not discard a report from an already entered effect.
use super::*;
use milkdrift_persistence::{PersistenceError, StorageFailureClass};
use milkdrift_redb_store::{FaultInjector, FaultPoint, RedbStoreConfig};

#[derive(Clone, Copy)]
enum Conflict {
    Publication,
    Usage,
    Unavailable,
}

struct ReportConflict {
    remaining: AtomicUsize,
    observed: AtomicUsize,
    kind: Conflict,
    run: RunId,
}

impl FaultInjector for ReportConflict {
    fn check(&self, point: FaultPoint) -> PersistenceResult<()> {
        if point != FaultPoint::BeforeCommandCommit
            || self
                .remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_err()
        {
            return Ok(());
        }
        self.observed.fetch_add(1, Ordering::SeqCst);
        Err(match self.kind {
            Conflict::Publication => PersistenceError::Storage {
                class: StorageFailureClass::OwnerBusy,
                message: "another branch has an active artifact publication".to_owned(),
            },
            Conflict::Usage => PersistenceError::WorkspaceUsageConflict {
                run: self.run.clone(),
            },
            Conflict::Unavailable => PersistenceError::Storage {
                class: StorageFailureClass::Unavailable,
                message: "storage is unavailable".to_owned(),
            },
        })
    }
}

struct ReportingExecutor {
    resolver: DeterministicExecutor,
    fault: Arc<ReportConflict>,
    failures: usize,
    entries: AtomicUsize,
}

impl TaskExecutor for ReportingExecutor {
    delegate_resolve!(resolver);
    delegate_cancel!(resolver);

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(prepared_test_entry(dispatch, move |dispatch, reporter| {
            self.entries.fetch_add(1, Ordering::SeqCst);
            // Only report persistence is contested; admission and external entry already happened.
            self.fault.remaining.store(self.failures, Ordering::SeqCst);
            reporter.invocation(InvocationEvent::new(
                dispatch.request().invocation().clone(),
                1,
                InvocationEventKind::Terminal {
                    terminal: InvocationTerminal::new(
                        TerminalStatus::Success,
                        Vec::new(),
                        None,
                        None,
                        SideEffectClass::None,
                    )?,
                },
            )?)?;
            Ok(())
        }))
    }
}

fn exercise(kind: Conflict, failures: usize, accepted: bool) -> TestResult {
    let directory = TempDir::new()?;
    let run = RunId::new("report-contention")?;
    let fault = Arc::new(ReportConflict {
        remaining: AtomicUsize::new(0),
        observed: AtomicUsize::new(0),
        kind,
        run: run.clone(),
    });
    let store = Arc::new(RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path()).with_fault_injector(fault.clone()),
    )?);
    let executor = Arc::new(ReportingExecutor {
        resolver: DeterministicExecutor::new(test_descriptor()?),
        fault: fault.clone(),
        failures,
        entries: AtomicUsize::new(0),
    });
    let runtime = recovery_service(
        store.clone(),
        Arc::new(ManualClock::new(NOW)),
        executor.clone(),
        "report-contention",
    )?;
    let revision = task_revision("report-contention")?;
    store.put_revision(&revision)?;
    submit_command(
        &runtime,
        &store,
        &run,
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(run.clone(), ScopeId::new("report-contention")?),
            workspace_budget: generous_budget()?,
            inputs: Vec::new(),
        },
    )?;
    submit_command(&runtime, &store, &run, RunCommand::StartRun)?;
    runtime.scheduler_tick()?;
    let mut actions = runtime.claim_execution_effects(PageSize::new(1)?)?;
    assert_eq!(actions.len(), 1);
    let outcome = runtime.execute_effect(actions.pop().ok_or("effect absent")?);
    assert_eq!(outcome.is_ok(), accepted, "{outcome:?}");
    assert_eq!(executor.entries.load(Ordering::SeqCst), 1);
    assert_eq!(fault.observed.load(Ordering::SeqCst), failures);
    let projection = runtime.projection(&run)?;
    assert_eq!(projection.is_completed(), accepted);
    assert_eq!(
        projection.unresolved_attempts().count(),
        usize::from(!accepted)
    );
    let history = runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { .. }))
            .count(),
        usize::from(!accepted)
    );
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::NodeTerminal { .. }))
            .count(),
        usize::from(accepted)
    );
    let sequence = projection.sequence();
    drop(runtime);
    drop(store);
    let store = Arc::new(RedbStore::open(directory.path())?);
    let reopened = recovery_service(
        store,
        Arc::new(ManualClock::new(NOW)),
        executor.clone(),
        "report-reopen",
    )?;
    assert_eq!(reopened.projection(&run)?.sequence(), sequence);
    assert_eq!(executor.entries.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn temporary_publication_and_usage_conflicts_preserve_one_effect_and_terminal() -> TestResult {
    for kind in [Conflict::Publication, Conflict::Usage] {
        exercise(kind, 1, true)?;
        exercise(kind, 15, true)?;
    }
    Ok(())
}

#[test]
fn exhausted_contention_and_other_storage_failures_remain_uncertain() -> TestResult {
    exercise(Conflict::Publication, 16, false)?;
    exercise(Conflict::Usage, 16, false)?;
    exercise(Conflict::Unavailable, 1, false)
}
