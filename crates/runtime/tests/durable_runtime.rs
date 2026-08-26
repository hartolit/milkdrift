//! Process-style headless runtime evidence against the production local store.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    AuthorityOperation, DecisionReasonCode, GrantId, PolicyId,
};

use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, Edge, EdgeId, EdgeKind, Mutation, MutationBatch, Node, NodeId,
    NodeKind, PortId, TerminalOutcome, WorkflowId, WorkflowInterface,
};
use milkdrift_capability::{
    BoundedJson, CapabilityDescriptorDocument, CapabilityRequirement, ErrorClass, OperationId,
};
use milkdrift_persistence::{
    ActorRef, AttemptId, AuthorityDecision, CommandId, NodeExecutionId, Reason,
    ReconciliationDecisionId, ReconciliationId, ReconciliationPlanId, ReconciliationPolicy,
    RepeatContinuationDecision, RepeatDecisionId, RevisionStore, RunJournal, RunSequence,
    SignalDeliveryMode, SignalId, SignalTypeId, TimerId, TimestampMillis, WorkerId,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    CommandAuthorityClaim, DeterministicExecutor, ExternalWorkAction, ManualClock, RetryPolicy,
    RunCommand, RunCommandDocument, RunLifecycle, RuntimeConfig, RuntimeService, SchedulerLimits,
    SequentialIdGenerator, SystemTransition,
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
        0,
    )?)
}

fn sequence_revision() -> TestResult<BlueprintRevision> {
    let workflow = WorkflowId::new("runtime-reopen")?;
    let task_id = NodeId::new("generate")?;
    let terminal_id = NodeId::new("done")?;
    let next = PortId::new("next")?;
    let input = PortId::new("in")?;
    let task = Node::new(
        task_id.clone(),
        NodeKind::task_direct_inputs(CapabilityRequirement::new(OperationId::new(
            "model.generate",
        )?))?,
    )?
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
        let projection = runtime.projection(&run)?;
        assert_eq!(projection.lifecycle(), RunLifecycle::Running);
        projection
    };

    let after_restart = {
        let store = Arc::new(RedbStore::open(directory.path())?);
        let runtime = service(store, clock, "after-restart")?;
        let projection = runtime.projection(&run)?;
        assert_eq!(projection, before_restart);
        runtime.recover()?;
        let tick = runtime.tick()?;
        assert_eq!(tick.dispatched, 1);
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
