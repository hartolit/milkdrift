//! Project registration is a daemon fact; clients only submit scoped commands and read projections.
mod commands;
mod directories;
mod environments;
use super::{DaemonHost, Owner, PublicFailure, read_model};
use crate::auth::ActorSession;
use milkdrift_authority::{
    AccessMode, AuthorityOperation, FilesystemScope, RequestedResourceFacts,
};
use milkdrift_blueprint::{ProjectEnvironment, ProjectSettings};
use milkdrift_capability_host::capture_native_project_root;
use milkdrift_control_protocol::{
    Cursor, ErrorCode,
    projects::{
        ProjectCommandRequest, ProjectDto, ProjectEnvironmentDto, ProjectPageDto,
        ProjectReceiptDto, ProjectSettingsDto,
    },
};
use milkdrift_persistence::{
    NativePathRoot, ProjectOperation, ProjectOperationState, ProjectRecord, ProjectStore,
};
use std::{collections::BTreeSet, path::Path};

pub(crate) enum ProjectRequest {
    List {
        cursor: Option<String>,
        limit: u16,
    },
    Read(String),
    Directories {
        parent: Option<String>,
        cursor: Option<String>,
        limit: u16,
    },
    Environments,
    Command {
        request: ProjectCommandRequest,
        canonical_request: Vec<u8>,
    },
    Receipt(String),
    Operations,
}
impl DaemonHost {
    pub(crate) async fn project_request(
        &self,
        session: ActorSession,
        expected: Option<String>,
        request: ProjectRequest,
    ) -> Result<serde_json::Value, PublicFailure> {
        if matches!(&request, ProjectRequest::Command { .. }) && !self.accepting_mutations() {
            return Err(PublicFailure::new(
                ErrorCode::Unavailable,
                "daemon is draining",
                true,
            ));
        }
        self.dispatch(false, move |owner| {
            owner.check_daemon_identity(expected.as_deref())?;
            match request {
                ProjectRequest::List { cursor, limit } => {
                    serde_json::to_value(owner.project_page(&session, cursor.as_deref(), limit)?)
                }
                ProjectRequest::Read(id) => {
                    serde_json::to_value(owner.project_read(&session, &id)?)
                }
                ProjectRequest::Directories {
                    parent,
                    cursor,
                    limit,
                } => serde_json::to_value(owner.project_directories(
                    &session,
                    parent.as_deref(),
                    cursor.as_deref(),
                    limit,
                )?),
                ProjectRequest::Environments => {
                    serde_json::to_value(owner.project_environments(&session)?)
                }
                ProjectRequest::Command {
                    request,
                    canonical_request,
                } => serde_json::to_value(owner.project_command(
                    &session,
                    request,
                    canonical_request,
                )?),
                ProjectRequest::Receipt(key) => {
                    let op = owner
                        .store
                        .project_operation(&session.actor, &key)
                        .map_err(read_model::public_persistence)?
                        .ok_or_else(read_model::not_found)?;
                    owner.authorize_operation_read(&session, &op)?;
                    serde_json::to_value(public_receipt(op))
                }
                ProjectRequest::Operations => {
                    let mut result = Vec::new();
                    for op in owner
                        .store
                        .project_operations(&session.actor)
                        .map_err(read_model::public_persistence)?
                    {
                        if owner.authorize_operation_read(&session, &op).is_ok() {
                            result.push(public_receipt(op));
                        }
                    }
                    serde_json::to_value(result)
                }
            }
            .map_err(|_| read_model::internal())
        })
        .await
    }
}
impl Owner {
    pub(in crate::host) fn authorized_project(
        &self,
        session: &ActorSession,
        id: &str,
        operation: AuthorityOperation,
    ) -> Result<ProjectRecord, PublicFailure> {
        let project = self
            .store
            .project(id)
            .map_err(read_model::public_persistence)?
            .ok_or_else(read_model::not_found)?;
        // All public facts, including existence, are filtered by the pinned directory scope.
        self.authorize_project_root(session, &project.folder, operation, false)
            .map_err(|_| read_model::not_found())?;
        Ok(project)
    }
    pub(in crate::host) fn require_project_available(
        &self,
        session: &ActorSession,
        id: &str,
    ) -> Result<ProjectRecord, PublicFailure> {
        let project = self.authorized_project(session, id, AuthorityOperation::InspectProject)?;
        revalidate(&project.folder)?;
        self.dedicated_project_folder(&project.folder)?;
        Ok(project)
    }
    fn project_read(&self, session: &ActorSession, id: &str) -> Result<ProjectDto, PublicFailure> {
        let project = self.authorized_project(session, id, AuthorityOperation::InspectProject)?;
        Ok(self.project_view(session, project))
    }
    fn project_view(&self, session: &ActorSession, project: ProjectRecord) -> ProjectDto {
        let availability = if revalidate(&project.folder).is_ok()
            && self.dedicated_project_folder(&project.folder).is_ok()
        {
            "available"
        } else {
            "Folder is missing, moved, replaced or overlaps protected operator files. New work is unavailable."
        }
        .to_owned();
        let can_manage = self
            .authorize_project_root(
                session,
                &project.folder,
                AuthorityOperation::ManageProject,
                false,
            )
            .is_ok();
        project_dto(project, availability, can_manage)
    }
    fn project_page(
        &self,
        session: &ActorSession,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<ProjectPageDto, PublicFailure> {
        checked_limit(limit)?;
        self.authorize(
            session,
            AuthorityOperation::InspectProject,
            RequestedResourceFacts::empty(),
            "projects:list",
        )?;
        let feed = format!("projects:{}", self.host_id);
        let mut after = decode_cursor(session, &feed, cursor)?;
        let mut items = Vec::new();
        // Scan at most the complete bounded catalog; only authorized rows affect returned pages.
        loop {
            let rows = self
                .store
                .projects(after.as_deref(), 200)
                .map_err(read_model::public_persistence)?;
            if rows.is_empty() {
                return Ok(ProjectPageDto {
                    items,
                    next_cursor: None,
                });
            }
            for project in rows {
                after = Some(project.id.clone());
                if self
                    .authorize_project_root(
                        session,
                        &project.folder,
                        AuthorityOperation::InspectProject,
                        false,
                    )
                    .is_err()
                {
                    continue;
                }
                if items.len() == usize::from(limit) {
                    // Return a cursor to the final disclosed record, never a hidden record identity.
                    let last = items
                        .last()
                        .map(|p: &ProjectDto| p.id.as_str())
                        .ok_or_else(read_model::internal)?;
                    return Ok(ProjectPageDto {
                        next_cursor: Some(encode_cursor(session, &feed, last)?),
                        items,
                    });
                }
                items.push(self.project_view(session, project));
            }
        }
    }
    fn authorize_project_path(
        &self,
        session: &ActorSession,
        path: &Path,
        operation: AuthorityOperation,
        write: bool,
    ) -> Result<(), PublicFailure> {
        let path = path
            .to_str()
            .ok_or_else(|| invalid("project paths must be UTF-8"))?;
        let mut resources = RequestedResourceFacts::empty();
        let mut modes = BTreeSet::from([AccessMode::Read]);
        if write {
            modes.insert(AccessMode::Write);
        }
        resources
            .filesystem
            .push(FilesystemScope::new(path, modes).map_err(|e| invalid(&e.to_string()))?);
        self.authorize(session, operation, resources, "project:filesystem")?;
        Ok(())
    }
    fn authorize_project_root(
        &self,
        session: &ActorSession,
        root: &NativePathRoot,
        operation: AuthorityOperation,
        write: bool,
    ) -> Result<(), PublicFailure> {
        self.authorize_project_path(session, &root.requested, operation, write)?;
        self.authorize_project_path(session, &root.physical, operation, write)
    }
    fn capture_project(
        &self,
        session: &ActorSession,
        path: &Path,
        operation: AuthorityOperation,
        write: bool,
    ) -> Result<NativePathRoot, PublicFailure> {
        self.authorize_project_path(session, path, operation, write)?;
        let captured =
            capture_native_project_root(path, write).map_err(|e| invalid(&e.to_string()))?;
        if captured.physical.starts_with(&self.project_protected_root) {
            return Err(invalid(
                "Choose a dedicated project folder outside the daemon's protected storage.",
            ));
        }
        self.authorize_project_root(session, &captured, operation, write)?;
        Ok(captured)
    }
    fn dedicated_project_folder(&self, root: &NativePathRoot) -> Result<(), PublicFailure> {
        if root.physical.parent().is_none()
            || self.project_protected_root.starts_with(&root.physical)
            || self
                .project_protected_paths
                .iter()
                .any(|path| path.starts_with(&root.physical) || root.physical.starts_with(path))
        {
            return Err(invalid(
                "Choose a dedicated project folder that does not expose daemon storage, configuration, credentials or operator executables.",
            ));
        }
        Ok(())
    }
    fn authorize_operation_read(
        &self,
        session: &ActorSession,
        operation: &ProjectOperation,
    ) -> Result<(), PublicFailure> {
        match &operation.state {
            ProjectOperationState::Creating { parent, .. } => self.authorize_project_root(
                session,
                parent,
                AuthorityOperation::InspectProject,
                false,
            ),
            ProjectOperationState::Accepted {
                project: Some(project),
                ..
            } => self.authorize_project_root(
                session,
                &project.folder,
                AuthorityOperation::InspectProject,
                false,
            ),
            ProjectOperationState::Accepted { project: None, .. }
            | ProjectOperationState::Abandoned => self
                .authorize(
                    session,
                    AuthorityOperation::InspectProject,
                    RequestedResourceFacts::empty(),
                    "project:receipt",
                )
                .map(|_| ()),
        }
    }
}
fn project_dto(project: ProjectRecord, availability: String, can_manage: bool) -> ProjectDto {
    ProjectDto {
        id: project.id,
        name: project.name.as_str().into(),
        generation: project.generation,
        folder: project.folder.requested.to_string_lossy().into_owned(),
        settings: public_settings(project.settings),
        availability,
        can_manage,
    }
}
fn public_receipt(operation: ProjectOperation) -> ProjectReceiptDto {
    let (project, workflow, status, operation_folder, operation_name) = match operation.state {
        ProjectOperationState::Accepted { project, workflow } => {
            (project, workflow, "accepted", None, None)
        }
        ProjectOperationState::Creating { project, .. } => (
            None,
            None,
            "unresolved",
            Some(project.folder.requested.to_string_lossy().into_owned()),
            Some(project.name.as_str().into()),
        ),
        ProjectOperationState::Abandoned => (None, None, "abandoned", None, None),
    };
    ProjectReceiptDto {
        operation_folder,
        operation_name,
        request_id: operation.key.clone(),
        project: project.map(|p| project_dto(p, "available".into(), true)),
        operation_id: (status == "unresolved").then_some(operation.key),
        status: status.into(),
        workflow_id: workflow.map(|w| w.to_string()),
    }
}
pub(in crate::host) fn domain_settings(settings: ProjectSettingsDto) -> ProjectSettings {
    ProjectSettings {
        allow_changes: settings.allow_changes,
        environment: match settings.environment {
            ProjectEnvironmentDto::Host => ProjectEnvironment::Host,
            ProjectEnvironmentDto::Process {
                capability,
                operation,
                generation,
            } => ProjectEnvironment::Process {
                capability,
                operation,
                generation,
            },
            ProjectEnvironmentDto::Podman {
                capability,
                operation,
                generation,
            } => ProjectEnvironment::Podman {
                capability,
                operation,
                generation,
            },
        },
    }
}
pub(in crate::host) fn public_settings(settings: ProjectSettings) -> ProjectSettingsDto {
    ProjectSettingsDto {
        allow_changes: settings.allow_changes,
        environment: match settings.environment {
            ProjectEnvironment::Host => ProjectEnvironmentDto::Host,
            ProjectEnvironment::Process {
                capability,
                operation,
                generation,
            } => ProjectEnvironmentDto::Process {
                capability,
                operation,
                generation,
            },
            ProjectEnvironment::Podman {
                capability,
                operation,
                generation,
            } => ProjectEnvironmentDto::Podman {
                capability,
                operation,
                generation,
            },
        },
    }
}
fn revalidate(root: &NativePathRoot) -> Result<(), PublicFailure> {
    let current=capture_native_project_root(&root.requested,root.write).map_err(|_|invalid("Project folder is unavailable; restore its original location and identity or attach another folder explicitly."))?;
    if &current != root {
        return Err(invalid(
            "Project folder was moved or replaced; new work is refused.",
        ));
    }
    Ok(())
}
fn invalid(message: &str) -> PublicFailure {
    PublicFailure::new(ErrorCode::InvalidInput, message, false)
}
fn checked_limit(limit: u16) -> Result<(), PublicFailure> {
    if limit == 0 || limit > 200 {
        Err(invalid("page limit must be 1..=200"))
    } else {
        Ok(())
    }
}
fn decode_cursor(
    session: &ActorSession,
    feed: &str,
    cursor: Option<&str>,
) -> Result<Option<String>, PublicFailure> {
    cursor
        .map(|cursor| {
            let cursor: Cursor = serde_json::from_value(serde_json::Value::String(cursor.into()))
                .map_err(|_| invalid("invalid project cursor"))?;
            cursor
                .key_for_bound(feed, &session.cursor_binding(feed), session.cursor_key())
                .map_err(|_| invalid("project cursor is not valid for this actor and query"))
        })
        .transpose()
}
fn encode_cursor(session: &ActorSession, feed: &str, key: &str) -> Result<String, PublicFailure> {
    let binding = session.cursor_binding(feed);
    let cursor = Cursor::new_bound_key(
        feed,
        key,
        binding.clone(),
        &binding.grant_digest,
        session.cursor_key(),
    )
    .map_err(|_| read_model::internal())?;
    serde_json::to_value(cursor)
        .map_err(|_| read_model::internal())?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(read_model::internal)
}
