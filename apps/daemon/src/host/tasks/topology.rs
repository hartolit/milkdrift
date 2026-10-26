//! Project accepted connections without loading instructions, result bodies or frozen payloads.

use std::collections::BTreeMap;

use milkdrift_authority::{AuthorityOperation, RequestedResourceFacts};
use milkdrift_blueprint::{TaskInputBinding, TaskInputValue, TaskOrderCondition};
use milkdrift_control_protocol::{
    Cursor, ErrorCode,
    tasks::{
        TaskCapability, TaskOrderConditionDto, TaskOutputDeclarationDto, TaskPublishedOutputDto,
        TaskSuppliedInputDto,
    },
    topology::{TopologyInputDto, TopologyTaskDto, WorkflowTopologyDto},
};
use milkdrift_persistence::ArtifactStore;
use milkdrift_runtime::{TaskRecord, WorkflowRecord};
use milkdrift_workspace::ArtifactId;

use super::{ActorSession, Owner, PublicFailure, failure};

const MAX_ROWS: usize = 200;
const MAX_PREVIEWS: usize = 65_536;
// Reserve bounded envelope/cursor space within the 2 MiB public topology budget.
const MAX_ROWS_BYTES: usize = 2 * 1024 * 1024 - 16 * 1024;

impl Owner {
    pub(in crate::host) fn read_task_topology(
        &self,
        session: &ActorSession,
        workflow: &str,
        cursor: Option<&str>,
    ) -> Result<WorkflowTopologyDto, PublicFailure> {
        let record = self.task_workflow(session, workflow, AuthorityOperation::InspectRun)?;
        let feed = format!(
            "workflow-topology:{}:{}:{}",
            self.host_id, record.id, record.generation
        );
        let binding = session.cursor_binding(&feed);
        let after = cursor
            .map(|cursor| {
                let cursor: Cursor =
                    serde_json::from_value(serde_json::Value::String(cursor.into()))
                        .map_err(failure)?;
                cursor
                    .key_for_bound(&feed, &binding, session.cursor_key())
                    .map_err(|_| {
                        PublicFailure::new(
                            ErrorCode::Conflict,
                            "Workflow or viewing authority changed; reload its connections",
                            false,
                        )
                    })
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
        let mut bytes = 0usize;
        let mut preview_remaining = MAX_PREVIEWS;
        while page.len() < MAX_ROWS {
            let Some(task) = tasks.peek() else {
                break;
            };
            let row = self.topology_task(session, &record, task, &mut preview_remaining)?;
            let row_bytes = serde_json::to_vec(&row).map_err(failure)?.len();
            if bytes.saturating_add(row_bytes) > MAX_ROWS_BYTES {
                if page.is_empty() {
                    return Err(failure(
                        "One task's connections exceed the bounded topology page",
                    ));
                }
                break;
            }
            tasks.next();
            bytes = bytes.saturating_add(row_bytes);
            page.push(row);
        }
        let truncated = tasks.peek().is_some();
        let next_cursor = if truncated {
            let last = page
                .last()
                .ok_or_else(|| failure("Topology page made no progress"))?;
            let cursor = Cursor::new_bound_key(
                &feed,
                &last.id,
                binding.clone(),
                &binding.grant_digest,
                session.cursor_key(),
            )
            .map_err(failure)?;
            Some(
                serde_json::to_value(cursor)
                    .map_err(failure)?
                    .as_str()
                    .ok_or_else(|| failure("Topology cursor encoding failed"))?
                    .to_owned(),
            )
        } else {
            None
        };
        Ok(WorkflowTopologyDto {
            workflow_generation: record.generation,
            tasks: page,
            next_cursor,
            truncated,
        })
    }

    fn topology_task(
        &self,
        session: &ActorSession,
        record: &WorkflowRecord,
        task: &TaskRecord,
        preview_remaining: &mut usize,
    ) -> Result<TopologyTaskDto, PublicFailure> {
        let inputs = task
            .definition
            .inputs
            .iter()
            .map(|(name, input)| {
                let (kind, source, output, optional, value) = match input {
                    TaskInputBinding::Inline { value } => (
                        "inline",
                        format!("inline:{}:{name}", task.id),
                        None,
                        false,
                        Some(value),
                    ),
                    TaskInputBinding::Supplied { name, optional } => (
                        "supplied",
                        format!("supplied:{name}"),
                        None,
                        *optional,
                        record.inputs.get(name),
                    ),
                    TaskInputBinding::TaskOutput {
                        task,
                        output,
                        optional,
                    } => (
                        "task_output",
                        task.to_string(),
                        Some(output.clone()),
                        *optional,
                        None,
                    ),
                    TaskInputBinding::SettledOutcome { task } => {
                        ("settled_outcome", task.to_string(), None, false, None)
                    }
                };
                let preview = match value {
                    Some(value) => {
                        self.topology_preview(session, name, value, preview_remaining)?
                    }
                    None if kind == "supplied" => {
                        let missing = TaskSuppliedInputDto {
                            availability: "missing".into(),
                            value: None,
                        };
                        let bytes = serde_json::to_vec(&missing).map_err(failure)?.len();
                        if bytes <= *preview_remaining {
                            *preview_remaining = preview_remaining.saturating_sub(bytes);
                            Some(missing)
                        } else {
                            None
                        }
                    }
                    None => None,
                };
                Ok((
                    name.clone(),
                    TopologyInputDto {
                        kind: kind.into(),
                        source,
                        output,
                        optional,
                        preview,
                    },
                ))
            })
            .collect::<Result<_, PublicFailure>>()?;
        let outputs = task
            .definition
            .outputs
            .iter()
            .map(|(name, output)| {
                (
                    name.clone(),
                    TaskOutputDeclarationDto {
                        required: output.required,
                        media_type: output.media_type.clone(),
                        max_bytes: output.max_bytes,
                    },
                )
            })
            .collect();
        let published_outputs = task
            .result
            .as_ref()
            .map(|result| {
                result
                    .outputs
                    .iter()
                    .map(|(name, output)| {
                        self.topology_output(session, output.identity())
                            .map(|output| (name.clone(), output))
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let dependencies = task
            .definition
            .dependencies
            .conditions(&task.definition.inputs)
            .into_iter()
            .map(|(task, condition)| {
                (
                    task.to_string(),
                    match condition {
                        TaskOrderCondition::Success => TaskOrderConditionDto::Success,
                        TaskOrderCondition::Settled => TaskOrderConditionDto::Settled,
                    },
                )
            })
            .collect();
        Ok(TopologyTaskDto {
            id: task.id.to_string(),
            version: task.version,
            capability: TaskCapability {
                id: task
                    .definition
                    .capability
                    .exact_capability()
                    .ok_or_else(|| failure("Task has no exact configured executor"))?
                    .to_string(),
                operation: task.definition.capability.operation().to_string(),
            },
            inputs,
            outputs,
            published_outputs,
            dependencies,
            dependencies_legacy: task.definition.dependencies.is_legacy(),
        })
    }

    fn topology_preview(
        &self,
        session: &ActorSession,
        name: &str,
        value: &TaskInputValue,
        remaining: &mut usize,
    ) -> Result<Option<TaskSuppliedInputDto>, PublicFailure> {
        if *remaining == 0 {
            return Ok(None);
        }
        let name = name.to_owned();
        let preview = self
            .supplied_input_reads(
                session,
                std::iter::once((&name, value)),
                remaining.saturating_sub(1024),
            )?
            .remove(&name);
        if let Some(preview) = &preview {
            let bytes = serde_json::to_vec(preview).map_err(failure)?.len();
            if bytes > *remaining {
                return Ok(None);
            }
            *remaining = remaining.saturating_sub(bytes);
        }
        Ok(preview)
    }

    fn topology_output(
        &self,
        session: &ActorSession,
        artifact: &str,
    ) -> Result<TaskPublishedOutputDto, PublicFailure> {
        let read = || {
            let metadata = self.artifact_metadata(session, artifact)?;
            let id = ArtifactId::new(artifact).map_err(failure)?;
            super::super::artifacts::preauthorize_artifact_identity(
                session,
                &id,
                AuthorityOperation::ReadArtifactContent,
            )?;
            let retained = self
                .store
                .metadata(&id)
                .map_err(super::super::read_model::public_persistence)?
                .ok_or_else(super::super::read_model::not_found)?;
            let mut facts = RequestedResourceFacts::empty();
            facts.artifact = Some(id);
            facts.artifact_sensitivity = Some(retained.sensitivity());
            let decision = self.authorize(
                session,
                AuthorityOperation::ReadArtifactContent,
                facts,
                "task-topology-output",
            )?;
            self.record_security_decision(&decision)?;
            Ok::<_, PublicFailure>(TaskPublishedOutputDto {
                artifact_id: Some(metadata.artifact_id),
                media_type: Some(metadata.content_type),
                bytes: metadata.size,
                available: true,
            })
        };
        match read() {
            Ok(output) => Ok(output),
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::Unauthorized | ErrorCode::Unauthenticated | ErrorCode::NotFound
                ) =>
            {
                Ok(TaskPublishedOutputDto {
                    artifact_id: None,
                    media_type: None,
                    bytes: 0,
                    available: false,
                })
            }
            Err(error) => Err(error),
        }
    }
}
