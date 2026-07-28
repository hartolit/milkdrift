#![no_std]
#![forbid(unsafe_code)]
#![doc = "Allocation-free task graph representation, validation, and runtime state."]

use core::num::NonZeroU16;

use domain_contracts::{
    ArtifactId, BackendId, CapacityExhausted, CapacityResource, ModelId, TaskId,
};

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

/// Identity of one started execution attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskAttempt {
    /// Task being executed.
    pub task: TaskId,
    /// One-based attempt number for the task.
    pub number: NonZeroU16,
}

impl TaskAttempt {
    /// Creates an attempt identity.
    #[must_use]
    pub const fn new(task: TaskId, number: NonZeroU16) -> Self {
        Self { task, number }
    }
}

/// Runtime state of one task.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskStatus {
    /// Waiting for all prerequisites.
    #[default]
    Pending,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Succeeded,
    /// Attempt failed and may be retried within budget.
    Failed,
    /// All configured attempts failed.
    Exhausted,
    /// Explicitly cancelled.
    Cancelled,
    /// Cannot run because a prerequisite terminated unsuccessfully.
    Blocked,
}

/// Mutable runtime fields aligned by index with graph nodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskRuntimeState {
    /// Current task state.
    pub status: TaskStatus,
    /// Number of attempts already started.
    pub attempts: u16,
}

/// Caller-owned runtime state table.
pub struct TaskStateTable<'a> {
    states: &'a mut [TaskRuntimeState],
}

impl<'a> TaskStateTable<'a> {
    /// Creates a state table after checking exact graph alignment.
    ///
    /// # Errors
    ///
    /// Returns [`TaskGraphError::StateLengthMismatch`] when `states` does not
    /// contain exactly one entry per graph node.
    pub const fn new(
        graph: &TaskGraph<'_>,
        states: &'a mut [TaskRuntimeState],
    ) -> Result<Self, TaskGraphError> {
        if states.len() != graph.nodes.len() {
            return Err(TaskGraphError::StateLengthMismatch {
                required: graph.nodes.len(),
                available: states.len(),
            });
        }
        Ok(Self { states })
    }

    /// Returns immutable state for one task.
    #[must_use]
    pub fn state(&self, graph: &TaskGraph<'_>, task_id: TaskId) -> Option<TaskRuntimeState> {
        let index = graph.node_index(task_id)?;
        self.states.get(index).copied()
    }

    /// Starts one ready task and increments its attempt count.
    ///
    /// # Errors
    ///
    /// Returns [`TaskGraphError::UnknownTask`] when the task or a prerequisite is
    /// absent, [`TaskGraphError::StateLengthMismatch`] when prerequisite state is
    /// unavailable, [`TaskGraphError::InvalidTransition`] when the task is not
    /// pending or failed or its prerequisites have not succeeded, and
    /// [`TaskGraphError::AttemptLimitReached`] when its attempt budget is spent.
    pub fn start(
        &mut self,
        graph: &TaskGraph<'_>,
        task_id: TaskId,
    ) -> Result<TaskAttempt, TaskGraphError> {
        let index = graph
            .node_index(task_id)
            .ok_or(TaskGraphError::UnknownTask(task_id))?;
        let node = graph
            .nodes
            .get(index)
            .ok_or(TaskGraphError::UnknownTask(task_id))?;
        let current = self
            .states
            .get(index)
            .copied()
            .ok_or(TaskGraphError::UnknownTask(task_id))?;
        if current.status != TaskStatus::Pending && current.status != TaskStatus::Failed {
            return Err(TaskGraphError::InvalidTransition {
                task: task_id,
                state: current.status,
            });
        }
        if !prerequisites_succeeded(graph, self.states, task_id)? {
            return Err(TaskGraphError::InvalidTransition {
                task: task_id,
                state: current.status,
            });
        }
        if current.attempts >= node.budget.maximum_attempts.get() {
            return Err(TaskGraphError::AttemptLimitReached(task_id));
        }
        let Some(state) = self.states.get_mut(index) else {
            return Err(TaskGraphError::UnknownTask(task_id));
        };
        let Some(number) = NonZeroU16::new(current.attempts.saturating_add(1)) else {
            return Err(TaskGraphError::AttemptLimitReached(task_id));
        };
        state.attempts = number.get();
        state.status = TaskStatus::Running;
        Ok(TaskAttempt::new(task_id, number))
    }

    /// Marks the running attempt identified by `attempt` successful.
    ///
    /// # Errors
    ///
    /// Returns [`TaskGraphError::UnknownTask`] when the token's task or state is
    /// absent, or [`TaskGraphError::InvalidAttempt`] when the token is stale or
    /// the identified task is not running.
    pub fn succeed_attempt(
        &mut self,
        graph: &TaskGraph<'_>,
        attempt: TaskAttempt,
    ) -> Result<(), TaskGraphError> {
        let index = self.validate_attempt(graph, attempt)?;
        let Some(state) = self.states.get_mut(index) else {
            return Err(TaskGraphError::UnknownTask(attempt.task));
        };
        state.status = TaskStatus::Succeeded;
        Ok(())
    }

    /// Marks the running attempt identified by `attempt` failed.
    ///
    /// The task becomes exhausted when this attempt spends its retry budget.
    ///
    /// # Errors
    ///
    /// Returns [`TaskGraphError::UnknownTask`] when the token's task or state is
    /// absent, or [`TaskGraphError::InvalidAttempt`] when the token is stale or
    /// the identified task is not running.
    pub fn fail_attempt(
        &mut self,
        graph: &TaskGraph<'_>,
        attempt: TaskAttempt,
    ) -> Result<(), TaskGraphError> {
        let index = self.validate_attempt(graph, attempt)?;
        let node = graph
            .nodes
            .get(index)
            .ok_or(TaskGraphError::UnknownTask(attempt.task))?;
        let Some(state) = self.states.get_mut(index) else {
            return Err(TaskGraphError::UnknownTask(attempt.task));
        };
        state.status = if state.attempts >= node.budget.maximum_attempts.get() {
            TaskStatus::Exhausted
        } else {
            TaskStatus::Failed
        };
        Ok(())
    }

    /// Cancels a pending, failed, or running task.
    ///
    /// # Errors
    ///
    /// Returns [`TaskGraphError::UnknownTask`] when the task or its state is
    /// absent, or [`TaskGraphError::InvalidTransition`] when it is not pending,
    /// failed, or running.
    pub fn cancel(&mut self, graph: &TaskGraph<'_>, task_id: TaskId) -> Result<(), TaskGraphError> {
        let index = graph
            .node_index(task_id)
            .ok_or(TaskGraphError::UnknownTask(task_id))?;
        let Some(state) = self.states.get_mut(index) else {
            return Err(TaskGraphError::UnknownTask(task_id));
        };
        match state.status {
            TaskStatus::Pending | TaskStatus::Failed | TaskStatus::Running => {
                state.status = TaskStatus::Cancelled;
                Ok(())
            }
            current => Err(TaskGraphError::InvalidTransition {
                task: task_id,
                state: current,
            }),
        }
    }

    /// Marks all pending descendants of unsuccessful prerequisites as blocked.
    ///
    /// # Errors
    ///
    /// Returns [`TaskGraphError::UnknownTask`] when a dependency references an
    /// absent task, or [`TaskGraphError::StateLengthMismatch`] when runtime state
    /// is unavailable for a graph node.
    pub fn propagate_blocked(&mut self, graph: &TaskGraph<'_>) -> Result<usize, TaskGraphError> {
        let mut total_changed = 0_usize;
        loop {
            let mut changed = 0_usize;
            for (index, node) in graph.nodes.iter().enumerate() {
                let Some(current) = self.states.get(index).copied() else {
                    return Err(TaskGraphError::StateLengthMismatch {
                        required: graph.nodes.len(),
                        available: self.states.len(),
                    });
                };
                if current.status != TaskStatus::Pending {
                    continue;
                }
                if has_unsuccessful_prerequisite(graph, self.states, node.id)? {
                    let Some(state) = self.states.get_mut(index) else {
                        return Err(TaskGraphError::UnknownTask(node.id));
                    };
                    state.status = TaskStatus::Blocked;
                    changed += 1;
                }
            }
            total_changed = total_changed.saturating_add(changed);
            if changed == 0 {
                return Ok(total_changed);
            }
        }
    }

    /// Writes all currently ready task identities into caller-owned output.
    ///
    /// # Errors
    ///
    /// Returns [`TaskGraphError::UnknownTask`] when a dependency references an
    /// absent task, [`TaskGraphError::StateLengthMismatch`] when runtime state is
    /// unavailable for a graph node, or [`TaskGraphError::CapacityExhausted`]
    /// when `output` cannot hold every ready task.
    pub fn ready_tasks(
        &self,
        graph: &TaskGraph<'_>,
        output: &mut [TaskId],
    ) -> Result<usize, TaskGraphError> {
        let mut written = 0_usize;
        for (index, node) in graph.nodes.iter().enumerate() {
            let Some(state) = self.states.get(index) else {
                return Err(TaskGraphError::StateLengthMismatch {
                    required: graph.nodes.len(),
                    available: self.states.len(),
                });
            };
            if (state.status == TaskStatus::Pending || state.status == TaskStatus::Failed)
                && prerequisites_succeeded(graph, self.states, node.id)?
                && state.attempts < node.budget.maximum_attempts.get()
            {
                let available = output.len();
                let Some(slot) = output.get_mut(written) else {
                    return Err(TaskGraphError::CapacityExhausted(CapacityExhausted::new(
                        CapacityResource::TaskNodes,
                        written.saturating_add(1) as u64,
                        available as u64,
                    )));
                };
                *slot = node.id;
                written += 1;
            }
        }
        Ok(written)
    }

    fn validate_attempt(
        &self,
        graph: &TaskGraph<'_>,
        attempt: TaskAttempt,
    ) -> Result<usize, TaskGraphError> {
        let index = graph
            .node_index(attempt.task)
            .ok_or(TaskGraphError::UnknownTask(attempt.task))?;
        let state = self
            .states
            .get(index)
            .copied()
            .ok_or(TaskGraphError::UnknownTask(attempt.task))?;
        let active =
            NonZeroU16::new(state.attempts).map(|number| TaskAttempt::new(attempt.task, number));
        if state.status != TaskStatus::Running || active != Some(attempt) {
            return Err(TaskGraphError::InvalidAttempt {
                attempt,
                active,
                state: state.status,
            });
        }
        Ok(index)
    }
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

fn prerequisites_succeeded(
    graph: &TaskGraph<'_>,
    states: &[TaskRuntimeState],
    task_id: TaskId,
) -> Result<bool, TaskGraphError> {
    for dependency in graph
        .dependencies
        .iter()
        .filter(|dependency| dependency.dependent == task_id)
    {
        let index = graph
            .node_index(dependency.prerequisite)
            .ok_or(TaskGraphError::UnknownTask(dependency.prerequisite))?;
        let Some(state) = states.get(index) else {
            return Err(TaskGraphError::StateLengthMismatch {
                required: graph.nodes.len(),
                available: states.len(),
            });
        };
        if state.status != TaskStatus::Succeeded {
            return Ok(false);
        }
    }
    Ok(true)
}

fn has_unsuccessful_prerequisite(
    graph: &TaskGraph<'_>,
    states: &[TaskRuntimeState],
    task_id: TaskId,
) -> Result<bool, TaskGraphError> {
    for dependency in graph
        .dependencies
        .iter()
        .filter(|dependency| dependency.dependent == task_id)
    {
        let index = graph
            .node_index(dependency.prerequisite)
            .ok_or(TaskGraphError::UnknownTask(dependency.prerequisite))?;
        let Some(state) = states.get(index) else {
            return Err(TaskGraphError::StateLengthMismatch {
                required: graph.nodes.len(),
                available: states.len(),
            });
        };
        if matches!(
            state.status,
            TaskStatus::Exhausted | TaskStatus::Cancelled | TaskStatus::Blocked
        ) {
            return Ok(true);
        }
    }
    Ok(false)
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
