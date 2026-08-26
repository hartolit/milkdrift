//! Golden schema evidence for authorization-bearing command results.

use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityOperation, AuthorityRequest,
    BoundaryTimeMillis, DecisionId, DecisionReasonCode, GrantId, PolicyId, RequestedResourceFacts,
};
use milkdrift_capability::{BoundedJson, SideEffectClass};
use milkdrift_persistence::{
    CommandDisposition, CommandId, CommandResultDocument, IntegrityDigest, RunSequence,
};
use milkdrift_workspace::RunId;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn authorized_result() -> TestResult<CommandResultDocument> {
    let request = AuthorityRequest {
        decision: DecisionId::new("decision:golden")?,
        actor: ActorRef::new("human:golden")?,
        grant: GrantId::new("grant:golden")?,
        grant_revision: 1,
        revocation_generation: 0,
        operation: AuthorityOperation::Inspect,
        resources: RequestedResourceFacts::empty(),
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(1_000),
    };
    let decision = AuthorityDecisionSnapshot::from_evaluation(
        PolicyId::new("policy:golden")?,
        1,
        request,
        vec![DecisionReasonCode::Allowed],
        AuthorityBudget::default(),
        SideEffectClass::None,
    )?;
    Ok(CommandResultDocument::new_authorized(
        CommandId::new("command:golden")?,
        RunId::new("run-golden")?,
        IntegrityDigest::hash(b"golden-command"),
        CommandDisposition::Rejected,
        RunSequence::ZERO,
        Vec::new(),
        BoundedJson::new(serde_json::json!({"status": "rejected"}))?,
        decision,
    )?)
}

#[test]
fn authorization_result_v2_matches_reviewed_golden() -> TestResult {
    let document = authorized_result()?;
    let fixture =
        String::from_utf8(include_bytes!("fixtures/command-result-authorized-v2.json").to_vec())?;
    assert_eq!(document.to_canonical_json()?, fixture.trim().as_bytes());
    assert_eq!(
        CommandResultDocument::from_json(fixture.trim().as_bytes())?,
        document
    );
    assert_eq!(document.schema_version(), 2);
    assert!(document.authorization().is_some());
    Ok(())
}
