//! Exact workflow sets retain ordinary action, digest, and frozen execution checks.
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityEvaluator, AuthorityExecutionProvenance, AuthorityGrant,
    AuthorityGrantBuilder, AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, DecisionId,
    ExecutionAuthorityBasis, GrantId, GrantSetEvaluator, MAX_SELECTION_ITEMS, PolicyId,
    RequestedResourceFacts, WorkflowRunScope, WorkflowSet,
};
use milkdrift_blueprint::WorkflowId;
use milkdrift_workspace::RunId;
use std::collections::{BTreeMap, BTreeSet};
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn scope() -> Result<WorkflowRunScope, Box<dyn std::error::Error>> {
    Ok(WorkflowRunScope::Workflows {
        workflows: WorkflowSet::new([WorkflowId::new("b")?, WorkflowId::new("a")?])?,
    })
}

#[test]
fn named_scope_has_one_bounded_canonical_form_and_refuses_ambiguous_input() -> TestResult {
    assert_eq!(
        serde_json::to_string(&scope()?)?,
        r#"{"type":"workflows","workflows":["a","b"]}"#
    );
    assert_eq!(
        serde_json::from_str::<WorkflowRunScope>(r#"{"type":"workflows","workflows":["b","a"]}"#)?,
        scope()?
    );
    for invalid in [
        r#"{"type":"workflows","workflows":[]}"#,
        r#"{"type":"workflows","workflows":["a","a"]}"#,
        r#"{"type":"workflows","workflows":[""]}"#,
        r#"{"type":"workflows","workflows":["a"],"workflow":"b"}"#,
        r#"{"type":"any","workflows":["a"]}"#,
        r#"{"type":"workflows","workflows":{"type":"any"}}"#,
        r#"{"type":"workflows"}"#,
    ] {
        assert!(
            serde_json::from_str::<WorkflowRunScope>(invalid).is_err(),
            "{invalid}"
        );
    }
    assert!(WorkflowSet::new([]).is_err());
    assert!(WorkflowSet::new([WorkflowId::new("a")?, WorkflowId::new("a")?]).is_err());
    let ids = (0..=MAX_SELECTION_ITEMS)
        .map(|n| WorkflowId::new(format!("workflow-{n}")))
        .collect::<Result<Vec<_>, _>>()?;
    assert!(WorkflowSet::new(ids.clone()).is_err());
    assert!(serde_json::from_value::<WorkflowSet>(serde_json::to_value(&ids)?).is_err());
    let accepted = WorkflowSet::new(ids.into_iter().take(MAX_SELECTION_ITEMS))?;
    assert_eq!(accepted.values().len(), MAX_SELECTION_ITEMS);
    // Previously supported shapes keep exactly their previous canonical bytes.
    for old in [
        r#"{"type":"any"}"#,
        r#"{"type":"workflow","workflow":"a"}"#,
        r#"{"type":"run","run":"run-a","workflow":"a"}"#,
    ] {
        assert_eq!(
            serde_json::to_string(&serde_json::from_str::<WorkflowRunScope>(old)?)?,
            old
        );
    }
    Ok(())
}

#[test]
fn actions_workflow_facts_digests_and_frozen_basis_cannot_expand_the_set() -> TestResult {
    let actor = ActorRef::new("human:named")?;
    let identity = GrantId::new("grant:named")?;
    let mut resources = AuthorityGrantBuilder::new(identity.clone(), 1, actor.clone())
        .operations(BTreeSet::from([AuthorityOperation::InspectRevision]))
        .build()?
        .resources()
        .clone();
    resources.workflow_run = scope()?;
    let grant = AuthorityGrantBuilder::new(identity, 1, actor.clone())
        .operations(BTreeSet::from([
            AuthorityOperation::InspectRevision,
            AuthorityOperation::StartRun,
        ]))
        .resources(resources)
        .build()?;
    let reopened = AuthorityGrant::from_json(&grant.to_canonical_json()?)?;
    assert_eq!(reopened, grant);
    assert_eq!(reopened.digest()?, grant.digest()?);
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("policy:named")?,
        1,
        [reopened],
        BTreeMap::new(),
    )?;
    let mut request = AuthorityRequest {
        decision: DecisionId::new("decision:named")?,
        actor,
        grant: grant.identity().clone(),
        grant_revision: 1,
        grant_digest: grant.digest()?,
        revocation_generation: 0,
        operation: AuthorityOperation::InspectRevision,
        resources: RequestedResourceFacts::empty(),
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(1),
        provenance: AuthorityExecutionProvenance::default(),
    };
    for (workflow, allowed) in [
        (None, false),
        (Some("a"), true),
        (Some("b"), true),
        (Some("c"), false),
    ] {
        request.resources.workflow = workflow.map(WorkflowId::new).transpose()?;
        assert_eq!(evaluator.evaluate(&request)?.is_allowed(), allowed);
    }
    request.resources.workflow = Some(WorkflowId::new("a")?);
    request.operation = AuthorityOperation::ImportBlueprint;
    assert!(
        !evaluator.evaluate(&request)?.is_allowed(),
        "selection must not add an action"
    );
    request.operation = AuthorityOperation::StartRun;
    request.resources.run = Some(RunId::new("run-a")?);
    let accepted = evaluator.evaluate(&request)?;
    let revision = serde_json::from_str(
        r#""rev_0000000000000000000000000000000000000000000000000000000000000000""#,
    )?;
    let basis = ExecutionAuthorityBasis::from_start_decision(
        &accepted,
        WorkflowId::new("a")?,
        RunId::new("run-a")?,
        revision,
    )?;
    let basis: ExecutionAuthorityBasis = serde_json::from_slice(&serde_json::to_vec(&basis)?)?;
    request.resources.workflow = Some(WorkflowId::new("c")?);
    request.resources.run = Some(RunId::new("run-c")?);
    let child = basis.request(
        DecisionId::new("decision:child")?,
        AuthorityOperation::InspectRevision,
        request.resources.clone(),
        AuthorityBudget::default(),
        BoundaryTimeMillis::new(2),
        AuthorityExecutionProvenance::default(),
    );
    assert_eq!(child.resources.workflow, Some(WorkflowId::new("a")?));
    assert_eq!(child.resources.run, Some(RunId::new("run-a")?));
    assert!(evaluator.evaluate(&child)?.is_allowed());
    let mut changed: serde_json::Value = serde_json::from_slice(&grant.to_canonical_json()?)?;
    *changed
        .pointer_mut("/resources/workflow_run")
        .ok_or("scope absent")? = serde_json::json!({"type":"workflows","workflows":["a","b","c"]});
    let expanded = AuthorityGrant::from_json(&serde_json::to_vec(&changed)?)?;
    assert_ne!(expanded.digest()?, grant.digest()?);
    request.grant_digest = expanded.digest()?;
    assert!(!evaluator.evaluate(&request)?.is_allowed());
    assert!(
        GrantSetEvaluator::new(
            PolicyId::new("policy:conflicting")?,
            1,
            [grant.clone(), expanded],
            BTreeMap::new()
        )
        .is_err()
    );
    let revoked = GrantSetEvaluator::new(
        PolicyId::new("policy:revoked")?,
        1,
        [grant.clone()],
        BTreeMap::from([(grant.identity().clone(), 1)]),
    )?;
    assert!(!revoked.evaluate(&child)?.is_allowed());
    // Destination import permission alone must not supply source inspection.
    let mut write_only: serde_json::Value = serde_json::from_slice(&grant.to_canonical_json()?)?;
    *write_only
        .get_mut("operations")
        .ok_or("operations absent")? = serde_json::json!(["import_blueprint"]);
    let write_only = AuthorityGrant::from_json(&serde_json::to_vec(&write_only)?)?;
    let writer = GrantSetEvaluator::new(
        PolicyId::new("policy:writer")?,
        1,
        [write_only.clone()],
        BTreeMap::new(),
    )?;
    let mut read = child;
    read.grant_digest = write_only.digest()?;
    assert!(!writer.evaluate(&read)?.is_allowed());
    Ok(())
}
