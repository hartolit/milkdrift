//! Identity-only, allocation-free artifact provenance validation.

use core::num::NonZeroU16;

use domain_contracts::ArtifactId;
use task_graph::{
    ArtifactFlow, TaskArtifactInput, TaskArtifactOutput, TaskDependency, TaskGraph, TaskGraphError,
    TaskId, TaskNode, validate_artifact_flow,
};

const fn node(id: u64) -> TaskNode<()> {
    TaskNode {
        id: TaskId::new(id),
        operation: (),
        maximum_attempts: NonZeroU16::MIN,
    }
}

#[test]
fn external_and_multi_output_provenance_is_generic() {
    let nodes = [node(1), node(2), node(3)];
    let dependencies = [
        TaskDependency {
            prerequisite: TaskId::new(1),
            dependent: TaskId::new(2),
        },
        TaskDependency {
            prerequisite: TaskId::new(1),
            dependent: TaskId::new(3),
        },
    ];
    let external = [ArtifactId::new(10)];
    let outputs = [
        TaskArtifactOutput {
            producer: TaskId::new(1),
            artifact: ArtifactId::new(20),
        },
        TaskArtifactOutput {
            producer: TaskId::new(1),
            artifact: ArtifactId::new(21),
        },
    ];
    let inputs = [
        TaskArtifactInput {
            consumer: TaskId::new(1),
            artifact: ArtifactId::new(10),
        },
        TaskArtifactInput {
            consumer: TaskId::new(2),
            artifact: ArtifactId::new(20),
        },
        TaskArtifactInput {
            consumer: TaskId::new(3),
            artifact: ArtifactId::new(21),
        },
    ];
    let graph = TaskGraph::new(&nodes, &dependencies);

    assert_eq!(
        validate_artifact_flow(&graph, &ArtifactFlow::new(&external, &inputs, &outputs)),
        Ok(())
    );
}

#[test]
fn tasks_are_not_required_to_produce_artifacts() {
    let nodes = [node(1), node(2)];
    let graph = TaskGraph::new(&nodes, &[]);
    assert_eq!(
        validate_artifact_flow(&graph, &ArtifactFlow::new(&[], &[], &[])),
        Ok(())
    );
}

#[test]
fn duplicate_and_missing_producers_are_rejected() {
    let nodes = [node(1), node(2)];
    let graph = TaskGraph::new(&nodes, &[]);
    let duplicate = ArtifactId::new(20);
    let outputs = [
        TaskArtifactOutput {
            producer: TaskId::new(1),
            artifact: duplicate,
        },
        TaskArtifactOutput {
            producer: TaskId::new(2),
            artifact: duplicate,
        },
    ];
    assert_eq!(
        validate_artifact_flow(&graph, &ArtifactFlow::new(&[], &[], &outputs)),
        Err(TaskGraphError::DuplicateArtifactProducer(duplicate))
    );

    let missing = ArtifactId::new(99);
    let inputs = [TaskArtifactInput {
        consumer: TaskId::new(1),
        artifact: missing,
    }];
    assert_eq!(
        validate_artifact_flow(&graph, &ArtifactFlow::new(&[], &inputs, &[])),
        Err(TaskGraphError::UnknownArtifact(missing))
    );
}

#[test]
fn produced_artifact_requires_direct_dependency() {
    let nodes = [node(1), node(2)];
    let artifact = ArtifactId::new(20);
    let inputs = [TaskArtifactInput {
        consumer: TaskId::new(2),
        artifact,
    }];
    let outputs = [TaskArtifactOutput {
        producer: TaskId::new(1),
        artifact,
    }];
    let graph = TaskGraph::new(&nodes, &[]);

    assert_eq!(
        validate_artifact_flow(&graph, &ArtifactFlow::new(&[], &inputs, &outputs)),
        Err(TaskGraphError::MissingArtifactDependency {
            producer: TaskId::new(1),
            consumer: TaskId::new(2),
            artifact,
        })
    );
}
