//! Structured graph integration scenarios.

use super::*;

#[test]
fn branch_freezes_exactly_one_route_and_never_creates_the_other_execution() -> TestResult {
    let harness = Harness::new("branch")?;
    let revision = branch_revision("workflow-branch")?;
    let run = RunId::new("run-branch")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(projection.branch_routes().len(), 1);
    assert_eq!(
        projection.branch_routes().values().next(),
        Some(&PortId::new("true")?)
    );
    assert_eq!(
        projection
            .executions_for_node(&NodeId::new("selected")?)
            .count(),
        1
    );
    assert_eq!(
        projection
            .executions_for_node(&NodeId::new("unselected")?)
            .count(),
        0
    );
    Ok(())
}

#[test]
fn all_join_preserves_independent_success_and_failure_branch_truth() -> TestResult {
    let harness = Harness::new("fork-all")?;
    harness.executor.set_script(
        OperationId::new("model.fail")?,
        vec![InvocationEventKind::Terminal {
            terminal: failed_terminal()?,
        }],
    )?;
    let revision = fork_revision("workflow-fork-all", JoinPolicy::All, "model.fail")?;
    let run = RunId::new("run-fork-all")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.drive(&run, 8)?, 2);

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(projection.branches().len(), 2);
    assert!(
        projection
            .branches()
            .values()
            .any(|branch| { branch.state() == BranchState::Completed(RunOutcome::Succeeded) })
    );
    assert!(
        projection
            .branches()
            .values()
            .any(|branch| branch.state() == BranchState::Completed(RunOutcome::Failed))
    );
    let join = projection
        .joins()
        .values()
        .next()
        .ok_or("join did not complete")?;
    assert_eq!(join.branches().len(), 2);
    assert!(join.retained_branches().is_empty());
    Ok(())
}

#[test]
fn nested_fork_waits_for_descendants_and_preserves_outputs_through_outer_join() -> TestResult {
    let harness = Harness::new("nested-fork")?;
    install_output_scripts(&harness)?;
    let revision = nested_fork_revision("workflow-nested-fork")?;
    let run = RunId::new("run-nested-fork")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.drive(&run, 16)?, 4);

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    let outer_fork_node = NodeId::new("outer-fork")?;
    let outer_fork = projection
        .executions_for_node(&outer_fork_node)
        .next()
        .ok_or("outer fork execution is absent")?;
    let outer_a_port = PortId::new("a")?;
    let outer_a = projection
        .branches()
        .values()
        .find(|branch| {
            branch.fork_execution() == outer_fork.execution() && branch.port() == &outer_a_port
        })
        .ok_or("outer a branch is absent")?;
    assert_eq!(
        outer_a.state(),
        BranchState::Completed(RunOutcome::Succeeded)
    );
    assert_eq!(
        outer_a.outputs().len(),
        1,
        "outer branch lost its declared post-join result output"
    );

    let outer_join_node = NodeId::new("outer-join")?;
    let outer_join_execution = projection
        .executions_for_node(&outer_join_node)
        .next()
        .ok_or("outer join execution is absent")?;
    let outer_join = projection
        .joins()
        .get(outer_join_execution.execution())
        .ok_or("outer join result is absent")?;
    let outer_result = outer_join
        .branches()
        .iter()
        .find(|result| result.branch == *outer_a.branch())
        .ok_or("outer join omitted branch a")?;
    assert_eq!(outer_result.outputs, outer_a.outputs());

    let inner_join_node = NodeId::new("inner-join")?;
    let inner_join_execution = projection
        .executions_for_node(&inner_join_node)
        .next()
        .ok_or("inner join execution is absent")?;
    let inner_join_sequence = projection
        .joins()
        .get(inner_join_execution.execution())
        .ok_or("inner join result is absent")?
        .sequence();
    let tail_node = NodeId::new("outer-a-tail")?;
    let tail_execution = projection
        .executions_for_node(&tail_node)
        .next()
        .ok_or("outer a successor is absent")?;
    let history = harness.runtime.history(&run)?;
    let tail_terminal_sequence = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::NodeTerminal { execution, .. }
                if execution == tail_execution.execution() =>
            {
                Some(event.sequence())
            }
            _ => None,
        })
        .ok_or("outer a successor terminal fact is absent")?;
    let outer_terminal_sequence = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::BranchTerminal { branch, .. } if branch == outer_a.branch() => {
                Some(event.sequence())
            }
            _ => None,
        })
        .ok_or("outer a terminal fact is absent")?;
    assert!(outer_terminal_sequence > inner_join_sequence);
    assert!(outer_terminal_sequence > tail_terminal_sequence);
    Ok(())
}

#[test]
fn fork_branches_may_end_at_direct_terminals_without_a_join() -> TestResult {
    for (suffix, a_outcome, b_outcome, expected) in [
        (
            "success",
            TerminalOutcome::Success,
            TerminalOutcome::Success,
            RunOutcome::Succeeded,
        ),
        (
            "mixed",
            TerminalOutcome::Failure,
            TerminalOutcome::Success,
            RunOutcome::Failed,
        ),
    ] {
        let harness = Harness::new(&format!("direct-terminal-fork-{suffix}"))?;
        let revision = direct_terminal_fork_revision(
            &format!("workflow-direct-terminal-fork-{suffix}"),
            a_outcome,
            b_outcome,
        )?;
        let run = RunId::new(format!("run-direct-terminal-fork-{suffix}"))?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        let projection = harness.runtime.projection(&run)?;
        assert_eq!(projection.lifecycle(), RunLifecycle::Terminal(expected));
        assert_eq!(projection.branches().len(), 2);
        assert!(
            projection
                .branches()
                .values()
                .all(|branch| !branch.is_active())
        );
        if suffix == "mixed" {
            assert!(
                projection
                    .branches()
                    .values()
                    .any(|branch| { branch.state() == BranchState::Completed(RunOutcome::Failed) })
            );
            assert!(
                projection.branches().values().any(|branch| {
                    branch.state() == BranchState::Completed(RunOutcome::Succeeded)
                })
            );
        }
    }
    Ok(())
}

#[test]
fn any_join_records_and_cancels_its_unfinished_loser_without_dispatch() -> TestResult {
    let harness = Harness::new("fork-any")?;
    let revision = fork_revision("workflow-fork-any", JoinPolicy::Any, "model.generate")?;
    let run = RunId::new("run-fork-any")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    assert_eq!(harness.drive(&run, 4)?, 1);
    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(projection.joins().len(), 1);
    assert!(
        projection
            .joins()
            .values()
            .all(|join| join.retained_branches().is_empty())
    );
    assert!(
        projection
            .branches()
            .values()
            .any(|branch| { branch.state() == BranchState::Completed(RunOutcome::Succeeded) })
    );
    assert!(
        projection
            .branches()
            .values()
            .any(|branch| { branch.state() == BranchState::Completed(RunOutcome::Cancelled) })
    );
    let history = harness.runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::BranchCancellationRequested { .. }
    )));
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeExecutionCancelledBeforeDispatch { .. }
    )));
    Ok(())
}

#[test]
fn first_success_and_quorum_cancel_losers_without_dispatching_them() -> TestResult {
    for (suffix, policy) in [
        ("first", JoinPolicy::FirstSuccess),
        ("quorum", JoinPolicy::Quorum(1)),
    ] {
        let harness = Harness::new(&format!("fork-{suffix}"))?;
        let revision = fork_revision(&format!("workflow-fork-{suffix}"), policy, "model.generate")?;
        let run = RunId::new(format!("run-fork-{suffix}"))?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        assert_eq!(harness.drive(&run, 8)?, 1, "{suffix} dispatched a loser");

        let projection = harness.runtime.projection(&run)?;
        assert!(
            projection.is_completed(),
            "{suffix} did not drain the loser"
        );
        assert_eq!(projection.branches().len(), 2);
        assert!(
            projection
                .branches()
                .values()
                .any(|branch| { branch.state() == BranchState::Completed(RunOutcome::Succeeded) })
        );
        assert!(
            projection
                .branches()
                .values()
                .any(|branch| { branch.state() == BranchState::Completed(RunOutcome::Cancelled) })
        );
        assert!(
            projection
                .joins()
                .values()
                .all(|join| join.retained_branches().is_empty())
        );
    }
    Ok(())
}

#[test]
fn impossible_first_success_and_quorum_fail_deterministically_instead_of_deadlocking() -> TestResult
{
    for (suffix, policy, fail_first) in [
        ("first-impossible", JoinPolicy::FirstSuccess, true),
        ("quorum-impossible", JoinPolicy::Quorum(2), false),
    ] {
        let harness = Harness::new(&format!("fork-{suffix}"))?;
        harness.executor.set_script(
            OperationId::new("model.fail")?,
            vec![InvocationEventKind::Terminal {
                terminal: failed_terminal()?,
            }],
        )?;
        if fail_first {
            harness.executor.set_script(
                OperationId::new("model.generate")?,
                vec![InvocationEventKind::Terminal {
                    terminal: failed_terminal()?,
                }],
            )?;
        }
        let revision = fork_revision(&format!("workflow-{suffix}"), policy, "model.fail")?;
        let run = RunId::new(format!("run-{suffix}"))?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        assert_eq!(harness.drive(&run, 8)?, 2);

        let projection = harness.runtime.projection(&run)?;
        assert_eq!(
            projection.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Failed)
        );
        let join_id = NodeId::new("join")?;
        let join = projection
            .executions_for_node(&join_id)
            .next()
            .ok_or("impossible join execution was not created")?;
        assert_eq!(
            join.state(),
            &NodeExecutionState::Terminal(milkdrift_persistence::NodeOutcome::Failed)
        );
    }
    Ok(())
}

#[test]
fn collect_and_first_reducers_publish_deterministic_workspace_outputs() -> TestResult {
    for (suffix, strategy) in [
        ("collect", ReducerStrategy::Collect),
        ("first", ReducerStrategy::First),
    ] {
        let harness = Harness::new(&format!("reducer-{suffix}"))?;
        install_output_scripts(&harness)?;
        let revision = reducer_revision(&format!("workflow-reducer-{suffix}"), strategy.clone())?;
        let run = RunId::new(format!("run-reducer-{suffix}"))?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        assert_eq!(harness.drive(&run, 8)?, 2);

        let projection = harness.runtime.projection(&run)?;
        assert!(projection.is_completed());
        let root_scope = projection
            .root_scope()
            .ok_or("reducer run has no root scope")?
            .reference();
        let mut sibling_output_scopes = BTreeSet::new();
        for task_id in [NodeId::new("a-task")?, NodeId::new("b-task")?] {
            let task_execution = projection
                .executions_for_node(&task_id)
                .next()
                .ok_or("branch task execution was not created")?;
            assert_ne!(task_execution.scope(), root_scope);
            assert!(matches!(
                projection
                    .scopes()
                    .get(task_execution.scope())
                    .ok_or("branch task scope was not projected")?
                    .kind(),
                ScopeKind::Branch { .. }
            ));
            assert_eq!(task_execution.outputs().len(), 1);
            assert_eq!(
                task_execution.outputs()[0].value().scope(),
                task_execution.scope()
            );
            assert!(sibling_output_scopes.insert(task_execution.scope().clone()));
        }
        assert_eq!(sibling_output_scopes.len(), 2);
        let reducer_id = NodeId::new("reduce")?;
        let execution = projection
            .executions_for_node(&reducer_id)
            .next()
            .ok_or("reducer execution was not created")?;
        assert_eq!(execution.scope(), root_scope);
        assert_eq!(execution.outputs().len(), 1);
        assert_eq!(execution.outputs()[0].value().scope(), root_scope);
        let output = harness
            .store
            .value(execution.outputs()[0].value())?
            .ok_or("reducer output is absent from workspace storage")?;
        let mut branches: Vec<_> = projection.branches().values().collect();
        branches.sort_by(|left, right| left.port().cmp(right.port()));
        let branch_outputs: Vec<_> = branches
            .into_iter()
            .flat_map(|branch| branch.outputs().iter().cloned())
            .collect();
        let mut lexical_outputs = branch_outputs.clone();
        lexical_outputs.sort();
        assert_ne!(
            branch_outputs, lexical_outputs,
            "fixture must distinguish declared branch order from reference lexical order"
        );
        match strategy {
            ReducerStrategy::Collect => {
                let values = output
                    .value()
                    .as_json()
                    .and_then(|value| value.value().as_array())
                    .ok_or("collect output is not a structured array")?;
                assert_eq!(values.len(), 2);
                assert_eq!(
                    output.value().as_json().map(BoundedJson::value),
                    Some(&serde_json::to_value(&branch_outputs)?)
                );
            }
            ReducerStrategy::First => {
                let expected = harness
                    .store
                    .value(branch_outputs.first().ok_or("branches had no outputs")?)?
                    .ok_or("first branch value is absent")?;
                assert_eq!(output.value(), expected.value());
            }
            ReducerStrategy::Capability(_) => unreachable!("fixture uses deterministic reducers"),
        }
    }
    Ok(())
}

#[test]
fn durable_timer_wait_fires_only_at_its_recorded_deadline() -> TestResult {
    let harness = Harness::new("timer")?;
    let revision = wait_revision("workflow-timer", 100)?;
    let run = RunId::new("run-timer")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    let initial = harness.runtime.projection(&run)?;
    assert_eq!(initial.timers().len(), 1);
    assert!(initial.timers().values().all(|timer| timer.is_pending()));
    assert!(initial.waits().values().all(|wait| wait.is_pending()));
    assert_eq!(harness.runtime.tick()?.dispatched, 0);
    harness.clock.advance(99)?;
    assert_eq!(harness.runtime.tick()?.dispatched, 0);
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Running
    );

    harness.clock.advance(1)?;
    harness.runtime.tick()?;
    let completed = harness.runtime.projection(&run)?;
    assert!(completed.is_completed());
    assert!(completed.timers().is_empty());
    assert!(completed.waits().is_empty());
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
    Ok(())
}

#[test]
fn typed_signal_is_consumed_once_and_duplicate_delivery_is_a_durable_fact() -> TestResult {
    let harness = Harness::new("signal")?;
    let revision = signal_revision("workflow-signal")?;
    let run = RunId::new("run-signal")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    let signal = SignalId::new("signal-ready-1")?;
    let delivery = || -> TestResult<RunCommand> {
        Ok(RunCommand::DeliverSignal {
            signal: signal.clone(),
            signal_type: SignalTypeId::new("notify.ready")?,
            correlation: None,
            mode: SignalDeliveryMode::OneShot,
            payload: BoundedJson::new(json!({"ready": true}))?,
        })
    };
    assert_eq!(
        harness.command(&run, delivery()?)?,
        CommandDisposition::Accepted
    );
    let after_first = harness.runtime.projection(&run)?;
    assert!(!after_first.signals().contains_key(&signal));
    assert_eq!(after_first.lifecycle(), RunLifecycle::Running);

    assert_eq!(
        harness.command(&run, delivery()?)?,
        CommandDisposition::Accepted
    );
    let after_duplicate = harness.runtime.projection(&run)?;
    assert!(!after_duplicate.signals().contains_key(&signal));
    let history = harness.runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::SignalReceived { .. }))
            .count(),
        1
    );
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::SignalDeduplicated { .. }))
            .count(),
        1
    );

    harness.clock.advance(50)?;
    harness.runtime.tick()?;
    assert!(harness.runtime.projection(&run)?.is_completed());
    Ok(())
}

#[test]
fn broadcast_signal_fanout_is_received_once_then_drained_in_bounded_batches() -> TestResult {
    const OUTPUTS_PER_WAIT: usize = 254;
    let harness = Harness::new("broadcast-fanout")?;
    let revision = broadcast_fanout_revision("workflow-broadcast-fanout", OUTPUTS_PER_WAIT)?;
    let run = RunId::new("run-broadcast-fanout")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.runtime.projection(&run)?.waits().len(), 2);

    let signal = SignalId::new("signal-broadcast-fanout")?;
    let command = harness.runtime.command(
        run.clone(),
        ActorRef::new("human:broadcast-fanout")?,
        harness.store.head(&run)?,
        Reason::new("deliver a fanout larger than one atomic event batch")?,
        Vec::new(),
        RunCommand::DeliverSignal {
            signal: signal.clone(),
            signal_type: SignalTypeId::new("notify.broadcast")?,
            correlation: None,
            mode: SignalDeliveryMode::Broadcast,
            payload: BoundedJson::new(json!({"broadcast": true}))?,
        },
    )?;
    let accepted = harness.runtime.handle_command(&command)?;
    assert!(!accepted.replayed());
    assert_eq!(
        accepted.result().disposition(),
        CommandDisposition::Accepted
    );
    assert_eq!(accepted.result().event_ids().len(), 1);
    let replayed = harness.runtime.handle_command(&command)?;
    assert!(replayed.replayed());
    assert_eq!(replayed.result(), accepted.result());

    let received = harness.runtime.projection(&run)?;
    let signal_view = received
        .signals()
        .get(&signal)
        .ok_or("broadcast signal is absent")?;
    assert!(signal_view.consumed_by().is_empty());
    assert_eq!(
        received
            .waits()
            .values()
            .filter(|wait| wait.is_pending())
            .count(),
        2
    );

    for _ in 0..8 {
        if harness.runtime.projection(&run)?.is_completed() {
            break;
        }
        harness.runtime.tick()?;
    }
    let completed = harness.runtime.projection(&run)?;
    assert_eq!(
        completed.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert!(!completed.signals().contains_key(&signal));
    let history = harness.runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::SignalReceived { .. }))
            .count(),
        1
    );
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::SignalConsumed { .. }))
            .count(),
        2
    );
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(
                event.kind(),
                RunEventKind::DeterministicOutputPublished { .. }
            ))
            .count(),
        OUTPUTS_PER_WAIT * 2
    );
    Ok(())
}

#[test]
fn unchanged_runnable_index_remains_dispatchable_after_an_unrelated_commit() -> TestResult {
    let harness = Harness::new("unchanged-runnable-index")?;
    let revision = task_revision("workflow-unchanged-runnable-index")?;
    let run = RunId::new("run-unchanged-runnable-index")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(
        harness.command(
            &run,
            RunCommand::DeliverSignal {
                signal: SignalId::new("unmatched-runnable-signal")?,
                signal_type: SignalTypeId::new("notify.unmatched")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(json!({}))?,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(harness.runtime.tick()?.dispatched, 1);
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    Ok(())
}

#[test]
fn attached_subworkflow_materializes_starts_and_links_a_terminal_child_run() -> TestResult {
    let harness = Harness::new("subworkflow")?;
    install_child_output_script(&harness)?;
    let child = output_child_revision("workflow-child")?;
    let parent = subworkflow_revision("workflow-parent", &child)?;
    let run = RunId::new("run-parent")?;
    harness.put_revision(&child)?;
    harness.put_revision(&parent)?;
    harness.create_and_start(&run, &parent)?;

    assert_eq!(harness.runtime.projection(&run)?.subworkflows().len(), 1);
    harness.drive(&run, 8)?;
    let projection = harness.runtime.projection(&run)?;
    assert!(projection.is_completed());
    let link = projection
        .subworkflows()
        .values()
        .next()
        .ok_or("parent has no child link")?;
    assert_eq!(link.child_revision(), child.id());
    assert_eq!(
        link.state(),
        SubworkflowState::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(link.imports().len(), 1);
    let imported = &link.imports()[0];
    assert_eq!(imported.child_value().scope().run(), link.child_run());
    assert_eq!(imported.parent_value().scope().run(), &run);
    let parent_entry = harness
        .store
        .value(imported.parent_value())?
        .ok_or("imported child output is absent from the parent workspace")?;
    match parent_entry.origin() {
        ValueOrigin::Imported { source } => assert_eq!(source, imported.child_value()),
        origin => return Err(format!("expected imported value origin, found {origin:?}").into()),
    }
    assert!(
        harness
            .runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::SubworkflowOutputImported { .. }))
    );
    let child_projection = harness.runtime.projection(link.child_run())?;
    assert_eq!(child_projection.revision(), Some(child.id()));
    assert_eq!(
        child_projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    Ok(())
}

#[test]
fn repeat_runs_each_pinned_child_in_an_isolated_scope_and_stops_at_the_bound() -> TestResult {
    let harness = Harness::new("repeat")?;
    install_child_output_script(&harness)?;
    let child = output_child_revision("workflow-repeat-child")?;
    let parent = repeat_revision("workflow-repeat-parent", &child)?;
    let run = RunId::new("run-repeat-parent")?;
    harness.put_revision(&child)?;
    harness.put_revision(&parent)?;
    harness.create_and_start(&run, &parent)?;
    harness.drive(&run, 12)?;

    let projection = harness.runtime.projection(&run)?;
    assert!(projection.is_completed());
    assert_eq!(projection.iterations().len(), 1);
    let latest_iteration = projection
        .iterations()
        .values()
        .next()
        .ok_or("latest repeat iteration is absent")?;
    assert_eq!(latest_iteration.iteration_number(), 2);
    assert!(latest_iteration.is_completed());
    let history = harness.runtime.history(&run)?;
    let iteration_scopes: BTreeSet<_> = history
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::RepeatIterationCreated { scope, .. } => Some(scope.reference().clone()),
            _ => None,
        })
        .collect();
    assert_eq!(iteration_scopes.len(), 2);
    let iteration_parents: BTreeSet<_> = history
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::RepeatIterationCreated { scope, .. } => {
                assert!(matches!(scope.kind(), ScopeKind::Iteration { .. }));
                scope.parent().cloned()
            }
            _ => None,
        })
        .collect();
    assert_eq!(iteration_parents.len(), 1, "iterations are not siblings");
    let termination = projection
        .repeat_terminations()
        .values()
        .next()
        .ok_or("repeat did not record a terminal bound")?;
    assert_eq!(
        termination.termination(),
        RepeatTerminationReason::MaximumIterations
    );
    assert_eq!(projection.subworkflows().len(), 1);
    let mut imported_parent_values = BTreeSet::new();
    let mut imported_child_values = BTreeSet::new();
    for event in &history {
        let RunEventKind::SubworkflowOutputImported {
            subworkflow,
            child_value,
            parent_value,
        } = event.kind()
        else {
            continue;
        };
        assert!(history.iter().any(|candidate| matches!(
            candidate.kind(),
            RunEventKind::SubworkflowCreated {
                subworkflow: created,
                scope,
                ..
            } if created == subworkflow
                && scope.reference() == parent_value.scope()
                && scope.parent().is_some_and(|parent| iteration_scopes.contains(parent))
        )));
        assert!(imported_parent_values.insert(parent_value.clone()));
        assert!(imported_child_values.insert(child_value.clone()));
        let entry = harness
            .store
            .value(parent_value)?
            .ok_or("repeat child import is absent")?;
        assert!(matches!(
            entry.origin(),
            ValueOrigin::Imported { source } if source == child_value
        ));
    }
    assert_eq!(imported_parent_values.len(), 2);
    assert_eq!(imported_child_values.len(), 2);
    let child_runs: BTreeSet<_> = history
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::SubworkflowCreated { child_run, .. } => Some(child_run.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(child_runs.len(), 2);
    for child_run in child_runs {
        assert!(harness.runtime.projection(&child_run)?.is_completed());
    }
    Ok(())
}

#[test]
fn await_approval_repeat_extends_exactly_once_then_rejection_terminates() -> TestResult {
    let harness = Harness::new("repeat-approval")?;
    let child = task_revision("workflow-repeat-approval-child")?;
    let parent = approval_repeat_revision("workflow-repeat-approval-parent", &child)?;
    let run = RunId::new("run-repeat-approval")?;
    harness.put_revision(&child)?;
    harness.put_revision(&parent)?;
    harness.create_and_start(&run, &parent)?;
    harness.drive(&run, 16)?;

    let boundary = harness.runtime.projection(&run)?;
    assert_eq!(boundary.lifecycle(), RunLifecycle::Running);
    assert_eq!(boundary.iterations().len(), 1);
    assert!(boundary.repeat_terminations().is_empty());
    assert_eq!(
        boundary
            .iterations()
            .values()
            .next()
            .map(|iteration| iteration.state()),
        Some(IterationState::ConditionRecorded(true))
    );
    let repeat_execution = boundary
        .executions_for_node(&NodeId::new("repeat")?)
        .next()
        .ok_or("await-approval repeat execution was not created")?
        .execution()
        .clone();
    let continuation = boundary
        .repeat_continuations()
        .get(&repeat_execution)
        .ok_or("await-approval repeat continuation request was not recorded")?;
    assert!(continuation.is_pending_approval());
    assert_eq!(
        continuation
            .pending_request()
            .map(|request| request.cause()),
        Some(&RepeatContinuationCause::IterationLimit)
    );

    let approval_id = RepeatDecisionId::new("repeat-approval-plus-two")?;
    let approval = harness.runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        harness.store.head(&run)?,
        Reason::new("authorize exactly two more repeat iterations")?,
        Vec::new(),
        RunCommand::DecideRepeatContinuation {
            repeat_execution: repeat_execution.clone(),
            decision: approval_id.clone(),
            outcome: RepeatContinuationDecision::Approved,
            approved_additional_iterations: Some(2),
        },
    )?;
    let approved = harness.runtime.handle_command(&approval)?;
    assert_eq!(
        approved.result().disposition(),
        CommandDisposition::Accepted
    );
    assert!(!approved.replayed());
    let approved_head = harness.store.head(&run)?;
    let replayed = harness.runtime.handle_command(&approval)?;
    assert!(replayed.replayed());
    assert_eq!(replayed.result(), approved.result());
    assert_eq!(harness.store.head(&run)?, approved_head);

    assert_eq!(
        harness.command(
            &run,
            RunCommand::DecideRepeatContinuation {
                repeat_execution: repeat_execution.clone(),
                decision: approval_id,
                outcome: RepeatContinuationDecision::Approved,
                approved_additional_iterations: Some(2),
            },
        )?,
        CommandDisposition::Rejected,
        "a new command cannot reuse a durable repeat decision identity"
    );
    let after_duplicate = harness.runtime.projection(&run)?;
    let continuation = after_duplicate
        .repeat_continuations()
        .get(&repeat_execution)
        .ok_or("approval did not create continuation authority")?;
    assert_eq!(continuation.initial_iteration_limit(), 1);
    assert_eq!(continuation.effective_iteration_limit(), 3);
    assert_eq!(continuation.decisions().len(), 1);

    harness.drive(&run, 32)?;
    let next_boundary = harness.runtime.projection(&run)?;
    assert_eq!(next_boundary.lifecycle(), RunLifecycle::Running);
    assert_eq!(next_boundary.iterations().len(), 1);
    assert_eq!(next_boundary.subworkflows().len(), 1);
    assert!(next_boundary.repeat_terminations().is_empty());
    let continuation = next_boundary
        .repeat_continuations()
        .get(&repeat_execution)
        .ok_or("repeat lost its continuation authority")?;
    assert!(continuation.is_pending_approval());
    assert_eq!(continuation.effective_iteration_limit(), 3);
    assert_eq!(continuation.decisions().len(), 1);
    assert_eq!(continuation.decision_count(), 1);
    assert_eq!(
        harness
            .runtime
            .history(&run)?
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::RepeatIterationCreated { .. }))
            .count(),
        3
    );

    assert_eq!(
        harness.command(
            &run,
            RunCommand::DecideRepeatContinuation {
                repeat_execution: repeat_execution.clone(),
                decision: RepeatDecisionId::new("repeat-approval-reject")?,
                outcome: RepeatContinuationDecision::Rejected,
                approved_additional_iterations: None,
            },
        )?,
        CommandDisposition::Accepted
    );
    harness.drive(&run, 8)?;
    let rejected = harness.runtime.projection(&run)?;
    assert_eq!(
        rejected.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert_eq!(rejected.iterations().len(), 1);
    assert_eq!(rejected.subworkflows().len(), 1);
    let continuation = rejected
        .repeat_continuations()
        .get(&repeat_execution)
        .ok_or("repeat lost the rejected continuation fact")?;
    assert!(continuation.is_rejected());
    assert_eq!(continuation.decisions().len(), 1);
    assert_eq!(continuation.decision_count(), 2);
    assert_eq!(
        rejected
            .repeat_terminations()
            .get(&repeat_execution)
            .ok_or("rejected repeat has no deterministic termination")?
            .termination(),
        RepeatTerminationReason::MaximumIterations
    );
    let history = harness.runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::RepeatContinuationDecided { .. }))
            .count(),
        2
    );
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::SubworkflowCreated { .. }))
            .count(),
        3
    );
    Ok(())
}
