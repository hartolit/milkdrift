use super::{RevocableAuthority, TaskStoreClock, TestResult, context, request};
use milkdrift_blueprint::{TaskActionPolicy, TaskBatch, TaskChange, TaskId, TaskInputValue};
use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, CapabilityDescriptorDocument,
    CapabilityRequirement, InvocationAdmissionEnvelope, InvocationEvent, InvocationEventKind,
    InvocationTerminal, SideEffectClass, TerminalStatus,
};
use milkdrift_control::TaskControlService;
use milkdrift_redb_store::{RedbStore, RedbStoreConfig};
use milkdrift_runtime::{
    DeterministicExecutor, ExecutionDispatch, ExecutorError, ManualClock, ModelAdmissionStatus,
    PreparedExecution, ResolvedCapability, SequentialIdGenerator, TaskExecutor, TaskRuntime,
    WorkflowState,
};
use std::sync::{Arc, atomic::AtomicBool};

/// This controlled boundary is a model capability with no remote resource consumption.
/// Production provider accounting remains covered by the HTTP and control admission suites.
struct ModelExecutor(DeterministicExecutor);

impl TaskExecutor for ModelExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        now: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.0.resolve(requirement, now)
    }

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(PreparedExecution::new(
            dispatch,
            InvocationAdmissionEnvelope::not_applicable(),
            |dispatch, reporter| {
                reporter.invocation(InvocationEvent::new(
                    dispatch.request().invocation().clone(),
                    1,
                    InvocationEventKind::Terminal {
                        terminal: InvocationTerminal::new(
                            TerminalStatus::Success,
                            Vec::new(),
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
        self.0.cancel(request)
    }
}

fn runtime(
    root: &std::path::Path,
    authority: Arc<RevocableAuthority>,
    clock: Arc<ManualClock>,
) -> TestResult<(Arc<TaskRuntime>, Arc<ModelExecutor>, TaskControlService)> {
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    let executor = Arc::new(ModelExecutor(DeterministicExecutor::new(
        descriptor.body().clone(),
    )));
    let store = Arc::new(RedbStore::open_with_config(
        RedbStoreConfig::new(root).with_clock(Arc::new(TaskStoreClock(clock.clone()))),
    )?);
    let runtime = Arc::new(TaskRuntime::new(
        store.clone(),
        executor.clone(),
        authority.clone(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("model-action-tests", 1)?),
    ));
    let control = TaskControlService::new(runtime.clone(), authority, clock, store);
    Ok((runtime, executor, control))
}

#[test]
fn saved_response_admission_and_receipt_survive_restart_without_duplicate_children() -> TestResult {
    let root = tempfile::tempdir()?;
    let clock = Arc::new(ManualClock::new(1000));
    let authority = Arc::new(RevocableAuthority(AtomicBool::new(false)));
    let (engine, executor, control) = runtime(root.path(), authority.clone(), clock.clone())?;
    let mut create = request()?;
    let source_definition = &mut create.tasks.first_mut().ok_or("source absent")?.definition;
    source_definition.actions = Some(TaskActionPolicy {
        automatic_repair: false,
    });
    create.inputs.insert(
        "manifest".into(),
        TaskInputValue::Text {
            text: "brief".into(),
        },
    );
    let receipt = control.create(&context()?, "model-create", create)?;
    let dispatch = engine.tick(4)?.pop().ok_or("source not ready")?;
    engine.execute(dispatch)?;
    let before = engine
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let source = before.tasks.keys().next().ok_or("task absent")?.clone();
    assert_eq!(
        before
            .admissions
            .get(&source)
            .ok_or("pending response missing")?
            .status,
        ModelAdmissionStatus::Pending
    );
    assert_ne!(before.state, WorkflowState::Completed);
    let mut child = before
        .tasks
        .get(&source)
        .ok_or("source absent")?
        .definition
        .clone();
    child.actions = None;
    child.title = "Child authored by saved response".into();
    let batch = TaskBatch {
        expected_generation: before
            .admissions
            .get(&source)
            .ok_or("admission absent")?
            .expected_generation,
        changes: vec![TaskChange::Add {
            alias: TaskId::new("child")?,
            definition: child,
        }],
    };
    drop(control);
    drop(engine);
    drop(executor);
    let (engine, executor, control) = runtime(root.path(), authority.clone(), clock.clone())?;
    let accepted = engine.admit_model_batch(
        context()?.actor().as_str(),
        &receipt.workflow,
        &source,
        batch.clone(),
        false,
    )?;
    let after = engine
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(after.tasks.len(), 2);
    assert_eq!(after.admitted_model_batches, 1);
    assert_eq!(
        after
            .admissions
            .get(&source)
            .ok_or("admission absent")?
            .status,
        ModelAdmissionStatus::Accepted
    );
    drop(control);
    drop(engine);
    drop(executor);
    let (engine, _, _) = runtime(root.path(), authority, clock)?;
    let replay = engine.admit_model_batch(
        context()?.actor().as_str(),
        &receipt.workflow,
        &source,
        batch,
        false,
    )?;
    assert_eq!(accepted, replay);
    let after = engine
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(after.tasks.len(), 2);
    assert_eq!(after.admitted_model_batches, 1);
    assert_eq!(engine.tick(4)?.len(), 1);
    Ok(())
}

#[test]
fn human_edit_conflicts_with_saved_model_snapshot_and_rejection_is_counted_once() -> TestResult {
    let root = tempfile::tempdir()?;
    let (engine, _, control) = runtime(
        root.path(),
        Arc::new(RevocableAuthority(AtomicBool::new(false))),
        Arc::new(ManualClock::new(1000)),
    )?;
    let mut create = request()?;
    create
        .tasks
        .first_mut()
        .ok_or("source absent")?
        .definition
        .actions = Some(TaskActionPolicy {
        automatic_repair: true,
    });
    create.inputs.insert(
        "manifest".into(),
        TaskInputValue::Text {
            text: "brief".into(),
        },
    );
    let receipt = control.create(&context()?, "model-create", create)?;
    engine.execute(engine.tick(4)?.pop().ok_or("source not ready")?)?;
    let before = engine
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    let source = before.tasks.keys().next().ok_or("task absent")?.clone();
    let proposal = TaskBatch {
        expected_generation: before
            .admissions
            .get(&source)
            .ok_or("admission absent")?
            .expected_generation,
        changes: vec![TaskChange::Wait {
            question: "model question".into(),
        }],
    };
    control.edit(
        &context()?,
        "human-edit",
        &receipt.workflow,
        TaskBatch {
            expected_generation: before.generation,
            changes: vec![TaskChange::BindInput {
                name: "manifest".into(),
                value: TaskInputValue::Text {
                    text: "human changed intent".into(),
                },
            }],
        },
    )?;
    assert!(
        engine
            .admit_model_batch(
                context()?.actor().as_str(),
                &receipt.workflow,
                &source,
                proposal,
                false
            )
            .is_err()
    );
    engine.reject_model_batch(&receipt.workflow, &source, "snapshot conflict", true)?;
    engine.reject_model_batch(&receipt.workflow, &source, "duplicate diagnostic", true)?;
    let after = engine
        .workflow(&receipt.workflow)?
        .ok_or("workflow absent")?;
    assert_eq!(after.admitted_model_batches, 0);
    assert_eq!(after.rejected_responses, 1);
    assert_eq!(after.tasks.len(), 1);
    assert_eq!(
        after
            .admissions
            .get(&source)
            .ok_or("admission absent")?
            .reason
            .as_deref(),
        Some("snapshot conflict")
    );
    assert!(after.wait_question.is_some());
    Ok(())
}

#[test]
fn no_progress_and_rejected_feedback_share_a_restart_stable_stop_allowance() -> TestResult {
    let root = tempfile::tempdir()?;
    let authority = Arc::new(RevocableAuthority(AtomicBool::new(false)));
    let clock = Arc::new(ManualClock::new(1000));
    let (workflow, rejected_source);
    {
        let (engine, _, control) = runtime(root.path(), authority.clone(), clock.clone())?;
        let mut create = request()?;
        create.finish_when_settled = false;
        create.limits.rejected_responses = 2;
        let definition = &mut create.tasks.first_mut().ok_or("source absent")?.definition;
        definition.inputs.clear();
        definition.actions = Some(TaskActionPolicy {
            automatic_repair: true,
        });
        let second = definition.clone();
        create.tasks.push(milkdrift_blueprint::NewTask {
            alias: TaskId::new("second")?,
            definition: second,
        });
        workflow = control.create(&context()?, "no-progress", create)?.workflow;
        let first = engine.tick(1)?.pop().ok_or("first source absent")?;
        let source = TaskId::new(first.node().as_str())?;
        engine.execute(first)?;
        let record = engine.workflow(&workflow)?.ok_or("workflow absent")?;
        engine.admit_model_batch(
            context()?.actor().as_str(),
            &workflow,
            &source,
            TaskBatch {
                expected_generation: record
                    .admissions
                    .get(&source)
                    .ok_or("source admission absent")?
                    .expected_generation,
                changes: vec![TaskChange::BindInput {
                    name: "nonprogress".into(),
                    value: TaskInputValue::Text {
                        text: "no new work or decision".into(),
                    },
                }],
            },
            false,
        )?;
        let second = engine.tick(1)?.pop().ok_or("second source absent")?;
        rejected_source = TaskId::new(second.node().as_str())?;
        engine.execute(second)?;
        engine.reject_model_batch(&workflow, &rejected_source, "invalid second response", true)?;
        let record = engine.workflow(&workflow)?.ok_or("workflow absent")?;
        assert_eq!(record.rejected_responses, 2);
        assert!(record.wait_question.is_some());
        assert!(engine.tick(1)?.is_empty());
    }
    let (engine, _, control) = runtime(root.path(), authority, clock)?;
    assert!(engine.recover()?.is_empty());
    let stopped = engine.workflow(&workflow)?.ok_or("workflow absent")?;
    assert_eq!(stopped.rejected_responses, 2);
    let mut repair = stopped
        .tasks
        .get(&rejected_source)
        .ok_or("source absent")?
        .definition
        .clone();
    repair.required = false;
    assert!(
        engine
            .admit_model_batch(
                context()?.actor().as_str(),
                &workflow,
                &rejected_source,
                TaskBatch {
                    expected_generation: stopped.generation,
                    changes: vec![TaskChange::Add {
                        alias: TaskId::new("feedback")?,
                        definition: repair
                    }],
                },
                true
            )
            .is_err()
    );
    assert!(
        control
            .edit(
                &context()?,
                "premature-continue",
                &workflow,
                TaskBatch {
                    expected_generation: stopped.generation,
                    changes: vec![TaskChange::Continue],
                }
            )
            .is_err()
    );
    assert_eq!(
        engine.workflow(&workflow)?.ok_or("workflow absent")?,
        stopped
    );
    let mut limits = stopped.limits.clone();
    limits.rejected_responses = 3;
    control.edit(
        &context()?,
        "raise-no-progress",
        &workflow,
        TaskBatch {
            expected_generation: stopped.generation,
            changes: vec![TaskChange::SetLimits { limits }, TaskChange::Continue],
        },
    )?;
    let resumed = engine.workflow(&workflow)?.ok_or("workflow absent")?;
    assert_eq!(resumed.rejected_responses, 2);
    assert_eq!(resumed.admitted_model_batches, 1);
    assert_eq!(resumed.committed_allowance, stopped.committed_allowance);
    assert_eq!(resumed.allowance, stopped.allowance);
    assert!(resumed.wait_question.is_none());
    assert_eq!(
        resumed
            .admissions
            .get(&rejected_source)
            .ok_or("rejection absent")?
            .status,
        ModelAdmissionStatus::Rejected
    );
    assert_ne!(resumed.state, WorkflowState::Completed);
    Ok(())
}

#[test]
fn model_batch_ceiling_refuses_whole_next_batch_after_reopen_but_keeps_exact_replay() -> TestResult
{
    let root = tempfile::tempdir()?;
    let authority = Arc::new(RevocableAuthority(AtomicBool::new(false)));
    let clock = Arc::new(ManualClock::new(1000));
    let (workflow, source, first_batch, receipt, next_source, refused_batch);
    {
        let (engine, _, control) = runtime(root.path(), authority.clone(), clock.clone())?;
        let mut create = request()?;
        create.finish_when_settled = false;
        create.limits.model_batches = 1;
        let definition = &mut create.tasks.first_mut().ok_or("source absent")?.definition;
        definition.inputs.clear();
        definition.actions = Some(TaskActionPolicy {
            automatic_repair: false,
        });
        workflow = control
            .create(&context()?, "model-ceiling", create)?
            .workflow;
        let first = engine.tick(1)?.pop().ok_or("source absent")?;
        source = TaskId::new(first.node().as_str())?;
        engine.execute(first)?;
        let record = engine.workflow(&workflow)?.ok_or("workflow absent")?;
        first_batch = TaskBatch {
            expected_generation: record
                .admissions
                .get(&source)
                .ok_or("admission absent")?
                .expected_generation,
            changes: vec![TaskChange::Add {
                alias: TaskId::new("child")?,
                definition: record
                    .tasks
                    .get(&source)
                    .ok_or("source absent")?
                    .definition
                    .clone(),
            }],
        };
        receipt = engine.admit_model_batch(
            context()?.actor().as_str(),
            &workflow,
            &source,
            first_batch.clone(),
            false,
        )?;
        let child = engine
            .tick(1)?
            .pop()
            .ok_or("already admitted child must still execute at exact cap")?;
        next_source = TaskId::new(child.node().as_str())?;
        engine.execute(child)?;
        let record = engine.workflow(&workflow)?.ok_or("workflow absent")?;
        let mut definition = record
            .tasks
            .get(&next_source)
            .ok_or("child absent")?
            .definition
            .clone();
        definition.actions = None;
        refused_batch = TaskBatch {
            expected_generation: record
                .admissions
                .get(&next_source)
                .ok_or("next admission absent")?
                .expected_generation,
            changes: vec![
                TaskChange::BindInput {
                    name: "partial-must-not-commit".into(),
                    value: TaskInputValue::Text {
                        text: "forbidden at cap".into(),
                    },
                },
                TaskChange::Add {
                    alias: TaskId::new("grandchild")?,
                    definition,
                },
            ],
        };
    }
    let (engine, _, _) = runtime(root.path(), authority, clock)?;
    let before = engine.workflow(&workflow)?.ok_or("workflow absent")?;
    assert_eq!(before.admitted_model_batches, 1);
    assert!(
        engine
            .admit_model_batch(
                context()?.actor().as_str(),
                &workflow,
                &next_source,
                refused_batch,
                false
            )
            .is_err()
    );
    assert_eq!(
        engine.workflow(&workflow)?.ok_or("workflow absent")?,
        before
    );
    assert_eq!(before.tasks.len(), 2);
    assert!(!before.inputs.contains_key("partial-must-not-commit"));
    assert_eq!(
        engine.admit_model_batch(
            context()?.actor().as_str(),
            &workflow,
            &source,
            first_batch,
            false
        )?,
        receipt
    );
    assert_eq!(
        engine
            .workflow(&workflow)?
            .ok_or("workflow absent")?
            .admitted_model_batches,
        1
    );
    Ok(())
}
