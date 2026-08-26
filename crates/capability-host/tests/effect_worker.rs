//! Bounded worker admission, running-work shutdown, and permit-release evidence.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    CapabilityAuthorityScope, DecisionReasonCode, GrantId, PolicyId,
};
use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, Edge, EdgeId, EdgeKind, Mutation, MutationBatch, Node, NodeId,
    NodeKind, PortId, TerminalOutcome, WorkflowId, WorkflowInterface,
};
use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, CapabilityDescriptorDocument,
    CapabilityObservation, CapabilityRequirement, ErrorClass, InvocationEvent, InvocationEventKind,
    InvocationTerminal, OperationId, SideEffectClass, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, CapabilityHost,
    CapabilitySelectionPolicy, EffectShutdownMode, EffectWorkerConfig, EffectWorkerHost,
    HostConfig,
};
use milkdrift_persistence::{
    ActorRef, CommandId, Reason, RevisionStore, RunJournal, RunSequence, TimestampMillis, WorkerId,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    CommandAuthorityClaim, ManualClock, RetryPolicy, RunCommand, RunCommandDocument, RuntimeConfig,
    RuntimeService, SchedulerLimits, SequentialIdGenerator,
};
use milkdrift_workspace::{RunId, ScopeId, WorkspaceBudget, WorkspaceScope};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct AllowAuthority;

impl AuthorityEvaluator for AllowAuthority {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.effect-worker")?,
            1,
            request.clone(),
            vec![DecisionReasonCode::Allowed],
            AuthorityBudget::default(),
            SideEffectClass::Unknown,
        )
    }
}

struct BlockingAdapter {
    gate: Arc<(Mutex<bool>, Condvar)>,
    entered: AtomicUsize,
}

impl BlockingAdapter {
    fn new(gate: Arc<(Mutex<bool>, Condvar)>) -> Self {
        Self {
            gate,
            entered: AtomicUsize::new(0),
        }
    }
}

impl CapabilityAdapter for BlockingAdapter {
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let (lock, changed) = &*self.gate;
        let mut released = lock
            .lock()
            .map_err(|_| AdapterError::external_failure("test gate poisoned"))?;
        while !*released {
            released = changed
                .wait(released)
                .map_err(|_| AdapterError::external_failure("test gate poisoned"))?;
        }
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
            true,
            false,
            Some("test cancellation accepted".to_owned()),
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            milkdrift_capability::CapabilityId::new("cap-local-test")
                .map_err(|error| AdapterError::unavailable(error.to_string()))?,
            observed_at_unix_ms,
            true,
            u32::try_from(self.entered.load(Ordering::SeqCst)).unwrap_or(u32::MAX),
            "test adapter ready",
        )
        .map_err(|error| AdapterError::unavailable(error.to_string()))
    }

    fn shutdown(&self) -> Result<(), AdapterError> {
        let (lock, changed) = &*self.gate;
        *lock
            .lock()
            .map_err(|_| AdapterError::external_failure("test gate poisoned"))? = true;
        changed.notify_all();
        Ok(())
    }
}

fn revision() -> TestResult<BlueprintRevision> {
    let task_id = NodeId::new("task")?;
    let terminal_id = NodeId::new("done")?;
    let task = Node::new(
        task_id.clone(),
        NodeKind::task_direct_inputs(CapabilityRequirement::new(OperationId::new(
            "model.generate",
        )?))?,
    )?
    .with_control_output(PortId::new("next")?)?;
    let terminal = Node::new(
        terminal_id.clone(),
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(PortId::new("in")?)?;
    Ok(BlueprintRevision::genesis(
        WorkflowId::new("effect-worker-workflow")?,
        MutationBatch::new(vec![
            Mutation::SetInterface {
                interface: WorkflowInterface::new([], [])?,
            },
            Mutation::AddNode { node: task },
            Mutation::AddNode { node: terminal },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("task-done")?,
                    EdgeKind::Control,
                    task_id,
                    PortId::new("next")?,
                    terminal_id,
                    PortId::new("in")?,
                ),
            },
        ])?,
        AuthorRef::new("human:effect-worker-test")?,
        "effect worker fixture",
    )?)
}

fn authority_claim() -> TestResult<CommandAuthorityClaim> {
    Ok(CommandAuthorityClaim::new(
        GrantId::new("grant:effect-worker-test")?,
        1,
        0,
    )?)
}

fn create_and_start(
    runtime: &RuntimeService,
    store: &RedbStore,
    revision: &BlueprintRevision,
    index: usize,
) -> TestResult {
    let run = RunId::new(format!("run-effect-worker-{index}"))?;
    let actor = ActorRef::new("human:effect-worker-test")?;
    let create = RunCommandDocument::new(
        CommandId::new(format!("command-create-effect-worker-{index}"))?,
        run.clone(),
        actor.clone(),
        RunSequence::ZERO,
        TimestampMillis::new(1_000),
        Reason::new("create effect-worker fixture")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(
                run.clone(),
                ScopeId::new(format!("scope-effect-worker-{index}"))?,
            ),
            workspace_budget: WorkspaceBudget::new(8, 1024, 4096, 4, 1024, 4096)?,
            inputs: Vec::new(),
        },
    )?;
    let _ = runtime.handle_authorized_command(&create, &authority_claim()?)?;
    let start = RunCommandDocument::new(
        CommandId::new(format!("command-start-effect-worker-{index}"))?,
        run.clone(),
        actor,
        store.head(&run)?,
        TimestampMillis::new(1_001),
        Reason::new("start effect-worker fixture")?,
        Vec::new(),
        RunCommand::StartRun,
    )?;
    let _ = runtime.handle_authorized_command(&start, &authority_claim()?)?;
    Ok(())
}

#[test]
fn bounded_queues_backpressure_and_forced_shutdown_preserves_unresolved_truth() -> TestResult {
    let directory = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let revision = revision()?;
    store.put_revision(&revision)?;
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 4,
            max_generations_per_capability: 2,
            max_concurrent_per_generation: 4,
            observation_stale_after_ms: 10_000,
        },
        CapabilitySelectionPolicy::new(
            CapabilityAuthorityScope::any(SideEffectClass::Unknown),
            AuthorityBudget::default(),
            BTreeMap::new(),
        ),
    )?;
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let adapter = Arc::new(BlockingAdapter::new(gate));
    host.register(
        descriptor.clone(),
        adapter.clone(),
        Some(CapabilityObservation::new(
            descriptor.identity().clone(),
            1_000,
            true,
            0,
            "ready",
        )?),
    )?;
    let runtime = Arc::new(RuntimeService::new_with_authority(
        store.clone(),
        Arc::new(host.clone()),
        Arc::new(AllowAuthority),
        Arc::new(ManualClock::new(1_000)),
        Arc::new(SequentialIdGenerator::new("effect-worker", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-effect-host")?,
            ActorRef::new("controller:effect-host")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, vec![ErrorClass::Transport], 10, 100, 0)?,
        )?,
    )?);
    for index in 0..3 {
        create_and_start(runtime.as_ref(), store.as_ref(), &revision, index)?;
    }
    let scheduled = runtime.scheduler_tick()?;
    assert_eq!(scheduled.dispatched, 3);
    let workers = EffectWorkerHost::start(
        runtime,
        host.clone(),
        EffectWorkerConfig {
            execution_threads: 1,
            execution_queue: 1,
            cancellation_queue: 1,
            maximum_claim_page: 1,
        },
    )?;
    assert_eq!(workers.poll()?.executions, 1);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while adapter.entered.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(adapter.entered.load(Ordering::SeqCst), 1);
    assert_eq!(workers.poll()?.executions, 1);
    assert_eq!(workers.poll()?.executions, 0);
    let health = workers.health()?;
    assert_eq!(health.active_executions, 1);
    assert_eq!(health.queued_executions, 1);

    let shutdown = workers.shutdown(EffectShutdownMode::Cancel, Duration::from_secs(3))?;
    assert!(shutdown.clean);
    assert_eq!(shutdown.unresolved_invocations.len(), 1);
    assert_eq!(shutdown.health.active_executions, 0);
    assert!(
        host.generations(
            &CapabilityAuthorityScope::any(SideEffectClass::Unknown),
            1_000
        )?
        .is_empty()
    );
    Ok(())
}

#[test]
fn worker_configuration_is_bounded() {
    assert!(
        EffectWorkerConfig {
            execution_threads: 0,
            execution_queue: 1,
            cancellation_queue: 1,
            maximum_claim_page: 1,
        }
        .validate()
        .is_err()
    );
}
