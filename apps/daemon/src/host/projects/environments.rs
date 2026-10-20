//! Environment choices project existing configured capabilities; no independent registry is added.
use super::{ActorSession, Owner, PublicFailure, invalid};
use milkdrift_authority::{AuthorityOperation, RequestedResourceFacts};
use milkdrift_capability::{CapabilityCategory, Locality};
use milkdrift_control_protocol::projects::{
    ProjectEnvironmentChoiceDto, ProjectEnvironmentDto, ProjectEnvironmentPageDto,
    ProjectSettingsDto,
};
impl Owner {
    pub(super) fn project_environments(
        &self,
        session: &ActorSession,
    ) -> Result<ProjectEnvironmentPageDto, PublicFailure> {
        self.authorize(
            session,
            AuthorityOperation::ListCapabilities,
            RequestedResourceFacts::empty(),
            "projects:environments",
        )?;
        let views = self.capability_generations(session)?;
        let catalog = self
            .capability_host
            .catalog_generations(&session.grant.resources().capability)
            .map_err(|_| crate::host::read_model::internal())?;
        let mut items = vec![ProjectEnvironmentChoiceDto {
            environment: ProjectEnvironmentDto::Host,
            label: "Host file tools".into(),
            available: cfg!(target_os = "linux"),
            reason: (!cfg!(target_os = "linux"))
                .then(|| "Native project file tools require Linux.".into()),
        }];
        for view in views.into_iter().filter(|v| {
            v.current
                && !v.draining
                && v.locality == Locality::Local
                && v.category == CapabilityCategory::Process
        }) {
            let Some(descriptor) = catalog
                .iter()
                .find(|c| {
                    c.descriptor.identity() == &view.capability
                        && c.descriptor.descriptor_revision() == view.descriptor_revision
                })
                .map(|c| &c.descriptor)
            else {
                continue;
            };
            let extension = |key: &str| {
                descriptor
                    .extensions()
                    .iter()
                    .find(|(k, _)| k.as_str() == key)
                    .map(|(_, v)| v.value())
            };
            let podman = extension("milkdrift.managed/project-attachment")
                .is_some_and(|v| v.get("destination").and_then(|s| s.as_str()) == Some("/project"));
            let process = extension("org.milkdrift/process-profile").is_some_and(|v| {
                v.get("project_directory").and_then(|s| s.as_bool()) == Some(true)
            });
            if !podman && !process {
                continue;
            }
            for operation in view.operation_contracts.keys() {
                let environment = if podman {
                    ProjectEnvironmentDto::Podman {
                        capability: view.capability.to_string(),
                        operation: operation.to_string(),
                        generation: view.descriptor_revision,
                    }
                } else {
                    ProjectEnvironmentDto::Process {
                        capability: view.capability.to_string(),
                        operation: operation.to_string(),
                        generation: view.descriptor_revision,
                    }
                };
                let label = view
                    .task_authoring
                    .get(operation)
                    .map_or_else(|| view.capability.to_string(), |v| v.label.clone());
                let available = view.available == Some(true);
                let reason = if !available {
                    Some("The configured environment is unavailable; ask the daemon operator to restore it.".into())
                } else if podman {
                    Some("Worker generation configured. Project attachment also requires the daemon's Rust directory-pin helper and an OCI runtime that can bind its held directory descriptor. A runtime mount refusal stops the task; owned-volume readiness does not qualify project attachment.".into())
                } else {
                    None
                };
                items.push(ProjectEnvironmentChoiceDto {
                    environment,
                    label,
                    available,
                    reason,
                });
            }
        }
        Ok(ProjectEnvironmentPageDto { items })
    }
    pub(super) fn validate_project_settings(
        &self,
        session: &ActorSession,
        settings: &ProjectSettingsDto,
    ) -> Result<(), PublicFailure> {
        let choices = self.project_environments(session)?;
        if !choices
            .items
            .iter()
            .any(|choice| choice.environment == settings.environment && choice.available)
        {
            return Err(invalid(
                "This environment or reviewed generation is unavailable. Ask the daemon operator to configure a project-aware host process or attached Podman worker, then refresh the choices.",
            ));
        }
        if !settings.allow_changes
            && matches!(settings.environment, ProjectEnvironmentDto::Process { .. })
        {
            return Err(invalid(
                "Trusted host process tools can change project files; explicitly allow project changes or choose host file tools/read-only Podman.",
            ));
        }
        Ok(())
    }
}
