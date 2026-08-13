use core::num::NonZeroU16;

use domain_contracts::{CapacityExhausted, CapacityResource};

use crate::{
    error::TaskGraphError,
    graph::{TaskGraph, TaskId},
};

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
    pub const fn new<Operation>(
        graph: &TaskGraph<'_, Operation>,
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
    pub fn state<Operation>(
        &self,
        graph: &TaskGraph<'_, Operation>,
        task_id: TaskId,
    ) -> Option<TaskRuntimeState> {
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
    pub fn start<Operation>(
        &mut self,
        graph: &TaskGraph<'_, Operation>,
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
        if current.attempts >= node.maximum_attempts.get() {
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
    pub fn succeed_attempt<Operation>(
        &mut self,
        graph: &TaskGraph<'_, Operation>,
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
    pub fn fail_attempt<Operation>(
        &mut self,
        graph: &TaskGraph<'_, Operation>,
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
        state.status = if state.attempts >= node.maximum_attempts.get() {
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
    pub fn cancel<Operation>(
        &mut self,
        graph: &TaskGraph<'_, Operation>,
        task_id: TaskId,
    ) -> Result<(), TaskGraphError> {
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
    pub fn propagate_blocked<Operation>(
        &mut self,
        graph: &TaskGraph<'_, Operation>,
    ) -> Result<usize, TaskGraphError> {
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
    pub fn ready_tasks<Operation>(
        &self,
        graph: &TaskGraph<'_, Operation>,
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
                && state.attempts < node.maximum_attempts.get()
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

    fn validate_attempt<Operation>(
        &self,
        graph: &TaskGraph<'_, Operation>,
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

fn prerequisites_succeeded<Operation>(
    graph: &TaskGraph<'_, Operation>,
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

fn has_unsuccessful_prerequisite<Operation>(
    graph: &TaskGraph<'_, Operation>,
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
