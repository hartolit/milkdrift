//! Remaining record growth is reserved before accepting work, independently of artifact bytes.

use milkdrift_blueprint::TaskInputBinding;
use milkdrift_capability::InvocationEvent;

use super::{TaskRecord, WorkflowRecord};
use crate::RuntimeError;

pub(super) const MAX_DISPATCH_BYTES: usize = 65_536;
const REPORT_BYTES: u64 = 262_144;
const TERMINAL_BYTES: u64 = 262_144;
const MAX_TASK_REPORTS: usize = 256;

pub(super) fn remaining(workflow: &WorkflowRecord) -> Result<u64, RuntimeError> {
    workflow.tasks.values().try_fold(0_u64, |sum, task| {
        let additional = if task.state.is_settled() {
            0
        } else {
            task_remaining(task)?
        };
        sum.checked_add(additional)
            .ok_or_else(|| invalid("remaining record reservation overflow"))
    })
}

fn task_remaining(task: &TaskRecord) -> Result<u64, RuntimeError> {
    let Some(frozen) = &task.frozen else {
        let definition =
            u64::try_from(serde_json::to_vec(&task.definition)?.len()).map_err(invalid)?;
        let inputs =
            task.definition
                .inputs
                .values()
                .try_fold(0_u64, |sum, input| {
                    let bytes = match input {
                        TaskInputBinding::Inline { value } => {
                            u64::try_from(serde_json::to_vec(value)?.len()).map_err(invalid)?
                        }
                        TaskInputBinding::TaskOutput { .. } => 2048,
                        TaskInputBinding::Supplied { .. }
                        | TaskInputBinding::SettledOutcome { .. } => 1_048_576,
                    };
                    sum.checked_add(bytes)
                        .ok_or_else(|| invalid("frozen input reservation overflow"))
                })?;
        // Dispatch has a checked byte ceiling; names, timestamps and stop facts have bounded slack.
        return definition
            .checked_add(inputs)
            .and_then(|n| n.checked_add(131_072 + REPORT_BYTES + TERMINAL_BYTES))
            .ok_or_else(|| invalid("frozen start reservation overflow"));
    };
    let used = u64::try_from(serde_json::to_vec(&frozen.reports)?.len()).map_err(invalid)?;
    Ok(REPORT_BYTES
        .saturating_sub(used)
        .saturating_add(TERMINAL_BYTES))
}

pub(super) fn validate_report(
    prior: &[InvocationEvent],
    report: &InvocationEvent,
) -> Result<(), RuntimeError> {
    if prior.len() >= MAX_TASK_REPORTS
        || report.kind().terminal().is_none() && prior.len() >= MAX_TASK_REPORTS.saturating_sub(1)
    {
        return Err(invalid(
            "task observation capacity exhausted; terminal slot remains reserved",
        ));
    }
    let bytes = serde_json::to_vec(report)?.len();
    if report.kind().terminal().is_some() {
        if bytes > 131_072 {
            return Err(invalid("terminal report exceeds its reserved allowance"));
        }
    } else {
        let used = serde_json::to_vec(prior)?.len();
        if used
            .checked_add(bytes)
            .and_then(|n| n.checked_add(1))
            .is_none_or(|n| u64::try_from(n).map_or(true, |n| n > REPORT_BYTES))
        {
            return Err(invalid(
                "task progress capacity exhausted; terminal allowance remains reserved",
            ));
        }
        if let Some((_, reference)) = report.kind().output() {
            let outputs = prior
                .iter()
                .filter_map(|event| event.kind().output())
                .collect::<Vec<_>>();
            let used = outputs
                .iter()
                .try_fold(0_u64, |sum, (_, value)| {
                    sum.checked_add(value.size_bytes().unwrap_or(super::TASK_OUTPUT_BYTES))
                })
                .ok_or_else(|| invalid("output byte accounting overflow"))?;
            if outputs.len() >= 64
                || reference
                    .size_bytes()
                    .and_then(|size| used.checked_add(size))
                    .is_none_or(|size| size > super::TASK_OUTPUT_BYTES)
            {
                return Err(invalid("task output count or byte allowance exceeded"));
            }
        }
    }
    Ok(())
}

fn invalid(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::InvalidTransition(error.to_string())
}
