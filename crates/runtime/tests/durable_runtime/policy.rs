//! Policy changes and readiness use the public control path and reopen the production store.

use super::{InvocationAllowance, TestResult, context, request, runtime};
use milkdrift_blueprint::{
    NewTask, TaskBatch, TaskChange, TaskId, TaskInputBinding, TaskInputValue,
    TaskOutputDeclaration, WorkflowCreate,
};
use milkdrift_capability::{
    ArtifactReference, ErrorClass, InvocationEventKind, InvocationFailure, InvocationTerminal,
    OperationId, SideEffectClass, TerminalStatus,
};
use milkdrift_runtime::{DeterministicExecutor, ManualClock, TaskState, WorkflowState};
use std::{collections::BTreeMap, sync::Arc};

fn ready_request() -> TestResult<WorkflowCreate> {
    let mut create = request()?;
    create
        .tasks
        .first_mut()
        .ok_or("task absent")?
        .definition
        .inputs
        .clear();
    create.finish_when_settled = false;
    Ok(create)
}

fn fail(executor: &DeterministicExecutor) -> TestResult {
    executor.set_script(
        OperationId::new("model.generate")?,
        vec![InvocationEventKind::Terminal {
            terminal: failure(Vec::new())?,
        }],
    )?;
    Ok(())
}

fn failure(outputs: Vec<ArtifactReference>) -> TestResult<InvocationTerminal> {
    Ok(InvocationTerminal::new(
        TerminalStatus::Failure,
        outputs,
        Some(InvocationFailure::new(
            ErrorClass::Provider,
            false,
            "research_failed",
            "The selected research operation failed.",
            None,
        )?),
        None,
        SideEffectClass::None,
    )?)
}

#[test]
fn corrective_lineage_stop_survives_restart_and_resume_never_refunds_failures() -> TestResult {
    let root = tempfile::tempdir()?;
    let clock = Arc::new(ManualClock::new(1_000));
    let authority = Arc::new(InvocationAllowance(Some(8)));
    let workflow;
    {
        let (runtime, executor, control) = runtime(root.path(), authority.clone(), clock.clone())?;
        let mut create = ready_request()?;
        create.limits.corrective_failures = 2;
        create.limits.concurrent_tasks = 1;
        let mut parent = TaskId::new("inspect")?;
        for alias in ["first_correction", "second_correction"] {
            let mut definition = create
                .tasks
                .first()
                .ok_or("task absent")?
                .definition
                .clone();
            definition.corrective_of = Some(parent);
            parent = TaskId::new(alias)?;
            create.tasks.push(NewTask {
                alias: parent.clone(),
                definition,
            });
        }
        workflow = control.create(&context()?, "lineage", create)?.workflow;
        fail(&executor)?;
        for _ in 0..2 {
            runtime.execute(runtime.tick(1)?.pop().ok_or("corrective dispatch absent")?)?;
        }
        let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
        assert!(
            record
                .wait_question
                .as_deref()
                .is_some_and(|reason| reason.contains("2 failed tasks"))
        );
        assert!(runtime.tick(1)?.is_empty());
        assert_eq!(executor.entry_count(), 2);
        assert!(
            control
                .edit(
                    &context()?,
                    "resume_without_limits",
                    &workflow,
                    TaskBatch {
                        expected_generation: record.generation,
                        changes: vec![TaskChange::Continue],
                    }
                )
                .is_err()
        );
        assert_eq!(
            runtime.workflow(&workflow)?.ok_or("workflow absent")?,
            record
        );
    }
    let (runtime, executor, control) = runtime(root.path(), authority, clock)?;
    runtime.recover()?;
    assert!(runtime.tick(1)?.is_empty());
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    let mut limits = record.limits.clone();
    limits.corrective_failures = 3;
    control.edit(
        &context()?,
        "prospective_increase",
        &workflow,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::SetLimits { limits }, TaskChange::Continue],
        },
    )?;
    let resumed = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    assert_eq!(resumed.committed_allowance, record.committed_allowance);
    assert_eq!(resumed.allowance, record.allowance);
    assert_eq!(resumed.admitted_tasks, 3);
    fail(&executor)?;
    runtime.execute(runtime.tick(1)?.pop().ok_or("resumed correction absent")?)?;
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    assert_eq!(record.committed_allowance.invocations, Some(3));
    assert!(
        record
            .wait_question
            .as_deref()
            .is_some_and(|reason| reason.contains("3 failed tasks"))
    );
    assert!(
        record
            .tasks
            .values()
            .all(|task| task.state == TaskState::Failed)
    );
    Ok(())
}

#[test]
fn independent_optional_failures_do_not_share_a_corrective_lineage() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let mut create = ready_request()?;
    create
        .tasks
        .first_mut()
        .ok_or("task absent")?
        .definition
        .required = false;
    for alias in ["other", "third"] {
        let definition = create
            .tasks
            .first()
            .ok_or("task absent")?
            .definition
            .clone();
        create.tasks.push(NewTask {
            alias: TaskId::new(alias)?,
            definition,
        });
    }
    create.limits.corrective_failures = 2;
    create.finish_when_settled = true;
    let workflow = control
        .create(&context()?, "separate_lineages", create)?
        .workflow;
    fail(&executor)?;
    for dispatch in runtime.tick(3)? {
        runtime.execute(dispatch)?;
    }
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    assert_eq!(record.state, WorkflowState::Completed);
    assert_eq!(record.committed_allowance.invocations, Some(3));
    assert!(record.wait_question.is_none());
    Ok(())
}

#[test]
fn settled_optional_failure_does_not_pause_finish_at_its_lineage_limit() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let mut create = ready_request()?;
    create
        .tasks
        .first_mut()
        .ok_or("task absent")?
        .definition
        .required = false;
    create.limits.corrective_failures = 1;
    create.finish_when_settled = true;
    let id = control
        .create(&context()?, "optional_failure_finished", create)?
        .workflow;
    fail(&executor)?;
    runtime.execute(runtime.tick(1)?.pop().ok_or("task absent")?)?;
    let record = runtime.workflow(&id)?.ok_or("workflow absent")?;
    assert_eq!(record.state, WorkflowState::Completed);
    assert!(record.wait_question.is_none());
    assert_eq!(
        record.tasks.values().next().ok_or("task absent")?.state,
        TaskState::Failed
    );
    Ok(())
}

#[test]
fn limit_edits_keep_removed_task_spending_and_exact_replay() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, _, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let workflow = control
        .create(&context()?, "limits", ready_request()?)?
        .workflow;
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    let task = record.tasks.values().next().ok_or("task absent")?;
    let definition = task.definition.clone();
    control.edit(
        &context()?,
        "remove",
        &workflow,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Remove {
                task: task.id.clone(),
                expected_version: task.version,
            }],
        },
    )?;
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    let mut limits = record.limits.clone();
    limits.tasks = 1;
    limits.concurrent_tasks = 1;
    let change = TaskBatch {
        expected_generation: record.generation,
        changes: vec![TaskChange::SetLimits {
            limits: limits.clone(),
        }],
    };
    let accepted = control.edit(&context()?, "tighten", &workflow, change.clone())?;
    assert_eq!(
        control.edit(&context()?, "tighten", &workflow, change)?,
        accepted
    );
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    assert_eq!(record.admitted_tasks, 1);
    assert!(
        control
            .edit(
                &context()?,
                "no_refund",
                &workflow,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![TaskChange::Add {
                        alias: TaskId::new("new")?,
                        definition: definition.clone()
                    }],
                }
            )
            .is_err()
    );
    limits.tasks = 2;
    control.edit(
        &context()?,
        "increase_then_add",
        &workflow,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![
                TaskChange::SetLimits { limits },
                TaskChange::Add {
                    alias: TaskId::new("new")?,
                    definition,
                },
            ],
        },
    )?;
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    let mut below_spent = record.limits.clone();
    below_spent.tasks = 1;
    assert!(
        control
            .edit(
                &context()?,
                "below_spent",
                &workflow,
                TaskBatch {
                    expected_generation: record.generation,
                    changes: vec![TaskChange::SetLimits {
                        limits: below_spent
                    }],
                }
            )
            .is_err()
    );
    assert_eq!(
        runtime.workflow(&workflow)?.ok_or("workflow absent")?,
        record
    );
    Ok(())
}

#[test]
fn explicit_wait_prevents_finish_until_authorized_continuation() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, _, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let workflow = control
        .create(&context()?, "wait_finish", ready_request()?)?
        .workflow;
    runtime.execute(runtime.tick(1)?.pop().ok_or("task absent")?)?;
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    assert_eq!(record.state, WorkflowState::Waiting);
    control.edit(
        &context()?,
        "decide",
        &workflow,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![
                TaskChange::Wait {
                    question: "Approve this result before completing?".into(),
                },
                TaskChange::Finish,
            ],
        },
    )?;
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    assert_eq!(record.state, WorkflowState::Waiting);
    control.edit(
        &context()?,
        "answer",
        &workflow,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Continue],
        },
    )?;
    assert_eq!(
        runtime.workflow(&workflow)?.ok_or("workflow absent")?.state,
        WorkflowState::Completed
    );
    Ok(())
}

#[test]
fn settled_outcome_does_not_waive_sibling_required_success_input() -> TestResult {
    let root = tempfile::tempdir()?;
    let (runtime, executor, control) = runtime(
        root.path(),
        Arc::new(InvocationAllowance(Some(8))),
        Arc::new(ManualClock::new(1_000)),
    )?;
    let mut create = ready_request()?;
    let producer = create.tasks.first_mut().ok_or("task absent")?;
    producer.definition.required = false;
    producer.definition.outputs.insert(
        "partial".into(),
        TaskOutputDeclaration {
            required: false,
            media_type: Some("text/plain".into()),
            max_bytes: 100,
        },
    );
    let mut consumer = producer.definition.clone();
    consumer.title = "Needs successful output and settled outcome".into();
    consumer.outputs.clear();
    consumer.required = true;
    consumer.dependencies.insert(TaskId::new("inspect")?);
    consumer.inputs = BTreeMap::from([
        (
            "required".into(),
            TaskInputBinding::TaskOutput {
                task: TaskId::new("inspect")?,
                output: "partial".into(),
                optional: false,
            },
        ),
        (
            "outcome".into(),
            TaskInputBinding::SettledOutcome {
                task: TaskId::new("inspect")?,
            },
        ),
    ]);
    create.tasks.push(NewTask {
        alias: TaskId::new("consumer")?,
        definition: consumer,
    });
    let workflow = control
        .create(&context()?, "mixed_input_policy", create)?
        .workflow;
    let artifact = ArtifactReference::new(
        "partial-result",
        blake3::hash(b"partial").to_hex().to_string(),
        Some("text/plain".into()),
        Some(7),
    )?;
    executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "partial".into(),
                reference: artifact,
            },
            InvocationEventKind::Terminal {
                terminal: failure(Vec::new())?,
            },
        ],
    )?;
    runtime.execute(runtime.tick(2)?.pop().ok_or("producer absent")?)?;
    assert!(runtime.tick(2)?.is_empty());
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    let consumer = record
        .tasks
        .values()
        .find(|task| task.definition.required)
        .ok_or("consumer absent")?;
    assert_eq!(consumer.state, TaskState::Queued);
    assert!(consumer.frozen.is_none());
    assert!(
        consumer
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("needs a successful producer"))
    );
    assert_eq!(executor.entry_count(), 1);
    assert!(
        record
            .tasks
            .values()
            .find(|task| !task.definition.required)
            .and_then(|task| task.result.as_ref())
            .and_then(|result| result.outputs.get("partial"))
            .is_some()
    );
    // A human can explicitly opt this future binding into the producer's partial result.
    let mut definition = consumer.definition.clone();
    if let Some(TaskInputBinding::TaskOutput { optional, .. }) =
        definition.inputs.get_mut("required")
    {
        *optional = true;
    }
    control.edit(
        &context()?,
        "accept_partial",
        &workflow,
        TaskBatch {
            expected_generation: record.generation,
            changes: vec![TaskChange::Revise {
                task: consumer.id.clone(),
                expected_version: consumer.version,
                definition,
            }],
        },
    )?;
    let dispatch = runtime.tick(2)?.pop().ok_or("optional consumer absent")?;
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    assert!(matches!(
        record
            .tasks
            .get(&consumer.id)
            .and_then(|task| task.frozen.as_ref())
            .and_then(|start| start.inputs.get("required")),
        Some(TaskInputValue::Artifact { .. })
    ));
    runtime.execute(dispatch)?;
    Ok(())
}
