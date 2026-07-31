use core::num::NonZeroU16;

use domain_contracts::{BackendId, CapacityExhausted, CapacityResource, ModelId};

use crate::{artifact::ArtifactKind, error::TaskGraphError};

/// Identity of one orchestration task.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(u64);

impl TaskId {
    /// Creates an identifier from its stable numeric representation.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the stable numeric representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Operation represented by a workflow node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskKind {
    /// Produce an initial response or artifact.
    Draft,
    /// Review an artifact for defects.
    Review,
    /// Run the Rust compiler or another deterministic type checker.
    CompileCheck,
    /// Run a deterministic validator.
    Validate,
    /// Normalize raw diagnostics into a stable representation.
    NormalizeDiagnostics,
    /// Revise an artifact using prior findings.
    Revise,
    /// Aggregate multiple artifacts into one result.
    Aggregate,
    /// Application-defined operation code.
    Other(u16),
}

/// Output requirements declared before execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskOutputContract {
    /// Required artifact category.
    pub kind: ArtifactKind,
    /// Hard upper bound for persisted output bytes. Zero means externally bounded.
    pub maximum_bytes: u64,
}

/// Model-selection policy interpreted by an orchestration engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelPolicy {
    /// Use one exact logical model.
    Exact(ModelId),
    /// Prefer any compatible model implemented by one backend.
    PreferredBackend(BackendId),
    /// Use any compatible admitted model.
    AnyCompatible,
    /// Run no model because the task is deterministic.
    Deterministic,
}

/// Hard task budgets known before execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskBudget {
    /// Maximum input tokens admitted to the task.
    pub maximum_input_tokens: u32,
    /// Maximum output tokens admitted to the task.
    pub maximum_output_tokens: u32,
    /// Maximum execution attempts including the first attempt.
    pub maximum_attempts: NonZeroU16,
}

impl TaskBudget {
    /// Creates a task budget from validated bounds.
    #[must_use]
    pub const fn new(
        maximum_input_tokens: u32,
        maximum_output_tokens: u32,
        maximum_attempts: NonZeroU16,
    ) -> Self {
        Self {
            maximum_input_tokens,
            maximum_output_tokens,
            maximum_attempts,
        }
    }
}

/// Immutable node definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskNode {
    /// Stable task identity.
    pub id: TaskId,
    /// Operation performed by the node.
    pub kind: TaskKind,
    /// Model-selection policy.
    pub model_policy: ModelPolicy,
    /// Hard execution budgets.
    pub budget: TaskBudget,
    /// Declared output contract.
    pub output: TaskOutputContract,
}

/// Directed dependency requiring one task to succeed before another can start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskDependency {
    /// Task that must succeed first.
    pub prerequisite: TaskId,
    /// Task gated by the prerequisite.
    pub dependent: TaskId,
}

/// Borrowed immutable task graph.
#[derive(Clone, Copy, Debug)]
pub struct TaskGraph<'a> {
    /// Node definitions.
    pub nodes: &'a [TaskNode],
    /// Directed dependency edges.
    pub dependencies: &'a [TaskDependency],
}

impl<'a> TaskGraph<'a> {
    /// Creates a borrowed graph view.
    #[must_use]
    pub const fn new(nodes: &'a [TaskNode], dependencies: &'a [TaskDependency]) -> Self {
        Self {
            nodes,
            dependencies,
        }
    }

    /// Returns the node with the requested identity.
    #[must_use]
    pub fn node(&self, task_id: TaskId) -> Option<&TaskNode> {
        self.nodes.iter().find(|node| node.id == task_id)
    }

    /// Returns the node index with the requested identity.
    #[must_use]
    pub fn node_index(&self, task_id: TaskId) -> Option<usize> {
        self.nodes.iter().position(|node| node.id == task_id)
    }
}

/// Caller-owned scratch required for acyclic graph validation.
pub struct GraphValidationScratch<'a> {
    /// Per-node incoming-edge counts.
    pub incoming_counts: &'a mut [u32],
    /// Kahn traversal queue storing node indices.
    pub queue: &'a mut [usize],
}

/// Validates node identity, dependency integrity, and acyclicity.
///
/// # Errors
///
/// Returns [`TaskGraphError::CapacityExhausted`] when the validation scratch is
/// too small or an edge count overflows; [`TaskGraphError::DuplicateTask`] or
/// [`TaskGraphError::DuplicateDependency`] for duplicate definitions;
/// [`TaskGraphError::UnknownTask`] or [`TaskGraphError::SelfDependency`] for an
/// invalid edge; and [`TaskGraphError::CycleDetected`] when the graph is cyclic.
pub fn validate_graph(
    graph: &TaskGraph<'_>,
    scratch: GraphValidationScratch<'_>,
) -> Result<(), TaskGraphError> {
    validate_scratch(graph.nodes.len(), &scratch)?;
    validate_nodes(graph.nodes)?;
    validate_dependencies(graph)?;

    let GraphValidationScratch {
        incoming_counts,
        queue,
    } = scratch;
    let node_count = graph.nodes.len();
    let incoming_capacity = incoming_counts.len();
    let Some(counts) = incoming_counts.get_mut(..node_count) else {
        return Err(node_capacity(node_count, incoming_capacity));
    };
    counts.fill(0);

    for dependency in graph.dependencies {
        let dependent_index = graph
            .node_index(dependency.dependent)
            .ok_or(TaskGraphError::UnknownTask(dependency.dependent))?;
        let count_capacity = counts.len();
        let Some(count) = counts.get_mut(dependent_index) else {
            return Err(node_capacity(
                dependent_index.saturating_add(1),
                count_capacity,
            ));
        };
        let current = *count;
        let next = current.checked_add(1).ok_or_else(|| {
            TaskGraphError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::TaskEdges,
                u64::from(u32::MAX) + 1,
                u64::from(current),
            ))
        })?;
        *count = next;
    }

    let mut queue_length = 0_usize;
    for (index, &count) in counts.iter().enumerate() {
        if count == 0 {
            write_queue(queue, queue_length, index)?;
            queue_length += 1;
        }
    }

    let mut head = 0_usize;
    let mut visited = 0_usize;
    while head < queue_length {
        let Some(&node_index) = queue.get(head) else {
            return Err(node_capacity(head.saturating_add(1), queue.len()));
        };
        head += 1;
        visited += 1;
        let Some(node) = graph.nodes.get(node_index) else {
            return Err(node_capacity(
                node_index.saturating_add(1),
                graph.nodes.len(),
            ));
        };

        for dependency in graph
            .dependencies
            .iter()
            .filter(|dependency| dependency.prerequisite == node.id)
        {
            let dependent_index = graph
                .node_index(dependency.dependent)
                .ok_or(TaskGraphError::UnknownTask(dependency.dependent))?;
            let count_capacity = counts.len();
            let Some(count) = counts.get_mut(dependent_index) else {
                return Err(node_capacity(
                    dependent_index.saturating_add(1),
                    count_capacity,
                ));
            };
            if *count == 0 {
                return Err(TaskGraphError::DuplicateDependency(*dependency));
            }
            *count -= 1;
            if *count == 0 {
                write_queue(queue, queue_length, dependent_index)?;
                queue_length += 1;
            }
        }
    }

    if visited == graph.nodes.len() {
        Ok(())
    } else {
        Err(TaskGraphError::CycleDetected)
    }
}

fn validate_scratch(
    required: usize,
    scratch: &GraphValidationScratch<'_>,
) -> Result<(), TaskGraphError> {
    let available = scratch.incoming_counts.len().min(scratch.queue.len());
    if available < required {
        return Err(node_capacity(required, available));
    }
    Ok(())
}

fn validate_nodes(nodes: &[TaskNode]) -> Result<(), TaskGraphError> {
    for (left_index, left) in nodes.iter().enumerate() {
        let Some(tail) = nodes.get(left_index.saturating_add(1)..) else {
            continue;
        };
        if tail.iter().any(|right| right.id == left.id) {
            return Err(TaskGraphError::DuplicateTask(left.id));
        }
    }
    Ok(())
}

fn validate_dependencies(graph: &TaskGraph<'_>) -> Result<(), TaskGraphError> {
    for (left_index, dependency) in graph.dependencies.iter().enumerate() {
        if graph.node(dependency.prerequisite).is_none() {
            return Err(TaskGraphError::UnknownTask(dependency.prerequisite));
        }
        if graph.node(dependency.dependent).is_none() {
            return Err(TaskGraphError::UnknownTask(dependency.dependent));
        }
        if dependency.prerequisite == dependency.dependent {
            return Err(TaskGraphError::SelfDependency(dependency.prerequisite));
        }
        let Some(tail) = graph.dependencies.get(left_index.saturating_add(1)..) else {
            continue;
        };
        if tail.iter().any(|right| right == dependency) {
            return Err(TaskGraphError::DuplicateDependency(*dependency));
        }
    }
    Ok(())
}

fn write_queue(queue: &mut [usize], position: usize, value: usize) -> Result<(), TaskGraphError> {
    let available = queue.len();
    let Some(slot) = queue.get_mut(position) else {
        return Err(node_capacity(position.saturating_add(1), available));
    };
    *slot = value;
    Ok(())
}

const fn node_capacity(required: usize, available: usize) -> TaskGraphError {
    TaskGraphError::CapacityExhausted(CapacityExhausted::new(
        CapacityResource::TaskNodes,
        required as u64,
        available as u64,
    ))
}
