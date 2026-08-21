//! Command validation, planning, worker-report incorporation, and atomic command commits.

use super::RuntimeService;
use super::support::{
    CommandPlan, checked_timestamp_add, collect_required_artifacts, entry_nodes, event_kind_name,
    node_execution_mode, require_lifecycle, wait_signal_matches,
};
use crate::projection::{AttemptState, IterationState, RunLifecycle, RunProjection, TimerPurpose};
use crate::{RunCommand, RunCommandDocument, RuntimeError, WorkerReport};
use milkdrift_blueprint::{NodeKind, PortId, RepeatTermination, RevisionId, WorkflowId};
use milkdrift_capability::{
    BoundedJson, ErrorClass, InvocationEvent, InvocationEventKind, InvocationTerminal,
    TerminalStatus,
};
use milkdrift_persistence::{
    AtomicRunCommitOutcome, AtomicRunCommitRequest, AttemptId, AttemptUsage, BoundedDetail,
    CommandDisposition, CommandReceipt, CommandResultDocument, CurrencyCode,
    MAX_WORKSPACE_MUTATIONS_PER_COMMIT, NodeExecutionId, NodeOutcome, Reason, RunEventEnvelope,
    RunEventKind, RunIndexUpdate, SignalDeliveryMode, TimerId, TimestampMillis, WaitSatisfaction,
    WorkerId, WorkspaceAccounting, WorkspaceMutation,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactReference, ValueKey, ValueOrigin, ValueVersion, WorkspaceBudget,
    WorkspaceScope, WorkspaceValue, WorkspaceValueEntry,
};
use serde_json::json;
use std::collections::BTreeSet;
use tracing::{debug, warn};

impl RuntimeService {
    pub(super) fn plan_command(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
    ) -> Result<CommandPlan, RuntimeError> {
        match document.command() {
            RunCommand::CreateRun {
                workflow,
                revision,
                root_scope,
                workspace_budget,
                inputs,
            } => self.plan_create_run(
                document,
                projection,
                workflow,
                revision,
                root_scope,
                workspace_budget,
                inputs,
            ),
            RunCommand::StartRun => self.plan_start_run(document, projection),
            RunCommand::PauseRun => {
                require_lifecycle(projection, RunLifecycle::Running, "pause")?;
                Ok(CommandPlan::one(RunEventKind::RunPaused {
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                }))
            }
            RunCommand::ResumeRun => {
                require_lifecycle(projection, RunLifecycle::Paused, "resume")?;
                Ok(CommandPlan::one(RunEventKind::RunResumed {
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                }))
            }
            RunCommand::RequestCancellation => {
                if !matches!(
                    projection.lifecycle(),
                    RunLifecycle::Created | RunLifecycle::Running | RunLifecycle::Paused
                ) {
                    return Err(RuntimeError::InvalidTransition(
                        "only a created, running, or paused run can be cancelled".to_owned(),
                    ));
                }
                Ok(CommandPlan::one(RunEventKind::RunCancellationRequested {
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                }))
            }
            RunCommand::DeliverSignal {
                signal,
                signal_type,
                correlation,
                mode,
                payload,
            } => self.plan_signal(
                document,
                projection,
                signal,
                signal_type,
                correlation.as_ref(),
                *mode,
                payload,
            ),
            RunCommand::FireTimer { timer } => self.plan_timer(document, projection, timer),
            RunCommand::RequestRevisionAdoption {
                reconciliation,
                revision,
                policy,
            } => self.plan_revision_adoption(projection, reconciliation, revision, *policy),
            RunCommand::DecideReconciliation {
                plan,
                decision,
                outcome,
            } => self.plan_reconciliation_decision(document, projection, plan, decision, *outcome),
            RunCommand::ApplyReconciliation { plan } => {
                self.plan_reconciliation_application(document.run_id(), projection, plan)
            }
            RunCommand::DecideRepeatContinuation {
                repeat_execution,
                decision,
                outcome,
                approved_additional_iterations,
            } => {
                let execution = projection
                    .node_executions()
                    .get(repeat_execution)
                    .ok_or_else(|| {
                        RuntimeError::InvalidTransition(
                            "repeat continuation references an unknown execution".to_owned(),
                        )
                    })?;
                let revision = self.revision_for_execution(projection, repeat_execution)?;
                let node = revision
                    .semantic()
                    .nodes()
                    .get(execution.node())
                    .ok_or_else(|| {
                        RuntimeError::InvalidHistory(
                            "repeat continuation node is absent from the revision".to_owned(),
                        )
                    })?;
                let NodeKind::Repeat { config } = node.kind() else {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat execution is not configured to await approval".to_owned(),
                    ));
                };
                if config.termination() != RepeatTermination::AwaitApproval {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat execution is not configured to await approval".to_owned(),
                    ));
                }
                let frontier = projection
                    .iterations()
                    .values()
                    .filter(|iteration| iteration.repeat_execution() == repeat_execution)
                    .max_by_key(|iteration| iteration.iteration_number())
                    .ok_or_else(|| {
                        RuntimeError::InvalidTransition(
                            "repeat continuation has no iteration frontier".to_owned(),
                        )
                    })?;
                if frontier.state() != IterationState::ConditionRecorded(true) {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat continuation requires a true-condition frontier".to_owned(),
                    ));
                }
                let continuation = projection
                    .repeat_continuations()
                    .get(repeat_execution)
                    .ok_or_else(|| {
                        RuntimeError::InvalidTransition(
                            "repeat continuation has no durable authority request".to_owned(),
                        )
                    })?;
                let pending_request = continuation.pending_request().ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "repeat continuation has no pending durable authority request".to_owned(),
                    )
                })?;
                if continuation.is_rejected()
                    || pending_request.frontier_iteration() != frontier.iteration()
                {
                    return Err(RuntimeError::InvalidTransition(
                        "repeat continuation decision is outside its exact authority boundary"
                            .to_owned(),
                    ));
                }
                Ok(CommandPlan::one(RunEventKind::RepeatContinuationDecided {
                    repeat_execution: repeat_execution.clone(),
                    decision: decision.clone(),
                    actor: document.actor().clone(),
                    outcome: *outcome,
                    approved_additional_iterations: *approved_additional_iterations,
                    reason: document.reason().clone(),
                    evidence: document.evidence().to_vec(),
                }))
            }
            RunCommand::ResolveExternalWork {
                attempt,
                decision,
                action,
                remediation_node,
            } => self.plan_external_resolution(
                document,
                projection,
                attempt,
                decision,
                *action,
                remediation_node.as_ref(),
            ),
            RunCommand::SystemTransition { .. } => Err(RuntimeError::InvalidCommand(
                "system transitions are runtime-owned and cannot be submitted externally"
                    .to_owned(),
            )),
            RunCommand::WorkerReport { worker, report } => {
                self.plan_worker_report(document, projection, worker, report)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_create_run(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        workflow: &WorkflowId,
        revision_id: &RevisionId,
        root_scope: &WorkspaceScope,
        budget: &WorkspaceBudget,
        inputs: &[WorkspaceValueEntry],
    ) -> Result<CommandPlan, RuntimeError> {
        if projection.lifecycle() != RunLifecycle::Uncreated {
            return Err(RuntimeError::InvalidTransition(
                "run identity already exists".to_owned(),
            ));
        }
        if root_scope.reference().run() != document.run_id() {
            return Err(RuntimeError::InvalidTransition(
                "root workspace scope belongs to another run".to_owned(),
            ));
        }
        let revision = self.load_validated_revision(revision_id, Some(workflow))?;
        let mut references = BTreeSet::new();
        let expected_usage = self.store.workspace_usage(document.run_id())?;
        let mut resulting_usage = expected_usage;
        let mut required_artifacts = BTreeSet::new();
        let declared_inputs = revision.semantic().interface().inputs();
        let mut supplied_fields = BTreeSet::new();
        for input in inputs {
            if input.reference().scope() != root_scope.reference()
                || input.reference().version() != ValueVersion::FIRST
                || !matches!(input.origin(), ValueOrigin::Initial)
            {
                return Err(RuntimeError::InvalidTransition(
                    "run inputs must be initial values in the declared root scope".to_owned(),
                ));
            }
            let field = declared_inputs
                .keys()
                .find(|field| field.as_str() == input.reference().key().as_str())
                .ok_or_else(|| {
                    RuntimeError::InvalidTransition(format!(
                        "run input {} is not declared by the pinned workflow interface",
                        input.reference().key()
                    ))
                })?;
            supplied_fields.insert(field.clone());
            if !references.insert(input.reference().clone()) {
                return Err(RuntimeError::InvalidTransition(
                    "initial workspace value references must be distinct".to_owned(),
                ));
            }
            if let Some(artifact) = input.value().as_artifact() {
                if !self.store.is_committed(artifact)? {
                    return Err(RuntimeError::InvalidTransition(format!(
                        "initial artifact {} is not durably committed",
                        artifact.artifact()
                    )));
                }
                required_artifacts.insert(artifact.clone());
            }
        }
        if let Some(missing) = declared_inputs
            .iter()
            .find(|(field, declaration)| {
                declaration.is_required() && !supplied_fields.contains(*field)
            })
            .map(|(field, _)| field)
        {
            return Err(RuntimeError::InvalidTransition(format!(
                "required workflow input {missing} is absent"
            )));
        }
        let mut newly_referenced_artifacts = BTreeSet::new();
        for artifact in &required_artifacts {
            if !self
                .store
                .is_referenced_by_run(document.run_id(), artifact)?
            {
                resulting_usage = budget
                    .admit_artifact_reference(&resulting_usage, artifact)
                    .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
                newly_referenced_artifacts.insert(artifact.clone());
            }
        }
        for input in inputs {
            resulting_usage = budget
                .admit_value(&resulting_usage, input.value())
                .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        }
        let mut plan = CommandPlan::one(RunEventKind::RunCreated {
            workflow: workflow.clone(),
            revision: revision_id.clone(),
            revision_digest: revision.content_digest().clone(),
            root_scope: root_scope.clone(),
            workspace_budget: budget.clone(),
            inputs: references.into_iter().collect(),
        });
        plan.workspace.push(WorkspaceMutation::CreateScope {
            scope: root_scope.clone(),
        });
        plan.workspace.extend(
            inputs
                .iter()
                .cloned()
                .map(|entry| WorkspaceMutation::PutValue { entry }),
        );
        plan.creation_usage = Some((expected_usage, resulting_usage, newly_referenced_artifacts));
        plan.required_artifacts.extend(required_artifacts);
        Ok(plan)
    }

    fn plan_start_run(
        &self,
        _document: &RunCommandDocument,
        projection: &RunProjection,
    ) -> Result<CommandPlan, RuntimeError> {
        require_lifecycle(projection, RunLifecycle::Created, "start")?;
        let revision = self.current_revision(projection)?;
        let scope = projection
            .root_scope()
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("created run has no root scope".to_owned())
            })?
            .reference()
            .clone();
        let mut plan = CommandPlan::one(RunEventKind::RunStarted);
        for node in entry_nodes(&revision) {
            let node_view = revision.semantic().nodes().get(node).ok_or_else(|| {
                RuntimeError::InvalidHistory("entry node is absent from its revision".to_owned())
            })?;
            plan.events.push(RunEventKind::NodeBecameEligible {
                node: node.clone(),
                execution: self.next_execution_id()?,
                scope: scope.clone(),
                mode: node_execution_mode(node_view),
            });
        }
        if plan.events.len() == 1 {
            return Err(RuntimeError::InvalidTransition(
                "pinned revision has no entry node".to_owned(),
            ));
        }
        Ok(plan)
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_signal(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        signal: &milkdrift_persistence::SignalId,
        signal_type: &milkdrift_persistence::SignalTypeId,
        correlation: Option<&milkdrift_persistence::CorrelationKey>,
        mode: SignalDeliveryMode,
        payload: &BoundedJson,
    ) -> Result<CommandPlan, RuntimeError> {
        if !projection.lifecycle().is_active() {
            return Err(RuntimeError::InvalidTransition(
                "signals are accepted only for an active run".to_owned(),
            ));
        }
        if let Some(existing) = projection.signals().get(signal) {
            if existing.signal_type() != signal_type
                || existing.correlation() != correlation
                || existing.mode() != mode
                || existing.payload() != payload
            {
                return Err(RuntimeError::InvalidTransition(
                    "signal identity was reused with conflicting delivery facts".to_owned(),
                ));
            }
            return Ok(CommandPlan::one(RunEventKind::SignalDeduplicated {
                signal: signal.clone(),
                duplicate_command: document.command_id().clone(),
            }));
        }
        let mut plan = CommandPlan::one(RunEventKind::SignalReceived {
            signal: signal.clone(),
            signal_type: signal_type.clone(),
            correlation: correlation.cloned(),
            mode,
            payload: payload.clone(),
        });
        if mode == SignalDeliveryMode::Broadcast {
            return Ok(plan);
        }
        if projection.lifecycle() == RunLifecycle::Paused {
            return Ok(plan);
        }
        let compatible = projection
            .waits()
            .values()
            .filter(|wait| {
                wait.is_pending() && wait_signal_matches(wait.condition(), signal_type, correlation)
            })
            .map(|wait| wait.execution().clone())
            .min();
        if let Some(execution) = compatible {
            let entries = self.signal_payload_entries(projection, &execution, payload, &[])?;
            let event_cost = entries.len().checked_add(2).ok_or_else(|| {
                RuntimeError::Scheduling("one-shot signal event cost overflow".to_owned())
            })?;
            if plan.events.len().saturating_add(event_cost)
                > milkdrift_persistence::MAX_EVENTS_PER_COMMIT
                || entries.len() > MAX_WORKSPACE_MUTATIONS_PER_COMMIT
            {
                return Err(RuntimeError::InvalidTransition(
                    "one signal consumer exceeds atomic runtime bounds".to_owned(),
                ));
            }
            plan.events.push(RunEventKind::SignalConsumed {
                signal: signal.clone(),
                execution: execution.clone(),
            });
            for entry in entries {
                let value = entry.reference().clone();
                plan.workspace.push(WorkspaceMutation::PutValue { entry });
                plan.events
                    .push(RunEventKind::DeterministicOutputPublished {
                        execution: execution.clone(),
                        value,
                        artifact: None,
                    });
            }
            plan.events.push(RunEventKind::WaitSatisfied {
                execution,
                cause: WaitSatisfaction::Signal {
                    signal: signal.clone(),
                },
            });
        }
        Ok(plan)
    }

    fn plan_timer(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        timer: &TimerId,
    ) -> Result<CommandPlan, RuntimeError> {
        let timer_view = projection.timers().get(timer).ok_or_else(|| {
            RuntimeError::InvalidTransition(format!("timer {timer} is not registered"))
        })?;
        if !timer_view.is_pending() {
            return Err(RuntimeError::InvalidTransition(format!(
                "timer {timer} already fired"
            )));
        }
        if document.issued_at() < timer_view.fire_at() {
            return Err(RuntimeError::InvalidTransition(format!(
                "timer {timer} is not due until {}",
                timer_view.fire_at()
            )));
        }
        let mut plan = CommandPlan::one(RunEventKind::TimerFired {
            timer: timer.clone(),
            observed_at: document.issued_at(),
        });
        if projection.lifecycle() == RunLifecycle::Paused {
            return Ok(plan);
        }
        if let TimerPurpose::Wait {
            execution: Some(execution),
        } = timer_view.purpose()
            && projection
                .waits()
                .get(execution)
                .is_some_and(|wait| wait.is_pending())
        {
            plan.events.push(RunEventKind::WaitSatisfied {
                execution: execution.clone(),
                cause: WaitSatisfaction::Timer {
                    timer: timer.clone(),
                },
            });
        }
        Ok(plan)
    }

    fn plan_worker_report(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        worker: &WorkerId,
        report: &WorkerReport,
    ) -> Result<CommandPlan, RuntimeError> {
        if document.actor() != &self.config.internal_actor || worker != &self.config.worker {
            return Err(RuntimeError::InvalidTransition(
                "worker reports require the configured worker and trusted internal actor boundary"
                    .to_owned(),
            ));
        }
        match report {
            WorkerReport::LeaseAccepted { lease, attempt } => {
                let lease_view = projection.leases().get(lease).ok_or_else(|| {
                    RuntimeError::InvalidTransition(format!("unknown lease {lease}"))
                })?;
                if lease_view.worker() != worker
                    || lease_view.attempt() != attempt
                    || !lease_view.is_active()
                {
                    return Err(RuntimeError::InvalidTransition(
                        "lease acceptance does not match active worker ownership".to_owned(),
                    ));
                }
                let attempt_view = projection.attempts().get(attempt).ok_or_else(|| {
                    RuntimeError::InvalidHistory("lease attempt is absent".to_owned())
                })?;
                let invocation = attempt_view.invocation().ok_or_else(|| {
                    RuntimeError::InvalidHistory("leased attempt has no invocation".to_owned())
                })?;
                Ok(CommandPlan::one(RunEventKind::NodeStarted {
                    execution: attempt_view.execution().clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                }))
            }
            WorkerReport::Heartbeat { lease, expires_at } => {
                let lease_view = projection.leases().get(lease).ok_or_else(|| {
                    RuntimeError::InvalidTransition(format!("unknown lease {lease}"))
                })?;
                if lease_view.worker() != worker || !lease_view.is_active() {
                    return Err(RuntimeError::InvalidTransition(
                        "heartbeat does not match active worker ownership".to_owned(),
                    ));
                }
                if *expires_at <= lease_view.expires_at() || *expires_at <= document.issued_at() {
                    return Err(RuntimeError::InvalidTransition(
                        "heartbeat expiration must advance the active lease into the future"
                            .to_owned(),
                    ));
                }
                Ok(CommandPlan::one(RunEventKind::LeaseHeartbeatRecorded {
                    lease: lease.clone(),
                    expires_at: *expires_at,
                }))
            }
            WorkerReport::Started { attempt } => {
                let attempt_view = self.worker_attempt(projection, worker, attempt)?;
                let invocation = attempt_view.invocation().ok_or_else(|| {
                    RuntimeError::InvalidHistory("scheduled attempt has no invocation".to_owned())
                })?;
                Ok(CommandPlan::one(RunEventKind::NodeStarted {
                    execution: attempt_view.execution().clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                }))
            }
            WorkerReport::Invocation { attempt, report } => {
                let attempt_view = self.historically_owned_attempt(projection, worker, attempt)?;
                if attempt_view.invocation() != Some(report.invocation()) {
                    return Err(RuntimeError::InvalidTransition(
                        "invocation report correlation does not match the attempt".to_owned(),
                    ));
                }
                if let InvocationEventKind::Terminal { terminal } = report.kind()
                    && !self.worker_owns_active_lease(projection, worker, attempt)
                {
                    return self.plan_late_terminal_report(
                        projection,
                        worker,
                        attempt,
                        report.sequence(),
                        terminal,
                    );
                }
                let _ = self.worker_attempt(projection, worker, attempt)?;
                self.plan_invocation_report(document, projection, attempt, report)
            }
            WorkerReport::Cancellation {
                attempt,
                acknowledgement,
            } => {
                let attempt_view = self.worker_attempt(projection, worker, attempt)?;
                if attempt_view.invocation() != Some(acknowledgement.invocation()) {
                    return Err(RuntimeError::InvalidTransition(
                        "cancellation acknowledgement names another invocation".to_owned(),
                    ));
                }
                let mut plan = CommandPlan::one(RunEventKind::InvocationCancellationAcknowledged {
                    attempt: attempt.clone(),
                    acknowledgement: acknowledgement.clone(),
                });
                if acknowledgement.accepted() && acknowledgement.terminal_boundary() {
                    plan.events.push(RunEventKind::NodeTerminal {
                        execution: attempt_view.execution().clone(),
                        attempt: attempt.clone(),
                        report_sequence: self.next_report_sequence(projection, attempt)?,
                        outcome: NodeOutcome::Cancelled,
                        error_class: None,
                        detail: acknowledgement
                            .detail()
                            .map(|detail| BoundedDetail::new(detail.to_owned()))
                            .transpose()?,
                    });
                }
                Ok(plan)
            }
            WorkerReport::Terminal {
                attempt,
                report_sequence,
                terminal,
            } => {
                let _ = self.historically_owned_attempt(projection, worker, attempt)?;
                if !self.worker_owns_active_lease(projection, worker, attempt) {
                    return self.plan_late_terminal_report(
                        projection,
                        worker,
                        attempt,
                        *report_sequence,
                        terminal,
                    );
                }
                self.plan_terminal_report(document, projection, attempt, *report_sequence, terminal)
            }
        }
    }

    fn worker_owns_active_lease(
        &self,
        projection: &RunProjection,
        worker: &WorkerId,
        attempt: &AttemptId,
    ) -> bool {
        projection.leases().values().any(|lease| {
            lease.attempt() == attempt && lease.worker() == worker && lease.is_active()
        })
    }

    fn historically_owned_attempt<'a>(
        &self,
        projection: &'a RunProjection,
        worker: &WorkerId,
        attempt: &AttemptId,
    ) -> Result<&'a crate::projection::NodeAttemptProjection, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        let historically_owned = attempt_view.leases().iter().any(|lease| {
            projection
                .leases()
                .get(lease)
                .is_some_and(|lease_view| lease_view.worker() == worker)
        });
        if !historically_owned {
            return Err(RuntimeError::InvalidTransition(
                "worker never owned a durable lease for the attempt".to_owned(),
            ));
        }
        Ok(attempt_view)
    }

    fn worker_attempt<'a>(
        &self,
        projection: &'a RunProjection,
        worker: &WorkerId,
        attempt: &AttemptId,
    ) -> Result<&'a crate::projection::NodeAttemptProjection, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        if !self.worker_owns_active_lease(projection, worker, attempt) {
            return Err(RuntimeError::InvalidTransition(
                "worker does not own an active lease for the attempt".to_owned(),
            ));
        }
        Ok(attempt_view)
    }

    fn plan_invocation_report(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        attempt: &AttemptId,
        report: &InvocationEvent,
    ) -> Result<CommandPlan, RuntimeError> {
        match report.kind() {
            InvocationEventKind::Progress {
                message,
                completed_units,
                total_units,
            } => {
                let expected = self.next_report_sequence(projection, attempt)?;
                if report.sequence() != expected {
                    return Err(RuntimeError::InvalidTransition(format!(
                        "progress report sequence must be exactly {expected}"
                    )));
                }
                Ok(CommandPlan::one(RunEventKind::NodeProgressRecorded {
                    attempt: attempt.clone(),
                    report_sequence: report.sequence(),
                    detail: BoundedDetail::new(message.clone())?,
                    completed_units: *completed_units,
                    total_units: *total_units,
                }))
            }
            InvocationEventKind::Output { name, reference } => {
                self.plan_output_report(projection, attempt, report.sequence(), name, reference)
            }
            InvocationEventKind::Terminal { terminal } => self.plan_terminal_report(
                document,
                projection,
                attempt,
                report.sequence(),
                terminal,
            ),
        }
    }

    fn plan_output_report(
        &self,
        projection: &RunProjection,
        attempt: &AttemptId,
        report_sequence: u64,
        name: &str,
        reference: &milkdrift_capability::ArtifactReference,
    ) -> Result<CommandPlan, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        if !matches!(attempt_view.state(), AttemptState::Running) {
            return Err(RuntimeError::InvalidTransition(
                "output report requires a running attempt".to_owned(),
            ));
        }
        let expected = self.next_report_sequence(projection, attempt)?;
        if report_sequence != expected {
            return Err(RuntimeError::InvalidTransition(format!(
                "output report sequence must be exactly {expected}"
            )));
        }
        let execution = projection
            .node_executions()
            .get(attempt_view.execution())
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("attempt execution is absent".to_owned())
            })?;
        let scheduled_revision = projection.revision_for_attempt(attempt).ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "scheduled attempt has no governing revision pin".to_owned(),
            )
        })?;
        let revision = self.load_validated_revision(scheduled_revision, projection.workflow())?;
        let node = revision
            .semantic()
            .nodes()
            .get(execution.node())
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "scheduled attempt node is absent from its governing revision".to_owned(),
                )
            })?;
        let output_port = PortId::new(name.to_owned())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        if !node.data_outputs().contains_key(&output_port) {
            return Err(RuntimeError::InvalidTransition(format!(
                "executor output {name} is not a declared data output of node {}",
                node.id()
            )));
        }
        let (metadata, artifact) = self.resolve_executor_artifact(reference)?;
        let key = ValueKey::new(output_port.as_str().to_owned())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let entry = self.projected_output_entry(
            projection,
            execution.scope(),
            key,
            WorkspaceValue::Artifact(artifact.clone()),
            &[],
        )?;
        let mut plan = CommandPlan::default();
        if !projection.artifacts().contains_key(artifact.artifact()) {
            plan.events
                .push(RunEventKind::ArtifactPublished { metadata });
        }
        plan.events.push(RunEventKind::NodeOutputPublished {
            execution: attempt_view.execution().clone(),
            attempt: attempt.clone(),
            report_sequence,
            value: entry.reference().clone(),
            artifact: Some(artifact.clone()),
        });
        plan.workspace.push(WorkspaceMutation::PutValue { entry });
        plan.required_artifacts.insert(artifact);
        Ok(plan)
    }

    fn plan_late_terminal_report(
        &self,
        projection: &RunProjection,
        worker: &WorkerId,
        attempt: &AttemptId,
        report_sequence: u64,
        terminal: &InvocationTerminal,
    ) -> Result<CommandPlan, RuntimeError> {
        let attempt_view = self.historically_owned_attempt(projection, worker, attempt)?;
        let obligation = attempt_view.obligation().ok_or_else(|| {
            RuntimeError::InvalidTransition(
                "late terminal evidence is accepted only for an uncertain external outcome"
                    .to_owned(),
            )
        })?;
        if attempt_view.terminal().is_some() || attempt_view.late_terminal_evidence().is_some() {
            return Err(RuntimeError::InvalidTransition(
                "attempt already has terminal evidence".to_owned(),
            ));
        }
        if report_sequence < obligation.report_sequence() {
            return Err(RuntimeError::InvalidTransition(format!(
                "late terminal report sequence must be at least {}",
                obligation.report_sequence()
            )));
        }
        if terminal.status() == TerminalStatus::Uncertain {
            return Err(RuntimeError::InvalidTransition(
                "late evidence must establish a known terminal observation".to_owned(),
            ));
        }
        let classified = attempt_view.side_effect().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "late terminal attempt has no side-effect classification".to_owned(),
            )
        })?;
        if terminal.side_effect() > classified.side_effect() {
            return Err(RuntimeError::InvalidTransition(
                "late terminal observation exceeds the frozen side-effect classification"
                    .to_owned(),
            ));
        }
        Ok(CommandPlan::one(
            RunEventKind::LateTerminalEvidenceRecorded {
                attempt: attempt.clone(),
                worker: worker.clone(),
                report_sequence,
                terminal: terminal.clone(),
            },
        ))
    }

    fn plan_terminal_report(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        attempt: &AttemptId,
        report_sequence: u64,
        terminal: &InvocationTerminal,
    ) -> Result<CommandPlan, RuntimeError> {
        let attempt_view = projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidTransition(format!("unknown attempt {attempt}")))?;
        let expected = self.next_report_sequence(projection, attempt)?;
        if report_sequence != expected || attempt_view.is_completed() {
            return Err(RuntimeError::InvalidTransition(format!(
                "terminal report sequence must be exactly {expected} and attempt must be active"
            )));
        }
        let classified = attempt_view.side_effect().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "terminal attempt has no side-effect classification".to_owned(),
            )
        })?;
        if terminal.side_effect() > classified.side_effect() {
            return Err(RuntimeError::InvalidTransition(
                "terminal observation exceeds the frozen side-effect classification".to_owned(),
            ));
        }
        let published: BTreeSet<_> = attempt_view
            .outputs()
            .iter()
            .filter_map(|output| output.artifact())
            .map(|artifact| artifact.digest().to_hex())
            .collect();
        if terminal
            .outputs()
            .iter()
            .any(|output| !published.contains(output.digest()))
        {
            return Err(RuntimeError::InvalidTransition(
                "terminal output was not first published as a durable workspace artifact"
                    .to_owned(),
            ));
        }
        let mut plan = CommandPlan::default();
        if let Some(usage) = terminal.usage() {
            let cost = match usage.cost_micros().zip(usage.currency()) {
                Some((micros, currency)) => Some(milkdrift_persistence::MonetaryUsage {
                    micros,
                    currency: CurrencyCode::new(currency.to_owned())?,
                }),
                None => None,
            };
            plan.events.push(RunEventKind::AttemptUsageRecorded {
                attempt: attempt.clone(),
                usage: AttemptUsage {
                    input_units: usage.input_units(),
                    output_units: usage.output_units(),
                    duration_ms: usage.duration_ms(),
                    cost,
                },
            });
        }
        if terminal.status() == TerminalStatus::Uncertain {
            plan.events.push(RunEventKind::ExternalOutcomeUncertain {
                attempt: attempt.clone(),
                report_sequence,
                side_effect: classified.side_effect(),
                reason: Reason::new(terminal.failure().map_or(
                    "executor reported an uncertain external outcome",
                    |failure| failure.message(),
                ))?,
                evidence: document.evidence().to_vec(),
            });
            return Ok(plan);
        }
        let (outcome, error_class, detail) = match terminal.status() {
            TerminalStatus::Success => (NodeOutcome::Succeeded, None, None),
            TerminalStatus::Cancelled => (NodeOutcome::Cancelled, None, None),
            TerminalStatus::Failure | TerminalStatus::Rejected => {
                let failure = terminal.failure().ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "failed terminal report lacks details".to_owned(),
                    )
                })?;
                (
                    if terminal.status() == TerminalStatus::Rejected {
                        NodeOutcome::Rejected
                    } else {
                        NodeOutcome::Failed
                    },
                    Some(failure.class()),
                    Some(BoundedDetail::new(failure.message().to_owned())?),
                )
            }
            TerminalStatus::Uncertain => {
                return Err(RuntimeError::InvalidHistory(
                    "uncertain terminal routing failure".to_owned(),
                ));
            }
        };
        plan.events.push(RunEventKind::NodeTerminal {
            execution: attempt_view.execution().clone(),
            attempt: attempt.clone(),
            report_sequence,
            outcome,
            error_class,
            detail,
        });
        if let Some(failure) = terminal.failure()
            && self.config.retry_policy.permits_automatic_retry(
                attempt_view.attempt_number(),
                failure.class(),
                failure.retryable(),
                classified.side_effect(),
                classified.idempotency(),
                classified.idempotency_key(),
            )
        {
            match self.build_retry_event(
                attempt_view.execution(),
                attempt,
                attempt_view.attempt_number(),
                document.issued_at(),
                failure.class(),
                failure.retry_after_ms(),
                "bounded automatic retry policy admitted another attempt",
            ) {
                Ok(retry) => plan.events.push(retry),
                Err(error) => warn!(
                    attempt = %attempt,
                    reason = %error,
                    "truthful terminal report retained without an out-of-policy retry timer"
                ),
            }
        }
        Ok(plan)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_retry_event(
        &self,
        execution: &NodeExecutionId,
        previous_attempt: &AttemptId,
        previous_attempt_number: u32,
        observed_at: TimestampMillis,
        error_class: ErrorClass,
        retry_after_ms: Option<u64>,
        rationale: &'static str,
    ) -> Result<RunEventKind, RuntimeError> {
        let attempt_number = previous_attempt_number
            .checked_add(1)
            .ok_or_else(|| RuntimeError::Scheduling("attempt number overflow".to_owned()))?;
        let delay = self
            .config
            .retry_policy
            .retry_delay_ms(attempt_number, 0, retry_after_ms)?;
        let fire_at = checked_timestamp_add(observed_at, delay)?;
        let reason = Reason::new(rationale)?;
        let next_attempt = self.next_attempt_id()?;
        let timer = self.next_timer_id()?;
        Ok(RunEventKind::NodeRetryScheduled {
            execution: execution.clone(),
            previous_attempt: previous_attempt.clone(),
            next_attempt,
            attempt_number,
            timer,
            fire_at,
            error_class,
            reason,
        })
    }

    fn resolve_executor_artifact(
        &self,
        reference: &milkdrift_capability::ArtifactReference,
    ) -> Result<(milkdrift_workspace::ArtifactMetadata, ArtifactReference), RuntimeError> {
        let artifact_id = ArtifactId::new(reference.identity().to_owned())
            .map_err(|error| RuntimeError::InvalidTransition(error.to_string()))?;
        let metadata = self.store.metadata(&artifact_id)?.ok_or_else(|| {
            RuntimeError::InvalidTransition(format!(
                "executor artifact {} has no committed metadata",
                reference.identity()
            ))
        })?;
        let durable = metadata.reference().clone();
        let media_matches = reference
            .media_type()
            .is_none_or(|media| media == durable.media_type().as_str());
        let size_matches = reference
            .size_bytes()
            .is_none_or(|size| size == durable.size_bytes());
        if reference.digest() != durable.digest().to_hex()
            || !media_matches
            || !size_matches
            || !self.store.is_committed(&durable)?
        {
            return Err(RuntimeError::InvalidTransition(
                "executor artifact reference differs from committed content metadata".to_owned(),
            ));
        }
        Ok((metadata, durable))
    }

    pub(super) fn next_report_sequence(
        &self,
        projection: &RunProjection,
        attempt: &AttemptId,
    ) -> Result<u64, RuntimeError> {
        projection
            .attempts()
            .get(attempt)
            .ok_or_else(|| RuntimeError::InvalidHistory("attempt is absent".to_owned()))?
            .last_report_sequence()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| RuntimeError::InvalidTransition("report sequence overflow".to_owned()))
    }

    pub(super) fn commit_accepted(
        &self,
        document: &RunCommandDocument,
        receipt: CommandReceipt,
        projection: RunProjection,
        mut plan: CommandPlan,
    ) -> Result<AtomicRunCommitOutcome, RuntimeError> {
        if plan.events.is_empty() {
            return Err(RuntimeError::InvalidTransition(
                "an accepted transition must emit at least one event".to_owned(),
            ));
        }
        let mut candidate = projection.clone();
        let mut envelopes = Vec::with_capacity(plan.events.len());
        let mut sequence = projection.sequence();
        for kind in plan.events.drain(..) {
            sequence = sequence.next()?;
            let event = RunEventEnvelope::new(
                self.next_event_id()?,
                document.run_id().clone(),
                sequence,
                document.issued_at(),
                kind,
            )?;
            candidate.apply_replayed(&event)?;
            debug!(
                event = %event.event_id(),
                sequence = event.sequence().get(),
                event_type = event_kind_name(event.kind()),
                "projected candidate event"
            );
            envelopes.push(event);
        }
        let revision = if candidate.revision().is_some() {
            Some(self.current_revision(&candidate)?)
        } else {
            None
        };
        if let (Some(revision), true) = (revision.as_ref(), candidate.lifecycle().is_active()) {
            self.extend_structured_progress(
                document.run_id(),
                document.issued_at(),
                revision,
                &mut candidate,
                &mut envelopes,
                &mut plan.workspace,
            )?;
        }
        let event_ids = envelopes
            .iter()
            .map(|event| event.event_id().clone())
            .collect::<Vec<_>>();
        let resulting_sequence = candidate.sequence();
        let result_payload = BoundedJson::new(json!({
            "status": "accepted",
            "event_count": event_ids.len(),
            "resulting_sequence": resulting_sequence.get(),
        }))
        .map_err(|error| RuntimeError::InvalidCommand(error.to_string()))?;
        let result = CommandResultDocument::new(
            document.command_id().clone(),
            document.run_id().clone(),
            receipt.fingerprint().clone(),
            CommandDisposition::Accepted,
            resulting_sequence,
            event_ids,
            result_payload,
        )?;
        let required_artifacts = collect_required_artifacts(&envelopes, &plan.workspace)?;
        for artifact in &required_artifacts {
            if !self.store.is_committed(artifact)? {
                return Err(RuntimeError::InvalidTransition(format!(
                    "event references uncommitted artifact {}",
                    artifact.artifact()
                )));
            }
        }
        if !plan.required_artifacts.is_empty()
            && !plan
                .required_artifacts
                .iter()
                .all(|artifact| required_artifacts.contains(artifact))
        {
            return Err(RuntimeError::InvalidTransition(
                "planned artifact set is not represented by event/workspace facts".to_owned(),
            ));
        }
        let budget = candidate.workspace_budget().ok_or_else(|| {
            RuntimeError::InvalidHistory(
                "accepted run transition has no workspace budget".to_owned(),
            )
        })?;
        let (expected_usage, resulting_usage, newly_referenced_artifacts) =
            match plan.creation_usage {
                Some(usage) => usage,
                None => self.workspace_accounting_transition(
                    &projection,
                    &plan.workspace,
                    budget,
                    &required_artifacts,
                )?,
            };
        let accounting = WorkspaceAccounting {
            budget: budget.clone(),
            expected_usage,
            resulting_usage,
        };
        let indexes = self.index_update(
            document.run_id(),
            &projection,
            &candidate,
            document.issued_at(),
        )?;
        let request = AtomicRunCommitRequest::new(
            receipt,
            envelopes,
            plan.workspace,
            Some(accounting),
            required_artifacts.into_iter().collect(),
            newly_referenced_artifacts.into_iter().collect(),
            plan.expected_lease_catalog,
            result,
            indexes,
        )?;
        let should_checkpoint =
            self.should_checkpoint_projection(projection.sequence(), &candidate);
        let outcome = self.store.commit_command(&request)?;
        if should_checkpoint
            && matches!(&outcome, AtomicRunCommitOutcome::Committed(_))
            && let Err(error) = self.persist_projection_snapshot(document.run_id(), &candidate)
        {
            warn!(
                run = %document.run_id(),
                sequence = candidate.sequence().get(),
                reason = %error,
                "optional projection checkpoint could not be persisted"
            );
        }
        Ok(outcome)
    }

    pub(super) fn commit_rejected(
        &self,
        document: &RunCommandDocument,
        receipt: CommandReceipt,
        detail: &str,
    ) -> Result<AtomicRunCommitOutcome, RuntimeError> {
        let payload = BoundedJson::new(json!({
            "status": "rejected",
            "reason": detail,
        }))
        .map_err(|error| RuntimeError::InvalidCommand(error.to_string()))?;
        let result = CommandResultDocument::new(
            document.command_id().clone(),
            document.run_id().clone(),
            receipt.fingerprint().clone(),
            CommandDisposition::Rejected,
            document.expected_sequence(),
            Vec::new(),
            payload,
        )?;
        let request = AtomicRunCommitRequest::new(
            receipt,
            Vec::new(),
            Vec::new(),
            None,
            Vec::new(),
            Vec::new(),
            None,
            result,
            RunIndexUpdate::default(),
        )?;
        Ok(self.store.commit_command(&request)?)
    }
}
