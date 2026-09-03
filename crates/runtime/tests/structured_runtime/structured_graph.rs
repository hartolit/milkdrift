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
    assert!(projection.branch_routes().is_empty());
    let selected = PortId::new("true")?;
    assert!(harness.runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::BranchRouteSelected { selected_port, .. }
            if selected_port == &selected
    )));
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
    assert!(projection.branches().is_empty());
    assert!(projection.joins().is_empty());
    let history = harness.runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::BranchTerminal {
            outcome: RunOutcome::Succeeded,
            ..
        }
    )));
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::BranchTerminal {
            outcome: RunOutcome::Failed,
            ..
        }
    )));
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::JoinSatisfied { branches, retained_branches, .. }
            if branches.len() == 2 && retained_branches.is_empty()
    )));
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
    let history = harness.runtime.history(&run)?;
    let outer_a = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::BranchScopeCreated {
                fork_execution,
                port,
                branch,
                ..
            } if fork_execution == outer_fork.execution() && port == &outer_a_port => {
                Some(branch.clone())
            }
            _ => None,
        })
        .ok_or("outer a branch history is absent")?;
    let (outer_terminal_sequence, outer_outputs) = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::BranchTerminal {
                branch,
                outcome: RunOutcome::Succeeded,
                outputs,
            } if branch == &outer_a => Some((event.sequence(), outputs.clone())),
            _ => None,
        })
        .ok_or("outer a terminal fact is absent")?;
    assert_eq!(outer_outputs.len(), 1);
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::JoinSatisfied { branches, .. }
            if branches.iter().any(|result| {
                result.branch == outer_a && result.outputs == outer_outputs
            })
    )));
    let inner_join_execution = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::NodeBecameEligible {
                node, execution, ..
            } if node.as_str() == "inner-join" => Some(execution.clone()),
            _ => None,
        })
        .ok_or("inner join execution history is absent")?;
    let inner_join_sequence = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::JoinSatisfied { execution, .. } if execution == &inner_join_execution => {
                Some(event.sequence())
            }
            _ => None,
        })
        .ok_or("inner join result history is absent")?;
    let tail_execution = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::NodeBecameEligible {
                node, execution, ..
            } if node.as_str() == "outer-a-tail" => Some(execution.clone()),
            _ => None,
        })
        .ok_or("outer a successor history is absent")?;
    let tail_terminal_sequence = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::NodeTerminal { execution, .. } if execution == &tail_execution => {
                Some(event.sequence())
            }
            _ => None,
        })
        .ok_or("outer a successor terminal fact is absent")?;
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
        assert!(projection.branches().is_empty());
        let history = harness.runtime.history(&run)?;
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::BranchTerminal { .. }))
                .count(),
            2
        );
        if suffix == "mixed" {
            assert!(history.iter().any(|event| matches!(
                event.kind(),
                RunEventKind::BranchTerminal {
                    outcome: RunOutcome::Failed,
                    ..
                }
            )));
            assert!(history.iter().any(|event| matches!(
                event.kind(),
                RunEventKind::BranchTerminal {
                    outcome: RunOutcome::Succeeded,
                    ..
                }
            )));
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
    assert!(projection.joins().is_empty());
    assert!(projection.branches().is_empty());
    let history = harness.runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::BranchTerminal {
            outcome: RunOutcome::Succeeded,
            ..
        }
    )));
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::BranchTerminal {
            outcome: RunOutcome::Cancelled,
            ..
        }
    )));
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
        assert!(projection.branches().is_empty());
        assert!(projection.joins().is_empty());
        let history = harness.runtime.history(&run)?;
        assert!(history.iter().any(|event| matches!(
            event.kind(),
            RunEventKind::BranchTerminal {
                outcome: RunOutcome::Succeeded,
                ..
            }
        )));
        assert!(history.iter().any(|event| matches!(
            event.kind(),
            RunEventKind::BranchTerminal {
                outcome: RunOutcome::Cancelled,
                ..
            }
        )));
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
        assert!(projection.branches().is_empty());
        assert!(projection.joins().is_empty());
        let root_scope = projection
            .root_scope()
            .ok_or("reducer run has no root scope")?
            .reference();
        let history = harness.runtime.history(&run)?;
        let mut sibling_output_scopes = BTreeSet::new();
        for task_id in [NodeId::new("a-task")?, NodeId::new("b-task")?] {
            let (task_execution, task_scope) = history
                .iter()
                .find_map(|event| match event.kind() {
                    RunEventKind::NodeBecameEligible {
                        node,
                        execution,
                        scope,
                        ..
                    } if node == &task_id => Some((execution.clone(), scope.clone())),
                    _ => None,
                })
                .ok_or("branch task execution was not created")?;
            assert_ne!(&task_scope, root_scope);
            assert!(history.iter().any(|event| matches!(
                event.kind(),
                RunEventKind::NodeOutputPublished { execution, value, .. }
                    if execution == &task_execution && value.scope() == &task_scope
            )));
            assert!(sibling_output_scopes.insert(task_scope));
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
        let mut branches: Vec<_> = history
            .iter()
            .filter_map(|event| match event.kind() {
                RunEventKind::BranchScopeCreated { port, branch, .. } => {
                    Some((port.clone(), branch.clone()))
                }
                _ => None,
            })
            .collect();
        branches.sort();
        let branch_outputs: Vec<_> = branches
            .iter()
            .flat_map(|(_, branch)| {
                history.iter().find_map(|event| match event.kind() {
                    RunEventKind::BranchTerminal {
                        branch: terminal,
                        outputs,
                        ..
                    } if terminal == branch => Some(outputs.clone()),
                    _ => None,
                })
            })
            .flatten()
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
    assert_eq!(runtime_tick(&harness.runtime)?.dispatched, 0);
    harness.clock.advance(99)?;
    assert_eq!(runtime_tick(&harness.runtime)?.dispatched, 0);
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Running
    );

    harness.clock.advance(1)?;
    runtime_tick(&harness.runtime)?;
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
    runtime_tick(&harness.runtime)?;
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
    let accepted = harness
        .runtime
        .handle_authorized_command(&command, &test_authority_claim()?)?;
    assert!(!accepted.replayed());
    assert_eq!(
        accepted.result().disposition(),
        CommandDisposition::Accepted
    );
    assert_eq!(accepted.result().event_ids().len(), 1);
    let replayed = harness
        .runtime
        .handle_authorized_command(&command, &test_authority_claim()?)?;
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
        runtime_tick(&harness.runtime)?;
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
    assert_eq!(runtime_tick(&harness.runtime)?.dispatched, 1);
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
    assert!(projection.subworkflows().is_empty());
    let history = harness.runtime.history(&run)?;
    let (subworkflow, child_run) = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::SubworkflowCreated {
                subworkflow,
                child_run,
                child_revision,
                ..
            } if child_revision == child.id() => Some((subworkflow.clone(), child_run.clone())),
            _ => None,
        })
        .ok_or("parent child-link history is absent")?;
    let (child_value, parent_value) = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::SubworkflowOutputImported {
                subworkflow: imported,
                child_value,
                parent_value,
            } if imported == &subworkflow => Some((child_value.clone(), parent_value.clone())),
            _ => None,
        })
        .ok_or("parent child import history is absent")?;
    assert_eq!(child_value.scope().run(), &child_run);
    assert_eq!(parent_value.scope().run(), &run);
    let parent_entry = harness
        .store
        .value(&parent_value)?
        .ok_or("imported child output is absent from the parent workspace")?;
    match parent_entry.origin() {
        ValueOrigin::Imported { source } => assert_eq!(source, &child_value),
        origin => return Err(format!("expected imported value origin, found {origin:?}").into()),
    }
    let child_projection = harness.runtime.projection(&child_run)?;
    assert_eq!(child_projection.revision(), Some(child.id()));
    assert_eq!(
        child_projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    Ok(())
}

#[test]
fn attached_subworkflow_refuses_a_preexisting_foreign_child_run() -> TestResult {
    let collision = RunId::new("run-foreign-child-collision")?;
    let harness = Harness::with_child_run_collision("subworkflow-collision", collision.clone())?;
    let foreign = task_revision("workflow-foreign-child")?;
    let child = output_child_revision("workflow-intended-child")?;
    let parent = subworkflow_revision("workflow-collision-parent", &child)?;
    harness.put_revision(&foreign)?;
    harness.put_revision(&child)?;
    harness.put_revision(&parent)?;
    harness.create(&collision, &foreign)?;
    let foreign_head = harness.store.head(&collision)?;

    let parent_run = RunId::new("run-collision-parent")?;
    harness.create_and_start(&parent_run, &parent)?;
    assert_eq!(
        harness
            .runtime
            .projection(&parent_run)?
            .subworkflows()
            .len(),
        1
    );
    assert!(matches!(
        harness.runtime.scheduler_tick(),
        Err(RuntimeError::InvalidHistory(_))
    ));
    assert_eq!(harness.store.head(&collision)?, foreign_head);
    let foreign_projection = harness.runtime.projection(&collision)?;
    assert_eq!(
        foreign_projection.workflow(),
        Some(foreign.semantic().workflow())
    );
    assert_eq!(foreign_projection.revision(), Some(foreign.id()));
    assert!(
        harness
            .runtime
            .history(&parent_run)?
            .iter()
            .all(|event| { !matches!(event.kind(), RunEventKind::SubworkflowTerminal { .. }) })
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
    assert!(projection.iterations().is_empty());
    assert!(projection.repeat_terminations().is_empty());
    assert!(projection.subworkflows().is_empty());
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
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::RepeatTerminated {
            termination: RepeatTerminationReason::MaximumIterations,
            ..
        }
    )));
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
    let approved = harness
        .runtime
        .handle_authorized_command(&approval, &test_authority_claim()?)?;
    assert_eq!(
        approved.result().disposition(),
        CommandDisposition::Accepted
    );
    assert!(!approved.replayed());
    let approved_head = harness.store.head(&run)?;
    let replayed = harness
        .runtime
        .handle_authorized_command(&approval, &test_authority_claim()?)?;
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
    assert!(rejected.iterations().is_empty());
    assert!(rejected.subworkflows().is_empty());
    assert!(rejected.repeat_continuations().is_empty());
    assert!(rejected.repeat_terminations().is_empty());
    let history = harness.runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::RepeatContinuationDecided { .. }))
            .count(),
        2
    );
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::RepeatTerminated {
            termination: RepeatTerminationReason::MaximumIterations,
            ..
        }
    )));
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::SubworkflowCreated { .. }))
            .count(),
        3
    );
    Ok(())
}
