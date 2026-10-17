//! Adoption changes future obligations without relabeling or modifying accepted starts.
use super::{InvocationAllowance, TestResult, context, request, runtime};
use milkdrift_blueprint::{
    NewTask, TaskBatch, TaskBindingRedirect, TaskChange, TaskId, TaskInputBinding, TaskInputValue,
    TaskOutputDeclaration,
};
use milkdrift_capability::{
    ArtifactReference, InvocationEventKind, InvocationTerminal, OperationId, SideEffectClass,
    TerminalStatus,
};
use milkdrift_runtime::{DeterministicExecutor, ManualClock, TaskState, WorkflowState};
use std::{collections::BTreeMap, sync::Arc};

fn output(executor: &DeterministicExecutor, media_type: &str, size: u64) -> TestResult {
    let artifact = ArtifactReference::new(
        "corrective-result",
        blake3::hash(b"corrective result").to_hex().to_string(),
        Some(media_type.into()),
        Some(size),
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
                    vec![artifact],
                    None,
                    None,
                    SideEffectClass::None,
                )?,
            },
        ],
    )?;
    Ok(())
}

#[test]
fn incompatible_successful_correction_cannot_satisfy_original_output_contract() -> TestResult {
    for (media_type, bytes) in [("text/markdown", 4), ("text/plain", 9)] {
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
            .ok_or("task absent")?
            .definition
            .outputs
            .insert(
                "report".into(),
                TaskOutputDeclaration {
                    required: true,
                    media_type: Some("text/plain".into()),
                    max_bytes: 8,
                },
            );
        let id = control
            .create(&context()?, "incompatible", create)?
            .workflow;
        let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
        let original = record.tasks.values().next().ok_or("original absent")?;
        let original_id = original.id.clone();
        let mut correction = original.definition.clone();
        correction.inputs.clear();
        correction.corrective_of = Some(original_id.clone());
        correction.outputs.insert(
            "report".into(),
            TaskOutputDeclaration {
                required: true,
                media_type: None,
                max_bytes: 100,
            },
        );
        control.edit(
            &context()?,
            "add_correction",
            &id,
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
        output(&executor, media_type, bytes)?;
        runtime.execute(runtime.tick(1)?.pop().ok_or("correction absent")?)?;
        let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
        let correction = record
            .tasks
            .values()
            .find(|task| task.definition.corrective_of.is_some())
            .ok_or("correction absent")?;
        assert_eq!(correction.state, TaskState::Completed);
        let refused = control.edit(
            &context()?,
            "adopt_incompatible",
            &id,
            TaskBatch {
                expected_generation: record.generation,
                changes: vec![TaskChange::AdoptCorrection {
                    task: original_id.clone(),
                    correction: correction.id.clone(),
                    redirects: Vec::new(),
                }],
            },
        );
        assert!(refused.is_err());
        assert_eq!(runtime.workflow(&id)?.ok_or("workflow absent")?, record);
        assert_eq!(
            record
                .tasks
                .get(&original_id)
                .ok_or("original absent")?
                .state,
            TaskState::Stopped
        );
        assert_ne!(record.state, WorkflowState::Completed);
    }
    Ok(())
}

#[test]
fn corrective_cycle_refuses_the_whole_batch_without_changing_limits() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, _, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let mut create = request()?;
    let mut second = create.tasks.first().ok_or("task absent")?.clone();
    second.alias = TaskId::new("consumer")?;
    second
        .definition
        .dependencies
        .insert(TaskId::new("inspect")?);
    create.tasks.push(second);
    let id = control
        .create(&context()?, "correction_cycle", create)?
        .workflow;
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    let original = record
        .tasks
        .values()
        .find(|task| task.definition.dependencies.is_empty())
        .ok_or("original absent")?;
    let consumer = record
        .tasks
        .values()
        .find(|task| !task.definition.dependencies.is_empty())
        .ok_or("consumer absent")?;
    let mut definition = original.definition.clone();
    definition.corrective_of = Some(consumer.id.clone());
    let mut limits = record.limits.clone();
    limits.tasks = 512;
    assert!(
        control
            .edit(
                &context()?,
                "cyclic_repair",
                &id,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![
                        TaskChange::SetLimits { limits },
                        TaskChange::Revise {
                            task: original.id.clone(),
                            expected_version: original.version,
                            definition
                        }
                    ]
                }
            )
            .is_err()
    );
    assert_eq!(runtime.workflow(&id)?.ok_or("workflow absent")?, record);
    Ok(())
}

#[test]
fn adoption_refuses_started_redirect_and_preserves_its_frozen_absence() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let mut create = request()?;
    let original = create.tasks.first_mut().ok_or("task absent")?;
    original.definition.outputs.insert(
        "report".into(),
        TaskOutputDeclaration {
            required: true,
            media_type: Some("text/plain".into()),
            max_bytes: 100,
        },
    );
    let mut consumer = original.definition.clone();
    consumer.outputs.clear();
    consumer.title = "Already started consumer".into();
    consumer.inputs = BTreeMap::from([(
        "report".into(),
        TaskInputBinding::TaskOutput {
            task: TaskId::new("inspect")?,
            output: "report".into(),
            optional: true,
        },
    )]);
    consumer.dependencies.insert(TaskId::new("inspect")?);
    create.tasks.push(NewTask {
        alias: TaskId::new("consumer")?,
        definition: consumer,
    });
    let id = control
        .create(&context()?, "started_consumer", create)?
        .workflow;
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    let original = record
        .tasks
        .values()
        .find(|task| task.definition.dependencies.is_empty())
        .ok_or("original absent")?;
    let original_id = original.id.clone();
    let mut correction = original.definition.clone();
    correction.inputs.clear();
    correction.corrective_of = Some(original_id.clone());
    control.edit(
        &context()?,
        "prepare_repair",
        &id,
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
    let dispatches = runtime.tick(2)?;
    assert_eq!(dispatches.len(), 2);
    let started = runtime.workflow(&id)?.ok_or("workflow absent")?;
    let consumer = started
        .tasks
        .values()
        .find(|task| task.definition.title == "Already started consumer")
        .ok_or("consumer absent")?;
    let consumer_id = consumer.id.clone();
    let consumer_start = consumer.frozen.clone();
    let correction = started
        .tasks
        .values()
        .find(|task| task.definition.corrective_of.is_some())
        .ok_or("correction absent")?
        .id
        .clone();
    output(&executor, "text/plain", 17)?;
    let mut consumer_dispatch = None;
    for dispatch in dispatches {
        if dispatch.node().as_str() == correction.as_str() {
            runtime.execute(dispatch)?;
        } else {
            consumer_dispatch = Some(dispatch);
        }
    }
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    assert!(
        control
            .edit(
                &context()?,
                "redirect_started",
                &id,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![TaskChange::AdoptCorrection {
                        task: original_id.clone(),
                        correction: correction.clone(),
                        redirects: vec![TaskBindingRedirect {
                            consumer: consumer_id.clone(),
                            expected_version: consumer.version,
                            input: "report".into()
                        }]
                    }]
                }
            )
            .is_err()
    );
    assert_eq!(runtime.workflow(&id)?.ok_or("workflow absent")?, record);
    control.edit(
        &context()?,
        "adopt_future_obligation",
        &id,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::AdoptCorrection {
                task: original_id.clone(),
                correction,
                redirects: Vec::new(),
            }],
        },
    )?;
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    assert_eq!(
        record
            .tasks
            .get(&consumer_id)
            .ok_or("consumer absent")?
            .frozen,
        consumer_start
    );
    assert_eq!(
        consumer_start
            .as_ref()
            .and_then(|start| start.inputs.get("report")),
        Some(&TaskInputValue::Absent)
    );
    assert_eq!(
        record
            .tasks
            .get(&original_id)
            .ok_or("original absent")?
            .state,
        TaskState::Stopped
    );
    assert_eq!(record.state, WorkflowState::Running);
    runtime.execute(consumer_dispatch.ok_or("consumer dispatch absent")?)?;
    assert_eq!(
        runtime.workflow(&id)?.ok_or("workflow absent")?.state,
        WorkflowState::Completed
    );
    Ok(())
}
