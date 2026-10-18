//! The daemon-owned ordinary task state machine and its durable execution boundary.

mod account;
mod admission;
mod capacity;
mod duration;
mod effects;
mod inputs;
mod model_actions;
mod policy;
mod scheduling;

pub use model_actions::{ModelAdmission, ModelAdmissionStatus};

use crate::{BoundaryClock, ExecutionDispatch, IdGenerator, RuntimeError, TaskExecutor};
use milkdrift_authority::{AuthorityBudget, AuthorityEvaluator, ExecutionAuthorityBasis};
use milkdrift_blueprint::{TaskDefinition, TaskId, TaskInputValue, WorkflowId, WorkflowLimits};
use milkdrift_capability::{InvocationEvent, InvocationTerminal};
use milkdrift_persistence::{
    TimestampMillis, WorkflowCommandAcceptance, WorkflowCommit, WorkflowCommitOutcome,
    WorkflowSnapshot, WorkflowStore,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

/// The externally observable outcome of an ordinary task.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// The task has no accepted start; missing inputs or dependencies may keep it here.
    Queued,
    /// An exact start is frozen and owned by the external execution boundary.
    Running,
    /// Durable successful terminal evidence and immutable result are retained.
    Completed,
    /// A live refusal or durable failure ended the task.
    Failed,
    /// Non-entry or terminal cancellation evidence proves this task stopped.
    Stopped,
    /// External entry may have happened and its outcome remains unresolved.
    Uncertain,
}

impl TaskState {
    /// Whether ordinary dependents accepting any settled outcome may proceed.
    #[must_use]
    pub const fn is_settled(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Stopped)
    }
}

/// Workflow completion requires the explicit finish request and settled obligations.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowState {
    /// No current entry is running and the workflow needs input, continuation, or a finish request.
    Waiting,
    /// At least one task currently has an active start.
    Running,
    /// Explicit finish was requested and all required obligations succeeded.
    Completed,
    /// Failure or unresolved entry prevents the required work from completing.
    Blocked,
    /// Every retained task has proven stopped.
    Stopped,
}

/// Exact instructions, bytes and selected generation accepted before any external entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenTaskStart {
    /// Workflow generation that atomically accepted this immutable start.
    pub accepted_generation: u64,
    /// Exact accepted definition used by this attempt.
    pub definition: TaskDefinition,
    /// Selected bytes and immutable output references, independent of later input edits.
    pub inputs: BTreeMap<String, TaskInputValue>,
    /// Exact capability generation, authority basis, and invocation identity.
    pub dispatch: ExecutionDispatch,
    /// Durable final-entry intent; absence is the only local recovery evidence of non-entry.
    pub entered: bool,
    /// Time and enforceable duration reserved at final external entry.
    /// Unresolved work keeps this reservation until terminal evidence proves settlement.
    pub execution_duration: Option<TaskExecutionDuration>,
    /// Bounded append-only adapter observations, including exact sequence replay evidence.
    pub reports: Vec<InvocationEvent>,
}

/// Durable duration evidence owned by one frozen execution and the workflow account.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskExecutionDuration {
    /// Final entry time, excluding local preparation. Uncertain preparation uses the earlier
    /// accepted start conservatively because absence of external entry could not be established.
    pub started_at: TimestampMillis,
    /// Last timestamp charged to the workflow's cumulative elapsed-time ledger.
    pub accounted_at: TimestampMillis,
    /// The adapter's enforceable maximum, reserved before entry; zero is explicit non-applicability.
    pub maximum_ms: u64,
}

/// Immutable terminal result. Outputs are content-addressed capability artifact references.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskResult {
    /// Original provider-neutral terminal observation, never rewritten by a correction.
    pub terminal: InvocationTerminal,
    /// Named content references published before terminal acceptance.
    pub outputs: BTreeMap<String, milkdrift_capability::ArtifactReference>,
    /// Durable acceptance time of the result.
    pub recorded_at: TimestampMillis,
}

/// Definition versions can change only before the first accepted start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRecord {
    /// Daemon-assigned task identity, stable across future definition edits.
    pub id: TaskId,
    /// Latest accepted future definition version.
    pub version: u64,
    /// Latest definition; a started task additionally retains the exact frozen copy.
    pub definition: TaskDefinition,
    /// Prior accepted future definitions, in increasing version order.
    pub previous_definitions: Vec<TaskDefinition>,
    /// Removal closes the obligation while retaining its accepted definition and identity.
    pub removed: bool,
    /// Current state derived from accepted starts, requests and reports.
    pub state: TaskState,
    /// Bounded missing-input, progress, refusal, or uncertainty explanation.
    pub reason: Option<String>,
    /// Original task admission time.
    pub created_at: TimestampMillis,
    /// Actual accepted start time; queued work has none.
    pub started_at: Option<TimestampMillis>,
    /// Time durable terminal evidence settled the task; uncertainty has none.
    pub ended_at: Option<TimestampMillis>,
    /// An accepted stop request, independent of terminal stop evidence.
    pub stop_requested: bool,
    /// The exact cancellation request was acknowledged; this does not imply settlement.
    pub cancellation_sent: bool,
    /// Accepted definition, bytes and external-entry evidence.
    pub frozen: Option<FrozenTaskStart>,
    /// Immutable original terminal result when one was observed.
    pub result: Option<TaskResult>,
    /// Successful explicitly linked work satisfying this task's failed obligation.
    pub adopted_correction: Option<TaskId>,
}

/// Runtime's canonical task projection, persisted atomically with public request receipts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRecord {
    /// Last accepted edit or task start. Progress reports do not change an author's snapshot.
    pub decision_generation: u64,
    /// Durable model response admissions, keyed by their unique source task.
    pub admissions: BTreeMap<TaskId, ModelAdmission>,
    /// Accepted model batches, including automatic feedback task batches.
    pub admitted_model_batches: u32,
    /// Consecutive rejected or nonprogressing model responses.
    pub rejected_responses: u32,
    /// Runtime document schema, interpreted independently of physical storage version.
    pub schema_version: u32,
    /// Owning daemon's workflow identity.
    pub id: WorkflowId,
    /// Searchable human-facing title.
    pub title: String,
    /// Bounded human-facing purpose.
    pub description: String,
    /// Compare-and-swap generation including internal starts and observations.
    pub generation: u64,
    /// Authenticated original owner; current access still requires a grant check.
    pub owner_actor: String,
    /// Original workflow acceptance time.
    pub created_at: TimestampMillis,
    /// Most recent accepted transition time.
    pub updated_at: TimestampMillis,
    /// Current aggregate outcome, including explicit completion intent.
    pub state: WorkflowState,
    /// Inputs available to future starts; changing them never changes frozen inputs.
    pub inputs: BTreeMap<String, TaskInputValue>,
    /// Cumulative workflow policy accepted at creation.
    pub limits: WorkflowLimits,
    /// Explicit initial or later request to finish after all obligations settle.
    pub finish_when_settled: bool,
    /// Actionable question preventing new starts until authorized continuation.
    pub wait_question: Option<String>,
    /// Rejected model source that owns this diagnostic wait; explicit and policy waits have none.
    pub model_admission_wait: Option<TaskId>,
    /// Task facts, including removed obligations and failed attempts.
    pub tasks: BTreeMap<TaskId, TaskRecord>,
    /// Frozen authority lineage rechecked against revocation before each entry.
    pub execution_authority: ExecutionAuthorityBasis,
    /// Cumulative tasks ever admitted, unaffected by removal or restart.
    pub admitted_tasks: u32,
    /// Cumulative elapsed execution, including every concurrently live or unresolved start.
    pub active_execution_ms: u64,
    /// Accepted grant ceilings. New entries never reset these cumulative limits.
    pub allowance: AuthorityBudget,
    /// Conservative resource maxima already committed for entered attempts.
    pub committed_allowance: AuthorityBudget,
    /// A workflow cannot combine incomparable currency amounts in one spending account.
    pub spending_currency: Option<String>,
}

/// One ordinary scheduler. The daemon supplies bounded workers; this service owns no thread.
pub struct TaskRuntime {
    store: Arc<dyn WorkflowStore>,
    executor: Arc<dyn TaskExecutor>,
    authority: Arc<dyn AuthorityEvaluator>,
    clock: Arc<dyn BoundaryClock>,
    ids: Arc<dyn IdGenerator>,
    gate: Mutex<()>,
    cursor: Mutex<Option<String>>,
    cancellation_cursor: Mutex<Option<String>>,
    maximum_active_tasks: u32,
    active_tasks: Mutex<Option<u64>>,
    executing: Mutex<BTreeSet<milkdrift_capability::InvocationId>>,
    admission_closed: std::sync::atomic::AtomicBool,
}

impl TaskRuntime {
    /// Connects one durable task owner to explicit execution, authority, clock and ID boundaries.
    #[must_use]
    pub fn new(
        store: Arc<dyn WorkflowStore>,
        executor: Arc<dyn TaskExecutor>,
        authority: Arc<dyn AuthorityEvaluator>,
        clock: Arc<dyn BoundaryClock>,
        ids: Arc<dyn IdGenerator>,
    ) -> Self {
        Self {
            store,
            executor,
            authority,
            clock,
            ids,
            gate: Mutex::new(()),
            cursor: Mutex::new(None),
            cancellation_cursor: Mutex::new(None),
            maximum_active_tasks: 32,
            active_tasks: Mutex::new(None),
            executing: Mutex::new(BTreeSet::new()),
            admission_closed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Bounds accepted unfinished starts across all workflows on this daemon.
    /// Existing retained starts keep their ownership when reopening with a tighter bound.
    ///
    /// # Errors
    /// Refuses zero or a bound above 65,536 tasks.
    pub fn with_maximum_active_tasks(mut self, maximum: u32) -> Result<Self, RuntimeError> {
        if maximum == 0 || maximum > 65_536 {
            return Err(RuntimeError::Scheduling(
                "global task concurrency must be 1..=65,536".into(),
            ));
        }
        self.maximum_active_tasks = maximum;
        Ok(self)
    }

    fn active_task_count(&self) -> Result<u64, RuntimeError> {
        let mut active = self
            .active_tasks
            .lock()
            .map_err(|_| RuntimeError::Scheduling("task occupancy lock unavailable".into()))?;
        if let Some(count) = *active {
            return Ok(count);
        }
        let mut count = 0_u64;
        let mut cursor = None;
        loop {
            let page = self.store.workflows(cursor.as_deref(), 200)?;
            for snapshot in page.items {
                let record = self.require_workflow(&snapshot.workflow)?;
                count = count.checked_add(active_starts(&record)?).ok_or_else(|| {
                    RuntimeError::Scheduling("global task occupancy overflow".into())
                })?;
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        *active = Some(count);
        Ok(count)
    }

    /// Closes new admission and scheduling while accepted reports and stop requests remain writable.
    /// Worker shutdown separately decides whether already frozen starts drain or remain for recovery.
    pub fn begin_shutdown(&self) {
        self.admission_closed
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn is_closed(&self) -> bool {
        self.admission_closed
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Reads current task state and verifies the discovery envelope matches its document.
    ///
    /// # Errors
    /// Returns storage, decoding, or mismatched durable-envelope errors.
    pub fn workflow(&self, id: &WorkflowId) -> Result<Option<WorkflowRecord>, RuntimeError> {
        self.store
            .workflow(id)?
            .map(|snapshot| {
                let record: WorkflowRecord = serde_json::from_value(snapshot.document)?;
                duration::read_version(&record)?;
                if record.id != snapshot.workflow
                    || record.generation != snapshot.generation
                    || record.owner_actor != snapshot.owner_actor
                    || record.title != snapshot.title
                    || record.description != snapshot.description
                    || record.updated_at != snapshot.updated_at
                    || entered_invocations(&record) != snapshot.entered_invocations
                    || managed_executions(&record) != snapshot.managed_executions
                {
                    return Err(RuntimeError::InvalidHistory(
                        "task workflow envelope differs from its durable document".into(),
                    ));
                }
                Ok(record)
            })
            .transpose()
    }

    fn require_workflow(&self, id: &WorkflowId) -> Result<WorkflowRecord, RuntimeError> {
        self.workflow(id)?
            .ok_or_else(|| RuntimeError::InvalidCommand("workflow does not exist".into()))
    }

    fn save(
        &self,
        record: &WorkflowRecord,
        expected_generation: Option<u64>,
        command: Option<WorkflowCommandAcceptance>,
    ) -> Result<WorkflowCommitOutcome, RuntimeError> {
        let pending = record
            .tasks
            .values()
            .filter(|task| !task.state.is_settled())
            .count();
        let reserved_bytes = capacity::remaining(record)?;
        let document = serde_json::to_value(record)?;
        let document_bytes = u64::try_from(serde_json::to_vec(&document)?.len())
            .map_err(|e| RuntimeError::InvalidTransition(e.to_string()))?;
        if document_bytes
            .checked_add(reserved_bytes)
            .is_none_or(|bytes| bytes > milkdrift_persistence::MAX_WORKFLOW_DOCUMENT_BYTES as u64)
        {
            return Err(RuntimeError::InvalidTransition("workflow document cannot reserve its remaining start, observation and settlement records".into()));
        }
        let reserved_receipts = u32::try_from(
            record
                .tasks
                .values()
                .filter(|task| !task.state.is_settled() && !task.stop_requested)
                .count(),
        )
        .map_err(|_| RuntimeError::InvalidTransition("task control reservation overflow".into()))?;
        // This count is a cache of committed frozen starts, rebuilt from the same store on reopen.
        // Keeping it under the transition gate avoids rescanning every workflow on each worker poll.
        let mut active = self
            .active_tasks
            .lock()
            .map_err(|_| RuntimeError::Scheduling("task occupancy lock unavailable".into()))?;
        let next_active = (*active)
            .map(|count| {
                let prior = self
                    .workflow(&record.id)?
                    .map(|value| active_starts(&value))
                    .transpose()?
                    .unwrap_or(0);
                count
                    .checked_sub(prior)
                    .and_then(|count| count.checked_add(active_starts(record).ok()?))
                    .ok_or_else(|| {
                        RuntimeError::Scheduling(
                            "global task occupancy contradicts durable starts".into(),
                        )
                    })
            })
            .transpose()?;
        let outcome = self.store.commit_workflow(WorkflowCommit {
            expected_generation,
            snapshot: WorkflowSnapshot {
                workflow: record.id.clone(),
                generation: record.generation,
                title: record.title.clone(),
                description: record.description.clone(),
                owner_actor: record.owner_actor.clone(),
                updated_at: record.updated_at,
                document,
                reserved_bytes,
                reserved_receipts,
                entered_invocations: entered_invocations(record),
                managed_executions: managed_executions(record),
                artifact_capacity: if pending == 0 {
                    0
                } else {
                    u64::from(record.admitted_tasks)
                        .checked_mul(TASK_OUTPUT_BYTES)
                        .ok_or_else(|| {
                            RuntimeError::InvalidTransition("artifact reservation overflow".into())
                        })?
                },
            },
            command,
        })?;
        if matches!(outcome, WorkflowCommitOutcome::Applied(_)) {
            *active = next_active;
        }
        Ok(outcome)
    }

    fn advance(&self, record: &mut WorkflowRecord) -> Result<u64, RuntimeError> {
        let prior = record.generation;
        record.generation = prior.checked_add(1).ok_or_else(|| {
            RuntimeError::InvalidTransition("workflow generation exhausted".into())
        })?;
        record.updated_at = self.clock.now()?;
        policy::request_limit_stop(record)?;
        update_workflow_state(record);
        Ok(prior)
    }
}

fn active_starts(record: &WorkflowRecord) -> Result<u64, RuntimeError> {
    u64::try_from(
        record
            .tasks
            .values()
            .filter(|task| task.frozen.is_some() && !task.state.is_settled())
            .count(),
    )
    .map_err(|error| RuntimeError::Scheduling(error.to_string()))
}

/// Enforceable maximum aggregate output bytes of one ordinary task.
pub(crate) const TASK_OUTPUT_BYTES: u64 = milkdrift_blueprint::MAX_TASK_OUTPUT_BYTES;

fn managed_executions(
    record: &WorkflowRecord,
) -> Vec<milkdrift_persistence::WorkflowManagedExecution> {
    record
        .tasks
        .values()
        .filter_map(|task| {
            let frozen = task.frozen.as_ref()?;
            let selection = frozen.dispatch.resolution();
            if !selection
                .descriptor_extensions()
                .keys()
                .any(|key| key.as_str() == milkdrift_capability::managed::MANAGED_BINDING_EXTENSION)
            {
                return None;
            }
            Some(milkdrift_persistence::WorkflowManagedExecution {
                task: task.id.clone(),
                invocation: frozen.dispatch.request().invocation().clone(),
                accepted_generation: frozen.accepted_generation,
                selection: selection.clone(),
                entered: frozen.entered,
                settled: task.state.is_settled(),
            })
        })
        .collect()
}

fn entered_invocations(record: &WorkflowRecord) -> Vec<milkdrift_capability::InvocationId> {
    let mut invocations: Vec<_> = record
        .tasks
        .values()
        .filter_map(|task| task.frozen.as_ref())
        .filter(|start| start.entered)
        .map(|start| start.dispatch.request().invocation().clone())
        .collect();
    invocations.sort();
    invocations.dedup();
    invocations
}

fn output_contract_failure(
    definition: &milkdrift_blueprint::TaskDefinition,
    outputs: &BTreeMap<String, milkdrift_capability::ArtifactReference>,
) -> Option<String> {
    for (name, declaration) in &definition.outputs {
        let Some(reference) = outputs.get(name) else {
            if declaration.required {
                return Some(format!("required output {name} was not published"));
            }
            continue;
        };
        if let Err(reason) = declaration.validate_reference(reference) {
            return Some(format!("output {name}: {reason}"));
        }
    }
    None
}

fn update_workflow_state(workflow: &mut WorkflowRecord) {
    let active = workflow
        .tasks
        .values()
        .any(|task| task.state == TaskState::Running);
    let uncertain = workflow
        .tasks
        .values()
        .any(|task| task.state == TaskState::Uncertain);
    let unsettled = workflow.tasks.values().any(|task| !task.state.is_settled());
    let obligations_met = workflow
        .tasks
        .values()
        .filter(|task| task.definition.required && !task.removed)
        .all(|task| {
            task.state == TaskState::Completed
                || task
                    .adopted_correction
                    .as_ref()
                    .and_then(|id| workflow.tasks.get(id))
                    .is_some_and(|correction| correction.state == TaskState::Completed)
        });
    workflow.state = if active {
        WorkflowState::Running
    } else if !unsettled
        && obligations_met
        && workflow.finish_when_settled
        && workflow.wait_question.is_none()
        && !workflow
            .admissions
            .values()
            .any(|admission| admission.status.blocks_completion())
    {
        WorkflowState::Completed
    } else if uncertain
        || (!obligations_met
            && workflow
                .tasks
                .values()
                .any(|task| task.state == TaskState::Failed))
    {
        WorkflowState::Blocked
    } else if !workflow.tasks.is_empty()
        && workflow
            .tasks
            .values()
            .all(|task| task.state == TaskState::Stopped)
    {
        WorkflowState::Stopped
    } else {
        WorkflowState::Waiting
    };
}
