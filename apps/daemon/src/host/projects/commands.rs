//! Exact project mutation admission and the filesystem effect between durable intent and settlement.
use super::{
    ActorSession, Owner, PublicFailure, domain_settings, invalid, public_receipt, revalidate,
};
use crate::host::read_model;
use milkdrift_authority::AuthorityOperation;
use milkdrift_blueprint::WorkflowId;
use milkdrift_control_protocol::{
    ErrorCode,
    projects::{ProjectCommandDto, ProjectCommandRequest, ProjectReceiptDto},
};
use milkdrift_persistence::{
    DaemonDisplayName, NativePathRoot, ProjectMutation, ProjectOperation, ProjectOperationState,
    ProjectRecord, ProjectStore,
};
use std::path::Path;

impl Owner {
    pub(in crate::host) fn project_command(
        &self,
        session: &ActorSession,
        request: ProjectCommandRequest,
        canonical_request: Vec<u8>,
    ) -> Result<ProjectReceiptDto, PublicFailure> {
        if let Some(existing) = self
            .store
            .project_operation(&session.actor, &request.request_id)
            .map_err(read_model::public_persistence)?
        {
            self.authorize_operation_read(session, &existing)?;
            if existing.canonical_request != canonical_request {
                return Err(PublicFailure::new(
                    ErrorCode::Conflict,
                    "project request identity already binds different content",
                    false,
                ));
            }
            return Ok(public_receipt(existing));
        }
        let hash = blake3::hash(
            &serde_json::to_vec(&(&self.host_id, &session.actor, &request.request_id))
                .map_err(|_| read_model::internal())?,
        )
        .to_string();
        let mut operation = ProjectOperation {
            id: format!("project-operation:{hash}"),
            actor: session.actor.clone(),
            key: request.request_id,
            canonical_request,
            state: ProjectOperationState::Accepted {
                project: None,
                workflow: None,
            },
        };
        let mutation = match request.command {
            ProjectCommandDto::Attach {
                name,
                folder,
                settings,
            } => {
                self.validate_project_settings(session, &settings)?;
                let folder = self.capture_project(
                    session,
                    Path::new(&folder),
                    AuthorityOperation::ManageProject,
                    settings.allow_changes,
                )?;
                self.dedicated_project_folder(&folder)?;
                ProjectMutation::Attach(ProjectRecord {
                    id: format!("project:{hash}"),
                    name: DaemonDisplayName::new(name).map_err(read_model::public_persistence)?,
                    generation: 1,
                    folder,
                    settings: domain_settings(settings),
                })
            }
            ProjectCommandDto::Create {
                name,
                parent,
                folder_name,
                settings,
            } => {
                valid_child(&folder_name)?;
                self.validate_project_settings(session, &settings)?;
                let parent = self.capture_project(
                    session,
                    Path::new(&parent),
                    AuthorityOperation::ManageProject,
                    true,
                )?;
                let folder = NativePathRoot {
                    requested: parent.requested.join(&folder_name),
                    physical: parent.physical.join(&folder_name),
                    identity: "unresolved-creation".into(),
                    write: settings.allow_changes,
                };
                self.dedicated_project_folder(&folder)?;
                let project = ProjectRecord {
                    id: format!("project:{hash}"),
                    name: DaemonDisplayName::new(name).map_err(read_model::public_persistence)?,
                    generation: 1,
                    folder,
                    settings: domain_settings(settings),
                };
                operation.state = ProjectOperationState::Creating {
                    parent,
                    folder_name,
                    project,
                };
                let intent = self
                    .store
                    .mutate_project(operation, ProjectMutation::BeginCreate)
                    .map_err(read_model::public_persistence)?;
                // Only this fresh reservation crosses the filesystem boundary. A replay/restart
                // returns the intent and requires explicit reconciliation; it never retries mkdir.
                let ProjectOperationState::Creating {
                    parent,
                    folder_name,
                    project,
                } = &intent.state
                else {
                    return Ok(public_receipt(intent));
                };
                let created = create_child(parent, folder_name, project.folder.write);
                return match created {
                    Ok(folder) => self
                        .store
                        .settle_project_creation(&session.actor, &intent.key, Some(folder))
                        .map(public_receipt)
                        .map_err(read_model::public_persistence),
                    Err(_) => Ok(public_receipt(intent)),
                };
            }
            ProjectCommandDto::Rename {
                project_id,
                expected_generation,
                name,
            } => {
                let mut project = self.authorized_project(
                    session,
                    &project_id,
                    AuthorityOperation::ManageProject,
                )?;
                project.name =
                    DaemonDisplayName::new(name).map_err(read_model::public_persistence)?;
                project.generation = expected_generation
                    .checked_add(1)
                    .ok_or_else(|| invalid("project generation exhausted"))?;
                ProjectMutation::Update {
                    project,
                    expected_generation,
                }
            }
            ProjectCommandDto::Settings {
                project_id,
                expected_generation,
                settings,
            } => {
                let mut project = self.authorized_project(
                    session,
                    &project_id,
                    AuthorityOperation::ManageProject,
                )?;
                revalidate(&project.folder)?;
                self.validate_project_settings(session, &settings)?;
                self.authorize_project_root(
                    session,
                    &project.folder,
                    AuthorityOperation::ManageProject,
                    settings.allow_changes,
                )?;
                project.settings = domain_settings(settings);
                project.generation = expected_generation
                    .checked_add(1)
                    .ok_or_else(|| invalid("project generation exhausted"))?;
                ProjectMutation::Update {
                    project,
                    expected_generation,
                }
            }
            ProjectCommandDto::Remove {
                project_id,
                expected_generation,
            } => {
                self.authorized_project(session, &project_id, AuthorityOperation::ManageProject)?;
                ProjectMutation::Remove {
                    id: project_id,
                    expected_generation,
                }
            }
            ProjectCommandDto::Associate {
                project_id,
                expected_generation,
                workflow_id,
            } => {
                self.authorized_project(session, &project_id, AuthorityOperation::ManageProject)?;
                let workflow = WorkflowId::new(workflow_id).map_err(|e| invalid(&e.to_string()))?;
                self.authorize_task_workflow(session, &workflow, AuthorityOperation::Apply)?;
                ProjectMutation::Associate {
                    id: project_id,
                    expected_generation,
                    workflow,
                }
            }
            ProjectCommandDto::Reconcile { operation_id } => {
                let original = self
                    .store
                    .project_operation(&session.actor, &operation_id)
                    .map_err(read_model::public_persistence)?
                    .ok_or_else(read_model::not_found)?;
                let ProjectOperationState::Creating {
                    parent,
                    folder_name,
                    project,
                } = original.state
                else {
                    return Err(invalid("creation is already resolved; recover its receipt"));
                };
                self.authorize_project_root(
                    session,
                    &parent,
                    AuthorityOperation::ManageProject,
                    true,
                )?;
                revalidate(&parent)?;
                let folder = self.capture_project(
                    session,
                    &parent.requested.join(folder_name),
                    AuthorityOperation::ManageProject,
                    project.settings.allow_changes,
                )?;
                self.dedicated_project_folder(&folder)?;
                if folder.physical.parent() != Some(parent.physical.as_path()) {
                    return Err(invalid("intended child escapes its pinned parent"));
                }
                ProjectMutation::Reconcile {
                    original_key: operation_id,
                    folder: Some(folder),
                }
            }
            ProjectCommandDto::Abandon { operation_id } => {
                let original = self
                    .store
                    .project_operation(&session.actor, &operation_id)
                    .map_err(read_model::public_persistence)?
                    .ok_or_else(read_model::not_found)?;
                let ProjectOperationState::Creating { parent, .. } = original.state else {
                    return Err(invalid("creation is already resolved; recover its receipt"));
                };
                self.authorize_project_root(
                    session,
                    &parent,
                    AuthorityOperation::ManageProject,
                    true,
                )?;
                ProjectMutation::Reconcile {
                    original_key: operation_id,
                    folder: None,
                }
            }
        };
        self.store
            .mutate_project(operation, mutation)
            .map(public_receipt)
            .map_err(read_model::public_persistence)
    }
}
fn valid_child(value: &str) -> Result<(), PublicFailure> {
    if value.is_empty()
        || value.len() > 255
        || matches!(value, "." | "..")
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
    {
        return Err(invalid(
            "Choose one new folder name without path separators or dot segments.",
        ));
    }
    Ok(())
}
#[cfg(target_os = "linux")]
fn create_child(
    parent: &NativePathRoot,
    name: &str,
    write: bool,
) -> Result<NativePathRoot, PublicFailure> {
    use rustix::fs::{Mode, OFlags, fstat, fsync, mkdirat, open, openat};
    let fail = |_: rustix::io::Errno| {
        invalid(
            "Directory creation is unresolved; inspect the intended folder and reconcile or abandon registration.",
        )
    };
    revalidate(parent)?;
    let directory = open(
        &parent.physical,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(fail)?;
    let stat = fstat(&directory).map_err(fail)?;
    if format!("unix:{}:{}", stat.st_dev, stat.st_ino) != parent.identity {
        return Err(invalid("Parent directory was replaced before creation."));
    }
    // mkdirat creates exactly one child relative to the pinned open parent. EEXIST is
    // unresolved, never permission to adopt another owner's existing contents.
    mkdirat(&directory, name, Mode::from_raw_mode(0o755)).map_err(fail)?;
    let child = openat(
        &directory,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(fail)?;
    let stat = fstat(&child).map_err(fail)?;
    fsync(&child).map_err(fail)?;
    fsync(&directory).map_err(fail)?;
    revalidate(parent)?;
    let folder =
        milkdrift_capability_host::capture_native_project_root(&parent.requested.join(name), write)
            .map_err(|_| {
                invalid(
                    "Created directory cannot be identified; explicit reconciliation is required.",
                )
            })?;
    if folder.identity != format!("unix:{}:{}", stat.st_dev, stat.st_ino)
        || folder.physical != parent.physical.join(name)
    {
        return Err(invalid(
            "Created directory changed before durable acceptance.",
        ));
    }
    Ok(folder)
}
#[cfg(not(target_os = "linux"))]
fn create_child(
    _parent: &NativePathRoot,
    _name: &str,
    _write: bool,
) -> Result<NativePathRoot, PublicFailure> {
    Err(invalid(
        "Exclusive project creation with pinned parents is not qualified on this platform.",
    ))
}
