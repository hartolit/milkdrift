//! Adapt conditional authoring and exact Run requests to the shared control owner.

use super::super::read_model::public_control;
use super::{ActorSession, Owner, PublicFailure, failure, mapping};
use milkdrift_blueprint::authoring::{AuthoringDocument, AuthoringSource, AuthoringTask};
use milkdrift_blueprint::{TaskChange, TaskInputBinding, TaskInputValue, WorkflowId};
use milkdrift_control::AuthoringScope;
use milkdrift_control_protocol::authoring::{
    AuthoringDocumentDto, AuthoringReviewDto, AuthoringSnapshotDto, AuthoringSourceDto,
    AuthoringTaskDto, CheckpointAuthoringRequest, PositionsSnapshotDto, PutPositionsRequest,
    ReviewAuthoringRequest, RunAuthoringRequest,
};
use milkdrift_control_protocol::tasks::{InputBinding, InputValue, TaskChangeDto, WorkflowReceipt};
use milkdrift_persistence::{
    WorkflowAuthoringSnapshot, WorkflowPositionsCommit, WorkflowPositionsSnapshot, WorkflowStore,
};

impl Owner {
    pub(in crate::host) fn read_authoring(
        &self,
        session: &ActorSession,
        workflow: &str,
    ) -> Result<AuthoringSnapshotDto, PublicFailure> {
        let workflow = WorkflowId::new(workflow).map_err(failure)?;
        public_snapshot(
            self.workflow()?
                .task_control
                .authoring(&session.context, &workflow)
                .map_err(public_control)?,
        )
    }

    pub(in crate::host) fn checkpoint_authoring(
        &self,
        session: &ActorSession,
        workflow: &str,
        request: CheckpointAuthoringRequest,
    ) -> Result<AuthoringSnapshotDto, PublicFailure> {
        let workflow = WorkflowId::new(workflow).map_err(failure)?;
        // Check workflow scope before resolving any supplied artifact identities.
        self.authorize_task_workflow(
            session,
            &workflow,
            milkdrift_authority::AuthorityOperation::Apply,
        )?;
        self.authorize_task_workflow(
            session,
            &workflow,
            milkdrift_authority::AuthorityOperation::InspectRun,
        )?;
        // The old document supplies immutable value identity, not permission to read it. The
        // shared control owner authorizes every source retained in the replacement below. Reading
        // it without its old source gates lets a person remove content they can no longer use.
        let snapshot = self
            .store
            .workflow_authoring(&workflow)
            .map_err(failure)?
            .ok_or_else(super::super::read_model::not_found)?;
        let prior = if snapshot.document.is_null() {
            AuthoringDocument::default()
        } else {
            serde_json::from_value(snapshot.document).map_err(failure)?
        };
        let record = self
            .workflow()?
            .tasks
            .workflow(&workflow)
            .map_err(failure)?
            .ok_or_else(super::super::read_model::not_found)?;
        let document = self.authoring_document(session, request.document, &prior, &record)?;
        public_snapshot(
            self.workflow()?
                .task_control
                .checkpoint_authoring(
                    &session.context,
                    &workflow,
                    request.expected_generation,
                    document,
                )
                .map_err(public_control)?,
        )
    }

    pub(in crate::host) fn read_positions(
        &self,
        session: &ActorSession,
        workflow: &str,
    ) -> Result<PositionsSnapshotDto, PublicFailure> {
        let workflow = WorkflowId::new(workflow).map_err(failure)?;
        public_positions(
            self.workflow()?
                .task_control
                .authoring_positions(&session.context, &workflow)
                .map_err(public_control)?,
        )
    }

    pub(in crate::host) fn checkpoint_positions(
        &self,
        session: &ActorSession,
        workflow: &str,
        request: PutPositionsRequest,
    ) -> Result<PositionsSnapshotDto, PublicFailure> {
        let workflow = WorkflowId::new(workflow).map_err(failure)?;
        public_positions(
            self.workflow()?
                .task_control
                .checkpoint_positions(
                    &session.context,
                    &workflow,
                    WorkflowPositionsCommit {
                        expected_generation: request.expected_generation,
                        positions: request
                            .positions
                            .into_iter()
                            .map(|(id, point)| {
                                (
                                    id,
                                    milkdrift_persistence::WorkflowPosition {
                                        x: point.x,
                                        y: point.y,
                                    },
                                )
                            })
                            .collect(),
                    },
                )
                .map_err(public_control)?,
        )
    }

    pub(in crate::host) fn review_authoring(
        &self,
        session: &ActorSession,
        workflow: &str,
        request: ReviewAuthoringRequest,
    ) -> Result<AuthoringReviewDto, PublicFailure> {
        let workflow = WorkflowId::new(workflow).map_err(failure)?;
        let review = self
            .workflow()?
            .task_control
            .review_authoring(
                &session.context,
                &workflow,
                request.authoring_generation,
                &AuthoringScope {
                    tasks: request.tasks,
                    changes: request.changes,
                    use_current: request.use_current,
                },
            )
            .map_err(public_control)?;
        serde_json::from_value(serde_json::to_value(review).map_err(failure)?).map_err(failure)
    }

    pub(in crate::host) fn run_authoring(
        &self,
        session: &ActorSession,
        workflow: &str,
        request: RunAuthoringRequest,
        canonical_request: &[u8],
    ) -> Result<WorkflowReceipt, PublicFailure> {
        let workflow = WorkflowId::new(workflow).map_err(failure)?;
        super::commands::public_receipt(
            self.workflow()?
                .task_control
                .run_authoring(
                    &session.context,
                    &workflow,
                    &request.request_id,
                    request.expected_generation,
                    request.authoring_generation,
                    &AuthoringScope {
                        tasks: request.tasks,
                        changes: request.changes,
                        use_current: request.use_current,
                    },
                    canonical_request,
                )
                .map_err(public_control)?,
        )
    }

    fn authoring_document(
        &self,
        session: &ActorSession,
        document: AuthoringDocumentDto,
        prior: &AuthoringDocument,
        record: &milkdrift_runtime::WorkflowRecord,
    ) -> Result<AuthoringDocument, PublicFailure> {
        let tasks = document
            .tasks
            .into_iter()
            .map(|mut task| {
                let previous = prior
                    .tasks
                    .iter()
                    .find(|value| value.id == task.id && value.version == task.version)
                    .map(|value| &value.definition.inputs)
                    .or_else(|| {
                        record
                            .tasks
                            .iter()
                            .find(|(id, value)| {
                                id.as_str() == task.id && task.version == Some(value.version)
                            })
                            .map(|(_, value)| &value.definition.inputs)
                    });
                let mut retained = std::collections::BTreeMap::new();
                for (name, binding) in &task.definition.inputs {
                    if let InputBinding::Inline { value } = binding
                        && let Some(TaskInputBinding::Inline { value: old }) =
                            previous.and_then(|inputs| inputs.get(name))
                        && mapping::input(old)? == *value
                    {
                        retained.insert(
                            name.clone(),
                            TaskInputBinding::Inline { value: old.clone() },
                        );
                    }
                }
                task.definition
                    .inputs
                    .retain(|name, _| !retained.contains_key(name));
                let mut definition = self.task_draft_definition(session, task.definition)?;
                definition.inputs.extend(retained);
                Ok(AuthoringTask {
                    id: task.id,
                    version: task.version,
                    title_edited: task.title_edited,
                    contract_generation: task.contract_generation,
                    definition,
                })
            })
            .collect::<Result<_, PublicFailure>>()?;
        let sources = document
            .sources
            .into_iter()
            .map(|source| {
                let previous = prior
                    .sources
                    .iter()
                    .find(|value| value.id == source.id)
                    .and_then(|value| value.value.as_ref())
                    .or_else(|| {
                        record.tasks.iter().find_map(|(task, value)| {
                            value.definition.inputs.iter().find_map(|(input, binding)| {
                                if source.id == format!("inline:{task}:{input}")
                                    && let TaskInputBinding::Inline { value } = binding
                                {
                                    Some(value)
                                } else {
                                    None
                                }
                            })
                        })
                    });
                Ok(AuthoringSource {
                    id: source.id,
                    label: source.label,
                    shared_name: source.shared_name,
                    value: source
                        .value
                        .map(|value| self.retained_authoring_value(session, value, previous))
                        .transpose()?,
                })
            })
            .collect::<Result<_, PublicFailure>>()?;
        let changes = document
            .changes
            .into_iter()
            .map(|change| {
                if let TaskChangeDto::BindInput { name, value } = change {
                    let previous = prior
                        .changes
                        .iter()
                        .find_map(|change| match change {
                            TaskChange::BindInput {
                                name: old_name,
                                value: old,
                            } if old_name == &name
                                && mapping::input(old).ok().as_ref() == Some(&value) =>
                            {
                                Some(old)
                            }
                            _ => None,
                        })
                        .or_else(|| record.inputs.get(&name));
                    return Ok(TaskChange::BindInput {
                        name,
                        value: self.retained_authoring_value(session, value, previous)?,
                    });
                }
                if matches!(
                    change,
                    TaskChangeDto::Add { .. } | TaskChangeDto::Revise { .. }
                ) {
                    return Err(failure("Use task cards for saved candidate definitions"));
                }
                serde_json::from_value(serde_json::to_value(change).map_err(failure)?)
                    .map_err(failure)
            })
            .collect::<Result<_, PublicFailure>>()?;
        Ok(AuthoringDocument {
            tasks,
            sources,
            changes,
            accepted: document.accepted,
            connections: serde_json::from_value(
                serde_json::to_value(document.connections).map_err(failure)?,
            )
            .map_err(failure)?,
            usual_model: serde_json::from_value(
                serde_json::to_value(document.usual_model).map_err(failure)?,
            )
            .map_err(failure)?,
        })
    }

    fn retained_authoring_value(
        &self,
        session: &ActorSession,
        requested: InputValue,
        retained: Option<&TaskInputValue>,
    ) -> Result<TaskInputValue, PublicFailure> {
        if let Some(value) = retained
            && mapping::input(value)? == requested
        {
            return Ok(value.clone());
        }
        // A new upload still belongs to its authenticated uploader and this daemon. Retaining
        // an already-owned value never turns an arbitrary artifact ID into a capture request.
        self.task_value(session, requested)
    }
}

fn public_snapshot(
    snapshot: WorkflowAuthoringSnapshot,
) -> Result<AuthoringSnapshotDto, PublicFailure> {
    let document = if snapshot.document.is_null() {
        AuthoringDocument::default()
    } else {
        serde_json::from_value(snapshot.document).map_err(failure)?
    };
    let tasks = document
        .tasks
        .into_iter()
        .map(|task| {
            Ok(AuthoringTaskDto {
                id: task.id,
                version: task.version,
                title_edited: task.title_edited,
                contract_generation: task.contract_generation,
                definition: mapping::draft_definition(&task.definition)?,
            })
        })
        .collect::<Result<_, PublicFailure>>()?;
    let sources = document
        .sources
        .into_iter()
        .map(|source| {
            Ok(AuthoringSourceDto {
                id: source.id,
                label: source.label,
                shared_name: source.shared_name,
                value: source.value.as_ref().map(mapping::input).transpose()?,
            })
        })
        .collect::<Result<_, PublicFailure>>()?;
    let changes = document
        .changes
        .into_iter()
        .map(|change| {
            if let TaskChange::BindInput { name, value } = change {
                return Ok(TaskChangeDto::BindInput {
                    name,
                    value: mapping::input(&value)?,
                });
            }
            serde_json::from_value(serde_json::to_value(change).map_err(failure)?).map_err(failure)
        })
        .collect::<Result<_, PublicFailure>>()?;
    Ok(AuthoringSnapshotDto {
        generation: snapshot.generation,
        digest: snapshot.digest,
        document: AuthoringDocumentDto {
            tasks,
            sources,
            changes,
            accepted: document.accepted,
            connections: serde_json::from_value(
                serde_json::to_value(document.connections).map_err(failure)?,
            )
            .map_err(failure)?,
            usual_model: serde_json::from_value(
                serde_json::to_value(document.usual_model).map_err(failure)?,
            )
            .map_err(failure)?,
        },
    })
}

fn public_positions(
    snapshot: WorkflowPositionsSnapshot,
) -> Result<PositionsSnapshotDto, PublicFailure> {
    Ok(PositionsSnapshotDto {
        generation: snapshot.generation,
        digest: snapshot.digest,
        positions: snapshot
            .positions
            .into_iter()
            .map(|(id, point)| {
                (
                    id,
                    milkdrift_control_protocol::authoring::CanvasPositionDto {
                        x: point.x,
                        y: point.y,
                    },
                )
            })
            .collect(),
    })
}
