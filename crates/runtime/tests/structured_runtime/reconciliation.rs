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
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement, observed_at_unix_ms)
    }

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

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.resolver.cancel(request)
    }
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
    assert_eq!(runtime.tick()?.completed, 1);
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
    assert_eq!(runtime.tick()?.completed, 1);
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
    assert_eq!(runtime.tick()?.completed, 1);
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
fn run_cancellation_after_lease_prevents_external_start() -> TestResult {
    let directory = TempDir::new()?;
    let executor = Arc::new(OperationCountingExecutor::new(test_descriptor()?));
    let (store, _clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "cancel-after-lease",
        "cancel-after-lease",
        NOW,
        64,
        executor.clone(),
    )?;
    let revision = task_revision("workflow-cancel-after-lease")?;
    let run = RunId::new("run-cancel-after-lease")?;
    store.put_revision(&revision)?;
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
                    ScopeId::new("scope-cancel-after-lease")?,
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
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::RequestCancellation,
        )?,
        CommandDisposition::Accepted
    );
    assert!(runtime.claim_effects(PageSize::new(1)?)?.is_empty());
    assert_eq!(executor.calls("model.generate")?, 0);
    assert!(
        !runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
    );
    Ok(())
}

#[test]
fn run_cancellation_after_effect_claim_prevents_final_adapter_entry() -> TestResult {
    let directory = TempDir::new()?;
    let executor = Arc::new(OperationCountingExecutor::new(test_descriptor()?));
    let (store, _clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "cancel-after-claim",
        "cancel-after-claim",
        NOW,
        64,
        executor.clone(),
    )?;
    let revision = task_revision("workflow-cancel-after-claim")?;
    let run = RunId::new("run-cancel-after-claim")?;
    store.put_revision(&revision)?;
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
                    ScopeId::new("scope-cancel-after-claim")?,
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
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    let mut actions = runtime.claim_execution_effects(PageSize::new(1)?)?;
    assert_eq!(actions.len(), 1);
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::RequestCancellation,
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        runtime.execute_effect(actions.remove(0))?,
        EffectExecutionResult::Completed { observations: 0 }
    );
    assert_eq!(executor.calls("model.generate")?, 0);
    assert!(!runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. }
    )));
    Ok(())
}

#[test]
fn reconciliation_cancellation_after_effect_claim_prevents_the_old_external_start() -> TestResult {
    let directory = TempDir::new()?;
    let executor = Arc::new(OperationCountingExecutor::new(test_descriptor()?));
    let (store, _clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "reconcile-cancel-after-lease",
        "reconcile-cancel-after-lease",
        NOW,
        64,
        executor.clone(),
    )?;
    let old = task_revision("workflow-reconcile-cancel-after-lease")?;
    let new = revised_task_revision(&old, "model.fail")?;
    let run = RunId::new("run-reconcile-cancel-after-lease")?;
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
                    ScopeId::new("scope-reconcile-cancel-after-lease")?,
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
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    let mut actions = runtime.claim_execution_effects(PageSize::new(1)?)?;
    assert_eq!(actions.len(), 1);
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconcile-cancel-after-lease")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::CancelAndRestartSafeWork,
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
        .ok_or("cancel-after-lease reconciliation plan is absent")?
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
    assert_eq!(
        runtime.execute_effect(actions.remove(0))?,
        EffectExecutionResult::Completed { observations: 0 }
    );
    assert_eq!(executor.calls("model.generate")?, 0);
    assert!(!runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. }
    )));
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

    assert_eq!(runtime.tick()?.uncertain, 1);
    clock.advance(1)?;
    assert_eq!(runtime.tick()?.completed, 1);
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
        blocked_runtime
            .tick()
            .map_err(|error| format!("blocked branch tick failed: {error}"))
    });
    executor.wait_until_entered()?;
    assert_eq!(runtime.tick()?.dispatched, 1);

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
    runtime.tick()?;
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
    harness.runtime.tick()?;
    harness.runtime.tick()?;
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

#[test]
fn cancel_and_restart_adoption_creates_one_replacement_after_confirmed_cancellation() -> TestResult
{
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new_with_authority(
        store.clone(),
        executor.clone(),
        test_authority(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("cancel-restart-adoption", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-cancel-restart-adoption")?,
            ActorRef::new("controller:cancel-restart-adoption")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?);
    let old = task_revision("workflow-cancel-restart-adoption")?;
    let new = revised_task_revision(&old, "model.fail")?;
    let run = RunId::new("run-cancel-restart-adoption")?;
    store.put_revision(&old)?;
    store.put_revision(&new)?;
    let create = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("create cancel-and-restart adoption run")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: old.semantic().workflow().clone(),
            revision: old.id().clone(),
            root_scope: WorkspaceScope::run_root(
                run.clone(),
                ScopeId::new("scope-cancel-restart-adoption")?,
            ),
            workspace_budget: generous_budget()?,
            inputs: Vec::new(),
        },
    )?;
    runtime.handle_authorized_command(&create, &test_authority_claim()?)?;
    let start = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("start cancel-and-restart adoption run")?,
        Vec::new(),
        RunCommand::StartRun,
    )?;
    runtime.handle_authorized_command(&start, &test_authority_claim()?)?;

    let dispatch_runtime = runtime.clone();
    let dispatch =
        std::thread::spawn(move || dispatch_runtime.tick().map_err(|error| error.to_string()));
    executor.wait_until_entered()?;
    let request = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("adopt a changed active safe task")?,
        Vec::new(),
        RunCommand::RequestRevisionAdoption {
            reconciliation: ReconciliationId::new("reconciliation-cancel-restart")?,
            revision: new.id().clone(),
            policy: ReconciliationPolicy::CancelAndRestartSafeWork,
        },
    )?;
    assert_eq!(
        runtime
            .handle_authorized_command(&request, &test_authority_claim()?)?
            .result()
            .disposition(),
        CommandDisposition::Accepted
    );
    let plan = runtime
        .projection(&run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("cancel-and-restart plan is absent")?
        .plan()
        .clone();
    let apply = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("apply cancel-and-restart plan")?,
        Vec::new(),
        RunCommand::ApplyReconciliation { plan },
    )?;
    assert_eq!(
        runtime
            .handle_authorized_command(&apply, &test_authority_claim()?)?
            .result()
            .disposition(),
        CommandDisposition::Accepted
    );

    runtime.tick()?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    executor.release()?;
    dispatch
        .join()
        .map_err(|_| "cancel-and-restart dispatch thread panicked")?
        .map_err(|error| format!("cancel-and-restart dispatch failed: {error}"))?;
    for _ in 0..4 {
        if runtime.projection(&run)?.is_completed() {
            break;
        }
        runtime.tick()?;
    }

    let projection = runtime.projection(&run)?;
    assert_eq!(projection.revision(), Some(new.id()));
    let work = NodeId::new("work")?;
    let executions: Vec<_> = projection.executions_for_node(&work).collect();
    assert_eq!(executions.len(), 1);
    assert_eq!(
        executions[0].state(),
        &NodeExecutionState::Terminal(milkdrift_persistence::NodeOutcome::Succeeded)
    );
    let history = runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeTerminal {
            outcome: milkdrift_persistence::NodeOutcome::Cancelled,
            ..
        }
    )));
    assert_eq!(
        runtime
            .history(&run)?
            .iter()
            .filter(|event| matches!(
                event.kind(),
                RunEventKind::ReconciliationCancellationRequested { .. }
            ))
            .count(),
        1
    );
    Ok(())
}
