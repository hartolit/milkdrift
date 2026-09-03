use super::*;

#[test]
fn controller_model_usage_is_descriptor_classified_and_unknown_units_fail_closed() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(
        directory.path().join("controller-usage.redb"),
    )?);
    let run = RunId::new("run-controller-usage")?;
    let actor = ActorRef::new("controller:usage")?;
    let grant_id = GrantId::new("grant:controller-usage")?;
    let (runtime, service, context) =
        services(store.clone(), &actor, &run, &grant_id, "controller-usage")?;
    let body = base_revision("controller-usage-body")?;
    store.put_revision(&body)?;
    let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new("controller-usage-wrapper")?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits: ControllerLimits::new(
            5, 4, 8, 4, 60_000, 1_000_000, 10_000, 10_000, 1_000_000, 5, 5, 3, 3, 2, 2, None,
        )?,
        author: AuthorRef::new("human:controller-test")?,
    })?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;
    for _ in 0..128 {
        runtime.tick()?;
        if runtime.projection(&run)?.lifecycle().is_completed() {
            break;
        }
    }
    let history = runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::RepeatIterationCreated { .. }))
            .count(),
        1
    );
    let account = store
        .controller_account_binding(&run)?
        .ok_or("controller run has no durable account binding")?;
    let child_runs = history
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::SubworkflowCreated { child_run, .. } => Some(child_run.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!child_runs.is_empty());
    for child in &child_runs {
        assert_eq!(
            store.controller_account_binding(child)?.as_ref(),
            Some(&account)
        );
    }
    let child_history = runtime.history(child_runs.last().ok_or("body child is absent")?)?;
    assert!(child_history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            controller_admission: ControllerAdmissionOutcome::Denied {
                reason: ControllerAdmissionDenial::Unknown { dimension },
                ..
            },
            ..
        } if dimension == "input_units"
    )));
    assert!(child_history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeTerminal {
            error_class: Some(ErrorClass::RateLimit),
            ..
        }
    )));
    let state = store
        .controller_account(&account)?
        .ok_or("controller account state is absent")?;
    assert_eq!(state.committed_totals()?.model_admissions(), 0);
    assert_complete_integrity(store.as_ref())?;
    Ok(())
}

#[test]
fn controller_process_ceiling_denies_n_plus_one_before_executor_entry() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(
        directory.path().join("controller-process-admission.redb"),
    )?);
    let run = RunId::new("run-controller-process-admission")?;
    let actor = ActorRef::new("controller:process-admission")?;
    let grant_id = GrantId::new("grant:controller-process-admission")?;
    let descriptor = process_descriptor()?;
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 4,
            max_generations_per_capability: 2,
            max_concurrent_per_generation: 4,
            observation_stale_after_ms: 60_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    let adapter = Arc::new(CountingProcessAdapter::default());
    host.register(
        descriptor.clone(),
        adapter.clone(),
        Some(CapabilityObservation::new(
            descriptor.identity().clone(),
            NOW,
            true,
            0,
            "controller process admission fixture ready",
        )?),
    )?;
    let (runtime, service, context) = services_with_executor_and_revocations(
        store.clone(),
        &actor,
        &grant_id,
        "controller-process-admission",
        grant(&actor, &run, &grant_id)?,
        BTreeMap::new(),
        Arc::new(host.clone()),
    )?;
    let body = three_process_body("controller-process-admission-body")?;
    store.put_revision(&body)?;
    let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new("controller-process-admission-wrapper")?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits: ControllerLimits::new(
            5, 5, 8, 4, 60_000, 1_000_000, 10_000, 10_000, 1_000_000, 2, 5, 1, 5, 2, 2, None,
        )?,
        author: AuthorRef::new("human:controller-process-admission")?,
    })?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;
    let mut effects = Vec::new();
    for _ in 0..128 {
        let _ = runtime.scheduler_tick()?;
        effects.extend(runtime.claim_execution_effects(PageSize::new(8)?)?);
        if effects.len() == 3 {
            break;
        }
    }
    assert_eq!(effects.len(), 3);
    let barrier = Arc::new(Barrier::new(effects.len()));
    let workers = effects
        .into_iter()
        .map(|effect| {
            let runtime = runtime.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                runtime.execute_effect(effect)
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker
            .join()
            .map_err(|_| "controller admission worker panicked")??;
    }
    for _ in 0..128 {
        runtime.tick()?;
        if runtime.projection(&run)?.lifecycle().is_completed() {
            break;
        }
    }
    let history = runtime.history(&run)?;
    let child_runs = history
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::SubworkflowCreated { child_run, .. } => Some(child_run.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut reserved = 0;
    let mut denied = 0;
    for child in child_runs {
        for event in runtime.history(&child)? {
            match event.kind() {
                RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                    controller_admission: ControllerAdmissionOutcome::Reserved { .. },
                    ..
                } => reserved += 1,
                RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                    controller_admission:
                        ControllerAdmissionOutcome::Denied {
                            reason: ControllerAdmissionDenial::Limit { dimension },
                            ..
                        },
                    ..
                } if dimension == "process_admissions" => denied += 1,
                _ => {}
            }
        }
    }
    assert_eq!(reserved, 2);
    assert_eq!(denied, 1);
    assert_eq!(adapter.entries(), 2);
    let generations = host.generations(
        &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
        NOW,
    )?;
    assert_eq!(generations.len(), 1);
    assert_eq!(generations[0].active_permits, 0);
    let account = store
        .controller_account_binding(&run)?
        .ok_or("controller account is unbound")?;
    let state = store
        .controller_account(&account)?
        .ok_or("controller account is absent")?;
    assert_eq!(state.committed_totals()?.process_admissions(), 2);
    assert_complete_integrity(store.as_ref())?;
    Ok(())
}

#[test]
fn cancellation_after_effect_claim_creates_no_controller_reservation_or_adapter_entry() -> TestResult
{
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(
        directory.path().join("controller-cancel-after-claim.redb"),
    )?);
    let run = RunId::new("run-controller-cancel-after-claim")?;
    let actor = ActorRef::new("controller:cancel-after-claim")?;
    let grant_id = GrantId::new("grant:controller-cancel-after-claim")?;
    let descriptor = process_descriptor()?;
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 4,
            max_generations_per_capability: 2,
            max_concurrent_per_generation: 4,
            observation_stale_after_ms: 60_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    let adapter = Arc::new(CountingProcessAdapter::default());
    host.register(
        descriptor.clone(),
        adapter.clone(),
        Some(CapabilityObservation::new(
            descriptor.identity().clone(),
            NOW,
            true,
            0,
            "controller cancellation fixture ready",
        )?),
    )?;
    let broad_grant = AuthorityPreset::Autonomous
        .template(
            grant_id.clone(),
            1,
            actor.clone(),
            WorkflowRunScope::Any,
            CapabilityAuthorityScope::allow_any(SideEffectClass::ReadOnly),
            AuthorityBudget {
                cost_minor: Some(1_000_000),
                duration_ms: Some(3_600_000),
                invocations: Some(1_000),
                artifact_bytes: Some(16_777_216),
                units: Some(1_000_000),
                concurrency: Some(32),
            },
        )
        .build()?;
    let (runtime, service, context) = services_with_executor_and_revocations(
        store.clone(),
        &actor,
        &grant_id,
        "controller-cancel-after-claim",
        broad_grant,
        BTreeMap::new(),
        Arc::new(host),
    )?;
    let body = base_revision("controller-cancel-after-claim-body")?;
    store.put_revision(&body)?;
    let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new("controller-cancel-after-claim-wrapper")?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits: ControllerLimits::new(
            2, 2, 8, 4, 60_000, 1_000_000, 8, 8, 1_000_000, 2, 2, 2, 2, 2, 2, None,
        )?,
        author: AuthorRef::new("human:controller-cancel-after-claim")?,
    })?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;

    let mut action = None;
    for _ in 0..128 {
        let _ = runtime.scheduler_tick()?;
        if let Some(claimed) = runtime.claim_execution_effects(PageSize::new(1)?)?.pop() {
            action = Some(claimed);
            break;
        }
    }
    let action = action.ok_or("controlled effect was not claimed")?;
    let child = runtime
        .history(&run)?
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::SubworkflowCreated { child_run, .. } => Some(child_run.clone()),
            _ => None,
        })
        .ok_or("controller body child is absent")?;
    let account = store
        .controller_account_binding(&child)?
        .ok_or("controller body child is unbound")?;
    let before = store
        .controller_account(&account)?
        .ok_or("controller account is absent before cancellation")?;

    let projection = runtime.projection(&child)?;
    service.execute(&command(
        "controller-cancel-after-claim-request",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(projection.sequence()),
            expected_revision: projection.revision().cloned(),
            expected_proposal_digest: None,
        },
        ControlCommand::RequestCancellation { run: child.clone() },
    )?)?;
    assert_eq!(
        runtime.execute_effect(action)?,
        milkdrift_runtime::EffectExecutionResult::Completed { observations: 0 }
    );

    let after = store
        .controller_account(&account)?
        .ok_or("controller account disappeared after cancellation")?;
    assert_eq!(after, before);
    assert!(after.reservations().is_empty());
    assert_eq!(after.settled().process_admissions(), 0);
    assert_eq!(adapter.entries(), 0);
    assert!(!runtime.history(&child)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. }
    )));
    assert_complete_integrity(store.as_ref())?;
    Ok(())
}

#[test]
fn terminal_cancellation_retains_missing_bounded_usage_and_blocks_the_account() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(
        directory
            .path()
            .join("controller-terminal-cancellation.redb"),
    )?);
    let run = RunId::new("run-controller-terminal-cancellation")?;
    let actor = ActorRef::new("controller:terminal-cancellation")?;
    let grant_id = GrantId::new("grant:controller-terminal-cancellation")?;
    let executor = Arc::new(TerminalCancellationExecutor::new(process_descriptor()?));
    let broad_grant = AuthorityPreset::Autonomous
        .template(
            grant_id.clone(),
            1,
            actor.clone(),
            WorkflowRunScope::Any,
            CapabilityAuthorityScope::allow_any(SideEffectClass::ReadOnly),
            AuthorityBudget {
                cost_minor: Some(1_000_000),
                duration_ms: Some(3_600_000),
                invocations: Some(1_000),
                artifact_bytes: Some(16_777_216),
                units: Some(1_000_000),
                concurrency: Some(32),
            },
        )
        .build()?;
    let (runtime, service, context) = services_with_executor_and_revocations(
        store.clone(),
        &actor,
        &grant_id,
        "controller-terminal-cancellation",
        broad_grant,
        BTreeMap::new(),
        executor.clone(),
    )?;
    let body = base_revision("controller-terminal-cancellation-body")?;
    store.put_revision(&body)?;
    let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new("controller-terminal-cancellation-wrapper")?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits: ControllerLimits::new(
            2, 2, 8, 4, 60_000, 1_000_000, 8, 8, 1_000_000, 2, 2, 2, 2, 2, 2, None,
        )?,
        author: AuthorRef::new("human:controller-terminal-cancellation")?,
    })?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;

    let mut action = None;
    for _ in 0..128 {
        let _ = runtime.scheduler_tick()?;
        if let Some(claimed) = runtime.claim_execution_effects(PageSize::new(1)?)?.pop() {
            action = Some(claimed);
            break;
        }
    }
    let action = action.ok_or("controlled cancellation effect was not scheduled")?;
    let execution_runtime = runtime.clone();
    let execution = thread::spawn(move || execution_runtime.execute_effect(action));
    executor.wait_until_entered()?;

    let account = store
        .controller_account_binding(&run)?
        .ok_or("controller cancellation run is unbound")?;
    let admitted = store
        .controller_account(&account)?
        .ok_or("controller cancellation account is absent")?;
    assert_eq!(admitted.outstanding().input_units(), 4);
    assert_eq!(admitted.settled().process_admissions(), 1);

    let child = runtime
        .history(&run)?
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::SubworkflowCreated { child_run, .. } => Some(child_run.clone()),
            _ => None,
        })
        .ok_or("controller cancellation body child is absent")?;
    assert_eq!(
        store.controller_account_binding(&child)?.as_ref(),
        Some(&account)
    );
    let projection = runtime.projection(&child)?;
    service.execute(&command(
        "controller-terminal-cancellation-request",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(projection.sequence()),
            expected_revision: projection.revision().cloned(),
            expected_proposal_digest: None,
        },
        ControlCommand::RequestCancellation { run: child },
    )?)?;
    for _ in 0..64 {
        runtime.tick()?;
        if executor.cancellations.load(Ordering::SeqCst) == 1 {
            break;
        }
    }
    assert_eq!(executor.cancellations.load(Ordering::SeqCst), 1);
    executor.release_after_cancellation_commit()?;
    execution
        .join()
        .map_err(|_| "controlled cancellation execution panicked")??;

    let settled = store
        .controller_account(&account)?
        .ok_or("controller cancellation account disappeared")?;
    assert_eq!(settled.outstanding().input_units(), 4);
    assert!(matches!(
        settled.blocked(),
        Some(milkdrift_persistence::ControllerAccountBlock::UnknownUsage {
            dimension,
            ..
        }) if dimension == "input_units"
    ));
    assert_complete_integrity(store.as_ref())?;
    Ok(())
}

#[test]
fn controller_artifact_charge_is_exact_replay_safe_abort_safe_and_restart_durable() -> TestResult {
    let directory = TempDir::new()?;
    let database = directory.path().join("controller-artifact-admission.redb");
    let run = RunId::new("run-controller-artifact-admission")?;
    let actor = ActorRef::new("controller:artifact-admission")?;
    let grant_id = GrantId::new("grant:controller-artifact-admission")?;
    let account;
    {
        let store = Arc::new(RedbStore::open(&database)?);
        let (runtime, service, context) = services(
            store.clone(),
            &actor,
            &run,
            &grant_id,
            "controller-artifact-admission",
        )?;
        let terminal = Node::new(
            NodeId::new("done")?,
            NodeKind::Terminal {
                outcome: TerminalOutcome::Success,
            },
        )?;
        let body = BlueprintRevision::genesis(
            WorkflowId::new("controller-artifact-admission-body")?,
            MutationBatch::new(vec![Mutation::AddNode { node: terminal }])?,
            AuthorRef::new("human:controller-artifact-admission")?,
            "controller direct artifact accounting body",
        )?;
        store.put_revision(&body)?;
        let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
            workflow: WorkflowId::new("controller-artifact-admission-wrapper")?,
            body: PinnedSubworkflow::new(
                body.semantic().workflow().clone(),
                body.id().clone(),
                WorkflowInterface::new([], [])?,
            ),
            continue_condition: Condition::Constant { value: true },
            limits: ControllerLimits::new(
                1, 1, 8, 4, 60_000, 1_000_000, 10_000, 10_000, 16, 2, 2, 2, 2, 2, 2, None,
            )?,
            author: AuthorRef::new("human:controller-artifact-admission")?,
        })?;
        store.put_revision(&wrapper)?;
        create_and_start(&service, &runtime, &context, &run, &wrapper)?;
        for _ in 0..64 {
            runtime.tick()?;
            if runtime.projection(&run)?.lifecycle().is_completed() {
                break;
            }
        }
        account = store
            .controller_account_binding(&run)?
            .ok_or("controller artifact run is unbound")?;
        let budget = WorkspaceBudget::new(128, 65_536, 1_048_576, 64, 1_048_576, 16_777_216)?;
        let expected_usage = store.workspace_usage(&run)?;
        let exact_bytes = b"12345678";
        let exact_metadata =
            controller_artifact_metadata("artifact-controller-exact", exact_bytes)?;
        let exact = BeginArtifactPublication::new(
            ArtifactPublicationId::new("publication-controller-exact")?,
            run.clone(),
            exact_metadata.clone(),
            budget.clone(),
            expected_usage,
        )?;
        let _ = store.begin_publication(&exact)?;
        let _ = store.write_chunk(exact.publication(), 0, exact_bytes)?;
        let _ = store.commit_publication(exact.publication())?;
        let _ = store.begin_publication(&exact)?;
        assert_eq!(
            store
                .controller_account(&account)?
                .ok_or("controller account disappeared after artifact commit")?
                .settled()
                .artifact_bytes(),
            8
        );

        let after_exact = store.workspace_usage(&run)?;
        let aborted_metadata = controller_artifact_metadata("artifact-controller-abort", b"a")?;
        let aborted = BeginArtifactPublication::new(
            ArtifactPublicationId::new("publication-controller-abort")?,
            run.clone(),
            aborted_metadata,
            budget.clone(),
            after_exact,
        )?;
        let _ = store.begin_publication(&aborted)?;
        let _ = store.write_chunk(aborted.publication(), 0, b"a")?;
        store.abort_publication(aborted.publication())?;
        assert_eq!(
            store
                .controller_account(&account)?
                .ok_or("controller account disappeared after artifact abort")?
                .settled()
                .artifact_bytes(),
            8
        );

        let dedup_metadata =
            controller_artifact_metadata("artifact-controller-dedup", exact_bytes)?;
        let dedup = BeginArtifactPublication::new(
            ArtifactPublicationId::new("publication-controller-dedup")?,
            run.clone(),
            dedup_metadata,
            budget.clone(),
            after_exact,
        )?;
        let _ = store.begin_publication(&dedup)?;
        let _ = store.write_chunk(dedup.publication(), 0, exact_bytes)?;
        let _ = store.commit_publication(dedup.publication())?;
        assert_eq!(
            store
                .controller_account(&account)?
                .ok_or("controller account disappeared after deduplicated publication")?
                .settled()
                .artifact_bytes(),
            16
        );

        let after_dedup = store.workspace_usage(&run)?;
        let excess_metadata = controller_artifact_metadata("artifact-controller-excess", b"x")?;
        let excess = BeginArtifactPublication::new(
            ArtifactPublicationId::new("publication-controller-excess")?,
            run.clone(),
            excess_metadata.clone(),
            budget,
            after_dedup,
        )?;
        let _ = store.begin_publication(&excess)?;
        let _ = store.write_chunk(excess.publication(), 0, b"x")?;
        assert!(matches!(
            store.commit_publication(excess.publication()),
            Err(milkdrift_persistence::PersistenceError::Bounds {
                location: "controller.artifact_budget",
                ..
            })
        ));
        assert!(
            store
                .metadata(excess_metadata.reference().artifact())?
                .is_none()
        );
        assert_complete_integrity(store.as_ref())?;
    }
    let reopened = RedbStore::open(&database)?;
    let state = reopened
        .controller_account(&account)?
        .ok_or("controller account did not survive reopen")?;
    assert_eq!(state.settled().artifact_bytes(), 16);
    assert_complete_integrity(&reopened)?;
    Ok(())
}

#[test]
#[ignore = "manual release-mode controller admission turnover and restart proof"]
fn release_controller_admission_longevity_turns_over_reservations_artifacts_and_restart()
-> TestResult {
    let directory = TempDir::new()?;
    let database = directory.path().join("controller-admission-longevity.redb");
    let run = RunId::new("run-controller-admission-longevity")?;
    let actor = ActorRef::new("controller:admission-longevity")?;
    let grant_id = GrantId::new("grant:controller-admission-longevity")?;
    let controller_execution;
    let artifact_after_first_checkpoint;

    {
        let store = Arc::new(RedbStore::open(&database)?);
        let (runtime, service, context, adapter) = counting_process_services(
            store.clone(),
            &actor,
            &run,
            &grant_id,
            "controller-admission-longevity-before",
        )?;
        let body = base_revision("controller-admission-longevity-body")?;
        store.put_revision(&body)?;
        let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
            workflow: WorkflowId::new("controller-admission-longevity-wrapper")?,
            body: PinnedSubworkflow::new(
                body.semantic().workflow().clone(),
                body.id().clone(),
                WorkflowInterface::new([], [])?,
            ),
            continue_condition: Condition::Constant { value: true },
            limits: ControllerLimits::new(
                33,
                4,
                8,
                4,
                60_000,
                1_000_000,
                33,
                33,
                1_000_000,
                33,
                33,
                3,
                3,
                2,
                2,
                Some(11),
            )?,
            author: AuthorRef::new("human:controller-admission-longevity")?,
        })?;
        store.put_revision(&wrapper)?;
        create_and_start(&service, &runtime, &context, &run, &wrapper)?;
        for _ in 0..512 {
            runtime
                .tick()
                .map_err(|error| format!("pre-restart controller tick failed: {error}"))?;
            if runtime
                .projection(&run)?
                .repeat_continuations()
                .values()
                .any(|value| value.is_pending_approval())
            {
                break;
            }
        }
        let checkpoint = runtime.projection(&run)?;
        controller_execution = checkpoint
            .repeat_continuations()
            .iter()
            .find(|(_, value)| value.is_pending_approval())
            .map(|(execution, _)| execution.clone())
            .ok_or("controller admission longevity did not reach checkpoint eleven")?;
        assert_eq!(adapter.entries(), 11);
        let account = store
            .controller_account_binding(&run)?
            .ok_or("controller admission longevity account is unbound")?;
        let before_artifacts = store
            .controller_account(&account)?
            .ok_or("controller admission longevity account is absent")?
            .settled()
            .artifact_bytes();
        publish_controller_artifacts(
            store.as_ref(),
            &run,
            "controller-admission-longevity-before",
            0,
            11,
        )?;
        let state = store
            .controller_account(&account)?
            .ok_or("controller admission longevity account disappeared")?;
        artifact_after_first_checkpoint = state.settled().artifact_bytes();
        assert_eq!(artifact_after_first_checkpoint, before_artifacts + 11);
        assert_eq!(state.settled().process_admissions(), 11);
        assert_eq!(state.outstanding().artifact_bytes(), 0);
        assert_eq!(state.outstanding().process_admissions(), 0);
        assert!(state.reservations().is_empty());
        assert_complete_integrity(store.as_ref())?;
    }

    {
        let store = Arc::new(RedbStore::open(&database)?);
        let (runtime, service, context, adapter) = counting_process_services(
            store.clone(),
            &actor,
            &run,
            &grant_id,
            "controller-admission-longevity-after",
        )?;
        let checkpoint = runtime.projection(&run)?;
        service.execute(&command(
            "controller-admission-longevity-continue-eleven",
            &context,
            OptimisticGuard {
                expected_run_sequence: Some(checkpoint.sequence()),
                expected_revision: checkpoint.revision().cloned(),
                expected_proposal_digest: None,
            },
            ControlCommand::ContinueController {
                run: run.clone(),
                controller_execution: controller_execution.clone(),
                decision: RepeatDecisionId::new("controller-admission-longevity-decision-eleven")?,
            },
        )?)?;
        for _ in 0..512 {
            runtime
                .tick()
                .map_err(|error| format!("post-restart checkpoint tick failed: {error}"))?;
            if runtime
                .projection(&run)?
                .repeat_continuations()
                .get(&controller_execution)
                .is_some_and(|value| value.is_pending_approval())
            {
                break;
            }
        }
        let checkpoint = runtime.projection(&run)?;
        assert!(
            checkpoint
                .repeat_continuations()
                .get(&controller_execution)
                .is_some_and(|value| value.is_pending_approval())
        );
        assert_eq!(adapter.entries(), 11);
        let account = store
            .controller_account_binding(&run)?
            .ok_or("reopened controller admission longevity account is unbound")?;
        let before_artifacts = store
            .controller_account(&account)?
            .ok_or("reopened controller admission longevity account is absent")?
            .settled()
            .artifact_bytes();
        assert!(before_artifacts > artifact_after_first_checkpoint);
        publish_controller_artifacts(
            store.as_ref(),
            &run,
            "controller-admission-longevity-after",
            11,
            11,
        )?;
        let state = store
            .controller_account(&account)?
            .ok_or("reopened controller admission longevity account disappeared")?;
        assert_eq!(state.settled().artifact_bytes(), before_artifacts + 11);
        assert_eq!(state.settled().process_admissions(), 22);
        assert_eq!(state.outstanding().artifact_bytes(), 0);
        assert_eq!(state.outstanding().process_admissions(), 0);
        assert!(state.reservations().is_empty());
        assert_complete_integrity(store.as_ref())?;

        service.execute(&command(
            "controller-admission-longevity-continue-twenty-two",
            &context,
            OptimisticGuard {
                expected_run_sequence: Some(checkpoint.sequence()),
                expected_revision: checkpoint.revision().cloned(),
                expected_proposal_digest: None,
            },
            ControlCommand::ContinueController {
                run: run.clone(),
                controller_execution: controller_execution.clone(),
                decision: RepeatDecisionId::new(
                    "controller-admission-longevity-decision-twenty-two",
                )?,
            },
        )?)?;
        for _ in 0..1_024 {
            runtime
                .tick()
                .map_err(|error| format!("terminal controller tick failed: {error}"))?;
            if runtime.projection(&run)?.lifecycle().is_completed() {
                break;
            }
        }
        assert_eq!(
            runtime.projection(&run)?.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Failed)
        );
        assert_eq!(adapter.entries(), 22);
        let state = store
            .controller_account(&account)?
            .ok_or("terminal controller admission longevity account is absent")?;
        assert!(state.settled().artifact_bytes() > before_artifacts + 11);
        assert_eq!(state.settled().process_admissions(), 33);
        assert_eq!(state.outstanding().cost_micros(), 0);
        assert_eq!(state.outstanding().input_units(), 0);
        assert_eq!(state.outstanding().output_units(), 0);
        assert_eq!(state.outstanding().artifact_bytes(), 0);
        assert_eq!(state.outstanding().process_admissions(), 0);
        assert_eq!(state.outstanding().model_admissions(), 0);
        assert!(state.reservations().is_empty());
        assert_complete_integrity(store.as_ref())?;
    }

    let store = RedbStore::open(&database)?;
    let account = store
        .controller_account_binding(&run)?
        .ok_or("terminal controller admission longevity account lost its binding")?;
    let state = store
        .controller_account(&account)?
        .ok_or("terminal controller admission longevity account did not survive reopen")?;
    assert!(state.settled().artifact_bytes() > artifact_after_first_checkpoint + 11);
    assert_eq!(state.settled().process_admissions(), 33);
    assert!(state.reservations().is_empty());
    assert_complete_integrity(&store)?;
    Ok(())
}

fn publish_controller_artifacts(
    store: &RedbStore,
    run: &RunId,
    prefix: &str,
    start: u64,
    count: u64,
) -> TestResult {
    let budget = WorkspaceBudget::new(128, 65_536, 1_048_576, 64, 1_048_576, 16_777_216)?;
    for index in start..start.checked_add(count).ok_or("artifact range overflow")? {
        let identity = format!("{prefix}-artifact-{index}");
        let publication = ArtifactPublicationId::new(format!("{prefix}-publication-{index}"))?;
        let bytes = b"x";
        let request = BeginArtifactPublication::new(
            publication.clone(),
            run.clone(),
            controller_artifact_metadata(&identity, bytes)?,
            budget.clone(),
            store.workspace_usage(run)?,
        )?;
        let _ = store.begin_publication(&request)?;
        let _ = store.write_chunk(&publication, 0, bytes)?;
        let _ = store.commit_publication(&publication)?;
    }
    Ok(())
}

pub(super) fn controller_artifact_metadata(
    identity: &str,
    bytes: &[u8],
) -> TestResult<milkdrift_workspace::ArtifactMetadata> {
    let reference = milkdrift_workspace::ArtifactReference::new(
        milkdrift_workspace::ArtifactId::new(identity)?,
        milkdrift_workspace::ContentDigest::for_bytes(bytes),
        milkdrift_workspace::MediaType::new("application/octet-stream")?,
        u64::try_from(bytes.len())?,
    );
    Ok(milkdrift_workspace::ArtifactMetadata::new(
        reference,
        milkdrift_workspace::ArtifactSensitivity::Public,
        milkdrift_workspace::ArtifactRetention::WhileReferenced,
        milkdrift_workspace::ArtifactProvenance::new(
            milkdrift_workspace::CausalReference::External {
                source: milkdrift_workspace::CausalId::new("controller-artifact-test")?,
            },
            Vec::new(),
        )?,
    )?)
}

pub(super) fn process_descriptor() -> TestResult<CapabilityDescriptor> {
    let base = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    Ok(DescriptorBuilder::new(
        CapabilityId::new("process-controller-test")?,
        1,
        CapabilityCategory::Process,
        AdmissionConstraints::new(8, 32)?,
        base.locality(),
    )
    .provider_profile(base.provider_profile().cloned())
    .operations(base.operations().clone())
    .trust_zones(base.trust_zones().clone())
    .execution_trust(base.execution_trust())
    .resource_observations(base.resource_observations().cloned())
    .labels(base.labels().clone())
    .extensions(base.extensions().clone())
    .build()?)
}
