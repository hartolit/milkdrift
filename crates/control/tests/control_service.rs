//! End-to-end tests for the shared workflow control service.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use milkdrift_authority::{
    ActorRef, AuthorityBudget, CapabilityAuthorityScope, GrantId, GrantSetEvaluator, PolicyId,
    WorkflowRunScope,
};
use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, Edge, EdgeId, EdgeKind, Mutation, MutationBatch, Node, NodeId,
    NodeKind, PortId, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{
    ArtifactReference, BoundedJson, CapabilityDescriptorDocument, CapabilityId,
    CapabilityRequirement, ErrorClass, InputReference, InvocationEvent, InvocationId,
    InvocationRequest, InvocationValueReference, OperationId, ProviderProfileRef,
    ResolvedCapabilitySnapshot, SideEffectClass, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter,
};
use milkdrift_control::{
    ActorAuthorityContext, AuthorityContextRef, AuthorityContextResolver, AuthorityPreset,
    ClaimedStopCondition, ControlCommand, ControlCommandDocument, ControlError, ControlId,
    ControlResult, ControlResultSink, ControlService, OptimisticGuard, ProposalApplicationPolicy,
    ProposalId, ProposalProvenance, RequestedRunAction, RiskClass, WORKFLOW_PROPOSE_OPERATION,
    WorkflowControlAdapter, WorkflowProposal, WorkflowProposalDocument,
    workflow_control_descriptor,
};
use milkdrift_persistence::{
    Reason, ReconciliationDecisionId, RevisionStore, RunSequence, TimestampMillis, WorkerId,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    CommandAuthorityClaim, DeterministicExecutor, ManualClock, RetryPolicy, RunLifecycle,
    RuntimeConfig, RuntimeService, SchedulerLimits, SequentialIdGenerator,
};
use milkdrift_workspace::{RunId, ScopeId, WorkspaceBudget, WorkspaceScope};
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const NOW: u64 = 20_000;

struct StaticAuthorityContext(ActorAuthorityContext);

impl AuthorityContextResolver for StaticAuthorityContext {
    fn resolve(
        &self,
        reference: &AuthorityContextRef,
    ) -> Result<ActorAuthorityContext, ControlError> {
        if reference.as_str() != "trusted-context" {
            return Err(ControlError::InvalidContract(
                "unknown authority context".to_owned(),
            ));
        }
        Ok(self.0.clone())
    }
}

struct UnusedResultSink;

impl ControlResultSink for UnusedResultSink {
    fn publish(
        &self,
        _invocation: &AdapterInvocation<'_>,
        _bytes: &[u8],
    ) -> Result<ArtifactReference, ControlError> {
        Err(ControlError::InvalidContract(
            "malformed input must not reach result publication".to_owned(),
        ))
    }
}

#[derive(Default)]
struct RecordingReporter(Mutex<Vec<InvocationEvent>>);

impl AdapterReporter for RecordingReporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        self.0
            .lock()
            .map_err(|_| AdapterError::external_failure("reporter lock poisoned"))?
            .push(event);
        Ok(())
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn task_node(identity: &str, operation: &str) -> TestResult<Node> {
    Ok(Node::new(
        NodeId::new(identity)?,
        NodeKind::task_direct_inputs(
            CapabilityRequirement::new(OperationId::new(operation)?)
                .maximum_side_effect(SideEffectClass::ReadOnly),
        )?,
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?)
}

fn base_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let work = Node::new(
        NodeId::new("work")?,
        NodeKind::task_direct_inputs(
            CapabilityRequirement::new(OperationId::new("model.generate")?)
                .maximum_side_effect(SideEffectClass::ReadOnly),
        )?,
    )?
    .with_control_output(PortId::new("out")?)?;
    let done = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(PortId::new("in")?)?;
    Ok(BlueprintRevision::genesis(
        WorkflowId::new(workflow)?,
        MutationBatch::new(vec![
            Mutation::AddNode { node: work },
            Mutation::AddNode { node: done },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("work-done")?,
                    EdgeKind::Control,
                    NodeId::new("work")?,
                    PortId::new("out")?,
                    NodeId::new("done")?,
                    PortId::new("in")?,
                ),
            },
        ])?,
        AuthorRef::new("human:control-service-test")?,
        "control service base",
    )?)
}

fn grant(
    actor: &ActorRef,
    run: &RunId,
    grant_id: &GrantId,
) -> TestResult<milkdrift_authority::AuthorityGrant> {
    Ok(AuthorityPreset::Autonomous
        .template(
            grant_id.clone(),
            1,
            actor.clone(),
            WorkflowRunScope::Run {
                run: run.clone(),
                workflow: None,
            },
            CapabilityAuthorityScope::any(SideEffectClass::ReadOnly),
            AuthorityBudget {
                cost_minor: Some(1_000_000),
                duration_ms: Some(3_600_000),
                invocations: Some(1_000),
                artifact_bytes: Some(16_777_216),
                units: Some(1_000_000),
                concurrency: Some(32),
            },
        )
        .build()?)
}

fn services(
    store: Arc<RedbStore>,
    actor: &ActorRef,
    run: &RunId,
    grant_id: &GrantId,
    id_prefix: &str,
) -> TestResult<(
    Arc<RuntimeService>,
    Arc<ControlService>,
    ActorAuthorityContext,
)> {
    services_with_grant(
        store,
        actor,
        grant_id,
        id_prefix,
        grant(actor, run, grant_id)?,
    )
}

fn services_with_grant(
    store: Arc<RedbStore>,
    actor: &ActorRef,
    grant_id: &GrantId,
    id_prefix: &str,
    grant: milkdrift_authority::AuthorityGrant,
) -> TestResult<(
    Arc<RuntimeService>,
    Arc<ControlService>,
    ActorAuthorityContext,
)> {
    let grant_digest = grant.digest()?;
    let authority = Arc::new(GrantSetEvaluator::new(
        PolicyId::new("test.control-service")?,
        1,
        [grant],
        BTreeMap::new(),
    )?);
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let runtime = Arc::new(RuntimeService::new_with_authority(
        store.clone(),
        Arc::new(DeterministicExecutor::new(descriptor)),
        authority.clone(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new(id_prefix, 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-control-service")?,
            ActorRef::new("controller:control-service")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(3, vec![ErrorClass::Transport], 100, 10_000, 25)?,
        )?,
    )?);
    let service = Arc::new(ControlService::new(store, runtime.clone(), authority));
    let context = ActorAuthorityContext::new(
        actor.clone(),
        CommandAuthorityClaim::new(grant_id.clone(), 1, grant_digest, 0)?,
    );
    Ok((runtime, service, context))
}

fn command(
    identity: &str,
    context: &ActorAuthorityContext,
    guard: OptimisticGuard,
    body: ControlCommand,
) -> TestResult<ControlCommandDocument> {
    Ok(ControlCommandDocument::new(
        ControlId::new(identity)?,
        context.clone(),
        TimestampMillis::new(NOW),
        guard,
        Reason::new("control service integration command")?,
        Vec::new(),
        body,
    )?)
}

fn create_and_start(
    service: &ControlService,
    runtime: &RuntimeService,
    context: &ActorAuthorityContext,
    run: &RunId,
    base: &BlueprintRevision,
) -> TestResult {
    service
        .execute(&command(
            "control-create-run",
            context,
            OptimisticGuard {
                expected_run_sequence: Some(RunSequence::ZERO),
                expected_revision: Some(base.id().clone()),
                expected_proposal_digest: None,
            },
            ControlCommand::CreateRun {
                run: run.clone(),
                workflow: base.semantic().workflow().clone(),
                revision: base.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-control-run")?,
                ),
                workspace_budget: WorkspaceBudget::new(
                    128, 65_536, 1_048_576, 64, 1_048_576, 16_777_216,
                )?,
                inputs: Vec::new(),
            },
        )?)
        .map_err(|error| format!("create run through control service: {error}"))?;
    let created = runtime.projection(run)?;
    service
        .execute(&command(
            "control-start-run",
            context,
            OptimisticGuard {
                expected_run_sequence: Some(created.sequence()),
                expected_revision: Some(base.id().clone()),
                expected_proposal_digest: None,
            },
            ControlCommand::StartRun { run: run.clone() },
        )?)
        .map_err(|error| format!("start run through control service: {error}"))?;
    Ok(())
}

fn reviewer_proposal(
    actor: &ActorRef,
    run: &RunId,
    base: &BlueprintRevision,
    sequence: RunSequence,
) -> TestResult<WorkflowProposalDocument> {
    let reviewer = task_node("review", "model.generate")?;
    let context_manifest = ArtifactReference::new(
        "artifact:controller-context",
        "a".repeat(64),
        Some("application/vnd.milkdrift.context-manifest.v1+json".to_owned()),
        Some(512),
    )?;
    let response_artifact = ArtifactReference::new(
        "artifact:controller-response",
        "b".repeat(64),
        Some("application/vnd.milkdrift.model-response.v1+json".to_owned()),
        Some(1_024),
    )?;
    let proposal = WorkflowProposal::new(
        ProposalId::new("proposal-insert-reviewer")?,
        actor.clone(),
        ProposalProvenance::Model {
            capability: CapabilityId::new("model-controller")?,
            invocation: InvocationId::new("invocation-controller-review")?,
            model_profile: ProviderProfileRef::new("profile-controller-reviewed")?,
            context_manifest: context_manifest.clone(),
            response_artifact: response_artifact.clone(),
        },
        base.semantic().workflow().clone(),
        Some(run.clone()),
        base.id().clone(),
        base.content_digest().clone(),
        Some(sequence),
        MutationBatch::new(vec![
            Mutation::RemoveEdge {
                edge: EdgeId::new("work-done")?,
            },
            Mutation::AddNode { node: reviewer },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("work-review")?,
                    EdgeKind::Control,
                    NodeId::new("work")?,
                    PortId::new("out")?,
                    NodeId::new("review")?,
                    PortId::new("in")?,
                ),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("review-done")?,
                    EdgeKind::Control,
                    NodeId::new("review")?,
                    PortId::new("out")?,
                    NodeId::new("done")?,
                    PortId::new("in")?,
                ),
            },
        ])?,
        "insert a read-only reviewer before a not-yet-started successor",
        None,
        vec!["model producer calls this low risk".to_owned()],
        vec!["the successor has not started".to_owned()],
        Vec::new(),
        vec![context_manifest, response_artifact],
        ProposalApplicationPolicy::AutoApplyLowRisk,
        Some(RequestedRunAction::Pause),
        ClaimedStopCondition::Continue,
    )?;
    Ok(WorkflowProposalDocument::new(proposal))
}

#[test]
fn low_risk_live_proposal_applies_pauses_replays_and_survives_restart() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let actor = ActorRef::new("ai:audited-controller")?;
    let run = RunId::new("run-control-low-risk")?;
    let base = base_revision("workflow-control-low-risk")?;
    let grant_id = GrantId::new("grant:control-low-risk")?;
    store.put_revision(&base)?;
    let (runtime, service, context) =
        services(store.clone(), &actor, &run, &grant_id, "control-before")?;
    create_and_start(&service, &runtime, &context, &run, &base)?;
    let before = runtime.projection(&run)?;
    assert_eq!(before.lifecycle(), RunLifecycle::Running);
    let proposal = reviewer_proposal(&actor, &run, &base, before.sequence())?;
    let proposal_guard = OptimisticGuard {
        expected_run_sequence: Some(before.sequence()),
        expected_revision: Some(base.id().clone()),
        expected_proposal_digest: Some(proposal.proposal().digest().clone()),
    };
    let submit = command(
        "control-submit-reviewer",
        &context,
        proposal_guard,
        ControlCommand::SubmitProposal {
            proposal: proposal.clone(),
        },
    )?;
    let first = service.execute(&submit)?;
    let (proposed_revision, risk) = match &first {
        ControlResult::ProposalSubmitted { value } => {
            assert!(value.applied);
            (value.proposed_revision.clone(), value.classification.risk)
        }
        _ => return Err("unexpected proposal result".into()),
    };
    assert_eq!(risk, RiskClass::Low);
    let proposed = store
        .revision(&proposed_revision)?
        .ok_or("proposed revision was not stored")?;
    assert!(proposed.reason().contains(
        "model:model-controller:invocation-controller-review:profile-controller-reviewed"
    ));
    assert!(proposed.reason().contains(&"a".repeat(64)));
    assert!(proposed.reason().contains(&"b".repeat(64)));
    let applied = runtime.projection(&run)?;
    assert_eq!(applied.revision(), Some(&proposed_revision));
    assert_eq!(applied.lifecycle(), RunLifecycle::Paused);
    assert_eq!(service.execute(&submit)?, first);

    let resume = command(
        "control-resume-after-reviewer",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(applied.sequence()),
            expected_revision: Some(proposed_revision.clone()),
            expected_proposal_digest: None,
        },
        ControlCommand::ResumeRun { run: run.clone() },
    )?;
    service.execute(&resume)?;
    let resumed_sequence = runtime.projection(&run)?.sequence();
    drop(service);
    drop(runtime);
    drop(store);

    let reopened_store = Arc::new(RedbStore::open(directory.path())?);
    let (reopened_runtime, reopened_service, reopened_context) =
        services(reopened_store, &actor, &run, &grant_id, "control-after")?;
    let inspection = reopened_service.execute(&command(
        "control-inspect-after-restart",
        &reopened_context,
        OptimisticGuard {
            expected_run_sequence: Some(resumed_sequence),
            expected_revision: Some(proposed_revision.clone()),
            expected_proposal_digest: None,
        },
        ControlCommand::InspectRun { run: run.clone() },
    )?)?;
    match inspection {
        ControlResult::RunInspection { value } => {
            assert_eq!(value.revision, Some(proposed_revision));
            assert_eq!(value.lifecycle, RunLifecycle::Running);
        }
        _ => return Err("unexpected inspection result".into()),
    }
    assert_eq!(
        reopened_runtime.projection(&run)?.sequence(),
        resumed_sequence
    );
    Ok(())
}

#[test]
fn terminal_change_requires_recorded_approval_before_apply() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let actor = ActorRef::new("human:workflow-supervisor")?;
    let run = RunId::new("run-control-approval")?;
    let base = base_revision("workflow-control-approval")?;
    let grant_id = GrantId::new("grant:control-approval")?;
    store.put_revision(&base)?;
    let (runtime, service, context) = services(store, &actor, &run, &grant_id, "control-approval")?;
    create_and_start(&service, &runtime, &context, &run, &base)?;
    let boundary = runtime.projection(&run)?.sequence();
    let replacement = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Failure,
        },
    )?
    .with_control_input(PortId::new("in")?)?;
    let proposal = WorkflowProposalDocument::new(WorkflowProposal::new(
        ProposalId::new("proposal-terminal-change")?,
        actor.clone(),
        ProposalProvenance::Direct,
        base.semantic().workflow().clone(),
        Some(run.clone()),
        base.id().clone(),
        base.content_digest().clone(),
        Some(boundary),
        MutationBatch::new(vec![Mutation::ReplaceNode { node: replacement }])?,
        "change a terminal condition with explicit human approval",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::AutoApplyLowRisk,
        None,
        ClaimedStopCondition::Complete,
    )?);
    let submitted = service.execute(&command(
        "control-submit-terminal-change",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(boundary),
            expected_revision: Some(base.id().clone()),
            expected_proposal_digest: Some(proposal.proposal().digest().clone()),
        },
        ControlCommand::SubmitProposal {
            proposal: proposal.clone(),
        },
    )?)?;
    let proposed_revision = match submitted {
        ControlResult::ProposalSubmitted { value } => {
            assert_eq!(value.classification.risk, RiskClass::ApprovalRequired);
            assert!(!value.applied);
            value.proposed_revision
        }
        _ => return Err("unexpected submit result".into()),
    };
    assert_eq!(runtime.projection(&run)?.revision(), Some(base.id()));

    let decision_boundary = runtime.projection(&run)?.sequence();
    service.execute(&command(
        "control-approve-terminal-change",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(decision_boundary),
            expected_revision: Some(proposed_revision.clone()),
            expected_proposal_digest: Some(proposal.proposal().digest().clone()),
        },
        ControlCommand::ApproveProposal {
            run: run.clone(),
            proposal: proposal.proposal().identity().clone(),
            proposal_digest: proposal.proposal().digest().clone(),
            proposed_revision: proposed_revision.clone(),
            decision: ReconciliationDecisionId::new("decision-terminal-change")?,
        },
    )?)?;
    let apply_boundary = runtime.projection(&run)?.sequence();
    service.execute(&command(
        "control-apply-terminal-change",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(apply_boundary),
            expected_revision: Some(proposed_revision.clone()),
            expected_proposal_digest: Some(proposal.proposal().digest().clone()),
        },
        ControlCommand::ApplyProposal {
            run: run.clone(),
            proposal: proposal.proposal().identity().clone(),
            proposal_digest: proposal.proposal().digest().clone(),
            proposed_revision: proposed_revision.clone(),
        },
    )?)?;
    assert_eq!(
        runtime.projection(&run)?.revision(),
        Some(&proposed_revision)
    );
    Ok(())
}

#[test]
fn stale_or_invalid_proposal_changes_no_run_state() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let actor = ActorRef::new("ai:stale-proposer")?;
    let run = RunId::new("run-control-stale")?;
    let base = base_revision("workflow-control-stale")?;
    let grant_id = GrantId::new("grant:control-stale")?;
    store.put_revision(&base)?;
    let (runtime, service, context) = services(store, &actor, &run, &grant_id, "control-stale")?;
    create_and_start(&service, &runtime, &context, &run, &base)?;
    let observed = runtime.projection(&run)?.sequence();
    let proposal = reviewer_proposal(&actor, &run, &base, observed)?;
    service.execute(&command(
        "control-pause-before-stale-submit",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(observed),
            expected_revision: Some(base.id().clone()),
            expected_proposal_digest: None,
        },
        ControlCommand::PauseRun { run: run.clone() },
    )?)?;
    let after_pause = runtime.projection(&run)?.sequence();
    let stale = service.execute(&command(
        "control-stale-submit",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(observed),
            expected_revision: Some(base.id().clone()),
            expected_proposal_digest: Some(proposal.proposal().digest().clone()),
        },
        ControlCommand::SubmitProposal { proposal },
    )?);
    assert!(matches!(stale, Err(ControlError::StaleRunSequence { .. })));
    let current = runtime.projection(&run)?;
    assert_eq!(current.sequence(), after_pause);
    assert_eq!(current.revision(), Some(base.id()));

    let invalid = WorkflowProposalDocument::new(WorkflowProposal::new(
        ProposalId::new("proposal-invalid-mutation")?,
        actor,
        ProposalProvenance::Direct,
        base.semantic().workflow().clone(),
        Some(run.clone()),
        base.id().clone(),
        base.content_digest().clone(),
        Some(after_pause),
        MutationBatch::new(vec![Mutation::RemoveNode {
            node: NodeId::new("absent-node")?,
        }])?,
        "invalid closed mutation",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::ProposeOnly,
        None,
        ClaimedStopCondition::Continue,
    )?);
    let rejected = service.execute(&command(
        "control-invalid-submit",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(after_pause),
            expected_revision: Some(base.id().clone()),
            expected_proposal_digest: Some(invalid.proposal().digest().clone()),
        },
        ControlCommand::SubmitProposal { proposal: invalid },
    )?);
    assert!(matches!(rejected, Err(ControlError::Blueprint(_))));
    assert_eq!(runtime.projection(&run)?.sequence(), after_pause);
    Ok(())
}

#[test]
fn unauthorized_provider_expansion_stores_no_revision() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let actor = ActorRef::new("ai:provider-confined")?;
    let workflow = WorkflowId::new("workflow-control-provider-confined")?;
    let base = base_revision(workflow.as_str())?;
    let grant_id = GrantId::new("grant:control-provider-confined")?;
    store.put_revision(&base)?;
    let allowed_profile = ProviderProfileRef::new("profile-allowed")?;
    let grant = AuthorityPreset::Controller
        .template(
            grant_id.clone(),
            1,
            actor.clone(),
            WorkflowRunScope::Workflow {
                workflow: workflow.clone(),
            },
            CapabilityAuthorityScope::new(
                BTreeSet::new(),
                BTreeSet::new(),
                BTreeSet::new(),
                BTreeSet::from([allowed_profile]),
                BTreeSet::new(),
                BTreeSet::new(),
                SideEffectClass::ReadOnly,
            )?,
            AuthorityBudget {
                invocations: Some(16),
                ..AuthorityBudget::default()
            },
        )
        .build()?;
    let (_runtime, service, context) = services_with_grant(
        store.clone(),
        &actor,
        &grant_id,
        "control-provider-confined",
        grant,
    )?;
    let replacement = Node::new(
        NodeId::new("work")?,
        NodeKind::task_direct_inputs(
            CapabilityRequirement::new(OperationId::new("model.generate")?)
                .provider_profile(ProviderProfileRef::new("profile-forbidden")?)
                .maximum_side_effect(SideEffectClass::ReadOnly),
        )?,
    )?
    .with_control_output(PortId::new("out")?)?;
    let mutation = MutationBatch::new(vec![Mutation::ReplaceNode { node: replacement }])?;
    let proposal = WorkflowProposal::new(
        ProposalId::new("proposal-provider-expansion")?,
        actor,
        ProposalProvenance::Direct,
        workflow,
        None,
        base.id().clone(),
        base.content_digest().clone(),
        None,
        mutation.clone(),
        "attempt to select an unauthorized provider profile",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::ProposeOnly,
        None,
        ClaimedStopCondition::Continue,
    )?;
    let expected = base.revise(
        base.id(),
        mutation,
        AuthorRef::new(format!("proposal:{}", &proposal.digest().as_str()[3..35]))?,
        format!(
            "proposal_id={};proposal_digest={};proposer={};source=direct",
            proposal.identity(),
            proposal.digest(),
            proposal.proposer()
        ),
    )?;
    let document = WorkflowProposalDocument::new(proposal);
    let result = service.execute(&command(
        "control-provider-expansion",
        &context,
        OptimisticGuard {
            expected_run_sequence: None,
            expected_revision: Some(base.id().clone()),
            expected_proposal_digest: Some(document.proposal().digest().clone()),
        },
        ControlCommand::SubmitProposal { proposal: document },
    )?);
    assert!(matches!(
        result,
        Err(ControlError::AuthorizationDenied { .. })
    ));
    assert!(store.revision(expected.id())?.is_none());
    Ok(())
}

#[test]
fn malformed_control_capability_input_is_a_normal_rejected_terminal() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let actor = ActorRef::new("ai:hostile-output")?;
    let run = RunId::new("run-control-hostile-output")?;
    let grant_id = GrantId::new("grant:control-hostile-output")?;
    let (_runtime, service, context) =
        services(store, &actor, &run, &grant_id, "control-hostile-output")?;
    let descriptor = workflow_control_descriptor()?;
    let operation = OperationId::new(WORKFLOW_PROPOSE_OPERATION)?;
    let resolution = ResolvedCapabilitySnapshot::from_descriptor(&descriptor, &operation)?;
    let request = InvocationRequest::new(
        InvocationId::new("invocation-hostile-control-output")?,
        descriptor.identity().clone(),
        operation,
        descriptor.provider_profile().cloned(),
        None,
        vec![
            InputReference::new(
                "milkdrift.control_request",
                InvocationValueReference::Inline {
                    value: BoundedJson::new(serde_json::json!({
                        "schema_version": 1,
                        "hostile_untrusted_output": ["not", "a", "command"]
                    }))?,
                },
            )?,
            InputReference::new(
                "milkdrift.authority_context",
                InvocationValueReference::Inline {
                    value: BoundedJson::new(serde_json::json!("trusted-context"))?,
                },
            )?,
        ],
        BTreeMap::new(),
    )?;
    let adapter = WorkflowControlAdapter::new(
        service,
        Arc::new(StaticAuthorityContext(context)),
        Arc::new(UnusedResultSink),
    );
    let reporter = RecordingReporter::default();
    adapter.execute(&AdapterInvocation::new(&resolution, &request), &reporter)?;

    let events = reporter.0.lock().map_err(|_| "reporter lock poisoned")?;
    assert_eq!(events.len(), 1);
    let terminal = events[0]
        .kind()
        .terminal()
        .ok_or("malformed control output did not produce a terminal event")?;
    assert_eq!(terminal.status(), TerminalStatus::Rejected);
    assert_eq!(terminal.side_effect(), SideEffectClass::None);
    assert!(terminal.failure().is_some());
    Ok(())
}
