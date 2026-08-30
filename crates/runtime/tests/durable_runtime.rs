//! Process-style headless runtime evidence against the production local store.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    AuthorityGrantBuilder, AuthorityOperation, BoundaryTimeMillis, CapabilityAuthorityScope,
    DecisionReasonCode, ExecutionAuthorityBasis, GrantDigest, GrantId, GrantSetEvaluator,
    NetworkScope, PolicyId, ResourceScope, WorkflowRunScope,
};

use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, Edge, EdgeId, EdgeKind, Mutation, MutationBatch, Node, NodeId,
    NodeKind, PortId, TerminalOutcome, WorkflowId, WorkflowInterface,
};
use milkdrift_capability::{
    BoundedJson, CancellationAcknowledgement, CancellationRequest, CapabilityDescriptorDocument,
    CapabilityId, CapabilityRequirement, ErrorClass, Locality, OperationId, SideEffectClass,
};
use milkdrift_persistence::{
    ActorRef, AttemptId, AuthorityDecision, CommandId, NodeExecutionId, NodeOutcome, PageSize,
    Reason, ReconciliationDecisionId, ReconciliationId, ReconciliationPlanId, ReconciliationPolicy,
    RepeatContinuationDecision, RepeatDecisionId, RevisionStore, RunJournal, RunSequence,
    SignalDeliveryMode, SignalId, SignalTypeId, TimerId, TimestampMillis, WorkerId,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    CommandAuthorityClaim, DeterministicExecutor, ExecutionDispatch, ExecutionReportBatch,
    ExecutorError, ExternalWorkAction, ManualClock, ResolvedCapability, RetryPolicy, RunCommand,
    RunCommandDocument, RunLifecycle, RuntimeConfig, RuntimeService, SchedulerLimits,
    SequentialIdGenerator, SystemTransition, TaskExecutor,
};
use milkdrift_workspace::{RunId, ScopeId, WorkspaceBudget, WorkspaceScope};
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct TestAuthority;

impl AuthorityEvaluator for TestAuthority {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.durable-runtime")?,
            1,
            request.clone(),
            vec![DecisionReasonCode::Allowed],
            AuthorityBudget {
                artifact_bytes: Some(u64::MAX),
                ..AuthorityBudget::default()
            },
            milkdrift_capability::SideEffectClass::Unknown,
        )
    }
}

fn claim() -> TestResult<CommandAuthorityClaim> {
    Ok(CommandAuthorityClaim::new(
        GrantId::new("grant:durable-runtime")?,
        1,
        GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
        0,
    )?)
}

fn sequence_revision() -> TestResult<BlueprintRevision> {
    sequence_revision_with_requirement(CapabilityRequirement::new(OperationId::new(
        "model.generate",
    )?))
}

fn sequence_revision_with_requirement(
    requirement: CapabilityRequirement,
) -> TestResult<BlueprintRevision> {
    let workflow = WorkflowId::new("runtime-reopen")?;
    let task_id = NodeId::new("generate")?;
    let terminal_id = NodeId::new("done")?;
    let next = PortId::new("next")?;
    let input = PortId::new("in")?;
    let task = Node::new(task_id.clone(), NodeKind::task_direct_inputs(requirement)?)?
        .with_control_output(next.clone())?;
    let terminal = Node::new(
        terminal_id.clone(),
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(input.clone())?;
    let batch = MutationBatch::new(vec![
        Mutation::SetInterface {
            interface: WorkflowInterface::new([], [])?,
        },
        Mutation::AddNode { node: task },
        Mutation::AddNode { node: terminal },
        Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new("generate-done")?,
                EdgeKind::Control,
                task_id,
                next,
                terminal_id,
                input,
            ),
        },
    ])?;
    Ok(BlueprintRevision::genesis(
        workflow,
        batch,
        AuthorRef::new("human:runtime-test")?,
        "durable runtime restart fixture",
    )?)
}

fn two_task_revision() -> TestResult<BlueprintRevision> {
    let workflow = WorkflowId::new("runtime-reopen")?;
    let first_id = NodeId::new("first")?;
    let second_id = NodeId::new("second")?;
    let terminal_id = NodeId::new("done")?;
    let first_out = PortId::new("first-out")?;
    let second_in = PortId::new("second-in")?;
    let second_out = PortId::new("second-out")?;
    let terminal_in = PortId::new("terminal-in")?;
    let requirement = CapabilityRequirement::new(OperationId::new("model.generate")?);
    let first = Node::new(
        first_id.clone(),
        NodeKind::task_direct_inputs(requirement.clone())?,
    )?
    .with_control_output(first_out.clone())?;
    let second = Node::new(
        second_id.clone(),
        NodeKind::task_direct_inputs(requirement)?,
    )?
    .with_control_input(second_in.clone())?
    .with_control_output(second_out.clone())?;
    let terminal = Node::new(
        terminal_id.clone(),
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(terminal_in.clone())?;
    Ok(BlueprintRevision::genesis(
        workflow,
        MutationBatch::new(vec![
            Mutation::SetInterface {
                interface: WorkflowInterface::new([], [])?,
            },
            Mutation::AddNode { node: first },
            Mutation::AddNode { node: second },
            Mutation::AddNode { node: terminal },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("first-second")?,
                    EdgeKind::Control,
                    first_id,
                    first_out,
                    second_id.clone(),
                    second_in,
                ),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("second-done")?,
                    EdgeKind::Control,
                    second_id,
                    second_out,
                    terminal_id,
                    terminal_in,
                ),
            },
        ])?,
        AuthorRef::new("human:runtime-test")?,
        "two task revocation fixture",
    )?)
}

fn service(
    store: Arc<RedbStore>,
    clock: Arc<ManualClock>,
    id_prefix: &str,
) -> TestResult<RuntimeService> {
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let executor = Arc::new(DeterministicExecutor::new(descriptor));
    let config = RuntimeConfig::new(
        WorkerId::new("worker-runtime-test")?,
        ActorRef::new("controller:runtime-test")?,
        30_000,
        32,
        SchedulerLimits::new(8, 4, 2, 4)?,
        RetryPolicy::new(3, vec![ErrorClass::Transport], 100, 10_000, 25)?,
    )?;
    Ok(RuntimeService::new_with_authority(
        store,
        executor,
        Arc::new(TestAuthority),
        clock,
        Arc::new(SequentialIdGenerator::new(id_prefix, 1)?),
        config,
    )?)
}

#[allow(clippy::too_many_arguments)] // This reviewed boundary keeps its complete invariant-bearing fact set explicit.
fn exact_grant_service(
    store: Arc<RedbStore>,
    clock: Arc<ManualClock>,
    descriptor: milkdrift_capability::CapabilityDescriptor,
    actor: &ActorRef,
    workflow: &WorkflowId,
    operations: BTreeSet<AuthorityOperation>,
    capability_scope: CapabilityAuthorityScope,
    id_prefix: &str,
) -> TestResult<(RuntimeService, CommandAuthorityClaim)> {
    let grant_id = GrantId::new(format!("grant:{id_prefix}"))?;
    let budget = AuthorityBudget {
        cost_minor: Some(u64::MAX),
        duration_ms: Some(u64::MAX),
        invocations: Some(u64::MAX),
        artifact_bytes: Some(u64::MAX),
        units: Some(u64::MAX),
        concurrency: Some(u32::MAX),
    };
    let grant = AuthorityGrantBuilder::new(grant_id.clone(), 1, actor.clone())
        .operations(operations)
        .resources(ResourceScope {
            workflow_run: WorkflowRunScope::Workflow {
                workflow: workflow.clone(),
            },
            capability: capability_scope,
            filesystem: Vec::new(),
            network: NetworkScope::empty(),
            secrets: BTreeSet::new(),
            artifacts: milkdrift_authority::ArtifactAuthorityScope::none(),
            layouts: milkdrift_authority::LayoutAuthorityScope::none(),
            peers: milkdrift_authority::PeerAuthorityScope::none(),
            daemon: milkdrift_authority::DaemonAuthorityScope::default(),
            workspace: milkdrift_authority::WorkspaceAuthorityScope::none(),
        })
        .budget(budget)
        .validity(BoundaryTimeMillis::new(0), BoundaryTimeMillis::new(10_000))
        .build()?;
    let claim = CommandAuthorityClaim::new(grant_id, 1, grant.digest()?, 0)?;
    let evaluator = Arc::new(GrantSetEvaluator::new(
        PolicyId::new(format!("test.{id_prefix}"))?,
        1,
        [grant],
        BTreeMap::new(),
    )?);
    let runtime = RuntimeService::new_with_authority(
        store,
        Arc::new(DeterministicExecutor::new(descriptor)),
        evaluator,
        clock,
        Arc::new(SequentialIdGenerator::new(id_prefix, 1)?),
        RuntimeConfig::new(
            WorkerId::new(format!("worker-{id_prefix}"))?,
            ActorRef::new(format!("controller:{id_prefix}"))?,
            30_000,
            16,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 100, 0)?,
        )?,
    )?;
    Ok((runtime, claim))
}

fn create_document(
    revision: &BlueprintRevision,
    run: &RunId,
    actor: &ActorRef,
    identity: &str,
) -> TestResult<RunCommandDocument> {
    Ok(RunCommandDocument::new(
        CommandId::new(identity)?,
        run.clone(),
        actor.clone(),
        RunSequence::ZERO,
        TimestampMillis::new(1_000),
        Reason::new("create exact-authority test run")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(
                run.clone(),
                ScopeId::new(format!("scope-{identity}"))?,
            ),
            workspace_budget: WorkspaceBudget::new(32, 16_384, 131_072, 8, 1_048_576, 4_194_304)?,
            inputs: Vec::new(),
        },
    )?)
}

#[test]
fn authority_requests_cover_every_external_command_family() -> TestResult {
    let revision = sequence_revision()?;
    let run = RunId::new("run-authority-command-families")?;
    let cases = vec![
        (
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-authority-command-families")?,
                ),
                workspace_budget: WorkspaceBudget::new(16, 8_192, 65_536, 4, 65_536, 262_144)?,
                inputs: Vec::new(),
            },
            AuthorityOperation::CreateRun,
        ),
        (RunCommand::StartRun, AuthorityOperation::StartRun),
        (RunCommand::PauseRun, AuthorityOperation::Pause),
        (RunCommand::ResumeRun, AuthorityOperation::Resume),
        (RunCommand::RequestCancellation, AuthorityOperation::Cancel),
        (
            RunCommand::DeliverSignal {
                signal: SignalId::new("signal-authority-family")?,
                signal_type: SignalTypeId::new("signal.type.authority-family")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(serde_json::json!({"allowed": true}))?,
            },
            AuthorityOperation::DeliverSignal,
        ),
        (
            RunCommand::FireTimer {
                timer: TimerId::new("timer-authority-family")?,
            },
            AuthorityOperation::FireTimer,
        ),
        (
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-authority-family")?,
                revision: revision.id().clone(),
                policy: ReconciliationPolicy::RequireAuthority,
            },
            AuthorityOperation::Propose,
        ),
        (
            RunCommand::DecideReconciliation {
                plan: ReconciliationPlanId::new("plan-authority-family")?,
                decision: ReconciliationDecisionId::new("decision-authority-family")?,
                outcome: AuthorityDecision::Approve,
            },
            AuthorityOperation::Approve,
        ),
        (
            RunCommand::ApplyReconciliation {
                plan: ReconciliationPlanId::new("plan-authority-family")?,
            },
            AuthorityOperation::Apply,
        ),
        (
            RunCommand::DecideRepeatContinuation {
                repeat_execution: NodeExecutionId::new("repeat-authority-family")?,
                decision: RepeatDecisionId::new("repeat-decision-authority-family")?,
                outcome: RepeatContinuationDecision::Approved,
                approved_additional_iterations: Some(1),
            },
            AuthorityOperation::Approve,
        ),
        (
            RunCommand::ResolveExternalWork {
                attempt: AttemptId::new("attempt-authority-family")?,
                decision: ReconciliationDecisionId::new("external-decision-authority-family")?,
                action: ExternalWorkAction::Retry,
                remediation_node: None,
            },
            AuthorityOperation::Retry,
        ),
    ];
    let authority_claim = claim()?;
    for (index, (command, expected_operation)) in cases.into_iter().enumerate() {
        let document = RunCommandDocument::new(
            CommandId::new(format!("command-authority-family-{index}"))?,
            run.clone(),
            ActorRef::new("ai:authority-path-test")?,
            RunSequence::ZERO,
            TimestampMillis::new(1_000),
            Reason::new("verify shared human and AI authority request path")?,
            Vec::new(),
            command,
        )?;
        let request = document.authority_request(&authority_claim)?;
        assert_eq!(request.operation, expected_operation);
        assert_eq!(request.actor, *document.actor());
        assert_eq!(request.resources.run.as_ref(), Some(&run));
        assert_eq!(request.grant, authority_claim.grant().clone());
        assert_eq!(request.grant_revision, authority_claim.grant_revision());
    }
    let internal = RunCommandDocument::new(
        CommandId::new("command-forged-system-transition")?,
        run,
        ActorRef::new("human:forged-system-transition")?,
        RunSequence::ZERO,
        TimestampMillis::new(1_000),
        Reason::new("attempt to forge runtime-owned authority")?,
        Vec::new(),
        RunCommand::SystemTransition {
            transition: SystemTransition::DriveStructuredProgress,
        },
    )?;
    assert!(matches!(
        internal.authority_request(&authority_claim),
        Err(milkdrift_runtime::RuntimeError::InvalidCommand(_))
    ));
    Ok(())
}

#[test]
fn draft_only_actor_cannot_start_and_out_of_envelope_start_keeps_typed_denial() -> TestResult {
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let operation = OperationId::new("model.generate")?;
    let actor = ActorRef::new("human:least-privilege")?;

    {
        let directory = TempDir::new()?;
        let store = Arc::new(RedbStore::open(directory.path())?);
        let revision = sequence_revision()?;
        store.put_revision(&revision)?;
        let capability_scope = CapabilityAuthorityScope::new(
            BTreeSet::from([descriptor.identity().clone()]),
            BTreeSet::new(),
            BTreeSet::from([operation.clone()]),
            BTreeSet::new(),
            BTreeSet::new(),
            BTreeSet::from([Locality::Remote]),
            SideEffectClass::Unknown,
        )?;
        let (runtime, authority_claim) = exact_grant_service(
            store.clone(),
            Arc::new(ManualClock::new(2_000)),
            descriptor.clone(),
            &actor,
            revision.semantic().workflow(),
            BTreeSet::from([AuthorityOperation::CreateRun]),
            capability_scope,
            "draft-only",
        )?;
        let run = RunId::new("run-draft-only")?;
        let create = create_document(&revision, &run, &actor, "command-draft-only-create")?;
        assert_eq!(
            runtime
                .handle_authorized_command(&create, &authority_claim)?
                .result()
                .disposition(),
            milkdrift_persistence::CommandDisposition::Accepted
        );
        let start = RunCommandDocument::new(
            CommandId::new("command-draft-only-start")?,
            run.clone(),
            actor.clone(),
            store.head(&run)?,
            TimestampMillis::new(2_001),
            Reason::new("attempt to start draft-only work")?,
            Vec::new(),
            RunCommand::StartRun,
        )?;
        let rejected = runtime.handle_authorized_command(&start, &authority_claim)?;
        assert_eq!(
            rejected.result().disposition(),
            milkdrift_persistence::CommandDisposition::Rejected
        );
        assert!(rejected.result().authorization().is_some_and(|decision| {
            decision
                .reason_codes()
                .contains(&DecisionReasonCode::OperationMismatch)
        }));
        let projection = runtime.projection(&run)?;
        assert_eq!(projection.lifecycle(), RunLifecycle::Created);
        assert!(projection.execution_authority().is_none());
    }

    {
        let directory = TempDir::new()?;
        let store = Arc::new(RedbStore::open(directory.path())?);
        let revision = sequence_revision_with_requirement(
            CapabilityRequirement::new(operation.clone()).exact(descriptor.identity().clone()),
        )?;
        store.put_revision(&revision)?;
        let allowed_other = CapabilityId::new("different-capability")?;
        let capability_scope = CapabilityAuthorityScope::new(
            BTreeSet::from([allowed_other]),
            BTreeSet::new(),
            BTreeSet::from([operation]),
            BTreeSet::new(),
            BTreeSet::new(),
            BTreeSet::from([Locality::Remote]),
            SideEffectClass::Unknown,
        )?;
        let (runtime, authority_claim) = exact_grant_service(
            store.clone(),
            Arc::new(ManualClock::new(2_000)),
            descriptor.clone(),
            &actor,
            revision.semantic().workflow(),
            BTreeSet::from([
                AuthorityOperation::CreateRun,
                AuthorityOperation::StartRun,
                AuthorityOperation::InvokeCapability,
            ]),
            capability_scope,
            "envelope-denial",
        )?;
        let run = RunId::new("run-envelope-denial")?;
        let create = create_document(&revision, &run, &actor, "command-envelope-denial-create")?;
        runtime.handle_authorized_command(&create, &authority_claim)?;
        let start = RunCommandDocument::new(
            CommandId::new("command-envelope-denial-start")?,
            run.clone(),
            actor,
            store.head(&run)?,
            TimestampMillis::new(2_001),
            Reason::new("reject stronger embedded capability requirement")?,
            Vec::new(),
            RunCommand::StartRun,
        )?;
        let rejected = runtime.handle_authorized_command(&start, &authority_claim)?;
        assert_eq!(
            rejected.result().disposition(),
            milkdrift_persistence::CommandDisposition::Rejected
        );
        let denied = rejected
            .result()
            .authorization()
            .ok_or("envelope denial decision is absent")?;
        assert_eq!(
            denied.request().operation,
            AuthorityOperation::InvokeCapability
        );
        assert!(
            denied
                .reason_codes()
                .contains(&DecisionReasonCode::CapabilityMismatch)
        );
        assert_eq!(
            denied.request().resources.capability.as_ref(),
            Some(descriptor.identity())
        );
        let replayed = runtime.handle_authorized_command(&start, &authority_claim)?;
        assert!(replayed.replayed());
        assert_eq!(rejected.result(), replayed.result());
        assert_eq!(runtime.projection(&run)?.lifecycle(), RunLifecycle::Created);
    }
    Ok(())
}

#[test]
fn redb_run_replays_after_complete_object_teardown_and_finishes() -> TestResult {
    let directory = TempDir::new()?;
    let revision = sequence_revision()?;
    let run = RunId::new("run-process-reopen")?;
    let actor = ActorRef::new("human:runtime-test")?;
    let clock = Arc::new(ManualClock::new(1_000));

    let before_restart = {
        let store = Arc::new(RedbStore::open(directory.path())?);
        store.put_revision(&revision)?;
        let runtime = service(store.clone(), clock.clone(), "before-restart")?;
        let create = RunCommandDocument::new(
            CommandId::new("command-create-run")?,
            run.clone(),
            actor.clone(),
            RunSequence::ZERO,
            TimestampMillis::new(1_000),
            Reason::new("create restart-test run")?,
            Vec::new(),
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-run-root")?),
                workspace_budget: WorkspaceBudget::new(
                    32, 16_384, 131_072, 8, 1_048_576, 4_194_304,
                )?,
                inputs: Vec::new(),
            },
        )?;
        let accepted = runtime.handle_authorized_command(&create, &claim()?)?;
        assert!(!accepted.replayed());
        let replayed = runtime.handle_authorized_command(&create, &claim()?)?;
        assert!(replayed.replayed());
        assert_eq!(accepted.result(), replayed.result());
        assert_eq!(accepted.result().schema_version(), 2);
        assert_eq!(
            accepted
                .result()
                .authorization()
                .map(|value| value.digest()),
            replayed
                .result()
                .authorization()
                .map(|value| value.digest())
        );
        assert_eq!(store.head(&run)?, RunSequence::FIRST);

        let start = RunCommandDocument::new(
            CommandId::new("command-start-run")?,
            run.clone(),
            actor,
            store.head(&run)?,
            TimestampMillis::new(1_001),
            Reason::new("start restart-test run")?,
            Vec::new(),
            RunCommand::StartRun,
        )?;
        runtime.handle_authorized_command(&start, &claim()?)?;
        assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
        let projection = runtime.projection(&run)?;
        assert_eq!(projection.lifecycle(), RunLifecycle::Running);
        assert!(projection.execution_authority().is_some());
        assert!(projection.attempts().values().any(|attempt| {
            attempt
                .capability()
                .and_then(|capability| capability.authorization())
                .is_some_and(AuthorityDecisionSnapshot::is_allowed)
        }));
        projection
    };

    let after_restart = {
        let store = Arc::new(RedbStore::open(directory.path())?);
        let runtime = service(store, clock, "after-restart")?;
        let projection = runtime.projection(&run)?;
        assert_eq!(projection, before_restart);
        assert_eq!(
            projection
                .execution_authority()
                .map(ExecutionAuthorityBasis::digest),
            before_restart
                .execution_authority()
                .map(ExecutionAuthorityBasis::digest)
        );
        runtime.recover()?;
        let tick = runtime.tick()?;
        assert_eq!(tick.dispatched, 0);
        assert_eq!(tick.completed, 1);
        let projection = runtime.projection(&run)?;
        assert!(matches!(projection.lifecycle(), RunLifecycle::Terminal(_)));
        projection
    };

    let store = Arc::new(RedbStore::open(directory.path())?);
    let runtime = service(store, Arc::new(ManualClock::new(2_000)), "terminal-reopen")?;
    assert_eq!(runtime.projection(&run)?, after_restart);
    Ok(())
}

struct DenyAuthority(AtomicUsize);

impl AuthorityEvaluator for DenyAuthority {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.deny")?,
            1,
            request.clone(),
            vec![DecisionReasonCode::OperationMismatch],
            AuthorityBudget::default(),
            milkdrift_capability::SideEffectClass::None,
        )
    }
}

struct RevocableAuthority {
    revoked: AtomicBool,
}

impl RevocableAuthority {
    fn new() -> Self {
        Self {
            revoked: AtomicBool::new(false),
        }
    }

    fn revoke(&self) {
        self.revoked.store(true, Ordering::SeqCst);
    }
}

impl AuthorityEvaluator for RevocableAuthority {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        let reasons = if request.operation == AuthorityOperation::InvokeCapability
            && self.revoked.load(Ordering::SeqCst)
        {
            vec![DecisionReasonCode::Revoked]
        } else {
            vec![DecisionReasonCode::Allowed]
        };
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.revocable-runtime")?,
            1,
            request.clone(),
            reasons,
            AuthorityBudget {
                cost_minor: Some(u64::MAX),
                duration_ms: Some(u64::MAX),
                invocations: Some(u64::MAX),
                artifact_bytes: Some(u64::MAX),
                units: Some(u64::MAX),
                concurrency: Some(u32::MAX),
            },
            milkdrift_capability::SideEffectClass::Unknown,
        )
    }
}

struct CountingExecutor {
    inner: DeterministicExecutor,
    entries: AtomicUsize,
    block: Option<Arc<(Mutex<EntryGate>, Condvar)>>,
}

#[derive(Default)]
struct EntryGate {
    entered: bool,
    release: bool,
}

impl CountingExecutor {
    fn new(descriptor: milkdrift_capability::CapabilityDescriptor) -> Self {
        Self {
            inner: DeterministicExecutor::new(descriptor),
            entries: AtomicUsize::new(0),
            block: None,
        }
    }

    fn blocking(
        descriptor: milkdrift_capability::CapabilityDescriptor,
        block: Arc<(Mutex<EntryGate>, Condvar)>,
    ) -> Self {
        Self {
            inner: DeterministicExecutor::new(descriptor),
            entries: AtomicUsize::new(0),
            block: Some(block),
        }
    }
}

impl TaskExecutor for CountingExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.inner.resolve(requirement, observed_at_unix_ms)
    }

    fn execute(&self, dispatch: &ExecutionDispatch) -> Result<ExecutionReportBatch, ExecutorError> {
        self.entries.fetch_add(1, Ordering::SeqCst);
        if let Some(block) = &self.block {
            let (lock, changed) = &**block;
            let mut state = lock
                .lock()
                .map_err(|_error| ExecutorError::Boundary("entry gate poisoned".to_owned()))?;
            state.entered = true;
            changed.notify_all();
            while !state.release {
                state = changed.wait(state).map_err(|_error| {
                    ExecutorError::Boundary("entry gate wait poisoned".to_owned())
                })?;
            }
        }
        self.inner.execute(dispatch)
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.inner.cancel(request)
    }
}

fn revocable_service(
    store: Arc<RedbStore>,
    clock: Arc<ManualClock>,
    authority: Arc<RevocableAuthority>,
    executor: Arc<CountingExecutor>,
    id_prefix: &str,
) -> TestResult<RuntimeService> {
    Ok(RuntimeService::new_with_authority(
        store,
        executor,
        authority,
        clock,
        Arc::new(SequentialIdGenerator::new(id_prefix, 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-revocation")?,
            ActorRef::new("controller:revocation")?,
            30_000,
            16,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 100, 0)?,
        )?,
    )?)
}

fn create_and_start_revocable_run(
    runtime: &RuntimeService,
    store: &RedbStore,
    revision: &BlueprintRevision,
    run: &RunId,
) -> TestResult {
    store.put_revision(revision)?;
    let actor = ActorRef::new("human:revocation-test")?;
    let create = RunCommandDocument::new(
        CommandId::new(format!("command-create-{run}"))?,
        run.clone(),
        actor.clone(),
        RunSequence::ZERO,
        TimestampMillis::new(1_000),
        Reason::new("create revocation-boundary run")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(
                run.clone(),
                ScopeId::new(format!("scope-{run}"))?,
            ),
            workspace_budget: WorkspaceBudget::new(32, 16_384, 131_072, 8, 1_048_576, 4_194_304)?,
            inputs: Vec::new(),
        },
    )?;
    runtime.handle_authorized_command(&create, &claim()?)?;
    let start = RunCommandDocument::new(
        CommandId::new(format!("command-start-{run}"))?,
        run.clone(),
        actor,
        store.head(run)?,
        TimestampMillis::new(1_001),
        Reason::new("start revocation-boundary run")?,
        Vec::new(),
        RunCommand::StartRun,
    )?;
    runtime.handle_authorized_command(&start, &claim()?)?;
    Ok(())
}

#[test]
fn revocation_before_resolution_is_a_typed_denial_not_capability_absence() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(2_000));
    let authority = Arc::new(RevocableAuthority::new());
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let executor = Arc::new(CountingExecutor::new(descriptor));
    let runtime = revocable_service(
        store.clone(),
        clock,
        authority.clone(),
        executor.clone(),
        "revoke-before-resolution",
    )?;
    let revision = sequence_revision()?;
    let run = RunId::new("run-revoke-before-resolution")?;
    create_and_start_revocable_run(&runtime, store.as_ref(), &revision, &run)?;
    authority.revoke();
    let tick = runtime.scheduler_tick()?;
    assert_eq!(tick.completed, 1);
    assert_eq!(tick.dispatched, 0);
    assert_eq!(executor.entries.load(Ordering::SeqCst), 0);
    let history = runtime.history(&run)?;
    let denied = history.iter().find_map(|event| match event.kind() {
        milkdrift_persistence::RunEventKind::CapabilityResolutionDenied {
            authorization, ..
        } => Some(authorization),
        _ => None,
    });
    assert!(
        denied.is_some_and(|decision| { decision.reason_codes() == [DecisionReasonCode::Revoked] })
    );
    assert!(runtime.projection(&run)?.attempts().is_empty());
    Ok(())
}

#[test]
fn revocation_after_resolution_denies_effect_claim_and_releases_the_lease() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(2_000));
    let authority = Arc::new(RevocableAuthority::new());
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let executor = Arc::new(CountingExecutor::new(descriptor));
    let runtime = revocable_service(
        store.clone(),
        clock,
        authority.clone(),
        executor.clone(),
        "revoke-after-resolution",
    )?;
    let revision = sequence_revision()?;
    let run = RunId::new("run-revoke-after-resolution")?;
    create_and_start_revocable_run(&runtime, store.as_ref(), &revision, &run)?;
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    authority.revoke();
    assert!(
        runtime
            .claim_execution_effects(PageSize::new(1)?)?
            .is_empty()
    );
    assert_eq!(executor.entries.load(Ordering::SeqCst), 0);
    let history = runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        milkdrift_persistence::RunEventKind::CapabilityEntryDecisionRecorded {
            authorization,
            ..
        } if !authorization.is_allowed()
            && authorization.reason_codes() == [DecisionReasonCode::Revoked]
    )));
    assert!(
        runtime
            .projection(&run)?
            .leases()
            .values()
            .all(|lease| !lease.is_active())
    );
    Ok(())
}

#[test]
fn revocation_after_effect_claim_is_durable_and_never_enters_executor() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(2_000));
    let authority = Arc::new(RevocableAuthority::new());
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let executor = Arc::new(CountingExecutor::new(descriptor));
    let runtime = revocable_service(
        store.clone(),
        clock,
        authority.clone(),
        executor.clone(),
        "revoke-before-adapter-entry",
    )?;
    let revision = sequence_revision()?;
    let run = RunId::new("run-revoke-before-adapter-entry")?;
    create_and_start_revocable_run(&runtime, store.as_ref(), &revision, &run)?;
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    let mut effects = runtime.claim_execution_effects(PageSize::new(1)?)?;
    assert_eq!(effects.len(), 1);
    authority.revoke();
    let result = runtime.execute_effect(effects.remove(0))?;
    assert_eq!(
        result,
        milkdrift_runtime::EffectExecutionResult::Completed { observations: 0 }
    );
    assert_eq!(executor.entries.load(Ordering::SeqCst), 0);
    let history = runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        milkdrift_persistence::RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            authorization,
            ..
        } if !authorization.is_allowed()
            && authorization.reason_codes() == [DecisionReasonCode::Revoked]
    )));
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        milkdrift_persistence::RunEventKind::NodeTerminal {
            outcome: NodeOutcome::Rejected,
            error_class: Some(ErrorClass::Authorization),
            ..
        }
    )));
    let projection = runtime.projection(&run)?;
    assert!(projection.leases().values().all(|lease| !lease.is_active()));
    Ok(())
}

#[test]
fn revocation_after_adapter_entry_preserves_success_and_blocks_later_work() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(2_000));
    let authority = Arc::new(RevocableAuthority::new());
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let gate = Arc::new((Mutex::new(EntryGate::default()), Condvar::new()));
    let executor = Arc::new(CountingExecutor::blocking(descriptor, gate.clone()));
    let runtime = Arc::new(revocable_service(
        store.clone(),
        clock,
        authority.clone(),
        executor.clone(),
        "revoke-after-entry",
    )?);
    let revision = two_task_revision()?;
    let run = RunId::new("run-revoke-after-entry")?;
    create_and_start_revocable_run(runtime.as_ref(), store.as_ref(), &revision, &run)?;
    assert_eq!(runtime.scheduler_tick()?.dispatched, 1);
    let action = runtime
        .claim_execution_effects(PageSize::new(1)?)?
        .pop()
        .ok_or("first effect was not claimed")?;
    let effect_runtime = runtime.clone();
    let thread = std::thread::spawn(move || effect_runtime.execute_effect(action));
    {
        let (lock, changed) = &*gate;
        let mut state = lock.lock().map_err(|_error| "entry gate poisoned")?;
        while !state.entered {
            state = changed
                .wait(state)
                .map_err(|_error| "entry gate wait poisoned")?;
        }
    }
    authority.revoke();
    {
        let (lock, changed) = &*gate;
        let mut state = lock.lock().map_err(|_error| "entry gate poisoned")?;
        state.release = true;
        changed.notify_all();
    }
    let effect = thread.join().map_err(|_panic| "effect thread panicked")??;
    assert_eq!(
        effect,
        milkdrift_runtime::EffectExecutionResult::Completed { observations: 1 }
    );
    let after_first = runtime.history(&run)?;
    assert!(after_first.iter().any(|event| matches!(
        event.kind(),
        milkdrift_persistence::RunEventKind::NodeTerminal {
            outcome: NodeOutcome::Succeeded,
            ..
        }
    )));
    let tick = runtime.scheduler_tick()?;
    assert_eq!(tick.completed, 1);
    assert_eq!(tick.dispatched, 0);
    assert_eq!(executor.entries.load(Ordering::SeqCst), 1);
    assert!(runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        milkdrift_persistence::RunEventKind::CapabilityResolutionDenied {
            authorization,
            ..
        } if authorization.reason_codes() == [DecisionReasonCode::Revoked]
    )));
    Ok(())
}

#[test]
fn denied_command_is_durable_idempotent_and_has_no_semantic_mutation() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let revision = sequence_revision()?;
    store.put_revision(&revision)?;
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let authority = Arc::new(DenyAuthority(AtomicUsize::new(0)));
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        Arc::new(DeterministicExecutor::new(descriptor)),
        authority.clone(),
        Arc::new(ManualClock::new(5_000)),
        Arc::new(SequentialIdGenerator::new("denied-command", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-denied")?,
            ActorRef::new("controller:denied")?,
            30_000,
            8,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 100, 0)?,
        )?,
    )?;
    let run = RunId::new("run-denied")?;
    let command = RunCommandDocument::new(
        CommandId::new("command-denied")?,
        run.clone(),
        ActorRef::new("human:denied")?,
        RunSequence::ZERO,
        TimestampMillis::new(5_000),
        Reason::new("must be denied")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-denied")?),
            workspace_budget: WorkspaceBudget::new(1, 1, 1, 0, 0, 0)?,
            inputs: Vec::new(),
        },
    )?;
    let first = runtime.handle_authorized_command(&command, &claim()?)?;
    assert_eq!(
        first.result().disposition(),
        milkdrift_persistence::CommandDisposition::Rejected
    );
    assert!(
        first
            .result()
            .authorization()
            .is_some_and(|value| !value.is_allowed())
    );
    assert_eq!(store.head(&run)?, RunSequence::ZERO);
    assert!(runtime.history(&run)?.is_empty());
    let replay = runtime.handle_authorized_command(&command, &claim()?)?;
    assert!(replay.replayed());
    assert_eq!(first.result(), replay.result());
    assert_eq!(authority.0.load(Ordering::SeqCst), 1);
    Ok(())
}
