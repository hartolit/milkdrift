//! Bounded scheduler maintenance and structured aggregate drivers.

use super::support::{
    CommandPlan, DispatchOutcome, bounded_projection_map_keys, node_execution_mode,
};
use super::{RuntimeService, SchedulerTickResult};
use crate::projection::{BranchState, NodeExecutionState};
use crate::{RunCommand, RunCommandDocument, RuntimeError, SystemTransition, select_fair_runnable};
use milkdrift_persistence::{
    NodeOutcome, PageSize, Reason, RunEventKind, RunSummaryCursor, RunSummaryIndex, RunnableCursor,
    RunnableIndexEntry, TimestampMillis,
};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use tracing::{info_span, warn};

impl RuntimeService {
    pub(super) fn next_nonterminal_page(
        &self,
        cursor: &Mutex<Option<RunSummaryCursor>>,
        limit: PageSize,
        operation: &'static str,
    ) -> Result<Vec<RunSummaryIndex>, RuntimeError> {
        let mut cursor = cursor.lock().map_err(|_error| {
            RuntimeError::Scheduling(format!(
                "runtime {operation} pagination cursor lock is poisoned"
            ))
        })?;
        let page = self.store.nonterminal_run_page(cursor.as_ref(), limit)?;
        *cursor = page.next;
        Ok(page.runs)
    }

    fn next_runnable_page(
        &self,
        eligible_through: TimestampMillis,
        limit: PageSize,
    ) -> Result<Vec<RunnableIndexEntry>, RuntimeError> {
        let mut cursor = self.runnable_cursor.lock().map_err(|_error| {
            RuntimeError::Scheduling(
                "runtime runnable pagination cursor lock is poisoned".to_owned(),
            )
        })?;
        let cycle_boundary = cursor
            .as_ref()
            .map_or(eligible_through, RunnableCursor::eligible_through);
        let page = self
            .store
            .runnable_page(cycle_boundary, cursor.as_ref(), limit)?;
        *cursor = page.next;
        Ok(page.entries)
    }

    /// Performs one bounded fair scheduling pass.  This call never spawns, polls, or
    /// retains an in-memory queue; another call is required for later work.
    #[expect(
        clippy::too_many_lines,
        reason = "scheduling is one ordered admission pass over a single projection snapshot"
    )]
    pub fn scheduler_tick(&self) -> Result<SchedulerTickResult, RuntimeError> {
        let now = self.clock.now()?;
        let span = info_span!(
            "runtime.scheduler_tick",
            worker = %self.config.worker,
            observed_at = now.get(),
            accepting = self.is_accepting_admission(),
        );
        let _entered = span.enter();
        let mut result = SchedulerTickResult::default();
        let limit = PageSize::new(u32::from(self.config.maximum_tick_items))?;
        let maximum_visits = usize::from(self.config.maximum_tick_items);
        // Reserve one physical visit for runnable admission while allowing the
        // deterministic driver to use the remainder of the scheduler-wide
        // budget. A structured transition commonly consumes one eligible visit
        // and one successor-frontier visit, so an arbitrary half split would
        // halve the documented bounded progress without improving the bound.
        let structured_visit_limit = maximum_visits.saturating_sub(1);
        let runnable_visit_limit;
        {
            let _scheduler_guard = self.scheduler_gate.lock().map_err(|_error| {
                RuntimeError::Scheduling(
                    "runtime scheduler coordination lock is poisoned".to_owned(),
                )
            })?;
            let accepting_admission = self.is_accepting_admission();
            let maintenance_visit_limit = if accepting_admission {
                structured_visit_limit
            } else {
                // No runnable admission follows a shutdown pass, so every bounded
                // visit remains available for draining already-owned work.
                maximum_visits
            };
            self.structured_scan_budget
                .store(maintenance_visit_limit, Ordering::Release);
            self.structured_scan_budget_active
                .store(true, Ordering::Release);
            let structured_result = (|| -> Result<(), RuntimeError> {
                if !accepting_admission {
                    // Closing dispatch admission must not suppress an already durable
                    // cancellation boundary. This path may release waits and request
                    // executor cancellation, but it never creates a new dispatch lease.
                    self.propagate_cancellation(now, limit)?;
                    return Ok(());
                }
                let timer_allowance = self.structured_scan_budget.load(Ordering::Acquire);
                if timer_allowance > 0 {
                    let timer_limit =
                        PageSize::new(u32::try_from(timer_allowance).map_err(|_error| {
                            RuntimeError::Scheduling(
                                "timer visit limit conversion overflow".to_owned(),
                            )
                        })?)?;
                    let due_timers = self.store.due_timers(now, timer_limit)?;
                    let claimed = self.claim_structured_scan_visits(due_timers.len());
                    for timer in due_timers.into_iter().take(claimed) {
                        let expected = self.store.head(&timer.run)?;
                        let command = RunCommandDocument::new(
                            self.next_command_id()?,
                            timer.run,
                            self.config.internal_actor.clone(),
                            expected,
                            now,
                            Reason::new(
                                "scheduler observed a durable timer at or after its deadline",
                            )?,
                            Vec::new(),
                            RunCommand::FireTimer { timer: timer.timer },
                        )?;
                        let _ = self.handle_internal_command(&command)?;
                    }
                }
                self.propagate_cancellation(now, limit)?;
                self.drive_reconciliation_restarts(now, limit)?;
                self.drive_child_aggregates(now, limit)?;
                self.drive_structured_runs(now, limit)
            })();
            self.structured_scan_budget_active
                .store(false, Ordering::Release);
            structured_result?;
            if !accepting_admission {
                result.deferred = 1;
                return Ok(result);
            }
            let unused = self.structured_scan_budget.load(Ordering::Acquire);
            let used = maintenance_visit_limit.saturating_sub(unused);
            runnable_visit_limit = maximum_visits.saturating_sub(used).max(1);
        }

        let runnable_limit =
            PageSize::new(u32::try_from(runnable_visit_limit).map_err(|_error| {
                RuntimeError::Scheduling("runnable visit limit conversion overflow".to_owned())
            })?)?;
        let entries = self.next_runnable_page(now, runnable_limit)?;
        let selected = select_fair_runnable(entries, runnable_visit_limit);
        for entry in selected {
            result.examined = result.examined.saturating_add(1);
            if !self.is_accepting_admission() {
                result.deferred = result.deferred.saturating_add(1);
                continue;
            }
            match self.dispatch_runnable(&entry, now) {
                Ok(DispatchOutcome::Dispatched) => {
                    result.dispatched = result.dispatched.saturating_add(1);
                }
                Ok(DispatchOutcome::Deferred) => {
                    result.deferred = result.deferred.saturating_add(1);
                }
                Ok(DispatchOutcome::PreDispatchFailed) => {
                    result.completed = result.completed.saturating_add(1);
                }
                Err(error) => {
                    warn!(
                        run = %entry.run,
                        execution = %entry.execution,
                        reason = %error,
                        "runnable dispatch failed"
                    );
                    return Err(error);
                }
            }
        }
        Ok(result)
    }

    fn drive_structured_runs(
        &self,
        now: TimestampMillis,
        limit: PageSize,
    ) -> Result<(), RuntimeError> {
        for summary in
            self.next_nonterminal_page(&self.structured_cursor, limit, "structured-progress")?
        {
            if self.structured_scan_budget.load(Ordering::Acquire) == 0 {
                break;
            }
            let projection = self.projection(&summary.run)?;
            if !projection.lifecycle().is_active() {
                continue;
            }
            let revision = self.current_revision(&projection)?;
            let mut candidate = projection.clone();
            let mut events = Vec::new();
            let mut workspace = Vec::new();
            self.extend_structured_progress(
                &summary.run,
                now,
                &revision,
                &mut candidate,
                &mut events,
                &mut workspace,
            )?;
            if !events.is_empty() {
                let plan = CommandPlan {
                    events: events.iter().map(|event| event.kind().clone()).collect(),
                    workspace,
                    ..CommandPlan::default()
                };
                let _ = self.commit_internal_plan(
                    &summary.run,
                    now,
                    SystemTransition::DriveStructuredProgress,
                    plan,
                )?;
            }
        }
        Ok(())
    }

    fn drive_reconciliation_restarts(
        &self,
        now: TimestampMillis,
        limit: PageSize,
    ) -> Result<(), RuntimeError> {
        for summary in self.next_nonterminal_page(
            &self.reconciliation_cursor,
            limit,
            "reconciliation-restart",
        )? {
            if self.structured_scan_budget.load(Ordering::Acquire) == 0 {
                break;
            }
            let projection = self.projection(&summary.run)?;
            let claimed = self
                .claim_structured_scan_visits(projection.pending_reconciliation_restarts().len());
            let mut allowance = claimed;
            let restart_keys = bounded_projection_map_keys(
                &summary.run,
                projection.pending_reconciliation_restarts(),
                &self.reconciliation_restart_cursors,
                &mut allowance,
                "reconciliation restart scan cursor",
            )?;
            for restart_key in restart_keys {
                let source_execution = projection
                    .pending_reconciliation_restarts()
                    .get(&restart_key)
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "reconciliation restart token disappeared".to_owned(),
                        )
                    })?;
                let source = projection
                    .node_executions()
                    .get(source_execution)
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "reconciliation cancellation source is absent".to_owned(),
                        )
                    })?;
                if source.state() != &NodeExecutionState::Terminal(NodeOutcome::Cancelled) {
                    continue;
                }
                let revision = self.current_revision(&projection)?;
                if !revision.semantic().nodes().contains_key(source.node()) {
                    return Err(RuntimeError::Reconciliation(
                        "cancel-and-restart target was removed from the adopted revision"
                            .to_owned(),
                    ));
                }
                let execution = self.next_execution_id()?;
                let mut plan = CommandPlan::one(RunEventKind::NodeBecameEligible {
                    node: source.node().clone(),
                    execution: execution.clone(),
                    scope: source.scope().clone(),
                    mode: node_execution_mode(
                        revision
                            .semantic()
                            .nodes()
                            .get(source.node())
                            .ok_or_else(|| {
                                RuntimeError::InvalidHistory(
                                    "reconciliation restart node is absent".to_owned(),
                                )
                            })?,
                    ),
                });
                if let Some(branch) = projection.branch_for_execution(source.execution())
                    && branch.state() == BranchState::Active
                {
                    plan.events.push(RunEventKind::BranchChildAdded {
                        branch: branch.branch().clone(),
                        execution,
                    });
                }
                let _ = self.commit_internal_plan(
                    &summary.run,
                    now,
                    SystemTransition::RestartReconciledExecution,
                    plan,
                )?;
            }
        }
        Ok(())
    }
}
