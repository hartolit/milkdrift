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
            project,
            earlier,
        } => session.output(
            "workflow.list",
            &session
                .client()
                .workflows_in_project(
                    search,
                    cursor.as_deref(),
                    *limit,
                    project.as_deref(),
                    *earlier,
                )
                .await?,
        ),
        WorkflowCommand::Show { workflow, cursor } => session.output(
            "workflow.show",
            &session
                .client()
                .workflow_page(workflow, cursor.as_deref())
                .await?,
        ),
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
        WorkflowCommand::Authoring { workflow } => session.output(
            "workflow.authoring",
            &session.client().workflow_authoring(workflow).await?,
        ),
        WorkflowCommand::Positions { workflow } => session.output(
            "workflow.positions",
            &session.client().workflow_positions(workflow).await?,
        ),
        WorkflowCommand::SaveAuthoring { workflow, file } => {
            let request = read_authoring_request(session, file).await?;
            session.output(
                "workflow.authoring.save",
                &session
                    .client()
                    .checkpoint_workflow_authoring(workflow, &request)
                    .await?,
            )
        }
        WorkflowCommand::SavePositions { workflow, file } => {
            let request = read_authoring_request(session, file).await?;
            session.output(
                "workflow.positions.save",
                &session
                    .client()
                    .checkpoint_workflow_positions(workflow, &request)
                    .await?,
            )
        }
        WorkflowCommand::ReviewAuthoring { workflow, file } => {
            let request = read_authoring_request(session, file).await?;
            session.output(
                "workflow.authoring.review",
                &session
                    .client()
                    .review_workflow_authoring(workflow, &request)
                    .await?,
            )
        }
        WorkflowCommand::RunAuthoring { workflow, file } => {
            let request = read_authoring_request(session, file).await?;
            session.output(
                "workflow.authoring.run",
                &session
                    .client()
                    .run_workflow_authoring(workflow, &request)
                    .await?,
            )
        }
        WorkflowCommand::Edit {
            workflow,
            file,
            preview,
        } => {
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
            if *preview {
                preview_change(session, workflow, &request).await
            } else {
                session.output(
                    "workflow.edit",
                    &session.client().change_workflow(workflow, &request).await?,
                )
            }
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
            let selected = session.client().task(workflow, task).await?;
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
                .read_bounded(
                    file,
                    milkdrift_blueprint::MAX_TASK_TEXT_BYTES,
                    "task text file",
                )
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

async fn read_authoring_request<T: serde::de::DeserializeOwned>(
    session: &CliSession,
    file: &std::path::Path,
) -> Result<T, CliError> {
    serde_json::from_value(
        session
            .read_json(
                file,
                milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                "saved authoring request",
            )
            .await?,
    )
    .map_err(|error| CliError::Invalid(error.to_string()))
}

/// Inspection preserves every requested guard; only the daemon can decide admission.
async fn preview_change(
    session: &CliSession,
    workflow: &str,
    request: &ChangeWorkflowRequest,
) -> Result<(), CliError> {
    let snapshot = session.client().workflow(workflow).await?;
    let mut requested_versions = std::collections::BTreeSet::new();
    for change in &request.changes {
        match change {
            TaskChangeDto::Revise {
                task,
                expected_version,
                ..
            }
            | TaskChangeDto::Remove {
                task,
                expected_version,
            } => {
                requested_versions.insert((task.as_str(), *expected_version));
            }
            TaskChangeDto::AdoptCorrection { redirects, .. } => {
                requested_versions.extend(
                    redirects
                        .iter()
                        .map(|redirect| (redirect.consumer.as_str(), redirect.expected_version)),
                );
            }
            _ => {}
        }
    }
    let mut guards = Vec::new();
    for (id, expected_version) in requested_versions {
        let task = session.client().task(workflow, id).await?;
        guards.push(serde_json::json!({
            "task": id,
            "expected_version": expected_version,
            "observed_version": task.version,
            "version_matches": expected_version == task.version,
            "state": task.state,
            "started_at": task.started_at,
        }));
    }
    session.output(
        "workflow.edit.preview",
        &serde_json::json!({
            "request": request,
            "observed_generation": snapshot.generation,
            "generation_matches": request.expected_generation == snapshot.generation,
            "guarded_tasks": guards,
            "workflow": snapshot,
            "notice": "Inspection only; no changes submitted. Refresh and review stale definitions before creating a new request. The daemon checks complete admission atomically on submission.",
        }),
    )
}
