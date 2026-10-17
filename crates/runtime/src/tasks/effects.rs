use milkdrift_authority::{BoundaryTimeMillis, DecisionId};
use milkdrift_blueprint::{TaskId, WorkflowId};
use milkdrift_capability::{CancellationRequest, InvocationEvent, TerminalStatus};

use super::{TaskResult, TaskRuntime, TaskState};
use crate::{
    ExecutionDispatch, ExecutionReporter, ExecutorError, ObservationDisposition, RuntimeError,
};

impl TaskRuntime {
    /// Prepares local resources, rechecks the grant, commits entry intent, then enters once.
    ///
    /// # Errors
    /// Returns mismatched ownership, authority, storage, or reporting failures. A failure after
    /// durable entry leaves uncertainty and never authorizes automatic re-entry.
    pub fn execute(&self, dispatch: ExecutionDispatch) -> Result<(), RuntimeError> {
        let mut execution = self.claim_execution(dispatch.request().invocation())?;
        let workflow_id = WorkflowId::new(dispatch.run().as_str()).map_err(invalid)?;
        let task_id = TaskId::new(dispatch.node().as_str()).map_err(invalid)?;
        {
            let _guard = self
                .gate
                .lock()
                .map_err(|_| invalid("task execution lock unavailable"))?;
            let workflow = self.require_workflow(&workflow_id)?;
            let task = workflow
                .tasks
                .get(&task_id)
                .ok_or_else(|| invalid("dispatch task is absent"))?;
            let frozen = task
                .frozen
                .as_ref()
                .ok_or_else(|| invalid("dispatch has no frozen start"))?;
            if frozen.dispatch != dispatch || frozen.entered || task.state != TaskState::Running {
                return Err(invalid("dispatch no longer owns the exact unentered task"));
            }
            if task.stop_requested {
                drop(_guard);
                return self.finish_without_entry(
                    &workflow_id,
                    &task_id,
                    TaskState::Stopped,
                    "stopped before external entry",
                );
            }
        }
        let prepared = match self.executor.prepare_exact_entry(&dispatch) {
            Ok(prepared) => prepared,
            Err(ExecutorError::CapacityUnavailable(reason)) => {
                return self.defer_before_entry(
                    &workflow_id,
                    &task_id,
                    &dispatch,
                    &reason,
                    &mut execution,
                );
            }
            Err(
                error @ (ExecutorError::BoundaryBeforeEntry(_)
                | ExecutorError::UnavailableGeneration { .. }
                | ExecutorError::Unavailable(_)
                | ExecutorError::AdmissionClosed
                | ExecutorError::InvalidDispatch(_)
                | ExecutorError::Contract(_)
                | ExecutorError::AdapterPanicked { after_entry: false }),
            ) => {
                return self.finish_without_entry(
                    &workflow_id,
                    &task_id,
                    TaskState::Failed,
                    &format!("local preparation refused before entry: {error}"),
                );
            }
            Err(error) => {
                return self.mark_uncertain(
                    &workflow_id,
                    &task_id,
                    &format!("preparation could not establish exclusive non-entry: {error}"),
                );
            }
        };
        let final_dispatch = {
            let _guard = self
                .gate
                .lock()
                .map_err(|_| invalid("task entry lock unavailable"))?;
            let mut workflow = self.require_workflow(&workflow_id)?;
            let now = self.clock.now()?;
            self.observe_duration(&mut workflow, now)?;
            if workflow
                .tasks
                .get(&task_id)
                .is_some_and(|task| task.state.is_settled())
            {
                // Stop may settle while local preparation is acquiring host resources. Repeat
                // release after that preparation returns, using the retained terminal proof.
                drop(_guard);
                drop(prepared);
                self.executor.settle(&dispatch)?;
                return Ok(());
            }
            if let Some(reason) = &workflow.wait_question {
                let reason = format!("workflow paused before external entry: {reason}");
                drop(_guard);
                return self.finish_without_entry(
                    &workflow_id,
                    &task_id,
                    TaskState::Failed,
                    &reason,
                );
            }
            let reservation = super::duration::reserve(&workflow, prepared.admission_envelope())
                .and_then(|maximum| {
                    super::account::reserve(
                        &workflow,
                        &dispatch,
                        prepared.admission_envelope(),
                        maximum,
                    )
                    .map(|(allowance, currency)| (allowance, currency, maximum))
                });
            let (allowance, spending_currency, maximum_ms) = match reservation {
                Ok(value) => value,
                Err(error) => {
                    drop(_guard);
                    return self.finish_without_entry_policy(
                        &workflow_id,
                        &task_id,
                        TaskState::Failed,
                        &error.to_string(),
                        true,
                    );
                }
            };
            let task = workflow
                .tasks
                .get_mut(&task_id)
                .ok_or_else(|| invalid("dispatch task is absent"))?;
            let frozen = task
                .frozen
                .as_mut()
                .ok_or_else(|| invalid("dispatch has no frozen start"))?;
            if frozen.dispatch != dispatch || frozen.entered || task.state != TaskState::Running {
                return Err(invalid("task changed during preparation"));
            }
            if task.stop_requested {
                drop(_guard);
                return self.finish_without_entry(
                    &workflow_id,
                    &task_id,
                    TaskState::Stopped,
                    "stopped during local preparation",
                );
            }
            let mut request = dispatch.entry_authorization().request().clone();
            request.decision = DecisionId::new(format!(
                "entry_{}",
                blake3::hash(dispatch.request().invocation().as_str().as_bytes())
            ))
            .map_err(invalid)?;
            request.evaluated_at = BoundaryTimeMillis::new(now.get());
            let decision = self.authority.evaluate(&request)?;
            if !decision.is_allowed() {
                drop(_guard);
                return self.finish_without_entry(
                    &workflow_id,
                    &task_id,
                    TaskState::Failed,
                    "authority was revoked before external entry",
                );
            }
            if now >= dispatch.lease_expires_at() {
                drop(_guard);
                return self.finish_without_entry(
                    &workflow_id,
                    &task_id,
                    TaskState::Failed,
                    "task execution allowance expired before entry",
                );
            }
            let exact = dispatch.with_entry_authorization(decision)?;
            frozen.dispatch = exact.clone();
            frozen.entered = true;
            frozen.execution_duration = Some(super::TaskExecutionDuration {
                started_at: now,
                accounted_at: now,
                maximum_ms,
            });
            workflow.committed_allowance = allowance;
            workflow.spending_currency = spending_currency;
            let expected = self.advance(&mut workflow)?;
            self.save(&workflow, Some(expected), None)?;
            exact
        };
        let reporter = TaskReporter {
            runtime: self,
            workflow: workflow_id.clone(),
            task: task_id.clone(),
            dispatch: final_dispatch.clone(),
        };
        let outcome = prepared.enter(&final_dispatch, &reporter);
        let workflow = self.require_workflow(&workflow_id)?;
        let state = workflow
            .tasks
            .get(&task_id)
            .ok_or_else(|| invalid("entered task disappeared"))?
            .state;
        if state.is_settled() {
            self.executor.settle(&final_dispatch)?;
            return Ok(());
        }
        if state == TaskState::Uncertain {
            return Ok(());
        }
        let reason = match outcome {
            Ok(()) => "adapter returned without durable terminal evidence".to_owned(),
            Err(error) => format!("external outcome remains uncertain: {error}"),
        };
        self.mark_uncertain(&workflow_id, &task_id, &reason)
    }

    fn claim_execution(
        &self,
        invocation: &milkdrift_capability::InvocationId,
    ) -> Result<ExecutionClaim<'_>, RuntimeError> {
        let mut executing = self
            .executing
            .lock()
            .map_err(|_| invalid("task worker ownership unavailable"))?;
        if executing.len() >= 65_536 || !executing.insert(invocation.clone()) {
            return Err(invalid(
                "exact task invocation is already owned by an active worker",
            ));
        }
        Ok(ExecutionClaim {
            runtime: self,
            invocation: invocation.clone(),
            active: true,
        })
    }

    fn defer_before_entry(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
        dispatch: &ExecutionDispatch,
        reason: &str,
        execution: &mut ExecutionClaim<'_>,
    ) -> Result<(), RuntimeError> {
        let guard = self
            .gate
            .lock()
            .map_err(|_| invalid("task capacity lock unavailable"))?;
        let mut workflow = self.require_workflow(workflow_id)?;
        super::duration::accrue(&mut workflow, self.clock.now()?)?;
        let task = workflow
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| invalid("capacity-deferred task is absent"))?;
        if task.state.is_settled() {
            drop(guard);
            self.executor.settle(dispatch)?;
            return Ok(());
        }
        let frozen = task
            .frozen
            .as_ref()
            .ok_or_else(|| invalid("capacity refusal lost its accepted start"))?;
        if frozen.dispatch != *dispatch || frozen.entered || task.state != TaskState::Running {
            return Err(invalid(
                "capacity refusal does not own the exact unentered task",
            ));
        }
        if task.stop_requested {
            drop(guard);
            return self.finish_without_entry(
                workflow_id,
                task_id,
                TaskState::Stopped,
                "stopped while awaiting capability capacity",
            );
        }
        task.state = TaskState::Queued;
        task.reason = Some(
            milkdrift_contracts::truncate_utf8(
                &format!("waiting for capability capacity: {reason}"),
                4096,
            )
            .to_owned(),
        );
        let expected = self.advance(&mut workflow)?;
        self.save(&workflow, Some(expected), None)?;
        // Release the old worker while scheduling is still excluded. Otherwise a new exact
        // dispatch could be queued before this guard drops and lose its only worker to conflict.
        execution.release()?;
        Ok(())
    }

    fn finish_without_entry(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
        state: TaskState,
        reason: &str,
    ) -> Result<(), RuntimeError> {
        self.finish_without_entry_policy(workflow_id, task_id, state, reason, false)
    }

    fn finish_without_entry_policy(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
        state: TaskState,
        reason: &str,
        pause: bool,
    ) -> Result<(), RuntimeError> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| invalid("task settlement lock unavailable"))?;
        let mut workflow = self.require_workflow(workflow_id)?;
        if pause {
            workflow.wait_question =
                Some(milkdrift_contracts::truncate_utf8(reason, 4096).to_owned());
            workflow.model_admission_wait = None;
        }
        let task = workflow
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| invalid("task is absent"))?;
        if task.state.is_settled() {
            let dispatch = task.frozen.as_ref().map(|start| start.dispatch.clone());
            drop(_guard);
            if let Some(dispatch) = dispatch {
                self.executor.settle(&dispatch)?;
            }
            return Ok(());
        }
        if task
            .frozen
            .as_ref()
            .is_some_and(|start| start.entered || start.execution_duration.is_some())
        {
            return Err(invalid(
                "entered or unresolved task cannot be settled using non-entry evidence",
            ));
        }
        task.state = state;
        task.reason = Some(milkdrift_contracts::truncate_utf8(reason, 4096).to_owned());
        task.ended_at = Some(self.clock.now()?);
        let dispatch = task.frozen.as_ref().map(|start| start.dispatch.clone());
        let expected = self.advance(&mut workflow)?;
        self.save(&workflow, Some(expected), None)?;
        if let Some(dispatch) = dispatch {
            self.executor.settle(&dispatch)?;
        }
        Ok(())
    }

    fn mark_uncertain(
        &self,
        workflow_id: &WorkflowId,
        task_id: &TaskId,
        reason: &str,
    ) -> Result<(), RuntimeError> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| invalid("task uncertainty lock unavailable"))?;
        let mut workflow = self.require_workflow(workflow_id)?;
        let task = workflow
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| invalid("task is absent"))?;
        if task.state.is_settled() {
            return Ok(());
        }
        task.state = TaskState::Uncertain;
        let frozen = task
            .frozen
            .as_mut()
            .ok_or_else(|| invalid("uncertain task lost its accepted start"))?;
        if frozen.execution_duration.is_none() {
            let started_at = task
                .started_at
                .ok_or_else(|| invalid("uncertain task lost its accepted start time"))?;
            frozen.execution_duration = Some(super::TaskExecutionDuration {
                started_at,
                accounted_at: started_at,
                maximum_ms: workflow.limits.active_execution_ms,
            });
        }
        task.reason = Some(milkdrift_contracts::truncate_utf8(reason, 4096).to_owned());
        super::duration::accrue(&mut workflow, self.clock.now()?)?;
        super::duration::request_limit_stop(&mut workflow);
        let expected = self.advance(&mut workflow)?;
        self.save(&workflow, Some(expected), None)?;
        Ok(())
    }

    /// Missing workers after restart never establish absence of an entered external effect.
    /// Unentered frozen starts return unchanged for bounded dispatch by the worker owner.
    ///
    /// # Errors
    /// Returns storage/integrity failures or a recovery page exceeding the bounded startup queue.
    pub fn recover(&self) -> Result<Vec<ExecutionDispatch>, RuntimeError> {
        let mut cursor = None;
        let mut ready = Vec::new();
        loop {
            let page = self.store.workflows(cursor.as_deref(), 200)?;
            for snapshot in page.items {
                let workflow = {
                    let _guard = self
                        .gate
                        .lock()
                        .map_err(|_| invalid("task recovery lock unavailable"))?;
                    let mut workflow = self.require_workflow(&snapshot.workflow)?;
                    self.observe_duration(&mut workflow, self.clock.now()?)?;
                    workflow
                };
                for (id, task) in &workflow.tasks {
                    if task.state.is_settled() {
                        if let Some(frozen) = &task.frozen {
                            self.executor.settle(&frozen.dispatch)?;
                        }
                        continue;
                    }
                    if task.state != TaskState::Running {
                        continue;
                    }
                    let frozen = task
                        .frozen
                        .as_ref()
                        .ok_or_else(|| invalid("running task lost its frozen start"))?;
                    if frozen.entered {
                        self.mark_uncertain(&workflow.id, id, "daemon restarted after durable external entry; no terminal evidence is available")?;
                    } else {
                        if ready.len() >= 200 {
                            let _guard = self
                                .gate
                                .lock()
                                .map_err(|_| invalid("task recovery lock unavailable"))?;
                            let mut current = self.require_workflow(&workflow.id)?;
                            let retained = current
                                .tasks
                                .get_mut(id)
                                .ok_or_else(|| invalid("recoverable task disappeared"))?;
                            retained.state = TaskState::Queued;
                            retained.reason =
                                Some("accepted start retained for bounded worker recovery".into());
                            let expected = self.advance(&mut current)?;
                            self.save(&current, Some(expected), None)?;
                            continue;
                        }
                        if !task.stop_requested && workflow.wait_question.is_none() {
                            ready.push(frozen.dispatch.clone());
                        }
                    }
                }
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        Ok(ready)
    }

    /// Dispatches exact cancellation requests independently of occupied execution workers.
    ///
    /// # Errors
    /// Returns durable read/write failures. Adapter refusal records unresolved stop evidence.
    pub fn execute_cancellation(
        &self,
        dispatch: &crate::CancellationDispatch,
    ) -> Result<(), RuntimeError> {
        let workflow_id = WorkflowId::new(dispatch.run().as_str()).map_err(invalid)?;
        let workflow = self.require_workflow(&workflow_id)?;
        for (id, task) in &workflow.tasks {
            if !task.stop_requested || task.cancellation_sent || task.state.is_settled() {
                continue;
            }
            let Some(frozen) = &task.frozen else {
                continue;
            };
            if frozen.dispatch.attempt() != dispatch.attempt()
                || frozen.dispatch.request().invocation() != dispatch.request().invocation()
            {
                continue;
            }
            if !frozen.entered && frozen.execution_duration.is_none() {
                self.finish_without_entry(
                    &workflow.id,
                    id,
                    TaskState::Stopped,
                    "stop won before external entry",
                )?;
                continue;
            }
            let request = CancellationRequest::new(
                frozen.dispatch.request().invocation().clone(),
                1,
                "authorized task stop request",
            )
            .map_err(invalid)?;
            match self.executor.cancel(&request) {
                Ok(acknowledgement)
                    if acknowledgement.invocation() == request.invocation()
                        && acknowledgement.request_sequence() == request.request_sequence() =>
                {
                    let _guard = self
                        .gate
                        .lock()
                        .map_err(|_| invalid("task cancellation lock unavailable"))?;
                    let mut current = self.require_workflow(&workflow.id)?;
                    let task = current
                        .tasks
                        .get_mut(id)
                        .ok_or_else(|| invalid("cancelled task disappeared"))?;
                    task.cancellation_sent = true;
                    if !task.state.is_settled() {
                        task.reason =
                            Some("stop requested; awaiting terminal stop evidence".into());
                    }
                    let expected = self.advance(&mut current)?;
                    self.save(&current, Some(expected), None)?;
                }
                Ok(_) => self.mark_uncertain(
                    &workflow.id,
                    id,
                    "cancellation acknowledgement named a different invocation or request",
                )?,
                Err(error) => self.mark_uncertain(
                    &workflow.id,
                    id,
                    &format!("stop request outcome is unresolved: {error}"),
                )?,
            }
        }
        Ok(())
    }

    /// Claims exact stop coordinates for the separate bounded cancellation worker.
    ///
    /// # Errors
    /// Returns malformed cancellation coordinates or storage failures.
    pub fn cancellation_actions(
        &self,
        maximum: u16,
    ) -> Result<Vec<crate::EffectAction>, RuntimeError> {
        let mut actions = Vec::new();
        let mut cursor = self
            .cancellation_cursor
            .lock()
            .map_err(|_| invalid("cancellation discovery lock unavailable"))?;
        let page = self.store.workflows(cursor.as_deref(), maximum.min(200))?;
        *cursor = page.next.clone();
        for snapshot in page.items {
            let workflow = self.require_workflow(&snapshot.workflow)?;
            for task in workflow.tasks.values() {
                if actions.len() >= usize::from(maximum) {
                    return Ok(actions);
                }
                if !task.stop_requested || task.cancellation_sent || task.state.is_settled() {
                    continue;
                }
                let Some(frozen) = &task.frozen else {
                    continue;
                };
                let request = CancellationRequest::new(
                    frozen.dispatch.request().invocation().clone(),
                    1,
                    "authorized task stop request",
                )
                .map_err(invalid)?;
                actions.push(crate::EffectAction::Cancel(
                    crate::CancellationDispatch::new(
                        frozen.dispatch.run().clone(),
                        frozen.dispatch.attempt().clone(),
                        request,
                    ),
                ));
            }
        }
        Ok(actions)
    }
}

struct TaskReporter<'a> {
    runtime: &'a TaskRuntime,
    workflow: WorkflowId,
    task: TaskId,
    dispatch: ExecutionDispatch,
}

struct ExecutionClaim<'a> {
    runtime: &'a TaskRuntime,
    invocation: milkdrift_capability::InvocationId,
    active: bool,
}

impl ExecutionClaim<'_> {
    fn release(&mut self) -> Result<(), RuntimeError> {
        let mut executing = self
            .runtime
            .executing
            .lock()
            .map_err(|_| invalid("task worker ownership unavailable"))?;
        executing.remove(&self.invocation);
        self.active = false;
        Ok(())
    }
}

impl Drop for ExecutionClaim<'_> {
    fn drop(&mut self) {
        if self.active
            && let Ok(mut executing) = self.runtime.executing.lock()
        {
            executing.remove(&self.invocation);
        }
    }
}

impl ExecutionReporter for TaskReporter<'_> {
    fn invocation(&self, report: InvocationEvent) -> Result<ObservationDisposition, ExecutorError> {
        self.record(report)
            .map_err(|error| ExecutorError::BoundaryAfterEntry(error.to_string()))
    }
    fn heartbeat(&self) -> Result<ObservationDisposition, ExecutorError> {
        self.check_continuation()
            .map_err(|error| ExecutorError::BoundaryAfterEntry(error.to_string()))
    }
}

impl TaskReporter<'_> {
    fn check_continuation(&self) -> Result<ObservationDisposition, RuntimeError> {
        let _guard = self
            .runtime
            .gate
            .lock()
            .map_err(|_| invalid("task continuation lock unavailable"))?;
        let mut workflow = self.runtime.require_workflow(&self.workflow)?;
        let now = self.runtime.clock.now()?;
        self.runtime.observe_duration(&mut workflow, now)?;
        let task = workflow
            .tasks
            .get(&self.task)
            .ok_or_else(|| invalid("heartbeat task disappeared"))?;
        if task
            .frozen
            .as_ref()
            .is_none_or(|start| !start.entered || start.dispatch != self.dispatch)
            || task.state.is_settled()
        {
            return Err(invalid(
                "heartbeat differs from an unsettled entered execution",
            ));
        }
        if task.stop_requested
            || super::duration::remaining(&workflow) == 0
            || now >= self.dispatch.lease_expires_at()
        {
            return Err(invalid(
                "task continuation refused by its stop or cumulative execution allowance",
            ));
        }
        Ok(ObservationDisposition::Applied)
    }

    fn record(&self, report: InvocationEvent) -> Result<ObservationDisposition, RuntimeError> {
        let _guard = self
            .runtime
            .gate
            .lock()
            .map_err(|_| invalid("task report lock unavailable"))?;
        if report.invocation() != self.dispatch.request().invocation() {
            return Err(invalid("report names another invocation"));
        }
        let mut workflow = self.runtime.require_workflow(&self.workflow)?;
        let now = self.runtime.clock.now()?;
        // Terminal proof must remain writable even after exhaustion. Charge every open
        // execution before closing this task's clock, without rejecting its final evidence.
        super::duration::accrue(&mut workflow, now)?;
        let task = workflow
            .tasks
            .get_mut(&self.task)
            .ok_or_else(|| invalid("report task disappeared"))?;
        let frozen = task
            .frozen
            .as_mut()
            .ok_or_else(|| invalid("report task lost its start"))?;
        if !frozen.entered || frozen.dispatch != self.dispatch {
            return Err(invalid("report differs from exact entered dispatch"));
        }
        if let Some(existing) = frozen
            .reports
            .iter()
            .find(|existing| existing.sequence() == report.sequence())
        {
            return if existing == &report {
                Ok(ObservationDisposition::Replayed)
            } else {
                Err(invalid("report sequence reused with different content"))
            };
        }
        if task.state.is_settled()
            || task.result.is_some()
            || frozen.reports.len() >= crate::MAX_REPORTS_PER_DISPATCH
            || report.sequence()
                != u64::try_from(frozen.reports.len())
                    .map_err(invalid)?
                    .checked_add(1)
                    .ok_or_else(|| invalid("report sequence exhausted"))?
        {
            return Err(invalid("report sequence or task state is invalid"));
        }
        super::capacity::validate_report(&frozen.reports, &report)?;
        if let Some((message, _, _)) = report.kind().progress() {
            task.reason = Some(message.to_owned());
        }
        if let Some((name, _)) = report.kind().output()
            && frozen.reports.iter().any(|prior| {
                prior
                    .kind()
                    .output()
                    .is_some_and(|(prior_name, _)| prior_name == name)
            })
        {
            return Err(invalid(
                "an immutable named output cannot be published twice",
            ));
        }
        frozen.reports.push(report.clone());
        if let Some(("model_response", reference)) = report.kind().output()
            && frozen.definition.actions.is_some()
        {
            workflow.admissions.insert(
                self.task.clone(),
                super::ModelAdmission {
                    response: Some(reference.clone()),
                    expected_generation: frozen.accepted_generation,
                    status: super::ModelAdmissionStatus::Pending,
                    reason: None,
                    repair_allowed: false,
                    receipt_key: None,
                    aliases: std::collections::BTreeMap::new(),
                },
            );
        }
        if let Some(terminal) = report.kind().terminal() {
            task.state = match terminal.status() {
                TerminalStatus::Success => TaskState::Completed,
                TerminalStatus::Failure | TerminalStatus::Rejected => TaskState::Failed,
                TerminalStatus::Cancelled => TaskState::Stopped,
                TerminalStatus::Uncertain => TaskState::Uncertain,
            };
            task.ended_at = task.state.is_settled().then_some(now);
            task.reason = terminal.failure().map(|failure| format!("{failure:?}"));
            let outputs = frozen
                .reports
                .iter()
                .filter_map(|report| {
                    report
                        .kind()
                        .output()
                        .map(|(name, reference)| (name.to_owned(), reference.clone()))
                })
                .collect::<std::collections::BTreeMap<_, _>>();
            if task.state == TaskState::Completed
                && let Some(reason) = super::output_contract_failure(&frozen.definition, &outputs)
            {
                task.state = TaskState::Failed;
                task.reason = Some(reason);
            }
            task.result = Some(TaskResult {
                terminal: terminal.clone(),
                outputs,
                recorded_at: now,
            });
            if task.state == TaskState::Completed && frozen.definition.actions.is_some() {
                workflow
                    .admissions
                    .entry(self.task.clone())
                    .or_insert(super::ModelAdmission {
                        response: task
                            .result
                            .as_ref()
                            .and_then(|result| result.outputs.get("model_response"))
                            .cloned(),
                        expected_generation: frozen.accepted_generation,
                        status: super::ModelAdmissionStatus::Pending,
                        reason: None,
                        repair_allowed: false,
                        receipt_key: None,
                        aliases: std::collections::BTreeMap::new(),
                    });
            }
        }
        super::duration::request_limit_stop(&mut workflow);
        let continuation_refused = report.kind().terminal().is_none()
            && workflow
                .tasks
                .get(&self.task)
                .is_some_and(|task| task.stop_requested);
        let expected = self.runtime.advance(&mut workflow)?;
        self.runtime.save(&workflow, Some(expected), None)?;
        if continuation_refused {
            return Err(invalid(
                "further execution refused after the retained report exhausted its duration or stop allowance",
            ));
        }
        Ok(ObservationDisposition::Applied)
    }
}

fn invalid(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::InvalidTransition(error.to_string())
}
