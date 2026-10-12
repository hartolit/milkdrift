use super::{ActorSession, Owner, PublicFailure, failure, mapping};
use milkdrift_authority::{AuthorityOperation, RequestedResourceFacts};
use milkdrift_blueprint::{TaskInputBinding, TaskInputValue};
use milkdrift_control_protocol::{
    Cursor, ErrorCode,
    tasks::{
        TaskCapability, TaskReadDto, TaskResultDto, TaskSessionDto, TaskStateDto, TaskSummaryDto,
        WorkflowPageDto, WorkflowReadDto, WorkflowSummaryDto,
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
            input_limit_bytes: u32::try_from(milkdrift_blueprint::MAX_TASK_TEXT_BYTES)
                .map_err(failure)?,
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
        let mut page_bytes = 0;
        // Bound examination separately from the public result count; no hidden count is returned.
        'pages: for _ in 0..20 {
            let page = self
                .store
                .workflows(after.as_deref(), 200)
                .map_err(failure)?;
            for snapshot in page.items {
                let previous_after = after.clone();
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

                if !title_matches {
                    let mut found = false;
                    for task in record.tasks.values() {
                        if task.definition.title.to_lowercase().contains(&search)
                            || task.definition.description.to_lowercase().contains(&search)
                            || self.search_task_text(session, task, &search)?
                        {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        continue;
                    }
                }
                let summary = workflow_summary(&record)?;
                let bytes = serde_json::to_vec(&summary).map_err(failure)?.len();
                if page_bytes + bytes > 900_000 && !items.is_empty() {
                    after = previous_after;
                    break 'pages;
                }
                page_bytes += bytes;
                items.push(summary);
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
        cursor: Option<&str>,
    ) -> Result<WorkflowReadDto, PublicFailure> {
        let record = self.task_workflow(session, workflow, AuthorityOperation::InspectRun)?;
        self.task_workflow_read(session, &record, cursor)
    }

    fn task_workflow_read(
        &self,
        session: &ActorSession,
        record: &WorkflowRecord,
        cursor: Option<&str>,
    ) -> Result<WorkflowReadDto, PublicFailure> {
        let feed = format!("workflow-tasks:{}:{}", self.host_id, record.id);
        let binding = session.cursor_binding(&feed);
        let after = cursor
            .map(|cursor| {
                let cursor: Cursor =
                    serde_json::from_value(serde_json::Value::String(cursor.into()))
                        .map_err(failure)?;
                cursor
                    .key_for_bound(&feed, &binding, session.cursor_key())
                    .map_err(failure)
            })
            .transpose()?;
        let mut tasks = record
            .tasks
            .values()
            .filter(|task| {
                after
                    .as_ref()
                    .is_none_or(|after| task.id.as_str() > after.as_str())
            })
            .peekable();
        let mut page = Vec::new();
        let mut page_bytes = 0;
        while page.len() < 50 {
            let Some(task) = tasks.peek() else {
                break;
            };
            let summary = task_summary(record.id.as_str(), task);
            let bytes = serde_json::to_vec(&summary).map_err(failure)?.len();
            if page_bytes + bytes > 900_000 && !page.is_empty() {
                break;
            }
            tasks.next();
            page_bytes += bytes;
            page.push(summary);
        }
        let next_task_cursor = if tasks.peek().is_some() {
            page.last()
                .map(|task| {
                    let cursor = Cursor::new_bound_key(
                        &feed,
                        &task.id,
                        binding.clone(),
                        &binding.grant_digest,
                        session.cursor_key(),
                    )
                    .map_err(failure)?;
                    serde_json::to_value(cursor)
                        .map_err(failure)?
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| failure("cursor encoding failed"))
                })
                .transpose()?
        } else {
            None
        };
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
            tasks: page,
            task_count: u32::try_from(record.tasks.len()).map_err(failure)?,
            next_task_cursor,
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
                    .map(|output| {
                        self.artifact_metadata(session, output.identity())
                            .map(|metadata| metadata.artifact_id)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let content = self.task_text_bytes(session, task)?;
                let digest = format!("b3_{}", blake3::hash(&content));
                let full_text = std::str::from_utf8(&content);
                let text_unavailable = full_text.is_err();
                let full_text = full_text.unwrap_or_default();
                let text = preview(full_text, 16_384).to_owned();
                let text_truncated = text.len() != full_text.len();
                let input_digests = task
                    .frozen
                    .as_ref()
                    .map(|start| {
                        start
                            .inputs
                            .iter()
                            .filter_map(|(name, value)| {
                                match self.input_visible(session, start, name, value) {
                                    Ok(true) => Some(Ok((name, value))),
                                    Ok(false) => None,
                                    Err(error) => Some(Err(error)),
                                }
                            })
                            .map(|item| {
                                let (name, value) = item?;
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
                    digest,
                    text_unavailable,
                    text,
                    text_truncated,
                    artifacts,
                    input_digests,
                })
            })
            .transpose();
        let (result, result_restricted) = match result {
            Ok(result) => (result, false),
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::Unauthorized | ErrorCode::Unauthenticated | ErrorCode::NotFound
                ) =>
            {
                (None, true)
            }
            Err(error) => return Err(error),
        };
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
            frozen: task
                .frozen
                .as_ref()
                .map(|start| frozen(self, session, start))
                .transpose()?,
            result,
            result_restricted,
        })
    }

    fn input_visible(
        &self,
        session: &ActorSession,
        start: &milkdrift_runtime::FrozenTaskStart,
        name: &str,
        value: &TaskInputValue,
    ) -> Result<bool, PublicFailure> {
        let mut artifacts = Vec::new();
        match value {
            TaskInputValue::TextFile {
                source_artifact: Some(artifact),
                ..
            } => artifacts.push(artifact.clone()),
            TaskInputValue::Artifact { reference } => {
                artifacts.push(reference.identity().to_owned())
            }
            TaskInputValue::Json { value }
                if matches!(
                    start.definition.inputs.get(name),
                    Some(TaskInputBinding::SettledOutcome { .. })
                ) =>
            {
                if let Some(result) = value.get("result").filter(|result| !result.is_null()) {
                    let result: milkdrift_runtime::TaskResult =
                        serde_json::from_value(result.clone()).map_err(failure)?;
                    artifacts.extend(
                        result
                            .terminal
                            .outputs()
                            .iter()
                            .chain(result.outputs.values())
                            .map(|output| output.identity().to_owned()),
                    );
                    artifacts.sort();
                    artifacts.dedup();
                }
            }
            _ => {}
        }
        for artifact in artifacts {
            match self.artifact_range(session, &artifact, 0, 1, "task-frozen-input") {
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.code,
                        ErrorCode::Unauthorized | ErrorCode::Unauthenticated | ErrorCode::NotFound
                    ) =>
                {
                    return Ok(false);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }

    fn search_task_text(
        &self,
        session: &ActorSession,
        task: &TaskRecord,
        search: &str,
    ) -> Result<bool, PublicFailure> {
        if task.result.is_none() {
            return Ok(false);
        }
        match self.task_text_bytes(session, task) {
            Ok(bytes) => {
                Ok(std::str::from_utf8(&bytes)
                    .is_ok_and(|text| text.to_lowercase().contains(search)))
            }
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::Unauthorized | ErrorCode::Unauthenticated | ErrorCode::NotFound
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn task_text_bytes(
        &self,
        session: &ActorSession,
        task: &TaskRecord,
    ) -> Result<Vec<u8>, PublicFailure> {
        let result = task
            .result
            .as_ref()
            .ok_or_else(super::super::read_model::not_found)?;
        let mut bytes = Vec::new();
        let maximum =
            2_097_152usize.saturating_add(result.terminal.outputs().len().saturating_sub(1));
        for output in result.terminal.outputs().iter().filter(|output| {
            output
                .media_type()
                .is_some_and(|media| media.starts_with("text/") || media.contains("json"))
        }) {
            if !bytes.is_empty() {
                bytes.push(b'\n');
            }
            let mut offset = 0;
            loop {
                let content = self.artifact_range(
                    session,
                    output.identity(),
                    offset,
                    524_288,
                    "task-result",
                )?;
                if bytes.len().saturating_add(content.bytes.len()) > maximum {
                    return Err(failure(
                        "combined text result exceeds 2 MiB; download individual artifacts",
                    ));
                }
                offset += u64::try_from(content.bytes.len()).map_err(failure)?;
                bytes.extend_from_slice(&content.bytes);
                if content.end {
                    break;
                }
                if content.bytes.is_empty() {
                    return Err(failure("artifact read made no progress"));
                }
            }
        }
        Ok(bytes)
    }

    pub(in crate::host) fn read_task_result_content(
        &self,
        session: &ActorSession,
        workflow: &str,
        task: &str,
    ) -> Result<String, PublicFailure> {
        let record = self.task_workflow(session, workflow, AuthorityOperation::InspectRun)?;
        let task = record
            .tasks
            .get(&milkdrift_blueprint::TaskId::new(task).map_err(failure)?)
            .ok_or_else(super::super::read_model::not_found)?;
        String::from_utf8(self.task_text_bytes(session, task)?).map_err(|_| {
            failure("declared text output is not UTF-8; download individual artifacts")
        })
    }

    pub(in crate::host) fn read_task(
        &self,
        session: &ActorSession,
        workflow: &str,
        task: &str,
    ) -> Result<TaskReadDto, PublicFailure> {
        let record = self.task_workflow(session, workflow, AuthorityOperation::InspectRun)?;
        let task = record
            .tasks
            .get(&milkdrift_blueprint::TaskId::new(task).map_err(failure)?)
            .ok_or_else(super::super::read_model::not_found)?;
        self.task_read(session, workflow, task)
    }

    pub(in crate::host) fn read_task_result(
        &self,
        session: &ActorSession,
        workflow: &str,
        task: &str,
    ) -> Result<TaskResultDto, PublicFailure> {
        let task = self.read_task(session, workflow, task)?;
        if task.result_restricted {
            return Err(super::super::read_model::unauthorized());
        }
        task.result.ok_or_else(|| {
            PublicFailure::new(ErrorCode::NotFound, "task has no retained result", false)
        })
    }
}

fn frozen(
    owner: &Owner,
    session: &ActorSession,
    start: &milkdrift_runtime::FrozenTaskStart,
) -> Result<milkdrift_control_protocol::tasks::FrozenTaskDto, PublicFailure> {
    use milkdrift_control_protocol::tasks::{FrozenInputDto, FrozenTaskDto};
    let mut remaining = 16_384;
    let inputs = start
        .inputs
        .iter()
        .map(|(key, value)| {
            if !owner.input_visible(session, start, key, value)? {
                return Ok((
                    key.clone(),
                    FrozenInputDto {
                        restricted: true,
                        kind: "restricted".into(),
                        name: None,
                        text: None,
                        text_truncated: false,
                        digest: None,
                        artifact_id: None,
                    },
                ));
            }
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
            let mut text_truncated = false;
            let text = text.map(|text| {
                let selected = preview(&text, remaining).to_owned();
                text_truncated = selected.len() != text.len();
                remaining = remaining.saturating_sub(selected.len());
                selected
            });
            Ok((
                key.clone(),
                FrozenInputDto {
                    restricted: false,
                    kind: kind.into(),
                    name,
                    text,
                    text_truncated,
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

fn state(task: &TaskRecord) -> TaskStateDto {
    match task.state {
        TaskState::Queued => TaskStateDto::Queued,
        TaskState::Running => TaskStateDto::Running,
        TaskState::Completed => TaskStateDto::Completed,
        TaskState::Failed => TaskStateDto::Failed,
        TaskState::Stopped => TaskStateDto::Stopped,
        TaskState::Uncertain => TaskStateDto::Uncertain,
    }
}
fn task_summary(workflow: &str, task: &TaskRecord) -> TaskSummaryDto {
    TaskSummaryDto {
        id: task.id.to_string(),
        version: task.version,
        title: task.definition.title.clone(),
        description: task.definition.description.clone(),
        dependencies: task
            .definition
            .dependencies
            .iter()
            .map(ToString::to_string)
            .collect(),
        state: state(task),
        reason: task.reason.clone(),
        started_at: task.started_at.map(|t| t.get()),
        ended_at: task.ended_at.map(|t| t.get()),
        stop_requested: task.stop_requested,
        stop_request_id: format!("stop:{workflow}:{}", task.id),
        has_result: task.result.is_some(),
    }
}
fn workflow_summary(record: &WorkflowRecord) -> Result<WorkflowSummaryDto, PublicFailure> {
    Ok(WorkflowSummaryDto {
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
        task_count: u32::try_from(record.tasks.len()).map_err(failure)?,
    })
}

fn preview(text: &str, maximum: usize) -> &str {
    milkdrift_contracts::truncate_utf8(text, maximum)
}
