//! End-to-end tests for the shared workflow control service.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use milkdrift_authority::{
    ActorRef, AuthorityBudget, CapabilityAuthorityScope, CapabilityAuthorityScopeBuilder, GrantId,
    GrantSetEvaluator, PolicyId, WorkflowRunScope,
};
use milkdrift_blueprint::{
    AuthorRef, BlueprintMetadata, BlueprintRevision, Condition, DataPort, Edge, EdgeId, EdgeKind,
    ForkConfig, JoinConfig, JoinPolicy, Mutation, MutationBatch, Node, NodeId, NodeKind,
    PinnedSubworkflow, PortId, ReducerConfig, ReducerStrategy, SchemaRef, TerminalOutcome,
    WorkflowId, WorkflowInterface,
};
use milkdrift_capability::{
    AdmissionConstraints, ArtifactReference, BoundedJson, CancellationAcknowledgement,
    CancellationRequest, CapabilityCategory, CapabilityDescriptor, CapabilityDescriptorDocument,
    CapabilityId, CapabilityObservation, CapabilityRequirement, DescriptorBuilder, ErrorClass,
    InputReference, InvocationAdmissionEnvelope, InvocationEvent, InvocationEventKind,
    InvocationId, InvocationRequest, InvocationTerminal, InvocationValueReference, OperationId,
    ProviderProfileRef, ResolvedCapabilitySnapshot, SchemaId, SideEffectClass, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, CapabilityHost,
    CapabilitySelectionPolicy, HostConfig,
};
use milkdrift_control::{
    ActorAuthorityContext, AuthorityPreset, ClaimedStopCondition, ControlCommand,
    ControlCommandDocument, ControlError, ControlId, ControlResult, ControlResultSink,
    ControlService, ControllerBlueprintSpec, ControllerLimits, ControllerPolicyDocument,
    OptimisticGuard, ProposalApplicationPolicy, ProposalId, ProposalProvenance, RequestedRunAction,
    RiskClass, WORKFLOW_PROPOSE_OPERATION, WorkflowControlAdapter, WorkflowProposal,
    WorkflowProposalDocument, build_controller_blueprint, workflow_control_descriptor,
};
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactStore, BeginArtifactPublication, ControllerAccountStore,
    ControllerAdmissionDenial, ControllerAdmissionOutcome, ControllerAssessmentBoundary,
    ControllerAssessmentOutcome, IntegrityScanRequest, PageSize, Reason, ReconciliationDecisionId,
    RepeatDecisionId, RevisionStore, RunEventKind, RunOutcome, RunSequence, StorageAdmin,
    TimestampMillis, WorkerId, WorkspaceStore,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    CommandAuthorityClaim, DeterministicExecutor, ManualClock, RetryPolicy, RunLifecycle,
    RuntimeConfig, RuntimeService, SchedulerLimits, SequentialIdGenerator, TaskExecutor,
};
use milkdrift_workspace::{RunId, ScopeId, WorkspaceBudget, WorkspaceScope};
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type DeterministicServices = (
    Arc<RuntimeService>,
    Arc<ControlService>,
    ActorAuthorityContext,
    Arc<DeterministicExecutor>,
);

const NOW: u64 = 20_000;

#[derive(Default)]
struct CountingProcessAdapter(AtomicU64);

impl CountingProcessAdapter {
    fn entries(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl CapabilityAdapter for CountingProcessAdapter {
    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(InvocationAdmissionEnvelope::not_applicable())
    }

    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let terminal = InvocationTerminal::new(
            TerminalStatus::Success,
            Vec::new(),
            None,
            None,
            invocation.resolution().operation_contract().side_effect(),
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                1,
                InvocationEventKind::Terminal { terminal },
            )
            .map_err(|error| AdapterError::external_failure(error.to_string()))?,
        )
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            false,
            false,
            Some("counting process fixture has no active cancellation target".to_owned()),
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            CapabilityId::new("process-controller-test")
                .map_err(|error| AdapterError::external_failure(error.to_string()))?,
            observed_at_unix_ms,
            true,
            u32::try_from(self.entries()).unwrap_or(u32::MAX),
            "controller process fixture ready",
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }
}

fn assert_complete_integrity(store: &RedbStore) -> TestResult {
    let mut cursor = None;
    loop {
        let page = store.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(1_000)?,
            verify_artifact_content: false,
            cursor,
        })?;
        assert!(
            page.failures.is_empty(),
            "integrity failures: {:?}",
            page.failures
        );
        let Some(next) = page.next_cursor else {
            return Ok(());
        };
        cursor = Some(next);
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

fn three_process_body(workflow: &str) -> TestResult<BlueprintRevision> {
    let branch_ports = [PortId::new("a")?, PortId::new("b")?, PortId::new("c")?];
    let mut fork = Node::new(
        NodeId::new("fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(BTreeSet::from(branch_ports.clone()))?,
        },
    )?;
    for port in &branch_ports {
        fork = fork.with_control_output(port.clone())?;
    }
    let mut join = Node::new(
        NodeId::new("join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("fork")?, JoinPolicy::All),
        },
    )?
    .with_control_output(PortId::new("out")?)?;
    for input in ["a-in", "b-in", "c-in"] {
        join = join.with_control_input(PortId::new(input)?)?;
    }
    let terminal = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(PortId::new("in")?)?;
    let mut mutations = vec![
        Mutation::AddNode { node: fork },
        Mutation::AddNode {
            node: task_node("a-task", "model.generate")?,
        },
        Mutation::AddNode {
            node: task_node("b-task", "model.generate")?,
        },
        Mutation::AddNode {
            node: task_node("c-task", "model.generate")?,
        },
        Mutation::AddNode { node: join },
        Mutation::AddNode { node: terminal },
    ];
    for (identity, source, source_port, target, target_port) in [
        ("fork-a", "fork", "a", "a-task", "in"),
        ("fork-b", "fork", "b", "b-task", "in"),
        ("fork-c", "fork", "c", "c-task", "in"),
        ("a-join", "a-task", "out", "join", "a-in"),
        ("b-join", "b-task", "out", "join", "b-in"),
        ("c-join", "c-task", "out", "join", "c-in"),
        ("join-done", "join", "out", "done", "in"),
    ] {
        mutations.push(Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new(identity)?,
                EdgeKind::Control,
                NodeId::new(source)?,
                PortId::new(source_port)?,
                NodeId::new(target)?,
                PortId::new(target_port)?,
            ),
        });
    }
    Ok(BlueprintRevision::genesis(
        WorkflowId::new(workflow)?,
        MutationBatch::new(mutations)?,
        AuthorRef::new("human:controller-process-test")?,
        "three concurrent controller process entries",
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
            CapabilityAuthorityScope::allow_any(SideEffectClass::ReadOnly),
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
    services_with_grant_and_revocations(store, actor, grant_id, id_prefix, grant, BTreeMap::new())
}

fn services_with_grant_and_revocations(
    store: Arc<RedbStore>,
    actor: &ActorRef,
    grant_id: &GrantId,
    id_prefix: &str,
    grant: milkdrift_authority::AuthorityGrant,
    revocations: BTreeMap<GrantId, u64>,
) -> TestResult<(
    Arc<RuntimeService>,
    Arc<ControlService>,
    ActorAuthorityContext,
)> {
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let (runtime, service, context, _executor) = services_with_descriptor_and_revocations(
        store,
        actor,
        grant_id,
        id_prefix,
        grant,
        revocations,
        descriptor,
    )?;
    Ok((runtime, service, context))
}

#[allow(clippy::too_many_arguments)]
fn services_with_descriptor_and_revocations(
    store: Arc<RedbStore>,
    actor: &ActorRef,
    grant_id: &GrantId,
    id_prefix: &str,
    grant: milkdrift_authority::AuthorityGrant,
    revocations: BTreeMap<GrantId, u64>,
    descriptor: CapabilityDescriptor,
) -> TestResult<DeterministicServices> {
    let executor = Arc::new(DeterministicExecutor::new(descriptor));
    let (runtime, service, context) = services_with_executor_and_revocations(
        store,
        actor,
        grant_id,
        id_prefix,
        grant,
        revocations,
        executor.clone(),
    )?;
    Ok((runtime, service, context, executor))
}

#[allow(clippy::too_many_arguments)]
fn services_with_executor_and_revocations(
    store: Arc<RedbStore>,
    actor: &ActorRef,
    grant_id: &GrantId,
    id_prefix: &str,
    grant: milkdrift_authority::AuthorityGrant,
    revocations: BTreeMap<GrantId, u64>,
    executor: Arc<dyn TaskExecutor>,
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
        revocations,
    )?);
    let runtime = Arc::new(RuntimeService::open_closed_with_authority(
        store.clone(),
        executor,
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
    runtime.install_controller_lifecycle(service.controller_lifecycle_owner())?;
    runtime.initialize_startup()?;
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

#[test]
fn installed_runtime_assesses_and_stops_a_controller_at_exact_cycle_bound() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path().join("controller.redb"))?);
    let run = RunId::new("run-controller-lifecycle")?;
    let actor = ActorRef::new("controller:bounded")?;
    let grant_id = GrantId::new("grant:controller-bounded")?;
    let (runtime, service, context) = services(
        store.clone(),
        &actor,
        &run,
        &grant_id,
        "controller-lifecycle",
    )?;

    let body_terminal = Node::new(
        NodeId::new("cycle-complete")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?;
    let body = BlueprintRevision::genesis(
        WorkflowId::new("controller-cycle-body")?,
        MutationBatch::new(vec![Mutation::AddNode {
            node: body_terminal,
        }])?,
        AuthorRef::new("human:controller-test")?,
        "controller cycle body",
    )?;
    store.put_revision(&body)?;
    let limits = ControllerLimits::new(
        2, 2, 8, 4, 60_000, 1_000_000, 10_000, 10_000, 1_000_000, 2, 2, 2, 2, 2, 2, None,
    )?;
    let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new("controller-wrapper")?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits,
        author: AuthorRef::new("human:controller-test")?,
    })?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;

    for _ in 0..64 {
        runtime.tick()?;
        if runtime.projection(&run)?.lifecycle().is_completed() {
            break;
        }
    }
    let projection = runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    let history = runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::RepeatIterationCreated { .. }))
            .count(),
        2
    );
    assert!(history.iter().any(|event| {
        matches!(
            event.kind(),
            RunEventKind::ControllerAssessmentRecorded {
                outcome: ControllerAssessmentOutcome::BoundReached { bound, .. },
                ..
            } if bound == "invocations"
        )
    }));
    let controller_execution = projection
        .controller_assessments()
        .keys()
        .next()
        .cloned()
        .ok_or("bound assessment is absent after terminal compaction")?;
    let status = service.execute(&command(
        "inspect-controller-after-bound",
        &context,
        OptimisticGuard::default(),
        ControlCommand::InspectController {
            run: run.clone(),
            controller_execution,
        },
    )?)?;
    assert!(matches!(
        status,
        ControlResult::ControllerStatus { value }
            if value.state == milkdrift_control::ControllerLifecycleState::BoundReached
                && value.reached_bound == Some(milkdrift_control::ControllerBound::Invocations)
                && value.progress.invocations == 2
                && !value.cycle_eligible
    ));
    Ok(())
}

#[test]
fn controller_progress_preserves_every_durable_counter_and_reassesses_matching_proposals()
-> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(
        directory.path().join("controller-progress.redb"),
    )?);
    let run = RunId::new("run-controller-progress")?;
    let actor = ActorRef::new("controller:progress")?;
    let grant_id = GrantId::new("grant:controller-progress")?;
    let (runtime, service, context) = services(
        store.clone(),
        &actor,
        &run,
        &grant_id,
        "controller-progress",
    )?;
    let body = base_revision("controller-progress-body")?;
    store.put_revision(&body)?;
    let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new("controller-progress-wrapper")?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits: ControllerLimits::new(
            2, 64, 8, 4, 60_000, 1_000_000, 10_000, 10_000, 1_000_000, 64, 64, 64, 64, 2, 2, None,
        )?,
        author: AuthorRef::new("human:controller-progress-test")?,
    })?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;

    let projection = runtime.projection(&run)?;
    let controller_execution = projection
        .node_executions()
        .values()
        .find(|execution| execution.node().as_str() == "controller-repeat")
        .map(|execution| execution.execution().clone())
        .ok_or("active controller execution is absent")?;
    let mut value = serde_json::to_value(&projection)?;
    value["subworkflow_usage_by_execution"] = serde_json::json!([[
        serde_json::to_value(&controller_execution)?,
        {
            "completed_children": 3,
            "failed_children": 2,
            "cost_micros": [["USD", 700]],
            "overflowed": false,
            "input_units": 11,
            "output_units": 13,
            "artifact_bytes": 17,
            "process_invocations": 19,
            "model_invocations": 23,
            "unknown_input_usage": 29,
            "unknown_output_usage": 31,
            "unknown_cost_usage": 37
        }
    ]]);
    value["run_actor_revision_requests"] = serde_json::json!(41);
    value["run_actor_rejections"] = serde_json::json!(43);
    let projection: milkdrift_runtime::RunProjection = serde_json::from_value(value)?;
    let document =
        ControllerPolicyDocument::from_revision(&wrapper, &NodeId::new("controller-repeat")?)?
            .ok_or("controller policy document is absent")?;
    let progress = service.controller_lifecycle_owner().progress(
        &document,
        &projection,
        &controller_execution,
        None,
        NOW + 47,
    )?;
    assert_eq!(progress.invocations, 3);
    assert_eq!(progress.elapsed_ms, 47);
    assert_eq!(progress.cost_micros, 0);
    assert_eq!(progress.input_units, 0);
    assert_eq!(progress.output_units, 0);
    assert_eq!(progress.artifact_bytes, 0);
    assert_eq!(progress.process_invocations, 0);
    assert_eq!(progress.model_invocations, 0);
    assert_eq!(progress.failures, 2);
    assert_eq!(progress.revisions, 41);
    assert_eq!(progress.rejections, 43);
    assert_eq!(progress.unknown_input_observations, 0);
    assert_eq!(progress.unknown_output_observations, 0);
    assert_eq!(progress.unknown_cost_observations, 0);

    let proposed = wrapper.revise(
        wrapper.id(),
        MutationBatch::new(vec![Mutation::SetMetadata {
            metadata: BlueprintMetadata::new(
                "controller-progress-proposal",
                "proposal transition cumulative-bound fixture",
                BTreeSet::from(["controller-progress".to_owned()]),
                BTreeMap::new(),
            )?,
        }])?,
        AuthorRef::new("controller:progress")?,
        format!("proposer={actor};source=controller-progress-test"),
    )?;
    assert!(matches!(
        service.controller_lifecycle_owner().assess_proposal_transition(
            &run,
            &projection,
            &proposed,
            ControllerAssessmentBoundary::ProposalApproval,
            None,
            NOW + 47,
        ),
        Err(ControlError::Bounds { location, .. })
            if location == "controller.proposal.invocations"
    ));
    Ok(())
}

#[path = "control_service/admission.rs"]
mod admission;
#[path = "control_service/revision_and_lifecycle.rs"]
mod revision_and_lifecycle;

use revision_and_lifecycle::create_and_start;
