//! Pausing prepared work releases real host resources without inventing a terminal result.

use super::*;
use milkdrift_blueprint::{TaskBatch, TaskChange, TaskPathAccess, WorkflowId};
use milkdrift_capability_host::PreparedAdapterExecution;
use milkdrift_persistence::NativePathClaimStore;
use milkdrift_runtime::{ExecutionDispatch, ExecutorError, TaskState, WorkflowRecord};
use std::sync::{
    atomic::AtomicBool,
    mpsc::{Receiver, SyncSender, sync_channel},
};

struct PreparedToken(Arc<AtomicUsize>);

impl Drop for PreparedToken {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct PausingAdapter {
    inner: BlockingAdapter,
    prepared: SyncSender<()>,
    resume: Mutex<Receiver<()>>,
    paused_once: AtomicBool,
    dropped: Arc<AtomicUsize>,
}

impl CapabilityAdapter for PausingAdapter {
    fn prepare(
        self: Arc<Self>,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<PreparedAdapterExecution, AdapterError> {
        let envelope = self.admission_envelope(invocation)?;
        let token = PreparedToken(self.dropped.clone());
        if !self.paused_once.swap(true, Ordering::SeqCst) {
            self.prepared
                .send(())
                .map_err(|error| AdapterError::rejected(error.to_string()))?;
            self.resume
                .lock()
                .map_err(|_| AdapterError::rejected("preparation fixture unavailable"))?
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| AdapterError::rejected(error.to_string()))?;
        }
        Ok(PreparedAdapterExecution::new(
            envelope,
            move |invocation, reporter| {
                let _token = token;
                self.inner.execute(invocation, reporter)
            },
        ))
    }

    fn admission_envelope(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        self.inner.admission_envelope(invocation)
    }

    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.inner.authority_requirements()
    }

    fn start(&self) -> Result<(), AdapterError> {
        self.inner.start()
    }

    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.inner.execute(invocation, reporter)
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        self.inner.cancel(request)
    }

    fn health(&self, now: u64) -> Result<CapabilityObservation, AdapterError> {
        self.inner.health(now)
    }

    fn begin_drain(&self) -> Result<(), AdapterError> {
        self.inner.begin_drain()
    }

    fn shutdown(&self) -> Result<(), AdapterError> {
        self.inner.shutdown()
    }
}

struct Fixture {
    store: Arc<RedbStore>,
    host: CapabilityHost,
    runtime: Arc<TaskRuntime>,
    workflow: WorkflowId,
    adapter: Arc<PausingAdapter>,
    prepared: Receiver<()>,
    resume: SyncSender<()>,
}

impl Fixture {
    fn open(root: &std::path::Path) -> TestResult<Self> {
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(root.join("store")).with_clock(Arc::new(FixtureClock)),
        )?);
        let host = CapabilityHost::new(
            HostConfig {
                max_registrations: 1,
                max_generations_per_capability: 1,
                max_concurrent_per_generation: 1,
                observation_stale_after_ms: 10_000,
            },
            CapabilitySelectionPolicy::priorities(BTreeMap::new()),
        )?;
        host.install_native_path_store(store.clone(), Vec::new())?;
        let (prepared_sender, prepared) = sync_channel(1);
        let (resume, resume_receiver) = sync_channel(1);
        let adapter = Arc::new(PausingAdapter {
            inner: BlockingAdapter::new(Arc::new((Mutex::new(true), Condvar::new()))),
            prepared: prepared_sender,
            resume: Mutex::new(resume_receiver),
            paused_once: AtomicBool::new(false),
            dropped: Arc::new(AtomicUsize::new(0)),
        });
        let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
            "../../../capability/tests/fixtures/descriptor-v1.json"
        ))?
        .body()
        .clone();
        host.register(
            descriptor.clone(),
            adapter.clone(),
            Some(CapabilityObservation::new(
                descriptor.identity().clone(),
                1_000,
                true,
                0,
                "ready",
            )?),
        )?;
        let runtime = Arc::new(TaskRuntime::new(
            store.clone(),
            Arc::new(host.clone()),
            Arc::new(AllowAuthority),
            Arc::new(ManualClock::new(1_000)),
            Arc::new(SequentialIdGenerator::new("prepared-pause", 1)?),
        ));
        let project = root.join("project");
        std::fs::create_dir(&project)?;
        create_with_paths(
            &runtime,
            1,
            vec![TaskPathAccess {
                path: project.to_string_lossy().into_owned(),
                write: true,
            }],
        )?;
        Ok(Self {
            store,
            host,
            runtime,
            workflow: TaskRuntime::workflow_identity(
                "human:effect-worker-test",
                "worker-create-1",
            )?,
            adapter,
            prepared,
            resume,
        })
    }

    fn record(&self) -> TestResult<WorkflowRecord> {
        self.runtime
            .workflow(&self.workflow)?
            .ok_or_else(|| "prepared workflow absent".into())
    }

    fn edit(&self, key: &str, change: TaskChange) -> TestResult {
        self.runtime.edit(
            "human:effect-worker-test",
            key,
            &self.workflow,
            TaskBatch {
                expected_generation: self.record()?.generation,
                changes: vec![change],
            },
        )?;
        Ok(())
    }

    fn dispatch(&self) -> TestResult<ExecutionDispatch> {
        self.runtime
            .tick(1)?
            .pop()
            .ok_or_else(|| "prepared dispatch absent".into())
    }

    fn active_permits(&self) -> TestResult<u32> {
        Ok(self
            .host
            .generations(
                &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
                1_000,
            )?
            .first()
            .ok_or("prepared generation absent")?
            .active_permits)
    }
}

#[test]
fn pause_during_preparation_releases_resources_and_resumes_through_worker_polling() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path())?;
    let dispatch = fixture.dispatch()?;
    let original = dispatch.clone();
    let runtime = fixture.runtime.clone();
    let worker = std::thread::spawn(move || runtime.execute(dispatch));
    fixture.prepared.recv_timeout(Duration::from_secs(5))?;
    fixture.edit(
        "pause",
        TaskChange::Wait {
            question: "Pause work".into(),
        },
    )?;
    assert_eq!(fixture.active_permits()?, 1);
    assert!(
        fixture
            .store
            .native_path_claim(original.attempt().as_str())?
            .is_some()
    );
    assert_eq!(fixture.adapter.dropped.load(Ordering::SeqCst), 0);
    fixture.resume.send(())?;
    worker.join().map_err(|_| "prepared worker panicked")??;

    let record = fixture.record()?;
    let task = record.tasks.values().next().ok_or("prepared task absent")?;
    assert_eq!(task.state, TaskState::Queued);
    assert_eq!(
        task.frozen.as_ref().ok_or("frozen start absent")?.dispatch,
        original
    );
    assert!(task.ended_at.is_none());
    assert!(task.result.is_none());
    assert_eq!(fixture.adapter.inner.entered.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.adapter.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.active_permits()?, 0);
    assert!(
        fixture
            .store
            .native_path_claim(original.attempt().as_str())?
            .is_none()
    );

    let workers = EffectWorkerHost::start(
        fixture.runtime.clone(),
        fixture.host.clone(),
        worker_config(),
    )?;
    assert_eq!(workers.poll()?.executions, 0);
    fixture.edit("continue", TaskChange::Continue)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !fixture
        .record()?
        .tasks
        .values()
        .all(|task| task.state.is_settled())
        && std::time::Instant::now() < deadline
    {
        workers.poll()?;
        std::thread::sleep(Duration::from_millis(5));
    }
    let shutdown = workers.shutdown(EffectShutdownMode::Drain, Duration::from_secs(5))?;
    assert!(shutdown.clean);
    let record = fixture.record()?;
    let task = record.tasks.values().next().ok_or("resumed task absent")?;
    assert_eq!(task.state, TaskState::Completed);
    assert_eq!(
        task.frozen
            .as_ref()
            .ok_or("resumed start absent")?
            .dispatch
            .attempt(),
        original.attempt()
    );
    assert_eq!(fixture.adapter.inner.entered.load(Ordering::SeqCst), 1);
    assert_eq!(shutdown.health.failed_actions, 0);
    assert!(
        fixture
            .store
            .native_path_claim(original.attempt().as_str())?
            .is_none()
    );
    Ok(())
}

#[test]
fn stop_during_preparation_keeps_claim_until_prepared_resources_are_dropped() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path())?;
    let dispatch = fixture.dispatch()?;
    let original = dispatch.clone();
    let runtime = fixture.runtime.clone();
    let worker = std::thread::spawn(move || runtime.execute(dispatch));
    fixture.prepared.recv_timeout(Duration::from_secs(5))?;
    let task = fixture
        .record()?
        .tasks
        .keys()
        .next()
        .ok_or("prepared task absent")?
        .clone();
    fixture.edit("stop", TaskChange::Stop { task })?;
    let cancellation = fixture
        .runtime
        .cancellation_actions(1)?
        .pop()
        .ok_or("stop action absent")?;
    let milkdrift_runtime::EffectAction::Cancel(cancellation) = cancellation else {
        return Err("expected cancellation action".into());
    };
    fixture.runtime.execute_cancellation(&cancellation)?;
    assert!(
        fixture
            .store
            .native_path_claim(original.attempt().as_str())?
            .is_some()
    );
    assert_eq!(fixture.adapter.dropped.load(Ordering::SeqCst), 0);
    fixture.resume.send(())?;
    worker.join().map_err(|_| "prepared worker panicked")??;
    assert_eq!(
        fixture
            .record()?
            .tasks
            .values()
            .next()
            .ok_or("stopped task absent")?
            .state,
        TaskState::Stopped
    );
    assert_eq!(fixture.adapter.inner.entered.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.adapter.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.active_permits()?, 0);
    assert!(
        fixture
            .store
            .native_path_claim(original.attempt().as_str())?
            .is_none()
    );
    Ok(())
}

#[test]
fn deferral_refuses_live_preparation_and_releases_only_after_drop() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path())?;
    fixture.adapter.paused_once.store(true, Ordering::SeqCst);
    let dispatch = fixture.dispatch()?;
    let prepared = fixture.host.prepare_exact_entry(&dispatch)?;
    assert!(matches!(
        fixture.host.defer_before_entry(&dispatch),
        Err(ExecutorError::BoundaryBeforeEntry(_))
    ));
    assert!(
        fixture
            .store
            .native_path_claim(dispatch.attempt().as_str())?
            .is_some()
    );
    assert_eq!(fixture.active_permits()?, 1);
    drop(prepared);
    fixture.host.defer_before_entry(&dispatch)?;
    assert_eq!(fixture.active_permits()?, 0);
    assert!(
        fixture
            .store
            .native_path_claim(dispatch.attempt().as_str())?
            .is_none()
    );
    assert_eq!(fixture.adapter.inner.entered.load(Ordering::SeqCst), 0);
    Ok(())
}

fn worker_config() -> EffectWorkerConfig {
    EffectWorkerConfig {
        execution_threads: 1,
        execution_queue: 1,
        cancellation_queue: 1,
        maximum_claim_page: 1,
    }
}

struct ReleaseGate(Arc<(Mutex<bool>, Condvar)>);

impl Drop for ReleaseGate {
    fn drop(&mut self) {
        let (lock, changed) = &*self.0;
        if let Ok(mut released) = lock.lock() {
            *released = true;
            changed.notify_all();
        }
    }
}

#[test]
fn native_contention_frees_a_worker_for_unrelated_work_and_resumes_once() -> TestResult {
    let root = tempfile::tempdir()?;
    let first = root.path().join("first");
    let other = root.path().join("other");
    std::fs::create_dir(&first)?;
    std::fs::create_dir(&other)?;
    let store = Arc::new(RedbStore::open_with_config(
        RedbStoreConfig::new(root.path().join("store")).with_clock(Arc::new(FixtureClock)),
    )?);
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 1,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 3,
            observation_stale_after_ms: 10_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    host.install_native_path_store(store.clone(), Vec::new())?;
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let release = ReleaseGate(gate.clone());
    let adapter = Arc::new(BlockingAdapter::new(gate));
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    host.register(
        descriptor.clone(),
        adapter.clone(),
        Some(CapabilityObservation::new(
            descriptor.identity().clone(),
            1_000,
            true,
            0,
            "ready",
        )?),
    )?;
    let runtime = Arc::new(TaskRuntime::new(
        store.clone(),
        Arc::new(host.clone()),
        Arc::new(AllowAuthority),
        Arc::new(ManualClock::new(1_000)),
        Arc::new(SequentialIdGenerator::new("busy-worker", 1)?),
    ));
    let paths = |path: &std::path::Path| {
        vec![TaskPathAccess {
            path: path.to_string_lossy().into_owned(),
            write: true,
        }]
    };
    create_with_paths(&runtime, 0, paths(&first))?;
    let workers = EffectWorkerHost::start(
        runtime.clone(),
        host,
        EffectWorkerConfig {
            execution_threads: 2,
            execution_queue: 2,
            ..worker_config()
        },
    )?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while adapter.entered.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
        workers.poll()?;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(adapter.entered.load(Ordering::SeqCst), 1);
    create_with_paths(&runtime, 1, paths(&first))?;
    let busy_id = TaskRuntime::workflow_identity("human:effect-worker-test", "worker-create-1")?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let busy_dispatch = loop {
        workers.poll()?;
        let workflow = runtime.workflow(&busy_id)?.ok_or("busy workflow absent")?;
        let task = workflow.tasks.values().next().ok_or("busy task absent")?;
        if task.state == TaskState::Queued && task.reason.as_deref() == Some("Project is in use") {
            break task
                .frozen
                .as_ref()
                .ok_or("busy start absent")?
                .dispatch
                .clone();
        }
        if std::time::Instant::now() >= deadline {
            return Err("native contention did not release its worker".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    create_with_paths(&runtime, 2, paths(&other))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while adapter.entered.load(Ordering::SeqCst) < 2 && std::time::Instant::now() < deadline {
        workers.poll()?;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(adapter.entered.load(Ordering::SeqCst), 2);
    assert!(
        store
            .native_path_claim(busy_dispatch.attempt().as_str())?
            .is_none()
    );
    let workflow = runtime.workflow(&busy_id)?.ok_or("busy workflow absent")?;
    assert!(
        !workflow
            .tasks
            .values()
            .next()
            .and_then(|task| task.frozen.as_ref())
            .ok_or("busy start absent")?
            .entered
    );
    drop(release);

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while adapter.entered.load(Ordering::SeqCst) < 3 && std::time::Instant::now() < deadline {
        workers.poll()?;
        std::thread::sleep(Duration::from_millis(5));
    }
    let shutdown = workers.shutdown(EffectShutdownMode::Drain, Duration::from_secs(5))?;
    assert!(shutdown.clean);
    assert_eq!(shutdown.health.failed_actions, 0);
    assert_eq!(adapter.entered.load(Ordering::SeqCst), 3);
    for index in 0..3 {
        let id = TaskRuntime::workflow_identity(
            "human:effect-worker-test",
            &format!("worker-create-{index}"),
        )?;
        let workflow = runtime.workflow(&id)?.ok_or("completed workflow absent")?;
        let task = workflow
            .tasks
            .values()
            .next()
            .ok_or("completed task absent")?;
        assert_eq!(task.state, TaskState::Completed);
        if index == 1 {
            assert_eq!(
                task.frozen
                    .as_ref()
                    .ok_or("completed start absent")?
                    .dispatch
                    .attempt(),
                busy_dispatch.attempt()
            );
        }
    }
    Ok(())
}
