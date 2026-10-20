//! Adapter-facing execution facts keep workflow definitions behind the host boundary.
use super::AdapterExecutionContext;
use crate::AdapterError;
use milkdrift_blueprint::{ProjectEnvironment, TaskProjectAccess, WorkflowId};

/// Process containment selected by one configured adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectProcessKind {
    /// A trusted native process uses the accepted host working directory.
    Native,
    /// A managed Podman process attaches the accepted directory.
    Podman,
}

/// Validated process access projected from one immutable task acceptance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectProcessAccess<'a> {
    /// No project file access was accepted.
    Isolated,
    /// One whole project folder is available at the selected access level.
    Directory {
        /// Original operator-selected path, retained for alias revalidation.
        requested: &'a str,
        /// Canonical native path captured at acceptance.
        physical: &'a str,
        /// Native filesystem identity captured at acceptance.
        identity: &'a str,
        /// Whether both task scope and project policy permit changes.
        write: bool,
    },
}

impl AdapterExecutionContext {
    /// Validate the selected process generation and project scope without exposing task definitions.
    /// Legacy invocations without a project return `None`.
    ///
    /// # Errors
    /// Refuses changed native identities, changed environments, narrower process scopes, missing
    /// write policy or a native process that cannot enforce the accepted read-only scope.
    pub fn project_process(
        &self,
        kind: ProjectProcessKind,
        capability: &str,
        operation: &str,
        generation: u64,
    ) -> Result<Option<ProjectProcessAccess<'_>>, AdapterError> {
        let Some(binding) = &self.project_binding else {
            return Ok(None);
        };
        self.verify_project()?;
        if binding.access == TaskProjectAccess::None {
            if !binding.paths.is_empty() || !binding.scopes.is_empty() || !binding.tools.is_empty()
            {
                return Err(AdapterError::rejected(
                    "isolated project task cannot carry file or tool scopes",
                ));
            }
            return Ok(Some(ProjectProcessAccess::Isolated));
        }
        let environment_matches = match (&binding.settings.environment, kind) {
            (
                ProjectEnvironment::Process {
                    capability: selected,
                    operation: selected_operation,
                    generation: selected_generation,
                },
                ProjectProcessKind::Native,
            )
            | (
                ProjectEnvironment::Podman {
                    capability: selected,
                    operation: selected_operation,
                    generation: selected_generation,
                },
                ProjectProcessKind::Podman,
            ) => {
                selected == capability
                    && selected_operation == operation
                    && *selected_generation == generation
            }
            _ => false,
        };
        let write = binding.access == TaskProjectAccess::Write;
        if !environment_matches
            || (write && !binding.settings.allow_changes)
            || (kind == ProjectProcessKind::Native && !write)
            || !matches!(binding.paths.as_slice(), [path]
                if path.path == binding.folder.requested && path.write == write)
            || !matches!(binding.scopes.as_slice(), [scope] if scope == &binding.folder)
        {
            return Err(AdapterError::rejected(
                "process requires its selected generation, exact whole-folder scope and permitted access",
            ));
        }
        Ok(Some(ProjectProcessAccess::Directory {
            requested: &binding.folder.requested,
            physical: &binding.folder.physical,
            identity: &binding.folder.identity,
            write,
        }))
    }

    /// Revalidate a task's accepted native folder and narrowed file scopes before external entry.
    ///
    /// # Errors
    /// Refuses missing or replaced paths; legacy work without a project has no project check.
    pub fn verify_project(&self) -> Result<(), AdapterError> {
        if let Some(binding) = &self.project_binding {
            crate::verify_project_binding(binding)
                .map_err(|error| AdapterError::rejected(error.to_string()))?;
        }
        Ok(())
    }

    /// Convert the runtime-certified task origin to the durable workflow identity used by stores.
    ///
    /// # Errors
    /// Refuses independent serving work without a task origin or an invalid identity.
    pub fn task_workflow_id(&self) -> Result<WorkflowId, AdapterError> {
        let workflow = self
            .workflow()
            .ok_or_else(|| AdapterError::rejected("entered task workflow is absent"))?;
        WorkflowId::new(workflow.run().as_str())
            .map_err(|error| AdapterError::rejected(error.to_string()))
    }
}
