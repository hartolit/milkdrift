//! Ordinary-task admission and entry use the production redb owner and executor boundary.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
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
    BoundaryClock, DeterministicExecutor, ManualClock, SequentialIdGenerator, TaskRuntime,
    TaskState, WorkflowState,
};

use super::{TestResult, claim};

#[path = "policy.rs"]
mod policy;

#[path = "correction.rs"]
mod correction;

#[path = "historical_outputs.rs"]
mod historical_outputs;

#[path = "model_faults.rs"]
mod model_faults;

#[path = "model_actions.rs"]
mod model_actions;

#[path = "preparation.rs"]
mod preparation;

#[path = "lifecycle.rs"]
mod lifecycle;

#[path = "lifecycle_faults.rs"]
mod lifecycle_faults;

fn request() -> TestResult<WorkflowCreate> {
    Ok(WorkflowCreate {
        project_id: None,
        title: "Inspect the supplied Cargo project".into(),
        description: "Read the captured manifest and report its targets".into(),
        tasks: vec![NewTask {
            alias: TaskId::new("inspect")?,
            definition: TaskDefinition {
                execution: milkdrift_blueprint::TaskExecution::Legacy,
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
                outputs: BTreeMap::new(),
                required: true,
                corrective_of: None,
                paths: Vec::new(),
                tools: Vec::new(),
                actions: None,
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
    runtime_with_limit(root, authority, clock, 32)
}

fn runtime_with_limit(
    root: &std::path::Path,
    authority: Arc<dyn AuthorityEvaluator>,
    clock: Arc<ManualClock>,
    maximum: u32,
) -> TestResult<(
    Arc<TaskRuntime>,
    Arc<DeterministicExecutor>,
    TaskControlService,
)> {
    let (runtime, executor, control, _) = runtime_and_store(root, authority, clock, maximum)?;
    Ok((runtime, executor, control))
}

type StoredTaskRuntime = (
    Arc<TaskRuntime>,
    Arc<DeterministicExecutor>,
    TaskControlService,
    Arc<RedbStore>,
);

fn runtime_and_store(
    root: &std::path::Path,
    authority: Arc<dyn AuthorityEvaluator>,
    clock: Arc<ManualClock>,
    maximum: u32,
) -> TestResult<StoredTaskRuntime> {
    let mut fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    fixture
        .get_mut("descriptor")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("fixture descriptor absent")?
        .insert("category".into(), serde_json::json!({"type":"process"}));
    let descriptor = CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&fixture)?)?;
    let executor = Arc::new(DeterministicExecutor::new(descriptor.body().clone()));
    let store = Arc::new(RedbStore::open_with_config(
        RedbStoreConfig::new(root).with_clock(Arc::new(TaskStoreClock(clock.clone()))),
    )?);
    let runtime = Arc::new(
        TaskRuntime::new(
            store.clone(),
            executor.clone(),
            authority.clone(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new("task-tests", 1)?),
        )
        .with_maximum_active_tasks(maximum)?,
    );
    let control = TaskControlService::new(runtime.clone(), authority, clock, store.clone());
    Ok((runtime, executor, control, store))
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
    assert!(matches!(
        control.create(&context()?, "submit", conflict),
        Err(milkdrift_control::ControlError::Runtime(
            milkdrift_runtime::RuntimeError::Persistence(
                milkdrift_persistence::PersistenceError::WorkflowIdempotencyConflict { .. }
            )
        ))
    ));
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
fn typed_edit_replay_preserves_storage_byte_bound_without_widening_public_transport() -> TestResult
{
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(RevocableAuthority(AtomicBool::new(false))),
        Arc::new(ManualClock::new(1000)),
    )?;
    let context = context()?;
    let created = control.create(&context, "typed-bound-create", request()?)?;
    let batch = TaskBatch {
        expected_generation: created.generation,
        changes: (0..32)
            .map(|index| TaskChange::BindInput {
                name: format!("future_{index}"),
                value: TaskInputValue::Text {
                    text: "\"".repeat(16_000),
                },
            })
            .collect(),
    };
    let canonical_public = serde_json::to_vec(&serde_json::to_value(&batch)?)?;
    assert!(canonical_public.len() > 1_000_000);
    let accepted = control.edit(
        &context,
        "typed-bound-edit",
        &created.workflow,
        batch.clone(),
    )?;
    assert!(
        accepted.acceptance.canonical_request.len()
            <= milkdrift_persistence::MAX_WORKFLOW_REQUEST_BYTES
    );
    assert_eq!(
        control.edit(
            &context,
            "typed-bound-edit",
            &created.workflow,
            batch.clone()
        )?,
        accepted
    );
    assert!(
        control
            .edit_exact(
                &context,
                "public-bound-edit",
                &created.workflow,
                batch.clone(),
                &canonical_public
            )
            .is_err_and(|error| error
                .to_string()
                .contains("canonical public request exceeds"))
    );

    let mut conflicting = batch;
    let Some(TaskChange::BindInput { value, .. }) = conflicting.changes.first_mut() else {
        return Err("input change absent".into());
    };
    *value = TaskInputValue::Text {
        text: "changed request".into(),
    };
    assert!(matches!(
        control.edit(
            &context,
            "typed-bound-edit",
            &created.workflow,
            conflicting.clone()
        ),
        Err(milkdrift_control::ControlError::Runtime(
            milkdrift_runtime::RuntimeError::Persistence(
                milkdrift_persistence::PersistenceError::WorkflowIdempotencyConflict { .. }
            )
        ))
    ));
    conflicting.expected_generation = accepted.generation;
    for change in &mut conflicting.changes {
        let TaskChange::BindInput { value, .. } = change else {
            return Err("input change absent".into());
        };
        *value = TaskInputValue::Text {
            text: "\"".repeat(16_384),
        };
    }
    assert!(
        serde_json::to_vec(&conflicting)?.len() > milkdrift_persistence::MAX_WORKFLOW_REQUEST_BYTES
    );
    assert!(
        control
            .edit(&context, "typed-too-large", &created.workflow, conflicting)
            .is_err()
    );
    assert_eq!(
        runtime
            .workflow(&created.workflow)?
            .ok_or("workflow absent")?
            .generation,
        accepted.generation
    );
    assert_eq!(executor.entry_count(), 0);
    Ok(())
}

#[test]
fn process_actions_refuse_create_add_and_revise_before_any_partial_work_or_entry() -> TestResult {
    for automatic_repair in [false, true] {
        let root = tempfile::tempdir()?;
        let (runtime, executor, control) = runtime(
            root.path(),
            Arc::new(RevocableAuthority(AtomicBool::new(false))),
            Arc::new(ManualClock::new(1000)),
        )?;
        let mut forbidden = request()?;
        forbidden
            .tasks
            .first_mut()
            .ok_or("task absent")?
            .definition
            .actions = Some(milkdrift_blueprint::TaskActionPolicy { automatic_repair });
        let rejected = control.create(&context()?, "invalid-process-actions", forbidden.clone());
        assert!(rejected.as_ref().is_err_and(|error| {
            error
                .to_string()
                .contains("only model task capabilities may publish structured actions")
        }));
        let rejected_id =
            TaskRuntime::workflow_identity(context()?.actor().as_str(), "invalid-process-actions")?;
        assert!(runtime.workflow(&rejected_id)?.is_none());
        assert_eq!(executor.entry_count(), 0);

        let created = control.create(&context()?, "ordinary-process", request()?)?;
        let before = runtime
            .workflow(&created.workflow)?
            .ok_or("workflow absent")?;
        let source = before.tasks.values().next().ok_or("source absent")?;
        let definition = forbidden
            .tasks
            .pop()
            .ok_or("forbidden task absent")?
            .definition;
        let add = TaskChange::Add {
            alias: TaskId::new("forbidden-child")?,
            definition: definition.clone(),
        };
        let revise = TaskChange::Revise {
            task: source.id.clone(),
            expected_version: source.version,
            definition,
        };
        for (key, change) in [
            ("add-process-actions", add),
            ("revise-process-actions", revise),
        ] {
            let batch = TaskBatch {
                expected_generation: before.generation,
                changes: vec![
                    TaskChange::BindInput {
                        name: "manifest".into(),
                        value: TaskInputValue::Text {
                            text: "must not partially release source".into(),
                        },
                    },
                    change,
                ],
            };
            assert!(
                control
                    .edit(&context()?, key, &created.workflow, batch)
                    .as_ref()
                    .is_err_and(|error| error
                        .to_string()
                        .contains("only model task capabilities may publish structured actions"))
            );
            assert_eq!(
                runtime
                    .workflow(&created.workflow)?
                    .ok_or("workflow absent")?,
                before
            );
        }
        assert!(runtime.tick(1)?.is_empty());
        assert_eq!(executor.entry_count(), 0);
    }
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
    runtime.recover()?;
    assert!(runtime.tick(1)?.is_empty());
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
        runtime.recover()?;
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
    create
        .tasks
        .first_mut()
        .ok_or("producer absent")?
        .definition
        .outputs
        .insert(
            "report".into(),
            milkdrift_blueprint::TaskOutputDeclaration {
                required: false,
                media_type: None,
                max_bytes: 4096,
            },
        );
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
                redirects: Vec::new(),
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

#[test]
fn exact_public_request_keeps_omitted_defaults_distinct_after_mapping() -> TestResult {
    let root = tempfile::tempdir()?;
    let (_, _, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1000)),
    )?;
    let request = request()?;
    let omitted = serde_json::to_vec(&serde_json::json!({"title":"same semantic request"}))?;
    let explicit = serde_json::to_vec(
        &serde_json::json!({"title":"same semantic request","finish_when_settled":true}),
    )?;
    let first = control.create_exact(&context()?, "transport", request.clone(), &omitted)?;
    assert_eq!(
        control.create_exact(&context()?, "transport", request.clone(), &omitted)?,
        first
    );
    assert!(
        control
            .create_exact(&context()?, "transport", request.clone(), &explicit)
            .is_err()
    );
    assert!(
        control
            .create_exact(
                &context()?,
                "invalid-canonical",
                request,
                br#"{ "title": "spaced" }"#
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn capability_success_missing_declared_output_is_failed_without_rewriting_terminal_evidence()
-> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, _, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1000)),
    )?;
    let mut request = request()?;
    request.inputs.insert(
        "manifest".into(),
        TaskInputValue::Text {
            text: "captured manifest".into(),
        },
    );
    request
        .tasks
        .first_mut()
        .ok_or("task absent")?
        .definition
        .outputs
        .insert(
            "report".into(),
            milkdrift_blueprint::TaskOutputDeclaration {
                required: true,
                media_type: Some("text/plain".into()),
                max_bytes: 1024,
            },
        );
    let receipt = control.create(&context()?, "output-contract", request)?;
    let dispatch = runtime.tick(1)?.pop().ok_or("dispatch absent")?;
    runtime.execute(dispatch)?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let task = record.tasks.values().next().ok_or("task absent")?;
    assert_eq!(task.state, TaskState::Failed);
    assert_eq!(
        task.result
            .as_ref()
            .ok_or("result absent")?
            .terminal
            .status(),
        milkdrift_capability::TerminalStatus::Success
    );
    assert_eq!(
        task.reason.as_deref(),
        Some("required output report was not published")
    );
    assert_eq!(record.state, WorkflowState::Blocked);
    Ok(())
}

#[test]
fn shutdown_closes_new_work_and_preserves_exact_replay_and_reserved_stop() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1000)),
    )?;
    let request = request()?;
    let receipt = control.create(&context()?, "shutdown", request.clone())?;
    runtime.begin_shutdown();
    assert_eq!(
        control.create(&context()?, "shutdown", request.clone())?,
        receipt
    );
    assert!(
        control
            .create(&context()?, "new-after-shutdown", request)
            .is_err()
    );
    assert!(runtime.tick(1)?.is_empty());
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let task = record.tasks.keys().next().ok_or("task absent")?.clone();
    control.edit(
        &context()?,
        "stop-after-shutdown",
        &receipt.workflow,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Stop { task }],
        },
    )?;
    assert_eq!(executor.entry_count(), 0);
    assert_eq!(
        runtime
            .workflow(&receipt.workflow)?
            .ok_or("workflow absent")?
            .state,
        WorkflowState::Stopped
    );
    Ok(())
}

#[test]
fn exhausted_account_pauses_peers_and_continuation_does_not_replenish_it() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(1))),
        Arc::new(ManualClock::new(1000)),
    )?;
    let mut create = request()?;
    create.inputs.insert(
        "manifest".into(),
        TaskInputValue::Text {
            text: "captured".into(),
        },
    );
    let task = create.tasks.first().ok_or("task absent")?.clone();
    for alias in ["second", "third"] {
        let mut next = task.clone();
        next.alias = TaskId::new(alias)?;
        create.tasks.push(next);
    }
    create.limits.concurrent_tasks = 1;
    let receipt = control.create(&context()?, "pause-budget", create)?;
    runtime.execute(runtime.tick(1)?.pop().ok_or("first absent")?)?;
    runtime.execute(runtime.tick(1)?.pop().ok_or("second absent")?)?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert!(record.wait_question.is_some());
    assert!(runtime.tick(1)?.is_empty());
    assert_eq!(executor.entry_count(), 1);
    assert_eq!(
        record
            .tasks
            .values()
            .filter(|task| task.state == TaskState::Queued)
            .count(),
        1
    );
    control.edit(
        &context()?,
        "continue-account",
        &record.id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Continue],
        },
    )?;
    runtime.execute(runtime.tick(1)?.pop().ok_or("continued task absent")?)?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert!(record.wait_question.is_some());
    assert_eq!(record.committed_allowance.invocations, Some(1));
    assert_eq!(executor.entry_count(), 1);
    Ok(())
}

#[test]
fn correction_adoption_redirects_only_named_future_binding_and_rejects_competition() -> TestResult {
    use milkdrift_capability::{
        ArtifactReference, InvocationEventKind, InvocationTerminal, TerminalStatus,
    };
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1000)),
    )?;
    let mut create = request()?;
    create
        .tasks
        .first_mut()
        .ok_or("producer absent")?
        .definition
        .outputs
        .insert(
            "report".into(),
            milkdrift_blueprint::TaskOutputDeclaration {
                required: false,
                media_type: None,
                max_bytes: 4096,
            },
        );
    let mut consumer = create.tasks.first().ok_or("original absent")?.clone();
    consumer.alias = TaskId::new("consumer")?;
    consumer
        .definition
        .dependencies
        .insert(TaskId::new("inspect")?);
    consumer.definition.inputs = BTreeMap::from([(
        "review".into(),
        TaskInputBinding::TaskOutput {
            task: TaskId::new("inspect")?,
            output: "report".into(),
            optional: false,
        },
    )]);
    create.tasks.push(consumer);
    let receipt = control.create(&context()?, "redirect-create", create)?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let original = record
        .tasks
        .values()
        .find(|task| task.definition.dependencies.is_empty())
        .ok_or("original absent")?;
    let original_id = original.id.clone();
    let mut correction = original.definition.clone();
    correction.inputs.clear();
    correction.corrective_of = Some(original_id.clone());
    let consumer = record
        .tasks
        .values()
        .find(|task| !task.definition.dependencies.is_empty())
        .ok_or("consumer absent")?
        .id
        .clone();
    control.edit(
        &context()?,
        "redirect-add",
        &record.id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![
                TaskChange::Stop {
                    task: original_id.clone(),
                },
                TaskChange::Add {
                    alias: TaskId::new("fix")?,
                    definition: correction,
                },
            ],
        },
    )?;
    let artifact = ArtifactReference::new(
        "correction-output",
        blake3::hash(b"corrected").to_hex().to_string(),
        Some("text/plain".into()),
        Some(9),
    )?;
    executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "report".into(),
                reference: artifact.clone(),
            },
            InvocationEventKind::Terminal {
                terminal: InvocationTerminal::new(
                    TerminalStatus::Success,
                    vec![artifact.clone()],
                    None,
                    None,
                    SideEffectClass::None,
                )?,
            },
        ],
    )?;
    runtime.execute(runtime.tick(4)?.pop().ok_or("corrective dispatch absent")?)?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let correction = record
        .tasks
        .values()
        .find(|task| task.definition.corrective_of.is_some())
        .ok_or("correction absent")?
        .id
        .clone();
    let change = TaskChange::AdoptCorrection {
        task: original_id.clone(),
        correction: correction.clone(),
        redirects: vec![milkdrift_blueprint::TaskBindingRedirect {
            consumer: consumer.clone(),
            expected_version: 1,
            input: "review".into(),
        }],
    };
    control.edit(
        &context()?,
        "redirect-adopt",
        &record.id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![change.clone()],
        },
    )?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let edited = record.tasks.get(&consumer).ok_or("consumer absent")?;
    assert_eq!(edited.version, 2);
    assert_eq!(edited.previous_definitions.len(), 1);
    assert_eq!(edited.definition.dependencies, BTreeSet::from([correction]));
    assert!(
        control
            .edit(
                &context()?,
                "competing-adoption",
                &record.id,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![change]
                }
            )
            .is_err()
    );
    let dispatch = runtime.tick(4)?.pop().ok_or("redirected consumer absent")?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(
        record
            .tasks
            .get(&consumer)
            .and_then(|task| task.frozen.as_ref())
            .and_then(|start| start.inputs.get("review")),
        Some(&TaskInputValue::Artifact {
            reference: artifact
        })
    );
    runtime.execute(dispatch)?;
    assert_eq!(
        runtime
            .workflow(&receipt.workflow)?
            .ok_or("workflow absent")?
            .tasks
            .get(&original_id)
            .map(|task| task.state),
        Some(TaskState::Stopped)
    );
    Ok(())
}

#[test]
fn global_task_start_bound_spans_workflows_and_recovers_accepted_occupancy() -> TestResult {
    let root = tempfile::tempdir()?;
    let authority = Arc::new(InvocationAllowance(Some(8)));
    let clock = Arc::new(ManualClock::new(1000));
    let (first, second) = {
        let (runtime, _executor, control) =
            runtime_with_limit(root.path(), authority.clone(), clock.clone(), 1)?;
        let mut create = request()?;
        create.inputs.insert(
            "manifest".into(),
            TaskInputValue::Text {
                text: "captured".into(),
            },
        );
        let first = control
            .create(&context()?, "global-first", create.clone())?
            .workflow;
        let second = control
            .create(&context()?, "global-second", create)?
            .workflow;
        assert_eq!(runtime.tick(4)?.len(), 1);
        assert!(runtime.tick(4)?.is_empty());
        (first, second)
    };
    let (runtime, executor, _control) = runtime_with_limit(root.path(), authority, clock, 1)?;
    runtime.recover()?;
    let retained = runtime.tick(4)?;
    assert_eq!(retained.len(), 1);
    assert!(
        runtime.tick(4)?.is_empty(),
        "reopen must count a redispatched frozen start"
    );
    runtime.execute(
        retained
            .into_iter()
            .next()
            .ok_or("recovered dispatch absent")?,
    )?;
    let next = runtime.tick(4)?;
    assert_eq!(next.len(), 1);
    runtime.execute(next.into_iter().next().ok_or("next dispatch absent")?)?;
    assert_eq!(executor.entry_count(), 2);
    for workflow in [first, second] {
        assert_eq!(
            runtime.workflow(&workflow)?.ok_or("workflow absent")?.state,
            WorkflowState::Completed
        );
    }
    Ok(())
}

#[test]
fn oversized_capture_and_combined_escaped_inputs_refuse_before_acceptance_or_edit() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, _executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1000)),
    )?;
    let mut create = request()?;
    create.inputs.insert(
        "manifest".into(),
        TaskInputValue::Text {
            text: "x".repeat(milkdrift_blueprint::MAX_TASK_TEXT_BYTES + 1),
        },
    );
    assert!(
        control
            .create(&context()?, "large-capture", create.clone())
            .is_err()
    );
    let refused = TaskRuntime::workflow_identity(context()?.actor().as_str(), "large-capture")?;
    assert!(runtime.workflow(&refused)?.is_none());
    create.inputs.insert(
        "manifest".into(),
        TaskInputValue::Text {
            text: "x".repeat(milkdrift_blueprint::MAX_TASK_TEXT_BYTES),
        },
    );
    let receipt = control.create(&context()?, "bounded-capture", create)?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert!(
        control
            .edit(
                &context()?,
                "escaped-overflow",
                &record.id,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![TaskChange::BindInput {
                        name: "manifest".into(),
                        value: TaskInputValue::Text {
                            text: "\0".repeat(5000)
                        }
                    }]
                }
            )
            .is_err()
    );
    assert_eq!(
        runtime.workflow(&record.id)?.ok_or("workflow absent")?,
        record
    );
    runtime.execute(
        runtime
            .tick(1)?
            .pop()
            .ok_or("exact boundary capture never became ready")?,
    )?;
    assert_eq!(
        runtime
            .workflow(&record.id)?
            .ok_or("workflow absent")?
            .state,
        WorkflowState::Completed
    );
    Ok(())
}

#[test]
fn oversized_dynamic_settled_outcomes_fail_before_external_entry() -> TestResult {
    use milkdrift_capability::{
        ErrorClass, InvocationEventKind, InvocationFailure, InvocationTerminal, TerminalStatus,
    };
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(RevocableAuthority(AtomicBool::new(false))),
        Arc::new(ManualClock::new(1000)),
    )?;
    let mut create = request()?;
    let producer = create.tasks.first_mut().ok_or("producer absent")?;
    producer.definition.inputs.clear();
    let mut consumer = producer.definition.clone();
    consumer.title = "Inspect settled outcome".into();
    consumer.dependencies.insert(TaskId::new("inspect")?);
    consumer.inputs = (0..8)
        .map(|index| {
            Ok((
                format!("outcome_{index}"),
                TaskInputBinding::SettledOutcome {
                    task: TaskId::new("inspect")?,
                },
            ))
        })
        .collect::<TestResult<_>>()?;
    create.tasks.push(NewTask {
        alias: TaskId::new("consumer")?,
        definition: consumer,
    });
    let receipt = control.create(&context()?, "dynamic-bound", create)?;
    executor.set_script(
        OperationId::new("model.generate")?,
        vec![InvocationEventKind::Terminal {
            terminal: InvocationTerminal::new(
                TerminalStatus::Failure,
                Vec::new(),
                Some(InvocationFailure::new(
                    ErrorClass::Provider,
                    false,
                    "bounded_failure",
                    "x".repeat(4096),
                    None,
                )?),
                None,
                SideEffectClass::None,
            )?,
        }],
    )?;
    runtime.execute(runtime.tick(4)?.pop().ok_or("producer dispatch absent")?)?;
    assert!(runtime.tick(4)?.is_empty());
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let consumer = record
        .tasks
        .values()
        .find(|task| task.definition.title == "Inspect settled outcome")
        .ok_or("consumer absent")?;
    assert_eq!(consumer.state, TaskState::Failed);
    assert!(
        consumer
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("combined 24 KiB"))
    );
    assert!(consumer.frozen.is_none());
    assert!(consumer.started_at.is_none());
    assert!(consumer.ended_at.is_some());
    assert_eq!(executor.entry_count(), 1);
    assert!(runtime.tick(4)?.is_empty());
    Ok(())
}

#[test]
fn discovery_fields_cannot_contradict_the_authoritative_workflow_document() -> TestResult {
    use milkdrift_persistence::{TimestampMillis, WorkflowCommit, WorkflowStore};
    for field in ["title", "description", "updated_at"] {
        let root = tempfile::tempdir()?;
        let clock = Arc::new(ManualClock::new(1000));
        let authority = Arc::new(RevocableAuthority(AtomicBool::new(false)));
        let (runtime, executor, control) = runtime(root.path(), authority.clone(), clock.clone())?;
        let receipt = control.create(&context()?, "discovery-proof", request()?)?;
        drop(control);
        drop(runtime);
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(root.path()).with_clock(Arc::new(TaskStoreClock(clock.clone()))),
        )?);
        let mut snapshot = store
            .workflow(&receipt.workflow)?
            .ok_or("snapshot absent")?;
        let expected = snapshot.generation;
        snapshot.generation = snapshot
            .generation
            .checked_add(1)
            .ok_or("generation overflow")?;
        snapshot
            .document
            .as_object_mut()
            .ok_or("workflow document is not an object")?
            .insert("generation".into(), serde_json::json!(snapshot.generation));
        match field {
            "title" => snapshot.title = "Different discovery title".into(),
            "description" => snapshot.description = "Different discovery description".into(),
            "updated_at" => snapshot.updated_at = TimestampMillis::new(1001),
            _ => unreachable!(),
        }
        store.commit_workflow(WorkflowCommit {
            project_guard: None,
            project_id: None,
            expected_generation: Some(expected),
            snapshot,
            command: None,
        })?;
        let runtime = TaskRuntime::new(
            store.clone(),
            executor,
            authority,
            clock,
            Arc::new(SequentialIdGenerator::new("discovery-test", 1)?),
        );
        assert!(
            matches!(
                runtime.workflow(&receipt.workflow),
                Err(milkdrift_runtime::RuntimeError::InvalidHistory(_))
            ),
            "discovery field {field} was accepted despite contradiction"
        );
    }
    Ok(())
}
