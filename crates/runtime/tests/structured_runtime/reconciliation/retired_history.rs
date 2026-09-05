use super::*;

#[test]
fn remediation_survives_reopen_and_dispatches_only_the_target_revision_operation() -> TestResult {
    let directory = TempDir::new()?;
    let descriptor = descriptor_with_model_side_effect("non_idempotent_write")?;
    let executor = Arc::new(OperationCountingExecutor::new(descriptor));
    let old = removable_task_revision("workflow-remediation-target-revision")?;
    let new = changed_retired_task_revision(&old, "model.fail")?;
    let run = RunId::new("run-remediation-target-revision")?;

    let (store, clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "remediation-target-before",
        "remediation-target-worker",
        NOW,
        64,
        executor.clone(),
    )?;
    store.put_revision(&old)?;
    store.put_revision(&new)?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: old.semantic().workflow().clone(),
                revision: old.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-remediation-target-revision")?,
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
    assert_eq!(runtime_tick(&runtime)?.completed, 1);
    assert_eq!(executor.calls("model.generate")?, 1);
    assert_eq!(executor.calls("model.fail")?, 0);

    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-remediation-target")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::CompensateOrRemediate,
            },
        )?,
        CommandDisposition::Accepted
    );
    let plan = runtime
        .projection(&run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("remediation target plan is absent")?
        .plan()
        .clone();
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::ApplyReconciliation { plan },
        )?,
        CommandDisposition::Accepted
    );
    let applied = runtime.projection(&run)?;
    let remediation = applied
        .node_executions()
        .values()
        .find(|execution| {
            execution.node().as_str() == "retired"
                && execution.state() == &NodeExecutionState::Eligible
        })
        .ok_or("eligible remediation execution is absent")?;
    assert_eq!(remediation.revision(), new.id());
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    drop(applied);
    drop(runtime);
    drop(clock);
    drop(store);

    let (store, clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "remediation-target-after",
        "remediation-target-worker",
        NOW,
        64,
        executor.clone(),
    )?;
    let actions = runtime.claim_effects(PageSize::new(1)?)?;
    let action = actions
        .into_iter()
        .next()
        .ok_or("reopened remediation action is absent")?;
    let dispatch = match &action {
        EffectAction::Execute(dispatch) => dispatch,
        _ => return Err("reopened remediation did not yield exactly one execution".into()),
    };
    assert_eq!(dispatch.revision(), new.id());
    assert_eq!(
        dispatch.request().operation(),
        &OperationId::new("model.fail")?
    );
    assert!(matches!(
        runtime.execute_effect(action)?,
        EffectExecutionResult::Completed { .. }
    ));
    assert_eq!(executor.calls("model.generate")?, 1);
    assert_eq!(executor.calls("model.fail")?, 1);
    drop(runtime);
    drop(clock);
    drop(store);

    let (_store, _clock, reopened) = runtime_with_executor_at(
        directory.path(),
        "remediation-target-final",
        "remediation-target-worker",
        NOW,
        64,
        executor.clone(),
    )?;
    assert!(reopened.claim_effects(PageSize::new(1)?)?.is_empty());
    assert_eq!(executor.calls("model.generate")?, 1);
    assert_eq!(executor.calls("model.fail")?, 1);
    Ok(())
}

#[test]
fn compacted_remediation_cannot_downgrade_its_source_side_effect_risk() -> TestResult {
    let directory = TempDir::new()?;
    let descriptor =
        descriptor_with_distinct_operation_side_effects("non_idempotent_write", "none")?;
    let executor = Arc::new(OperationCountingExecutor::new(descriptor));
    let first = removable_task_revision("workflow-remediation-risk")?;
    let second = changed_retired_task_revision(&first, "model.fail")?;
    let third = revision_without_completed_task(&second)?;
    let run = RunId::new("run-remediation-risk")?;
    let (store, clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "remediation-risk-before",
        "remediation-risk-worker",
        NOW,
        64,
        executor.clone(),
    )?;
    store.put_revision(&first)?;
    store.put_revision(&second)?;
    store.put_revision(&third)?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: first.semantic().workflow().clone(),
                revision: first.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-remediation-risk")?,
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
    assert_eq!(runtime_tick(&runtime)?.completed, 1);
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("remediation-risk-second")?,
                revision: second.id().clone(),
                policy: ReconciliationPolicy::CompensateOrRemediate,
            },
        )?,
        CommandDisposition::Accepted
    );
    let plan = runtime
        .projection(&run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("remediation-risk plan is absent")?
        .plan()
        .clone();
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::ApplyReconciliation { plan },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(runtime_tick(&runtime)?.completed, 1);
    assert_eq!(executor.calls("model.generate")?, 1);
    assert_eq!(executor.calls("model.fail")?, 1);

    let mut lifecycle_transitions = 0_u64;
    while store.head(&run)?.get() % 128 != 0 {
        lifecycle_transitions += 1;
        let command = match runtime.projection(&run)?.lifecycle() {
            RunLifecycle::Running => RunCommand::PauseRun,
            RunLifecycle::Paused => RunCommand::ResumeRun,
            lifecycle => {
                return Err(format!(
                    "remediation-risk run cannot checkpoint from lifecycle {lifecycle:?}"
                )
                .into());
            }
        };
        assert_eq!(
            submit_command(&runtime, store.as_ref(), &run, command)?,
            CommandDisposition::Accepted
        );
    }
    let snapshot = store.latest_snapshot(&run)?;
    assert!(
        matches!(snapshot, milkdrift_persistence::SnapshotLoad::Verified(_)),
        "remediation-risk checkpoint was not verified at head {} after {lifecycle_transitions} lifecycle transitions: {snapshot:?}",
        store.head(&run)?.get()
    );
    drop(runtime);
    drop(clock);
    drop(store);

    let (store, _clock, reopened) = runtime_with_executor_at(
        directory.path(),
        "remediation-risk-after",
        "remediation-risk-worker",
        NOW,
        64,
        executor,
    )?;
    let settled = reopened
        .projection(&run)?
        .settled_node_executions()
        .values()
        .find(|execution| execution.node().as_str() == "retired")
        .ok_or("remediation-risk settled frontier is absent")?
        .side_effect();
    assert_eq!(settled, SideEffectClass::NonIdempotentWrite);
    assert_eq!(
        submit_command(
            &reopened,
            store.as_ref(),
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("remediation-risk-third")?,
                revision: third.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let projection = reopened.projection(&run)?;
    let plan = projection
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("post-compaction risk plan is absent")?;
    assert!(plan.items().iter().any(|item| {
        item.node.as_ref() == NodeId::new("retired").ok().as_ref()
            && item.classification == ReconciliationClassification::CompletedOrUncertainSideEffects
            && item.action == ReconciliationAction::RejectRetrospectiveRewrite
    }));
    Ok(())
}

#[test]
fn compacted_retry_history_still_blocks_retrospective_side_effect_rewrite() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BoundaryFailingExecutor::new(
        descriptor_with_model_side_effect("idempotent_write")?,
        1,
    ));
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        executor,
        test_authority(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("reconcile-compacted-retry", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-reconcile-compacted-retry")?,
            ActorRef::new("controller:reconcile-compacted-retry")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(2, vec![ErrorClass::Adapter], 1, 1_000, 0)?,
        )?,
    )?;
    let old = removable_task_revision("workflow-reconcile-compacted-retry")?;
    let new = revision_without_completed_task(&old)?;
    let run = RunId::new("run-reconcile-compacted-retry")?;
    store.put_revision(&old)?;
    store.put_revision(&new)?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: old.semantic().workflow().clone(),
                revision: old.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-reconcile-compacted-retry")?,
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

    assert_eq!(runtime_tick(&runtime)?.uncertain, 1);
    clock.advance(1)?;
    assert_eq!(runtime_tick(&runtime)?.completed, 1);
    let compacted = runtime.projection(&run)?;
    let completed = compacted
        .settled_node_executions()
        .values()
        .find(|execution| execution.node().as_str() == "retired")
        .ok_or("completed retry execution is absent")?;
    assert_eq!(completed.attempt_count(), 2);
    assert_eq!(completed.attempts().len(), 1);
    assert!(compacted.attempts().is_empty());
    assert_eq!(compacted.unresolved_attempts().count(), 0);

    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-compacted-retry")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let planned = runtime.projection(&run)?;
    let plan = planned
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("compacted-retry reconciliation plan is absent")?;
    assert!(plan.items().iter().any(|item| {
        item.node
            .as_ref()
            .is_some_and(|node| node.as_str() == "retired")
            && item.classification == ReconciliationClassification::CompletedOrUncertainSideEffects
            && item.action == ReconciliationAction::RejectRetrospectiveRewrite
    }));

    let history = runtime.history(&run)?;
    assert!(
        history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { .. }))
    );
    assert!(
        history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeRetryScheduled { .. }))
    );
    Ok(())
}

#[test]
fn removed_completed_history_is_inert_after_revision_adoption() -> TestResult {
    let harness = Harness::new("removed-completed-adoption")?;
    let old = removable_task_revision("workflow-removed-completed-adoption")?;
    let new = revision_without_completed_task(&old)?;
    let run = RunId::new("run-removed-completed-adoption")?;
    harness.put_revision(&old)?;
    harness.put_revision(&new)?;
    harness.create_and_start(&run, &old)?;
    assert_eq!(harness.drive(&run, 4)?, 1);

    assert_eq!(
        harness.command(
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-removed-completed")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
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
        .ok_or("removed-completed plan is absent")?;
    assert!(plan.items().iter().any(|item| {
        item.node.as_ref() == NodeId::new("retired").ok().as_ref()
            && item.classification == ReconciliationClassification::ChangedCompleted
            && item.action == ReconciliationAction::UseNewOnNextInvocation
    }));
    let plan_id = plan.plan().clone();
    assert_eq!(
        harness.command(&run, RunCommand::ApplyReconciliation { plan: plan_id })?,
        CommandDisposition::Accepted
    );
    assert_eq!(harness.runtime.projection(&run)?.revision(), Some(new.id()));
    assert_eq!(
        harness.command(
            &run,
            RunCommand::DeliverSignal {
                signal: SignalId::new("removed-completed-signal")?,
                signal_type: SignalTypeId::new("notify.ready")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(json!({}))?,
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
fn removed_side_effecting_history_requires_authority_and_cannot_fabricate_remediation() -> TestResult
{
    let harness = Harness::with_descriptor(
        "removed-side-effect-adoption",
        RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        descriptor_with_model_side_effect("non_idempotent_write")?,
    )?;
    install_non_idempotent_success_script(&harness)?;
    let old = removable_task_revision("workflow-removed-side-effect-adoption")?;
    let new = revision_without_completed_task(&old)?;
    harness.put_revision(&old)?;
    harness.put_revision(&new)?;

    let rejected_run = RunId::new("run-removed-side-effect-rejected")?;
    harness.create_and_start(&rejected_run, &old)?;
    assert_eq!(harness.drive(&rejected_run, 4)?, 1);
    assert_eq!(
        harness.command(
            &rejected_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-removed-remediation")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::CompensateOrRemediate,
            },
        )?,
        CommandDisposition::Accepted
    );
    let rejected_projection = harness.runtime.projection(&rejected_run)?;
    let rejected_plan = rejected_projection
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("removed-side-effect remediation plan is absent")?;
    assert!(rejected_plan.items().iter().any(|item| {
        item.node.as_ref() == NodeId::new("retired").ok().as_ref()
            && item.classification == ReconciliationClassification::CompletedOrUncertainSideEffects
            && item.action == ReconciliationAction::RejectRetrospectiveRewrite
    }));
    assert_eq!(
        harness.command(
            &rejected_run,
            RunCommand::ApplyReconciliation {
                plan: rejected_plan.plan().clone(),
            },
        )?,
        CommandDisposition::Rejected
    );
    assert_eq!(
        harness.runtime.projection(&rejected_run)?.revision(),
        Some(old.id())
    );

    let authorized_run = RunId::new("run-removed-side-effect-authorized")?;
    harness.create_and_start(&authorized_run, &old)?;
    assert_eq!(harness.drive(&authorized_run, 4)?, 1);
    assert_eq!(
        harness.command(
            &authorized_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-removed-authority")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::RequireAuthority,
            },
        )?,
        CommandDisposition::Accepted
    );
    let authority_projection = harness.runtime.projection(&authorized_run)?;
    let authority_plan = authority_projection
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("removed-side-effect authority plan is absent")?;
    assert!(authority_plan.items().iter().any(|item| {
        item.node.as_ref() == NodeId::new("retired").ok().as_ref()
            && item.classification == ReconciliationClassification::CompletedOrUncertainSideEffects
            && item.action == ReconciliationAction::RequireAuthority
    }));
    let authority_plan_id = authority_plan.plan().clone();
    assert_eq!(
        harness.command(
            &authorized_run,
            RunCommand::DecideReconciliation {
                plan: authority_plan_id.clone(),
                decision: ReconciliationDecisionId::new("decision-removed-authority")?,
                outcome: AuthorityDecision::Approve,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(
            &authorized_run,
            RunCommand::ApplyReconciliation {
                plan: authority_plan_id,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.runtime.projection(&authorized_run)?.revision(),
        Some(new.id())
    );
    Ok(())
}
