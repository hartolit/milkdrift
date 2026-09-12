use super::*;

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
fn attached_child_refuses_a_different_creation_pin_in_the_same_workflow() -> TestResult {
    let original = wait_revision("workflow-child-collision-pin", 5_000)?;
    let different = original.revise(
        original.id(),
        MutationBatch::new(vec![Mutation::SetMetadata {
            metadata: milkdrift_blueprint::BlueprintMetadata::new(
                "Different creation pin",
                "Same workflow and executable shape",
                BTreeSet::new(),
                BTreeMap::new(),
            )?,
        }])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "A different immutable creation revision",
    )?;
    let parent = subworkflow_revision("workflow-parent-collision-pin", &original)?;
    let run = RunId::new("run-parent-collision-pin")?;
    // A separate valid materialization supplies the exact workspace identity. The refusal
    // fixture must differ only in creation revision, not workflow, budget, inputs, or scope.
    let (expected_child, expected_root) = {
        let reference = Harness::new("child-collision-pin")?;
        reference.put_revision(&original)?;
        reference.put_revision(&parent)?;
        reference.create_and_start(&run, &parent)?;
        reference.runtime.scheduler_tick()?;
        let projection = reference.runtime.projection(&run)?;
        let child = projection
            .subworkflows()
            .values()
            .next()
            .ok_or("reference child link absent")?
            .child_run()
            .clone();
        let root = reference
            .runtime
            .projection(&child)?
            .root_scope()
            .ok_or("reference child root absent")?
            .clone();
        (child, root)
    };
    let harness = Harness::new("child-collision-pin")?;
    for revision in [&original, &different, &parent] {
        harness.put_revision(revision)?;
    }
    harness.create_and_start(&run, &parent)?;
    let projection = harness.runtime.projection(&run)?;
    let child = projection
        .subworkflows()
        .values()
        .next()
        .ok_or("child link absent")?;
    assert_eq!(child.child_run(), &expected_child);
    assert_eq!(
        harness.command(
            &expected_child,
            RunCommand::CreateRun {
                workflow: different.semantic().workflow().clone(),
                revision: different.id().clone(),
                root_scope: expected_root,
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    let before = harness.store.head(&expected_child)?;
    assert!(matches!(
        harness.runtime.scheduler_tick(),
        Err(RuntimeError::InvalidHistory(_))
    ));
    assert_eq!(harness.store.head(&expected_child)?, before);
    assert_eq!(
        harness.runtime.projection(&expected_child)?.revision(),
        Some(different.id())
    );
    Ok(())
}

#[test]
fn attached_child_keeps_its_creation_pin_after_authorized_prospective_revision() -> TestResult {
    let harness = Harness::new("child-prospective-pin")?;
    let original = wait_revision("workflow-child-prospective", 5_000)?;
    let revised = original.revise(
        original.id(),
        MutationBatch::new(vec![Mutation::SetMetadata {
            metadata: milkdrift_blueprint::BlueprintMetadata::new(
                "Revised child",
                "The entered wait is unchanged",
                BTreeSet::new(),
                BTreeMap::new(),
            )?,
        }])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "Prospective metadata revision",
    )?;
    let parent = subworkflow_revision("workflow-parent-prospective", &original)?;
    for revision in [&original, &revised, &parent] {
        harness.put_revision(revision)?;
    }
    let run = RunId::new("run-parent-prospective")?;
    harness.create_and_start(&run, &parent)?;
    harness.runtime.scheduler_tick()?;
    let projection = harness.runtime.projection(&run)?;
    let child = projection
        .subworkflows()
        .values()
        .next()
        .ok_or("child link absent")?
        .child_run()
        .clone();
    assert_eq!(
        harness.command(
            &child,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("child-prospective")?,
                revision: revised.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            }
        )?,
        CommandDisposition::Accepted
    );
    let projection = harness.runtime.projection(&child)?;
    let plan = projection
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("child plan absent")?
        .plan()
        .clone();
    assert_eq!(
        harness.command(
            &child,
            RunCommand::DecideReconciliation {
                plan: plan.clone(),
                decision: ReconciliationDecisionId::new("approve-child-prospective")?,
                outcome: AuthorityDecision::Approve,
            }
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(&child, RunCommand::ApplyReconciliation { plan })?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.runtime.projection(&child)?.revision(),
        Some(revised.id())
    );
    harness.runtime.scheduler_tick()?;
    harness.clock.advance(5_000)?;
    harness.drive(&run, 12)?;
    assert!(harness.runtime.projection(&run)?.is_completed());
    assert!(
        matches!(harness.runtime.history(&child)?.first().map(RunEventEnvelope::kind), Some(RunEventKind::RunCreated { revision, .. }) if revision == original.id())
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
