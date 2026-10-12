use std::collections::BTreeMap;

use milkdrift_authority::BoundaryTimeMillis;
use milkdrift_blueprint::{NodeId, TaskInputBinding, TaskInputValue};
use milkdrift_capability::{
    BoundedJson, CapabilityCategory, IdempotencyBehavior, IdempotencyKey, InputReference,
    InvocationId, InvocationRequest, InvocationValueReference,
};
use milkdrift_model::{
    ContentPart, Message, MessageRole, ModelTaskRequest, ModelTaskRequestDocument, SessionSelection,
};
use milkdrift_persistence::{AttemptId, LeaseId, NodeExecutionId, TimestampMillis};
use milkdrift_workspace::{RunId, WorkspaceBudget};

use super::{FrozenTaskStart, TaskRecord, TaskRuntime, TaskState, WorkflowRecord};
use crate::{CapabilityResolutionContext, ExecutionDispatch, RuntimeError};

impl TaskRuntime {
    /// Selects a bounded fair page, freezes ready work and returns exactly the committed starts.
    ///
    /// # Errors
    /// Refuses invalid page bounds and returns authority, clock, synchronization or store failures.
    pub fn tick(&self, maximum: u16) -> Result<Vec<ExecutionDispatch>, RuntimeError> {
        if maximum == 0 || maximum > 200 {
            return Err(invalid("task tick bound must be 1..=200"));
        }
        let _guard = self
            .gate
            .lock()
            .map_err(|_| invalid("task scheduler lock unavailable"))?;
        let mut cursor = self
            .cursor
            .lock()
            .map_err(|_| invalid("task scheduler cursor unavailable"))?;
        let page = self.store.workflows(cursor.as_deref(), maximum)?;
        *cursor = page.next.clone();
        let mut dispatches = Vec::new();
        for snapshot in page.items {
            let mut workflow = self.require_workflow(&snapshot.workflow)?;
            if workflow.wait_question.is_some() {
                continue;
            }
            let mut active = workflow
                .tasks
                .values()
                .filter(|task| matches!(task.state, TaskState::Running | TaskState::Uncertain))
                .count();
            let candidates: Vec<_> = workflow
                .tasks
                .iter()
                .filter(|(_, task)| task.state == TaskState::Queued)
                .map(|(id, _)| id.clone())
                .collect();
            for task_id in candidates {
                if dispatches.len() >= usize::from(maximum)
                    || active
                        >= usize::try_from(workflow.limits.concurrent_tasks).map_err(invalid)?
                {
                    break;
                }
                let task = workflow
                    .tasks
                    .get(&task_id)
                    .ok_or_else(|| invalid("queued task disappeared"))?
                    .clone();
                let inputs = match resolve_inputs(&workflow, &task) {
                    Ok(inputs) => inputs,
                    Err(reason) => {
                        if task.reason.as_ref() != Some(&reason) {
                            workflow
                                .tasks
                                .get_mut(&task_id)
                                .ok_or_else(|| invalid("task disappeared"))?
                                .reason = Some(reason);
                            let expected = self.advance(&mut workflow)?;
                            self.save(&workflow, Some(expected), None)?;
                        }
                        continue;
                    }
                };
                if workflow.active_execution_ms >= workflow.limits.active_execution_ms {
                    workflow
                        .tasks
                        .get_mut(&task_id)
                        .ok_or_else(|| invalid("task disappeared"))?
                        .reason = Some("workflow active execution allowance exhausted".into());
                    let expected = self.advance(&mut workflow)?;
                    self.save(&workflow, Some(expected), None)?;
                    continue;
                }
                let now = self.clock.now()?;
                match self.prepare_dispatch(&workflow, &task, &inputs, now) {
                    Ok(dispatch) => {
                        let selected = workflow
                            .tasks
                            .get_mut(&task_id)
                            .ok_or_else(|| invalid("task disappeared"))?;
                        selected.state = TaskState::Running;
                        selected.started_at = Some(now);
                        selected.reason = None;
                        selected.frozen = Some(FrozenTaskStart {
                            definition: task.definition,
                            inputs,
                            dispatch: dispatch.clone(),
                            entered: false,
                            reports: Vec::new(),
                        });
                        let expected = self.advance(&mut workflow)?;
                        self.save(&workflow, Some(expected), None)?;
                        dispatches.push(dispatch);
                        active = active
                            .checked_add(1)
                            .ok_or_else(|| invalid("task concurrency overflow"))?;
                    }
                    Err(error) => {
                        let selected = workflow
                            .tasks
                            .get_mut(&task_id)
                            .ok_or_else(|| invalid("task disappeared"))?;
                        selected.state = TaskState::Failed;
                        selected.reason = Some(
                            milkdrift_contracts::truncate_utf8(&error.to_string(), 4096).to_owned(),
                        );
                        selected.ended_at = Some(now);
                        let expected = self.advance(&mut workflow)?;
                        self.save(&workflow, Some(expected), None)?;
                    }
                }
            }
        }
        Ok(dispatches)
    }

    fn prepare_dispatch(
        &self,
        workflow: &WorkflowRecord,
        task: &TaskRecord,
        inputs: &BTreeMap<String, TaskInputValue>,
        now: TimestampMillis,
    ) -> Result<ExecutionDispatch, RuntimeError> {
        let revision = super::admission::revision_identity(&serde_json::to_vec(&task.definition)?)?;
        let node = NodeId::new(task.id.as_str()).map_err(invalid)?;
        let execution = NodeExecutionId::new(format!("execution_{}", task.id)).map_err(invalid)?;
        let attempt = AttemptId::new(self.ids.next("attempt")?).map_err(invalid)?;
        let context = CapabilityResolutionContext::new(
            workflow.execution_authority.clone(),
            revision.clone(),
            node.clone(),
            execution.clone(),
            attempt.clone(),
        );
        let resolved = self.executor.resolve_authorized(
            &task.definition.capability,
            &context,
            self.authority.as_ref(),
            now.get(),
        )?;
        let authorization = resolved
            .authorization()
            .ok_or_else(|| invalid("executor omitted exact selection authorization"))?
            .clone();
        let invocation = InvocationId::new(self.ids.next("invocation")?).map_err(invalid)?;
        let key = match resolved.snapshot().operation_contract().idempotency() {
            IdempotencyBehavior::Unsupported => None,
            IdempotencyBehavior::CapabilityScoped | IdempotencyBehavior::ProviderProfileScoped => {
                Some(IdempotencyKey::new(format!("task_{}", task.id)).map_err(invalid)?)
            }
        };
        let mut invocation_inputs = Vec::new();
        if resolved.descriptor().category() == &CapabilityCategory::Model {
            let mut content = task.definition.instructions.clone();
            for (name, value) in inputs {
                content.push_str(&format!("\n\nInput {name}:\n"));
                match value {
                    TaskInputValue::Text { text } | TaskInputValue::TextFile { text, .. } => {
                        content.push_str(text)
                    }
                    TaskInputValue::Json { value } => {
                        content.push_str(&serde_json::to_string(value)?)
                    }
                    TaskInputValue::Absent => {
                        content.push_str("[explicitly absent optional input]")
                    }
                    TaskInputValue::Artifact { .. } => {
                        return Err(invalid(
                            "model task output binding requires a captured text result",
                        ));
                    }
                }
            }
            let message = Message::new(
                MessageRole::User,
                vec![ContentPart::Text { text: content }],
                None,
            )
            .map_err(invalid)?;
            let model_task = ModelTaskRequest::new(
                vec![message],
                Vec::new(),
                None,
                SessionSelection::Fresh,
                None,
                4096,
                false,
                BTreeMap::new(),
            )
            .map_err(invalid)?;
            invocation_inputs.push(inline(
                milkdrift_model::MODEL_TASK_INPUT_NAME,
                serde_json::to_value(ModelTaskRequestDocument::new(model_task))?,
            )?);
        } else {
            invocation_inputs.push(inline(
                "instructions",
                serde_json::Value::String(task.definition.instructions.clone()),
            )?);
            for (name, value) in inputs {
                let input = match value {
                    TaskInputValue::Text { text } | TaskInputValue::TextFile { text, .. } => {
                        inline(name, serde_json::Value::String(text.clone()))?
                    }
                    TaskInputValue::Json { value } => inline(name, value.clone())?,
                    TaskInputValue::Absent => inline(name, serde_json::Value::Null)?,
                    TaskInputValue::Artifact { reference } => InputReference::new(
                        name,
                        InvocationValueReference::Artifact {
                            reference: reference.clone(),
                        },
                    )
                    .map_err(invalid)?,
                };
                invocation_inputs.push(input);
            }
        }
        let request = InvocationRequest::new(
            invocation,
            resolved.snapshot().capability().clone(),
            resolved.snapshot().operation().clone(),
            resolved.snapshot().provider_profile().cloned(),
            key,
            invocation_inputs,
            BTreeMap::new(),
        )
        .map_err(invalid)?;
        let lease = LeaseId::new(self.ids.next("lease")?).map_err(invalid)?;
        let expires_at = TimestampMillis::new(
            now.get()
                .checked_add(workflow.limits.active_execution_ms)
                .ok_or_else(|| invalid("task lease deadline overflow"))?,
        );
        let mut entry_request = authorization.request().clone();
        entry_request.evaluated_at = BoundaryTimeMillis::new(now.get());
        let entry = self.authority.evaluate(&entry_request)?;
        if !entry.is_allowed() {
            return Err(invalid("grant was revoked before task start"));
        }
        let budget = WorkspaceBudget::new(0, 0, 0, 256, 1_048_576, 2_097_152).map_err(invalid)?;
        Ok(ExecutionDispatch::from_snapshot(
            RunId::new(workflow.id.as_str()).map_err(invalid)?,
            revision,
            node,
            execution,
            attempt,
            lease,
            expires_at,
            resolved.snapshot().clone(),
            workflow.execution_authority.clone(),
            authorization,
            entry,
            request,
        )?
        .for_task(budget, task.definition.paths.clone()))
    }
}

fn inline(name: &str, value: serde_json::Value) -> Result<InputReference, RuntimeError> {
    InputReference::new(
        name,
        InvocationValueReference::Inline {
            value: BoundedJson::new(value).map_err(invalid)?,
        },
    )
    .map_err(invalid)
}

fn resolve_inputs(
    workflow: &WorkflowRecord,
    task: &TaskRecord,
) -> Result<BTreeMap<String, TaskInputValue>, String> {
    for dependency in &task.definition.dependencies {
        let producer = workflow
            .tasks
            .get(dependency)
            .ok_or_else(|| format!("dependency {dependency} is absent"))?;
        let accepts_settled = task.definition.inputs.values().any(|binding| matches!(binding, TaskInputBinding::SettledOutcome { task } if task == dependency) || matches!(binding, TaskInputBinding::TaskOutput { task, optional: true, .. } if task == dependency));
        if producer.state != TaskState::Completed
            && !(accepts_settled && producer.state.is_settled())
        {
            return Err(format!("dependency {dependency} is {:?}", producer.state));
        }
    }
    task.definition.inputs.iter().map(|(name, binding)| {
        let value = match binding {
            TaskInputBinding::Inline { value } => value.clone(),
            TaskInputBinding::Supplied { name, optional } => match workflow.inputs.get(name) { Some(value) => value.clone(), None if *optional => TaskInputValue::Absent, None => return Err(format!("missing supplied input {name}")) },
            TaskInputBinding::TaskOutput { task, output, optional } => {
                let producer = workflow.tasks.get(task).ok_or_else(|| format!("producer {task} is absent"))?;
                match producer.result.as_ref().and_then(|result| result.outputs.get(output)) { Some(reference) => TaskInputValue::Artifact { reference: reference.clone() }, None if *optional && producer.state.is_settled() => TaskInputValue::Absent, None => return Err(format!("missing output {task}/{output}")) }
            }
            TaskInputBinding::SettledOutcome { task } => {
                let producer = workflow.tasks.get(task).ok_or_else(|| format!("producer {task} is absent"))?;
                if !producer.state.is_settled() { return Err(format!("producer {task} is not settled")); }
                TaskInputValue::Json { value: serde_json::json!({"task":task,"state":producer.state,"result":producer.result}) }
            }
        };
        Ok((name.clone(), value))
    }).collect()
}

fn invalid(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Scheduling(error.to_string())
}
