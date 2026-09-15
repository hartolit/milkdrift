use super::*;

#[test]
fn revocation_before_resolution_is_a_typed_denial_not_capability_absence() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(2_000));
    let authority = Arc::new(RevocableAuthority::new());
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let executor = Arc::new(CountingExecutor::new(descriptor));
    let runtime = revocable_service(
        store.clone(),
        clock,
        authority.clone(),
        executor.clone(),
        "revoke-before-resolution",
    )?;
    let revision = sequence_revision()?;
    let run = RunId::new("run-revoke-before-resolution")?;
    create_and_start_revocable_run(&runtime, store.as_ref(), &revision, &run)?;
    authority.revoke();
    let tick = runtime.scheduler_tick()?;
    assert_eq!(tick.completed, 1);
    assert_eq!(tick.dispatched, 0);
    assert_eq!(executor.entries.load(Ordering::SeqCst), 0);
    let history = runtime.history(&run)?;
    let denied = history.iter().find_map(|event| match event.kind() {
        milkdrift_persistence::RunEventKind::CapabilityResolutionDenied {
            authorization, ..
        } => Some(authorization),
        _ => None,
    });
    assert!(
        denied.is_some_and(|decision| { decision.reason_codes() == [DecisionReasonCode::Revoked] })
    );
    assert!(runtime.projection(&run)?.attempts().is_empty());
    Ok(())
}

#[test]
fn revocation_after_resolution_denies_effect_claim_and_releases_the_lease() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(2_000));
    let authority = Arc::new(RevocableAuthority::new());
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let executor = Arc::new(CountingExecutor::new(descriptor));
    let runtime = revocable_service(
        store.clone(),
        clock,
        authority.clone(),
        executor.clone(),
        "revoke-after-resolution",
    )?;
    let revision = sequence_revision()?;
    let run = RunId::new("run-revoke-after-resolution")?;
    create_and_start_revocable_run(&runtime, store.as_ref(), &revision, &run)?;
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    authority.revoke();
    assert!(
        runtime
            .claim_execution_effects(PageSize::new(1)?)?
            .is_empty()
    );
    assert_eq!(executor.entries.load(Ordering::SeqCst), 0);
    let history = runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        milkdrift_persistence::RunEventKind::CapabilityEntryDecisionRecorded {
            authorization,
            ..
        } if !authorization.is_allowed()
            && authorization.reason_codes() == [DecisionReasonCode::Revoked]
    )));
    assert!(
        runtime
            .projection(&run)?
            .leases()
            .values()
            .all(|lease| !lease.is_active())
    );
    Ok(())
}

#[test]
fn revocation_after_effect_claim_is_durable_and_never_enters_executor() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(2_000));
    let authority = Arc::new(RevocableAuthority::new());
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let mut wire = serde_json::to_value(&descriptor)?;
    wire["locality"] = serde_json::json!("peer");
    wire["peer"] = serde_json::json!("peer-pinned");
    let descriptor = serde_json::from_value(wire)?;
    let executor = Arc::new(CountingExecutor::new(descriptor));
    let runtime = revocable_service(
        store.clone(),
        clock,
        authority.clone(),
        executor.clone(),
        "revoke-before-adapter-entry",
    )?;
    let revision = sequence_revision_with_requirement(
        CapabilityRequirement::new(OperationId::new("model.generate")?).with_placement(
            milkdrift_capability::PlacementRequirement::new(
                None,
                Some(BTreeSet::from([milkdrift_capability::PeerId::new(
                    "peer-pinned",
                )?])),
            )?,
        ),
    )?;
    let run = RunId::new("run-revoke-before-adapter-entry")?;
    create_and_start_revocable_run(&runtime, store.as_ref(), &revision, &run)?;
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    let mut effects = runtime.claim_execution_effects(PageSize::new(1)?)?;
    assert_eq!(effects.len(), 1);
    authority.revoke();
    let result = runtime.execute_effect(effects.remove(0))?;
    assert_eq!(
        result,
        milkdrift_runtime::EffectExecutionResult::Completed { observations: 0 }
    );
    assert_eq!(executor.entries.load(Ordering::SeqCst), 0);
    let history = runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        milkdrift_persistence::RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            authorization,
            ..
        } if !authorization.is_allowed()
            && authorization.reason_codes() == [DecisionReasonCode::Revoked]
    )));
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        milkdrift_persistence::RunEventKind::NodeTerminal {
            outcome: NodeOutcome::Rejected,
            error_class: Some(ErrorClass::Authorization),
            ..
        }
    )));
    let projection = runtime.projection(&run)?;
    assert!(projection.leases().values().all(|lease| !lease.is_active()));
    Ok(())
}

#[test]
fn revocation_after_adapter_entry_preserves_success_and_blocks_later_work() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(2_000));
    let authority = Arc::new(RevocableAuthority::new());
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let gate = Arc::new((Mutex::new(EntryGate::default()), Condvar::new()));
    let executor = Arc::new(CountingExecutor::blocking(descriptor, gate.clone()));
    let runtime = Arc::new(revocable_service(
        store.clone(),
        clock,
        authority.clone(),
        executor.clone(),
        "revoke-after-entry",
    )?);
    let revision = two_task_revision()?;
    let run = RunId::new("run-revoke-after-entry")?;
    create_and_start_revocable_run(runtime.as_ref(), store.as_ref(), &revision, &run)?;
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    let action = runtime
        .claim_execution_effects(PageSize::new(1)?)?
        .pop()
        .ok_or("first effect was not claimed")?;
    let effect_runtime = runtime.clone();
    let thread = std::thread::spawn(move || effect_runtime.execute_effect(action));
    {
        let (lock, changed) = &*gate;
        let state = lock.lock().map_err(|_error| "entry gate poisoned")?;
        let (mut state, _) = changed
            .wait_timeout_while(state, std::time::Duration::from_secs(5), |state| {
                !state.entered
            })
            .map_err(|_error| "entry gate wait poisoned")?;
        if !state.entered {
            // Release even a late entrant before joining the failed scenario's worker.
            state.release = true;
            changed.notify_all();
            drop(state);
            let result = thread.join().map_err(|_panic| "effect thread panicked")?;
            return Err(
                format!("adapter entry was not observed before deadline: {result:?}").into(),
            );
        }
    }
    authority.revoke();
    {
        let (lock, changed) = &*gate;
        let mut state = lock.lock().map_err(|_error| "entry gate poisoned")?;
        state.release = true;
        changed.notify_all();
    }
    let effect = thread.join().map_err(|_panic| "effect thread panicked")??;
    assert_eq!(
        effect,
        milkdrift_runtime::EffectExecutionResult::Completed { observations: 1 }
    );
    let after_first = runtime.history(&run)?;
    assert!(after_first.iter().any(|event| matches!(
        event.kind(),
        milkdrift_persistence::RunEventKind::NodeTerminal {
            outcome: NodeOutcome::Succeeded,
            ..
        }
    )));
    let tick = runtime.scheduler_tick()?;
    assert_eq!(tick.completed, 1);
    assert_eq!(tick.dispatched, 0);
    assert_eq!(executor.entries.load(Ordering::SeqCst), 1);
    assert!(runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        milkdrift_persistence::RunEventKind::CapabilityResolutionDenied {
            authorization,
            ..
        } if authorization.reason_codes() == [DecisionReasonCode::Revoked]
    )));
    Ok(())
}

#[test]
fn denied_command_is_durable_idempotent_and_has_no_semantic_mutation() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let revision = sequence_revision()?;
    store.put_revision(&revision)?;
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let authority = Arc::new(DenyAuthority(AtomicUsize::new(0)));
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        Arc::new(DeterministicExecutor::new(descriptor)),
        authority.clone(),
        Arc::new(ManualClock::new(5_000)),
        Arc::new(SequentialIdGenerator::new("denied-command", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-denied")?,
            ActorRef::new("controller:denied")?,
            30_000,
            8,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 100, 0)?,
        )?,
    )?;
    let run = RunId::new("run-denied")?;
    let command = RunCommandDocument::new(
        CommandId::new("command-denied")?,
        run.clone(),
        ActorRef::new("human:denied")?,
        RunSequence::ZERO,
        TimestampMillis::new(5_000),
        Reason::new("must be denied")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-denied")?),
            workspace_budget: WorkspaceBudget::new(1, 1, 1, 0, 0, 0)?,
            inputs: Vec::new(),
        },
    )?;
    let first = runtime.handle_authorized_command(&command, &claim()?)?;
    assert_eq!(
        first.result().disposition(),
        milkdrift_persistence::CommandDisposition::Rejected
    );
    assert!(
        first
            .result()
            .authorization()
            .is_some_and(|value| !value.is_allowed())
    );
    assert_eq!(store.head(&run)?, RunSequence::ZERO);
    assert!(runtime.history(&run)?.is_empty());
    let replay = runtime.handle_authorized_command(&command, &claim()?)?;
    assert!(replay.replayed());
    assert_eq!(first.result(), replay.result());
    assert_eq!(authority.0.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn narrowed_placement_admission_requires_task_proof_and_retains_exact_host() -> TestResult {
    use milkdrift_capability::{PeerId, PlacementRequirement};
    let peer = PeerId::new("peer-approved")?;
    let base = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    let mut value = serde_json::to_value(base.body())?;
    value["locality"] = serde_json::json!("peer");
    value["peer"] = serde_json::json!(peer);
    let descriptor: milkdrift_capability::CapabilityDescriptor = serde_json::from_value(value)?;
    for (label, placement, accepted) in [
        (
            "pinned",
            Some(PlacementRequirement::new(
                None,
                Some(BTreeSet::from([peer.clone()])),
            )?),
            true,
        ),
        ("unconstrained", None, false),
        (
            "other",
            Some(PlacementRequirement::new(
                None,
                Some(BTreeSet::from([PeerId::new("peer-other")?])),
            )?),
            false,
        ),
    ] {
        let directory = TempDir::new()?;
        let store = Arc::new(RedbStore::open(directory.path())?);
        let actor = ActorRef::new("human:placement")?;
        let mut requirement = CapabilityRequirement::new(OperationId::new("model.generate")?);
        if let Some(placement) = placement {
            requirement = requirement.with_placement(placement);
        }
        let revision = sequence_revision_with_requirement(requirement)?;
        store.put_revision(&revision)?;
        let scope = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
            .only_localities(BTreeSet::from([Locality::Peer]))?
            .only_peers(BTreeSet::from([peer.clone()]))?
            .build();
        let (runtime, claim) = exact_grant_service(
            store.clone(),
            Arc::new(ManualClock::new(2000)),
            descriptor.clone(),
            &actor,
            revision.semantic().workflow(),
            BTreeSet::from([
                AuthorityOperation::CreateRun,
                AuthorityOperation::StartRun,
                AuthorityOperation::InvokeCapability,
            ]),
            scope,
            label,
        )?;
        let run = RunId::new(format!("run-{label}"))?;
        runtime.handle_authorized_command(
            &create_document(&revision, &run, &actor, "placement-create")?,
            &claim,
        )?;
        let start = RunCommandDocument::new(
            CommandId::new("placement-start")?,
            run.clone(),
            actor,
            store.head(&run)?,
            TimestampMillis::new(2000),
            Reason::new("verify task placement proof")?,
            Vec::new(),
            RunCommand::StartRun,
        )?;
        let result = runtime.handle_authorized_command(&start, &claim)?;
        assert_eq!(
            result.result().disposition() == milkdrift_persistence::CommandDisposition::Accepted,
            accepted
        );
        if accepted {
            assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
            let projection = runtime.projection(&run)?;
            let attempt = projection
                .attempts()
                .values()
                .next()
                .ok_or("attempt absent")?;
            let selection = attempt.capability().ok_or("selection absent")?;
            assert_eq!(selection.snapshot().peer(), Some(&peer));
            assert_eq!(
                selection
                    .authorization()
                    .ok_or("decision absent")?
                    .request()
                    .resources
                    .peer
                    .as_ref(),
                Some(&peer)
            );
        } else {
            assert!(runtime.projection(&run)?.attempts().is_empty());
        }
    }
    Ok(())
}
