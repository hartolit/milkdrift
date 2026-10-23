//! Retain exact worker ownership when a local failure prevents a durable handoff.

use milkdrift_blueprint::{TaskId, WorkflowId};

use super::{FailedExecution, TaskRuntime, TaskState, effects::ExecutionClaim};
use crate::{ExecutionDispatch, RuntimeError};

impl TaskRuntime {
    pub(super) fn reconcile_execution_failure(
        &self,
        dispatch: &ExecutionDispatch,
        execution: &mut ExecutionClaim<'_>,
        non_entry_proven: &mut bool,
    ) -> Result<(), RuntimeError> {
        let workflow_id = WorkflowId::new(dispatch.run().as_str()).map_err(invalid)?;
        let task_id = TaskId::new(dispatch.node().as_str()).map_err(invalid)?;
        let guard = self
            .gate
            .lock()
            .map_err(|_| invalid("task recovery lock unavailable"))?;
        let Some(workflow) = self.workflow(&workflow_id)? else {
            return Ok(());
        };
        let Some(task) = workflow.tasks.get(&task_id) else {
            return Ok(());
        };
        let Some(frozen) = &task.frozen else {
            return Ok(());
        };
        // A lost final-entry commit reply may have advanced only the authorization. All other
        // accepted dispatch facts must still belong to this worker before it may repair anything.
        let exact =
            dispatch.with_entry_authorization(frozen.dispatch.entry_authorization().clone())?;
        if exact != frozen.dispatch {
            return Ok(());
        }
        if task.state.is_settled() {
            drop(guard);
            return self.executor.settle(&frozen.dispatch).map_err(Into::into);
        }
        if task.state == TaskState::Queued {
            // A deferral commit can succeed while its reply is lost. Release the former worker
            // before the scheduler can select that already queued start.
            execution.release()?;
            return Ok(());
        }
        if task.state == TaskState::Uncertain {
            return Ok(());
        }
        if frozen.entered || frozen.execution_duration.is_some() || !*non_entry_proven {
            *non_entry_proven = false;
            drop(guard);
            return self.mark_uncertain(
                &workflow_id,
                &task_id,
                "local execution recovery cannot establish safe non-entry",
            );
        }
        drop(guard);
        self.defer_before_entry(
            &workflow_id,
            &task_id,
            dispatch,
            "Local execution is awaiting retry",
            execution,
            non_entry_proven,
        )
    }

    pub(super) fn retain_failed_execution(
        &self,
        dispatch: ExecutionDispatch,
        non_entry_proven: bool,
        execution: &mut ExecutionClaim<'_>,
    ) -> Result<(), RuntimeError> {
        let mut pending = self
            .failed_executions
            .lock()
            .map_err(|_| invalid("task recovery ownership unavailable"))?;
        // Every element retains one invocation in `executing`. Its 65,536-owner admission bound
        // therefore reserves this slot before any preparation starts, without a second quota.
        pending.push_back(FailedExecution {
            dispatch,
            non_entry_proven,
        });
        execution.active = false;
        Ok(())
    }

    pub(super) fn reconcile_failed_executions(&self, maximum: u16) -> Result<(), RuntimeError> {
        for _ in 0..maximum {
            let pending = self
                .failed_executions
                .lock()
                .map_err(|_| invalid("task recovery ownership unavailable"))?
                .pop_front();
            let Some(mut pending) = pending else {
                break;
            };
            let mut execution = ExecutionClaim {
                runtime: self,
                invocation: pending.dispatch.request().invocation().clone(),
                active: true,
            };
            if let Err(error) = self.reconcile_execution_failure(
                &pending.dispatch,
                &mut execution,
                &mut pending.non_entry_proven,
            ) {
                self.retain_failed_execution(
                    pending.dispatch,
                    pending.non_entry_proven,
                    &mut execution,
                )?;
                return Err(error);
            }
        }
        Ok(())
    }
}

fn invalid(message: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::InvalidTransition(message.to_string())
}
