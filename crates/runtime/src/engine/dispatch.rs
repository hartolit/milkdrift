//! Runnable dispatch, durable leases, immutable request construction, and executor reports.

use super::support::{
    CommandPlan, DispatchOutcome, ResolvedInputValue, checked_timestamp_add,
    execution_branch_state, invocation_value_reference, stable_idempotency_key,
};
use super::{CommandExecution, MAX_DURABLE_INVOCATION_REQUEST_BYTES, RuntimeService};
use crate::projection::{
    AttemptState, BranchState, NodeExecutionState, RunLifecycle, RunProjection,
};
use crate::{AdmissionRequest, RunCommand, RunCommandDocument, RuntimeError, SystemTransition};
use milkdrift_blueprint::{BlueprintRevision, Node, NodeKind, ReducerStrategy};
use milkdrift_capability::{
    BoundedJson, ErrorClass, IdempotencyBehavior, IdempotencyKey, InputReference, InvocationId,
    InvocationRequest, SideEffectClass,
};
use milkdrift_persistence::{
    AtomicRunCommitOutcome, BoundedDetail, NodeExecutionId, PersistenceError, Reason, RunEventKind,
    RunnableIndexEntry, TimestampMillis,
};
use milkdrift_workspace::{RunId, ScopeKind, ScopeReference};
use std::collections::BTreeMap;
use tracing::{info, warn};

impl RuntimeService {
    pub(super) fn dispatch_runnable(
        &self,
        entry: &RunnableIndexEntry,
        now: TimestampMillis,
    ) -> Result<DispatchOutcome, RuntimeError> {
        // Serialize the exact durable admission snapshot and lease CAS, then release
        // before entering the potentially blocking executor boundary. This prevents
        // same-service oversubscription without suppressing concurrent cancellation.
        let scheduler_guard = self.scheduler_gate.lock().map_err(|_error| {
            RuntimeError::Scheduling("runtime scheduler coordination lock is poisoned".to_owned())
        })?;
        let projection = self.projection(&entry.run)?;
        if projection.sequence() < entry.through_sequence
            || projection.lifecycle() != RunLifecycle::Running
        {
            return Ok(DispatchOutcome::Deferred);
        }
        if self.runnable_executions(&projection)?.get(&entry.execution) != Some(&entry.eligible_at)
        {
            return Ok(DispatchOutcome::Deferred);
        }
        let execution = projection
            .node_executions()
            .get(&entry.execution)
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("runnable execution is absent".to_owned())
            })?;
        if execution_branch_state(&projection, execution.execution())
            .is_some_and(|state| state != BranchState::Active)
        {
            return Ok(DispatchOutcome::Deferred);
        }
        let revision = self.revision_for_execution(&projection, &entry.execution)?;
        let node = revision
            .semantic()
            .nodes()
            .get(execution.node())
            .ok_or_else(|| RuntimeError::InvalidHistory("runnable node is absent".to_owned()))?;
        let requirement = match node.kind() {
            NodeKind::Task { requirement } => requirement.clone(),
            NodeKind::Reducer { config } => match config.strategy() {
                ReducerStrategy::Capability(operation) => {
                    milkdrift_capability::CapabilityRequirement::new(operation.clone())
                }
                ReducerStrategy::Collect | ReducerStrategy::First => {
                    return Ok(DispatchOutcome::Deferred);
                }
            },
            NodeKind::Branch { .. }
            | NodeKind::Fork { .. }
            | NodeKind::Join { .. }
            | NodeKind::Repeat { .. }
            | NodeKind::Wait { .. }
            | NodeKind::SignalWait { .. }
            | NodeKind::Subworkflow { .. }
            | NodeKind::Terminal { .. } => return Ok(DispatchOutcome::Deferred),
        };
        let branch = projection
            .scopes()
            .get(execution.scope())
            .and_then(|scope| match scope.kind() {
                ScopeKind::Branch { branch } => Some(branch.clone()),
                ScopeKind::RunRoot
                | ScopeKind::Iteration { .. }
                | ScopeKind::Subworkflow { .. } => None,
            });
        let admission = AdmissionRequest {
            run: entry.run.clone(),
            branch: branch.clone(),
            operation: requirement.operation().clone(),
        };
        let (usage, lease_revision) = self.admission_usage()?;
        if !self.config.scheduler_limits.allows(&admission, &usage) {
            return Ok(DispatchOutcome::Deferred);
        }
        let resolution = self.executor.resolve(&requirement, now.get())?;
        let contract = resolution.snapshot().operation_contract();
        let attempt = match execution.state() {
            NodeExecutionState::Eligible => self.next_attempt_id()?,
            NodeExecutionState::RetryPending(attempt)
                if projection
                    .attempts()
                    .get(attempt)
                    .is_some_and(|value| value.state() == &AttemptState::ReadyToSchedule) =>
            {
                attempt.clone()
            }
            NodeExecutionState::Scheduled(_)
            | NodeExecutionState::Running(_)
            | NodeExecutionState::RetryPending(_)
            | NodeExecutionState::Uncertain(_)
            | NodeExecutionState::CancelledBeforeDispatch
            | NodeExecutionState::RemovedProspectively(_)
            | NodeExecutionState::Terminal(_) => return Ok(DispatchOutcome::Deferred),
        };
        let invocation = self.next_invocation_id()?;
        let idempotency_key = match contract.idempotency() {
            IdempotencyBehavior::Unsupported => None,
            IdempotencyBehavior::CapabilityScoped | IdempotencyBehavior::ProviderProfileScoped => {
                Some(stable_idempotency_key(&entry.run, execution.execution())?)
            }
        };
        if let NodeExecutionState::RetryPending(retry_attempt) = execution.state() {
            let retry = projection
                .retries()
                .values()
                .find(|retry| retry.next_attempt() == retry_attempt)
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "retry-pending execution has no retry decision".to_owned(),
                    )
                })?;
            let previous = projection
                .attempts()
                .get(retry.previous_attempt())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "retry decision has no previous attempt".to_owned(),
                    )
                })?;
            if previous.side_effect().is_some_and(|classification| {
                classification.side_effect() == SideEffectClass::IdempotentWrite
            }) {
                let same_resolution = previous
                    .capability()
                    .is_some_and(|capability| capability.snapshot() == resolution.snapshot());
                let same_key = previous.idempotency_key() == idempotency_key.as_ref();
                if !same_resolution || !same_key {
                    warn!(
                        run = %entry.run,
                        execution = %execution.execution(),
                        previous_attempt = %previous.attempt(),
                        "idempotent-write retry retained because capability resolution or key changed"
                    );
                    return Ok(DispatchOutcome::Deferred);
                }
            }
        }
        let request = match self.invocation_request(
            &revision,
            &projection,
            node,
            execution.scope(),
            invocation.clone(),
            resolution.snapshot().capability().clone(),
            resolution.snapshot().provider_profile().cloned(),
            idempotency_key.clone(),
        ) {
            Ok(request) => request,
            Err(RuntimeError::Scheduling(_)) => {
                self.commit_pre_dispatch_failure(
                    &entry.run,
                    now,
                    execution.execution(),
                    "immutable invocation input materialization failed",
                )?;
                return Ok(DispatchOutcome::PreDispatchFailed);
            }
            Err(error) => return Err(error),
        };
        let request_bytes = serde_json::to_vec(&request)
            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
        if request_bytes.len() > MAX_DURABLE_INVOCATION_REQUEST_BYTES {
            self.commit_pre_dispatch_failure(
                &entry.run,
                now,
                execution.execution(),
                "immutable invocation request exceeds the durable event size budget",
            )?;
            return Ok(DispatchOutcome::PreDispatchFailed);
        }
        let lease = self.next_lease_id()?;
        let expires_at = checked_timestamp_add(now, self.config.lease_duration_ms)?;
        let schedule = CommandPlan {
            events: vec![
                RunEventKind::NodeScheduled {
                    node: node.id().clone(),
                    execution: execution.execution().clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                    idempotency_key: idempotency_key.clone(),
                    request: request.clone(),
                },
                RunEventKind::CapabilityResolved {
                    execution: execution.execution().clone(),
                    attempt: attempt.clone(),
                    requirement: requirement.clone(),
                    snapshot: resolution.snapshot().clone(),
                },
                RunEventKind::SideEffectClassified {
                    attempt: attempt.clone(),
                    side_effect: contract.side_effect(),
                    idempotency: contract.idempotency(),
                    idempotency_key: idempotency_key.clone(),
                },
                RunEventKind::LeaseGranted {
                    lease: lease.clone(),
                    execution: execution.execution().clone(),
                    attempt: attempt.clone(),
                    worker: self.config.worker.clone(),
                    expires_at,
                },
            ],
            expected_lease_revision: Some(lease_revision),
            ..CommandPlan::default()
        };
        match self.commit_internal_plan(
            &entry.run,
            now,
            SystemTransition::ScheduleAndLease {
                attempt: attempt.clone(),
            },
            schedule,
        ) {
            Ok(_) => {}
            Err(RuntimeError::Persistence(PersistenceError::LeaseRevisionConflict { .. }))
            | Err(RuntimeError::Persistence(PersistenceError::SequenceConflict { .. })) => {
                return Ok(DispatchOutcome::Deferred);
            }
            Err(error) => return Err(error),
        }
        drop(scheduler_guard);
        info!(
            run = %entry.run,
            revision = %revision.id(),
            node = %node.id(),
            execution = %execution.execution(),
            attempt = %attempt,
            invocation = %invocation,
            lease = %lease,
            "durable lease committed for caller-owned effect execution"
        );
        Ok(DispatchOutcome::Dispatched)
    }

    fn commit_pre_dispatch_failure(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        execution: &NodeExecutionId,
        detail: &'static str,
    ) -> Result<(), RuntimeError> {
        let plan = CommandPlan::one(RunEventKind::NodePreDispatchFailed {
            execution: execution.clone(),
            error_class: ErrorClass::InvalidRequest,
            detail: Some(BoundedDetail::new(detail)?),
        });
        let _ = self.commit_internal_plan(
            run,
            occurred_at,
            SystemTransition::TerminalizePreDispatchFailure {
                execution: execution.clone(),
            },
            plan,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn invocation_request(
        &self,
        revision: &BlueprintRevision,
        projection: &RunProjection,
        node: &Node,
        occurrence_scope: &ScopeReference,
        invocation: InvocationId,
        capability: milkdrift_capability::CapabilityId,
        provider_profile: Option<milkdrift_capability::ProviderProfileRef>,
        idempotency_key: Option<IdempotencyKey>,
    ) -> Result<InvocationRequest, RuntimeError> {
        self.validate_projected_scope(projection, occurrence_scope, &[])?;
        let mut inputs = Vec::new();
        for (port, declaration) in node.data_inputs() {
            let resolved = match node.kind() {
                NodeKind::Reducer { config }
                    if matches!(config.strategy(), ReducerStrategy::Capability(_))
                        && port == config.input_port() =>
                {
                    let references = self.ordered_reducer_references(
                        revision,
                        projection,
                        node,
                        port,
                        occurrence_scope,
                        &[],
                    )?;
                    if references.len() < usize::from(config.minimum_items()) {
                        return Err(RuntimeError::Scheduling(format!(
                            "capability reducer {} requires at least {} collected inputs",
                            node.id(),
                            config.minimum_items()
                        )));
                    }
                    vec![ResolvedInputValue::Inline {
                        value: BoundedJson::new(serde_json::to_value(references)?)
                            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?,
                        source: None,
                    }]
                }
                NodeKind::Task { .. }
                | NodeKind::Reducer { .. }
                | NodeKind::Branch { .. }
                | NodeKind::Fork { .. }
                | NodeKind::Join { .. }
                | NodeKind::Repeat { .. }
                | NodeKind::Wait { .. }
                | NodeKind::SignalWait { .. }
                | NodeKind::Subworkflow { .. }
                | NodeKind::Terminal { .. } => self.resolve_node_port_inputs(
                    revision,
                    projection,
                    node,
                    port,
                    occurrence_scope,
                    &[],
                )?,
            };
            if resolved.is_empty() {
                if declaration.is_required() {
                    return Err(RuntimeError::Scheduling(format!(
                        "required task input {}:{} is unresolved",
                        node.id(),
                        port
                    )));
                }
                continue;
            }
            if resolved.len() != 1 {
                return Err(RuntimeError::Scheduling(format!(
                    "task input {}:{} resolved to more than one exact value",
                    node.id(),
                    port
                )));
            }
            let resolved_value = resolved.into_iter().next().ok_or_else(|| {
                RuntimeError::InvalidHistory("resolved invocation input disappeared".to_owned())
            })?;
            let value = invocation_value_reference(resolved_value)?;
            inputs.push(
                InputReference::new(port.as_str().to_owned(), value)
                    .map_err(|error| RuntimeError::Scheduling(error.to_string()))?,
            );
        }
        InvocationRequest::new(
            invocation,
            capability,
            match node.kind() {
                NodeKind::Task { requirement } => requirement.operation().clone(),
                NodeKind::Reducer { config } => match config.strategy() {
                    ReducerStrategy::Capability(operation) => operation.clone(),
                    ReducerStrategy::Collect | ReducerStrategy::First => {
                        return Err(RuntimeError::Scheduling(
                            "a deterministic reducer cannot build an invocation".to_owned(),
                        ));
                    }
                },
                NodeKind::Branch { .. }
                | NodeKind::Fork { .. }
                | NodeKind::Join { .. }
                | NodeKind::Repeat { .. }
                | NodeKind::Wait { .. }
                | NodeKind::SignalWait { .. }
                | NodeKind::Subworkflow { .. }
                | NodeKind::Terminal { .. } => {
                    return Err(RuntimeError::Scheduling(
                        "only task or capability-backed reducer nodes can build invocations"
                            .to_owned(),
                    ));
                }
            },
            provider_profile,
            idempotency_key,
            inputs,
            BTreeMap::new(),
        )
        .map_err(|error| RuntimeError::Scheduling(error.to_string()))
    }

    pub(super) fn commit_internal_plan(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        transition: SystemTransition,
        plan: CommandPlan,
    ) -> Result<CommandExecution, RuntimeError> {
        let projection = self.projection(run)?;
        let reason = Reason::new(format!("internal runtime action: {}", transition.label()))?;
        let document = RunCommandDocument::new(
            self.next_command_id()?,
            run.clone(),
            self.config.internal_actor.clone(),
            projection.sequence(),
            occurred_at,
            reason,
            Vec::new(),
            RunCommand::SystemTransition { transition },
        )?;
        let receipt = document.receipt()?;
        let outcome = self.commit_accepted(&document, receipt, projection, plan, None)?;
        let (result, replayed) = match outcome {
            AtomicRunCommitOutcome::Committed(value) => (value, false),
            AtomicRunCommitOutcome::Replayed(value) => (value, true),
        };
        Ok(CommandExecution { result, replayed })
    }
}
