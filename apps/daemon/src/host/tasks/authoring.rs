//! Authorized projections of adapter-owned authoring facts and captured shared inputs.

use super::{ActorSession, Owner, PublicFailure, failure};
use milkdrift_authority::{AccessMode, AuthorityOperation};
use milkdrift_blueprint::TaskInputValue;
use milkdrift_control_protocol::{
    ErrorCode,
    tasks::{
        FrozenInputDto, TaskCapability, TaskCapabilityAuthoringDto, TaskInputSlotDto,
        TaskOutputPortDto, TaskProjectRootDto, TaskSuppliedInputDto,
    },
};
use milkdrift_persistence::ArtifactStore;
use milkdrift_workspace::ArtifactId;
use std::collections::BTreeMap;

impl Owner {
    pub(super) fn task_authoring(
        &self,
        session: &ActorSession,
    ) -> Result<Vec<TaskCapabilityAuthoringDto>, PublicFailure> {
        Ok(self
            .capability_generations(session)?
            .into_iter()
            .filter(|generation| generation.current && !generation.draining)
            .flat_map(|generation| {
                generation
                    .task_authoring
                    .into_iter()
                    .map(move |(operation, contract)| TaskCapabilityAuthoringDto {
                        input_media: match contract.input_media {
                            milkdrift_capability::TaskInputMedia::Any => milkdrift_control_protocol::tasks::TaskInputMediaDto::Any,
                            milkdrift_capability::TaskInputMedia::TextJsonOrImage => milkdrift_control_protocol::tasks::TaskInputMediaDto::TextJsonOrImage,
                        },
                        capability: TaskCapability {
                            id: generation.capability.to_string(),
                            operation: operation.to_string(),
                        },
                        generation: generation.descriptor_revision,
                        label: contract.label,
                        category: super::super::read_model::snake_debug(&generation.category),
                        inputs: contract
                            .inputs
                            .into_iter()
                            .map(|port| TaskInputSlotDto {
                                name: port.name,
                                label: port.label,
                                required: port.required,
                                accepts_artifacts: port.accepts_artifacts,
                                media_type: port.media_type,
                                max_bytes: port.max_bytes,
                            })
                            .collect(),
                        allows_additional_inputs: contract.allows_additional_inputs,
                        outputs: contract
                            .outputs
                            .into_iter()
                            .map(|port| TaskOutputPortDto {
                                name: port.name,
                                label: port.label,
                                required: port.required,
                                media_type: port.media_type,
                                max_bytes: port
                                    .max_bytes
                                    .min(milkdrift_blueprint::MAX_TASK_OUTPUT_BYTES),
                            })
                            .collect(),
                        default_output: contract.default_output,
                    })
            })
            .collect())
    }

    pub(super) fn task_project_roots(session: &ActorSession) -> Vec<TaskProjectRootDto> {
        session
            .grant
            .resources()
            .filesystem
            .iter()
            .filter_map(|scope| {
                let read = scope.access().contains(&AccessMode::Read);
                let write = scope.access().contains(&AccessMode::Write);
                (read || write).then(|| TaskProjectRootDto {
                    path: scope.root().into(),
                    label: scope
                        .root()
                        .rsplit('/')
                        .find(|part| !part.is_empty())
                        .unwrap_or("Filesystem root")
                        .into(),
                    read,
                    write,
                })
            })
            .collect()
    }

    pub(super) fn supplied_input_reads<'a>(
        &self,
        session: &ActorSession,
        inputs: impl Iterator<Item = (&'a String, &'a TaskInputValue)>,
        maximum_text: usize,
    ) -> Result<BTreeMap<String, TaskSuppliedInputDto>, PublicFailure> {
        let mut remaining = maximum_text;
        inputs
            .map(|(name, value)| {
                let artifact = match value {
                    TaskInputValue::TextFile {
                        source_artifact, ..
                    } => source_artifact.as_deref(),
                    TaskInputValue::Artifact { reference } => Some(reference.identity()),
                    _ => None,
                };
                if let Some(artifact) = artifact {
                    let identity = ArtifactId::new(artifact).map_err(failure)?;
                    // The workflow already owns this source reference. Classify its availability
                    // without returning the protected artifact's metadata, digest, or content.
                    if super::super::artifacts::preauthorize_artifact_identity(
                        session,
                        &identity,
                        AuthorityOperation::ReadArtifactContent,
                    )
                    .is_err()
                    {
                        return Ok((
                            name.clone(),
                            TaskSuppliedInputDto {
                                availability: "restricted".into(),
                                value: None,
                            },
                        ));
                    }
                    let metadata = self
                        .store
                        .metadata(&identity)
                        .map_err(super::super::read_model::public_persistence)?;
                    let Some(metadata) = metadata else {
                        return Ok((
                            name.clone(),
                            TaskSuppliedInputDto {
                                availability: "expired".into(),
                                value: None,
                            },
                        ));
                    };
                    if !session
                        .grant
                        .resources()
                        .artifacts
                        .sensitivities()
                        .is_some_and(|allowed| allowed.contains(&metadata.sensitivity()))
                    {
                        return Ok((
                            name.clone(),
                            TaskSuppliedInputDto {
                                availability: "restricted".into(),
                                value: None,
                            },
                        ));
                    }
                    match self.artifact_range(session, artifact, 0, 1, "task-supplied-input") {
                        Ok(_) => {}
                        Err(error) if error.code == ErrorCode::NotFound => {
                            return Ok((
                                name.clone(),
                                TaskSuppliedInputDto {
                                    availability: "expired".into(),
                                    value: None,
                                },
                            ));
                        }
                        Err(error)
                            if matches!(
                                error.code,
                                ErrorCode::Unauthorized | ErrorCode::Unauthenticated
                            ) =>
                        {
                            return Ok((
                                name.clone(),
                                TaskSuppliedInputDto {
                                    availability: "restricted".into(),
                                    value: None,
                                },
                            ));
                        }
                        Err(error) => return Err(error),
                    }
                }
                let (kind, filename, text, digest, artifact_id) = match value {
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
                    let bounded =
                        milkdrift_contracts::truncate_utf8(&text, remaining.min(16_384)).to_owned();
                    text_truncated = bounded.len() != text.len();
                    remaining = remaining.saturating_sub(bounded.len());
                    bounded
                });
                Ok((
                    name.clone(),
                    TaskSuppliedInputDto {
                        availability: "available".into(),
                        value: Some(FrozenInputDto {
                            restricted: false,
                            kind: kind.into(),
                            name: filename,
                            text,
                            text_truncated,
                            digest,
                            artifact_id,
                        }),
                    },
                ))
            })
            .collect()
    }
}
