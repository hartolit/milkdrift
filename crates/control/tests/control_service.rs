//! End-to-end tests for the shared workflow control service.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Barrier, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use milkdrift_authority::{
    ActorRef, AuthorityBudget, CapabilityAuthorityScope, CapabilityAuthorityScopeBuilder,
    CapabilityExecutionRequirements, GrantId, GrantSetEvaluator, PolicyId, WorkflowRunScope,
};
use milkdrift_blueprint::{
    AuthorRef, BlueprintMetadata, BlueprintRevision, Condition, DataPort, Edge, EdgeId, EdgeKind,
    ForkConfig, JoinConfig, JoinPolicy, Mutation, MutationBatch, Node, NodeId, NodeKind,
    PinnedSubworkflow, PortId, ReducerConfig, ReducerStrategy, SchemaRef, TerminalOutcome,
    WorkflowId, WorkflowInterface,
};
use milkdrift_capability::{
    AdmissionBound, AdmissionConstraints, AdmissionMonetaryBound, ArtifactReference, BoundedJson,
    CancellationAcknowledgement, CancellationRequest, CapabilityCategory, CapabilityDescriptor,
    CapabilityDescriptorDocument, CapabilityId, CapabilityObservation, CapabilityRequirement,
    DescriptorBuilder, ErrorClass, IdempotencyKey, InputReference, InvocationAdmissionEnvelope,
    InvocationEvent, InvocationEventKind, InvocationId, InvocationRequest, InvocationTerminal,
    InvocationValueReference, OperationId, ProviderProfileRef, ResolvedCapabilitySnapshot,
    SchemaId, SideEffectClass, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterExecutionContext, AdapterInvocation, AdapterReporter, CapabilityAdapter,
    CapabilityHost, CapabilitySelectionPolicy, HostConfig,
    conformance::{
        AdapterConformanceCase, AdapterConformanceExpectations, ConformanceScenario,
        StartReplayExpectation, UnknownCancellationExpectation, run_adapter_conformance,
    },
};
use milkdrift_control::{
    ActorAuthorityContext, AuthorityPreset, ClaimedStopCondition, ControlArtifactAccess,
    ControlCommand, ControlCommandDocument, ControlError, ControlId, ControlResult, ControlService,
    ControllerBlueprintSpec, ControllerLimits, ControllerPolicyDocument, MAX_CONTROL_RESULT_BYTES,
    OptimisticGuard, ProposalApplicationPolicy, ProposalId, ProposalProvenance, RequestedRunAction,
    RiskClass, WORKFLOW_PROPOSE_OPERATION, WorkflowControlAdapter, WorkflowProposal,
    WorkflowProposalDocument, build_controller_blueprint, workflow_control_descriptor,
};
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactStore, AttemptUsage, BeginArtifactPublication,
    ControllerAccountDeclaration, ControllerAccountState, ControllerAccountStore,
    ControllerAdmissionDenial, ControllerAdmissionOutcome, ControllerAssessmentBoundary,
    ControllerAssessmentOutcome, ControllerReservationId, ControllerResourceBudget, CurrencyCode,
    IntegrityScanRequest, MAX_PAGE_SIZE, MonetaryUsage, PageSize, Reason, ReconciliationDecisionId,
    RepeatDecisionId, RevisionStore, RunEventKind, RunOutcome, RunSequence, StorageAdmin,
    TimestampMillis, WorkerId, WorkspaceStore,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    CommandAuthorityClaim, ControllerAssessmentContext, ControllerLifecycle, DeterministicExecutor,
    EffectExecutionResult, ExecutionDispatch, ExecutionReporter, ExecutorError, ManualClock,
    PreparedExecution, ResolvedCapability, RetryPolicy, RunLifecycle, RuntimeConfig, RuntimeError,
    RuntimeService, SchedulerLimits, SchedulerTickResult, SequentialIdGenerator, TaskExecutor,
};
use milkdrift_workspace::{RunId, ScopeId, WorkspaceBudget, WorkspaceScope};
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type CountingProcessServices = (
    Arc<RuntimeService>,
    Arc<ControlService>,
    ActorAuthorityContext,
    Arc<CountingProcessAdapter>,
);

const NOW: u64 = 20_000;

fn runtime_tick(runtime: &RuntimeService) -> Result<SchedulerTickResult, RuntimeError> {
    let mut scheduled = runtime.scheduler_tick()?;
    let actions = runtime.claim_effects(PageSize::new(MAX_PAGE_SIZE)?)?;
    for action in actions {
        match runtime.execute_effect(action)? {
            EffectExecutionResult::Completed { .. } => {
                scheduled.completed = scheduled.completed.saturating_add(1);
            }
            EffectExecutionResult::Uncertain { .. } => {
                scheduled.uncertain = scheduled.uncertain.saturating_add(1);
            }
            EffectExecutionResult::CancellationAcknowledged
            | EffectExecutionResult::CancellationDeferred => {}
        }
    }
    Ok(scheduled)
}

#[derive(Default)]
struct CountingProcessAdapter(AtomicU64, std::sync::atomic::AtomicBool);

struct TerminalCancellationExecutor {
    resolver: DeterministicExecutor,
    entered: (Mutex<bool>, Condvar),
    cancelled: (Mutex<bool>, Condvar),
    entries: AtomicU64,
    cancellations: AtomicU64,
}

impl TerminalCancellationExecutor {
    fn new(descriptor: CapabilityDescriptor) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            entered: (Mutex::new(false), Condvar::new()),
            cancelled: (Mutex::new(false), Condvar::new()),
            entries: AtomicU64::new(0),
            cancellations: AtomicU64::new(0),
        }
    }

    fn wait_until_entered(&self) -> TestResult {
        let entered = self.entered.0.lock().map_err(|_| "entry lock poisoned")?;
        let (entered, timeout) = self
            .entered
            .1
            .wait_timeout_while(entered, Duration::from_secs(5), |entered| !*entered)
            .map_err(|_| "entry wait poisoned")?;
        if timeout.timed_out() || !*entered {
            return Err("controlled cancellation executor did not enter".into());
        }
        Ok(())
    }

    fn release_after_cancellation_commit(&self) -> TestResult {
        *self
            .cancelled
            .0
            .lock()
            .map_err(|_| "cancellation lock poisoned")? = true;
        self.cancelled.1.notify_all();
        Ok(())
    }
}

impl TaskExecutor for TerminalCancellationExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement, observed_at_unix_ms)
    }

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(PreparedExecution::new(
            dispatch,
            InvocationAdmissionEnvelope::new(
                AdmissionBound::Bounded(4),
                AdmissionBound::NotApplicable,
                AdmissionBound::NotApplicable,
                AdmissionBound::NotApplicable,
            ),
            move |_dispatch, _reporter: &dyn ExecutionReporter| {
                self.entries.fetch_add(1, Ordering::SeqCst);
                *self
                    .entered
                    .0
                    .lock()
                    .map_err(|_| ExecutorError::Boundary("entry lock poisoned".to_owned()))? = true;
                self.entered.1.notify_all();
                let cancelled = self.cancelled.0.lock().map_err(|_| {
                    ExecutorError::Boundary("cancellation lock poisoned".to_owned())
                })?;
                let _cancelled = self
                    .cancelled
                    .1
                    .wait_while(cancelled, |cancelled| !*cancelled)
                    .map_err(|_| {
                        ExecutorError::Boundary("cancellation wait poisoned".to_owned())
                    })?;
                Err(ExecutorError::BoundaryAfterEntry(
                    "terminal cancellation ended the controlled adapter".to_owned(),
                ))
            },
        ))
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.cancellations.fetch_add(1, Ordering::SeqCst);
        Ok(CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            true,
            true,
            Some("controlled adapter reached its terminal cancellation boundary".to_owned()),
        )?)
    }
}

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
        if self.1.load(Ordering::SeqCst) {
            return Err(AdapterError::rejected("injected local preparation refusal"));
        }
        Ok(InvocationAdmissionEnvelope::not_applicable())
    }

    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        CapabilityExecutionRequirements::default()
    }

    fn start(&self) -> Result<(), AdapterError> {
        Ok(())
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

    fn begin_drain(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    fn shutdown(&self) -> Result<(), AdapterError> {
        Ok(())
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

impl ControlArtifactAccess for UnusedResultSink {
    fn read(
        &self,
        _invocation: &AdapterInvocation<'_>,
        _input: &InputReference,
    ) -> Result<(ArtifactReference, Vec<u8>), ControlError> {
        Err(ControlError::InvalidContract(
            "unexpected artifact read".to_owned(),
        ))
    }
    fn publish(
        &self,
        _invocation: &AdapterInvocation<'_>,
        _output_name: &str,
        _bytes: &[u8],
    ) -> Result<ArtifactReference, ControlError> {
        Err(ControlError::InvalidContract(
            "malformed input must not reach result publication".to_owned(),
        ))
    }
}

fn workflow_control_conformance_case(
    _scenario: ConformanceScenario,
) -> TestResult<AdapterConformanceCase> {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let actor = ActorRef::new("ai:adapter-conformance")?;
    let run = RunId::new("run-control-adapter-conformance")?;
    let grant_id = GrantId::new("grant:control-adapter-conformance")?;
    let (_runtime, service, _authority) = services(
        store,
        &actor,
        &run,
        &grant_id,
        "control-adapter-conformance",
    )?;
    let descriptor = workflow_control_descriptor()?;
    let operation = OperationId::new(WORKFLOW_PROPOSE_OPERATION)?;
    let request = InvocationRequest::new(
        InvocationId::new("invocation-control-adapter-conformance")?,
        descriptor.identity().clone(),
        operation,
        descriptor.provider_profile().cloned(),
        Some(IdempotencyKey::new("control-adapter-conformance")?),
        vec![InputReference::new(
            "milkdrift.control_request",
            InvocationValueReference::Inline {
                value: BoundedJson::new(serde_json::json!({
                    "schema_version": 1,
                    "invalid_conformance_probe": true
                }))?,
            },
        )?],
        BTreeMap::new(),
    )?;
    let context = AdapterExecutionContext::new(
        run,
        serde_json::from_value(serde_json::json!(format!("rev_{}", "0".repeat(64))))?,
        NodeId::new("control-adapter-conformance")?,
        milkdrift_persistence::NodeExecutionId::new("execution-control-adapter-conformance")?,
        milkdrift_persistence::AttemptId::new("attempt-control-adapter-conformance")?,
    );
    Ok(AdapterConformanceCase::new(
        Arc::new(WorkflowControlAdapter::new(
            service,
            Arc::new(UnusedResultSink),
        )),
        descriptor,
        request,
        context,
        AdapterConformanceExpectations {
            start_replay: StartReplayExpectation::Idempotent,
            available_while_draining: true,
            available_after_shutdown: true,
            unknown_cancellation: UnknownCancellationExpectation::NegativeAcknowledgement,
        },
    )?
    .with_keepalive(directory))
}

#[test]
fn workflow_control_adapter_passes_shared_conformance() -> TestResult {
    run_adapter_conformance(workflow_control_conformance_case)?;
    Ok(())
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
    services_with_grant_and_revocations(
        store,
        actor,
        grant_id,
        id_prefix,
        grant(actor, run, grant_id)?,
        BTreeMap::new(),
    )
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
    services_with_executor_and_revocations(
        store,
        actor,
        grant_id,
        id_prefix,
        grant,
        revocations,
        Arc::new(DeterministicExecutor::new(descriptor)),
    )
}

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

fn counting_process_services(
    store: Arc<RedbStore>,
    actor: &ActorRef,
    run: &RunId,
    grant_id: &GrantId,
    id_prefix: &str,
) -> TestResult<CountingProcessServices> {
    let descriptor = admission::process_descriptor()?;
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 4,
            max_generations_per_capability: 2,
            max_concurrent_per_generation: 4,
            observation_stale_after_ms: 60_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    let adapter = Arc::new(CountingProcessAdapter::default());
    host.register(
        descriptor.clone(),
        adapter.clone(),
        Some(CapabilityObservation::new(
            descriptor.identity().clone(),
            NOW,
            true,
            0,
            "controller admission longevity fixture ready",
        )?),
    )?;
    let (runtime, service, context) = services_with_executor_and_revocations(
        store,
        actor,
        grant_id,
        id_prefix,
        grant(actor, run, grant_id)?,
        BTreeMap::new(),
        Arc::new(host),
    )?;
    Ok((runtime, service, context, adapter))
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

#[path = "control_service/activation.rs"]
mod activation;
#[path = "control_service/admission.rs"]
mod admission;
#[path = "control_service/revision_and_lifecycle.rs"]
mod revision_and_lifecycle;

use revision_and_lifecycle::create_and_start;

#[path = "control_service/progress.rs"]
mod progress;

use milkdrift_capability_host::conformance::RecordingReporter;
