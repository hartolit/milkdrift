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
        if let Some(expected) = definition.authoring_generation {
            let current = self
                .task_authoring(session)?
                .into_iter()
                .find(|contract| contract.capability == definition.capability);
            if current
                .as_ref()
                .is_none_or(|contract| contract.generation != expected)
            {
                return Err(PublicFailure::new(
                    milkdrift_control_protocol::ErrorCode::Conflict,
                    "The executor's input and result choices changed. Refresh its choices and review this task before submitting again.",
                    false,
                ));
            }
        }
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
            execution: super::execution_mapping::domain_execution(definition.execution)?,
            actions: definition
                .actions
                .map(|policy| milkdrift_blueprint::TaskActionPolicy {
                    automatic_repair: policy.automatic_repair,
                }),
            tools: definition
                .tools
                .into_iter()
                .map(|tool| {
                    Ok(match tool {
                        milkdrift_control_protocol::tasks::TaskToolDto::ReadFile => {
                            milkdrift_blueprint::TaskTool::ReadFile
                        }
                        milkdrift_control_protocol::tasks::TaskToolDto::WriteFile => {
                            milkdrift_blueprint::TaskTool::WriteFile
                        }
                        milkdrift_control_protocol::tasks::TaskToolDto::Process { capability } => {
                            milkdrift_blueprint::TaskTool::Process {
                                capability: CapabilityRequirement::new(
                                    OperationId::new(capability.operation).map_err(failure)?,
                                )
                                .exact(CapabilityId::new(capability.id).map_err(failure)?),
                            }
                        }
                    })
                })
                .collect::<Result<_, PublicFailure>>()?,
            title: definition.title,
            description: definition.description,
            instructions: definition.instructions,
            capability: CapabilityRequirement::new(
                OperationId::new(definition.capability.operation).map_err(failure)?,
            )
            .exact(CapabilityId::new(definition.capability.id).map_err(failure)?),
            outputs: definition
                .outputs
                .into_iter()
                .map(|(name, output)| {
                    (
                        name,
                        milkdrift_blueprint::TaskOutputDeclaration {
                            required: output.required,
                            media_type: output.media_type,
                            max_bytes: output.max_bytes,
                        },
                    )
                })
                .collect(),
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
        execution: super::execution_mapping::public_execution(&value.execution)?,
        authoring_generation: None,
        actions: value.actions.as_ref().map(|policy| milkdrift_control_protocol::tasks::TaskActionPolicyDto { automatic_repair:policy.automatic_repair }),
        tools: value.tools.iter().map(|tool| Ok(match tool {
            milkdrift_blueprint::TaskTool::ReadFile => milkdrift_control_protocol::tasks::TaskToolDto::ReadFile,
            milkdrift_blueprint::TaskTool::WriteFile => milkdrift_control_protocol::tasks::TaskToolDto::WriteFile,
            milkdrift_blueprint::TaskTool::Process { capability } => milkdrift_control_protocol::tasks::TaskToolDto::Process {
                capability: TaskCapability { id:capability.exact_capability().ok_or_else(|| failure("process tool must select an exact configured capability"))?.to_string(),operation:capability.operation().to_string() },
            },
        })).collect::<Result<_,PublicFailure>>()?,
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
        outputs: value
            .outputs
            .iter()
            .map(|(name, output)| {
                (
                    name.clone(),
                    milkdrift_control_protocol::tasks::TaskOutputDeclarationDto {
                        required: output.required,
                        media_type: output.media_type.clone(),
                        max_bytes: output.max_bytes,
                    },
                )
            })
            .collect(),
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
