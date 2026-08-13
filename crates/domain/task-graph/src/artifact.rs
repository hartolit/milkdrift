use domain_contracts::ArtifactId;

use crate::{
    error::TaskGraphError,
    graph::{TaskGraph, TaskId},
};

/// Artifact consumed by one task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskArtifactInput {
    /// Task that consumes the artifact.
    pub consumer: TaskId,
    /// Consumed artifact identity.
    pub artifact: ArtifactId,
}

/// Artifact produced by one task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskArtifactOutput {
    /// Task that produces the artifact.
    pub producer: TaskId,
    /// Produced artifact identity.
    pub artifact: ArtifactId,
}

/// Borrowed identity and provenance declarations for a task graph.
///
/// Artifact media, role, payload, and size policy remain caller-owned metadata.
/// A task may produce zero, one, or many artifacts.
#[derive(Clone, Copy, Debug)]
pub struct ArtifactFlow<'a> {
    /// Immutable artifacts supplied from outside this graph.
    pub external_inputs: &'a [ArtifactId],
    /// Artifact identities consumed by tasks.
    pub task_inputs: &'a [TaskArtifactInput],
    /// Artifact identities produced by tasks.
    pub task_outputs: &'a [TaskArtifactOutput],
}

impl<'a> ArtifactFlow<'a> {
    /// Creates a borrowed artifact-flow view.
    #[must_use]
    pub const fn new(
        external_inputs: &'a [ArtifactId],
        task_inputs: &'a [TaskArtifactInput],
        task_outputs: &'a [TaskArtifactOutput],
    ) -> Self {
        Self {
            external_inputs,
            task_inputs,
            task_outputs,
        }
    }
}

/// Validates identity-only artifact provenance and direct dependencies.
///
/// The algorithm performs repeated borrowed-slice scans and allocates nothing.
/// External inputs cannot also be task outputs. Every consumed artifact must
/// have exactly one external or task producer, and task-produced inputs require
/// a direct producer-to-consumer graph dependency.
///
/// # Errors
///
/// Returns a typed [`TaskGraphError`] for unknown tasks or artifacts, duplicate
/// sources or bindings, self-consumption, and missing direct dependencies.
pub fn validate_artifact_flow<Operation>(
    graph: &TaskGraph<'_, Operation>,
    flow: &ArtifactFlow<'_>,
) -> Result<(), TaskGraphError> {
    validate_artifact_tasks(graph, flow)?;
    validate_external_inputs(flow)?;
    validate_task_outputs(flow)?;
    validate_task_inputs(graph, flow)
}

fn validate_artifact_tasks<Operation>(
    graph: &TaskGraph<'_, Operation>,
    flow: &ArtifactFlow<'_>,
) -> Result<(), TaskGraphError> {
    for input in flow.task_inputs {
        if graph.node(input.consumer).is_none() {
            return Err(TaskGraphError::UnknownTask(input.consumer));
        }
    }
    for output in flow.task_outputs {
        if graph.node(output.producer).is_none() {
            return Err(TaskGraphError::UnknownTask(output.producer));
        }
    }
    Ok(())
}

fn validate_external_inputs(flow: &ArtifactFlow<'_>) -> Result<(), TaskGraphError> {
    for (left_index, input) in flow.external_inputs.iter().enumerate() {
        let Some(tail) = flow.external_inputs.get(left_index.saturating_add(1)..) else {
            continue;
        };
        if tail.contains(input) {
            return Err(TaskGraphError::DuplicateExternalInput(*input));
        }
    }
    Ok(())
}

fn validate_task_outputs(flow: &ArtifactFlow<'_>) -> Result<(), TaskGraphError> {
    for (left_index, output) in flow.task_outputs.iter().enumerate() {
        if flow.external_inputs.contains(&output.artifact) {
            return Err(TaskGraphError::ExternalInputProducedByTask(output.artifact));
        }
        let Some(tail) = flow.task_outputs.get(left_index.saturating_add(1)..) else {
            continue;
        };
        if tail.iter().any(|other| other.artifact == output.artifact) {
            return Err(TaskGraphError::DuplicateArtifactProducer(output.artifact));
        }
    }
    Ok(())
}

fn validate_task_inputs<Operation>(
    graph: &TaskGraph<'_, Operation>,
    flow: &ArtifactFlow<'_>,
) -> Result<(), TaskGraphError> {
    for (left_index, input) in flow.task_inputs.iter().enumerate() {
        let Some(tail) = flow.task_inputs.get(left_index.saturating_add(1)..) else {
            continue;
        };
        if tail.iter().any(|other| other == input) {
            return Err(TaskGraphError::DuplicateTaskArtifactInput(*input));
        }
        if flow.external_inputs.contains(&input.artifact) {
            continue;
        }
        let Some(output) = flow
            .task_outputs
            .iter()
            .find(|output| output.artifact == input.artifact)
        else {
            return Err(TaskGraphError::UnknownArtifact(input.artifact));
        };
        if output.producer == input.consumer {
            return Err(TaskGraphError::SelfArtifactConsumption {
                task: input.consumer,
                artifact: input.artifact,
            });
        }
        if !graph.dependencies.iter().any(|dependency| {
            dependency.prerequisite == output.producer && dependency.dependent == input.consumer
        }) {
            return Err(TaskGraphError::MissingArtifactDependency {
                producer: output.producer,
                consumer: input.consumer,
                artifact: input.artifact,
            });
        }
    }
    Ok(())
}
