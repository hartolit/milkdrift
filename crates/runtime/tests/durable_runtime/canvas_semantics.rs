//! Shared canvas choices remain prospective runtime semantics after restart and direct commands.
use super::{InvocationAllowance, TestResult, context, request, runtime_and_store};
use milkdrift_blueprint::{
    NewTask, TaskBatch, TaskChange, TaskDependencies, TaskId, TaskInputBinding, TaskInputValue,
    TaskOrderCondition, TaskOutputDeclaration,
};
use milkdrift_persistence::{WorkflowCommandAcceptance, WorkflowCommit, WorkflowStore};
use milkdrift_runtime::{ManualClock, TaskState, WorkflowState};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[test]
fn empty_authoring_workflow_stays_available_and_rename_has_no_task_cost() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, _, control, _) = runtime_and_store(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1000)),
        32,
    )?;
    let mut create = request()?;
    create.tasks.clear();
    let id = control
        .create(&context()?, "empty-canvas", create)?
        .workflow;
    assert!(runtime.tick(4)?.is_empty());
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    assert_eq!(record.state, WorkflowState::Waiting);
    assert_eq!(record.admitted_tasks, 0);
    control.edit(
        &context()?,
        "rename-empty",
        &id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Rename {
                title: "Release notes".into(),
                description: "Saved unfinished work".into(),
            }],
        },
    )?;
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    assert_eq!(record.title, "Release notes");
    assert_eq!(record.admitted_tasks, 0);
    assert_eq!(record.state, WorkflowState::Waiting);
    Ok(())
}

#[test]
fn human_revision_accepts_progress_but_refuses_an_intervening_decision() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, _, control, _) = runtime_and_store(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1000)),
        32,
    )?;
    let receipt = control.create(&context()?, "semantic-guard", request()?)?;
    let observed = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let task = observed.tasks.values().next().ok_or("task absent")?;
    let mut changed = task.definition.clone();
    changed.instructions = "Write a concise overview of the supplied manifest".into();
    let batch = TaskBatch {
        expected_generation: observed.generation,
        changes: vec![TaskChange::Revise {
            task: task.id.clone(),
            expected_version: task.version,
            definition: changed,
        }],
    };
    assert!(runtime.tick(1)?.is_empty());
    let progressed = runtime.workflow(&observed.id)?.ok_or("workflow absent")?;
    assert!(progressed.generation > observed.generation);
    assert_eq!(progressed.decision_generation, observed.decision_generation);
    let accepted = control.edit(
        &context()?,
        "revise-after-progress",
        &observed.id,
        batch.clone(),
    )?;
    assert_eq!(
        control.edit(&context()?, "revise-after-progress", &observed.id, batch)?,
        accepted
    );
    assert!(
        control
            .edit(
                &context()?,
                "stale-new-edit",
                &observed.id,
                TaskBatch {
                    expected_generation: observed.generation,
                    changes: vec![TaskChange::Rename {
                        title: "Conflicting stale title".into(),
                        description: String::new()
                    }]
                }
            )
            .is_err()
    );
    let current = runtime.workflow(&observed.id)?.ok_or("workflow absent")?;
    assert!(
        control
            .edit(
                &context()?,
                "future-generation",
                &observed.id,
                TaskBatch {
                    expected_generation: current.generation + 1,
                    changes: vec![TaskChange::Continue]
                }
            )
            .is_err()
    );
    assert_eq!(
        runtime
            .workflow(&observed.id)?
            .ok_or("workflow absent")?
            .tasks
            .get(&task.id)
            .ok_or("task absent")?
            .previous_definitions,
        vec![task.definition.clone()]
    );
    Ok(())
}

struct MediaExecutor(milkdrift_runtime::DeterministicExecutor);
impl milkdrift_runtime::TaskExecutor for MediaExecutor {
    fn task_authoring(
        &self,
        requirement: &milkdrift_capability::CapabilityRequirement,
    ) -> Result<
        Option<(
            milkdrift_capability::CapabilityDescriptor,
            milkdrift_capability::TaskAuthoringContract,
        )>,
        milkdrift_runtime::ExecutorError,
    > {
        let descriptor = self.0.resolve(requirement, 0)?.descriptor().clone();
        Ok(Some((
            descriptor,
            milkdrift_capability::TaskAuthoringContract {
                label: "Configured context reader".into(),
                inputs: vec![],
                allows_additional_inputs: true,
                input_media: milkdrift_capability::TaskInputMedia::TextJsonOrImage,
                outputs: vec![],
                default_output: None,
            },
        )))
    }
    fn resolve(
        &self,
        requirement: &milkdrift_capability::CapabilityRequirement,
        now: u64,
    ) -> Result<milkdrift_runtime::ResolvedCapability, milkdrift_runtime::ExecutorError> {
        self.0.resolve(requirement, now)
    }
    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &milkdrift_runtime::ExecutionDispatch,
    ) -> Result<milkdrift_runtime::PreparedExecution<'a>, milkdrift_runtime::ExecutorError> {
        self.0.prepare_exact_entry(dispatch)
    }
    fn cancel(
        &self,
        request: &milkdrift_capability::CancellationRequest,
    ) -> Result<milkdrift_capability::CancellationAcknowledgement, milkdrift_runtime::ExecutorError>
    {
        self.0.cancel(request)
    }
}

#[test]
fn stable_output_ceilings_still_share_the_actual_task_output_limit() -> TestResult {
    use milkdrift_capability::{
        ArtifactReference, InvocationEventKind, InvocationTerminal, OperationId, SideEffectClass,
        TerminalStatus,
    };
    let root = tempfile::tempdir()?;
    let (runtime, executor, control, _) = runtime_and_store(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1000)),
        32,
    )?;
    let mut create = request()?;
    let task = create.tasks.first_mut().ok_or("task absent")?;
    task.definition.inputs.clear();
    for name in ["report", "diagnostic"] {
        task.definition.outputs.insert(
            name.into(),
            TaskOutputDeclaration {
                required: false,
                media_type: Some("text/plain".into()),
                max_bytes: 2097152,
            },
        );
    }
    let id = control
        .create(&context()?, "aggregate-output", create)?
        .workflow;
    let first = ArtifactReference::new(
        "report",
        "a".repeat(64),
        Some("text/plain".into()),
        Some(1_500_000),
    )?;
    let second = ArtifactReference::new(
        "diagnostic",
        "b".repeat(64),
        Some("text/plain".into()),
        Some(1_000_000),
    )?;
    executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "report".into(),
                reference: first,
            },
            InvocationEventKind::Output {
                name: "diagnostic".into(),
                reference: second,
            },
            InvocationEventKind::Terminal {
                terminal: InvocationTerminal::new(
                    TerminalStatus::Success,
                    vec![],
                    None,
                    None,
                    SideEffectClass::None,
                )?,
            },
        ],
    )?;
    let dispatch = runtime.tick(1)?.pop().ok_or("start absent")?;
    runtime.execute(dispatch)?;
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    let task = record.tasks.values().next().ok_or("task absent")?;
    assert_eq!(task.state, TaskState::Uncertain);
    assert!(
        task.reason
            .as_deref()
            .is_some_and(|reason| reason.contains("output count or byte allowance exceeded"))
    );
    let reports = &task.frozen.as_ref().ok_or("start absent")?.reports;
    assert_eq!(
        reports
            .iter()
            .filter_map(|event| event.kind().output())
            .count(),
        1,
        "second result exceeded the shared allowance before acceptance"
    );
    Ok(())
}

fn generation_runtime(
    root: &std::path::Path,
    generation: u64,
) -> TestResult<(
    Arc<milkdrift_runtime::TaskRuntime>,
    milkdrift_control::TaskControlService,
)> {
    let clock = Arc::new(ManualClock::new(1000));
    let authority = Arc::new(InvocationAllowance(Some(8)));
    let store = Arc::new(milkdrift_redb_store::RedbStore::open_with_config(
        milkdrift_redb_store::RedbStoreConfig::new(root)
            .with_clock(Arc::new(super::TaskStoreClock(clock.clone()))),
    )?);
    let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    *descriptor
        .pointer_mut("/descriptor/descriptor_revision")
        .ok_or("descriptor revision absent")? = serde_json::json!(generation);
    *descriptor
        .pointer_mut("/descriptor/category")
        .ok_or("descriptor category absent")? = serde_json::json!({"type":"process"});
    let descriptor = milkdrift_capability::CapabilityDescriptorDocument::from_json(
        &serde_json::to_vec(&descriptor)?,
    )?;
    let executor = Arc::new(milkdrift_runtime::DeterministicExecutor::new(
        descriptor.body().clone(),
    ));
    let runtime = Arc::new(milkdrift_runtime::TaskRuntime::new(
        store.clone(),
        executor,
        authority.clone(),
        clock.clone(),
        Arc::new(milkdrift_runtime::SequentialIdGenerator::new(
            format!("generation-{generation}"),
            1,
        )?),
    ));
    let control =
        milkdrift_control::TaskControlService::new(runtime.clone(), authority, clock, store);
    Ok((runtime, control))
}

#[test]
fn reviewed_generation_survives_reopen_and_blocks_silent_new_selection_until_revision() -> TestResult
{
    let root = tempfile::tempdir()?;
    let id = {
        let (runtime, control) = generation_runtime(root.path(), 3)?;
        let mut create = request()?;
        let task = create.tasks.first_mut().ok_or("task absent")?;
        task.definition.capability =
            task.definition
                .capability
                .clone()
                .exact(milkdrift_capability::CapabilityId::new(
                    "anthropic-primary",
                )?);
        task.definition.capability_generation = Some(3);
        let id = control
            .create(&context()?, "reviewed-choice", create)?
            .workflow;
        assert!(runtime.tick(1)?.is_empty());
        id
    };
    let (runtime, control) = generation_runtime(root.path(), 4)?;
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    control.edit(
        &context()?,
        "supply-after-model-change",
        &id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::BindInput {
                name: "manifest".into(),
                value: TaskInputValue::Text {
                    text: "name = 'reviewed'".into(),
                },
            }],
        },
    )?;
    assert!(runtime.tick(1)?.is_empty());
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    let task = record.tasks.values().next().ok_or("task absent")?;
    assert_eq!(task.state, TaskState::Queued);
    assert!(task.frozen.is_none());
    assert!(
        task.reason
            .as_deref()
            .is_some_and(|reason| reason.contains("selected model or agent changed"))
    );
    assert!(runtime.tick(1)?.is_empty());
    assert_eq!(
        runtime.workflow(&id)?.ok_or("workflow absent")?.generation,
        record.generation,
        "unchanged blocked choice does not churn generations"
    );
    let mut reviewed = task.definition.clone();
    reviewed.capability_generation = Some(4);
    control.edit(
        &context()?,
        "review-current-choice",
        &id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Revise {
                task: task.id.clone(),
                expected_version: task.version,
                definition: reviewed,
            }],
        },
    )?;
    let dispatch = runtime.tick(1)?.pop().ok_or("reviewed start absent")?;
    assert_eq!(dispatch.resolution().descriptor_revision(), 4);
    runtime.execute(dispatch)?;
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    let task = record.tasks.values().next().ok_or("task absent")?;
    assert_eq!(
        task.previous_definitions
            .first()
            .ok_or("previous definition absent")?
            .capability_generation,
        Some(3)
    );
    assert_eq!(task.state, TaskState::Completed);

    let mut stale = request()?;
    let task = stale.tasks.first_mut().ok_or("task absent")?;
    task.definition.capability =
        task.definition
            .capability
            .clone()
            .exact(milkdrift_capability::CapabilityId::new(
                "anthropic-primary",
            )?);
    task.definition.capability_generation = Some(3);
    assert!(
        control
            .create(&context()?, "stale-admission", stale.clone())
            .is_err()
    );
    stale
        .tasks
        .first_mut()
        .ok_or("task absent")?
        .definition
        .capability_generation = None;
    assert!(
        control
            .create(&context()?, "unreviewed-direct-admission", stale)
            .is_ok()
    );
    Ok(())
}

#[test]
fn shared_admission_rejects_known_uninterpretable_media_and_accepts_supported_families()
-> TestResult {
    use milkdrift_runtime::TaskExecutor;
    for (media, allowed) in [
        ("text/plain", true),
        ("application/json", true),
        ("application/vnd.example+json", true),
        ("image/png", true),
        ("application/octet-stream", false),
        ("application/zip", false),
    ] {
        let root = tempfile::tempdir()?;
        let clock = Arc::new(ManualClock::new(1000));
        let authority = Arc::new(InvocationAllowance(Some(8)));
        let (_, inner, _, store) =
            runtime_and_store(root.path(), authority.clone(), clock.clone(), 32)?;
        let mut create = request()?;
        let base = create.tasks.first_mut().ok_or("source absent")?;
        base.definition.inputs.clear();
        let descriptor = inner
            .resolve(&base.definition.capability, 1000)?
            .descriptor()
            .clone();
        let executor = Arc::new(MediaExecutor(
            milkdrift_runtime::DeterministicExecutor::new(descriptor),
        ));
        let runtime = Arc::new(milkdrift_runtime::TaskRuntime::new(
            store.clone(),
            executor,
            authority.clone(),
            clock.clone(),
            Arc::new(milkdrift_runtime::SequentialIdGenerator::new("media", 1)?),
        ));
        let control = milkdrift_control::TaskControlService::new(
            runtime.clone(),
            authority,
            clock,
            store.clone(),
        );
        base.definition.outputs.insert(
            "report".into(),
            TaskOutputDeclaration {
                required: false,
                media_type: Some(media.into()),
                max_bytes: 2097152,
            },
        );
        // Exposing another output does not reduce the first port's stable ceiling.
        base.definition.outputs.insert(
            "diagnostic".into(),
            TaskOutputDeclaration {
                required: false,
                media_type: Some("text/plain".into()),
                max_bytes: 2097152,
            },
        );
        let mut consumer = base.definition.clone();
        consumer.outputs.clear();
        consumer.inputs.insert(
            "source".into(),
            TaskInputBinding::TaskOutput {
                task: base.alias.clone(),
                output: "report".into(),
                optional: false,
            },
        );
        create.tasks.push(NewTask {
            alias: TaskId::new("consumer")?,
            definition: consumer,
        });
        let result = control.create(&context()?, "media-admission", create);
        assert_eq!(result.is_ok(), allowed, "known {media}: {result:?}");
        if let Ok(receipt) = result {
            let snapshot = store
                .workflow(&receipt.workflow)?
                .ok_or("snapshot absent")?;
            assert_eq!(
                snapshot.artifact_capacity,
                2 * milkdrift_blueprint::MAX_TASK_OUTPUT_BYTES,
                "one aggregate reservation per task, not per output"
            );
        }
    }
    Ok(())
}

#[test]
fn optional_data_does_not_weaken_independent_success_order() -> TestResult {
    for condition in [TaskOrderCondition::Success, TaskOrderCondition::Settled] {
        let root = tempfile::tempdir()?;
        let (runtime, _, control, _) = runtime_and_store(
            root.path(),
            Arc::new(InvocationAllowance(Some(8))),
            Arc::new(ManualClock::new(1000)),
            32,
        )?;
        let mut create = request()?;
        let producer = create.tasks.first_mut().ok_or("producer absent")?;
        producer.definition.required = false;
        producer.definition.outputs.insert(
            "report".into(),
            TaskOutputDeclaration {
                required: false,
                media_type: Some("text/plain".into()),
                max_bytes: 2097152,
            },
        );
        let mut consumer = producer.definition.clone();
        consumer.required = true;
        consumer.outputs.clear();
        consumer.inputs = BTreeMap::from([(
            "source".into(),
            TaskInputBinding::TaskOutput {
                task: producer.alias.clone(),
                output: "report".into(),
                optional: true,
            },
        )]);
        consumer.dependencies =
            TaskDependencies::Ordered(BTreeMap::from([(producer.alias.clone(), condition)]));
        create.tasks.push(NewTask {
            alias: TaskId::new("consumer")?,
            definition: consumer,
        });
        let receipt = control.create(&context()?, "independent-order", create)?;
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
            "stop-original",
            &record.id,
            TaskBatch {
                expected_generation: record.generation,
                changes: vec![TaskChange::Stop { task: producer }],
            },
        )?;
        let starts = runtime.tick(4)?;
        assert_eq!(
            starts.len(),
            usize::from(condition == TaskOrderCondition::Settled)
        );
        let record = runtime.workflow(&record.id)?.ok_or("workflow absent")?;
        let consumer = record
            .tasks
            .values()
            .find(|task| task.definition.required)
            .ok_or("consumer absent")?;
        if condition == TaskOrderCondition::Settled {
            assert_eq!(
                consumer
                    .frozen
                    .as_ref()
                    .and_then(|start| start.inputs.get("source")),
                Some(&TaskInputValue::Absent)
            );
        } else {
            assert_eq!(consumer.state, TaskState::Queued);
        }
    }
    Ok(())
}

#[test]
fn retained_dependency_arrays_keep_their_bytes_and_predicate_until_explicit_revision() -> TestResult
{
    let root = tempfile::tempdir()?;
    let clock = Arc::new(ManualClock::new(1000));
    let authority = Arc::new(InvocationAllowance(Some(8)));
    let (id, consumer, original_bytes, old_batch, old_receipt) = {
        let (runtime, _, control, store) =
            runtime_and_store(root.path(), authority.clone(), clock.clone(), 32)?;
        let mut create = request()?;
        let producer = create.tasks.first_mut().ok_or("producer absent")?;
        producer.definition.required = false;
        producer.definition.outputs.insert(
            "report".into(),
            TaskOutputDeclaration {
                required: false,
                media_type: None,
                max_bytes: 2097152,
            },
        );
        let mut consumer = producer.definition.clone();
        consumer.required = true;
        consumer.outputs.clear();
        consumer.inputs = BTreeMap::from([(
            "source".into(),
            TaskInputBinding::TaskOutput {
                task: producer.alias.clone(),
                output: "report".into(),
                optional: true,
            },
        )]);
        create.tasks.push(NewTask {
            alias: TaskId::new("consumer")?,
            definition: consumer,
        });
        let id = control
            .create(&context()?, "retained-old-definition", create)?
            .workflow;
        let mut record = runtime.workflow(&id)?.ok_or("workflow absent")?;
        let producer = record
            .tasks
            .values()
            .find(|task| !task.definition.required)
            .ok_or("producer absent")?
            .id
            .clone();
        let consumer = record
            .tasks
            .values_mut()
            .find(|task| task.definition.required)
            .ok_or("consumer absent")?;
        consumer.definition.dependencies = TaskDependencies::Legacy(BTreeSet::from([producer]));
        let original_bytes = serde_json::to_vec(&consumer.definition)?;
        let old_batch = TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Revise {
                task: consumer.id.clone(),
                expected_version: consumer.version,
                definition: consumer.definition.clone(),
            }],
        };
        let consumer = consumer.id.clone();
        // This is an older durable row, not a fresh legacy-format admission.
        let mut snapshot = store.workflow(&id)?.ok_or("snapshot absent")?;
        let prior = snapshot.generation;
        snapshot.generation += 1;
        record.generation = snapshot.generation;
        snapshot.document = serde_json::to_value(record)?;
        let outcome = store.commit_workflow(WorkflowCommit {
            authoring: None,
            project_guard: None,
            project_id: None,
            expected_generation: Some(prior),
            snapshot,
            command: Some(WorkflowCommandAcceptance {
                actor:"human:task-test".into(),key:"older-revision".into(),reserved_action:None,
                canonical_request:serde_json::to_vec(&serde_json::json!({"operation":"edit_workflow","workflow":id,"batch":old_batch}))?,
                result:serde_json::json!({"workflow_id":id,"generation":prior+1,"aliases":{}}),
            }),
        })?;
        let milkdrift_persistence::WorkflowCommitOutcome::Applied(Some(old_receipt)) = outcome
        else {
            return Err("historical receipt absent".into());
        };
        (id, consumer, original_bytes, old_batch, old_receipt)
    };
    let (runtime, _, control, _) = runtime_and_store(root.path(), authority, clock, 32)?;
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    assert_eq!(
        control.edit(&context()?, "older-revision", &id, old_batch.clone())?,
        old_receipt
    );
    let mut conflicting = old_batch;
    conflicting.expected_generation += 1;
    assert!(
        control
            .edit(&context()?, "older-revision", &id, conflicting)
            .is_err()
    );
    let task = record.tasks.get(&consumer).ok_or("consumer absent")?;
    assert_eq!(serde_json::to_vec(&task.definition)?, original_bytes);
    let conditions = task
        .definition
        .dependencies
        .conditions(&task.definition.inputs);
    assert!(
        conditions
            .values()
            .all(|condition| *condition == TaskOrderCondition::Settled)
    );
    let mut definition = task.definition.clone();
    assert!(
        control
            .edit(
                &context()?,
                "fresh-old-shape",
                &id,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![TaskChange::Revise {
                        task: consumer.clone(),
                        expected_version: task.version,
                        definition: definition.clone()
                    }]
                }
            )
            .is_err()
    );
    definition.dependencies = TaskDependencies::Ordered(conditions);
    definition.inputs.clear();
    control.edit(
        &context()?,
        "explicit-order-revision",
        &id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Revise {
                task: consumer.clone(),
                expected_version: task.version,
                definition,
            }],
        },
    )?;
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    let task = record.tasks.get(&consumer).ok_or("consumer absent")?;
    assert_eq!(
        serde_json::to_vec(
            task.previous_definitions
                .first()
                .ok_or("prior definition absent")?
        )?,
        original_bytes
    );
    assert!(
        runtime.tick(4)?.is_empty(),
        "removing optional data did not erase retained independent order"
    );
    Ok(())
}
