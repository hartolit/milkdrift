//! Interrupted scheduler and entry commits retain a live owner without repeating effects.
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use milkdrift_blueprint::{NewTask, TaskId, WorkflowId};
use milkdrift_capability::{CapabilityDescriptorDocument, CapabilityRequirement};
use milkdrift_control::TaskControlService;
use milkdrift_persistence::PersistenceError;
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use milkdrift_runtime::{
    DeterministicExecutor, ExecutionDispatch, ExecutorError, ManualClock, PreparedExecution,
    ResolvedCapability, SequentialIdGenerator, TaskExecutor, TaskRecord, TaskRuntime, TaskState,
    WorkflowRecord,
};

use super::{InvocationAllowance, TaskStoreClock, TestResult, context, request};

#[derive(Clone, Copy)]
struct Failure {
    point: FaultPoint,
    skip: usize,
    persistent: bool,
}

#[derive(Default)]
struct Faults {
    armed: Mutex<Option<Failure>>,
    failures: AtomicUsize,
}

impl Faults {
    fn arm(&self, failure: Failure) -> TestResult {
        *self.armed.lock().map_err(|_| "fault fixture lock")? = Some(failure);
        Ok(())
    }

    fn clear(&self) -> TestResult {
        *self.armed.lock().map_err(|_| "fault fixture lock")? = None;
        Ok(())
    }
}

impl FaultInjector for Faults {
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
        let mut armed = self.armed.lock().map_err(|_| injected_failure(point))?;
        let Some(failure) = armed.as_mut() else {
            return Ok(());
        };
        if failure.point != point {
            return Ok(());
        }
        if failure.skip > 0 {
            failure.skip -= 1;
            return Ok(());
        }
        if !failure.persistent {
            *armed = None;
        }
        self.failures.fetch_add(1, Ordering::SeqCst);
        Err(injected_failure(point))
    }
}

struct Executor {
    inner: DeterministicExecutor,
    faults: Arc<Faults>,
    preparation_fault: Mutex<Option<Failure>>,
    prepared: AtomicUsize,
    cleanup_calls: AtomicUsize,
    claim_held: AtomicBool,
    fail_cleanup: AtomicBool,
}

struct PreparedClaim<'a>(&'a AtomicUsize);

impl Drop for PreparedClaim<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl TaskExecutor for Executor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        now: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.inner.resolve(requirement, now)
    }

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        let prepared = self.inner.prepare_exact_entry(dispatch)?;
        self.claim_held.store(true, Ordering::SeqCst);
        self.prepared.fetch_add(1, Ordering::SeqCst);
        let claim = PreparedClaim(&self.prepared);
        let handle = PreparedExecution::new(
            dispatch,
            prepared.admission_envelope().clone(),
            move |dispatch, reporter| {
                let result = prepared.enter(dispatch, reporter);
                drop(claim);
                result
            },
        );
        // The next workflow commit is final-entry intent, after this handle is returned.
        let failure = self
            .preparation_fault
            .lock()
            .map_err(|_| ExecutorError::Boundary("preparation fault fixture lock".into()))?
            .take();
        if let Some(failure) = failure {
            self.faults
                .arm(failure)
                .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        }
        Ok(handle)
    }

    fn defer_before_entry(&self, _dispatch: &ExecutionDispatch) -> Result<(), ExecutorError> {
        assert_eq!(self.prepared.load(Ordering::SeqCst), 0);
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_cleanup.load(Ordering::SeqCst) {
            return Err(ExecutorError::Boundary(
                "cleanup outcome unavailable".into(),
            ));
        }
        self.claim_held.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn settle(&self, _dispatch: &ExecutionDispatch) -> Result<(), ExecutorError> {
        assert_eq!(self.prepared.load(Ordering::SeqCst), 0);
        self.claim_held.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn cancel(
        &self,
        request: &milkdrift_capability::CancellationRequest,
    ) -> Result<milkdrift_capability::CancellationAcknowledgement, ExecutorError> {
        self.inner.cancel(request)
    }
}

struct Fixture {
    runtime: Arc<TaskRuntime>,
    executor: Arc<Executor>,
    control: TaskControlService,
    faults: Arc<Faults>,
}

impl Fixture {
    fn open(path: &std::path::Path) -> TestResult<Self> {
        let clock = Arc::new(ManualClock::new(1_000));
        let faults = Arc::new(Faults::default());
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(path)
                .with_clock(Arc::new(TaskStoreClock(clock.clone())))
                .with_fault_injector(faults.clone()),
        )?);
        let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../capability/tests/fixtures/descriptor-v1.json"
        ))?;
        descriptor
            .get_mut("descriptor")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("descriptor absent")?
            .insert("category".into(), serde_json::json!({ "type": "process" }));
        let descriptor =
            CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&descriptor)?)?;
        let executor = Arc::new(Executor {
            inner: DeterministicExecutor::new(descriptor.body().clone()),
            faults: faults.clone(),
            preparation_fault: Mutex::new(None),
            prepared: AtomicUsize::new(0),
            cleanup_calls: AtomicUsize::new(0),
            claim_held: AtomicBool::new(false),
            fail_cleanup: AtomicBool::new(false),
        });
        let authority = Arc::new(InvocationAllowance(Some(8)));
        let runtime = Arc::new(TaskRuntime::new(
            store.clone(),
            executor.clone(),
            authority.clone(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new("lifecycle-fault", 1)?),
        ));
        let control = TaskControlService::new(runtime.clone(), authority, clock, store);
        Ok(Self {
            runtime,
            executor,
            control,
            faults,
        })
    }

    fn create(&self, count: usize) -> TestResult<WorkflowId> {
        let mut create = request()?;
        let mut definition = create
            .tasks
            .first()
            .ok_or("source task absent")?
            .definition
            .clone();
        definition.inputs.clear();
        create.tasks = (0..count)
            .map(|index| {
                Ok(NewTask {
                    alias: TaskId::new(format!("work_{index}"))?,
                    definition: definition.clone(),
                })
            })
            .collect::<TestResult<Vec<_>>>()?;
        create.limits.concurrent_tasks = 4;
        Ok(self.control.create(&context()?, "create", create)?.workflow)
    }

    fn record(&self, workflow: &WorkflowId) -> TestResult<WorkflowRecord> {
        self.runtime
            .workflow(workflow)?
            .ok_or_else(|| "workflow absent".into())
    }

    fn task(&self, dispatch: &ExecutionDispatch) -> TestResult<TaskRecord> {
        self.record(&WorkflowId::new(dispatch.run().as_str())?)?
            .tasks
            .remove(&TaskId::new(dispatch.node().as_str())?)
            .ok_or_else(|| "task absent".into())
    }

    fn arm_after_preparation(&self, point: FaultPoint, persistent: bool) -> TestResult {
        *self
            .executor
            .preparation_fault
            .lock()
            .map_err(|_| "preparation fault lock")? = Some(Failure {
            point,
            skip: 0,
            persistent,
        });
        Ok(())
    }
}

#[test]
fn partial_tick_commit_failure_delivers_each_accepted_start_once_without_restart() -> TestResult {
    for point in [
        FaultPoint::BeforeCommandCommit,
        FaultPoint::AfterCommandCommit,
    ] {
        let root = tempfile::tempdir()?;
        let fixture = Fixture::open(root.path())?;
        let workflow = fixture.create(2)?;
        fixture.faults.arm(Failure {
            point,
            skip: 1,
            persistent: false,
        })?;
        assert!(fixture.runtime.tick(2).is_err());
        assert_eq!(fixture.faults.failures.load(Ordering::SeqCst), 1);
        let interrupted = fixture.record(&workflow)?;
        let accepted: Vec<_> = interrupted
            .tasks
            .values()
            .filter_map(|task| task.frozen.as_ref().map(|frozen| frozen.dispatch.clone()))
            .collect();
        assert_eq!(
            accepted.len(),
            if point == FaultPoint::BeforeCommandCommit {
                1
            } else {
                2
            }
        );
        let mut delivered = BTreeSet::new();
        // One pending slot can correspond to the transaction that did not commit; discovery
        // must discard that slot and still return to ordinary scheduling within bounded polls.
        for _ in 0..6 {
            let dispatches = fixture.runtime.tick(1)?;
            assert!(dispatches.len() <= 1);
            for dispatch in dispatches {
                assert!(delivered.insert(dispatch.request().invocation().clone()));
                if let Some(original) = accepted
                    .iter()
                    .find(|accepted| accepted.node() == dispatch.node())
                {
                    assert_eq!(&dispatch, original);
                }
                fixture.runtime.execute(dispatch)?;
            }
            if fixture
                .record(&workflow)?
                .tasks
                .values()
                .all(|task| task.state == TaskState::Completed)
            {
                break;
            }
        }
        assert_eq!(delivered.len(), 2);
        assert_eq!(fixture.executor.inner.entry_count(), 2);
        assert!(
            fixture
                .record(&workflow)?
                .tasks
                .values()
                .all(|task| task.state == TaskState::Completed)
        );
        assert!(fixture.runtime.tick(2)?.is_empty());
    }
    Ok(())
}

#[test]
fn final_entry_commit_failure_retries_only_proven_nonentry_after_dropping_preparation() -> TestResult
{
    for point in [
        FaultPoint::BeforeCommandCommit,
        FaultPoint::AfterCommandCommit,
    ] {
        let root = tempfile::tempdir()?;
        let fixture = Fixture::open(root.path())?;
        fixture.create(1)?;
        let dispatch = fixture.runtime.tick(1)?.pop().ok_or("start absent")?;
        fixture.arm_after_preparation(point, false)?;
        assert!(fixture.runtime.execute(dispatch.clone()).is_err());
        assert_eq!(fixture.faults.failures.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.executor.prepared.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.executor.inner.entry_count(), 0);
        let task = fixture.task(&dispatch)?;
        let frozen = task.frozen.as_ref().ok_or("frozen start absent")?;
        assert!(task.result.is_none());
        assert!(task.ended_at.is_none());
        if point == FaultPoint::BeforeCommandCommit {
            assert_eq!(task.state, TaskState::Queued);
            assert_eq!(frozen.dispatch, dispatch);
            assert!(!frozen.entered);
            assert!(frozen.execution_duration.is_none());
            assert_eq!(fixture.executor.cleanup_calls.load(Ordering::SeqCst), 1);
            assert!(!fixture.executor.claim_held.load(Ordering::SeqCst));
            let retry = fixture.runtime.tick(1)?.pop().ok_or("live retry absent")?;
            assert_eq!(retry, dispatch);
            fixture.runtime.execute(retry)?;
            assert_eq!(fixture.task(&dispatch)?.state, TaskState::Completed);
            assert_eq!(fixture.executor.inner.entry_count(), 1);
        } else {
            assert_eq!(task.state, TaskState::Uncertain);
            assert!(frozen.entered);
            assert!(frozen.execution_duration.is_some());
            assert_eq!(fixture.executor.cleanup_calls.load(Ordering::SeqCst), 0);
            assert!(fixture.executor.claim_held.load(Ordering::SeqCst));
            assert!(fixture.runtime.tick(1)?.is_empty());
            fixture.runtime.recover()?;
            assert!(fixture.runtime.tick(1)?.is_empty());
            assert_eq!(fixture.executor.inner.entry_count(), 0);
            assert!(fixture.executor.claim_held.load(Ordering::SeqCst));
        }
    }
    Ok(())
}

#[test]
fn unavailable_requeue_commit_retains_live_reconciliation_until_storage_returns() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path())?;
    fixture.create(1)?;
    let dispatch = fixture.runtime.tick(1)?.pop().ok_or("start absent")?;
    fixture.arm_after_preparation(FaultPoint::BeforeCommandCommit, true)?;
    assert!(fixture.runtime.execute(dispatch.clone()).is_err());
    assert_eq!(fixture.executor.prepared.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.task(&dispatch)?.state, TaskState::Running);
    assert!(fixture.runtime.tick(1).is_err());
    assert_eq!(fixture.executor.inner.entry_count(), 0);
    assert!(fixture.faults.failures.load(Ordering::SeqCst) >= 3);
    fixture.faults.clear()?;
    let mut retry = None;
    for _ in 0..3 {
        let mut starts = fixture.runtime.tick(1)?;
        assert!(starts.len() <= 1);
        if let Some(start) = starts.pop() {
            retry = Some(start);
            break;
        }
    }
    let retry = retry.ok_or("reconciled live retry absent")?;
    assert_eq!(retry, dispatch);
    fixture.runtime.execute(retry)?;
    assert_eq!(fixture.task(&dispatch)?.state, TaskState::Completed);
    assert_eq!(fixture.executor.inner.entry_count(), 1);
    assert!(!fixture.executor.claim_held.load(Ordering::SeqCst));
    assert!(fixture.runtime.tick(1)?.is_empty());
    Ok(())
}

#[test]
fn unknown_cleanup_keeps_uncertainty_when_its_first_durable_record_also_fails() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path())?;
    fixture.create(1)?;
    let dispatch = fixture.runtime.tick(1)?.pop().ok_or("start absent")?;
    fixture.executor.fail_cleanup.store(true, Ordering::SeqCst);
    fixture.arm_after_preparation(FaultPoint::BeforeCommandCommit, true)?;
    assert!(fixture.runtime.execute(dispatch.clone()).is_err());
    assert_eq!(fixture.executor.prepared.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.task(&dispatch)?.state, TaskState::Running);
    assert_eq!(fixture.executor.cleanup_calls.load(Ordering::SeqCst), 1);
    assert!(fixture.executor.claim_held.load(Ordering::SeqCst));
    assert!(fixture.runtime.tick(1).is_err());
    // Availability returning is not evidence about the earlier unknown cleanup outcome.
    fixture.faults.clear()?;
    fixture.executor.fail_cleanup.store(false, Ordering::SeqCst);
    assert!(fixture.runtime.tick(1)?.is_empty());
    let task = fixture.task(&dispatch)?;
    assert_eq!(task.state, TaskState::Uncertain);
    let frozen = task.frozen.as_ref().ok_or("frozen start absent")?;
    assert!(!frozen.entered);
    assert!(frozen.execution_duration.is_some());
    assert!(task.result.is_none());
    assert!(task.ended_at.is_none());
    fixture.runtime.recover()?;
    assert!(fixture.runtime.tick(1)?.is_empty());
    assert_eq!(fixture.executor.cleanup_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.executor.inner.entry_count(), 0);
    assert!(fixture.executor.claim_held.load(Ordering::SeqCst));
    Ok(())
}
