use super::*;

#[test]
fn deleted_optional_supplied_input_is_corruption_not_absence() -> TestResult {
    let harness = Harness::new("deleted-optional-input")?;
    let revision = optional_workflow_input_revision("workflow-deleted-optional-input")?;
    let run = RunId::new("run-deleted-optional-input")?;
    harness.put_revision(&revision)?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-deleted-optional-input")?);
    let input = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("optional")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"supplied": true}))?),
    );
    let input_reference = input.reference().clone();
    assert_eq!(
        harness.command(
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: root,
                workspace_budget: generous_budget()?,
                inputs: vec![input],
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(&run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    storage_fault::remove_workspace_value(directory.path(), &input_reference)?;

    let (store, _clock, executor, runtime) =
        open_closed_runtime_at(directory.path(), "deleted-optional-input-reopen", NOW, 64)?;
    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());
    let history = runtime.history(&run)?;
    let Err(error) = runtime.initialize_startup() else {
        return Err("startup treated a deleted supplied optional input as absent".into());
    };
    assert_integrity_error(&error);
    assert!(error.to_string().contains(run.as_str()));
    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());
    assert_eq!(executor.entry_count(), 0);
    assert_eq!(store.head(&run)?, head);
    assert_eq!(runtime.history(&run)?, history);
    let consume = NodeId::new("consume")?;
    assert!(!history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeScheduled { node, .. } if node == &consume
    )));
    Ok(())
}

#[test]
fn orphan_latest_optional_input_is_rejected_against_the_projection() -> TestResult {
    let harness = Harness::new("orphan-optional-input")?;
    let revision = optional_workflow_input_revision("workflow-orphan-optional-input")?;
    let run = RunId::new("run-orphan-optional-input")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    let root = harness
        .runtime
        .projection(&run)?
        .root_scope()
        .ok_or("run root scope was not projected")?
        .reference()
        .clone();
    let orphan = WorkspaceValueEntry::initial(
        root,
        ValueKey::new("optional")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"orphan": true}))?),
    );
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    storage_fault::insert_orphan_workspace_value(directory.path(), &orphan)?;

    let (store, _clock, runtime) =
        runtime_at(directory.path(), "orphan-optional-input-reopen", NOW, 64)?;
    let Err(error) = runtime_tick(&runtime) else {
        return Err("scheduler accepted an unprojected durable latest input".into());
    };
    assert_integrity_error(&error);
    assert_eq!(store.head(&run)?, head);
    Ok(())
}

#[test]
fn deleted_required_producer_output_cannot_be_scheduled_as_an_invocation_input() -> TestResult {
    let harness = Harness::new("deleted-producer-output")?;
    let revision = producer_consumer_revision("workflow-deleted-producer-output")?;
    let run = RunId::new("run-deleted-producer-output")?;
    let output = publish_artifact(&harness, "deleted-producer-output", b"producer-output")?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "result".to_owned(),
                reference: output,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(runtime_tick(&harness.runtime)?.dispatched, 1);
    let projection = harness.runtime.projection(&run)?;
    let output_reference = projection
        .executions_for_node(&NodeId::new("produce")?)
        .flat_map(|execution| execution.outputs())
        .map(|output| output.value().clone())
        .next()
        .ok_or("producer output was not projected")?;
    assert_eq!(
        projection
            .executions_for_node(&NodeId::new("consume")?)
            .next()
            .map(|execution| execution.state()),
        Some(&NodeExecutionState::Eligible)
    );
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    storage_fault::remove_workspace_value(directory.path(), &output_reference)?;

    let (store, _clock, executor, runtime) =
        open_closed_runtime_at(directory.path(), "deleted-producer-output-reopen", NOW, 64)?;
    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());
    let history = runtime.history(&run)?;
    let Err(error) = runtime.initialize_startup() else {
        return Err("startup accepted an active run with a deleted producer output".into());
    };
    assert_integrity_error(&error);
    assert!(error.to_string().contains(run.as_str()));
    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());
    assert_eq!(executor.entry_count(), 0);
    assert_eq!(store.head(&run)?, head);
    assert_eq!(runtime.history(&run)?, history);
    let consume = NodeId::new("consume")?;
    assert!(!history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeScheduled { node, .. } if node == &consume
    )));
    Ok(())
}

#[test]
fn deleted_root_scope_blocks_even_an_inputless_invocation() -> TestResult {
    let harness = Harness::new("deleted-root-scope")?;
    let revision = task_revision("workflow-deleted-root-scope")?;
    let run = RunId::new("run-deleted-root-scope")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    let root = harness
        .runtime
        .projection(&run)?
        .root_scope()
        .ok_or("run root scope was not projected")?
        .reference()
        .clone();
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    storage_fault::remove_workspace_scope(directory.path(), &root)?;

    let (store, _clock, executor, runtime) =
        open_closed_runtime_at(directory.path(), "deleted-root-scope-reopen", NOW, 64)?;
    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());
    let history = runtime.history(&run)?;
    let Err(error) = runtime.initialize_startup() else {
        return Err("startup accepted an active run whose root scope was deleted".into());
    };
    assert_integrity_error(&error);
    assert!(error.to_string().contains(run.as_str()));
    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());
    assert_eq!(executor.entry_count(), 0);
    assert_eq!(store.head(&run)?, head);
    assert_eq!(runtime.history(&run)?, history);
    assert!(
        !history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeScheduled { .. }))
    );
    Ok(())
}

#[test]
fn deleted_branch_scope_blocks_its_inputless_child_invocation() -> TestResult {
    let harness = Harness::new("deleted-branch-scope")?;
    let revision = fork_revision(
        "workflow-deleted-branch-scope",
        JoinPolicy::All,
        "model.fail",
    )?;
    let run = RunId::new("run-deleted-branch-scope")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    let projection = harness.runtime.projection(&run)?;
    let mut scopes = Vec::new();
    for node in [NodeId::new("a-task")?, NodeId::new("b-task")?] {
        let scope = projection
            .executions_for_node(&node)
            .next()
            .ok_or("fork child task was not made eligible")?
            .scope()
            .clone();
        assert!(matches!(
            projection.scopes().get(&scope).map(WorkspaceScope::kind),
            Some(ScopeKind::Branch { .. })
        ));
        scopes.push(scope);
    }
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    for scope in &scopes {
        storage_fault::remove_workspace_scope(directory.path(), scope)?;
    }

    let (store, _clock, executor, runtime) =
        open_closed_runtime_at(directory.path(), "deleted-branch-scope-reopen", NOW, 64)?;
    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());
    let history = runtime.history(&run)?;
    let Err(error) = runtime.initialize_startup() else {
        return Err("startup accepted an active run whose branch scope was deleted".into());
    };
    assert_integrity_error(&error);
    assert!(error.to_string().contains(run.as_str()));
    assert_eq!(runtime.startup_state(), RuntimeStartupState::OpenedClosed);
    assert!(!runtime.is_accepting_admission());
    assert_eq!(executor.entry_count(), 0);
    assert_eq!(store.head(&run)?, head);
    assert_eq!(runtime.history(&run)?, history);
    assert!(
        !history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeScheduled { .. }))
    );
    Ok(())
}

#[test]
fn orphan_latest_value_cannot_become_a_worker_output_predecessor() -> TestResult {
    let directory = TempDir::new()?;
    let identity = "orphan-worker-output";
    let (store, clock, runtime) = runtime_with_executor_at(
        directory.path(),
        identity,
        identity,
        NOW,
        64,
        Arc::new(PanickingExecutor {
            resolver: DeterministicExecutor::new(test_descriptor()?),
        }),
    )?;
    let revision = output_child_revision("workflow-orphan-worker-output")?;
    let run = RunId::new("run-orphan-worker-output")?;
    store.put_revision(&revision)?;
    let artifact = publish_artifact_in_store(
        store.as_ref(),
        &RunId::new("artifact-owner-orphan-worker-output")?,
        "orphan-worker-output",
        b"worker-output",
    )?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-orphan-worker-output")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime_tick(&runtime)));
    assert!(
        crash.is_err(),
        "executor did not stop after durable dispatch"
    );
    let projection = runtime.projection(&run)?;
    let attempt_view = projection
        .attempts()
        .values()
        .next()
        .ok_or("durably scheduled attempt is absent")?;
    let attempt = attempt_view.attempt().clone();
    let invocation = attempt_view
        .invocation()
        .ok_or("durably scheduled invocation is absent")?
        .clone();
    let scope = projection
        .node_executions()
        .get(attempt_view.execution())
        .ok_or("durably scheduled execution is absent")?
        .scope()
        .clone();
    let orphan = WorkspaceValueEntry::initial(
        scope,
        ValueKey::new("result")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"orphan": true}))?),
    );
    drop(projection);
    drop(runtime);
    drop(clock);
    drop(store);
    storage_fault::insert_orphan_workspace_value(directory.path(), &orphan)?;

    let (store, _clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "orphan-worker-output-reopen",
        identity,
        NOW,
        64,
        Arc::new(DeterministicExecutor::new(test_descriptor()?)),
    )?;
    let head = store.head(&run)?;
    match submit_worker_report(
        &runtime,
        store.as_ref(),
        &run,
        identity,
        WorkerReport::Started {
            attempt: attempt.clone(),
        },
    ) {
        Ok(disposition) => assert_eq!(disposition, CommandDisposition::Accepted),
        Err(error) => {
            let runtime_error = error
                .downcast_ref::<RuntimeError>()
                .ok_or("unexpected non-runtime corruption error")?;
            assert!(
                matches!(runtime_error, RuntimeError::InvalidCommand(detail) if detail.contains("worker reports cannot be submitted"))
            );
            assert_eq!(store.head(&run)?, head);
            assert!(
                !runtime
                    .history(&run)?
                    .iter()
                    .any(|event| matches!(event.kind(), RunEventKind::NodeOutputPublished { .. }))
            );
            return Ok(());
        }
    }
    let head = store.head(&run)?;
    let output = InvocationEvent::new(
        invocation,
        1,
        InvocationEventKind::Output {
            name: "result".to_owned(),
            reference: artifact,
        },
    )?;
    let Err(error) = submit_worker_report(
        &runtime,
        store.as_ref(),
        &run,
        identity,
        WorkerReport::Invocation {
            attempt,
            report: output,
        },
    ) else {
        return Err("worker output accepted an orphan durable predecessor".into());
    };
    let message = error.to_string();
    assert!(
        message.contains("orphan latest value")
            || message.contains("workspace values disagree with global value accounting")
            || message.contains("immutable workspace_value conflict")
            || message.contains("Corruption"),
        "unexpected corruption error: {message}"
    );
    assert_eq!(store.head(&run)?, head);
    assert!(
        !runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeOutputPublished { .. }))
    );
    assert!(store.value(orphan.reference()).is_err());
    Ok(())
}

#[test]
fn orphan_latest_value_cannot_version_a_deterministic_terminal_output() -> TestResult {
    let harness = Harness::new("orphan-terminal-output")?;
    let revision = terminal_binding_revision("workflow-orphan-terminal-output")?;
    let run = RunId::new("run-orphan-terminal-output")?;
    harness.put_revision(&revision)?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-orphan-terminal-output")?);
    let input = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("source")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"source": true}))?),
    );
    assert_eq!(
        harness.command(
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: root.clone(),
                workspace_budget: generous_budget()?,
                inputs: vec![input],
            },
        )?,
        CommandDisposition::Accepted
    );
    let orphan = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("pass")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"orphan": true}))?),
    );
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    storage_fault::insert_orphan_workspace_value(directory.path(), &orphan)?;

    let (store, _clock, runtime) =
        runtime_at(directory.path(), "orphan-terminal-output-reopen", NOW, 64)?;
    let Err(error) = submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun) else {
        return Err("terminal output accepted an orphan durable predecessor".into());
    };
    let message = error.to_string();
    assert!(
        message.contains("orphan latest value")
            || message.contains("workspace values disagree with global value accounting")
            || message.contains("immutable workspace_value conflict")
            || message.contains("Corruption"),
        "unexpected corruption error: {message}"
    );
    assert_eq!(store.head(&run)?, head);
    assert_eq!(runtime.projection(&run)?.lifecycle(), RunLifecycle::Created);
    assert!(store.value(orphan.reference()).is_err());
    Ok(())
}
