//! Independent authority schema, policy, type-separation, and redaction evidence.

use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
};

use milkdrift_authority::{
    AccessMode, ActorRef, AuthorityBudget, AuthorityEvaluator, AuthorityGrant,
    AuthorityGrantBuilder, AuthorityOperation, AuthorityRequest, BoundaryTimeMillis,
    CapabilityAuthorityScope, DecisionId, DecisionReasonCode, FilesystemScope, GrantId,
    GrantSetEvaluator, NetworkProfileRef, NetworkScope, PolicyId, RequestedResourceFacts,
    ResourceScope, SecretRef, SensitiveSecret, WorkflowRunScope,
};
use milkdrift_blueprint::{AuthorRef, WorkflowId};
use milkdrift_capability::{
    CapabilityCategory, CapabilityId, Locality, OperationId, ProviderProfileRef, SideEffectClass,
    TrustZone,
};
use milkdrift_workspace::RunId;

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
    })
    .budget(AuthorityBudget {
        cost_minor: Some(100),
        duration_ms: Some(10_000),
        invocations: Some(5),
        artifact_bytes: Some(1_024),
        concurrency: Some(2),
    })
    .validity(BoundaryTimeMillis::new(100), BoundaryTimeMillis::new(200))
    .revocation_generation(4)
    .build()?)
}

fn request() -> TestResult<AuthorityRequest> {
    Ok(AuthorityRequest {
        decision: DecisionId::new("decision:a")?,
        actor: ActorRef::new("human:alice")?,
        grant: GrantId::new("grant:alice")?,
        grant_revision: 3,
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
            locality: Some(Locality::Remote),
            side_effect: SideEffectClass::ReadOnly,
            filesystem: vec![FilesystemScope::new(
                "/workspace/input",
                set(AccessMode::Read),
            )?],
            network_profiles: set(NetworkProfileRef::new("network-default")?),
            network_destinations: set("example.com:443".to_owned()),
            secrets: set(SecretRef::new("secret:provider-api")?),
        },
        budget: AuthorityBudget {
            cost_minor: Some(10),
            duration_ms: Some(1_000),
            invocations: Some(1),
            artifact_bytes: Some(128),
            concurrency: Some(1),
        },
        evaluated_at: BoundaryTimeMillis::new(150),
    })
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
        String::from_utf8(include_bytes!("fixtures/authority-grant-v1.json").to_vec())?
            .trim()
            .as_bytes()
    );
    assert_eq!(
        AuthorityGrant::from_json(&grant.to_canonical_json()?)?,
        grant
    );
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
