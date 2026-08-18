//! Synchronous, headless orchestration over the durable runtime ports.
//!
//! The service deliberately owns no thread, task, polling loop, or hidden mutable
//! projection.  Callers drive it with bounded command, scheduler, and recovery calls;
//! every decision which can affect replay is committed before an executor is entered.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicBool, Ordering},
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
    JoinRule, LeaseId, LeaseIndexEntry, LeaseIndexMutation, MAX_PAGE_SIZE,
    MAX_REPEAT_CONTINUATION_DECISIONS, NodeExecutionId, NodeExecutionMode, NodeOutcome, PageSize,
    PersistenceError, Reason, ReconciliationAction, ReconciliationDecisionId, ReconciliationPlanId,
    RecoveryClassification, RepeatContinuationCause, RepeatContinuationDecision,
    RepeatTerminationReason, RevisionStore, RunEventEnvelope, RunEventKind, RunIndexUpdate,
    RunJournal, RunOutcome, RunQueryStore, RunSequence, RunSummaryCursor, RunSummaryIndex,
    RunnableCursor, RunnableIndexEntry, RunnableIndexMutation, SignalDeliveryMode, SnapshotStore,
    StorageAdmin, StorageHealth, StorageSchemaCompatibility, SubworkflowOwnership, TimerId,
    TimerIndexEntry, TimerIndexMutation, TimestampMillis, WaitCondition, WaitSatisfaction, WorkerId,
    WorkspaceAccounting, WorkspaceMutation, WorkspaceStore,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactReference, BranchId, CausalReference, IterationId, RunId, ScopeId,
    ScopeKind, ScopeReference, SubworkflowId, ValueKey, ValueOrigin, ValueVersion, WorkspaceBudget,
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
/// recovery admission. Deployments with multiple service instances must additionally
/// elect one scheduler owner when global admission limits must hold across processes;
/// durable sequence guards and leases still prevent duplicate ownership of one attempt.
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
                let revision = self.current_revision(projection)?;
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
            let node_view = revision.semantic().nodes().get(&node).ok_or_else(|| {
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
        let mut compatible: Vec<_> = projection
            .waits()
            .values()
            .filter(|wait| {
                wait.is_pending() && wait_signal_matches(wait.condition(), signal_type, correlation)
            })
            .map(|wait| wait.execution().clone())
            .collect();
        compatible.sort();
        if mode == SignalDeliveryMode::OneShot {
            compatible.truncate(1);
        }
        for execution in compatible {
            plan.events.push(RunEventKind::SignalConsumed {
                signal: signal.clone(),
                execution: execution.clone(),
            });
            self.append_signal_payload_to_plan(projection, &mut plan, &execution, payload)?;
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
        if !projection.lifecycle().is_active() || projection.reconciliation().is_active() {
            return Err(RuntimeError::InvalidTransition(
                "revision adoption requires an active run with no active reconciliation".to_owned(),
            ));
        }
        let old = self.current_revision(projection)?;
        let workflow = projection
            .workflow()
            .ok_or_else(|| RuntimeError::InvalidHistory("run has no workflow".to_owned()))?;
        let new = self.load_validated_revision(requested_revision, Some(workflow))?;
        let history = reconciliation_history(projection);
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
                    let execution = item.execution.as_ref().ok_or_else(|| {
                        RuntimeError::Reconciliation(
                            "remove-unstarted item has no exact execution".to_owned(),
                        )
                    })?;
                    result
                        .events
                        .push(RunEventKind::ReconciliationExecutionRemoved {
                            plan: plan.clone(),
                            execution: execution.clone(),
                        });
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
                let attempt_number =
                    attempt_view
                        .attempt_number()
                        .checked_add(1)
                        .ok_or_else(|| {
                            RuntimeError::Scheduling("attempt number overflow".to_owned())
                        })?;
                let next_attempt = self.next_attempt_id()?;
                let timer = self.next_timer_id()?;
                plan.events.push(RunEventKind::NodeRetryScheduled {
                    execution: attempt_view.execution().clone(),
                    previous_attempt: attempt.clone(),
                    next_attempt,
                    attempt_number,
                    timer,
                    fire_at: document.issued_at(),
                    error_class: ErrorClass::Unknown,
                    reason: document.reason().clone(),
                });
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
        let (metadata, artifact) = self.resolve_executor_artifact(reference)?;
        let key = ValueKey::new(name.to_owned())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let entry = match self.store.latest_value(execution.scope(), &key)? {
            Some(previous) => WorkspaceValueEntry::successor(
                previous.reference().clone(),
                WorkspaceValue::Artifact(artifact.clone()),
            )
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?,
            None => WorkspaceValueEntry::initial(
                execution.scope().clone(),
                key,
                WorkspaceValue::Artifact(artifact.clone()),
            ),
        };
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
                let next_number =
                    attempt_view
                        .attempt_number()
                        .checked_add(1)
                        .ok_or_else(|| {
                            RuntimeError::Scheduling("attempt number overflow".to_owned())
                        })?;
                let delay = self.config.retry_policy.retry_delay_ms(
                    next_number,
                    0,
                    failure.retry_after_ms(),
                )?;
                let fire_at = checked_timestamp_add(document.issued_at(), delay)?;
                plan.events.push(RunEventKind::NodeRetryScheduled {
                    execution: attempt_view.execution().clone(),
                    previous_attempt: attempt.clone(),
                    next_attempt: self.next_attempt_id()?,
                    attempt_number: next_number,
                    timer: self.next_timer_id()?,
                    fire_at,
                    error_class: failure.class(),
                    reason: Reason::new("bounded automatic retry policy admitted another attempt")?,
                });
            }
        }
        Ok(plan)
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
            candidate.apply(&event)?;
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
        let required_artifacts = collect_required_artifacts(&envelopes, &plan.workspace);
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
            revision.as_ref(),
            document.issued_at(),
        )?;
        let request = AtomicRunCommitRequest::new(
            receipt,
            envelopes,
            plan.workspace,
            Some(accounting),
            required_artifacts.into_iter().collect(),
            newly_referenced_artifacts.into_iter().collect(),
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
        for _ in 0..MAX_DRIVER_PASSES {
            if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            let before = events.len();
            let eligible: Vec<_> = projection
                .node_executions()
                .values()
                .filter(|execution| execution.state() == &NodeExecutionState::Eligible)
                .map(|execution| {
                    (
                        execution.execution().clone(),
                        execution.node().clone(),
                        execution.scope().clone(),
                    )
                })
                .collect();
            for (execution, node_id, scope_reference) in eligible {
                if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                    return Ok(());
                }
                let node = revision.semantic().nodes().get(&node_id).ok_or_else(|| {
                    RuntimeError::InvalidHistory(format!(
                        "eligible node {node_id} is absent from pinned revision {}",
                        revision.id()
                    ))
                })?;
                let structurally_cancelling = projection.lifecycle() == RunLifecycle::Cancelling
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
                        TerminalOutcome::Success => self.complete_deterministic(
                            run,
                            occurred_at,
                            projection,
                            events,
                            node,
                            &execution,
                        )?,
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
                            {
                                self.push_projected_event(
                                    run,
                                    occurred_at,
                                    projection,
                                    events,
                                    RunEventKind::RunCancellationRequested {
                                        reason: Reason::new(
                                            "explicit failure terminal is draining owned work",
                                        )?,
                                        evidence: Vec::new(),
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
                            let registered_condition = projection
                                .waits()
                                .get(&execution)
                                .ok_or_else(|| {
                                    RuntimeError::InvalidHistory(
                                        "newly registered signal wait is absent".to_owned(),
                                    )
                                })?
                                .condition()
                                .clone();
                            let queued = projection
                                .signals()
                                .values()
                                .filter(|candidate| {
                                    candidate.is_pending()
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
                                    let entry = self.derived_output_entry_with_pending(
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
                    NodeKind::Branch { config } => {
                        if !projection.branch_routes().contains_key(&execution) {
                            let mut selected = None;
                            let context =
                                self.evaluation_context(node, projection, &scope_reference)?;
                            for (port, condition) in config.arms() {
                                if evaluate_condition(condition, &context)? {
                                    selected = Some(port.clone());
                                    break;
                                }
                            }
                            let selected = selected.or_else(|| config.fallback().cloned()).ok_or_else(
                                || {
                                    RuntimeError::Scheduling(format!(
                                        "branch node {node_id} selected no route and has no fallback"
                                    ))
                                },
                            )?;
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
                        let already_created = projection
                            .branches()
                            .values()
                            .any(|branch| branch.fork_execution() == &execution);
                        if !already_created {
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
                                let branch = self.next_branch_id()?;
                                let scope = WorkspaceScope::branch(
                                    self.next_scope_id()?,
                                    &parent,
                                    branch.clone(),
                                )
                                .map_err(|error| {
                                    RuntimeError::InvalidTransition(error.to_string())
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
                                let targets: Vec<_> = revision
                                    .semantic()
                                    .edges()
                                    .values()
                                    .filter(|edge| {
                                        edge.kind() == EdgeKind::Control
                                            && edge.source_node() == &node_id
                                            && edge.source_port() == port
                                    })
                                    .map(|edge| edge.target_node().clone())
                                    .collect();
                                for target in targets {
                                    let child_execution = self.next_execution_id()?;
                                    self.push_projected_event(
                                        run,
                                        occurred_at,
                                        projection,
                                        events,
                                        RunEventKind::NodeBecameEligible {
                                            mode: node_execution_mode(
                                                revision
                                                    .semantic()
                                                    .nodes()
                                                    .get(&target)
                                                    .ok_or_else(|| {
                                                        RuntimeError::InvalidHistory(
                                                            "branch target node is absent"
                                                                .to_owned(),
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
                                            branch: branch.clone(),
                                            execution: child_execution,
                                        },
                                    )?;
                                }
                            }
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
                                revision,
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

            self.close_finished_branches(run, occurred_at, projection, events)?;
            self.add_ready_successors(run, occurred_at, revision, projection, events)?;
            self.try_finalize_run(run, occurred_at, revision, projection, events)?;
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
        let mut values = BTreeSet::new();
        for edge in self
            .current_revision(projection)?
            .semantic()
            .edges()
            .values()
            .filter(|edge| {
                edge.kind() == EdgeKind::Data
                    && edge.target_node() == node.id()
                    && edge.target_port() == config.input_port()
            })
        {
            for source in projection
                .executions_for_node(edge.source_node())
                .filter(|source| {
                    execution_scope_related(projection, source.scope(), scope_reference)
                })
            {
                for output in source
                    .outputs()
                    .iter()
                    .filter(|output| output.value().key().as_str() == edge.source_port().as_str())
                {
                    values.insert(output.value().clone());
                }
            }
        }
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
                let json_value = serde_json::to_value(values.iter().collect::<Vec<_>>())?;
                (
                    WorkspaceValue::Json(
                        BoundedJson::new(json_value)
                            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?,
                    ),
                    None,
                )
            }
            ReducerStrategy::First => {
                let reference = values.first().ok_or_else(|| {
                    RuntimeError::Scheduling("first reducer has no input".to_owned())
                })?;
                let entry = self.store.value(reference)?.ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "reducer input workspace value is absent".to_owned(),
                    )
                })?;
                let artifact = entry.value().as_artifact().cloned();
                (entry.value().clone(), artifact)
            }
            ReducerStrategy::Capability(_) => return Ok(()),
        };
        let entry = self.derived_output_entry(scope_reference, key, value)?;
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
        let parent_revision = self.current_revision(projection)?;
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
        for (field, interface_field) in child_revision.semantic().interface().inputs() {
            let port = PortId::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let Some(parent_declaration) = node.data_inputs().get(&port) else {
                if interface_field.is_required() {
                    return Err(RuntimeError::InvalidTransition(format!(
                        "required subworkflow input {field} has no parent node data port"
                    )));
                }
                continue;
            };
            let resolved = self.resolve_node_port_inputs(
                &parent_revision,
                projection,
                node,
                &port,
                occurrence_scope,
                workspace,
            )?;
            if resolved.is_empty() {
                if interface_field.is_required() || parent_declaration.is_required() {
                    return Err(RuntimeError::InvalidTransition(format!(
                        "required subworkflow input {field} is unresolved"
                    )));
                }
                continue;
            }
            if resolved.len() != 1 {
                return Err(RuntimeError::InvalidTransition(format!(
                    "subworkflow input {field} resolved to more than one value"
                )));
            }
            let key = ValueKey::new(field.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let resolved_value = resolved.into_iter().next().ok_or_else(|| {
                RuntimeError::InvalidHistory("resolved subworkflow input disappeared".to_owned())
            })?;
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
            projection
                .cancellation()
                .map(|cancellation| cancellation.reason()),
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

        let budget_exhaustion = if authority_budget_override {
            None
        } else {
            self.repeat_budget_exhaustion(config, projection, execution, occurred_at)?
        };
        if let Some(cause) = budget_exhaustion {
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
                        result: config.termination() == RepeatTermination::AwaitApproval,
                    },
                )?;
            }
            if config.termination() == RepeatTermination::AwaitApproval {
                if let Some((iteration, _, _)) = latest.as_ref() {
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
            let outcome = match config.termination() {
                RepeatTermination::SucceedWithLatest if has_success => NodeOutcome::Succeeded,
                RepeatTermination::SucceedWithLatest | RepeatTermination::Fail => {
                    NodeOutcome::Failed
                }
                RepeatTermination::AwaitApproval => {
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
                (outcome != NodeOutcome::Succeeded)
                    .then(|| BoundedDetail::new("repeat budget was exhausted"))
                    .transpose()?,
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

        let result = evaluate_condition(
            config.condition(),
            &self.evaluation_context(node, projection, scope_reference)?,
        )?;
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
    ) -> Result<Option<RepeatContinuationCause>, RuntimeError> {
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
                return Ok(Some(RepeatContinuationCause::DurationBudget {
                    maximum_ms: maximum,
                    observed_ms: observed,
                }));
            }
        }
        let Some(maximum_cost) = config.budget().max_cost_micros else {
            return Ok(None);
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
                observed_cost = observed_cost.checked_add(*cost).ok_or_else(|| {
                    RuntimeError::Scheduling("repeat cost accounting overflow".to_owned())
                })?;
            }
        }
        if observed_cost >= maximum_cost {
            Ok(Some(RepeatContinuationCause::CostBudget {
                maximum_micros: maximum_cost,
                observed_micros: observed_cost,
                currency,
            }))
        } else {
            Ok(None)
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
            let source = workspace
                .iter()
                .rev()
                .find_map(|mutation| match mutation {
                    WorkspaceMutation::PutValue { entry } if entry.reference() == &imported => {
                        Some(entry.clone())
                    }
                    WorkspaceMutation::CreateScope { .. } | WorkspaceMutation::PutValue { .. } => {
                        None
                    }
                })
                .or(self.store.value(&imported)?)
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "latest repeat output import is absent from durable workspace".to_owned(),
                    )
                })?;
            let output = self.derived_output_entry(
                execution_scope,
                source.reference().key().clone(),
                source.value().clone(),
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
        let declaration = node.data_inputs().get(port).ok_or_else(|| {
            RuntimeError::InvalidHistory(format!(
                "node {} has no declared data input {port}",
                node.id()
            ))
        })?;
        if let Some(binding) = declaration.binding() {
            return self
                .resolve_binding(
                    projection,
                    occurrence_scope,
                    binding,
                    pending_workspace,
                    true,
                )
                .map(|value| vec![value]);
        }
        Ok(self
            .incoming_data_references(revision, projection, node, port, occurrence_scope)
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
                    execution_scope_related(projection, source.scope(), occurrence_scope)
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
    ) -> Vec<WorkspaceValueReference> {
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
                    execution_scope_related(projection, source.scope(), occurrence_scope)
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
                ordered.push(reference);
            }
        }
        ordered
    }

    fn resolve_binding(
        &self,
        projection: &RunProjection,
        occurrence_scope: &ScopeReference,
        binding: &BindingSource,
        pending_workspace: &[WorkspaceMutation],
        apply_path: bool,
    ) -> Result<ResolvedInputValue, RuntimeError> {
        match binding {
            BindingSource::Literal { value } => Ok(ResolvedInputValue::Inline {
                value: value.clone(),
                source: None,
            }),
            BindingSource::WorkflowInput { field }
            | BindingSource::SubworkflowParameter { field } => {
                let root = projection.root_scope().ok_or_else(|| {
                    RuntimeError::InvalidHistory("run has no root scope".to_owned())
                })?;
                let key = ValueKey::new(field.as_str().to_owned())
                    .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
                let entry = self
                    .latest_workspace_value(root.reference(), &key, pending_workspace)?
                    .ok_or_else(|| {
                        RuntimeError::Scheduling(format!(
                            "required workflow input {field} is absent"
                        ))
                    })?;
                Ok(ResolvedInputValue::Workspace(entry.reference().clone()))
            }
            BindingSource::NodeOutput { node, port, path } => {
                let references: BTreeSet<_> = projection
                    .executions_for_node(node)
                    .filter(|source| {
                        execution_scope_related(projection, source.scope(), occurrence_scope)
                            && source.state()
                                == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                    })
                    .flat_map(|source| source.outputs())
                    .filter(|output| output.value().key().as_str() == port.as_str())
                    .map(|output| output.value().clone())
                    .collect();
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
                    return Ok(ResolvedInputValue::Workspace(reference));
                }
                let entry = self
                    .workspace_value(&reference, pending_workspace)?
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "node output workspace value is absent".to_owned(),
                        )
                    })?;
                let json_value = entry.value().as_json().ok_or_else(|| {
                    RuntimeError::Scheduling(format!(
                        "node output {node}:{port} is an artifact and cannot be path-selected"
                    ))
                })?;
                let selected = select_json_path(json_value, path.segments())?;
                Ok(ResolvedInputValue::Inline {
                    value: selected,
                    source: Some(reference),
                })
            }
            BindingSource::WorkspaceValue { reference, .. } => {
                let parsed = serde_json::from_str::<WorkspaceValueReference>(reference).map_err(
                    |error| {
                        RuntimeError::Scheduling(format!(
                            "workspace binding is not an exact canonical reference: {error}"
                        ))
                    },
                )?;
                if self.workspace_value(&parsed, pending_workspace)?.is_none() {
                    return Err(RuntimeError::Scheduling(
                        "workspace binding references an absent immutable value".to_owned(),
                    ));
                }
                self.ensure_readable_ancestor(projection, parsed.scope(), occurrence_scope)?;
                Ok(ResolvedInputValue::Workspace(parsed))
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
                Ok(ResolvedInputValue::Artifact(parsed))
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

    fn latest_workspace_value(
        &self,
        scope: &ScopeReference,
        key: &ValueKey,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Option<WorkspaceValueEntry>, RuntimeError> {
        if let Some(entry) = pending_workspace
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
        {
            return Ok(Some(entry.clone()));
        }
        self.store
            .latest_value(scope, key)
            .map_err(RuntimeError::from)
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
                    self.ensure_readable_ancestor(projection, source.scope(), target_parent_scope)?;
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
                self.ensure_readable_ancestor(projection, source.scope(), target_parent_scope)?;
                let entry = self
                    .workspace_value(&source, pending_workspace)?
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "subworkflow input source value is absent".to_owned(),
                        )
                    })?;
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
    ) -> Result<(), RuntimeError> {
        let mut cursor = Some(target_parent);
        for _ in 0..=milkdrift_workspace::MAX_SCOPE_DEPTH {
            let Some(scope) = cursor else {
                break;
            };
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
        projection.apply(&event)?;
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
        let _ = node;
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
        )
    }

    fn try_finalize_run(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        revision: &BlueprintRevision,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
    ) -> Result<(), RuntimeError> {
        if projection.is_completed()
            || projection
                .node_executions()
                .values()
                .any(|execution| !execution.is_completed())
            || projection.attempts().values().any(|attempt| {
                !matches!(
                    attempt.state(),
                    AttemptState::Terminal(_) | AttemptState::Resolved(_)
                )
            })
            || projection.leases().values().any(|lease| lease.is_active())
            || projection
                .branches()
                .values()
                .any(|branch| branch.is_active())
            || projection
                .iterations()
                .values()
                .any(|iteration| iteration.is_active())
            || projection.timers().values().any(|timer| timer.is_pending())
            || projection
                .retries()
                .values()
                .any(|retry| retry.is_pending())
            || projection.waits().values().any(|wait| wait.is_pending())
            || projection.subworkflows().values().any(|child| {
                child.ownership() == SubworkflowOwnership::Attached && child.is_active()
            })
            || projection.reconciliation().is_active()
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
        let outcome = terminal_executions.last().map_or_else(
            || {
                if projection.lifecycle() == RunLifecycle::Cancelling {
                    RunOutcome::Cancelled
                } else if projection.node_executions().values().any(|execution| {
                    matches!(
                        execution.state(),
                        NodeExecutionState::Terminal(NodeOutcome::Failed | NodeOutcome::Rejected)
                    )
                }) {
                    RunOutcome::Failed
                } else {
                    RunOutcome::Succeeded
                }
            },
            |(_, _, terminal)| match terminal {
                TerminalOutcome::Success => RunOutcome::Succeeded,
                TerminalOutcome::Failure => RunOutcome::Failed,
                TerminalOutcome::Cancelled => RunOutcome::Cancelled,
            },
        );
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
                let has_port = terminal_node
                    .data_inputs()
                    .keys()
                    .any(|port| port.as_str() == field.as_str());
                let resolved = if has_port {
                    revision
                        .semantic()
                        .edges()
                        .values()
                        .filter(|edge| {
                            edge.kind() == EdgeKind::Data
                                && edge.target_node() == terminal_node.id()
                                && edge.target_port().as_str() == field.as_str()
                        })
                        .find_map(|edge| {
                            projection
                                .executions_for_node(edge.source_node())
                                .filter(|source| {
                                    source.state()
                                        == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                                        && execution_scope_related(
                                            projection,
                                            source.scope(),
                                            terminal_execution.scope(),
                                        )
                                })
                                .flat_map(|source| source.outputs())
                                .find(|output| {
                                    output.value().key().as_str() == edge.source_port().as_str()
                                })
                                .map(|output| output.value().clone())
                        })
                } else {
                    None
                };
                match resolved {
                    Some(reference) => {
                        if let Some(artifact) = self
                            .store
                            .value(&reference)?
                            .and_then(|entry| entry.value().as_artifact().cloned())
                        {
                            if projection.artifacts().contains_key(artifact.artifact()) {
                                artifacts.insert(artifact);
                            }
                        }
                        outputs.insert(reference);
                    }
                    None if declaration.is_required() => {
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
                    .map(|cancellation| cancellation.reason().clone()),
            },
        )
    }

    fn close_finished_branches(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        projection: &mut RunProjection,
        events: &mut Vec<RunEventEnvelope>,
    ) -> Result<(), RuntimeError> {
        let terminal: Vec<_> = projection
            .branches()
            .values()
            .filter(|branch| {
                matches!(
                    branch.state(),
                    BranchState::Active | BranchState::Cancelling
                ) && branch.children().iter().all(|child| {
                    projection
                        .node_executions()
                        .get(child)
                        .is_some_and(|execution| execution.is_completed())
                })
            })
            .map(|branch| {
                let children: Vec<_> = branch
                    .children()
                    .iter()
                    .filter_map(|child| projection.node_executions().get(child))
                    .collect();
                let outcome = if branch.state() == BranchState::Cancelling {
                    RunOutcome::Cancelled
                } else if children.iter().all(|child| {
                    child.state() == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                }) {
                    RunOutcome::Succeeded
                } else if children.iter().all(|child| {
                    matches!(
                        child.state(),
                        NodeExecutionState::Terminal(
                            NodeOutcome::Succeeded | NodeOutcome::Cancelled
                        )
                    )
                }) {
                    RunOutcome::Cancelled
                } else {
                    RunOutcome::Failed
                };
                let outputs: BTreeSet<_> = children
                    .iter()
                    .flat_map(|child| child.outputs())
                    .map(|output| output.value().clone())
                    .collect();
                (branch.branch().clone(), outcome, outputs)
            })
            .collect();
        for (branch, outcome, outputs) in terminal {
            if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
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
            .filter(|value| value.scope() == &join_scope)
            .last()
            .map(|value| value.execution().clone());
        let Some(fork_execution) = fork_execution else {
            return Ok(());
        };
        let mut completed = Vec::new();
        let mut active = Vec::new();
        for branch in projection
            .branches()
            .values()
            .filter(|branch| branch.fork_execution() == &fork_execution)
        {
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
    ) -> Result<(), RuntimeError> {
        let completed: Vec<_> = projection
            .node_executions()
            .values()
            .filter(|execution| {
                execution.state() == &NodeExecutionState::Terminal(NodeOutcome::Succeeded)
            })
            .map(|execution| {
                (
                    execution.node().clone(),
                    execution.scope().clone(),
                    execution.execution().clone(),
                )
            })
            .collect();
        let mut candidates = BTreeSet::new();
        for (source_node, source_scope, source_execution) in completed {
            let source = revision
                .semantic()
                .nodes()
                .get(&source_node)
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "completed node is absent from revision".to_owned(),
                    )
                })?;
            if matches!(
                source.kind(),
                NodeKind::Fork { .. } | NodeKind::Terminal { .. }
            ) {
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
        }

        // A join is owned by its fork occurrence, not by a successful branch tail.
        // In particular, FirstSuccess and Quorum must become runnable when every
        // branch has failed so that the impossible threshold is recorded on the
        // join execution instead of being hidden by generic graph exhaustion.
        for target in revision.semantic().nodes().values() {
            let NodeKind::Join { config } = target.kind() else {
                continue;
            };
            for fork in projection.executions_for_node(config.fork()) {
                let has_terminal_branch = projection.branches().values().any(|branch| {
                    branch.fork_execution() == fork.execution()
                        && matches!(branch.state(), BranchState::Completed(_))
                });
                if has_terminal_branch {
                    candidates.insert((target.id().clone(), fork.scope().clone()));
                }
            }
        }
        for (target, mut scope) in candidates {
            if events.len() >= STRUCTURED_EVENT_SOFT_LIMIT {
                return Ok(());
            }
            let target_node = revision.semantic().nodes().get(&target).ok_or_else(|| {
                RuntimeError::InvalidHistory("control edge target is absent".to_owned())
            })?;
            if let NodeKind::Join { config } = target_node.kind() {
                if let Some(fork) = projection.executions_for_node(config.fork()).last() {
                    scope = fork.scope().clone();
                }
            }
            if projection
                .node_executions()
                .values()
                .any(|execution| execution.node() == &target && execution.scope() == &scope)
            {
                continue;
            }
            if !predecessors_ready(revision, projection, target_node, &scope) {
                continue;
            }
            let execution = self.next_execution_id()?;
            let owning_branch = projection
                .branches()
                .values()
                .find(|branch| branch.scope().reference() == &scope)
                .map(|branch| branch.branch().clone());
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
        Ok(())
    }

    fn evaluation_context(
        &self,
        node: &Node,
        projection: &RunProjection,
        occurrence_scope: &ScopeReference,
    ) -> Result<EvaluationContext, RuntimeError> {
        let mut context = EvaluationContext::default();
        for port in node.data_inputs().values() {
            let Some(source) = port.binding() else {
                continue;
            };
            if matches!(source, BindingSource::Literal { .. }) {
                continue;
            }
            let resolved =
                self.resolve_binding(projection, occurrence_scope, source, &[], false)?;
            let value = match resolved {
                ResolvedInputValue::Inline { value, .. } => value,
                ResolvedInputValue::Workspace(reference) => {
                    let entry = self.store.value(&reference)?.ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "condition workspace value is absent".to_owned(),
                        )
                    })?;
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

    fn derived_output_entry(
        &self,
        scope: &ScopeReference,
        key: ValueKey,
        value: WorkspaceValue,
    ) -> Result<WorkspaceValueEntry, RuntimeError> {
        match self.store.latest_value(scope, &key)? {
            Some(previous) => WorkspaceValueEntry::successor(previous.reference().clone(), value)
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string())),
            None => Ok(WorkspaceValueEntry::initial(scope.clone(), key, value)),
        }
    }

    fn derived_output_entry_with_pending(
        &self,
        scope: &ScopeReference,
        key: ValueKey,
        value: WorkspaceValue,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<WorkspaceValueEntry, RuntimeError> {
        match self.latest_workspace_value(scope, &key, pending_workspace)? {
            Some(previous) => WorkspaceValueEntry::successor(previous.reference().clone(), value)
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string())),
            None => Ok(WorkspaceValueEntry::initial(scope.clone(), key, value)),
        }
    }

    fn append_signal_payload_to_plan(
        &self,
        projection: &RunProjection,
        plan: &mut CommandPlan,
        execution: &NodeExecutionId,
        payload: &BoundedJson,
    ) -> Result<(), RuntimeError> {
        let execution_view = projection.node_executions().get(execution).ok_or_else(|| {
            RuntimeError::InvalidHistory("signal wait execution is absent".to_owned())
        })?;
        let revision = self.current_revision(projection)?;
        let node = revision
            .semantic()
            .nodes()
            .get(execution_view.node())
            .ok_or_else(|| RuntimeError::InvalidHistory("signal wait node is absent".to_owned()))?;
        for port in node.data_outputs().keys() {
            let key = ValueKey::new(port.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            let entry = self.derived_output_entry_with_pending(
                execution_view.scope(),
                key,
                WorkspaceValue::Json(payload.clone()),
                &plan.workspace,
            )?;
            let value = entry.reference().clone();
            plan.workspace.push(WorkspaceMutation::PutValue { entry });
            plan.events
                .push(RunEventKind::DeterministicOutputPublished {
                    execution: execution.clone(),
                    value,
                    artifact: None,
                });
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
        revision: Option<&BlueprintRevision>,
        updated_at: TimestampMillis,
    ) -> Result<RunIndexUpdate, RuntimeError> {
        let through = new.sequence();
        let workflow = new.workflow().ok_or_else(|| {
            RuntimeError::InvalidHistory("indexed run has no workflow".to_owned())
        })?;
        let revision_id = new.revision().ok_or_else(|| {
            RuntimeError::InvalidHistory("indexed run has no revision pin".to_owned())
        })?;
        let runnable = runnable_executions(new, revision);
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

        let all_executions: BTreeSet<_> = old
            .node_executions()
            .keys()
            .chain(new.node_executions().keys())
            .cloned()
            .collect();
        for execution in all_executions {
            if let Some(eligible_at) = runnable.get(&execution) {
                update.runnable.push(RunnableIndexMutation::Upsert {
                    entry: RunnableIndexEntry {
                        run: run.clone(),
                        execution,
                        eligible_at: *eligible_at,
                        priority: 0,
                        through_sequence: through,
                    },
                });
            } else {
                update.runnable.push(RunnableIndexMutation::Remove {
                    run: run.clone(),
                    execution,
                });
            }
        }

        let all_timers: BTreeSet<_> = old
            .timers()
            .keys()
            .chain(new.timers().keys())
            .cloned()
            .collect();
        for timer in all_timers {
            match new
                .timers()
                .get(&timer)
                .filter(|candidate| candidate.is_pending())
            {
                Some(candidate) => update.timers.push(TimerIndexMutation::Upsert {
                    entry: TimerIndexEntry {
                        run: run.clone(),
                        timer,
                        fire_at: candidate.fire_at(),
                        through_sequence: through,
                    },
                }),
                None => update.timers.push(TimerIndexMutation::Remove {
                    run: run.clone(),
                    timer,
                }),
            }
        }

        let all_leases: BTreeSet<_> = old
            .leases()
            .keys()
            .chain(new.leases().keys())
            .cloned()
            .collect();
        for lease in all_leases {
            match new
                .leases()
                .get(&lease)
                .filter(|candidate| candidate.is_active())
            {
                Some(candidate) => update.leases.push(LeaseIndexMutation::Upsert {
                    entry: LeaseIndexEntry {
                        run: run.clone(),
                        lease,
                        attempt: candidate.attempt().clone(),
                        worker: candidate.worker().clone(),
                        expires_at: candidate.expires_at(),
                        through_sequence: through,
                    },
                }),
                None => update.leases.push(LeaseIndexMutation::Remove {
                    run: run.clone(),
                    lease,
                }),
            }
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
        {
            let _scheduler_guard = self.scheduler_gate.lock().map_err(|_error| {
                RuntimeError::Scheduling(
                    "runtime scheduler coordination lock is poisoned".to_owned(),
                )
            })?;
            if !self.is_accepting_admission() {
                // Closing dispatch admission must not suppress an already durable
                // cancellation boundary. This path may release waits and request
                // executor cancellation, but it never creates a new dispatch lease.
                self.propagate_cancellation(now, limit)?;
                result.deferred = 1;
                return Ok(result);
            }
            for timer in self.store.due_timers(now, limit)? {
                let expected = self.store.head(&timer.run)?;
                let command = RunCommandDocument::new(
                    self.next_command_id()?,
                    timer.run,
                    self.config.internal_actor.clone(),
                    expected,
                    now,
                    Reason::new("scheduler observed a durable timer at or after its deadline")?,
                    Vec::new(),
                    RunCommand::FireTimer { timer: timer.timer },
                )?;
                let _ = self.handle_command(&command)?;
            }
            self.propagate_cancellation(now, limit)?;
            self.drive_reconciliation_restarts(now, limit)?;
            self.drive_child_aggregates(now, limit)?;
            self.drive_structured_runs(now, limit)?;
        }

        let entries = self.store.runnable(now, limit)?;
        let selected = select_fair_runnable(entries, usize::from(self.config.maximum_tick_items));
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

    /// Replays and classifies a bounded page of nonterminal runs.  Expired safe work
    /// is converted into an explicit failed attempt plus bounded retry timer; unsafe
    /// work becomes an authority-visible uncertainty obligation.
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
        for summary in self.store.nonterminal_runs(limit)? {
            let projection = self.projection(&summary.run)?;
            result.runs_examined = result.runs_examined.saturating_add(1);
            let mut plan = CommandPlan::one(RunEventKind::RecoveryStarted {
                controller: self.config.worker.clone(),
                through_sequence: projection.sequence(),
            });
            for attempt in projection
                .attempts()
                .values()
                .take(usize::from(self.config.maximum_tick_items))
            {
                if plan.events.len()
                    > milkdrift_persistence::MAX_EVENTS_PER_COMMIT.saturating_sub(4)
                {
                    break;
                }
                let active_lease = projection
                    .leases()
                    .values()
                    .rfind(|lease| lease.attempt() == attempt.attempt() && lease.is_active());
                let classification = if attempt.is_completed() {
                    RecoveryClassification::TerminalObserved
                } else if let Some(lease) = active_lease {
                    if lease.expires_at() > now {
                        RecoveryClassification::LeaseStillValid
                    } else {
                        recovery_classification(attempt)
                    }
                } else if attempt.is_unresolved() {
                    RecoveryClassification::Uncertain
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
                        result.retryable = result.retryable.saturating_add(1);
                        let next_report =
                            self.next_report_sequence(&projection, attempt.attempt())?;
                        plan.events.push(RunEventKind::NodeTerminal {
                            execution: attempt.execution().clone(),
                            attempt: attempt.attempt().clone(),
                            report_sequence: next_report,
                            outcome: NodeOutcome::Failed,
                            error_class: Some(ErrorClass::Transport),
                            detail: Some(BoundedDetail::new(
                                "lease expired before a terminal outcome was recorded",
                            )?),
                        });
                        let side_effect = attempt.side_effect();
                        let permit = side_effect.is_some_and(|classification| {
                            self.config.retry_policy.permits_automatic_retry(
                                attempt.attempt_number(),
                                ErrorClass::Transport,
                                true,
                                classification.side_effect(),
                                classification.idempotency(),
                                classification.idempotency_key(),
                            )
                        });
                        if permit {
                            let next_number =
                                attempt.attempt_number().checked_add(1).ok_or_else(|| {
                                    RuntimeError::Scheduling("attempt number overflow".to_owned())
                                })?;
                            let retry_delay =
                                self.config
                                    .retry_policy
                                    .retry_delay_ms(next_number, 0, None)?;
                            plan.events.push(RunEventKind::NodeRetryScheduled {
                                execution: attempt.execution().clone(),
                                previous_attempt: attempt.attempt().clone(),
                                next_attempt: self.next_attempt_id()?,
                                attempt_number: next_number,
                                timer: self.next_timer_id()?,
                                fire_at: checked_timestamp_add(now, retry_delay)?,
                                error_class: ErrorClass::Transport,
                                reason: Reason::new(
                                    "recovery admitted a safe bounded retry after lease expiry",
                                )?,
                            });
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
        for summary in self.store.nonterminal_runs(limit)? {
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
        for summary in self.store.nonterminal_runs(limit)? {
            let projection = self.projection(&summary.run)?;
            for cancellation in projection.reconciliation_cancellations().values() {
                let source = projection
                    .node_executions()
                    .get(cancellation.execution())
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "reconciliation cancellation source is absent".to_owned(),
                        )
                    })?;
                if source.state() != &NodeExecutionState::Terminal(NodeOutcome::Cancelled)
                    || projection.node_executions().values().any(|candidate| {
                        candidate.node() == source.node()
                            && candidate.scope() == source.scope()
                            && candidate.created_sequence() > cancellation.sequence()
                    })
                {
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
                if let Some(branch) = projection
                    .branches()
                    .values()
                    .find(|branch| branch.children().contains(source.execution()))
                {
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
        let mut remaining = usize::from(self.config.maximum_tick_items);
        for summary in self.store.nonterminal_runs(limit)? {
            if remaining == 0 {
                break;
            }
            let parent = self.projection(&summary.run)?;
            let revision = self.current_revision(&parent)?;
            let children: Vec<_> = parent
                .subworkflows()
                .values()
                .filter(|child| !child.is_completed())
                .map(|child| {
                    (
                        child.subworkflow().clone(),
                        child.parent_execution().clone(),
                        child.child_run().clone(),
                        child.child_revision().clone(),
                        child.scope().clone(),
                        child.inputs().to_vec(),
                        child.state(),
                    )
                })
                .collect();
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
                if remaining == 0 {
                    return Ok(());
                }
                remaining = remaining.saturating_sub(1);
                let mut child_head = self.store.head(&child_run)?;
                if child_head == RunSequence::ZERO {
                    let child_blueprint = self.load_validated_revision(&child_revision, None)?;
                    let root_scope =
                        WorkspaceScope::run_root(child_run.clone(), self.next_scope_id()?);
                    let mut inputs_by_key = BTreeMap::new();
                    for reference in &input_references {
                        let entry = self.store.value(reference)?.ok_or_else(|| {
                            RuntimeError::InvalidHistory(
                                "subworkflow input value is absent from parent workspace"
                                    .to_owned(),
                            )
                        })?;
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
                let parent_node = revision
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
                    let source = self.store.value(child_value)?.ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "child terminal output is absent from durable workspace".to_owned(),
                        )
                    })?;
                    let imported = WorkspaceValueEntry::imported(
                        import_scope.clone(),
                        source.reference().key().clone(),
                        child_value.clone(),
                        source.value().clone(),
                    )
                    .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
                    let parent_value = imported.reference().clone();
                    plan.workspace
                        .push(WorkspaceMutation::PutValue { entry: imported });
                    plan.events.push(RunEventKind::SubworkflowOutputImported {
                        subworkflow: subworkflow.clone(),
                        child_value: child_value.clone(),
                        parent_value: parent_value.clone(),
                    });
                    if publish_on_parent {
                        let published = self.derived_output_entry(
                            parent_execution_view.scope(),
                            source.reference().key().clone(),
                            source.value().clone(),
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
        let history = load_complete_history(self.store.as_ref(), &entry.run)?;
        let projection = RunProjection::replay(&history)?;
        if projection.sequence() != entry.through_sequence
            || projection.lifecycle() != RunLifecycle::Running
        {
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
        let revision = self.current_revision(&projection)?;
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
        let admission_limit = PageSize::new(u32::from(self.config.maximum_tick_items))?;
        let mut usage = self.admission_usage(admission_limit)?;
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
        let invocation = self.next_invocation_id()?;
        let idempotency_key = match contract.idempotency() {
            IdempotencyBehavior::Unsupported => None,
            IdempotencyBehavior::CapabilityScoped | IdempotencyBehavior::ProviderProfileScoped => {
                Some(
                    IdempotencyKey::new(format!("{}-{}", entry.run, execution.execution()))
                        .map_err(|error| RuntimeError::Scheduling(error.to_string()))?,
                )
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
        let request = self.invocation_request(
            &revision,
            &projection,
            node,
            execution.scope(),
            invocation.clone(),
            resolution.snapshot().capability().clone(),
            resolution.snapshot().provider_profile().cloned(),
            idempotency_key.clone(),
        )?;
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
                    idempotency_key,
                },
                RunEventKind::LeaseGranted {
                    lease: lease.clone(),
                    execution: execution.execution().clone(),
                    attempt: attempt.clone(),
                    worker: self.config.worker.clone(),
                    expires_at,
                },
            ],
            ..CommandPlan::default()
        };
        self.commit_internal_plan(
            &entry.run,
            now,
            "schedule_and_lease",
            Some(&attempt),
            schedule,
        )?;
        increment_admission(&mut usage, &admission)?;
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
                let unsafe_outcome = matches!(
                    contract.side_effect(),
                    SideEffectClass::NonIdempotentWrite | SideEffectClass::Unknown
                );
                if unsafe_outcome {
                    let plan = CommandPlan::one(RunEventKind::ExternalOutcomeUncertain {
                        attempt: attempt.clone(),
                        report_sequence: 1,
                        side_effect: contract.side_effect(),
                        reason: Reason::new("executor boundary failed after durable dispatch")?,
                        evidence: Vec::new(),
                    });
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
                let failure = milkdrift_capability::InvocationFailure::new(
                    ErrorClass::Adapter,
                    true,
                    "executor_boundary",
                    error.to_string(),
                    None,
                )
                .map_err(|contract_error| RuntimeError::Scheduling(contract_error.to_string()))?;
                let terminal = InvocationTerminal::new(
                    TerminalStatus::Failure,
                    Vec::new(),
                    Some(failure),
                    None,
                    contract.side_effect(),
                )
                .map_err(|contract_error| RuntimeError::Scheduling(contract_error.to_string()))?;
                self.submit_worker_terminal(&entry.run, now, &attempt, 1, terminal)?;
                return Ok(DispatchOutcome::Completed);
            }
        };

        self.submit_worker_start(&entry.run, now, &lease, &attempt)?;
        for report in reports.reports() {
            self.submit_worker_invocation(&entry.run, now, &attempt, report.clone())?;
        }
        Ok(DispatchOutcome::Completed)
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
                    );
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
        let history = load_complete_history(self.store.as_ref(), run)?;
        let projection = RunProjection::replay(&history)?;
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
        let outcome = self.commit_accepted(&document, receipt, history, projection, plan)?;
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

    fn submit_worker_terminal(
        &self,
        run: &RunId,
        now: TimestampMillis,
        attempt: &AttemptId,
        report_sequence: u64,
        terminal: InvocationTerminal,
    ) -> Result<(), RuntimeError> {
        let command = RunCommandDocument::new(
            self.next_command_id()?,
            run.clone(),
            self.config.internal_actor.clone(),
            self.store.head(run)?,
            now,
            Reason::new("executor boundary supplied a terminal observation")?,
            Vec::new(),
            RunCommand::WorkerReport {
                worker: self.config.worker.clone(),
                report: WorkerReport::Terminal {
                    attempt: attempt.clone(),
                    report_sequence,
                    terminal,
                },
            },
        )?;
        let _ = self.handle_command(&command)?;
        Ok(())
    }

    fn propagate_cancellation(
        &self,
        now: TimestampMillis,
        limit: PageSize,
    ) -> Result<(), RuntimeError> {
        for summary in self.store.nonterminal_runs(limit)? {
            let projection = self.projection(&summary.run)?;
            let run_reason = projection
                .cancellation()
                .map(|cancellation| cancellation.reason().clone());
            let has_branch_cancellation = projection
                .branches()
                .values()
                .any(|branch| branch.state() == BranchState::Cancelling);
            if run_reason.is_none()
                && !has_branch_cancellation
                && projection.reconciliation_cancellations().is_empty()
            {
                continue;
            }
            let revision = self.current_revision(&projection)?;
            let history = self.history(&summary.run)?;
            let already_requested: BTreeSet<_> = history
                .iter()
                .filter_map(|event| match event.kind() {
                    RunEventKind::NodeExecutionCancellationRequested { execution, .. }
                    | RunEventKind::ReconciliationCancellationRequested { execution, .. } => {
                        Some(execution.clone())
                    }
                    _ => None,
                })
                .collect();
            let mut propagation = CommandPlan::default();
            if let Some(reason) = &run_reason {
                for branch in projection
                    .branches()
                    .values()
                    .filter(|branch| branch.state() == BranchState::Active)
                {
                    propagation
                        .events
                        .push(RunEventKind::BranchCancellationRequested {
                            branch: branch.branch().clone(),
                            reason: reason.clone(),
                        });
                }
            }
            for child in projection.subworkflows().values() {
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
            for execution in projection.node_executions().values() {
                let Some(reason) = cancellation_reason_for_execution(
                    &projection,
                    execution.execution(),
                    run_reason.as_ref(),
                ) else {
                    continue;
                };
                match execution.state() {
                    NodeExecutionState::Eligible | NodeExecutionState::RetryPending(_) => {
                        let structured_active = revision
                            .semantic()
                            .nodes()
                            .get(execution.node())
                            .is_some_and(|node| match node.kind() {
                                NodeKind::Subworkflow { .. } | NodeKind::Repeat { .. } => {
                                    projection.subworkflows().values().any(|child| {
                                        child.parent_execution() == execution.execution()
                                            && child.is_active()
                                    })
                                }
                                _ => false,
                            });
                        if structured_active {
                            continue;
                        }
                        for timer in projection.timers().values().filter(|timer| {
                            timer.is_pending()
                                && match timer.purpose() {
                                    TimerPurpose::Wait {
                                        execution: Some(owner),
                                    } => owner == execution.execution(),
                                    TimerPurpose::Retry { attempt } => {
                                        execution.attempts().contains(attempt)
                                    }
                                    TimerPurpose::Wait { execution: None } => false,
                                }
                        }) {
                            propagation.events.push(RunEventKind::TimerCancelled {
                                timer: timer.timer().clone(),
                                reason: reason.clone(),
                            });
                        }
                        if projection
                            .waits()
                            .get(execution.execution())
                            .is_some_and(|wait| wait.is_pending())
                        {
                            propagation.events.push(RunEventKind::WaitCancelled {
                                execution: execution.execution().clone(),
                                reason: reason.clone(),
                            });
                        }
                        // Cancelling a retry timer atomically terminalizes the
                        // reserved attempt and its execution. A first-attempt
                        // eligible execution has no such timer-owned transition.
                        if execution.state() == &NodeExecutionState::Eligible {
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
                        if !already_requested.contains(execution.execution()) {
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
            let projection = self.projection(&summary.run)?;
            let active: Vec<_> = projection
                .attempts()
                .values()
                .filter(|attempt| attempt.is_active())
                .filter(|attempt| attempt.cancellation_acknowledgements().is_empty())
                .filter(|attempt| {
                    cancellation_reason_for_execution(
                        &projection,
                        attempt.execution(),
                        projection.cancellation().map(|value| value.reason()),
                    )
                    .is_some()
                })
                .filter(|attempt| {
                    projection.leases().values().any(|lease| {
                        lease.attempt() == attempt.attempt()
                            && lease.worker() == &self.config.worker
                            && lease.is_active()
                    })
                })
                .filter_map(|attempt| {
                    let reason = cancellation_reason_for_execution(
                        &projection,
                        attempt.execution(),
                        projection.cancellation().map(|value| value.reason()),
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

    fn admission_usage(&self, limit: PageSize) -> Result<AdmissionUsage, RuntimeError> {
        let mut usage = AdmissionUsage::default();
        for summary in self.store.nonterminal_runs(limit)? {
            let projection = self.projection(&summary.run)?;
            for lease in projection
                .leases()
                .values()
                .filter(|lease| lease.is_active())
            {
                let Some(attempt) = projection.attempts().get(lease.attempt()) else {
                    return Err(RuntimeError::InvalidHistory(
                        "active lease has no attempt".to_owned(),
                    ));
                };
                let Some(capability) = attempt.capability() else {
                    return Err(RuntimeError::InvalidHistory(
                        "active lease has no capability resolution".to_owned(),
                    ));
                };
                usage.global = usage.global.checked_add(1).ok_or_else(|| {
                    RuntimeError::Scheduling("global admission count overflow".to_owned())
                })?;
                checked_increment(&mut usage.runs, summary.run.clone())?;
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
                    checked_increment(&mut usage.branches, (summary.run.clone(), branch.clone()))?;
                }
            }
        }
        Ok(usage)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DispatchOutcome {
    Completed,
    Uncertain,
    Deferred,
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

fn runnable_executions(
    projection: &RunProjection,
    revision: Option<&BlueprintRevision>,
) -> BTreeMap<NodeExecutionId, TimestampMillis> {
    if projection.lifecycle() != RunLifecycle::Running {
        return BTreeMap::new();
    }
    let mut result = BTreeMap::new();
    for execution in projection.node_executions().values() {
        if execution_branch_state(projection, execution.execution())
            .is_some_and(|state| state != BranchState::Active)
        {
            continue;
        }
        let is_task = revision
            .and_then(|value| value.semantic().nodes().get(execution.node()))
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
        match execution.state() {
            NodeExecutionState::Eligible => {
                result.insert(execution.execution().clone(), TimestampMillis::new(0));
            }
            NodeExecutionState::RetryPending(attempt) => {
                if projection
                    .attempts()
                    .get(attempt)
                    .is_some_and(|value| value.state() == &AttemptState::ReadyToSchedule)
                {
                    let eligible_at = projection
                        .retries()
                        .values()
                        .find(|retry| retry.next_attempt() == attempt)
                        .map_or(TimestampMillis::new(0), |retry| retry.fire_at());
                    result.insert(execution.execution().clone(), eligible_at);
                }
            }
            NodeExecutionState::Scheduled(_)
            | NodeExecutionState::Running(_)
            | NodeExecutionState::Uncertain(_)
            | NodeExecutionState::CancelledBeforeDispatch
            | NodeExecutionState::RemovedProspectively(_)
            | NodeExecutionState::Terminal(_) => {}
        }
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
            .any(|execution| execution.scope() == target_scope);
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
                    execution.scope() == target_scope
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
        .filter(|edge| edge.kind() == EdgeKind::Data && edge.target_node() == target.id())
        .all(|edge| {
            projection
                .executions_for_node(edge.source_node())
                .filter(|execution| {
                    execution_scope_related(projection, execution.scope(), target_scope)
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
    projection
        .branches()
        .values()
        .find(|branch| branch.children().contains(execution))
        .map(|branch| branch.state())
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
    projection
        .branches()
        .values()
        .find(|branch| {
            branch.state() == BranchState::Cancelling && branch.children().contains(execution)
        })
        .and_then(|branch| branch.cancellation_reason().cloned())
}

fn reconciliation_history(projection: &RunProjection) -> BTreeMap<NodeId, Vec<NodeHistory>> {
    let mut result: BTreeMap<NodeId, Vec<NodeHistory>> = BTreeMap::new();
    for execution in projection.node_executions().values() {
        let attempt = execution
            .attempts()
            .last()
            .and_then(|attempt| projection.attempts().get(attempt));
        let side_effect = attempt
            .and_then(|attempt| attempt.side_effect())
            .map_or(SideEffectClass::None, |classification| {
                classification.side_effect()
            });
        let state = match execution.state() {
            NodeExecutionState::Eligible | NodeExecutionState::RetryPending(_) => {
                HistoricalExecutionState::Pending
            }
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
    result
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
) -> BTreeSet<ArtifactReference> {
    let mut required = BTreeSet::new();
    for mutation in workspace {
        if let WorkspaceMutation::PutValue { entry } = mutation {
            if let Some(artifact) = entry.value().as_artifact() {
                required.insert(artifact.clone());
            }
        }
    }
    for event in events {
        match event.kind() {
            RunEventKind::RunTerminal { artifacts, .. } => {
                required.extend(artifacts.iter().cloned());
            }
            RunEventKind::NodeOutputPublished {
                artifact: Some(artifact),
                ..
            } => {
                required.insert(artifact.clone());
            }
            RunEventKind::ArtifactPublished { metadata } => {
                required.insert(metadata.reference().clone());
                for causal in std::iter::once(metadata.provenance().producer())
                    .chain(metadata.provenance().causes())
                {
                    if let CausalReference::Artifact { reference } = causal {
                        required.insert(reference.clone());
                    }
                }
            }
            _ => {}
        }
    }
    required
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

fn increment_admission(
    usage: &mut AdmissionUsage,
    request: &AdmissionRequest,
) -> Result<(), RuntimeError> {
    usage.global = usage
        .global
        .checked_add(1)
        .ok_or_else(|| RuntimeError::Scheduling("global admission count overflow".to_owned()))?;
    checked_increment(&mut usage.runs, request.run.clone())?;
    checked_increment(&mut usage.capability_classes, request.operation.clone())?;
    if let Some(branch) = &request.branch {
        checked_increment(&mut usage.branches, (request.run.clone(), branch.clone()))?;
    }
    Ok(())
}
