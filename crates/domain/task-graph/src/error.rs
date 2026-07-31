use domain_contracts::{ArtifactId, CapacityExhausted};

use crate::{
    artifact::{ArtifactKind, ArtifactReference, TaskArtifactInput},
    graph::{TaskDependency, TaskId},
    state::{TaskAttempt, TaskStatus},
};

/// Stable graph or workflow-state failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskGraphError {
    /// Two nodes share one identity.
    DuplicateTask(TaskId),
    /// Two identical dependency edges were supplied.
    DuplicateDependency(TaskDependency),
    /// A dependency references an unknown task.
    UnknownTask(TaskId),
    /// A task depends directly on itself.
    SelfDependency(TaskId),
    /// The graph contains at least one directed cycle.
    CycleDetected,
    /// A caller-owned fixed-capacity buffer is too small.
    CapacityExhausted(CapacityExhausted),
    /// Runtime state storage does not match the graph node count.
    StateLengthMismatch {
        /// Number of graph nodes.
        required: usize,
        /// Number of supplied state entries.
        available: usize,
    },
    /// Requested task transition is invalid.
    InvalidTransition {
        /// Task being transitioned.
        task: TaskId,
        /// Current state.
        state: TaskStatus,
    },
    /// Task exhausted its configured attempt budget.
    AttemptLimitReached(TaskId),
    /// Two workflow inputs share one artifact identity.
    DuplicateWorkflowInput(ArtifactId),
    /// A workflow input is also declared as a task output.
    WorkflowInputProducedByTask(ArtifactId),
    /// More than one task output uses one artifact identity.
    DuplicateArtifactProducer(ArtifactId),
    /// A graph task has no declared artifact output.
    MissingTaskOutput(TaskId),
    /// A graph task has more than one declared artifact output.
    DuplicateTaskOutput(TaskId),
    /// A task output does not satisfy the node's declared output kind.
    TaskOutputKindMismatch {
        /// Task whose output kind is inconsistent.
        task: TaskId,
        /// Kind required by the task node.
        expected: ArtifactKind,
        /// Kind declared by the artifact output.
        actual: ArtifactKind,
    },
    /// A task input references no workflow input or task output.
    UnknownArtifact(ArtifactId),
    /// A task input's kind or role differs from its source declaration.
    ArtifactReferenceMismatch {
        /// Complete reference declared by the source.
        expected: ArtifactReference,
        /// Complete reference requested by the consumer.
        actual: ArtifactReference,
    },
    /// The same task input binding was declared more than once.
    DuplicateTaskArtifactInput(TaskArtifactInput),
    /// A task consumes an artifact that it produces itself.
    SelfArtifactConsumption {
        /// Task consuming its own output.
        task: TaskId,
        /// Self-produced artifact identity.
        artifact: ArtifactId,
    },
    /// A produced artifact is consumed without a direct graph dependency.
    MissingArtifactDependency {
        /// Task producing the consumed artifact.
        producer: TaskId,
        /// Task consuming the produced artifact.
        consumer: TaskId,
        /// Artifact requiring the dependency.
        artifact: ArtifactId,
    },
    /// A completion token is stale or does not identify a running attempt.
    InvalidAttempt {
        /// Attempt supplied by the caller.
        attempt: TaskAttempt,
        /// Current attempt identity, when the task has been started.
        active: Option<TaskAttempt>,
        /// Current runtime status of the task.
        state: TaskStatus,
    },
}

impl From<CapacityExhausted> for TaskGraphError {
    fn from(value: CapacityExhausted) -> Self {
        Self::CapacityExhausted(value)
    }
}
