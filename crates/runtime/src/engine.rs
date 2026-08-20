//! Synchronous, headless orchestration over the durable runtime ports.
//!
//! The service deliberately owns no thread, task, polling loop, or hidden mutable
//! projection.  Callers drive it with bounded command, scheduler, and recovery calls;
//! every decision which can affect replay is committed before an executor is entered.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ops::Bound::{Excluded, Unbounded},
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use milkdrift_blueprint::{
    BindingSource, BlueprintRevision, EdgeKind, JoinPolicy, Node, NodeId, NodeKind, PathSegment,
    PortId, ReducerStrategy, RepeatTermination, RevisionId, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{
    BoundedJson, CancellationRequest, ErrorClass, IdempotencyBehavior, IdempotencyKey,
    InputReference, InvocationEvent, InvocationEventKind, InvocationId, InvocationRequest,
    InvocationTerminal, InvocationValueReference, SideEffectClass, TerminalStatus,
};
use milkdrift_persistence::{
    ActorRef, ArtifactStore, AtomicRunCommitOutcome, AtomicRunCommitRequest, AttemptId,
    AttemptUsage, AuthorityDecision, BoundedDetail, BranchResultReference, CommandDisposition,
    CommandId, CommandReceipt, CommandResultDocument, CurrencyCode, EventId, IndexedRunState,
    IntegrityDigest, JoinRule, LeaseId, LeaseIndexEntry, LeaseIndexMutation, MAX_PAGE_SIZE,
    MAX_RECONCILIATION_PLAN_ITEMS, MAX_REPEAT_CONTINUATION_DECISIONS,
    MAX_WORKSPACE_MUTATIONS_PER_COMMIT, NodeExecutionId, NodeExecutionMode, NodeOutcome, PageSize,
    PersistenceError, Reason, ReconciliationAction, ReconciliationClassification,
    ReconciliationDecisionId, ReconciliationPlanId, RecoveryClassification,
    RepeatContinuationCause, RepeatContinuationDecision, RepeatTerminationReason, RevisionStore,
    RunEventEnvelope, RunEventKind, RunIndexUpdate, RunJournal, RunOutcome, RunQueryStore,
    RunSequence, RunSummaryCursor, RunSummaryIndex, RunnableCursor, RunnableIndexEntry,
    RunnableIndexMutation, SignalDeliveryMode, SnapshotStore, StorageAdmin, StorageHealth,
    StorageSchemaCompatibility, SubworkflowOwnership, TimerId, TimerIndexEntry, TimerIndexMutation,
    TimestampMillis, WaitCondition, WaitSatisfaction, WorkerId, WorkspaceAccounting,
    WorkspaceMutation, WorkspaceStore,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactReference, BranchId, IterationId, RunId, ScopeId, ScopeKind,
    ScopeReference, SubworkflowId, ValueKey, ValueOrigin, ValueVersion, WorkspaceBudget,
    WorkspaceScope, WorkspaceUsage, WorkspaceValue, WorkspaceValueEntry, WorkspaceValueReference,
};
use serde_json::json;
use tracing::{debug, info, info_span, warn};

use crate::projection::{
    AttemptState, BranchState, IterationState, NodeExecutionState, RunLifecycle, RunProjection,
    SubworkflowState, TimerPurpose,
};
use crate::query::{fold_complete_history, load_bounded_history, project_complete_history};
use crate::{
    AdmissionRequest, AdmissionUsage, BoundaryClock, EvaluationContext, ExecutionDispatch,
    ExternalWorkAction, HistoricalExecutionState, IdGenerator, NodeHistory, RetryPolicy,
    RunCommand, RunCommandDocument, RuntimeError, SchedulerLimits, TaskExecutor, WorkerReport,
    evaluate_condition, plan_reconciliation, select_fair_runnable,
};

const STRUCTURED_EVENT_SOFT_LIMIT: usize = milkdrift_persistence::MAX_EVENTS_PER_COMMIT - 192;
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
    reconciliation_restart_cursors:
        Mutex<BTreeMap<RunId, (NodeId, ScopeReference)>>,
    cancellation_branch_cursors: Mutex<BTreeMap<RunId, BranchId>>,
    cancellation_subworkflow_cursors: Mutex<BTreeMap<RunId, SubworkflowId>>,
    cancellation_execution_cursors: Mutex<BTreeMap<RunId, NodeExecutionId>>,
    cancellation_attempt_cursors: Mutex<BTreeMap<RunId, AttemptId>>,
    structured_scan_budget_active: AtomicBool,
    structured_scan_budget: AtomicUsize,
}

impl RuntimeService {
    /// Opens a headless service after refusing non-current physical storage schemas.
    pub fn new(
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
            accepting_admission: AtomicBool::new(true),
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
            cancellation_attempt_cursors: Mutex::new(BTreeMap::new()),
            structured_scan_budget_active: AtomicBool::new(false),
            structured_scan_budget: AtomicUsize::new(0),
        })
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

    /// Reads and purely replays the complete authoritative run history.
    pub fn projection(&self, run: &RunId) -> Result<RunProjection, RuntimeError> {
        project_complete_history(self.store.as_ref(), run)
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

    /// Re-opens admission after the owner has completed restart/recovery coordination.
    pub fn resume_admission(&self) {
        self.accepting_admission.store(true, Ordering::SeqCst);
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

        let projection = project_complete_history(self.store.as_ref(), command.run_id())?;
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

    fn plan_command(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
    ) -> Result<CommandPlan, RuntimeError> {
        match document.command() {
            RunCommand::CreateRun {
                workflow,
                revision,
                root_scope,
                workspace_budget,
                inputs,
            } => self.plan_create_run(
                document,
                projection,
                workflow,
                revision,
                root_scope,
                workspace_budget,
                inputs,
            ),
            RunCommand::StartRun => self.plan_start_run(document, projection),
            RunCommand::PauseRun => {
                require_lifecycle(projection, RunLifecycle::Running, "pause")?;
                Ok(CommandPlan::one(RunEventKind::RunPaused {
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                }))
            }
            RunCommand::ResumeRun => {
                require_lifecycle(projection, RunLifecycle::Paused, "resume")?;
                Ok(CommandPlan::one(RunEventKind::RunResumed {
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                }))
            }
            RunCommand::RequestCancellation => {
                if !matches!(
                    projection.lifecycle(),
                    RunLifecycle::Created | RunLifecycle::Running | RunLifecycle::Paused
                ) {
                    return Err(RuntimeError::InvalidTransition(
                        "only a created, running, or paused run can be cancelled".to_owned(),
                    ));
                }
                Ok(CommandPlan::one(RunEventKind::RunCancellationRequested {
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                }))
            }
            RunCommand::DeliverSignal {
                signal,
                signal_type,
                correlation,
                mode,
                payload,
            } => self.plan_signal(
                document,
                projection,
                signal,
                signal_type,
                correlation.as_ref(),
                *mode,
                payload,
            ),
            RunCommand::FireTimer { timer } => self.plan_timer(document, projection, timer),
            RunCommand::RequestRevisionAdoption {
                reconciliation,
                revision,
                policy,
            } => self.plan_revision_adoption(projection, reconciliation, revision, *policy),
            RunCommand::DecideReconciliation {
                plan,
                decision,
                outcome,
            } => self.plan_reconciliation_decision(document, projection, plan, decision, *outcome),
            RunCommand::ApplyReconciliation { plan } => {
                self.plan_reconciliation_application(document.run_id(), projection, plan)
            }
            RunCommand::DecideRepeatContinuation {
                repeat_execution,
                decision,
                outcome,
                approved_additional_iterations,
            } => {
                let execution = projection
                    .node_executions()
                    .get(repeat_execution)
                    .ok_or_else(|| {
                        RuntimeError::InvalidTransition(
                            "repeat continuation references an unknown execution".to_owned(),
                        )
                    })?;
                let revision = self.revision_for_execution(projection, repeat_execution)?;
                let node = revision
                    .semantic()
                    .nodes()
                    .get(execution.node())
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "repeat continuation node is absent from the revision".to_owned(),
                        )
                    })?;
                let NodeKind::Repeat { config } = node.kind() else {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat execution is not configured to await approval".to_owned(),
                    ));
                };
                if config.termination() != RepeatTermination::AwaitApproval {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat execution is not configured to await approval".to_owned(),
                    ));
                }
                let frontier = projection
                    .iterations()
                    .values()
                    .filter(|iteration| iteration.repeat_execution() == repeat_execution)
                    .max_by_key(|iteration| iteration.iteration_number())
                    .ok_or_else(|| {
                        RuntimeError::InvalidTransition(
                            "repeat continuation has no iteration frontier".to_owned(),
                        )
                    })?;
                if frontier.state() != IterationState::ConditionRecorded(true) {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat continuation requires a true-condition frontier".to_owned(),
                    ));
                }
                let continuation = projection
                    .repeat_continuations()
                    .get(repeat_execution)
                    .ok_or_else(|| {
                        RuntimeError::InvalidTransition(
                            "repeat continuation has no durable authority request".to_owned(),
                        )
                    })?;
                let pending_request = continuation.pending_request().ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "repeat continuation has no pending durable authority request".to_owned(),
                    )
                })?;
                if continuation.is_rejected()
                    || pending_request.frontier_iteration() != frontier.iteration()
                {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat continuation decision is outside its exact authority boundary"
                            .to_owned(),
                    ));
                }
                Ok(CommandPlan::one(RunEventKind::RepeatContinuationDecided {
                    repeat_execution: repeat_execution.clone(),
                    decision: decision.clone(),
                    actor: document.actor().clone(),
                    outcome: *outcome,
                    approved_additional_iterations: *approved_additional_iterations,
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                }))
            }
            RunCommand::ResolveExternalWork {
                attempt,
                decision,
                action,
                remediation_node,
            } => self.plan_external_resolution(
                document,
                projection,
                attempt,
                decision,
                *action,
                remediation_node.as_ref(),
            ),
            RunCommand::WorkerReport { worker, report } => {
                self.plan_worker_report(document, projection, worker, report)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_create_run(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        workflow: &WorkflowId,
        revision_id: &RevisionId,
        root_scope: &WorkspaceScope,
        budget: &WorkspaceBudget,
        inputs: &[WorkspaceValueEntry],
    ) -> Result<CommandPlan, RuntimeError> {
        if projection.lifecycle() != RunLifecycle::Uncreated {
            return Err(RuntimeError::InvalidTransition(
                "run identity already exists".to_owned(),
            ));
        }
        if root_scope.reference().run() != document.run_id() {
            return Err(RuntimeError::InvalidTransition(
                "root workspace scope belongs to another run".to_owned(),
            ));
        }
        let revision = self.load_validated_revision(revision_id, Some(workflow))?;
        let mut references = BTreeSet::new();
        let expected_usage = self.store.workspace_usage(document.run_id())?;
        let mut resulting_usage = expected_usage;
        let mut required_artifacts = BTreeSet::new();
        let declared_inputs = revision.semantic().interface().inputs();
        let mut supplied_fields = BTreeSet::new();
        for input in inputs {
            if input.reference().scope() != root_scope.reference()
                || input.reference().version() != ValueVersion::FIRST
                || !matches!(input.origin(), ValueOrigin::Initial)
            {
                return Err(RuntimeError::InvalidTransition(
                    "run inputs must be initial values in the declared root scope".to_owned(),
                ));
            }
            let field = declared_inputs
                .keys()
                .find(|field| field.as_str() == input.reference().key().as_str())
                .ok_or_else(|| {
                    RuntimeError::InvalidTransition(format!(
                        "run input {} is not declared by the pinned workflow interface",
                        input.reference().key()
                    ))
                })?;
            supplied_fields.insert(field.clone());
            if !references.insert(input.reference().clone()) {
                return Err(RuntimeError::InvalidTransition(
                    "initial workspace value references must be distinct".to_owned(),
                ));
            }
            if let Some(artifact) = input.value().as_artifact() {
                if !self.store.is_committed(artifact)? {
                    return Err(RuntimeError::InvalidTransition(format!(
                        "initial artifact {} is not durably committed",
                        artifact.artifact()
                    )));
                }
                required_artifacts.insert(artifact.clone());
            }
        }
        if let Some(missing) = declared_inputs
            .iter()
            .find(|(field, declaration)| {
                declaration.is_required() && !supplied_fields.contains(*field)
            })
            .map(|(field, _)| field)
        {
            return Err(RuntimeError::InvalidTransition(format!(
                "required workflow input {missing} is absent"
            )));
        }
        let mut newly_referenced_artifacts = BTreeSet::new();
        for artifact in &required_artifacts {
            if !self
                .store
                .is_referenced_by_run(document.run_id(), artifact)?
            {
                resulting_usage = budget
                    .admit_artifact_reference(&resulting_usage, artifact)
                    .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
                newly_referenced_artifacts.insert(artifact.clone());
            }
        }
        for input in inputs {
            resulting_usage = budget
                .admit_value(&resulting_usage, input.value())
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        }
        let mut plan = CommandPlan::one(RunEventKind::RunCreated {
            workflow: workflow.clone(),
            revision: revision_id.clone(),
            revision_digest: revision.content_digest().clone(),
            root_scope: root_scope.clone(),
            workspace_budget: budget.clone(),
            inputs: references.into_iter().collect(),
        });
        plan.workspace.push(WorkspaceMutation::CreateScope {
            scope: root_scope.clone(),
        });
        plan.workspace.extend(
            inputs
                .iter()
                .cloned()
                .map(|entry| WorkspaceMutation::PutValue { entry }),
        );
        plan.creation_usage = Some((expected_usage, resulting_usage, newly_referenced_artifacts));
        plan.required_artifacts.extend(required_artifacts);
        Ok(plan)
    }

    fn plan_start_run(
        &self,
        _document: &RunCommandDocument,
        projection: &RunProjection,
    ) -> Result<CommandPlan, RuntimeError> {
        require_lifecycle(projection, RunLifecycle::Created, "start")?;
        let revision = self.current_revision(projection)?;
        let scope = projection
            .root_scope()
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("created run has no root scope".to_owned())
            })?
            .reference()
            .clone();
        let mut plan = CommandPlan::one(RunEventKind::RunStarted);
        for node in entry_nodes(&revision) {
            let node_view = revision.semantic().nodes().get(node).ok_or_else(|| {
                RuntimeError::InvalidHistory("entry node is absent from its revision".to_owned())
            })?;
            plan.events.push(RunEventKind::NodeBecameEligible {
                node: node.clone(),
                execution: self.next_execution_id()?,
                scope: scope.clone(),
                mode: node_execution_mode(node_view),
            });
        }
        if plan.events.len() == 1 {
            return Err(RuntimeError::InvalidTransition(
                "pinned revision has no entry node".to_owned(),
            ));
        }
        Ok(plan)
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_signal(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        signal: &milkdrift_persistence::SignalId,
        signal_type: &milkdrift_persistence::SignalTypeId,
        correlation: Option<&milkdrift_persistence::CorrelationKey>,
        mode: SignalDeliveryMode,
        payload: &BoundedJson,
    ) -> Result<CommandPlan, RuntimeError> {
        if !projection.lifecycle().is_active() {
            return Err(RuntimeError::InvalidTransition(
                "signals are accepted only for an active run".to_owned(),
            ));
        }
        if let Some(existing) = projection.signals().get(signal) {
            if existing.signal_type() != signal_type
                || existing.correlation() != correlation
                || existing.mode() != mode
                || existing.payload() != payload
            {
                return Err(RuntimeError::InvalidTransition(
                    "signal identity was reused with conflicting delivery facts".to_owned(),
                ));
            }
            return Ok(CommandPlan::one(RunEventKind::SignalDeduplicated {
                signal: signal.clone(),
                duplicate_command: document.command_id().clone(),
            }));
        }
        let mut plan = CommandPlan::one(RunEventKind::SignalReceived {
            signal: signal.clone(),
            signal_type: signal_type.clone(),
            correlation: correlation.cloned(),
            mode,
            payload: payload.clone(),
        });
        if mode == SignalDeliveryMode::Broadcast {
            return Ok(plan);
        }
        if projection.lifecycle() == RunLifecycle::Paused {
            return Ok(plan);
        }
        let compatible = projection
            .waits()
            .values()
            .filter(|wait| {
                wait.is_pending() && wait_signal_matches(wait.condition(), signal_type, correlation)
            })
            .map(|wait| wait.execution().clone())
            .min();
        if let Some(execution) = compatible {
            let entries = self.signal_payload_entries(projection, &execution, payload, &[])?;
            let event_cost = entries.len().checked_add(2).ok_or_else(|| {
                RuntimeError::Scheduling("one-shot signal event cost overflow".to_owned())
            })?;
            if plan.events.len().saturating_add(event_cost)
                > milkdrift_persistence::MAX_EVENTS_PER_COMMIT
                || entries.len() > MAX_WORKSPACE_MUTATIONS_PER_COMMIT
            {
                return Err(RuntimeError::InvalidTransition(
                    "one signal consumer exceeds atomic runtime bounds".to_owned(),
                ));
            }
            plan.events.push(RunEventKind::SignalConsumed {
                signal: signal.clone(),
                execution: execution.clone(),
            });
            for entry in entries {
                let value = entry.reference().clone();
                plan.workspace.push(WorkspaceMutation::PutValue { entry });
                plan.events
                    .push(RunEventKind::DeterministicOutputPublished {
                        execution: execution.clone(),
                        value,
                        artifact: None,
                    });
            }
            plan.events.push(RunEventKind::WaitSatisfied {
                execution,
                cause: WaitSatisfaction::Signal {
                    signal: signal.clone(),
                },
            });
        }
        Ok(plan)
    }

    fn plan_timer(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        timer: &TimerId,
    ) -> Result<CommandPlan, RuntimeError> {
        let timer_view = projection.timers().get(timer).ok_or_else(|| {
            RuntimeError::InvalidTransition(format!("timer {timer} is not registered"))
        })?;
        if !timer_view.is_pending() {
            return Err(RuntimeError::InvalidTransition(format!(
                "timer {timer} already fired"
            )));
        }
        if document.issued_at() < timer_view.fire_at() {
            return Err(RuntimeError::InvalidTransition(format!(
                "timer {timer} is not due until {}",
                timer_view.fire_at()
            )));
        }
        let mut plan = CommandPlan::one(RunEventKind::TimerFired {
            timer: timer.clone(),
            observed_at: document.issued_at(),
        });
        if projection.lifecycle() == RunLifecycle::Paused {
            return Ok(plan);
        }
        if let TimerPurpose::Wait {
            execution: Some(execution),
        } = timer_view.purpose()
        {
            if projection
                .waits()
                .get(execution)
                .is_some_and(|wait| wait.is_pending())
            {
                plan.events.push(RunEventKind::WaitSatisfied {
                    execution: execution.clone(),
                    cause: WaitSatisfaction::Timer {
                        timer: timer.clone(),
                    },
                });
            }
        }
        Ok(plan)
    }

    fn plan_revision_adoption(
        &self,
        projection: &RunProjection,
        reconciliation: &milkdrift_persistence::ReconciliationId,
        requested_revision: &RevisionId,
        policy: milkdrift_persistence::ReconciliationPolicy,
    ) -> Result<CommandPlan, RuntimeError> {
        if !projection.lifecycle().is_active()
            || projection.termination().is_some()
            || projection.reconciliation().is_active()
        {
            return Err(RuntimeError::InvalidTransition(
                "revision adoption requires an active non-draining run with no active reconciliation"
                    .to_owned(),
            ));
        }
        let old = self.current_revision(projection)?;
        let workflow = projection
            .workflow()
            .ok_or_else(|| RuntimeError::InvalidHistory("run has no workflow".to_owned()))?;
        let new = self.load_validated_revision(requested_revision, Some(workflow))?;
        let history = reconciliation_history(projection, &old, &new)?;
        let plan_id = self.next_plan_id()?;
        let plan = plan_reconciliation(
            reconciliation.clone(),
            plan_id,
            &old,
            &new,
            projection.sequence(),
            &history,
            policy,
        )?;
        let mut result = CommandPlan::one(RunEventKind::RevisionAdoptionRequested {
            reconciliation: reconciliation.clone(),
            from_revision: old.id().clone(),
            to_revision: new.id().clone(),
            policy,
        });
        result.events.push(plan.recorded_event());
        Ok(result)
    }

    fn plan_reconciliation_decision(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        plan: &ReconciliationPlanId,
        decision: &ReconciliationDecisionId,
        outcome: AuthorityDecision,
    ) -> Result<CommandPlan, RuntimeError> {
        let plan_view = projection
            .reconciliation()
            .plans()
            .get(plan)
            .ok_or_else(|| RuntimeError::Reconciliation(format!("unknown plan {plan}")))?;
        if !plan_view.is_pending()
            || !matches!(
                outcome,
                AuthorityDecision::Approve | AuthorityDecision::Reject
            )
        {
            return Err(RuntimeError::Reconciliation(
                "only a pending plan can receive an approve/reject decision".to_owned(),
            ));
        }
        if projection
            .reconciliation()
            .plans()
            .values()
            .flat_map(|candidate| candidate.decisions())
            .any(|existing| existing.decision() == decision)
        {
            return Err(RuntimeError::Reconciliation(
                "reconciliation decision identity was already used".to_owned(),
            ));
        }
        Ok(CommandPlan::one(
            RunEventKind::ReconciliationDecisionRecorded {
                plan: plan.clone(),
                decision: decision.clone(),
                actor: document.actor().clone(),
                outcome,
                reason: document.reason().clone(),
                evidence: document.evidence().to_vec(),
            },
        ))
    }

    fn plan_reconciliation_application(
        &self,
        run: &RunId,
        projection: &RunProjection,
        plan: &ReconciliationPlanId,
    ) -> Result<CommandPlan, RuntimeError> {
        let plan_view = projection
            .reconciliation()
            .plans()
            .get(plan)
            .ok_or_else(|| RuntimeError::Reconciliation(format!("unknown plan {plan}")))?;
        if !plan_view.is_pending() {
            return Err(RuntimeError::Reconciliation(
                "reconciliation plan was already applied".to_owned(),
            ));
        }
        if plan_view
            .items()
            .iter()
            .any(|item| item.action == ReconciliationAction::RejectRetrospectiveRewrite)
        {
            return Err(RuntimeError::Reconciliation(
                "plan contains a retrospective rewrite and cannot be applied".to_owned(),
            ));
        }
        let needs_authority = plan_view
            .items()
            .iter()
            .any(|item| item.action == ReconciliationAction::RequireAuthority);
        let last_decision = plan_view.decisions().last().map(|value| value.outcome());
        if last_decision == Some(AuthorityDecision::Reject)
            || (needs_authority && last_decision != Some(AuthorityDecision::Approve))
        {
            return Err(RuntimeError::Reconciliation(
                "plan lacks a final approving authority decision".to_owned(),
            ));
        }
        if projection.revision() != Some(plan_view.from_revision()) {
            return Err(RuntimeError::Reconciliation(
                "revision pin moved after the plan was created".to_owned(),
            ));
        }
        fold_complete_history(self.store.as_ref(), run, (), |_unit, event| {
            if event.sequence() <= plan_view.based_on_sequence() {
                return Ok(());
            }
            let allowed = match event.kind() {
                RunEventKind::RevisionAdoptionRequested { reconciliation, .. }
                | RunEventKind::ReconciliationPlanRecorded { reconciliation, .. } => {
                    reconciliation == plan_view.reconciliation()
                }
                RunEventKind::ReconciliationDecisionRecorded {
                    plan: event_plan, ..
                } => event_plan == plan,
                _ => false,
            };
            if allowed {
                Ok(())
            } else {
                Err(RuntimeError::Reconciliation(format!(
                    "plan became stale at event {} sequence {}",
                    event.event_id(),
                    event.sequence()
                )))
            }
        })?;
        let next = self.load_validated_revision(plan_view.to_revision(), projection.workflow())?;
        let mut result = CommandPlan::default();
        for item in plan_view.items() {
            match item.action {
                ReconciliationAction::RemoveUnstarted => {
                    if let Some(execution) = &item.execution {
                        result
                            .events
                            .push(RunEventKind::ReconciliationExecutionRemoved {
                                plan: plan.clone(),
                                execution: execution.clone(),
                            });
                    }
                }
                ReconciliationAction::CancelAndRestart => {
                    let execution = item.execution.as_ref().ok_or_else(|| {
                        RuntimeError::Reconciliation(
                            "cancel-and-restart item has no exact execution".to_owned(),
                        )
                    })?;
                    let execution_view =
                        projection.node_executions().get(execution).ok_or_else(|| {
                            RuntimeError::Reconciliation(
                                "cancel-and-restart execution is absent".to_owned(),
                            )
                        })?;
                    let attempt = execution_view.attempts().last().ok_or_else(|| {
                        RuntimeError::Reconciliation(
                            "cancel-and-restart execution has no active attempt".to_owned(),
                        )
                    })?;
                    result
                        .events
                        .push(RunEventKind::ReconciliationCancellationRequested {
                            plan: plan.clone(),
                            execution: execution.clone(),
                            attempt: attempt.clone(),
                            reason: item.reason.clone(),
                        });
                }
                ReconciliationAction::CompensateOrRemediate => {
                    let source_execution = item.execution.as_ref().ok_or_else(|| {
                        RuntimeError::Reconciliation(
                            "remediation item has no source execution".to_owned(),
                        )
                    })?;
                    let node = item.node.as_ref().ok_or_else(|| {
                        RuntimeError::Reconciliation(
                            "remediation item has no target node".to_owned(),
                        )
                    })?;
                    if !next.semantic().nodes().contains_key(node) {
                        return Err(RuntimeError::Reconciliation(
                            "remediation target is absent from the adopted revision".to_owned(),
                        ));
                    }
                    let source = projection
                        .node_executions()
                        .get(source_execution)
                        .ok_or_else(|| {
                            RuntimeError::Reconciliation(
                                "remediation source execution is absent".to_owned(),
                            )
                        })?;
                    result
                        .events
                        .push(RunEventKind::ReconciliationRemediationCreated {
                            plan: plan.clone(),
                            source_execution: source_execution.clone(),
                            source_attempt: source.attempts().last().cloned(),
                            execution: self.next_execution_id()?,
                            node: node.clone(),
                            scope: source.scope().clone(),
                            mode: node_execution_mode(
                                next.semantic().nodes().get(node).ok_or_else(|| {
                                    RuntimeError::InvalidHistory(
                                        "reconciliation remediation node is absent".to_owned(),
                                    )
                                })?,
                            ),
                            reason: item.reason.clone(),
                        });
                }
                ReconciliationAction::UseNewOnNextInvocation
                    if item.classification == ReconciliationClassification::ChangedPending =>
                {
                    if let Some(execution) = &item.execution {
                        result
                            .events
                            .push(RunEventKind::ReconciliationExecutionRemoved {
                            plan: plan.clone(),
                            execution: execution.clone(),
                        });
                    }
                }
                ReconciliationAction::Preserve
                | ReconciliationAction::UseNewOnNextInvocation
                | ReconciliationAction::RequireAuthority => {}
                ReconciliationAction::RejectRetrospectiveRewrite => {
                    return Err(RuntimeError::Reconciliation(
                        "rejected retrospective rewrite cannot be enacted".to_owned(),
                    ));
                }
            }
        }
        let mut application_boundary = projection.sequence();
        for _ in &result.events {
            application_boundary = application_boundary.next()?;
        }
        result.events.push(RunEventKind::ReconciliationApplied {
            plan: plan.clone(),
            from_revision: plan_view.from_revision().clone(),
            to_revision: plan_view.to_revision().clone(),
            based_on_sequence: application_boundary,
        });
        result.events.push(RunEventKind::RevisionPinned {
            previous: plan_view.from_revision().clone(),
            revision: plan_view.to_revision().clone(),
            revision_digest: next.content_digest().clone(),
            plan: plan.clone(),
        });
        Ok(result)
    }

    fn plan_external_resolution(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        attempt: &AttemptId,
        decision: &ReconciliationDecisionId,
        action: ExternalWorkAction,
        remediation_node: Option<&NodeId>,
    ) -> Result<CommandPlan, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        if !attempt_view.is_unresolved() {
            return Err(RuntimeError::InvalidTransition(
                "external-work resolution requires an uncertain or retained attempt".to_owned(),
            ));
        }
        let retry_event = if action == ExternalWorkAction::Retry {
            let classified = attempt_view.side_effect().ok_or_else(|| {
                RuntimeError::InvalidTransition(
                    "manual retry requires a durable side-effect classification".to_owned(),
                )
            })?;
            let error_class = attempt_view
                .terminal()
                .and_then(crate::projection::AttemptTerminal::error_class)
                .unwrap_or_else(|| unresolved_retry_error_class(attempt_view));
            if !self.config.retry_policy.permits_automatic_retry(
                attempt_view.attempt_number(),
                error_class,
                true,
                classified.side_effect(),
                classified.idempotency(),
                classified.idempotency_key(),
            ) {
                return Err(RuntimeError::InvalidTransition(
                    "manual retry exceeds the bounded retry policy or is unsafe for the durable side-effect/idempotency facts"
                        .to_owned(),
                ));
            }
            Some(self.build_retry_event(
                attempt_view.execution(),
                attempt,
                attempt_view.attempt_number(),
                document.issued_at(),
                error_class,
                None,
                "bounded authority retry admitted by durable side-effect and idempotency policy",
            )?)
        } else {
            None
        };
        let authority = match action {
            ExternalWorkAction::Retain => AuthorityDecision::Retain,
            ExternalWorkAction::Query => AuthorityDecision::Query,
            ExternalWorkAction::Retry => AuthorityDecision::Retry,
            ExternalWorkAction::Compensate => AuthorityDecision::Compensate,
            ExternalWorkAction::ResolveSucceeded => AuthorityDecision::ResolveSucceeded,
            ExternalWorkAction::ResolveFailed => AuthorityDecision::ResolveFailed,
        };
        let mut plan = CommandPlan::one(RunEventKind::RecoveryDecisionRecorded {
            attempt: attempt.clone(),
            decision: decision.clone(),
            actor: document.actor().clone(),
            outcome: authority,
            reason: document.reason().clone(),
            evidence: document.evidence().to_vec(),
        });
        match action {
            ExternalWorkAction::Retain => {
                plan.events.push(RunEventKind::ExternalOutcomeRetained {
                    attempt: attempt.clone(),
                    decision: decision.clone(),
                    reason: document.reason().clone(),
                });
            }
            ExternalWorkAction::Query => {}
            ExternalWorkAction::Retry => {
                plan.events.push(retry_event.ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "validated manual retry event disappeared".to_owned(),
                    )
                })?);
            }
            ExternalWorkAction::Compensate => {
                let node = remediation_node.ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "compensation requires an exact remediation target node".to_owned(),
                    )
                })?;
                let revision = self.current_revision(projection)?;
                let target = revision.semantic().nodes().get(node).ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "remediation target is absent from the pinned revision".to_owned(),
                    )
                })?;
                let source_execution = projection
                    .node_executions()
                    .get(attempt_view.execution())
                    .ok_or_else(|| {
                    RuntimeError::InvalidHistory("uncertain source execution is absent".to_owned())
                })?;
                plan.events.push(RunEventKind::RemediationWorkCreated {
                    source_attempt: attempt.clone(),
                    execution: self.next_execution_id()?,
                    node: node.clone(),
                    scope: source_execution.scope().clone(),
                    mode: node_execution_mode(target),
                    decision: decision.clone(),
                    reason: document.reason().clone(),
                });
            }
            ExternalWorkAction::ResolveSucceeded | ExternalWorkAction::ResolveFailed => {}
        }
        Ok(plan)
    }

    fn plan_worker_report(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        worker: &WorkerId,
        report: &WorkerReport,
    ) -> Result<CommandPlan, RuntimeError> {
        if document.actor() != &self.config.internal_actor || worker != &self.config.worker {
            return Err(RuntimeError::InvalidTransition(
                "worker reports require the configured worker and trusted internal actor boundary"
                    .to_owned(),
            ));
        }
        match report {
            WorkerReport::LeaseAccepted { lease, attempt } => {
                let lease_view = projection.leases().get(lease).ok_or_else(|| {
                    RuntimeError::InvalidTransition(format!("unknown lease {lease}"))
                })?;
                if lease_view.worker() != worker
                    || lease_view.attempt() != attempt
                    || !lease_view.is_active()
                {
                    return Err(RuntimeError::InvalidTransition(
                        "lease acceptance does not match active worker ownership".to_owned(),
                    ));
                }
                let attempt_view = projection.attempts().get(attempt).ok_or_else(|| {
                    RuntimeError::InvalidHistory("lease attempt is absent".to_owned())
                })?;
                let invocation = attempt_view.invocation().ok_or_else(|| {
                    RuntimeError::InvalidHistory("leased attempt has no invocation".to_owned())
                })?;
                Ok(CommandPlan::one(RunEventKind::NodeStarted {
                    execution: attempt_view.execution().clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                }))
            }
            WorkerReport::Heartbeat { lease, expires_at } => {
                let lease_view = projection.leases().get(lease).ok_or_else(|| {
                    RuntimeError::InvalidTransition(format!("unknown lease {lease}"))
                })?;
                if lease_view.worker() != worker || !lease_view.is_active() {
                    return Err(RuntimeError::InvalidTransition(
                        "heartbeat does not match active worker ownership".to_owned(),
                    ));
                }
                if *expires_at <= lease_view.expires_at() || *expires_at <= document.issued_at() {
                    return Err(RuntimeError::InvalidTransition(
                        "heartbeat expiration must advance the active lease into the future"
                            .to_owned(),
                    ));
                }
                Ok(CommandPlan::one(RunEventKind::LeaseHeartbeatRecorded {
                    lease: lease.clone(),
                    expires_at: *expires_at,
                }))
            }
            WorkerReport::Started { attempt } => {
                let attempt_view = self.worker_attempt(projection, worker, attempt)?;
                let invocation = attempt_view.invocation().ok_or_else(|| {
                    RuntimeError::InvalidHistory("scheduled attempt has no invocation".to_owned())
                })?;
                Ok(CommandPlan::one(RunEventKind::NodeStarted {
                    execution: attempt_view.execution().clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                }))
            }
            WorkerReport::Invocation { attempt, report } => {
                let attempt_view = self.worker_attempt(projection, worker, attempt)?;
                if attempt_view.invocation() != Some(report.invocation()) {
                    return Err(RuntimeError::InvalidTransition(
                        "invocation report correlation does not match the attempt".to_owned(),
                    ));
                }
                self.plan_invocation_report(document, projection, attempt, report)
            }
            WorkerReport::Cancellation {
                attempt,
                acknowledgement,
            } => {
                let attempt_view = self.worker_attempt(projection, worker, attempt)?;
                if attempt_view.invocation() != Some(acknowledgement.invocation()) {
                    return Err(RuntimeError::InvalidTransition(
                        "cancellation acknowledgement names another invocation".to_owned(),
                    ));
                }
                let mut plan = CommandPlan::one(RunEventKind::InvocationCancellationAcknowledged {
                    attempt: attempt.clone(),
                    acknowledgement: acknowledgement.clone(),
                });
                if acknowledgement.accepted() && acknowledgement.terminal_boundary() {
                    plan.events.push(RunEventKind::NodeTerminal {
                        execution: attempt_view.execution().clone(),
                        attempt: attempt.clone(),
                        report_sequence: self.next_report_sequence(projection, attempt)?,
                        outcome: NodeOutcome::Cancelled,
                        error_class: None,
                        detail: acknowledgement
                            .detail()
                            .map(|detail| BoundedDetail::new(detail.to_owned()))
                            .transpose()?,
                    });
                }
                Ok(plan)
            }
            WorkerReport::Terminal {
                attempt,
                report_sequence,
                terminal,
            } => {
                let _ = self.worker_attempt(projection, worker, attempt)?;
                self.plan_terminal_report(document, projection, attempt, *report_sequence, terminal)
            }
        }
    }

    fn worker_attempt<'a>(
        &self,
        projection: &'a RunProjection,
        worker: &WorkerId,
        attempt: &AttemptId,
    ) -> Result<&'a crate::projection::NodeAttemptProjection, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        let owned = projection.leases().values().any(|lease| {
            lease.attempt() == attempt && lease.worker() == worker && lease.is_active()
        });
        if !owned {
            return Err(RuntimeError::InvalidTransition(
                "worker does not own an active lease for the attempt".to_owned(),
            ));
        }
        Ok(attempt_view)
    }

    fn plan_invocation_report(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        attempt: &AttemptId,
        report: &InvocationEvent,
    ) -> Result<CommandPlan, RuntimeError> {
        match report.kind() {
            InvocationEventKind::Progress {
                message,
                completed_units,
                total_units,
            } => {
                let expected = self.next_report_sequence(projection, attempt)?;
                if report.sequence() != expected {
                    return Err(RuntimeError::InvalidTransition(format!(
                        "progress report sequence must be exactly {expected}"
                    )));
                }
                Ok(CommandPlan::one(RunEventKind::NodeProgressRecorded {
                    attempt: attempt.clone(),
                    report_sequence: report.sequence(),
                    detail: BoundedDetail::new(message.clone())?,
                    completed_units: *completed_units,
                    total_units: *total_units,
                }))
            }
            InvocationEventKind::Output { name, reference } => {
                self.plan_output_report(projection, attempt, report.sequence(), name, reference)
            }
            InvocationEventKind::Terminal { terminal } => self.plan_terminal_report(
                document,
                projection,
                attempt,
                report.sequence(),
                terminal,
            ),
        }
    }

    fn plan_output_report(
        &self,
        projection: &RunProjection,
        attempt: &AttemptId,
        report_sequence: u64,
        name: &str,
        reference: &milkdrift_capability::ArtifactReference,
    ) -> Result<CommandPlan, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        if !matches!(attempt_view.state(), AttemptState::Running) {
            return Err(RuntimeError::InvalidTransition(
                "output report requires a running attempt".to_owned(),
            ));
        }
        let expected = self.next_report_sequence(projection, attempt)?;
        if report_sequence != expected {
            return Err(RuntimeError::InvalidTransition(format!(
                "output report sequence must be exactly {expected}"
            )));
        }
        let execution = projection
            .node_executions()
            .get(attempt_view.execution())
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("attempt execution is absent".to_owned())
            })?;
        let scheduled_revision = projection.revision_for_attempt(attempt).ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "scheduled attempt has no governing revision pin".to_owned(),
            )
        })?;
        let revision = self.load_validated_revision(scheduled_revision, projection.workflow())?;
        let node = revision
            .semantic()
            .nodes()
            .get(execution.node())
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "scheduled attempt node is absent from its governing revision".to_owned(),
                )
            })?;
        let output_port = PortId::new(name.to_owned())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        if !node.data_outputs().contains_key(&output_port) {
            return Err(RuntimeError::InvalidTransition(format!(
                "executor output {name} is not a declared data output of node {}",
                node.id()
            )));
        }
        let (metadata, artifact) = self.resolve_executor_artifact(reference)?;
        let key = ValueKey::new(output_port.as_str().to_owned())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let entry = self.projected_output_entry(
            projection,
            execution.scope(),
            key,
            WorkspaceValue::Artifact(artifact.clone()),
            &[],
        )?;
        let mut plan = CommandPlan::default();
        if !projection.artifacts().contains_key(artifact.artifact()) {
            plan.events
                .push(RunEventKind::ArtifactPublished { metadata });
        }
        plan.events.push(RunEventKind::NodeOutputPublished {
            execution: attempt_view.execution().clone(),
            attempt: attempt.clone(),
            report_sequence,
            value: entry.reference().clone(),
            artifact: Some(artifact.clone()),
        });
        plan.workspace.push(WorkspaceMutation::PutValue { entry });
        plan.required_artifacts.insert(artifact);
        Ok(plan)
    }

    fn plan_terminal_report(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        attempt: &AttemptId,
        report_sequence: u64,
        terminal: &InvocationTerminal,
    ) -> Result<CommandPlan, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        let expected = self.next_report_sequence(projection, attempt)?;
        if report_sequence != expected || attempt_view.is_completed() {
            return Err(RuntimeError::InvalidTransition(format!(
                "terminal report sequence must be exactly {expected} and attempt must be active"
            )));
        }
        let classified = attempt_view.side_effect().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "terminal attempt has no side-effect classification".to_owned(),
            )
        })?;
        if terminal.side_effect() > classified.side_effect() {
            return Err(RuntimeError::InvalidTransition(
                "terminal observation exceeds the frozen side-effect classification".to_owned(),
            ));
        }
        let published: BTreeSet<_> = attempt_view
            .outputs()
            .iter()
            .filter_map(|output| output.artifact())
            .map(|artifact| artifact.digest().to_hex())
            .collect();
        if terminal
            .outputs()
            .iter()
            .any(|output| !published.contains(output.digest()))
        {
            return Err(RuntimeError::InvalidTransition(
                "terminal output was not first published as a durable workspace artifact"
                    .to_owned(),
            ));
        }
        let mut plan = CommandPlan::default();
        if let Some(usage) = terminal.usage() {
            let cost = match usage.cost_micros().zip(usage.currency()) {
                Some((micros, currency)) => Some(milkdrift_persistence::MonetaryUsage {
                    micros,
                    currency: CurrencyCode::new(currency.to_owned())?,
                }),
                None => None,
            };
            plan.events.push(RunEventKind::AttemptUsageRecorded {
                attempt: attempt.clone(),
                usage: AttemptUsage {
                    input_units: usage.input_units(),
                    output_units: usage.output_units(),
                    duration_ms: usage.duration_ms(),
                    cost,
                },
            });
        }
        if terminal.status() == TerminalStatus::Uncertain {
            plan.events.push(RunEventKind::ExternalOutcomeUncertain {
                attempt: attempt.clone(),
                report_sequence,
                side_effect: classified.side_effect(),
                reason: Reason::new(terminal.failure().map_or(
                    "executor reported an uncertain external outcome",
                    |failure| failure.message(),
                ))?,
                evidence: document.evidence().to_vec(),
            });
            return Ok(plan);
        }
        let (outcome, error_class, detail) = match terminal.status() {
            TerminalStatus::Success => (NodeOutcome::Succeeded, None, None),
            TerminalStatus::Cancelled => (NodeOutcome::Cancelled, None, None),
            TerminalStatus::Failure | TerminalStatus::Rejected => {
                let failure = terminal.failure().ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "failed terminal report lacks details".to_owned(),
                    )
                })?;
                (
                    if terminal.status() == TerminalStatus::Rejected {
                        NodeOutcome::Rejected
                    } else {
                        NodeOutcome::Failed
                    },
                    Some(failure.class()),
                    Some(BoundedDetail::new(failure.message().to_owned())?),
                )
            }
            TerminalStatus::Uncertain => {
                return Err(RuntimeError::InvalidHistory(
                    "uncertain terminal routing failure".to_owned(),
                ));
            }
        };
        plan.events.push(RunEventKind::NodeTerminal {
            execution: attempt_view.execution().clone(),
            attempt: attempt.clone(),
            report_sequence,
            outcome,
            error_class,
            detail,
        });
        if let Some(failure) = terminal.failure() {
            if self.config.retry_policy.permits_automatic_retry(
                attempt_view.attempt_number(),
                failure.class(),
                failure.retryable(),
                classified.side_effect(),
                classified.idempotency(),
                classified.idempotency_key(),
            ) {
                match self.build_retry_event(
                    attempt_view.execution(),
                    attempt,
                    attempt_view.attempt_number(),
                    document.issued_at(),
                    failure.class(),
                    failure.retry_after_ms(),
                    "bounded automatic retry policy admitted another attempt",
                ) {
                    Ok(retry) => plan.events.push(retry),
                    Err(error) => warn!(
                        attempt = %attempt,
                        reason = %error,
                        "truthful terminal report retained without an out-of-policy retry timer"
                    ),
                }
            }
        }
        Ok(plan)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_retry_event(
        &self,
        execution: &NodeExecutionId,
        previous_attempt: &AttemptId,
        previous_attempt_number: u32,
        observed_at: TimestampMillis,
        error_class: ErrorClass,
        retry_after_ms: Option<u64>,
        rationale: &'static str,
    ) -> Result<RunEventKind, RuntimeError> {
        let attempt_number = previous_attempt_number
            .checked_add(1)
            .ok_or_else(|| RuntimeError::Scheduling("attempt number overflow".to_owned()))?;
        let delay = self
            .config
            .retry_policy
            .retry_delay_ms(attempt_number, 0, retry_after_ms)?;
        let fire_at = checked_timestamp_add(observed_at, delay)?;
        let reason = Reason::new(rationale)?;
        let next_attempt = self.next_attempt_id()?;
        let timer = self.next_timer_id()?;
        Ok(RunEventKind::NodeRetryScheduled {
            execution: execution.clone(),
            previous_attempt: previous_attempt.clone(),
            next_attempt,
            attempt_number,
            timer,
            fire_at,
            error_class,
            reason,
        })
    }

    fn resolve_executor_artifact(
        &self,
        reference: &milkdrift_capability::ArtifactReference,
    ) -> Result<(milkdrift_workspace::ArtifactMetadata, ArtifactReference), RuntimeError> {
        let artifact_id = ArtifactId::new(reference.identity().to_owned())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let metadata = self.store.metadata(&artifact_id)?.ok_or_else(|| {
            RuntimeError::InvalidTransition(format!(
                "executor artifact {} has no committed metadata",
                reference.identity()
            ))
        })?;
        let durable = metadata.reference().clone();
        let media_matches = reference
            .media_type()
            .is_none_or(|media| media == durable.media_type().as_str());
        let size_matches = reference
            .size_bytes()
            .is_none_or(|size| size == durable.size_bytes());
        if reference.digest() != durable.digest().to_hex()
            || !media_matches
            || !size_matches
            || !self.store.is_committed(&durable)?
        {
            return Err(RuntimeError::InvalidTransition(
                "executor artifact reference differs from committed content metadata".to_owned(),
            ));
        }
        Ok((metadata, durable))
    }

    fn next_report_sequence(
        &self,
        projection: &RunProjection,
        attempt: &AttemptId,
    ) -> Result<u64, RuntimeError> {
        projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidHistory("attempt is absent".to_owned()))?
            .last_report_sequence()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| RuntimeError::InvalidTransition("report sequence overflow".to_owned()))
    }

    fn commit_accepted(
        &self,
        document: &RunCommandDocument,
        receipt: CommandReceipt,
        projection: RunProjection,
        mut plan: CommandPlan,
    ) -> Result<AtomicRunCommitOutcome, RuntimeError> {
        if plan.events.is_empty() {
            return Err(RuntimeError::InvalidTransition(
                "an accepted transition must emit at least one event".to_owned(),
            ));
        }
        let mut candidate = projection.clone();
        let mut envelopes = Vec::with_capacity(plan.events.len());
        let mut sequence = projection.sequence();
        for kind in plan.events.drain(..) {
            sequence = sequence.next()?;
            let event = RunEventEnvelope::new(
                self.next_event_id()?,
                document.run_id().clone(),
                sequence,
                document.issued_at(),
                kind,
            )?;
            candidate.apply_replayed(&event)?;
            debug!(
                event = %event.event_id(),
                sequence = event.sequence().get(),
                event_type = event_kind_name(event.kind()),
                "projected candidate event"
            );
            envelopes.push(event);
        }
        let revision = if candidate.revision().is_some() {
            Some(self.current_revision(&candidate)?)
        } else {
            None
        };
        if let (Some(revision), true) = (revision.as_ref(), candidate.lifecycle().is_active()) {
            self.extend_structured_progress(
                document.run_id(),
                document.issued_at(),
                revision,
                &mut candidate,
                &mut envelopes,
                &mut plan.workspace,
            )?;
        }
        let event_ids = envelopes
            .iter()
            .map(|event| event.event_id().clone())
            .collect::<Vec<_>>();
        let resulting_sequence = candidate.sequence();
        let result_payload = BoundedJson::new(json!({
            "status": "accepted",
            "event_count": event_ids.len(),
            "resulting_sequence": resulting_sequence.get(),
        }))
        .map_err(|error| RuntimeError::InvalidCommand(error.to_string()))?;
        let result = CommandResultDocument::new(
            document.command_id().clone(),
            document.run_id().clone(),
            receipt.fingerprint().clone(),
            CommandDisposition::Accepted,
            resulting_sequence,
            event_ids,
            result_payload,
        )?;
        let required_artifacts = collect_required_artifacts(&envelopes, &plan.workspace)?;
        for artifact in &required_artifacts {
            if !self.store.is_committed(artifact)? {
                return Err(RuntimeError::InvalidTransition(format!(
                    "event references uncommitted artifact {}",
                    artifact.artifact()
                )));
            }
        }
        if !plan.required_artifacts.is_empty()
            && !plan
                .required_artifacts
                .iter()
                .all(|artifact| required_artifacts.contains(artifact))
        {
            return Err(RuntimeError::InvalidTransition(
                "planned artifact set is not represented by event/workspace facts".to_owned(),
            ));
        }
        let budget = candidate.workspace_budget().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "accepted run transition has no workspace budget".to_owned(),
            )
        })?;
        let (expected_usage, resulting_usage, newly_referenced_artifacts) =
            match plan.creation_usage {
                Some(usage) => usage,
                None => self.workspace_accounting_transition(
                    &projection,
                    &plan.workspace,
                    budget,
                    &required_artifacts,
                )?,
            };
        let accounting = WorkspaceAccounting {
            budget: budget.clone(),
            expected_usage,
            resulting_usage,
        };
        let indexes = self.index_update(
            document.run_id(),
            &projection,
            &candidate,
            document.issued_at(),
        )?;
        let request = AtomicRunCommitRequest::new(
            receipt,
            envelopes,
            plan.workspace,
            Some(accounting),
            required_artifacts.into_iter().collect(),
            newly_referenced_artifacts.into_iter().collect(),
            plan.expected_lease_catalog,
            result,
            indexes,
        )?;
        Ok(self.store.commit_command(&request)?)
    }

    fn commit_rejected(
        &self,
        document: &RunCommandDocument,
        receipt: CommandReceipt,
        detail: &str,
    ) -> Result<AtomicRunCommitOutcome, RuntimeError> {
        let payload = BoundedJson::new(json!({
            "status": "rejected",
            "reason": detail,
        }))
        .map_err(|error| RuntimeError::InvalidCommand(error.to_string()))?;
        let result = CommandResultDocument::new(
            document.command_id().clone(),
            document.run_id().clone(),
            receipt.fingerprint().clone(),
            CommandDisposition::Rejected,
            document.expected_sequence(),
            Vec::new(),
            payload,
        )?;
        let request = AtomicRunCommitRequest::new(
            receipt,
            Vec::new(),
            Vec::new(),
            None,
            Vec::new(),
            Vec::new(),
            None,
            result,
            RunIndexUpdate::default(),
        )?;
        Ok(self.store.commit_command(&request)?)
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn extend_structured_progress(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
    ) -> Result<(), RuntimeError> {
        const MAX_DRIVER_PASSES: usize = 512;
        let scan_limit = usize::from(self.config.maximum_tick_items);
        let mut eligible_scan_remaining = scan_limit;
        let mut successor_scan_remaining = scan_limit;
        let mut branch_scan_remaining = scan_limit;
        // External facts (including worker reports, signals, and timer firings) are
        // committed while paused, but they must not advance deterministic work or
        // materialize new eligibility until an explicit resume transitions the
        // projected lifecycle back to Running. Cancellation changes the lifecycle
        // to Cancelling and therefore continues to drain already-owned work.
        if projection.lifecycle() == RunLifecycle::Paused {
            return Ok(());
        }
        let received_signal_in_current_commit = events
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::SignalReceived { .. }));
        if !received_signal_in_current_commit && run_drain_reason(projection).is_none() {
            self.drain_broadcast_signals(run, occurred_at, projection, events, workspace)?;
        }
        if projection.lifecycle() == RunLifecycle::Running && projection.termination().is_none() {
            let root_scope = projection
                .root_scope()
                .ok_or_else(|| RuntimeError::InvalidHistory("run root scope is absent".to_owned()))?
                .reference()
                .clone();
            for node_id in entry_nodes(revision) {
                if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
                if node_occurrence_exists_for_current_pin(projection, node_id, &root_scope)
                {
                    continue;
                }
                let node = revision.semantic().nodes().get(node_id).ok_or_else(|| {
                    RuntimeError::InvalidHistory("current revision entry node is absent".to_owned())
                })?;
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::NodeBecameEligible {
                        node: node_id.clone(),
                        execution: self.next_execution_id()?,
                        scope: root_scope.clone(),
                        mode: node_execution_mode(node),
                    },
                )?;
            }
        }
        if let Some(reason) = run_drain_reason(projection).cloned() {
            let active_branches: Vec<_> = self
                .scan_branch_ids(run, projection, &mut branch_scan_remaining)?
                .into_iter()
                .filter(|branch| {
                    projection
                        .branches()
                        .get(branch)
                        .is_some_and(|branch| branch.state() == BranchState::Active)
                })
                .collect();
            for branch in active_branches {
                if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::BranchCancellationRequested {
                        branch,
                        reason: reason.clone(),
                    },
                )?;
            }
        }
        for _ in 0..MAX_DRIVER_PASSES {
            if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            let before = events.len();
            let eligible: Vec<_> = self
                .scan_eligible_execution_ids(run, projection, &mut eligible_scan_remaining)?
                .into_iter()
                .filter_map(|execution| {
                    let execution = projection.node_executions().get(&execution)?;
                    (execution.state() == &NodeExecutionState::Eligible
                        && (execution.mode() == NodeExecutionMode::Runtime
                            || run_drain_reason(projection).is_some()
                            || execution_branch_state(projection, execution.execution())
                                == Some(BranchState::Cancelling)))
                    .then(|| {
                        (
                            execution.execution().clone(),
                            execution.node().clone(),
                            execution.scope().clone(),
                        )
                    })
                })
                .collect();
            for (execution, node_id, scope_reference) in eligible {
                if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
                let execution_revision = self.revision_for_execution(projection, &execution)?;
                let node = execution_revision
                    .semantic()
                    .nodes()
                    .get(&node_id)
                    .ok_or_else(|| {
                    RuntimeError::InvalidHistory(format!(
                        "eligible node {node_id} is absent from governing revision {}",
                        execution_revision.id()
                    ))
                    })?;
                let structurally_cancelling = run_drain_reason(projection).is_some()
                    || execution_branch_state(projection, &execution)
                        == Some(BranchState::Cancelling);
                if structurally_cancelling
                    && !matches!(
                        node.kind(),
                        NodeKind::Repeat { .. } | NodeKind::Subworkflow { .. }
                    )
                {
                    let timers: Vec<_> = projection
                        .timers()
                        .values()
                        .filter(|timer| {
                            timer.is_pending()
                                && matches!(
                                    timer.purpose(),
                                    TimerPurpose::Wait { execution: Some(owner) }
                                        if owner == &execution
                                )
                        })
                        .map(|timer| timer.timer().clone())
                        .collect();
                    for timer in timers {
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::TimerCancelled {
                                timer,
                                reason: Reason::new(
                                    "structured cancellation released a pending timer",
                                )?,
                            },
                        )?;
                    }
                    if projection
                        .waits()
                        .get(&execution)
                        .is_some_and(|wait| wait.is_pending())
                    {
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::WaitCancelled {
                                execution: execution.clone(),
                                reason: Reason::new(
                                    "structured cancellation released a pending wait",
                                )?,
                            },
                        )?;
                    }
                    self.push_projected_event(
                        run,
                        occurred_at,
                        projection,
                        events,
                        RunEventKind::NodeExecutionCancelledBeforeDispatch {
                            execution: execution.clone(),
                            reason: Reason::new(
                                "execution was cancelled before an external dispatch boundary",
                            )?,
                        },
                    )?;
                    continue;
                }
                match node.kind() {
                    NodeKind::Task { .. } => {}
                    NodeKind::Terminal { outcome } => match outcome {
                        TerminalOutcome::Success => {
                            match self.materialize_success_terminal_outputs(
                                run,
                                occurred_at,
                                &execution_revision,
                                projection,
                                events,
                                workspace,
                                node,
                                &execution,
                                &scope_reference,
                            ) {
                                Ok(true) => self.complete_deterministic(
                                    run,
                                    occurred_at,
                                    projection,
                                    events,
                                    node,
                                    &execution,
                                )?,
                                Ok(false) => {}
                                Err(RuntimeError::Scheduling(_)) => {
                                    self.complete_deterministic_with_outcome(
                                        run,
                                        occurred_at,
                                        projection,
                                        events,
                                        node,
                                        &execution,
                                        NodeOutcome::Failed,
                                        Some(BoundedDetail::new(
                                            "terminal outputs could not be resolved from immutable inputs",
                                        )?),
                                    )?;
                                }
                                Err(error) => return Err(error),
                            }
                        }
                        TerminalOutcome::Failure => {
                            self.complete_deterministic_with_outcome(
                                run,
                                occurred_at,
                                projection,
                                events,
                                node,
                                &execution,
                                NodeOutcome::Failed,
                                Some(BoundedDetail::new(
                                    "the explicit workflow terminal selected failure",
                                )?),
                            )?;
                            if execution_branch_state(projection, &execution).is_none()
                                && projection.cancellation().is_none()
                                && projection.termination().is_none()
                            {
                                self.push_projected_event(
                                    run,
                                    occurred_at,
                                    projection,
                                    events,
                                    RunEventKind::RunTerminationRequested {
                                        outcome: RunOutcome::Failed,
                                        reason: Reason::new(
                                            "explicit failure terminal is draining owned work",
                                        )?,
                                    },
                                )?;
                            }
                        }
                        TerminalOutcome::Cancelled => {
                            let branch = projection
                                .branches()
                                .values()
                                .find(|branch| {
                                    branch.state() == BranchState::Active
                                        && branch.children().contains(&execution)
                                })
                                .map(|branch| branch.branch().clone());
                            if let Some(branch) = branch {
                                self.push_projected_event(
                                    run,
                                    occurred_at,
                                    projection,
                                    events,
                                    RunEventKind::BranchCancellationRequested {
                                        branch,
                                        reason: Reason::new(
                                            "explicit cancelled terminal ended its fork branch",
                                        )?,
                                    },
                                )?;
                            } else if projection.cancellation().is_none() {
                                self.push_projected_event(
                                    run,
                                    occurred_at,
                                    projection,
                                    events,
                                    RunEventKind::RunCancellationRequested {
                                        reason: Reason::new(
                                            "explicit cancelled terminal is draining owned work",
                                        )?,
                                        evidence: Vec::new(),
                                    },
                                )?;
                            }
                            self.complete_deterministic_with_outcome(
                                run,
                                occurred_at,
                                projection,
                                events,
                                node,
                                &execution,
                                NodeOutcome::Cancelled,
                                None,
                            )?;
                        }
                    },
                    NodeKind::Wait { duration_ms } => {
                        if !projection.waits().contains_key(&execution) {
                            let timer = self.next_timer_id()?;
                            let fire_at = checked_timestamp_add(occurred_at, *duration_ms)?;
                            self.push_projected_event(
                                run,
                                occurred_at,
                                projection,
                                events,
                                RunEventKind::TimerRegistered {
                                    timer: timer.clone(),
                                    execution: Some(execution.clone()),
                                    fire_at,
                                },
                            )?;
                            self.push_projected_event(
                                run,
                                occurred_at,
                                projection,
                                events,
                                RunEventKind::WaitRegistered {
                                    execution: execution.clone(),
                                    condition: WaitCondition::Timer { timer },
                                },
                            )?;
                        } else if let Some(timer) = projection
                            .waits()
                            .get(&execution)
                            .filter(|wait| wait.is_pending())
                            .and_then(|wait| match wait.condition() {
                                WaitCondition::Timer { timer }
                                | WaitCondition::SignalOrTimer { timer, .. }
                                    if projection
                                        .timers()
                                        .get(timer)
                                        .is_some_and(|timer| timer.is_completed()) =>
                                {
                                    Some(timer.clone())
                                }
                                WaitCondition::Timer { .. }
                                | WaitCondition::Signal { .. }
                                | WaitCondition::SignalOrTimer { .. } => None,
                            })
                        {
                            self.push_projected_event(
                                run,
                                occurred_at,
                                projection,
                                events,
                                RunEventKind::WaitSatisfied {
                                    execution: execution.clone(),
                                    cause: WaitSatisfaction::Timer { timer },
                                },
                            )?;
                        } else if projection
                            .waits()
                            .get(&execution)
                            .is_some_and(|wait| wait.is_completed())
                        {
                            self.complete_deterministic(
                                run,
                                occurred_at,
                                projection,
                                events,
                                node,
                                &execution,
                            )?;
                        }
                    }
                    NodeKind::SignalWait { signal } => {
                        if !projection.waits().contains_key(&execution) {
                            let signal_type = milkdrift_persistence::SignalTypeId::new(
                                signal.as_str().to_owned(),
                            )?;
                            self.push_projected_event(
                                run,
                                occurred_at,
                                projection,
                                events,
                                RunEventKind::WaitRegistered {
                                    execution: execution.clone(),
                                    condition: WaitCondition::Signal {
                                        signal_type,
                                        correlation: None,
                                    },
                                },
                            )?;
                        }
                        if let Some(registered_condition) = projection
                            .waits()
                            .get(&execution)
                            .filter(|wait| wait.is_pending())
                            .map(|wait| wait.condition().clone())
                        {
                            let queued = projection
                                .signals()
                                .values()
                                .filter(|candidate| {
                                    candidate.is_pending()
                                        && candidate.mode() == SignalDeliveryMode::OneShot
                                        && wait_signal_matches(
                                            &registered_condition,
                                            candidate.signal_type(),
                                            candidate.correlation(),
                                        )
                                })
                                .min_by_key(|candidate| candidate.received_sequence())
                                .map(|candidate| {
                                    (candidate.signal().clone(), candidate.payload().clone())
                                });
                            if let Some((queued_signal, payload)) = queued {
                                self.push_projected_event(
                                    run,
                                    occurred_at,
                                    projection,
                                    events,
                                    RunEventKind::SignalConsumed {
                                        signal: queued_signal.clone(),
                                        execution: execution.clone(),
                                    },
                                )?;
                                for port in node.data_outputs().keys() {
                                    let key = ValueKey::new(port.as_str().to_owned()).map_err(
                                        |error| RuntimeError::Scheduling(error.to_string()),
                                    )?;
                                    let entry = self.projected_output_entry(
                                        projection,
                                        &scope_reference,
                                        key,
                                        WorkspaceValue::Json(payload.clone()),
                                        workspace,
                                    )?;
                                    let value = entry.reference().clone();
                                    workspace.push(WorkspaceMutation::PutValue { entry });
                                    self.push_projected_event(
                                        run,
                                        occurred_at,
                                        projection,
                                        events,
                                        RunEventKind::DeterministicOutputPublished {
                                            execution: execution.clone(),
                                            value,
                                            artifact: None,
                                        },
                                    )?;
                                }
                                self.push_projected_event(
                                    run,
                                    occurred_at,
                                    projection,
                                    events,
                                    RunEventKind::WaitSatisfied {
                                        execution: execution.clone(),
                                        cause: WaitSatisfaction::Signal {
                                            signal: queued_signal,
                                        },
                                    },
                                )?;
                            }
                        }
                        if projection
                            .waits()
                            .get(&execution)
                            .is_some_and(|wait| wait.is_completed())
                        {
                            self.complete_deterministic(
                                run,
                                occurred_at,
                                projection,
                                events,
                                node,
                                &execution,
                            )?;
                        }
                    }
                    NodeKind::Branch { config } => {
                        if !projection.branch_routes().contains_key(&execution) {
                            let mut selected = None;
                            let context = match self.evaluation_context(
                                node,
                                projection,
                                &scope_reference,
                                workspace,
                            ) {
                                Ok(context) => context,
                                Err(RuntimeError::Scheduling(_)) => {
                                    self.complete_deterministic_with_outcome(
                                        run,
                                        occurred_at,
                                        projection,
                                        events,
                                        node,
                                        &execution,
                                        NodeOutcome::Failed,
                                        Some(BoundedDetail::new(
                                            "branch inputs could not be evaluated deterministically",
                                        )?),
                                    )?;
                                    continue;
                                }
                                Err(error) => return Err(error),
                            };
                            let mut evaluation_failed = false;
                            for (port, condition) in config.arms() {
                                match evaluate_condition(condition, &context) {
                                    Ok(true) => {
                                        selected = Some(port.clone());
                                        break;
                                    }
                                    Ok(false) => {}
                                    Err(RuntimeError::Scheduling(_)) => {
                                        evaluation_failed = true;
                                        break;
                                    }
                                    Err(error) => return Err(error),
                                }
                            }
                            if evaluation_failed {
                                self.complete_deterministic_with_outcome(
                                    run,
                                    occurred_at,
                                    projection,
                                    events,
                                    node,
                                    &execution,
                                    NodeOutcome::Failed,
                                    Some(BoundedDetail::new(
                                        "branch condition evaluation failed on immutable input data",
                                    )?),
                                )?;
                                continue;
                            }
                            let Some(selected) =
                                selected.or_else(|| config.fallback().cloned())
                            else {
                                self.complete_deterministic_with_outcome(
                                    run,
                                    occurred_at,
                                    projection,
                                    events,
                                    node,
                                    &execution,
                                    NodeOutcome::Failed,
                                    Some(BoundedDetail::new(
                                        "branch selected no route and declared no fallback",
                                    )?),
                                )?;
                                continue;
                            };
                            self.push_projected_event(
                                run,
                                occurred_at,
                                projection,
                                events,
                                RunEventKind::BranchRouteSelected {
                                    execution: execution.clone(),
                                    selected_port: selected,
                                },
                            )?;
                            self.complete_deterministic(
                                run,
                                occurred_at,
                                projection,
                                events,
                                node,
                                &execution,
                            )?;
                        }
                    }
                    NodeKind::Fork { config } => {
                        let parent = projection
                            .scopes()
                            .get(&scope_reference)
                            .ok_or_else(|| {
                                RuntimeError::InvalidHistory(
                                    "fork execution scope is absent".to_owned(),
                                )
                            })?
                            .clone();
                        for port in config.branches() {
                            if projection.branch_for_fork_port(&execution, port).is_some() {
                                continue;
                            }
                            // BranchScopeCreated, NodeBecameEligible, and
                            // BranchChildAdded are one atomic expansion unit.
                            if events.len().saturating_add(3) > STRUCTURED_EVENT_SOFT_LIMIT {
                                return Ok(());
                            }
                            let branch = self.next_branch_id()?;
                            let scope = WorkspaceScope::branch(
                                self.next_scope_id()?,
                                &parent,
                                branch.clone(),
                            )
                            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
                            let target = execution_revision
                                .semantic()
                                .edges()
                                .values()
                                .find(|edge| {
                                    edge.kind() == EdgeKind::Control
                                        && edge.source_node() == &node_id
                                        && edge.source_port() == port
                                })
                                .map(|edge| edge.target_node().clone())
                                .ok_or_else(|| {
                                    RuntimeError::InvalidHistory(
                                        "fork branch has no exact control target".to_owned(),
                                    )
                                })?;
                            self.push_projected_event(
                                run,
                                occurred_at,
                                projection,
                                events,
                                RunEventKind::BranchScopeCreated {
                                    fork_execution: execution.clone(),
                                    port: port.clone(),
                                    branch: branch.clone(),
                                    scope: scope.clone(),
                                },
                            )?;
                            workspace.push(WorkspaceMutation::CreateScope {
                                scope: scope.clone(),
                            });
                            let child_execution = self.next_execution_id()?;
                            self.push_projected_event(
                                run,
                                occurred_at,
                                projection,
                                events,
                                RunEventKind::NodeBecameEligible {
                                    mode: node_execution_mode(
                                        execution_revision
                                            .semantic()
                                            .nodes()
                                            .get(&target)
                                            .ok_or_else(|| {
                                                RuntimeError::InvalidHistory(
                                                    "branch target node is absent".to_owned(),
                                                )
                                            })?,
                                    ),
                                    node: target,
                                    execution: child_execution.clone(),
                                    scope: scope.reference().clone(),
                                },
                            )?;
                            self.push_projected_event(
                                run,
                                occurred_at,
                                projection,
                                events,
                                RunEventKind::BranchChildAdded {
                                    branch,
                                    execution: child_execution,
                                },
                            )?;
                        }
                        let expansion_complete = config.branches().iter().all(|port| {
                            projection.branch_for_fork_port(&execution, port).is_some()
                        });
                        if expansion_complete {
                            self.complete_deterministic(
                                run,
                                occurred_at,
                                projection,
                                events,
                                node,
                                &execution,
                            )?;
                        }
                    }
                    NodeKind::Reducer { config } => self.drive_reducer(
                        run,
                        occurred_at,
                        projection,
                        events,
                        workspace,
                        node,
                        &execution,
                        &scope_reference,
                        config,
                        &execution_revision,
                    )?,
                    NodeKind::Repeat { config } => self.drive_repeat_intent(
                        run,
                        occurred_at,
                        projection,
                        events,
                        workspace,
                        node,
                        &execution,
                        &scope_reference,
                        config,
                    )?,
                    NodeKind::Subworkflow { reference } => {
                        let child = projection
                            .subworkflows()
                            .values()
                            .find(|child| child.parent_execution() == &execution);
                        if let Some(child) = child {
                            if let SubworkflowState::Terminal(outcome) = child.state() {
                                self.complete_deterministic_with_outcome(
                                    run,
                                    occurred_at,
                                    projection,
                                    events,
                                    node,
                                    &execution,
                                    node_outcome(outcome),
                                    None,
                                )?;
                            }
                        } else {
                            self.create_subworkflow_intent(
                                run,
                                occurred_at,
                                projection,
                                events,
                                workspace,
                                node,
                                &execution,
                                &scope_reference,
                                &scope_reference,
                                reference,
                            )?;
                        }
                    }
                    NodeKind::Join { config } => {
                        if !projection.joins().contains_key(&execution) {
                            self.try_satisfy_join(
                                run,
                                occurred_at,
                                &execution_revision,
                                projection,
                                events,
                                node,
                                &execution,
                                config,
                            )?;
                        }
                    }
                }
                if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
            }

            self.close_finished_branches(
                run,
                occurred_at,
                revision,
                projection,
                events,
                &mut branch_scan_remaining,
            )?;
            self.add_ready_successors(
                run,
                occurred_at,
                revision,
                projection,
                events,
                &mut successor_scan_remaining,
            )?;
            self.try_finalize_run(run, occurred_at, revision, projection, events, workspace)?;
            if events.len() == before || projection.is_completed() {
                return Ok(());
            }
            if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
        }
        Err(RuntimeError::Scheduling(
            "structured driver did not converge within its bounded pass count".to_owned(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn drive_reducer(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: &NodeExecutionId,
        scope_reference: &ScopeReference,
        config: &milkdrift_blueprint::ReducerConfig,
        revision: &BlueprintRevision,
    ) -> Result<(), RuntimeError> {
        if matches!(config.strategy(), ReducerStrategy::Capability(_)) {
            return Ok(());
        }
        if !projection
            .node_executions()
            .get(execution)
            .is_some_and(|value| value.outputs().is_empty())
        {
            return self.complete_deterministic(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
            );
        }
        let values = self.ordered_reducer_references(
            revision,
            projection,
            node,
            config.input_port(),
            scope_reference,
            workspace,
        )?;
        if values.len() < usize::from(config.minimum_items()) {
            return Ok(());
        }
        let output_port = node.data_outputs().keys().next().ok_or_else(|| {
            RuntimeError::Scheduling(format!("reducer node {} has no output port", node.id()))
        })?;
        let key = ValueKey::new(output_port.as_str().to_owned())
            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
        let (value, artifact) = match config.strategy() {
            ReducerStrategy::Collect => {
                let Ok(json_value) = serde_json::to_value(&values) else {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "deterministic reducer result could not be serialized",
                        )?),
                    );
                };
                let Ok(collected) = BoundedJson::new(json_value) else {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "deterministic reducer result exceeds the bounded JSON contract",
                        )?),
                    );
                };
                (
                    WorkspaceValue::Json(collected),
                    None,
                )
            }
            ReducerStrategy::First => {
                let reference = values.first().ok_or_else(|| {
                    RuntimeError::Scheduling("first reducer has no input".to_owned())
                })?;
                let entry = self.projected_workspace_value(projection, reference, workspace)?;
                let artifact = entry.value().as_artifact().cloned();
                (entry.value().clone(), artifact)
            }
            ReducerStrategy::Capability(_) => return Ok(()),
        };
        let entry =
            self.projected_output_entry(projection, scope_reference, key, value, workspace)?;
        let reference = entry.reference().clone();
        workspace.push(WorkspaceMutation::PutValue { entry });
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::DeterministicOutputPublished {
                execution: execution.clone(),
                value: reference,
                artifact,
            },
        )?;
        self.complete_deterministic(run, occurred_at, projection, events, node, execution)
    }

    #[allow(clippy::too_many_arguments)]
    fn create_subworkflow_intent(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        parent_execution: &NodeExecutionId,
        occurrence_scope: &ScopeReference,
        parent_scope: &ScopeReference,
        reference: &milkdrift_blueprint::PinnedSubworkflow,
    ) -> Result<(), RuntimeError> {
        let child_revision =
            self.load_validated_revision(reference.revision(), Some(reference.workflow()))?;
        let parent_revision = self.revision_for_execution(projection, parent_execution)?;
        let mut resolved_inputs = Vec::new();
        for (field, interface_field) in child_revision.semantic().interface().inputs() {
            let port = PortId::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let Some(parent_declaration) = node.data_inputs().get(&port) else {
                if interface_field.is_required() {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        parent_execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "required subworkflow input has no parent node data port",
                        )?),
                    );
                }
                continue;
            };
            let resolved = match self.resolve_node_port_inputs(
                &parent_revision,
                projection,
                node,
                &port,
                occurrence_scope,
                workspace,
            ) {
                Ok(resolved) => resolved,
                Err(RuntimeError::Scheduling(_)) => {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        parent_execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "subworkflow inputs could not be resolved from immutable parent data",
                        )?),
                    );
                }
                Err(error) => return Err(error),
            };
            if resolved.is_empty() {
                if interface_field.is_required() || parent_declaration.is_required() {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        parent_execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "required subworkflow input is absent from immutable parent data",
                        )?),
                    );
                }
                continue;
            }
            if resolved.len() != 1 {
                return self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    parent_execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "subworkflow input resolved to more than one immutable value",
                    )?),
                );
            }
            let key = ValueKey::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let resolved_value = resolved.into_iter().next().ok_or_else(|| {
                RuntimeError::InvalidHistory("resolved subworkflow input disappeared".to_owned())
            })?;
            resolved_inputs.push((key, resolved_value));
        }
        let parent = projection.scopes().get(parent_scope).ok_or_else(|| {
            RuntimeError::InvalidHistory("subworkflow parent scope is absent".to_owned())
        })?;
        let subworkflow = self.next_subworkflow_id()?;
        let scope = WorkspaceScope::subworkflow(self.next_scope_id()?, parent, subworkflow.clone())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let scope_reference = scope.reference().clone();
        workspace.push(WorkspaceMutation::CreateScope {
            scope: scope.clone(),
        });
        let mut inputs = Vec::new();
        for (key, resolved_value) in resolved_inputs {
            let entry = self.materialize_subworkflow_input(
                projection,
                workspace,
                &scope_reference,
                parent_scope,
                key,
                resolved_value,
            )?;
            inputs.push(entry.reference().clone());
            workspace.push(WorkspaceMutation::PutValue { entry });
        }
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::SubworkflowCreated {
                subworkflow,
                parent_execution: parent_execution.clone(),
                child_run: self.next_run_id()?,
                child_revision: reference.revision().clone(),
                scope: scope.clone(),
                ownership: SubworkflowOwnership::Attached,
                inputs,
            },
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn drive_repeat_intent(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: &NodeExecutionId,
        scope_reference: &ScopeReference,
        config: &milkdrift_blueprint::RepeatConfig,
    ) -> Result<(), RuntimeError> {
        let latest = projection
            .iterations()
            .values()
            .filter(|iteration| iteration.repeat_execution() == execution)
            .max_by_key(|iteration| iteration.iteration_number())
            .map(|iteration| {
                (
                    iteration.iteration().clone(),
                    iteration.iteration_number(),
                    iteration.state(),
                )
            });
        let children: Vec<_> = projection
            .subworkflows()
            .values()
            .filter(|child| child.parent_execution() == execution)
            .map(|child| child.state())
            .collect();
        let latest_child_state = latest.as_ref().and_then(|(iteration, _, _)| {
            let iteration_scope = projection.iterations().get(iteration)?.scope().reference();
            projection
                .subworkflows()
                .values()
                .find(|child| {
                    child.parent_execution() == execution
                        && child.scope().parent() == Some(iteration_scope)
                })
                .map(|child| child.state())
        });

        let structurally_cancelling = cancellation_reason_for_execution(
            projection,
            execution,
            run_drain_reason(projection),
        )
        .is_some();
        if structurally_cancelling {
            if children.iter().any(|state| {
                matches!(
                    state,
                    SubworkflowState::Active | SubworkflowState::Cancelling
                )
            }) {
                return Ok(());
            }
            if let Some((iteration, _, IterationState::Active)) = latest.as_ref() {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::RepeatConditionRecorded {
                        iteration: iteration.clone(),
                        result: false,
                    },
                )?;
            }
            let last_iteration = latest.as_ref().map(|(iteration, _, _)| iteration.clone());
            if !projection.repeat_terminations().contains_key(execution) {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::RepeatTerminated {
                        repeat_execution: execution.clone(),
                        termination: RepeatTerminationReason::Cancelled,
                        last_iteration,
                    },
                )?;
            }
            return self.complete_deterministic_with_outcome(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
                NodeOutcome::Cancelled,
                None,
            );
        }

        if let Some((iteration, _, IterationState::ConditionRecorded(true))) = latest.as_ref() {
            if config.termination() == RepeatTermination::AwaitApproval {
                if let Some(continuation) = projection.repeat_continuations().get(execution) {
                    if continuation.is_rejected() {
                        let termination = continuation.requests().last().map_or(
                            RepeatTerminationReason::MaximumIterations,
                            |request| match request.cause() {
                                RepeatContinuationCause::IterationLimit => {
                                    RepeatTerminationReason::MaximumIterations
                                }
                                RepeatContinuationCause::DurationBudget { .. }
                                | RepeatContinuationCause::CostBudget { .. } => {
                                    RepeatTerminationReason::BudgetExhausted
                                }
                            },
                        );
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::RepeatTerminated {
                                repeat_execution: execution.clone(),
                                termination,
                                last_iteration: Some(iteration.clone()),
                            },
                        )?;
                        return self.complete_deterministic_with_outcome(
                            run,
                            occurred_at,
                            projection,
                            events,
                            node,
                            execution,
                            NodeOutcome::Failed,
                            Some(BoundedDetail::new(
                                "repeat continuation was rejected by authority",
                            )?),
                        );
                    }
                    if continuation.is_pending_approval() {
                        return Ok(());
                    }
                }
            }
        }

        let authority_budget_override = latest.as_ref().is_some_and(|(_, number, state)| {
            projection
                .repeat_continuations()
                .get(execution)
                .is_some_and(|continuation| {
                    !continuation.is_pending_approval()
                        && !continuation.is_rejected()
                        && continuation
                            .budget_override_iteration_limit()
                            .is_some_and(|limit| match state {
                                IterationState::Active => *number <= limit,
                                IterationState::ConditionRecorded(true) => *number < limit,
                                IterationState::ConditionRecorded(false)
                                | IterationState::Completed(_) => false,
                            })
                })
        });

        let budget_status = if authority_budget_override {
            RepeatBudgetStatus::Within
        } else {
            self.repeat_budget_exhaustion(config, projection, execution, occurred_at)?
        };
        if budget_status != RepeatBudgetStatus::Within {
            let accounting_overflow = budget_status == RepeatBudgetStatus::AccountingOverflow;
            let active_children: Vec<_> = projection
                .subworkflows()
                .values()
                .filter(|child| {
                    child.parent_execution() == execution
                        && matches!(
                            child.state(),
                            SubworkflowState::Active | SubworkflowState::Cancelling
                        )
                })
                .map(|child| {
                    (
                        child.subworkflow().clone(),
                        child.child_run().clone(),
                        child.state(),
                    )
                })
                .collect();
            for (subworkflow, child_run, state) in &active_children {
                if *state == SubworkflowState::Active {
                    self.push_projected_event(
                        run,
                        occurred_at,
                        projection,
                        events,
                        RunEventKind::SubworkflowCancellationRequested {
                            subworkflow: subworkflow.clone(),
                            child_run: child_run.clone(),
                            reason: Reason::new("repeat budget was exhausted")?,
                        },
                    )?;
                }
            }
            if !active_children.is_empty() {
                return Ok(());
            }
            if let Some((iteration, _, IterationState::Active)) = latest.as_ref() {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::RepeatConditionRecorded {
                        iteration: iteration.clone(),
                        result: config.termination() == RepeatTermination::AwaitApproval
                            && !accounting_overflow,
                    },
                )?;
            }
            if config.termination() == RepeatTermination::AwaitApproval && !accounting_overflow {
                if let Some((iteration, _, _)) = latest.as_ref() {
                    let RepeatBudgetStatus::Exhausted(cause) = budget_status else {
                        return Err(RuntimeError::InvalidHistory(
                            "repeat budget exhaustion has no typed continuation cause".to_owned(),
                        ));
                    };
                    return self.request_repeat_continuation(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        execution,
                        iteration,
                        config,
                        cause,
                    );
                }
            }
            let last_iteration = latest.as_ref().map(|(iteration, _, _)| iteration.clone());
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatTerminated {
                    repeat_execution: execution.clone(),
                    termination: RepeatTerminationReason::BudgetExhausted,
                    last_iteration,
                },
            )?;
            let has_success =
                latest_child_state == Some(SubworkflowState::Terminal(RunOutcome::Succeeded));
            let outcome = match (accounting_overflow, config.termination()) {
                (true, _) => NodeOutcome::Failed,
                (false, RepeatTermination::SucceedWithLatest) if has_success => {
                    NodeOutcome::Succeeded
                }
                (false, RepeatTermination::SucceedWithLatest | RepeatTermination::Fail) => {
                    NodeOutcome::Failed
                }
                (false, RepeatTermination::AwaitApproval) => {
                    return Err(RuntimeError::InvalidHistory(
                        "await-approval repeat reached an unreachable terminal branch".to_owned(),
                    ));
                }
            };
            if outcome == NodeOutcome::Succeeded {
                if let Some(iteration) = latest.as_ref().map(|(iteration, _, _)| iteration) {
                    self.publish_repeat_latest_outputs(
                        run,
                        occurred_at,
                        projection,
                        events,
                        workspace,
                        execution,
                        scope_reference,
                        iteration,
                    )?;
                }
            }
            return self.complete_deterministic_with_outcome(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
                outcome,
                if accounting_overflow {
                    Some(BoundedDetail::new(
                        "repeat cost accounting exceeded its durable numeric range",
                    )?)
                } else {
                    (outcome != NodeOutcome::Succeeded)
                        .then(|| BoundedDetail::new("repeat budget was exhausted"))
                        .transpose()?
                },
            );
        }

        if config.termination() == RepeatTermination::AwaitApproval {
            if let Some((iteration, iteration_number, IterationState::ConditionRecorded(true))) =
                latest.as_ref()
            {
                let effective_limit = projection.repeat_continuations().get(execution).map_or(
                    config.maximum_iterations(),
                    |continuation| {
                        continuation
                            .budget_override_iteration_limit()
                            .unwrap_or(continuation.effective_iteration_limit())
                    },
                );
                if *iteration_number < effective_limit {
                    return self.create_repeat_iteration(
                        run,
                        occurred_at,
                        projection,
                        events,
                        workspace,
                        node,
                        execution,
                        scope_reference,
                        config,
                        iteration_number.checked_add(1).ok_or_else(|| {
                            RuntimeError::Scheduling("repeat iteration number overflow".to_owned())
                        })?,
                    );
                }
                let cause = projection
                    .repeat_continuations()
                    .get(execution)
                    .and_then(|continuation| {
                        continuation
                            .budget_override_iteration_limit()
                            .filter(|limit| *iteration_number >= *limit)
                            .and_then(|_| continuation.requests().last())
                    })
                    .map_or(RepeatContinuationCause::IterationLimit, |request| {
                        request.cause().clone()
                    });
                return self.request_repeat_continuation(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    execution,
                    iteration,
                    config,
                    cause,
                );
            }
        }

        let Some((iteration, iteration_number, state)) = latest else {
            return self.create_repeat_iteration(
                run,
                occurred_at,
                projection,
                events,
                workspace,
                node,
                execution,
                scope_reference,
                config,
                1,
            );
        };
        if state != IterationState::Active
            || latest_child_state.is_none()
            || latest_child_state.is_some_and(|state| {
                matches!(
                    state,
                    SubworkflowState::Active | SubworkflowState::Cancelling
                )
            })
        {
            return Ok(());
        }
        let body_failed = matches!(
            latest_child_state,
            Some(SubworkflowState::Terminal(
                RunOutcome::Failed | RunOutcome::Cancelled
            ))
        );
        if body_failed {
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatConditionRecorded {
                    iteration: iteration.clone(),
                    result: false,
                },
            )?;
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatTerminated {
                    repeat_execution: execution.clone(),
                    termination: RepeatTerminationReason::BodyFailure,
                    last_iteration: Some(iteration.clone()),
                },
            )?;
            return self.complete_deterministic_with_outcome(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
                NodeOutcome::Failed,
                Some(BoundedDetail::new("the pinned repeat body failed")?),
            );
        }

        let context = match self.evaluation_context(
            node,
            projection,
            scope_reference,
            workspace,
        ) {
            Ok(context) => context,
            Err(RuntimeError::Scheduling(_)) => {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::RepeatTerminated {
                        repeat_execution: execution.clone(),
                        termination: RepeatTerminationReason::ConditionEvaluationFailed,
                        last_iteration: Some(iteration.clone()),
                    },
                )?;
                return self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "repeat condition inputs could not be resolved deterministically",
                    )?),
                );
            }
            Err(error) => return Err(error),
        };
        let result = match evaluate_condition(config.condition(), &context) {
            Ok(result) => result,
            Err(RuntimeError::Scheduling(_)) => {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::RepeatTerminated {
                        repeat_execution: execution.clone(),
                        termination: RepeatTerminationReason::ConditionEvaluationFailed,
                        last_iteration: Some(iteration.clone()),
                    },
                )?;
                return self.complete_deterministic_with_outcome(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    execution,
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "repeat condition could not be evaluated against immutable inputs",
                    )?),
                );
            }
            Err(error) => return Err(error),
        };
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::RepeatConditionRecorded {
                iteration: iteration.clone(),
                result,
            },
        )?;
        if !result {
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatTerminated {
                    repeat_execution: execution.clone(),
                    termination: RepeatTerminationReason::ConditionFalse,
                    last_iteration: Some(iteration.clone()),
                },
            )?;
            self.publish_repeat_latest_outputs(
                run,
                occurred_at,
                projection,
                events,
                workspace,
                execution,
                scope_reference,
                &iteration,
            )?;
            return self.complete_deterministic(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
            );
        }
        let effective_limit = projection.repeat_continuations().get(execution).map_or(
            config.maximum_iterations(),
            |continuation| {
                continuation
                    .budget_override_iteration_limit()
                    .unwrap_or(continuation.effective_iteration_limit())
            },
        );
        if iteration_number >= effective_limit {
            if config.termination() == RepeatTermination::AwaitApproval {
                let cause = projection
                    .repeat_continuations()
                    .get(execution)
                    .and_then(|continuation| {
                        continuation
                            .budget_override_iteration_limit()
                            .filter(|limit| iteration_number >= *limit)
                            .and_then(|_| continuation.requests().last())
                    })
                    .map_or(RepeatContinuationCause::IterationLimit, |request| {
                        request.cause().clone()
                    });
                return self.request_repeat_continuation(
                    run,
                    occurred_at,
                    projection,
                    events,
                    node,
                    execution,
                    &iteration,
                    config,
                    cause,
                );
            }
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatTerminated {
                    repeat_execution: execution.clone(),
                    termination: RepeatTerminationReason::MaximumIterations,
                    last_iteration: Some(iteration.clone()),
                },
            )?;
            let (outcome, detail) = match config.termination() {
                RepeatTermination::SucceedWithLatest => (NodeOutcome::Succeeded, None),
                RepeatTermination::Fail => (
                    NodeOutcome::Failed,
                    Some(BoundedDetail::new(
                        "repeat reached its maximum iteration bound",
                    )?),
                ),
                RepeatTermination::AwaitApproval => {
                    return Err(RuntimeError::InvalidHistory(
                        "await-approval repeat reached an unreachable terminal branch".to_owned(),
                    ));
                }
            };
            if outcome == NodeOutcome::Succeeded {
                self.publish_repeat_latest_outputs(
                    run,
                    occurred_at,
                    projection,
                    events,
                    workspace,
                    execution,
                    scope_reference,
                    &iteration,
                )?;
            }
            return self.complete_deterministic_with_outcome(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
                outcome,
                detail,
            );
        }
        self.create_repeat_iteration(
            run,
            occurred_at,
            projection,
            events,
            workspace,
            node,
            execution,
            scope_reference,
            config,
            iteration_number.checked_add(1).ok_or_else(|| {
                RuntimeError::Scheduling("repeat iteration number overflow".to_owned())
            })?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn request_repeat_continuation(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        node: &Node,
        execution: &NodeExecutionId,
        frontier_iteration: &IterationId,
        config: &milkdrift_blueprint::RepeatConfig,
        cause: RepeatContinuationCause,
    ) -> Result<(), RuntimeError> {
        let continuation = projection.repeat_continuations().get(execution);
        if continuation.is_some_and(|value| value.is_pending_approval()) {
            return Ok(());
        }
        let (initial_iteration_limit, effective_iteration_limit, request_count) = continuation
            .map_or(
                (config.maximum_iterations(), config.maximum_iterations(), 0),
                |value| {
                    (
                        value.initial_iteration_limit(),
                        value.effective_iteration_limit(),
                        value.requests().len(),
                    )
                },
            );
        if request_count >= MAX_REPEAT_CONTINUATION_DECISIONS {
            let termination = match cause {
                RepeatContinuationCause::IterationLimit => {
                    RepeatTerminationReason::MaximumIterations
                }
                RepeatContinuationCause::DurationBudget { .. }
                | RepeatContinuationCause::CostBudget { .. } => {
                    RepeatTerminationReason::BudgetExhausted
                }
            };
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::RepeatTerminated {
                    repeat_execution: execution.clone(),
                    termination,
                    last_iteration: Some(frontier_iteration.clone()),
                },
            )?;
            return self.complete_deterministic_with_outcome(
                run,
                occurred_at,
                projection,
                events,
                node,
                execution,
                NodeOutcome::Failed,
                Some(BoundedDetail::new(
                    "repeat continuation reached its hard authority-cycle bound",
                )?),
            );
        }
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::RepeatContinuationRequested {
                repeat_execution: execution.clone(),
                frontier_iteration: frontier_iteration.clone(),
                initial_iteration_limit,
                effective_iteration_limit,
                cause,
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_repeat_iteration(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        node: &Node,
        execution: &NodeExecutionId,
        scope_reference: &ScopeReference,
        config: &milkdrift_blueprint::RepeatConfig,
        iteration_number: u32,
    ) -> Result<(), RuntimeError> {
        let parent = projection.scopes().get(scope_reference).ok_or_else(|| {
            RuntimeError::InvalidHistory("repeat execution scope is absent".to_owned())
        })?;
        let iteration = self.next_iteration_id()?;
        let scope = WorkspaceScope::iteration(self.next_scope_id()?, parent, iteration.clone())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let iteration_scope = scope.reference().clone();
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::RepeatIterationCreated {
                repeat_execution: execution.clone(),
                iteration,
                iteration_number,
                scope: scope.clone(),
            },
        )?;
        workspace.push(WorkspaceMutation::CreateScope { scope });
        self.create_subworkflow_intent(
            run,
            occurred_at,
            projection,
            events,
            workspace,
            node,
            execution,
            scope_reference,
            &iteration_scope,
            config.body(),
        )
    }

    fn repeat_budget_exhaustion(
        &self,
        config: &milkdrift_blueprint::RepeatConfig,
        projection: &RunProjection,
        execution: &NodeExecutionId,
        observed_at: TimestampMillis,
    ) -> Result<RepeatBudgetStatus, RuntimeError> {
        if let Some(maximum) = config.budget().max_duration_ms {
            let created_at = projection
                .node_executions()
                .get(execution)
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory("repeat execution is absent".to_owned())
                })?
                .created_at();
            let observed = observed_at.get().saturating_sub(created_at.get());
            if observed >= maximum {
                return Ok(RepeatBudgetStatus::Exhausted(
                    RepeatContinuationCause::DurationBudget {
                    maximum_ms: maximum,
                    observed_ms: observed,
                    },
                ));
            }
        }
        let Some(maximum_cost) = config.budget().max_cost_micros else {
            return Ok(RepeatBudgetStatus::Within);
        };
        let configured_currency = config.budget().max_cost_currency.as_ref().ok_or_else(|| {
            RuntimeError::InvalidHistory("repeat cost budget has no configured currency".to_owned())
        })?;
        let currency = CurrencyCode::new(configured_currency.as_str().to_owned())?;
        let mut observed_cost = 0_u64;
        for child in projection
            .subworkflows()
            .values()
            .filter(|child| child.parent_execution() == execution)
        {
            if self.store.head(child.child_run())? == RunSequence::ZERO {
                continue;
            }
            let child_projection = self.projection(child.child_run())?;
            if let Some(cost) = child_projection
                .resource_usage()
                .cost_micros()
                .get(&currency)
            {
                let Some(total) = observed_cost.checked_add(*cost) else {
                    return Ok(RepeatBudgetStatus::AccountingOverflow);
                };
                observed_cost = total;
            }
        }
        if observed_cost >= maximum_cost {
            Ok(RepeatBudgetStatus::Exhausted(
                RepeatContinuationCause::CostBudget {
                    maximum_micros: maximum_cost,
                    observed_micros: observed_cost,
                    currency,
                },
            ))
        } else {
            Ok(RepeatBudgetStatus::Within)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_repeat_latest_outputs(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        execution: &NodeExecutionId,
        execution_scope: &ScopeReference,
        iteration: &IterationId,
    ) -> Result<(), RuntimeError> {
        let iteration_scope = projection
            .iterations()
            .get(iteration)
            .ok_or_else(|| RuntimeError::InvalidHistory("repeat iteration is absent".to_owned()))?
            .scope()
            .reference()
            .clone();
        let imports: Vec<_> = projection
            .subworkflows()
            .values()
            .find(|child| {
                child.parent_execution() == execution
                    && child.scope().parent() == Some(&iteration_scope)
                    && child.state() == SubworkflowState::Terminal(RunOutcome::Succeeded)
            })
            .map(|child| {
                child
                    .imports()
                    .iter()
                    .map(|import| import.parent_value().clone())
                    .collect()
            })
            .unwrap_or_default();
        for imported in imports {
            let source = self.projected_workspace_value(projection, &imported, workspace)?;
            let output = self.projected_output_entry(
                projection,
                execution_scope,
                source.reference().key().clone(),
                source.value().clone(),
                workspace,
            )?;
            let reference = output.reference().clone();
            workspace.push(WorkspaceMutation::PutValue { entry: output });
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::DeterministicOutputPublished {
                    execution: execution.clone(),
                    value: reference,
                    artifact: None,
                },
            )?;
        }
        Ok(())
    }

    fn resolve_node_port_inputs(
        &self,
        revision: &BlueprintRevision,
        projection: &RunProjection,
        node: &Node,
        port: &PortId,
        occurrence_scope: &ScopeReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Vec<ResolvedInputValue>, RuntimeError> {
        self.validate_projected_scope(projection, occurrence_scope, pending_workspace)?;
        let declaration = node.data_inputs().get(port).ok_or_else(|| {
            RuntimeError::InvalidHistory(format!(
                "node {} has no declared data input {port}",
                node.id()
            ))
        })?;
        if let Some(binding) = declaration.binding() {
            return Ok(self
                .resolve_optional_binding(
                    projection,
                    node.id(),
                    occurrence_scope,
                    binding,
                    pending_workspace,
                    true,
                )?
                .into_iter()
                .collect());
        }
        let references =
            self.incoming_data_references(revision, projection, node, port, occurrence_scope);
        for reference in &references {
            self.projected_workspace_value(projection, reference, pending_workspace)?;
        }
        Ok(references
            .into_iter()
            .map(ResolvedInputValue::Workspace)
            .collect())
    }

    fn incoming_data_references(
        &self,
        revision: &BlueprintRevision,
        projection: &RunProjection,
        node: &Node,
        port: &PortId,
        occurrence_scope: &ScopeReference,
    ) -> BTreeSet<WorkspaceValueReference> {
        let mut references = BTreeSet::new();
        for edge in revision.semantic().edges().values().filter(|edge| {
            edge.kind() == EdgeKind::Data
                && edge.target_node() == node.id()
                && edge.target_port() == port
        }) {
            for execution in projection
                .executions_for_node(edge.source_node())
                .filter(|source| {
                    source_execution_is_valid_for_occurrence(
                        projection,
                        source,
                        node.id(),
                        occurrence_scope,
                    )
                        && execution_scope_related(projection, source.scope(), occurrence_scope)
                        && source.state() == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                })
            {
                references.extend(
                    execution
                        .outputs()
                        .iter()
                        .filter(|output| {
                            output.value().key().as_str() == edge.source_port().as_str()
                        })
                        .map(|output| output.value().clone()),
                );
            }
        }
        references
    }

    fn ordered_reducer_references(
        &self,
        revision: &BlueprintRevision,
        projection: &RunProjection,
        node: &Node,
        port: &PortId,
        occurrence_scope: &ScopeReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Vec<WorkspaceValueReference>, RuntimeError> {
        self.validate_projected_scope(projection, occurrence_scope, pending_workspace)?;
        let mut candidates = Vec::new();
        for (edge_order, edge) in revision
            .semantic()
            .edges()
            .values()
            .filter(|edge| {
                edge.kind() == EdgeKind::Data
                    && edge.target_node() == node.id()
                    && edge.target_port() == port
            })
            .enumerate()
        {
            for execution in projection
                .executions_for_node(edge.source_node())
                .filter(|source| {
                    source_execution_is_valid_for_occurrence(
                        projection,
                        source,
                        node.id(),
                        occurrence_scope,
                    )
                        && execution_scope_related(projection, source.scope(), occurrence_scope)
                        && source.state() == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                })
            {
                let branch_order = projection
                    .branches()
                    .values()
                    .find(|branch| branch.children().contains(execution.execution()))
                    .and_then(|branch| {
                        projection
                            .node_executions()
                            .get(branch.fork_execution())
                            .map(|fork| {
                                (
                                    fork.created_sequence().get(),
                                    branch.port().as_str().to_owned(),
                                )
                            })
                    });
                for output in execution
                    .outputs()
                    .iter()
                    .filter(|output| output.value().key().as_str() == edge.source_port().as_str())
                {
                    let (class, owner_order, port_order) = branch_order.clone().map_or_else(
                        || {
                            (
                                1_u8,
                                u64::try_from(edge_order).unwrap_or(u64::MAX),
                                String::new(),
                            )
                        },
                        |(fork_sequence, branch_port)| (0_u8, fork_sequence, branch_port),
                    );
                    candidates.push((
                        class,
                        owner_order,
                        port_order,
                        execution.created_sequence().get(),
                        output.sequence().get(),
                        output.value().clone(),
                    ));
                }
            }
        }
        candidates.sort();
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        for (_, _, _, _, _, reference) in candidates {
            if seen.insert(reference.clone()) {
                self.projected_workspace_value(projection, &reference, pending_workspace)?;
                ordered.push(reference);
            }
        }
        Ok(ordered)
    }

    fn resolve_optional_binding(
        &self,
        projection: &RunProjection,
        occurrence_node: &NodeId,
        occurrence_scope: &ScopeReference,
        binding: &BindingSource,
        pending_workspace: &[WorkspaceMutation],
        apply_path: bool,
    ) -> Result<Option<ResolvedInputValue>, RuntimeError> {
        self.validate_projected_scope(projection, occurrence_scope, pending_workspace)?;
        match binding {
            BindingSource::Literal { value } => Ok(Some(ResolvedInputValue::Inline {
                value: value.clone(),
                source: None,
            })),
            BindingSource::WorkflowInput { field }
            | BindingSource::SubworkflowParameter { field } => {
                let key = ValueKey::new(field.as_str().to_owned())
                    .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
                let reference = projection
                    .inputs()
                    .iter()
                    .find(|reference| reference.key() == &key)
                    .cloned();
                if reference.is_none() {
                    let input_scope = projection
                        .root_scope()
                        .ok_or_else(|| {
                            RuntimeError::InvalidHistory(
                                "workflow input lookup has no projected root scope".to_owned(),
                            )
                        })?
                        .reference();
                    // Absence is itself an integrity claim. Never select a later
                    // value by key as an immutable creation input, but still
                    // compare the durable latest row with replay-derived state so
                    // an injected unprojected row cannot turn omission into input.
                    let initial = WorkspaceValueReference::new(
                        input_scope.clone(),
                        key.clone(),
                        ValueVersion::FIRST,
                    );
                    if !projection.workspace_values().contains(&initial)
                        && self.workspace_value(&initial, pending_workspace)?.is_some()
                    {
                        return Err(RuntimeError::InvalidHistory(format!(
                            "durable workspace contains an orphan initial input {}:{}",
                            input_scope.scope(),
                            key
                        )));
                    }
                    let _ = self.projected_latest_workspace_value(
                        projection,
                        input_scope,
                        &key,
                        pending_workspace,
                    )?;
                }
                reference
                    .map(|reference| {
                        self.projected_workspace_value(
                            projection,
                            &reference,
                            pending_workspace,
                        )
                        .map(|_| ResolvedInputValue::Workspace(reference))
                    })
                    .transpose()
            }
            BindingSource::NodeOutput { node, port, path } => {
                let references: BTreeSet<_> = projection
                    .executions_for_node(node)
                    .filter(|source| {
                        source_execution_is_valid_for_occurrence(
                            projection,
                            source,
                            occurrence_node,
                            occurrence_scope,
                        )
                            && execution_scope_related(
                                projection,
                                source.scope(),
                                occurrence_scope,
                            )
                            && source.state()
                                == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                    })
                    .flat_map(|source| source.outputs())
                    .filter(|output| output.value().key().as_str() == port.as_str())
                    .map(|output| output.value().clone())
                    .collect();
                for reference in &references {
                    self.projected_workspace_value(projection, reference, pending_workspace)?;
                }
                if references.is_empty() {
                    return Ok(None);
                }
                if references.len() != 1 {
                    return Err(RuntimeError::Scheduling(format!(
                        "required node output {node}:{port} resolved to {} values",
                        references.len()
                    )));
                }
                let reference = references.into_iter().next().ok_or_else(|| {
                    RuntimeError::InvalidHistory("resolved node output disappeared".to_owned())
                })?;
                if path.segments().is_empty() || !apply_path {
                    return Ok(Some(ResolvedInputValue::Workspace(reference)));
                }
                let entry =
                    self.projected_workspace_value(projection, &reference, pending_workspace)?;
                let json_value = entry.value().as_json().ok_or_else(|| {
                    RuntimeError::Scheduling(format!(
                        "node output {node}:{port} is an artifact and cannot be path-selected"
                    ))
                })?;
                let selected = select_json_path(json_value, path.segments())?;
                Ok(Some(ResolvedInputValue::Inline {
                    value: selected,
                    source: Some(reference),
                }))
            }
            BindingSource::WorkspaceValue { reference, .. } => {
                let parsed = serde_json::from_str::<WorkspaceValueReference>(reference).map_err(
                    |error| {
                        RuntimeError::Scheduling(format!(
                            "workspace binding is not an exact canonical reference: {error}"
                        ))
                    },
                )?;
                self.projected_workspace_value(projection, &parsed, pending_workspace)?;
                self.ensure_readable_ancestor(
                    projection,
                    parsed.scope(),
                    occurrence_scope,
                    pending_workspace,
                )?;
                Ok(Some(ResolvedInputValue::Workspace(parsed)))
            }
            BindingSource::Artifact { reference, .. } => {
                let parsed =
                    serde_json::from_str::<ArtifactReference>(reference).map_err(|error| {
                        RuntimeError::Scheduling(format!(
                            "artifact binding is not an exact canonical reference: {error}"
                        ))
                    })?;
                if !self.store.is_committed(&parsed)? {
                    return Err(RuntimeError::Scheduling(
                        "artifact binding references uncommitted content".to_owned(),
                    ));
                }
                Ok(Some(ResolvedInputValue::Artifact(parsed)))
            }
        }
    }

    fn workspace_value(
        &self,
        reference: &WorkspaceValueReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Option<WorkspaceValueEntry>, RuntimeError> {
        if let Some(entry) = pending_workspace
            .iter()
            .rev()
            .find_map(|mutation| match mutation {
                WorkspaceMutation::PutValue { entry } if entry.reference() == reference => {
                    Some(entry.clone())
                }
                WorkspaceMutation::CreateScope { .. } | WorkspaceMutation::PutValue { .. } => None,
            })
        {
            return Ok(Some(entry));
        }
        self.store.value(reference).map_err(RuntimeError::from)
    }

    fn validate_projected_scope(
        &self,
        projection: &RunProjection,
        reference: &ScopeReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<(), RuntimeError> {
        let expected = projection.scopes().get(reference).ok_or_else(|| {
            RuntimeError::InvalidHistory(format!(
                "projected workspace scope {}:{} is absent",
                reference.run(),
                reference.scope()
            ))
        })?;
        if let Some(pending) = pending_workspace
            .iter()
            .rev()
            .find_map(|mutation| match mutation {
                WorkspaceMutation::CreateScope { scope } if scope.reference() == reference => {
                    Some(scope)
                }
                WorkspaceMutation::CreateScope { .. } | WorkspaceMutation::PutValue { .. } => None,
            })
        {
            if pending != expected {
                return Err(RuntimeError::InvalidHistory(format!(
                    "pending workspace scope {}:{} contradicts its projection",
                    reference.run(),
                    reference.scope()
                )));
            }
            return Ok(());
        }
        let durable = self
            .store
            .scope(reference.run(), reference.scope())?
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(format!(
                    "projected workspace scope {}:{} is absent from durable storage",
                    reference.run(),
                    reference.scope()
                ))
            })?;
        if &durable != expected {
            return Err(RuntimeError::InvalidHistory(format!(
                "durable workspace scope {}:{} contradicts its projection",
                reference.run(),
                reference.scope()
            )));
        }
        Ok(())
    }

    fn projected_workspace_value(
        &self,
        projection: &RunProjection,
        reference: &WorkspaceValueReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<WorkspaceValueEntry, RuntimeError> {
        self.validate_projected_scope(projection, reference.scope(), pending_workspace)?;
        if !projection.workspace_values().contains(reference) {
            return Err(RuntimeError::InvalidHistory(format!(
                "workspace value {}:{}:{} is absent from its event projection",
                reference.scope().scope(),
                reference.key(),
                reference.version()
            )));
        }
        let entry = self
            .workspace_value(reference, pending_workspace)?
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(format!(
                    "projected workspace value {}:{}:{} is absent from durable storage",
                    reference.scope().scope(),
                    reference.key(),
                    reference.version()
                ))
            })?;
        if entry.reference() != reference {
            return Err(RuntimeError::InvalidHistory(
                "durable workspace value contradicts its exact projected reference".to_owned(),
            ));
        }
        Ok(entry)
    }

    fn projected_latest_workspace_value(
        &self,
        projection: &RunProjection,
        scope: &ScopeReference,
        key: &ValueKey,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Option<WorkspaceValueEntry>, RuntimeError> {
        self.validate_projected_scope(projection, scope, pending_workspace)?;
        let expected = projection
            .workspace_values()
            .iter()
            .filter(|reference| reference.scope() == scope && reference.key() == key)
            .max_by_key(|reference| reference.version());
        let pending = pending_workspace
            .iter()
            .filter_map(|mutation| match mutation {
                WorkspaceMutation::PutValue { entry }
                    if entry.reference().scope() == scope && entry.reference().key() == key =>
                {
                    Some(entry)
                }
                WorkspaceMutation::CreateScope { .. } | WorkspaceMutation::PutValue { .. } => None,
            })
            .max_by_key(|entry| entry.reference().version())
            .cloned();
        if pending
            .as_ref()
            .is_some_and(|entry| !projection.workspace_values().contains(entry.reference()))
        {
            return Err(RuntimeError::InvalidHistory(format!(
                "pending workspace contains an unprojected latest value {}:{}",
                scope.scope(),
                key
            )));
        }
        let durable = self.store.latest_value(scope, key)?;
        if durable
            .as_ref()
            .is_some_and(|entry| !projection.workspace_values().contains(entry.reference()))
        {
            return Err(RuntimeError::InvalidHistory(format!(
                "durable workspace contains orphan latest value {}:{}",
                scope.scope(),
                key
            )));
        }
        if pending
            .as_ref()
            .zip(durable.as_ref())
            .is_some_and(|(pending, durable)| pending.reference() == durable.reference())
        {
            return Err(RuntimeError::InvalidHistory(format!(
                "pending workspace duplicates durable latest value {}:{}",
                scope.scope(),
                key
            )));
        }
        let observed = match (pending, durable) {
            (Some(pending), Some(durable))
                if pending.reference().version() < durable.reference().version() =>
            {
                Some(durable)
            }
            (Some(pending), Some(_)) => Some(pending),
            (Some(pending), None) => Some(pending),
            (None, Some(durable)) => Some(durable),
            (None, None) => None,
        };
        match (expected, observed) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(RuntimeError::InvalidHistory(format!(
                "durable workspace contains orphan latest value {}:{}",
                scope.scope(),
                key
            ))),
            (Some(expected), None) => Err(RuntimeError::InvalidHistory(format!(
                "projected latest workspace value {}:{}:{} is absent from durable storage",
                scope.scope(),
                key,
                expected.version()
            ))),
            (Some(expected), Some(entry)) if entry.reference() == expected => Ok(Some(entry)),
            (Some(expected), Some(entry)) => Err(RuntimeError::InvalidHistory(format!(
                "durable latest workspace value {}:{}:{} contradicts projected version {}",
                scope.scope(),
                key,
                entry.reference().version(),
                expected.version()
            ))),
        }
    }

    fn materialize_subworkflow_input(
        &self,
        projection: &RunProjection,
        pending_workspace: &[WorkspaceMutation],
        target_scope: &ScopeReference,
        target_parent_scope: &ScopeReference,
        key: ValueKey,
        value: ResolvedInputValue,
    ) -> Result<WorkspaceValueEntry, RuntimeError> {
        match value {
            ResolvedInputValue::Inline { value, source } => {
                if let Some(source) = source {
                    self.ensure_readable_ancestor(
                        projection,
                        source.scope(),
                        target_parent_scope,
                        pending_workspace,
                    )?;
                    WorkspaceValueEntry::inherited(
                        target_scope.clone(),
                        key,
                        source,
                        WorkspaceValue::Json(value),
                    )
                    .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
                } else {
                    Ok(WorkspaceValueEntry::initial(
                        target_scope.clone(),
                        key,
                        WorkspaceValue::Json(value),
                    ))
                }
            }
            ResolvedInputValue::Workspace(source) => {
                self.ensure_readable_ancestor(
                    projection,
                    source.scope(),
                    target_parent_scope,
                    pending_workspace,
                )?;
                let entry =
                    self.projected_workspace_value(projection, &source, pending_workspace)?;
                WorkspaceValueEntry::inherited(
                    target_scope.clone(),
                    key,
                    source,
                    entry.value().clone(),
                )
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
            }
            ResolvedInputValue::Artifact(reference) => Ok(WorkspaceValueEntry::initial(
                target_scope.clone(),
                key,
                WorkspaceValue::Artifact(reference),
            )),
        }
    }

    fn ensure_readable_ancestor(
        &self,
        projection: &RunProjection,
        source: &ScopeReference,
        target_parent: &ScopeReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<(), RuntimeError> {
        self.validate_projected_scope(projection, source, pending_workspace)?;
        let mut cursor = Some(target_parent);
        for _ in 0..=milkdrift_workspace::MAX_SCOPE_DEPTH {
            let Some(scope) = cursor else {
                break;
            };
            self.validate_projected_scope(projection, scope, pending_workspace)?;
            if scope == source {
                return Ok(());
            }
            cursor = projection
                .scopes()
                .get(scope)
                .and_then(WorkspaceScope::parent);
        }
        Err(RuntimeError::InvalidTransition(
            "workspace input aliases a sibling or unrelated scope; scope isolation forbids the read"
                .to_owned(),
        ))
    }

    fn push_projected_event(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        kind: RunEventKind,
    ) -> Result<(), RuntimeError> {
        if events.len() >= milkdrift_persistence::MAX_EVENTS_PER_COMMIT {
            return Err(RuntimeError::Scheduling(
                "event commit bound reached while driving structured work".to_owned(),
            ));
        }
        let event = RunEventEnvelope::new(
            self.next_event_id()?,
            run.clone(),
            projection.sequence().next()?,
            occurred_at,
            kind,
        )?;
        projection.apply_replayed(&event)?;
        events.push(event);
        Ok(())
    }

    fn complete_deterministic(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        node: &Node,
        execution: &NodeExecutionId,
    ) -> Result<(), RuntimeError> {
        self.complete_deterministic_with_outcome(
            run,
            occurred_at,
            projection,
            events,
            node,
            execution,
            NodeOutcome::Succeeded,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn complete_deterministic_with_outcome(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        node: &Node,
        execution: &NodeExecutionId,
        outcome: NodeOutcome,
        detail: Option<BoundedDetail>,
    ) -> Result<(), RuntimeError> {
        if outcome == NodeOutcome::Cancelled {
            return self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::NodeExecutionCancelledBeforeDispatch {
                    execution: execution.clone(),
                    reason: Reason::new(
                        "deterministic execution was cancelled by its structured owner",
                    )?,
                },
            );
        }
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::DeterministicNodeTerminal {
                execution: execution.clone(),
                outcome,
                error_class: matches!(outcome, NodeOutcome::Failed | NodeOutcome::Rejected)
                    .then_some(ErrorClass::Unknown),
                detail,
            },
        )?;
        if outcome == NodeOutcome::Succeeded
            && matches!(node.kind(), NodeKind::Fork { .. } | NodeKind::Terminal { .. })
        {
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::StructuredSuccessorScanCompleted {
                    execution: execution.clone(),
                },
            )?;
        }
        Ok(())
    }

    fn try_finalize_run(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &[WorkspaceMutation],
    ) -> Result<(), RuntimeError> {
        if projection.is_completed()
            || !projection.pending_successor_execution_ids().is_empty()
            || projection.has_active_owned_work()
        {
            return Ok(());
        }

        let mut terminal_executions: Vec<_> = projection
            .node_executions()
            .values()
            .filter_map(|execution| {
                let node = revision.semantic().nodes().get(execution.node())?;
                match node.kind() {
                    NodeKind::Terminal { outcome } => {
                        Some((execution.created_sequence(), execution, *outcome))
                    }
                    _ => None,
                }
            })
            .collect();
        terminal_executions.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.execution().cmp(right.1.execution()))
        });
        let joined_branches: BTreeSet<_> = projection
            .joins()
            .values()
            .flat_map(|join| join.branches().iter().map(|result| result.branch.clone()))
            .collect();
        let unjoined_failed_terminal = terminal_executions.iter().any(
            |(_, execution, terminal)| {
                if *terminal != TerminalOutcome::Failure {
                    return false;
                }
                projection
                    .branches()
                    .values()
                    .find(|branch| branch.children().contains(execution.execution()))
                    .is_none_or(|branch| !joined_branches.contains(branch.branch()))
            },
        );
        let unjoined_failed_branch = projection.branches().values().any(|branch| {
            branch.state() == BranchState::Completed(RunOutcome::Failed)
                && !joined_branches.contains(branch.branch())
        });
        let outcome = if projection.lifecycle() == RunLifecycle::Cancelling {
            RunOutcome::Cancelled
        } else if let Some(termination) = projection.termination() {
            termination.outcome()
        } else if unjoined_failed_terminal
            || unjoined_failed_branch
            || (terminal_executions.is_empty()
                && projection.node_executions().values().any(|execution| {
                    matches!(
                        execution.state(),
                        NodeExecutionState::Terminal(NodeOutcome::Failed | NodeOutcome::Rejected)
                    )
                }))
        {
            RunOutcome::Failed
        } else {
            RunOutcome::Succeeded
        };
        if outcome == RunOutcome::Cancelled && projection.cancellation().is_none() {
            return Ok(());
        }
        let mut outputs = BTreeSet::new();
        let mut artifacts = BTreeSet::new();
        if let Some((_, terminal_execution, _)) = terminal_executions.last() {
            let terminal_node = revision
                .semantic()
                .nodes()
                .get(terminal_execution.node())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory("terminal node is absent".to_owned())
                })?;
            for (field, declaration) in revision.semantic().interface().outputs() {
                let _terminal_port = terminal_node
                    .data_inputs()
                    .keys()
                    .find(|port| port.as_str() == field.as_str());
                let resolved = terminal_execution
                    .outputs()
                    .iter()
                    .find(|output| output.value().key().as_str() == field.as_str())
                    .map(|output| output.value().clone());
                match resolved {
                    Some(reference) => {
                        if let Some(artifact) = self
                            .projected_workspace_value(projection, &reference, workspace)?
                            .value()
                            .as_artifact()
                            .cloned()
                        {
                            if projection.artifacts().contains_key(artifact.artifact()) {
                                artifacts.insert(artifact);
                            }
                        }
                        outputs.insert(reference);
                    }
                    None if declaration.is_required() && outcome == RunOutcome::Succeeded => {
                        return Err(RuntimeError::InvalidHistory(format!(
                            "required workflow output {field} is unresolved at terminal boundary"
                        )));
                    }
                    None => {}
                }
            }
        }
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::RunTerminal {
                outcome,
                outputs: outputs.into_iter().collect(),
                artifacts: artifacts.into_iter().collect(),
                reason: projection
                    .cancellation()
                    .map(|cancellation| cancellation.reason().clone())
                    .or_else(|| {
                        projection
                            .termination()
                            .map(|termination| termination.reason().clone())
                    }),
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_success_terminal_outputs(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
        terminal_node: &Node,
        execution: &NodeExecutionId,
        scope: &ScopeReference,
    ) -> Result<bool, RuntimeError> {
        for (field, declaration) in revision.semantic().interface().outputs() {
            let port = PortId::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            if projection
                .node_executions()
                .get(execution)
                .is_some_and(|view| {
                    view.outputs()
                        .iter()
                        .any(|output| output.value().key().as_str() == field.as_str())
                })
            {
                continue;
            }
            let Some(_port_declaration) = terminal_node.data_inputs().get(&port) else {
                if declaration.is_required() {
                    return Err(RuntimeError::InvalidHistory(format!(
                        "required terminal workflow output {field} has no declared terminal port"
                    )));
                }
                continue;
            };
            let mut resolved = self.resolve_node_port_inputs(
                revision,
                projection,
                terminal_node,
                &port,
                scope,
                workspace,
            )?;
            if resolved.is_empty() {
                if declaration.is_required() {
                    return Err(RuntimeError::Scheduling(format!(
                        "required terminal workflow output {field} did not resolve from immutable inputs"
                    )));
                }
                continue;
            }
            if resolved.len() != 1 {
                return Err(RuntimeError::InvalidHistory(format!(
                    "terminal workflow output {field} resolved to more than one exact value"
                )));
            }
            if events.len().saturating_add(2) >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(false);
            }
            let resolved = resolved.pop().ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "resolved terminal workflow output disappeared".to_owned(),
                )
            })?;
            let key = ValueKey::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let value = match resolved {
                ResolvedInputValue::Inline { value, .. } => WorkspaceValue::Json(value),
                ResolvedInputValue::Workspace(reference) => {
                    let entry =
                        self.projected_workspace_value(projection, &reference, workspace)?;
                    entry.value().clone()
                }
                ResolvedInputValue::Artifact(reference) => WorkspaceValue::Artifact(reference),
            };
            let artifact = value.as_artifact().cloned();
            if let Some(artifact) = &artifact {
                if !projection.artifacts().contains_key(artifact.artifact()) {
                    let metadata = self.store.metadata(artifact.artifact())?.ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "terminal output artifact metadata is absent".to_owned(),
                        )
                    })?;
                    if metadata.reference() != artifact {
                        return Err(RuntimeError::InvalidHistory(
                            "terminal output artifact metadata contradicts its binding".to_owned(),
                        ));
                    }
                    self.push_projected_event(
                        run,
                        occurred_at,
                        projection,
                        events,
                        RunEventKind::ArtifactPublished { metadata },
                    )?;
                }
            }
            let entry = self.projected_output_entry(projection, scope, key, value, workspace)?;
            let reference = entry.reference().clone();
            workspace.push(WorkspaceMutation::PutValue { entry });
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::DeterministicOutputPublished {
                    execution: execution.clone(),
                    value: reference,
                    artifact,
                },
            )?;
        }
        Ok(true)
    }

    fn close_finished_branches(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        scan_remaining: &mut usize,
    ) -> Result<(), RuntimeError> {
        let mut terminal = Vec::new();
        for branch_id in self.scan_branch_ids(run, projection, scan_remaining)? {
            let branch = projection.branches().get(&branch_id).ok_or_else(|| {
                RuntimeError::InvalidHistory("scanned branch identity is absent".to_owned())
            })?;
            if !matches!(
                branch.state(),
                BranchState::Active | BranchState::Cancelling
            ) || projection.branch_has_active_descendant_ownership(branch.branch())
            {
                continue;
            }
            let fork = projection
                .node_executions()
                .get(branch.fork_execution())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "branch owner fork execution is absent".to_owned(),
                    )
            })?;
            let branch_revision = self.revision_for_execution(projection, fork.execution())?;
            let mut frontier = Vec::new();
            for child_node in branch_revision.semantic().nodes().values() {
                let Some(child) = projection.latest_descendant_execution(
                    branch.scope().reference(),
                    child_node.id(),
                ) else {
                    continue;
                };
                let reaches_owning_join = branch_revision.semantic().edges().values().any(|edge| {
                    edge.kind() == EdgeKind::Control
                        && edge.source_node() == child.node()
                        && branch_revision
                            .semantic()
                            .nodes()
                            .get(edge.target_node())
                            .is_some_and(|target| {
                                matches!(
                                    target.kind(),
                                    NodeKind::Join { config } if config.fork() == fork.node()
                                )
                            })
                });
                let is_explicit_terminal = branch_revision
                    .semantic()
                    .nodes()
                    .get(child.node())
                    .is_some_and(|node| matches!(node.kind(), NodeKind::Terminal { .. }));
                let stopped_before_join = child.state()
                    != &NodeExecutionState::Terminal(NodeOutcome::Succeeded);
                if reaches_owning_join || is_explicit_terminal || stopped_before_join {
                    frontier.push(child);
                }
            }
            // A successfully completed nested fork is not the enclosing branch's
            // terminal frontier. Its inner join (or a later outer-scope successor)
            // must become durable before the enclosing branch may close.
            if frontier.is_empty() {
                continue;
            }
            let outcome = if branch.state() == BranchState::Cancelling {
                RunOutcome::Cancelled
            } else if frontier.iter().all(|child| {
                child.state() == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
            }) {
                RunOutcome::Succeeded
            } else if frontier.iter().all(|child| {
                matches!(
                    child.state(),
                    NodeExecutionState::Terminal(NodeOutcome::Succeeded | NodeOutcome::Cancelled)
                )
            }) {
                RunOutcome::Cancelled
            } else {
                RunOutcome::Failed
            };
            let owning_join = branch_revision
                .semantic()
                .nodes()
                .values()
                .find_map(|node| match node.kind() {
                    NodeKind::Join { config } if config.fork() == fork.node() => {
                        Some(node.id().clone())
                    }
                    NodeKind::Branch { .. }
                    | NodeKind::Fork { .. }
                    | NodeKind::Join { .. }
                    | NodeKind::Repeat { .. }
                    | NodeKind::Wait { .. }
                    | NodeKind::SignalWait { .. }
                    | NodeKind::Subworkflow { .. }
                    | NodeKind::Terminal { .. }
                    | NodeKind::Task { .. }
                    | NodeKind::Reducer { .. } => None,
                });
            let mut outputs = BTreeSet::new();
            if let Some(ref owning_join) = owning_join {
                let branch_start = branch_revision
                    .semantic()
                    .edges()
                    .values()
                    .find(|edge| {
                        edge.kind() == EdgeKind::Control
                            && edge.source_node() == fork.node()
                            && edge.source_port() == branch.port()
                    })
                    .map(|edge| edge.target_node().clone())
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "fork branch has no declared control-flow start".to_owned(),
                        )
                    })?;
                let branch_nodes = control_nodes_before_join(
                    &branch_revision,
                    &branch_start,
                    owning_join,
                );
                let mut routes: Vec<_> = branch_revision
                    .semantic()
                    .edges()
                    .values()
                    .filter(|edge| {
                        edge.kind() == EdgeKind::Data
                            && branch_nodes.contains(edge.source_node())
                            && !branch_nodes.contains(edge.target_node())
                    })
                    .map(|edge| (edge.source_node().clone(), edge.source_port().clone()))
                    .collect();
                routes.sort();
                routes.dedup();
                for (source_node, source_port) in routes {
                    let selected = projection
                        .latest_descendant_execution(
                            branch.scope().reference(),
                            &source_node,
                        )
                        .filter(|execution| {
                            execution.state()
                                == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                        })
                        .and_then(|execution| {
                            execution
                                .outputs()
                                .iter()
                                .filter(|output| {
                                    output.value().key().as_str() == source_port.as_str()
                                })
                                .max_by_key(|output| output.sequence())
                                .map(|output| output.value().clone())
                        });
                    if let Some(selected) = selected {
                        outputs.insert(selected);
                    }
                }
            }
            let join = owning_join.and_then(|join| {
                revision
                    .semantic()
                    .nodes()
                    .get(&join)
                    .filter(|node| {
                        matches!(node.kind(), NodeKind::Join { config } if config.fork() == fork.node())
                    })
                    .map(|node| (node.clone(), fork.scope().clone()))
            });
            terminal.push((branch.branch().clone(), outcome, outputs, join));
        }
        for (branch, outcome, outputs, join) in terminal {
            let join_needs_materialization = join.as_ref().is_some_and(|(node, scope)| {
                !node_occurrence_exists_for_current_pin(projection, node.id(), scope)
            });
            let join_needs_membership = join_needs_materialization
                && join.as_ref().is_some_and(|(_, scope)| {
                    projection
                        .scopes()
                        .get(scope)
                        .is_some_and(|scope| matches!(scope.kind(), ScopeKind::Branch { .. }))
                });
            let required_events = 1_usize
                .saturating_add(usize::from(join_needs_materialization))
                .saturating_add(usize::from(join_needs_membership));
            if events.len().saturating_add(required_events) > STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::BranchTerminal {
                    branch,
                    outcome,
                    outputs: outputs.into_iter().collect(),
                },
            )?;
            if join_needs_materialization {
                let (join, scope) = join.ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "branch join materialization candidate disappeared".to_owned(),
                    )
                })?;
                if !scope_has_inactive_branch(projection, &scope)
                    && predecessors_ready(revision, projection, &join, &scope)
                {
                    let execution = self.next_execution_id()?;
                    let owning_branch = projection.scopes().get(&scope).and_then(|scope| {
                        if let ScopeKind::Branch { branch } = scope.kind() {
                            Some(branch.clone())
                        } else {
                            None
                        }
                    });
                    self.push_projected_event(
                        run,
                        occurred_at,
                        projection,
                        events,
                        RunEventKind::NodeBecameEligible {
                            node: join.id().clone(),
                            execution: execution.clone(),
                            scope,
                            mode: node_execution_mode(&join),
                        },
                    )?;
                    if let Some(branch) = owning_branch {
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::BranchChildAdded { branch, execution },
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    #[allow(deprecated)]
    #[allow(clippy::too_many_arguments)]
    fn try_satisfy_join(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        _revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        node: &Node,
        execution: &NodeExecutionId,
        config: &milkdrift_blueprint::JoinConfig,
    ) -> Result<(), RuntimeError> {
        let join_scope = projection
            .node_executions()
            .get(execution)
            .ok_or_else(|| RuntimeError::InvalidHistory("join execution is absent".to_owned()))?
            .scope()
            .clone();
        let fork_execution = projection
            .executions_for_node(config.fork())
            .filter(|value| {
                value.scope() == &join_scope
                    && source_execution_is_valid_for_occurrence(
                        projection,
                        value,
                        node.id(),
                        &join_scope,
                    )
            })
            .last()
            .map(|value| value.execution().clone());
        let Some(fork_execution) = fork_execution else {
            return Ok(());
        };
        let mut completed = Vec::new();
        let mut active = Vec::new();
        for branch in projection.branches_for_fork(&fork_execution) {
            match branch.state() {
                BranchState::Completed(outcome) => completed.push(BranchResultReference {
                    branch: branch.branch().clone(),
                    scope: branch.scope().reference().clone(),
                    outcome,
                    outputs: branch.outputs().to_vec(),
                }),
                BranchState::Active | BranchState::Cancelling => {
                    active.push(branch.branch().clone());
                }
                BranchState::Retained => {}
            }
        }
        completed.sort_by(|left, right| left.branch.cmp(&right.branch));
        active.sort();
        let (rule, selected, retained) = match config.policy() {
            JoinPolicy::All if active.is_empty() && !completed.is_empty() => {
                (JoinRule::All, completed, Vec::new())
            }
            JoinPolicy::Any if !completed.is_empty() => {
                for branch in &active {
                    if projection
                        .branches()
                        .get(branch)
                        .is_some_and(|branch| branch.state() == BranchState::Active)
                    {
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::BranchCancellationRequested {
                                branch: branch.clone(),
                                reason: Reason::new(
                                    "any-completion join cancelled an unfinished losing branch",
                                )?,
                            },
                        )?;
                    }
                }
                (JoinRule::AnyCompletion, completed, Vec::new())
            }
            JoinPolicy::FirstSuccess | JoinPolicy::AnySuccessful => {
                let has_success = completed
                    .iter()
                    .any(|result| result.outcome == RunOutcome::Succeeded);
                if !has_success && active.is_empty() {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "all fork branches terminated without a successful result",
                        )?),
                    );
                }
                if !has_success {
                    return Ok(());
                }
                for branch in &active {
                    let state = projection
                        .branches()
                        .get(branch)
                        .map(|branch| branch.state());
                    if state == Some(BranchState::Active) {
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::BranchCancellationRequested {
                                branch: branch.clone(),
                                reason: Reason::new(
                                    "first-success join cancelled an unfinished losing branch",
                                )?,
                            },
                        )?;
                    }
                }
                (JoinRule::FirstSuccess, completed, Vec::new())
            }
            JoinPolicy::Quorum(required) => {
                let required_usize = usize::from(required);
                let successes = completed
                    .iter()
                    .filter(|result| result.outcome == RunOutcome::Succeeded)
                    .count();
                if successes < required_usize && active.is_empty() {
                    return self.complete_deterministic_with_outcome(
                        run,
                        occurred_at,
                        projection,
                        events,
                        node,
                        execution,
                        NodeOutcome::Failed,
                        Some(BoundedDetail::new(
                            "all fork branches terminated before the required quorum was reached",
                        )?),
                    );
                }
                if successes < required_usize {
                    return Ok(());
                }
                for branch in &active {
                    let state = projection
                        .branches()
                        .get(branch)
                        .map(|branch| branch.state());
                    if state == Some(BranchState::Active) {
                        self.push_projected_event(
                            run,
                            occurred_at,
                            projection,
                            events,
                            RunEventKind::BranchCancellationRequested {
                                branch: branch.clone(),
                                reason: Reason::new(
                                    "quorum join cancelled an unfinished losing branch",
                                )?,
                            },
                        )?;
                    }
                }
                (
                    JoinRule::Quorum {
                        required: u32::from(required),
                    },
                    completed,
                    Vec::new(),
                )
            }
            JoinPolicy::All | JoinPolicy::Any => return Ok(()),
        };
        self.push_projected_event(
            run,
            occurred_at,
            projection,
            events,
            RunEventKind::JoinSatisfied {
                execution: execution.clone(),
                rule,
                branches: selected,
                retained_branches: retained,
            },
        )?;
        self.complete_deterministic(run, occurred_at, projection, events, node, execution)
    }

    fn add_ready_successors(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        scan_remaining: &mut usize,
    ) -> Result<(), RuntimeError> {
        if run_drain_reason(projection).is_some() {
            return Ok(());
        }
        let mut candidates = BTreeSet::new();
        let requested = (*scan_remaining).min(
            projection.pending_successor_execution_ids().len(),
        );
        let claimed = self.claim_structured_scan_visits(requested);
        let pending_sources: Vec<_> = projection
            .pending_successor_execution_ids()
            .iter()
            .take(claimed)
            .cloned()
            .collect();
        *scan_remaining = scan_remaining.saturating_sub(pending_sources.len());
        let mut processed_sources = Vec::with_capacity(pending_sources.len());
        for source_execution in pending_sources {
            let Some(execution) = projection.node_executions().get(&source_execution) else {
                return Err(RuntimeError::InvalidHistory(
                    "scanned successor execution identity is absent".to_owned(),
                ));
            };
            if let Some(branch) = projection
                .branch_for_execution(execution.execution())
                .filter(|branch| matches!(branch.state(), BranchState::Completed(_)))
            {
                let fork = projection
                    .node_executions()
                    .get(branch.fork_execution())
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "completed branch owner fork is absent".to_owned(),
                        )
                    })?;
                if let Some(join) = revision.semantic().nodes().values().find(|target| {
                    matches!(target.kind(), NodeKind::Join { config } if config.fork() == fork.node())
                }) {
                    candidates.insert((join.id().clone(), fork.scope().clone()));
                }
            }
            if execution.state() != &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                || !execution_is_in_current_node_epoch(projection, execution)
                || execution_branch_state(projection, execution.execution())
                    .is_some_and(|state| state != BranchState::Active)
            {
                processed_sources.push(source_execution);
                continue;
            }
            let source_node = execution.node().clone();
            let source_scope = execution.scope().clone();
            let Some(source) = revision.semantic().nodes().get(&source_node) else {
                // Reconciliation may prospectively remove a node while preserving
                // its immutable completed execution. It has no successors in the
                // adopted graph and must remain inert rather than being reinterpreted.
                processed_sources.push(source_execution);
                continue;
            };
            if matches!(
                source.kind(),
                NodeKind::Fork { .. } | NodeKind::Terminal { .. }
            ) {
                processed_sources.push(source_execution);
                continue;
            }
            let selected_port = projection.branch_routes().get(&source_execution);
            for edge in revision
                .semantic()
                .edges()
                .values()
                .filter(|edge| edge.source_node() == &source_node)
            {
                let admits_target = match edge.kind() {
                    EdgeKind::Control => {
                        selected_port.is_none_or(|port| edge.source_port() == port)
                    }
                    EdgeKind::Data => !revision.semantic().edges().values().any(|candidate| {
                        candidate.kind() == EdgeKind::Control
                            && candidate.target_node() == edge.target_node()
                    }),
                };
                if admits_target {
                    candidates.insert((edge.target_node().clone(), source_scope.clone()));
                }
            }
            processed_sources.push(source_execution);
        }

        for (target, scope) in candidates {
            if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            let target_node = revision.semantic().nodes().get(&target).ok_or_else(|| {
                RuntimeError::InvalidHistory("control edge target is absent".to_owned())
            })?;
            if scope_has_inactive_branch(projection, &scope) {
                continue;
            }
            if node_occurrence_exists_for_current_pin(projection, &target, &scope) {
                continue;
            }
            if !predecessors_ready(revision, projection, target_node, &scope) {
                continue;
            }
            let execution = self.next_execution_id()?;
            let owning_branch = projection.scopes().get(&scope).and_then(|scope| {
                if let ScopeKind::Branch { branch } = scope.kind() {
                    Some(branch.clone())
                } else {
                    None
                }
            });
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::NodeBecameEligible {
                    node: target,
                    execution: execution.clone(),
                    scope,
                    mode: node_execution_mode(target_node),
                },
            )?;
            if let Some(branch) = owning_branch {
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::BranchChildAdded { branch, execution },
                )?;
            }
        }
        for execution in processed_sources {
            if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::StructuredSuccessorScanCompleted { execution },
            )?;
        }
        Ok(())
    }

    fn evaluation_context(
        &self,
        node: &Node,
        projection: &RunProjection,
        occurrence_scope: &ScopeReference,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<EvaluationContext, RuntimeError> {
        let mut context = EvaluationContext::default();
        for port in node.data_inputs().values() {
            let Some(source) = port.binding() else {
                continue;
            };
            if matches!(source, BindingSource::Literal { .. }) {
                continue;
            }
            let Some(resolved) = self.resolve_optional_binding(
                projection,
                node.id(),
                occurrence_scope,
                source,
                pending_workspace,
                false,
            )?
            else {
                continue;
            };
            let value = match resolved {
                ResolvedInputValue::Inline { value, .. } => value,
                ResolvedInputValue::Workspace(reference) => {
                    let entry =
                        self.projected_workspace_value(projection, &reference, pending_workspace)?;
                    workspace_value_as_bounded(entry.value())?
                }
                ResolvedInputValue::Artifact(reference) => {
                    artifact_reference_as_bounded(&reference)?
                }
            };
            context.insert(source, value)?;
        }
        Ok(context)
    }

    fn projected_output_entry(
        &self,
        projection: &RunProjection,
        scope: &ScopeReference,
        key: ValueKey,
        value: WorkspaceValue,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<WorkspaceValueEntry, RuntimeError> {
        match self.projected_latest_workspace_value(projection, scope, &key, pending_workspace)? {
            Some(previous) => WorkspaceValueEntry::successor(previous.reference().clone(), value)
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string())),
            None => Ok(WorkspaceValueEntry::initial(scope.clone(), key, value)),
        }
    }

    fn projected_imported_output_entry(
        &self,
        projection: &RunProjection,
        scope: &ScopeReference,
        key: ValueKey,
        source: WorkspaceValueReference,
        value: WorkspaceValue,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<WorkspaceValueEntry, RuntimeError> {
        match self.projected_latest_workspace_value(projection, scope, &key, pending_workspace)? {
            Some(previous) => WorkspaceValueEntry::successor(previous.reference().clone(), value)
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string())),
            None => WorkspaceValueEntry::imported(scope.clone(), key, source, value)
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string())),
        }
    }

    fn signal_payload_entries(
        &self,
        projection: &RunProjection,
        execution: &NodeExecutionId,
        payload: &BoundedJson,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Vec<WorkspaceValueEntry>, RuntimeError> {
        let execution_view = projection.node_executions().get(execution).ok_or_else(|| {
            RuntimeError::InvalidHistory("signal wait execution is absent".to_owned())
        })?;
        let revision = self.revision_for_execution(projection, execution)?;
        let node = revision
            .semantic()
            .nodes()
            .get(execution_view.node())
            .ok_or_else(|| RuntimeError::InvalidHistory("signal wait node is absent".to_owned()))?;
        let mut entries = Vec::with_capacity(node.data_outputs().len());
        for port in node.data_outputs().keys() {
            let key = ValueKey::new(port.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let entry = self.projected_output_entry(
                projection,
                execution_view.scope(),
                key,
                WorkspaceValue::Json(payload.clone()),
                pending_workspace,
            )?;
            entries.push(entry);
        }
        Ok(entries)
    }

    fn drain_broadcast_signals(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
        workspace: &mut Vec<WorkspaceMutation>,
    ) -> Result<(), RuntimeError> {
        if events.len().saturating_add(1) > STRUCTURED_EVENT_SOFT_LIMIT {
            return Ok(());
        }
        let Some((_, signal)) = projection.pending_broadcast_signals().iter().next().cloned()
        else {
            return Ok(());
        };
        let signal_view = projection.signals().get(&signal).ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "pending broadcast scan references an absent signal".to_owned(),
            )
        })?;
        let signal_type = signal_view.signal_type().clone();
        let correlation = signal_view.correlation().cloned();
        let received_sequence = signal_view.received_sequence();
        let payload = signal_view.payload().clone();
        let original_cursor = signal_view.broadcast_scan_through().cloned();
        let mut through = original_cursor.clone();
        let mut exhausted = false;
        let scan_limit = usize::from(self.config.maximum_tick_items);
        let mut scanned = 0_usize;

        while scanned < scan_limit {
            let lower = through
                .as_ref()
                .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
            let next_wait = projection
                .waits()
                .range((lower, std::ops::Bound::Unbounded))
                .next()
                .map(|(_, wait)| {
                    (
                        wait.execution().clone(),
                        wait.registered_sequence(),
                        wait.condition().clone(),
                        wait.is_pending(),
                    )
                });
            let Some((execution, registered_sequence, condition, pending)) = next_wait else {
                exhausted = true;
                break;
            };
            let consumed = projection
                .signals()
                .get(&signal)
                .is_some_and(|received| received.consumed_by().contains(&execution));
            let eligible = pending
                && registered_sequence < received_sequence
                && !consumed
                && wait_signal_matches(&condition, &signal_type, correlation.as_ref());
            if eligible {
                let entries =
                    self.signal_payload_entries(projection, &execution, &payload, workspace)?;
            let event_cost = entries.len().checked_add(2).ok_or_else(|| {
                RuntimeError::Scheduling("broadcast signal event cost overflow".to_owned())
            })?;
                if event_cost.saturating_add(1) > STRUCTURED_EVENT_SOFT_LIMIT
                || entries.len() > MAX_WORKSPACE_MUTATIONS_PER_COMMIT
            {
                return Err(RuntimeError::InvalidHistory(
                    "one broadcast signal consumer exceeds atomic runtime bounds".to_owned(),
                ));
            }
                if events.len().saturating_add(event_cost).saturating_add(1)
                    > STRUCTURED_EVENT_SOFT_LIMIT
                || workspace.len().saturating_add(entries.len())
                    > MAX_WORKSPACE_MUTATIONS_PER_COMMIT
            {
                    break;
            }
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::SignalConsumed {
                    signal: signal.clone(),
                    execution: execution.clone(),
                },
            )?;
            for entry in entries {
                let value = entry.reference().clone();
                workspace.push(WorkspaceMutation::PutValue { entry });
                self.push_projected_event(
                    run,
                    occurred_at,
                    projection,
                    events,
                    RunEventKind::DeterministicOutputPublished {
                        execution: execution.clone(),
                        value,
                        artifact: None,
                    },
                )?;
            }
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::WaitSatisfied {
                    execution: execution.clone(),
                    cause: WaitSatisfaction::Signal {
                        signal: signal.clone(),
                    },
                },
            )?;
            }
            through = Some(execution);
            scanned = scanned.saturating_add(1);
        }
        if !exhausted && scanned == scan_limit {
            let lower = through
                .as_ref()
                .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
            exhausted = projection
                .waits()
                .range((lower, std::ops::Bound::Unbounded))
                .next()
                .is_none();
        }
        if through != original_cursor || exhausted {
            self.push_projected_event(
                run,
                occurred_at,
                projection,
                events,
                RunEventKind::SignalBroadcastScanAdvanced {
                    signal,
                    through_execution: through,
                    complete: exhausted,
                },
            )?;
        }
        Ok(())
    }

    fn workspace_accounting_transition(
        &self,
        projection: &RunProjection,
        mutations: &[WorkspaceMutation],
        budget: &WorkspaceBudget,
        required_artifacts: &BTreeSet<ArtifactReference>,
    ) -> Result<(WorkspaceUsage, WorkspaceUsage, BTreeSet<ArtifactReference>), RuntimeError> {
        let run = projection.run_id().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "workspace accounting transition has no run identity".to_owned(),
            )
        })?;
        let expected = self.store.workspace_usage(run)?;
        let mut newly_referenced_artifacts = BTreeSet::new();
        for artifact in required_artifacts {
            if !self.store.is_referenced_by_run(run, artifact)? {
                newly_referenced_artifacts.insert(artifact.clone());
            }
        }
        let mut resulting = expected;
        for artifact in &newly_referenced_artifacts {
            resulting = budget
                .admit_artifact_reference(&resulting, artifact)
                .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
        }
        for mutation in mutations {
            if let WorkspaceMutation::PutValue { entry } = mutation {
                resulting = budget
                    .admit_value(&resulting, entry.value())
                    .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
            }
        }
        Ok((expected, resulting, newly_referenced_artifacts))
    }

    fn index_update(
        &self,
        run: &RunId,
        old: &RunProjection,
        new: &RunProjection,
        updated_at: TimestampMillis,
    ) -> Result<RunIndexUpdate, RuntimeError> {
        let through = new.sequence();
        let workflow = new.workflow().ok_or_else(|| {
            RuntimeError::InvalidHistory("indexed run has no workflow".to_owned())
        })?;
        let revision_id = new.revision().ok_or_else(|| {
            RuntimeError::InvalidHistory("indexed run has no revision pin".to_owned())
        })?;
        let runnable = self.runnable_executions(new)?;
        let state = if new.lifecycle().is_completed() {
            IndexedRunState::Terminal
        } else if new.lifecycle() == RunLifecycle::Cancelling {
            IndexedRunState::Cancelling
        } else if new.lifecycle() == RunLifecycle::Paused {
            IndexedRunState::Paused
        } else if new.unresolved_attempts().next().is_some() {
            IndexedRunState::Uncertain
        } else if !runnable.is_empty() {
            IndexedRunState::Runnable
        } else if new.lifecycle() == RunLifecycle::Created {
            IndexedRunState::Created
        } else if new.timers().values().any(|timer| timer.is_pending())
            || new.waits().values().any(|wait| wait.is_pending())
            || new.subworkflows().values().any(|child| child.is_active())
            || new.reconciliation().is_active()
        {
            IndexedRunState::Waiting
        } else {
            IndexedRunState::Active
        };
        let mut update = RunIndexUpdate {
            summary: Some(RunSummaryIndex {
                run: run.clone(),
                workflow: workflow.clone(),
                revision: revision_id.clone(),
                state,
                through_sequence: through,
                updated_at,
            }),
            ..RunIndexUpdate::default()
        };

        let old_runnable = self.runnable_executions(old)?;
        for (execution, eligible_at) in &runnable {
            if old_runnable.get(execution) == Some(eligible_at) {
                continue;
            }
            update.runnable.push(RunnableIndexMutation::Upsert {
                entry: RunnableIndexEntry {
                    run: run.clone(),
                    execution: execution.clone(),
                    eligible_at: *eligible_at,
                    priority: 0,
                    through_sequence: through,
                },
            });
        }
        for execution in old_runnable
            .keys()
            .filter(|execution| !runnable.contains_key(*execution))
        {
            update.runnable.push(RunnableIndexMutation::Remove {
                run: run.clone(),
                execution: execution.clone(),
            });
        }

        let old_timers: BTreeMap<_, _> = old
            .timers()
            .values()
            .filter(|timer| timer.is_pending())
            .map(|timer| (timer.timer().clone(), timer.fire_at()))
            .collect();
        let new_timers: BTreeMap<_, _> = new
            .timers()
            .values()
            .filter(|timer| timer.is_pending())
            .map(|timer| (timer.timer().clone(), timer.fire_at()))
            .collect();
        for (timer, fire_at) in &new_timers {
            if old_timers.get(timer) == Some(fire_at) {
                continue;
            }
            update.timers.push(TimerIndexMutation::Upsert {
                entry: TimerIndexEntry {
                    run: run.clone(),
                    timer: timer.clone(),
                    fire_at: *fire_at,
                    through_sequence: through,
                },
            });
        }
        for timer in old_timers
            .keys()
            .filter(|timer| !new_timers.contains_key(*timer))
        {
            update.timers.push(TimerIndexMutation::Remove {
                run: run.clone(),
                timer: timer.clone(),
            });
        }

        let old_leases: BTreeSet<_> = old
            .leases()
            .values()
            .filter(|lease| lease.is_active())
            .map(|lease| lease.lease().clone())
            .collect();
        let new_leases: BTreeSet<_> = new
            .leases()
            .values()
            .filter(|lease| lease.is_active())
            .map(|lease| lease.lease().clone())
            .collect();
        for lease in &new_leases {
            let candidate = new.leases().get(lease).ok_or_else(|| {
                RuntimeError::InvalidHistory("active lease projection disappeared".to_owned())
            })?;
            if old.leases().get(lease).is_some_and(|previous| {
                previous.is_active()
                    && previous.attempt() == candidate.attempt()
                    && previous.worker() == candidate.worker()
                    && previous.expires_at() == candidate.expires_at()
            }) {
                continue;
            }
            update.leases.push(LeaseIndexMutation::Upsert {
                entry: LeaseIndexEntry {
                    run: run.clone(),
                    lease: lease.clone(),
                    attempt: candidate.attempt().clone(),
                    worker: candidate.worker().clone(),
                    expires_at: candidate.expires_at(),
                    through_sequence: through,
                },
            });
        }
        for lease in old_leases.difference(&new_leases) {
            update.leases.push(LeaseIndexMutation::Remove {
                run: run.clone(),
                lease: lease.clone(),
            });
        }
        Ok(update)
    }

    fn current_revision(
        &self,
        projection: &RunProjection,
    ) -> Result<BlueprintRevision, RuntimeError> {
        let revision = projection
            .revision()
            .ok_or_else(|| RuntimeError::InvalidHistory("run has no pinned revision".to_owned()))?;
        self.load_validated_revision(revision, projection.workflow())
    }

    fn revision_for_execution(
        &self,
        projection: &RunProjection,
        execution: &NodeExecutionId,
    ) -> Result<BlueprintRevision, RuntimeError> {
        let execution_view = projection.node_executions().get(execution).ok_or_else(|| {
            RuntimeError::InvalidHistory("node execution is absent".to_owned())
        })?;
        let revision = projection
            .revision_at(execution_view.created_sequence())
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "node execution has no governing revision pin".to_owned(),
                )
            })?;
        self.load_validated_revision(revision, projection.workflow())
    }

    fn scan_eligible_execution_ids(
        &self,
        run: &RunId,
        projection: &RunProjection,
        remaining: &mut usize,
    ) -> Result<Vec<NodeExecutionId>, RuntimeError> {
        let claimed = self.claim_structured_scan_visits(
            (*remaining).min(projection.eligible_execution_ids().len()),
        );
        let mut allowance = claimed;
        let selected = bounded_projection_set(
            run,
            projection.eligible_execution_ids(),
            &self.structured_eligible_cursors,
            &mut allowance,
            "structured eligible scan cursor",
        )?;
        *remaining = remaining.saturating_sub(claimed.saturating_sub(allowance));
        Ok(selected)
    }

    fn scan_branch_ids(
        &self,
        run: &RunId,
        projection: &RunProjection,
        remaining: &mut usize,
    ) -> Result<Vec<BranchId>, RuntimeError> {
        let claimed = self.claim_structured_scan_visits(
            (*remaining).min(projection.active_branch_ids().len()),
        );
        let mut allowance = claimed;
        let selected = bounded_projection_set(
            run,
            projection.active_branch_ids(),
            &self.structured_branch_cursors,
            &mut allowance,
            "structured branch scan cursor",
        )?;
        *remaining = remaining.saturating_sub(claimed.saturating_sub(allowance));
        Ok(selected)
    }

    fn claim_structured_scan_visits(&self, requested: usize) -> usize {
        if requested == 0
            || !self
                .structured_scan_budget_active
                .load(Ordering::Acquire)
        {
            return requested;
        }
        let mut claimed = 0;
        let _ = self.structured_scan_budget.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |remaining| {
                claimed = requested.min(remaining);
                Some(remaining.saturating_sub(claimed))
            },
        );
        claimed
    }

    fn runnable_executions(
        &self,
        projection: &RunProjection,
    ) -> Result<BTreeMap<NodeExecutionId, TimestampMillis>, RuntimeError> {
        if projection.lifecycle() != RunLifecycle::Running || projection.termination().is_some() {
            return Ok(BTreeMap::new());
        }
        let mut result = BTreeMap::new();
        let mut revisions = BTreeMap::new();
        for execution_id in projection.active_execution_ids() {
            let execution = projection
                .node_executions()
                .get(execution_id)
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "active execution frontier identity is absent".to_owned(),
                    )
                })?;
            let eligible_at = match execution.state() {
                NodeExecutionState::Eligible => TimestampMillis::new(0),
                NodeExecutionState::RetryPending(attempt)
                    if projection
                        .attempts()
                        .get(attempt)
                        .is_some_and(|value| value.state() == &AttemptState::ReadyToSchedule) =>
                {
                    projection
                        .retries()
                        .values()
                        .find(|retry| retry.next_attempt() == attempt)
                        .map_or(TimestampMillis::new(0), |retry| retry.fire_at())
                }
                NodeExecutionState::RetryPending(_)
                | NodeExecutionState::Scheduled(_)
                | NodeExecutionState::Running(_)
                | NodeExecutionState::Uncertain(_)
                | NodeExecutionState::CancelledBeforeDispatch
                | NodeExecutionState::RemovedProspectively(_)
                | NodeExecutionState::Terminal(_) => continue,
            };
            if execution_branch_state(projection, execution.execution())
                .is_some_and(|state| state != BranchState::Active)
            {
                continue;
            }
            let revision_id = projection
                .revision_at(execution.created_sequence())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "node execution has no governing revision pin".to_owned(),
                    )
                })?
                .clone();
            if !revisions.contains_key(&revision_id) {
                revisions.insert(
                    revision_id.clone(),
                    self.load_validated_revision(&revision_id, projection.workflow())?,
                );
            }
            let is_task = revisions
                .get(&revision_id)
                .and_then(|revision| revision.semantic().nodes().get(execution.node()))
                .is_some_and(|node| {
                    matches!(node.kind(), NodeKind::Task { .. })
                        || matches!(
                            node.kind(),
                            NodeKind::Reducer { config }
                                if matches!(config.strategy(), ReducerStrategy::Capability(_))
                        )
                });
            if !is_task {
                continue;
            }
            result.insert(execution.execution().clone(), eligible_at);
        }
        Ok(result)
    }

    fn load_validated_revision(
        &self,
        revision: &RevisionId,
        expected_workflow: Option<&WorkflowId>,
    ) -> Result<BlueprintRevision, RuntimeError> {
        let root = self.store.revision(revision)?.ok_or_else(|| {
            RuntimeError::InvalidTransition(format!("revision {revision} does not exist"))
        })?;
        if expected_workflow.is_some_and(|workflow| root.semantic().workflow() != workflow) {
            return Err(RuntimeError::InvalidTransition(
                "revision belongs to another workflow lineage".to_owned(),
            ));
        }
        let mut visiting = BTreeSet::new();
        let mut verified = BTreeSet::new();
        self.validate_pinned_children(&root, &mut visiting, &mut verified, 0)?;
        Ok(root)
    }

    fn validate_pinned_children(
        &self,
        revision: &BlueprintRevision,
        visiting: &mut BTreeSet<RevisionId>,
        verified: &mut BTreeSet<RevisionId>,
        depth: usize,
    ) -> Result<(), RuntimeError> {
        if depth > 64 {
            return Err(RuntimeError::InvalidTransition(
                "pinned subworkflow nesting exceeds 64 revisions".to_owned(),
            ));
        }
        if verified.contains(revision.id()) {
            return Ok(());
        }
        if !visiting.insert(revision.id().clone()) {
            return Err(RuntimeError::InvalidTransition(format!(
                "pinned subworkflow cycle reaches revision {}",
                revision.id()
            )));
        }
        for node in revision.semantic().nodes().values() {
            let reference = match node.kind() {
                NodeKind::Subworkflow { reference } => Some(reference),
                NodeKind::Repeat { config } => Some(config.body()),
                NodeKind::Task { .. }
                | NodeKind::Branch { .. }
                | NodeKind::Fork { .. }
                | NodeKind::Join { .. }
                | NodeKind::Reducer { .. }
                | NodeKind::Wait { .. }
                | NodeKind::SignalWait { .. }
                | NodeKind::Terminal { .. } => None,
            };
            if let Some(reference) = reference {
                let child = self.store.revision(reference.revision())?.ok_or_else(|| {
                    RuntimeError::InvalidTransition(format!(
                        "pinned child revision {} does not exist",
                        reference.revision()
                    ))
                })?;
                if child.semantic().workflow() != reference.workflow()
                    || child.semantic().interface() != reference.interface()
                {
                    return Err(RuntimeError::InvalidTransition(format!(
                        "pinned child revision {} has a different workflow or interface",
                        reference.revision()
                    )));
                }
                self.validate_pinned_children(&child, visiting, verified, depth + 1)?;
            }
        }
        visiting.remove(revision.id());
        verified.insert(revision.id().clone());
        Ok(())
    }

    fn next_command_id(&self) -> Result<CommandId, RuntimeError> {
        Ok(CommandId::new(self.ids.next("command")?)?)
    }

    fn next_event_id(&self) -> Result<EventId, RuntimeError> {
        Ok(EventId::new(self.ids.next("event")?)?)
    }

    fn next_execution_id(&self) -> Result<NodeExecutionId, RuntimeError> {
        Ok(NodeExecutionId::new(self.ids.next("execution")?)?)
    }

    fn next_attempt_id(&self) -> Result<AttemptId, RuntimeError> {
        Ok(AttemptId::new(self.ids.next("attempt")?)?)
    }

    fn next_lease_id(&self) -> Result<LeaseId, RuntimeError> {
        Ok(LeaseId::new(self.ids.next("lease")?)?)
    }

    fn next_timer_id(&self) -> Result<TimerId, RuntimeError> {
        Ok(TimerId::new(self.ids.next("timer")?)?)
    }

    fn next_plan_id(&self) -> Result<ReconciliationPlanId, RuntimeError> {
        Ok(ReconciliationPlanId::new(
            self.ids.next("reconciliation-plan")?,
        )?)
    }

    fn next_invocation_id(&self) -> Result<InvocationId, RuntimeError> {
        InvocationId::new(self.ids.next("invocation")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }

    fn next_scope_id(&self) -> Result<ScopeId, RuntimeError> {
        ScopeId::new(self.ids.next("scope")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }

    fn next_branch_id(&self) -> Result<BranchId, RuntimeError> {
        BranchId::new(self.ids.next("branch")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }

    fn next_iteration_id(&self) -> Result<IterationId, RuntimeError> {
        IterationId::new(self.ids.next("iteration")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }

    fn next_subworkflow_id(&self) -> Result<SubworkflowId, RuntimeError> {
        SubworkflowId::new(self.ids.next("subworkflow")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }

    fn next_run_id(&self) -> Result<RunId, RuntimeError> {
        RunId::new(self.ids.next("child-run")?)
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))
    }
}

impl RuntimeService {
    fn next_nonterminal_page(
        &self,
        cursor: &Mutex<Option<RunSummaryCursor>>,
        limit: PageSize,
        operation: &'static str,
    ) -> Result<Vec<RunSummaryIndex>, RuntimeError> {
        let mut cursor = cursor.lock().map_err(|_error| {
            RuntimeError::Scheduling(format!(
                "runtime {operation} pagination cursor lock is poisoned"
            ))
        })?;
        let page = self.store.nonterminal_run_page(cursor.as_ref(), limit)?;
        *cursor = page.next;
        Ok(page.runs)
    }

    fn next_runnable_page(
        &self,
        eligible_through: TimestampMillis,
        limit: PageSize,
    ) -> Result<Vec<RunnableIndexEntry>, RuntimeError> {
        let mut cursor = self.runnable_cursor.lock().map_err(|_error| {
            RuntimeError::Scheduling(
                "runtime runnable pagination cursor lock is poisoned".to_owned(),
            )
        })?;
        let cycle_boundary = cursor
            .as_ref()
            .map_or(eligible_through, RunnableCursor::eligible_through);
        let page = self
            .store
            .runnable_page(cycle_boundary, cursor.as_ref(), limit)?;
        *cursor = page.next;
        Ok(page.entries)
    }

    /// Performs one bounded fair scheduling pass.  This call never spawns, polls, or
    /// retains an in-memory queue; another call is required for later work.
    #[allow(clippy::too_many_lines)]
    pub fn scheduler_tick(&self) -> Result<SchedulerTickResult, RuntimeError> {
        let now = self.clock.now()?;
        let span = info_span!(
            "runtime.scheduler_tick",
            worker = %self.config.worker,
            observed_at = now.get(),
            accepting = self.is_accepting_admission(),
        );
        let _entered = span.enter();
        let mut result = SchedulerTickResult::default();
        let limit = PageSize::new(u32::from(self.config.maximum_tick_items))?;
        let maximum_visits = usize::from(self.config.maximum_tick_items);
        // Reserve one physical visit for runnable admission while allowing the
        // deterministic driver to use the remainder of the scheduler-wide
        // budget. A structured transition commonly consumes one eligible visit
        // and one successor-frontier visit, so an arbitrary half split would
        // halve the documented bounded progress without improving the bound.
        let structured_visit_limit = maximum_visits.saturating_sub(1);
        let runnable_visit_limit;
        {
            let _scheduler_guard = self.scheduler_gate.lock().map_err(|_error| {
                RuntimeError::Scheduling(
                    "runtime scheduler coordination lock is poisoned".to_owned(),
                )
            })?;
            let accepting_admission = self.is_accepting_admission();
            let maintenance_visit_limit = if accepting_admission {
                structured_visit_limit
            } else {
                // No runnable admission follows a shutdown pass, so every bounded
                // visit remains available for draining already-owned work.
                maximum_visits
            };
            self.structured_scan_budget
                .store(maintenance_visit_limit, Ordering::Release);
            self.structured_scan_budget_active
                .store(true, Ordering::Release);
            let structured_result = (|| -> Result<(), RuntimeError> {
                if !accepting_admission {
                    // Closing dispatch admission must not suppress an already durable
                    // cancellation boundary. This path may release waits and request
                    // executor cancellation, but it never creates a new dispatch lease.
                    self.propagate_cancellation(now, limit)?;
                    return Ok(());
                }
                let timer_allowance = self.structured_scan_budget.load(Ordering::Acquire);
                if timer_allowance > 0 {
                    let timer_limit = PageSize::new(u32::try_from(timer_allowance).map_err(
                        |_error| {
                            RuntimeError::Scheduling(
                                "timer visit limit conversion overflow".to_owned(),
                            )
                        },
                    )?)?;
                    let due_timers = self.store.due_timers(now, timer_limit)?;
                    let claimed = self.claim_structured_scan_visits(due_timers.len());
                    for timer in due_timers.into_iter().take(claimed) {
                    let expected = self.store.head(&timer.run)?;
                    let command = RunCommandDocument::new(
                        self.next_command_id()?,
                        timer.run,
                        self.config.internal_actor.clone(),
                        expected,
                        now,
                        Reason::new(
                            "scheduler observed a durable timer at or after its deadline",
                        )?,
                        Vec::new(),
                        RunCommand::FireTimer { timer: timer.timer },
                    )?;
                    let _ = self.handle_command(&command)?;
                    }
                }
                self.propagate_cancellation(now, limit)?;
                self.drive_reconciliation_restarts(now, limit)?;
                self.drive_child_aggregates(now, limit)?;
                self.drive_structured_runs(now, limit)
            })();
            self.structured_scan_budget_active
                .store(false, Ordering::Release);
            structured_result?;
            if !accepting_admission {
                result.deferred = 1;
                return Ok(result);
            }
            let unused = self.structured_scan_budget.load(Ordering::Acquire);
            let used = maintenance_visit_limit.saturating_sub(unused);
            runnable_visit_limit = maximum_visits.saturating_sub(used).max(1);
        }

        let runnable_limit = PageSize::new(u32::try_from(runnable_visit_limit).map_err(|_error| {
            RuntimeError::Scheduling("runnable visit limit conversion overflow".to_owned())
        })?)?;
        let entries = self.next_runnable_page(now, runnable_limit)?;
        let selected = select_fair_runnable(entries, runnable_visit_limit);
        for entry in selected {
            result.examined = result.examined.saturating_add(1);
            if !self.is_accepting_admission() {
                result.deferred = result.deferred.saturating_add(1);
                continue;
            }
            match self.dispatch_runnable(&entry, now) {
                Ok(DispatchOutcome::Completed) => {
                    result.dispatched = result.dispatched.saturating_add(1);
                    result.completed = result.completed.saturating_add(1);
                }
                Ok(DispatchOutcome::Uncertain) => {
                    result.dispatched = result.dispatched.saturating_add(1);
                    result.uncertain = result.uncertain.saturating_add(1);
                }
                Ok(DispatchOutcome::Deferred) => {
                    result.deferred = result.deferred.saturating_add(1);
                }
                Ok(DispatchOutcome::PreDispatchFailed) => {
                    result.completed = result.completed.saturating_add(1);
                }
                Err(error) => {
                    warn!(
                        run = %entry.run,
                        execution = %entry.execution,
                        reason = %error,
                        "runnable dispatch failed"
                    );
                    return Err(error);
                }
            }
        }
        Ok(result)
    }

    /// Short alias useful to simple synchronous hosts.
    pub fn tick(&self) -> Result<SchedulerTickResult, RuntimeError> {
        self.scheduler_tick()
    }

    /// Replays and classifies a bounded page of nonterminal runs. Expired dispatches
    /// become truthful uncertainty obligations; only work whose frozen side-effect
    /// and idempotency facts permit exact replay receives a bounded retry timer.
    #[allow(clippy::too_many_lines)]
    pub fn recover(&self) -> Result<RecoveryResult, RuntimeError> {
        let now = self.clock.now()?;
        let span = info_span!(
            "runtime.recovery",
            controller = %self.config.worker,
            observed_at = now.get(),
        );
        let _entered = span.enter();
        let _scheduler_guard = match self.scheduler_gate.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                return Err(RuntimeError::Scheduling(
                    "runtime scheduler or recovery pass is already active".to_owned(),
                ));
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(RuntimeError::Scheduling(
                    "runtime scheduler coordination lock is poisoned".to_owned(),
                ));
            }
        };
        let limit = PageSize::new(u32::from(self.config.maximum_tick_items))?;
        let mut result = RecoveryResult::default();
        let mut remaining = usize::from(self.config.maximum_tick_items);
        for summary in self.next_nonterminal_page(&self.recovery_cursor, limit, "recovery")? {
            if remaining == 0 {
                break;
            }
            let projection = self.projection(&summary.run)?;
            result.runs_examined = result.runs_examined.saturating_add(1);
            let scanned = bounded_projection_set(
                &summary.run,
                projection.active_attempt_ids(),
                &self.recovery_attempt_cursors,
                &mut remaining,
                "recovery attempt scan cursor",
            )?;
            let actionable: Vec<_> = scanned
                .iter()
                .filter_map(|attempt| projection.attempts().get(attempt))
                .filter(|attempt| {
                    if attempt.is_active() {
                        return projection
                            .active_lease_for_attempt(attempt.attempt())
                            .is_none_or(|lease| lease.expires_at() <= now);
                    }
                    if !attempt.is_unresolved()
                        || recovery_classification(attempt) != RecoveryClassification::Retryable
                    {
                        return false;
                    }
                    let Some(side_effect) = attempt.side_effect() else {
                        return false;
                    };
                    self.config.retry_policy.permits_automatic_retry(
                        attempt.attempt_number(),
                        unresolved_retry_error_class(attempt),
                        true,
                        side_effect.side_effect(),
                        side_effect.idempotency(),
                        side_effect.idempotency_key(),
                    )
                })
                .collect();
            if actionable.is_empty() {
                continue;
            }
            let mut plan = CommandPlan::one(RunEventKind::RecoveryStarted {
                controller: self.config.worker.clone(),
                through_sequence: projection.sequence(),
            });
            for attempt in actionable {
                if plan.events.len()
                    > milkdrift_persistence::MAX_EVENTS_PER_COMMIT.saturating_sub(4)
                {
                    break;
                }
                let active_lease = projection.active_lease_for_attempt(attempt.attempt());
                let classification = if attempt.is_completed() {
                    RecoveryClassification::TerminalObserved
                } else if let Some(lease) = active_lease {
                    if lease.expires_at() > now {
                        RecoveryClassification::LeaseStillValid
                    } else {
                        recovery_classification(attempt)
                    }
                } else if attempt.is_unresolved() {
                    recovery_classification(attempt)
                } else {
                    RecoveryClassification::NotStarted
                };
                if let Some(lease) = active_lease {
                    if lease.expires_at() <= now {
                        plan.events.push(RunEventKind::LeaseExpired {
                            lease: lease.lease().clone(),
                            classification,
                        });
                        result.expired_leases = result.expired_leases.saturating_add(1);
                    }
                }
                plan.events.push(RunEventKind::RecoveryClassified {
                    attempt: attempt.attempt().clone(),
                    lease: active_lease.map(|lease| lease.lease().clone()),
                    classification,
                    reason: Reason::new(recovery_reason(classification))?,
                });
                match classification {
                    RecoveryClassification::Retryable => {
                        if !attempt.is_unresolved() {
                            plan.events.push(RunEventKind::ExternalOutcomeUncertain {
                                attempt: attempt.attempt().clone(),
                                report_sequence: self
                                    .next_report_sequence(&projection, attempt.attempt())?,
                                side_effect: attempt
                                    .side_effect()
                                    .map_or(SideEffectClass::Unknown, |classification| {
                                        classification.side_effect()
                                    }),
                                reason: Reason::new(
                                    "lease expired before an external outcome was observed",
                                )?,
                                evidence: Vec::new(),
                            });
                            result.uncertain = result.uncertain.saturating_add(1);
                        }
                        let side_effect = attempt.side_effect();
                        let retry_error = if active_lease.is_some() {
                            ErrorClass::Transport
                        } else {
                            unresolved_retry_error_class(attempt)
                        };
                        let permit = side_effect.is_some_and(|classification| {
                            self.config.retry_policy.permits_automatic_retry(
                                attempt.attempt_number(),
                                retry_error,
                                true,
                                classification.side_effect(),
                                classification.idempotency(),
                                classification.idempotency_key(),
                            )
                        });
                        if permit {
                            match self.build_retry_event(
                                attempt.execution(),
                                attempt.attempt(),
                                attempt.attempt_number(),
                                now,
                                retry_error,
                                None,
                                "recovery admitted a safe bounded retry after lease expiry",
                            ) {
                                Ok(retry) => {
                                    plan.events.push(retry);
                                    result.retryable = result.retryable.saturating_add(1);
                                }
                                Err(error) => warn!(
                                    attempt = %attempt.attempt(),
                                    reason = %error,
                                    "recovery uncertainty retained without an unavailable retry timer"
                                ),
                            }
                        }
                    }
                    RecoveryClassification::Uncertain if !attempt.is_unresolved() => {
                        let side_effect = attempt
                            .side_effect()
                            .map_or(SideEffectClass::Unknown, |value| value.side_effect());
                        plan.events.push(RunEventKind::ExternalOutcomeUncertain {
                            attempt: attempt.attempt().clone(),
                            report_sequence: self
                                .next_report_sequence(&projection, attempt.attempt())?,
                            side_effect,
                            reason: Reason::new(
                                "lease expired and external side effects cannot be established",
                            )?,
                            evidence: Vec::new(),
                        });
                        result.uncertain = result.uncertain.saturating_add(1);
                    }
                    RecoveryClassification::NotStarted
                    | RecoveryClassification::LeaseStillValid
                    | RecoveryClassification::TerminalObserved
                    | RecoveryClassification::Uncertain => {}
                }
            }
            let marker = projection.attempts().keys().next().cloned();
            let _ = self.commit_internal_plan(
                &summary.run,
                now,
                "recover_nonterminal_run",
                marker.as_ref(),
                plan,
            )?;
        }
        Ok(result)
    }

    /// Alias for hosts that name restart orchestration explicitly.
    pub fn recover_nonterminal_runs(&self) -> Result<RecoveryResult, RuntimeError> {
        self.recover()
    }

    fn drive_structured_runs(
        &self,
        now: TimestampMillis,
        limit: PageSize,
    ) -> Result<(), RuntimeError> {
        for summary in
            self.next_nonterminal_page(&self.structured_cursor, limit, "structured-progress")?
        {
            if self.structured_scan_budget.load(Ordering::Acquire) == 0 {
                break;
            }
            let projection = self.projection(&summary.run)?;
            if !projection.lifecycle().is_active() {
                continue;
            }
            let revision = self.current_revision(&projection)?;
            let mut candidate = projection.clone();
            let mut events = Vec::new();
            let mut workspace = Vec::new();
            self.extend_structured_progress(
                &summary.run,
                now,
                &revision,
                &mut candidate,
                &mut events,
                &mut workspace,
            )?;
            if !events.is_empty() {
                let plan = CommandPlan {
                    events: events.iter().map(|event| event.kind().clone()).collect(),
                    workspace,
                    ..CommandPlan::default()
                };
                let _ = self.commit_internal_plan(
                    &summary.run,
                    now,
                    "drive_bounded_structured_progress",
                    None,
                    plan,
                )?;
            }
        }
        Ok(())
    }

    fn drive_reconciliation_restarts(
        &self,
        now: TimestampMillis,
        limit: PageSize,
    ) -> Result<(), RuntimeError> {
        for summary in self.next_nonterminal_page(
            &self.reconciliation_cursor,
            limit,
            "reconciliation-restart",
        )? {
            if self.structured_scan_budget.load(Ordering::Acquire) == 0 {
                break;
            }
            let projection = self.projection(&summary.run)?;
            let claimed = self.claim_structured_scan_visits(
                projection.pending_reconciliation_restarts().len(),
            );
            let mut allowance = claimed;
            let restart_keys = bounded_projection_map_keys(
                &summary.run,
                projection.pending_reconciliation_restarts(),
                &self.reconciliation_restart_cursors,
                &mut allowance,
                "reconciliation restart scan cursor",
            )?;
            for restart_key in restart_keys {
                let source_execution = projection
                    .pending_reconciliation_restarts()
                    .get(&restart_key)
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "reconciliation restart token disappeared".to_owned(),
                        )
                    })?;
                let source = projection
                    .node_executions()
                    .get(source_execution)
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "reconciliation cancellation source is absent".to_owned(),
                        )
                    })?;
                if source.state() != &NodeExecutionState::Terminal(NodeOutcome::Cancelled) {
                    continue;
                }
                let revision = self.current_revision(&projection)?;
                if !revision.semantic().nodes().contains_key(source.node()) {
                    return Err(RuntimeError::Reconciliation(
                        "cancel-and-restart target was removed from the adopted revision"
                            .to_owned(),
                    ));
                }
                let execution = self.next_execution_id()?;
                let mut plan = CommandPlan::one(RunEventKind::NodeBecameEligible {
                    node: source.node().clone(),
                    execution: execution.clone(),
                    scope: source.scope().clone(),
                    mode: node_execution_mode(
                        revision
                            .semantic()
                            .nodes()
                            .get(source.node())
                            .ok_or_else(|| {
                                RuntimeError::InvalidHistory(
                                    "reconciliation restart node is absent".to_owned(),
                                )
                            })?,
                    ),
                });
                if let Some(branch) = projection.branch_for_execution(source.execution()) {
                    if branch.state() == BranchState::Active {
                        plan.events.push(RunEventKind::BranchChildAdded {
                            branch: branch.branch().clone(),
                            execution,
                        });
                    }
                }
                let _ = self.commit_internal_plan(
                    &summary.run,
                    now,
                    "restart_reconciled_execution_after_confirmed_cancellation",
                    None,
                    plan,
                )?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn drive_child_aggregates(
        &self,
        now: TimestampMillis,
        limit: PageSize,
    ) -> Result<(), RuntimeError> {
        for summary in self.next_nonterminal_page(&self.child_cursor, limit, "child-aggregate")? {
            if self.structured_scan_budget.load(Ordering::Acquire) == 0 {
                break;
            }
            let parent = self.projection(&summary.run)?;
            let claimed = self.claim_structured_scan_visits(
                parent.active_subworkflow_ids().len(),
            );
            let mut allowance = claimed;
            let child_ids = bounded_projection_set(
                &summary.run,
                parent.active_subworkflow_ids(),
                &self.child_subworkflow_cursors,
                &mut allowance,
                "active child scan cursor",
            )?;
            let children: Vec<_> = child_ids
                .iter()
                .map(|subworkflow| {
                    let child = parent.subworkflows().get(subworkflow).ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "active child frontier identity is absent".to_owned(),
                        )
                    })?;
                    Ok((
                        child.subworkflow().clone(),
                        child.parent_execution().clone(),
                        child.child_run().clone(),
                        child.child_revision().clone(),
                        child.scope().clone(),
                        child.inputs().to_vec(),
                        child.state(),
                    ))
                })
                .collect::<Result<_, RuntimeError>>()?;
            for (
                subworkflow,
                parent_execution,
                child_run,
                child_revision,
                child_scope,
                input_references,
                parent_child_state,
            ) in children
            {
                let mut child_head = self.store.head(&child_run)?;
                if child_head == RunSequence::ZERO {
                    let child_blueprint = self.load_validated_revision(&child_revision, None)?;
                    let root_scope =
                        WorkspaceScope::run_root(child_run.clone(), self.next_scope_id()?);
                    let mut inputs_by_key = BTreeMap::new();
                    for reference in &input_references {
                        let entry = self.projected_workspace_value(&parent, reference, &[])?;
                        if inputs_by_key
                            .insert(entry.reference().key().clone(), entry.value().clone())
                            .is_some()
                        {
                            return Err(RuntimeError::InvalidTransition(
                                "subworkflow inputs must map to distinct child keys".to_owned(),
                            ));
                        }
                    }
                    let inputs = inputs_by_key
                        .into_iter()
                        .map(|(key, value)| {
                            WorkspaceValueEntry::initial(root_scope.reference().clone(), key, value)
                        })
                        .collect();
                    let budget = parent.workspace_budget().ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "parent run has no workspace budget".to_owned(),
                        )
                    })?;
                    let create = RunCommandDocument::new(
                        self.next_command_id()?,
                        child_run.clone(),
                        self.config.internal_actor.clone(),
                        RunSequence::ZERO,
                        now,
                        Reason::new("parent materialized a pinned child run aggregate")?,
                        Vec::new(),
                        RunCommand::CreateRun {
                            workflow: child_blueprint.semantic().workflow().clone(),
                            revision: child_revision.clone(),
                            root_scope,
                            workspace_budget: budget.clone(),
                            inputs,
                        },
                    )?;
                    let created = self.handle_command(&create)?;
                    if created.result().disposition() != CommandDisposition::Accepted {
                        return Err(RuntimeError::InvalidTransition(
                            "pinned child run creation was durably rejected".to_owned(),
                        ));
                    }
                    child_head = created.result().resulting_sequence();
                }

                let mut child = self.projection(&child_run)?;
                if parent_child_state == SubworkflowState::Cancelling
                    && !child.lifecycle().is_completed()
                    && child.lifecycle() != RunLifecycle::Cancelling
                {
                    let cancel = RunCommandDocument::new(
                        self.next_command_id()?,
                        child_run.clone(),
                        self.config.internal_actor.clone(),
                        child.sequence(),
                        now,
                        Reason::new("attached parent propagated structured cancellation")?,
                        Vec::new(),
                        RunCommand::RequestCancellation,
                    )?;
                    let _ = self.handle_command(&cancel)?;
                    child = self.projection(&child_run)?;
                } else if child.lifecycle() == RunLifecycle::Created {
                    let start = RunCommandDocument::new(
                        self.next_command_id()?,
                        child_run.clone(),
                        self.config.internal_actor.clone(),
                        child_head,
                        now,
                        Reason::new("parent started its pinned child run")?,
                        Vec::new(),
                        RunCommand::StartRun,
                    )?;
                    let _ = self.handle_command(&start)?;
                    child = self.projection(&child_run)?;
                }

                let Some(terminal) = child.terminal() else {
                    continue;
                };
                let parent = self.projection(&summary.run)?;
                let child_view = parent.subworkflows().get(&subworkflow).ok_or_else(|| {
                    RuntimeError::InvalidHistory("parent lost its durable child link".to_owned())
                })?;
                if child_view.is_completed() {
                    continue;
                }
                let parent_execution_view = parent
                    .node_executions()
                    .get(&parent_execution)
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "subworkflow parent execution is absent".to_owned(),
                        )
                    })?;
                let parent_revision = self.revision_for_execution(&parent, &parent_execution)?;
                let parent_node = parent_revision
                    .semantic()
                    .nodes()
                    .get(parent_execution_view.node())
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory("subworkflow parent node is absent".to_owned())
                    })?;
                let publish_on_parent = matches!(parent_node.kind(), NodeKind::Subworkflow { .. });
                let import_scope = child_scope.reference().clone();
                let mut plan = CommandPlan::one(RunEventKind::SubworkflowTerminal {
                    subworkflow: subworkflow.clone(),
                    child_run: child_run.clone(),
                    outcome: terminal.outcome(),
                    outputs: terminal.outputs().to_vec(),
                });
                for child_value in terminal.outputs() {
                    let source = self.projected_workspace_value(&child, child_value, &[])?;
                    let imported = self.projected_imported_output_entry(
                        &parent,
                        &import_scope,
                        source.reference().key().clone(),
                        child_value.clone(),
                        source.value().clone(),
                        &plan.workspace,
                    )?;
                    let parent_value = imported.reference().clone();
                    plan.workspace
                        .push(WorkspaceMutation::PutValue { entry: imported });
                    plan.events.push(RunEventKind::SubworkflowOutputImported {
                        subworkflow: subworkflow.clone(),
                        child_value: child_value.clone(),
                        parent_value: parent_value.clone(),
                    });
                    if publish_on_parent {
                        let published = self.projected_output_entry(
                            &parent,
                            parent_execution_view.scope(),
                            source.reference().key().clone(),
                            source.value().clone(),
                            &plan.workspace,
                        )?;
                        let published_value = published.reference().clone();
                        plan.workspace
                            .push(WorkspaceMutation::PutValue { entry: published });
                        plan.events
                            .push(RunEventKind::DeterministicOutputPublished {
                                execution: parent_execution.clone(),
                                value: published_value,
                                artifact: None,
                            });
                    }
                }
                let _ = self.commit_internal_plan(
                    &summary.run,
                    now,
                    "observe_child_terminal_and_import_outputs",
                    None,
                    plan,
                )?;
            }
        }
        Ok(())
    }

    fn dispatch_runnable(
        &self,
        entry: &RunnableIndexEntry,
        now: TimestampMillis,
    ) -> Result<DispatchOutcome, RuntimeError> {
        // Serialize the exact durable admission snapshot and lease CAS, then release
        // before entering the potentially blocking executor boundary. This prevents
        // same-service oversubscription without suppressing concurrent cancellation.
        let scheduler_guard = self.scheduler_gate.lock().map_err(|_error| {
            RuntimeError::Scheduling("runtime scheduler coordination lock is poisoned".to_owned())
        })?;
        let projection = project_complete_history(self.store.as_ref(), &entry.run)?;
        if projection.sequence() < entry.through_sequence
            || projection.lifecycle() != RunLifecycle::Running
        {
            return Ok(DispatchOutcome::Deferred);
        }
        if self.runnable_executions(&projection)?.get(&entry.execution) != Some(&entry.eligible_at) {
            return Ok(DispatchOutcome::Deferred);
        }
        let execution = projection
            .node_executions()
            .get(&entry.execution)
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("runnable execution is absent".to_owned())
            })?;
        if execution_branch_state(&projection, execution.execution())
            .is_some_and(|state| state != BranchState::Active)
        {
            return Ok(DispatchOutcome::Deferred);
        }
        let revision = self.revision_for_execution(&projection, &entry.execution)?;
        let node = revision
            .semantic()
            .nodes()
            .get(execution.node())
            .ok_or_else(|| RuntimeError::InvalidHistory("runnable node is absent".to_owned()))?;
        let requirement = match node.kind() {
            NodeKind::Task { requirement } => requirement.clone(),
            NodeKind::Reducer { config } => match config.strategy() {
                ReducerStrategy::Capability(operation) => {
                    milkdrift_capability::CapabilityRequirement::new(operation.clone())
                }
                ReducerStrategy::Collect | ReducerStrategy::First => {
                    return Ok(DispatchOutcome::Deferred);
                }
            },
            NodeKind::Branch { .. }
            | NodeKind::Fork { .. }
            | NodeKind::Join { .. }
            | NodeKind::Repeat { .. }
            | NodeKind::Wait { .. }
            | NodeKind::SignalWait { .. }
            | NodeKind::Subworkflow { .. }
            | NodeKind::Terminal { .. } => return Ok(DispatchOutcome::Deferred),
        };
        let branch = projection
            .scopes()
            .get(execution.scope())
            .and_then(|scope| match scope.kind() {
                ScopeKind::Branch { branch } => Some(branch.clone()),
                ScopeKind::RunRoot
                | ScopeKind::Iteration { .. }
                | ScopeKind::Subworkflow { .. } => None,
            });
        let admission = AdmissionRequest {
            run: entry.run.clone(),
            branch: branch.clone(),
            operation: requirement.operation().clone(),
        };
        let (usage, lease_catalog_witness) = self.admission_usage()?;
        if !self.config.scheduler_limits.allows(&admission, &usage) {
            return Ok(DispatchOutcome::Deferred);
        }
        let resolution = self.executor.resolve(&requirement)?;
        let contract = resolution.snapshot().operation_contract();
        let attempt = match execution.state() {
            NodeExecutionState::Eligible => self.next_attempt_id()?,
            NodeExecutionState::RetryPending(attempt)
                if projection
                    .attempts()
                    .get(attempt)
                    .is_some_and(|value| value.state() == &AttemptState::ReadyToSchedule) =>
            {
                attempt.clone()
            }
            NodeExecutionState::Scheduled(_)
            | NodeExecutionState::Running(_)
            | NodeExecutionState::RetryPending(_)
            | NodeExecutionState::Uncertain(_)
            | NodeExecutionState::CancelledBeforeDispatch
            | NodeExecutionState::RemovedProspectively(_)
            | NodeExecutionState::Terminal(_) => return Ok(DispatchOutcome::Deferred),
        };
        let attempt_number = projection
            .attempts()
            .get(&attempt)
            .map_or(1, |attempt| attempt.attempt_number());
        let invocation = self.next_invocation_id()?;
        let idempotency_key = match contract.idempotency() {
            IdempotencyBehavior::Unsupported => None,
            IdempotencyBehavior::CapabilityScoped | IdempotencyBehavior::ProviderProfileScoped => {
                Some(stable_idempotency_key(&entry.run, execution.execution())?)
            }
        };
        if let NodeExecutionState::RetryPending(retry_attempt) = execution.state() {
            let retry = projection
                .retries()
                .values()
                .find(|retry| retry.next_attempt() == retry_attempt)
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "retry-pending execution has no retry decision".to_owned(),
                    )
                })?;
            let previous = projection
                .attempts()
                .get(retry.previous_attempt())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "retry decision has no previous attempt".to_owned(),
                    )
                })?;
            if previous.side_effect().is_some_and(|classification| {
                classification.side_effect() == SideEffectClass::IdempotentWrite
            }) {
                let same_resolution = previous
                    .capability()
                    .is_some_and(|capability| capability.snapshot() == resolution.snapshot());
                let same_key = previous.idempotency_key() == idempotency_key.as_ref();
                if !same_resolution || !same_key {
                    warn!(
                        run = %entry.run,
                        execution = %execution.execution(),
                        previous_attempt = %previous.attempt(),
                        "idempotent-write retry retained because capability resolution or key changed"
                    );
                    return Ok(DispatchOutcome::Deferred);
                }
            }
        }
        let request = match self.invocation_request(
            &revision,
            &projection,
            node,
            execution.scope(),
            invocation.clone(),
            resolution.snapshot().capability().clone(),
            resolution.snapshot().provider_profile().cloned(),
            idempotency_key.clone(),
        ) {
            Ok(request) => request,
            Err(RuntimeError::Scheduling(_)) => {
                self.commit_pre_dispatch_failure(
                    &entry.run,
                    now,
                    execution.execution(),
                    "immutable invocation input materialization failed",
                )?;
                return Ok(DispatchOutcome::PreDispatchFailed);
            }
            Err(error) => return Err(error),
        };
        let request_bytes = serde_json::to_vec(&request)
            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
        if request_bytes.len() > MAX_DURABLE_INVOCATION_REQUEST_BYTES {
            self.commit_pre_dispatch_failure(
                &entry.run,
                now,
                execution.execution(),
                "immutable invocation request exceeds the durable event size budget",
            )?;
            return Ok(DispatchOutcome::PreDispatchFailed);
        }
        let lease = self.next_lease_id()?;
        let expires_at = checked_timestamp_add(now, self.config.lease_duration_ms)?;
        let dispatch = ExecutionDispatch::new(
            entry.run.clone(),
            revision.id().clone(),
            node.id().clone(),
            execution.execution().clone(),
            attempt.clone(),
            lease.clone(),
            expires_at,
            resolution.clone(),
            request.clone(),
        )?;
        let schedule = CommandPlan {
            events: vec![
                RunEventKind::NodeScheduled {
                    node: node.id().clone(),
                    execution: execution.execution().clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                    idempotency_key: idempotency_key.clone(),
                    request: request.clone(),
                },
                RunEventKind::CapabilityResolved {
                    execution: execution.execution().clone(),
                    attempt: attempt.clone(),
                    requirement: requirement.clone(),
                    snapshot: resolution.snapshot().clone(),
                },
                RunEventKind::SideEffectClassified {
                    attempt: attempt.clone(),
                    side_effect: contract.side_effect(),
                    idempotency: contract.idempotency(),
                    idempotency_key: idempotency_key.clone(),
                },
                RunEventKind::LeaseGranted {
                    lease: lease.clone(),
                    execution: execution.execution().clone(),
                    attempt: attempt.clone(),
                    worker: self.config.worker.clone(),
                    expires_at,
                },
            ],
            expected_lease_catalog: Some(lease_catalog_witness),
            ..CommandPlan::default()
        };
        match self.commit_internal_plan(
            &entry.run,
            now,
            "schedule_and_lease",
            Some(&attempt),
            schedule,
        ) {
            Ok(_) => {}
            Err(RuntimeError::Persistence(PersistenceError::LeaseCatalogConflict {
                ..
            }))
            | Err(RuntimeError::Persistence(PersistenceError::SequenceConflict { .. })) => {
                return Ok(DispatchOutcome::Deferred);
            }
            Err(error) => return Err(error),
        }
        drop(scheduler_guard);
        info!(
            run = %entry.run,
            revision = %revision.id(),
            node = %node.id(),
            execution = %execution.execution(),
            attempt = %attempt,
            invocation = %invocation,
            lease = %lease,
            "durable lease committed before executor dispatch"
        );
        let reports = match self.executor.execute(&dispatch) {
            Ok(reports) => reports,
            Err(error) => {
                let mut plan = CommandPlan::one(RunEventKind::ExternalOutcomeUncertain {
                    attempt: attempt.clone(),
                    report_sequence: 1,
                    side_effect: contract.side_effect(),
                    reason: Reason::new("executor boundary failed after durable dispatch")?,
                    evidence: Vec::new(),
                });
                if self.config.retry_policy.permits_automatic_retry(
                    attempt_number,
                    ErrorClass::Adapter,
                    true,
                    contract.side_effect(),
                    contract.idempotency(),
                    idempotency_key.as_ref(),
                ) {
                    match self.build_retry_event(
                        execution.execution(),
                        &attempt,
                        attempt_number,
                        now,
                        ErrorClass::Adapter,
                        None,
                        "automatic retry admitted after an uncertain executor boundary",
                    ) {
                        Ok(retry) => plan.events.push(retry),
                        Err(retry_error) => warn!(
                            attempt = %attempt,
                            reason = %retry_error,
                            "executor uncertainty retained without an unavailable retry timer"
                        ),
                    }
                }
                self.commit_internal_plan(
                    &entry.run,
                    now,
                    "dispatch_uncertain",
                    Some(&attempt),
                    plan,
                )?;
                warn!(
                    attempt = %attempt,
                    reason = %error,
                    "executor failure retained as uncertain"
                );
                return Ok(DispatchOutcome::Uncertain);
            }
        };

        self.submit_worker_start(&entry.run, now, &lease, &attempt)?;
        for report in reports.reports() {
            self.submit_worker_invocation(&entry.run, now, &attempt, report.clone())?;
        }
        Ok(DispatchOutcome::Completed)
    }

    fn commit_pre_dispatch_failure(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        execution: &NodeExecutionId,
        detail: &'static str,
    ) -> Result<(), RuntimeError> {
        let plan = CommandPlan::one(RunEventKind::NodePreDispatchFailed {
            execution: execution.clone(),
            error_class: ErrorClass::InvalidRequest,
            detail: Some(BoundedDetail::new(detail)?),
        });
        let _ = self.commit_internal_plan(
            run,
            occurred_at,
            "terminalize_immutable_pre_dispatch_failure",
            None,
            plan,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn invocation_request(
        &self,
        revision: &BlueprintRevision,
        projection: &RunProjection,
        node: &Node,
        occurrence_scope: &ScopeReference,
        invocation: InvocationId,
        capability: milkdrift_capability::CapabilityId,
        provider_profile: Option<milkdrift_capability::ProviderProfileRef>,
        idempotency_key: Option<IdempotencyKey>,
    ) -> Result<InvocationRequest, RuntimeError> {
        self.validate_projected_scope(projection, occurrence_scope, &[])?;
        let mut inputs = Vec::new();
        for (port, declaration) in node.data_inputs() {
            let resolved = match node.kind() {
                NodeKind::Reducer { config }
                    if matches!(config.strategy(), ReducerStrategy::Capability(_))
                        && port == config.input_port() =>
                {
                    let references = self.ordered_reducer_references(
                        revision,
                        projection,
                        node,
                        port,
                        occurrence_scope,
                        &[],
                    )?;
                    if references.len() < usize::from(config.minimum_items()) {
                        return Err(RuntimeError::Scheduling(format!(
                            "capability reducer {} requires at least {} collected inputs",
                            node.id(),
                            config.minimum_items()
                        )));
                    }
                    vec![ResolvedInputValue::Inline {
                        value: BoundedJson::new(serde_json::to_value(references)?)
                            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?,
                        source: None,
                    }]
                }
                NodeKind::Task { .. }
                | NodeKind::Reducer { .. }
                | NodeKind::Branch { .. }
                | NodeKind::Fork { .. }
                | NodeKind::Join { .. }
                | NodeKind::Repeat { .. }
                | NodeKind::Wait { .. }
                | NodeKind::SignalWait { .. }
                | NodeKind::Subworkflow { .. }
                | NodeKind::Terminal { .. } => self.resolve_node_port_inputs(
                    revision,
                    projection,
                    node,
                    port,
                    occurrence_scope,
                    &[],
                )?,
            };
            if resolved.is_empty() {
                if declaration.is_required() {
                    return Err(RuntimeError::Scheduling(format!(
                        "required task input {}:{} is unresolved",
                        node.id(),
                        port
                    )));
                }
                continue;
            }
            if resolved.len() != 1 {
                return Err(RuntimeError::Scheduling(format!(
                    "task input {}:{} resolved to more than one exact value",
                    node.id(),
                    port
                )));
            }
            let resolved_value = resolved.into_iter().next().ok_or_else(|| {
                RuntimeError::InvalidHistory("resolved invocation input disappeared".to_owned())
            })?;
            let value = invocation_value_reference(resolved_value)?;
            inputs.push(
                InputReference::new(port.as_str().to_owned(), value)
                    .map_err(|error| RuntimeError::Scheduling(error.to_string()))?,
            );
        }
        InvocationRequest::new(
            invocation,
            capability,
            match node.kind() {
                NodeKind::Task { requirement } => requirement.operation().clone(),
                NodeKind::Reducer { config } => match config.strategy() {
                    ReducerStrategy::Capability(operation) => operation.clone(),
                    ReducerStrategy::Collect | ReducerStrategy::First => {
                        return Err(RuntimeError::Scheduling(
                            "a deterministic reducer cannot build an invocation".to_owned(),
                        ));
                    }
                },
                NodeKind::Branch { .. }
                | NodeKind::Fork { .. }
                | NodeKind::Join { .. }
                | NodeKind::Repeat { .. }
                | NodeKind::Wait { .. }
                | NodeKind::SignalWait { .. }
                | NodeKind::Subworkflow { .. }
                | NodeKind::Terminal { .. } => {
                    return Err(RuntimeError::Scheduling(
                        "only task or capability-backed reducer nodes can build invocations"
                            .to_owned(),
                    ));
                }
            },
            provider_profile,
            idempotency_key,
            inputs,
            BTreeMap::new(),
        )
        .map_err(|error| RuntimeError::Scheduling(error.to_string()))
    }

    fn commit_internal_plan(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        action: &'static str,
        subject_attempt: Option<&AttemptId>,
        plan: CommandPlan,
    ) -> Result<CommandExecution, RuntimeError> {
        let projection = project_complete_history(self.store.as_ref(), run)?;
        let marker = match subject_attempt {
            Some(attempt) => attempt.clone(),
            None => self.next_attempt_id()?,
        };
        let document = RunCommandDocument::new(
            self.next_command_id()?,
            run.clone(),
            self.config.internal_actor.clone(),
            projection.sequence(),
            occurred_at,
            Reason::new(format!("internal runtime action: {action}"))?,
            Vec::new(),
            RunCommand::WorkerReport {
                worker: self.config.worker.clone(),
                report: WorkerReport::Started { attempt: marker },
            },
        )?;
        let receipt = document.receipt()?;
        let outcome = self.commit_accepted(&document, receipt, projection, plan)?;
        let (result, replayed) = match outcome {
            AtomicRunCommitOutcome::Committed(value) => (value, false),
            AtomicRunCommitOutcome::Replayed(value) => (value, true),
        };
        Ok(CommandExecution { result, replayed })
    }

    fn submit_worker_start(
        &self,
        run: &RunId,
        now: TimestampMillis,
        lease: &LeaseId,
        attempt: &AttemptId,
    ) -> Result<(), RuntimeError> {
        let command = RunCommandDocument::new(
            self.next_command_id()?,
            run.clone(),
            self.config.internal_actor.clone(),
            self.store.head(run)?,
            now,
            Reason::new("synchronous executor accepted its durable lease")?,
            Vec::new(),
            RunCommand::WorkerReport {
                worker: self.config.worker.clone(),
                report: WorkerReport::LeaseAccepted {
                    lease: lease.clone(),
                    attempt: attempt.clone(),
                },
            },
        )?;
        let _ = self.handle_command(&command)?;
        Ok(())
    }

    fn submit_worker_invocation(
        &self,
        run: &RunId,
        now: TimestampMillis,
        attempt: &AttemptId,
        report: InvocationEvent,
    ) -> Result<(), RuntimeError> {
        let command = RunCommandDocument::new(
            self.next_command_id()?,
            run.clone(),
            self.config.internal_actor.clone(),
            self.store.head(run)?,
            now,
            Reason::new("synchronous executor supplied a bounded invocation report")?,
            Vec::new(),
            RunCommand::WorkerReport {
                worker: self.config.worker.clone(),
                report: WorkerReport::Invocation {
                    attempt: attempt.clone(),
                    report,
                },
            },
        )?;
        let execution = self.handle_command(&command)?;
        if execution.result().disposition() == CommandDisposition::Rejected {
            return Err(RuntimeError::InvalidTransition(
                "executor report was durably rejected".to_owned(),
            ));
        }
        Ok(())
    }

    fn propagate_cancellation(
        &self,
        now: TimestampMillis,
        limit: PageSize,
    ) -> Result<(), RuntimeError> {
        for summary in
            self.next_nonterminal_page(&self.cancellation_cursor, limit, "cancellation")?
        {
            if self.structured_scan_budget.load(Ordering::Acquire) == 0 {
                break;
            }
            let projection = self.projection(&summary.run)?;
            let run_reason = run_drain_reason(&projection).cloned();
            let has_branch_cancellation = !projection.cancelling_branch_ids().is_empty();
            if run_reason.is_none()
                && !has_branch_cancellation
                && projection.reconciliation_cancellations().is_empty()
            {
                continue;
            }
            let mut propagation = CommandPlan::default();
            let event_limit = STRUCTURED_EVENT_SOFT_LIMIT;
            let claimed = self.claim_structured_scan_visits(projection.active_branch_ids().len());
            let mut allowance = claimed;
            let branch_ids = bounded_projection_set(
                &summary.run,
                projection.active_branch_ids(),
                &self.cancellation_branch_cursors,
                &mut allowance,
                "cancellation branch scan cursor",
            )?;
            for branch_id in branch_ids {
                if propagation.events.len() == event_limit {
                    break;
                }
                let branch = projection.branches().get(&branch_id).ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "active cancellation branch identity is absent".to_owned(),
                    )
                })?;
                if branch.state() != BranchState::Active {
                    continue;
                }
                let Some(reason) = cancellation_reason_for_branch(
                    &projection,
                    branch.branch(),
                    run_reason.as_ref(),
                ) else {
                    continue;
                };
                propagation
                    .events
                    .push(RunEventKind::BranchCancellationRequested {
                        branch: branch.branch().clone(),
                        reason,
                    });
            }
            let claimed =
                self.claim_structured_scan_visits(projection.active_subworkflow_ids().len());
            let mut allowance = claimed;
            let child_ids = bounded_projection_set(
                &summary.run,
                projection.active_subworkflow_ids(),
                &self.cancellation_subworkflow_cursors,
                &mut allowance,
                "cancellation child scan cursor",
            )?;
            for child_id in child_ids {
                if propagation.events.len() == event_limit {
                    break;
                }
                let child = projection.subworkflows().get(&child_id).ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "active cancellation child identity is absent".to_owned(),
                    )
                })?;
                let reason = cancellation_reason_for_execution(
                    &projection,
                    child.parent_execution(),
                    run_reason.as_ref(),
                );
                if child.state() == SubworkflowState::Active
                    && child.ownership() == SubworkflowOwnership::Attached
                {
                    if let Some(reason) = reason {
                        propagation
                            .events
                            .push(RunEventKind::SubworkflowCancellationRequested {
                                subworkflow: child.subworkflow().clone(),
                                child_run: child.child_run().clone(),
                                reason,
                            });
                    }
                }
            }
            let claimed =
                self.claim_structured_scan_visits(projection.active_execution_ids().len());
            let mut allowance = claimed;
            let execution_ids = bounded_projection_set(
                &summary.run,
                projection.active_execution_ids(),
                &self.cancellation_execution_cursors,
                &mut allowance,
                "cancellation execution scan cursor",
            )?;
            for execution_id in execution_ids {
                if propagation.events.len() == event_limit {
                    break;
                }
                let execution = projection.node_executions().get(&execution_id).ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "active cancellation execution identity is absent".to_owned(),
                    )
                })?;
                let Some(reason) = cancellation_reason_for_execution(
                    &projection,
                    execution.execution(),
                    run_reason.as_ref(),
                ) else {
                    continue;
                };
                match execution.state() {
                    NodeExecutionState::Eligible | NodeExecutionState::RetryPending(_) => {
                        if projection.execution_has_active_child_ownership(
                            execution.execution(),
                        ) {
                            continue;
                        }
                        for timer in
                            projection.pending_timers_for_execution(execution.execution())
                        {
                            if propagation.events.len() == event_limit {
                                break;
                            }
                            propagation.events.push(RunEventKind::TimerCancelled {
                                timer: timer.clone(),
                                reason: reason.clone(),
                            });
                        }
                        if projection
                            .waits()
                            .get(execution.execution())
                            .is_some_and(|wait| wait.is_pending())
                            && propagation.events.len() < event_limit
                        {
                            propagation.events.push(RunEventKind::WaitCancelled {
                                execution: execution.execution().clone(),
                                reason: reason.clone(),
                            });
                        }
                        // Cancelling a retry timer atomically terminalizes the
                        // reserved attempt and its execution. A first-attempt
                        // eligible execution has no such timer-owned transition.
                        if execution.state() == &NodeExecutionState::Eligible
                            && propagation.events.len() < event_limit
                        {
                            propagation.events.push(
                                RunEventKind::NodeExecutionCancelledBeforeDispatch {
                                    execution: execution.execution().clone(),
                                    reason,
                                },
                            );
                        }
                    }
                    NodeExecutionState::Scheduled(attempt)
                    | NodeExecutionState::Running(attempt) => {
                        if execution.cancellation().is_none()
                            && !projection
                                .reconciliation_cancellations()
                                .contains_key(execution.execution())
                            && propagation.events.len() < event_limit
                        {
                            propagation.events.push(
                                RunEventKind::NodeExecutionCancellationRequested {
                                    execution: execution.execution().clone(),
                                    attempt: attempt.clone(),
                                    reason,
                                },
                            );
                        }
                    }
                    NodeExecutionState::Uncertain(_)
                    | NodeExecutionState::CancelledBeforeDispatch
                    | NodeExecutionState::RemovedProspectively(_)
                    | NodeExecutionState::Terminal(_) => {}
                }
            }
            if !propagation.events.is_empty() {
                let marker = projection.attempts().keys().next().cloned();
                let _ = self.commit_internal_plan(
                    &summary.run,
                    now,
                    "propagate_structured_cancellation",
                    marker.as_ref(),
                    propagation,
                )?;
            }
            if self.structured_scan_budget.load(Ordering::Acquire) == 0 {
                continue;
            }
            let projection = self.projection(&summary.run)?;
            let claimed = self.claim_structured_scan_visits(projection.active_attempt_ids().len());
            let mut allowance = claimed;
            let attempt_ids = bounded_projection_set(
                &summary.run,
                projection.active_attempt_ids(),
                &self.cancellation_attempt_cursors,
                &mut allowance,
                "cancellation attempt scan cursor",
            )?;
            let active: Vec<_> = attempt_ids
                .iter()
                .filter_map(|attempt| projection.attempts().get(attempt))
                .filter(|attempt| attempt.is_active())
                .filter(|attempt| attempt.cancellation_acknowledgements().is_empty())
                .filter(|attempt| {
                    cancellation_reason_for_execution(
                        &projection,
                        attempt.execution(),
                        run_drain_reason(&projection),
                    )
                    .is_some()
                })
                .filter(|attempt| {
                    projection
                        .active_lease_for_attempt(attempt.attempt())
                        .is_some_and(|lease| lease.worker() == &self.config.worker)
                })
                .filter_map(|attempt| {
                    let reason = cancellation_reason_for_execution(
                        &projection,
                        attempt.execution(),
                        run_drain_reason(&projection),
                    )?;
                    attempt
                        .invocation()
                        .map(|invocation| (attempt.attempt().clone(), invocation.clone(), reason))
                })
                .collect();
            for (attempt, invocation, reason) in active {
                let projection = self.projection(&summary.run)?;
                let attempt_view = projection.attempts().get(&attempt).ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "active cancellation attempt disappeared".to_owned(),
                    )
                })?;
                let request_sequence = attempt_view
                    .cancellation_acknowledgements()
                    .last()
                    .map_or(1, |acknowledgement| {
                        acknowledgement.request_sequence().saturating_add(1)
                    });
                let request = CancellationRequest::new(
                    invocation,
                    request_sequence,
                    reason.as_str().to_owned(),
                )
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
                let acknowledgement = match self.executor.cancel(&request) {
                    Ok(acknowledgement) => acknowledgement,
                    Err(error) => {
                        warn!(
                            run = %summary.run,
                            attempt = %attempt,
                            reason = %error,
                            "executor cancellation boundary failed; lease remains recoverable"
                        );
                        continue;
                    }
                };
                let command = RunCommandDocument::new(
                    self.next_command_id()?,
                    summary.run.clone(),
                    self.config.internal_actor.clone(),
                    self.store.head(&summary.run)?,
                    now,
                    Reason::new("executor acknowledged durable cancellation intent")?,
                    Vec::new(),
                    RunCommand::WorkerReport {
                        worker: self.config.worker.clone(),
                        report: WorkerReport::Cancellation {
                            attempt,
                            acknowledgement,
                        },
                    },
                )?;
                let _ = self.handle_command(&command)?;
            }
        }
        Ok(())
    }

    fn admission_usage(&self) -> Result<(AdmissionUsage, IntegrityDigest), RuntimeError> {
        let mut usage = AdmissionUsage::default();
        let global_limit = self.config.scheduler_limits.global();
        let snapshot = self.store.active_leases(PageSize::new(global_limit)?)?;
        if snapshot.entries.len()
            == usize::try_from(global_limit).map_err(|_error| {
                RuntimeError::Scheduling("global admission limit does not fit usize".to_owned())
            })?
        {
            // The queried bound is the hard global limit. Reaching it is sufficient
            // to decline every new dispatch without projecting unrelated aggregates.
            usage.global = global_limit;
            return Ok((usage, snapshot.witness));
        }

        let mut projections = BTreeMap::new();
        for indexed in &snapshot.entries {
            if !projections.contains_key(&indexed.run) {
                projections.insert(indexed.run.clone(), self.projection(&indexed.run)?);
            }
        }
        for indexed in snapshot.entries {
            let projection = projections.get(&indexed.run).ok_or_else(|| {
                RuntimeError::InvalidHistory("active lease run projection is absent".to_owned())
            })?;
            let lease = projection.leases().get(&indexed.lease).ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "active lease index references an absent lease".to_owned(),
                )
            })?;
            if !lease.is_active()
                || lease.attempt() != &indexed.attempt
                || lease.worker() != &indexed.worker
                || lease.expires_at() != indexed.expires_at
                || projection.sequence() < indexed.through_sequence
            {
                return Err(RuntimeError::InvalidHistory(
                    "active lease index disagrees with authoritative run history".to_owned(),
                ));
            }
            let attempt = projection.attempts().get(lease.attempt()).ok_or_else(|| {
                RuntimeError::InvalidHistory("active lease has no attempt".to_owned())
            })?;
            let capability = attempt.capability().ok_or_else(|| {
                RuntimeError::InvalidHistory("active lease has no capability resolution".to_owned())
            })?;
            usage.global = usage.global.checked_add(1).ok_or_else(|| {
                RuntimeError::Scheduling("global admission count overflow".to_owned())
            })?;
            checked_increment(&mut usage.runs, indexed.run.clone())?;
            checked_increment(
                &mut usage.capability_classes,
                capability.snapshot().operation().clone(),
            )?;
            let execution = projection
                .node_executions()
                .get(attempt.execution())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory("attempt execution is absent".to_owned())
                })?;
            if let Some(ScopeKind::Branch { branch }) = projection
                .scopes()
                .get(execution.scope())
                .map(WorkspaceScope::kind)
            {
                checked_increment(&mut usage.branches, (indexed.run.clone(), branch.clone()))?;
            }
        }
        Ok((usage, snapshot.witness))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DispatchOutcome {
    Completed,
    Uncertain,
    Deferred,
    PreDispatchFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RepeatBudgetStatus {
    Within,
    Exhausted(RepeatContinuationCause),
    AccountingOverflow,
}

#[derive(Clone, Debug)]
enum ResolvedInputValue {
    Inline {
        value: BoundedJson,
        source: Option<WorkspaceValueReference>,
    },
    Workspace(WorkspaceValueReference),
    Artifact(ArtifactReference),
}

#[derive(Default)]
struct CommandPlan {
    events: Vec<RunEventKind>,
    workspace: Vec<WorkspaceMutation>,
    creation_usage: Option<(WorkspaceUsage, WorkspaceUsage, BTreeSet<ArtifactReference>)>,
    required_artifacts: BTreeSet<ArtifactReference>,
    expected_lease_catalog: Option<IntegrityDigest>,
}

impl CommandPlan {
    fn one(event: RunEventKind) -> Self {
        Self {
            events: vec![event],
            ..Self::default()
        }
    }
}

fn require_lifecycle(
    projection: &RunProjection,
    required: RunLifecycle,
    transition: &str,
) -> Result<(), RuntimeError> {
    if projection.lifecycle() == required {
        Ok(())
    } else {
        Err(RuntimeError::InvalidTransition(format!(
            "cannot {transition} from lifecycle {:?}",
            projection.lifecycle()
        )))
    }
}

fn durable_rejection(error: &RuntimeError) -> bool {
    matches!(
        error,
        RuntimeError::InvalidCommand(_)
            | RuntimeError::InvalidTransition(_)
            | RuntimeError::Scheduling(_)
            | RuntimeError::Reconciliation(_)
            | RuntimeError::Executor(_)
    )
}

const fn node_outcome(outcome: RunOutcome) -> NodeOutcome {
    match outcome {
        RunOutcome::Succeeded => NodeOutcome::Succeeded,
        RunOutcome::Failed => NodeOutcome::Failed,
        RunOutcome::Cancelled => NodeOutcome::Cancelled,
    }
}

fn checked_timestamp_add(
    timestamp: TimestampMillis,
    duration_ms: u64,
) -> Result<TimestampMillis, RuntimeError> {
    timestamp
        .get()
        .checked_add(duration_ms)
        .map(TimestampMillis::new)
        .ok_or_else(|| RuntimeError::Scheduling("timestamp overflow".to_owned()))
}

const fn node_execution_mode(node: &Node) -> NodeExecutionMode {
    match node.kind() {
        NodeKind::Task { .. } => NodeExecutionMode::Executor,
        NodeKind::Reducer { config }
            if matches!(config.strategy(), ReducerStrategy::Capability(_)) =>
        {
            NodeExecutionMode::Executor
        }
        NodeKind::Branch { .. }
        | NodeKind::Fork { .. }
        | NodeKind::Join { .. }
        | NodeKind::Reducer { .. }
        | NodeKind::Repeat { .. }
        | NodeKind::Wait { .. }
        | NodeKind::SignalWait { .. }
        | NodeKind::Subworkflow { .. }
        | NodeKind::Terminal { .. } => NodeExecutionMode::Runtime,
    }
}

fn bounded_projection_set<K: Clone + Ord>(
    run: &RunId,
    values: &BTreeSet<K>,
    cursor: &Mutex<BTreeMap<RunId, K>>,
    remaining: &mut usize,
    label: &'static str,
) -> Result<Vec<K>, RuntimeError> {
    let limit = (*remaining).min(values.len());
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut cursors = cursor.lock().map_err(|_error| {
        RuntimeError::Scheduling(format!("{label} coordination lock is poisoned"))
    })?;
    let previous = cursors.get(run).cloned();
    let mut selected = Vec::with_capacity(limit);
    if let Some(previous) = previous {
        selected.extend(
            values
                .range((Excluded(previous.clone()), Unbounded))
                .take(limit)
                .cloned(),
        );
        if selected.len() < limit {
            selected.extend(
                values
                    .range(..=previous)
                    .take(limit.saturating_sub(selected.len()))
                    .cloned(),
            );
        }
    } else {
        selected.extend(values.iter().take(limit).cloned());
    }
    if let Some(last) = selected.last() {
        cursors.insert(run.clone(), last.clone());
    }
    *remaining = remaining.saturating_sub(selected.len());
    Ok(selected)
}

fn bounded_projection_map_keys<K: Clone + Ord, V>(
    run: &RunId,
    values: &BTreeMap<K, V>,
    cursor: &Mutex<BTreeMap<RunId, K>>,
    remaining: &mut usize,
    label: &'static str,
) -> Result<Vec<K>, RuntimeError> {
    let limit = (*remaining).min(values.len());
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut cursors = cursor.lock().map_err(|_error| {
        RuntimeError::Scheduling(format!("{label} coordination lock is poisoned"))
    })?;
    let previous = cursors.get(run).cloned();
    let mut selected = Vec::with_capacity(limit);
    if let Some(previous) = previous {
        selected.extend(
            values
                .range((Excluded(previous.clone()), Unbounded))
                .take(limit)
                .map(|(key, _)| key.clone()),
        );
        if selected.len() < limit {
            selected.extend(
                values
                    .range(..=previous)
                    .take(limit.saturating_sub(selected.len()))
                    .map(|(key, _)| key.clone()),
            );
        }
    } else {
        selected.extend(values.keys().take(limit).cloned());
    }
    if let Some(last) = selected.last() {
        cursors.insert(run.clone(), last.clone());
    }
    *remaining = remaining.saturating_sub(selected.len());
    Ok(selected)
}

fn entry_nodes(revision: &BlueprintRevision) -> Vec<&NodeId> {
    let targeted: BTreeSet<_> = revision
        .semantic()
        .edges()
        .values()
        .map(|edge| edge.target_node())
        .collect();
    revision
        .semantic()
        .nodes()
        .keys()
        .filter(|node| !targeted.contains(node))
        .collect()
}

fn control_nodes_before_join(
    revision: &BlueprintRevision,
    start: &NodeId,
    join: &NodeId,
) -> BTreeSet<NodeId> {
    let mut result = BTreeSet::new();
    let mut pending = VecDeque::from([start.clone()]);
    while let Some(node) = pending.pop_front() {
        if &node == join || !result.insert(node.clone()) {
            continue;
        }
        pending.extend(
            revision
                .semantic()
                .edges()
                .values()
                .filter(|edge| edge.kind() == EdgeKind::Control && edge.source_node() == &node)
                .map(|edge| edge.target_node().clone()),
        );
    }
    result
}

fn wait_signal_matches(
    condition: &WaitCondition,
    signal_type: &milkdrift_persistence::SignalTypeId,
    correlation: Option<&milkdrift_persistence::CorrelationKey>,
) -> bool {
    match condition {
        WaitCondition::Signal {
            signal_type: expected,
            correlation: expected_correlation,
        }
        | WaitCondition::SignalOrTimer {
            signal_type: expected,
            correlation: expected_correlation,
            ..
        } => expected == signal_type && expected_correlation.as_ref() == correlation,
        WaitCondition::Timer { .. } => false,
    }
}

fn predecessors_ready(
    revision: &BlueprintRevision,
    projection: &RunProjection,
    target: &Node,
    target_scope: &ScopeReference,
) -> bool {
    if let NodeKind::Join { config } = target.kind() {
        return projection
            .executions_for_node(config.fork())
            .any(|execution| {
                execution.scope() == target_scope
                    && execution_is_in_current_node_epoch(projection, execution)
            });
    }
    let control_ready = revision
        .semantic()
        .edges()
        .values()
        .filter(|edge| edge.kind() == EdgeKind::Control && edge.target_node() == target.id())
        .all(|edge| {
            projection
                .executions_for_node(edge.source_node())
                .any(|execution| {
                    execution_is_in_current_node_epoch(projection, execution)
                        && execution.scope() == target_scope
                        && execution.state()
                            == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                        && projection
                            .branch_routes()
                            .get(execution.execution())
                            .is_none_or(|port| port == edge.source_port())
                })
        });
    let data_ready = revision
        .semantic()
        .edges()
        .values()
        .filter(|edge| {
            edge.kind() == EdgeKind::Data
                && edge.target_node() == target.id()
                && target
                    .data_inputs()
                    .get(edge.target_port())
                    .is_some_and(milkdrift_blueprint::DataPort::is_required)
        })
        .all(|edge| {
            projection
                .executions_for_node(edge.source_node())
                .filter(|execution| {
                    execution_is_in_current_node_epoch(projection, execution)
                        && execution_scope_related(projection, execution.scope(), target_scope)
                        && execution.state()
                            == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                })
                .any(|execution| {
                    execution
                        .outputs()
                        .iter()
                        .any(|output| output.value().key().as_str() == edge.source_port().as_str())
                })
        });
    control_ready && data_ready
}

fn execution_scope_related(
    projection: &RunProjection,
    source_scope: &ScopeReference,
    target_scope: &ScopeReference,
) -> bool {
    if source_scope == target_scope {
        return true;
    }
    let mut cursor = projection
        .scopes()
        .get(target_scope)
        .and_then(WorkspaceScope::parent);
    for _ in 0..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let Some(scope) = cursor else {
            break;
        };
        if scope == source_scope {
            return true;
        }
        cursor = projection
            .scopes()
            .get(scope)
            .and_then(WorkspaceScope::parent);
    }
    let Some(ScopeKind::Branch { branch }) = projection
        .scopes()
        .get(source_scope)
        .map(WorkspaceScope::kind)
    else {
        return false;
    };
    projection
        .branches()
        .get(branch)
        .and_then(|branch| projection.node_executions().get(branch.fork_execution()))
        .is_some_and(|fork| fork.scope() == target_scope)
}

fn execution_branch_state(
    projection: &RunProjection,
    execution: &NodeExecutionId,
) -> Option<BranchState> {
    let execution = projection.node_executions().get(execution)?;
    let mut cursor = Some(execution.scope());
    let mut active_branch = None;
    for _ in 0..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let Some(scope) = cursor else {
            break;
        };
        let scope_view = projection.scopes().get(scope)?;
        if let ScopeKind::Branch { branch } = scope_view.kind() {
            let state = projection.branches().get(branch)?.state();
            if state != BranchState::Active {
                return Some(state);
            }
            active_branch = Some(state);
        }
        cursor = scope_view.parent();
    }
    active_branch
}

fn scope_has_inactive_branch(projection: &RunProjection, scope: &ScopeReference) -> bool {
    let mut cursor = Some(scope);
    for _ in 0..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let Some(reference) = cursor else {
            break;
        };
        let Some(scope_view) = projection.scopes().get(reference) else {
            return true;
        };
        if let ScopeKind::Branch { branch } = scope_view.kind() {
            if projection
                .branches()
                .get(branch)
                .is_none_or(|branch| branch.state() != BranchState::Active)
            {
                return true;
            }
        }
        cursor = scope_view.parent();
    }
    false
}

fn run_drain_reason(projection: &RunProjection) -> Option<&Reason> {
    projection
        .cancellation()
        .map(|cancellation| cancellation.reason())
        .or_else(|| {
            projection
                .termination()
                .map(|termination| termination.reason())
        })
}

fn cancellation_reason_for_branch(
    projection: &RunProjection,
    branch: &BranchId,
    run_reason: Option<&Reason>,
) -> Option<Reason> {
    if let Some(reason) = run_reason {
        return Some(reason.clone());
    }
    let branch = projection.branches().get(branch)?;
    let mut cursor = branch.scope().parent();
    for _ in 0..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let Some(scope) = cursor else {
            break;
        };
        let scope_view = projection.scopes().get(scope)?;
        if let ScopeKind::Branch { branch } = scope_view.kind() {
            let ancestor = projection.branches().get(branch)?;
            if ancestor.state() == BranchState::Cancelling {
                return ancestor.cancellation_reason().cloned();
            }
        }
        cursor = scope_view.parent();
    }
    None
}

fn cancellation_reason_for_execution(
    projection: &RunProjection,
    execution: &NodeExecutionId,
    run_reason: Option<&Reason>,
) -> Option<Reason> {
    if let Some(reason) = run_reason {
        return Some(reason.clone());
    }
    if let Some(cancellation) = projection.reconciliation_cancellations().get(execution) {
        return Some(cancellation.reason().clone());
    }
    let execution = projection.node_executions().get(execution)?;
    let mut cursor = Some(execution.scope());
    for _ in 0..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let Some(scope) = cursor else {
            break;
        };
        let scope_view = projection.scopes().get(scope)?;
        if let ScopeKind::Branch { branch } = scope_view.kind() {
            let branch = projection.branches().get(branch)?;
            if branch.state() == BranchState::Cancelling {
                return branch.cancellation_reason().cloned();
            }
        }
        cursor = scope_view.parent();
    }
    None
}

fn reconciliation_history(
    projection: &RunProjection,
    revision: &BlueprintRevision,
    target_revision: &BlueprintRevision,
) -> Result<BTreeMap<NodeId, Vec<NodeHistory>>, RuntimeError> {
    let mut result: BTreeMap<NodeId, Vec<NodeHistory>> = BTreeMap::new();
    let mut retained = 0_usize;
    for execution in projection.node_executions().values() {
        if !revision.semantic().nodes().contains_key(execution.node())
            && !target_revision
                .semantic()
                .nodes()
                .contains_key(execution.node())
        {
            continue;
        }
        retained = retained.saturating_add(1);
        if retained > MAX_RECONCILIATION_PLAN_ITEMS {
            return Err(RuntimeError::Reconciliation(format!(
                "reconciliation history exceeds {MAX_RECONCILIATION_PLAN_ITEMS} relevant executions"
            )));
        }
        let attempt = execution
            .attempts()
            .last()
            .and_then(|attempt| projection.attempts().get(attempt));
        let side_effect = attempt
            .and_then(|attempt| attempt.side_effect())
            .map_or(SideEffectClass::None, |classification| {
                classification.side_effect()
            });
        let structured_active = projection
            .execution_has_active_structured_ownership(execution.execution())
            || revision
                .semantic()
                .nodes()
                .get(execution.node())
                .is_some_and(|node| match node.kind() {
                    NodeKind::Join { config } => projection
                        .executions_for_node(config.fork())
                        .filter(|fork| fork.scope() == execution.scope())
                        .any(|fork| {
                            projection.branches().values().any(|branch| {
                                branch.is_active() && branch.fork_execution() == fork.execution()
                            })
                        }),
                    NodeKind::Branch { .. }
                    | NodeKind::Fork { .. }
                    | NodeKind::Repeat { .. }
                    | NodeKind::Wait { .. }
                    | NodeKind::SignalWait { .. }
                    | NodeKind::Subworkflow { .. }
                    | NodeKind::Terminal { .. }
                    | NodeKind::Task { .. }
                    | NodeKind::Reducer { .. } => false,
                });
        let state = match execution.state() {
            NodeExecutionState::Eligible if structured_active => HistoricalExecutionState::Active {
                side_effect,
                // Structured cancellation has its own durable protocol; the
                // attempt-local cancel-and-restart action is not enactable.
                cancellation_safe: false,
            },
            NodeExecutionState::Eligible => HistoricalExecutionState::Pending,
            NodeExecutionState::RetryPending(_) => HistoricalExecutionState::Active {
                side_effect,
                cancellation_safe: false,
            },
            NodeExecutionState::Scheduled(_) | NodeExecutionState::Running(_) => {
                HistoricalExecutionState::Active {
                    side_effect,
                    cancellation_safe: matches!(
                        side_effect,
                        SideEffectClass::None | SideEffectClass::ReadOnly
                    ),
                }
            }
            NodeExecutionState::Uncertain(_) => HistoricalExecutionState::Uncertain { side_effect },
            NodeExecutionState::CancelledBeforeDispatch => HistoricalExecutionState::Completed {
                side_effect: SideEffectClass::None,
            },
            NodeExecutionState::RemovedProspectively(_) => HistoricalExecutionState::Pending,
            NodeExecutionState::Terminal(_) => HistoricalExecutionState::Completed { side_effect },
        };
        result
            .entry(execution.node().clone())
            .or_default()
            .push(NodeHistory::new(
                execution.execution().clone(),
                execution.scope().clone(),
                execution.created_sequence(),
                state,
            ));
    }
    for histories in result.values_mut() {
        histories.sort_by(|left, right| {
            left.created_sequence()
                .cmp(&right.created_sequence())
                .then_with(|| left.execution().cmp(right.execution()))
                .then_with(|| left.scope().cmp(right.scope()))
        });
    }
    Ok(result)
}

fn node_occurrence_exists_for_current_pin(
    projection: &RunProjection,
    node: &NodeId,
    scope: &ScopeReference,
) -> bool {
    let added_after = node_current_epoch_boundary(projection, node);
    projection.executions_for_node(node).any(|execution| {
        execution.scope() == scope
            && added_after.is_none_or(|boundary| execution.created_sequence() > boundary)
    })
}

fn node_current_epoch_boundary(
    projection: &RunProjection,
    node: &NodeId,
) -> Option<RunSequence> {
    node_epoch_boundary_at(projection, node, projection.sequence())
}

fn node_epoch_boundary_at(
    projection: &RunProjection,
    node: &NodeId,
    through: RunSequence,
) -> Option<RunSequence> {
    projection.pins().iter().rev().find_map(|pin| {
        if pin.effective_sequence() > through {
            return None;
        }
        let plan = pin.plan()?;
        projection
            .reconciliation()
            .plans()
            .get(plan)?
            .items()
            .iter()
            .any(|item| {
                item.node.as_ref() == Some(node)
                    && (item.classification == ReconciliationClassification::Added
                        && item.execution.is_none()
                        || item.classification == ReconciliationClassification::ChangedPending)
                    && item.action == ReconciliationAction::UseNewOnNextInvocation
            })
            .then_some(pin.effective_sequence())
    })
}

fn execution_is_in_current_node_epoch(
    projection: &RunProjection,
    execution: &crate::projection::NodeExecutionProjection,
) -> bool {
    node_current_epoch_boundary(projection, execution.node())
        .is_none_or(|boundary| execution.created_sequence() > boundary)
}

fn source_execution_is_valid_for_occurrence(
    projection: &RunProjection,
    source: &crate::projection::NodeExecutionProjection,
    target_node: &NodeId,
    target_scope: &ScopeReference,
) -> bool {
    let target_created = projection
        .executions_for_node(target_node)
        .filter(|target| target.scope() == target_scope)
        .map(crate::projection::NodeExecutionProjection::created_sequence)
        .max();
    let Some(target_created) = target_created else {
        return false;
    };
    source.created_sequence() <= target_created
        && node_epoch_boundary_at(projection, source.node(), target_created)
            .is_none_or(|boundary| source.created_sequence() > boundary)
}

fn recovery_classification(
    attempt: &crate::projection::NodeAttemptProjection,
) -> RecoveryClassification {
    let Some(side_effect) = attempt.side_effect() else {
        return RecoveryClassification::Uncertain;
    };
    match side_effect.side_effect() {
        SideEffectClass::None | SideEffectClass::ReadOnly => RecoveryClassification::Retryable,
        SideEffectClass::IdempotentWrite
            if side_effect.idempotency() != IdempotencyBehavior::Unsupported
                && side_effect.idempotency_key().is_some() =>
        {
            RecoveryClassification::Retryable
        }
        SideEffectClass::IdempotentWrite
        | SideEffectClass::NonIdempotentWrite
        | SideEffectClass::Unknown => RecoveryClassification::Uncertain,
    }
}

fn unresolved_retry_error_class(
    attempt: &crate::projection::NodeAttemptProjection,
) -> ErrorClass {
    if attempt
        .recovery()
        .iter()
        .any(|observation| observation.lease().is_some())
    {
        ErrorClass::Transport
    } else {
        ErrorClass::Adapter
    }
}

const fn recovery_reason(classification: RecoveryClassification) -> &'static str {
    match classification {
        RecoveryClassification::NotStarted => {
            "no executor start or active lease was observed during recovery"
        }
        RecoveryClassification::Retryable => {
            "the expired work is read-only, side-effect-free, or protected by durable idempotency"
        }
        RecoveryClassification::LeaseStillValid => {
            "an unexpired durable lease still owns this invocation"
        }
        RecoveryClassification::Uncertain => {
            "the expired invocation may have externally visible effects"
        }
        RecoveryClassification::TerminalObserved => {
            "a durable terminal outcome was already observed"
        }
    }
}

fn collect_required_artifacts(
    events: &[RunEventEnvelope],
    workspace: &[WorkspaceMutation],
) -> Result<BTreeSet<ArtifactReference>, RuntimeError> {
    let mut required = BTreeSet::new();
    for mutation in workspace {
        if let WorkspaceMutation::PutValue { entry } = mutation {
            if let Some(artifact) = entry.value().as_artifact() {
                required.insert(artifact.clone());
            }
        }
    }
    for event in events {
        required.extend(event.kind().required_artifacts()?);
    }
    Ok(required)
}

fn command_kind_name(command: &RunCommand) -> &'static str {
    match command {
        RunCommand::CreateRun { .. } => "create_run",
        RunCommand::StartRun => "start_run",
        RunCommand::PauseRun => "pause_run",
        RunCommand::ResumeRun => "resume_run",
        RunCommand::RequestCancellation => "request_cancellation",
        RunCommand::DeliverSignal { .. } => "deliver_signal",
        RunCommand::FireTimer { .. } => "fire_timer",
        RunCommand::RequestRevisionAdoption { .. } => "request_revision_adoption",
        RunCommand::DecideReconciliation { .. } => "decide_reconciliation",
        RunCommand::ApplyReconciliation { .. } => "apply_reconciliation",
        RunCommand::DecideRepeatContinuation { .. } => "decide_repeat_continuation",
        RunCommand::ResolveExternalWork { .. } => "resolve_external_work",
        RunCommand::WorkerReport { .. } => "worker_report",
    }
}

fn event_kind_name(event: &RunEventKind) -> &'static str {
    match event {
        RunEventKind::RunCreated { .. } => "run_created",
        RunEventKind::RevisionPinned { .. } => "revision_pinned",
        RunEventKind::RunStarted => "run_started",
        RunEventKind::RunPaused { .. } => "run_paused",
        RunEventKind::RunResumed { .. } => "run_resumed",
        RunEventKind::RunCancellationRequested { .. } => "run_cancellation_requested",
        RunEventKind::RunTerminationRequested { .. } => "run_termination_requested",
        RunEventKind::RunTerminal { .. } => "run_terminal",
        RunEventKind::NodeBecameEligible { .. } => "node_became_eligible",
        RunEventKind::NodeExecutionCancelledBeforeDispatch { .. } => {
            "node_execution_cancelled_before_dispatch"
        }
        RunEventKind::NodeExecutionCancellationRequested { .. } => {
            "node_execution_cancellation_requested"
        }
        RunEventKind::NodeScheduled { .. } => "node_scheduled",
        RunEventKind::CapabilityResolved { .. } => "capability_resolved",
        RunEventKind::SideEffectClassified { .. } => "side_effect_classified",
        RunEventKind::LeaseGranted { .. } => "lease_granted",
        RunEventKind::LeaseHeartbeatRecorded { .. } => "lease_heartbeat_recorded",
        RunEventKind::LeaseExpired { .. } => "lease_expired",
        RunEventKind::NodeReLeased { .. } => "node_re_leased",
        RunEventKind::NodeStarted { .. } => "node_started",
        RunEventKind::NodeProgressRecorded { .. } => "node_progress_recorded",
        RunEventKind::AttemptUsageRecorded { .. } => "attempt_usage_recorded",
        RunEventKind::InvocationCancellationAcknowledged { .. } => {
            "invocation_cancellation_acknowledged"
        }
        RunEventKind::NodeOutputPublished { .. } => "node_output_published",
        RunEventKind::DeterministicOutputPublished { .. } => "deterministic_output_published",
        RunEventKind::DeterministicNodeTerminal { .. } => "deterministic_node_terminal",
        RunEventKind::NodePreDispatchFailed { .. } => "node_pre_dispatch_failed",
        RunEventKind::StructuredSuccessorScanCompleted { .. } => {
            "structured_successor_scan_completed"
        }
        RunEventKind::NodeTerminal { .. } => "node_terminal",
        RunEventKind::NodeRetryScheduled { .. } => "node_retry_scheduled",
        RunEventKind::ExternalOutcomeUncertain { .. } => "external_outcome_uncertain",
        RunEventKind::ExternalOutcomeRetained { .. } => "external_outcome_retained",
        RunEventKind::ArtifactPublished { .. } => "artifact_published",
        RunEventKind::BranchScopeCreated { .. } => "branch_scope_created",
        RunEventKind::BranchRouteSelected { .. } => "branch_route_selected",
        RunEventKind::BranchChildAdded { .. } => "branch_child_added",
        RunEventKind::BranchCancellationRequested { .. } => "branch_cancellation_requested",
        RunEventKind::BranchTerminal { .. } => "branch_terminal",
        RunEventKind::JoinSatisfied { .. } => "join_satisfied",
        RunEventKind::RepeatIterationCreated { .. } => "repeat_iteration_created",
        RunEventKind::RepeatConditionRecorded { .. } => "repeat_condition_recorded",
        RunEventKind::RepeatContinuationRequested { .. } => "repeat_continuation_requested",
        RunEventKind::RepeatContinuationDecided { .. } => "repeat_continuation_decided",
        RunEventKind::RepeatTerminated { .. } => "repeat_terminated",
        RunEventKind::TimerRegistered { .. } => "timer_registered",
        RunEventKind::TimerFired { .. } => "timer_fired",
        RunEventKind::TimerCancelled { .. } => "timer_cancelled",
        RunEventKind::WaitRegistered { .. } => "wait_registered",
        RunEventKind::WaitSatisfied { .. } => "wait_satisfied",
        RunEventKind::WaitCancelled { .. } => "wait_cancelled",
        RunEventKind::SignalReceived { .. } => "signal_received",
        RunEventKind::SignalBroadcastScanAdvanced { .. } => "signal_broadcast_scan_advanced",
        RunEventKind::SignalDeduplicated { .. } => "signal_deduplicated",
        RunEventKind::SignalConsumed { .. } => "signal_consumed",
        RunEventKind::SubworkflowCreated { .. } => "subworkflow_created",
        RunEventKind::SubworkflowTerminal { .. } => "subworkflow_terminal",
        RunEventKind::SubworkflowOutputImported { .. } => "subworkflow_output_imported",
        RunEventKind::SubworkflowCancellationRequested { .. } => {
            "subworkflow_cancellation_requested"
        }
        RunEventKind::RevisionAdoptionRequested { .. } => "revision_adoption_requested",
        RunEventKind::ReconciliationPlanRecorded { .. } => "reconciliation_plan_recorded",
        RunEventKind::ReconciliationDecisionRecorded { .. } => "reconciliation_decision_recorded",
        RunEventKind::ReconciliationApplied { .. } => "reconciliation_applied",
        RunEventKind::ReconciliationExecutionRemoved { .. } => "reconciliation_execution_removed",
        RunEventKind::ReconciliationCancellationRequested { .. } => {
            "reconciliation_cancellation_requested"
        }
        RunEventKind::ReconciliationRemediationCreated { .. } => {
            "reconciliation_remediation_created"
        }
        RunEventKind::RecoveryStarted { .. } => "recovery_started",
        RunEventKind::RecoveryClassified { .. } => "recovery_classified",
        RunEventKind::RecoveryDecisionRecorded { .. } => "recovery_decision_recorded",
        RunEventKind::RemediationWorkCreated { .. } => "remediation_work_created",
    }
}

fn stable_idempotency_key(
    run: &RunId,
    execution: &NodeExecutionId,
) -> Result<IdempotencyKey, RuntimeError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.runtime.idempotency-key.v1\0");
    hasher.update(run.as_str().as_bytes());
    // Durable identities exclude NUL, so these separators make the tuple framing
    // unambiguous without length conversion or concatenating its bounded inputs.
    hasher.update(b"\0");
    hasher.update(execution.as_str().as_bytes());
    IdempotencyKey::new(format!("milkdrift-v1-{}", hasher.finalize().to_hex()))
        .map_err(|error| RuntimeError::Scheduling(error.to_string()))
}

fn invocation_workspace_reference(
    reference: &WorkspaceValueReference,
) -> Result<InvocationValueReference, RuntimeError> {
    let identity = serde_json::to_string(reference)?;
    Ok(InvocationValueReference::WorkspaceValue {
        identity,
        version: reference.version().get().to_string(),
    })
}

fn invocation_value_reference(
    value: ResolvedInputValue,
) -> Result<InvocationValueReference, RuntimeError> {
    match value {
        ResolvedInputValue::Inline { value, .. } => Ok(InvocationValueReference::Inline { value }),
        ResolvedInputValue::Workspace(reference) => invocation_workspace_reference(&reference),
        ResolvedInputValue::Artifact(reference) => {
            let capability_reference = milkdrift_capability::ArtifactReference::new(
                reference.artifact().as_str().to_owned(),
                reference.digest().to_hex(),
                Some(reference.media_type().as_str().to_owned()),
                Some(reference.size_bytes()),
            )
            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            Ok(InvocationValueReference::Artifact {
                reference: capability_reference,
            })
        }
    }
}

fn select_json_path(
    value: &BoundedJson,
    segments: &[PathSegment],
) -> Result<BoundedJson, RuntimeError> {
    let mut selected = value.value();
    for segment in segments {
        selected = match segment {
            PathSegment::Field(field) => selected.get(field.as_str()).ok_or_else(|| {
                RuntimeError::Scheduling(format!("structured input path field {field} is absent"))
            })?,
            PathSegment::Index(index) => selected.get(usize::from(*index)).ok_or_else(|| {
                RuntimeError::Scheduling(format!("structured input path index {index} is absent"))
            })?,
        };
    }
    BoundedJson::new(selected.clone()).map_err(|error| RuntimeError::Scheduling(error.to_string()))
}

fn workspace_value_as_bounded(value: &WorkspaceValue) -> Result<BoundedJson, RuntimeError> {
    match value {
        WorkspaceValue::Json(value) => Ok(value.clone()),
        WorkspaceValue::Artifact(reference) => artifact_reference_as_bounded(reference),
    }
}

fn artifact_reference_as_bounded(
    reference: &ArtifactReference,
) -> Result<BoundedJson, RuntimeError> {
    BoundedJson::new(serde_json::to_value(reference)?)
        .map_err(|error| RuntimeError::Scheduling(error.to_string()))
}

fn checked_increment<K: Ord>(map: &mut BTreeMap<K, u32>, key: K) -> Result<(), RuntimeError> {
    let value = map.entry(key).or_insert(0);
    *value = value
        .checked_add(1)
        .ok_or_else(|| RuntimeError::Scheduling("admission count overflow".to_owned()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn maximum_length_durable_identities_have_a_fixed_length_idempotency_key() -> TestResult {
        let run = RunId::new("r".repeat(128))?;
        let execution = NodeExecutionId::new("e".repeat(192))?;
        let other = NodeExecutionId::new("f".repeat(192))?;
        let key = stable_idempotency_key(&run, &execution)?;
        assert!(key.as_str().len() <= 192);
        assert_eq!(key, stable_idempotency_key(&run, &execution)?);
        assert_ne!(key, stable_idempotency_key(&run, &other)?);
        Ok(())
    }
}
