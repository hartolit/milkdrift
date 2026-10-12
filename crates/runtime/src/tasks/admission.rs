use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::{AuthorityDecisionSnapshot, ExecutionAuthorityBasis};
use milkdrift_blueprint::{
    MAX_TASK_BATCH_ACTIONS, TaskBatch, TaskChange, TaskDefinition, TaskId, TaskInputBinding,
    WorkflowCreate, WorkflowId, validate_task_set,
};
use milkdrift_persistence::{
    WorkflowCommandAcceptance, WorkflowCommandReceipt, WorkflowCommitOutcome,
};
use milkdrift_workspace::RunId;

use super::{TaskRecord, TaskRuntime, TaskState, WorkflowRecord, WorkflowState};
use crate::RuntimeError;

impl TaskRuntime {
    /// Stable daemon namespace used by authorization before admission; replay allocates no IDs.
    ///
    /// # Errors
    /// Returns canonical encoding or identity construction failures.
    pub fn workflow_identity(actor: &str, key: &str) -> Result<WorkflowId, RuntimeError> {
        let bytes = serde_json::to_vec(&(actor, key))?;
        WorkflowId::new(format!("workflow_{}", blake3::hash(&bytes))).map_err(invalid)
    }

    /// Accepts complete initial work and its exact alias map in one durable transaction.
    ///
    /// # Errors
    /// Refuses malformed definitions, owner mismatch, conflicting request reuse and capacity.
    pub fn create(
        &self,
        owner_actor: &str,
        key: &str,
        request: WorkflowCreate,
        decision: AuthorityDecisionSnapshot,
    ) -> Result<WorkflowCommandReceipt, RuntimeError> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| invalid("task admission lock unavailable"))?;
        let canonical_request = serde_json::to_vec(
            &serde_json::json!({"operation":"create_workflow","request":request}),
        )?;
        if let Some(receipt) = self.replay(owner_actor, key, &canonical_request)? {
            return Ok(receipt);
        }
        if request.title.trim().is_empty()
            || request.title.len() > 256
            || request.description.len() > 4096
            || request.tasks.len() > MAX_TASK_BATCH_ACTIONS
            || request.tasks.is_empty()
        {
            return Err(invalid(
                "workflow needs a bounded title and 1..=32 initial tasks",
            ));
        }
        if decision.request().actor.as_str() != owner_actor {
            return Err(invalid(
                "workflow owner differs from authenticated start decision",
            ));
        }
        let id = Self::workflow_identity(owner_actor, key)?;
        let revision = revision_identity(&canonical_request)?;
        let execution_authority = ExecutionAuthorityBasis::from_start_decision(
            &decision,
            id.clone(),
            RunId::new(id.as_str()).map_err(invalid)?,
            revision,
        )?;
        let now = self.clock.now()?;
        let aliases = request
            .tasks
            .iter()
            .map(|task| {
                Ok((
                    task.alias.clone(),
                    TaskId::new(self.ids.next("task")?).map_err(invalid)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, RuntimeError>>()?;
        if aliases.len() != request.tasks.len() {
            return Err(invalid("new task aliases must be unique"));
        }
        let mut tasks = BTreeMap::new();
        for task in request.tasks {
            let id = aliases
                .get(&task.alias)
                .ok_or_else(|| invalid("task alias missing"))?
                .clone();
            let definition = remap_definition(task.definition, &aliases);
            tasks.insert(
                id.clone(),
                TaskRecord {
                    id,
                    version: 1,
                    definition,
                    previous_definitions: Vec::new(),
                    removed: false,
                    state: TaskState::Queued,
                    reason: None,
                    created_at: now,
                    started_at: None,
                    ended_at: None,
                    stop_requested: false,
                    cancellation_sent: false,
                    frozen: None,
                    result: None,
                    adopted_correction: None,
                },
            );
        }
        validate_task_set(&definitions(&tasks), &request.limits).map_err(invalid)?;
        for value in request.inputs.values() {
            value.validate().map_err(invalid)?;
        }
        let record = WorkflowRecord {
            schema_version: 1,
            id: id.clone(),
            title: request.title,
            description: request.description,
            generation: 1,
            owner_actor: owner_actor.to_owned(),
            created_at: now,
            updated_at: now,
            state: WorkflowState::Waiting,
            inputs: request.inputs,
            limits: request.limits,
            finish_when_settled: request.finish_when_settled,
            wait_question: None,
            admitted_tasks: u32::try_from(tasks.len()).map_err(invalid)?,
            tasks,
            execution_authority,
            active_execution_ms: 0,
            allowance: decision.evaluated_budget(),
            committed_allowance: milkdrift_authority::AuthorityBudget::default(),
            spending_currency: None,
        };
        let acceptance = WorkflowCommandAcceptance {
            actor: owner_actor.to_owned(),
            key: key.to_owned(),
            canonical_request,
            result: serde_json::json!({"workflow_id":id,"generation":1,"aliases":aliases}),
        };
        receipt(self.save(&record, None, Some(acceptance))?)
    }

    /// Applies a checked future-work batch. Its caller must supply scoped edit authority.
    ///
    /// # Errors
    /// Refuses stale generations/versions, invalid references, started-task edits and storage failures.
    pub fn edit(
        &self,
        owner_actor: &str,
        key: &str,
        workflow: &WorkflowId,
        batch: TaskBatch,
    ) -> Result<WorkflowCommandReceipt, RuntimeError> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| invalid("task admission lock unavailable"))?;
        let canonical_request = serde_json::to_vec(
            &serde_json::json!({"operation":"edit_workflow","workflow":workflow,"batch":batch}),
        )?;
        if let Some(receipt) = self.replay(owner_actor, key, &canonical_request)? {
            return Ok(receipt);
        }
        let mut record = self.require_workflow(workflow)?;
        if batch.expected_generation != record.generation {
            return Err(RuntimeError::InvalidTransition(format!(
                "stale workflow generation: expected {}, actual {}",
                batch.expected_generation, record.generation
            )));
        }
        if batch.changes.is_empty() || batch.changes.len() > MAX_TASK_BATCH_ACTIONS {
            return Err(invalid("task batch must contain 1..=32 changes"));
        }
        let mut aliases = BTreeMap::new();
        for change in &batch.changes {
            if let TaskChange::Add { alias, .. } = change {
                if aliases
                    .insert(
                        alias.clone(),
                        TaskId::new(self.ids.next("task")?).map_err(invalid)?,
                    )
                    .is_some()
                    || record.tasks.contains_key(alias)
                {
                    return Err(invalid(
                        "new aliases must be unique and cannot name existing tasks",
                    ));
                }
            }
        }
        let now = self.clock.now()?;
        for change in batch.changes {
            match change {
                TaskChange::Add { alias, definition } => {
                    let id = aliases
                        .get(&alias)
                        .ok_or_else(|| invalid("missing allocated alias"))?
                        .clone();
                    record.admitted_tasks = record
                        .admitted_tasks
                        .checked_add(1)
                        .ok_or_else(|| invalid("admitted task accounting overflow"))?;
                    if record.admitted_tasks > record.limits.tasks {
                        return Err(invalid("cumulative workflow task capacity exhausted"));
                    }
                    record.tasks.insert(
                        id.clone(),
                        TaskRecord {
                            id,
                            version: 1,
                            definition: remap_definition(definition, &aliases),
                            previous_definitions: Vec::new(),
                            removed: false,
                            state: TaskState::Queued,
                            reason: None,
                            created_at: now,
                            started_at: None,
                            ended_at: None,
                            stop_requested: false,
                            cancellation_sent: false,
                            frozen: None,
                            result: None,
                            adopted_correction: None,
                        },
                    );
                }
                TaskChange::Revise {
                    task,
                    expected_version,
                    definition,
                } => {
                    let value = future_task(&mut record, &task, expected_version)?;
                    if value.previous_definitions.len() >= 128 {
                        return Err(invalid(
                            "retained task definition version capacity exhausted",
                        ));
                    }
                    value.previous_definitions.push(value.definition.clone());
                    value.definition = remap_definition(definition, &aliases);
                    value.version = value
                        .version
                        .checked_add(1)
                        .ok_or_else(|| invalid("task version exhausted"))?;
                    value.reason = None;
                }
                TaskChange::Remove {
                    task,
                    expected_version,
                } => {
                    let task = future_task(&mut record, &task, expected_version)?;
                    task.removed = true;
                    task.state = TaskState::Stopped;
                    task.ended_at = Some(now);
                    task.reason = Some("unstarted obligation removed by an authorized edit".into());
                }
                TaskChange::BindInput { name, value } => {
                    value.validate().map_err(invalid)?;
                    if name.is_empty()
                        || name.len() > 128
                        || record.inputs.len() >= 256 && !record.inputs.contains_key(&name)
                    {
                        return Err(invalid("workflow input name or count exceeds its bound"));
                    }
                    record.inputs.insert(name, value);
                    record.wait_question = None;
                }
                TaskChange::Stop { task } => {
                    let task = record
                        .tasks
                        .get_mut(&task)
                        .ok_or_else(|| invalid("stop task does not exist"))?;
                    if !task.state.is_settled() {
                        task.stop_requested = true;
                        if task.frozen.is_none() {
                            task.state = TaskState::Stopped;
                            task.ended_at = Some(now);
                            task.reason = Some("stopped before task start".into());
                        }
                    }
                }
                TaskChange::AdoptCorrection { task, correction } => {
                    let correction = aliases.get(&correction).unwrap_or(&correction);
                    let replacement = record
                        .tasks
                        .get(correction)
                        .ok_or_else(|| invalid("corrective task does not exist"))?;
                    if replacement.state != TaskState::Completed
                        || replacement.definition.corrective_of.as_ref() != Some(&task)
                    {
                        return Err(invalid(
                            "adoption requires a successful explicitly linked correction",
                        ));
                    }
                    let original = record
                        .tasks
                        .get_mut(&task)
                        .ok_or_else(|| invalid("corrected task does not exist"))?;
                    if !matches!(original.state, TaskState::Failed | TaskState::Stopped) {
                        return Err(invalid(
                            "only settled failed/stopped work can adopt correction",
                        ));
                    }
                    original.adopted_correction = Some(correction.clone());
                }
                TaskChange::Continue => record.wait_question = None,
                TaskChange::Wait { question } => {
                    if question.trim().is_empty() || question.len() > 4096 {
                        return Err(invalid("wait question is empty or oversized"));
                    }
                    record.wait_question = Some(question);
                }
                TaskChange::Finish => record.finish_when_settled = true,
            }
        }
        validate_task_set(&definitions(&record.tasks), &record.limits).map_err(invalid)?;
        let expected = self.advance(&mut record)?;
        let acceptance = WorkflowCommandAcceptance {
            actor: owner_actor.to_owned(),
            key: key.to_owned(),
            canonical_request,
            result: serde_json::json!({"workflow_id":workflow,"generation":record.generation,"aliases":aliases}),
        };
        receipt(self.save(&record, Some(expected), Some(acceptance))?)
    }

    fn replay(
        &self,
        actor: &str,
        key: &str,
        request: &[u8],
    ) -> Result<Option<WorkflowCommandReceipt>, RuntimeError> {
        if let Some(receipt) = self.store.workflow_receipt(actor, key)? {
            if receipt.acceptance.canonical_request != request {
                return Err(invalid(
                    "request idempotency key was reused with a different canonical request",
                ));
            }
            return Ok(Some(receipt));
        }
        Ok(None)
    }
}

fn receipt(outcome: WorkflowCommitOutcome) -> Result<WorkflowCommandReceipt, RuntimeError> {
    match outcome {
        WorkflowCommitOutcome::Applied(Some(receipt)) | WorkflowCommitOutcome::Replay(receipt) => {
            Ok(receipt)
        }
        WorkflowCommitOutcome::Applied(None) => Err(RuntimeError::InvalidHistory(
            "external task acceptance omitted its receipt".into(),
        )),
    }
}

fn future_task<'a>(
    record: &'a mut WorkflowRecord,
    task: &TaskId,
    expected_version: u64,
) -> Result<&'a mut TaskRecord, RuntimeError> {
    let task = record
        .tasks
        .get_mut(task)
        .ok_or_else(|| invalid("task does not exist"))?;
    if task.version != expected_version || task.frozen.is_some() || task.state != TaskState::Queued
    {
        return Err(RuntimeError::InvalidTransition(
            "task version changed or task already started".into(),
        ));
    }
    Ok(task)
}

fn definitions(tasks: &BTreeMap<TaskId, TaskRecord>) -> BTreeMap<TaskId, TaskDefinition> {
    tasks
        .iter()
        .filter(|(_, task)| !task.removed)
        .map(|(id, task)| (id.clone(), task.definition.clone()))
        .collect()
}

fn remap_definition(
    mut definition: TaskDefinition,
    aliases: &BTreeMap<TaskId, TaskId>,
) -> TaskDefinition {
    definition.dependencies = definition
        .dependencies
        .iter()
        .map(|id| aliases.get(id).unwrap_or(id).clone())
        .collect::<BTreeSet<_>>();
    for binding in definition.inputs.values_mut() {
        if let TaskInputBinding::TaskOutput { task, .. }
        | TaskInputBinding::SettledOutcome { task } = binding
        {
            *task = aliases.get(task).unwrap_or(task).clone();
        }
    }
    if let Some(task) = &mut definition.corrective_of {
        *task = aliases.get(task).unwrap_or(task).clone();
    }
    definition
}

pub(super) fn revision_identity(
    bytes: &[u8],
) -> Result<milkdrift_blueprint::RevisionId, RuntimeError> {
    Ok(serde_json::from_value(serde_json::Value::String(format!(
        "rev_{}",
        blake3::hash(bytes)
    )))?)
}
fn invalid(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::InvalidCommand(error.to_string())
}
