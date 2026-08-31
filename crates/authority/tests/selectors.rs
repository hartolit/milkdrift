//! Exhaustive and generative evidence for explicit authority selectors.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::{
    ActorRef, ArtifactAuthorityScope, AuthorityBudget, AuthorityEvaluator,
    AuthorityExecutionProvenance, AuthorityGrantBuilder, AuthorityOperation, AuthorityRequest,
    BoundaryTimeMillis, CapabilityAuthorityScope, CapabilityAuthorityScopeBuilder,
    DaemonAuthorityScope, DecisionId, DecisionReasonCode, GrantId, GrantSetEvaluator,
    LayoutAuthorityScope, NetworkScope, PeerAuthorityScope, PolicyId, RequestedResourceFacts,
    ResourceScope, Selection, WorkflowRunScope, WorkspaceAuthorityScope,
};
use milkdrift_capability::{
    CapabilityCategory, CapabilityId, CapabilityRequirement, ExecutionTrustClass, OperationId,
    ProviderProfileRef, SideEffectClass, TrustZone,
};
use proptest::prelude::*;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn only_empty_unknown_shape_duplicates_and_oversize_are_rejected() {
    assert!(Selection::<u16>::only(BTreeSet::new()).is_err());
    assert!(serde_json::from_str::<Selection<u16>>(r#"{"type":"only","values":[]}"#).is_err());
    assert!(serde_json::from_str::<Selection<u16>>(r#"{"type":"future"}"#).is_err());
    assert!(serde_json::from_str::<Selection<u16>>(r#"{"type":"any","values":[1]}"#).is_err());
    assert!(serde_json::from_str::<Selection<u16>>(r#"{"type":"only","values":[1,1]}"#).is_err());
    assert!(serde_json::from_str::<Selection<u16>>(r#"{"type":"any","future":true}"#).is_err());

    let oversized = format!(
        r#"{{"type":"only","values":[{}]}}"#,
        (0..=128)
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(serde_json::from_str::<Selection<u16>>(&oversized).is_err());
}

#[test]
fn selector_and_scope_wire_shapes_are_distinct_canonical_and_exact() -> TestResult {
    let any = Selection::<u16>::any();
    let only = Selection::only(BTreeSet::from([1_u16, 2]))?;
    assert_eq!(serde_json::to_string(&any)?, r#"{"type":"any"}"#);
    assert_eq!(
        serde_json::to_string(&only)?,
        r#"{"type":"only","values":[1,2]}"#
    );
    assert!(any.matches(&99));
    assert!(only.matches(&1));
    assert!(!only.matches(&3));

    let deny = CapabilityAuthorityScope::deny_all();
    let allow = CapabilityAuthorityScopeBuilder::new(SideEffectClass::ReadOnly)
        .only_operations(BTreeSet::from([OperationId::new("model.generate")?]))?
        .build();
    assert_eq!(serde_json::to_string(&deny)?, r#"{"type":"deny_all"}"#);
    let allow_json = serde_json::to_value(&allow)?;
    assert_eq!(allow_json["type"], "allow");
    assert_eq!(allow_json["identities"]["type"], "any");
    assert_eq!(allow_json["operations"]["type"], "only");
    assert_ne!(serde_json::to_vec(&deny)?, serde_json::to_vec(&allow)?);
    Ok(())
}

#[test]
fn selector_and_scope_containment_truth_tables_are_complete() -> TestResult {
    let any = Selection::<u16>::any();
    let one = Selection::only(BTreeSet::from([1]))?;
    let two = Selection::only(BTreeSet::from([1, 2]))?;
    assert!(any.is_subset_of(&any));
    assert!(!any.is_subset_of(&one));
    assert!(one.is_subset_of(&any));
    assert!(one.is_subset_of(&two));
    assert!(!two.is_subset_of(&one));

    let requested_any = CapabilityAuthorityScope::allow_any(SideEffectClass::ReadOnly);
    let allowed_only = CapabilityAuthorityScopeBuilder::new(SideEffectClass::ReadOnly)
        .only_capabilities(BTreeSet::from([CapabilityId::new("cap-a")?]))?
        .build();
    let requested_only = CapabilityAuthorityScopeBuilder::new(SideEffectClass::ReadOnly)
        .only_capabilities(BTreeSet::from([CapabilityId::new("cap-a")?]))?
        .build();
    assert!(!requested_any.is_subset_of(&allowed_only));
    assert!(requested_only.is_subset_of(&requested_any));
    assert!(CapabilityAuthorityScope::deny_all().is_subset_of(&allowed_only));
    assert!(!allowed_only.is_subset_of(&CapabilityAuthorityScope::deny_all()));

    let excessive_effect = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
        .only_capabilities(BTreeSet::from([CapabilityId::new("cap-a")?]))?
        .build();
    assert!(!excessive_effect.is_subset_of(&allowed_only));
    Ok(())
}

#[test]
fn requirement_envelopes_make_exact_and_unspecified_dimensions_deliberate() -> TestResult {
    let operation = OperationId::new("model.generate")?;
    let unconstrained = CapabilityAuthorityScope::requirement_envelope(
        &CapabilityRequirement::new(operation.clone())
            .maximum_side_effect(SideEffectClass::ReadOnly),
    )?;
    assert!(
        unconstrained
            .identity_selection()
            .is_some_and(Selection::is_any)
    );
    assert!(
        unconstrained
            .provider_profile_selection()
            .is_some_and(Selection::is_any)
    );
    assert!(
        unconstrained
            .operation_selection()
            .is_some_and(|selection| selection.matches(&operation))
    );

    let identity = CapabilityId::new("model-a")?;
    let profile = ProviderProfileRef::new("profile-a")?;
    let zone = TrustZone::new("trusted")?;
    let exact = CapabilityAuthorityScope::requirement_envelope(
        &CapabilityRequirement::new(operation)
            .exact(identity.clone())
            .category(CapabilityCategory::Model)
            .provider_profile(profile.clone())
            .trust_zone(zone.clone())
            .execution_trust(ExecutionTrustClass::SandboxedProcess)
            .maximum_side_effect(SideEffectClass::ReadOnly),
    )?;
    assert!(
        exact
            .identity_selection()
            .is_some_and(|selection| selection.only_values() == Some(&BTreeSet::from([identity])))
    );
    assert!(
        exact
            .category_selection()
            .is_some_and(|selection| selection.matches(&CapabilityCategory::Model))
    );
    assert!(
        exact
            .provider_profile_selection()
            .is_some_and(|selection| selection.matches(&profile))
    );
    assert!(
        exact
            .trust_zone_selection()
            .is_some_and(|selection| selection.matches(&zone))
    );
    assert!(
        exact
            .execution_trust_class_selection()
            .is_some_and(|selection| selection.matches(&ExecutionTrustClass::SandboxedProcess))
    );
    assert_eq!(exact.maximum_side_effect(), SideEffectClass::ReadOnly);
    Ok(())
}

#[test]
fn deny_all_never_authorizes_capability_facts() -> TestResult {
    let actor = ActorRef::new("human:deny-capability")?;
    let grant =
        AuthorityGrantBuilder::new(GrantId::new("grant:deny-capability")?, 1, actor.clone())
            .operations(BTreeSet::from([AuthorityOperation::InvokeCapability]))
            .resources(ResourceScope {
                workflow_run: WorkflowRunScope::Any,
                capability: CapabilityAuthorityScope::deny_all(),
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
        PolicyId::new("policy:deny-capability")?,
        1,
        [grant.clone()],
        BTreeMap::new(),
    )?;
    let mut resources = RequestedResourceFacts::empty();
    resources.capability = Some(CapabilityId::new("cap-a")?);
    resources.capability_operation = Some(OperationId::new("model.generate")?);
    let decision = evaluator.evaluate(&AuthorityRequest {
        decision: DecisionId::new("decision:deny-capability")?,
        actor,
        grant: grant.identity().clone(),
        grant_revision: grant.revision(),
        grant_digest: grant.digest()?,
        revocation_generation: 0,
        operation: AuthorityOperation::InvokeCapability,
        resources,
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(1),
        provenance: AuthorityExecutionProvenance::default(),
    })?;
    assert!(!decision.is_allowed());
    assert!(
        decision
            .reason_codes()
            .contains(&DecisionReasonCode::CapabilityMismatch)
    );
    Ok(())
}

#[test]
fn any_matches_explicit_capability_facts_but_not_excessive_side_effects() -> TestResult {
    let actor = ActorRef::new("human:any-capability")?;
    let grant = AuthorityGrantBuilder::new(GrantId::new("grant:any-capability")?, 1, actor.clone())
        .operations(BTreeSet::from([AuthorityOperation::InvokeCapability]))
        .resources(ResourceScope {
            workflow_run: WorkflowRunScope::Any,
            capability: CapabilityAuthorityScope::allow_any(SideEffectClass::ReadOnly),
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
        PolicyId::new("policy:any-capability")?,
        1,
        [grant.clone()],
        BTreeMap::new(),
    )?;
    let mut resources = RequestedResourceFacts::empty();
    resources.capability = Some(CapabilityId::new("otherwise-unlisted")?);
    resources.category = Some(CapabilityCategory::Process);
    resources.capability_operation = Some(OperationId::new("process.inspect")?);
    resources.side_effect = SideEffectClass::ReadOnly;
    let request = AuthorityRequest {
        decision: DecisionId::new("decision:any-capability")?,
        actor,
        grant: grant.identity().clone(),
        grant_revision: grant.revision(),
        grant_digest: grant.digest()?,
        revocation_generation: 0,
        operation: AuthorityOperation::InvokeCapability,
        resources,
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(1),
        provenance: AuthorityExecutionProvenance::default(),
    };
    assert!(evaluator.evaluate(&request)?.is_allowed());

    let mut excessive = request;
    excessive.decision = DecisionId::new("decision:any-capability-excessive")?;
    excessive.resources.side_effect = SideEffectClass::IdempotentWrite;
    let decision = evaluator.evaluate(&excessive)?;
    assert!(!decision.is_allowed());
    assert!(
        decision
            .reason_codes()
            .contains(&DecisionReasonCode::SideEffectExcess)
    );
    Ok(())
}

fn generated_selection(any: bool, values: BTreeSet<u8>) -> Option<Selection<u8>> {
    if any {
        Some(Selection::any())
    } else {
        Selection::only(values).ok()
    }
}

proptest! {
    #[test]
    fn containment_is_transitive_and_antisymmetric(
        a_any in any::<bool>(),
        b_any in any::<bool>(),
        c_any in any::<bool>(),
        a_values in prop::collection::btree_set(any::<u8>(), 1..16),
        b_values in prop::collection::btree_set(any::<u8>(), 1..16),
        c_values in prop::collection::btree_set(any::<u8>(), 1..16),
    ) {
        let a = match generated_selection(a_any, a_values) {
            Some(selection) => selection,
            None => return Err(TestCaseError::fail("nonempty selector a was rejected")),
        };
        let b = match generated_selection(b_any, b_values) {
            Some(selection) => selection,
            None => return Err(TestCaseError::fail("nonempty selector b was rejected")),
        };
        let c = match generated_selection(c_any, c_values) {
            Some(selection) => selection,
            None => return Err(TestCaseError::fail("nonempty selector c was rejected")),
        };

        if a.is_subset_of(&b) && b.is_subset_of(&c) {
            prop_assert!(a.is_subset_of(&c));
        }
        if a.is_subset_of(&b) && b.is_subset_of(&a) {
            prop_assert_eq!(a, b);
        }
    }
}
