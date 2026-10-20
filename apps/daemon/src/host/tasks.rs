//! Adapt public task commands and authorized projections to their semantic owners.
mod authoring;
mod commands;
mod execution_mapping;
mod mapping;
mod reads;

use super::{Owner, PublicFailure};
use crate::auth::ActorSession;
use milkdrift_authority::AuthorityOperation;
use milkdrift_blueprint::WorkflowId;
use milkdrift_control_protocol::ErrorCode;
use milkdrift_runtime::{RuntimeError, WorkflowRecord};

fn failure(error: impl std::fmt::Display) -> PublicFailure {
    PublicFailure::new(ErrorCode::InvalidInput, error.to_string(), false)
}

fn runtime_failure(error: RuntimeError) -> PublicFailure {
    match &error {
        RuntimeError::InvalidTransition(_) => {
            PublicFailure::new(ErrorCode::Conflict, error.to_string(), false)
        }
        RuntimeError::Persistence(
            milkdrift_persistence::PersistenceError::WorkflowGenerationConflict { .. }
            | milkdrift_persistence::PersistenceError::WorkflowIdempotencyConflict { .. },
        ) => PublicFailure::new(ErrorCode::Conflict, error.to_string(), false),
        _ => failure(error),
    }
}

impl Owner {
    fn task_workflow(
        &self,
        session: &ActorSession,
        workflow: &str,
        operation: AuthorityOperation,
    ) -> Result<WorkflowRecord, PublicFailure> {
        let id = WorkflowId::new(workflow).map_err(failure)?;
        self.authorize_task_workflow(session, &id, operation)?;
        self.workflow()?
            .tasks
            .workflow(&id)
            .map_err(runtime_failure)?
            .ok_or_else(super::read_model::not_found)
    }

    pub(in crate::host) fn authorize_task_workflow(
        &self,
        session: &ActorSession,
        id: &WorkflowId,
        operation: AuthorityOperation,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, PublicFailure> {
        self.workflow()?
            .task_control
            .authorize(&session.context, id, operation, "task-workflow")
            .map_err(super::read_model::public_control)
    }
}

/// Public operation adaptation enters the same bounded durable owner queue.
pub(crate) enum TaskRequest {
    Session,
    List {
        project_id: Option<String>,
        earlier: bool,
        search: String,
        cursor: Option<String>,
        limit: u16,
    },
    Workflow {
        workflow: String,
        cursor: Option<String>,
    },
    Task {
        workflow: String,
        task: String,
    },
    Result {
        workflow: String,
        task: String,
    },
    ResultContent {
        workflow: String,
        task: String,
    },
    Receipt(String),
    Create {
        request: milkdrift_control_protocol::tasks::CreateWorkflowRequest,
        canonical_request: Vec<u8>,
    },
    Change {
        workflow: String,
        request: milkdrift_control_protocol::tasks::ChangeWorkflowRequest,
        canonical_request: Vec<u8>,
    },
    Upload(milkdrift_control_protocol::tasks::TaskUploadRequest),
}

impl super::DaemonHost {
    pub(crate) async fn task_request(
        &self,
        session: ActorSession,
        expected_host: Option<String>,
        request: TaskRequest,
    ) -> Result<serde_json::Value, PublicFailure> {
        if matches!(
            &request,
            TaskRequest::Create { .. } | TaskRequest::Change { .. } | TaskRequest::Upload(_)
        ) && !self.accepting_mutations()
        {
            return Err(PublicFailure::new(
                ErrorCode::Unavailable,
                "daemon is draining",
                true,
            ));
        }
        self.dispatch(false, move |owner| {
            owner.check_daemon_identity(expected_host.as_deref())?;
            match request {
                TaskRequest::Session => serde_json::to_value(owner.task_session(&session)?),
                TaskRequest::List {
                    project_id,
                    earlier,
                    search,
                    cursor,
                    limit,
                } => serde_json::to_value(owner.task_workflows(
                    &session,
                    &search,
                    cursor.as_deref(),
                    limit,
                    project_id.as_deref(),
                    earlier,
                )?),
                TaskRequest::Workflow { workflow, cursor } => serde_json::to_value(
                    owner.read_task_workflow(&session, &workflow, cursor.as_deref())?,
                ),
                TaskRequest::Task { workflow, task } => {
                    serde_json::to_value(owner.read_task(&session, &workflow, &task)?)
                }
                TaskRequest::Result { workflow, task } => {
                    serde_json::to_value(owner.read_task_result(&session, &workflow, &task)?)
                }
                TaskRequest::ResultContent { workflow, task } => serde_json::to_value(
                    owner.read_task_result_content(&session, &workflow, &task)?,
                ),
                TaskRequest::Receipt(request) => {
                    serde_json::to_value(owner.task_request_receipt(&session, &request)?)
                }
                TaskRequest::Create {
                    request,
                    canonical_request,
                } => serde_json::to_value(owner.create_task_workflow(
                    &session,
                    request,
                    &canonical_request,
                )?),
                TaskRequest::Change {
                    workflow,
                    request,
                    canonical_request,
                } => serde_json::to_value(owner.change_task_workflow(
                    &session,
                    &workflow,
                    request,
                    &canonical_request,
                )?),
                TaskRequest::Upload(request) => {
                    serde_json::to_value(owner.upload_task_input(&session, request)?)
                }
            }
            .map_err(failure)
        })
        .await
    }
}
