//! Resolve project intent once, before the ordinary task transaction accepts immutable content.

use super::TaskControlService;
use crate::{ActorAuthorityContext, ControlError};
use milkdrift_blueprint::{
    ProjectEnvironment, TaskBatch, TaskChange, TaskDefinition, TaskExecution, TaskPathAccess,
    TaskProjectAccess, TaskProjectBinding, TaskProjectFolder, TaskTool,
};
use milkdrift_capability::{CapabilityId, CapabilityRequirement, OperationId};
use milkdrift_persistence::ProjectRecord;
use milkdrift_runtime::WorkflowRecord;

impl TaskControlService {
    pub(super) fn project(&self, id: Option<&str>) -> Result<Option<ProjectRecord>, ControlError> {
        id.map(|id| {
            self.projects
                .as_ref()
                .ok_or_else(|| invalid("project owner is not configured"))?
                .project(id)?
                .ok_or_else(|| invalid("project is unavailable"))
        })
        .transpose()
    }

    pub(super) fn resolve_project_batch(
        &self,
        context: &ActorAuthorityContext,
        record: &WorkflowRecord,
        batch: &mut TaskBatch,
        feedback: bool,
    ) -> Result<(), ControlError> {
        let project = self
            .projects
            .as_ref()
            .map(|store| store.project_for_workflow(&record.id))
            .transpose()?
            .flatten();
        for change in &mut batch.changes {
            match change {
                TaskChange::Add { definition, .. } => {
                    // Feedback retains the source's already admitted binding and permissions.
                    if feedback && definition.execution.binding().is_some() {
                        self.authorize_project_definition(context, definition)?;
                        continue;
                    }
                    resolve_definition(definition, project.as_ref(), None)?;
                    self.runtime.pin_project_paths(definition)?;
                    self.authorize_project_definition(context, definition)?;
                }
                TaskChange::Revise {
                    task, definition, ..
                } => {
                    let previous = record.tasks.get(task).map(|task| &task.definition);
                    let preserved =
                        previous.is_some_and(|old| old.execution == definition.execution);
                    resolve_definition(definition, project.as_ref(), previous)?;
                    if !preserved {
                        self.runtime.pin_project_paths(definition)?;
                    }
                    self.authorize_project_definition(context, definition)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn authorize_project_definition(
        &self,
        context: &ActorAuthorityContext,
        definition: &TaskDefinition,
    ) -> Result<(), ControlError> {
        let Some(binding) = definition.execution.binding() else {
            return Ok(());
        };
        self.authorize_project_binding(context, binding)
    }

    pub(super) fn authorize_project_binding(
        &self,
        context: &ActorAuthorityContext,
        binding: &TaskProjectBinding,
    ) -> Result<(), ControlError> {
        use milkdrift_authority::{
            AccessMode, AuthorityBudget, AuthorityExecutionProvenance, AuthorityOperation,
            AuthorityRequest, BoundaryTimeMillis, DecisionId, FilesystemScope,
            RequestedResourceFacts,
        };
        let mut resources = RequestedResourceFacts::empty();
        for path in [&binding.folder.requested, &binding.folder.physical] {
            resources.filesystem.push(FilesystemScope::new(
                path.clone(),
                std::collections::BTreeSet::from([AccessMode::Read]),
            )?);
        }
        for (path, scope) in binding.paths.iter().zip(&binding.scopes) {
            let access = if path.write {
                std::collections::BTreeSet::from([AccessMode::Read, AccessMode::Write])
            } else {
                std::collections::BTreeSet::from([AccessMode::Read])
            };
            for root in [&scope.requested, &scope.physical] {
                resources
                    .filesystem
                    .push(FilesystemScope::new(root.clone(), access.clone())?);
            }
        }
        let claim = context.authority();
        let identity = serde_json::to_vec(&(context.actor(), binding))?;
        let request = AuthorityRequest {
            decision: DecisionId::new(format!("project-task:{}", blake3::hash(&identity)))?,
            actor: context.actor().clone(),
            grant: claim.grant().clone(),
            grant_revision: claim.grant_revision(),
            grant_digest: claim.grant_digest().clone(),
            revocation_generation: claim.revocation_generation(),
            operation: AuthorityOperation::InspectProject,
            resources,
            budget: AuthorityBudget::default(),
            evaluated_at: BoundaryTimeMillis::new(self.clock.now()?.get()),
            provenance: AuthorityExecutionProvenance::default(),
        };
        let decision = self.authority.evaluate(&request)?;
        if !decision.is_allowed() || decision.request() != &request {
            return Err(ControlError::AuthorizationDenied {
                reasons: decision.reason_codes().to_vec(),
                decision_digest: Some(decision.digest().to_owned()),
            });
        }
        Ok(())
    }
}

pub(super) fn resolve_definition(
    definition: &mut TaskDefinition,
    project: Option<&ProjectRecord>,
    previous: Option<&TaskDefinition>,
) -> Result<(), ControlError> {
    if let Some(previous) = previous
        && definition.execution == previous.execution
        && definition.paths == previous.paths
        && definition.tools == previous.tools
    {
        return Ok(());
    }
    if matches!(definition.execution, TaskExecution::Bound { .. }) {
        return Err(invalid(
            "accepted execution bindings can only be retained unchanged; select reviewed project or explicit settings to change execution",
        ));
    }
    if !definition.execution.is_legacy() && !definition.tools.is_empty() {
        return Err(invalid(
            "project execution uses its typed tool selection; remove the separate legacy tool field",
        ));
    }
    if matches!(definition.execution, TaskExecution::Project { .. }) && !definition.paths.is_empty()
    {
        return Err(invalid(
            "inherited settings claim the project root; choose explicit settings to narrow file scope",
        ));
    }
    let Some(project) = project else {
        return if definition.execution.is_legacy() {
            Ok(())
        } else {
            Err(invalid(
                "associate this workflow with a project before selecting project settings",
            ))
        };
    };
    let (expected, settings, access, mut tools) = match &definition.execution {
        TaskExecution::Project {
            expected_generation,
        } => {
            let settings = project.settings.clone();
            let access = if settings.allow_changes {
                TaskProjectAccess::Write
            } else {
                TaskProjectAccess::Read
            };
            let mut tools = vec![TaskTool::ReadFile];
            if settings.allow_changes {
                tools.push(TaskTool::WriteFile);
            }
            if let Some(tool) = process_tool(&settings.environment)? {
                tools.push(tool);
            }
            (*expected_generation, settings, access, tools)
        }
        TaskExecution::Explicit {
            expected_generation,
            settings,
            access,
            tools,
        } => (
            *expected_generation,
            settings.clone(),
            *access,
            tools.clone(),
        ),
        TaskExecution::Legacy => {
            return Err(invalid(
                "new project work must explicitly use the reviewed project settings or complete task settings",
            ));
        }
        TaskExecution::Bound { .. } => {
            return Err(invalid(
                "new task cannot assert an accepted execution binding",
            ));
        }
    };
    if expected != project.generation {
        return Err(ControlError::Persistence(
            milkdrift_persistence::PersistenceError::ProjectGenerationConflict {
                expected,
                actual: project.generation,
            },
        ));
    }
    if access == TaskProjectAccess::Write && !settings.allow_changes {
        return Err(invalid(
            "writable task access requires explicit project changes consent",
        ));
    }
    for tool in &tools {
        match tool {
            TaskTool::ReadFile if access == TaskProjectAccess::None => {
                return Err(invalid("file tools require project access"));
            }
            TaskTool::WriteFile if access != TaskProjectAccess::Write => {
                return Err(invalid("file writes require writable project access"));
            }
            TaskTool::Process { .. } if access == TaskProjectAccess::None => {
                return Err(invalid("process tools require project access"));
            }
            TaskTool::Process { .. }
                if Some(tool) != process_tool(&settings.environment)?.as_ref() =>
            {
                return Err(invalid(
                    "process tool differs from the selected project environment",
                ));
            }
            TaskTool::Process { .. }
                if matches!(settings.environment, ProjectEnvironment::Process { .. })
                    && access != TaskProjectAccess::Write =>
            {
                return Err(invalid(
                    "trusted host processes require writable project access",
                ));
            }
            _ => {}
        }
    }
    // A process selected as the task executor consumes the binding itself, without a model loop.
    if process_tool(&settings.environment)?.as_ref().is_some_and(|tool| {
        matches!(tool, TaskTool::Process { capability } if capability == &definition.capability)
    }) {
        if matches!(definition.execution, TaskExecution::Explicit { .. }) && !tools.is_empty() {
            return Err(invalid(
                "direct process tasks cannot use a model tool catalog; select an explicit empty tool list",
            ));
        }
        tools.clear();
    }
    let whole_project = match access {
        TaskProjectAccess::None => Vec::new(),
        TaskProjectAccess::Read | TaskProjectAccess::Write => vec![TaskPathAccess {
            path: project.folder.requested.to_string_lossy().into_owned(),
            write: access == TaskProjectAccess::Write,
        }],
    };
    if !definition.paths.is_empty() {
        if access == TaskProjectAccess::None
            || definition
                .paths
                .iter()
                .any(|path| path.write && access != TaskProjectAccess::Write)
        {
            return Err(invalid(
                "explicit native scope exceeds the task's selected project access",
            ));
        }
        if (tools.iter().any(|tool| matches!(tool, TaskTool::Process { .. }))
            || process_tool(&settings.environment)?.as_ref().is_some_and(|tool| matches!(tool, TaskTool::Process { capability } if capability == &definition.capability)))
            && definition.paths != whole_project
        {
            return Err(invalid("process tools require the whole selected project directory; use file-only tools for narrower research scopes"));
        }
    } else {
        definition.paths = whole_project;
    }
    definition.tools.clone_from(&tools);
    definition.execution = TaskExecution::Bound {
        binding: TaskProjectBinding {
            paths: definition.paths.clone(),
            scopes: Vec::new(),
            project_id: project.id.clone(),
            generation: project.generation,
            folder: TaskProjectFolder {
                requested: project.folder.requested.to_string_lossy().into_owned(),
                physical: project.folder.physical.to_string_lossy().into_owned(),
                identity: project.folder.identity.clone(),
            },
            settings,
            access,
            tools,
        },
    };
    Ok(())
}

fn process_tool(environment: &ProjectEnvironment) -> Result<Option<TaskTool>, ControlError> {
    match environment {
        ProjectEnvironment::Host => Ok(None),
        ProjectEnvironment::Process {
            capability,
            operation,
            generation,
        }
        | ProjectEnvironment::Podman {
            capability,
            operation,
            generation,
        } => {
            if *generation == 0 {
                return Err(invalid("project environment generation must be nonzero"));
            }
            Ok(Some(TaskTool::Process {
                capability: CapabilityRequirement::new(
                    OperationId::new(operation).map_err(|error| invalid(error.to_string()))?,
                )
                .exact(CapabilityId::new(capability).map_err(|error| invalid(error.to_string()))?),
            }))
        }
    }
}

fn invalid(message: impl Into<String>) -> ControlError {
    ControlError::InvalidContract(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use milkdrift_blueprint::ProjectSettings;
    use milkdrift_persistence::{DaemonDisplayName, NativePathRoot};

    fn project() -> Result<ProjectRecord, Box<dyn std::error::Error>> {
        Ok(ProjectRecord {
            id: "project:settings".into(),
            name: DaemonDisplayName::new("Project")?,
            generation: 4,
            folder: NativePathRoot {
                requested: "/projects/work".into(),
                physical: "/projects/work".into(),
                identity: "unix:1:2".into(),
                write: true,
            },
            settings: ProjectSettings {
                environment: ProjectEnvironment::Host,
                allow_changes: true,
            },
        })
    }
    fn definition(execution: TaskExecution) -> Result<TaskDefinition, Box<dyn std::error::Error>> {
        Ok(TaskDefinition {
            capability_generation: None,
            execution,
            actions: None,
            title: "Inspect".into(),
            description: String::new(),
            instructions: "Inspect the project".into(),
            capability: CapabilityRequirement::new(OperationId::new("model.generate")?)
                .exact(CapabilityId::new("model:fixture")?),
            inputs: Default::default(),
            outputs: Default::default(),
            dependencies: Default::default(),
            required: true,
            corrective_of: None,
            paths: Vec::new(),
            tools: Vec::new(),
        })
    }

    #[test]
    fn inherited_choices_are_frozen_and_text_edits_retain_them()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut project = project()?;
        let mut original = definition(TaskExecution::Project {
            expected_generation: 4,
        })?;
        resolve_definition(&mut original, Some(&project), None)?;
        assert_eq!(
            original.paths,
            vec![TaskPathAccess {
                path: "/projects/work".into(),
                write: true
            }]
        );
        assert_eq!(
            original.tools,
            vec![TaskTool::ReadFile, TaskTool::WriteFile]
        );
        let mut edited = original.clone();
        edited.title = "Changed title".into();
        edited.instructions = "Changed instructions".into();
        project.generation = 5;
        project.settings.allow_changes = false;
        resolve_definition(&mut edited, Some(&project), Some(&original))?;
        assert_eq!(edited.execution, original.execution);
        assert_eq!(edited.paths, original.paths);
        assert_eq!(edited.tools, original.tools);
        let mut stale = definition(TaskExecution::Project {
            expected_generation: 4,
        })?;
        assert!(resolve_definition(&mut stale, Some(&project), None).is_err());
        Ok(())
    }

    #[test]
    fn explicit_empty_permissions_remain_empty_after_defaults_change()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut project = project()?;
        let mut original = definition(TaskExecution::Explicit {
            expected_generation: 4,
            settings: project.settings.clone(),
            access: TaskProjectAccess::None,
            tools: Vec::new(),
        })?;
        resolve_definition(&mut original, Some(&project), None)?;
        assert!(original.paths.is_empty());
        assert!(original.tools.is_empty());
        project.generation = 5;
        let mut edited = original.clone();
        edited.title = "Still no tools".into();
        resolve_definition(&mut edited, Some(&project), Some(&original))?;
        assert_eq!(edited.execution, original.execution);
        let mut forged = original;
        assert!(resolve_definition(&mut forged, Some(&project), None).is_err());
        Ok(())
    }

    #[test]
    fn coupled_overrides_refuse_readonly_host_process_and_wrong_tools()
    -> Result<(), Box<dyn std::error::Error>> {
        let project = project()?;
        let settings = ProjectSettings {
            environment: ProjectEnvironment::Process {
                capability: "cargo".into(),
                operation: "process.execute".into(),
                generation: 1,
            },
            allow_changes: false,
        };
        let mut task = definition(TaskExecution::Explicit {
            expected_generation: 4,
            tools: vec![process_tool(&settings.environment)?.ok_or("process missing")?],
            settings,
            access: TaskProjectAccess::Read,
        })?;
        assert!(resolve_definition(&mut task, Some(&project), None).is_err());
        let mut task = definition(TaskExecution::Explicit {
            expected_generation: 4,
            settings: project.settings.clone(),
            access: TaskProjectAccess::Read,
            tools: vec![TaskTool::WriteFile],
        })?;
        assert!(resolve_definition(&mut task, Some(&project), None).is_err());
        Ok(())
    }

    #[test]
    fn file_only_scopes_preserve_separate_prepared_research_directories()
    -> Result<(), Box<dyn std::error::Error>> {
        let project = project()?;
        let mut task = definition(TaskExecution::Explicit {
            expected_generation: 4,
            settings: project.settings.clone(),
            access: TaskProjectAccess::Write,
            tools: vec![TaskTool::ReadFile, TaskTool::WriteFile],
        })?;
        task.paths = vec![TaskPathAccess {
            path: "/projects/work/research-a".into(),
            write: true,
        }];
        resolve_definition(&mut task, Some(&project), None)?;
        assert_eq!(
            task.paths.first().ok_or("scope absent")?.path,
            "/projects/work/research-a"
        );
        assert_eq!(
            task.execution.binding().ok_or("binding missing")?.paths,
            task.paths
        );
        let mut legacy = definition(TaskExecution::Legacy)?;
        assert!(resolve_definition(&mut legacy, Some(&project), None).is_err());
        Ok(())
    }

    #[test]
    fn direct_process_does_not_silently_discard_an_explicit_tool_catalog()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut project = project()?;
        project.settings.environment = ProjectEnvironment::Process {
            capability: "cargo".into(),
            operation: "process.execute".into(),
            generation: 1,
        };
        let Some(TaskTool::Process { capability }) = process_tool(&project.settings.environment)?
        else {
            return Err("process capability absent".into());
        };
        let mut explicit = definition(TaskExecution::Explicit {
            expected_generation: 4,
            settings: project.settings.clone(),
            access: TaskProjectAccess::Write,
            tools: vec![TaskTool::WriteFile],
        })?;
        explicit.capability = capability.clone();
        assert!(resolve_definition(&mut explicit, Some(&project), None).is_err());
        let mut inherited = definition(TaskExecution::Project {
            expected_generation: 4,
        })?;
        inherited.capability = capability;
        resolve_definition(&mut inherited, Some(&project), None)?;
        assert!(inherited.tools.is_empty());
        assert_eq!(
            inherited
                .execution
                .binding()
                .ok_or("binding absent")?
                .access,
            TaskProjectAccess::Write
        );
        Ok(())
    }
}
