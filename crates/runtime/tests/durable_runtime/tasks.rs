//! Ordinary-task admission and entry use the production redb owner and executor boundary.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    DecisionReasonCode, PolicyId,
};
use milkdrift_blueprint::{
    NewTask, TaskBatch, TaskChange, TaskDefinition, TaskId, TaskInputBinding, TaskInputValue,
    WorkflowCreate, WorkflowLimits,
};
use milkdrift_capability::{
    CapabilityDescriptorDocument, CapabilityRequirement, OperationId, SideEffectClass,
};
use milkdrift_control::{ActorAuthorityContext, TaskControlService};
use milkdrift_redb_store::{RedbStore, RedbStoreConfig, StoreClock};
use milkdrift_runtime::{
    BoundaryClock, DeterministicExecutor, ManualClock, SequentialIdGenerator, TaskExecutor,
    TaskRuntime, TaskState, WorkflowState,
};

use super::{TestResult, claim};

fn request() -> TestResult<WorkflowCreate> {
    Ok(WorkflowCreate {
        title: "Inspect the supplied Cargo project".into(),
        description: "Read the captured manifest and report its targets".into(),
        tasks: vec![NewTask {
            alias: TaskId::new("inspect")?,
            definition: TaskDefinition {
                title: "Read Cargo manifest".into(),
                description: String::new(),
                instructions:
                    "Summarize the package name and dependencies in the supplied manifest.".into(),
                capability: CapabilityRequirement::new(OperationId::new("model.generate")?),
                inputs: BTreeMap::from([(
                    "manifest".into(),
                    TaskInputBinding::Supplied {
                        name: "manifest".into(),
                        optional: false,
                    },
                )]),
                dependencies: BTreeSet::new(),
                required: true,
                corrective_of: None,
                paths: Vec::new(),
            },
        }],
        inputs: BTreeMap::new(),
        limits: WorkflowLimits::default(),
        finish_when_settled: true,
    })
}

fn context() -> TestResult<ActorAuthorityContext> {
    Ok(ActorAuthorityContext::new(
        milkdrift_authority::ActorRef::new("human:task-test")?,
        claim()?,
    ))
}

fn runtime(
    root: &std::path::Path,
    authority: Arc<dyn AuthorityEvaluator>,
    clock: Arc<ManualClock>,
) -> TestResult<(
    Arc<TaskRuntime>,
    Arc<DeterministicExecutor>,
    TaskControlService,
)> {
    let mut fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    fixture["descriptor"]["category"] = serde_json::json!({"type":"process"});
    let descriptor = CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&fixture)?)?;
    let executor = Arc::new(DeterministicExecutor::new(descriptor.body().clone()));
    let store = Arc::new(RedbStore::open_with_config(
        RedbStoreConfig::new(root).with_clock(Arc::new(TaskStoreClock(clock.clone()))),
    )?);
    let runtime = Arc::new(TaskRuntime::new(
        store,
        executor.clone(),
        authority.clone(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("task-tests", 1)?),
    ));
    let control = TaskControlService::new(runtime.clone(), authority, clock);
    Ok((runtime, executor, control))
}

#[test]
fn ordinary_submission_replays_aliases_and_refuses_conflicting_request() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(RevocableAuthority(AtomicBool::new(false))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let request = request()?;
    let first = control.create(&context()?, "submit", request.clone())?;
    let replay = control.create(&context()?, "submit", request.clone())?;
    assert_eq!(first, replay);
    let mut conflict = request;
    conflict.title.push_str(" changed");
    assert!(control.create(&context()?, "submit", conflict).is_err());
    assert!(runtime.tick(4)?.is_empty());
    assert_eq!(executor.entry_count(), 0);
    let workflow = runtime
        .workflow(&first.workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(workflow.tasks.len(), 1);
    assert!(workflow.tasks.values().all(|task| task.started_at.is_none()
        && task.reason.as_deref() == Some("missing supplied input manifest")));
    Ok(())
}

#[test]
fn captured_file_bytes_and_definition_freeze_at_start_and_edit_race_refuses() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(RevocableAuthority(AtomicBool::new(false))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let mut create = request()?;
    let captured = TaskInputValue::text_file(
        "Cargo.toml".into(),
        "[package]\nname = 'captured-project'".into(),
    )?;
    create.inputs.insert("manifest".into(), captured.clone());
    let receipt = control.create(&context()?, "submit", create)?;
    let before = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let task = before.tasks.values().next().ok_or("task absent")?;
    let task_id = task.id.clone();
    let original = task.definition.clone();
    let mut dispatches = runtime.tick(4)?;
    assert_eq!(dispatches.len(), 1);
    assert!(
        control
            .edit(
                &context()?,
                "stale-edit",
                &receipt.workflow,
                TaskBatch {
                    expected_generation: before.generation,
                    changes: vec![TaskChange::Revise {
                        task: task_id.clone(),
                        expected_version: 1,
                        definition: original.clone()
                    }]
                }
            )
            .is_err()
    );
    let started = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert!(
        control
            .edit(
                &context()?,
                "started-edit",
                &receipt.workflow,
                TaskBatch {
                    expected_generation: started.generation,
                    changes: vec![TaskChange::Revise {
                        task: task_id.clone(),
                        expected_version: 1,
                        definition: original
                    }]
                }
            )
            .is_err()
    );
    control.edit(
        &context()?,
        "future-input",
        &receipt.workflow,
        TaskBatch {
            expected_generation: started.generation,
            changes: vec![TaskChange::BindInput {
                name: "manifest".into(),
                value: TaskInputValue::Text {
                    text: "later value".into(),
                },
            }],
        },
    )?;
    let changed = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(
        changed
            .tasks
            .get(&task_id)
            .and_then(|task| task.frozen.as_ref())
            .and_then(|frozen| frozen.inputs.get("manifest")),
        Some(&captured)
    );
    runtime.execute(dispatches.pop().ok_or("dispatch absent")?)?;
    let complete = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(executor.entry_count(), 1);
    assert_eq!(complete.state, WorkflowState::Completed);
    assert_eq!(
        complete.tasks.get(&task_id).map(|task| task.state),
        Some(TaskState::Completed)
    );
    assert!(runtime.recover()?.is_empty());
    Ok(())
}

struct RevocableAuthority(AtomicBool);

struct TaskStoreClock(Arc<ManualClock>);
impl StoreClock for TaskStoreClock {
    fn now(
        &self,
    ) -> Result<milkdrift_persistence::TimestampMillis, milkdrift_persistence::PersistenceError>
    {
        self.0
            .now()
            .map_err(|error| milkdrift_persistence::PersistenceError::Storage {
                class: milkdrift_persistence::StorageFailureClass::Unavailable,
                message: error.to_string(),
            })
    }
}
impl AuthorityEvaluator for RevocableAuthority {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.task-revocation")?,
            1,
            request.clone(),
            vec![if self.0.load(Ordering::SeqCst) {
                DecisionReasonCode::Revoked
            } else {
                DecisionReasonCode::Allowed
            }],
            AuthorityBudget {
                artifact_bytes: Some(u64::MAX),
                invocations: Some(256),
                concurrency: Some(4),
                ..AuthorityBudget::default()
            },
            SideEffectClass::Unknown,
        )
    }
}

#[test]
fn revoked_grant_before_entry_prevents_external_effect_and_retains_failed_result() -> TestResult {
    let root = tempfile::tempdir()?;
    let authority = Arc::new(RevocableAuthority(AtomicBool::new(false)));
    let (runtime, executor, control) = runtime(
        root.path(),
        authority.clone(),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let mut create = request()?;
    create.inputs.insert(
        "manifest".into(),
        TaskInputValue::Text {
            text: "[package]".into(),
        },
    );
    let receipt = control.create(&context()?, "submit", create)?;
    let dispatch = runtime.tick(4)?.pop().ok_or("dispatch absent")?;
    authority.0.store(true, Ordering::SeqCst);
    runtime.execute(dispatch)?;
    assert_eq!(executor.entry_count(), 0);
    let workflow = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert!(
        workflow
            .tasks
            .values()
            .all(|task| task.state == TaskState::Failed
                && task.reason.as_deref() == Some("authority was revoked before external entry"))
    );
    Ok(())
}

struct InvocationAllowance(Option<u64>);
impl AuthorityEvaluator for InvocationAllowance {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.task-allowance")?,
            1,
            request.clone(),
            vec![DecisionReasonCode::Allowed],
            AuthorityBudget {
                invocations: self.0,
                concurrency: Some(4),
                artifact_bytes: Some(16 * 1024 * 1024),
                ..AuthorityBudget::default()
            },
            SideEffectClass::Unknown,
        )
    }
}

#[test]
fn absent_external_allowance_refuses_prepared_entry_without_external_call() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(None)),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let mut create = request()?;
    create.inputs.insert(
        "manifest".into(),
        TaskInputValue::Text {
            text: "first".into(),
        },
    );
    let receipt = control.create(&context()?, "submit", create)?;
    runtime.execute(runtime.tick(4)?.pop().ok_or("dispatch absent")?)?;
    assert_eq!(executor.entry_count(), 0);
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert!(
        record
            .tasks
            .values()
            .all(|task| task.state == TaskState::Failed
                && task.reason.as_ref().is_some_and(|reason| reason
                    .contains("external invocations cumulative allowance exhausted or absent")))
    );
    assert_eq!(record.committed_allowance.invocations, None);
    Ok(())
}

#[test]
fn cumulative_invocation_allowance_survives_settlement_and_store_reopen() -> TestResult {
    let root = tempfile::tempdir()?;
    let clock = Arc::new(ManualClock::new(1_000));
    let authority = Arc::new(InvocationAllowance(Some(1)));
    let workflow;
    {
        let (runtime, executor, control) = runtime(root.path(), authority.clone(), clock.clone())?;
        let mut create = request()?;
        let mut second = create.tasks.first().ok_or("task absent")?.clone();
        second.alias = TaskId::new("second")?;
        create.tasks.push(second);
        create.limits.concurrent_tasks = 1;
        create.inputs.insert(
            "manifest".into(),
            TaskInputValue::Text {
                text: "first".into(),
            },
        );
        workflow = control.create(&context()?, "submit", create)?.workflow;
        runtime.execute(runtime.tick(1)?.pop().ok_or("dispatch absent")?)?;
        assert_eq!(executor.entry_count(), 1);
        assert_eq!(
            runtime
                .workflow(&workflow)?
                .ok_or("workflow absent")?
                .committed_allowance
                .invocations,
            Some(1)
        );
    }
    {
        let (runtime, executor, _control) = runtime(root.path(), authority, clock)?;
        assert!(runtime.recover()?.is_empty());
        runtime.execute(runtime.tick(1)?.pop().ok_or("second dispatch absent")?)?;
        assert_eq!(executor.entry_count(), 0);
        let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
        assert_eq!(record.committed_allowance.invocations, Some(1));
        assert_eq!(
            record
                .tasks
                .values()
                .filter(|task| task.state == TaskState::Completed)
                .count(),
            1
        );
        assert_eq!(
            record
                .tasks
                .values()
                .filter(|task| task.state == TaskState::Failed)
                .count(),
            1
        );
    }
    Ok(())
}

#[test]
fn future_definition_edits_and_removal_keep_accepted_history_and_cumulative_count() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, _executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let receipt = control.create(&context()?, "submit", request()?)?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let task = record.tasks.values().next().ok_or("task absent")?;
    let id = task.id.clone();
    let original = task.definition.clone();
    let mut changed = original.clone();
    changed.title = "Future corrected title".into();
    control.edit(
        &context()?,
        "revise",
        &record.id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Revise {
                task: id.clone(),
                expected_version: 1,
                definition: changed.clone(),
            }],
        },
    )?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    control.edit(
        &context()?,
        "remove",
        &record.id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Remove {
                task: id.clone(),
                expected_version: 2,
            }],
        },
    )?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let retained = record.tasks.get(&id).ok_or("removed task history absent")?;
    assert!(retained.removed);
    assert_eq!(retained.definition, changed);
    assert_eq!(retained.previous_definitions, vec![original]);
    assert_eq!(record.admitted_tasks, 1);
    Ok(())
}

#[test]
fn optional_output_waits_for_producer_settlement_then_supplies_typed_absence() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let mut create = request()?;
    let mut consumer = create.tasks.first().ok_or("producer absent")?.clone();
    let producer_alias = consumer.alias.clone();
    consumer.alias = TaskId::new("consume-optional")?;
    consumer
        .definition
        .dependencies
        .insert(producer_alias.clone());
    consumer.definition.inputs = BTreeMap::from([(
        "optional_result".into(),
        TaskInputBinding::TaskOutput {
            task: producer_alias,
            output: "report".into(),
            optional: true,
        },
    )]);
    create
        .tasks
        .first_mut()
        .ok_or("producer absent")?
        .definition
        .required = false;
    create.tasks.push(consumer);
    let receipt = control.create(&context()?, "submit", create)?;
    assert!(runtime.tick(4)?.is_empty());
    assert_eq!(executor.entry_count(), 0);
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let producer = record
        .tasks
        .values()
        .find(|task| !task.definition.required)
        .ok_or("producer absent")?
        .id
        .clone();
    control.edit(
        &context()?,
        "stop-producer",
        &record.id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Stop { task: producer }],
        },
    )?;
    let dispatch = runtime
        .tick(4)?
        .pop()
        .ok_or("optional consumer never became ready")?;
    runtime.execute(dispatch)?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let consumer = record
        .tasks
        .values()
        .find(|task| task.definition.required)
        .ok_or("consumer absent")?;
    assert_eq!(
        consumer
            .frozen
            .as_ref()
            .and_then(|frozen| frozen.inputs.get("optional_result")),
        Some(&TaskInputValue::Absent)
    );
    assert_eq!(record.state, WorkflowState::Completed);
    Ok(())
}

#[test]
fn successful_correction_requires_explicit_adoption_and_preserves_original_stop() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, _executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let receipt = control.create(&context()?, "submit", request()?)?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let original = record.tasks.values().next().ok_or("task absent")?;
    let original_id = original.id.clone();
    let mut correction = original.definition.clone();
    correction.inputs.clear();
    correction.corrective_of = Some(original_id.clone());
    control.edit(
        &context()?,
        "correct",
        &record.id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![
                TaskChange::Stop {
                    task: original_id.clone(),
                },
                TaskChange::Add {
                    alias: TaskId::new("correction")?,
                    definition: correction,
                },
            ],
        },
    )?;
    runtime.execute(runtime.tick(4)?.pop().ok_or("correction absent")?)?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert_ne!(record.state, WorkflowState::Completed);
    let correction_id = record
        .tasks
        .values()
        .find(|task| task.definition.corrective_of.as_ref() == Some(&original_id))
        .ok_or("correction absent")?
        .id
        .clone();
    control.edit(
        &context()?,
        "adopt",
        &record.id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::AdoptCorrection {
                task: original_id.clone(),
                correction: correction_id,
            }],
        },
    )?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(record.state, WorkflowState::Completed);
    assert_eq!(
        record.tasks.get(&original_id).map(|task| task.state),
        Some(TaskState::Stopped)
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum PreparationBehavior {
    ProvenRefusal,
    OwnershipUnknown,
    Success,
}

struct PreparationExecutor {
    inner: DeterministicExecutor,
    behavior: PreparationBehavior,
    settlements: AtomicUsize,
}

impl milkdrift_runtime::TaskExecutor for PreparationExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        now: u64,
    ) -> Result<milkdrift_runtime::ResolvedCapability, milkdrift_runtime::ExecutorError> {
        self.inner.resolve(requirement, now)
    }
    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &milkdrift_runtime::ExecutionDispatch,
    ) -> Result<milkdrift_runtime::PreparedExecution<'a>, milkdrift_runtime::ExecutorError> {
        match self.behavior {
            PreparationBehavior::ProvenRefusal => {
                Err(milkdrift_runtime::ExecutorError::BoundaryBeforeEntry(
                    "fixture proved no entry".into(),
                ))
            }
            PreparationBehavior::OwnershipUnknown => {
                Err(milkdrift_runtime::ExecutorError::Overloaded(
                    "another worker may own this invocation".into(),
                ))
            }
            PreparationBehavior::Success => self.inner.prepare_exact_entry(dispatch),
        }
    }
    fn cancel(
        &self,
        request: &milkdrift_capability::CancellationRequest,
    ) -> Result<milkdrift_capability::CancellationAcknowledgement, milkdrift_runtime::ExecutorError>
    {
        self.inner.cancel(request)
    }
    fn settle(
        &self,
        _dispatch: &milkdrift_runtime::ExecutionDispatch,
    ) -> Result<(), milkdrift_runtime::ExecutorError> {
        self.settlements.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn preparation_unknown_ownership_never_fabricates_non_entry_or_releases_claims() -> TestResult {
    for (behavior, expected_state, expected_settles) in [
        (PreparationBehavior::ProvenRefusal, TaskState::Failed, 1),
        (
            PreparationBehavior::OwnershipUnknown,
            TaskState::Uncertain,
            0,
        ),
        (PreparationBehavior::Success, TaskState::Completed, 1),
    ] {
        let root = tempfile::tempdir()?;
        let clock = Arc::new(ManualClock::new(1_000));
        let mut fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../capability/tests/fixtures/descriptor-v1.json"
        ))?;
        fixture["descriptor"]["category"] = serde_json::json!({"type":"process"});
        let descriptor = CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&fixture)?)?;
        let executor = Arc::new(PreparationExecutor {
            inner: DeterministicExecutor::new(descriptor.body().clone()),
            behavior,
            settlements: AtomicUsize::new(0),
        });
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(root.path()).with_clock(Arc::new(TaskStoreClock(clock.clone()))),
        )?);
        let authority = Arc::new(InvocationAllowance(Some(8)));
        let runtime = Arc::new(TaskRuntime::new(
            store,
            executor.clone(),
            authority.clone(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new("prepare-test", 1)?),
        ));
        let control = TaskControlService::new(runtime.clone(), authority, clock);
        let mut create = request()?;
        create.inputs.insert(
            "manifest".into(),
            TaskInputValue::Text {
                text: "captured input".into(),
            },
        );
        let receipt = control.create(&context()?, "submit", create)?;
        let dispatch = runtime.tick(4)?.pop().ok_or("dispatch absent")?;
        // Verify this is an actual executor boundary fixture, not a state-construction fixture.
        executor.resolve(
            &CapabilityRequirement::new(OperationId::new("model.generate")?),
            1_000,
        )?;
        runtime.execute(dispatch)?;
        let record = runtime
            .workflow(&receipt.workflow)?
            .ok_or("workflow absent")?;
        assert!(
            record
                .tasks
                .values()
                .all(|task| task.state == expected_state)
        );
        assert_eq!(
            executor.settlements.load(Ordering::SeqCst),
            expected_settles
        );
        assert!(runtime.recover()?.is_empty());
        assert_eq!(
            executor.settlements.load(Ordering::SeqCst),
            expected_settles * 2
        );
    }
    Ok(())
}
