//! Controlled external work exercises the production duration ledger, entry and recovery owner.
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};

use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    DecisionReasonCode, PolicyId,
};
use milkdrift_blueprint::{
    NewTask, TaskBatch, TaskChange, TaskDefinition, TaskId, WorkflowCreate, WorkflowId,
    WorkflowLimits,
};
use milkdrift_capability::{
    AdmissionBound, CancellationAcknowledgement, CancellationRequest, CapabilityDescriptorDocument,
    CapabilityRequirement, InvocationAdmissionEnvelope, InvocationEvent, InvocationEventKind,
    InvocationTerminal, OperationId, SideEffectClass, TerminalStatus,
};
use milkdrift_control::{ActorAuthorityContext, TaskControlService};
use milkdrift_persistence::{StorageFailureClass, WorkflowStore};
use milkdrift_redb_store::{RedbStore, RedbStoreConfig, StoreClock};
use milkdrift_runtime::{
    BoundaryClock, DeterministicExecutor, ExecutionDispatch, ExecutorError, ManualClock,
    PreparedExecution, ResolvedCapability, RuntimeError, SequentialIdGenerator, TaskExecutor,
    TaskRuntime, TaskState, WorkflowRecord,
};

use super::{TestResult, claim};

#[path = "duration_policy.rs"]
mod policy;

const WAIT: Duration = Duration::from_secs(5);

struct PermitAuthority;
impl AuthorityEvaluator for PermitAuthority {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.duration")?,
            1,
            request.clone(),
            vec![DecisionReasonCode::Allowed],
            AuthorityBudget {
                duration_ms: Some(u64::MAX),
                invocations: Some(256),
                concurrency: Some(4),
                artifact_bytes: Some(u64::MAX),
                ..AuthorityBudget::default()
            },
            SideEffectClass::Unknown,
        )
    }
}

struct Clock(Arc<ManualClock>);
impl StoreClock for Clock {
    fn now(
        &self,
    ) -> Result<milkdrift_persistence::TimestampMillis, milkdrift_persistence::PersistenceError>
    {
        self.0
            .now()
            .map_err(|error| milkdrift_persistence::PersistenceError::Storage {
                class: StorageFailureClass::Unavailable,
                message: error.to_string(),
            })
    }
}

enum Step {
    Heartbeat(SyncSender<bool>),
    Terminal(TerminalStatus),
    LoseOutcome,
}

struct ControlledExecutor {
    resolver: DeterministicExecutor,
    maximum_ms: AtomicU64,
    uncertain_preparation: AtomicBool,
    entries: AtomicU64,
    entered: SyncSender<SyncSender<Step>>,
}

impl TaskExecutor for ControlledExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        now: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement, now)
    }

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        if self.uncertain_preparation.load(Ordering::SeqCst) {
            return Err(ExecutorError::Overloaded(
                "another worker may own this invocation".into(),
            ));
        }
        let maximum = self.maximum_ms.load(Ordering::SeqCst);
        let duration = if maximum == u64::MAX {
            AdmissionBound::Unknown
        } else {
            AdmissionBound::Bounded(maximum)
        };
        Ok(PreparedExecution::new(
            dispatch,
            InvocationAdmissionEnvelope::not_applicable().with_execution_duration_ms(duration),
            move |dispatch, reporter| {
                self.entries.fetch_add(1, Ordering::SeqCst);
                let (steps, next) = mpsc::sync_channel(4);
                self.entered.send(steps).map_err(boundary)?;
                loop {
                    match next.recv_timeout(WAIT).map_err(boundary)? {
                        Step::Heartbeat(reply) => {
                            reply.send(reporter.heartbeat().is_ok()).map_err(boundary)?;
                        }
                        Step::Terminal(status) => {
                            reporter.invocation(InvocationEvent::new(
                                dispatch.request().invocation().clone(),
                                1,
                                InvocationEventKind::Terminal {
                                    terminal: InvocationTerminal::new(
                                        status,
                                        Vec::new(),
                                        None,
                                        None,
                                        SideEffectClass::None,
                                    )?,
                                },
                            )?)?;
                            return Ok(());
                        }
                        Step::LoseOutcome => return Err(boundary("outcome unavailable")),
                    }
                }
            },
        ))
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            true,
            false,
            None,
        )
        .map_err(ExecutorError::from)
    }
}

fn boundary(error: impl std::fmt::Display) -> ExecutorError {
    ExecutorError::BoundaryAfterEntry(error.to_string())
}

struct Fixture {
    runtime: Arc<TaskRuntime>,
    executor: Arc<ControlledExecutor>,
    entered: Receiver<SyncSender<Step>>,
    control: TaskControlService,
    store: Arc<RedbStore>,
    clock: Arc<ManualClock>,
}

impl Fixture {
    fn open(root: &std::path::Path, clock: Arc<ManualClock>, maximum: u64) -> TestResult<Self> {
        Self::open_with_storage_limits(
            root,
            clock,
            maximum,
            milkdrift_persistence::WorkflowStorageLimits::default(),
        )
    }

    fn open_with_storage_limits(
        root: &std::path::Path,
        clock: Arc<ManualClock>,
        maximum: u64,
        limits: milkdrift_persistence::WorkflowStorageLimits,
    ) -> TestResult<Self> {
        let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../capability/tests/fixtures/descriptor-v1.json"
        ))?;
        value
            .get_mut("descriptor")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("fixture descriptor absent")?
            .insert("category".into(), serde_json::json!({"type":"process"}));
        let descriptor = CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&value)?)?;
        let (entered, receiver) = mpsc::sync_channel(4);
        let executor = Arc::new(ControlledExecutor {
            resolver: DeterministicExecutor::new(descriptor.body().clone()),
            maximum_ms: AtomicU64::new(maximum),
            uncertain_preparation: AtomicBool::new(false),
            entries: AtomicU64::new(0),
            entered,
        });
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(root)
                .with_clock(Arc::new(Clock(clock.clone())))
                .with_workflow_limits(limits),
        )?);
        let authority = Arc::new(PermitAuthority);
        let runtime = Arc::new(TaskRuntime::new(
            store.clone(),
            executor.clone(),
            authority.clone(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new(
                format!("duration-{}", clock.now()?.get()),
                1,
            )?),
        ));
        let control =
            TaskControlService::new(runtime.clone(), authority, clock.clone(), store.clone());
        Ok(Self {
            runtime,
            executor,
            entered: receiver,
            control,
            store,
            clock,
        })
    }

    fn create(&self, tasks: usize) -> TestResult<WorkflowId> {
        let request = WorkflowCreate {
            project_id: None,
            title: "Cumulative duration".into(),
            description: String::new(),
            tasks: (0..tasks)
                .map(|index| {
                    Ok(NewTask {
                        alias: TaskId::new(format!("task-{index}"))?,
                        definition: definition()?,
                    })
                })
                .collect::<TestResult<_>>()?,
            inputs: BTreeMap::new(),
            limits: WorkflowLimits {
                active_execution_ms: 10_000,
                ..WorkflowLimits::default()
            },
            finish_when_settled: true,
        };
        Ok(self
            .control
            .create(&context()?, "create", request)?
            .workflow)
    }

    fn record(&self, id: &WorkflowId) -> TestResult<WorkflowRecord> {
        self.runtime
            .workflow(id)?
            .ok_or_else(|| "workflow absent".into())
    }

    fn start(&self, dispatch: ExecutionDispatch) -> TestResult<Active> {
        let runtime = self.runtime.clone();
        let worker = std::thread::spawn(move || runtime.execute(dispatch));
        let steps = self.entered.recv_timeout(WAIT)?;
        Ok(Active { steps, worker })
    }

    fn add(&self, id: &WorkflowId, key: &str) -> TestResult {
        let record = self.record(id)?;
        self.control.edit(
            &context()?,
            key,
            id,
            TaskBatch {
                expected_generation: record.generation,
                changes: vec![TaskChange::Add {
                    alias: TaskId::new(key)?,
                    definition: definition()?,
                }],
            },
        )?;
        Ok(())
    }
}

struct Active {
    steps: SyncSender<Step>,
    worker: JoinHandle<Result<(), RuntimeError>>,
}
impl Active {
    fn heartbeat(&self) -> TestResult<bool> {
        let (send, reply) = mpsc::sync_channel(1);
        self.steps.send(Step::Heartbeat(send))?;
        Ok(reply.recv_timeout(WAIT)?)
    }
    fn finish(self, status: TerminalStatus) -> TestResult {
        self.steps.send(Step::Terminal(status))?;
        self.worker
            .join()
            .map_err(|_| "duration worker panicked")??;
        Ok(())
    }
    fn lose_outcome(self) -> TestResult {
        self.steps.send(Step::LoseOutcome)?;
        self.worker
            .join()
            .map_err(|_| "duration worker panicked")??;
        Ok(())
    }
}

fn context() -> TestResult<ActorAuthorityContext> {
    Ok(ActorAuthorityContext::new(
        milkdrift_authority::ActorRef::new("human:duration")?,
        claim()?,
    ))
}

fn definition() -> TestResult<TaskDefinition> {
    Ok(TaskDefinition {
        capability_generation: None,
        execution: milkdrift_blueprint::TaskExecution::Legacy,
        title: "Controlled duration".into(),
        description: String::new(),
        instructions: "Use only the admitted execution duration.".into(),
        capability: CapabilityRequirement::new(OperationId::new("model.generate")?),
        inputs: BTreeMap::new(),
        dependencies: Default::default(),
        outputs: BTreeMap::new(),
        required: true,
        corrective_of: None,
        paths: Vec::new(),
        tools: Vec::new(),
        actions: None,
    })
}

#[test]
fn serial_near_exhaustion_reserves_only_the_shared_remainder() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path(), Arc::new(ManualClock::new(1_000)), 10_000)?;
    let id = fixture.create(1)?;
    let active = fixture.start(
        fixture
            .runtime
            .tick(4)?
            .pop()
            .ok_or("first dispatch absent")?,
    )?;
    fixture.clock.advance(9_000)?;
    active.finish(TerminalStatus::Success)?;
    assert_eq!(fixture.record(&id)?.active_execution_ms, 9_000);
    fixture.add(&id, "next")?;
    let next = fixture
        .runtime
        .tick(4)?
        .pop()
        .ok_or("next dispatch absent")?;
    assert_eq!(next.lease_expires_at().get(), 11_000);
    fixture.runtime.execute(next)?;
    assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 1);
    let record = fixture.record(&id)?;
    assert_eq!(record.active_execution_ms, 9_000);
    assert!(
        record
            .wait_question
            .as_deref()
            .is_some_and(|reason| reason.contains("cannot reserve"))
    );
    assert!(
        record
            .tasks
            .values()
            .any(|task| task.state == TaskState::Failed
                && task.frozen.as_ref().is_some_and(|start| !start.entered))
    );
    Ok(())
}

#[test]
fn exact_serial_remainder_allows_only_its_final_millisecond() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path(), Arc::new(ManualClock::new(1_000)), 10_000)?;
    let id = fixture.create(1)?;
    let first = fixture.start(
        fixture
            .runtime
            .tick(4)?
            .pop()
            .ok_or("first dispatch absent")?,
    )?;
    fixture.clock.advance(9_000)?;
    first.finish(TerminalStatus::Success)?;
    fixture.executor.maximum_ms.store(1_000, Ordering::SeqCst);
    fixture.add(&id, "final-second")?;
    let second = fixture.start(
        fixture
            .runtime
            .tick(4)?
            .pop()
            .ok_or("second dispatch absent")?,
    )?;
    fixture.clock.advance(999)?;
    assert!(second.heartbeat()?);
    assert_eq!(fixture.record(&id)?.active_execution_ms, 9_999);
    fixture.clock.advance(1)?;
    assert!(!second.heartbeat()?);
    second.finish(TerminalStatus::Cancelled)?;
    let record = fixture.record(&id)?;
    assert_eq!(record.active_execution_ms, 10_000);
    assert!(
        record
            .tasks
            .values()
            .any(|task| task.state == TaskState::Stopped && task.ended_at.is_some())
    );
    assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn ambiguous_preparation_cannot_be_refunded_by_a_stop_acknowledgement() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path(), Arc::new(ManualClock::new(1_000)), 10_000)?;
    let id = fixture.create(1)?;
    fixture
        .executor
        .uncertain_preparation
        .store(true, Ordering::SeqCst);
    fixture
        .runtime
        .execute(fixture.runtime.tick(4)?.pop().ok_or("dispatch absent")?)?;
    let record = fixture.record(&id)?;
    let task = record.tasks.values().next().ok_or("task absent")?;
    assert_eq!(task.state, TaskState::Uncertain);
    assert!(
        task.frozen
            .as_ref()
            .is_some_and(|start| !start.entered && start.execution_duration.is_some())
    );
    fixture.control.edit(
        &context()?,
        "stop-unknown",
        &id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Stop {
                task: task.id.clone(),
            }],
        },
    )?;
    let action = fixture
        .runtime
        .cancellation_actions(4)?
        .pop()
        .ok_or("cancel absent")?;
    let milkdrift_runtime::EffectAction::Cancel(cancellation) = action else {
        return Err("wrong cancellation action".into());
    };
    fixture.runtime.execute_cancellation(&cancellation)?;
    fixture.clock.advance(3_000)?;
    assert!(fixture.runtime.tick(4)?.is_empty());
    let record = fixture.record(&id)?;
    assert_eq!(record.active_execution_ms, 3_000);
    assert!(
        record
            .tasks
            .values()
            .all(|task| task.state == TaskState::Uncertain
                && task.cancellation_sent
                && task.ended_at.is_none())
    );
    assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn concurrent_entries_cannot_both_reserve_the_same_remainder() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path(), Arc::new(ManualClock::new(1_000)), 6_000)?;
    let id = fixture.create(2)?;
    let mut dispatches = fixture.runtime.tick(4)?;
    let active = fixture.start(dispatches.pop().ok_or("first dispatch absent")?)?;
    fixture.clock.advance(1_000)?;
    fixture
        .runtime
        .execute(dispatches.pop().ok_or("second dispatch absent")?)?;
    assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.record(&id)?.active_execution_ms, 1_000);
    fixture.clock.advance(4_000)?;
    assert!(active.heartbeat()?);
    active.finish(TerminalStatus::Success)?;
    assert_eq!(fixture.record(&id)?.active_execution_ms, 5_000);
    Ok(())
}

#[test]
fn stopping_work_retains_its_maximum_until_terminal_proof() -> TestResult {
    // Both explicit early stop and deadline-triggered stop must keep the reservation,
    // independently of whether the adapter has acknowledged the cancellation request.
    for (elapsed, acknowledge) in [(2_000, false), (2_000, true), (4_000, false), (4_000, true)] {
        let root = tempfile::tempdir()?;
        let fixture = Fixture::open(root.path(), Arc::new(ManualClock::new(1_000)), 4_000)?;
        let id = fixture.create(2)?;
        let mut dispatches = fixture.runtime.tick(4)?;
        let first_dispatch = dispatches.pop().ok_or("first dispatch absent")?;
        let first_id = TaskId::new(first_dispatch.node().as_str())?;
        let first = fixture.start(first_dispatch)?;
        fixture.clock.advance(elapsed)?;
        if elapsed < 4_000 {
            let record = fixture.record(&id)?;
            fixture.control.edit(
                &context()?,
                "early-stop",
                &id,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![TaskChange::Stop {
                        task: first_id.clone(),
                    }],
                },
            )?;
        }
        assert!(!first.heartbeat()?);
        if acknowledge {
            let action = fixture
                .runtime
                .cancellation_actions(4)?
                .pop()
                .ok_or("cancel absent")?;
            let milkdrift_runtime::EffectAction::Cancel(cancellation) = action else {
                return Err("wrong cancellation action".into());
            };
            fixture.runtime.execute_cancellation(&cancellation)?;
        }
        fixture.executor.maximum_ms.store(6_000, Ordering::SeqCst);
        fixture
            .runtime
            .execute(dispatches.pop().ok_or("second dispatch absent")?)?;
        let record = fixture.record(&id)?;
        assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 1);
        assert_eq!(record.active_execution_ms, elapsed);
        let pending = record.tasks.get(&first_id).ok_or("first task absent")?;
        assert_eq!(pending.state, TaskState::Running);
        assert!(pending.stop_requested);
        assert_eq!(pending.cancellation_sent, acknowledge);
        assert!(pending.ended_at.is_none());
        first.finish(TerminalStatus::Cancelled)?;
        let record = fixture.record(&id)?;
        assert_eq!(record.active_execution_ms, elapsed);
        assert_eq!(
            record
                .tasks
                .get(&first_id)
                .ok_or("first task absent")?
                .state,
            TaskState::Stopped
        );
    }
    Ok(())
}

#[test]
fn concurrent_clocks_reach_exact_shared_boundary_before_continuing() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path(), Arc::new(ManualClock::new(1_000)), 5_000)?;
    let id = fixture.create(2)?;
    let mut dispatches = fixture.runtime.tick(4)?;
    let first = fixture.start(dispatches.pop().ok_or("first dispatch absent")?)?;
    let second = fixture.start(dispatches.pop().ok_or("second dispatch absent")?)?;
    fixture.clock.advance(4_999)?;
    assert!(first.heartbeat()?);
    assert_eq!(fixture.record(&id)?.active_execution_ms, 9_998);
    fixture.clock.advance(1)?;
    assert!(!second.heartbeat()?);
    let exhausted = fixture.record(&id)?;
    assert_eq!(exhausted.active_execution_ms, 10_000);
    assert!(
        exhausted
            .tasks
            .values()
            .all(|task| task.state == TaskState::Running && task.stop_requested)
    );
    assert_eq!(fixture.runtime.cancellation_actions(4)?.len(), 2);
    first.finish(TerminalStatus::Success)?;
    second.finish(TerminalStatus::Success)?;
    let settled = fixture.record(&id)?;
    assert_eq!(settled.active_execution_ms, 10_000);
    assert!(
        settled
            .tasks
            .values()
            .all(|task| task.state == TaskState::Completed)
    );
    assert!(fixture.runtime.tick(4)?.is_empty());
    Ok(())
}

#[test]
fn acknowledged_cancel_and_restart_do_not_refund_unknown_execution() -> TestResult {
    let root = tempfile::tempdir()?;
    let clock = Arc::new(ManualClock::new(1_000));
    let fixture = Fixture::open(root.path(), clock.clone(), 10_000)?;
    let id = fixture.create(1)?;
    let active = fixture.start(fixture.runtime.tick(4)?.pop().ok_or("dispatch absent")?)?;
    clock.advance(2_000)?;
    let record = fixture.record(&id)?;
    let task = record.tasks.keys().next().ok_or("task absent")?.clone();
    fixture.control.edit(
        &context()?,
        "stop",
        &id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Stop { task: task.clone() }],
        },
    )?;
    let cancellation = fixture
        .runtime
        .cancellation_actions(4)?
        .pop()
        .ok_or("cancel absent")?;
    let milkdrift_runtime::EffectAction::Cancel(cancellation) = cancellation else {
        return Err("wrong cancellation action".into());
    };
    fixture.runtime.execute_cancellation(&cancellation)?;
    let cancelled = fixture.record(&id)?;
    assert!(
        cancelled
            .tasks
            .get(&task)
            .is_some_and(|task| task.state == TaskState::Running
                && task.cancellation_sent
                && task.ended_at.is_none())
    );
    active.lose_outcome()?;
    assert_eq!(fixture.record(&id)?.active_execution_ms, 2_000);
    let frozen = fixture
        .record(&id)?
        .tasks
        .get(&task)
        .ok_or("task absent")?
        .frozen
        .clone();
    drop(fixture);
    clock.advance(3_000)?;
    let restarted = Fixture::open(root.path(), clock.clone(), 1_000)?;
    restarted.runtime.recover()?;
    assert!(restarted.runtime.tick(1)?.is_empty());
    let recovered = restarted.record(&id)?;
    assert_eq!(recovered.active_execution_ms, 5_000);
    assert_eq!(
        recovered
            .tasks
            .get(&task)
            .ok_or("task absent")?
            .frozen
            .as_ref()
            .map(|start| &start.dispatch),
        frozen.as_ref().map(|start| &start.dispatch)
    );
    restarted.add(&id, "after-restart")?;
    restarted.runtime.execute(
        restarted
            .runtime
            .tick(4)?
            .pop()
            .ok_or("new dispatch absent")?,
    )?;
    assert_eq!(restarted.executor.entries.load(Ordering::SeqCst), 0);
    assert_eq!(restarted.record(&id)?.active_execution_ms, 5_000);
    clock.advance(5_000)?;
    assert!(restarted.runtime.tick(4)?.is_empty());
    let exhausted = restarted.record(&id)?;
    assert_eq!(exhausted.active_execution_ms, 10_000);
    assert!(
        exhausted
            .tasks
            .get(&task)
            .is_some_and(|task| task.state == TaskState::Uncertain && task.ended_at.is_none())
    );
    Ok(())
}

#[test]
fn missing_enforceable_duration_refuses_entry_and_old_document_version_refuses_mutation()
-> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path(), Arc::new(ManualClock::new(1_000)), u64::MAX)?;
    let id = fixture.create(1)?;
    fixture
        .runtime
        .execute(fixture.runtime.tick(4)?.pop().ok_or("dispatch absent")?)?;
    assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 0);
    let mut snapshot = fixture.store.workflow(&id)?.ok_or("snapshot absent")?;
    let prior = snapshot.generation;
    snapshot.generation = prior.checked_add(1).ok_or("generation overflow")?;
    let document = snapshot
        .document
        .as_object_mut()
        .ok_or("workflow document is not an object")?;
    document.insert("generation".into(), serde_json::json!(snapshot.generation));
    document.insert("schema_version".into(), serde_json::json!(1));
    fixture
        .store
        .commit_workflow(milkdrift_persistence::WorkflowCommit {
            authoring: None,
            project_guard: None,
            project_id: None,
            expected_generation: Some(prior),
            snapshot: snapshot.clone(),
            command: None,
        })?;
    assert!(matches!(
        fixture.runtime.workflow(&id),
        Err(RuntimeError::InvalidHistory(_))
    ));
    assert!(fixture.runtime.tick(4).is_err());
    assert_eq!(
        fixture.store.workflow(&id)?.ok_or("snapshot absent")?,
        snapshot
    );
    Ok(())
}
