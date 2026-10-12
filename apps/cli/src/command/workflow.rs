//! Ordinary workflows use daemon discovery and receipts, without local recovery files.
use crate::{
    error::CliError,
    session::CliSession,
    workflow_args::{WorkflowArgs, WorkflowCommand},
};
use milkdrift_control_protocol::tasks::{
    ChangeWorkflowRequest, CreateWorkflowRequest, TaskChangeDto, TaskUploadRequest,
};

pub(crate) async fn execute(session: &CliSession, args: &WorkflowArgs) -> Result<(), CliError> {
    match &args.command {
        WorkflowCommand::List {
            search,
            limit,
            cursor,
        } => session.output(
            "workflow.list",
            &session
                .client()
                .workflows(search, cursor.as_deref(), *limit)
                .await?,
        ),
        WorkflowCommand::Show { workflow } => {
            session.output("workflow.show", &session.client().workflow(workflow).await?)
        }
        WorkflowCommand::Create { file } => {
            let request: CreateWorkflowRequest = serde_json::from_value(
                session
                    .read_json(
                        file,
                        milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                        "task request",
                    )
                    .await?,
            )
            .map_err(|e| CliError::Invalid(e.to_string()))?;
            session.output(
                "workflow.create",
                &session.client().create_workflow(&request).await?,
            )
        }
        WorkflowCommand::Edit { workflow, file } => {
            let request: ChangeWorkflowRequest = serde_json::from_value(
                session
                    .read_json(
                        file,
                        milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                        "task changes",
                    )
                    .await?,
            )
            .map_err(|e| CliError::Invalid(e.to_string()))?;
            session.output(
                "workflow.edit",
                &session.client().change_workflow(workflow, &request).await?,
            )
        }
        WorkflowCommand::Receipt { request } => session.output(
            "workflow.receipt",
            &session.client().workflow_receipt(request).await?,
        ),
        WorkflowCommand::Task { workflow, task } => session.output(
            "workflow.task",
            &session.client().task(workflow, task).await?,
        ),
        WorkflowCommand::Result { workflow, task } => session.output(
            "workflow.result",
            &session.client().task_result(workflow, task).await?,
        ),
        WorkflowCommand::Stop { workflow, task } => {
            let snapshot = session.client().workflow(workflow).await?;
            let selected = snapshot
                .tasks
                .iter()
                .find(|candidate| &candidate.id == task)
                .ok_or_else(|| CliError::Invalid("task is absent from this workflow".into()))?;
            session.output(
                "workflow.stop",
                &session
                    .client()
                    .change_workflow(
                        workflow,
                        &ChangeWorkflowRequest {
                            request_id: selected.stop_request_id.clone(),
                            expected_generation: snapshot.generation,
                            changes: vec![TaskChangeDto::Stop { task: task.clone() }],
                        },
                    )
                    .await?,
            )
        }
        WorkflowCommand::Upload { file, request } => {
            let bytes = session
                .read_bounded(file, 1_048_576, "task text file")
                .await?;
            let text = String::from_utf8(bytes)
                .map_err(|_| CliError::Invalid("task file must contain UTF-8 text".into()))?;
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| CliError::Invalid("input file requires a UTF-8 basename".into()))?;
            session.output(
                "workflow.upload",
                &session
                    .client()
                    .upload_task_input(&TaskUploadRequest {
                        request_id: request.clone(),
                        name: name.into(),
                        media_type: "text/plain".into(),
                        text,
                    })
                    .await?,
            )
        }
        WorkflowCommand::Capabilities => session.output(
            "workflow.capabilities",
            &session.client().task_session().await?,
        ),
    }
}
