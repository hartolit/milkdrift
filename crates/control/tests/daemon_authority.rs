//! Daemon naming is independent from permission to edit workflows.
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityEvaluator, AuthorityExecutionProvenance,
    AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, CapabilityAuthorityScope, DecisionId,
    DecisionOutcome, DecisionReasonCode, GrantId, GrantSetEvaluator, PolicyId,
    RequestedResourceFacts, WorkflowRunScope,
};
use milkdrift_blueprint::WorkflowId;
use milkdrift_control::AuthorityPreset;
use std::collections::BTreeMap;

#[test]
fn only_controller_preset_offers_rename_and_configuration_scope_is_still_required()
-> Result<(), Box<dyn std::error::Error>> {
    let actor = ActorRef::new("human:operator")?;
    for preset in [
        AuthorityPreset::Invoker,
        AuthorityPreset::Observer,
        AuthorityPreset::Advisor,
        AuthorityPreset::Supervisor,
        AuthorityPreset::Controller,
        AuthorityPreset::Autonomous,
    ] {
        for configuration in [false, true] {
            let template = preset.template(
                GrantId::new("grant:configuration")?,
                2,
                actor.clone(),
                WorkflowRunScope::Workflow {
                    workflow: WorkflowId::new("workflow:one")?,
                },
                CapabilityAuthorityScope::deny_all(),
                AuthorityBudget::default(),
            );
            let initial = template.clone().build()?;
            let mut resources = initial.resources().clone();
            resources.daemon.configuration = configuration;
            let grant = template.resources(resources).build()?;
            assert_eq!(
                grant
                    .operations()
                    .contains(&AuthorityOperation::RenameDaemon),
                preset == AuthorityPreset::Controller
            );
            let request = AuthorityRequest {
                decision: DecisionId::new("decision:rename")?,
                actor: actor.clone(),
                grant: grant.identity().clone(),
                grant_revision: grant.revision(),
                grant_digest: grant.digest()?,
                revocation_generation: 0,
                operation: AuthorityOperation::RenameDaemon,
                resources: RequestedResourceFacts {
                    daemon_configuration: true,
                    ..RequestedResourceFacts::empty()
                },
                budget: AuthorityBudget::default(),
                evaluated_at: BoundaryTimeMillis::new(1),
                provenance: AuthorityExecutionProvenance::default(),
            };
            let evaluator = GrantSetEvaluator::new(
                PolicyId::new("policy:naming")?,
                1,
                [grant],
                BTreeMap::new(),
            )?;
            let decision = evaluator.evaluate(&request)?;
            assert_eq!(
                decision.outcome() == DecisionOutcome::Allow,
                preset == AuthorityPreset::Controller && configuration
            );
            if !configuration {
                assert!(
                    decision
                        .reason_codes()
                        .contains(&DecisionReasonCode::DaemonScopeMismatch)
                );
            }
        }
    }
    Ok(())
}
