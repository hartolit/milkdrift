//! Reconciliation integration scenarios.

use super::*;

struct OperationCountingExecutor {
    resolver: DeterministicExecutor,
    calls: Mutex<BTreeMap<OperationId, usize>>,
}

impl OperationCountingExecutor {
    fn new(descriptor: CapabilityDescriptor) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            calls: Mutex::new(BTreeMap::new()),
        }
    }

    fn calls(&self, operation: &str) -> TestResult<usize> {
        Ok(*self
            .calls
            .lock()
            .map_err(|_| "operation-count lock poisoned")?
            .get(&OperationId::new(operation)?)
            .unwrap_or(&0))
    }
}

impl TaskExecutor for OperationCountingExecutor {
    delegate_resolve!(resolver);

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        let prepared = self.resolver.prepare_exact_entry(dispatch)?;
        let envelope = prepared.admission_envelope().clone();
        Ok(PreparedExecution::new(
            dispatch,
            envelope,
            move |dispatch, reporter| {
                let mut calls = self.calls.lock().map_err(|_| {
                    ExecutorError::Boundary("operation-count lock poisoned".to_owned())
                })?;
                *calls
                    .entry(dispatch.request().operation().clone())
                    .or_default() += 1;
                drop(calls);
                prepared.enter(dispatch, reporter)
            },
        ))
    }

    delegate_cancel!(resolver);
}

fn changed_retired_task_revision(
    base: &BlueprintRevision,
    operation: &str,
) -> TestResult<BlueprintRevision> {
    Ok(base.revise(
        base.id(),
        MutationBatch::new(vec![Mutation::ReplaceNode {
            node: task("retired", operation)?,
        }])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "change the completed task operation for remediation",
    )?)
}

fn descriptor_with_distinct_operation_side_effects(
    generate: &str,
    fail: &str,
) -> TestResult<CapabilityDescriptor> {
    let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    let operations = value
        .get_mut("descriptor")
        .and_then(|descriptor| descriptor.get_mut("operations"))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("descriptor fixture has no operations object")?;
    let mut remediation = operations
        .get("model.generate")
        .cloned()
        .ok_or("descriptor fixture has no model.generate operation")?;
    operations
        .get_mut("model.generate")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("model.generate operation is not an object")?
        .insert(
            "side_effect".to_owned(),
            serde_json::Value::String(generate.to_owned()),
        );
    remediation
        .as_object_mut()
        .ok_or("remediation operation is not an object")?
        .insert(
            "side_effect".to_owned(),
            serde_json::Value::String(fail.to_owned()),
        );
    operations.insert("model.fail".to_owned(), remediation);
    Ok(
        CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&value)?)?
            .body()
            .clone(),
    )
}

#[test]
fn reconciliation_guards_reject_each_invalid_state_independently() -> TestResult {
    let harness = Harness::new("reconciliation-guards")?;
    let old = wait_revision("workflow-reconciliation-guards", 5_000)?;
    let new = revised_wait_revision(&old, 7_500)?;
    harness.put_revision(&old)?;
    harness.put_revision(&new)?;

    let created_run = RunId::new("run-reconciliation-guard-created")?;
    harness.create(&created_run, &old)?;
    assert_eq!(
        harness.command(
            &created_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-created")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Rejected
    );

    let failing_harness = Harness::with_plan_id_failure("reconciliation-plan-id-failure")?;
    failing_harness.put_revision(&old)?;
    failing_harness.put_revision(&new)?;
    let failing_run = RunId::new("run-reconciliation-guard-plan-id-failure")?;
    failing_harness.create_and_start(&failing_run, &old)?;
    let before_failure = failing_harness.store.head(&failing_run)?;
    assert!(
        failing_harness
            .command(
                &failing_run,
                RunCommand::RequestRevisionAdoption {
                    reconciliation: ReconciliationId::new("reconciliation-plan-id-failure")?,
                    revision: new.id().clone(),
                    policy: ReconciliationPolicy::FinishCurrentThenAdopt,
                },
            )
            .is_err()
    );
    assert_eq!(failing_harness.store.head(&failing_run)?, before_failure);
    assert!(
        failing_harness
            .runtime
            .projection(&failing_run)?
            .reconciliation()
            .plans()
            .is_empty()
    );

    let active_run = RunId::new("run-reconciliation-guard-active")?;
    harness.create_and_start(&active_run, &old)?;
    assert_eq!(
        harness.command(
            &active_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-active-first")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(
            &active_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-active-second")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Rejected
    );
    let active_plan = harness
        .runtime
        .projection(&active_run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("active reconciliation plan is absent")?
        .plan()
        .clone();
    assert_eq!(
        harness.command(
            &active_run,
            RunCommand::DecideReconciliation {
                plan: active_plan.clone(),
                decision: ReconciliationDecisionId::new("decision-invalid-outcome")?,
                outcome: AuthorityDecision::Retain,
            },
        )?,
        CommandDisposition::Rejected
    );
    let reused_decision = ReconciliationDecisionId::new("decision-reused")?;
    assert_eq!(
        harness.command(
            &active_run,
            RunCommand::DecideReconciliation {
                plan: active_plan.clone(),
                decision: reused_decision.clone(),
                outcome: AuthorityDecision::Approve,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(
            &active_run,
            RunCommand::DecideReconciliation {
                plan: active_plan.clone(),
                decision: reused_decision,
                outcome: AuthorityDecision::Approve,
            },
        )?,
        CommandDisposition::Rejected
    );
    assert_eq!(
        harness.command(
            &active_run,
            RunCommand::ApplyReconciliation {
                plan: active_plan.clone(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(
            &active_run,
            RunCommand::DecideReconciliation {
                plan: active_plan,
                decision: ReconciliationDecisionId::new("decision-after-application")?,
                outcome: AuthorityDecision::Approve,
            },
        )?,
        CommandDisposition::Rejected
    );

    let rejected_run = RunId::new("run-reconciliation-guard-rejected")?;
    harness.create_and_start(&rejected_run, &old)?;
    assert_eq!(
        harness.command(
            &rejected_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-rejected")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let rejected_plan = harness
        .runtime
        .projection(&rejected_run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("reject-path reconciliation plan is absent")?
        .plan()
        .clone();
    assert_eq!(
        harness.command(
            &rejected_run,
            RunCommand::DecideReconciliation {
                plan: rejected_plan.clone(),
                decision: ReconciliationDecisionId::new("decision-reject")?,
                outcome: AuthorityDecision::Reject,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(
            &rejected_run,
            RunCommand::ApplyReconciliation {
                plan: rejected_plan,
            },
        )?,
        CommandDisposition::Rejected
    );
    Ok(())
}

#[test]
fn prospective_revision_adoption_is_persisted_actionable_and_stale_safe() -> TestResult {
    let harness = Harness::new("adoption")?;
    let old = wait_revision("workflow-adoption", 5_000)?;
    let new = revised_wait_revision(&old, 7_500)?;
    harness.put_revision(&old)?;
    harness.put_revision(&new)?;

    let adopted_run = RunId::new("run-adoption-applied")?;
    harness.create_and_start(&adopted_run, &old)?;
    assert_eq!(
        harness.command(
            &adopted_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-applied")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let planned = harness.runtime.projection(&adopted_run)?;
    let plan = planned
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("adoption plan was not persisted")?;
    assert!(plan.items().iter().any(|item| {
        item.node.as_ref() == NodeId::new("wait").ok().as_ref()
            && item.classification == ReconciliationClassification::ChangedActive
            && item.action == ReconciliationAction::UseNewOnNextInvocation
    }));
    let plan_id = plan.plan().clone();
    assert_eq!(
        harness.command(
            &adopted_run,
            RunCommand::DecideReconciliation {
                plan: plan_id.clone(),
                decision: ReconciliationDecisionId::new("decision-approve-adoption")?,
                outcome: AuthorityDecision::Approve,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(
            &adopted_run,
            RunCommand::ApplyReconciliation {
                plan: plan_id.clone(),
            },
        )?,
        CommandDisposition::Accepted
    );
    let applied = harness.runtime.projection(&adopted_run)?;
    assert_eq!(applied.revision(), Some(new.id()));
    assert!(
        applied
            .reconciliation()
            .plans()
            .get(&plan_id)
            .is_some_and(|plan| plan.applied_sequence().is_some())
    );

    let stale_run = RunId::new("run-adoption-stale")?;
    harness.create_and_start(&stale_run, &old)?;
    assert_eq!(
        harness.command(
            &stale_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-stale")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let stale_plan = harness
        .runtime
        .projection(&stale_run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("stale test plan was not persisted")?
        .plan()
        .clone();
    assert_eq!(
        harness.command(&stale_run, RunCommand::PauseRun)?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(
            &stale_run,
            RunCommand::ApplyReconciliation {
                plan: stale_plan.clone(),
            },
        )?,
        CommandDisposition::Rejected
    );
    let stale = harness.runtime.projection(&stale_run)?;
    assert_eq!(stale.revision(), Some(old.id()));
    assert!(
        stale
            .reconciliation()
            .plans()
            .get(&stale_plan)
            .is_some_and(|plan| plan.stale_sequence().is_some())
    );
    Ok(())
}

#[test]
fn runtime_owned_structured_work_is_never_planned_as_unstarted_removal_or_attempt_restart()
-> TestResult {
    {
        let harness = Harness::new("reconcile-active-wait-change")?;
        let old = wait_revision("workflow-reconcile-active-wait-change", 60_000)?;
        let new = revised_wait_revision(&old, 120_000)?;
        let run = RunId::new("run-reconcile-active-wait-change")?;
        harness.put_revision(&old)?;
        harness.put_revision(&new)?;
        harness.create_and_start(&run, &old)?;
        assert!(
            harness
                .runtime
                .projection(&run)?
                .waits()
                .values()
                .any(|wait| wait.is_pending())
        );
        assert_eq!(
            harness.command(
                &run,
                RunCommand::RequestRevisionAdoption {
                    reconciliation: ReconciliationId::new("reconcile-active-wait-change")?,
                    revision: new.id().clone(),
                    policy: ReconciliationPolicy::CancelAndRestartSafeWork,
                },
            )?,
            CommandDisposition::Accepted
        );
        let projection = harness.runtime.projection(&run)?;
        let plan = projection
            .reconciliation()
            .plans()
            .values()
            .next()
            .ok_or("active wait change plan is absent")?;
        assert!(plan.items().iter().any(|item| {
            item.node.as_ref() == NodeId::new("wait").ok().as_ref()
                && item.classification == ReconciliationClassification::ChangedActive
                && item.action == ReconciliationAction::RejectRetrospectiveRewrite
        }));
        assert_eq!(
            harness.command(
                &run,
                RunCommand::ApplyReconciliation {
                    plan: plan.plan().clone(),
                },
            )?,
            CommandDisposition::Rejected
        );
    }

    {
        let harness = Harness::new("reconcile-active-wait-remove")?;
        let old = wait_revision("workflow-reconcile-active-wait-remove", 60_000)?;
        let new = revision_without_entry_node(&old, "wait", &["wait-done"])?;
        let run = RunId::new("run-reconcile-active-wait-remove")?;
        harness.put_revision(&old)?;
        harness.put_revision(&new)?;
        harness.create_and_start(&run, &old)?;
        assert_eq!(
            harness.command(
                &run,
                RunCommand::RequestRevisionAdoption {
                    reconciliation: ReconciliationId::new("reconcile-active-wait-remove")?,
                    revision: new.id().clone(),
                    policy: ReconciliationPolicy::RemoveUnstartedOnly,
                },
            )?,
            CommandDisposition::Accepted
        );
        let projection = harness.runtime.projection(&run)?;
        let plan = projection
            .reconciliation()
            .plans()
            .values()
            .next()
            .ok_or("active wait removal plan is absent")?;
        assert!(plan.items().iter().any(|item| {
            item.node.as_ref() == NodeId::new("wait").ok().as_ref()
                && item.classification == ReconciliationClassification::ChangedActive
                && item.action == ReconciliationAction::RejectRetrospectiveRewrite
        }));
        assert!(!plan.items().iter().any(|item| {
            item.node.as_ref() == NodeId::new("wait").ok().as_ref()
                && item.action == ReconciliationAction::RemoveUnstarted
        }));
    }

    for (suffix, repeat) in [("subworkflow", false), ("repeat", true)] {
        let child = wait_revision(&format!("workflow-{suffix}-child"), 60_000)?;
        let old = if repeat {
            repeat_revision(&format!("workflow-active-{suffix}"), &child)?
        } else {
            subworkflow_revision(&format!("workflow-active-{suffix}"), &child)?
        };
        let node = if repeat { "repeat" } else { "child" };
        let edge = if repeat { "repeat-done" } else { "child-done" };
        let new = revision_without_entry_node(&old, node, &[edge])?;
        let harness = Harness::new(&format!("reconcile-active-{suffix}"))?;
        let run = RunId::new(format!("run-reconcile-active-{suffix}"))?;
        harness.put_revision(&child)?;
        harness.put_revision(&old)?;
        harness.put_revision(&new)?;
        harness.create_and_start(&run, &old)?;
        let active = harness.runtime.projection(&run)?;
        assert!(
            active
                .subworkflows()
                .values()
                .any(|child| child.is_active()),
            "{suffix} fixture did not retain active child ownership"
        );
        assert_eq!(
            harness.command(
                &run,
                RunCommand::RequestRevisionAdoption {
                    reconciliation: ReconciliationId::new(format!("reconcile-active-{suffix}"))?,
                    revision: new.id().clone(),
                    policy: ReconciliationPolicy::RemoveUnstartedOnly,
                },
            )?,
            CommandDisposition::Accepted
        );
        let projection = harness.runtime.projection(&run)?;
        let plan = projection
            .reconciliation()
            .plans()
            .values()
            .next()
            .ok_or("structured active removal plan is absent")?;
        assert!(plan.items().iter().any(|item| {
            item.node.as_ref() == NodeId::new(node).ok().as_ref()
                && item.classification == ReconciliationClassification::ChangedActive
                && item.action == ReconciliationAction::RejectRetrospectiveRewrite
        }));
    }
    Ok(())
}

#[test]
fn active_branch_frontier_does_not_capture_unowned_post_join_pending_work() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new_with_authority(
        store.clone(),
        executor.clone(),
        test_authority(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("reconcile-branch-frontier", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-reconcile-branch-frontier")?,
            ActorRef::new("controller:reconcile-branch-frontier")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?);
    let old = fork_revision_with_post_join_task("workflow-active-branch-frontier")?;
    let new = revision_without_post_join_task(&old)?;
    let run = RunId::new("run-reconcile-active-branch-frontier")?;
    store.put_revision(&old)?;
    store.put_revision(&new)?;
    assert_eq!(
        submit_command(
            runtime.as_ref(),
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: old.semantic().workflow().clone(),
                revision: old.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-reconcile-active-branch-frontier")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(runtime.as_ref(), store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    let blocked_runtime = runtime.clone();
    let blocked = std::thread::spawn(move || {
        runtime_tick(&blocked_runtime)
            .map_err(|error| format!("blocked branch tick failed: {error}"))
    });
    executor.wait_until_entered()?;
    assert_eq!(runtime_tick(&runtime)?.dispatched, 1);

    let active = runtime.projection(&run)?;
    assert!(active.branches().values().any(|branch| branch.is_active()));
    let owned_before: Vec<_> = active
        .branches()
        .values()
        .filter(|branch| branch.is_active())
        .map(|branch| {
            (
                branch.branch().clone(),
                branch.fork_execution().clone(),
                branch.children().clone(),
                branch.state(),
            )
        })
        .collect();
    assert_eq!(
        active
            .executions_for_node(&NodeId::new("independent")?)
            .next()
            .map(|execution| execution.state()),
        Some(&NodeExecutionState::Eligible)
    );

    assert_eq!(
        submit_command(
            runtime.as_ref(),
            store.as_ref(),
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconcile-branch-frontier")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::RemoveUnstartedOnly,
            },
        )?,
        CommandDisposition::Accepted
    );
    let projection = runtime.projection(&run)?;
    let plan = projection
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("branch-frontier plan is absent")?;
    assert!(plan.items().iter().any(|item| {
        item.node.as_ref() == NodeId::new("independent").ok().as_ref()
            && item.classification == ReconciliationClassification::RemovedPending
            && item.action == ReconciliationAction::RemoveUnstarted
    }));
    let removed_execution = plan
        .items()
        .iter()
        .find(|item| item.node.as_ref() == NodeId::new("independent").ok().as_ref())
        .and_then(|item| item.execution.clone())
        .ok_or("removed independent execution identity is absent")?;
    let plan_id = plan.plan().clone();
    assert_eq!(
        submit_command(
            runtime.as_ref(),
            store.as_ref(),
            &run,
            RunCommand::ApplyReconciliation { plan: plan_id },
        )?,
        CommandDisposition::Accepted,
        "reconciliation items: {:?}",
        plan.items()
    );
    let applied = runtime.projection(&run)?;
    assert_eq!(applied.revision(), Some(new.id()));
    assert_eq!(
        applied
            .executions_for_node(&NodeId::new("independent")?)
            .next()
            .map(|execution| execution.state()),
        None,
        "prospectively removed work absent from the pinned revision must retire from active state"
    );
    assert!(runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::ReconciliationExecutionRemoved { execution, .. }
            if execution == &removed_execution
    )));
    for (branch, fork, children, state) in owned_before {
        let after = applied
            .branches()
            .get(&branch)
            .ok_or("active branch ownership disappeared during adoption")?;
        assert_eq!(after.fork_execution(), &fork);
        assert_eq!(after.children(), &children);
        assert_eq!(after.state(), state);
        assert!(after.is_active());
    }
    assert!(
        applied
            .attempts()
            .values()
            .any(|attempt| attempt.is_active())
    );
    runtime_tick(&runtime)?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    executor.release()?;
    blocked
        .join()
        .map_err(|_| "blocked branch tick panicked")??;
    Ok(())
}

#[test]
fn revision_adoption_materializes_a_new_root_entry_exactly_once() -> TestResult {
    let harness = Harness::new("adoption-added-root")?;
    let old = wait_revision("workflow-adoption-added-root", 60_000)?;
    let new = revision_with_added_root_wait(&old, 60_000)?;
    let run = RunId::new("run-adoption-added-root")?;
    harness.put_revision(&old)?;
    harness.put_revision(&new)?;
    harness.create_and_start(&run, &old)?;
    assert_eq!(
        harness.command(
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-added-root")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let plan = harness
        .runtime
        .projection(&run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("added-root adoption plan is absent")?
        .plan()
        .clone();
    assert_eq!(
        harness.command(&run, RunCommand::ApplyReconciliation { plan })?,
        CommandDisposition::Accepted
    );
    let added = NodeId::new("added-root")?;
    assert_eq!(
        harness
            .runtime
            .projection(&run)?
            .executions_for_node(&added)
            .count(),
        1
    );
    runtime_tick(&harness.runtime)?;
    runtime_tick(&harness.runtime)?;
    assert_eq!(
        harness
            .runtime
            .projection(&run)?
            .executions_for_node(&added)
            .count(),
        1,
        "structured driving must not duplicate an adopted root entry"
    );
    Ok(())
}

#[path = "reconciliation/retired_history.rs"]
mod retired_history;

#[path = "reconciliation/cancellation.rs"]
mod cancellation;
