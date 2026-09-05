use super::*;

#[test]
fn resolution_is_stable_policy_constrained_and_health_aware() -> TestResult {
    let a = descriptor("cap-a", 1, "profile-a", 2)?;
    let b = descriptor("cap-b", 1, "profile-b", 2)?;
    let host_one = host(BTreeMap::new(), 2)?;
    host_one.register(
        b.clone(),
        Arc::new(FakeAdapter::new(b.identity().clone())),
        Some(observation(&b, 100, true)?),
    )?;
    host_one.register(
        a.clone(),
        Arc::new(FakeAdapter::new(a.identity().clone())),
        Some(observation(&a, 100, true)?),
    )?;
    let host_two = host(BTreeMap::new(), 2)?;
    host_two.register(
        a.clone(),
        Arc::new(FakeAdapter::new(a.identity().clone())),
        Some(observation(&a, 100, true)?),
    )?;
    host_two.register(
        b.clone(),
        Arc::new(FakeAdapter::new(b.identity().clone())),
        Some(observation(&b, 100, true)?),
    )?;
    let requirement = CapabilityRequirement::new(OperationId::new("model.generate")?);
    assert_eq!(
        host_one
            .resolve_at(&requirement, 150)?
            .snapshot()
            .capability(),
        a.identity()
    );
    assert_eq!(
        host_two
            .resolve_at(&requirement, 150)?
            .snapshot()
            .capability(),
        a.identity()
    );

    let profile = requirement
        .clone()
        .provider_profile(ProviderProfileRef::new("profile-b")?);
    assert_eq!(
        host_one.resolve_at(&profile, 150)?.snapshot().capability(),
        b.identity()
    );
    assert!(matches!(
        host_one.resolve_at(&requirement, 201),
        Err(ExecutorError::Unavailable(_))
    ));
    let views = host_one.generations(
        &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
        201,
    )?;
    assert!(
        views
            .iter()
            .all(|view| view.health == GenerationHealth::Stale)
    );

    let priority_host = host(BTreeMap::from([(b.identity().clone(), 10)]), 2)?;
    priority_host.register(
        a.clone(),
        Arc::new(FakeAdapter::new(a.identity().clone())),
        Some(observation(&a, 100, true)?),
    )?;
    priority_host.register(
        b.clone(),
        Arc::new(FakeAdapter::new(b.identity().clone())),
        Some(observation(&b, 100, true)?),
    )?;
    assert_eq!(
        priority_host
            .resolve_at(&requirement, 150)?
            .snapshot()
            .capability(),
        b.identity()
    );

    let constrained_host = CapabilityHost::new(
        HostConfig {
            max_registrations: 8,
            max_generations_per_capability: 2,
            max_concurrent_per_generation: 2,
            observation_stale_after_ms: 100,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    constrained_host.register(
        a.clone(),
        Arc::new(FakeAdapter::new(a.identity().clone())),
        Some(observation(&a, 100, true)?),
    )?;
    constrained_host.register(
        b.clone(),
        Arc::new(FakeAdapter::new(b.identity().clone())),
        Some(observation(&b, 100, true)?),
    )?;
    assert_eq!(
        constrained_host
            .resolve_at(&requirement, 150)?
            .snapshot()
            .capability(),
        a.identity()
    );
    assert_eq!(
        constrained_host
            .generations(
                &CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
                    .only_capabilities(BTreeSet::from([a.identity().clone()]))?
                    .build(),
                150,
            )?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn exact_actor_authority_filters_identity_profile_locality_and_peer_before_health() -> TestResult {
    let operation = OperationId::new("model.generate")?;
    let requirement = CapabilityRequirement::new(operation.clone());

    let a = placed_descriptor("cap-a", "profile-a", Locality::Local, None)?;
    let b = placed_descriptor("cap-b", "profile-b", Locality::Local, None)?;
    let identity_host = host(BTreeMap::new(), 2)?;
    identity_host.register(
        a.clone(),
        Arc::new(FakeAdapter::new(a.identity().clone())),
        Some(observation(&a, 100, true)?),
    )?;
    identity_host.register(
        b.clone(),
        Arc::new(FakeAdapter::new(b.identity().clone())),
        Some(observation(&b, 100, true)?),
    )?;
    let identity_scope = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
        .only_capabilities(BTreeSet::from([b.identity().clone()]))?
        .only_operations(BTreeSet::from([operation.clone()]))?
        .build();
    let (identity_evaluator, identity_context) = exact_authority(identity_scope, BTreeSet::new())?;
    assert_eq!(
        identity_host
            .resolve_authorized_at(&requirement, &identity_context, &identity_evaluator, 150)?
            .snapshot()
            .capability(),
        b.identity()
    );

    let profile_scope = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
        .only_operations(BTreeSet::from([operation.clone()]))?
        .only_provider_profiles(BTreeSet::from([ProviderProfileRef::new("profile-a")?]))?
        .build();
    let (profile_evaluator, profile_context) = exact_authority(profile_scope, BTreeSet::new())?;
    assert_eq!(
        identity_host
            .resolve_authorized_at(&requirement, &profile_context, &profile_evaluator, 150)?
            .snapshot()
            .provider_profile(),
        Some(&ProviderProfileRef::new("profile-a")?)
    );

    let remote_peer = PeerId::new("peer:remote-a")?;
    let local = placed_descriptor("cap-local", "profile-local", Locality::Local, None)?;
    let peer = placed_descriptor(
        "cap-peer",
        "profile-peer",
        Locality::Peer,
        Some(remote_peer.clone()),
    )?;
    let placement_host = host(BTreeMap::new(), 2)?;
    placement_host.register(
        local.clone(),
        Arc::new(FakeAdapter::new(local.identity().clone())),
        Some(observation(&local, 100, false)?),
    )?;
    placement_host.register(
        peer.clone(),
        Arc::new(FakeAdapter::new(peer.identity().clone())),
        Some(observation(&peer, 100, true)?),
    )?;
    let local_scope = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
        .only_operations(BTreeSet::from([operation.clone()]))?
        .only_localities(BTreeSet::from([Locality::Local]))?
        .build();
    let (local_evaluator, local_context) = exact_authority(local_scope, BTreeSet::new())?;
    assert!(matches!(
        placement_host.resolve_authorized_at(&requirement, &local_context, &local_evaluator, 150,),
        Err(ExecutorError::Unavailable(_))
    ));

    placement_host.update_observation(
        local.identity(),
        local.descriptor_revision(),
        observation(&local, 110, true)?,
    )?;
    placement_host.update_observation(
        peer.identity(),
        peer.descriptor_revision(),
        observation(&peer, 110, false)?,
    )?;
    let peer_scope = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
        .only_operations(BTreeSet::from([operation]))?
        .only_localities(BTreeSet::from([Locality::Peer]))?
        .only_peers(BTreeSet::from([remote_peer]))?
        .build();
    let (peer_evaluator, peer_context) = exact_authority(peer_scope, BTreeSet::new())?;
    assert!(matches!(
        placement_host.resolve_authorized_at(&requirement, &peer_context, &peer_evaluator, 150,),
        Err(ExecutorError::Unavailable(_))
    ));
    Ok(())
}

#[test]
fn deny_all_hides_catalog_and_returns_authorization_denial_not_absence() -> TestResult {
    let operation = OperationId::new("model.generate")?;
    let descriptor = placed_descriptor("cap-denied", "profile-denied", Locality::Local, None)?;
    let denied_host = host(BTreeMap::new(), 1)?;
    denied_host.register(
        descriptor.clone(),
        Arc::new(FakeAdapter::new(descriptor.identity().clone())),
        Some(observation(&descriptor, 100, true)?),
    )?;
    let deny_all = CapabilityAuthorityScope::deny_all();
    assert!(denied_host.generations(&deny_all, 150)?.is_empty());

    let (evaluator, context) = exact_authority(deny_all, BTreeSet::new())?;
    let denied = denied_host.resolve_authorized_at(
        &CapabilityRequirement::new(operation),
        &context,
        &evaluator,
        150,
    );
    assert!(matches!(
        denied,
        Err(ExecutorError::AuthorityDenied { reasons, .. })
            if reasons.contains(&DecisionReasonCode::CapabilityMismatch)
    ));
    Ok(())
}

#[test]
fn declared_adapter_resources_are_denied_before_unavailable_health() -> TestResult {
    let capability = placed_descriptor(
        "cap-secret-model",
        "profile-secret-model",
        Locality::Remote,
        None,
    )?;
    let required_secret = SecretRef::new("secret:model-api")?;
    let resource_host = host(BTreeMap::new(), 2)?;
    resource_host.register(
        capability.clone(),
        Arc::new(FakeAdapter::with_authority_requirements(
            capability.identity().clone(),
            CapabilityExecutionRequirements {
                secrets: BTreeSet::from([required_secret.clone()]),
                ..CapabilityExecutionRequirements::default()
            },
        )),
        Some(observation(&capability, 100, false)?),
    )?;
    let scope = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
        .only_capabilities(BTreeSet::from([capability.identity().clone()]))?
        .only_operations(BTreeSet::from([OperationId::new("model.generate")?]))?
        .build();
    let (denying_evaluator, denying_context) = exact_authority(scope.clone(), BTreeSet::new())?;
    let denied = resource_host.resolve_authorized_at(
        &CapabilityRequirement::new(OperationId::new("model.generate")?),
        &denying_context,
        &denying_evaluator,
        150,
    );
    match denied {
        Err(ExecutorError::AuthorityDenied { reasons, decision }) => {
            assert!(reasons.contains(&DecisionReasonCode::SecretScopeMismatch));
            assert!(
                decision
                    .request()
                    .resources
                    .secrets
                    .contains(&required_secret)
            );
        }
        other => return Err(format!("expected typed authority denial, got {other:?}").into()),
    }
    let (allowing_evaluator, allowing_context) =
        exact_authority(scope, BTreeSet::from([required_secret]))?;
    assert!(matches!(
        resource_host.resolve_authorized_at(
            &CapabilityRequirement::new(OperationId::new("model.generate")?),
            &allowing_context,
            &allowing_evaluator,
            150,
        ),
        Err(ExecutorError::Unavailable(_))
    ));
    Ok(())
}

#[test]
fn visible_generation_operations_are_filtered_by_the_exact_selector() -> TestResult {
    let allowed = OperationId::new("model.generate")?;
    let denied = OperationId::new("model.embed")?;
    let base = descriptor("cap-operation-filter", 1, "profile-filter", 1)?;
    let contract = base
        .operation(&allowed)
        .ok_or("fixture descriptor lacks model.generate")?
        .clone();
    let mut operations = base.operations().clone();
    operations.insert(denied.clone(), contract);
    let descriptor = DescriptorBuilder::new(
        base.identity().clone(),
        base.descriptor_revision(),
        base.category().clone(),
        base.admission().clone(),
        base.locality(),
    )
    .provider_profile(base.provider_profile().cloned())
    .operations(operations)
    .trust_zones(base.trust_zones().clone())
    .execution_trust(base.execution_trust())
    .resource_observations(base.resource_observations().cloned())
    .labels(base.labels().clone())
    .extensions(base.extensions().clone())
    .build()?;
    let host = host(BTreeMap::new(), 1)?;
    host.register(
        descriptor.clone(),
        Arc::new(FakeAdapter::new(descriptor.identity().clone())),
        Some(observation(&descriptor, 100, true)?),
    )?;

    let allow_one = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
        .only_operations(BTreeSet::from([allowed.clone()]))?
        .build();
    let views = host.generations(&allow_one, 150)?;
    assert_eq!(views.len(), 1);
    assert_eq!(
        views[0]
            .operation_contracts
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([allowed])
    );
    assert!(!views[0].operation_contracts.contains_key(&denied));

    let deny_every_operation = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
        .only_operations(BTreeSet::from([OperationId::new("model.missing")?]))?
        .build();
    assert!(host.generations(&deny_every_operation, 150)?.is_empty());
    assert!(host.catalog_generations(&deny_every_operation)?.is_empty());
    Ok(())
}
fn placed_descriptor(
    identity: &str,
    profile: &str,
    locality: Locality,
    peer: Option<PeerId>,
) -> TestResult<CapabilityDescriptor> {
    let base = descriptor(identity, 1, profile, 2)?;
    Ok(DescriptorBuilder::new(
        base.identity().clone(),
        base.descriptor_revision(),
        base.category().clone(),
        base.admission().clone(),
        locality,
    )
    .peer(peer)
    .provider_profile(base.provider_profile().cloned())
    .operations(base.operations().clone())
    .trust_zones(base.trust_zones().clone())
    .execution_trust(base.execution_trust())
    .resource_observations(base.resource_observations().cloned())
    .labels(base.labels().clone())
    .extensions(base.extensions().clone())
    .build()?)
}

fn exact_authority(
    capability: CapabilityAuthorityScope,
    secrets: BTreeSet<SecretRef>,
) -> TestResult<(GrantSetEvaluator, CapabilityResolutionContext)> {
    let actor = ActorRef::new("human:host-authority-test")?;
    let grant_id = GrantId::new("grant:host-authority-test")?;
    let workflow = WorkflowId::new("host-authority-test")?;
    let run = RunId::new("run-host-authority-test")?;
    let budget = AuthorityBudget {
        cost_minor: Some(u64::MAX),
        duration_ms: Some(u64::MAX),
        invocations: Some(u64::MAX),
        artifact_bytes: Some(u64::MAX),
        units: Some(u64::MAX),
        concurrency: Some(u32::MAX),
    };
    let grant = AuthorityGrantBuilder::new(grant_id.clone(), 1, actor.clone())
        .operations(BTreeSet::from([
            AuthorityOperation::StartRun,
            AuthorityOperation::InvokeCapability,
        ]))
        .resources(ResourceScope {
            workflow_run: WorkflowRunScope::Workflow {
                workflow: workflow.clone(),
            },
            capability,
            filesystem: Vec::new(),
            network: NetworkScope::empty(),
            secrets,
            artifacts: milkdrift_authority::ArtifactAuthorityScope::none(),
            layouts: milkdrift_authority::LayoutAuthorityScope::none(),
            peers: milkdrift_authority::PeerAuthorityScope::none(),
            daemon: milkdrift_authority::DaemonAuthorityScope::default(),
            workspace: milkdrift_authority::WorkspaceAuthorityScope::none(),
        })
        .budget(budget)
        .validity(BoundaryTimeMillis::new(0), BoundaryTimeMillis::new(1_000))
        .build()?;
    let digest = grant.digest()?;
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("test.host-exact-authority")?,
        1,
        [grant],
        BTreeMap::new(),
    )?;
    let mut resources = RequestedResourceFacts::empty();
    resources.workflow = Some(workflow.clone());
    resources.run = Some(run.clone());
    let start = AuthorityRequest {
        decision: DecisionId::new("decision:host-start")?,
        actor,
        grant: grant_id,
        grant_revision: 1,
        grant_digest: digest,
        revocation_generation: 0,
        operation: AuthorityOperation::StartRun,
        resources,
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(100),
        provenance: AuthorityExecutionProvenance::default(),
    };
    let start_decision = evaluator.evaluate(&start)?;
    assert!(start_decision.is_allowed());
    let revision: RevisionId =
        serde_json::from_value(serde_json::json!(format!("rev_{}", "1".repeat(64))))?;
    let basis = ExecutionAuthorityBasis::from_start_decision(
        &start_decision,
        workflow,
        run,
        revision.clone(),
    )?;
    let context = CapabilityResolutionContext::new(
        basis,
        revision,
        NodeId::new("task")?,
        NodeExecutionId::new("execution-host-authority")?,
        AttemptId::new("attempt-host-authority")?,
    );
    Ok((evaluator, context))
}
