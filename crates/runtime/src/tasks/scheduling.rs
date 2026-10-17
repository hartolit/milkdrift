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

use super::inputs::resolve_inputs;
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
        if self.is_closed() {
            return Ok(Vec::new());
        }
        let mut cursor = self
            .cursor
            .lock()
            .map_err(|_| invalid("task scheduler cursor unavailable"))?;
        let page = self.store.workflows(cursor.as_deref(), maximum)?;
        *cursor = page.next.clone();
        let mut dispatches = Vec::new();
        for snapshot in page.items {
            let mut workflow = self.require_workflow(&snapshot.workflow)?;
            self.observe_duration(&mut workflow, self.clock.now()?)?;
            if super::policy::request_limit_stop(&mut workflow)? {
                let expected = self.advance(&mut workflow)?;
                self.save(&workflow, Some(expected), None)?;
            }
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
                        >= usize::try_from(
                            workflow.limits.concurrent_tasks.min(
                                workflow
                                    .allowance
                                    .concurrency
                                    .unwrap_or(workflow.limits.concurrent_tasks),
                            ),
                        )
                        .map_err(invalid)?
                {
                    break;
                }
                let task = workflow
                    .tasks
                    .get(&task_id)
                    .ok_or_else(|| invalid("queued task disappeared"))?
                    .clone();
                if let Some(frozen) = &task.frozen {
                    if !frozen.entered && !task.stop_requested {
                        let dispatch = frozen.dispatch.clone();
                        let selected = workflow
                            .tasks
                            .get_mut(&task_id)
                            .ok_or_else(|| invalid("retained start disappeared"))?;
                        selected.state = TaskState::Running;
                        selected.reason = None;
                        let expected = self.advance(&mut workflow)?;
                        workflow.decision_generation = workflow.generation;
                        self.save(&workflow, Some(expected), None)?;
                        dispatches.push(dispatch);
                        active = active
                            .checked_add(1)
                            .ok_or_else(|| invalid("task concurrency overflow"))?;
                    }
                    continue;
                }
                if self.active_task_count()? >= u64::from(self.maximum_active_tasks) {
                    break;
                }
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
                let now = self.clock.now()?;
                self.observe_duration(&mut workflow, now)?;
                if super::duration::remaining(&workflow) == 0 {
                    break;
                }
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
                            accepted_generation: workflow
                                .generation
                                .checked_add(1)
                                .ok_or_else(|| invalid("workflow generation exhausted"))?,
                            definition: task.definition,
                            inputs,
                            dispatch: dispatch.clone(),
                            entered: false,
                            execution_duration: None,
                            reports: Vec::new(),
                        });
                        let expected = self.advance(&mut workflow)?;
                        workflow.decision_generation = workflow.generation;
                        self.save(&workflow, Some(expected), None)?;
                        dispatches.push(dispatch);
                        active = active
                            .checked_add(1)
                            .ok_or_else(|| invalid("task concurrency overflow"))?;
                    }
                    Err(
                        error @ RuntimeError::Executor(
                            crate::ExecutorError::CapacityUnavailable(_)
                            | crate::ExecutorError::Unavailable(_),
                        ),
                    ) => {
                        let reason =
                            milkdrift_contracts::truncate_utf8(&error.to_string(), 4096).to_owned();
                        if task.reason.as_ref() != Some(&reason) {
                            workflow
                                .tasks
                                .get_mut(&task_id)
                                .ok_or_else(|| invalid("task disappeared"))?
                                .reason = Some(reason);
                            let expected = self.advance(&mut workflow)?;
                            self.save(&workflow, Some(expected), None)?;
                        }
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
        let mut resolved_definition = task.definition.clone();
        resolved_definition.inputs = inputs
            .iter()
            .map(|(name, value)| {
                (
                    name.clone(),
                    TaskInputBinding::Inline {
                        value: value.clone(),
                    },
                )
            })
            .collect();
        resolved_definition
            .validate_supplied_inputs(&BTreeMap::new())
            .map_err(invalid)?;
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
        )
        .with_filesystem(
            task.definition
                .paths
                .iter()
                .map(|access| {
                    milkdrift_authority::FilesystemScope::new(
                        access.path.clone(),
                        std::collections::BTreeSet::from([if access.write {
                            milkdrift_authority::AccessMode::Write
                        } else {
                            milkdrift_authority::AccessMode::Read
                        }]),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
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
                    TaskInputValue::Artifact { reference } => {
                        content.push_str("[immutable named artifact supplied with this request]");
                        invocation_inputs.push(
                            InputReference::new(
                                name,
                                InvocationValueReference::Artifact {
                                    reference: reference.clone(),
                                },
                            )
                            .map_err(invalid)?,
                        );
                    }
                }
            }
            let message = Message::new(
                MessageRole::User,
                vec![ContentPart::Text { text: content }],
                None,
            )
            .map_err(invalid)?;
            let mut messages = vec![message];
            let structured_output = if task.definition.actions.is_some() {
                let (snapshot, schema) = super::model_actions::authoring_context(workflow, task)?;
                messages.push(
                    Message::new(
                        MessageRole::User,
                        vec![ContentPart::Text { text: snapshot }],
                        None,
                    )
                    .map_err(invalid)?,
                );
                Some(schema)
            } else {
                None
            };
            let model_task = ModelTaskRequest::new(
                messages,
                Vec::new(),
                structured_output,
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
                .checked_add(super::duration::remaining(workflow))
                .ok_or_else(|| invalid("task lease deadline overflow"))?,
        );
        let mut entry_request = authorization.request().clone();
        entry_request.evaluated_at = BoundaryTimeMillis::new(now.get());
        let entry = self.authority.evaluate(&entry_request)?;
        if !entry.is_allowed() {
            return Err(invalid("grant was revoked before task start"));
        }
        let budget = WorkspaceBudget::new(
            0,
            0,
            0,
            u64::from(workflow.limits.tasks)
                .checked_mul(64)
                .ok_or_else(|| invalid("output count overflow"))?,
            super::TASK_OUTPUT_BYTES,
            u64::from(workflow.limits.tasks)
                .checked_mul(super::TASK_OUTPUT_BYTES)
                .ok_or_else(|| invalid("output capacity overflow"))?,
        )
        .map_err(invalid)?;
        let dispatch = ExecutionDispatch::from_snapshot(
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
        .for_task(budget, task.definition.paths.clone())
        .with_task_tools(
            task.definition.tools.clone(),
            workflow.limits.tool_calls_per_task,
        );
        if serde_json::to_vec(&dispatch)?.len() > super::capacity::MAX_DISPATCH_BYTES {
            return Err(invalid(
                "frozen external dispatch exceeds its reserved 64 KiB allowance",
            ));
        }
        Ok(dispatch)
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

fn invalid(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Scheduling(error.to_string())
}
