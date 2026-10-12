use super::{ActorSession, Owner, PublicFailure, failure};
use milkdrift_blueprint::{
    TaskDefinition, TaskId, TaskInputBinding, TaskInputValue, WorkflowLimits,
};
use milkdrift_capability::{CapabilityId, CapabilityRequirement, OperationId};
use milkdrift_control_protocol::tasks::{
    InputBinding, InputValue, PathAccess, TaskCapability, TaskDefinitionDto, WorkflowLimitsDto,
};
use milkdrift_persistence::ArtifactStore;
use milkdrift_workspace::ArtifactId;
use std::collections::BTreeMap;

impl Owner {
    pub(super) fn task_value(
        &self,
        session: &ActorSession,
        value: InputValue,
    ) -> Result<TaskInputValue, PublicFailure> {
        match value {
            InputValue::Text { text } => Ok(TaskInputValue::Text { text }),
            InputValue::Absent => Ok(TaskInputValue::Absent),
            InputValue::Uploaded { input_id } => {
                let id = ArtifactId::new(&input_id).map_err(failure)?;
                let metadata = self
                    .store
                    .metadata(&id)
                    .map_err(failure)?
                    .ok_or_else(super::super::read_model::not_found)?;
                match metadata.provenance().producer() {
                    milkdrift_workspace::CausalReference::ClientUpload { host, client, .. }
                        if host == &self.host_id && client.as_str() == session.actor.as_str() => {}
                    _ => {
                        return Err(failure(
                            "task input must be an upload owned by this authenticated actor and daemon",
                        ));
                    }
                }
                let content =
                    self.artifact_range(session, &input_id, 0, 524_288, "task-input-capture")?;
                if !content.end {
                    return Err(failure("text input exceeds capture bound"));
                }
                let upload: milkdrift_control_protocol::tasks::TaskUploadRequest =
                    milkdrift_control_protocol::decode_json(&content.bytes).map_err(failure)?;
                let mut captured =
                    TaskInputValue::text_file(upload.name, upload.text).map_err(failure)?;
                if let TaskInputValue::TextFile {
                    source_artifact, ..
                } = &mut captured
                {
                    *source_artifact = Some(input_id);
                }
                Ok(captured)
            }
        }
    }

    pub(super) fn task_definition(
        &self,
        session: &ActorSession,
        definition: TaskDefinitionDto,
    ) -> Result<TaskDefinition, PublicFailure> {
        let inputs = definition
            .inputs
            .into_iter()
            .map(|(name, value)| {
                let value = match value {
                    InputBinding::Inline { value } => TaskInputBinding::Inline {
                        value: self.task_value(session, value)?,
                    },
                    InputBinding::Supplied { name, optional } => {
                        TaskInputBinding::Supplied { name, optional }
                    }
                    InputBinding::TaskOutput {
                        task,
                        output,
                        optional,
                    } => TaskInputBinding::TaskOutput {
                        task: TaskId::new(task).map_err(failure)?,
                        output,
                        optional,
                    },
                    InputBinding::SettledOutcome { task } => TaskInputBinding::SettledOutcome {
                        task: TaskId::new(task).map_err(failure)?,
                    },
                };
                Ok((name, value))
            })
            .collect::<Result<BTreeMap<_, _>, PublicFailure>>()?;
        Ok(TaskDefinition {
            title: definition.title,
            description: definition.description,
            instructions: definition.instructions,
            capability: CapabilityRequirement::new(
                OperationId::new(definition.capability.operation).map_err(failure)?,
            )
            .exact(CapabilityId::new(definition.capability.id).map_err(failure)?),
            inputs,
            dependencies: definition
                .dependencies
                .into_iter()
                .map(TaskId::new)
                .collect::<Result<_, _>>()
                .map_err(failure)?,
            required: definition.required,
            corrective_of: definition
                .corrective_of
                .map(TaskId::new)
                .transpose()
                .map_err(failure)?,
            paths: definition
                .paths
                .into_iter()
                .map(|path| milkdrift_blueprint::TaskPathAccess {
                    path: path.path,
                    write: path.write,
                })
                .collect(),
        })
    }
}

pub(super) fn limits(value: &WorkflowLimits) -> WorkflowLimitsDto {
    WorkflowLimitsDto {
        tasks: value.tasks,
        concurrent_tasks: value.concurrent_tasks,
        model_batches: value.model_batches,
        rejected_responses: value.rejected_responses,
        corrective_failures: value.corrective_failures,
        active_execution_ms: value.active_execution_ms,
        tool_calls_per_task: value.tool_calls_per_task,
    }
}

pub(super) fn domain_limits(value: WorkflowLimitsDto) -> WorkflowLimits {
    WorkflowLimits {
        tasks: value.tasks,
        concurrent_tasks: value.concurrent_tasks,
        model_batches: value.model_batches,
        rejected_responses: value.rejected_responses,
        corrective_failures: value.corrective_failures,
        active_execution_ms: value.active_execution_ms,
        tool_calls_per_task: value.tool_calls_per_task,
    }
}

pub(super) fn definition(value: &TaskDefinition) -> Result<TaskDefinitionDto, PublicFailure> {
    let inputs = value
        .inputs
        .iter()
        .map(|(name, binding)| {
            let binding = match binding {
                TaskInputBinding::Inline { value } => InputBinding::Inline {
                    value: input(value)?,
                },
                TaskInputBinding::Supplied { name, optional } => InputBinding::Supplied {
                    name: name.clone(),
                    optional: *optional,
                },
                TaskInputBinding::TaskOutput {
                    task,
                    output,
                    optional,
                } => InputBinding::TaskOutput {
                    task: task.to_string(),
                    output: output.clone(),
                    optional: *optional,
                },
                TaskInputBinding::SettledOutcome { task } => InputBinding::SettledOutcome {
                    task: task.to_string(),
                },
            };
            Ok((name.clone(), binding))
        })
        .collect::<Result<_, PublicFailure>>()?;
    Ok(TaskDefinitionDto {
        title: value.title.clone(),
        description: value.description.clone(),
        instructions: value.instructions.clone(),
        capability: TaskCapability {
            id: value
                .capability
                .exact_capability()
                .ok_or_else(|| failure("task has no exact configured selection"))?
                .to_string(),
            operation: value.capability.operation().to_string(),
        },
        inputs,
        dependencies: value.dependencies.iter().map(ToString::to_string).collect(),
        required: value.required,
        corrective_of: value.corrective_of.as_ref().map(ToString::to_string),
        paths: value
            .paths
            .iter()
            .map(|path| PathAccess {
                path: path.path.clone(),
                write: path.write,
            })
            .collect(),
    })
}

fn input(value: &TaskInputValue) -> Result<InputValue, PublicFailure> {
    match value {
        TaskInputValue::TextFile {
            source_artifact: Some(input_id),
            ..
        } => Ok(InputValue::Uploaded {
            input_id: input_id.clone(),
        }),
        TaskInputValue::Text { text } | TaskInputValue::TextFile { text, .. } => {
            Ok(InputValue::Text { text: text.clone() })
        }
        TaskInputValue::Absent => Ok(InputValue::Absent),
        TaskInputValue::Json { value } => Ok(InputValue::Text {
            text: serde_json::to_string(value).map_err(failure)?,
        }),
        TaskInputValue::Artifact { reference } => Ok(InputValue::Uploaded {
            input_id: reference.identity().into(),
        }),
    }
}
