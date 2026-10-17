//! One cumulative elapsed-time account, advanced at the runtime's serialized boundaries.
#![deny(clippy::arithmetic_side_effects)]

use super::{TaskRuntime, TaskState, WorkflowRecord};
use crate::RuntimeError;
use milkdrift_capability::{AdmissionBound, InvocationAdmissionEnvelope};
use milkdrift_persistence::TimestampMillis;

pub(super) const SCHEMA_VERSION: u32 = 3;
pub(super) const EXHAUSTED: &str = "workflow cumulative active execution allowance exhausted";

/// Earlier documents do not retain enforceable reservations or exact entry-time watermarks.
/// Refuse them before a transition rather than inventing missing execution evidence.
pub(super) fn read_version(workflow: &WorkflowRecord) -> Result<(), RuntimeError> {
    if workflow.schema_version != SCHEMA_VERSION {
        return Err(history(
            "unsupported task workflow document schema; retained timing evidence cannot be reinterpreted",
        ));
    }
    for task in workflow.tasks.values() {
        if let Some(start) = &task.frozen {
            if (start.entered || task.state == TaskState::Uncertain)
                && start.execution_duration.is_none()
            {
                return Err(history("open execution has no cumulative-time watermark"));
            }
            if let Some(duration) = &start.execution_duration
                && (task
                    .started_at
                    .is_none_or(|started| duration.started_at < started)
                    || duration.accounted_at < duration.started_at
                    || duration.accounted_at > workflow.updated_at)
            {
                return Err(history(
                    "execution timing contradicts durable workflow times",
                ));
            }
        }
    }
    Ok(())
}

/// Account all open clocks together. Settled tasks retain their final watermark for inspection;
/// uncertain results and acknowledged cancellation deliberately remain open.
pub(super) fn accrue(
    workflow: &mut WorkflowRecord,
    now: TimestampMillis,
) -> Result<bool, RuntimeError> {
    let mut changed = false;
    for task in workflow
        .tasks
        .values_mut()
        .filter(|task| !task.state.is_settled())
    {
        let Some(start) = &mut task.frozen else {
            continue;
        };
        let Some(duration) = &mut start.execution_duration else {
            continue;
        };
        let elapsed = now
            .get()
            .checked_sub(duration.accounted_at.get())
            .ok_or_else(|| {
                history("clock moved backward during cumulative execution accounting")
            })?;
        workflow.active_execution_ms = workflow
            .active_execution_ms
            .checked_add(elapsed)
            .ok_or_else(|| history("cumulative execution duration overflow"))?;
        duration.accounted_at = now;
        changed |= elapsed != 0;
    }
    Ok(changed)
}

/// A profile estimate is not a bound. Only preparation's enforceable maximum can reserve
/// the shared remainder, and unresolved ownership never gives that reservation back.
pub(super) fn reserve(
    workflow: &WorkflowRecord,
    envelope: &InvocationAdmissionEnvelope,
) -> Result<u64, RuntimeError> {
    let requested = match envelope.execution_duration_ms() {
        AdmissionBound::Bounded(maximum) if *maximum > 0 => *maximum,
        AdmissionBound::NotApplicable => 0,
        AdmissionBound::Unknown | AdmissionBound::Bounded(_) => {
            return Err(RuntimeError::Scheduling(
                "external execution duration has no enforceable positive bound".into(),
            ));
        }
    };
    let mut outstanding = 0_u64;
    for task in workflow
        .tasks
        .values()
        .filter(|task| !task.state.is_settled())
    {
        let Some(duration) = task
            .frozen
            .as_ref()
            .and_then(|start| start.execution_duration.as_ref())
        else {
            continue;
        };
        let elapsed = duration
            .accounted_at
            .get()
            .checked_sub(duration.started_at.get())
            .ok_or_else(|| history("execution duration watermark precedes entry"))?;
        let reserved = if task.state == TaskState::Uncertain
            || task.stop_requested
            || task.cancellation_sent
            || (duration.maximum_ms != 0 && elapsed >= duration.maximum_ms)
        {
            duration.maximum_ms
        } else {
            // Beyond the maximum, the actual overrun is already in the ledger. It is never erased.
            duration.maximum_ms.saturating_sub(elapsed)
        };
        outstanding = outstanding
            .checked_add(reserved)
            .ok_or_else(|| history("outstanding duration reservation overflow"))?;
    }
    let admitted = workflow
        .active_execution_ms
        .checked_add(outstanding)
        .and_then(|total| total.checked_add(requested))
        .ok_or_else(|| history("cumulative duration admission overflow"))?;
    if admitted > workflow.limits.active_execution_ms || remaining(workflow) == 0 {
        return Err(RuntimeError::Scheduling(
            "workflow cumulative execution allowance cannot reserve this adapter's enforced duration alongside live work".into(),
        ));
    }
    Ok(requested)
}

pub(super) fn remaining(workflow: &WorkflowRecord) -> u64 {
    // Observed overrun remains in the ledger. No remaining admission exists at or above the limit.
    workflow
        .limits
        .active_execution_ms
        .saturating_sub(workflow.active_execution_ms)
}

pub(super) fn request_limit_stop(workflow: &mut WorkflowRecord) -> bool {
    let exhausted = remaining(workflow) == 0;
    let mut changed = false;
    if exhausted && workflow.wait_question.is_none() {
        workflow.wait_question = Some(EXHAUSTED.into());
        workflow.model_admission_wait = None;
        changed = true;
    }
    for task in workflow.tasks.values_mut() {
        let elapsed_limit = task
            .frozen
            .as_ref()
            .and_then(|start| start.execution_duration.as_ref())
            .is_some_and(|duration| {
                duration.maximum_ms != 0
                    && duration
                        .accounted_at
                        .get()
                        .checked_sub(duration.started_at.get())
                        .is_some_and(|elapsed| elapsed >= duration.maximum_ms)
            });
        if (exhausted || elapsed_limit)
            && !task.state.is_settled()
            && task.frozen.is_some()
            && !task.stop_requested
        {
            task.stop_requested = true;
            changed = true;
        }
    }
    changed
}

impl TaskRuntime {
    /// Called with the existing task transition gate held. A heartbeat or scheduler poll may
    /// discover exhaustion, but only terminal proof can release execution ownership.
    pub(super) fn observe_duration(
        &self,
        workflow: &mut WorkflowRecord,
        now: TimestampMillis,
    ) -> Result<(), RuntimeError> {
        let changed = accrue(workflow, now)?;
        if request_limit_stop(workflow) || changed {
            let expected = self.advance(workflow)?;
            self.save(workflow, Some(expected), None)?;
        }
        Ok(())
    }
}

fn history(message: &str) -> RuntimeError {
    RuntimeError::InvalidHistory(message.into())
}
