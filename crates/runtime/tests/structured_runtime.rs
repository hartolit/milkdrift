//! Black-box structured-runtime evidence using the production redb store.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        Arc, Barrier, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use milkdrift_blueprint::{
    AuthorRef, BindingSource, BlueprintRevision, BranchConfig, Comparison, Condition,
    ConditionOperand, DataPort, Edge, EdgeId, EdgeKind, FieldId, ForkConfig, InterfaceField,
    JoinConfig, JoinPolicy, Mutation, MutationBatch, Node, NodeId, NodeKind, PathSegment,
    PathSelector, PinnedSubworkflow, PortId, ReducerConfig, ReducerStrategy, RepeatBudget,
    RepeatConfig, RepeatTermination, SchemaRef, TerminalOutcome, WorkflowId, WorkflowInterface,
};
use milkdrift_capability::{
    ArtifactReference as InvocationArtifactReference, BoundedJson, CancellationAcknowledgement,
    CancellationRequest, CapabilityDescriptor, CapabilityDescriptorDocument, CapabilityRequirement,
    ErrorClass, InvocationEvent, InvocationEventKind, InvocationFailure, InvocationTerminal,
    InvocationValueReference, OperationId, SchemaId, SideEffectClass, TerminalStatus,
};
use milkdrift_persistence::{
    ActorRef, ArtifactPublicationId, ArtifactStore, AtomicRunCommitRequest, AuthorityDecision,
    BeginArtifactPublication, CommandDisposition, CommandId, CommandReceipt, CommandResultDocument,
    EventId, IndexedRunState, MAX_INDEX_MUTATIONS_PER_COMMIT, NodeExecutionId, NodeExecutionMode,
    NodeOutcome, PageSize, Reason, ReconciliationAction, ReconciliationClassification,
    ReconciliationDecisionId, ReconciliationId, ReconciliationPolicy, RecoveryClassification,
    RepeatContinuationCause, RepeatContinuationDecision, RepeatDecisionId, RepeatTerminationReason,
    RevisionStore, RunEventEnvelope, RunEventKind, RunIndexUpdate, RunJournal, RunOutcome,
    RunQueryStore, RunSummaryIndex, SignalDeliveryMode, SignalId, SignalTypeId, TimestampMillis, WorkerId,
    WorkspaceAccounting, WorkspaceStore,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    AttemptState, BranchState, DeterministicExecutor, ExecutionDispatch, ExecutionReportBatch,
    ExecutorError, IdGenerator, IterationState, LeaseState, ManualClock, NodeExecutionState,
    ResolvedCapability, RetryPolicy, RunCommand, RunLifecycle, RuntimeConfig, RuntimeError,
    RuntimeService, SchedulerLimits, SequentialIdGenerator, SubworkflowState, TaskExecutor,
    WorkerReport,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactRetention, ArtifactSensitivity,
    CausalId, CausalReference, ContentDigest, MediaType, RunId, ScopeId, ScopeKind, ScopeReference,
    ValueKey, ValueOrigin, WorkspaceBudget, WorkspaceScope, WorkspaceUsage, WorkspaceValue,
    WorkspaceValueEntry, WorkspaceValueReference,
};
use redb::{Database, TableDefinition};
use serde_json::json;
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const NOW: u64 = 10_000;
const RAW_SCOPES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.workspace.scopes");
const RAW_VALUES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.workspace.values");

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

struct BoundaryFailingExecutor {
    resolver: DeterministicExecutor,
    failures_remaining: AtomicUsize,
    dispatches: Mutex<Vec<ExecutionDispatch>>,
}

struct BoundaryThenBlockingExecutor {
    resolver: DeterministicExecutor,
    calls: AtomicUsize,
    entered: (Mutex<bool>, Condvar),
    released: (Mutex<bool>, Condvar),
    cancellation_requests: AtomicUsize,
}

struct AdmissionRaceExecutor {
    resolver: DeterministicExecutor,
    resolver_barrier: Barrier,
    barrier_enabled: AtomicBool,
    resolver_entries: (Mutex<usize>, Condvar),
    execute_entries: (Mutex<usize>, Condvar),
    released: (Mutex<bool>, Condvar),
}

impl AdmissionRaceExecutor {
    fn new(descriptor: CapabilityDescriptor) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            resolver_barrier: Barrier::new(2),
            barrier_enabled: AtomicBool::new(true),
            resolver_entries: (Mutex::new(0), Condvar::new()),
            execute_entries: (Mutex::new(0), Condvar::new()),
            released: (Mutex::new(false), Condvar::new()),
        }
    }

    fn wait_for_resolvers(&self, expected: usize) -> TestResult {
        wait_for_count(&self.resolver_entries, expected, "resolver admission")
    }

    fn wait_for_execute(&self, expected: usize) -> TestResult {
        wait_for_count(&self.execute_entries, expected, "executor dispatch")
    }

    fn release(&self) -> TestResult {
        let (lock, released) = &self.released;
        *lock.lock().map_err(|_| "admission release lock poisoned")? = true;
        released.notify_all();
        Ok(())
    }
}

fn wait_for_count(
    state: &(Mutex<usize>, Condvar),
    expected: usize,
    label: &str,
) -> TestResult {
    let (lock, ready) = state;
    let count = lock.lock().map_err(|_| format!("{label} lock poisoned"))?;
    let (count, timeout) = ready
        .wait_timeout_while(count, Duration::from_secs(5), |count| *count < expected)
        .map_err(|_| format!("{label} wait poisoned"))?;
    if timeout.timed_out() || *count < expected {
        return Err(format!("{label} did not reach {expected} before timeout").into());
    }
    Ok(())
}

impl TaskExecutor for AdmissionRaceExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
    ) -> Result<ResolvedCapability, ExecutorError> {
        let resolved = self.resolver.resolve(requirement)?;
        if self.barrier_enabled.load(Ordering::SeqCst) {
            {
                let (lock, entered) = &self.resolver_entries;
                let mut count = lock.lock().map_err(|_| {
                    ExecutorError::Boundary("resolver count lock poisoned".to_owned())
                })?;
                *count = count.saturating_add(1);
                entered.notify_all();
            }
            if self.resolver_barrier.wait().is_leader() {
                self.barrier_enabled.store(false, Ordering::SeqCst);
            }
        }
        Ok(resolved)
    }

    fn execute(&self, dispatch: &ExecutionDispatch) -> Result<ExecutionReportBatch, ExecutorError> {
        {
            let (lock, entered) = &self.execute_entries;
            let mut count = lock
                .lock()
                .map_err(|_| ExecutorError::Boundary("execute count lock poisoned".to_owned()))?;
            *count = count.saturating_add(1);
            entered.notify_all();
        }
        let (lock, released) = &self.released;
        let mut permit = lock
            .lock()
            .map_err(|_| ExecutorError::Boundary("admission release lock poisoned".to_owned()))?;
        while !*permit {
            permit = released.wait(permit).map_err(|_| {
                ExecutorError::Boundary("admission release wait poisoned".to_owned())
            })?;
        }
        let terminal = InvocationTerminal::new(
            TerminalStatus::Success,
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
        self.resolver.cancel(request)
    }
}

struct TransientAttemptIdGenerator {
    inner: SequentialIdGenerator,
    attempt_calls: AtomicUsize,
    fail_on_attempt_call: usize,
}

impl TransientAttemptIdGenerator {
    fn new(prefix: &str, fail_on_attempt_call: usize) -> TestResult<Self> {
        Ok(Self {
            inner: SequentialIdGenerator::new(prefix, 1)?,
            attempt_calls: AtomicUsize::new(0),
            fail_on_attempt_call,
        })
    }
}

impl IdGenerator for TransientAttemptIdGenerator {
    fn next(&self, kind: &'static str) -> Result<String, RuntimeError> {
        if kind == "attempt"
            && self.attempt_calls.fetch_add(1, Ordering::SeqCst) == self.fail_on_attempt_call
        {
            return Err(RuntimeError::InvalidTransition(
                "scripted transient retry attempt identity failure".to_owned(),
            ));
        }
        self.inner.next(kind)
    }
}

impl BoundaryThenBlockingExecutor {
    fn new(descriptor: CapabilityDescriptor) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            calls: AtomicUsize::new(0),
            entered: (Mutex::new(false), Condvar::new()),
            released: (Mutex::new(false), Condvar::new()),
            cancellation_requests: AtomicUsize::new(0),
        }
    }

    fn wait_until_entered(&self) -> TestResult {
        let (lock, ready) = &self.entered;
        let entered = lock.lock().map_err(|_| "entered lock poisoned")?;
        let (entered, timeout) = ready
            .wait_timeout_while(entered, Duration::from_secs(5), |entered| !*entered)
            .map_err(|_| "entered wait poisoned")?;
        if timeout.timed_out() || !*entered {
            return Err("retry dispatch was not observed before timeout".into());
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

impl TaskExecutor for BoundaryThenBlockingExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement)
    }

    fn execute(&self, dispatch: &ExecutionDispatch) -> Result<ExecutionReportBatch, ExecutorError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(ExecutorError::Boundary(
                "executor disconnected after accepting first dispatch".to_owned(),
            ));
        }
        {
            let (lock, entered) = &self.entered;
            *lock
                .lock()
                .map_err(|_| ExecutorError::Boundary("entered lock poisoned".to_owned()))? = true;
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
        let terminal = InvocationTerminal::new(
            TerminalStatus::Cancelled,
            Vec::new(),
            None,
            None,
            dispatch
                .resolution()
                .snapshot()
                .operation_contract()
                .side_effect(),
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
            Some("retry executor observed cancellation intent".to_owned()),
        )?)
    }
}

impl BoundaryFailingExecutor {
    fn new(descriptor: CapabilityDescriptor, failures: usize) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            failures_remaining: AtomicUsize::new(failures),
            dispatches: Mutex::new(Vec::new()),
        }
    }

    fn dispatches(&self) -> TestResult<Vec<ExecutionDispatch>> {
        Ok(self
            .dispatches
            .lock()
            .map_err(|_| "dispatch log lock poisoned")?
            .clone())
    }

    fn set_script(
        &self,
        operation: OperationId,
        events: Vec<InvocationEventKind>,
    ) -> Result<(), ExecutorError> {
        self.resolver.set_script(operation, events)
    }
}

impl TaskExecutor for BoundaryFailingExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement)
    }

    fn execute(&self, dispatch: &ExecutionDispatch) -> Result<ExecutionReportBatch, ExecutorError> {
        self.dispatches
            .lock()
            .map_err(|_| ExecutorError::Boundary("dispatch log lock poisoned".to_owned()))?
            .push(dispatch.clone());
        if self
            .failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(ExecutorError::Boundary(
                "executor disconnected after accepting dispatch".to_owned(),
            ));
        }
        self.resolver.execute(dispatch)
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.resolver.cancel(request)
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
        Self::with_retry_policy(prefix, RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?)
    }

    fn with_retry_policy(prefix: &str, retry_policy: RetryPolicy) -> TestResult<Self> {
        Self::with_descriptor(prefix, retry_policy, test_descriptor()?)
    }

    fn with_descriptor(
        prefix: &str,
        retry_policy: RetryPolicy,
        descriptor: CapabilityDescriptor,
    ) -> TestResult<Self> {
        let directory = TempDir::new()?;
        let store = Arc::new(RedbStore::open(directory.path())?);
        let clock = Arc::new(ManualClock::new(NOW));
        let executor = Arc::new(DeterministicExecutor::new(descriptor));
        let config = RuntimeConfig::new(
            WorkerId::new(format!("worker-{prefix}"))?,
            ActorRef::new(format!("controller:{prefix}"))?,
            30_000,
            64,
            SchedulerLimits::new(64, 32, 16, 32)?,
            retry_policy,
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

    fn close(self) -> TempDir {
        let Self {
            _directory,
            store,
            clock,
            executor,
            runtime,
        } = self;
        drop(runtime);
        drop(executor);
        drop(clock);
        drop(store);
        _directory
    }
}

fn runtime_at(
    root: &Path,
    prefix: &str,
    now: u64,
    maximum_tick_items: u16,
) -> TestResult<(Arc<RedbStore>, Arc<ManualClock>, RuntimeService)> {
    runtime_with_executor_at(
        root,
        prefix,
        prefix,
        now,
        maximum_tick_items,
        Arc::new(DeterministicExecutor::new(test_descriptor()?)),
    )
}

fn runtime_with_executor_at(
    root: &Path,
    id_prefix: &str,
    identity_prefix: &str,
    now: u64,
    maximum_tick_items: u16,
    executor: Arc<dyn TaskExecutor>,
) -> TestResult<(Arc<RedbStore>, Arc<ManualClock>, RuntimeService)> {
    let store = Arc::new(RedbStore::open(root)?);
    let clock = Arc::new(ManualClock::new(now));
    let runtime = RuntimeService::new(
        store.clone(),
        executor,
        clock.clone(),
        Arc::new(SequentialIdGenerator::new(id_prefix, 1)?),
        RuntimeConfig::new(
            WorkerId::new(format!("worker-{identity_prefix}"))?,
            ActorRef::new(format!("controller:{identity_prefix}"))?,
            30_000,
            maximum_tick_items,
            SchedulerLimits::new(64, 32, 16, 32)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?;
    Ok((store, clock, runtime))
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

fn submit_worker_report(
    runtime: &RuntimeService,
    store: &RedbStore,
    run: &RunId,
    identity_prefix: &str,
    report: WorkerReport,
) -> TestResult<CommandDisposition> {
    let document = runtime.command(
        run.clone(),
        ActorRef::new(format!("controller:{identity_prefix}"))?,
        store.head(run)?,
        Reason::new("focused hostile worker report")?,
        Vec::new(),
        RunCommand::WorkerReport {
            worker: WorkerId::new(format!("worker-{identity_prefix}"))?,
            report,
        },
    )?;
    Ok(runtime.handle_command(&document)?.result().disposition())
}

fn raw_push_component(encoded: &mut Vec<u8>, value: &str) -> TestResult {
    encoded.extend_from_slice(&u32::try_from(value.len())?.to_be_bytes());
    encoded.extend_from_slice(value.as_bytes());
    Ok(())
}

fn raw_scope_key(reference: &ScopeReference) -> TestResult<Vec<u8>> {
    let mut encoded = Vec::new();
    raw_push_component(&mut encoded, reference.run().as_str())?;
    raw_push_component(&mut encoded, reference.scope().as_str())?;
    Ok(encoded)
}

fn raw_value_key(reference: &WorkspaceValueReference) -> TestResult<Vec<u8>> {
    let mut encoded = raw_scope_key(reference.scope())?;
    raw_push_component(&mut encoded, reference.key().as_str())?;
    encoded.extend_from_slice(&reference.version().get().to_be_bytes());
    Ok(encoded)
}

fn delete_raw_row(
    root: &Path,
    definition: TableDefinition<'static, &'static [u8], &'static [u8]>,
    key: &[u8],
) -> TestResult {
    let database = Database::create(root.join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    let removed = {
        let mut table = write.open_table(definition)?;
        table.remove(key)?.is_some()
    };
    if !removed {
        return Err("raw corruption fixture could not find its target row".into());
    }
    write.commit()?;
    Ok(())
}

fn insert_raw_workspace_value(root: &Path, entry: &WorkspaceValueEntry) -> TestResult {
    const FAMILY: &str = "workspace value";
    const DOMAIN: &[u8] = b"milkdrift.redb.internal-document.v1\0";
    #[derive(serde::Serialize)]
    struct Envelope<'a> {
        schema_version: u32,
        family: &'static str,
        checksum: String,
        payload: &'a WorkspaceValueEntry,
    }

    let payload = serde_json::to_vec(entry)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN);
    hasher.update(&(FAMILY.len() as u64).to_be_bytes());
    hasher.update(FAMILY.as_bytes());
    hasher.update(&(payload.len() as u64).to_be_bytes());
    hasher.update(&payload);
    let encoded = serde_json::to_vec(&Envelope {
        schema_version: 1,
        family: FAMILY,
        checksum: hasher.finalize().to_hex().to_string(),
        payload: entry,
    })?;
    let key = raw_value_key(entry.reference())?;
    let database = Database::create(root.join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    let replaced = {
        let mut table = write.open_table(RAW_VALUES)?;
        table.insert(key.as_slice(), encoded.as_slice())?.is_some()
    };
    if replaced {
        return Err("raw orphan fixture unexpectedly replaced an existing row".into());
    }
    write.commit()?;
    Ok(())
}

fn assert_integrity_error(error: &RuntimeError) {
    assert!(
        matches!(
            error,
            RuntimeError::InvalidHistory(_) | RuntimeError::Persistence(_)
        ),
        "expected a durable integrity error, got {error}"
    );
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
    descriptor_with_model_side_effect("none")
}

fn descriptor_with_model_side_effect(side_effect: &str) -> TestResult<CapabilityDescriptor> {
    let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    let operations = value
        .get_mut("descriptor")
        .and_then(|descriptor| descriptor.get_mut("operations"))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("descriptor fixture has no operations object")?;
    operations
        .get_mut("model.generate")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("descriptor fixture has no model.generate operation")?
        .insert(
            "side_effect".to_owned(),
            serde_json::Value::String(side_effect.to_owned()),
        );
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

fn optional_unselected_edge_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let selected = PortId::new("selected")?;
    let unselected = PortId::new("unselected")?;
    let branch = Node::new(
        NodeId::new("route")?,
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(selected.clone(), Condition::Constant { value: true })]),
                Some(unselected.clone()),
            )?,
        },
    )?
    .with_control_output(selected)?
    .with_control_output(unselected)?;
    let consume = task("consume", "model.generate")?.with_data_input(
        PortId::new("optional")?,
        DataPort::input(schema.clone(), false, None)?,
    )?;
    let produce = task("produce", "model.generate")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema))?;
    revision(
        workflow,
        vec![
            branch,
            consume,
            produce,
            terminal("done", TerminalOutcome::Success)?,
            terminal("unused", TerminalOutcome::Failure)?,
        ],
        vec![
            control_edge("route-consume", "route", "selected", "consume", "in")?,
            control_edge("route-produce", "route", "unselected", "produce", "in")?,
            control_edge("consume-done", "consume", "out", "done", "in")?,
            control_edge("produce-unused", "produce", "out", "unused", "in")?,
            data_edge("optional-item", "produce", "item", "consume", "optional")?,
        ],
    )
}

fn fork_revision(
    workflow: &str,
    policy: JoinPolicy,
    second_operation: &str,
) -> TestResult<BlueprintRevision> {
    fork_revision_with_terminal(workflow, policy, second_operation, TerminalOutcome::Success)
}

fn fork_revision_with_terminal(
    workflow: &str,
    policy: JoinPolicy,
    second_operation: &str,
    terminal_outcome: TerminalOutcome,
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
            terminal("done", terminal_outcome)?,
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

fn nested_fork_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let outer_a = PortId::new("a")?;
    let outer_b = PortId::new("b")?;
    let inner_left = PortId::new("left")?;
    let inner_right = PortId::new("right")?;
    let outer_fork = Node::new(
        NodeId::new("outer-fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(BTreeSet::from([outer_a.clone(), outer_b.clone()]))?,
        },
    )?
    .with_control_output(outer_a)?
    .with_control_output(outer_b)?;
    let inner_fork = Node::new(
        NodeId::new("inner-fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(BTreeSet::from([
                inner_left.clone(),
                inner_right.clone(),
            ]))?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(inner_left)?
    .with_control_output(inner_right)?;
    let inner_left_task = task("inner-left", "model.generate")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let inner_right_task = task("inner-right", "model.fail")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let inner_join = Node::new(
        NodeId::new("inner-join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("inner-fork")?, JoinPolicy::All),
        },
    )?
    .with_control_input(PortId::new("left-in")?)?
    .with_control_input(PortId::new("right-in")?)?
    .with_control_output(PortId::new("out")?)?;
    let outer_a_tail = task("outer-a-tail", "model.generate")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let outer_b_task = task("outer-b-task", "model.fail")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let outer_join = Node::new(
        NodeId::new("outer-join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("outer-fork")?, JoinPolicy::All),
        },
    )?
    .with_control_input(PortId::new("a-in")?)?
    .with_control_input(PortId::new("b-in")?)?
    .with_control_output(PortId::new("out")?)?
    .with_data_input(
        PortId::new("tail-item")?,
        DataPort::input(schema, false, None)?,
    )?;
    revision(
        workflow,
        vec![
            outer_fork,
            inner_fork,
            inner_left_task,
            inner_right_task,
            inner_join,
            outer_a_tail,
            outer_b_task,
            outer_join,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("outer-fork-a", "outer-fork", "a", "inner-fork", "in")?,
            control_edge(
                "outer-fork-b",
                "outer-fork",
                "b",
                "outer-b-task",
                "in",
            )?,
            control_edge(
                "inner-fork-left",
                "inner-fork",
                "left",
                "inner-left",
                "in",
            )?,
            control_edge(
                "inner-fork-right",
                "inner-fork",
                "right",
                "inner-right",
                "in",
            )?,
            control_edge(
                "inner-left-join",
                "inner-left",
                "out",
                "inner-join",
                "left-in",
            )?,
            control_edge(
                "inner-right-join",
                "inner-right",
                "out",
                "inner-join",
                "right-in",
            )?,
            control_edge(
                "inner-join-tail",
                "inner-join",
                "out",
                "outer-a-tail",
                "in",
            )?,
            control_edge(
                "outer-a-join",
                "outer-a-tail",
                "out",
                "outer-join",
                "a-in",
            )?,
            control_edge(
                "outer-b-join",
                "outer-b-task",
                "out",
                "outer-join",
                "b-in",
            )?,
            data_edge(
                "outer-tail-data",
                "outer-a-tail",
                "item",
                "outer-join",
                "tail-item",
            )?,
            control_edge("outer-join-done", "outer-join", "out", "done", "in")?,
        ],
    )
}

fn direct_terminal_fork_revision(
    workflow: &str,
    a_outcome: TerminalOutcome,
    b_outcome: TerminalOutcome,
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
    revision(
        workflow,
        vec![
            fork,
            terminal("a-terminal", a_outcome)?,
            terminal("b-terminal", b_outcome)?,
        ],
        vec![
            control_edge("fork-a-terminal", "fork", "a", "a-terminal", "in")?,
            control_edge("fork-b-terminal", "fork", "b", "b-terminal", "in")?,
        ],
    )
}

fn fork_revision_with_post_join_task(workflow: &str) -> TestResult<BlueprintRevision> {
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
            config: JoinConfig::new(NodeId::new("fork")?, JoinPolicy::Any),
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
            task("b-task", "model.fail")?,
            join,
            task("independent", "model.generate")?,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("fork-a", "fork", "a", "a-task", "in")?,
            control_edge("fork-b", "fork", "b", "b-task", "in")?,
            control_edge("a-join", "a-task", "out", "join", "a-in")?,
            control_edge("b-join", "b-task", "out", "join", "b-in")?,
            control_edge("join-independent", "join", "out", "independent", "in")?,
            control_edge("independent-done", "independent", "out", "done", "in")?,
        ],
    )
}

fn revision_without_post_join_task(base: &BlueprintRevision) -> TestResult<BlueprintRevision> {
    Ok(base.revise(
        base.id(),
        MutationBatch::new(vec![
            Mutation::RemoveEdge {
                edge: EdgeId::new("join-independent")?,
            },
            Mutation::RemoveEdge {
                edge: EdgeId::new("independent-done")?,
            },
            Mutation::RemoveNode {
                node: NodeId::new("independent")?,
            },
            Mutation::AddEdge {
                edge: control_edge("join-done", "join", "out", "done", "in")?,
            },
        ])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "remove an actually unstarted post-join task",
    )?)
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
    publish_artifact_in_store(
        harness.store.as_ref(),
        &RunId::new(format!("artifact-publisher-{suffix}"))?,
        suffix,
        bytes,
    )
}

fn publish_artifact_in_store(
    store: &RedbStore,
    owner: &RunId,
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
        owner.clone(),
        metadata.clone(),
        generous_budget()?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&request)?;
    store.write_chunk(&publication, 0, bytes)?;
    store.commit_publication(&publication)?;
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
    let a = publish_artifact(harness, "z-branch-a", b"artifact-a")?;
    let b = publish_artifact(harness, "a-branch-b", b"artifact-b")?;
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

fn install_non_idempotent_success_script(harness: &Harness) -> TestResult {
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![InvocationEventKind::Terminal {
            terminal: InvocationTerminal::new(
                TerminalStatus::Success,
                Vec::new(),
                None,
                None,
                SideEffectClass::NonIdempotentWrite,
            )?,
        }],
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

fn broadcast_fanout_revision(
    workflow: &str,
    outputs_per_wait: usize,
) -> TestResult<BlueprintRevision> {
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
    let mut left = Node::new(
        NodeId::new("left-wait")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.broadcast")?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?;
    let mut right = Node::new(
        NodeId::new("right-wait")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.broadcast")?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?;
    for index in 0..outputs_per_wait {
        let port = PortId::new(format!("payload-{index:03}"))?;
        left = left.with_data_output(port.clone(), DataPort::output(schema.clone()))?;
        right = right.with_data_output(port, DataPort::output(schema.clone()))?;
    }
    let join = Node::new(
        NodeId::new("join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("fork")?, JoinPolicy::All),
        },
    )?
    .with_control_input(PortId::new("a-in")?)?
    .with_control_input(PortId::new("b-in")?)?
    .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![
            fork,
            left,
            right,
            join,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("fork-left", "fork", "a", "left-wait", "in")?,
            control_edge("fork-right", "fork", "b", "right-wait", "in")?,
            control_edge("left-join", "left-wait", "out", "join", "a-in")?,
            control_edge("right-join", "right-wait", "out", "join", "b-in")?,
            control_edge("join-done", "join", "out", "done", "in")?,
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

fn optional_workflow_input_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let field = FieldId::new("optional")?;
    let consume = task("consume", "model.generate")?.with_data_input(
        PortId::new("optional")?,
        DataPort::input(
            schema.clone(),
            false,
            Some(BindingSource::WorkflowInput {
                field: field.clone(),
            }),
        )?,
    )?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new([(field, InterfaceField::optional(schema))], [])?,
        vec![consume, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge(
            "consume-done",
            "consume",
            "out",
            "done",
            "in",
        )?],
    )
}

fn producer_consumer_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let produce = task("produce", "model.generate")?
        .with_data_output(PortId::new("result")?, DataPort::output(schema.clone()))?;
    let consume = task("consume", "model.fail")?
        .with_data_input(PortId::new("input")?, DataPort::input(schema, true, None)?)?;
    revision(
        workflow,
        vec![
            produce,
            consume,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("produce-consume", "produce", "out", "consume", "in")?,
            control_edge("consume-done", "consume", "out", "done", "in")?,
            data_edge("result-input", "produce", "result", "consume", "input")?,
        ],
    )
}

fn removable_task_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let signal = Node::new(
        NodeId::new("signal")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.ready")?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![
            task("retired", "model.generate")?,
            signal,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("retired-signal", "retired", "out", "signal", "in")?,
            control_edge("signal-done", "signal", "out", "done", "in")?,
        ],
    )
}

fn revision_without_completed_task(base: &BlueprintRevision) -> TestResult<BlueprintRevision> {
    Ok(base.revise(
        base.id(),
        MutationBatch::new(vec![
            Mutation::RemoveEdge {
                edge: EdgeId::new("retired-signal")?,
            },
            Mutation::RemoveNode {
                node: NodeId::new("retired")?,
            },
        ])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "remove completed work without reinterpreting its history",
    )?)
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

fn direct_artifact_input_revision(
    workflow: &str,
    artifact: &milkdrift_workspace::ArtifactReference,
) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let artifact_binding = BindingSource::Artifact {
        reference: serde_json::to_string(artifact)?,
        contract: schema.clone(),
    };
    let optional_binding = BindingSource::WorkflowInput {
        field: FieldId::new("optional")?,
    };
    let task = task("consume", "model.generate")?
        .with_data_input(
            PortId::new("artifact")?,
            DataPort::input(schema.clone(), true, Some(artifact_binding))?,
        )?
        .with_data_input(
            PortId::new("optional")?,
            DataPort::input(schema.clone(), false, Some(optional_binding))?,
        )?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new(
            [(FieldId::new("optional")?, InterfaceField::optional(schema))],
            [],
        )?,
        vec![task, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge(
            "consume-done",
            "consume",
            "out",
            "done",
            "in",
        )?],
    )
}

fn terminal_binding_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let terminal = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_data_input(
        PortId::new("pass")?,
        DataPort::input(
            schema.clone(),
            true,
            Some(BindingSource::WorkflowInput {
                field: FieldId::new("source")?,
            }),
        )?,
    )?
    .with_data_input(
        PortId::new("literal")?,
        DataPort::input(
            schema.clone(),
            true,
            Some(BindingSource::Literal {
                value: BoundedJson::new(json!({"materialized": true}))?,
            }),
        )?,
    )?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new(
            [(
                FieldId::new("source")?,
                InterfaceField::required(schema.clone()),
            )],
            [
                (
                    FieldId::new("pass")?,
                    InterfaceField::required(schema.clone()),
                ),
                (FieldId::new("literal")?, InterfaceField::required(schema)),
            ],
        )?,
        vec![terminal],
        Vec::new(),
    )
}

fn missing_optional_condition_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let source = BindingSource::WorkflowInput {
        field: FieldId::new("maybe")?,
    };
    let present = PortId::new("present")?;
    let missing = PortId::new("missing")?;
    let branch = Node::new(
        NodeId::new("route")?,
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(
                    present.clone(),
                    Condition::Exists {
                        source: source.clone(),
                    },
                )]),
                Some(missing.clone()),
            )?,
        },
    )?
    .with_control_output(present)?
    .with_control_output(missing)?
    .with_data_input(
        PortId::new("maybe")?,
        DataPort::input(schema.clone(), false, Some(source))?,
    )?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new(
            [(FieldId::new("maybe")?, InterfaceField::optional(schema))],
            [],
        )?,
        vec![
            branch,
            terminal("unexpected", TerminalOutcome::Failure)?,
            terminal("expected", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("present-route", "route", "present", "unexpected", "in")?,
            control_edge("missing-route", "route", "missing", "expected", "in")?,
        ],
    )
}

fn multi_path_condition_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let signal = Node::new(
        NodeId::new("signal")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.payload")?,
        },
    )?
    .with_control_output(PortId::new("out")?)?
    .with_data_output(PortId::new("payload")?, DataPort::output(schema.clone()))?;
    let left = BindingSource::NodeOutput {
        node: NodeId::new("signal")?,
        port: PortId::new("payload")?,
        path: PathSelector::new(vec![PathSegment::Field(FieldId::new("left")?)])?,
    };
    let right = BindingSource::NodeOutput {
        node: NodeId::new("signal")?,
        port: PortId::new("payload")?,
        path: PathSelector::new(vec![PathSegment::Field(FieldId::new("right")?)])?,
    };
    let expected = PortId::new("expected")?;
    let unexpected = PortId::new("unexpected")?;
    let branch = Node::new(
        NodeId::new("route")?,
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(
                    expected.clone(),
                    Condition::All {
                        conditions: vec![
                            Condition::Compare {
                                left: ConditionOperand::Binding {
                                    source: left.clone(),
                                },
                                comparison: Comparison::Equal,
                                right: ConditionOperand::Literal {
                                    value: BoundedJson::new(json!(1))?,
                                },
                            },
                            Condition::Compare {
                                left: ConditionOperand::Binding {
                                    source: right.clone(),
                                },
                                comparison: Comparison::Equal,
                                right: ConditionOperand::Literal {
                                    value: BoundedJson::new(json!(2))?,
                                },
                            },
                        ],
                    },
                )]),
                Some(unexpected.clone()),
            )?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(expected)?
    .with_control_output(unexpected)?
    .with_data_input(
        PortId::new("left")?,
        DataPort::input(schema.clone(), true, Some(left))?,
    )?
    .with_data_input(
        PortId::new("right")?,
        DataPort::input(schema, true, Some(right))?,
    )?;
    revision(
        workflow,
        vec![
            signal,
            branch,
            terminal("good", TerminalOutcome::Success)?,
            terminal("bad", TerminalOutcome::Failure)?,
        ],
        vec![
            control_edge("signal-route", "signal", "out", "route", "in")?,
            data_edge("signal-left", "signal", "payload", "route", "left")?,
            data_edge("signal-right", "signal", "payload", "route", "right")?,
            control_edge("route-good", "route", "expected", "good", "in")?,
            control_edge("route-bad", "route", "unexpected", "bad", "in")?,
        ],
    )
}

fn long_deterministic_chain_revision(
    workflow: &str,
    branch_count: usize,
) -> TestResult<BlueprintRevision> {
    let next = PortId::new("next")?;
    let mut nodes = Vec::with_capacity(branch_count.saturating_add(1));
    let mut edges = Vec::with_capacity(branch_count);
    for index in 0..branch_count {
        let id = format!("step-{index:04}");
        let mut node = Node::new(
            NodeId::new(id.clone())?,
            NodeKind::Branch {
                config: BranchConfig::new(
                    BTreeMap::from([(next.clone(), Condition::Constant { value: true })]),
                    None,
                )?,
            },
        )?
        .with_control_output(next.clone())?;
        if index > 0 {
            node = node.with_control_input(PortId::new("in")?)?;
            edges.push(control_edge(
                &format!("edge-{index:04}"),
                &format!("step-{:04}", index - 1),
                "next",
                &id,
                "in",
            )?);
        }
        nodes.push(node);
    }
    nodes.push(terminal("done", TerminalOutcome::Success)?);
    edges.push(control_edge(
        "edge-terminal",
        &format!("step-{:04}", branch_count - 1),
        "next",
        "done",
        "in",
    )?);
    revision(workflow, nodes, edges)
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

fn revision_without_entry_node(
    base: &BlueprintRevision,
    node: &str,
    incident_edges: &[&str],
) -> TestResult<BlueprintRevision> {
    let mut mutations = incident_edges
        .iter()
        .map(|edge| {
            Ok(Mutation::RemoveEdge {
                edge: EdgeId::new((*edge).to_owned())?,
            })
        })
        .collect::<TestResult<Vec<_>>>()?;
    mutations.push(Mutation::RemoveNode {
        node: NodeId::new(node.to_owned())?,
    });
    Ok(base.revise(
        base.id(),
        MutationBatch::new(mutations)?,
        AuthorRef::new("human:structured-runtime-test")?,
        "remove a prospective entry without rewriting its runtime-owned history",
    )?)
}

fn revision_with_added_root_wait(
    base: &BlueprintRevision,
    duration_ms: u64,
) -> TestResult<BlueprintRevision> {
    let prior_duration = match base
        .semantic()
        .nodes()
        .get(&NodeId::new("wait")?)
        .map(Node::kind)
    {
        Some(NodeKind::Wait { duration_ms }) => *duration_ms,
        _ => return Err("base revision has no wait node".into()),
    };
    let node = Node::new(NodeId::new("added-root")?, NodeKind::Wait { duration_ms })?
        .with_control_output(PortId::new("out")?)?;
    let wait = Node::new(
        NodeId::new("wait")?,
        NodeKind::Wait {
            duration_ms: prior_duration,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?;
    Ok(base.revise(
        base.id(),
        MutationBatch::new(vec![
            Mutation::AddNode { node },
            Mutation::ReplaceNode { node: wait },
            Mutation::AddEdge {
                edge: control_edge("added-root-wait", "added-root", "out", "wait", "in")?,
            },
        ])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "add an independent root entry node",
    )?)
}

fn revised_task_revision(
    base: &BlueprintRevision,
    operation: &str,
) -> TestResult<BlueprintRevision> {
    Ok(base.revise(
        base.id(),
        MutationBatch::new(vec![Mutation::ReplaceNode {
            node: task("work", operation)?,
        }])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "change an active task's prospective capability",
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
        assert!(
            preserved
                .attempts()
                .values()
                .next()
                .is_some_and(|attempt| attempt.recovery().is_empty()),
            "healthy unexpired leases must not grow recovery history"
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
        let first = completed
            .attempts()
            .values()
            .find(|attempt| attempt.attempt_number() == 1)
            .ok_or("first crash-boundary attempt is absent")?;
        let second = completed
            .attempts()
            .values()
            .find(|attempt| attempt.attempt_number() == 2)
            .ok_or("recovery retry attempt is absent")?;
        assert_eq!(
            first.state(),
            &AttemptState::UncertainSupersededByRetry {
                covering_attempt: second.attempt().clone(),
            }
        );
        assert!(first.terminal().is_none());
        assert!(first.obligation().is_some());
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
        assert!(!history.iter().any(|event| matches!(
            event.kind(),
            RunEventKind::NodeTerminal {
                attempt,
                outcome: NodeOutcome::Failed,
                ..
            } if attempt == first.attempt()
        )));
    }
    Ok(())
}

#[test]
fn idempotent_boundary_error_retries_exact_request_and_keeps_first_attempt_truthful() -> TestResult
{
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BoundaryFailingExecutor::new(
        descriptor_with_model_side_effect("idempotent_write")?,
        1,
    ));
    let runtime = RuntimeService::new(
        store.clone(),
        executor.clone(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("idempotent-boundary-retry", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-idempotent-boundary-retry")?,
            ActorRef::new("controller:idempotent-boundary-retry")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(2, vec![ErrorClass::Adapter], 1, 1_000, 0)?,
        )?,
    )?;
    let revision = task_revision("workflow-idempotent-boundary-retry")?;
    let run = RunId::new("run-idempotent-boundary-retry")?;
    store.put_revision(&revision)?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-idempotent-boundary-retry")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );

    let first_tick = runtime.tick()?;
    assert_eq!(first_tick.dispatched, 1);
    assert_eq!(first_tick.completed, 0);
    assert_eq!(first_tick.uncertain, 1);
    let pending = runtime.projection(&run)?;
    assert_eq!(pending.unresolved_attempts().count(), 1);
    assert!(
        pending
            .attempts()
            .values()
            .all(|attempt| attempt.terminal().is_none())
    );
    assert!(
        runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { .. }))
    );
    assert!(!runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeTerminal {
            outcome: NodeOutcome::Failed,
            ..
        }
    )));

    clock.advance(1)?;
    let retry_tick = runtime.tick()?;
    assert_eq!(retry_tick.dispatched, 1);
    assert_eq!(retry_tick.completed, 1);
    let projection = runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    let first = projection
        .attempts()
        .values()
        .find(|attempt| attempt.attempt_number() == 1)
        .ok_or("first idempotent attempt is absent")?;
    let second = projection
        .attempts()
        .values()
        .find(|attempt| attempt.attempt_number() == 2)
        .ok_or("second idempotent attempt is absent")?;
    assert_eq!(
        first.state(),
        &AttemptState::UncertainSupersededByRetry {
            covering_attempt: second.attempt().clone(),
        }
    );
    assert!(first.terminal().is_none());
    assert!(first.obligation().is_some());
    assert_eq!(projection.unresolved_attempts().count(), 0);

    let dispatches = executor.dispatches()?;
    assert_eq!(dispatches.len(), 2);
    assert_eq!(
        dispatches[0].request().idempotency_key(),
        dispatches[1].request().idempotency_key()
    );
    assert!(dispatches[0].request().idempotency_key().is_some());
    assert_eq!(
        dispatches[0].resolution().snapshot(),
        dispatches[1].resolution().snapshot()
    );
    assert_eq!(
        dispatches[0].request().capability(),
        dispatches[1].request().capability()
    );
    assert_eq!(
        dispatches[0].request().operation(),
        dispatches[1].request().operation()
    );
    assert_eq!(
        dispatches[0].request().inputs(),
        dispatches[1].request().inputs()
    );
    assert_eq!(
        dispatches[0].request().extensions(),
        dispatches[1].request().extensions()
    );
    Ok(())
}

#[test]
fn uncertainty_survives_transient_retry_id_failure_and_recovery_retries_later() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BoundaryFailingExecutor::new(test_descriptor()?, 1));
    let runtime = RuntimeService::new(
        store.clone(),
        executor,
        clock.clone(),
        Arc::new(TransientAttemptIdGenerator::new("transient-retry-id", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-transient-retry-id")?,
            ActorRef::new("controller:transient-retry-id")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(
                2,
                vec![ErrorClass::Adapter, ErrorClass::Transport],
                1,
                1_000,
                0,
            )?,
        )?,
    )?;
    let revision = task_revision("workflow-transient-retry-id")?;
    let run = RunId::new("run-transient-retry-id")?;
    store.put_revision(&revision)?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-transient-retry-id")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );

    let first_tick = runtime.tick()?;
    assert_eq!(first_tick.uncertain, 1);
    let uncertain = runtime.projection(&run)?;
    assert_eq!(uncertain.unresolved_attempts().count(), 1);
    assert!(uncertain.retries().is_empty());
    let history = runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::ExternalOutcomeUncertain { .. }
    )));
    assert!(!history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeRetryScheduled { .. }
    )));

    let recovered = runtime.recover()?;
    assert_eq!(recovered.retryable, 1);
    let pending = runtime.projection(&run)?;
    assert_eq!(pending.retries().len(), 1);
    assert_eq!(pending.unresolved_attempts().count(), 1);
    clock.advance(1)?;
    assert_eq!(runtime.tick()?.completed, 1);
    let completed = runtime.projection(&run)?;
    assert_eq!(
        completed.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(completed.unresolved_attempts().count(), 0);
    Ok(())
}

#[test]
fn uncertainty_is_committed_when_retry_deadline_overflows() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(u64::MAX - 10));
    let executor = Arc::new(BoundaryFailingExecutor::new(test_descriptor()?, 1));
    let runtime = RuntimeService::new(
        store.clone(),
        executor,
        clock,
        Arc::new(SequentialIdGenerator::new("retry-time-overflow", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-retry-time-overflow")?,
            ActorRef::new("controller:retry-time-overflow")?,
            1,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(2, vec![ErrorClass::Adapter], 100, 100, 0)?,
        )?,
    )?;
    let revision = task_revision("workflow-retry-time-overflow")?;
    let run = RunId::new("run-retry-time-overflow")?;
    store.put_revision(&revision)?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-retry-time-overflow")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );

    assert_eq!(runtime.tick()?.uncertain, 1);
    let projection = runtime.projection(&run)?;
    assert_eq!(projection.unresolved_attempts().count(), 1);
    assert!(projection.retries().is_empty());
    let history = runtime.history(&run)?;
    assert!(history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::ExternalOutcomeUncertain { .. }
    )));
    assert!(!history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeRetryScheduled { .. }
    )));
    Ok(())
}

#[test]
fn concurrent_runtime_services_cannot_oversubscribe_one_global_lease_slot() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(AdmissionRaceExecutor::new(test_descriptor()?));
    let make_runtime = |suffix: &str| -> TestResult<Arc<RuntimeService>> {
        Ok(Arc::new(RuntimeService::new(
            store.clone(),
            executor.clone(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new(
                format!("cross-service-{suffix}"),
                1,
            )?),
            RuntimeConfig::new(
                WorkerId::new(format!("worker-cross-service-{suffix}"))?,
                ActorRef::new(format!("controller:cross-service-{suffix}"))?,
                30_000,
                1,
                SchedulerLimits::new(1, 1, 1, 1)?,
                RetryPolicy::new(1, Vec::new(), 1, 1_000, 0)?,
            )?,
        )?))
    };
    let first_runtime = make_runtime("first")?;
    let second_runtime = make_runtime("second")?;
    let revision = task_revision("workflow-cross-service-admission")?;
    store.put_revision(&revision)?;

    let first_run = RunId::new("run-z-cross-service")?;
    assert_eq!(
        submit_command(
            first_runtime.as_ref(),
            store.as_ref(),
            &first_run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    first_run.clone(),
                    ScopeId::new("scope-z-cross-service")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(
            first_runtime.as_ref(),
            store.as_ref(),
            &first_run,
            RunCommand::StartRun,
        )?,
        CommandDisposition::Accepted
    );
    let first_tick_runtime = first_runtime.clone();
    let first_tick = std::thread::spawn(move || {
        first_tick_runtime
            .tick()
            .map_err(|error| format!("first cross-service tick failed: {error}"))
    });
    executor.wait_for_resolvers(1)?;

    let second_run = RunId::new("run-a-cross-service")?;
    assert_eq!(
        submit_command(
            second_runtime.as_ref(),
            store.as_ref(),
            &second_run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    second_run.clone(),
                    ScopeId::new("scope-a-cross-service")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(
            second_runtime.as_ref(),
            store.as_ref(),
            &second_run,
            RunCommand::StartRun,
        )?,
        CommandDisposition::Accepted
    );
    let second_tick_runtime = second_runtime.clone();
    let second_tick = std::thread::spawn(move || {
        second_tick_runtime
            .tick()
            .map_err(|error| format!("second cross-service tick failed: {error}"))
    });
    executor.wait_for_resolvers(2)?;
    executor.wait_for_execute(1)?;
    assert_eq!(
        store.active_leases(PageSize::new(2)?)?.entries.len(),
        1,
        "two runtime services both committed against one global slot"
    );
    executor.release()?;
    let first_result = first_tick
        .join()
        .map_err(|_| "first cross-service scheduler thread panicked")??;
    let second_result = second_tick
        .join()
        .map_err(|_| "second cross-service scheduler thread panicked")??;
    assert_eq!(
        first_result.dispatched + second_result.dispatched,
        1,
        "stale admission witness allowed two dispatches"
    );
    assert_eq!(first_result.completed + second_result.completed, 1);
    assert_eq!(first_result.deferred + second_result.deferred, 1);
    let granted = first_runtime
        .history(&first_run)?
        .into_iter()
        .chain(second_runtime.history(&second_run)?)
        .filter(|event| matches!(event.kind(), RunEventKind::LeaseGranted { .. }))
        .count();
    assert_eq!(granted, 1);

    for _ in 0..4 {
        if first_runtime.projection(&first_run)?.is_completed()
            && first_runtime.projection(&second_run)?.is_completed()
        {
            break;
        }
        first_runtime.tick()?;
    }
    assert!(first_runtime.projection(&first_run)?.is_completed());
    assert!(first_runtime.projection(&second_run)?.is_completed());
    Ok(())
}

#[test]
fn harmless_uncertain_attempt_is_covered_by_exact_terminal_failure_retry() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BoundaryFailingExecutor::new(test_descriptor()?, 1));
    executor.set_script(
        OperationId::new("model.generate")?,
        vec![InvocationEventKind::Terminal {
            terminal: failed_terminal()?,
        }],
    )?;
    let runtime = RuntimeService::new(
        store.clone(),
        executor,
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("harmless-failure-retry", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-harmless-failure-retry")?,
            ActorRef::new("controller:harmless-failure-retry")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(2, vec![ErrorClass::Adapter], 1, 1_000, 0)?,
        )?,
    )?;
    let revision = task_revision("workflow-harmless-failure-retry")?;
    let run = RunId::new("run-harmless-failure-retry")?;
    store.put_revision(&revision)?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-harmless-failure-retry")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    runtime.tick()?;
    clock.advance(1)?;
    runtime.tick()?;
    let projection = runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    let first = projection
        .attempts()
        .values()
        .find(|attempt| attempt.attempt_number() == 1)
        .ok_or("harmless uncertain attempt is absent")?;
    let retry = projection
        .attempts()
        .values()
        .find(|attempt| attempt.attempt_number() == 2)
        .ok_or("terminal failure retry is absent")?;
    assert_eq!(
        first.state(),
        &AttemptState::UncertainSupersededByRetry {
            covering_attempt: retry.attempt().clone(),
        }
    );
    assert!(first.terminal().is_none());
    assert!(first.obligation().is_some());
    assert_eq!(retry.state(), &AttemptState::Terminal(NodeOutcome::Failed));
    assert_eq!(projection.unresolved_attempts().count(), 0);
    Ok(())
}

#[test]
fn exhausted_idempotent_boundary_retries_remain_uncertain_without_fabricated_failure() -> TestResult
{
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ManualClock::new(NOW));
    let executor = Arc::new(BoundaryFailingExecutor::new(
        descriptor_with_model_side_effect("idempotent_write")?,
        2,
    ));
    let runtime = RuntimeService::new(
        store.clone(),
        executor,
        clock.clone(),
        Arc::new(SequentialIdGenerator::new(
            "idempotent-boundary-exhausted",
            1,
        )?),
        RuntimeConfig::new(
            WorkerId::new("worker-idempotent-boundary-exhausted")?,
            ActorRef::new("controller:idempotent-boundary-exhausted")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(2, vec![ErrorClass::Adapter], 1, 1_000, 0)?,
        )?,
    )?;
    let revision = task_revision("workflow-idempotent-boundary-exhausted")?;
    let run = RunId::new("run-idempotent-boundary-exhausted")?;
    store.put_revision(&revision)?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-idempotent-boundary-exhausted")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    runtime.tick()?;
    clock.advance(1)?;
    runtime.tick()?;
    let projection = runtime.projection(&run)?;
    assert_eq!(projection.lifecycle(), RunLifecycle::Running);
    assert_eq!(projection.attempts().len(), 2);
    assert_eq!(projection.unresolved_attempts().count(), 2);
    assert!(
        projection
            .attempts()
            .values()
            .all(|attempt| attempt.state() == &AttemptState::Uncertain)
    );
    assert!(
        !runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeTerminal { .. }))
    );
    Ok(())
}

#[test]
fn active_retry_cancellation_only_closes_harmless_prior_uncertainty() -> TestResult {
    for (suffix, side_effect, closes) in [
        ("none", "none", true),
        ("idempotent", "idempotent_write", false),
    ] {
        let directory = TempDir::new()?;
        let store = Arc::new(RedbStore::open(directory.path())?);
        let clock = Arc::new(ManualClock::new(NOW));
        let executor = Arc::new(BoundaryThenBlockingExecutor::new(
            descriptor_with_model_side_effect(side_effect)?,
        ));
        let runtime = Arc::new(RuntimeService::new(
            store.clone(),
            executor.clone(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new(
                format!("active-retry-cancel-{suffix}"),
                1,
            )?),
            RuntimeConfig::new(
                WorkerId::new(format!("worker-active-retry-cancel-{suffix}"))?,
                ActorRef::new(format!("controller:active-retry-cancel-{suffix}"))?,
                30_000,
                32,
                SchedulerLimits::new(8, 4, 2, 4)?,
                RetryPolicy::new(2, vec![ErrorClass::Adapter], 1, 1_000, 0)?,
            )?,
        )?);
        let revision = task_revision(&format!("workflow-active-retry-cancel-{suffix}"))?;
        let run = RunId::new(format!("run-active-retry-cancel-{suffix}"))?;
        store.put_revision(&revision)?;
        assert_eq!(
            submit_command(
                runtime.as_ref(),
                store.as_ref(),
                &run,
                RunCommand::CreateRun {
                    workflow: revision.semantic().workflow().clone(),
                    revision: revision.id().clone(),
                    root_scope: WorkspaceScope::run_root(
                        run.clone(),
                        ScopeId::new(format!("scope-active-retry-cancel-{suffix}"))?,
                    ),
                    workspace_budget: generous_budget()?,
                    inputs: Vec::new(),
                },
            )?,
            CommandDisposition::Accepted
        );
        assert_eq!(
            submit_command(runtime.as_ref(), store.as_ref(), &run, RunCommand::StartRun,)?,
            CommandDisposition::Accepted
        );
        runtime.tick()?;
        clock.advance(1)?;
        let retry_runtime = runtime.clone();
        let retry = std::thread::spawn(move || {
            retry_runtime
                .tick()
                .map_err(|error| format!("active retry tick failed: {error}"))
        });
        executor.wait_until_entered()?;
        assert_eq!(
            submit_command(
                runtime.as_ref(),
                store.as_ref(),
                &run,
                RunCommand::RequestCancellation,
            )?,
            CommandDisposition::Accepted
        );
        runtime.tick()?;
        assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
        executor.release()?;
        retry
            .join()
            .map_err(|_| "active retry dispatch thread panicked")??;
        for _ in 0..4 {
            if runtime.projection(&run)?.is_completed() {
                break;
            }
            runtime.tick()?;
        }
        let projection = runtime.projection(&run)?;
        let first = projection
            .attempts()
            .values()
            .find(|attempt| attempt.attempt_number() == 1)
            .ok_or("uncertain first attempt is absent")?;
        let retry_attempt = projection
            .attempts()
            .values()
            .find(|attempt| attempt.attempt_number() == 2)
            .ok_or("cancelled retry attempt is absent")?;
        if closes {
            assert_eq!(
                first.state(),
                &AttemptState::UncertainAbandonedByCancellation {
                    cancelled_retry: retry_attempt.attempt().clone(),
                }
            );
            assert_eq!(projection.unresolved_attempts().count(), 0);
            assert_eq!(
                projection.lifecycle(),
                RunLifecycle::Terminal(RunOutcome::Cancelled)
            );
        } else {
            assert_eq!(first.state(), &AttemptState::Uncertain);
            assert_eq!(projection.unresolved_attempts().count(), 1);
            assert_eq!(projection.lifecycle(), RunLifecycle::Cancelling);
        }
        assert!(first.terminal().is_none());
        assert!(first.obligation().is_some());
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
fn nested_fork_waits_for_descendants_and_preserves_outputs_through_outer_join() -> TestResult {
    let harness = Harness::new("nested-fork")?;
    install_output_scripts(&harness)?;
    let revision = nested_fork_revision("workflow-nested-fork")?;
    let run = RunId::new("run-nested-fork")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.drive(&run, 16)?, 4);

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    let outer_fork_node = NodeId::new("outer-fork")?;
    let outer_fork = projection
        .executions_for_node(&outer_fork_node)
        .next()
        .ok_or("outer fork execution is absent")?;
    let outer_a_port = PortId::new("a")?;
    let outer_a = projection
        .branches()
        .values()
        .find(|branch| {
            branch.fork_execution() == outer_fork.execution()
                && branch.port() == &outer_a_port
        })
        .ok_or("outer a branch is absent")?;
    assert_eq!(
        outer_a.state(),
        BranchState::Completed(RunOutcome::Succeeded)
    );
    assert_eq!(
        outer_a.outputs().len(),
        1,
        "outer branch lost its declared post-join result output"
    );

    let outer_join_node = NodeId::new("outer-join")?;
    let outer_join_execution = projection
        .executions_for_node(&outer_join_node)
        .next()
        .ok_or("outer join execution is absent")?;
    let outer_join = projection
        .joins()
        .get(outer_join_execution.execution())
        .ok_or("outer join result is absent")?;
    let outer_result = outer_join
        .branches()
        .iter()
        .find(|result| result.branch == *outer_a.branch())
        .ok_or("outer join omitted branch a")?;
    assert_eq!(outer_result.outputs, outer_a.outputs());

    let inner_join_node = NodeId::new("inner-join")?;
    let inner_join_execution = projection
        .executions_for_node(&inner_join_node)
        .next()
        .ok_or("inner join execution is absent")?;
    let inner_join_sequence = projection
        .joins()
        .get(inner_join_execution.execution())
        .ok_or("inner join result is absent")?
        .sequence();
    let tail_node = NodeId::new("outer-a-tail")?;
    let tail_execution = projection
        .executions_for_node(&tail_node)
        .next()
        .ok_or("outer a successor is absent")?;
    let history = harness.runtime.history(&run)?;
    let tail_terminal_sequence = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::NodeTerminal { execution, .. }
                if execution == tail_execution.execution() =>
            {
                Some(event.sequence())
            }
            _ => None,
        })
        .ok_or("outer a successor terminal fact is absent")?;
    let outer_terminal_sequence = history
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::BranchTerminal { branch, .. } if branch == outer_a.branch() => {
                Some(event.sequence())
            }
            _ => None,
        })
        .ok_or("outer a terminal fact is absent")?;
    assert!(outer_terminal_sequence > inner_join_sequence);
    assert!(outer_terminal_sequence > tail_terminal_sequence);
    Ok(())
}

#[test]
fn fork_branches_may_end_at_direct_terminals_without_a_join() -> TestResult {
    for (suffix, a_outcome, b_outcome, expected) in [
        (
            "success",
            TerminalOutcome::Success,
            TerminalOutcome::Success,
            RunOutcome::Succeeded,
        ),
        (
            "mixed",
            TerminalOutcome::Failure,
            TerminalOutcome::Success,
            RunOutcome::Failed,
        ),
    ] {
        let harness = Harness::new(&format!("direct-terminal-fork-{suffix}"))?;
        let revision = direct_terminal_fork_revision(
            &format!("workflow-direct-terminal-fork-{suffix}"),
            a_outcome,
            b_outcome,
        )?;
        let run = RunId::new(format!("run-direct-terminal-fork-{suffix}"))?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        let projection = harness.runtime.projection(&run)?;
        assert_eq!(projection.lifecycle(), RunLifecycle::Terminal(expected));
        assert_eq!(projection.branches().len(), 2);
        assert!(projection.branches().values().all(|branch| !branch.is_active()));
        if suffix == "mixed" {
            assert!(projection.branches().values().any(|branch| {
                branch.state() == BranchState::Completed(RunOutcome::Failed)
            }));
            assert!(projection.branches().values().any(|branch| {
                branch.state() == BranchState::Completed(RunOutcome::Succeeded)
            }));
        }
    }
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
        let mut branches: Vec<_> = projection.branches().values().collect();
        branches.sort_by(|left, right| left.port().cmp(right.port()));
        let branch_outputs: Vec<_> = branches
            .into_iter()
            .flat_map(|branch| branch.outputs().iter().cloned())
            .collect();
        let mut lexical_outputs = branch_outputs.clone();
        lexical_outputs.sort();
        assert_ne!(
            branch_outputs, lexical_outputs,
            "fixture must distinguish declared branch order from reference lexical order"
        );
        match strategy {
            ReducerStrategy::Collect => {
                let values = output
                    .value()
                    .as_json()
                    .and_then(|value| value.value().as_array())
                    .ok_or("collect output is not a structured array")?;
                assert_eq!(values.len(), 2);
                assert_eq!(
                    output.value().as_json().map(BoundedJson::value),
                    Some(&serde_json::to_value(&branch_outputs)?)
                );
            }
            ReducerStrategy::First => {
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
fn broadcast_signal_fanout_is_received_once_then_drained_in_bounded_batches() -> TestResult {
    const OUTPUTS_PER_WAIT: usize = 254;
    let harness = Harness::new("broadcast-fanout")?;
    let revision = broadcast_fanout_revision("workflow-broadcast-fanout", OUTPUTS_PER_WAIT)?;
    let run = RunId::new("run-broadcast-fanout")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.runtime.projection(&run)?.waits().len(), 2);

    let signal = SignalId::new("signal-broadcast-fanout")?;
    let command = harness.runtime.command(
        run.clone(),
        ActorRef::new("human:broadcast-fanout")?,
        harness.store.head(&run)?,
        Reason::new("deliver a fanout larger than one atomic event batch")?,
        Vec::new(),
        RunCommand::DeliverSignal {
            signal: signal.clone(),
            signal_type: SignalTypeId::new("notify.broadcast")?,
            correlation: None,
            mode: SignalDeliveryMode::Broadcast,
            payload: BoundedJson::new(json!({"broadcast": true}))?,
        },
    )?;
    let accepted = harness.runtime.handle_command(&command)?;
    assert!(!accepted.replayed());
    assert_eq!(accepted.result().disposition(), CommandDisposition::Accepted);
    assert_eq!(accepted.result().event_ids().len(), 1);
    let replayed = harness.runtime.handle_command(&command)?;
    assert!(replayed.replayed());
    assert_eq!(replayed.result(), accepted.result());

    let received = harness.runtime.projection(&run)?;
    let signal_view = received
        .signals()
        .get(&signal)
        .ok_or("broadcast signal is absent")?;
    assert!(signal_view.consumed_by().is_empty());
    assert_eq!(received.waits().values().filter(|wait| wait.is_pending()).count(), 2);

    for _ in 0..8 {
        if harness.runtime.projection(&run)?.is_completed() {
            break;
        }
        harness.runtime.tick()?;
    }
    let completed = harness.runtime.projection(&run)?;
    assert_eq!(
        completed.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(
        completed
            .signals()
            .get(&signal)
            .ok_or("broadcast signal disappeared")?
            .consumed_by()
            .len(),
        2
    );
    let history = harness.runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::SignalReceived { .. }))
            .count(),
        1
    );
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::SignalConsumed { .. }))
            .count(),
        2
    );
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(
                event.kind(),
                RunEventKind::DeterministicOutputPublished { .. }
            ))
            .count(),
        OUTPUTS_PER_WAIT * 2
    );
    Ok(())
}

#[test]
fn unchanged_runnable_index_remains_dispatchable_after_an_unrelated_commit() -> TestResult {
    let harness = Harness::new("unchanged-runnable-index")?;
    let revision = task_revision("workflow-unchanged-runnable-index")?;
    let run = RunId::new("run-unchanged-runnable-index")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(
        harness.command(
            &run,
            RunCommand::DeliverSignal {
                signal: SignalId::new("unmatched-runnable-signal")?,
                signal_type: SignalTypeId::new("notify.unmatched")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(json!({}))?,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(harness.runtime.tick()?.dispatched, 1);
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
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
fn later_cancellation_dominates_completed_explicit_success_and_failure_terminals() -> TestResult {
    for (suffix, terminal_outcome, node_outcome) in [
        ("success", TerminalOutcome::Success, NodeOutcome::Succeeded),
        ("failure", TerminalOutcome::Failure, NodeOutcome::Failed),
    ] {
        let directory = TempDir::new()?;
        let store = Arc::new(RedbStore::open(directory.path())?);
        let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
        let runtime = Arc::new(RuntimeService::new(
            store.clone(),
            executor.clone(),
            Arc::new(ManualClock::new(NOW)),
            Arc::new(SequentialIdGenerator::new(
                format!("terminal-cancellation-{suffix}"),
                1,
            )?),
            RuntimeConfig::new(
                WorkerId::new(format!("worker-terminal-cancellation-{suffix}"))?,
                ActorRef::new(format!("controller:terminal-cancellation-{suffix}"))?,
                30_000,
                32,
                SchedulerLimits::new(8, 4, 2, 4)?,
                RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
            )?,
        )?);
        let revision = fork_revision_with_terminal(
            &format!("workflow-terminal-cancellation-{suffix}"),
            JoinPolicy::Any,
            "model.fail",
            terminal_outcome,
        )?;
        let run = RunId::new(format!("run-terminal-cancellation-{suffix}"))?;
        store.put_revision(&revision)?;
        assert_eq!(
            submit_command(
                runtime.as_ref(),
                store.as_ref(),
                &run,
                RunCommand::CreateRun {
                    workflow: revision.semantic().workflow().clone(),
                    revision: revision.id().clone(),
                    root_scope: WorkspaceScope::run_root(
                        run.clone(),
                        ScopeId::new(format!("scope-terminal-cancellation-{suffix}"))?,
                    ),
                    workspace_budget: generous_budget()?,
                    inputs: Vec::new(),
                },
            )?,
            CommandDisposition::Accepted
        );
        assert_eq!(
            submit_command(runtime.as_ref(), store.as_ref(), &run, RunCommand::StartRun)?,
            CommandDisposition::Accepted
        );

        let blocked_runtime = runtime.clone();
        let blocked = std::thread::spawn(move || {
            blocked_runtime
                .tick()
                .map_err(|error| format!("terminal cancellation blocked tick failed: {error}"))
        });
        executor.wait_until_entered()?;
        assert_eq!(runtime.tick()?.dispatched, 1);
        let midway = runtime.projection(&run)?;
        let done = NodeId::new("done")?;
        assert_eq!(
            midway
                .executions_for_node(&done)
                .next()
                .map(|execution| execution.state()),
            Some(&NodeExecutionState::Terminal(node_outcome))
        );
        assert_eq!(midway.lifecycle(), RunLifecycle::Running);
        if terminal_outcome == TerminalOutcome::Failure {
            let history = runtime.history(&run)?;
            let failure_sequence = history
                .iter()
                .find_map(|event| match event.kind() {
                    RunEventKind::DeterministicNodeTerminal {
                        execution,
                        outcome: NodeOutcome::Failed,
                        ..
                    } if midway
                        .node_executions()
                        .get(execution)
                        .is_some_and(|execution| execution.node() == &done) =>
                    {
                        Some(event.sequence())
                    }
                    _ => None,
                })
                .ok_or("explicit failure terminal event is absent")?;
            let termination = midway
                .termination()
                .ok_or("failure drain termination intent is absent")?;
            assert_eq!(termination.outcome(), RunOutcome::Failed);
            assert!(termination.sequence() > failure_sequence);
            assert!(midway.cancellation().is_none());
        }
        assert_eq!(
            submit_command(
                runtime.as_ref(),
                store.as_ref(),
                &run,
                RunCommand::RequestCancellation,
            )?,
            CommandDisposition::Accepted
        );
        runtime.tick()?;
        assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
        executor.release()?;
        blocked
            .join()
            .map_err(|_| "terminal cancellation dispatch thread panicked")??;
        for _ in 0..4 {
            if runtime.projection(&run)?.is_completed() {
                break;
            }
            runtime.tick()?;
        }
        let completed = runtime.projection(&run)?;
        assert_eq!(
            completed.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Cancelled),
            "explicit {suffix} terminal overrode later cancellation intent"
        );
        let terminal_outcomes: Vec<_> = runtime
            .history(&run)?
            .iter()
            .filter_map(|event| match event.kind() {
                RunEventKind::RunTerminal { outcome, .. } => Some(*outcome),
                _ => None,
            })
            .collect();
        assert_eq!(terminal_outcomes, vec![RunOutcome::Cancelled]);
    }
    Ok(())
}

#[test]
fn explicit_failure_terminal_drains_owned_work_and_finishes_failed() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new(
        store.clone(),
        executor.clone(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("explicit-failure-drain", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-explicit-failure-drain")?,
            ActorRef::new("controller:explicit-failure-drain")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?);
    let revision = fork_revision_with_terminal(
        "workflow-explicit-failure-drain",
        JoinPolicy::Any,
        "model.fail",
        TerminalOutcome::Failure,
    )?;
    let run = RunId::new("run-explicit-failure-drain")?;
    store.put_revision(&revision)?;
    assert_eq!(
        submit_command(
            runtime.as_ref(),
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-explicit-failure-drain")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(runtime.as_ref(), store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );

    let blocked_runtime = runtime.clone();
    let blocked = std::thread::spawn(move || {
        blocked_runtime
            .tick()
            .map_err(|error| format!("explicit failure blocked tick failed: {error}"))
    });
    executor.wait_until_entered()?;
    assert_eq!(runtime.tick()?.dispatched, 1);

    let draining = runtime.projection(&run)?;
    assert_eq!(draining.lifecycle(), RunLifecycle::Running);
    assert_eq!(
        draining
            .termination()
            .ok_or("explicit failure drain intent is absent")?
            .outcome(),
        RunOutcome::Failed
    );
    assert!(draining.cancellation().is_none());
    assert!(!runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::RunCancellationRequested { .. }
    )));

    runtime.tick()?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    executor.release()?;
    blocked
        .join()
        .map_err(|_| "explicit failure dispatch thread panicked")??;
    for _ in 0..4 {
        if runtime.projection(&run)?.is_completed() {
            break;
        }
        runtime.tick()?;
    }
    let completed = runtime.projection(&run)?;
    assert_eq!(
        completed.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert!(completed.cancellation().is_none());
    let terminal_outcomes: Vec<_> = runtime
        .history(&run)?
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::RunTerminal { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .collect();
    assert_eq!(terminal_outcomes, vec![RunOutcome::Failed]);
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
fn more_than_index_mutation_limit_inactive_identities_do_not_block_commits() -> TestResult {
    let harness = Harness::new("large-inactive-index-history")?;
    let revision = signal_revision("workflow-large-inactive-index-history")?;
    let run = RunId::new("run-large-inactive-index-history")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    let initial = harness.runtime.projection(&run)?;
    let root_scope = initial
        .root_scope()
        .ok_or("large-history run has no root scope")?
        .reference()
        .clone();
    let budget = initial
        .workspace_budget()
        .ok_or("large-history run has no workspace budget")?
        .clone();
    let usage = harness.store.workspace_usage(&run)?;
    let historical_count = MAX_INDEX_MUTATIONS_PER_COMMIT + 1;
    let mut created = 0_usize;
    let mut batch_number = 0_usize;

    // Seed validated, immutable terminal execution facts in bounded journal commits.
    // The final runtime command is the behavior under test: the old implementation
    // generated one index tombstone for every seeded historical identity and failed
    // the atomic index-mutation bound before it could pause the run.
    while created < historical_count {
        let expected = harness.store.head(&run)?;
        let batch_size = historical_count.saturating_sub(created).min(256);
        let mut sequence = expected;
        let mut events = Vec::with_capacity(batch_size.saturating_mul(2));
        for offset in 0..batch_size {
            let number = created.saturating_add(offset);
            let execution = NodeExecutionId::new(format!("historical-execution-{number:04}"))?;
            sequence = sequence.next()?;
            events.push(RunEventEnvelope::new(
                EventId::new(format!("historical-eligible-{number:04}"))?,
                run.clone(),
                sequence,
                TimestampMillis::new(NOW),
                RunEventKind::NodeBecameEligible {
                    node: NodeId::new("done")?,
                    execution: execution.clone(),
                    scope: root_scope.clone(),
                    mode: NodeExecutionMode::Runtime,
                },
            )?);
            sequence = sequence.next()?;
            events.push(RunEventEnvelope::new(
                EventId::new(format!("historical-terminal-{number:04}"))?,
                run.clone(),
                sequence,
                TimestampMillis::new(NOW),
                RunEventKind::DeterministicNodeTerminal {
                    execution,
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            )?);
        }
        let command = CommandId::new(format!("seed-index-history-{batch_number:02}"))?;
        let receipt = CommandReceipt::new(
            command.clone(),
            run.clone(),
            ActorRef::new("controller:large-inactive-index-history")?,
            expected,
            TimestampMillis::new(NOW),
            format!(r#"{{"batch":{batch_number},"schema_version":1,"type":"seed_index_history"}}"#)
                .into_bytes(),
        )?;
        let event_ids = events
            .iter()
            .map(|event| event.event_id().clone())
            .collect();
        let result = CommandResultDocument::new(
            command,
            run.clone(),
            receipt.fingerprint().clone(),
            CommandDisposition::Accepted,
            sequence,
            event_ids,
            BoundedJson::new(json!({"accepted": true}))?,
        )?;
        harness.store.commit_command(&AtomicRunCommitRequest::new(
            receipt,
            events,
            Vec::new(),
            Some(WorkspaceAccounting {
                budget: budget.clone(),
                expected_usage: usage,
                resulting_usage: usage,
            }),
            Vec::new(),
            Vec::new(),
            None,
            result,
            RunIndexUpdate {
                summary: Some(RunSummaryIndex {
                    run: run.clone(),
                    workflow: revision.semantic().workflow().clone(),
                    revision: revision.id().clone(),
                    state: IndexedRunState::Waiting,
                    through_sequence: sequence,
                    updated_at: TimestampMillis::new(NOW),
                }),
                ..RunIndexUpdate::default()
            },
        )?)?;
        created = created.saturating_add(batch_size);
        batch_number = batch_number.saturating_add(1);
    }

    let projection = harness.runtime.projection(&run)?;
    assert!(projection.waits().values().any(|wait| wait.is_pending()));
    assert!(
        projection.node_executions().len() > MAX_INDEX_MUTATIONS_PER_COMMIT,
        "fixture accumulated only {} execution identities",
        projection.node_executions().len()
    );
    assert_eq!(
        harness.command(&run, RunCommand::PauseRun)?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Paused
    );
    Ok(())
}

#[test]
fn removed_completed_history_is_inert_after_revision_adoption() -> TestResult {
    let harness = Harness::new("removed-completed-adoption")?;
    let old = removable_task_revision("workflow-removed-completed-adoption")?;
    let new = revision_without_completed_task(&old)?;
    let run = RunId::new("run-removed-completed-adoption")?;
    harness.put_revision(&old)?;
    harness.put_revision(&new)?;
    harness.create_and_start(&run, &old)?;
    assert_eq!(harness.drive(&run, 4)?, 1);

    assert_eq!(
        harness.command(
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-removed-completed")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let projection = harness.runtime.projection(&run)?;
    let plan = projection
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("removed-completed plan is absent")?;
    assert!(plan.items().iter().any(|item| {
        item.node.as_ref() == NodeId::new("retired").ok().as_ref()
            && item.classification == ReconciliationClassification::ChangedCompleted
            && item.action == ReconciliationAction::UseNewOnNextInvocation
    }));
    let plan_id = plan.plan().clone();
    assert_eq!(
        harness.command(&run, RunCommand::ApplyReconciliation { plan: plan_id })?,
        CommandDisposition::Accepted
    );
    assert_eq!(harness.runtime.projection(&run)?.revision(), Some(new.id()));
    assert_eq!(
        harness.command(
            &run,
            RunCommand::DeliverSignal {
                signal: SignalId::new("removed-completed-signal")?,
                signal_type: SignalTypeId::new("notify.ready")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(json!({}))?,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    Ok(())
}

#[test]
fn removed_side_effecting_history_requires_authority_and_cannot_fabricate_remediation() -> TestResult
{
    let harness = Harness::with_descriptor(
        "removed-side-effect-adoption",
        RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        descriptor_with_model_side_effect("non_idempotent_write")?,
    )?;
    install_non_idempotent_success_script(&harness)?;
    let old = removable_task_revision("workflow-removed-side-effect-adoption")?;
    let new = revision_without_completed_task(&old)?;
    harness.put_revision(&old)?;
    harness.put_revision(&new)?;

    let rejected_run = RunId::new("run-removed-side-effect-rejected")?;
    harness.create_and_start(&rejected_run, &old)?;
    assert_eq!(harness.drive(&rejected_run, 4)?, 1);
    assert_eq!(
        harness.command(
            &rejected_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-removed-remediation")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::CompensateOrRemediate,
            },
        )?,
        CommandDisposition::Accepted
    );
    let rejected_projection = harness.runtime.projection(&rejected_run)?;
    let rejected_plan = rejected_projection
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("removed-side-effect remediation plan is absent")?;
    assert!(rejected_plan.items().iter().any(|item| {
        item.node.as_ref() == NodeId::new("retired").ok().as_ref()
            && item.classification == ReconciliationClassification::CompletedOrUncertainSideEffects
            && item.action == ReconciliationAction::RejectRetrospectiveRewrite
    }));
    assert_eq!(
        harness.command(
            &rejected_run,
            RunCommand::ApplyReconciliation {
                plan: rejected_plan.plan().clone(),
            },
        )?,
        CommandDisposition::Rejected
    );
    assert_eq!(
        harness.runtime.projection(&rejected_run)?.revision(),
        Some(old.id())
    );

    let authorized_run = RunId::new("run-removed-side-effect-authorized")?;
    harness.create_and_start(&authorized_run, &old)?;
    assert_eq!(harness.drive(&authorized_run, 4)?, 1);
    assert_eq!(
        harness.command(
            &authorized_run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-removed-authority")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::RequireAuthority,
            },
        )?,
        CommandDisposition::Accepted
    );
    let authority_projection = harness.runtime.projection(&authorized_run)?;
    let authority_plan = authority_projection
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("removed-side-effect authority plan is absent")?;
    assert!(authority_plan.items().iter().any(|item| {
        item.node.as_ref() == NodeId::new("retired").ok().as_ref()
            && item.classification == ReconciliationClassification::CompletedOrUncertainSideEffects
            && item.action == ReconciliationAction::RequireAuthority
    }));
    let authority_plan_id = authority_plan.plan().clone();
    assert_eq!(
        harness.command(
            &authorized_run,
            RunCommand::DecideReconciliation {
                plan: authority_plan_id.clone(),
                decision: ReconciliationDecisionId::new("decision-removed-authority")?,
                outcome: AuthorityDecision::Approve,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(
            &authorized_run,
            RunCommand::ApplyReconciliation {
                plan: authority_plan_id,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.runtime.projection(&authorized_run)?.revision(),
        Some(new.id())
    );
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
            && item.classification == ReconciliationClassification::ChangedActive
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

#[test]
fn runtime_owned_structured_work_is_never_planned_as_unstarted_removal_or_attempt_restart()
-> TestResult {
    {
        let harness = Harness::new("reconcile-active-wait-change")?;
        let old = wait_revision("workflow-reconcile-active-wait-change", 60_000)?;
        let new = revised_wait_revision(&old, 120_000)?;
        let run = RunId::new("run-reconcile-active-wait-change")?;
        harness.put_revision(&old)?;
        harness.put_revision(&new)?;
        harness.create_and_start(&run, &old)?;
        assert!(
            harness
                .runtime
                .projection(&run)?
                .waits()
                .values()
                .any(|wait| wait.is_pending())
        );
        assert_eq!(
            harness.command(
                &run,
                RunCommand::RequestRevisionAdoption {
                    reconciliation: ReconciliationId::new("reconcile-active-wait-change")?,
                    revision: new.id().clone(),
                    policy: ReconciliationPolicy::CancelAndRestartSafeWork,
                },
            )?,
            CommandDisposition::Accepted
        );
        let projection = harness.runtime.projection(&run)?;
        let plan = projection
            .reconciliation()
            .plans()
            .values()
            .next()
            .ok_or("active wait change plan is absent")?;
        assert!(plan.items().iter().any(|item| {
            item.node.as_ref() == NodeId::new("wait").ok().as_ref()
                && item.classification == ReconciliationClassification::ChangedActive
                && item.action == ReconciliationAction::RejectRetrospectiveRewrite
        }));
        assert_eq!(
            harness.command(
                &run,
                RunCommand::ApplyReconciliation {
                    plan: plan.plan().clone(),
                },
            )?,
            CommandDisposition::Rejected
        );
    }

    {
        let harness = Harness::new("reconcile-active-wait-remove")?;
        let old = wait_revision("workflow-reconcile-active-wait-remove", 60_000)?;
        let new = revision_without_entry_node(&old, "wait", &["wait-done"])?;
        let run = RunId::new("run-reconcile-active-wait-remove")?;
        harness.put_revision(&old)?;
        harness.put_revision(&new)?;
        harness.create_and_start(&run, &old)?;
        assert_eq!(
            harness.command(
                &run,
                RunCommand::RequestRevisionAdoption {
                    reconciliation: ReconciliationId::new("reconcile-active-wait-remove")?,
                    revision: new.id().clone(),
                    policy: ReconciliationPolicy::RemoveUnstartedOnly,
                },
            )?,
            CommandDisposition::Accepted
        );
        let projection = harness.runtime.projection(&run)?;
        let plan = projection
            .reconciliation()
            .plans()
            .values()
            .next()
            .ok_or("active wait removal plan is absent")?;
        assert!(plan.items().iter().any(|item| {
            item.node.as_ref() == NodeId::new("wait").ok().as_ref()
                && item.classification == ReconciliationClassification::ChangedActive
                && item.action == ReconciliationAction::RejectRetrospectiveRewrite
        }));
        assert!(!plan.items().iter().any(|item| {
            item.node.as_ref() == NodeId::new("wait").ok().as_ref()
                && item.action == ReconciliationAction::RemoveUnstarted
        }));
    }

    for (suffix, repeat) in [("subworkflow", false), ("repeat", true)] {
        let child = wait_revision(&format!("workflow-{suffix}-child"), 60_000)?;
        let old = if repeat {
            repeat_revision(&format!("workflow-active-{suffix}"), &child)?
        } else {
            subworkflow_revision(&format!("workflow-active-{suffix}"), &child)?
        };
        let node = if repeat { "repeat" } else { "child" };
        let edge = if repeat { "repeat-done" } else { "child-done" };
        let new = revision_without_entry_node(&old, node, &[edge])?;
        let harness = Harness::new(&format!("reconcile-active-{suffix}"))?;
        let run = RunId::new(format!("run-reconcile-active-{suffix}"))?;
        harness.put_revision(&child)?;
        harness.put_revision(&old)?;
        harness.put_revision(&new)?;
        harness.create_and_start(&run, &old)?;
        let active = harness.runtime.projection(&run)?;
        assert!(
            active
                .subworkflows()
                .values()
                .any(|child| child.is_active()),
            "{suffix} fixture did not retain active child ownership"
        );
        assert_eq!(
            harness.command(
                &run,
                RunCommand::RequestRevisionAdoption {
                    reconciliation: ReconciliationId::new(format!("reconcile-active-{suffix}"))?,
                    revision: new.id().clone(),
                    policy: ReconciliationPolicy::RemoveUnstartedOnly,
                },
            )?,
            CommandDisposition::Accepted
        );
        let projection = harness.runtime.projection(&run)?;
        let plan = projection
            .reconciliation()
            .plans()
            .values()
            .next()
            .ok_or("structured active removal plan is absent")?;
        assert!(plan.items().iter().any(|item| {
            item.node.as_ref() == NodeId::new(node).ok().as_ref()
                && item.classification == ReconciliationClassification::ChangedActive
                && item.action == ReconciliationAction::RejectRetrospectiveRewrite
        }));
    }
    Ok(())
}

#[test]
fn active_branch_frontier_does_not_capture_unowned_post_join_pending_work() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new(
        store.clone(),
        executor.clone(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("reconcile-branch-frontier", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-reconcile-branch-frontier")?,
            ActorRef::new("controller:reconcile-branch-frontier")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?);
    let old = fork_revision_with_post_join_task("workflow-active-branch-frontier")?;
    let new = revision_without_post_join_task(&old)?;
    let run = RunId::new("run-reconcile-active-branch-frontier")?;
    store.put_revision(&old)?;
    store.put_revision(&new)?;
    assert_eq!(
        submit_command(
            runtime.as_ref(),
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: old.semantic().workflow().clone(),
                revision: old.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-reconcile-active-branch-frontier")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(runtime.as_ref(), store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    let blocked_runtime = runtime.clone();
    let blocked = std::thread::spawn(move || {
        blocked_runtime
            .tick()
            .map_err(|error| format!("blocked branch tick failed: {error}"))
    });
    executor.wait_until_entered()?;
    assert_eq!(runtime.tick()?.dispatched, 1);

    let active = runtime.projection(&run)?;
    assert!(active.branches().values().any(|branch| branch.is_active()));
    let owned_before: Vec<_> = active
        .branches()
        .values()
        .filter(|branch| branch.is_active())
        .map(|branch| {
            (
                branch.branch().clone(),
                branch.fork_execution().clone(),
                branch.children().clone(),
                branch.state(),
            )
        })
        .collect();
    assert_eq!(
        active
            .executions_for_node(&NodeId::new("independent")?)
            .next()
            .map(|execution| execution.state()),
        Some(&NodeExecutionState::Eligible)
    );

    assert_eq!(
        submit_command(
            runtime.as_ref(),
            store.as_ref(),
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconcile-branch-frontier")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::RemoveUnstartedOnly,
            },
        )?,
        CommandDisposition::Accepted
    );
    let projection = runtime.projection(&run)?;
    let plan = projection
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("branch-frontier plan is absent")?;
    assert!(plan.items().iter().any(|item| {
        item.node.as_ref() == NodeId::new("independent").ok().as_ref()
            && item.classification == ReconciliationClassification::RemovedPending
            && item.action == ReconciliationAction::RemoveUnstarted
    }));
    let plan_id = plan.plan().clone();
    assert_eq!(
        submit_command(
            runtime.as_ref(),
            store.as_ref(),
            &run,
            RunCommand::ApplyReconciliation { plan: plan_id },
        )?,
        CommandDisposition::Accepted,
        "reconciliation items: {:?}",
        plan.items()
    );
    let applied = runtime.projection(&run)?;
    assert_eq!(applied.revision(), Some(new.id()));
    assert_eq!(
        applied
            .executions_for_node(&NodeId::new("independent")?)
            .next()
            .map(|execution| execution.state()),
        Some(&NodeExecutionState::RemovedProspectively(
            plan.plan().clone()
        ))
    );
    for (branch, fork, children, state) in owned_before {
        let after = applied
            .branches()
            .get(&branch)
            .ok_or("active branch ownership disappeared during adoption")?;
        assert_eq!(after.fork_execution(), &fork);
        assert_eq!(after.children(), &children);
        assert_eq!(after.state(), state);
        assert!(after.is_active());
    }
    assert!(
        applied
            .attempts()
            .values()
            .any(|attempt| attempt.is_active())
    );
    runtime.tick()?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    executor.release()?;
    blocked
        .join()
        .map_err(|_| "blocked branch tick panicked")??;
    Ok(())
}

#[test]
fn revision_adoption_materializes_a_new_root_entry_exactly_once() -> TestResult {
    let harness = Harness::new("adoption-added-root")?;
    let old = wait_revision("workflow-adoption-added-root", 60_000)?;
    let new = revision_with_added_root_wait(&old, 60_000)?;
    let run = RunId::new("run-adoption-added-root")?;
    harness.put_revision(&old)?;
    harness.put_revision(&new)?;
    harness.create_and_start(&run, &old)?;
    assert_eq!(
        harness.command(
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconciliation-added-root")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let plan = harness
        .runtime
        .projection(&run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("added-root adoption plan is absent")?
        .plan()
        .clone();
    assert_eq!(
        harness.command(&run, RunCommand::ApplyReconciliation { plan })?,
        CommandDisposition::Accepted
    );
    let added = NodeId::new("added-root")?;
    assert_eq!(
        harness
            .runtime
            .projection(&run)?
            .executions_for_node(&added)
            .count(),
        1
    );
    harness.runtime.tick()?;
    harness.runtime.tick()?;
    assert_eq!(
        harness
            .runtime
            .projection(&run)?
            .executions_for_node(&added)
            .count(),
        1,
        "structured driving must not duplicate an adopted root entry"
    );
    Ok(())
}

#[test]
fn cancel_and_restart_adoption_creates_one_replacement_after_confirmed_cancellation() -> TestResult
{
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let executor = Arc::new(BlockingExecutor::new(test_descriptor()?)?);
    let runtime = Arc::new(RuntimeService::new(
        store.clone(),
        executor.clone(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("cancel-restart-adoption", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-cancel-restart-adoption")?,
            ActorRef::new("controller:cancel-restart-adoption")?,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?);
    let old = task_revision("workflow-cancel-restart-adoption")?;
    let new = revised_task_revision(&old, "model.fail")?;
    let run = RunId::new("run-cancel-restart-adoption")?;
    store.put_revision(&old)?;
    store.put_revision(&new)?;
    let create = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("create cancel-and-restart adoption run")?,
        Vec::new(),
        RunCommand::CreateRun {
            workflow: old.semantic().workflow().clone(),
            revision: old.id().clone(),
            root_scope: WorkspaceScope::run_root(
                run.clone(),
                ScopeId::new("scope-cancel-restart-adoption")?,
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
        Reason::new("start cancel-and-restart adoption run")?,
        Vec::new(),
        RunCommand::StartRun,
    )?;
    runtime.handle_command(&start)?;

    let dispatch_runtime = runtime.clone();
    let dispatch =
        std::thread::spawn(move || dispatch_runtime.tick().map_err(|error| error.to_string()));
    executor.wait_until_entered()?;
    let request = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("adopt a changed active safe task")?,
        Vec::new(),
        RunCommand::RequestRevisionAdoption {
            reconciliation: ReconciliationId::new("reconciliation-cancel-restart")?,
            revision: new.id().clone(),
            policy: ReconciliationPolicy::CancelAndRestartSafeWork,
        },
    )?;
    assert_eq!(
        runtime.handle_command(&request)?.result().disposition(),
        CommandDisposition::Accepted
    );
    let plan = runtime
        .projection(&run)?
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("cancel-and-restart plan is absent")?
        .plan()
        .clone();
    let apply = runtime.command(
        run.clone(),
        ActorRef::new("human:structured-runtime-test")?,
        store.head(&run)?,
        Reason::new("apply cancel-and-restart plan")?,
        Vec::new(),
        RunCommand::ApplyReconciliation { plan },
    )?;
    assert_eq!(
        runtime.handle_command(&apply)?.result().disposition(),
        CommandDisposition::Accepted
    );

    runtime.tick()?;
    assert_eq!(executor.cancellation_requests.load(Ordering::SeqCst), 1);
    executor.release()?;
    dispatch
        .join()
        .map_err(|_| "cancel-and-restart dispatch thread panicked")?
        .map_err(|error| format!("cancel-and-restart dispatch failed: {error}"))?;
    for _ in 0..4 {
        if runtime.projection(&run)?.is_completed() {
            break;
        }
        runtime.tick()?;
    }

    let projection = runtime.projection(&run)?;
    assert_eq!(projection.revision(), Some(new.id()));
    let work = NodeId::new("work")?;
    let mut executions: Vec<_> = projection.executions_for_node(&work).collect();
    executions.sort_by_key(|execution| execution.created_sequence());
    assert_eq!(executions.len(), 2);
    assert_eq!(executions[0].scope(), executions[1].scope());
    assert_eq!(
        executions[0].state(),
        &NodeExecutionState::Terminal(milkdrift_persistence::NodeOutcome::Cancelled)
    );
    assert_eq!(
        executions[1].state(),
        &NodeExecutionState::Terminal(milkdrift_persistence::NodeOutcome::Succeeded)
    );
    assert_eq!(
        runtime
            .history(&run)?
            .iter()
            .filter(|event| matches!(
                event.kind(),
                RunEventKind::ReconciliationCancellationRequested { .. }
            ))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn deterministic_progress_larger_than_one_commit_resumes_to_completion() -> TestResult {
    let harness = Harness::new("long-deterministic-chain")?;
    let revision = long_deterministic_chain_revision("workflow-long-chain", 250)?;
    let run = RunId::new("run-long-chain")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert!(
        !harness.runtime.projection(&run)?.is_completed(),
        "the first bounded command must stop before consuming the whole chain"
    );
    harness.drive(&run, 8)?;
    let projection = harness.runtime.projection(&run)?;
    assert!(projection.is_completed());
    assert!(
        projection.sequence().get() > 512,
        "fixture must prove deterministic closure spans more than one commit (sequence {})",
        projection.sequence().get()
    );
    Ok(())
}

#[test]
fn undeclared_executor_output_is_durably_rejected_without_workspace_mutation() -> TestResult {
    let harness = Harness::new("undeclared-output")?;
    let revision = task_revision("workflow-undeclared-output")?;
    let run = RunId::new("run-undeclared-output")?;
    let artifact = publish_artifact(&harness, "rogue-output", b"rogue-output")?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "rogue".to_owned(),
                reference: artifact,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    assert!(harness.runtime.tick().is_err());
    let projection = harness.runtime.projection(&run)?;
    let root = projection
        .root_scope()
        .ok_or("undeclared-output run has no root scope")?;
    assert!(
        harness
            .store
            .latest_value(root.reference(), &ValueKey::new("rogue")?)?
            .is_none()
    );
    assert!(
        projection
            .node_executions()
            .values()
            .all(|execution| execution.outputs().is_empty())
    );
    assert!(
        !harness
            .runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeOutputPublished { .. }))
    );
    Ok(())
}

#[test]
fn oversized_provider_retry_after_preserves_the_terminal_failure() -> TestResult {
    let harness = Harness::with_retry_policy(
        "retry-after-cap",
        RetryPolicy::new(2, vec![ErrorClass::Transport], 10, 1_000, 0)?,
    )?;
    let revision = task_revision("workflow-retry-after-cap")?;
    let run = RunId::new("run-retry-after-cap")?;
    let terminal = InvocationTerminal::new(
        TerminalStatus::Failure,
        Vec::new(),
        Some(InvocationFailure::new(
            ErrorClass::Transport,
            true,
            "provider_busy",
            "provider requested an out-of-policy delay",
            Some(10_000),
        )?),
        None,
        SideEffectClass::None,
    )?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![InvocationEventKind::Terminal { terminal }],
    )?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.runtime.tick()?.completed, 1);

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert_eq!(projection.attempts().len(), 1);
    assert!(projection.retries().is_empty());
    assert!(
        !harness
            .runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeRetryScheduled { .. }))
    );
    Ok(())
}

#[test]
fn direct_artifact_input_is_owned_accounted_and_optional_absence_is_omitted() -> TestResult {
    let harness = Harness::new("direct-artifact-input")?;
    let source_run = RunId::new("artifact-source-run")?;
    let bytes = b"direct-artifact-input";
    let artifact = publish_artifact_for_run(&harness, &source_run, "direct-artifact-input", bytes)?;
    let revision = direct_artifact_input_revision("workflow-direct-artifact", &artifact)?;
    let run = RunId::new("run-direct-artifact")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.drive(&run, 8)?, 1);

    let projection = harness.runtime.projection(&run)?;
    let request = projection
        .attempts()
        .values()
        .next()
        .and_then(|attempt| attempt.request())
        .ok_or("scheduled invocation request was not persisted")?;
    assert_eq!(request.inputs().len(), 1);
    assert_eq!(request.inputs()[0].name(), "artifact");
    assert!(matches!(
        request.inputs()[0].value(),
        InvocationValueReference::Artifact { reference }
            if reference.identity() == artifact.artifact().as_str()
    ));
    assert!(harness.store.is_referenced_by_run(&run, &artifact)?);
    assert_eq!(
        harness.store.workspace_usage(&run)?,
        WorkspaceUsage::new(0, 0, 1, u64::try_from(bytes.len())?)
    );
    Ok(())
}

#[test]
fn successful_terminal_materializes_workflow_and_literal_bindings() -> TestResult {
    let harness = Harness::new("terminal-bindings")?;
    let revision = terminal_binding_revision("workflow-terminal-bindings")?;
    let run = RunId::new("run-terminal-bindings")?;
    harness.put_revision(&revision)?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-terminal-bindings")?);
    let source = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("source")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"source": 7}))?),
    );
    assert_eq!(
        harness.command(
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: root,
                workspace_budget: generous_budget()?,
                inputs: vec![source],
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(&run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );

    let projection = harness.runtime.projection(&run)?;
    let terminal = projection.terminal().ok_or("run did not terminalize")?;
    assert_eq!(terminal.outputs().len(), 2);
    let mut values = BTreeMap::new();
    for reference in terminal.outputs() {
        let entry = harness
            .store
            .value(reference)?
            .ok_or("terminal output workspace value is absent")?;
        values.insert(reference.key().as_str().to_owned(), entry.value().clone());
    }
    assert_eq!(
        values
            .get("pass")
            .and_then(WorkspaceValue::as_json)
            .map(BoundedJson::value),
        Some(&json!({"source": 7}))
    );
    assert_eq!(
        values
            .get("literal")
            .and_then(WorkspaceValue::as_json)
            .map(BoundedJson::value),
        Some(&json!({"materialized": true}))
    );
    Ok(())
}

#[test]
fn missing_optional_condition_binding_routes_exists_to_false() -> TestResult {
    let harness = Harness::new("missing-optional-condition")?;
    let revision = missing_optional_condition_revision("workflow-missing-optional-condition")?;
    let run = RunId::new("run-missing-optional-condition")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    Ok(())
}

#[test]
fn unresolved_optional_edge_does_not_block_selected_target() -> TestResult {
    let harness = Harness::new("optional-unselected-edge")?;
    let revision = optional_unselected_edge_revision("workflow-optional-unselected-edge")?;
    let run = RunId::new("run-optional-unselected-edge")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.drive(&run, 8)?, 1);
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    let scheduled: Vec<_> = harness
        .runtime
        .history(&run)?
        .iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::NodeScheduled { node, request, .. } => {
                Some((node.clone(), request.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].0, NodeId::new("consume")?);
    assert!(scheduled[0].1.inputs().is_empty());
    Ok(())
}

#[test]
fn two_condition_paths_can_share_one_durable_node_output() -> TestResult {
    let harness = Harness::new("multi-path-condition")?;
    let revision = multi_path_condition_revision("workflow-multi-path-condition")?;
    let run = RunId::new("run-multi-path-condition")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(
        harness.command(
            &run,
            RunCommand::DeliverSignal {
                signal: SignalId::new("signal-multi-path")?,
                signal_type: SignalTypeId::new("notify.payload")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(json!({"left": 1, "right": 2}))?,
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    Ok(())
}

#[test]
fn deleted_optional_supplied_input_is_corruption_not_absence() -> TestResult {
    let harness = Harness::new("deleted-optional-input")?;
    let revision = optional_workflow_input_revision("workflow-deleted-optional-input")?;
    let run = RunId::new("run-deleted-optional-input")?;
    harness.put_revision(&revision)?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-deleted-optional-input")?);
    let input = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("optional")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"supplied": true}))?),
    );
    let input_reference = input.reference().clone();
    assert_eq!(
        harness.command(
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: root,
                workspace_budget: generous_budget()?,
                inputs: vec![input],
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(&run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    delete_raw_row(
        directory.path(),
        RAW_VALUES,
        &raw_value_key(&input_reference)?,
    )?;

    let (store, _clock, runtime) =
        runtime_at(directory.path(), "deleted-optional-input-reopen", NOW, 64)?;
    let Err(error) = runtime.tick() else {
        return Err("scheduler treated a deleted supplied optional input as absent".into());
    };
    assert_integrity_error(&error);
    assert_eq!(store.head(&run)?, head);
    let consume = NodeId::new("consume")?;
    assert!(!runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeScheduled { node, .. } if node == &consume
    )));
    Ok(())
}

#[test]
fn orphan_latest_optional_input_is_rejected_against_the_projection() -> TestResult {
    let harness = Harness::new("orphan-optional-input")?;
    let revision = optional_workflow_input_revision("workflow-orphan-optional-input")?;
    let run = RunId::new("run-orphan-optional-input")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    let root = harness
        .runtime
        .projection(&run)?
        .root_scope()
        .ok_or("run root scope was not projected")?
        .reference()
        .clone();
    let orphan = WorkspaceValueEntry::initial(
        root,
        ValueKey::new("optional")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"orphan": true}))?),
    );
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    insert_raw_workspace_value(directory.path(), &orphan)?;

    let (store, _clock, runtime) =
        runtime_at(directory.path(), "orphan-optional-input-reopen", NOW, 64)?;
    let Err(error) = runtime.tick() else {
        return Err("scheduler accepted an unprojected durable latest input".into());
    };
    assert_integrity_error(&error);
    assert_eq!(store.head(&run)?, head);
    Ok(())
}

#[test]
fn deleted_required_producer_output_cannot_be_scheduled_as_an_invocation_input() -> TestResult {
    let harness = Harness::new("deleted-producer-output")?;
    let revision = producer_consumer_revision("workflow-deleted-producer-output")?;
    let run = RunId::new("run-deleted-producer-output")?;
    let output = publish_artifact(&harness, "deleted-producer-output", b"producer-output")?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "result".to_owned(),
                reference: output,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(harness.runtime.tick()?.dispatched, 1);
    let projection = harness.runtime.projection(&run)?;
    let output_reference = projection
        .executions_for_node(&NodeId::new("produce")?)
        .flat_map(|execution| execution.outputs())
        .map(|output| output.value().clone())
        .next()
        .ok_or("producer output was not projected")?;
    assert_eq!(
        projection
            .executions_for_node(&NodeId::new("consume")?)
            .next()
            .map(|execution| execution.state()),
        Some(&NodeExecutionState::Eligible)
    );
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    delete_raw_row(
        directory.path(),
        RAW_VALUES,
        &raw_value_key(&output_reference)?,
    )?;

    let (store, _clock, runtime) =
        runtime_at(directory.path(), "deleted-producer-output-reopen", NOW, 64)?;
    let Err(error) = runtime.tick() else {
        return Err("scheduler dispatched an invocation with a deleted producer output".into());
    };
    assert_integrity_error(&error);
    assert_eq!(store.head(&run)?, head);
    let consume = NodeId::new("consume")?;
    assert!(!runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeScheduled { node, .. } if node == &consume
    )));
    Ok(())
}

#[test]
fn deleted_root_scope_blocks_even_an_inputless_invocation() -> TestResult {
    let harness = Harness::new("deleted-root-scope")?;
    let revision = task_revision("workflow-deleted-root-scope")?;
    let run = RunId::new("run-deleted-root-scope")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    let root = harness
        .runtime
        .projection(&run)?
        .root_scope()
        .ok_or("run root scope was not projected")?
        .reference()
        .clone();
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    delete_raw_row(directory.path(), RAW_SCOPES, &raw_scope_key(&root)?)?;

    let (store, _clock, runtime) =
        runtime_at(directory.path(), "deleted-root-scope-reopen", NOW, 64)?;
    let Err(error) = runtime.tick() else {
        return Err("scheduler dispatched work whose projected root scope was deleted".into());
    };
    assert_integrity_error(&error);
    assert_eq!(store.head(&run)?, head);
    Ok(())
}

#[test]
fn deleted_branch_scope_blocks_its_inputless_child_invocation() -> TestResult {
    let harness = Harness::new("deleted-branch-scope")?;
    let revision = fork_revision(
        "workflow-deleted-branch-scope",
        JoinPolicy::All,
        "model.fail",
    )?;
    let run = RunId::new("run-deleted-branch-scope")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    let projection = harness.runtime.projection(&run)?;
    let mut scopes = Vec::new();
    for node in [NodeId::new("a-task")?, NodeId::new("b-task")?] {
        let scope = projection
            .executions_for_node(&node)
            .next()
            .ok_or("fork child task was not made eligible")?
            .scope()
            .clone();
        assert!(matches!(
            projection.scopes().get(&scope).map(WorkspaceScope::kind),
            Some(ScopeKind::Branch { .. })
        ));
        scopes.push(scope);
    }
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    for scope in &scopes {
        delete_raw_row(directory.path(), RAW_SCOPES, &raw_scope_key(scope)?)?;
    }

    let (store, _clock, runtime) =
        runtime_at(directory.path(), "deleted-branch-scope-reopen", NOW, 64)?;
    let Err(error) = runtime.tick() else {
        return Err("scheduler dispatched work whose projected branch scope was deleted".into());
    };
    assert_integrity_error(&error);
    assert_eq!(store.head(&run)?, head);
    Ok(())
}

#[test]
fn runnable_cursor_keeps_its_cycle_boundary_across_an_advancing_clock() -> TestResult {
    let directory = TempDir::new()?;
    let (store, clock, runtime) = runtime_at(directory.path(), "runnable-cycle", NOW, 1)?;
    let revision = task_revision("workflow-runnable-cycle")?;
    store.put_revision(&revision)?;
    let first = RunId::new("run-a-runnable-cycle")?;
    let second = RunId::new("run-b-runnable-cycle")?;
    for run in [&first, &second] {
        assert_eq!(
            submit_command(
                &runtime,
                store.as_ref(),
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
        assert_eq!(
            submit_command(&runtime, store.as_ref(), run, RunCommand::StartRun)?,
            CommandDisposition::Accepted
        );
    }

    assert_eq!(runtime.tick()?.dispatched, 1);
    let first_after_tick = runtime.projection(&first)?.lifecycle();
    let second_after_tick = runtime.projection(&second)?.lifecycle();
    assert_eq!(
        [first_after_tick, second_after_tick]
            .into_iter()
            .filter(|state| *state == RunLifecycle::Terminal(RunOutcome::Succeeded))
            .count(),
        1,
        "one bounded page must dispatch exactly one of the equally eligible runs"
    );
    clock.advance(1)?;
    assert_eq!(runtime.tick()?.dispatched, 1);
    assert_eq!(
        runtime.projection(&first)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(
        runtime.projection(&second)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    Ok(())
}

#[test]
fn orphan_latest_value_cannot_become_a_worker_output_predecessor() -> TestResult {
    let directory = TempDir::new()?;
    let identity = "orphan-worker-output";
    let (store, clock, runtime) = runtime_with_executor_at(
        directory.path(),
        identity,
        identity,
        NOW,
        64,
        Arc::new(PanickingExecutor {
            resolver: DeterministicExecutor::new(test_descriptor()?),
        }),
    )?;
    let revision = output_child_revision("workflow-orphan-worker-output")?;
    let run = RunId::new("run-orphan-worker-output")?;
    store.put_revision(&revision)?;
    let artifact = publish_artifact_in_store(
        store.as_ref(),
        &RunId::new("artifact-owner-orphan-worker-output")?,
        "orphan-worker-output",
        b"worker-output",
    )?;
    assert_eq!(
        submit_command(
            &runtime,
            store.as_ref(),
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-orphan-worker-output")?,
                ),
                workspace_budget: generous_budget()?,
                inputs: Vec::new(),
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.tick()));
    assert!(
        crash.is_err(),
        "executor did not stop after durable dispatch"
    );
    let projection = runtime.projection(&run)?;
    let attempt_view = projection
        .attempts()
        .values()
        .next()
        .ok_or("durably scheduled attempt is absent")?;
    let attempt = attempt_view.attempt().clone();
    let invocation = attempt_view
        .invocation()
        .ok_or("durably scheduled invocation is absent")?
        .clone();
    let scope = projection
        .node_executions()
        .get(attempt_view.execution())
        .ok_or("durably scheduled execution is absent")?
        .scope()
        .clone();
    let orphan = WorkspaceValueEntry::initial(
        scope,
        ValueKey::new("result")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"orphan": true}))?),
    );
    drop(projection);
    drop(runtime);
    drop(clock);
    drop(store);
    insert_raw_workspace_value(directory.path(), &orphan)?;

    let (store, _clock, runtime) = runtime_with_executor_at(
        directory.path(),
        "orphan-worker-output-reopen",
        identity,
        NOW,
        64,
        Arc::new(DeterministicExecutor::new(test_descriptor()?)),
    )?;
    let head = store.head(&run)?;
    match submit_worker_report(
        &runtime,
        store.as_ref(),
        &run,
        identity,
        WorkerReport::Started {
            attempt: attempt.clone(),
        },
    ) {
        Ok(disposition) => assert_eq!(disposition, CommandDisposition::Accepted),
        Err(error) => {
            assert_integrity_error(
                error
                    .downcast_ref::<RuntimeError>()
                    .ok_or("unexpected non-runtime corruption error")?,
            );
            assert_eq!(store.head(&run)?, head);
            assert!(
                !runtime
                    .history(&run)?
                    .iter()
                    .any(|event| matches!(event.kind(), RunEventKind::NodeOutputPublished { .. }))
            );
            return Ok(());
        }
    }
    let head = store.head(&run)?;
    let output = InvocationEvent::new(
        invocation,
        1,
        InvocationEventKind::Output {
            name: "result".to_owned(),
            reference: artifact,
        },
    )?;
    let Err(error) = submit_worker_report(
        &runtime,
        store.as_ref(),
        &run,
        identity,
        WorkerReport::Invocation {
            attempt,
            report: output,
        },
    ) else {
        return Err("worker output accepted an orphan durable predecessor".into());
    };
    let message = error.to_string();
    assert!(
        message.contains("orphan latest value")
            || message.contains("workspace values disagree with global value accounting")
            || message.contains("immutable workspace_value conflict")
            || message.contains("Corruption"),
        "unexpected corruption error: {message}"
    );
    assert_eq!(store.head(&run)?, head);
    assert!(
        !runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeOutputPublished { .. }))
    );
    assert!(store.value(orphan.reference()).is_err());
    Ok(())
}

#[test]
fn orphan_latest_value_cannot_version_a_deterministic_terminal_output() -> TestResult {
    let harness = Harness::new("orphan-terminal-output")?;
    let revision = terminal_binding_revision("workflow-orphan-terminal-output")?;
    let run = RunId::new("run-orphan-terminal-output")?;
    harness.put_revision(&revision)?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-orphan-terminal-output")?);
    let input = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("source")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"source": true}))?),
    );
    assert_eq!(
        harness.command(
            &run,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: root.clone(),
                workspace_budget: generous_budget()?,
                inputs: vec![input],
            },
        )?,
        CommandDisposition::Accepted
    );
    let orphan = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("pass")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"orphan": true}))?),
    );
    let head = harness.store.head(&run)?;
    let directory = harness.close();
    insert_raw_workspace_value(directory.path(), &orphan)?;

    let (store, _clock, runtime) =
        runtime_at(directory.path(), "orphan-terminal-output-reopen", NOW, 64)?;
    let Err(error) = submit_command(&runtime, store.as_ref(), &run, RunCommand::StartRun) else {
        return Err("terminal output accepted an orphan durable predecessor".into());
    };
    let message = error.to_string();
    assert!(
        message.contains("orphan latest value")
            || message.contains("workspace values disagree with global value accounting")
            || message.contains("immutable workspace_value conflict")
            || message.contains("Corruption"),
        "unexpected corruption error: {message}"
    );
    assert_eq!(store.head(&run)?, head);
    assert_eq!(runtime.projection(&run)?.lifecycle(), RunLifecycle::Created);
    assert!(store.value(orphan.reference()).is_err());
    Ok(())
}

#[test]
fn changed_pending_adoption_supersedes_old_eligibility_and_runs_new_definition() -> TestResult {
    let harness = Harness::new("changed-pending-supersession")?;
    let old = task_revision("workflow-changed-pending-supersession")?;
    let new = revised_task_revision(&old, "model.fail")?;
    let run = RunId::new("run-changed-pending-supersession")?;
    harness.put_revision(&old)?;
    harness.put_revision(&new)?;
    harness.create_and_start(&run, &old)?;

    let original = harness
        .runtime
        .projection(&run)?
        .executions_for_node(&NodeId::new("work")?)
        .next()
        .ok_or("old pending execution is absent")?
        .execution()
        .clone();
    assert_eq!(
        harness.command(
            &run,
            RunCommand::RequestRevisionAdoption {
                reconciliation: ReconciliationId::new("reconcile-changed-pending")?,
                revision: new.id().clone(),
                policy: ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?,
        CommandDisposition::Accepted
    );
    let planned = harness.runtime.projection(&run)?;
    let plan = planned
        .reconciliation()
        .plans()
        .values()
        .next()
        .ok_or("changed-pending reconciliation plan is absent")?;
    assert!(plan.items().iter().any(|item| {
        item.execution.as_ref() == Some(&original)
            && item.classification == ReconciliationClassification::ChangedPending
            && item.action == ReconciliationAction::UseNewOnNextInvocation
    }));
    let plan_id = plan.plan().clone();
    assert_eq!(
        harness.command(&run, RunCommand::ApplyReconciliation { plan: plan_id.clone() })?,
        CommandDisposition::Accepted
    );

    let applied = harness.runtime.projection(&run)?;
    assert_eq!(applied.revision(), Some(new.id()));
    assert_eq!(
        applied
            .node_executions()
            .get(&original)
            .map(|execution| execution.state()),
        Some(&NodeExecutionState::RemovedProspectively(plan_id))
    );
    assert_eq!(
        applied
            .executions_for_node(&NodeId::new("work")?)
            .filter(|execution| execution.execution() != &original)
            .count(),
        1,
        "the new pin must materialize exactly one replacement occurrence"
    );

    assert_eq!(harness.drive(&run, 4)?, 1);
    let completed = harness.runtime.projection(&run)?;
    let work = NodeId::new("work")?;
    let replacement = completed
        .executions_for_node(&work)
        .find(|execution| execution.execution() != &original)
        .ok_or("replacement execution is absent")?;
    let attempt = replacement
        .attempts()
        .last()
        .and_then(|attempt| completed.attempts().get(attempt))
        .ok_or("replacement attempt is absent")?;
    assert_eq!(
        attempt.request().map(|request| request.operation()),
        Some(&OperationId::new("model.fail")?),
        "dispatch must use the adopted definition, not the superseded eligibility"
    );
    Ok(())
}

#[test]
fn paused_runs_record_signal_and_timer_facts_without_advancing_until_resume() -> TestResult {
    {
        let harness = Harness::new("paused-signal")?;
        let revision = signal_revision("workflow-paused-signal")?;
        let run = RunId::new("run-paused-signal")?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        assert_eq!(
            harness.command(&run, RunCommand::PauseRun)?,
            CommandDisposition::Accepted
        );
        let signal = SignalId::new("paused-signal-receipt")?;
        assert_eq!(
            harness.command(
                &run,
                RunCommand::DeliverSignal {
                    signal: signal.clone(),
                    signal_type: SignalTypeId::new("notify.ready")?,
                    correlation: None,
                    mode: SignalDeliveryMode::OneShot,
                    payload: BoundedJson::new(json!({"ready": true}))?,
                },
            )?,
            CommandDisposition::Accepted
        );
        let paused = harness.runtime.projection(&run)?;
        assert_eq!(paused.lifecycle(), RunLifecycle::Paused);
        assert!(paused.waits().values().all(|wait| wait.is_pending()));
        assert!(
            paused
                .signals()
                .get(&signal)
                .is_some_and(|signal| signal.consumed_by().is_empty())
        );
        assert_eq!(
            harness.command(&run, RunCommand::ResumeRun)?,
            CommandDisposition::Accepted
        );
        let resumed = harness.runtime.projection(&run)?;
        assert!(resumed.waits().values().any(|wait| {
            matches!(wait.condition(), milkdrift_persistence::WaitCondition::Signal { .. })
                && wait.is_completed()
        }));
        assert_eq!(
            resumed
                .signals()
                .get(&signal)
                .map(|signal| signal.consumed_by().len()),
            Some(1)
        );
    }

    {
        let harness = Harness::new("paused-timer")?;
        let revision = wait_revision("workflow-paused-timer", 100)?;
        let run = RunId::new("run-paused-timer")?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        assert_eq!(
            harness.command(&run, RunCommand::PauseRun)?,
            CommandDisposition::Accepted
        );
        harness.clock.advance(100)?;
        assert_eq!(harness.runtime.tick()?.dispatched, 0);
        let paused = harness.runtime.projection(&run)?;
        assert_eq!(paused.lifecycle(), RunLifecycle::Paused);
        assert!(paused.timers().values().all(|timer| timer.is_completed()));
        assert!(paused.waits().values().all(|wait| wait.is_pending()));
        assert_eq!(
            harness.command(&run, RunCommand::ResumeRun)?,
            CommandDisposition::Accepted
        );
        let resumed = harness.runtime.projection(&run)?;
        assert!(resumed.waits().values().all(|wait| wait.is_completed()));
        assert_eq!(
            resumed.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Succeeded)
        );
    }
    Ok(())
}

#[test]
fn immutable_repeat_condition_error_is_durably_terminalized_once() -> TestResult {
    let harness = Harness::new("repeat-condition-error")?;
    let child = task_revision("workflow-repeat-condition-error-child")?;
    let seed_field = FieldId::new("seed")?;
    let seed_port = PortId::new("seed")?;
    let seed_binding = BindingSource::WorkflowInput {
        field: seed_field.clone(),
    };
    let repeat = Node::new(
        NodeId::new("repeat")?,
        NodeKind::Repeat {
            config: RepeatConfig::new(
                PinnedSubworkflow::new(
                    child.semantic().workflow().clone(),
                    child.id().clone(),
                    child.semantic().interface().clone(),
                ),
                Condition::Compare {
                    left: ConditionOperand::Binding {
                        source: seed_binding.clone(),
                    },
                    comparison: Comparison::GreaterThan,
                    right: ConditionOperand::Literal {
                        value: BoundedJson::new(json!(0))?,
                    },
                },
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
    .with_control_output(PortId::new("out")?)?
    .with_data_input(
        seed_port,
        DataPort::input(item_schema()?, true, Some(seed_binding))?,
    )?;
    let parent = revision_with_interface(
        "workflow-repeat-condition-error-parent",
        WorkflowInterface::new(
            [(seed_field, InterfaceField::required(item_schema()?))],
            [],
        )?,
        vec![repeat, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("repeat-done", "repeat", "out", "done", "in")?],
    )?;
    let run = RunId::new("run-repeat-condition-error")?;
    let root = WorkspaceScope::run_root(
        run.clone(),
        ScopeId::new("scope-repeat-condition-error")?,
    );
    let input = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("seed")?,
        WorkspaceValue::Json(BoundedJson::new(json!("not-a-number"))?),
    );
    harness.put_revision(&child)?;
    harness.put_revision(&parent)?;
    assert_eq!(
        harness.command(
            &run,
            RunCommand::CreateRun {
                workflow: parent.semantic().workflow().clone(),
                revision: parent.id().clone(),
                root_scope: root,
                workspace_budget: generous_budget()?,
                inputs: vec![input],
            },
        )?,
        CommandDisposition::Accepted
    );
    assert_eq!(
        harness.command(&run, RunCommand::StartRun)?,
        CommandDisposition::Accepted
    );
    harness.drive(&run, 8)?;

    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert_eq!(projection.iterations().len(), 1);
    assert!(projection
        .iterations()
        .values()
        .all(|iteration| iteration.state() == IterationState::Completed(false)));
    assert_eq!(projection.repeat_terminations().len(), 1);
    assert_eq!(
        projection
            .repeat_terminations()
            .values()
            .next()
            .ok_or("repeat condition error has no termination fact")?
            .termination(),
        RepeatTerminationReason::ConditionEvaluationFailed
    );
    Ok(())
}

#[test]
fn immutable_task_input_path_error_is_durably_failed_before_dispatch() -> TestResult {
    let harness = Harness::new("immutable-task-input-path")?;
    let schema = item_schema()?;
    let payload_port = PortId::new("payload")?;
    let source = BindingSource::NodeOutput {
        node: NodeId::new("signal")?,
        port: payload_port.clone(),
        path: PathSelector::new(vec![PathSegment::Field(FieldId::new("missing")?)])?,
    };
    let signal = Node::new(
        NodeId::new("signal")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.payload")?,
        },
    )?
    .with_control_output(PortId::new("out")?)?
    .with_data_output(payload_port.clone(), DataPort::output(schema.clone()))?;
    let consume = task("consume", "model.generate")?.with_data_input(
        PortId::new("input")?,
        DataPort::input(schema, true, Some(source))?,
    )?;
    let revision = revision(
        "workflow-immutable-task-input-path",
        vec![
            signal,
            consume,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("signal-consume", "signal", "out", "consume", "in")?,
            data_edge(
                "signal-payload-consume",
                "signal",
                "payload",
                "consume",
                "input",
            )?,
            control_edge("consume-done", "consume", "out", "done", "in")?,
        ],
    )?;
    let run = RunId::new("run-immutable-task-input-path")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    assert_eq!(
        harness.command(
            &run,
            RunCommand::DeliverSignal {
                signal: SignalId::new("signal-immutable-task-input-path")?,
                signal_type: SignalTypeId::new("notify.payload")?,
                correlation: None,
                mode: SignalDeliveryMode::OneShot,
                payload: BoundedJson::new(json!({"present": true}))?,
            },
        )?,
        CommandDisposition::Accepted
    );

    let tick = harness.runtime.tick()?;
    assert_eq!(tick.dispatched, 0);
    assert_eq!(tick.completed, 1);
    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert!(projection.attempts().is_empty());
    assert!(projection.leases().is_empty());
    let history = harness.runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::NodePreDispatchFailed { .. }))
            .count(),
        1
    );
    assert!(!history.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeScheduled { .. } | RunEventKind::LeaseGranted { .. }
    )));
    let head = harness.store.head(&run)?;
    let _ = harness.runtime.tick()?;
    assert_eq!(harness.store.head(&run)?, head);
    Ok(())
}

#[test]
fn oversized_immutable_invocation_is_durably_failed_before_dispatch() -> TestResult {
    let harness = Harness::new("oversized-immutable-invocation")?;
    let schema = item_schema()?;
    let mut work = task("work", "model.generate")?;
    let large_literal = BoundedJson::new(json!("x".repeat(32_000)))?;
    for index in 0..17 {
        work = work.with_data_input(
            PortId::new(format!("input-{index:02}"))?,
            DataPort::input(
                schema.clone(),
                true,
                Some(BindingSource::Literal {
                    value: large_literal.clone(),
                }),
            )?,
        )?;
    }
    let revision = revision(
        "workflow-oversized-immutable-invocation",
        vec![work, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("work-done", "work", "out", "done", "in")?],
    )?;
    let run = RunId::new("run-oversized-immutable-invocation")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;

    let tick = harness.runtime.tick()?;
    assert_eq!(tick.dispatched, 0);
    assert_eq!(tick.completed, 1);
    let projection = harness.runtime.projection(&run)?;
    assert_eq!(
        projection.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Failed)
    );
    assert!(projection.attempts().is_empty());
    assert_eq!(
        harness
            .runtime
            .history(&run)?
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::NodePreDispatchFailed { .. }))
            .count(),
        1
    );
    Ok(())
}
