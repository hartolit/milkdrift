//! Project execution DTOs adapt to the shared semantic selection without resolving defaults.
use super::{PublicFailure, failure};
use milkdrift_blueprint::{
    TaskExecution, TaskProjectAccess, TaskProjectBinding, TaskProjectFolder, TaskTool,
};
use milkdrift_capability::{CapabilityId, CapabilityRequirement, OperationId};
use milkdrift_control_protocol::{
    task_execution::{
        TaskExecutionDto, TaskProjectAccessDto, TaskProjectBindingDto, TaskProjectFolderDto,
    },
    tasks::{TaskCapability, TaskToolDto},
};

fn domain_tools(tools: Vec<TaskToolDto>) -> Result<Vec<TaskTool>, PublicFailure> {
    tools
        .into_iter()
        .map(|tool| {
            Ok(match tool {
                TaskToolDto::ReadFile => TaskTool::ReadFile,
                TaskToolDto::WriteFile => TaskTool::WriteFile,
                TaskToolDto::Process { capability } => TaskTool::Process {
                    capability: CapabilityRequirement::new(
                        OperationId::new(capability.operation).map_err(failure)?,
                    )
                    .exact(CapabilityId::new(capability.id).map_err(failure)?),
                },
            })
        })
        .collect()
}

fn public_tools(tools: &[TaskTool]) -> Result<Vec<TaskToolDto>, PublicFailure> {
    tools
        .iter()
        .map(|tool| {
            Ok(match tool {
                TaskTool::ReadFile => TaskToolDto::ReadFile,
                TaskTool::WriteFile => TaskToolDto::WriteFile,
                TaskTool::Process { capability } => TaskToolDto::Process {
                    capability: TaskCapability {
                        id: capability
                            .exact_capability()
                            .ok_or_else(|| failure("project process selection must be exact"))?
                            .to_string(),
                        operation: capability.operation().to_string(),
                    },
                },
            })
        })
        .collect()
}

fn domain_access(access: TaskProjectAccessDto) -> TaskProjectAccess {
    match access {
        TaskProjectAccessDto::None => TaskProjectAccess::None,
        TaskProjectAccessDto::Read => TaskProjectAccess::Read,
        TaskProjectAccessDto::Write => TaskProjectAccess::Write,
    }
}
fn public_access(access: TaskProjectAccess) -> TaskProjectAccessDto {
    match access {
        TaskProjectAccess::None => TaskProjectAccessDto::None,
        TaskProjectAccess::Read => TaskProjectAccessDto::Read,
        TaskProjectAccess::Write => TaskProjectAccessDto::Write,
    }
}

pub(super) fn domain_execution(value: TaskExecutionDto) -> Result<TaskExecution, PublicFailure> {
    Ok(match value {
        TaskExecutionDto::Legacy => TaskExecution::Legacy,
        TaskExecutionDto::Restricted => {
            return Err(failure(
                "restricted execution details cannot be reused for authoring",
            ));
        }
        TaskExecutionDto::Project {
            expected_generation,
        } => TaskExecution::Project {
            expected_generation,
        },
        TaskExecutionDto::Explicit {
            expected_generation,
            settings,
            access,
            tools,
        } => TaskExecution::Explicit {
            expected_generation,
            settings: super::super::projects::domain_settings(settings),
            access: domain_access(access),
            tools: domain_tools(tools)?,
        },
        TaskExecutionDto::Bound { binding } => TaskExecution::Bound {
            binding: TaskProjectBinding {
                paths: binding
                    .paths
                    .into_iter()
                    .map(|path| milkdrift_blueprint::TaskPathAccess {
                        path: path.path,
                        write: path.write,
                    })
                    .collect(),
                scopes: binding
                    .scopes
                    .into_iter()
                    .map(|scope| TaskProjectFolder {
                        requested: scope.requested,
                        physical: scope.physical,
                        identity: scope.identity,
                    })
                    .collect(),
                project_id: binding.project_id,
                generation: binding.generation,
                folder: TaskProjectFolder {
                    requested: binding.folder.requested,
                    physical: binding.folder.physical,
                    identity: binding.folder.identity,
                },
                settings: super::super::projects::domain_settings(binding.settings),
                access: domain_access(binding.access),
                tools: domain_tools(binding.tools)?,
            },
        },
    })
}

pub(super) fn public_execution(value: &TaskExecution) -> Result<TaskExecutionDto, PublicFailure> {
    Ok(match value {
        TaskExecution::Legacy => TaskExecutionDto::Legacy,
        TaskExecution::Project {
            expected_generation,
        } => TaskExecutionDto::Project {
            expected_generation: *expected_generation,
        },
        TaskExecution::Explicit {
            expected_generation,
            settings,
            access,
            tools,
        } => TaskExecutionDto::Explicit {
            expected_generation: *expected_generation,
            settings: super::super::projects::public_settings(settings.clone()),
            access: public_access(*access),
            tools: public_tools(tools)?,
        },
        TaskExecution::Bound { binding } => TaskExecutionDto::Bound {
            binding: TaskProjectBindingDto {
                paths: binding
                    .paths
                    .iter()
                    .map(|path| milkdrift_control_protocol::tasks::PathAccess {
                        path: path.path.clone(),
                        write: path.write,
                    })
                    .collect(),
                scopes: binding
                    .scopes
                    .iter()
                    .map(|scope| TaskProjectFolderDto {
                        requested: scope.requested.clone(),
                        physical: scope.physical.clone(),
                        identity: scope.identity.clone(),
                    })
                    .collect(),
                project_id: binding.project_id.clone(),
                generation: binding.generation,
                folder: TaskProjectFolderDto {
                    requested: binding.folder.requested.clone(),
                    physical: binding.folder.physical.clone(),
                    identity: binding.folder.identity.clone(),
                },
                settings: super::super::projects::public_settings(binding.settings.clone()),
                access: public_access(binding.access),
                tools: public_tools(&binding.tools)?,
            },
        },
    })
}
