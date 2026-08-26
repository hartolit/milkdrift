//! Black-box structured-runtime evidence using the production redb store.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        Arc, Barrier, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    DecisionReasonCode, GrantId, PolicyId,
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
    ErrorClass, InvocationEvent, InvocationEventKind, InvocationFailure, InvocationId,
    InvocationRequest, InvocationTerminal, InvocationValueReference, OperationId,
    ResolvedCapabilitySnapshot, SchemaId, SideEffectClass, TerminalStatus,
};
use milkdrift_persistence::{
    ActorRef, ArtifactPublicationId, ArtifactStore, AtomicRunCommitRequest, AuthorityDecision,
    BeginArtifactPublication, CommandDisposition, CommandId, CommandReceipt, CommandResultDocument,
    EventId, EventPageQuery, IndexedRunState, IntegrityScanRequest, MAX_INDEX_MUTATIONS_PER_COMMIT,
    MAX_PAGE_SIZE, NodeExecutionId, NodeExecutionMode, NodeOutcome, PageSize, Reason,
    ReconciliationAction, ReconciliationClassification, ReconciliationDecisionId, ReconciliationId,
    ReconciliationPolicy, RecoveryClassification, RepeatContinuationCause,
    RepeatContinuationDecision, RepeatDecisionId, RepeatTerminationReason, RevisionStore,
    RunEventEnvelope, RunEventKind, RunIndexUpdate, RunJournal, RunOutcome, RunQueryStore,
    RunSummaryIndex, SignalDeliveryMode, SignalId, SignalTypeId, SnapshotStore, StorageAdmin,
    TimestampMillis, WorkerId, WorkspaceAccounting, WorkspaceStore,
};
use milkdrift_redb_store::{RedbStore, testing as storage_fault};
use milkdrift_runtime::{
    AttemptState, BoundaryClock, CommandAuthorityClaim, DeterministicExecutor, EffectAction,
    EffectExecutionResult, ExecutionDispatch, ExecutionReportBatch, ExecutorError, IdGenerator,
    IterationState, LeaseState, ManualClock, NodeExecutionState, ResolvedCapability, RetryPolicy,
    RunCommand, RunLifecycle, RuntimeConfig, RuntimeError, RuntimeService, RuntimeStartupState,
    SchedulerLimits, SequentialIdGenerator, TaskExecutor, WorkerReport,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactRetention, ArtifactSensitivity,
    CausalId, CausalReference, ContentDigest, MediaType, RunId, ScopeId, ScopeKind, ScopeReference,
    ValueKey, ValueOrigin, WorkspaceBudget, WorkspaceScope, WorkspaceUsage, WorkspaceValue,
    WorkspaceValueEntry, WorkspaceValueReference,
};
use serde_json::json;
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type PersistenceResult<T> = Result<T, milkdrift_persistence::PersistenceError>;
type ClosedRuntimeFixture = (
    Arc<RedbStore>,
    Arc<ManualClock>,
    Arc<DispatchCountingExecutor>,
    RuntimeService,
);

const NOW: u64 = 10_000;

struct TestAuthorityEvaluator;

impl AuthorityEvaluator for TestAuthorityEvaluator {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.allow-all")?,
            1,
            request.clone(),
            vec![DecisionReasonCode::Allowed],
            AuthorityBudget {
                cost_minor: Some(u64::MAX),
                duration_ms: Some(u64::MAX),
                invocations: Some(u64::MAX),
                artifact_bytes: Some(u64::MAX),
                concurrency: Some(u32::MAX),
            },
            SideEffectClass::Unknown,
        )
    }
}

fn test_authority() -> Arc<dyn AuthorityEvaluator> {
    Arc::new(TestAuthorityEvaluator)
}

fn test_authority_claim() -> TestResult<CommandAuthorityClaim> {
    Ok(CommandAuthorityClaim::new(
        GrantId::new("grant:structured-runtime-test")?,
        1,
        0,
    )?)
}
struct Harness {
    _directory: TempDir,
    store: Arc<RedbStore>,
    clock: Arc<ManualClock>,
    executor: Arc<DeterministicExecutor>,
    runtime: RuntimeService,
}

struct ChildRunCollisionIdGenerator {
    fallback: SequentialIdGenerator,
    child_run: RunId,
}

struct AdvancingClock(AtomicU64);

impl AdvancingClock {
    const fn new(initial: u64) -> Self {
        Self(AtomicU64::new(initial))
    }
}

impl BoundaryClock for AdvancingClock {
    fn now(&self) -> Result<TimestampMillis, RuntimeError> {
        Ok(TimestampMillis::new(self.0.fetch_add(2, Ordering::SeqCst)))
    }
}

impl ChildRunCollisionIdGenerator {
    fn new(prefix: &str, child_run: RunId) -> TestResult<Self> {
        Ok(Self {
            fallback: SequentialIdGenerator::new(prefix, 1)?,
            child_run,
        })
    }
}

impl IdGenerator for ChildRunCollisionIdGenerator {
    fn next(&self, kind: &'static str) -> Result<String, RuntimeError> {
        if kind == "child-run" {
            Ok(self.child_run.as_str().to_owned())
        } else {
            self.fallback.next(kind)
        }
    }
}

struct BlockingExecutor {
    resolver: DeterministicExecutor,
    blocking_operation: Mutex<OperationId>,
    entered: (Mutex<bool>, Condvar),
    released: (Mutex<bool>, Condvar),
    cancellation_requests: AtomicUsize,
    cancellation_sequences: Mutex<BTreeMap<InvocationId, u64>>,
}

struct PanickingExecutor {
    resolver: DeterministicExecutor,
}

struct DispatchCountingExecutor {
    resolver: DeterministicExecutor,
    dispatches: AtomicUsize,
}

struct InvalidReportsCountingExecutor {
    resolver: DeterministicExecutor,
    dispatches: AtomicUsize,
}

impl InvalidReportsCountingExecutor {
    fn new(descriptor: CapabilityDescriptor) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            dispatches: AtomicUsize::new(0),
        }
    }

    fn dispatches(&self) -> usize {
        self.dispatches.load(Ordering::SeqCst)
    }
}

impl TaskExecutor for InvalidReportsCountingExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement, observed_at_unix_ms)
    }

    fn execute(
        &self,
        _dispatch: &ExecutionDispatch,
    ) -> Result<ExecutionReportBatch, ExecutorError> {
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        Err(ExecutorError::InvalidReports(
            "deterministic invalid report fixture".to_owned(),
        ))
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.resolver.cancel(request)
    }
}

impl DispatchCountingExecutor {
    fn new(descriptor: CapabilityDescriptor) -> Self {
        Self {
            resolver: DeterministicExecutor::new(descriptor),
            dispatches: AtomicUsize::new(0),
        }
    }

    fn dispatches(&self) -> usize {
        self.dispatches.load(Ordering::SeqCst)
    }
}

impl TaskExecutor for DispatchCountingExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement, observed_at_unix_ms)
    }

    fn execute(&self, dispatch: &ExecutionDispatch) -> Result<ExecutionReportBatch, ExecutorError> {
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        self.resolver.execute(dispatch)
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.resolver.cancel(request)
    }
}

struct BoundaryFailingExecutor {
    resolver: DeterministicExecutor,
    failures_remaining: AtomicUsize,
    dispatches: Mutex<Vec<RecordedDispatch>>,
}

#[derive(Clone)]
struct RecordedDispatch {
    resolution: ResolvedCapabilitySnapshot,
    request: InvocationRequest,
}

impl RecordedDispatch {
    fn resolution(&self) -> &ResolvedCapabilitySnapshot {
        &self.resolution
    }

    fn request(&self) -> &InvocationRequest {
        &self.request
    }
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

fn wait_for_count(state: &(Mutex<usize>, Condvar), expected: usize, label: &str) -> TestResult {
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
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        let resolved = self.resolver.resolve(requirement, observed_at_unix_ms)?;
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
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement, observed_at_unix_ms)
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
            dispatch.resolution().operation_contract().side_effect(),
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

    fn dispatches(&self) -> TestResult<Vec<RecordedDispatch>> {
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
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement, observed_at_unix_ms)
    }

    fn execute(&self, dispatch: &ExecutionDispatch) -> Result<ExecutionReportBatch, ExecutorError> {
        self.dispatches
            .lock()
            .map_err(|_| ExecutorError::Boundary("dispatch log lock poisoned".to_owned()))?
            .push(RecordedDispatch {
                resolution: dispatch.resolution().clone(),
                request: dispatch.request().clone(),
            });
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
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement, observed_at_unix_ms)
    }

    fn execute(
        &self,
        _dispatch: &ExecutionDispatch,
    ) -> Result<ExecutionReportBatch, ExecutorError> {
        std::panic::resume_unwind(Box::new("intentional crash after durable invocation start"))
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
            blocking_operation: Mutex::new(OperationId::new("model.generate")?),
            entered: (Mutex::new(false), Condvar::new()),
            released: (Mutex::new(false), Condvar::new()),
            cancellation_requests: AtomicUsize::new(0),
            cancellation_sequences: Mutex::new(BTreeMap::new()),
        })
    }

    fn block_operation(&self, operation: OperationId) -> TestResult {
        *self
            .blocking_operation
            .lock()
            .map_err(|_| "blocking operation lock poisoned")? = operation;
        Ok(())
    }

    fn has_entered(&self) -> TestResult<bool> {
        let (lock, _) = &self.entered;
        Ok(*lock.lock().map_err(|_| "entered lock poisoned")?)
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

    fn cancellation_request_sequence(&self, invocation: &InvocationId) -> TestResult<Option<u64>> {
        Ok(self
            .cancellation_sequences
            .lock()
            .map_err(|_| "cancellation sequence lock poisoned")?
            .get(invocation)
            .copied())
    }
}

impl TaskExecutor for BlockingExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement, observed_at_unix_ms)
    }

    fn execute(&self, dispatch: &ExecutionDispatch) -> Result<ExecutionReportBatch, ExecutorError> {
        let blocked = dispatch.request().operation()
            == &*self.blocking_operation.lock().map_err(|_| {
                ExecutorError::Boundary("blocking operation lock poisoned".to_owned())
            })?;
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
        let cancellation_requested = self
            .cancellation_sequences
            .lock()
            .map_err(|_| ExecutorError::Boundary("cancellation sequence lock poisoned".to_owned()))?
            .contains_key(dispatch.request().invocation());
        let terminal = InvocationTerminal::new(
            if cancellation_requested {
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
        self.cancellation_sequences
            .lock()
            .map_err(|_| ExecutorError::Boundary("cancellation sequence lock poisoned".to_owned()))?
            .insert(request.invocation().clone(), request.request_sequence());
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

fn block_first_runnable_operation(
    store: &RedbStore,
    runtime: &RuntimeService,
    run: &RunId,
    executor: &BlockingExecutor,
) -> TestResult {
    let entry = store
        .runnable_page(TimestampMillis::new(NOW), None, PageSize::new(1)?)?
        .entries
        .into_iter()
        .next()
        .ok_or("run has no runnable executor operation")?;
    let projection = runtime.projection(run)?;
    let execution = projection
        .node_executions()
        .get(&entry.execution)
        .ok_or("runnable execution is absent from its projection")?;
    let revision_id = execution.revision();
    let revision = store
        .revision(revision_id)?
        .ok_or("runnable execution governing revision is absent")?;
    let node = revision
        .semantic()
        .nodes()
        .get(execution.node())
        .ok_or("runnable execution node is absent from its revision")?;
    let NodeKind::Task { config } = node.kind() else {
        return Err("first runnable execution is not a task".into());
    };
    executor.block_operation(config.requirement().operation().clone())
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
        let runtime = RuntimeService::new_with_authority(
            store.clone(),
            executor.clone(),
            test_authority(),
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

    fn with_child_run_collision(prefix: &str, child_run: RunId) -> TestResult<Self> {
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
        let runtime = RuntimeService::new_with_authority(
            store.clone(),
            executor.clone(),
            test_authority(),
            clock.clone(),
            Arc::new(ChildRunCollisionIdGenerator::new(prefix, child_run)?),
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
            .handle_authorized_command(&document, &test_authority_claim()?)?
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

fn open_closed_runtime_at(
    root: &Path,
    prefix: &str,
    now: u64,
    maximum_tick_items: u16,
) -> TestResult<ClosedRuntimeFixture> {
    let store = Arc::new(RedbStore::open(root)?);
    let clock = Arc::new(ManualClock::new(now));
    let executor = Arc::new(DispatchCountingExecutor::new(test_descriptor()?));
    let runtime = RuntimeService::open_closed_with_authority(
        store.clone(),
        executor.clone(),
        test_authority(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new(prefix, 1)?),
        RuntimeConfig::new(
            WorkerId::new(format!("worker-{prefix}"))?,
            ActorRef::new(format!("controller:{prefix}"))?,
            30_000,
            maximum_tick_items,
            SchedulerLimits::new(64, 32, 16, 32)?,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
        )?,
    )?;
    Ok((store, clock, executor, runtime))
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
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        executor,
        test_authority(),
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
    Ok(runtime
        .handle_authorized_command(&document, &test_authority_claim()?)?
        .result()
        .disposition())
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
    Ok(runtime
        .handle_authorized_command(&document, &test_authority_claim()?)?
        .result()
        .disposition())
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
    Ok(RuntimeService::new_with_authority(
        store,
        executor,
        test_authority(),
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
        NodeKind::task_direct_inputs(CapabilityRequirement::new(OperationId::new(operation)?))?,
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
            config: ForkConfig::new(BTreeSet::from([inner_left.clone(), inner_right.clone()]))?,
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
            control_edge("outer-fork-b", "outer-fork", "b", "outer-b-task", "in")?,
            control_edge("inner-fork-left", "inner-fork", "left", "inner-left", "in")?,
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
            control_edge("inner-join-tail", "inner-join", "out", "outer-a-tail", "in")?,
            control_edge("outer-a-join", "outer-a-tail", "out", "outer-join", "a-in")?,
            control_edge("outer-b-join", "outer-b-task", "out", "outer-join", "b-in")?,
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

#[path = "structured_runtime/data_integrity.rs"]
mod data_integrity;
#[path = "structured_runtime/fixtures.rs"]
mod fixtures;
#[path = "structured_runtime/lifecycle.rs"]
mod lifecycle;
#[path = "structured_runtime/reconciliation.rs"]
mod reconciliation;
#[path = "structured_runtime/retry_recovery.rs"]
mod retry_recovery;
#[path = "structured_runtime/structured_graph.rs"]
mod structured_graph;

use fixtures::*;
