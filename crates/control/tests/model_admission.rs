//! Immutable model bytes are consumed by production control, including refusal and replay.
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    AuthorityOperation, AuthorityRequest, DecisionReasonCode, GrantDigest, GrantId, PolicyId,
};
use milkdrift_blueprint::{
    TaskActionPolicy, TaskBatch, TaskChange, TaskDefinition, TaskId, TaskInputBinding,
    TaskInputValue, TaskTool, WorkflowCreate, WorkflowId,
};
use milkdrift_capability::{
    BoundedJson, CancellationAcknowledgement, CancellationRequest, CapabilityDescriptorDocument,
    CapabilityRequirement, InvocationAdmissionEnvelope, InvocationEvent, InvocationEventKind,
    InvocationTerminal, OperationId, SideEffectClass, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterExecutionContext, InvocationDataAccess, MaterializationLimits, StoreInvocationDataAccess,
};
use milkdrift_control::{ActorAuthorityContext, TaskControlService};
use milkdrift_model::{FinishReason, ModelResponse, ModelResponseDocument, Usage};
use milkdrift_persistence::{
    ArtifactReadAuthority, EvidenceId, PersistenceError, StorageFailureClass,
};
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use milkdrift_runtime::{
    CommandAuthorityClaim, DeterministicExecutor, ExecutionDispatch, ExecutorError,
    ModelAdmissionStatus, PreparedExecution, ResolvedCapability, SequentialIdGenerator,
    SystemBoundaryClock, TaskExecutor, TaskRuntime,
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type AfterEntry = Box<dyn FnOnce() -> Result<(), ExecutorError> + Send>;

struct Scoped;
impl AuthorityEvaluator for Scoped {
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        let deny = request.operation == AuthorityOperation::InvokeCapability
            && !request.resources.filesystem.is_empty();
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.model-control")?,
            1,
            request.clone(),
            vec![if deny {
                DecisionReasonCode::Revoked
            } else {
                DecisionReasonCode::Allowed
            }],
            AuthorityBudget {
                invocations: Some(128),
                artifact_bytes: Some(16_777_216),
                concurrency: Some(64),
                ..AuthorityBudget::default()
            },
            SideEffectClass::Unknown,
        )
    }
}

#[derive(Default)]
struct Fault(Mutex<Option<(FaultPoint, bool)>>);
impl FaultInjector for Fault {
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
        let mut state = self.0.lock().map_err(|_| injected_failure(point))?;
        if let Some((armed, capacity)) = *state
            && armed == point
        {
            *state = None;
            return Err(if capacity {
                PersistenceError::Storage {
                    class: StorageFailureClass::ResourceExhausted,
                    message: "fixture receipt capacity exhausted".into(),
                }
            } else {
                injected_failure(point)
            });
        }
        Ok(())
    }
}

struct ResponseExecutor {
    resolver: DeterministicExecutor,
    data: Arc<StoreInvocationDataAccess>,
    response: Mutex<Option<ModelResponse>>,
    uncertain: AtomicBool,
    after_entry: Mutex<Option<AfterEntry>>,
}
impl TaskExecutor for ResponseExecutor {
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
        let response = self
            .response
            .lock()
            .map_err(|_| ExecutorError::BoundaryBeforeEntry("fixture lock unavailable".into()))?
            .clone()
            .ok_or_else(|| ExecutorError::BoundaryBeforeEntry("fixture response absent".into()))?;
        Ok(PreparedExecution::new(
            dispatch,
            InvocationAdmissionEnvelope::not_applicable(),
            move |dispatch, reporter| {
                let after_entry = self
                    .after_entry
                    .lock()
                    .map_err(|_| {
                        ExecutorError::BoundaryAfterEntry("fixture callback unavailable".into())
                    })?
                    .take();
                if let Some(after_entry) = after_entry {
                    after_entry()?;
                }
                let fail = |error: Box<dyn std::error::Error>| {
                    ExecutorError::BoundaryAfterEntry(error.to_string())
                };
                let context = AdapterExecutionContext::from_dispatch(dispatch)
                    .map_err(|error| fail(Box::new(error)))?;
                let bytes = ModelResponseDocument::new(response)
                    .to_canonical_json()
                    .map_err(|error| fail(Box::new(error)))?;
                let reference = self
                    .data
                    .publish_bytes(
                        &context,
                        dispatch.request(),
                        "model_response",
                        "application/vnd.milkdrift.model-response.v1+json",
                        &bytes,
                        MaterializationLimits {
                            max_files: 1,
                            max_file_bytes: 2_097_152,
                            max_total_bytes: 2_097_152,
                            max_path_bytes: 4096,
                            max_directory_depth: 8,
                            chunk_bytes: 262_144,
                        },
                    )
                    .map_err(|error| fail(Box::new(error)))?;
                reporter.invocation(InvocationEvent::new(
                    dispatch.request().invocation().clone(),
                    1,
                    InvocationEventKind::Output {
                        name: "model_response".into(),
                        reference: reference.clone(),
                    },
                )?)?;
                if self.uncertain.load(Ordering::SeqCst) {
                    return Err(ExecutorError::BoundaryAfterEntry(
                        "fixture lost terminal report".into(),
                    ));
                }
                reporter.invocation(InvocationEvent::new(
                    dispatch.request().invocation().clone(),
                    2,
                    InvocationEventKind::Terminal {
                        terminal: InvocationTerminal::new(
                            TerminalStatus::Success,
                            vec![reference],
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
    store: Arc<RedbStore>,
    runtime: Arc<TaskRuntime>,
    executor: Arc<ResponseExecutor>,
    control: Arc<TaskControlService>,
    faults: Arc<Fault>,
}
impl Fixture {
    fn open(path: &Path) -> TestResult<Self> {
        let faults = Arc::new(Fault::default());
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(path).with_fault_injector(faults.clone()),
        )?);
        let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
            "../../capability/tests/fixtures/descriptor-v1.json"
        ))?;
        let executor = Arc::new(ResponseExecutor {
            resolver: DeterministicExecutor::new(descriptor.body().clone()),
            data: Arc::new(StoreInvocationDataAccess::new(
                store.clone(),
                path.join("materialized"),
                ArtifactReadAuthority::Authorized {
                    actor: ActorRef::new("test:reader")?,
                    evidence: EvidenceId::new("test-model-read")?,
                },
            )?),
            response: Mutex::new(None),
            uncertain: AtomicBool::new(false),
            after_entry: Mutex::new(None),
        });
        let authority = Arc::new(Scoped);
        let clock = Arc::new(SystemBoundaryClock);
        let runtime = Arc::new(
            TaskRuntime::new(
                store.clone(),
                executor.clone(),
                authority.clone(),
                clock.clone(),
                Arc::new(SequentialIdGenerator::new("model-control", 1)?),
            )
            .with_maximum_active_tasks(64)?,
        );
        let control = Arc::new(TaskControlService::new(runtime.clone(), authority, clock));
        Ok(Self {
            store,
            runtime,
            executor,
            control,
            faults,
        })
    }
    fn start(
        &self,
        repair: bool,
    ) -> TestResult<(WorkflowId, TaskId, ExecutionDispatch, TaskDefinition)> {
        self.start_with_inputs(repair, BTreeMap::new())
    }
    fn start_with_inputs(
        &self,
        repair: bool,
        inputs: BTreeMap<String, TaskInputBinding>,
    ) -> TestResult<(WorkflowId, TaskId, ExecutionDispatch, TaskDefinition)> {
        let definition: TaskDefinition = serde_json::from_value(
            serde_json::json!({"title":"Plan work","instructions":"Return the next structured batch","inputs":inputs,"capability":CapabilityRequirement::new(OperationId::new("model.generate")?),"actions":TaskActionPolicy { automatic_repair:repair }}),
        )?;
        let create: WorkflowCreate = serde_json::from_value(
            serde_json::json!({"title":"Automatic admission","tasks":[{"alias":"planner","definition":definition}]}),
        )?;
        let receipt = self.control.create(&actor()?, "create-model", create)?;
        let dispatch = self.runtime.tick(1)?.pop().ok_or("source did not start")?;
        let source = TaskId::new(dispatch.node().as_str())?;
        Ok((receipt.workflow, source, dispatch, definition))
    }
    fn respond(
        &self,
        dispatch: ExecutionDispatch,
        text: &str,
        structured: Option<serde_json::Value>,
    ) -> TestResult {
        *self
            .executor
            .response
            .lock()
            .map_err(|_| "fixture lock poisoned")? = Some(ModelResponse::new(
            text.to_owned(),
            structured.map(BoundedJson::new).transpose()?,
            Vec::new(),
            FinishReason::Stop,
            Usage {
                input_units: None,
                output_units: None,
                cached_input_units: None,
                cost_micros: None,
                currency: None,
            },
            BTreeMap::new(),
        )?);
        self.runtime.execute(dispatch)?;
        Ok(())
    }
    fn process(&self) -> Result<(), milkdrift_control::ControlError> {
        self.control
            .process_model_admissions(self.store.as_ref(), self.store.as_ref())
    }
    fn generation(&self, workflow: &WorkflowId, source: &TaskId) -> TestResult<u64> {
        Ok(self
            .runtime
            .workflow(workflow)?
            .ok_or("workflow absent")?
            .tasks
            .get(source)
            .and_then(|task| task.frozen.as_ref())
            .ok_or("frozen source absent")?
            .accepted_generation)
    }
}
fn actor() -> TestResult<ActorAuthorityContext> {
    Ok(ActorAuthorityContext::new(
        ActorRef::new("human:model-author")?,
        CommandAuthorityClaim::new(
            GrantId::new("grant:model-author")?,
            1,
            GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
            0,
        )?,
    ))
}

#[test]
fn production_control_reads_saved_structured_response_after_restart_exactly_once() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path())?;
    let (workflow, source, dispatch, mut definition) = fixture.start(false)?;
    definition.actions = None;
    let batch = TaskBatch {
        expected_generation: fixture.generation(&workflow, &source)?,
        changes: vec![TaskChange::Add {
            alias: TaskId::new("research")?,
            definition,
        }],
    };
    fixture.respond(
        dispatch,
        "proposed research",
        Some(serde_json::to_value(batch)?),
    )?;
    let before = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    let response = before
        .admissions
        .get(&source)
        .ok_or("pending absent")?
        .response
        .clone();
    assert_eq!(
        before
            .admissions
            .get(&source)
            .ok_or("pending absent")?
            .status,
        ModelAdmissionStatus::Pending
    );
    drop(fixture);
    let fixture = Fixture::open(root.path())?;
    fixture.runtime.recover()?;
    fixture.process()?;
    let accepted = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    let admission = accepted.admissions.get(&source).ok_or("admission absent")?;
    assert_eq!(accepted.tasks.len(), 2);
    assert_eq!(accepted.admitted_model_batches, 1);
    assert_eq!(admission.status, ModelAdmissionStatus::Accepted);
    assert_eq!(admission.response, response);
    let aliases = admission.aliases.clone();
    fixture.process()?;
    let replay = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(replay.tasks.len(), 2);
    assert_eq!(replay.admitted_model_batches, 1);
    assert_eq!(
        replay
            .admissions
            .get(&source)
            .ok_or("admission absent")?
            .aliases,
        aliases
    );
    Ok(())
}

#[test]
fn malicious_text_and_out_of_grant_tool_batches_never_create_partial_work() -> TestResult {
    for structured in [false, true] {
        let root = tempfile::tempdir()?;
        let fixture = Fixture::open(root.path())?;
        let (workflow, source, dispatch, mut definition) = fixture.start(false)?;
        definition.actions = None;
        definition.tools = vec![TaskTool::WriteFile];
        definition.paths = vec![milkdrift_blueprint::TaskPathAccess {
            path: root.path().to_string_lossy().into_owned(),
            write: true,
        }];
        let batch = TaskBatch {
            expected_generation: fixture.generation(&workflow, &source)?,
            changes: vec![
                TaskChange::BindInput {
                    name: "must-not-be-accepted".into(),
                    value: milkdrift_blueprint::TaskInputValue::Text {
                        text: "partial".into(),
                    },
                },
                TaskChange::Add {
                    alias: TaskId::new("forbidden")?,
                    definition,
                },
            ],
        };
        let value = serde_json::to_value(batch)?;
        fixture.respond(
            dispatch,
            &value.to_string(),
            structured.then_some(value.clone()),
        )?;
        fixture.process()?;
        let result = fixture
            .runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?;
        assert_eq!(result.tasks.len(), 1);
        assert!(!result.inputs.contains_key("must-not-be-accepted"));
        assert_eq!(result.admitted_model_batches, 0);
        assert_eq!(
            result
                .admissions
                .get(&source)
                .ok_or("admission absent")?
                .status,
            ModelAdmissionStatus::Rejected
        );
        assert!(
            !result
                .admissions
                .get(&source)
                .ok_or("admission absent")?
                .repair_allowed
        );
        fixture.process()?;
        assert_eq!(
            fixture
                .runtime
                .workflow(&workflow)?
                .ok_or("workflow absent")?
                .rejected_responses,
            1
        );
    }
    Ok(())
}

#[test]
fn feedback_storage_faults_retry_the_same_response_without_disabling_or_duplicating_repair()
-> TestResult {
    for point in [
        FaultPoint::BeforeCommandCommit,
        FaultPoint::AfterCommandCommit,
    ] {
        let root = tempfile::tempdir()?;
        let fixture = Fixture::open(root.path())?;
        let (workflow, source, dispatch, _) = fixture.start(true)?;
        fixture.respond(
            dispatch,
            "invalid action shape",
            Some(serde_json::json!({"not":"a task batch"})),
        )?;
        fixture.process()?;
        *fixture.faults.0.lock().map_err(|_| "fault lock poisoned")? = Some((point, false));
        assert!(fixture.process().is_err());
        let interrupted = fixture
            .runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?;
        let admission = interrupted
            .admissions
            .get(&source)
            .ok_or("admission absent")?;
        if point == FaultPoint::BeforeCommandCommit {
            assert_eq!(admission.status, ModelAdmissionStatus::Rejected);
            assert!(admission.repair_allowed);
            assert_eq!(interrupted.tasks.len(), 1);
        } else {
            assert_eq!(admission.status, ModelAdmissionStatus::Repaired);
            assert_eq!(interrupted.tasks.len(), 2);
        }
        drop(fixture);
        let fixture = Fixture::open(root.path())?;
        fixture.process()?;
        fixture.process()?;
        let recovered = fixture
            .runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?;
        assert_eq!(recovered.tasks.len(), 2);
        assert_eq!(recovered.admitted_model_batches, 1);
        assert_eq!(
            recovered
                .admissions
                .get(&source)
                .ok_or("admission absent")?
                .status,
            ModelAdmissionStatus::Repaired
        );
    }
    Ok(())
}

#[test]
fn storage_capacity_refuses_impossible_actions_but_preserves_committed_admissions() -> TestResult {
    for repair in [false, true] {
        for point in [
            FaultPoint::BeforeCommandCommit,
            FaultPoint::AfterCommandCommit,
        ] {
            let root = tempfile::tempdir()?;
            let fixture = Fixture::open(root.path())?;
            let (workflow, source, dispatch, _) = fixture.start(true)?;
            let structured = if repair {
                serde_json::json!({"not":"a task batch"})
            } else {
                serde_json::to_value(TaskBatch {
                    expected_generation: fixture.generation(&workflow, &source)?,
                    changes: vec![TaskChange::Wait {
                        question: "Review the result".into(),
                    }],
                })?
            };
            fixture.respond(dispatch, "result", Some(structured))?;
            if repair {
                fixture.process()?;
            }
            *fixture.faults.0.lock().map_err(|_| "fault lock poisoned")? = Some((point, true));
            fixture.process()?;
            let record = fixture
                .runtime
                .workflow(&workflow)?
                .ok_or("workflow absent")?;
            let admission = record.admissions.get(&source).ok_or("admission absent")?;
            if point == FaultPoint::BeforeCommandCommit {
                assert_eq!(admission.status, ModelAdmissionStatus::Rejected);
                assert!(!admission.repair_allowed);
                assert_eq!(record.admitted_model_batches, 0);
                assert_eq!(record.tasks.len(), 1);
                assert!(
                    record
                        .wait_question
                        .as_deref()
                        .is_some_and(|reason| reason.contains("capacity"))
                );
            } else {
                assert_eq!(
                    admission.status,
                    if repair {
                        ModelAdmissionStatus::Repaired
                    } else {
                        ModelAdmissionStatus::Accepted
                    }
                );
                assert_eq!(record.admitted_model_batches, 1);
                assert_eq!(record.tasks.len(), if repair { 2 } else { 1 });
            }
            fixture.process()?;
            assert_eq!(
                fixture
                    .runtime
                    .workflow(&workflow)?
                    .ok_or("workflow absent")?,
                record
            );
        }
    }
    Ok(())
}

#[test]
fn uncertain_responses_do_not_starve_later_settled_admissions() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path())?;
    let definition: TaskDefinition = serde_json::from_value(serde_json::json!({
        "title":"Return actions", "instructions":"Return a structured batch",
        "capability":CapabilityRequirement::new(OperationId::new("model.generate")?),
        "actions":{"automatic_repair":false}
    }))?;
    let tasks = (0..32)
        .map(|index| serde_json::json!({"alias":format!("source_{index}"),"definition":definition}))
        .collect::<Vec<_>>();
    let mut create: WorkflowCreate =
        serde_json::from_value(serde_json::json!({"title":"Admission fairness","tasks":tasks}))?;
    create.limits.concurrent_tasks = 64;
    let created = fixture.control.create(&actor()?, "fairness", create)?;
    let before = fixture
        .runtime
        .workflow(&created.workflow)?
        .ok_or("workflow absent")?;
    fixture.control.edit(
        &actor()?,
        "add-last",
        &created.workflow,
        TaskBatch {
            expected_generation: before.generation,
            changes: vec![TaskChange::Add {
                alias: TaskId::new("last")?,
                definition,
            }],
        },
    )?;
    let mut dispatches = fixture.runtime.tick(32)?;
    dispatches.extend(fixture.runtime.tick(1)?);
    assert_eq!(dispatches.len(), 33);
    dispatches.sort_by(|left, right| left.node().cmp(right.node()));
    let settled = dispatches.pop().ok_or("last source absent")?;
    let settled_id = TaskId::new(settled.node().as_str())?;
    fixture.executor.uncertain.store(true, Ordering::SeqCst);
    for dispatch in dispatches {
        fixture.respond(dispatch, "terminal evidence unavailable", None)?;
    }
    fixture.executor.uncertain.store(false, Ordering::SeqCst);
    fixture.respond(settled, "completed without structured actions", None)?;
    let before = fixture
        .runtime
        .workflow(&created.workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(before.admissions.len(), 33);
    assert_eq!(
        before
            .tasks
            .values()
            .filter(|task| task.state == milkdrift_runtime::TaskState::Uncertain)
            .count(),
        32
    );
    fixture.process()?;
    let after = fixture
        .runtime
        .workflow(&created.workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(
        after
            .admissions
            .get(&settled_id)
            .ok_or("settled admission absent")?
            .status,
        ModelAdmissionStatus::Rejected
    );
    assert_eq!(
        after
            .admissions
            .values()
            .filter(|admission| admission.status == ModelAdmissionStatus::Pending)
            .count(),
        32
    );
    assert_eq!(after.rejected_responses, 1);
    Ok(())
}

#[test]
fn explicit_wait_survives_rejection_and_restart_until_authorized_continuation() -> TestResult {
    for wait_during_entry in [true, false] {
        let root = tempfile::tempdir()?;
        let fixture = Fixture::open(root.path())?;
        let (workflow, source, dispatch, _) = fixture.start(true)?;
        let pause = {
            let runtime = fixture.runtime.clone();
            let control = fixture.control.clone();
            let workflow = workflow.clone();
            move || -> Result<(), ExecutorError> {
                let pause = || -> TestResult {
                    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
                    control.edit(
                        &actor()?,
                        "explicit-wait",
                        &workflow,
                        TaskBatch {
                            expected_generation: record.generation,
                            changes: vec![TaskChange::Wait {
                                question: "Review before any further work".into(),
                            }],
                        },
                    )?;
                    Ok(())
                };
                pause().map_err(|error| ExecutorError::BoundaryAfterEntry(error.to_string()))
            }
        };
        if wait_during_entry {
            *fixture
                .executor
                .after_entry
                .lock()
                .map_err(|_| "fixture lock poisoned")? = Some(Box::new(pause));
        } else {
            fixture.respond(
                dispatch.clone(),
                "invalid actions",
                Some(serde_json::json!({"invalid":true})),
            )?;
            fixture.process()?;
            pause()?;
            drop(pause);
        }
        if wait_during_entry {
            fixture.respond(
                dispatch,
                "invalid actions",
                Some(serde_json::json!({"invalid":true})),
            )?;
        }
        fixture.process()?;
        let paused = fixture
            .runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?;
        assert_eq!(
            paused.wait_question.as_deref(),
            Some("Review before any further work")
        );
        assert_eq!(paused.model_admission_wait, None);
        assert_eq!(paused.tasks.len(), 1);
        assert_eq!(paused.rejected_responses, 1);
        assert!(
            paused
                .admissions
                .get(&source)
                .ok_or("admission absent")?
                .repair_allowed
        );
        drop(fixture);
        let fixture = Fixture::open(root.path())?;
        fixture.process()?;
        assert!(fixture.runtime.tick(1)?.is_empty());
        assert_eq!(
            fixture
                .runtime
                .workflow(&workflow)?
                .ok_or("workflow absent")?,
            paused
        );
        fixture.control.edit(
            &actor()?,
            "explicit-continue",
            &workflow,
            TaskBatch {
                expected_generation: paused.generation,
                changes: vec![TaskChange::Continue],
            },
        )?;
        fixture.process()?;
        let resumed = fixture
            .runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?;
        assert_eq!(resumed.tasks.len(), 2);
        assert_eq!(resumed.rejected_responses, 1);
        assert_eq!(resumed.admitted_model_batches, 1);
        assert_eq!(resumed.model_admission_wait, None);
        assert_eq!(resumed.wait_question, None);
        assert_eq!(
            resumed
                .admissions
                .get(&source)
                .ok_or("admission absent")?
                .status,
            ModelAdmissionStatus::Repaired
        );
        assert_eq!(fixture.runtime.tick(1)?.len(), 1);
    }
    Ok(())
}

#[test]
fn dismissing_rejected_actions_does_not_clear_a_distinct_explicit_wait() -> TestResult {
    let root = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path())?;
    let (workflow, source, dispatch, _) = fixture.start(false)?;
    fixture.respond(dispatch, "missing actions", None)?;
    fixture.process()?;
    let rejected = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(rejected.model_admission_wait, Some(source.clone()));
    fixture.control.edit(
        &actor()?,
        "wait-and-dismiss",
        &workflow,
        TaskBatch {
            expected_generation: rejected.generation,
            changes: vec![
                TaskChange::Wait {
                    question: "Inspect outputs before continuing".into(),
                },
                TaskChange::DismissAdmission {
                    task: source.clone(),
                },
            ],
        },
    )?;
    let dismissed = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(
        dismissed.wait_question.as_deref(),
        Some("Inspect outputs before continuing")
    );
    assert_eq!(dismissed.model_admission_wait, None);
    assert_eq!(
        dismissed
            .admissions
            .get(&source)
            .ok_or("admission absent")?
            .status,
        ModelAdmissionStatus::Dismissed
    );
    Ok(())
}

#[test]
fn feedback_preserves_colliding_frozen_inputs_and_refuses_full_input_capacity() -> TestResult {
    for full in [false, true] {
        let root = tempfile::tempdir()?;
        let fixture = Fixture::open(root.path())?;
        let count = if full { 64 } else { 2 };
        let inputs = (0..count)
            .map(|index| {
                let name = if index == 0 {
                    "rejected_response".to_owned()
                } else {
                    format!("rejected_response_{index}")
                };
                (
                    name,
                    TaskInputBinding::Inline {
                        value: TaskInputValue::Text {
                            text: format!("original frozen value {index}"),
                        },
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let (workflow, source, dispatch, _) = fixture.start_with_inputs(true, inputs.clone())?;
        fixture.respond(dispatch, "invalid actions", None)?;
        fixture.process()?;
        fixture.process()?;
        let record = fixture
            .runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?;
        let admission = record.admissions.get(&source).ok_or("admission absent")?;
        if full {
            assert_eq!(record.tasks.len(), 1);
            assert_eq!(admission.status, ModelAdmissionStatus::Rejected);
            assert!(!admission.repair_allowed);
            assert!(
                admission
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("64 frozen inputs"))
            );
            assert_eq!(
                record
                    .tasks
                    .get(&source)
                    .ok_or("source absent")?
                    .definition
                    .inputs,
                inputs
            );
        } else {
            assert_eq!(record.tasks.len(), 2);
            assert_eq!(admission.status, ModelAdmissionStatus::Repaired);
            let feedback = record
                .tasks
                .values()
                .find(|task| task.id != source)
                .ok_or("feedback absent")?;
            assert_eq!(feedback.definition.inputs.len(), 3);
            for (name, original) in inputs {
                assert_eq!(feedback.definition.inputs.get(&name), Some(&original));
            }
            assert_eq!(
                feedback.definition.inputs.get("rejected_response_2"),
                Some(&TaskInputBinding::Inline {
                    value: TaskInputValue::Artifact {
                        reference: admission.response.clone().ok_or("response absent")?
                    }
                })
            );
            assert!(
                feedback
                    .definition
                    .instructions
                    .contains("rejected response is supplied as input rejected_response_2")
            );
        }
    }
    Ok(())
}
