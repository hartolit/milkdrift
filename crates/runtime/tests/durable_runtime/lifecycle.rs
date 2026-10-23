//! Pauses, temporary contention and recovery use the maintained store and execution boundary.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use milkdrift_authority::{AuthorityBudget, AuthorityEvaluator};
use milkdrift_blueprint::{
    TaskBatch, TaskChange, TaskId, TaskInputValue, WorkflowCreate, WorkflowId,
};
use milkdrift_capability::{CapabilityDescriptorDocument, CapabilityRequirement};
use milkdrift_control::TaskControlService;
use milkdrift_persistence::WorkflowStore;
use milkdrift_redb_store::{RedbStore, RedbStoreConfig};
use milkdrift_runtime::{
    DeterministicExecutor, EffectAction, ExecutionDispatch, ExecutorError, ManualClock,
    PreparedExecution, ResolvedCapability, SequentialIdGenerator, TaskExecutor, TaskRuntime,
    TaskState, WorkflowRecord,
};

use super::{
    InvocationAllowance, RevocableAuthority, TaskStoreClock, TestResult, context, request,
};

type PreparationHook = Box<dyn FnOnce() -> TestResult + Send>;

struct LifecycleExecutor {
    inner: DeterministicExecutor,
    busy: Mutex<Option<String>>,
    preparation_hook: Mutex<Option<PreparationHook>>,
    prepared: AtomicUsize,
    deferrals: AtomicUsize,
    settlements: AtomicUsize,
    fail_cleanup: AtomicBool,
}

struct PreparedClaim<'a>(&'a AtomicUsize);

impl Drop for PreparedClaim<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl TaskExecutor for LifecycleExecutor {
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
        if self
            .busy
            .lock()
            .map_err(|_| ExecutorError::Boundary("busy fixture lock".into()))?
            .as_deref()
            == Some(dispatch.node().as_str())
        {
            return Err(ExecutorError::NativePathBusy);
        }
        let prepared = self.inner.prepare_exact_entry(dispatch)?;
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
        let hook = self
            .preparation_hook
            .lock()
            .map_err(|_| ExecutorError::Boundary("preparation fixture lock".into()))?
            .take();
        if let Some(hook) = hook {
            hook().map_err(|error| ExecutorError::BoundaryBeforeEntry(error.to_string()))?;
        }
        Ok(handle)
    }

    fn defer_before_entry(&self, _dispatch: &ExecutionDispatch) -> Result<(), ExecutorError> {
        assert_eq!(self.prepared.load(Ordering::SeqCst), 0);
        self.deferrals.fetch_add(1, Ordering::SeqCst);
        if self.fail_cleanup.load(Ordering::SeqCst) {
            return Err(ExecutorError::Boundary(
                "cleanup outcome unavailable".into(),
            ));
        }
        Ok(())
    }

    fn settle(&self, _dispatch: &ExecutionDispatch) -> Result<(), ExecutorError> {
        assert_eq!(self.prepared.load(Ordering::SeqCst), 0);
        self.settlements.fetch_add(1, Ordering::SeqCst);
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
    executor: Arc<LifecycleExecutor>,
    control: Arc<TaskControlService>,
    store: Arc<RedbStore>,
}

impl Fixture {
    fn open(
        root: &std::path::Path,
        authority: Arc<dyn AuthorityEvaluator>,
        clock: Arc<ManualClock>,
    ) -> TestResult<Self> {
        let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../capability/tests/fixtures/descriptor-v1.json"
        ))?;
        descriptor
            .get_mut("descriptor")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("descriptor absent")?
            .insert("category".into(), serde_json::json!({"type":"process"}));
        let descriptor =
            CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&descriptor)?)?;
        let executor = Arc::new(LifecycleExecutor {
            inner: DeterministicExecutor::new(descriptor.body().clone()),
            busy: Mutex::new(None),
            preparation_hook: Mutex::new(None),
            prepared: AtomicUsize::new(0),
            deferrals: AtomicUsize::new(0),
            settlements: AtomicUsize::new(0),
            fail_cleanup: AtomicBool::new(false),
        });
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(root).with_clock(Arc::new(TaskStoreClock(clock.clone()))),
        )?);
        let runtime = Arc::new(
            TaskRuntime::new(
                store.clone(),
                executor.clone(),
                authority.clone(),
                clock.clone(),
                Arc::new(SequentialIdGenerator::new("lifecycle", 1)?),
            )
            .with_maximum_active_tasks(1)?,
        );
        let control = Arc::new(TaskControlService::new(
            runtime.clone(),
            authority,
            clock,
            store.clone(),
        ));
        Ok(Self {
            runtime,
            executor,
            control,
            store,
        })
    }

    fn record(&self, workflow: &WorkflowId) -> TestResult<WorkflowRecord> {
        self.runtime
            .workflow(workflow)?
            .ok_or_else(|| "workflow absent".into())
    }

    fn edit(&self, workflow: &WorkflowId, key: &str, change: TaskChange) -> TestResult {
        self.control.edit(
            &context()?,
            key,
            workflow,
            TaskBatch {
                expected_generation: self.record(workflow)?.generation,
                changes: vec![change],
            },
        )?;
        Ok(())
    }
}

fn ready() -> TestResult<WorkflowCreate> {
    let mut create = request()?;
    create.inputs.insert(
        "manifest".into(),
        TaskInputValue::Text {
            text: "captured".into(),
        },
    );
    create.limits.concurrent_tasks = 1;
    Ok(create)
}

#[test]
fn supplying_input_preserves_human_wait_and_old_continue_cannot_release_a_later_wait() -> TestResult
{
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1000)),
    )?;
    let workflow = fixture
        .control
        .create(&context()?, "create", request()?)?
        .workflow;
    fixture.edit(
        &workflow,
        "pause",
        TaskChange::Wait {
            question: "Choose the review scope".into(),
        },
    )?;
    fixture.edit(
        &workflow,
        "input",
        TaskChange::BindInput {
            name: "manifest".into(),
            value: TaskInputValue::Text {
                text: "now supplied".into(),
            },
        },
    )?;
    assert_eq!(
        fixture.record(&workflow)?.wait_question.as_deref(),
        Some("Choose the review scope")
    );
    assert!(fixture.runtime.tick(1)?.is_empty());
    let resume = TaskBatch {
        expected_generation: fixture.record(&workflow)?.generation,
        changes: vec![TaskChange::Continue],
    };
    let receipt = fixture
        .control
        .edit(&context()?, "continue", &workflow, resume.clone())?;
    fixture.edit(
        &workflow,
        "pause-again",
        TaskChange::Wait {
            question: "A different decision".into(),
        },
    )?;
    assert_eq!(
        fixture
            .control
            .edit(&context()?, "continue", &workflow, resume.clone())?,
        receipt
    );
    assert!(
        fixture
            .control
            .edit(&context()?, "stale-new-continue", &workflow, resume)
            .is_err()
    );
    assert_eq!(
        fixture.record(&workflow)?.wait_question.as_deref(),
        Some("A different decision")
    );
    assert!(fixture.runtime.tick(1)?.is_empty());
    fixture.edit(&workflow, "continue-current", TaskChange::Continue)?;
    fixture
        .runtime
        .execute(fixture.runtime.tick(1)?.pop().ok_or("ready start absent")?)?;
    assert_eq!(fixture.executor.inner.entry_count(), 1);
    Ok(())
}

#[test]
fn pause_before_preparation_and_restart_keep_the_exact_start_without_charging_entry() -> TestResult
{
    for execute_before_restart in [false, true] {
        let root = tempfile::tempdir()?;
        let clock = Arc::new(ManualClock::new(1000));
        let authority = Arc::new(InvocationAllowance(Some(8)));
        let (workflow, frozen) = {
            let fixture = Fixture::open(root.path(), authority.clone(), clock.clone())?;
            let workflow = fixture
                .control
                .create(&context()?, "create", ready()?)?
                .workflow;
            let dispatch = fixture.runtime.tick(1)?.pop().ok_or("start absent")?;
            let frozen = fixture
                .record(&workflow)?
                .tasks
                .values()
                .next()
                .and_then(|task| task.frozen.clone())
                .ok_or("frozen start absent")?;
            fixture.edit(
                &workflow,
                "pause",
                TaskChange::Wait {
                    question: "Review before entering".into(),
                },
            )?;
            if execute_before_restart {
                fixture.runtime.execute(dispatch)?;
                assert_eq!(fixture.executor.deferrals.load(Ordering::SeqCst), 1);
            }
            assert_eq!(fixture.executor.inner.entry_count(), 0);
            (workflow, frozen)
        };
        // Waiting longer than the dispatch's original wall-clock lease must not spend active
        // execution time or require changing the already accepted dispatch.
        clock.advance(frozen.dispatch.lease_expires_at().get())?;
        let fixture = Fixture::open(root.path(), authority, clock)?;
        fixture.runtime.recover()?;
        let record = fixture.record(&workflow)?;
        let task = record.tasks.values().next().ok_or("task absent")?;
        assert_eq!(task.state, TaskState::Queued);
        assert_eq!(task.frozen.as_ref(), Some(&frozen));
        assert_eq!(record.active_execution_ms, 0);
        assert_eq!(record.committed_allowance, AuthorityBudget::default());
        assert!(fixture.runtime.tick(1)?.is_empty());
        fixture.edit(&workflow, "resume", TaskChange::Continue)?;
        let dispatch = fixture
            .runtime
            .tick(1)?
            .pop()
            .ok_or("resumed start absent")?;
        assert_eq!(dispatch, frozen.dispatch);
        fixture.runtime.execute(dispatch)?;
        assert_eq!(
            fixture.executor.inner.entry_count(),
            1,
            "resumed record: {:?}",
            fixture.record(&workflow)?
        );
        assert_eq!(
            fixture.record(&workflow)?.committed_allowance.invocations,
            Some(1)
        );
    }
    Ok(())
}

#[test]
fn pause_and_stop_during_preparation_drop_the_handle_before_releasing_claims() -> TestResult {
    for stop in [false, true] {
        let root = tempfile::tempdir()?;
        let fixture = Fixture::open(
            root.path(),
            Arc::new(InvocationAllowance(Some(8))),
            Arc::new(ManualClock::new(1000)),
        )?;
        let workflow = fixture
            .control
            .create(&context()?, "create", ready()?)?
            .workflow;
        let dispatch = fixture.runtime.tick(1)?.pop().ok_or("start absent")?;
        let task = TaskId::new(dispatch.node().as_str())?;
        let runtime = fixture.runtime.clone();
        let control = fixture.control.clone();
        let executor = fixture.executor.clone();
        let id = workflow.clone();
        *fixture
            .executor
            .preparation_hook
            .lock()
            .map_err(|_| "hook lock")? = Some(Box::new(move || {
            let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
            control.edit(
                &context()?,
                "during-prepare",
                &id,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![if stop {
                        TaskChange::Stop { task }
                    } else {
                        TaskChange::Wait {
                            question: "Hold prepared work".into(),
                        }
                    }],
                },
            )?;
            for action in runtime.cancellation_actions(1)? {
                if let EffectAction::Cancel(dispatch) = action {
                    runtime.execute_cancellation(&dispatch)?;
                }
            }
            assert_eq!(executor.prepared.load(Ordering::SeqCst), 1);
            assert_eq!(executor.settlements.load(Ordering::SeqCst), 0);
            assert_eq!(executor.deferrals.load(Ordering::SeqCst), 0);
            Ok(())
        }));
        fixture.runtime.execute(dispatch.clone())?;
        let record = fixture.record(&workflow)?;
        let retained = record.tasks.values().next().ok_or("task absent")?;
        assert_eq!(
            retained.state,
            if stop {
                TaskState::Stopped
            } else {
                TaskState::Queued
            }
        );
        assert_eq!(
            retained.frozen.as_ref().map(|start| &start.dispatch),
            Some(&dispatch)
        );
        assert_eq!(fixture.executor.inner.entry_count(), 0);
        assert_eq!(fixture.executor.prepared.load(Ordering::SeqCst), 0);
        assert_eq!(record.committed_allowance, AuthorityBudget::default());
        if !stop {
            fixture.edit(&workflow, "resume", TaskChange::Continue)?;
            let retry = fixture
                .runtime
                .tick(1)?
                .pop()
                .ok_or("resumed start absent")?;
            assert_eq!(retry, dispatch);
            fixture.runtime.execute(retry)?;
            assert_eq!(fixture.executor.inner.entry_count(), 1);
        }
    }
    Ok(())
}

#[test]
fn repeated_busy_start_yields_same_workflow_and_global_slots_without_growing_history() -> TestResult
{
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1000)),
    )?;
    let mut create = ready()?;
    let mut other = create.tasks.first().ok_or("task absent")?.clone();
    other.alias = TaskId::new("other")?;
    create.tasks.push(other);
    let workflow = fixture
        .control
        .create(&context()?, "create", create)?
        .workflow;
    let dispatch = fixture.runtime.tick(1)?.pop().ok_or("first start absent")?;
    *fixture.executor.busy.lock().map_err(|_| "busy lock")? =
        Some(dispatch.node().as_str().to_owned());
    fixture.runtime.execute(dispatch.clone())?;
    let frozen = fixture
        .record(&workflow)?
        .tasks
        .get(&TaskId::new(dispatch.node().as_str())?)
        .and_then(|task| task.frozen.clone())
        .ok_or("busy start absent")?;
    let decision = fixture.record(&workflow)?.decision_generation;
    let next = fixture
        .runtime
        .tick(1)?
        .pop()
        .ok_or("unrelated task starved")?;
    assert_ne!(next.node(), dispatch.node());
    assert!(
        fixture.runtime.tick(1)?.is_empty(),
        "busy start must respect global capacity"
    );
    fixture.runtime.execute(next)?;
    assert_eq!(fixture.executor.inner.entry_count(), 1);
    let global = fixture
        .control
        .create(&context()?, "other-workflow", ready()?)?
        .workflow;
    for _ in 0..4 {
        if let Some(next) = fixture.runtime.tick(1)?.pop() {
            fixture.runtime.execute(next)?;
        }
    }
    assert!(
        fixture
            .record(&global)?
            .tasks
            .values()
            .all(|task| task.state == TaskState::Completed)
    );
    let before = fixture.record(&workflow)?;
    let usage = fixture.store.workflow_storage_usage()?;
    for _ in 0..64 {
        let batch = fixture.runtime.tick(1)?;
        assert!(batch.len() <= 1);
        for retry in batch {
            assert_eq!(retry, dispatch);
            fixture.runtime.execute(retry)?;
        }
    }
    let after = fixture.record(&workflow)?;
    let retained = after
        .tasks
        .get(&TaskId::new(dispatch.node().as_str())?)
        .ok_or("busy task absent")?;
    assert_eq!(retained.state, TaskState::Queued);
    assert_eq!(retained.reason.as_deref(), Some("Project is in use"));
    assert_eq!(retained.frozen.as_ref(), Some(&frozen));
    assert_eq!(after.decision_generation, before.decision_generation);
    assert!(
        after.decision_generation > decision,
        "the other task's initial start is a decision"
    );
    assert_eq!(after.committed_allowance, before.committed_allowance);
    assert_eq!(after.active_execution_ms, before.active_execution_ms);
    assert_eq!(after.tasks.len(), before.tasks.len());
    assert_eq!(after.admitted_tasks, before.admitted_tasks);
    let after_usage = fixture.store.workflow_storage_usage()?;
    assert_eq!(after_usage.receipt_entries, usage.receipt_entries);
    assert_eq!(after_usage.receipt_bytes, usage.receipt_bytes);
    assert!(
        after_usage.history_bytes <= usage.history_bytes + 16,
        "only bounded generation digits may grow"
    );
    // The snapshot read before these retries still authorizes a pause: only progress changed.
    fixture.control.edit(
        &context()?,
        "pause-after-retries",
        &workflow,
        TaskBatch {
            expected_generation: before.generation,
            changes: vec![TaskChange::Wait {
                question: "Pause the busy task".into(),
            }],
        },
    )?;
    for _ in 0..2 {
        assert!(fixture.runtime.tick(1)?.is_empty());
    }
    let paused = fixture.record(&workflow)?;
    assert!(
        fixture
            .control
            .edit(
                &context()?,
                "stale-control",
                &workflow,
                TaskBatch {
                    expected_generation: before.generation,
                    changes: vec![TaskChange::Continue],
                }
            )
            .is_err()
    );
    assert!(
        fixture
            .control
            .edit(
                &context()?,
                "future-control",
                &workflow,
                TaskBatch {
                    expected_generation: paused.generation + 1,
                    changes: vec![TaskChange::Continue],
                }
            )
            .is_err()
    );
    fixture.edit(&workflow, "resume-busy", TaskChange::Continue)?;
    *fixture.executor.busy.lock().map_err(|_| "busy lock")? = None;
    for _ in 0..2 {
        if let Some(retry) = fixture.runtime.tick(1)?.pop() {
            assert_eq!(retry, dispatch);
            fixture.runtime.execute(retry)?;
        }
    }
    assert_eq!(fixture.executor.inner.entry_count(), 3);
    Ok(())
}

#[test]
fn unresolved_pre_entry_cleanup_remains_uncertain_after_stop_and_restart() -> TestResult {
    let root = tempfile::tempdir()?;
    let clock = Arc::new(ManualClock::new(1000));
    let authority = Arc::new(InvocationAllowance(Some(8)));
    let (workflow, frozen) = {
        let fixture = Fixture::open(root.path(), authority.clone(), clock.clone())?;
        let workflow = fixture
            .control
            .create(&context()?, "create", ready()?)?
            .workflow;
        let dispatch = fixture.runtime.tick(1)?.pop().ok_or("start absent")?;
        let task = TaskId::new(dispatch.node().as_str())?;
        *fixture.executor.busy.lock().map_err(|_| "busy lock")? =
            Some(dispatch.node().as_str().to_owned());
        fixture.executor.fail_cleanup.store(true, Ordering::SeqCst);
        fixture.runtime.execute(dispatch)?;
        fixture.edit(&workflow, "stop-uncertain", TaskChange::Stop { task })?;
        for action in fixture.runtime.cancellation_actions(1)? {
            if let EffectAction::Cancel(dispatch) = action {
                fixture.runtime.execute_cancellation(&dispatch)?;
            }
        }
        let record = fixture.record(&workflow)?;
        let task = record.tasks.values().next().ok_or("task absent")?;
        assert_eq!(task.state, TaskState::Uncertain);
        let frozen = task.frozen.clone().ok_or("frozen start absent")?;
        assert!(!frozen.entered);
        assert!(frozen.execution_duration.is_some());
        assert_eq!(fixture.executor.inner.entry_count(), 0);
        assert_eq!(fixture.executor.settlements.load(Ordering::SeqCst), 0);
        (workflow, frozen)
    };
    let fixture = Fixture::open(root.path(), authority, clock)?;
    fixture.runtime.recover()?;
    let record = fixture.record(&workflow)?;
    let task = record.tasks.values().next().ok_or("task absent")?;
    assert_eq!(task.state, TaskState::Uncertain);
    assert_eq!(task.frozen.as_ref(), Some(&frozen));
    assert!(fixture.runtime.tick(1)?.is_empty());
    assert_eq!(fixture.executor.inner.entry_count(), 0);
    assert_eq!(fixture.executor.deferrals.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.executor.settlements.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn resumed_start_still_rechecks_authority_before_entry() -> TestResult {
    let root = tempfile::tempdir()?;
    let authority = Arc::new(RevocableAuthority(AtomicBool::new(false)));
    let fixture = Fixture::open(
        root.path(),
        authority.clone(),
        Arc::new(ManualClock::new(1000)),
    )?;
    let workflow = fixture
        .control
        .create(&context()?, "create", ready()?)?
        .workflow;
    let dispatch = fixture.runtime.tick(1)?.pop().ok_or("start absent")?;
    fixture.edit(
        &workflow,
        "pause",
        TaskChange::Wait {
            question: "Wait for access review".into(),
        },
    )?;
    fixture.runtime.execute(dispatch.clone())?;
    fixture.edit(&workflow, "resume", TaskChange::Continue)?;
    let retry = fixture
        .runtime
        .tick(1)?
        .pop()
        .ok_or("resumed start absent")?;
    assert_eq!(retry, dispatch);
    authority.0.store(true, Ordering::SeqCst);
    fixture.runtime.execute(retry)?;
    assert_eq!(fixture.executor.inner.entry_count(), 0);
    assert!(
        fixture
            .record(&workflow)?
            .tasks
            .values()
            .all(|task| task.state == TaskState::Failed)
    );
    assert_eq!(fixture.executor.settlements.load(Ordering::SeqCst), 1);
    Ok(())
}
