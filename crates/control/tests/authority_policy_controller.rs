//! Authority preset, policy, and bounded-controller integration tests.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityEvaluator, AuthorityExecutionProvenance,
    AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, CapabilityAuthorityScope,
    CapabilityAuthorityScopeBuilder, DecisionId, GrantDigest, GrantId, GrantSetEvaluator, PolicyId,
    RequestedResourceFacts, WorkflowRunScope,
};
use milkdrift_blueprint::{
    AuthorRef, Condition, Mutation, MutationBatch, Node, NodeId, NodeKind, PinnedSubworkflow,
    TerminalOutcome, WorkflowId, WorkflowInterface,
};
use milkdrift_capability::ProviderProfileRef;
use milkdrift_capability::SideEffectClass;
use milkdrift_control::{
    AuthorityPreset, ControllerBlueprintSpec, ControllerBound, ControllerLimits,
    ControllerProgress, ControllerStop, build_controller_blueprint,
};
use milkdrift_workspace::RunId;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn request(
    actor: &ActorRef,
    grant: &GrantId,
    grant_digest: GrantDigest,
    workflow: &WorkflowId,
    run: &RunId,
    operation: AuthorityOperation,
) -> TestResult<AuthorityRequest> {
    let mut resources = RequestedResourceFacts::empty();
    resources.workflow = Some(workflow.clone());
    resources.run = Some(run.clone());
    Ok(AuthorityRequest {
        decision: DecisionId::new(format!("decision:{}:{}", actor.as_str(), run.as_str()))?,
        actor: actor.clone(),
        grant: grant.clone(),
        grant_revision: 1,
        grant_digest,
        revocation_generation: 0,
        operation,
        resources,
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(10),
        provenance: AuthorityExecutionProvenance::default(),
    })
}

#[test]
fn every_preset_is_an_ordinary_scoped_grant() -> TestResult {
    let actor = ActorRef::new("ai:scoped-controller")?;
    let workflow = WorkflowId::new("preset-scope")?;
    let run_a = RunId::new("run-preset-a")?;
    let run_b = RunId::new("run-preset-b")?;
    let presets = [
        AuthorityPreset::Observer,
        AuthorityPreset::Advisor,
        AuthorityPreset::Supervisor,
        AuthorityPreset::Controller,
        AuthorityPreset::Autonomous,
    ];
    for (index, preset) in presets.into_iter().enumerate() {
        let grant = preset
            .template(
                GrantId::new(format!("grant:preset-{index}"))?,
                1,
                actor.clone(),
                WorkflowRunScope::Run {
                    run: run_a.clone(),
                    workflow: Some(workflow.clone()),
                },
                CapabilityAuthorityScope::allow_any(SideEffectClass::ReadOnly),
                AuthorityBudget::default(),
            )
            .build()?;
        assert_eq!(grant.actor(), &actor);
        assert!(grant.operations().contains(&AuthorityOperation::Inspect));
        if matches!(preset, AuthorityPreset::Observer | AuthorityPreset::Advisor) {
            assert!(
                !grant
                    .operations()
                    .contains(&AuthorityOperation::InvokeCapability)
            );
        }
    }

    let grant_id = GrantId::new("grant:same-preset-different-scope")?;
    let grant_a = AuthorityPreset::Advisor
        .template(
            grant_id.clone(),
            1,
            actor.clone(),
            WorkflowRunScope::Run {
                run: run_a.clone(),
                workflow: Some(workflow.clone()),
            },
            CapabilityAuthorityScope::allow_any(SideEffectClass::ReadOnly),
            AuthorityBudget::default(),
        )
        .build()?;
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("test.preset-scope")?,
        1,
        [grant_a.clone()],
        BTreeMap::new(),
    )?;
    assert!(
        evaluator
            .evaluate(&request(
                &actor,
                &grant_id,
                grant_a.digest()?,
                &workflow,
                &run_a,
                AuthorityOperation::Propose,
            )?)?
            .is_allowed()
    );
    assert!(
        !evaluator
            .evaluate(&request(
                &actor,
                &grant_id,
                grant_a.digest()?,
                &workflow,
                &run_b,
                AuthorityOperation::Propose,
            )?)?
            .is_allowed()
    );
    Ok(())
}

#[test]
fn preset_never_bypasses_provider_or_budget_scope() -> TestResult {
    let actor = ActorRef::new("ai:budgeted-advisor")?;
    let workflow = WorkflowId::new("preset-resource-scope")?;
    let run = RunId::new("run-preset-resource-scope")?;
    let grant_id = GrantId::new("grant:preset-resource-scope")?;
    let grant = AuthorityPreset::Advisor
        .template(
            grant_id.clone(),
            1,
            actor.clone(),
            WorkflowRunScope::Run {
                run: run.clone(),
                workflow: Some(workflow.clone()),
            },
            CapabilityAuthorityScopeBuilder::new(SideEffectClass::ReadOnly)
                .only_provider_profiles(BTreeSet::from([ProviderProfileRef::new(
                    "profile-allowed",
                )?]))?
                .build(),
            AuthorityBudget {
                invocations: Some(2),
                ..AuthorityBudget::default()
            },
        )
        .build()?;
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("test.preset-resource-scope")?,
        1,
        [grant.clone()],
        BTreeMap::new(),
    )?;

    let mut provider_request = request(
        &actor,
        &grant_id,
        grant.digest()?,
        &workflow,
        &run,
        AuthorityOperation::Propose,
    )?;
    provider_request.resources.provider_profile =
        Some(ProviderProfileRef::new("profile-forbidden")?);
    assert!(!evaluator.evaluate(&provider_request)?.is_allowed());

    let mut budget_request = request(
        &actor,
        &grant_id,
        grant.digest()?,
        &workflow,
        &run,
        AuthorityOperation::Propose,
    )?;
    budget_request.resources.provider_profile = Some(ProviderProfileRef::new("profile-allowed")?);
    budget_request.budget.invocations = Some(3);
    assert!(!evaluator.evaluate(&budget_request)?.is_allowed());
    Ok(())
}

fn body_revision() -> TestResult<milkdrift_blueprint::BlueprintRevision> {
    let workflow = WorkflowId::new("controller-body")?;
    let terminal = Node::new(
        NodeId::new("cycle-done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?;
    Ok(milkdrift_blueprint::BlueprintRevision::genesis(
        workflow,
        MutationBatch::new(vec![Mutation::AddNode { node: terminal }])?,
        AuthorRef::new("human:controller-test")?,
        "controller body",
    )?)
}

#[test]
fn controller_pattern_is_explicit_bounded_repeat() -> TestResult {
    let body = body_revision()?;
    let limits = ControllerLimits::new(
        10,
        8,
        16,
        8,
        60_000,
        1_000_000,
        100_000,
        100_000,
        1_000_000,
        10,
        10,
        3,
        3,
        2,
        2,
        Some(5),
    )?;
    let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new("bounded-controller")?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits: limits.clone(),
        author: AuthorRef::new("human:controller-test")?,
    })?;
    assert_eq!(wrapper.semantic().nodes().len(), 2);
    assert_eq!(wrapper.semantic().edges().len(), 1);
    assert!(wrapper.semantic().nodes().values().any(|node| {
        matches!(node.kind(), NodeKind::Repeat { config } if config.maximum_iterations() == 5)
    }));
    assert!(wrapper.semantic().metadata().extensions().contains_key(
        &milkdrift_capability::ExtensionKey::new("org.milkdrift/controller-limits")?
    ));
    assert_eq!(
        limits.assess(&ControllerProgress::default()),
        ControllerStop::Continue
    );
    assert_eq!(
        limits.assess(&ControllerProgress {
            invocations: 5,
            ..ControllerProgress::default()
        }),
        ControllerStop::HumanCheckpoint
    );
    assert_eq!(
        limits.assess(&ControllerProgress {
            rejections: 3,
            ..ControllerProgress::default()
        }),
        ControllerStop::BoundReached {
            bound: ControllerBound::Rejections
        }
    );
    for (progress, bound) in [
        (
            ControllerProgress {
                revisions: 8,
                ..ControllerProgress::default()
            },
            ControllerBound::Revisions,
        ),
        (
            ControllerProgress {
                mutations_in_proposal: 17,
                ..ControllerProgress::default()
            },
            ControllerBound::MutationsPerProposal,
        ),
        (
            ControllerProgress {
                nodes_in_proposal: 9,
                ..ControllerProgress::default()
            },
            ControllerBound::NodesPerProposal,
        ),
        (
            ControllerProgress {
                elapsed_ms: 60_000,
                ..ControllerProgress::default()
            },
            ControllerBound::ElapsedTime,
        ),
        (
            ControllerProgress {
                cost_micros: 1_000_000,
                ..ControllerProgress::default()
            },
            ControllerBound::Cost,
        ),
        (
            ControllerProgress {
                failures: 3,
                ..ControllerProgress::default()
            },
            ControllerBound::Failures,
        ),
    ] {
        assert_eq!(
            limits.assess(&progress),
            ControllerStop::BoundReached { bound }
        );
    }
    assert_eq!(BTreeSet::from([wrapper.id().clone()]).len(), 1);
    Ok(())
}
