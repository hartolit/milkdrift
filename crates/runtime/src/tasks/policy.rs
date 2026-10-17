//! Prospective policy changes retain the work already charged to a workflow.
#![deny(clippy::arithmetic_side_effects)]

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::{TaskId, WorkflowLimits};

use super::{TaskState, WorkflowRecord};
use crate::RuntimeError;

/// Command receipts retain the old request and its accepted limits; the current snapshot owns
/// only the next ceilings. Changing them never changes authority or an execution's frozen input.
pub(super) fn change_limits(
    workflow: &mut WorkflowRecord,
    limits: WorkflowLimits,
) -> Result<(), RuntimeError> {
    limits.validate().map_err(invalid)?;
    if limits.tasks < workflow.admitted_tasks
        || limits.model_batches < workflow.admitted_model_batches
        || limits.active_execution_ms < workflow.active_execution_ms
    {
        return Err(invalid(
            "new workflow limits cannot be below already admitted tasks, model batches or elapsed execution",
        ));
    }
    workflow.limits = limits;
    Ok(())
}

/// An explicit continuation releases a question, not the cumulative account that stopped work.
pub(super) fn resume(workflow: &mut WorkflowRecord) -> Result<(), RuntimeError> {
    if super::duration::remaining(workflow) == 0 {
        return Err(invalid(super::duration::EXHAUSTED));
    }
    if workflow.rejected_responses >= workflow.limits.rejected_responses {
        return Err(invalid(
            "model rejected/no-progress response allowance exhausted; raise its workflow limit before continuing",
        ));
    }
    if let Some(reason) = lineage_stop(workflow)? {
        return Err(invalid(reason));
    }
    workflow.wait_question = None;
    workflow.model_admission_wait = None;
    Ok(())
}

/// The failed attempts themselves own lineage spending, including across restarts and removal.
/// A paused workflow can still accept prospective repairs and an explicit limit increase.
pub(super) fn request_limit_stop(workflow: &mut WorkflowRecord) -> Result<bool, RuntimeError> {
    let reason = if workflow.rejected_responses >= workflow.limits.rejected_responses {
        Some("model rejected/no-progress response allowance exhausted; raise its workflow limit before continuing".into())
    } else {
        lineage_stop(workflow)?
    };
    let Some(reason) = reason else {
        return Ok(false);
    };
    let mut changed = false;
    if workflow.wait_question.is_none() {
        workflow.wait_question = Some(reason);
        workflow.model_admission_wait = None;
        changed = true;
    }
    for task in workflow.tasks.values_mut() {
        if task.frozen.is_some() && !task.state.is_settled() && !task.stop_requested {
            task.stop_requested = true;
            changed = true;
        }
    }
    Ok(changed)
}

fn lineage_stop(workflow: &WorkflowRecord) -> Result<Option<String>, RuntimeError> {
    let mut lineages = BTreeMap::<TaskId, (u32, bool)>::new();
    for task in workflow.tasks.values() {
        let root = lineage_root(workflow, &task.id)?;
        let (failures, outstanding) = lineages.entry(root).or_default();
        if task.state == TaskState::Failed {
            *failures = failures
                .checked_add(1)
                .ok_or_else(|| history("corrective failure accounting overflow"))?;
        }
        let adopted = task
            .adopted_correction
            .as_ref()
            .and_then(|id| workflow.tasks.get(id))
            .is_some_and(|correction| correction.state == TaskState::Completed);
        *outstanding |= !task.removed
            && (!task.state.is_settled()
                || task.definition.required && task.state != TaskState::Completed && !adopted);
    }
    Ok(lineages.into_iter().find_map(|(root, (failures, outstanding))| {
        (outstanding && failures >= workflow.limits.corrective_failures).then(|| {
            format!(
                "corrective lineage {root} has {failures} failed tasks; raise its workflow failure limit before continuing"
            )
        })
    }))
}

fn lineage_root(workflow: &WorkflowRecord, task: &TaskId) -> Result<TaskId, RuntimeError> {
    let mut current = task;
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(current) {
            return Err(history("retained corrective lineage contains a cycle"));
        }
        let task = workflow
            .tasks
            .get(current)
            .ok_or_else(|| history("retained corrective lineage names an absent task"))?;
        match &task.definition.corrective_of {
            Some(parent) => current = parent,
            None => return Ok(current.clone()),
        }
    }
}

fn invalid(message: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::InvalidCommand(message.to_string())
}

fn history(message: &str) -> RuntimeError {
    RuntimeError::InvalidHistory(message.into())
}
