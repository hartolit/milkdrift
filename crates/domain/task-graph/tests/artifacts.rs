//! Integration tests for allocation-free artifact-flow validation.

use core::num::NonZeroU16;

use domain_contracts::{ArtifactId, TaskId};
use task_graph::{
    ArtifactFlow, ArtifactKind, ArtifactReference, ArtifactRole, ModelPolicy, TaskArtifactInput,
    TaskArtifactOutput, TaskBudget, TaskDependency, TaskGraph, TaskGraphError, TaskKind, TaskNode,
    TaskOutputContract, validate_artifact_flow,
};

fn node(id: u64, kind: TaskKind, output_kind: ArtifactKind) -> Result<TaskNode, &'static str> {
    let attempts = NonZeroU16::new(2).ok_or("attempt count must be non-zero")?;
    Ok(TaskNode {
        id: TaskId::new(id),
        kind,
        model_policy: ModelPolicy::Deterministic,
        budget: TaskBudget::new(1024, 512, attempts),
        output: TaskOutputContract {
            kind: output_kind,
            maximum_bytes: 4096,
        },
    })
}

const fn artifact(id: u64, kind: ArtifactKind, role: ArtifactRole) -> ArtifactReference {
    ArtifactReference {
        id: ArtifactId::new(id),
        kind,
        role,
    }
}

#[test]
fn valid_artifact_flow_is_accepted() -> Result<(), &'static str> {
    let nodes = [
        node(1, TaskKind::Draft, ArtifactKind::Text)?,
        node(2, TaskKind::Review, ArtifactKind::Diagnostics)?,
    ];
    let dependencies = [TaskDependency {
        prerequisite: TaskId::new(1),
        dependent: TaskId::new(2),
    }];
    let specification = artifact(10, ArtifactKind::Text, ArtifactRole::Specification);
    let draft = artifact(20, ArtifactKind::Text, ArtifactRole::Draft);
    let review = artifact(30, ArtifactKind::Diagnostics, ArtifactRole::Review);
    let workflow_inputs = [specification];
    let task_inputs = [
        TaskArtifactInput {
            consumer: TaskId::new(1),
            artifact: specification,
        },
        TaskArtifactInput {
            consumer: TaskId::new(2),
            artifact: draft,
        },
    ];
    let task_outputs = [
        TaskArtifactOutput {
            producer: TaskId::new(1),
            artifact: draft,
        },
        TaskArtifactOutput {
            producer: TaskId::new(2),
            artifact: review,
        },
    ];
    let graph = TaskGraph::new(&nodes, &dependencies);
    let flow = ArtifactFlow::new(&workflow_inputs, &task_inputs, &task_outputs);

    validate_artifact_flow(&graph, &flow).map_err(|_| "valid artifact flow rejected")
}

#[test]
fn duplicate_task_output_provenance_is_rejected() -> Result<(), &'static str> {
    let nodes = [
        node(1, TaskKind::Draft, ArtifactKind::Text)?,
        node(2, TaskKind::Review, ArtifactKind::Text)?,
    ];
    let duplicate = artifact(20, ArtifactKind::Text, ArtifactRole::Draft);
    let task_outputs = [
        TaskArtifactOutput {
            producer: TaskId::new(1),
            artifact: duplicate,
        },
        TaskArtifactOutput {
            producer: TaskId::new(2),
            artifact: duplicate,
        },
    ];
    let graph = TaskGraph::new(&nodes, &[]);
    let flow = ArtifactFlow::new(&[], &[], &task_outputs);

    assert_eq!(
        validate_artifact_flow(&graph, &flow),
        Err(TaskGraphError::DuplicateArtifactProducer(duplicate.id))
    );
    Ok(())
}

#[test]
fn missing_artifact_provenance_is_rejected() -> Result<(), &'static str> {
    let nodes = [node(1, TaskKind::Draft, ArtifactKind::Text)?];
    let output = artifact(20, ArtifactKind::Text, ArtifactRole::Draft);
    let missing = artifact(99, ArtifactKind::Text, ArtifactRole::Specification);
    let task_inputs = [TaskArtifactInput {
        consumer: TaskId::new(1),
        artifact: missing,
    }];
    let task_outputs = [TaskArtifactOutput {
        producer: TaskId::new(1),
        artifact: output,
    }];
    let graph = TaskGraph::new(&nodes, &[]);
    let flow = ArtifactFlow::new(&[], &task_inputs, &task_outputs);

    assert_eq!(
        validate_artifact_flow(&graph, &flow),
        Err(TaskGraphError::UnknownArtifact(missing.id))
    );
    Ok(())
}

#[test]
fn mismatched_artifact_provenance_is_rejected() -> Result<(), &'static str> {
    let nodes = [node(1, TaskKind::Draft, ArtifactKind::Text)?];
    let specification = artifact(10, ArtifactKind::Text, ArtifactRole::Specification);
    let mismatched = artifact(10, ArtifactKind::Text, ArtifactRole::Draft);
    let output = artifact(20, ArtifactKind::Text, ArtifactRole::Draft);
    let workflow_inputs = [specification];
    let task_inputs = [TaskArtifactInput {
        consumer: TaskId::new(1),
        artifact: mismatched,
    }];
    let task_outputs = [TaskArtifactOutput {
        producer: TaskId::new(1),
        artifact: output,
    }];
    let graph = TaskGraph::new(&nodes, &[]);
    let flow = ArtifactFlow::new(&workflow_inputs, &task_inputs, &task_outputs);

    assert_eq!(
        validate_artifact_flow(&graph, &flow),
        Err(TaskGraphError::ArtifactReferenceMismatch {
            expected: specification,
            actual: mismatched,
        })
    );
    Ok(())
}

#[test]
fn task_artifact_requires_direct_dependency() -> Result<(), &'static str> {
    let nodes = [
        node(1, TaskKind::Draft, ArtifactKind::Text)?,
        node(2, TaskKind::Review, ArtifactKind::Diagnostics)?,
    ];
    let draft = artifact(20, ArtifactKind::Text, ArtifactRole::Draft);
    let review = artifact(30, ArtifactKind::Diagnostics, ArtifactRole::Review);
    let task_inputs = [TaskArtifactInput {
        consumer: TaskId::new(2),
        artifact: draft,
    }];
    let task_outputs = [
        TaskArtifactOutput {
            producer: TaskId::new(1),
            artifact: draft,
        },
        TaskArtifactOutput {
            producer: TaskId::new(2),
            artifact: review,
        },
    ];
    let graph = TaskGraph::new(&nodes, &[]);
    let flow = ArtifactFlow::new(&[], &task_inputs, &task_outputs);

    assert_eq!(
        validate_artifact_flow(&graph, &flow),
        Err(TaskGraphError::MissingArtifactDependency {
            producer: TaskId::new(1),
            consumer: TaskId::new(2),
            artifact: draft.id,
        })
    );
    Ok(())
}
