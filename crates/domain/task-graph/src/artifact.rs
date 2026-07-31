use domain_contracts::ArtifactId;

use crate::{
    error::TaskGraphError,
    graph::{TaskGraph, TaskId},
};

/// Artifact category produced by a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArtifactKind {
    /// Plain UTF-8 text.
    Text,
    /// Source code.
    SourceCode,
    /// Structured compiler or review findings.
    Diagnostics,
    /// Token sequence.
    Tokens,
    /// Application-defined artifact category.
    Other(u16),
}

/// Semantic role an artifact serves in a workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArtifactRole {
    /// Workflow specification supplied to generation tasks.
    Specification,
    /// Initial generated artifact.
    Draft,
    /// Unprocessed diagnostics emitted by a checker.
    RawDiagnostics,
    /// Diagnostics normalized for downstream consumption.
    NormalizedDiagnostics,
    /// Review findings for a draft or revision.
    Review,
    /// Revised artifact produced from prior findings.
    Revision,
    /// Final deterministic validation result.
    FinalValidation,
    /// Application-defined artifact role.
    Other(u16),
}

/// Immutable artifact reference passed between tasks by identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactReference {
    /// Artifact identity.
    pub id: ArtifactId,
    /// Artifact category.
    pub kind: ArtifactKind,
    /// Semantic role within the workflow.
    pub role: ArtifactRole,
}

/// Artifact consumed by one task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskArtifactInput {
    /// Task that consumes the artifact.
    pub consumer: TaskId,
    /// Artifact reference expected by the consumer.
    pub artifact: ArtifactReference,
}

/// Artifact produced by one task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskArtifactOutput {
    /// Task that produces the artifact.
    pub producer: TaskId,
    /// Artifact reference emitted by the producer.
    pub artifact: ArtifactReference,
}

/// Borrowed artifact declarations and task bindings for a workflow.
#[derive(Clone, Copy, Debug)]
pub struct ArtifactFlow<'a> {
    /// Artifacts supplied externally to the workflow.
    pub workflow_inputs: &'a [ArtifactReference],
    /// Artifact bindings consumed by tasks.
    pub task_inputs: &'a [TaskArtifactInput],
    /// Artifact bindings produced by tasks.
    pub task_outputs: &'a [TaskArtifactOutput],
}

impl<'a> ArtifactFlow<'a> {
    /// Creates a borrowed artifact-flow view.
    #[must_use]
    pub const fn new(
        workflow_inputs: &'a [ArtifactReference],
        task_inputs: &'a [TaskArtifactInput],
        task_outputs: &'a [TaskArtifactOutput],
    ) -> Self {
        Self {
            workflow_inputs,
            task_inputs,
            task_outputs,
        }
    }
}

/// Validates artifact provenance and direct task-to-task data dependencies.
///
/// Validation uses only borrowed declarations and repeated slice scans; it does
/// not allocate or require caller-owned scratch storage.
///
/// # Errors
///
/// Returns [`TaskGraphError::UnknownTask`] for bindings involving absent tasks;
/// [`TaskGraphError::DuplicateWorkflowInput`],
/// [`TaskGraphError::WorkflowInputProducedByTask`], or
/// [`TaskGraphError::DuplicateArtifactProducer`] for ambiguous artifact sources;
/// [`TaskGraphError::MissingTaskOutput`],
/// [`TaskGraphError::DuplicateTaskOutput`], or
/// [`TaskGraphError::TaskOutputKindMismatch`] for invalid output declarations;
/// [`TaskGraphError::UnknownArtifact`] or
/// [`TaskGraphError::ArtifactReferenceMismatch`] for an input without an exact
/// source; [`TaskGraphError::DuplicateTaskArtifactInput`] for a repeated binding;
/// [`TaskGraphError::SelfArtifactConsumption`] for a task consuming its own
/// output; and [`TaskGraphError::MissingArtifactDependency`] when a consumer has
/// no direct dependency on the task producing its input.
pub fn validate_artifact_flow(
    graph: &TaskGraph<'_>,
    flow: &ArtifactFlow<'_>,
) -> Result<(), TaskGraphError> {
    validate_artifact_tasks(graph, flow)?;
    validate_workflow_inputs(flow)?;
    validate_task_outputs(graph, flow)?;
    validate_task_output_contracts(graph, flow)?;
    validate_task_inputs(graph, flow)
}

fn validate_artifact_tasks(
    graph: &TaskGraph<'_>,
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

fn validate_workflow_inputs(flow: &ArtifactFlow<'_>) -> Result<(), TaskGraphError> {
    for (left_index, input) in flow.workflow_inputs.iter().enumerate() {
        let Some(tail) = flow.workflow_inputs.get(left_index.saturating_add(1)..) else {
            continue;
        };
        if tail.iter().any(|other| other.id == input.id) {
            return Err(TaskGraphError::DuplicateWorkflowInput(input.id));
        }
    }
    Ok(())
}

fn validate_task_outputs(
    _graph: &TaskGraph<'_>,
    flow: &ArtifactFlow<'_>,
) -> Result<(), TaskGraphError> {
    for (left_index, output) in flow.task_outputs.iter().enumerate() {
        if flow
            .workflow_inputs
            .iter()
            .any(|input| input.id == output.artifact.id)
        {
            return Err(TaskGraphError::WorkflowInputProducedByTask(
                output.artifact.id,
            ));
        }
        let Some(tail) = flow.task_outputs.get(left_index.saturating_add(1)..) else {
            continue;
        };
        if tail
            .iter()
            .any(|other| other.artifact.id == output.artifact.id)
        {
            return Err(TaskGraphError::DuplicateArtifactProducer(
                output.artifact.id,
            ));
        }
    }
    Ok(())
}

fn validate_task_output_contracts(
    graph: &TaskGraph<'_>,
    flow: &ArtifactFlow<'_>,
) -> Result<(), TaskGraphError> {
    for node in graph.nodes {
        let mut matching = flow
            .task_outputs
            .iter()
            .filter(|output| output.producer == node.id);
        let Some(output) = matching.next() else {
            return Err(TaskGraphError::MissingTaskOutput(node.id));
        };
        if matching.next().is_some() {
            return Err(TaskGraphError::DuplicateTaskOutput(node.id));
        }
        if output.artifact.kind != node.output.kind {
            return Err(TaskGraphError::TaskOutputKindMismatch {
                task: node.id,
                expected: node.output.kind,
                actual: output.artifact.kind,
            });
        }
    }
    Ok(())
}

fn validate_task_inputs(
    graph: &TaskGraph<'_>,
    flow: &ArtifactFlow<'_>,
) -> Result<(), TaskGraphError> {
    for (left_index, input) in flow.task_inputs.iter().enumerate() {
        let Some(tail) = flow.task_inputs.get(left_index.saturating_add(1)..) else {
            continue;
        };
        if tail.iter().any(|other| other == input) {
            return Err(TaskGraphError::DuplicateTaskArtifactInput(*input));
        }

        if let Some(source) = flow
            .workflow_inputs
            .iter()
            .find(|source| source.id == input.artifact.id)
        {
            validate_artifact_reference(*source, input.artifact)?;
            continue;
        }

        let Some(output) = flow
            .task_outputs
            .iter()
            .find(|output| output.artifact.id == input.artifact.id)
        else {
            return Err(TaskGraphError::UnknownArtifact(input.artifact.id));
        };
        validate_artifact_reference(output.artifact, input.artifact)?;
        if output.producer == input.consumer {
            return Err(TaskGraphError::SelfArtifactConsumption {
                task: input.consumer,
                artifact: input.artifact.id,
            });
        }
        if !graph.dependencies.iter().any(|dependency| {
            dependency.prerequisite == output.producer && dependency.dependent == input.consumer
        }) {
            return Err(TaskGraphError::MissingArtifactDependency {
                producer: output.producer,
                consumer: input.consumer,
                artifact: input.artifact.id,
            });
        }
    }
    Ok(())
}

fn validate_artifact_reference(
    expected: ArtifactReference,
    actual: ArtifactReference,
) -> Result<(), TaskGraphError> {
    if expected != actual {
        return Err(TaskGraphError::ArtifactReferenceMismatch { expected, actual });
    }
    Ok(())
}
