//! Explicit bounded caller-owned worker threads for durable runtime effects.

use std::{
    collections::BTreeSet,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use milkdrift_capability::InvocationId;
use milkdrift_persistence::PageSize;
use milkdrift_runtime::{EffectAction, EffectExecutionResult, RuntimeService};
use thiserror::Error;

use crate::{CapabilityHost, HostError, ShutdownReport};

/// Fixed worker and queue bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectWorkerConfig {
    /// Threads permitted to enter external invocation effects.
    pub execution_threads: u16,
    /// Already-claimed execution actions retained in memory.
    pub execution_queue: u16,
    /// Cancellation actions retained for the dedicated control worker.
    pub cancellation_queue: u16,
    /// Maximum actions claimed for either queue by one poll.
    pub maximum_claim_page: u16,
}

impl EffectWorkerConfig {
    /// Validates nonzero, deliberately modest process-local bounds.
    pub fn validate(self) -> Result<Self, EffectWorkerError> {
        if self.execution_threads == 0
            || self.execution_threads > 256
            || self.execution_queue == 0
            || self.execution_queue > 4_096
            || self.cancellation_queue == 0
            || self.cancellation_queue > 4_096
            || self.maximum_claim_page == 0
            || self.maximum_claim_page > 1_024
        {
            return Err(EffectWorkerError::InvalidConfig);
        }
        Ok(self)
    }
}

/// Policy applied after effect-worker admission is closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectShutdownMode {
    /// Let queued and running work finish until the deadline.
    Drain,
    /// Ask hosted adapters to terminate their live resources, preserving unresolved IDs.
    Cancel,
    /// Do not enter queued effects; retain their durable started state for recovery.
    Retain,
}

/// Bounded result from one explicit queue-admission poll.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EffectPollReport {
    /// Invocation effects accepted into the execution queue.
    pub executions: u32,
    /// Cancellation effects accepted into the control queue.
    pub cancellations: u32,
    /// Cancellation redeliveries already queued or active and therefore suppressed.
    pub duplicate_cancellations: u32,
}

/// Immutable bounded health/read model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectWorkerHealth {
    /// Whether explicit polling still admits actions.
    pub admission_open: bool,
    /// Execution actions waiting in the bounded queue.
    pub queued_executions: usize,
    /// Invocation effects currently executing.
    pub active_executions: usize,
    /// Cancellation actions waiting in the bounded queue.
    pub queued_cancellations: usize,
    /// Cancellation effects currently executing.
    pub active_cancellations: usize,
    /// Completed worker calls.
    pub completed_actions: u64,
    /// Worker calls returning typed errors.
    pub failed_actions: u64,
    /// Panics contained at the worker boundary.
    pub contained_panics: u64,
    /// Bounded last failure summary.
    pub last_failure: Option<String>,
}

/// Result of bounded shutdown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectWorkerShutdown {
    /// True when every owned worker joined before the deadline.
    pub clean: bool,
    /// Exact capability owners left unresolved by forced adapter removal.
    pub unresolved_invocations: Vec<InvocationId>,
    /// Final bounded worker health.
    pub health: EffectWorkerHealth,
}

/// Lifecycle or queue error from the explicit worker owner.
#[derive(Debug, Error)]
pub enum EffectWorkerError {
    /// Worker or queue bounds were zero or excessive.
    #[error("invalid effect-worker configuration")]
    InvalidConfig,
    /// Polling was attempted after admission closed.
    #[error("effect-worker admission is closed")]
    AdmissionClosed,
    /// Runtime effect claiming failed.
    #[error("runtime effect boundary failed: {0}")]
    Runtime(String),
    /// An owned queue disconnected unexpectedly.
    #[error("effect-worker queue disconnected")]
    QueueDisconnected,
    /// Capability-host lifecycle failed.
    #[error(transparent)]
    Host(#[from] HostError),
    /// Worker ownership synchronization failed.
    #[error("effect-worker state is unavailable")]
    StateUnavailable,
}

/// Embeddable owner of fixed caller-created threads and bounded effect queues.
pub struct EffectWorkerHost {
    runtime: Arc<RuntimeService>,
    capability_host: CapabilityHost,
    config: EffectWorkerConfig,
    shared: Arc<WorkerShared>,
    execution_sender: Mutex<Option<SyncSender<EffectAction>>>,
    cancellation_sender: Mutex<Option<SyncSender<EffectAction>>>,
    poll_gate: Mutex<()>,
    joins: Mutex<Vec<JoinHandle<()>>>,
}

impl EffectWorkerHost {
    /// Starts exactly the configured threads; no singleton or async runtime is used.
    pub fn start(
        runtime: Arc<RuntimeService>,
        capability_host: CapabilityHost,
        config: EffectWorkerConfig,
    ) -> Result<Self, EffectWorkerError> {
        let config = config.validate()?;
        let (execution_sender, execution_receiver) =
            sync_channel(usize::from(config.execution_queue));
        let (cancellation_sender, cancellation_receiver) =
            sync_channel(usize::from(config.cancellation_queue));
        let shared = Arc::new(WorkerShared::default());
        let execution_receiver = Arc::new(Mutex::new(execution_receiver));
        let cancellation_receiver = Arc::new(Mutex::new(cancellation_receiver));
        let mut joins = Vec::with_capacity(usize::from(config.execution_threads) + 1);
        for index in 0..config.execution_threads {
            let runtime = runtime.clone();
            let receiver = execution_receiver.clone();
            let worker_shared = shared.clone();
            joins.push(
                thread::Builder::new()
                    .name(format!("milkdrift-effect-{index}"))
                    .spawn(move || execution_worker(runtime, receiver, worker_shared))
                    .map_err(|error| EffectWorkerError::Runtime(bounded(&error.to_string())))?,
            );
        }
        {
            let worker_runtime = runtime.clone();
            let worker_shared = shared.clone();
            joins.push(
                thread::Builder::new()
                    .name("milkdrift-effect-control".to_owned())
                    .spawn(move || {
                        cancellation_worker(worker_runtime, cancellation_receiver, worker_shared);
                    })
                    .map_err(|error| EffectWorkerError::Runtime(bounded(&error.to_string())))?,
            );
        }
        Ok(Self {
            runtime,
            capability_host,
            config,
            shared,
            execution_sender: Mutex::new(Some(execution_sender)),
            cancellation_sender: Mutex::new(Some(cancellation_sender)),
            poll_gate: Mutex::new(()),
            joins: Mutex::new(joins),
        })
    }

    /// Claims only enough actions to fit the exact currently available queue space.
    pub fn poll(&self) -> Result<EffectPollReport, EffectWorkerError> {
        let _poll = self
            .poll_gate
            .lock()
            .map_err(|_error| EffectWorkerError::StateUnavailable)?;
        if !self.shared.admission.load(Ordering::SeqCst) {
            return Err(EffectWorkerError::AdmissionClosed);
        }
        let mut report = EffectPollReport::default();
        let execution_free = usize::from(self.config.execution_queue)
            .saturating_sub(self.shared.queued_executions.load(Ordering::SeqCst));
        let execution_claim = execution_free.min(usize::from(self.config.maximum_claim_page));
        if execution_claim != 0 {
            let actions = self
                .runtime
                .claim_execution_effects(page_size(execution_claim)?)
                .map_err(|error| EffectWorkerError::Runtime(bounded(&error.to_string())))?;
            let sender = self
                .execution_sender
                .lock()
                .map_err(|_error| EffectWorkerError::StateUnavailable)?;
            let sender = sender.as_ref().ok_or(EffectWorkerError::AdmissionClosed)?;
            for action in actions {
                self.shared.queued_executions.fetch_add(1, Ordering::SeqCst);
                if let Err(error) = sender.try_send(action) {
                    self.shared.queued_executions.fetch_sub(1, Ordering::SeqCst);
                    return Err(match error {
                        TrySendError::Full(_action) => EffectWorkerError::Runtime(
                            "execution queue contradicted its bounded capacity accounting"
                                .to_owned(),
                        ),
                        TrySendError::Disconnected(_action) => EffectWorkerError::QueueDisconnected,
                    });
                }
                report.executions = report.executions.saturating_add(1);
            }
        }

        let cancellation_free = usize::from(self.config.cancellation_queue)
            .saturating_sub(self.shared.queued_cancellations.load(Ordering::SeqCst));
        let cancellation_claim = cancellation_free.min(usize::from(self.config.maximum_claim_page));
        if cancellation_claim != 0 {
            let actions = self
                .runtime
                .claim_cancellation_effects(page_size(cancellation_claim)?)
                .map_err(|error| EffectWorkerError::Runtime(bounded(&error.to_string())))?;
            let sender = self
                .cancellation_sender
                .lock()
                .map_err(|_error| EffectWorkerError::StateUnavailable)?;
            let sender = sender.as_ref().ok_or(EffectWorkerError::AdmissionClosed)?;
            let mut pending = self
                .shared
                .pending_cancellations
                .lock()
                .map_err(|_error| EffectWorkerError::StateUnavailable)?;
            for action in actions {
                let invocation = cancellation_invocation(&action)?;
                if !pending.insert(invocation) {
                    report.duplicate_cancellations =
                        report.duplicate_cancellations.saturating_add(1);
                    continue;
                }
                self.shared
                    .queued_cancellations
                    .fetch_add(1, Ordering::SeqCst);
                if let Err(error) = sender.try_send(action) {
                    self.shared
                        .queued_cancellations
                        .fetch_sub(1, Ordering::SeqCst);
                    pending.remove(&cancellation_invocation_from_error(&error));
                    return Err(match error {
                        TrySendError::Full(_action) => EffectWorkerError::Runtime(
                            "cancellation queue contradicted its bounded capacity accounting"
                                .to_owned(),
                        ),
                        TrySendError::Disconnected(_action) => EffectWorkerError::QueueDisconnected,
                    });
                }
                report.cancellations = report.cancellations.saturating_add(1);
            }
        }
        Ok(report)
    }

    /// Returns one bounded lock-free/small-lock health snapshot.
    pub fn health(&self) -> Result<EffectWorkerHealth, EffectWorkerError> {
        let counters = self
            .shared
            .counters
            .lock()
            .map_err(|_error| EffectWorkerError::StateUnavailable)?;
        Ok(EffectWorkerHealth {
            admission_open: self.shared.admission.load(Ordering::SeqCst),
            queued_executions: self.shared.queued_executions.load(Ordering::SeqCst),
            active_executions: self.shared.active_executions.load(Ordering::SeqCst),
            queued_cancellations: self.shared.queued_cancellations.load(Ordering::SeqCst),
            active_cancellations: self.shared.active_cancellations.load(Ordering::SeqCst),
            completed_actions: counters.completed,
            failed_actions: counters.failed,
            contained_panics: counters.panics,
            last_failure: counters.last_failure.clone(),
        })
    }

    /// Stops admission, applies the explicit policy, and joins every worker on clean shutdown.
    pub fn shutdown(
        &self,
        mode: EffectShutdownMode,
        deadline: Duration,
    ) -> Result<EffectWorkerShutdown, EffectWorkerError> {
        self.shared.admission.store(false, Ordering::SeqCst);
        if mode == EffectShutdownMode::Retain {
            self.shared.retain_queued.store(true, Ordering::SeqCst);
        }
        let expires = Instant::now() + deadline;
        let mut unresolved_invocations = Vec::new();
        let mut lifecycle_complete = mode != EffectShutdownMode::Cancel;
        if mode == EffectShutdownMode::Cancel
            && let Some(report) =
                capability_shutdown_before(self.capability_host.clone(), true, expires)?
        {
            unresolved_invocations = report.unresolved_invocations;
            lifecycle_complete = true;
        }
        self.execution_sender
            .lock()
            .map_err(|_error| EffectWorkerError::StateUnavailable)?
            .take();
        self.cancellation_sender
            .lock()
            .map_err(|_error| EffectWorkerError::StateUnavailable)?
            .take();

        while !self.is_idle() && Instant::now() < expires {
            thread::sleep(Duration::from_millis(5));
        }
        let idle = self.is_idle();
        if idle && mode != EffectShutdownMode::Cancel {
            if let Some(report) =
                capability_shutdown_before(self.capability_host.clone(), false, expires)?
            {
                unresolved_invocations.extend(report.unresolved_invocations);
                lifecycle_complete = true;
            } else {
                lifecycle_complete = false;
            }
        }
        if idle {
            let mut joins = self
                .joins
                .lock()
                .map_err(|_error| EffectWorkerError::StateUnavailable)?;
            for join in joins.drain(..) {
                if join.join().is_err() {
                    record_panic(&self.shared);
                }
            }
        }
        let clean = idle && lifecycle_complete;
        Ok(EffectWorkerShutdown {
            clean,
            unresolved_invocations,
            health: self.health()?,
        })
    }

    fn is_idle(&self) -> bool {
        self.shared.queued_executions.load(Ordering::SeqCst) == 0
            && self.shared.active_executions.load(Ordering::SeqCst) == 0
            && self.shared.queued_cancellations.load(Ordering::SeqCst) == 0
            && self.shared.active_cancellations.load(Ordering::SeqCst) == 0
    }
}

fn capability_shutdown_before(
    host: CapabilityHost,
    force: bool,
    expires: Instant,
) -> Result<Option<ShutdownReport>, EffectWorkerError> {
    let remaining = expires.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Ok(None);
    }
    let (sender, receiver) = sync_channel(1);
    thread::Builder::new()
        .name("milkdrift-capability-shutdown".to_owned())
        .spawn(move || {
            let result = if force {
                host.force_shutdown()
            } else {
                host.shutdown()
            };
            let _ = sender.send(result);
        })
        .map_err(|_| EffectWorkerError::StateUnavailable)?;
    match receiver.recv_timeout(remaining) {
        Ok(result) => result.map(Some).map_err(EffectWorkerError::Host),
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => Ok(None),
    }
}

#[derive(Default)]
struct WorkerCounters {
    completed: u64,
    failed: u64,
    panics: u64,
    last_failure: Option<String>,
}

struct WorkerShared {
    admission: AtomicBool,
    retain_queued: AtomicBool,
    queued_executions: AtomicUsize,
    active_executions: AtomicUsize,
    queued_cancellations: AtomicUsize,
    active_cancellations: AtomicUsize,
    pending_cancellations: Mutex<BTreeSet<InvocationId>>,
    counters: Mutex<WorkerCounters>,
}

impl Default for WorkerShared {
    fn default() -> Self {
        Self {
            admission: AtomicBool::new(true),
            retain_queued: AtomicBool::new(false),
            queued_executions: AtomicUsize::new(0),
            active_executions: AtomicUsize::new(0),
            queued_cancellations: AtomicUsize::new(0),
            active_cancellations: AtomicUsize::new(0),
            pending_cancellations: Mutex::new(BTreeSet::new()),
            counters: Mutex::new(WorkerCounters::default()),
        }
    }
}

fn execution_worker(
    runtime: Arc<RuntimeService>,
    receiver: Arc<Mutex<Receiver<EffectAction>>>,
    shared: Arc<WorkerShared>,
) {
    loop {
        let action = match receiver.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_error) => return,
        };
        let Ok(action) = action else {
            return;
        };
        shared.queued_executions.fetch_sub(1, Ordering::SeqCst);
        if shared.retain_queued.load(Ordering::SeqCst) {
            continue;
        }
        shared.active_executions.fetch_add(1, Ordering::SeqCst);
        let _active = ActiveCounter(&shared.active_executions);
        execute_contained(&runtime, action, &shared);
    }
}

fn cancellation_worker(
    runtime: Arc<RuntimeService>,
    receiver: Arc<Mutex<Receiver<EffectAction>>>,
    shared: Arc<WorkerShared>,
) {
    loop {
        let action = match receiver.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_error) => return,
        };
        let Ok(action) = action else {
            return;
        };
        shared.queued_cancellations.fetch_sub(1, Ordering::SeqCst);
        let invocation = cancellation_invocation(&action).ok();
        if shared.retain_queued.load(Ordering::SeqCst) {
            remove_pending_cancellation(&shared, invocation.as_ref());
            continue;
        }
        shared.active_cancellations.fetch_add(1, Ordering::SeqCst);
        {
            let _active = ActiveCounter(&shared.active_cancellations);
            execute_contained(&runtime, action, &shared);
        }
        remove_pending_cancellation(&shared, invocation.as_ref());
    }
}

fn execute_contained(runtime: &RuntimeService, action: EffectAction, shared: &WorkerShared) {
    match catch_unwind(AssertUnwindSafe(|| runtime.execute_effect(action))) {
        Ok(Ok(
            EffectExecutionResult::Completed { .. }
            | EffectExecutionResult::Uncertain { .. }
            | EffectExecutionResult::CancellationAcknowledged
            | EffectExecutionResult::CancellationDeferred,
        )) => record_complete(shared),
        Ok(Err(error)) => record_failure(shared, &error.to_string()),
        Err(_panic) => record_panic(shared),
    }
}

struct ActiveCounter<'a>(&'a AtomicUsize);

impl Drop for ActiveCounter<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn record_complete(shared: &WorkerShared) {
    if let Ok(mut counters) = shared.counters.lock() {
        counters.completed = counters.completed.saturating_add(1);
    }
}

fn record_failure(shared: &WorkerShared, failure: &str) {
    if let Ok(mut counters) = shared.counters.lock() {
        counters.failed = counters.failed.saturating_add(1);
        counters.last_failure = Some(bounded(failure));
    }
}

fn record_panic(shared: &WorkerShared) {
    if let Ok(mut counters) = shared.counters.lock() {
        counters.panics = counters.panics.saturating_add(1);
        counters.last_failure = Some("effect worker contained a panic".to_owned());
    }
}

fn remove_pending_cancellation(shared: &WorkerShared, invocation: Option<&InvocationId>) {
    if let (Some(invocation), Ok(mut pending)) = (invocation, shared.pending_cancellations.lock()) {
        pending.remove(invocation);
    }
}

fn cancellation_invocation(action: &EffectAction) -> Result<InvocationId, EffectWorkerError> {
    match action {
        EffectAction::Cancel(dispatch) => Ok(dispatch.request().invocation().clone()),
        EffectAction::Execute(_dispatch) => Err(EffectWorkerError::Runtime(
            "runtime returned an execution action to the cancellation-only claim".to_owned(),
        )),
    }
}

fn cancellation_invocation_from_error(error: &TrySendError<EffectAction>) -> InvocationId {
    let action = match error {
        TrySendError::Full(action) | TrySendError::Disconnected(action) => action,
    };
    match action {
        EffectAction::Cancel(dispatch) => dispatch.request().invocation().clone(),
        EffectAction::Execute(dispatch) => dispatch.request().invocation().clone(),
    }
}

fn page_size(value: usize) -> Result<PageSize, EffectWorkerError> {
    let value = u32::try_from(value)
        .map_err(|_error| EffectWorkerError::Runtime("claim page overflow".to_owned()))?;
    PageSize::new(value).map_err(|error| EffectWorkerError::Runtime(bounded(&error.to_string())))
}

fn bounded(value: &str) -> String {
    if value.len() <= 512 {
        return value.to_owned();
    }
    let mut end = 512;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
