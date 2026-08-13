use domain_contracts::{CapacityExhausted, CapacityResource};

use crate::error::TaskGraphError;

/// Identity of one node in a directed task graph.
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

/// Immutable node definition carrying caller-owned operation metadata.
///
/// Graph algorithms never inspect `operation`; it can be an opaque operation
/// code or a small domain-specific definition owned by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskNode<Operation> {
    /// Stable task identity within this graph.
    pub id: TaskId,
    /// Caller-owned operation metadata.
    pub operation: Operation,
    /// Maximum attempts, including the first attempt.
    pub maximum_attempts: core::num::NonZeroU16,
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
pub struct TaskGraph<'a, Operation> {
    /// Node definitions in deterministic scheduling order.
    pub nodes: &'a [TaskNode<Operation>],
    /// Directed dependency edges.
    pub dependencies: &'a [TaskDependency],
}

impl<'a, Operation> TaskGraph<'a, Operation> {
    /// Creates a borrowed graph view.
    #[must_use]
    pub const fn new(nodes: &'a [TaskNode<Operation>], dependencies: &'a [TaskDependency]) -> Self {
        Self {
            nodes,
            dependencies,
        }
    }

    /// Returns the node with the requested identity.
    #[must_use]
    pub fn node(&self, task_id: TaskId) -> Option<&TaskNode<Operation>> {
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
/// The traversal uses only caller-owned scratch and never interprets node
/// operation metadata.
///
/// # Errors
///
/// Returns [`TaskGraphError::CapacityExhausted`] when validation scratch is too
/// small or an edge count overflows; duplicate, unknown, or self-referential
/// definitions receive their corresponding typed errors; cyclic graphs return
/// [`TaskGraphError::CycleDetected`].
pub fn validate_graph<Operation>(
    graph: &TaskGraph<'_, Operation>,
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
        *count = current.checked_add(1).ok_or_else(|| {
            TaskGraphError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::TaskEdges,
                u64::from(u32::MAX) + 1,
                u64::from(current),
            ))
        })?;
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

fn validate_nodes<Operation>(nodes: &[TaskNode<Operation>]) -> Result<(), TaskGraphError> {
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

fn validate_dependencies<Operation>(
    graph: &TaskGraph<'_, Operation>,
) -> Result<(), TaskGraphError> {
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
