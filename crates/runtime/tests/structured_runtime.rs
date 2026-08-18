//! Black-box structured-runtime evidence using the production redb store.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use milkdrift_blueprint::{
    AuthorRef, BindingSource, BlueprintRevision, BranchConfig, Condition, DataPort, Edge, EdgeId,
    EdgeKind, FieldId, ForkConfig, InterfaceField, JoinConfig, JoinPolicy, Mutation, MutationBatch,
    Node, NodeId, NodeKind, PathSegment, PathSelector, PinnedSubworkflow, PortId, ReducerConfig,
    ReducerStrategy, RepeatBudget, RepeatConfig, RepeatTermination, SchemaRef, TerminalOutcome,
    WorkflowId, WorkflowInterface,
};
use milkdrift_capability::{
    ArtifactReference as InvocationArtifactReference, BoundedJson, CancellationAcknowledgement,
    CancellationRequest, CapabilityDescriptor, CapabilityDescriptorDocument, CapabilityRequirement,
    ErrorClass, InvocationEvent, InvocationEventKind, InvocationFailure, InvocationRequest,
    InvocationTerminal, InvocationValueReference, OperationId, SchemaId, SideEffectClass,
    TerminalStatus,
};
use milkdrift_persistence::{
    ActorRef, ArtifactPublicationId, ArtifactStore, AuthorityDecision, BeginArtifactPublication,
    CommandDisposition, Reason, ReconciliationAction, ReconciliationClassification,
    ReconciliationDecisionId, ReconciliationId, ReconciliationPolicy, RecoveryClassification,
    RepeatContinuationCause, RepeatContinuationDecision, RepeatDecisionId, RepeatTerminationReason,
    RevisionStore, RunEventKind, RunJournal, RunOutcome, SignalDeliveryMode, SignalId,
    SignalTypeId, WorkerId, WorkspaceStore,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    AttemptState, BranchState, DeterministicExecutor, ExecutionDispatch, ExecutionReportBatch,
    ExecutorError, IterationState, LeaseState, ManualClock, NodeExecutionState, ResolvedCapability,
    RetryPolicy, RunCommand, RunLifecycle, RuntimeConfig, RuntimeService, SchedulerLimits,
    SequentialIdGenerator, SubworkflowState, TaskExecutor,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactRetention, ArtifactSensitivity,
    CausalId, CausalReference, ContentDigest, MediaType, RunId, ScopeId, ScopeKind, ValueKey,
    ValueOrigin, WorkspaceBudget, WorkspaceScope, WorkspaceUsage, WorkspaceValue,
    WorkspaceValueEntry,
};
use serde_json::json;
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const NOW: u64 = 10_000;

struct Harness {
    _directory: TempDir,
    store: Arc<RedbStore>,
    clock: Arc<ManualClock>,
    executor: Arc<DeterministicExecutor>,
    runtime: RuntimeService,
}

struct BlockingExecutor {
    resolver: DeterministicExecutor,
    blocking_operation: OperationId,
    entered: (Mutex<bool>, Condvar),
    released: (Mutex<bool>, Condvar),
    cancellation_requests: AtomicUsize,
}

struct PanickingExecutor {
    resolver: DeterministicExecutor,
}

struct RecordingExecutor {
    delegate: DeterministicExecutor,
    dispatches: Mutex<Vec<ExecutionDispatch>>,
}

impl RecordingExecutor {
    fn new() -> TestResult<Self> {
        Ok(Self {
            delegate: DeterministicExecutor::new(test_descriptor()?),
            dispatches: Mutex::new(Vec::new()),
        })
    }

    fn set_script(
        &self,
        operation: OperationId,
        script: Vec<InvocationEventKind>,
    ) -> TestResult {
        self.delegate.set_script(operation, script)?;
        Ok(())
    }

    fn dispatches(&self) -> TestResult<Vec<ExecutionDispatch>> {
        Ok(self
            .dispatches
            .lock()
            .map_err(|_| "recording executor lock poisoned")?
            .clone())
    }
}

impl TaskExecutor for RecordingExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.delegate.resolve(requirement)
    }

    fn execute(&self, dispatch: &ExecutionDispatch) -> Result<ExecutionReportBatch, ExecutorError> {
        self.dispatches
            .lock()
            .map_err(|_| ExecutorError::Boundary("recording executor lock poisoned".to_owned()))?
            .push(dispatch.clone());
        self.delegate.execute(dispatch)
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.delegate.cancel(request)
    }
}

impl TaskExecutor for PanickingExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement)
    }

    fn execute(
        &self,
        _dispatch: &ExecutionDispatch,
    ) -> Result<ExecutionReportBatch, ExecutorError> {
        std::panic::resume_unwind(Box::new(
            "intentional crash after durable schedule and lease",
        ))
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.resolver.cancel(request)
    }
}

impl BlockingExecutor {
    fn new(descriptor: CapabilityDescriptor) -> TestResult<Self> {
        Ok(Self {
            resolver: DeterministicExecutor::new(descriptor),
            blocking_operation: OperationId::new("model.generate")?,
            entered: (Mutex::new(false), Condvar::new()),
            released: (Mutex::new(false), Condvar::new()),
            cancellation_requests: AtomicUsize::new(0),
        })
    }

    fn wait_until_entered(&self) -> TestResult {
        let (lock, ready) = &self.entered;
        let entered = lock.lock().map_err(|_| "entered lock poisoned")?;
        let (entered, timeout) = ready
            .wait_timeout_while(entered, Duration::from_secs(5), |entered| !*entered)
            .map_err(|_| "entered wait poisoned")?;
        if timeout.timed_out() || !*entered {
            return Err("executor dispatch was not observed before timeout".into());
        }
        Ok(())
    }

    fn release(&self) -> TestResult {
        let (lock, released) = &self.released;
        *lock.lock().map_err(|_| "release lock poisoned")? = true;
        released.notify_all();
        Ok(())
    }
}

impl TaskExecutor for BlockingExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement)
    }

    fn execute(&self, dispatch: &ExecutionDispatch) -> Result<ExecutionReportBatch, ExecutorError> {
        let blocked = dispatch.request().operation() == &self.blocking_operation;
        if blocked {
            {
                let (lock, entered) = &self.entered;
                *lock
                    .lock()
                    .map_err(|_| ExecutorError::Boundary("entered lock poisoned".to_owned()))? =
                    true;
                entered.notify_all();
            }
            let (lock, released) = &self.released;
            let mut permit = lock
                .lock()
                .map_err(|_| ExecutorError::Boundary("release lock poisoned".to_owned()))?;
            while !*permit {
                permit = released
                    .wait(permit)
                    .map_err(|_| ExecutorError::Boundary("release wait poisoned".to_owned()))?;
            }
        }
        let terminal = InvocationTerminal::new(
            if blocked {
                TerminalStatus::Cancelled
            } else {
                TerminalStatus::Success
            },
            Vec::new(),
            None,
            None,
            SideEffectClass::None,
        )?;
        let event = InvocationEvent::new(
            dispatch.request().invocation().clone(),
            1,
            InvocationEventKind::Terminal { terminal },
        )?;
        ExecutionReportBatch::new(dispatch.request(), vec![event])
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.cancellation_requests.fetch_add(1, Ordering::SeqCst);
        Ok(CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            true,
            false,
            Some("blocking executor observed cancellation intent".to_owned()),
        )?)
    }
}

impl Harness {
    fn new(prefix: &str) -> TestResult<Self> {
        let directory = TempDir::new()?;
        let store = Arc::new(RedbStore::open(directory.path())?);
        let clock = Arc::new(ManualClock::new(NOW));
        let executor = Arc::new(DeterministicExecutor::new(test_descriptor()?));
        let config = RuntimeConfig::new(
            WorkerId::new(format!("worker-{prefix}"))?,
            ActorRef::new(format!("controller:{prefix}"))?,
            30_000,
            64,
            SchedulerLimits::new(64, 32, 16, 32)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?;
        let runtime = RuntimeService::new(
            store.clone(),
            executor.clone(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new(prefix, 1)?),
            config,
        )?;
        Ok(Self {
            _directory: directory,
            store,
            clock,
            executor,
            runtime,
        })
    }

    fn put_revision(&self, revision: &BlueprintRevision) -> TestResult {
        self.store.put_revision(revision)?;
        Ok(())
    }

    fn command(&self, run: &RunId, command: RunCommand) -> TestResult<CommandDisposition> {
        let document = self.runtime.command(
            run.clone(),
            ActorRef::new("human:structured-runtime-test")?,
            self.store.head(run)?,
            Reason::new("structured runtime integration transition")?,
            Vec::new(),
            command,
        )?;
        Ok(self
            .runtime
            .handle_command(&document)?
            .result()
            .disposition())
    }

    fn create(&self, run: &RunId, revision: &BlueprintRevision) -> TestResult {
        assert_eq!(
            self.command(
                run,
                RunCommand::CreateRun {
                    workflow: revision.semantic().workflow().clone(),
                    revision: revision.id().clone(),
                    root_scope: WorkspaceScope::run_root(
                        run.clone(),
                        ScopeId::new(format!("scope-{run}"))?,
                    ),
                    workspace_budget: generous_budget()?,
                    inputs: Vec::new(),
                },
            )?,
            CommandDisposition::Accepted
        );
        Ok(())
    }

    fn create_and_start(&self, run: &RunId, revision: &BlueprintRevision) -> TestResult {
        self.create(run, revision)?;
        assert_eq!(
            self.command(run, RunCommand::StartRun)?,
            CommandDisposition::Accepted
        );
        Ok(())
    }

    fn drive(&self, run: &RunId, maximum_ticks: usize) -> TestResult<u32> {
        let mut dispatched = 0_u32;
        for _ in 0..maximum_ticks {
            if self.runtime.projection(run)?.is_completed() {
                break;
            }
            let tick = self.runtime.tick()?;
            dispatched = dispatched.saturating_add(tick.dispatched);
        }
        Ok(dispatched)
    }
}

fn recovery_service(
    store: Arc<RedbStore>,
    clock: Arc<ManualClock>,
    executor: Arc<dyn TaskExecutor>,
    prefix: &str,
) -> TestResult<RuntimeService> {
    Ok(RuntimeService::new(
        store,
        executor,
        clock,
        Arc::new(SequentialIdGenerator::new(prefix, 1)?),
        RuntimeConfig::new(
            WorkerId::new(format!("worker-{prefix}"))?,
            ActorRef::new(format!("controller:{prefix}"))?,
            100,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(2, vec![ErrorClass::Transport], 1, 1_000, 0)?,
        )?,
    )?)
}

fn submit_command(
    runtime: &RuntimeService,
    store: &RedbStore,
    run: &RunId,
    command: RunCommand,
) -> TestResult<CommandDisposition> {
    let document = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(run)?,
        Reason::new("focused structured runtime transition")?,
        Vec::new(),
        command,
    )?;
    Ok(runtime.handle_command(&document)?.result().disposition())
}

fn generous_budget() -> TestResult<WorkspaceBudget> {
    Ok(WorkspaceBudget::new(
        2_048,
        8 * 1_024 * 1_024,
        16 * 1_024 * 1_024,
        256,
        64 * 1_024 * 1_024,
        128 * 1_024 * 1_024,
    )?)
}

fn test_descriptor() -> TestResult<CapabilityDescriptor> {
    let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    let operations = value
        .get_mut("descriptor")
        .and_then(|descriptor| descriptor.get_mut("operations"))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("descriptor fixture has no operations object")?;
    let template = operations
        .get("model.generate")
        .cloned()
        .ok_or("descriptor fixture has no model.generate operation")?;
    operations.insert("model.fail".to_owned(), template);
    Ok(
        CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&value)?)?
            .body()
            .clone(),
    )
}

fn empty_interface() -> TestResult<WorkflowInterface> {
    Ok(WorkflowInterface::new([], [])?)
}

fn revision(workflow: &str, nodes: Vec<Node>, edges: Vec<Edge>) -> TestResult<BlueprintRevision> {
    revision_with_interface(workflow, empty_interface()?, nodes, edges)
}

fn revision_with_interface(
    workflow: &str,
    interface: WorkflowInterface,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
) -> TestResult<BlueprintRevision> {
    let mut operations = vec![Mutation::SetInterface { interface }];
    operations.extend(nodes.into_iter().map(|node| Mutation::AddNode { node }));
    operations.extend(edges.into_iter().map(|edge| Mutation::AddEdge { edge }));
    Ok(BlueprintRevision::genesis(
        WorkflowId::new(workflow)?,
        MutationBatch::new(operations)?,
        AuthorRef::new("human:structured-runtime-test")?,
        "structured runtime integration fixture",
    )?)
}

fn control_edge(
    id: &str,
    source: &str,
    source_port: &str,
    target: &str,
    target_port: &str,
) -> TestResult<Edge> {
    Ok(Edge::new(
        EdgeId::new(id)?,
        EdgeKind::Control,
        NodeId::new(source)?,
        PortId::new(source_port)?,
        NodeId::new(target)?,
        PortId::new(target_port)?,
    ))
}

fn data_edge(
    id: &str,
    source: &str,
    source_port: &str,
    target: &str,
    target_port: &str,
) -> TestResult<Edge> {
    Ok(Edge::new(
        EdgeId::new(id)?,
        EdgeKind::Data,
        NodeId::new(source)?,
        PortId::new(source_port)?,
        NodeId::new(target)?,
        PortId::new(target_port)?,
    ))
}

fn terminal(id: &str, outcome: TerminalOutcome) -> TestResult<Node> {
    Ok(Node::new(NodeId::new(id)?, NodeKind::Terminal { outcome })?
        .with_control_input(PortId::new("in")?)?)
}

fn task(id: &str, operation: &str) -> TestResult<Node> {
    Ok(Node::new(
        NodeId::new(id)?,
        NodeKind::Task {
            requirement: CapabilityRequirement::new(OperationId::new(operation)?),
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?)
}

fn successful_terminal() -> TestResult<InvocationTerminal> {
    Ok(InvocationTerminal::new(
        TerminalStatus::Success,
        Vec::new(),
        None,
        None,
        SideEffectClass::None,
    )?)
}

fn failed_terminal() -> TestResult<InvocationTerminal> {
    Ok(InvocationTerminal::new(
        TerminalStatus::Failure,
        Vec::new(),
        Some(InvocationFailure::new(
            ErrorClass::Provider,
            false,
            "scripted_failure",
            "deterministic branch failure",
            None,
        )?),
        None,
        SideEffectClass::None,
    )?)
}

fn branch_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let true_port = PortId::new("true")?;
    let false_port = PortId::new("false")?;
    let branch = Node::new(
        NodeId::new("route")?,
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(true_port.clone(), Condition::Constant { value: true })]),
                Some(false_port.clone()),
            )?,
        },
    )?
    .with_control_output(true_port)?
    .with_control_output(false_port)?;
    revision(
        workflow,
        vec![
            branch,
            terminal("selected", TerminalOutcome::Success)?,
            terminal("unselected", TerminalOutcome::Failure)?,
        ],
        vec![
            control_edge("route-selected", "route", "true", "selected", "in")?,
            control_edge("route-unselected", "route", "false", "unselected", "in")?,
        ],
    )
}

fn fork_revision(
    workflow: &str,
    policy: JoinPolicy,
    second_operation: &str,
) -> TestResult<BlueprintRevision> {
    let a = PortId::new("a")?;
    let b = PortId::new("b")?;
    let fork = Node::new(
        NodeId::new("fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(BTreeSet::from([a.clone(), b.clone()]))?,
        },
    )?
    .with_control_output(a)?
    .with_control_output(b)?;
    let join = Node::new(
        NodeId::new("join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("fork")?, policy),
        },
    )?
    .with_control_input(PortId::new("a-in")?)?
    .with_control_input(PortId::new("b-in")?)?
    .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![
            fork,
            task("a-task", "model.generate")?,
            task("b-task", second_operation)?,
            join,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("fork-a", "fork", "a", "a-task", "in")?,
            control_edge("fork-b", "fork", "b", "b-task", "in")?,
            control_edge("a-join", "a-task", "out", "join", "a-in")?,
            control_edge("b-join", "b-task", "out", "join", "b-in")?,
            control_edge("join-done", "join", "out", "done", "in")?,
        ],
    )
}

fn item_schema() -> TestResult<SchemaRef> {
    Ok(SchemaRef::new(SchemaId::new("milkdrift.test.item")?, 1)?)
}

fn reducer_revision(workflow: &str, strategy: ReducerStrategy) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let a = PortId::new("a")?;
    let b = PortId::new("b")?;
    let fork = Node::new(
        NodeId::new("fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(BTreeSet::from([a.clone(), b.clone()]))?,
        },
    )?
    .with_control_output(a)?
    .with_control_output(b)?;
    let a_task = task("a-task", "model.generate")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let b_task = task("b-task", "model.fail")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let join = Node::new(
        NodeId::new("join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("fork")?, JoinPolicy::All),
        },
    )?
    .with_control_input(PortId::new("a-in")?)?
    .with_control_input(PortId::new("b-in")?)?
    .with_control_output(PortId::new("out")?)?;
    let reducer = Node::new(
        NodeId::new("reduce")?,
        NodeKind::Reducer {
            config: ReducerConfig::new(PortId::new("items")?, schema.clone(), 2, strategy)?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?
    .with_data_input(
        PortId::new("items")?,
        DataPort::input(schema.clone(), true, None)?,
    )?
    .with_data_output(PortId::new("reduced")?, DataPort::output(schema))?;
    revision(
        workflow,
        vec![
            fork,
            a_task,
            b_task,
            join,
            reducer,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("fork-a", "fork", "a", "a-task", "in")?,
            control_edge("fork-b", "fork", "b", "b-task", "in")?,
            control_edge("a-join", "a-task", "out", "join", "a-in")?,
            control_edge("b-join", "b-task", "out", "join", "b-in")?,
            control_edge("join-reduce", "join", "out", "reduce", "in")?,
            control_edge("reduce-done", "reduce", "out", "done", "in")?,
            data_edge("a-reduce", "a-task", "item", "reduce", "items")?,
            data_edge("b-reduce", "b-task", "item", "reduce", "items")?,
        ],
    )
}

fn publish_artifact(
    harness: &Harness,
    suffix: &str,
    bytes: &[u8],
) -> TestResult<InvocationArtifactReference> {
    let digest = ContentDigest::for_bytes(bytes);
    let artifact = ArtifactId::new(format!("artifact-{suffix}"))?;
    let reference = milkdrift_workspace::ArtifactReference::new(
        artifact,
        digest,
        MediaType::new("application/octet-stream")?,
        u64::try_from(bytes.len())?,
    );
    let metadata = ArtifactMetadata::new(
        reference,
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new(format!("source-{suffix}"))?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = ArtifactPublicationId::new(format!("publication-{suffix}"))?;
    let request = BeginArtifactPublication::new(
        publication.clone(),
        RunId::new(format!("artifact-publisher-{suffix}"))?,
        metadata.clone(),
        generous_budget()?,
        WorkspaceUsage::EMPTY,
    )?;
    harness.store.begin_publication(&request)?;
    harness.store.write_chunk(&publication, 0, bytes)?;
    harness.store.commit_publication(&publication)?;
    Ok(InvocationArtifactReference::new(
        metadata.reference().artifact().as_str(),
        digest.to_hex(),
        Some("application/octet-stream".to_owned()),
        Some(u64::try_from(bytes.len())?),
    )?)
}

fn publish_artifact_for_run(
    harness: &Harness,
    run: &RunId,
    suffix: &str,
    bytes: &[u8],
) -> TestResult<milkdrift_workspace::ArtifactReference> {
    let digest = ContentDigest::for_bytes(bytes);
    let reference = milkdrift_workspace::ArtifactReference::new(
        ArtifactId::new(format!("artifact-{suffix}"))?,
        digest,
        MediaType::new("application/octet-stream")?,
        u64::try_from(bytes.len())?,
    );
    let metadata = ArtifactMetadata::new(
        reference.clone(),
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new(format!("source-{suffix}"))?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = ArtifactPublicationId::new(format!("publication-{suffix}"))?;
    harness
        .store
        .begin_publication(&BeginArtifactPublication::new(
            publication.clone(),
            run.clone(),
            metadata,
            generous_budget()?,
            WorkspaceUsage::EMPTY,
        )?)?;
    harness.store.write_chunk(&publication, 0, bytes)?;
    harness.store.commit_publication(&publication)?;
    Ok(reference)
}

fn install_output_scripts(harness: &Harness) -> TestResult {
    let a = publish_artifact(harness, "a", b"artifact-a")?;
    let b = publish_artifact(harness, "b", b"artifact-b")?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "item".to_owned(),
                reference: a,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    harness.executor.set_script(
        OperationId::new("model.fail")?,
        vec![
            InvocationEventKind::Output {
                name: "item".to_owned(),
                reference: b,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    Ok(())
}

fn install_child_output_script(harness: &Harness) -> TestResult {
    let artifact = publish_artifact(harness, "child-result", b"child-result")?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "result".to_owned(),
                reference: artifact,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    Ok(())
}

fn wait_revision(workflow: &str, duration_ms: u64) -> TestResult<BlueprintRevision> {
    let wait = Node::new(NodeId::new("wait")?, NodeKind::Wait { duration_ms })?
        .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![wait, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("wait-done", "wait", "out", "done", "in")?],
    )
}

fn signal_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let signal = Node::new(
        NodeId::new("signal")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.ready")?,
        },
    )?
    .with_control_output(PortId::new("out")?)?;
    let wait = Node::new(NodeId::new("settle")?, NodeKind::Wait { duration_ms: 50 })?
        .with_control_input(PortId::new("in")?)?
        .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![signal, wait, terminal("done", TerminalOutcome::Success)?],
        vec![
            control_edge("signal-settle", "signal", "out", "settle", "in")?,
            control_edge("settle-done", "settle", "out", "done", "in")?,
        ],
    )
}

fn output_child_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let task = task("produce", "model.generate")?
        .with_data_output(PortId::new("result")?, DataPort::output(schema.clone()))?;
    let terminal = terminal("done", TerminalOutcome::Success)?.with_data_input(
        PortId::new("result")?,
        DataPort::input(schema.clone(), true, None)?,
    )?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new(
            [],
            [(FieldId::new("result")?, InterfaceField::required(schema))],
        )?,
        vec![task, terminal],
        vec![
            control_edge("produce-done", "produce", "out", "done", "in")?,
            data_edge("produce-result", "produce", "result", "done", "result")?,
        ],
    )
}

fn task_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    revision(
        workflow,
        vec![
            task("work", "model.generate")?,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![control_edge("work-done", "work", "out", "done", "in")?],
    )
}

fn artifact_reuse_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let task = task("reuse", "model.generate")?
        .with_data_output(PortId::new("result")?, DataPort::output(schema.clone()))?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new(
            [(
                FieldId::new("initial-artifact")?,
                InterfaceField::required(schema),
            )],
            [],
        )?,
        vec![task, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("reuse-done", "reuse", "out", "done", "in")?],
    )
}

fn subworkflow_revision(
    workflow: &str,
    child: &BlueprintRevision,
) -> TestResult<BlueprintRevision> {
    let mut child_node = Node::new(
        NodeId::new("child")?,
        NodeKind::Subworkflow {
            reference: PinnedSubworkflow::new(
                child.semantic().workflow().clone(),
                child.id().clone(),
                child.semantic().interface().clone(),
            ),
        },
    )?
    .with_control_output(PortId::new("out")?)?;
    for (field, declaration) in child.semantic().interface().outputs() {
        child_node = child_node.with_data_output(
            PortId::new(field.as_str())?,
            DataPort::output(declaration.schema().clone()),
        )?;
    }
    revision(
        workflow,
        vec![child_node, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("child-done", "child", "out", "done", "in")?],
    )
}

fn repeat_revision(workflow: &str, child: &BlueprintRevision) -> TestResult<BlueprintRevision> {
    let mut repeat = Node::new(
        NodeId::new("repeat")?,
        NodeKind::Repeat {
            config: RepeatConfig::new(
                PinnedSubworkflow::new(
                    child.semantic().workflow().clone(),
                    child.id().clone(),
                    child.semantic().interface().clone(),
                ),
                Condition::Constant { value: true },
                2,
                RepeatBudget {
                    max_duration_ms: None,
                    max_cost_micros: None,
                    max_cost_currency: None,
                },
                RepeatTermination::SucceedWithLatest,
            )?,
        },
    )?
    .with_control_output(PortId::new("out")?)?;
    for (field, declaration) in child.semantic().interface().outputs() {
        repeat = repeat.with_data_output(
            PortId::new(field.as_str())?,
            DataPort::output(declaration.schema().clone()),
        )?;
    }
    revision(
        workflow,
        vec![repeat, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("repeat-done", "repeat", "out", "done", "in")?],
    )
}

fn approval_repeat_revision(
    workflow: &str,
    child: &BlueprintRevision,
) -> TestResult<BlueprintRevision> {
    let repeat = Node::new(
        NodeId::new("repeat")?,
        NodeKind::Repeat {
            config: RepeatConfig::new(
                PinnedSubworkflow::new(
                    child.semantic().workflow().clone(),
                    child.id().clone(),
                    child.semantic().interface().clone(),
                ),
                Condition::Constant { value: true },
                1,
                RepeatBudget {
                    max_duration_ms: None,
                    max_cost_micros: None,
                    max_cost_currency: None,
                },
                RepeatTermination::AwaitApproval,
            )?,
        },
    )?
    .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![repeat, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("repeat-done", "repeat", "out", "done", "in")?],
    )
}

fn revised_wait_revision(
    base: &BlueprintRevision,
    duration_ms: u64,
) -> TestResult<BlueprintRevision> {
    let wait = Node::new(NodeId::new("wait")?, NodeKind::Wait { duration_ms })?
        .with_control_output(PortId::new("out")?)?;
    Ok(base.revise(
        base.id(),
        MutationBatch::new(vec![Mutation::ReplaceNode { node: wait }])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "change the prospective wait definition",
    )?)
}

#[test]
fn precreated_run_artifact_is_charged_once_across_initial_input_and_later_reuse() -> TestResult {
    let harness = Harness::new("artifact-accounting")?;
    let revision = artifact_reuse_revision("workflow-artifact-accounting")?;
    let run = RunId::new("run-artifact-accounting")?;
    let bytes = b"precreated-run-artifact";
    let artifact = publish_artifact_for_run(&harness, &run, "precreated-run", bytes)?;
    let artifact_bytes = u64::try_from(bytes.len())?;
    assert_eq!(
        harness.store.workspace_usage(&run)?,
        WorkspaceUsage::new(0, 0, 1, artifact_bytes)
    );

    let invocation_reference = InvocationArtifactReference::new(
        artifact.artifact().as_str(),
        artifact.digest().to_hex(),
        Some("application/octet-stream".to_owned()),
        Some(artifact_bytes),
    )?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "result".to_owned(),
                reference: invocation_reference,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    harness.put_revision(&revision)?;
    let root_scope =
        WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-artifact-accounting")?);
    let initial = WorkspaceValueEntry::initial(
        root_scope.reference().clone(),
        ValueKey::new("initial-artifact")?,
        WorkspaceValue::Artifact(artifact),
    );
    assert_eq!(
        harness.command(
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope,
                workspace_budget: generous_budget()?,
                inputs: vec![initial],
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.store.workspace_usage(&run)?,
        WorkspaceUsage::new(1, 0, 1, artifact_bytes)
    );

    assert_eq!(
        harness.command(&run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    assert_eq!(harness.drive(&run, 8)?, 1);
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(
        harness.store.workspace_usage(&run)?,
        WorkspaceUsage::new(2, 0, 1, artifact_bytes),
        "reusing the same artifact may charge a value version but not artifact counters"
    );
    Ok(())
}

#[test]
fn crash_after_durable_lease_recovers_only_after_expiry_and_retries_once() -> TestResult {
    let directory = TempDir::new()?;
    let revision = task_revision("workflow-crash-after-lease")?;
    let run = RunId::new("run-crash-after-lease")?;

    {
        let store = Arc::new(RedbStore::open(directory.path())?);
        store.put_revision(&revision)?;
        let executor = Arc::new(PanickingExecutor {
            resolver: DeterministicExecutor::new(test_descriptor()?),
        });
        let runtime = recovery_service(
            store.clone(),
            Arc::new(ManualClock::new(NOW)),
            executor,
            "crash-after-lease",
        )?;
        let root_scope =
            WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-crash-after-lease")?);
        let create = runtime.command(
            run.clone(),
            ActorRef::new("human:structured-runtime-test")?,
            store.head(&run)?,
            Reason::new("create crash-boundary run")?,
            Vec::new(),
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope,
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?;
        assert_eq!(
            runtime.handle_command(&create)?.result().disposition(),
            CommandDisposition::Accepted
        );
        let start = runtime.command(
            run.clone(),
            ActorRef::new("human:structured-runtime-test")?,
            store.head(&run)?,
            Reason::new("start crash-boundary run")?,
            Vec::new(),
            RunCommand::StartRun,
        )?;
        runtime.handle_command(&start)?;

        let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.tick()));
        assert!(
            crash.is_err(),
            "panicking executor did not simulate a crash"
        );
        let stranded = runtime.projection(&run)?;
        assert_eq!(stranded.attempts().len(), 1);
        assert_eq!(
            stranded
                .attempts()
                .values()
                .next()
                .map(|attempt| attempt.state()),
            Some(&AttemptState::Leased)
        );
        assert_eq!(stranded.leases().len(), 1);
        assert!(stranded.leases().values().all(|lease| lease.is_active()));
        let history = runtime.history(&run)?;
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::NodeScheduled { .. }))
                .count(),
            1
        );
        assert!(
            !history
                .iter()
                .any(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
        );
    }

    {
        let store = Arc::new(RedbStore::open(directory.path())?);
        let runtime = recovery_service(
            store,
            Arc::new(ManualClock::new(NOW + 50)),
            Arc::new(DeterministicExecutor::new(test_descriptor()?)),
            "recover-before-expiry",
        )?;
        let recovery = runtime.recover()?;
        assert_eq!(recovery.runs_examined, 1);
        assert_eq!(recovery.expired_leases, 0);
        assert_eq!(recovery.retryable, 0);
        let preserved = runtime.projection(&run)?;
        assert_eq!(preserved.attempts().len(), 1);
        assert_eq!(
            preserved
                .attempts()
                .values()
                .next()
                .and_then(|attempt| attempt.recovery().last())
                .map(|observation| observation.classification()),
            Some(RecoveryClassification::LeaseStillValid)
        );
        assert!(preserved.leases().values().all(|lease| lease.is_active()));
        let tick = runtime.tick()?;
        assert_eq!(tick.dispatched, 0);
        assert_eq!(tick.completed, 0);
        assert_eq!(runtime.projection(&run)?.attempts().len(), 1);
    }

    {
        let store = Arc::new(RedbStore::open(directory.path())?);
        let recovery_clock = Arc::new(ManualClock::new(NOW + 101));
        let runtime = recovery_service(
            store,
            recovery_clock.clone(),
            Arc::new(DeterministicExecutor::new(test_descriptor()?)),
            "recover-after-expiry",
        )?;
        let recovery = runtime.recover()?;
        assert_eq!(recovery.runs_examined, 1);
        assert_eq!(recovery.expired_leases, 1);
        assert_eq!(recovery.retryable, 1);
        let retry_pending = runtime.projection(&run)?;
        assert_eq!(retry_pending.attempts().len(), 2);
        assert!(retry_pending.leases().values().any(|lease| matches!(
            lease.state(),
            LeaseState::Expired(RecoveryClassification::Retryable)
        )));
        assert!(
            retry_pending
                .attempts()
                .values()
                .any(|attempt| attempt.state() == &AttemptState::AwaitingRetryTimer)
        );

        let before_backoff = runtime.tick()?;
        assert_eq!(before_backoff.dispatched, 0);
        assert_eq!(before_backoff.completed, 0);
        let _ = recovery_clock.advance(1)?;
        let tick = runtime.tick()?;
        assert_eq!(tick.dispatched, 1);
        assert_eq!(tick.completed, 1);
        let completed = runtime.projection(&run)?;
        assert_eq!(
            completed.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Succeeded)
        );
        assert_eq!(completed.attempts().len(), 2);
        let attempt_numbers: BTreeSet<_> = completed
            .attempts()
            .values()
            .map(|attempt| attempt.attempt_number())
            .collect();
        assert_eq!(attempt_numbers, BTreeSet::from([1, 2]));
        let history = runtime.history(&run)?;
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::NodeScheduled { .. }))
                .count(),
            2
        );
        assert_eq!(
            history
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
                .count(),
            1,
            "only the post-recovery retry may receive a start acknowledgement"
        );
    }
    Ok(())
}

#[test]
fn branch_freezes_exactly_one_route_and_never_creates_the_other_execution() -> TestResult {
    let harness = Harness::new("branch")?;
    let revision = branch_revision("workflow-branch")?;
    let run = RunId::new("run-branch")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(projection.branch_routes().len(), 1);
    assert_eq!(
        projection.branch_routes().values().next(),
        Some(&PortId::new("true")?)
    );
    assert_eq!(
        projection
            .executions_for_node(&NodeId::new("selected")?)
            .count(),
        1
    );
    assert_eq!(
        projection
            .executions_for_node(&NodeId::new("unselected")?)
            .count(),
        0
    );
    Ok(())
}

#[test]
fn all_join_preserves_independent_success_and_failure_branch_truth() -> TestResult {
    let harness = Harness::new("fork-all")?;
    harness.executor.set_script(
        OperationId::new("model.fail")?,
        vec![InvocationEventKind::Terminal {
            terminal: failed_terminal()?,
        }],
    )?;
    let revision = fork_revision("workflow-fork-all", JoinPolicy::All, "model.fail")?;
    let run = RunId::new("run-fork-all")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.drive(&run, 8)?, 2);

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(projection.branches().len(), 2);
    assert!(
        projection
            .branches()
            .values()
            .any(|branch| { branch.state() == BranchState::Completed(RunOutcome::Succeeded) })
    );
    assert!(
        projection
            .branches()
            .values()
            .any(|branch| branch.state() == BranchState::Completed(RunOutcome::Failed))
    );
    let join = projection
        .joins()
        .values()
        .next()
        .ok_or("join did not complete")?;
    assert_eq!(join.branches().len(), 2);
    assert!(join.retained_branches().is_empty());
    Ok(())
}

#[test]
fn any_join_records_and_cancels_its_unfinished_loser_without_dispatch() -> TestResult {
    let harness = Harness::new("fork-any")?;
    let revision = fork_revision("workflow-fork-any", JoinPolicy::Any, "model.generate")?;
    let run = RunId::new("run-fork-any")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    assert_eq!(harness.drive(&run, 4)?, 1);
    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(projection.joins().len(), 1);
    assert!(
        projection
            .joins()
            .values()
            .all(|join| join.retained_branches().is_empty())
    );
    assert!(
        projection
            .branches()
            .values()
            .any(|branch| { branch.state() == BranchState::Completed(RunOutcome::Succeeded) })
    );
    assert!(
        projection
            .branches()
            .values()
            .any(|branch| { branch.state() == BranchState::Completed(RunOutcome::Cancelled) })
    );
    let history = harness.runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::BranchCancellationRequested { .. }
    )));
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeExecutionCancelledBeforeDispatch { .. }
    )));
    Ok(())
}

#[test]
fn first_success_and_quorum_cancel_losers_without_dispatching_them() -> TestResult {
    for (suffix, policy) in [
        ("first", JoinPolicy::FirstSuccess),
        ("quorum", JoinPolicy::Quorum(1)),
    ] {
        let harness = Harness::new(&format!("fork-{suffix}"))?;
        let revision = fork_revision(&format!("workflow-fork-{suffix}"), policy, "model.generate")?;
        let run = RunId::new(format!("run-fork-{suffix}"))?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        assert_eq!(harness.drive(&run, 8)?, 1, "{suffix} dispatched a loser");

        let projection = harness.runtime.projection(&run)?;
        assert!(
            projection.is_completed(),
            "{suffix} did not drain the loser"
        );
        assert_eq!(projection.branches().len(), 2);
        assert!(
            projection
                .branches()
                .values()
                .any(|branch| { branch.state() == BranchState::Completed(RunOutcome::Succeeded) })
        );
        assert!(
            projection
                .branches()
                .values()
                .any(|branch| { branch.state() == BranchState::Completed(RunOutcome::Cancelled) })
        );
        assert!(
            projection
                .joins()
                .values()
                .all(|join| join.retained_branches().is_empty())
        );
    }
    Ok(())
}

#[test]
fn impossible_first_success_and_quorum_fail_deterministically_instead_of_deadlocking() -> TestResult
{
    for (suffix, policy, fail_first) in [
        ("first-impossible", JoinPolicy::FirstSuccess, true),
        ("quorum-impossible", JoinPolicy::Quorum(2), false),
    ] {
        let harness = Harness::new(&format!("fork-{suffix}"))?;
        harness.executor.set_script(
            OperationId::new("model.fail")?,
            vec![InvocationEventKind::Terminal {
                terminal: failed_terminal()?,
            }],
        )?;
        if fail_first {
            harness.executor.set_script(
                OperationId::new("model.generate")?,
                vec![InvocationEventKind::Terminal {
                    terminal: failed_terminal()?,
                }],
            )?;
        }
        let revision = fork_revision(&format!("workflow-{suffix}"), policy, "model.fail")?;
        let run = RunId::new(format!("run-{suffix}"))?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        assert_eq!(harness.drive(&run, 8)?, 2);

        let projection = harness.runtime.projection(&run)?;
        assert_eq!(
            projection.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Failed)
        );
        let join_id = NodeId::new("join")?;
        let join = projection
            .executions_for_node(&join_id)
            .next()
            .ok_or("impossible join execution was not created")?;
        assert_eq!(
            join.state(),
            &NodeExecutionState::Terminal(milkdrift_persistence::NodeOutcome::Failed)
        );
    }
    Ok(())
}

#[test]
fn collect_and_first_reducers_publish_deterministic_workspace_outputs() -> TestResult {
    for (suffix, strategy) in [
        ("collect", ReducerStrategy::Collect),
        ("first", ReducerStrategy::First),
    ] {
        let harness = Harness::new(&format!("reducer-{suffix}"))?;
        install_output_scripts(&harness)?;
        let revision = reducer_revision(&format!("workflow-reducer-{suffix}"), strategy.clone())?;
        let run = RunId::new(format!("run-reducer-{suffix}"))?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        assert_eq!(harness.drive(&run, 8)?, 2);

        let projection = harness.runtime.projection(&run)?;
        assert!(projection.is_completed());
        let root_scope = projection
            .root_scope()
            .ok_or("reducer run has no root scope")?
            .reference();
        let mut sibling_output_scopes = BTreeSet::new();
        for task_id in [NodeId::new("a-task")?, NodeId::new("b-task")?] {
            let task_execution = projection
                .executions_for_node(&task_id)
                .next()
                .ok_or("branch task execution was not created")?;
            assert_ne!(task_execution.scope(), root_scope);
            assert!(matches!(
                projection
                    .scopes()
                    .get(task_execution.scope())
                    .ok_or("branch task scope was not projected")?
                    .kind(),
                ScopeKind::Branch { .. }
            ));
            assert_eq!(task_execution.outputs().len(), 1);
            assert_eq!(
                task_execution.outputs()[0].value().scope(),
                task_execution.scope()
            );
            assert!(sibling_output_scopes.insert(task_execution.scope().clone()));
        }
        assert_eq!(sibling_output_scopes.len(), 2);
        let reducer_id = NodeId::new("reduce")?;
        let execution = projection
            .executions_for_node(&reducer_id)
            .next()
            .ok_or("reducer execution was not created")?;
        assert_eq!(execution.scope(), root_scope);
        assert_eq!(execution.outputs().len(), 1);
        assert_eq!(execution.outputs()[0].value().scope(), root_scope);
        let output = harness
            .store
            .value(execution.outputs()[0].value())?
            .ok_or("reducer output is absent from workspace storage")?;
        match strategy {
            ReducerStrategy::Collect => {
                let values = output
                    .value()
                    .as_json()
                    .and_then(|value| value.value().as_array())
                    .ok_or("collect output is not a structured array")?;
                assert_eq!(values.len(), 2);
            }
            ReducerStrategy::First => {
                let mut branch_outputs: Vec<_> = projection
                    .branches()
                    .values()
                    .flat_map(|branch| branch.outputs().iter().cloned())
                    .collect();
                branch_outputs.sort();
                let expected = harness
                    .store
                    .value(branch_outputs.first().ok_or("branches had no outputs")?)?
                    .ok_or("first branch value is absent")?;
                assert_eq!(output.value(), expected.value());
            }
            ReducerStrategy::Capability(_) => unreachable!("fixture uses deterministic reducers"),
        }
    }
    Ok(())
}

#[test]
fn durable_timer_wait_fires_only_at_its_recorded_deadline() -> TestResult {
    let harness = Harness::new("timer")?;
    let revision = wait_revision("workflow-timer", 100)?;
    let run = RunId::new("run-timer")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    let initial = harness.runtime.projection(&run)?;
    assert_eq!(initial.timers().len(), 1);
    assert!(initial.timers().values().all(|timer| timer.is_pending()));
    assert!(initial.waits().values().all(|wait| wait.is_pending()));
    assert_eq!(harness.runtime.tick()?.dispatched, 0);
    harness.clock.advance(99)?;
    assert_eq!(harness.runtime.tick()?.dispatched, 0);
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Running
    );

    harness.clock.advance(1)?;
    harness.runtime.tick()?;
    let completed = harness.runtime.projection(&run)?;
    assert!(completed.is_completed());
    assert!(
        completed
            .timers()
            .values()
            .all(|timer| timer.is_completed())
    );
    assert!(completed.waits().values().all(|wait| wait.is_completed()));
    Ok(())
}

#[test]
fn typed_signal_is_consumed_once_and_duplicate_delivery_is_a_durable_fact() -> TestResult {
    let harness = Harness::new("signal")?;
    let revision = signal_revision("workflow-signal")?;
    let run = RunId::new("run-signal")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    let signal = SignalId::new("signal-ready-1")?;
    let delivery = || -> TestResult<RunCommand> {
        Ok(RunCommand::DeliverSignal {
            signal: signal.clone(),
            signal_type: SignalTypeId::new("notify.ready")?,
            correlation: None,
            mode: SignalDeliveryMode::OneShot,
            payload: BoundedJson::new(json!({"ready": true}))?,
        })
    };
    assert_eq!(
        harness.command(&run, delivery()?)?,
        CommandDisposition::Accepted
    );
    let after_first = harness.runtime.projection(&run)?;
    let signal_view = after_first
        .signals()
        .get(&signal)
        .ok_or("signal was not projected")?;
    assert_eq!(signal_view.consumed_by().len(), 1);
    assert!(signal_view.duplicate_commands().is_empty());
    assert_eq!(after_first.lifecycle(), RunLifecycle::Running);

    assert_eq!(
        harness.command(&run, delivery()?)?,
        CommandDisposition::Accepted
    );
    let after_duplicate = harness.runtime.projection(&run)?;
    let signal_view = after_duplicate
        .signals()
        .get(&signal)
        .ok_or("deduplicated signal disappeared")?;
    assert_eq!(signal_view.consumed_by().len(), 1);
    assert_eq!(signal_view.duplicate_commands().len(), 1);

    harness.clock.advance(50)?;
    harness.runtime.tick()?;
    assert!(harness.runtime.projection(&run)?.is_completed());
    Ok(())
}

#[test]
fn attached_subworkflow_materializes_starts_and_links_a_terminal_child_run() -> TestResult {
    let harness = Harness::new("subworkflow")?;
    install_child_output_script(&harness)?;
    let child = output_child_revision("workflow-child")?;
    let parent = subworkflow_revision("workflow-parent", &child)?;
    let run = RunId::new("run-parent")?;
    harness.put_revision(&child)?;
    harness.put_revision(&parent)?;
    harness.create_and_start(&run, &parent)?;

    assert_eq!(harness.runtime.projection(&run)?.subworkflows().len(), 1);
    harness.drive(&run, 8)?;
    let projection = harness.runtime.projection(&run)?;
    assert!(projection.is_completed());
    let link = projection
        .subworkflows()
        .values()
        .next()
        .ok_or("parent has no child link")?;
    assert_eq!(link.child_revision(), child.id());
    assert_eq!(
        link.state(),
        SubworkflowState::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(link.imports().len(), 1);
    let imported = &link.imports()[0];
    assert_eq!(imported.child_value().scope().run(), link.child_run());
    assert_eq!(imported.parent_value().scope().run(), &run);
    let parent_entry = harness
        .store
        .value(imported.parent_value())?
        .ok_or("imported child output is absent from the parent workspace")?;
    match parent_entry.origin() {
        ValueOrigin::Imported { source } => assert_eq!(source, imported.child_value()),
        origin => return Err(format!("expected imported value origin, found {origin:?}").into()),
    }
    assert!(
        harness
            .runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::SubworkflowOutputImported { .. }))
    );
    let child_projection = harness.runtime.projection(link.child_run())?;
    assert_eq!(child_projection.revision(), Some(child.id()));
    assert_eq!(
        child_projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    Ok(())
}

#[test]
fn repeat_runs_each_pinned_child_in_an_isolated_scope_and_stops_at_the_bound() -> TestResult {
    let harness = Harness::new("repeat")?;
    install_child_output_script(&harness)?;
    let child = output_child_revision("workflow-repeat-child")?;
    let parent = repeat_revision("workflow-repeat-parent", &child)?;
    let run = RunId::new("run-repeat-parent")?;
    harness.put_revision(&child)?;
    harness.put_revision(&parent)?;
    harness.create_and_start(&run, &parent)?;
    harness.drive(&run, 12)?;

    let projection = harness.runtime.projection(&run)?;
    assert!(projection.is_completed());
    assert_eq!(projection.iterations().len(), 2);
    assert!(
        projection
            .iterations()
            .values()
            .all(|iteration| iteration.is_completed())
    );
    let iteration_scopes: BTreeSet<_> = projection
        .iterations()
        .values()
        .map(|iteration| iteration.scope().reference().clone())
        .collect();
    assert_eq!(iteration_scopes.len(), 2);
    let iteration_parents: BTreeSet<_> = projection
        .iterations()
        .values()
        .map(|iteration| {
            assert!(matches!(
                iteration.scope().kind(),
                ScopeKind::Iteration { .. }
            ));
            iteration.scope().parent().cloned()
        })
        .collect();
    assert_eq!(iteration_parents.len(), 1, "iterations are not siblings");
    let termination = projection
        .repeat_terminations()
        .values()
        .next()
        .ok_or("repeat did not record a terminal bound")?;
    assert_eq!(
        termination.termination(),
        RepeatTerminationReason::MaximumIterations
    );
    assert_eq!(projection.subworkflows().len(), 2);
    let mut imported_parent_values = BTreeSet::new();
    let mut imported_child_values = BTreeSet::new();
    for child_link in projection.subworkflows().values() {
        assert_eq!(
            child_link.state(),
            SubworkflowState::Terminal(RunOutcome::Succeeded)
        );
        let owner = child_link
            .scope()
            .parent()
            .ok_or("repeat child scope has no iteration parent")?;
        assert!(iteration_scopes.contains(owner));
        assert_eq!(child_link.imports().len(), 1);
        let imported = &child_link.imports()[0];
        assert_eq!(
            imported.parent_value().scope(),
            child_link.scope().reference()
        );
        assert_eq!(imported.child_value().scope().run(), child_link.child_run());
        assert!(imported_parent_values.insert(imported.parent_value().clone()));
        assert!(imported_child_values.insert(imported.child_value().clone()));
        let entry = harness
            .store
            .value(imported.parent_value())?
            .ok_or("repeat child import is absent")?;
        assert!(matches!(
            entry.origin(),
            ValueOrigin::Imported { source } if source == imported.child_value()
        ));
        assert!(
            harness
                .runtime
                .projection(child_link.child_run())?
                .is_completed()
        );
    }
    assert_eq!(imported_parent_values.len(), 2);
    assert_eq!(imported_child_values.len(), 2);
    Ok(())
}

#[test]
fn await_approval_repeat_extends_exactly_once_then_rejection_terminates() -> TestResult {
    let harness = Harness::new("repeat-approval")?;
    let child = task_revision("workflow-repeat-approval-child")?;
    let parent = approval_repeat_revision("workflow-repeat-approval-parent", &child)?;
    let run = RunId::new("run-repeat-approval")?;
    harness.put_revision(&child)?;
    harness.put_revision(&parent)?;
    harness.create_and_start(&run, &parent)?;
    harness.drive(&run, 16)?;

    let boundary = harness.runtime.projection(&run)?;
    assert_eq!(boundary.lifecycle(), RunLifecycle::Running);
    assert_eq!(boundary.iterations().len(), 1);
    assert!(boundary.repeat_terminations().is_empty());
    assert_eq!(
        boundary
            .iterations()
            .values()
            .next()
            .map(|iteration| iteration.state()),
        Some(IterationState::ConditionRecorded(true))
    );
    let repeat_execution = boundary
        .executions_for_node(&NodeId::new("repeat")?)
        .next()
        .ok_or("await-approval repeat execution was not created")?
        .execution()
        .clone();
    let continuation = boundary
        .repeat_continuations()
        .get(&repeat_execution)
        .ok_or("await-approval repeat continuation request was not recorded")?;
    assert!(continuation.is_pending_approval());
    assert_eq!(
        continuation
            .pending_request()
            .map(|request| request.cause()),
        Some(&RepeatContinuationCause::IterationLimit)
    );

    let approval_id = RepeatDecisionId::new("repeat-approval-plus-two")?;
    let approval = harness.runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        harness.store.head(&run)?,
        Reason::new("authorize exactly two more repeat iterations")?,
        Vec::new(),
        RunCommand::DecideRepeatContinuation {
            repeat_execution: repeat_execution.clone(),
            decision: approval_id.clone(),
            outcome: RepeatContinuationDecision::Approved,
            approved_additional_iterations: Some(2),
        },
    )?;
    let approved = harness.runtime.handle_command(&approval)?;
    assert_eq!(
        approved.result().disposition(),
        CommandDisposition::Accepted
    );
    assert!(!approved.replayed());
    let approved_head = harness.store.head(&run)?;
    let replayed = harness.runtime.handle_command(&approval)?;
    assert!(replayed.replayed());
    assert_eq!(replayed.result(), approved.result());
    assert_eq!(harness.store.head(&run)?, approved_head);

    assert_eq!(
        harness.command(
            &run,
            RunCommand::DecideRepeatContinuation {
                repeat_execution: repeat_execution.clone(),
                decision: approval_id,
                outcome: RepeatContinuationDecision::Approved,
                approved_additional_iterations: Some(2),
            },
        )?,
        CommandDisposition::Rejected,
        "a new command cannot reuse a durable repeat decision identity"
    );
    let after_duplicate = harness.runtime.projection(&run)?;
    let continuation = after_duplicate
        .repeat_continuations()
        .get(&repeat_execution)
        .ok_or("approval did not create continuation authority")?;
    assert_eq!(continuation.initial_iteration_limit(), 1);
    assert_eq!(continuation.effective_iteration_limit(), 3);
    assert_eq!(continuation.decisions().len(), 1);

    harness.drive(&run, 32)?;
    let next_boundary = harness.runtime.projection(&run)?;
    assert_eq!(next_boundary.lifecycle(), RunLifecycle::Running);
    assert_eq!(next_boundary.iterations().len(), 3);
    assert_eq!(next_boundary.subworkflows().len(), 3);
    assert!(next_boundary.repeat_terminations().is_empty());
    let continuation = next_boundary
        .repeat_continuations()
        .get(&repeat_execution)
        .ok_or("repeat lost its continuation authority")?;
    assert!(continuation.is_pending_approval());
    assert_eq!(continuation.effective_iteration_limit(), 3);
    assert_eq!(continuation.decisions().len(), 1);

    assert_eq!(
        harness.command(
            &run,
            RunCommand::DecideRepeatContinuation {
                repeat_execution: repeat_execution.clone(),
                decision: RepeatDecisionId::new("repeat-approval-reject")?,
                outcome: RepeatContinuationDecision::Rejected,
                approved_additional_iterations: None,
            },
        )?,
        CommandDisposition::Accepted
    );
    harness.drive(&run, 8)?;
    let rejected = harness.runtime.projection(&run)?;
    assert_eq!(
        rejected.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert_eq!(rejected.iterations().len(), 3);
    assert_eq!(rejected.subworkflows().len(), 3);
    let continuation = rejected
        .repeat_continuations()
        .get(&repeat_execution)
        .ok_or("repeat lost the rejected continuation fact")?;
    assert!(continuation.is_rejected());
    assert_eq!(continuation.decisions().len(), 2);
    assert_eq!(
        rejected
            .repeat_terminations()
            .get(&repeat_execution)
            .ok_or("rejected repeat has no deterministic termination")?
            .termination(),
        RepeatTerminationReason::MaximumIterations
    );
    Ok(())
}

#[test]
fn explicit_terminal_waits_for_an_already_dispatched_any_join_loser() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new(
        store.clone(),
        executor.clone(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("terminal-deferral", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-terminal-deferral")?,
            ActorRef::new("controller:terminal-deferral")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?);
    let revision = fork_revision("workflow-terminal-deferral", JoinPolicy::Any, "model.fail")?;
    let run = RunId::new("run-terminal-deferral")?;
    store.put_revision(&revision)?;
    let create = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("create terminal deferral run")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(
                run.clone(),
                ScopeId::new("scope-terminal-deferral")?,
            ),
            workspace_budget: generous_budget()?,
            inputs: Vec::new(),
        },
    )?;
    runtime.handle_command(&create)?;
    let start = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("start terminal deferral run")?,
        Vec::new(),
        RunCommand::StartRun,
    )?;
    runtime.handle_command(&start)?;

    let first_runtime = runtime.clone();
    let first_tick =
        std::thread::spawn(move || first_runtime.tick().map_err(|error| error.to_string()));
    executor.wait_until_entered()?;
    let second_tick = runtime.tick()?;
    assert_eq!(second_tick.dispatched, 1);

    let midway = runtime.projection(&run)?;
    assert_eq!(midway.lifecycle(), RunLifecycle::Running);
    assert_eq!(midway.joins().len(), 1);
    let done_id = NodeId::new("done")?;
    assert_eq!(
        midway
            .executions_for_node(&done_id)
            .next()
            .map(|execution| execution.state()),
        Some(&NodeExecutionState::Terminal(
            milkdrift_persistence::NodeOutcome::Succeeded
        ))
    );
    assert!(
        midway
            .attempts()
            .values()
            .any(|attempt| attempt.is_active())
    );
    assert!(
        !runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::RunTerminal { .. }))
    );

    runtime.tick()?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.projection(&run)?.lifecycle(), RunLifecycle::Running);
    executor.release()?;
    first_tick
        .join()
        .map_err(|_| "first scheduler thread panicked")?
        .map_err(|error| format!("first scheduler tick failed: {error}"))?;

    assert_eq!(
        runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert!(
        runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::RunTerminal { .. }))
    );
    Ok(())
}

#[test]
fn active_invocation_cancellation_reaches_the_executor_and_is_acknowledged_durably() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let config = RuntimeConfig::new(
        WorkerId::new("worker-active-cancel")?,
        ActorRef::new("controller:active-cancel")?,
        30_000,
        32,
        SchedulerLimits::new(8, 4, 2, 4)?,
        RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
    )?;
    let runtime = Arc::new(RuntimeService::new(
        store.clone(),
        executor.clone(),
        clock,
        Arc::new(SequentialIdGenerator::new("active-cancel", 1)?),
        config,
    )?);
    let revision = task_revision("workflow-active-cancel")?;
    let run = RunId::new("run-active-cancel")?;
    store.put_revision(&revision)?;

    let create = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("create active cancellation run")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(),
            root_scope: WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-active-cancel")?),
            workspace_budget: generous_budget()?,
            inputs: Vec::new(),
        },
    )?;
    assert_eq!(
        runtime.handle_command(&create)?.result().disposition(),
        CommandDisposition::Accepted
    );
    let start = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("start active cancellation run")?,
        Vec::new(),
        RunCommand::StartRun,
    )?;
    runtime.handle_command(&start)?;

    let tick_runtime = runtime.clone();
    let dispatch =
        std::thread::spawn(move || tick_runtime.tick().map_err(|error| error.to_string()));
    executor.wait_until_entered()?;
    let cancel = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("cancel an invocation after durable dispatch")?,
        Vec::new(),
        RunCommand::RequestCancellation,
    )?;
    assert_eq!(
        runtime.handle_command(&cancel)?.result().disposition(),
        CommandDisposition::Accepted
    );

    runtime.tick()?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    executor.release()?;
    dispatch
        .join()
        .map_err(|_| "dispatch thread panicked")?
        .map_err(|error| format!("dispatch failed: {error}"))?;

    let projection = runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Cancelled)
    );
    let history = runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeExecutionCancellationRequested { .. }
    )));
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::InvocationCancellationAcknowledged { .. }
    )));
    Ok(())
}

#[test]
fn shutdown_closes_admission_and_cancellation_explicitly_drains_wait_ownership() -> TestResult {
    let harness = Harness::new("cancel")?;
    let revision = wait_revision("workflow-cancel", 60_000)?;
    let run = RunId::new("run-cancel")?;
    harness.put_revision(&revision)?;
    harness.create(&run, &revision)?;

    harness.runtime.begin_shutdown();
    assert!(!harness.runtime.is_accepting_admission());
    assert_eq!(harness.runtime.tick()?.deferred, 1);
    assert_eq!(
        harness.command(&run, RunCommand::StartRun)?,
        CommandDisposition::Rejected
    );
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Created
    );

    harness.runtime.resume_admission();
    assert_eq!(
        harness.command(&run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(&run, RunCommand::RequestCancellation)?,
        CommandDisposition::Accepted
    );
    harness.drive(&run, 4)?;

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Cancelled)
    );
    assert!(
        projection
            .timers()
            .values()
            .all(|timer| !timer.is_pending())
    );
    assert!(projection.waits().values().all(|wait| !wait.is_pending()));
    let history = harness.runtime.history(&run)?;
    assert!(
        history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::TimerCancelled { .. }))
    );
    assert!(
        history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::WaitCancelled { .. }))
    );
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeExecutionCancelledBeforeDispatch { .. }
    )));
    Ok(())
}

#[test]
fn prospective_revision_adoption_is_persisted_actionable_and_stale_safe() -> TestResult {
    let harness = Harness::new("adoption")?;
    let old = wait_revision("workflow-adoption", 5_000)?;
    let new = revised_wait_revision(&old, 7_500)?;
    harness.put_revision(&old)?;
    harness.put_revision(&new)?;

    let adopted_run = RunId::new("run-adoption-applied")?;
    harness.create_and_start(&adopted_run, &old)?;
    assert_eq!(
        harness.command(
            &adopted_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-applied")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let planned = harness.runtime.projection(&adopted_run)?;
    let plan = planned
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("adoption plan was not persisted")?;
    assert!(plan.items().iter().any(|item| {
        item.node.as_ref() == NodeId::new("wait").ok().as_ref()
            && item.classification == ReconciliationClassification::ChangedPending
            && item.action == ReconciliationAction::UseNewOnNextInvocation
    }));
    let plan_id = plan.plan().clone();
    assert_eq!(
        harness.command(
            &adopted_run,
            RunCommand::DecideReconciliation {
                plan: plan_id.clone(),
                decision: ReconciliationDecisionId::new("decision-approve-adoption")?,
                outcome: AuthorityDecision::Approve,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(
            &adopted_run,
            RunCommand::ApplyReconciliation {
                plan: plan_id.clone(),
            },
        )?,
        CommandDisposition::Accepted
    );
    let applied = harness.runtime.projection(&adopted_run)?;
    assert_eq!(applied.revision(), Some(new.id()));
    assert!(
        applied
            .reconciliation()
            .plans()
            .get(&plan_id)
            .is_some_and(|plan| plan.applied_sequence().is_some())
    );

    let stale_run = RunId::new("run-adoption-stale")?;
    harness.create_and_start(&stale_run, &old)?;
    assert_eq!(
        harness.command(
            &stale_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-stale")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let stale_plan = harness
        .runtime
        .projection(&stale_run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("stale test plan was not persisted")?
        .plan()
        .clone();
    assert_eq!(
        harness.command(&stale_run, RunCommand::PauseRun)?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(
            &stale_run,
            RunCommand::ApplyReconciliation {
                plan: stale_plan.clone(),
            },
        )?,
        CommandDisposition::Rejected
    );
    let stale = harness.runtime.projection(&stale_run)?;
    assert_eq!(stale.revision(), Some(old.id()));
    assert!(
        stale
            .reconciliation()
            .plans()
            .get(&stale_plan)
            .is_some_and(|plan| plan.stale_sequence().is_some())
    );
    Ok(())
}
