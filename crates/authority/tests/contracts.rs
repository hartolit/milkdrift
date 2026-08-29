//! Independent authority schema, policy, type-separation, and redaction evidence.

use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
};

use milkdrift_authority::{
    AccessMode, ActorRef, ArtifactAuthorityScope, AuthorityBudget, AuthorityEvaluator,
    AuthorityExecutionProvenance, AuthorityGrant, AuthorityGrantBuilder, AuthorityOperation,
    AuthorityRequest, BoundaryTimeMillis, CapabilityAuthorityScope, DaemonAuthorityScope,
    DecisionId, DecisionReasonCode, FilesystemScope, GrantId, GrantSetEvaluator,
    LayoutAuthorityScope, NetworkProfileRef, NetworkScope, PeerAuthorityScope, PolicyId,
    RequestedResourceFacts, ResourceScope, SecretRef, SensitiveSecret, WorkflowRunScope,
    WorkspaceAuthorityScope,
};
use milkdrift_blueprint::{AuthorRef, WorkflowId};
use milkdrift_capability::{
    CapabilityCategory, CapabilityId, ExecutionTrustClass, Locality, OperationId,
    ProviderProfileRef, SideEffectClass, TrustZone,
};
use milkdrift_workspace::{ArtifactId, ArtifactSensitivity, RunId};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn set<T: Ord>(value: T) -> BTreeSet<T> {
    BTreeSet::from([value])
}

fn grant() -> TestResult<AuthorityGrant> {
    Ok(AuthorityGrantBuilder::new(
        GrantId::new("grant:alice")?,
        3,
        ActorRef::new("human:alice")?,
    )
    .operations(set(AuthorityOperation::Approve))
    .resources(ResourceScope {
        workflow_run: WorkflowRunScope::Run {
            run: RunId::new("run-a")?,
            workflow: Some(WorkflowId::new("workflow-a")?),
        },
        capability: CapabilityAuthorityScope::new(
            set(CapabilityId::new("cap-a")?),
            set(CapabilityCategory::Model),
            set(OperationId::new("model.generate")?),
            set(ProviderProfileRef::new("profile-a")?),
            set(TrustZone::new("trusted")?),
            set(Locality::Remote),
            SideEffectClass::ReadOnly,
        )?,
        filesystem: vec![FilesystemScope::new("/workspace", set(AccessMode::Read))?],
        network: NetworkScope::new(
            set(NetworkProfileRef::new("network-default")?),
            set("example.com:443".to_owned()),
        )?,
        secrets: set(SecretRef::new("secret:provider-api")?),
        artifacts: ArtifactAuthorityScope::none(),
        layouts: LayoutAuthorityScope::none(),
        peers: PeerAuthorityScope::none(),
        daemon: DaemonAuthorityScope::default(),
        workspace: WorkspaceAuthorityScope::none(),
    })
    .budget(AuthorityBudget {
        cost_minor: Some(100),
        duration_ms: Some(10_000),
        invocations: Some(5),
        artifact_bytes: Some(1_024),
        units: Some(10_000),
        concurrency: Some(2),
    })
    .validity(BoundaryTimeMillis::new(100), BoundaryTimeMillis::new(200))
    .revocation_generation(4)
    .build()?)
}

fn request() -> TestResult<AuthorityRequest> {
    let grant_digest = grant()?.digest()?;
    Ok(AuthorityRequest {
        decision: DecisionId::new("decision:a")?,
        actor: ActorRef::new("human:alice")?,
        grant: GrantId::new("grant:alice")?,
        grant_revision: 3,
        grant_digest,
        revocation_generation: 4,
        operation: AuthorityOperation::Approve,
        resources: RequestedResourceFacts {
            workflow: Some(WorkflowId::new("workflow-a")?),
            run: Some(RunId::new("run-a")?),
            capability: Some(CapabilityId::new("cap-a")?),
            category: Some(CapabilityCategory::Model),
            capability_operation: Some(OperationId::new("model.generate")?),
            provider_profile: Some(ProviderProfileRef::new("profile-a")?),
            trust_zone: Some(TrustZone::new("trusted")?),
            trust_zones: set(TrustZone::new("trusted")?),
            execution_trust_class: None,
            locality: Some(Locality::Remote),
            peer: None,
            capability_envelope: None,
            side_effect: SideEffectClass::ReadOnly,
            filesystem: vec![FilesystemScope::new(
                "/workspace/input",
                set(AccessMode::Read),
            )?],
            network_profiles: set(NetworkProfileRef::new("network-default")?),
            network_destinations: set("example.com:443".to_owned()),
            secrets: set(SecretRef::new("secret:provider-api")?),
            artifact: None,
            artifact_sensitivity: None,
            revision: None,
            layout_owner: None,
            workspace_scope: None,
            daemon_readiness: false,
            daemon_detailed_health: false,
            daemon_own_authority: false,
            daemon_configuration: false,
            daemon_audit: false,
        },
        budget: AuthorityBudget {
            cost_minor: Some(10),
            duration_ms: Some(1_000),
            invocations: Some(1),
            artifact_bytes: Some(128),
            units: Some(100),
            concurrency: Some(1),
        },
        evaluated_at: BoundaryTimeMillis::new(150),
        provenance: AuthorityExecutionProvenance::default(),
    })
}

#[test]
fn grants_can_independently_permit_or_deny_trusted_host_processes() -> TestResult {
    let actor = ActorRef::new("service:trusted-process-runner")?;
    let trusted_scope = CapabilityAuthorityScope::any(SideEffectClass::Unknown)
        .with_execution_trust_classes(set(ExecutionTrustClass::TrustedHostProcess))?;
    let trusted_grant =
        AuthorityGrantBuilder::new(GrantId::new("grant:trusted-process")?, 1, actor.clone())
            .operations(set(AuthorityOperation::InvokeCapability))
            .resources(ResourceScope {
                workflow_run: WorkflowRunScope::Any,
                capability: trusted_scope,
                filesystem: Vec::new(),
                network: NetworkScope::empty(),
                secrets: BTreeSet::new(),
                artifacts: ArtifactAuthorityScope::none(),
                layouts: LayoutAuthorityScope::none(),
                peers: PeerAuthorityScope::none(),
                daemon: DaemonAuthorityScope::default(),
                workspace: WorkspaceAuthorityScope::none(),
            })
            .build()?;
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("policy:trusted-process")?,
        1,
        [trusted_grant.clone()],
        BTreeMap::new(),
    )?;
    let mut resources = RequestedResourceFacts::empty();
    resources.capability = Some(CapabilityId::new("trusted-tool")?);
    resources.category = Some(CapabilityCategory::Process);
    resources.capability_operation = Some(OperationId::new("process.execute")?);
    resources.execution_trust_class = Some(ExecutionTrustClass::TrustedHostProcess);
    let allowed = AuthorityRequest {
        decision: DecisionId::new("decision:trusted-process-allowed")?,
        actor: actor.clone(),
        grant: trusted_grant.identity().clone(),
        grant_revision: trusted_grant.revision(),
        grant_digest: trusted_grant.digest()?,
        revocation_generation: 0,
        operation: AuthorityOperation::InvokeCapability,
        resources: resources.clone(),
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(1),
        provenance: AuthorityExecutionProvenance::default(),
    };
    assert!(evaluator.evaluate(&allowed)?.is_allowed());

    let sandbox_only =
        AuthorityGrantBuilder::new(GrantId::new("grant:sandbox-only")?, 1, actor.clone())
            .operations(set(AuthorityOperation::InvokeCapability))
            .resources(ResourceScope {
                workflow_run: WorkflowRunScope::Any,
                capability: CapabilityAuthorityScope::any(SideEffectClass::Unknown)
                    .with_execution_trust_classes(set(ExecutionTrustClass::SandboxedProcess))?,
                filesystem: Vec::new(),
                network: NetworkScope::empty(),
                secrets: BTreeSet::new(),
                artifacts: ArtifactAuthorityScope::none(),
                layouts: LayoutAuthorityScope::none(),
                peers: PeerAuthorityScope::none(),
                daemon: DaemonAuthorityScope::default(),
                workspace: WorkspaceAuthorityScope::none(),
            })
            .build()?;
    let sandbox_evaluator = GrantSetEvaluator::new(
        PolicyId::new("policy:sandbox-only")?,
        1,
        [sandbox_only.clone()],
        BTreeMap::new(),
    )?;
    let denied = AuthorityRequest {
        decision: DecisionId::new("decision:trusted-process-denied")?,
        actor,
        grant: sandbox_only.identity().clone(),
        grant_revision: sandbox_only.revision(),
        grant_digest: sandbox_only.digest()?,
        revocation_generation: 0,
        operation: AuthorityOperation::InvokeCapability,
        resources,
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(1),
        provenance: AuthorityExecutionProvenance::default(),
    };
    let decision = sandbox_evaluator.evaluate(&denied)?;
    assert!(!decision.is_allowed());
    assert!(
        decision
            .reason_codes()
            .contains(&DecisionReasonCode::PlacementMismatch)
    );
    Ok(())
}

fn evaluate(
    request: &AuthorityRequest,
) -> TestResult<milkdrift_authority::AuthorityDecisionSnapshot> {
    let grant = grant()?;
    Ok(GrantSetEvaluator::new(
        PolicyId::new("policy:core")?,
        7,
        [grant],
        BTreeMap::from([(GrantId::new("grant:alice")?, 4)]),
    )?
    .evaluate(request)?)
}

#[test]
fn actor_author_grant_and_capability_identities_are_distinct_types() {
    assert_ne!(TypeId::of::<ActorRef>(), TypeId::of::<AuthorRef>());
    assert_ne!(TypeId::of::<ActorRef>(), TypeId::of::<GrantId>());
    assert_ne!(TypeId::of::<GrantId>(), TypeId::of::<CapabilityId>());
}

#[test]
fn grant_schema_has_a_canonical_golden_fixture_and_hostile_bounds() -> TestResult {
    let grant = AuthorityGrantBuilder::new(
        GrantId::new("grant:golden")?,
        1,
        ActorRef::new("human:golden")?,
    )
    .operations(set(AuthorityOperation::Inspect))
    .build()?;
    assert_eq!(
        grant.to_canonical_json()?,
        String::from_utf8(include_bytes!("fixtures/authority-grant-v2.json").to_vec())?
            .trim()
            .as_bytes()
    );
    assert_eq!(
        AuthorityGrant::from_json(&grant.to_canonical_json()?)?,
        grant
    );
    let mut old: serde_json::Value = serde_json::from_slice(&grant.to_canonical_json()?)?;
    old["schema_version"] = serde_json::json!(1);
    assert!(AuthorityGrant::from_json(&serde_json::to_vec(&old)?).is_err());
    assert!(AuthorityGrant::from_json(br#"{"schema_version":1,"schema_version":1}"#).is_err());
    assert!(ActorRef::new("x".repeat(193)).is_err());
    assert!(FilesystemScope::new("/workspace/../secret", set(AccessMode::Read)).is_err());
    assert!(NetworkScope::new(BTreeSet::new(), set("user@example.com".to_owned())).is_err());
    let mut hostile: serde_json::Value = serde_json::from_slice(&grant.to_canonical_json()?)?;
    hostile["resources"]["filesystem"] = serde_json::json!([{
        "root": "/workspace/../secret",
        "access": ["read"]
    }]);
    assert!(AuthorityGrant::from_json(&serde_json::to_vec(&hostile)?).is_err());
    Ok(())
}

#[test]
fn evaluation_covers_allow_and_every_stable_denial_family() -> TestResult {
    let base = request()?;
    assert!(evaluate(&base)?.is_allowed());

    let mut wrong_actor = base.clone();
    wrong_actor.actor = ActorRef::new("human:bob")?;
    assert!(
        evaluate(&wrong_actor)?
            .reason_codes()
            .contains(&DecisionReasonCode::WrongActor)
    );
    let mut stale = base.clone();
    stale.grant_revision = 2;
    assert!(
        evaluate(&stale)?
            .reason_codes()
            .contains(&DecisionReasonCode::GrantRevisionMismatch)
    );
    let mut revoked = base.clone();
    revoked.revocation_generation = 3;
    assert!(
        evaluate(&revoked)?
            .reason_codes()
            .contains(&DecisionReasonCode::Revoked)
    );
    let mut early = base.clone();
    early.evaluated_at = BoundaryTimeMillis::new(99);
    assert!(
        evaluate(&early)?
            .reason_codes()
            .contains(&DecisionReasonCode::NotYetValid)
    );
    let mut expired = base.clone();
    expired.evaluated_at = BoundaryTimeMillis::new(201);
    assert!(
        evaluate(&expired)?
            .reason_codes()
            .contains(&DecisionReasonCode::Expired)
    );
    let mut operation = base.clone();
    operation.operation = AuthorityOperation::Apply;
    assert!(
        evaluate(&operation)?
            .reason_codes()
            .contains(&DecisionReasonCode::OperationMismatch)
    );

    let mut wrong_run = base.clone();
    wrong_run.resources.run = Some(RunId::new("run-b")?);
    assert!(
        evaluate(&wrong_run)?
            .reason_codes()
            .contains(&DecisionReasonCode::WorkflowRunMismatch)
    );
    let mut wrong_workflow = base.clone();
    wrong_workflow.resources.workflow = Some(WorkflowId::new("workflow-b")?);
    assert!(
        evaluate(&wrong_workflow)?
            .reason_codes()
            .contains(&DecisionReasonCode::WorkflowRunMismatch)
    );
    let mut side_effect = base.clone();
    side_effect.resources.side_effect = SideEffectClass::NonIdempotentWrite;
    assert!(
        evaluate(&side_effect)?
            .reason_codes()
            .contains(&DecisionReasonCode::SideEffectExcess)
    );
    let mut trust = base.clone();
    trust.resources.trust_zone = Some(TrustZone::new("untrusted")?);
    assert!(
        evaluate(&trust)?
            .reason_codes()
            .contains(&DecisionReasonCode::PlacementMismatch)
    );
    let mut budget = base.clone();
    budget.budget.cost_minor = Some(101);
    assert!(
        evaluate(&budget)?
            .reason_codes()
            .contains(&DecisionReasonCode::BudgetExcess)
    );
    let mut secret = base;
    secret.resources.secrets = set(SecretRef::new("secret:other")?);
    assert!(
        evaluate(&secret)?
            .reason_codes()
            .contains(&DecisionReasonCode::SecretScopeMismatch)
    );
    Ok(())
}

#[test]
fn decision_digest_is_deterministic_and_secret_formatting_is_redacted() -> TestResult {
    let request = request()?;
    let first = evaluate(&request)?;
    let second = evaluate(&request)?;
    assert_eq!(first.digest(), second.digest());
    assert_eq!(first.to_canonical_json()?, second.to_canonical_json()?);

    let reference = SecretRef::new("secret:do-not-log-this")?;
    assert!(!format!("{reference:?} {reference}").contains("do-not-log-this"));
    assert_eq!(
        serde_json::to_string(&reference)?,
        "\"secret:do-not-log-this\""
    );
    let secret = SensitiveSecret::new(b"resolved-secret-value".to_vec());
    assert!(!format!("{secret:?} {secret}").contains("resolved-secret-value"));
    assert_eq!(secret.expose(|bytes| bytes.len()), 21);
    Ok(())
}

#[test]
fn artifact_metadata_and_content_require_exact_identity_and_sensitivity_scope() -> TestResult {
    let actor = ActorRef::new("human:artifact-reader")?;
    let artifact = ArtifactId::new("artifact-allowed")?;
    let grant =
        AuthorityGrantBuilder::new(GrantId::new("grant:artifact-reader")?, 1, actor.clone())
            .operations(BTreeSet::from([
                AuthorityOperation::ReadArtifactMetadata,
                AuthorityOperation::ReadArtifactContent,
            ]))
            .resources(ResourceScope {
                workflow_run: WorkflowRunScope::Any,
                capability: CapabilityAuthorityScope::none(SideEffectClass::None),
                filesystem: Vec::new(),
                network: NetworkScope::empty(),
                secrets: BTreeSet::new(),
                artifacts: ArtifactAuthorityScope::new(
                    set(artifact.clone()),
                    set(ArtifactSensitivity::Restricted),
                )?,
                layouts: LayoutAuthorityScope::none(),
                peers: PeerAuthorityScope::none(),
                daemon: DaemonAuthorityScope::default(),
                workspace: WorkspaceAuthorityScope::none(),
            })
            .build()?;
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("policy:artifact")?,
        1,
        [grant.clone()],
        BTreeMap::new(),
    )?;
    let mut resources = RequestedResourceFacts::empty();
    resources.artifact = Some(artifact);
    resources.artifact_sensitivity = Some(ArtifactSensitivity::Restricted);
    let base = AuthorityRequest {
        decision: DecisionId::new("decision:artifact-allowed")?,
        actor,
        grant: grant.identity().clone(),
        grant_revision: grant.revision(),
        grant_digest: grant.digest()?,
        revocation_generation: 0,
        operation: AuthorityOperation::ReadArtifactMetadata,
        resources,
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(1),
        provenance: AuthorityExecutionProvenance::default(),
    };
    assert!(evaluator.evaluate(&base)?.is_allowed());

    let mut content = base.clone();
    content.decision = DecisionId::new("decision:artifact-content")?;
    content.operation = AuthorityOperation::ReadArtifactContent;
    assert!(evaluator.evaluate(&content)?.is_allowed());

    let mut wrong_sensitivity = base.clone();
    wrong_sensitivity.decision = DecisionId::new("decision:artifact-wrong-sensitivity")?;
    wrong_sensitivity.resources.artifact_sensitivity = Some(ArtifactSensitivity::Internal);
    assert!(
        evaluator
            .evaluate(&wrong_sensitivity)?
            .reason_codes()
            .contains(&DecisionReasonCode::ArtifactScopeMismatch)
    );
    let mut wrong_identity = base;
    wrong_identity.decision = DecisionId::new("decision:artifact-wrong-identity")?;
    wrong_identity.resources.artifact = Some(ArtifactId::new("artifact-hidden")?);
    assert!(
        evaluator
            .evaluate(&wrong_identity)?
            .reason_codes()
            .contains(&DecisionReasonCode::ArtifactScopeMismatch)
    );
    Ok(())
}

#[test]
fn human_and_ai_actors_receive_identical_results_for_equivalent_grants() -> TestResult {
    let workflow = WorkflowId::new("workflow-shared")?;
    let actors = [
        ActorRef::new("human:shared-controller")?,
        ActorRef::new("ai:shared-controller")?,
    ];
    let grants = actors
        .iter()
        .enumerate()
        .map(|(index, actor)| {
            AuthorityGrantBuilder::new(
                GrantId::new(format!("grant:shared-{index}"))?,
                1,
                actor.clone(),
            )
            .operations(set(AuthorityOperation::InspectRun))
            .resources(ResourceScope {
                workflow_run: WorkflowRunScope::Workflow {
                    workflow: workflow.clone(),
                },
                capability: CapabilityAuthorityScope::none(SideEffectClass::None),
                filesystem: Vec::new(),
                network: NetworkScope::empty(),
                secrets: BTreeSet::new(),
                artifacts: ArtifactAuthorityScope::none(),
                layouts: LayoutAuthorityScope::none(),
                peers: PeerAuthorityScope::none(),
                daemon: DaemonAuthorityScope::default(),
                workspace: WorkspaceAuthorityScope::none(),
            })
            .build()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("policy:shared-human-ai")?,
        1,
        grants.clone(),
        BTreeMap::new(),
    )?;
    let mut outcomes = Vec::new();
    for (index, grant) in grants.iter().enumerate() {
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = Some(workflow.clone());
        resources.run = Some(RunId::new("run-shared")?);
        let decision = evaluator.evaluate(&AuthorityRequest {
            decision: DecisionId::new(format!("decision:shared-{index}"))?,
            actor: actors[index].clone(),
            grant: grant.identity().clone(),
            grant_revision: grant.revision(),
            grant_digest: grant.digest()?,
            revocation_generation: 0,
            operation: AuthorityOperation::InspectRun,
            resources,
            budget: AuthorityBudget::default(),
            evaluated_at: BoundaryTimeMillis::new(1),
            provenance: AuthorityExecutionProvenance::default(),
        })?;
        outcomes.push((decision.outcome(), decision.reason_codes().to_vec()));
    }
    assert_eq!(outcomes[0], outcomes[1]);
    assert!(matches!(
        outcomes[0].0,
        milkdrift_authority::DecisionOutcome::Allow
    ));
    Ok(())
}
