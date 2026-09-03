//! Data integrity integration scenarios.

use super::*;

#[test]
fn precreated_run_artifact_is_charged_once_across_initial_input_and_later_reuse() -> TestResult {
    let harness = Harness::new("artifact-accounting")?;
    let revision = artifact_reuse_revision("workflow-artifact-accounting")?;
    let run = RunId::new("run-artifact-accounting")?;
    let bytes = b"precreated-run-artifact";
    let artifact = publish_artifact_for_run(&harness, &run, "precreated-run", bytes)?;
    let artifact_bytes = u64::try_from(bytes.len())?;
    assert_eq!(
        harness.store.workspace_usage(&run)?,
        WorkspaceUsage::new(0, 0, 1, artifact_bytes)
    );

    let invocation_reference = InvocationArtifactReference::new(
        artifact.artifact().as_str(),
        artifact.digest().to_hex(),
        Some("application/octet-stream".to_owned()),
        Some(artifact_bytes),
    )?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "result".to_owned(),
                reference: invocation_reference,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    harness.put_revision(&revision)?;
    let root_scope =
        WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-artifact-accounting")?);
    let initial = WorkspaceValueEntry::initial(
        root_scope.reference().clone(),
        ValueKey::new("initial-artifact")?,
        WorkspaceValue::Artifact(artifact),
    );
    assert_eq!(
        harness.command(
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope,
                workspace_budget: generous_budget()?,
                inputs: vec![initial],
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.store.workspace_usage(&run)?,
        WorkspaceUsage::new(1, 0, 1, artifact_bytes)
    );

    assert_eq!(
        harness.command(&run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    assert_eq!(harness.drive(&run, 8)?, 1);
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    let context_bytes = harness
        .runtime
        .history(&run)?
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::NodeScheduled { request, .. } => request
                .context_manifest()
                .and_then(InvocationArtifactReference::size_bytes),
            _ => None,
        })
        .ok_or("model invocation did not bind a context manifest")?;
    assert_eq!(
        harness.store.workspace_usage(&run)?,
        WorkspaceUsage::new(2, 0, 2, artifact_bytes + context_bytes),
        "the reused input is charged once and the frozen context manifest is a distinct artifact"
    );
    Ok(())
}

#[test]
fn deterministic_progress_larger_than_one_commit_resumes_to_completion() -> TestResult {
    let harness = Harness::new("long-deterministic-chain")?;
    let revision = long_deterministic_chain_revision("workflow-long-chain", 250)?;
    let run = RunId::new("run-long-chain")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert!(
        !harness.runtime.projection(&run)?.is_completed(),
        "the first bounded command must stop before consuming the whole chain"
    );
    harness.drive(&run, 8)?;
    let projection = harness.runtime.projection(&run)?;
    assert!(projection.is_completed());
    assert!(
        projection.sequence().get() > 512,
        "fixture must prove deterministic closure spans more than one commit (sequence {})",
        projection.sequence().get()
    );
    Ok(())
}

#[test]
fn undeclared_executor_output_is_durably_rejected_without_workspace_mutation() -> TestResult {
    let harness = Harness::new("undeclared-output")?;
    let revision = task_revision("workflow-undeclared-output")?;
    let run = RunId::new("run-undeclared-output")?;
    let artifact = publish_artifact(&harness, "rogue-output", b"rogue-output")?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "rogue".to_owned(),
                reference: artifact,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    let error = match runtime_tick(&harness.runtime) {
        Ok(result) => {
            return Err(
                format!("undeclared executor output unexpectedly completed: {result:?}").into(),
            );
        }
        Err(error) => error,
    };
    assert!(
        matches!(
            &error,
            RuntimeError::InvalidTransition(detail)
                if detail.contains("not a declared data output")
        ),
        "deterministic report rejection lost its typed runtime error: {error}"
    );
    let projection = harness.runtime.projection(&run)?;
    let root = projection
        .root_scope()
        .ok_or("undeclared-output run has no root scope")?;
    assert!(
        harness
            .store
            .latest_value(root.reference(), &ValueKey::new("rogue")?)?
            .is_none()
    );
    assert!(
        projection
            .node_executions()
            .values()
            .all(|execution| execution.outputs().is_empty())
    );
    let history = harness.runtime.history(&run)?;
    assert!(
        !history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeOutputPublished { .. }))
    );
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::ExternalOutcomeUncertain { reason, .. }
            if reason.as_str().contains("report rejected after adapter entry")
    )));
    assert_eq!(harness.store.head(&run)?, projection.sequence());
    assert_eq!(
        u64::try_from(history.len())?,
        projection.sequence().get(),
        "accepted event history must remain contiguous through the durable head"
    );
    assert_eq!(
        history.last().map(RunEventEnvelope::sequence),
        Some(projection.sequence())
    );
    Ok(())
}

#[test]
fn oversized_provider_retry_after_preserves_the_terminal_failure() -> TestResult {
    let harness = Harness::with_retry_policy(
        "retry-after-cap",
        RetryPolicy::new(2, vec![ErrorClass::Transport], 10, 1_000, 0)?,
    )?;
    let revision = task_revision("workflow-retry-after-cap")?;
    let run = RunId::new("run-retry-after-cap")?;
    let terminal = InvocationTerminal::new(
        TerminalStatus::Failure,
        Vec::new(),
        Some(InvocationFailure::new(
            ErrorClass::Transport,
            true,
            "provider_busy",
            "provider requested an out-of-policy delay",
            Some(10_000),
        )?),
        None,
        SideEffectClass::None,
    )?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![InvocationEventKind::Terminal { terminal }],
    )?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(runtime_tick(&harness.runtime)?.completed, 1);

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert!(projection.attempts().is_empty());
    assert!(
        projection
            .settled_node_executions()
            .values()
            .any(|execution| execution.state()
                == &NodeExecutionState::Terminal(NodeOutcome::Failed)
                && execution.attempt_count() == 1)
    );
    assert!(projection.retries().is_empty());
    assert!(
        !harness
            .runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeRetryScheduled { .. }))
    );
    Ok(())
}

#[test]
fn direct_artifact_input_is_owned_accounted_and_optional_absence_is_omitted() -> TestResult {
    let harness = Harness::new("direct-artifact-input")?;
    let source_run = RunId::new("artifact-source-run")?;
    let bytes = b"direct-artifact-input";
    let artifact = publish_artifact_for_run(&harness, &source_run, "direct-artifact-input", bytes)?;
    let revision = direct_artifact_input_revision("workflow-direct-artifact", &artifact)?;
    let run = RunId::new("run-direct-artifact")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.drive(&run, 8)?, 1);

    let history = harness.runtime.history(&run)?;
    let request = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::NodeScheduled { request, .. } => Some(request),
            _ => None,
        })
        .ok_or("scheduled invocation request was not persisted")?;
    assert_eq!(request.inputs().len(), 1);
    assert_eq!(request.inputs()[0].name(), "artifact");
    assert!(matches!(
        request.inputs()[0].value(),
        InvocationValueReference::Artifact { reference }
            if reference.identity() == artifact.artifact().as_str()
    ));
    assert!(harness.store.is_referenced_by_run(&run, &artifact)?);
    let context = request
        .context_manifest()
        .ok_or("model invocation did not bind a context manifest")?;
    assert_eq!(
        harness.store.workspace_usage(&run)?,
        WorkspaceUsage::new(
            0,
            0,
            2,
            u64::try_from(bytes.len())?
                + context
                    .size_bytes()
                    .ok_or("context manifest has no exact size")?
        )
    );
    Ok(())
}

#[test]
fn successful_terminal_materializes_workflow_and_literal_bindings() -> TestResult {
    let harness = Harness::new("terminal-bindings")?;
    let revision = terminal_binding_revision("workflow-terminal-bindings")?;
    let run = RunId::new("run-terminal-bindings")?;
    harness.put_revision(&revision)?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-terminal-bindings")?);
    let source = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("source")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"source": 7}))?),
    );
    assert_eq!(
        harness.command(
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: root,
                workspace_budget: generous_budget()?,
                inputs: vec![source],
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(&run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );

    let projection = harness.runtime.projection(&run)?;
    let terminal = projection.terminal().ok_or("run did not terminalize")?;
    assert_eq!(terminal.outputs().len(), 2);
    let mut values = BTreeMap::new();
    for reference in terminal.outputs() {
        let entry = harness
            .store
            .value(reference)?
            .ok_or("terminal output workspace value is absent")?;
        values.insert(reference.key().as_str().to_owned(), entry.value().clone());
    }
    assert_eq!(
        values
            .get("pass")
            .and_then(WorkspaceValue::as_json)
            .map(BoundedJson::value),
        Some(&json!({"source": 7}))
    );
    assert_eq!(
        values
            .get("literal")
            .and_then(WorkspaceValue::as_json)
            .map(BoundedJson::value),
        Some(&json!({"materialized": true}))
    );
    Ok(())
}

#[test]
fn missing_optional_condition_binding_routes_exists_to_false() -> TestResult {
    let harness = Harness::new("missing-optional-condition")?;
    let revision = missing_optional_condition_revision("workflow-missing-optional-condition")?;
    let run = RunId::new("run-missing-optional-condition")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    Ok(())
}

#[test]
fn unresolved_optional_edge_does_not_block_selected_target() -> TestResult {
    let harness = Harness::new("optional-unselected-edge")?;
    let revision = optional_unselected_edge_revision("workflow-optional-unselected-edge")?;
    let run = RunId::new("run-optional-unselected-edge")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.drive(&run, 8)?, 1);
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    let scheduled: Vec<_> = harness
        .runtime
        .history(&run)?
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::NodeScheduled { node, request, .. } => {
                Some((node.clone(), request.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].0, NodeId::new("consume")?);
    assert!(scheduled[0].1.inputs().is_empty());
    Ok(())
}

#[test]
fn two_condition_paths_can_share_one_durable_node_output() -> TestResult {
    let harness = Harness::new("multi-path-condition")?;
    let revision = multi_path_condition_revision("workflow-multi-path-condition")?;
    let run = RunId::new("run-multi-path-condition")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(
        harness.command(
            &run,
            RunCommand::DeliverSignal {
                signal: SignalId::new("signal-multi-path")?,
                signal_type: SignalTypeId::new("notify.payload")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(json!({"left": 1, "right": 2}))?,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    Ok(())
}

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
    assert_eq!(executor.dispatches(), 0);
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
    assert_eq!(executor.dispatches(), 0);
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
    assert_eq!(executor.dispatches(), 0);
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
    assert_eq!(executor.dispatches(), 0);
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
fn terminal_unrelated_corruption_is_skipped_by_startup_and_found_by_explicit_scrub() -> TestResult {
    let harness = Harness::new("terminal-corruption-boundary")?;
    let revision = task_revision("workflow-terminal-corruption-boundary")?;
    let run = RunId::new("run-terminal-corruption-boundary")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.drive(&run, 8)?, 1);
    let projection = harness.runtime.projection(&run)?;
    assert!(projection.is_completed());
    let root = projection
        .root_scope()
        .ok_or("terminal run root scope was not projected")?
        .reference()
        .clone();
    let head = harness.store.head(&run)?;
    let history = harness.runtime.history(&run)?;
    let directory = harness.close();
    storage_fault::remove_workspace_scope(directory.path(), &root)?;

    let (store, _clock, executor, runtime) = open_closed_runtime_at(
        directory.path(),
        "terminal-corruption-boundary-reopen",
        NOW,
        64,
    )?;
    runtime.initialize_startup()?;
    assert_eq!(
        runtime.startup_state(),
        RuntimeStartupState::RecoveryCompleted
    );
    assert!(runtime.is_accepting_admission());
    assert_eq!(executor.dispatches(), 0);
    assert_eq!(store.head(&run)?, head);
    assert_eq!(runtime.history(&run)?, history);

    let Err(read_error) = store.scope(root.run(), root.scope()) else {
        return Err("ordinary access treated a deleted terminal scope as absence".into());
    };
    assert!(
        matches!(
            read_error,
            milkdrift_persistence::PersistenceError::Corruption(_)
                | milkdrift_persistence::PersistenceError::Storage {
                    class: milkdrift_persistence::StorageFailureClass::Corruption,
                    ..
                }
        ),
        "deleted indexed root returned {read_error:?}"
    );

    let mut cursor = None;
    let mut failures = 0_usize;
    loop {
        let result = store.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(32)?,
            verify_artifact_content: false,
            cursor: cursor.clone(),
        })?;
        failures = failures.saturating_add(result.failures.len());
        let Some(next) = result.next_cursor else {
            break;
        };
        if cursor.as_ref() == Some(&next) {
            return Err("explicit integrity scrub cursor did not advance".into());
        }
        cursor = Some(next);
    }
    assert!(
        failures > 0,
        "explicit scrub did not report terminal corruption"
    );
    Ok(())
}

#[test]
fn runnable_cursor_keeps_its_cycle_boundary_across_an_advancing_clock() -> TestResult {
    let directory = TempDir::new()?;
    let (store, clock, runtime) = runtime_at(directory.path(), "runnable-cycle", NOW, 1)?;
    let revision = task_revision("workflow-runnable-cycle")?;
    store.put_revision(&revision)?;
    let first = RunId::new("run-a-runnable-cycle")?;
    let second = RunId::new("run-b-runnable-cycle")?;
    for run in [&first, &second] {
        assert_eq!(
            submit_command(
                &runtime,
                store.as_ref(),
                run,
                RunCommand::CreateRun {
                    workflow: revision.semantic().workflow().clone(),
                    revision: revision.id().clone(),
                    root_scope: WorkspaceScope::run_root(
                        run.clone(),
                        ScopeId::new(format!("scope-{run}"))?,
                    ),
                    workspace_budget: generous_budget()?,
                    inputs: Vec::new(),
                },
            )?,
            CommandDisposition::Accepted
        );
        assert_eq!(
            submit_command(&runtime, store.as_ref(), run, RunCommand::StartRun)?,
            CommandDisposition::Accepted
        );
    }

    assert_eq!(runtime_tick(&runtime)?.dispatched, 1);
    let first_after_tick = runtime.projection(&first)?.lifecycle();
    let second_after_tick = runtime.projection(&second)?.lifecycle();
    assert_eq!(
        [first_after_tick, second_after_tick]
            .into_iter()
            .filter(|state| *state == RunLifecycle::Terminal(RunOutcome::Succeeded))
            .count(),
        1,
        "one bounded page must dispatch exactly one of the equally eligible runs"
    );
    clock.advance(1)?;
    assert_eq!(runtime_tick(&runtime)?.dispatched, 1);
    assert_eq!(
        runtime.projection(&first)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(
        runtime.projection(&second)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
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

#[test]
fn changed_pending_adoption_supersedes_old_eligibility_and_runs_new_definition() -> TestResult {
    let harness = Harness::new("changed-pending-supersession")?;
    let old = task_revision("workflow-changed-pending-supersession")?;
    let new = revised_task_revision(&old, "model.fail")?;
    let run = RunId::new("run-changed-pending-supersession")?;
    harness.put_revision(&old)?;
    harness.put_revision(&new)?;
    harness.create_and_start(&run, &old)?;

    let original = harness
        .runtime
        .projection(&run)?
        .executions_for_node(&NodeId::new("work")?)
        .next()
        .ok_or("old pending execution is absent")?
        .execution()
        .clone();
    assert_eq!(
        harness.command(
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconcile-changed-pending")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let planned = harness.runtime.projection(&run)?;
    let plan = planned
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("changed-pending reconciliation plan is absent")?;
    assert!(plan.items().iter().any(|item| {
        item.execution.as_ref() == Some(&original)
            && item.classification == ReconciliationClassification::ChangedPending
            && item.action == ReconciliationAction::UseNewOnNextInvocation
    }));
    let plan_id = plan.plan().clone();
    assert_eq!(
        harness.command(
            &run,
            RunCommand::ApplyReconciliation {
                plan: plan_id.clone()
            }
        )?,
        CommandDisposition::Accepted
    );

    let applied = harness.runtime.projection(&run)?;
    assert_eq!(applied.revision(), Some(new.id()));
    assert!(!applied.node_executions().contains_key(&original));
    assert!(harness.runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::ReconciliationExecutionRemoved { plan, execution }
            if plan == &plan_id && execution == &original
    )));
    assert_eq!(
        applied
            .executions_for_node(&NodeId::new("work")?)
            .filter(|execution| execution.execution() != &original)
            .count(),
        1,
        "the new pin must materialize exactly one replacement occurrence"
    );

    assert_eq!(harness.drive(&run, 4)?, 1);
    let completed = harness.runtime.projection(&run)?;
    let work = NodeId::new("work")?;
    let replacement = completed
        .executions_for_node(&work)
        .find(|execution| execution.execution() != &original)
        .ok_or("replacement execution is absent")?;
    let history = harness.runtime.history(&run)?;
    let request = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::NodeScheduled {
                execution, request, ..
            } if execution == replacement.execution() => Some(request),
            _ => None,
        })
        .ok_or("replacement scheduled request is absent from history")?;
    assert_eq!(
        request.operation(),
        &OperationId::new("model.fail")?,
        "dispatch must use the adopted definition, not the superseded eligibility"
    );
    Ok(())
}

#[test]
fn paused_runs_record_signal_and_timer_facts_without_advancing_until_resume() -> TestResult {
    {
        let harness = Harness::new("paused-signal")?;
        let revision = signal_revision("workflow-paused-signal")?;
        let run = RunId::new("run-paused-signal")?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        assert_eq!(
            harness.command(&run, RunCommand::PauseRun)?,
            CommandDisposition::Accepted
        );
        let signal = SignalId::new("paused-signal-receipt")?;
        assert_eq!(
            harness.command(
                &run,
                RunCommand::DeliverSignal {
                    signal: signal.clone(),
                    signal_type: SignalTypeId::new("notify.ready")?,
                    correlation: None,
                    mode: SignalDeliveryMode::OneShot,
                    payload: BoundedJson::new(json!({"ready": true}))?,
                },
            )?,
            CommandDisposition::Accepted
        );
        let paused = harness.runtime.projection(&run)?;
        assert_eq!(paused.lifecycle(), RunLifecycle::Paused);
        assert!(paused.waits().values().all(|wait| wait.is_pending()));
        assert!(
            paused
                .signals()
                .get(&signal)
                .is_some_and(|signal| signal.consumed_by().is_empty())
        );
        assert_eq!(
            harness.command(&run, RunCommand::ResumeRun)?,
            CommandDisposition::Accepted
        );
        let resumed = harness.runtime.projection(&run)?;
        assert_eq!(resumed.waits().len(), 1);
        assert!(resumed.waits().values().all(|wait| wait.is_pending()));
        assert!(resumed.waits().values().all(|wait| !matches!(
            wait.condition(),
            milkdrift_persistence::WaitCondition::Signal { .. }
        )));
        assert!(!resumed.signals().contains_key(&signal));
        let history = harness.runtime.history(&run)?;
        assert!(history.iter().any(|event| matches!(
            event.kind(),
            RunEventKind::SignalConsumed { signal: consumed, .. } if consumed == &signal
        )));
        assert!(history.iter().any(|event| matches!(
            event.kind(),
            RunEventKind::WaitSatisfied {
                cause: milkdrift_persistence::WaitSatisfaction::Signal { signal: consumed },
                ..
            } if consumed == &signal
        )));
    }

    {
        let harness = Harness::new("paused-timer")?;
        let revision = wait_revision("workflow-paused-timer", 100)?;
        let run = RunId::new("run-paused-timer")?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        assert_eq!(
            harness.command(&run, RunCommand::PauseRun)?,
            CommandDisposition::Accepted
        );
        harness.clock.advance(100)?;
        assert_eq!(runtime_tick(&harness.runtime)?.dispatched, 0);
        let paused = harness.runtime.projection(&run)?;
        assert_eq!(paused.lifecycle(), RunLifecycle::Paused);
        assert!(paused.timers().values().all(|timer| timer.is_completed()));
        assert!(paused.waits().values().all(|wait| wait.is_pending()));
        assert_eq!(
            harness.command(&run, RunCommand::ResumeRun)?,
            CommandDisposition::Accepted
        );
        let resumed = harness.runtime.projection(&run)?;
        assert!(resumed.waits().is_empty());
        assert!(resumed.timers().is_empty());
        assert_eq!(
            resumed.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Succeeded)
        );
        let history = harness.runtime.history(&run)?;
        assert!(
            history
                .iter()
                .any(|event| matches!(event.kind(), RunEventKind::TimerFired { .. }))
        );
        assert!(history.iter().any(|event| matches!(
            event.kind(),
            RunEventKind::WaitSatisfied {
                cause: milkdrift_persistence::WaitSatisfaction::Timer { .. },
                ..
            }
        )));
    }
    Ok(())
}

#[test]
fn immutable_repeat_condition_error_is_durably_terminalized_once() -> TestResult {
    let harness = Harness::new("repeat-condition-error")?;
    let child = task_revision("workflow-repeat-condition-error-child")?;
    let seed_field = FieldId::new("seed")?;
    let seed_port = PortId::new("seed")?;
    let seed_binding = BindingSource::WorkflowInput {
        field: seed_field.clone(),
    };
    let repeat = Node::new(
        NodeId::new("repeat")?,
        NodeKind::Repeat {
            config: RepeatConfig::new(
                PinnedSubworkflow::new(
                    child.semantic().workflow().clone(),
                    child.id().clone(),
                    child.semantic().interface().clone(),
                ),
                Condition::Compare {
                    left: ConditionOperand::Binding {
                        source: seed_binding.clone(),
                    },
                    comparison: Comparison::GreaterThan,
                    right: ConditionOperand::Literal {
                        value: BoundedJson::new(json!(0))?,
                    },
                },
                2,
                RepeatBudget {
                    max_duration_ms: None,
                    max_cost_micros: None,
                    max_cost_currency: None,
                },
                RepeatTermination::SucceedWithLatest,
            )?,
        },
    )?
    .with_control_output(PortId::new("out")?)?
    .with_data_input(
        seed_port,
        DataPort::input(item_schema()?, true, Some(seed_binding))?,
    )?;
    let parent = revision_with_interface(
        "workflow-repeat-condition-error-parent",
        WorkflowInterface::new([(seed_field, InterfaceField::required(item_schema()?))], [])?,
        vec![repeat, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("repeat-done", "repeat", "out", "done", "in")?],
    )?;
    let run = RunId::new("run-repeat-condition-error")?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-repeat-condition-error")?);
    let input = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("seed")?,
        WorkspaceValue::Json(BoundedJson::new(json!("not-a-number"))?),
    );
    harness.put_revision(&child)?;
    harness.put_revision(&parent)?;
    assert_eq!(
        harness.command(
            &run,
            RunCommand::CreateRun {
                workflow: parent.semantic().workflow().clone(),
                revision: parent.id().clone(),
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
    harness.drive(&run, 8)?;

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert!(projection.iterations().is_empty());
    assert!(projection.repeat_terminations().is_empty());
    let history = harness.runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(
                event.kind(),
                RunEventKind::RepeatTerminated {
                    termination: RepeatTerminationReason::ConditionEvaluationFailed,
                    ..
                }
            ))
            .count(),
        1,
        "the immutable journal retains the single terminal repeat decision"
    );
    Ok(())
}

#[test]
fn immutable_task_input_path_error_is_durably_failed_before_dispatch() -> TestResult {
    let harness = Harness::new("immutable-task-input-path")?;
    let schema = item_schema()?;
    let payload_port = PortId::new("payload")?;
    let source = BindingSource::NodeOutput {
        node: NodeId::new("signal")?,
        port: payload_port.clone(),
        path: PathSelector::new(vec![PathSegment::Field(FieldId::new("missing")?)])?,
    };
    let signal = Node::new(
        NodeId::new("signal")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.payload")?,
        },
    )?
    .with_control_output(PortId::new("out")?)?
    .with_data_output(payload_port.clone(), DataPort::output(schema.clone()))?;
    let consume = task("consume", "model.generate")?.with_data_input(
        PortId::new("input")?,
        DataPort::input(schema, true, Some(source))?,
    )?;
    let revision = revision(
        "workflow-immutable-task-input-path",
        vec![signal, consume, terminal("done", TerminalOutcome::Success)?],
        vec![
            control_edge("signal-consume", "signal", "out", "consume", "in")?,
            data_edge(
                "signal-payload-consume",
                "signal",
                "payload",
                "consume",
                "input",
            )?,
            control_edge("consume-done", "consume", "out", "done", "in")?,
        ],
    )?;
    let run = RunId::new("run-immutable-task-input-path")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(
        harness.command(
            &run,
            RunCommand::DeliverSignal {
                signal: SignalId::new("signal-immutable-task-input-path")?,
                signal_type: SignalTypeId::new("notify.payload")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(json!({"present": true}))?,
            },
        )?,
        CommandDisposition::Accepted
    );

    let tick = runtime_tick(&harness.runtime)?;
    assert_eq!(tick.dispatched, 0);
    assert_eq!(tick.completed, 1);
    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert!(projection.attempts().is_empty());
    assert!(projection.leases().is_empty());
    let history = harness.runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::NodePreDispatchFailed { .. }))
            .count(),
        1
    );
    assert!(!history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeScheduled { .. } | RunEventKind::LeaseGranted { .. }
    )));
    let head = harness.store.head(&run)?;
    let _ = runtime_tick(&harness.runtime)?;
    assert_eq!(harness.store.head(&run)?, head);
    Ok(())
}

#[test]
fn oversized_immutable_invocation_is_durably_failed_before_dispatch() -> TestResult {
    let harness = Harness::new("oversized-immutable-invocation")?;
    let schema = item_schema()?;
    let mut work = task("work", "model.generate")?;
    let large_literal = BoundedJson::new(json!("x".repeat(32_000)))?;
    for index in 0..17 {
        work = work.with_data_input(
            PortId::new(format!("input-{index:02}"))?,
            DataPort::input(
                schema.clone(),
                true,
                Some(BindingSource::Literal {
                    value: large_literal.clone(),
                }),
            )?,
        )?;
    }
    let revision = revision(
        "workflow-oversized-immutable-invocation",
        vec![work, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("work-done", "work", "out", "done", "in")?],
    )?;
    let run = RunId::new("run-oversized-immutable-invocation")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    let tick = runtime_tick(&harness.runtime)?;
    assert_eq!(tick.dispatched, 0);
    assert_eq!(tick.completed, 1);
    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert!(projection.attempts().is_empty());
    assert_eq!(
        harness
            .runtime
            .history(&run)?
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::NodePreDispatchFailed { .. }))
            .count(),
        1
    );
    Ok(())
}
