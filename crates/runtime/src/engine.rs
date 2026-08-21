//! Synchronous, headless orchestration over the durable runtime ports.
//!
//! The service deliberately owns no thread, task, polling loop, or hidden mutable
//! projection.  Callers drive it with bounded command, scheduler, and recovery calls;
//! every decision which can affect replay is committed before an executor is entered.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
};

use milkdrift_blueprint::NodeId;
use milkdrift_persistence::{
    ActorRef, ArtifactStore, AtomicRunCommitOutcome, AttemptId, CommandResultDocument,
    IntegrityScanRequest, MAX_PAGE_SIZE, NodeExecutionId, PageSize, PersistenceError, Reason,
    RepeatContinuationDecision, RevisionStore, RunEventEnvelope, RunJournal, RunQueryStore,
    RunSequence, RunSummaryCursor, RunnableCursor, SnapshotDocument, SnapshotId, SnapshotStore,
    StorageAdmin, StorageHealth, StorageSchemaCompatibility, TimestampMillis, WorkerId,
    WorkspaceStore,
};
use milkdrift_workspace::{BranchId, RunId, ScopeReference, SubworkflowId};
use tracing::{debug, info, info_span, warn};

use crate::projection::RunProjection;
use crate::query::{
    RUN_PROJECTION_SNAPSHOT_SCHEMA_V1, encode_projection_snapshot, load_bounded_history,
    project_from_latest_snapshot,
};
use crate::{
    BoundaryClock, IdGenerator, RetryPolicy, RunCommand, RunCommandDocument, RuntimeError,
    SchedulerLimits, TaskExecutor,
};

const STRUCTURED_EVENT_SOFT_LIMIT: usize = milkdrift_persistence::MAX_EVENTS_PER_COMMIT - 192;
const PROJECTION_SNAPSHOT_INTERVAL_EVENTS: u64 = 128;
// Invocation requests larger than half an event document are rejected before any
// attempt/lease fact is committed. The remaining half covers the enclosing event,
// every maximum-length durable identity, and future schema-v1-compatible fields.
const MAX_DURABLE_INVOCATION_REQUEST_BYTES: usize =
    milkdrift_persistence::MAX_EVENT_DOCUMENT_BYTES / 2;

/// One object-safe durable owner used by the headless runtime.
pub trait RuntimeStore:
    RevisionStore
    + RunJournal
    + RunQueryStore
    + WorkspaceStore
    + SnapshotStore
    + ArtifactStore
    + StorageAdmin
{
}

impl<T> RuntimeStore for T where
    T: RevisionStore
        + RunJournal
        + RunQueryStore
        + WorkspaceStore
        + SnapshotStore
        + ArtifactStore
        + StorageAdmin
{
}

/// Startup progress for a runtime handle.
///
/// Admission is a separate reversible gate. A runtime may be fully recovered but
/// temporarily closed for shutdown; it may never be reopened before reaching
/// [`Self::RecoveryCompleted`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RuntimeStartupState {
    /// The storage schema is current, but integrity and restart recovery have not run.
    OpenedClosed = 0,
    /// A complete read-only integrity scan succeeded.
    IntegrityVerified = 1,
    /// Nonterminal runs were recovered to a bounded fixed point.
    RecoveryCompleted = 2,
}

impl RuntimeStartupState {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::OpenedClosed,
            1 => Self::IntegrityVerified,
            _ => Self::RecoveryCompleted,
        }
    }
}

/// Bounded scheduler and recovery policy owned by one service instance.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    worker: WorkerId,
    internal_actor: ActorRef,
    lease_duration_ms: u64,
    maximum_tick_items: u16,
    scheduler_limits: SchedulerLimits,
    retry_policy: RetryPolicy,
}

impl RuntimeConfig {
    /// Constructs a service policy.  A tick is deliberately capped at 1,024 items.
    pub fn new(
        worker: WorkerId,
        internal_actor: ActorRef,
        lease_duration_ms: u64,
        maximum_tick_items: u16,
        scheduler_limits: SchedulerLimits,
        retry_policy: RetryPolicy,
    ) -> Result<Self, RuntimeError> {
        if lease_duration_ms == 0
            || maximum_tick_items == 0
            || u32::from(maximum_tick_items) > MAX_PAGE_SIZE
        {
            return Err(RuntimeError::Scheduling(format!(
                "lease duration and scheduler tick bound must be non-zero; tick bound is at most {MAX_PAGE_SIZE}"
            )));
        }
        Ok(Self {
            worker,
            internal_actor,
            lease_duration_ms,
            maximum_tick_items,
            scheduler_limits,
            retry_policy,
        })
    }

    /// Worker/controller identity recorded on leases and recovery passes.
    #[must_use]
    pub const fn worker(&self) -> &WorkerId {
        &self.worker
    }

    /// Maximum synchronous work items considered by one scheduler/recovery call.
    #[must_use]
    pub const fn maximum_tick_items(&self) -> u16 {
        self.maximum_tick_items
    }
}

/// Durable outcome of a submitted command.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandExecution {
    result: CommandResultDocument,
    replayed: bool,
}

impl CommandExecution {
    /// Exact persistence-owned result, including durable rejection detail.
    #[must_use]
    pub const fn result(&self) -> &CommandResultDocument {
        &self.result
    }

    /// Whether this call returned the byte-identical prior idempotency result.
    #[must_use]
    pub const fn replayed(&self) -> bool {
        self.replayed
    }
}

/// Bounded observations from one explicit scheduler call.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SchedulerTickResult {
    /// Runnable entries examined after fair interleaving.
    pub examined: u32,
    /// Dispatch leases made durable.
    pub dispatched: u32,
    /// Executor batches fully incorporated.
    pub completed: u32,
    /// Candidates skipped because admission was closed or a limit was reached.
    pub deferred: u32,
    /// Attempts conservatively retained as uncertain.
    pub uncertain: u32,
}

/// Bounded observations from one explicit external-effect host call.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EffectTickResult {
    /// Durable effects claimed from already committed leases/cancellation intents.
    pub claimed: u32,
    /// Invocation and heartbeat observations durably incorporated.
    pub observations: u32,
    /// Invocations whose terminal observation became durable.
    pub completed: u32,
    /// Invocations retained because the external boundary returned without a terminal fact.
    pub uncertain: u32,
    /// Cancellation requests durably acknowledged.
    pub cancellations: u32,
    /// Cancellation requests whose adapter boundary failed and remain eligible for redelivery.
    pub cancellation_deferred: u32,
}

/// Bounded observations from one explicit restart-recovery call.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecoveryResult {
    /// Nonterminal runs whose authoritative history was examined.
    pub runs_examined: u32,
    /// Expired leases classified durably.
    pub expired_leases: u32,
    /// Safe redispatch candidates.
    pub retryable: u32,
    /// External outcomes retained for explicit authority.
    pub uncertain: u32,
}

/// Immutable adapter-neutral health observation for a headless runtime instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeHealth {
    /// Boundary-clock observation shared with the storage health probe.
    pub observed_at: TimestampMillis,
    /// Whether this process currently admits create/start/resume/dispatch work.
    pub accepting_admission: bool,
    /// Completed startup gate. Admission cannot be reopened before recovery completes.
    pub startup_state: RuntimeStartupState,
    /// Configured worker/controller identity.
    pub worker: WorkerId,
    /// Durable adapter health and schema facts.
    pub storage: StorageHealth,
}

/// Synchronous durable runtime.  It starts no background work and owns no mutable
/// execution truth outside the injected store. One instance serializes scheduler and
/// recovery admission; multiple instances use the store's opaque active-lease catalog
/// witness so each lease grant atomically conflicts when another instance changed the
/// global admission snapshot.
pub struct RuntimeService {
    store: Arc<dyn RuntimeStore>,
    executor: Arc<dyn TaskExecutor>,
    clock: Arc<dyn BoundaryClock>,
    ids: Arc<dyn IdGenerator>,
    config: RuntimeConfig,
    accepting_admission: AtomicBool,
    startup_state: AtomicU8,
    startup_gate: Mutex<()>,
    scheduler_gate: Mutex<()>,
    runnable_cursor: Mutex<Option<RunnableCursor>>,
    recovery_cursor: Mutex<Option<RunSummaryCursor>>,
    structured_cursor: Mutex<Option<RunSummaryCursor>>,
    reconciliation_cursor: Mutex<Option<RunSummaryCursor>>,
    child_cursor: Mutex<Option<RunSummaryCursor>>,
    cancellation_cursor: Mutex<Option<RunSummaryCursor>>,
    structured_eligible_cursors: Mutex<BTreeMap<RunId, NodeExecutionId>>,
    structured_branch_cursors: Mutex<BTreeMap<RunId, BranchId>>,
    recovery_attempt_cursors: Mutex<BTreeMap<RunId, AttemptId>>,
    child_subworkflow_cursors: Mutex<BTreeMap<RunId, SubworkflowId>>,
    reconciliation_restart_cursors: Mutex<BTreeMap<RunId, (NodeId, ScopeReference)>>,
    cancellation_branch_cursors: Mutex<BTreeMap<RunId, BranchId>>,
    cancellation_subworkflow_cursors: Mutex<BTreeMap<RunId, SubworkflowId>>,
    cancellation_execution_cursors: Mutex<BTreeMap<RunId, NodeExecutionId>>,
    effect_claim_gate: Mutex<()>,
    structured_scan_budget_active: AtomicBool,
    structured_scan_budget: AtomicUsize,
}

mod command_planning;
mod completion;
mod dispatch;
mod effects;
mod reconciliation;
mod recovery;
mod scheduling;
mod state;
mod structured;
mod support;
mod workspace;

pub use effects::EffectExecutionResult;
use support::{command_kind_name, durable_rejection};

impl RuntimeService {
    /// Opens, integrity-checks, recovers, and only then admits work.
    ///
    /// Hosts that need to expose startup progress may call [`Self::open_closed`] and
    /// [`Self::initialize_startup`] explicitly. No constructor returns an admitting
    /// handle before both gates complete.
    pub fn new(
        store: Arc<dyn RuntimeStore>,
        executor: Arc<dyn TaskExecutor>,
        clock: Arc<dyn BoundaryClock>,
        ids: Arc<dyn IdGenerator>,
        config: RuntimeConfig,
    ) -> Result<Self, RuntimeError> {
        let service = Self::open_closed(store, executor, clock, ids, config)?;
        service.initialize_startup()?;
        Ok(service)
    }

    /// Opens a schema-compatible runtime with admission closed.
    ///
    /// This is the only intentionally uninitialized constructor. The returned
    /// handle may serve health/startup queries, but create/start/resume/dispatch
    /// work remains rejected until [`Self::initialize_startup`] succeeds.
    pub fn open_closed(
        store: Arc<dyn RuntimeStore>,
        executor: Arc<dyn TaskExecutor>,
        clock: Arc<dyn BoundaryClock>,
        ids: Arc<dyn IdGenerator>,
        config: RuntimeConfig,
    ) -> Result<Self, RuntimeError> {
        let schema = store.schema_info()?;
        match schema.compatibility {
            StorageSchemaCompatibility::Current => {}
            StorageSchemaCompatibility::MigrationRequired => {
                return Err(PersistenceError::MigrationRequired {
                    found: schema.stored_version,
                    target: schema.current_version,
                }
                .into());
            }
            StorageSchemaCompatibility::FutureUnsupported => {
                return Err(PersistenceError::UnsupportedVersion {
                    document: "storage",
                    found: schema.stored_version,
                    supported: schema.current_version,
                }
                .into());
            }
        }
        Ok(Self {
            store,
            executor,
            clock,
            ids,
            config,
            accepting_admission: AtomicBool::new(false),
            startup_state: AtomicU8::new(RuntimeStartupState::OpenedClosed as u8),
            startup_gate: Mutex::new(()),
            scheduler_gate: Mutex::new(()),
            runnable_cursor: Mutex::new(None),
            recovery_cursor: Mutex::new(None),
            structured_cursor: Mutex::new(None),
            reconciliation_cursor: Mutex::new(None),
            child_cursor: Mutex::new(None),
            cancellation_cursor: Mutex::new(None),
            structured_eligible_cursors: Mutex::new(BTreeMap::new()),
            structured_branch_cursors: Mutex::new(BTreeMap::new()),
            recovery_attempt_cursors: Mutex::new(BTreeMap::new()),
            child_subworkflow_cursors: Mutex::new(BTreeMap::new()),
            reconciliation_restart_cursors: Mutex::new(BTreeMap::new()),
            cancellation_branch_cursors: Mutex::new(BTreeMap::new()),
            cancellation_subworkflow_cursors: Mutex::new(BTreeMap::new()),
            cancellation_execution_cursors: Mutex::new(BTreeMap::new()),
            effect_claim_gate: Mutex::new(()),
            structured_scan_budget_active: AtomicBool::new(false),
            structured_scan_budget: AtomicUsize::new(0),
        })
    }

    fn verify_complete_integrity(&self) -> Result<(), RuntimeError> {
        let limit = PageSize::new(MAX_PAGE_SIZE)?;
        let mut cursor = None;
        loop {
            let result = self.store.scan_integrity(IntegrityScanRequest {
                limit,
                verify_artifact_content: true,
                cursor: cursor.clone(),
            })?;
            if !result.failures.is_empty() {
                let details = result
                    .failures
                    .iter()
                    .take(8)
                    .map(|failure| {
                        format!(
                            "{}: {}",
                            failure.component.as_str(),
                            failure.detail.as_str()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(RuntimeError::InvalidHistory(format!(
                    "startup integrity verification found {} failure(s): {details}",
                    result.failures.len()
                )));
            }
            let Some(next) = result.next_cursor else {
                return Ok(());
            };
            if cursor.as_ref() == Some(&next) {
                return Err(RuntimeError::InvalidHistory(
                    "startup integrity cursor did not advance".to_owned(),
                ));
            }
            cursor = Some(next);
        }
    }

    fn recover_startup_to_completion(&self) -> Result<(), RuntimeError> {
        loop {
            let _ = self.recover()?;
            let page_complete = self
                .recovery_cursor
                .lock()
                .map_err(|_error| {
                    RuntimeError::Scheduling(
                        "runtime recovery pagination cursor lock is poisoned".to_owned(),
                    )
                })?
                .is_none();
            let attempts_complete = self
                .recovery_attempt_cursors
                .lock()
                .map_err(|_error| {
                    RuntimeError::Scheduling(
                        "runtime recovery attempt cursor lock is poisoned".to_owned(),
                    )
                })?
                .is_empty();
            if page_complete && attempts_complete {
                return Ok(());
            }
        }
    }

    pub(super) fn should_checkpoint_projection(
        &self,
        previous: RunSequence,
        current: &RunProjection,
    ) -> bool {
        current.sequence().get() / PROJECTION_SNAPSHOT_INTERVAL_EVENTS
            > previous.get() / PROJECTION_SNAPSHOT_INTERVAL_EVENTS
            || current.lifecycle().is_completed()
    }

    pub(super) fn persist_projection_snapshot(
        &self,
        run: &RunId,
        projection: &RunProjection,
    ) -> Result<(), RuntimeError> {
        if projection.sequence() == RunSequence::ZERO || projection.run_id() != Some(run) {
            return Err(RuntimeError::InvalidHistory(
                "projection snapshot requires a non-empty matching aggregate".to_owned(),
            ));
        }
        let payload = encode_projection_snapshot(projection)?;
        let history_digest = self.store.history_digest(run, projection.sequence())?;
        let mut identity = blake3::Hasher::new();
        identity.update(b"milkdrift.runtime-projection-snapshot.v1\0");
        identity.update(run.as_str().as_bytes());
        identity.update(&projection.sequence().get().to_be_bytes());
        let snapshot = SnapshotId::new(format!(
            "projection-{}",
            &identity.finalize().to_hex().as_str()[..32]
        ))?;
        self.store.put_snapshot(&SnapshotDocument::new(
            snapshot,
            run.clone(),
            projection.sequence(),
            history_digest,
            RUN_PROJECTION_SNAPSHOT_SCHEMA_V1,
            payload,
        )?)?;
        Ok(())
    }

    /// Builds a complete command using the injected clock and identity boundary.
    pub fn command(
        &self,
        run: RunId,
        actor: ActorRef,
        expected_sequence: RunSequence,
        reason: Reason,
        evidence: Vec<milkdrift_persistence::EvidenceReference>,
        command: RunCommand,
    ) -> Result<RunCommandDocument, RuntimeError> {
        RunCommandDocument::new(
            self.next_command_id()?,
            run,
            actor,
            expected_sequence,
            self.clock.now()?,
            reason,
            evidence,
            command,
        )
    }

    /// Loads the latest verified operational checkpoint and replays only its
    /// authoritative tail. Complete history remains available from the journal.
    pub fn projection(&self, run: &RunId) -> Result<RunProjection, RuntimeError> {
        project_from_latest_snapshot(self.store.as_ref(), run)
    }

    /// Reads at most one persistence page of checksummed history for diagnostics/tests.
    ///
    /// Runtime decisions use a paged fold and never call this materializing helper.
    /// Histories above [`MAX_PAGE_SIZE`] return a bounds error rather than truncating.
    pub fn history(&self, run: &RunId) -> Result<Vec<RunEventEnvelope>, RuntimeError> {
        load_bounded_history(self.store.as_ref(), run, PageSize::new(MAX_PAGE_SIZE)?)
    }

    /// Closes new create/start/resume and dispatch admission.  Typed reports and
    /// cancellation/recovery commands remain accepted so already durable work can drain.
    pub fn begin_shutdown(&self) {
        self.accepting_admission.store(false, Ordering::SeqCst);
    }

    /// Runs complete integrity verification and restart recovery before admission.
    ///
    /// The operation is idempotent. A failure leaves admission closed and preserves
    /// the last successfully completed startup stage for an explicit retry.
    pub fn initialize_startup(&self) -> Result<(), RuntimeError> {
        let _guard = self.startup_gate.lock().map_err(|_error| {
            RuntimeError::Scheduling("runtime startup coordination lock is poisoned".to_owned())
        })?;
        self.accepting_admission.store(false, Ordering::SeqCst);

        if self.startup_state() == RuntimeStartupState::OpenedClosed {
            self.verify_complete_integrity()?;
            self.startup_state.store(
                RuntimeStartupState::IntegrityVerified as u8,
                Ordering::SeqCst,
            );
        }
        if self.startup_state() == RuntimeStartupState::IntegrityVerified {
            self.recover_startup_to_completion()?;
            self.startup_state.store(
                RuntimeStartupState::RecoveryCompleted as u8,
                Ordering::SeqCst,
            );
        }
        self.accepting_admission.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Re-opens admission only after integrity and restart recovery completed.
    pub fn resume_admission(&self) -> Result<(), RuntimeError> {
        if self.startup_state() != RuntimeStartupState::RecoveryCompleted {
            return Err(RuntimeError::Scheduling(
                "runtime admission cannot open before integrity verification and recovery complete"
                    .to_owned(),
            ));
        }
        self.accepting_admission.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Last successfully completed startup stage.
    #[must_use]
    pub fn startup_state(&self) -> RuntimeStartupState {
        RuntimeStartupState::from_u8(self.startup_state.load(Ordering::SeqCst))
    }

    /// Current in-process admission gate; durable run state remains authoritative.
    #[must_use]
    pub fn is_accepting_admission(&self) -> bool {
        self.accepting_admission.load(Ordering::SeqCst)
    }

    /// Reads one immutable runtime/storage health snapshot without mutating state.
    pub fn health(&self) -> Result<RuntimeHealth, RuntimeError> {
        let observed_at = self.clock.now()?;
        Ok(RuntimeHealth {
            observed_at,
            accepting_admission: self.is_accepting_admission(),
            startup_state: self.startup_state(),
            worker: self.config.worker.clone(),
            storage: self.store.health(observed_at)?,
        })
    }

    /// Validates, projects, and crash-atomically commits one versioned command.
    /// State-dependent rejection is itself a durable idempotent result.
    pub fn handle_command(
        &self,
        command: &RunCommandDocument,
    ) -> Result<CommandExecution, RuntimeError> {
        let span = info_span!(
            "runtime.command",
            run = %command.run_id(),
            command = %command.command_id(),
            expected_sequence = command.expected_sequence().get(),
            command_type = command_kind_name(command.command()),
        );
        let _entered = span.enter();
        let receipt = command.receipt()?;
        if let Some(prior) = self
            .store
            .command_result(command.run_id(), command.command_id())?
        {
            if prior.command_fingerprint() != receipt.fingerprint() {
                return Err(PersistenceError::IdempotencyConflict {
                    run: command.run_id().clone(),
                    command: command.command_id().clone(),
                    existing: prior.command_fingerprint().clone(),
                    supplied: receipt.fingerprint().clone(),
                }
                .into());
            }
            debug!(
                resulting_sequence = prior.resulting_sequence().get(),
                "returning durable idempotent command result"
            );
            return Ok(CommandExecution {
                result: prior,
                replayed: true,
            });
        }

        let projection = self.projection(command.run_id())?;
        if projection.sequence() != command.expected_sequence() {
            return Err(PersistenceError::SequenceConflict {
                run: command.run_id().clone(),
                expected: command.expected_sequence(),
                actual: projection.sequence(),
            }
            .into());
        }

        let planned = if !self.command_allowed_while_draining(command.command()) {
            Err(RuntimeError::InvalidTransition(
                "runtime admission is closed for graceful shutdown".to_owned(),
            ))
        } else {
            self.plan_command(command, &projection)
        };
        let outcome = match planned {
            Ok(plan) => self.commit_accepted(command, receipt, projection, plan)?,
            Err(error) if durable_rejection(&error) => {
                warn!(reason = %error, "command rejected durably");
                self.commit_rejected(command, receipt, &error.to_string())?
            }
            Err(error) => return Err(error),
        };
        let (result, replayed) = match outcome {
            AtomicRunCommitOutcome::Committed(result) => (result, false),
            AtomicRunCommitOutcome::Replayed(result) => (result, true),
        };
        info!(
            replayed,
            disposition = ?result.disposition(),
            resulting_sequence = result.resulting_sequence().get(),
            "command result is durable"
        );
        Ok(CommandExecution { result, replayed })
    }

    /// Alias emphasizing that command execution means durable transition handling,
    /// never direct executor mutation.
    pub fn execute_command(
        &self,
        command: &RunCommandDocument,
    ) -> Result<CommandExecution, RuntimeError> {
        self.handle_command(command)
    }

    fn command_allowed_while_draining(&self, command: &RunCommand) -> bool {
        self.is_accepting_admission()
            || !matches!(
                command,
                RunCommand::CreateRun { .. }
                    | RunCommand::StartRun
                    | RunCommand::ResumeRun
                    | RunCommand::RequestRevisionAdoption { .. }
                    | RunCommand::ApplyReconciliation { .. }
                    | RunCommand::DecideRepeatContinuation {
                        outcome: RepeatContinuationDecision::Approved,
                        ..
                    }
            )
    }
}
