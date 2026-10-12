//! The daemon-owned ordinary task state machine and its durable execution boundary.

mod account;
mod admission;
mod effects;
mod scheduling;

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
    collections::BTreeMap,
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
    /// Exact accepted definition used by this attempt.
    pub definition: TaskDefinition,
    /// Selected bytes and immutable output references, independent of later input edits.
    pub inputs: BTreeMap<String, TaskInputValue>,
    /// Exact capability generation, authority basis, and invocation identity.
    pub dispatch: ExecutionDispatch,
    /// Durable final-entry intent; absence is the only local recovery evidence of non-entry.
    pub entered: bool,
    /// Bounded append-only adapter observations, including exact sequence replay evidence.
    pub reports: Vec<InvocationEvent>,
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
    /// Task facts, including removed obligations and failed attempts.
    pub tasks: BTreeMap<TaskId, TaskRecord>,
    /// Frozen authority lineage rechecked against revocation before each entry.
    pub execution_authority: ExecutionAuthorityBasis,
    /// Cumulative tasks ever admitted, unaffected by removal or restart.
    pub admitted_tasks: u32,
    /// Sum of observed elapsed task execution durations.
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
        }
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
                if record.schema_version != 1
                    || record.id != snapshot.workflow
                    || record.generation != snapshot.generation
                    || record.owner_actor != snapshot.owner_actor
                    || entered_invocations(&record) != snapshot.entered_invocations
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
        let reserved_bytes = u64::try_from(pending)
            .ok()
            .and_then(|count| count.checked_mul(2 * 1024 * 1024))
            .ok_or_else(|| {
                RuntimeError::InvalidTransition("task settlement reservation overflow".into())
            })?;
        let reserved_receipts = u32::try_from(pending).map_err(|_| {
            RuntimeError::InvalidTransition("task control reservation overflow".into())
        })?;
        Ok(self.store.commit_workflow(WorkflowCommit {
            expected_generation,
            snapshot: WorkflowSnapshot {
                workflow: record.id.clone(),
                generation: record.generation,
                title: record.title.clone(),
                description: record.description.clone(),
                owner_actor: record.owner_actor.clone(),
                updated_at: record.updated_at,
                document: serde_json::to_value(record)?,
                reserved_bytes,
                reserved_receipts,
                entered_invocations: entered_invocations(record),
            },
            command,
        })?)
    }

    fn advance(&self, record: &mut WorkflowRecord) -> Result<u64, RuntimeError> {
        let prior = record.generation;
        record.generation = prior.checked_add(1).ok_or_else(|| {
            RuntimeError::InvalidTransition("workflow generation exhausted".into())
        })?;
        record.updated_at = self.clock.now()?;
        update_workflow_state(record);
        Ok(prior)
    }
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
    } else if !unsettled && obligations_met && workflow.finish_when_settled {
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
