use super::{ActorSession, Owner, PublicFailure, failure, mapping};
use milkdrift_authority::{AuthorityOperation, RequestedResourceFacts};
use milkdrift_blueprint::TaskInputValue;
use milkdrift_control_protocol::{
    Cursor, ErrorCode,
    tasks::{
        TaskCapability, TaskReadDto, TaskResultDto, TaskSessionDto, TaskStateDto, WorkflowPageDto,
        WorkflowReadDto,
    },
};
use milkdrift_persistence::WorkflowStore;
use milkdrift_runtime::{TaskRecord, TaskState, WorkflowRecord};
use std::collections::BTreeMap;

impl Owner {
    pub(in crate::host) fn task_session(
        &self,
        session: &ActorSession,
    ) -> Result<TaskSessionDto, PublicFailure> {
        self.authorize(
            session,
            AuthorityOperation::NegotiateControlProtocol,
            RequestedResourceFacts::empty(),
            "task:session",
        )?;
        let capabilities = self
            .capabilities(session)?
            .into_iter()
            .filter(|capability| capability.current && !capability.draining)
            .flat_map(|capability| {
                capability
                    .operations
                    .into_iter()
                    .filter(|operation| {
                        !operation.starts_with("workflow.") && !operation.starts_with("controller.")
                    })
                    .map(move |operation| TaskCapability {
                        id: capability.capability_id.clone(),
                        operation,
                    })
            })
            .collect();
        Ok(TaskSessionDto {
            daemon_id: self.host_id.to_string(),
            actor_id: session.actor.to_string(),
            limits: mapping::limits(&milkdrift_blueprint::WorkflowLimits::default()),
            capabilities,
        })
    }

    pub(in crate::host) fn task_workflows(
        &self,
        session: &ActorSession,
        search: &str,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<WorkflowPageDto, PublicFailure> {
        if !(1..=200).contains(&limit) || search.len() > 256 {
            return Err(failure("page bounds or search are invalid"));
        }
        if !session
            .grant
            .operations()
            .contains(&AuthorityOperation::InspectRun)
        {
            return Err(super::super::read_model::unauthorized());
        }
        let feed = format!("tasks:{}:{}", self.host_id, blake3::hash(search.as_bytes()));
        let binding = session.cursor_binding(&feed);
        let mut after = cursor
            .map(|cursor| {
                let cursor: Cursor =
                    serde_json::from_value(serde_json::Value::String(cursor.into()))
                        .map_err(failure)?;
                cursor
                    .key_for_bound(&feed, &binding, session.cursor_key())
                    .map_err(failure)
            })
            .transpose()?;
        let mut items = Vec::new();
        let search = search.to_lowercase();
        let mut next_cursor = None;
        let mut exhausted = false;
        // Bound examination separately from the public result count; no hidden count is returned.
        for _ in 0..20 {
            let page = self
                .store
                .workflows(after.as_deref(), 200)
                .map_err(failure)?;
            for snapshot in page.items {
                after = Some(snapshot.workflow.to_string());
                let mut resources = RequestedResourceFacts::empty();
                resources.workflow = Some(snapshot.workflow.clone());
                resources.run = Some(
                    milkdrift_workspace::RunId::new(snapshot.workflow.as_str()).map_err(failure)?,
                );
                if !self
                    .evaluate_authority(
                        session,
                        AuthorityOperation::InspectRun,
                        resources,
                        "task:list",
                    )?
                    .is_allowed()
                {
                    continue;
                }
                let record: WorkflowRecord =
                    serde_json::from_value(snapshot.document).map_err(failure)?;
                let title_matches = search.is_empty()
                    || record.title.to_lowercase().contains(&search)
                    || record.description.to_lowercase().contains(&search);
                let read = self.task_workflow_read(session, &record)?;
                if !title_matches
                    && !read
                        .tasks
                        .iter()
                        .filter_map(|task| task.result.as_ref())
                        .any(|result| result.text.to_lowercase().contains(&search))
                {
                    continue;
                }
                items.push(read);
                if items.len() == usize::from(limit) {
                    break;
                }
            }
            if items.len() == usize::from(limit) {
                break;
            }
            if page.next.is_none() {
                exhausted = true;
                break;
            }
        }
        if !exhausted && let Some(after) = after {
            // The grant digest is also a stable canonical digest for this disclosure decision.
            next_cursor = Some(
                serde_json::to_value(
                    Cursor::new_bound_key(
                        &feed,
                        &after,
                        binding.clone(),
                        &binding.grant_digest,
                        session.cursor_key(),
                    )
                    .map_err(failure)?,
                )
                .map_err(failure)?
                .as_str()
                .ok_or_else(|| failure("cursor encoding failed"))?
                .to_owned(),
            );
        }
        Ok(WorkflowPageDto {
            items,
            next_cursor,
            truncated: !exhausted,
        })
    }

    pub(in crate::host) fn read_task_workflow(
        &self,
        session: &ActorSession,
        workflow: &str,
    ) -> Result<WorkflowReadDto, PublicFailure> {
        let record = self.task_workflow(session, workflow, AuthorityOperation::InspectRun)?;
        self.task_workflow_read(session, &record)
    }

    fn task_workflow_read(
        &self,
        session: &ActorSession,
        record: &WorkflowRecord,
    ) -> Result<WorkflowReadDto, PublicFailure> {
        Ok(WorkflowReadDto {
            id: record.id.to_string(),
            title: record.title.clone(),
            description: record.description.clone(),
            generation: record.generation,
            state: serde_json::to_value(record.state)
                .map_err(failure)?
                .as_str()
                .ok_or_else(|| failure("workflow state encoding failed"))?
                .into(),
            created_at: record.created_at.get(),
            updated_at: record.updated_at.get(),
            limits: mapping::limits(&record.limits),
            tasks: record
                .tasks
                .values()
                .map(|task| self.task_read(session, &record.id.to_string(), task))
                .collect::<Result<_, _>>()?,
        })
    }

    fn task_read(
        &self,
        session: &ActorSession,
        workflow: &str,
        task: &TaskRecord,
    ) -> Result<TaskReadDto, PublicFailure> {
        let result = task
            .result
            .as_ref()
            .map(|result| {
                let artifacts = result
                    .terminal
                    .outputs()
                    .iter()
                    .map(|output| output.identity().to_owned())
                    .collect();
                let mut text = String::new();
                for output in result.terminal.outputs().iter().filter(|output| {
                    output
                        .media_type()
                        .is_some_and(|media| media.starts_with("text/"))
                }) {
                    let content =
                        self.artifact_range(session, output.identity(), 0, 524_288, "task-result")?;
                    if !content.end {
                        return Err(failure("task result exceeds bounded text inspector"));
                    }
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(std::str::from_utf8(&content.bytes).map_err(failure)?);
                    if text.len() > 524_288 {
                        return Err(failure("combined text results exceed inspector bound"));
                    }
                }
                let input_digests = task
                    .frozen
                    .as_ref()
                    .map(|start| {
                        start
                            .inputs
                            .iter()
                            .map(|(name, value)| {
                                let digest = match value {
                                    TaskInputValue::TextFile { digest, .. } => digest.clone(),
                                    TaskInputValue::Text { text } => {
                                        format!("b3_{}", blake3::hash(text.as_bytes()))
                                    }
                                    other => format!(
                                        "b3_{}",
                                        blake3::hash(&serde_json::to_vec(other).map_err(failure)?)
                                    ),
                                };
                                Ok((name.clone(), digest))
                            })
                            .collect::<Result<BTreeMap<_, _>, PublicFailure>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                Ok::<_, PublicFailure>(TaskResultDto {
                    digest: format!("b3_{}", blake3::hash(text.as_bytes())),
                    text,
                    artifacts,
                    input_digests,
                })
            })
            .transpose()?;
        Ok(TaskReadDto {
            id: task.id.to_string(),
            version: task.version,
            definition: mapping::definition(&task.definition)?,
            state: match task.state {
                TaskState::Queued => TaskStateDto::Queued,
                TaskState::Running => TaskStateDto::Running,
                TaskState::Completed => TaskStateDto::Completed,
                TaskState::Failed => TaskStateDto::Failed,
                TaskState::Stopped => TaskStateDto::Stopped,
                TaskState::Uncertain => TaskStateDto::Uncertain,
            },
            reason: task.reason.clone(),
            started_at: task.started_at.map(|time| time.get()),
            ended_at: task.ended_at.map(|time| time.get()),
            stop_requested: task.stop_requested,
            stop_request_id: format!("stop:{}:{}", workflow, task.id),
            frozen: task.frozen.as_ref().map(frozen).transpose()?,
            result,
        })
    }

    pub(in crate::host) fn read_task(
        &self,
        session: &ActorSession,
        workflow: &str,
        task: &str,
    ) -> Result<TaskReadDto, PublicFailure> {
        self.read_task_workflow(session, workflow)?
            .tasks
            .into_iter()
            .find(|value| value.id == task)
            .ok_or_else(super::super::read_model::not_found)
    }

    pub(in crate::host) fn read_task_result(
        &self,
        session: &ActorSession,
        workflow: &str,
        task: &str,
    ) -> Result<TaskResultDto, PublicFailure> {
        self.read_task(session, workflow, task)?
            .result
            .ok_or_else(|| {
                PublicFailure::new(ErrorCode::NotFound, "task has no retained result", false)
            })
    }
}

fn frozen(
    start: &milkdrift_runtime::FrozenTaskStart,
) -> Result<milkdrift_control_protocol::tasks::FrozenTaskDto, PublicFailure> {
    use milkdrift_control_protocol::tasks::{FrozenInputDto, FrozenTaskDto};
    let inputs = start
        .inputs
        .iter()
        .map(|(key, value)| {
            let (kind, name, text, digest, artifact_id) = match value {
                TaskInputValue::Text { text } => (
                    "text",
                    None,
                    Some(text.clone()),
                    Some(format!("b3_{}", blake3::hash(text.as_bytes()))),
                    None,
                ),
                TaskInputValue::TextFile {
                    name,
                    text,
                    digest,
                    source_artifact,
                } => (
                    "text_file",
                    Some(name.clone()),
                    Some(text.clone()),
                    Some(digest.clone()),
                    source_artifact.clone(),
                ),
                TaskInputValue::Json { value } => {
                    let text = serde_json::to_string(value).map_err(failure)?;
                    let digest = format!("b3_{}", blake3::hash(text.as_bytes()));
                    ("json", None, Some(text), Some(digest), None)
                }
                TaskInputValue::Artifact { reference } => (
                    "artifact",
                    None,
                    None,
                    Some(reference.digest().into()),
                    Some(reference.identity().into()),
                ),
                TaskInputValue::Absent => ("absent", None, None, None, None),
            };
            Ok((
                key.clone(),
                FrozenInputDto {
                    kind: kind.into(),
                    name,
                    text,
                    digest,
                    artifact_id,
                },
            ))
        })
        .collect::<Result<_, PublicFailure>>()?;
    let basis = start.dispatch.execution_authority();
    Ok(FrozenTaskDto {
        inputs,
        capability_generation: start.dispatch.resolution().descriptor_revision(),
        actor_id: basis.actor().to_string(),
        grant_id: basis.grant().to_string(),
        grant_revision: basis.grant_revision(),
        entered: start.entered,
    })
}
