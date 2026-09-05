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

#[path = "structured_graph/continuations.rs"]
mod continuations;
