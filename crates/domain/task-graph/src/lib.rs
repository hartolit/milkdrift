#![no_std]
#![forbid(unsafe_code)]
#![doc = "Generic allocation-free directed task graph mechanics and provenance."]

mod artifact;
mod error;
mod graph;
mod state;

pub use artifact::{ArtifactFlow, TaskArtifactInput, TaskArtifactOutput, validate_artifact_flow};
pub use error::TaskGraphError;
pub use graph::{
    GraphValidationScratch, TaskDependency, TaskGraph, TaskId, TaskNode, validate_graph,
};
pub use state::{TaskAttempt, TaskRuntimeState, TaskStateTable, TaskStatus};
