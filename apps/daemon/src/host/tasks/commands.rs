use super::{ActorSession, Owner, PublicFailure, failure, mapping};
use milkdrift_authority::AuthorityOperation;
use milkdrift_blueprint::{NewTask, TaskBatch, TaskChange, TaskId, WorkflowCreate};
use milkdrift_control_protocol::{
    InputUploadRequest,
    tasks::{
        ChangeWorkflowRequest, CreateWorkflowRequest, TaskChangeDto, TaskUploadReceipt,
        TaskUploadRequest, WorkflowReceipt,
    },
};
use milkdrift_persistence::WorkflowStore;
use std::collections::BTreeMap;

impl Owner {
    pub(in crate::host) fn create_task_workflow(
        &self,
        session: &ActorSession,
        request: CreateWorkflowRequest,
        canonical_request: &[u8],
    ) -> Result<WorkflowReceipt, PublicFailure> {
        if let Some(receipt) = self
            .workflow()?
            .task_control
            .replay_exact(
                &session.context,
                &request.request_id,
                None,
                canonical_request,
            )
            .map_err(super::super::read_model::public_control)?
        {
            return public_receipt(receipt);
        }
        let project_id = request
            .project_id
            .as_deref()
            .ok_or_else(|| failure("Choose a project before creating a workflow"))?;
        self.require_project_available(session, project_id)?;
        let tasks = request
            .tasks
            .into_iter()
            .map(|task| {
                Ok(NewTask {
                    alias: TaskId::new(task.alias).map_err(failure)?,
                    definition: self.task_definition(session, task.definition)?,
                })
            })
            .collect::<Result<_, PublicFailure>>()?;
        let inputs = request
            .inputs
            .into_iter()
            .map(|(name, value)| Ok((name, self.task_value(session, value)?)))
            .collect::<Result<_, PublicFailure>>()?;
        let create = WorkflowCreate {
            project_id: Some(
                request
                    .project_id
                    .ok_or_else(|| failure("Choose a project before creating a workflow"))?,
            ),
            title: request.title,
            description: request.description,
            tasks,
            inputs,
            limits: request
                .limits
                .map(mapping::domain_limits)
                .unwrap_or_default(),
            finish_when_settled: request.finish_when_settled,
        };
        let receipt = self
            .workflow()?
            .task_control
            .create_exact(
                &session.context,
                &request.request_id,
                create,
                canonical_request,
            )
            .map_err(super::super::read_model::public_control)?;
        public_receipt(receipt)
    }

    pub(in crate::host) fn change_task_workflow(
        &self,
        session: &ActorSession,
        workflow: &str,
        request: ChangeWorkflowRequest,
        canonical_request: &[u8],
    ) -> Result<WorkflowReceipt, PublicFailure> {
        let workflow = milkdrift_blueprint::WorkflowId::new(workflow).map_err(failure)?;
        if let Some(receipt) = self
            .workflow()?
            .task_control
            .replay_exact(
                &session.context,
                &request.request_id,
                Some(&workflow),
                canonical_request,
            )
            .map_err(super::super::read_model::public_control)?
        {
            return public_receipt(receipt);
        }
        let changes = request
            .changes
            .into_iter()
            .map(|change| {
                Ok(match change {
                    TaskChangeDto::Rename { title, description } => {
                        TaskChange::Rename { title, description }
                    }
                    TaskChangeDto::SetLimits { limits } => TaskChange::SetLimits {
                        limits: mapping::domain_limits(limits),
                    },
                    TaskChangeDto::DismissAdmission { task } => TaskChange::DismissAdmission {
                        task: TaskId::new(task).map_err(failure)?,
                    },
                    TaskChangeDto::Add { alias, definition } => TaskChange::Add {
                        alias: TaskId::new(alias).map_err(failure)?,
                        definition: self.task_definition(session, definition)?,
                    },
                    TaskChangeDto::Revise {
                        task,
                        expected_version,
                        definition,
                    } => TaskChange::Revise {
                        task: TaskId::new(task).map_err(failure)?,
                        expected_version,
                        definition: self.task_definition(session, definition)?,
                    },
                    TaskChangeDto::Remove {
                        task,
                        expected_version,
                    } => TaskChange::Remove {
                        task: TaskId::new(task).map_err(failure)?,
                        expected_version,
                    },
                    TaskChangeDto::BindInput { name, value } => TaskChange::BindInput {
                        name,
                        value: self.task_value(session, value)?,
                    },
                    TaskChangeDto::Stop { task } => TaskChange::Stop {
                        task: TaskId::new(task).map_err(failure)?,
                    },
                    TaskChangeDto::AdoptCorrection {
                        task,
                        correction,
                        redirects,
                    } => TaskChange::AdoptCorrection {
                        task: TaskId::new(task).map_err(failure)?,
                        correction: TaskId::new(correction).map_err(failure)?,
                        redirects: redirects
                            .into_iter()
                            .map(|redirect| {
                                Ok(milkdrift_blueprint::TaskBindingRedirect {
                                    consumer: TaskId::new(redirect.consumer).map_err(failure)?,
                                    expected_version: redirect.expected_version,
                                    input: redirect.input,
                                })
                            })
                            .collect::<Result<_, PublicFailure>>()?,
                    },
                    TaskChangeDto::Continue => TaskChange::Continue,
                    TaskChangeDto::Wait { question } => TaskChange::Wait { question },
                    TaskChangeDto::Finish => TaskChange::Finish,
                })
            })
            .collect::<Result<Vec<_>, PublicFailure>>()?;
        let receipt = self
            .workflow()?
            .task_control
            .edit_exact(
                &session.context,
                &request.request_id,
                &workflow,
                TaskBatch {
                    expected_generation: request.expected_generation,
                    changes,
                },
                canonical_request,
            )
            .map_err(super::super::read_model::public_control)?;
        public_receipt(receipt)
    }

    pub(in crate::host) fn task_request_receipt(
        &self,
        session: &ActorSession,
        request: &str,
    ) -> Result<WorkflowReceipt, PublicFailure> {
        let receipt = self
            .store
            .workflow_receipt(session.actor.as_str(), request)
            .map_err(failure)?
            .ok_or_else(super::super::read_model::not_found)?;
        self.authorize_task_workflow(session, &receipt.workflow, AuthorityOperation::InspectRun)?;
        public_receipt(receipt)
    }

    pub(in crate::host) fn upload_task_input(
        &self,
        session: &ActorSession,
        request: TaskUploadRequest,
    ) -> Result<TaskUploadReceipt, PublicFailure> {
        if request.text.len() > milkdrift_blueprint::MAX_TASK_TEXT_BYTES {
            return Err(failure(
                "task upload exceeds the 16 KiB UTF-8 content bound",
            ));
        }
        if !request.media_type.starts_with("text/")
            || request.name.is_empty()
            || request.name.len() > 255
            || request.name.contains(['/', '\\', '\0'])
        {
            return Err(failure(
                "task upload requires a bounded filename and UTF-8 text media type",
            ));
        }
        let bytes = milkdrift_control_protocol::encode_json(&request).map_err(failure)?;
        let upload = InputUploadRequest::from_content(
            self.host_id.to_string(),
            request.request_id.clone(),
            "application/vnd.milkdrift.task-input+json".into(),
            "internal".into(),
            &bytes,
        )
        .map_err(failure)?;
        let metadata = self.upload_input(session, &upload)?;
        Ok(TaskUploadReceipt {
            input_id: metadata.artifact_id,
            name: request.name,
            digest: format!("b3_{}", blake3::hash(request.text.as_bytes())),
            bytes: u64::try_from(request.text.len()).map_err(failure)?,
        })
    }
}

pub(super) fn public_receipt(
    receipt: milkdrift_persistence::WorkflowCommandReceipt,
) -> Result<WorkflowReceipt, PublicFailure> {
    let aliases: BTreeMap<String, String> = serde_json::from_value(
        receipt
            .acceptance
            .result
            .get("aliases")
            .cloned()
            .ok_or_else(|| failure("receipt alias map missing"))?,
    )
    .map_err(failure)?;
    Ok(WorkflowReceipt {
        request_id: receipt.acceptance.key,
        workflow_id: receipt.workflow.to_string(),
        generation: receipt.generation,
        aliases,
    })
}
