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
        self.create_request(owner_actor, key, request, decision, None)
    }

    /// Preserves the exact canonical public envelope before transport defaults or uploads are resolved.
    /// The authenticated control owner supplies both representations of the same request.
    ///
    /// # Errors
    /// Refuses noncanonical/oversized transport JSON and the ordinary admission failures.
    pub fn create_exact(
        &self,
        owner_actor: &str,
        key: &str,
        request: WorkflowCreate,
        decision: AuthorityDecisionSnapshot,
        canonical_transport: &[u8],
    ) -> Result<WorkflowCommandReceipt, RuntimeError> {
        self.create_request(
            owner_actor,
            key,
            request,
            decision,
            Some(canonical_transport),
        )
    }

    fn create_request(
        &self,
        owner_actor: &str,
        key: &str,
        request: WorkflowCreate,
        decision: AuthorityDecisionSnapshot,
        transport: Option<&[u8]>,
    ) -> Result<WorkflowCommandReceipt, RuntimeError> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| invalid("task admission lock unavailable"))?;
        let request_identity = transport
            .map(canonical_transport_value)
            .transpose()?
            .unwrap_or(serde_json::to_value(&request)?);
        let canonical_request = serde_json::to_vec(
            &serde_json::json!({"operation":"create_workflow","request":request_identity}),
        )?;
        if let Some(receipt) = self.replay(owner_actor, key, &canonical_request)? {
            return Ok(receipt);
        }
        if self.is_closed() {
            return Err(invalid("task admission is closed"));
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
        for task in tasks.values() {
            task.definition
                .validate_supplied_inputs(&request.inputs)
                .map_err(invalid)?;
        }
        let record = WorkflowRecord {
            decision_generation: 1,
            admissions: BTreeMap::new(),
            admitted_model_batches: 0,
            rejected_responses: 0,
            schema_version: super::duration::SCHEMA_VERSION,
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
            model_admission_wait: None,
            admitted_tasks: u32::try_from(tasks.len()).map_err(invalid)?,
            tasks,
            execution_authority,
            active_execution_ms: 0,
            allowance: decision.evaluated_budget(),
            committed_allowance: milkdrift_authority::AuthorityBudget::default(),
            spending_currency: None,
        };
        let acceptance = WorkflowCommandAcceptance {
            reserved_action: None,
            actor: owner_actor.to_owned(),
            key: key.to_owned(),
            canonical_request,
            result: serde_json::json!({"workflow_id":id,"generation":1,"aliases":aliases}),
        };
        for task in record.tasks.values() {
            self.validate_future_definition(&record, &task.definition)?;
        }
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
        self.edit_request(owner_actor, key, workflow, batch, None, None)
    }

    /// Binds replay to the original canonical public edit, including explicit defaults.
    ///
    /// # Errors
    /// Refuses noncanonical/oversized transport JSON and the ordinary edit failures.
    pub fn edit_exact(
        &self,
        owner_actor: &str,
        key: &str,
        workflow: &WorkflowId,
        batch: TaskBatch,
        canonical_transport: &[u8],
    ) -> Result<WorkflowCommandReceipt, RuntimeError> {
        self.edit_request(
            owner_actor,
            key,
            workflow,
            batch,
            Some(canonical_transport),
            None,
        )
    }

    pub(super) fn edit_request(
        &self,
        owner_actor: &str,
        key: &str,
        workflow: &WorkflowId,
        batch: TaskBatch,
        transport: Option<&[u8]>,
        model_source: Option<(&TaskId, bool)>,
    ) -> Result<WorkflowCommandReceipt, RuntimeError> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| invalid("task admission lock unavailable"))?;
        let request_identity = transport
            .map(canonical_transport_value)
            .transpose()?
            .unwrap_or(serde_json::to_value(&batch)?);
        let canonical_request = serde_json::to_vec(
            &serde_json::json!({"operation":"edit_workflow","workflow":workflow,"batch":request_identity}),
        )?;
        if let Some(receipt) = self.replay(owner_actor, key, &canonical_request)? {
            return Ok(receipt);
        }
        if self.is_closed()
            && batch
                .changes
                .iter()
                .any(|change| !matches!(change, TaskChange::Stop { .. }))
        {
            return Err(invalid(
                "task edits are closed during shutdown; stop remains available",
            ));
        }
        let mut record = self.require_workflow(workflow)?;
        if let Some((source, repair)) = model_source {
            super::model_actions::check_admission(&record, source, &batch, repair)?;
        } else if batch.expected_generation != record.generation {
            return Err(RuntimeError::InvalidTransition(format!(
                "stale workflow generation: expected {}, actual {}",
                batch.expected_generation, record.generation
            )));
        }
        if batch.changes.is_empty() || batch.changes.len() > MAX_TASK_BATCH_ACTIONS {
            return Err(invalid("task batch must contain 1..=32 changes"));
        }
        let reserved_action = match batch.changes.as_slice() {
            [TaskChange::Stop { task }]
                if key == format!("stop:{workflow}:{task}")
                    && record
                        .tasks
                        .get(task)
                        .is_some_and(|task| !task.stop_requested && !task.state.is_settled()) =>
            {
                Some(key.to_owned())
            }
            _ => None,
        };
        let mut aliases = BTreeMap::new();
        for change in &batch.changes {
            if let TaskChange::Add { alias, .. } = change
                && (aliases
                    .insert(alias.clone(), self.unused_task_id(&record, &aliases)?)
                    .is_some()
                    || record.tasks.contains_key(alias))
            {
                return Err(invalid(
                    "new aliases must be unique and cannot name existing tasks",
                ));
            }
        }
        let now = self.clock.now()?;
        for change in batch.changes {
            match change {
                TaskChange::DismissAdmission { task } => {
                    let admission = record
                        .admissions
                        .get_mut(&task)
                        .ok_or_else(|| invalid("model admission does not exist"))?;
                    if admission.status != super::ModelAdmissionStatus::Rejected {
                        return Err(invalid("only a rejected model admission can be dismissed"));
                    }
                    admission.status = super::ModelAdmissionStatus::Dismissed;
                    super::model_actions::clear_owned_wait(&mut record, &task);
                }
                TaskChange::SetLimits { limits } => {
                    super::policy::change_limits(&mut record, limits)?
                }
                TaskChange::Add { alias, definition } => {
                    self.validate_future_definition(&record, &definition)?;
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
                    self.validate_future_definition(&record, &definition)?;
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
                    record.model_admission_wait = None;
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
                TaskChange::AdoptCorrection {
                    task,
                    correction,
                    redirects,
                } => {
                    let correction = aliases.get(&correction).unwrap_or(&correction);
                    adopt_correction(&mut record, &task, correction, &redirects)?;
                }
                TaskChange::Continue => super::policy::resume(&mut record)?,
                TaskChange::Wait { question } => {
                    if question.trim().is_empty() || question.len() > 4096 {
                        return Err(invalid("wait question is empty or oversized"));
                    }
                    record.wait_question = Some(question);
                    record.model_admission_wait = None;
                }
                TaskChange::Finish => record.finish_when_settled = true,
            }
        }
        validate_task_set(&definitions(&record.tasks), &record.limits).map_err(invalid)?;
        for task in record
            .tasks
            .values()
            .filter(|task| task.frozen.is_none() && !task.removed)
        {
            task.definition
                .validate_supplied_inputs(&record.inputs)
                .map_err(invalid)?;
        }
        if let Some((source, repair)) = model_source {
            super::model_actions::accept_admission(&mut record, source, key, &aliases, repair)?;
        }
        let expected = self.advance(&mut record)?;
        record.decision_generation = record.generation;
        let acceptance = WorkflowCommandAcceptance {
            reserved_action,
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
                return Err(
                    milkdrift_persistence::PersistenceError::WorkflowIdempotencyConflict {
                        actor: actor.to_owned(),
                        key: key.to_owned(),
                    }
                    .into(),
                );
            }
            return Ok(Some(receipt));
        }
        Ok(None)
    }

    fn unused_task_id(
        &self,
        record: &WorkflowRecord,
        aliases: &BTreeMap<TaskId, TaskId>,
    ) -> Result<TaskId, RuntimeError> {
        for _ in 0..=4096 {
            let id = TaskId::new(self.ids.next("task")?).map_err(invalid)?;
            if !record.tasks.contains_key(&id) && !aliases.values().any(|value| value == &id) {
                return Ok(id);
            }
        }
        Err(invalid(
            "task identity source cannot allocate an unused identity",
        ))
    }
}

fn adopt_correction(
    record: &mut WorkflowRecord,
    original_id: &TaskId,
    correction: &TaskId,
    redirects: &[milkdrift_blueprint::TaskBindingRedirect],
) -> Result<(), RuntimeError> {
    if redirects.len() > 64 || original_id == correction {
        return Err(invalid(
            "correction redirects exceed their bound or form a cycle",
        ));
    }
    let original = record
        .tasks
        .get(original_id)
        .ok_or_else(|| invalid("corrected task does not exist"))?;
    if !matches!(original.state, TaskState::Failed | TaskState::Stopped)
        || original.adopted_correction.is_some()
    {
        return Err(invalid(
            "adoption requires a settled failed/stopped task without an existing adoption",
        ));
    }
    let replacement = record
        .tasks
        .get(correction)
        .ok_or_else(|| invalid("corrective task does not exist"))?;
    if replacement.state != TaskState::Completed
        || replacement.definition.corrective_of.as_ref() != Some(original_id)
        || replacement.adopted_correction.is_some()
    {
        return Err(invalid(
            "adoption requires successful explicitly linked correction without an adoption chain",
        ));
    }
    let outputs = replacement
        .result
        .as_ref()
        .ok_or_else(|| invalid("successful correction lacks immutable result"))?
        .outputs
        .clone();
    if let Some(reason) = super::output_contract_failure(&original.definition, &outputs) {
        return Err(invalid(format!(
            "correction does not satisfy original outputs: {reason}"
        )));
    }
    let mut changed = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for redirect in redirects {
        if !seen.insert((redirect.consumer.clone(), redirect.input.clone())) {
            return Err(invalid(
                "correction redirects the same input more than once",
            ));
        }
        let consumer = future_task(record, &redirect.consumer, redirect.expected_version)?;
        let Some(TaskInputBinding::TaskOutput { task, output, .. }) =
            consumer.definition.inputs.get_mut(&redirect.input)
        else {
            return Err(invalid(
                "correction redirect must name an existing task-output binding",
            ));
        };
        if task != original_id || !outputs.contains_key(output) {
            return Err(invalid(
                "correction redirect names another source or an absent replacement output",
            ));
        }
        if changed.insert(redirect.consumer.clone()) {
            if consumer.previous_definitions.len() >= 128 {
                return Err(invalid("retained task definition capacity exhausted"));
            }
            consumer
                .previous_definitions
                .push(consumer.definition.clone());
        }
        if let Some(TaskInputBinding::TaskOutput { task, .. }) =
            consumer.definition.inputs.get_mut(&redirect.input)
        {
            *task = correction.clone();
        }
        consumer.definition.dependencies.insert(correction.clone());
    }
    for id in changed {
        let consumer = record
            .tasks
            .get_mut(&id)
            .ok_or_else(|| invalid("redirected consumer disappeared"))?;
        if !consumer.definition.inputs.values().any(|binding| matches!(binding, TaskInputBinding::TaskOutput { task, .. } | TaskInputBinding::SettledOutcome { task } if task == original_id)) {
            consumer.definition.dependencies.remove(original_id);
        }
        consumer.version = consumer
            .version
            .checked_add(1)
            .ok_or_else(|| invalid("task version exhausted"))?;
        consumer.reason = None;
    }
    record
        .tasks
        .get_mut(original_id)
        .ok_or_else(|| invalid("corrected task disappeared"))?
        .adopted_correction = Some(correction.clone());
    Ok(())
}

fn canonical_transport_value(bytes: &[u8]) -> Result<serde_json::Value, RuntimeError> {
    if bytes.is_empty() || bytes.len() > 1_000_000 {
        return Err(invalid(
            "canonical public request exceeds its bounded receipt allowance",
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if !value.is_object() || serde_json::to_vec(&value)? != bytes {
        return Err(invalid("public request must be a canonical JSON object"));
    }
    Ok(value)
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
