//! Bounded observation of workflow-backed work after its execution worker has been released.
use super::super::support::{cancellation_reason_for_execution, run_drain_reason};
use super::{RuntimeService, reporter::stable_effect_command_id};
use crate::{RuntimeError, WorkerReport};
use milkdrift_persistence::{PageSize, Reason};

impl RuntimeService {
    /// A service grant permits the mechanism, but cannot extend the accepted public call.
    /// Follow local ancestors to their owning caller; a serving boundary evaluates its own policy.
    pub(super) fn published_entry_allowed(
        &self,
        projection: &crate::projection::RunProjection,
        now: milkdrift_persistence::TimestampMillis,
    ) -> Result<bool, RuntimeError> {
        let mut source = projection.published_source().cloned();
        for _ in 0..milkdrift_capability::MAX_PUBLICATION_DEPTH {
            let Some(current) = source else {
                return Ok(true);
            };
            let plan = self.store.published_invocation(&current)?.ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "internal entry lost its published association".to_owned(),
                )
            })?;
            if now.get() >= plan.deadline_unix_ms {
                return Ok(false);
            }
            match current {
                milkdrift_persistence::published::PublishedInvocationSource::Local {
                    run,
                    attempt,
                } => {
                    let caller = self.projection(&run)?;
                    let Some(active) = caller.attempts().get(&attempt) else {
                        return Ok(false);
                    };
                    if !active.is_active()
                        || active.published_invocation() != Some(&plan)
                        || cancellation_reason_for_execution(
                            &caller,
                            active.execution(),
                            run_drain_reason(&caller),
                        )
                        .is_some()
                    {
                        return Ok(false);
                    }
                    let mut request = plan.caller.request().clone();
                    request.evaluated_at = milkdrift_authority::BoundaryTimeMillis::new(now.get());
                    if !self.authority.evaluate(&request)?.is_allowed() {
                        return Ok(false);
                    }
                    source = caller.published_source().cloned();
                }
                milkdrift_persistence::published::PublishedInvocationSource::Serving { .. } => {
                    return Ok(self.executor.published_serving_entry_allowed(&plan)?);
                }
            }
        }
        if source.is_some() {
            return Err(RuntimeError::InvalidHistory(
                "published entry ancestry exceeds its bound".to_owned(),
            ));
        }
        Ok(true)
    }

    pub(in crate::engine) fn drive_published_invocations(
        &self,
        limit: PageSize,
    ) -> Result<(), RuntimeError> {
        let _guard = self.published_gate.lock().map_err(|_| {
            RuntimeError::Scheduling("published continuation lock poisoned".to_owned())
        })?;
        let mut cursor = self.published_cursor.lock().map_err(|_| {
            RuntimeError::Scheduling("published continuation cursor poisoned".to_owned())
        })?;
        let (plans, next_cursor) = self.store.published_local_page(cursor.as_ref(), limit)?;
        for plan in &plans {
            let milkdrift_persistence::published::PublishedInvocationSource::Local {
                run,
                attempt: id,
            } = &plan.source
            else {
                return Err(RuntimeError::InvalidHistory(
                    "local pending page returned a serving operation".to_owned(),
                ));
            };
            let projection = self.projection(run)?;
            let attempt = projection.attempts().get(id).ok_or_else(|| {
                RuntimeError::InvalidHistory("pending publication lost its attempt".to_owned())
            })?;
            if !attempt.is_active() || attempt.published_invocation() != Some(plan) {
                return Err(RuntimeError::InvalidHistory(
                    "pending publication index differs from projection".to_owned(),
                ));
            }
            let mut request = plan.caller.request().clone();
            request.evaluated_at =
                milkdrift_authority::BoundaryTimeMillis::new(self.clock.now()?.get());
            let caller_allowed = self.authority.evaluate(&request)?.is_allowed();
            let cancelled = !caller_allowed
                || self.clock.now()?.get() >= plan.deadline_unix_ms
                || cancellation_reason_for_execution(
                    &projection,
                    attempt.execution(),
                    run_drain_reason(&projection),
                )
                .is_some();
            if cancelled
                && projection
                    .node_executions()
                    .get(attempt.execution())
                    .is_some_and(|execution| execution.cancellation().is_none())
            {
                self.commit_internal_plan_from_projection(
                    run,
                    projection.clone(),
                    self.clock.now()?,
                    crate::SystemTransition::PropagateStructuredCancellation,
                    super::super::support::CommandPlan::one(
                        milkdrift_persistence::RunEventKind::NodeExecutionCancellationRequested {
                            execution: attempt.execution().clone(),
                            attempt: id.clone(),
                            reason: Reason::new(
                                "published caller cancelled, expired, or lost current authority",
                            )?,
                        },
                    ),
                )?;
            }
            let next = attempt
                .last_report_sequence()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory("published report sequence exhausted".to_owned())
                })?;
            let event = match self.executor.continue_published(plan, cancelled, next) {
                Ok(Some(event)) => event,
                Ok(None) => continue,
                Err(error) => {
                    // The saved association remains recoverable. Never repeat ordinary entry or
                    // manufacture a terminal outcome because observation/recording is unavailable.
                    tracing::warn!(run = %run, attempt = %attempt.attempt(), reason = %error, "published continuation remains pending");
                    continue;
                }
            };
            let terminal = matches!(
                event.kind(),
                milkdrift_capability::InvocationEventKind::Terminal { .. }
            );
            if !cancelled
                && event.kind().terminal().is_some_and(|terminal| {
                    terminal.status() == milkdrift_capability::TerminalStatus::Cancelled
                })
                && projection
                    .node_executions()
                    .get(attempt.execution())
                    .is_some_and(|execution| execution.cancellation().is_none())
            {
                // An authorized editor can cancel the linked internal run independently.
                // Preserve that causal cancellation before applying its terminal observation
                // to the caller's attempt; do not fabricate a caller cancellation request.
                self.commit_internal_plan_from_projection(
                    run,
                    projection.clone(),
                    self.clock.now()?,
                    crate::SystemTransition::PropagateStructuredCancellation,
                    super::super::support::CommandPlan::one(
                        milkdrift_persistence::RunEventKind::NodeExecutionCancellationRequested {
                            execution: attempt.execution().clone(),
                            attempt: id.clone(),
                            reason: Reason::new("linked internal workflow reported cancellation")?,
                        },
                    ),
                )?;
            }
            let report = WorkerReport::Invocation {
                attempt: attempt.attempt().clone(),
                report: event,
            };
            let observed = self.submit_effect_observation(
                run,
                stable_effect_command_id(run, attempt.attempt(), &report)?,
                self.clock.now()?,
                Reason::new("observed exact published workflow continuation")?,
                report,
            );
            if terminal && !self.store.published_local_pending(&plan.source)? {
                // Terminal projection compaction may already have removed the attempt. The
                // journal transaction removes its pending index even if its reply was lost.
                self.executor.complete_published(plan)?;
            }
            observed?;
        }
        *cursor = next_cursor;
        Ok(())
    }
}

impl RuntimeService {
    /// Recover the child through the canonical create/start command association saved by its
    /// caller. Runtime still owns both command receipts and every internal run event.
    pub fn arrange_published_run(
        &self,
        plan: &milkdrift_persistence::published::PublishedInvocationPlan,
        publications: &dyn milkdrift_persistence::published::PublishedMethodStore,
    ) -> Result<(), RuntimeError> {
        use crate::{CommandAuthorityClaim, RunCommand, RunCommandDocument};
        use milkdrift_persistence::CommandDisposition;
        plan.validate()?;
        if publications.published_invocation(&plan.source)?.as_ref() != Some(plan) {
            return Err(RuntimeError::InvalidCommand(
                "published child lacks its exact authoritative association".to_owned(),
            ));
        }
        let claim = CommandAuthorityClaim::new(
            plan.service.grant.clone(),
            plan.service.grant_revision,
            plan.service.grant_digest.clone(),
            plan.service.revocation_generation,
        )?;
        let method = publications
            .published_method(&plan.capability, plan.generation)?
            .ok_or_else(|| {
                RuntimeError::InvalidCommand("published implementation is absent".to_owned())
            })?
            .method;
        if method.digest()? != plan.method_digest || method.service != plan.service {
            return Err(RuntimeError::InvalidCommand(
                "published implementation identity changed".to_owned(),
            ));
        }
        let create = RunCommandDocument::from_json(plan.create_command.as_bytes())?;
        let start = RunCommandDocument::from_json(plan.start_command.as_bytes())?;
        if create.run_id() != &plan.child_run
            || start.run_id() != &plan.child_run
            || create.actor() != &plan.service.actor
            || start.actor() != &plan.service.actor
            || create.expected_sequence() != milkdrift_persistence::RunSequence::ZERO
            || start.expected_sequence() != milkdrift_persistence::RunSequence::new(2)
            || create.issued_at() != start.issued_at()
            || !matches!(create.command(), RunCommand::CreateRun { revision, workspace_budget, .. }
                if revision == &method.revision && workspace_budget == &method.workspace_budget)
            || !matches!(start.command(), RunCommand::StartRun)
        {
            return Err(RuntimeError::InvalidCommand(
                "published commands differ from the service association".to_owned(),
            ));
        }
        let created = self.handle_authorized_command(&create, &claim)?;
        if created.result().disposition() != CommandDisposition::Accepted {
            return Err(RuntimeError::InvalidTransition(
                "published child creation was refused".to_owned(),
            ));
        }
        self.bind_published_run(plan)?;
        let started = self.handle_authorized_command(&start, &claim)?;
        if started.result().disposition() != CommandDisposition::Accepted {
            return Err(RuntimeError::InvalidTransition(
                "published child start was refused".to_owned(),
            ));
        }
        Ok(())
    }

    fn bind_published_run(
        &self,
        plan: &milkdrift_persistence::published::PublishedInvocationPlan,
    ) -> Result<(), RuntimeError> {
        use super::super::support::CommandPlan;
        use crate::SystemTransition;
        use milkdrift_persistence::RunEventKind;
        let child = self.projection(&plan.child_run)?;
        match child.published_source() {
            Some(source) if source == &plan.source => {}
            Some(_) => {
                return Err(RuntimeError::InvalidHistory(
                    "published child belongs to another acceptance".to_owned(),
                ));
            }
            None => {
                self.commit_internal_plan_from_projection(
                    &plan.child_run,
                    child,
                    self.clock.now()?,
                    SystemTransition::InheritExecutionAuthority,
                    CommandPlan::one(RunEventKind::PublishedRunBound {
                        source: plan.source.clone(),
                    }),
                )?;
            }
        }
        Ok(())
    }
}

impl RuntimeService {
    /// Record cleanup for this exact accepted child. The child command receipt is the stable
    /// cancellation association, so a lost reply cannot request a second cancellation action.
    pub fn cancel_published_run(
        &self,
        plan: &milkdrift_persistence::published::PublishedInvocationPlan,
        publications: &dyn milkdrift_persistence::published::PublishedMethodStore,
    ) -> Result<(), RuntimeError> {
        use crate::{RunCommand, RunCommandDocument};
        if publications.published_invocation(&plan.source)?.as_ref() != Some(plan) {
            return Err(RuntimeError::InvalidCommand(
                "published cancellation lacks exact acceptance".to_owned(),
            ));
        }
        let child = self.projection(&plan.child_run)?;
        if child.lifecycle() == crate::RunLifecycle::Uncreated {
            return Ok(());
        }
        if child.published_source().is_none() {
            // Creation may have committed just before its reply was lost. Complete the saved
            // association/account before cancellation can make that never-started child terminal.
            // Replay an existing receipt only: cleanup never authorizes a new create or start.
            let create = RunCommandDocument::from_json(plan.create_command.as_bytes())?;
            if self
                .store
                .command_result(&plan.child_run, create.command_id())?
                .is_none()
            {
                return Err(RuntimeError::InvalidHistory(
                    "published cancellation lost its exact create receipt".to_owned(),
                ));
            }
            let claim = crate::CommandAuthorityClaim::new(
                plan.service.grant.clone(),
                plan.service.grant_revision,
                plan.service.grant_digest.clone(),
                plan.service.revocation_generation,
            )?;
            let created = self.handle_authorized_command(&create, &claim)?;
            if !created.replayed()
                || created.result().disposition()
                    != milkdrift_persistence::CommandDisposition::Accepted
            {
                return Err(RuntimeError::InvalidHistory(
                    "published cleanup requires the accepted create receipt".to_owned(),
                ));
            }
            self.bind_published_run(plan)?;
        } else if child.published_source() != Some(&plan.source) {
            return Err(RuntimeError::InvalidHistory(
                "published cancellation child belongs to another acceptance".to_owned(),
            ));
        }
        if let Some(receipt) = self
            .store
            .command_result(&plan.child_run, &plan.cancel_command)?
        {
            if receipt.disposition() != milkdrift_persistence::CommandDisposition::Accepted
                || receipt.authorization().is_some()
                || self.projection(&plan.child_run)?.cancellation().is_none()
            {
                return Err(RuntimeError::InvalidTransition(
                    "linked cancellation receipt does not establish internal cleanup".to_owned(),
                ));
            }
            return Ok(());
        }
        let child = self.projection(&plan.child_run)?;
        if child.lifecycle().is_completed() {
            return Ok(());
        }
        let command = RunCommandDocument::new(
            plan.cancel_command.clone(),
            plan.child_run.clone(),
            self.config.internal_actor.clone(),
            child.sequence(),
            self.clock.now()?,
            Reason::new("accepted public invocation requested internal cancellation")?,
            Vec::new(),
            RunCommand::RequestCancellation,
        )?;
        let receipt = self.handle_internal_command(&command)?;
        if receipt.result().disposition() != milkdrift_persistence::CommandDisposition::Accepted {
            return Err(RuntimeError::InvalidTransition(
                "linked cancellation was rejected".to_owned(),
            ));
        }
        Ok(())
    }
}
