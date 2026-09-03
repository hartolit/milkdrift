//! Deterministic live-registry, generation, admission, cancellation, and shutdown evidence.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityEvaluator, AuthorityExecutionProvenance,
    AuthorityGrantBuilder, AuthorityOperation, AuthorityRequest, BoundaryTimeMillis,
    CapabilityAuthorityScope, CapabilityAuthorityScopeBuilder, CapabilityExecutionRequirements,
    DecisionId, DecisionReasonCode, ExecutionAuthorityBasis, GrantId, GrantSetEvaluator,
    NetworkScope, PolicyId, RequestedResourceFacts, ResourceScope, SecretRef, WorkflowRunScope,
};
use milkdrift_blueprint::{NodeId, RevisionId, WorkflowId};
use milkdrift_capability::{
    AdmissionConstraints, CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor,
    CapabilityDescriptorDocument, CapabilityId, CapabilityObservation, CapabilityRequirement,
    DescriptorBuilder, InvocationAdmissionEnvelope, InvocationEvent, InvocationEventKind,
    InvocationId, InvocationRequest, InvocationTerminal, Locality, OperationId, PeerId,
    ProviderProfileRef, ResolvedCapabilitySnapshot, SideEffectClass, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterFailureKind, AdapterInvocation, AdapterReporter, CapabilityAdapter,
    CapabilityHost, CapabilitySelectionPolicy, GenerationHealth, HostConfig, HostError,
    InMemorySecretResolver, RegistrationOutcome, SecretResolver,
};
use milkdrift_persistence::{AttemptId, NodeExecutionId};
use milkdrift_runtime::{CapabilityResolutionContext, ExecutorError};
use milkdrift_workspace::RunId;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn descriptor(
    identity: &str,
    revision: u64,
    profile: &str,
    limit: u32,
) -> TestResult<CapabilityDescriptor> {
    let base = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    Ok(DescriptorBuilder::new(
        CapabilityId::new(identity)?,
        revision,
        base.category().clone(),
        AdmissionConstraints::new(limit, 0)?,
        base.locality(),
    )
    .provider_profile(Some(ProviderProfileRef::new(profile)?))
    .operations(base.operations().clone())
    .trust_zones(base.trust_zones().clone())
    .execution_trust(base.execution_trust())
    .resource_observations(base.resource_observations().cloned())
    .labels(base.labels().clone())
    .extensions(base.extensions().clone())
    .build()?)
}

fn observation(
    descriptor: &CapabilityDescriptor,
    at: u64,
    available: bool,
) -> TestResult<CapabilityObservation> {
    Ok(CapabilityObservation::new(
        descriptor.identity().clone(),
        at,
        available,
        0,
        if available { "healthy" } else { "unavailable" },
    )?)
}

fn host(
    priorities: BTreeMap<CapabilityId, i32>,
    max_generations: usize,
) -> TestResult<CapabilityHost> {
    Ok(CapabilityHost::new(
        HostConfig {
            max_registrations: 8,
            max_generations_per_capability: max_generations,
            max_concurrent_per_generation: 2,
            observation_stale_after_ms: 100,
        },
        CapabilitySelectionPolicy::priorities(priorities),
    )?)
}

#[derive(Default)]
struct AdapterControl {
    entered: bool,
    release: bool,
}

struct FakeAdapter {
    capability: CapabilityId,
    execute_count: AtomicUsize,
    cancel_count: AtomicUsize,
    panic_execute: AtomicBool,
    block: Option<Arc<(Mutex<AdapterControl>, Condvar)>>,
    authority_requirements: CapabilityExecutionRequirements,
}

impl FakeAdapter {
    fn new(capability: CapabilityId) -> Self {
        Self {
            capability,
            execute_count: AtomicUsize::new(0),
            cancel_count: AtomicUsize::new(0),
            panic_execute: AtomicBool::new(false),
            block: None,
            authority_requirements: CapabilityExecutionRequirements::default(),
        }
    }

    fn blocking(capability: CapabilityId, block: Arc<(Mutex<AdapterControl>, Condvar)>) -> Self {
        Self {
            block: Some(block),
            ..Self::new(capability)
        }
    }

    fn with_authority_requirements(
        capability: CapabilityId,
        authority_requirements: CapabilityExecutionRequirements,
    ) -> Self {
        Self {
            authority_requirements,
            ..Self::new(capability)
        }
    }
}

impl CapabilityAdapter for FakeAdapter {
    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(InvocationAdmissionEnvelope::not_applicable())
    }

    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.authority_requirements.clone()
    }

    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.execute_count.fetch_add(1, Ordering::SeqCst);
        assert!(
            !self.panic_execute.load(Ordering::SeqCst),
            "fake adapter panic"
        );
        if let Some(block) = &self.block {
            let (lock, changed) = &**block;
            let mut state = lock
                .lock()
                .map_err(|_error| AdapterError::external_failure("block lock"))?;
            state.entered = true;
            changed.notify_all();
            while !state.release {
                state = changed
                    .wait(state)
                    .map_err(|_error| AdapterError::external_failure("block wait"))?;
            }
        }
        let terminal = InvocationTerminal::new(
            TerminalStatus::Success,
            Vec::new(),
            None,
            None,
            invocation.resolution().operation_contract().side_effect(),
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        let event = InvocationEvent::new(
            invocation.request().invocation().clone(),
            1,
            InvocationEventKind::Terminal { terminal },
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        reporter.invocation(event)
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        self.cancel_count.fetch_add(1, Ordering::SeqCst);
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            true,
            false,
            None,
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            self.capability.clone(),
            observed_at_unix_ms,
            true,
            0,
            "healthy",
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }
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

struct FailingAdapter {
    capability: CapabilityId,
    kind: AdapterFailureKind,
}

impl CapabilityAdapter for FailingAdapter {
    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(InvocationAdmissionEnvelope::not_applicable())
    }

    fn execute(
        &self,
        _invocation: &AdapterInvocation<'_>,
        _reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        Err(match self.kind {
            AdapterFailureKind::Rejected => AdapterError::rejected("planned rejection"),
            AdapterFailureKind::Unavailable => AdapterError::unavailable("planned unavailable"),
            AdapterFailureKind::ExternalFailure => {
                AdapterError::external_failure("planned external failure")
            }
        })
    }

    fn cancel(
        &self,
        _request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        Err(AdapterError::unavailable("planned unavailable"))
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            self.capability.clone(),
            observed_at_unix_ms,
            true,
            0,
            "healthy",
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }
}

#[derive(Default)]
struct CountingReporter(AtomicUsize);

impl AdapterReporter for CountingReporter {
    fn invocation(&self, _event: InvocationEvent) -> Result<(), AdapterError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn request(snapshot: &ResolvedCapabilitySnapshot, identity: &str) -> TestResult<InvocationRequest> {
    Ok(InvocationRequest::new(
        InvocationId::new(identity)?,
        snapshot.capability().clone(),
        snapshot.operation().clone(),
        snapshot.provider_profile().cloned(),
        None,
        Vec::new(),
        BTreeMap::new(),
    )?)
}

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
fn descriptor_generations_are_immutable_drained_and_never_fallback() -> TestResult {
    let host = host(BTreeMap::new(), 2)?;
    let old = descriptor("cap-generation", 1, "profile-generation", 2)?;
    let new = descriptor("cap-generation", 2, "profile-generation", 2)?;
    let old_adapter = Arc::new(FakeAdapter::new(old.identity().clone()));
    assert_eq!(
        host.register(
            old.clone(),
            old_adapter.clone(),
            Some(observation(&old, 100, true)?)
        )?,
        RegistrationOutcome::Registered
    );
    assert_eq!(
        host.register(
            old.clone(),
            old_adapter,
            Some(observation(&old, 100, true)?)
        )?,
        RegistrationOutcome::Idempotent
    );
    let conflict = descriptor("cap-generation", 1, "other-profile", 2)?;
    assert!(matches!(
        host.register(
            conflict,
            Arc::new(FakeAdapter::new(old.identity().clone())),
            None
        ),
        Err(HostError::ConflictingRevision { .. })
    ));

    let requirement = CapabilityRequirement::new(OperationId::new("model.generate")?)
        .exact(old.identity().clone());
    let old_snapshot = host.resolve_at(&requirement, 150)?.snapshot().clone();
    host.register(
        new.clone(),
        Arc::new(FakeAdapter::new(new.identity().clone())),
        Some(observation(&new, 100, true)?),
    )?;
    assert_eq!(
        host.resolve_at(&requirement, 150)?
            .snapshot()
            .descriptor_revision(),
        2
    );
    let reporter = CountingReporter::default();
    host.execute_exact(
        &old_snapshot,
        &request(&old_snapshot, "invocation-old")?,
        &reporter,
    )?;
    host.begin_drain(old.identity(), 1)?;
    host.execute_exact(
        &old_snapshot,
        &request(&old_snapshot, "invocation-old-draining")?,
        &reporter,
    )?;
    host.finish_drain(old.identity(), 1)?;
    assert!(matches!(
        host.execute_exact(
            &old_snapshot,
            &request(&old_snapshot, "invocation-removed")?,
            &reporter
        ),
        Err(ExecutorError::UnavailableGeneration { .. })
    ));
    assert_eq!(reporter.0.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn permits_cancel_exact_owner_and_release_after_panic_and_completion() -> TestResult {
    let host = host(BTreeMap::new(), 2)?;
    let descriptor = descriptor("cap-admission", 1, "profile-admission", 1)?;
    let block = Arc::new((Mutex::new(AdapterControl::default()), Condvar::new()));
    let adapter = Arc::new(FakeAdapter::blocking(
        descriptor.identity().clone(),
        block.clone(),
    ));
    host.register(
        descriptor.clone(),
        adapter.clone(),
        Some(observation(&descriptor, 100, true)?),
    )?;
    let requirement = CapabilityRequirement::new(OperationId::new("model.generate")?);
    let snapshot = host.resolve_at(&requirement, 150)?.snapshot().clone();
    let first_request = request(&snapshot, "invocation-blocked")?;
    let thread_host = host.clone();
    let thread_snapshot = snapshot.clone();
    let thread_request = first_request.clone();
    let thread = std::thread::spawn(move || {
        thread_host.execute_exact(
            &thread_snapshot,
            &thread_request,
            &CountingReporter::default(),
        )
    });
    {
        let (lock, changed) = &*block;
        let mut state = lock.lock().map_err(|_error| "block lock poisoned")?;
        while !state.entered {
            state = changed
                .wait(state)
                .map_err(|_error| "block wait poisoned")?;
        }
    }
    assert!(matches!(
        host.execute_exact(
            &snapshot,
            &request(&snapshot, "invocation-overload")?,
            &CountingReporter::default()
        ),
        Err(ExecutorError::Overloaded(_))
    ));
    let cancellation = CancellationRequest::new(first_request.invocation().clone(), 1, "stop")?;
    assert!(milkdrift_runtime::TaskExecutor::cancel(&host, &cancellation)?.accepted());
    assert_eq!(adapter.cancel_count.load(Ordering::SeqCst), 1);
    {
        let (lock, changed) = &*block;
        let mut state = lock.lock().map_err(|_error| "block lock poisoned")?;
        state.release = true;
        changed.notify_all();
    }
    thread
        .join()
        .map_err(|_panic| "adapter thread panicked")??;
    let views = host.generations(
        &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
        150,
    )?;
    assert_eq!(views[0].active_permits, 0);

    adapter.panic_execute.store(true, Ordering::SeqCst);
    assert!(matches!(
        host.execute_exact(
            &snapshot,
            &request(&snapshot, "invocation-panic")?,
            &CountingReporter::default()
        ),
        Err(ExecutorError::AdapterPanicked { after_entry: true })
    ));
    let views = host.generations(
        &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
        150,
    )?;
    assert_eq!(views[0].active_permits, 0);
    Ok(())
}

#[test]
fn adapter_failures_preserve_pre_entry_and_post_entry_uncertainty() -> TestResult {
    for (identity, kind, expected_after_entry) in [
        ("cap-rejected", AdapterFailureKind::Rejected, false),
        (
            "cap-external-failure",
            AdapterFailureKind::ExternalFailure,
            true,
        ),
    ] {
        let host = host(BTreeMap::new(), 1)?;
        let descriptor = descriptor(identity, 1, "profile-failure", 1)?;
        host.register(
            descriptor.clone(),
            Arc::new(FailingAdapter {
                capability: descriptor.identity().clone(),
                kind,
            }),
            Some(observation(&descriptor, 100, true)?),
        )?;
        let snapshot = host
            .resolve_at(
                &CapabilityRequirement::new(OperationId::new("model.generate")?),
                150,
            )?
            .snapshot()
            .clone();
        let error = match host.execute_exact(
            &snapshot,
            &request(&snapshot, &format!("invocation-{identity}"))?,
            &CountingReporter::default(),
        ) {
            Ok(()) => return Err("planned adapter failure did not propagate".into()),
            Err(error) => error,
        };
        assert_eq!(
            matches!(&error, ExecutorError::BoundaryAfterEntry(_)),
            expected_after_entry
        );
        assert_eq!(
            matches!(&error, ExecutorError::BoundaryBeforeEntry(_)),
            !expected_after_entry
        );
        assert_eq!(
            host.generations(
                &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
                150,
            )?[0]
                .active_permits,
            0
        );
    }
    Ok(())
}

#[test]
fn registry_bounds_shutdown_and_secret_resolution_are_explicit() -> TestResult {
    let host = host(BTreeMap::new(), 1)?;
    let first = descriptor("cap-bounded", 1, "profile-bounded", 1)?;
    let second = descriptor("cap-bounded", 2, "profile-bounded", 1)?;
    host.register(
        first.clone(),
        Arc::new(FakeAdapter::new(first.identity().clone())),
        Some(observation(&first, 100, true)?),
    )?;
    assert!(matches!(
        host.register(
            second,
            Arc::new(FakeAdapter::new(first.identity().clone())),
            None
        ),
        Err(HostError::RegistryBound("retained generations"))
    ));
    assert!(host.shutdown()?.unresolved_invocations.is_empty());
    assert!(matches!(
        host.resolve_at(
            &CapabilityRequirement::new(OperationId::new("model.generate")?),
            150
        ),
        Err(ExecutorError::AdmissionClosed)
    ));

    let resolver = InMemorySecretResolver::new();
    let reference = SecretRef::new("secret:test")?;
    resolver.insert(reference.clone(), b"super-secret".to_vec())?;
    let value = resolver.resolve(&reference)?;
    assert_eq!(value.expose(|bytes| bytes.len()), 12);
    assert!(!format!("{resolver:?} {reference:?} {value:?}").contains("super-secret"));
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
    assert_eq!(views[0].operations, BTreeSet::from([allowed]));
    assert!(!views[0].operations.contains(&denied));

    let deny_every_operation = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
        .only_operations(BTreeSet::from([OperationId::new("model.missing")?]))?
        .build();
    assert!(host.generations(&deny_every_operation, 150)?.is_empty());
    assert!(host.catalog_generations(&deny_every_operation)?.is_empty());
    Ok(())
}
