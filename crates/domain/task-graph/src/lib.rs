#![no_std]
#![forbid(unsafe_code)]
#![doc = "Allocation-free task graph representation, validation, and runtime state."]

mod artifact;
mod error;
mod graph;
mod state;

pub use artifact::{
    ArtifactFlow, ArtifactKind, ArtifactReference, ArtifactRole, TaskArtifactInput,
    TaskArtifactOutput, validate_artifact_flow,
};
pub use error::TaskGraphError;
pub use graph::{
    GraphValidationScratch, ModelPolicy, TaskBudget, TaskDependency, TaskGraph, TaskId, TaskKind,
    TaskNode, TaskOutputContract, validate_graph,
};
pub use state::{TaskAttempt, TaskRuntimeState, TaskStateTable, TaskStatus};
