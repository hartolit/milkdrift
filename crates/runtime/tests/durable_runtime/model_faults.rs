//! Fail the actual redb transaction boundaries around response, admission and external entry.
use super::{InvocationAllowance, TaskStoreClock, TestResult, context, request};
use milkdrift_blueprint::{TaskActionPolicy, TaskBatch, TaskChange, TaskId};
use milkdrift_capability::{
    ArtifactReference, CancellationAcknowledgement, CancellationRequest,
    CapabilityDescriptorDocument, CapabilityRequirement, InvocationAdmissionEnvelope,
    InvocationEvent, InvocationEventKind, InvocationTerminal, SideEffectClass, TerminalStatus,
};
use milkdrift_control::TaskControlService;
use milkdrift_persistence::PersistenceError;
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use milkdrift_runtime::{
    DeterministicExecutor, ExecutionDispatch, ExecutorError, ManualClock, ModelAdmissionStatus,
    PreparedExecution, ResolvedCapability, SequentialIdGenerator, TaskExecutor, TaskRuntime,
    TaskState,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Fault {
    armed: Mutex<Option<FaultPoint>>,
}

impl Fault {
    fn arm(&self, point: FaultPoint) -> TestResult {
        *self.armed.lock().map_err(|_| "fault lock poisoned")? = Some(point);
        Ok(())
    }
}

impl FaultInjector for Fault {
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
        let mut armed = self.armed.lock().map_err(|_| injected_failure(point))?;
        if *armed == Some(point) {
            *armed = None;
            Err(injected_failure(point))
        } else {
            Ok(())
        }
    }
}

struct Executor {
    resolver: DeterministicExecutor,
    faults: Arc<Fault>,
    response_fault: Mutex<Option<FaultPoint>>,
    entries: AtomicUsize,
}

impl TaskExecutor for Executor {
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
        Ok(PreparedExecution::new(
            dispatch,
            InvocationAdmissionEnvelope::not_applicable(),
            move |dispatch, reporter| {
                self.entries.fetch_add(1, Ordering::SeqCst);
                let response = ArtifactReference::new(
                    "saved-response",
                    blake3::hash(b"immutable response").to_hex().to_string(),
                    Some("application/json".into()),
                    Some(18),
                )?;
                let response_fault = self
                    .response_fault
                    .lock()
                    .map_err(|_| {
                        ExecutorError::BoundaryAfterEntry("response fault lock poisoned".into())
                    })?
                    .take();
                if let Some(point) = response_fault {
                    self.faults
                        .arm(point)
                        .map_err(|error| ExecutorError::BoundaryAfterEntry(error.to_string()))?;
                }
                reporter.invocation(InvocationEvent::new(
                    dispatch.request().invocation().clone(),
                    1,
                    InvocationEventKind::Output {
                        name: "model_response".into(),
                        reference: response.clone(),
                    },
                )?)?;
                reporter.invocation(InvocationEvent::new(
                    dispatch.request().invocation().clone(),
                    2,
                    InvocationEventKind::Terminal {
                        terminal: InvocationTerminal::new(
                            TerminalStatus::Success,
                            vec![response],
                            None,
                            None,
                            SideEffectClass::None,
                        )?,
                    },
                )?)?;
                Ok(())
            },
        ))
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.resolver.cancel(request)
    }
}

struct Fixture {
    engine: Arc<TaskRuntime>,
    executor: Arc<Executor>,
    control: TaskControlService,
    faults: Arc<Fault>,
}

impl Fixture {
    fn open(path: &std::path::Path, label: &str) -> TestResult<Self> {
        let clock = Arc::new(ManualClock::new(1_000));
        let faults = Arc::new(Fault::default());
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(path)
                .with_clock(Arc::new(TaskStoreClock(clock.clone())))
                .with_fault_injector(faults.clone()),
        )?);
        let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
            "../../../capability/tests/fixtures/descriptor-v1.json"
        ))?;
        let executor = Arc::new(Executor {
            resolver: DeterministicExecutor::new(descriptor.body().clone()),
            faults: faults.clone(),
            response_fault: Mutex::new(None),
            entries: AtomicUsize::new(0),
        });
        let authority = Arc::new(InvocationAllowance(Some(8)));
        let engine = Arc::new(TaskRuntime::new(
            store,
            executor.clone(),
            authority.clone(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new(label, 1)?),
        ));
        let control = TaskControlService::new(engine.clone(), authority, clock);
        Ok(Self {
            engine,
            executor,
            control,
            faults,
        })
    }

    fn create(&self) -> TestResult<milkdrift_blueprint::WorkflowId> {
        self.create_with_repair(false)
    }

    fn create_with_repair(
        &self,
        automatic_repair: bool,
    ) -> TestResult<milkdrift_blueprint::WorkflowId> {
        let mut create = request()?;
        let task = &mut create.tasks.first_mut().ok_or("source absent")?.definition;
        task.inputs.clear();
        task.actions = Some(TaskActionPolicy { automatic_repair });
        Ok(self
            .control
            .create(&context()?, "fault-model-create", create)?
            .workflow)
    }

    fn child_batch(&self, id: &milkdrift_blueprint::WorkflowId) -> TestResult<(TaskId, TaskBatch)> {
        let record = self.engine.workflow(id)?.ok_or("workflow absent")?;
        let source = record
            .tasks
            .values()
            .find(|task| task.definition.actions.is_some())
            .ok_or("source absent")?;
        let mut child = source.definition.clone();
        child.actions = None;
        child.title = "Exactly one admitted child".into();
        Ok((
            source.id.clone(),
            TaskBatch {
                expected_generation: record
                    .admissions
                    .get(&source.id)
                    .ok_or("pending admission absent")?
                    .expected_generation,
                changes: vec![TaskChange::Add {
                    alias: TaskId::new("child")?,
                    definition: child,
                }],
            },
        ))
    }
}

#[test]
fn feedback_commit_fault_retains_one_repair_and_its_cumulative_spending() -> TestResult {
    for point in [
        FaultPoint::BeforeCommandCommit,
        FaultPoint::AfterCommandCommit,
    ] {
        let root = tempfile::tempdir()?;
        let (id, source, batch);
        {
            let fixture = Fixture::open(root.path(), "feedback-before")?;
            id = fixture.create_with_repair(true)?;
            fixture.engine.execute(
                fixture
                    .engine
                    .tick(1)?
                    .pop()
                    .ok_or("source dispatch absent")?,
            )?;
            source = fixture.child_batch(&id)?.0;
            fixture
                .engine
                .reject_model_batch(&id, &source, "invalid structured response", true)?;
            let rejected = fixture.engine.workflow(&id)?.ok_or("workflow absent")?;
            let mut definition = rejected
                .tasks
                .get(&source)
                .ok_or("source absent")?
                .definition
                .clone();
            definition.title = "Repair rejected actions".into();
            definition.required = false;
            batch = TaskBatch {
                expected_generation: rejected.generation,
                changes: vec![TaskChange::Add {
                    alias: TaskId::new("feedback")?,
                    definition,
                }],
            };
            fixture.faults.arm(point)?;
            assert!(
                fixture
                    .engine
                    .admit_model_batch(
                        context()?.actor().as_str(),
                        &id,
                        &source,
                        batch.clone(),
                        true,
                    )
                    .is_err()
            );
        }
        let fixture = Fixture::open(root.path(), "feedback-after")?;
        let interrupted = fixture.engine.workflow(&id)?.ok_or("workflow absent")?;
        let admission = interrupted
            .admissions
            .get(&source)
            .ok_or("admission absent")?;
        if point == FaultPoint::BeforeCommandCommit {
            assert_eq!(interrupted.tasks.len(), 1);
            assert_eq!(admission.status, ModelAdmissionStatus::Rejected);
            assert!(admission.repair_allowed);
            assert!(interrupted.wait_question.is_some());
        } else {
            assert_eq!(interrupted.tasks.len(), 2);
            assert_eq!(admission.status, ModelAdmissionStatus::Repaired);
        }
        let accepted = fixture.engine.admit_model_batch(
            context()?.actor().as_str(),
            &id,
            &source,
            batch.clone(),
            true,
        )?;
        assert_eq!(
            fixture.engine.admit_model_batch(
                context()?.actor().as_str(),
                &id,
                &source,
                batch,
                true,
            )?,
            accepted
        );
        let repaired = fixture.engine.workflow(&id)?.ok_or("workflow absent")?;
        assert_eq!(repaired.tasks.len(), 2);
        assert_eq!(repaired.admitted_tasks, 2);
        assert_eq!(repaired.admitted_model_batches, 1);
        assert_eq!(repaired.rejected_responses, 1);
        assert_eq!(
            repaired
                .admissions
                .get(&source)
                .ok_or("admission absent")?
                .status,
            ModelAdmissionStatus::Repaired
        );
        assert!(repaired.wait_question.is_none());
        assert_eq!(fixture.engine.tick(1)?.len(), 1);
        assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[test]
fn response_save_fault_preserves_pending_bytes_without_manufacturing_terminal_proof() -> TestResult
{
    for point in [
        FaultPoint::BeforeCommandCommit,
        FaultPoint::AfterCommandCommit,
    ] {
        let root = tempfile::tempdir()?;
        let id;
        {
            let fixture = Fixture::open(root.path(), "response-before")?;
            id = fixture.create()?;
            *fixture
                .executor
                .response_fault
                .lock()
                .map_err(|_| "fault lock poisoned")? = Some(point);
            fixture.engine.execute(
                fixture
                    .engine
                    .tick(1)?
                    .pop()
                    .ok_or("source dispatch absent")?,
            )?;
            assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 1);
        }
        let fixture = Fixture::open(root.path(), "response-after")?;
        assert!(fixture.engine.recover()?.is_empty());
        let record = fixture.engine.workflow(&id)?.ok_or("workflow absent")?;
        let source = record.tasks.values().next().ok_or("source absent")?;
        assert_eq!(source.state, TaskState::Uncertain);
        assert!(source.result.is_none());
        assert!(fixture.engine.tick(1)?.is_empty());
        if point == FaultPoint::BeforeCommandCommit {
            assert!(record.admissions.is_empty());
        } else {
            assert_eq!(
                record
                    .admissions
                    .get(&source.id)
                    .ok_or("admission absent")?
                    .status,
                ModelAdmissionStatus::Pending
            );
            assert!(
                record
                    .admissions
                    .get(&source.id)
                    .ok_or("admission absent")?
                    .response
                    .is_some()
            );
        }
        assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[test]
fn admission_commit_fault_replays_one_atomic_alias_map_after_reopen() -> TestResult {
    for point in [
        FaultPoint::BeforeCommandCommit,
        FaultPoint::AfterCommandCommit,
    ] {
        let root = tempfile::tempdir()?;
        let (id, source, batch);
        {
            let fixture = Fixture::open(root.path(), "admission-before")?;
            id = fixture.create()?;
            fixture.engine.execute(
                fixture
                    .engine
                    .tick(1)?
                    .pop()
                    .ok_or("source dispatch absent")?,
            )?;
            (source, batch) = fixture.child_batch(&id)?;
            fixture.faults.arm(point)?;
            assert!(
                fixture
                    .engine
                    .admit_model_batch(
                        context()?.actor().as_str(),
                        &id,
                        &source,
                        batch.clone(),
                        false
                    )
                    .is_err()
            );
        }
        let fixture = Fixture::open(root.path(), "admission-after")?;
        let interrupted = fixture.engine.workflow(&id)?.ok_or("workflow absent")?;
        assert_eq!(
            interrupted.tasks.len(),
            if point == FaultPoint::BeforeCommandCommit {
                1
            } else {
                2
            }
        );
        let accepted = fixture.engine.admit_model_batch(
            context()?.actor().as_str(),
            &id,
            &source,
            batch.clone(),
            false,
        )?;
        assert_eq!(
            fixture.engine.admit_model_batch(
                context()?.actor().as_str(),
                &id,
                &source,
                batch,
                false
            )?,
            accepted
        );
        let record = fixture.engine.workflow(&id)?.ok_or("workflow absent")?;
        assert_eq!(record.tasks.len(), 2);
        assert_eq!(record.admitted_model_batches, 1);
        assert_eq!(
            record
                .admissions
                .get(&source)
                .ok_or("admission absent")?
                .aliases
                .len(),
            1
        );
        assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[test]
fn child_entry_commit_fault_never_infers_safe_retry_from_a_lost_reply() -> TestResult {
    for point in [
        FaultPoint::BeforeCommandCommit,
        FaultPoint::AfterCommandCommit,
    ] {
        let root = tempfile::tempdir()?;
        let (id, child_id);
        {
            let fixture = Fixture::open(root.path(), "entry-before")?;
            id = fixture.create()?;
            fixture.engine.execute(
                fixture
                    .engine
                    .tick(1)?
                    .pop()
                    .ok_or("source dispatch absent")?,
            )?;
            let (source, batch) = fixture.child_batch(&id)?;
            fixture.engine.admit_model_batch(
                context()?.actor().as_str(),
                &id,
                &source,
                batch,
                false,
            )?;
            let child = fixture
                .engine
                .tick(1)?
                .pop()
                .ok_or("child dispatch absent")?;
            child_id = TaskId::new(child.node().as_str())?;
            fixture.faults.arm(point)?;
            assert!(fixture.engine.execute(child).is_err());
            assert_eq!(
                fixture.executor.entries.load(Ordering::SeqCst),
                1,
                "the child never crossed its external entry callback"
            );
        }
        let fixture = Fixture::open(root.path(), "entry-after")?;
        let recovered = fixture.engine.recover()?;
        let record = fixture.engine.workflow(&id)?.ok_or("workflow absent")?;
        assert_eq!(record.tasks.len(), 2);
        assert_eq!(record.admitted_model_batches, 1);
        if point == FaultPoint::BeforeCommandCommit {
            assert_eq!(recovered.len(), 1);
            fixture.engine.execute(
                recovered
                    .into_iter()
                    .next()
                    .ok_or("safe unentered child absent")?,
            )?;
            assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 1);
        } else {
            assert!(recovered.is_empty());
            assert_eq!(
                record.tasks.get(&child_id).ok_or("child absent")?.state,
                TaskState::Uncertain
            );
            assert!(fixture.engine.tick(1)?.is_empty());
            assert_eq!(fixture.executor.entries.load(Ordering::SeqCst), 0);
        }
    }
    Ok(())
}
