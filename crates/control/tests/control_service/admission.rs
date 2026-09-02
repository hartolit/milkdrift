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
    let base = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let descriptor = DescriptorBuilder::new(
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
    .build()?;
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

fn controller_artifact_metadata(
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
