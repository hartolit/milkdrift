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
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    AuthorityOperation, DecisionReasonCode, GrantDigest, GrantId, PolicyId,
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
    ErrorClass, InvocationAdmissionEnvelope, InvocationEvent, InvocationEventKind,
    InvocationFailure, InvocationId, InvocationRequest, InvocationTerminal,
    InvocationValueReference, OperationId, ResolvedCapabilitySnapshot, SchemaId, SideEffectClass,
    TerminalStatus,
};
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactStore, AtomicRunCommitRequest, AuthorityDecision,
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
    EffectExecutionResult, ExecutionDispatch, ExecutionReporter, ExecutorError, IdGenerator,
    IterationState, LeaseState, ManualClock, NodeExecutionState, PreparedExecution,
    ResolvedCapability, RetryPolicy, RunCommand, RunLifecycle, RuntimeConfig, RuntimeError,
    RuntimeService, RuntimeStartupState, SchedulerLimits, SequentialIdGenerator, TaskExecutor,
    WorkerReport,
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

fn prepared_test_entry<'a>(
    dispatch: &ExecutionDispatch,
    entry: impl FnOnce(&ExecutionDispatch, &dyn ExecutionReporter) -> Result<(), ExecutorError>
    + Send
    + 'a,
) -> PreparedExecution<'a> {
    PreparedExecution::new(
        dispatch,
        InvocationAdmissionEnvelope::not_applicable(),
        entry,
    )
}
type ClosedRuntimeFixture = (
    Arc<RedbStore>,
    Arc<ManualClock>,
    Arc<DispatchCountingExecutor>,
    RuntimeService,
);

const NOW: u64 = 10_000;

struct TestAuthorityEvaluator;

struct EntryDenyAuthorityEvaluator {
    invoke_evaluations: AtomicUsize,
}

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
                units: Some(u64::MAX),
                concurrency: Some(u32::MAX),
            },
            SideEffectClass::Unknown,
        )
    }
}

impl AuthorityEvaluator for EntryDenyAuthorityEvaluator {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        let deny_entry = request.operation == AuthorityOperation::InvokeCapability
            && self.invoke_evaluations.fetch_add(1, Ordering::SeqCst) == 2;
        let reasons = if deny_entry {
            vec![DecisionReasonCode::GrantNotFound]
        } else {
            vec![DecisionReasonCode::Allowed]
        };
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.deny-entry")?,
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
        GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
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

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(prepared_test_entry(dispatch, move |dispatch, reporter| {
            self.dispatches.fetch_add(1, Ordering::SeqCst);
            let skipped_first = InvocationEvent::new(
                dispatch.request().invocation().clone(),
                2,
                InvocationEventKind::Progress {
                    message: "invalid sequence fixture".to_owned(),
                    completed_units: None,
                    total_units: None,
                },
            )?;
            let _disposition = reporter.invocation(skipped_first)?;
            Ok(())
        }))
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

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        let prepared = self.resolver.prepare_exact_entry(dispatch)?;
        let envelope = prepared.admission_envelope().clone();
        Ok(PreparedExecution::new(
            dispatch,
            envelope,
            move |dispatch, reporter| {
                self.dispatches.fetch_add(1, Ordering::SeqCst);
                prepared.enter(dispatch, reporter)
            },
        ))
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

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(prepared_test_entry(dispatch, move |dispatch, reporter| {
            {
                let (lock, entered) = &self.execute_entries;
                let mut count = lock.lock().map_err(|_| {
                    ExecutorError::Boundary("execute count lock poisoned".to_owned())
                })?;
                *count = count.saturating_add(1);
                entered.notify_all();
            }
            let (lock, released) = &self.released;
            let mut permit = lock.lock().map_err(|_| {
                ExecutorError::Boundary("admission release lock poisoned".to_owned())
            })?;
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
            let _disposition = reporter.invocation(event)?;
            Ok(())
        }))
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

struct PlanIdFailureGenerator {
    inner: SequentialIdGenerator,
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

impl PlanIdFailureGenerator {
    fn new(prefix: &str) -> TestResult<Self> {
        Ok(Self {
            inner: SequentialIdGenerator::new(prefix, 1)?,
        })
    }
}

impl IdGenerator for PlanIdFailureGenerator {
    fn next(&self, kind: &'static str) -> Result<String, RuntimeError> {
        if kind == "reconciliation-plan" {
            return Err(RuntimeError::Persistence(
                milkdrift_persistence::PersistenceError::Corruption(
                    "scripted transient plan identity failure".to_owned(),
                ),
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

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(prepared_test_entry(dispatch, move |dispatch, reporter| {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(ExecutorError::Boundary(
                    "executor disconnected after accepting first dispatch".to_owned(),
                ));
            }
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
            let _disposition = reporter.invocation(event)?;
            Ok(())
        }))
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

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        let prepared = self.resolver.prepare_exact_entry(dispatch)?;
        let envelope = prepared.admission_envelope().clone();
        Ok(PreparedExecution::new(
            dispatch,
            envelope,
            move |dispatch, reporter| {
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
                prepared.enter(dispatch, reporter)
            },
        ))
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

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(prepared_test_entry(
            dispatch,
            move |_dispatch, _reporter| {
                std::panic::resume_unwind(Box::new(
                    "intentional crash after durable invocation start",
                ))
            },
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

    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        Ok(prepared_test_entry(dispatch, move |dispatch, reporter| {
            let blocked = dispatch.request().operation()
                == &*self.blocking_operation.lock().map_err(|_| {
                    ExecutorError::Boundary("blocking operation lock poisoned".to_owned())
                })?;
            if blocked {
                {
                    let (lock, entered) = &self.entered;
                    *lock.lock().map_err(|_| {
                        ExecutorError::Boundary("entered lock poisoned".to_owned())
                    })? = true;
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
                .map_err(|_| {
                    ExecutorError::Boundary("cancellation sequence lock poisoned".to_owned())
                })?
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
            let _disposition = reporter.invocation(event)?;
            Ok(())
        }))
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
        Self::with_descriptor_and_ids(
            prefix,
            retry_policy,
            descriptor,
            Arc::new(SequentialIdGenerator::new(prefix, 1)?),
        )
    }

    fn with_plan_id_failure(prefix: &str) -> TestResult<Self> {
        Self::with_descriptor_and_ids(
            prefix,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
            test_descriptor()?,
            Arc::new(PlanIdFailureGenerator::new(prefix)?),
        )
    }

    fn with_entry_denied(prefix: &str) -> TestResult<Self> {
        Self::with_descriptor_ids_and_authority(
            prefix,
            RetryPolicy::new(1, Vec::new(), 10, 1_000, 0)?,
            test_descriptor()?,
            Arc::new(SequentialIdGenerator::new(prefix, 1)?),
            Arc::new(EntryDenyAuthorityEvaluator {
                invoke_evaluations: AtomicUsize::new(0),
            }),
        )
    }

    fn with_descriptor_and_ids(
        prefix: &str,
        retry_policy: RetryPolicy,
        descriptor: CapabilityDescriptor,
        ids: Arc<dyn IdGenerator>,
    ) -> TestResult<Self> {
        Self::with_descriptor_ids_and_authority(
            prefix,
            retry_policy,
            descriptor,
            ids,
            test_authority(),
        )
    }

    fn with_descriptor_ids_and_authority(
        prefix: &str,
        retry_policy: RetryPolicy,
        descriptor: CapabilityDescriptor,
        ids: Arc<dyn IdGenerator>,
        authority: Arc<dyn AuthorityEvaluator>,
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
            authority,
            clock.clone(),
            ids,
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

#[path = "structured_runtime/builders.rs"]
mod builders;
use builders::*;

#[path = "structured_runtime/causal_context_production.rs"]
mod causal_context_production;
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
