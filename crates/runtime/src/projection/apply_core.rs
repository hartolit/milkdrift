use milkdrift_capability::{IdempotencyBehavior, SideEffectClass};
use milkdrift_persistence::{
    AuthorityDecision, NodeExecutionMode, NodeOutcome, ReconciliationAction,
    ReconciliationClassification, RecoveryClassification, RunEventEnvelope, RunEventKind,
    RunOutcome,
};

use crate::RuntimeError;

use super::helpers::{ensure_unique, invalid_at, new_attempt, same_logical_invocation_request};
use super::node::{
    AttemptState, AttemptTerminal, CapabilityResolution, DeterministicNodeTerminalProjection,
    ExternalOutcomeObligation, LateTerminalEvidence, LeaseProjection, LeaseState,
    NodeAttemptProjection, NodeExecutionCancellationProjection, NodeExecutionProjection,
    NodeExecutionState, ProgressObservation, PublishedNodeOutput, RetainedExternalOutcome,
    RetryProjection, RetryState, SideEffectClassification, TimerProjection, TimerPurpose,
    TimerState,
};
use super::run::{
    RevisionPin, RunCancellation, RunLifecycle, RunProjection, RunTerminalProjection,
    RunTerminationIntent,
};

impl RunProjection {
    #[allow(clippy::too_many_lines)]
    pub(super) fn apply_kind(&mut self, event: &RunEventEnvelope) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::RunCreated {
                workflow,
                revision,
                revision_digest,
                root_scope,
                workspace_budget,
                inputs,
            } => {
                if self.lifecycle != RunLifecycle::Uncreated {
                    return Err(invalid_at(event, "run creation may occur exactly once"));
                }
                if root_scope.reference().run() != event.run_id()
                    || !root_scope.kind().is_run_root()
                    || root_scope.parent().is_some()
                {
                    return Err(invalid_at(
                        event,
                        "run creation requires a parentless root scope owned by the envelope run",
                    ));
                }
                ensure_unique(inputs, event, "run input reference")?;
                if u64::try_from(inputs.len())
                    .map_err(|_| invalid_at(event, "input count overflow"))?
                    > workspace_budget.max_value_versions()
                {
                    return Err(invalid_at(
                        event,
                        "run inputs exceed the workspace value budget",
                    ));
                }
                for input in inputs {
                    if input.scope() != root_scope.reference() {
                        return Err(invalid_at(
                            event,
                            "every run input must belong to the root scope",
                        ));
                    }
                }
                self.run_id = Some(event.run_id().clone());
                self.lifecycle = RunLifecycle::Created;
                self.workflow = Some(workflow.clone());
                self.revision = Some(revision.clone());
                self.revision_digest = Some(revision_digest.clone());
                self.pins.clear();
                self.pins.push(RevisionPin {
                    revision: revision.clone(),
                    digest: revision_digest.clone(),
                    effective_sequence: sequence,
                    plan: None,
                });
                self.root_scope = Some(root_scope.clone());
                self.workspace_budget = Some(workspace_budget.clone());
                self.inputs = inputs.clone();
                self.scopes
                    .insert(root_scope.reference().clone(), root_scope.clone());
                for input in inputs {
                    self.record_workspace_value(input, event)?;
                }
            }
            RunEventKind::RevisionPinned {
                previous,
                revision,
                revision_digest,
                plan,
            } => {
                if self.pending_pin.as_ref() != Some(plan)
                    || self.revision.as_ref() != Some(previous)
                    || previous == revision
                {
                    return Err(invalid_at(
                        event,
                        "revision pin does not match the immediately preceding applied plan",
                    ));
                }
                let recorded =
                    self.reconciliation.plans.get(plan).ok_or_else(|| {
                        invalid_at(event, "revision pin references an unknown plan")
                    })?;
                if recorded.from_revision != *previous || recorded.to_revision != *revision {
                    return Err(invalid_at(
                        event,
                        "revision pin differs from its immutable plan",
                    ));
                }
                let retired_epoch_nodes: Vec<_> = recorded
                    .items
                    .iter()
                    .filter(|item| {
                        matches!(
                            item.classification,
                            ReconciliationClassification::Added
                                | ReconciliationClassification::ChangedPending
                        ) && item.action == ReconciliationAction::UseNewOnNextInvocation
                    })
                    .filter_map(|item| item.node.clone())
                    .collect();
                let completed_to_reconsider: Vec<_> = recorded
                    .items
                    .iter()
                    .filter_map(|item| item.execution.as_ref())
                    .filter(|execution| {
                        self.node_executions
                            .get(*execution)
                            .is_some_and(|execution| {
                                execution.state
                                    == NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                            })
                    })
                    .cloned()
                    .collect();
                self.revision = Some(revision.clone());
                self.revision_digest = Some(revision_digest.clone());
                self.pins.clear();
                self.pins.push(RevisionPin {
                    revision: revision.clone(),
                    digest: revision_digest.clone(),
                    effective_sequence: sequence,
                    plan: Some(plan.clone()),
                });
                for execution in self.node_executions.values_mut() {
                    if retired_epoch_nodes.contains(&execution.node)
                        && execution.epoch_retired_sequence.is_none()
                    {
                        execution.epoch_retired_sequence = Some(sequence);
                    }
                }
                for execution in self.settled_node_executions.values_mut() {
                    if retired_epoch_nodes.contains(&execution.node)
                        && execution.epoch_retired_sequence.is_none()
                    {
                        execution.epoch_retired_sequence = Some(sequence);
                    }
                }
                self.pending_successor_executions
                    .extend(completed_to_reconsider);
                self.pending_pin = None;
            }
            RunEventKind::RunStarted => {
                if self.lifecycle != RunLifecycle::Created {
                    return Err(invalid_at(event, "only a created run may start"));
                }
                self.lifecycle = RunLifecycle::Running;
            }
            RunEventKind::RunPaused { .. } => {
                if self.lifecycle != RunLifecycle::Running {
                    return Err(invalid_at(event, "only a running run may pause"));
                }
                self.lifecycle = RunLifecycle::Paused;
            }
            RunEventKind::RunResumed { .. } => {
                if self.lifecycle != RunLifecycle::Paused {
                    return Err(invalid_at(event, "only a paused run may resume"));
                }
                self.lifecycle = RunLifecycle::Running;
            }
            RunEventKind::RunCancellationRequested { reason, evidence } => {
                if !matches!(
                    self.lifecycle,
                    RunLifecycle::Created | RunLifecycle::Running | RunLifecycle::Paused
                ) || self.cancellation.is_some()
                {
                    return Err(invalid_at(
                        event,
                        "cancellation intent is duplicate or out of state",
                    ));
                }
                self.cancellation = Some(RunCancellation {
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    sequence,
                });
                self.lifecycle = RunLifecycle::Cancelling;
            }
            RunEventKind::RunTerminationRequested { outcome, reason } => {
                if self.lifecycle != RunLifecycle::Running
                    || self.cancellation.is_some()
                    || self.termination.is_some()
                    || *outcome != RunOutcome::Failed
                {
                    return Err(invalid_at(
                        event,
                        "run termination intent must be one first explicit failed drain on a running run",
                    ));
                }
                self.termination = Some(RunTerminationIntent {
                    outcome: *outcome,
                    reason: reason.clone(),
                    sequence,
                });
            }
            RunEventKind::RunTerminal {
                outcome,
                outputs,
                artifacts,
                reason,
            } => {
                if !matches!(
                    self.lifecycle,
                    RunLifecycle::Running
                        | RunLifecycle::Paused
                        | RunLifecycle::Cancelling
                        | RunLifecycle::Created
                ) {
                    return Err(invalid_at(event, "run terminal fact is out of state"));
                }
                if *outcome == RunOutcome::Cancelled && self.cancellation.is_none() {
                    return Err(invalid_at(
                        event,
                        "cancelled outcome requires durable cancellation intent",
                    ));
                }
                if self.lifecycle == RunLifecycle::Cancelling && *outcome != RunOutcome::Cancelled {
                    return Err(invalid_at(
                        event,
                        "durable cancellation intent requires a cancelled run outcome",
                    ));
                }
                if self.cancellation.is_none()
                    && self.termination.as_ref().is_some_and(|termination| {
                        *outcome != termination.outcome
                            || reason.as_ref() != Some(&termination.reason)
                    })
                {
                    return Err(invalid_at(
                        event,
                        "run terminal outcome contradicts its durable explicit-terminal drain",
                    ));
                }
                if self.lifecycle == RunLifecycle::Created && *outcome != RunOutcome::Cancelled {
                    return Err(invalid_at(
                        event,
                        "an unstarted run may only terminate as cancelled",
                    ));
                }
                self.ensure_terminal_quiescent(event)?;
                ensure_unique(outputs, event, "terminal output reference")?;
                ensure_unique(artifacts, event, "terminal artifact reference")?;
                for output in outputs {
                    self.validate_known_workspace_value(output, event)?;
                }
                for artifact in artifacts {
                    self.validate_published_artifact(artifact, event)?;
                }
                self.terminal = Some(RunTerminalProjection {
                    outcome: *outcome,
                    outputs: outputs.clone(),
                    artifacts: artifacts.clone(),
                    reason: reason.clone(),
                    sequence,
                });
                self.lifecycle = RunLifecycle::Terminal(*outcome);
            }
            RunEventKind::NodeBecameEligible {
                node,
                execution,
                scope,
                mode,
            } => {
                self.validate_scope_reference(scope, event)?;
                if self.node_executions.contains_key(execution)
                    || self.settled_node_executions.contains_key(execution)
                {
                    return Err(invalid_at(
                        event,
                        "node execution identity was already created",
                    ));
                }
                self.reserved_executions.remove(execution);
                self.node_executions.insert(
                    execution.clone(),
                    NodeExecutionProjection {
                        execution: execution.clone(),
                        node: node.clone(),
                        scope: scope.clone(),
                        mode: *mode,
                        revision: self
                            .revision
                            .clone()
                            .ok_or_else(|| invalid_at(event, "node execution has no revision"))?,
                        epoch_retired_sequence: None,
                        created_sequence: sequence,
                        created_at: event.occurred_at(),
                        attempts: Vec::new(),
                        attempt_count: 0,
                        state: NodeExecutionState::Eligible,
                        cancellation: None,
                        deterministic_terminal: None,
                        outputs: Vec::new(),
                    },
                );
                self.execution_ids_by_node
                    .entry(node.clone())
                    .or_default()
                    .insert(execution.clone());
                self.eligible_executions.insert(execution.clone());
                self.activate_execution(execution, event)?;
                if self
                    .pending_reconciliation_restarts
                    .remove(&(node.clone(), scope.clone()))
                    .is_some()
                {
                    self.adjust_scope_ownership(scope, false, event)?;
                }
            }
            RunEventKind::NodeExecutionCancelledBeforeDispatch { execution, reason } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.state != NodeExecutionState::Eligible
                    || !execution_view.attempts.is_empty()
                    || execution_view.cancellation.is_some()
                    || !self.has_execution_cancellation_source(execution)
                {
                    return Err(invalid_at(
                        event,
                        "pre-dispatch cancellation requires an eligible, attempt-free execution and a structured cancellation source",
                    ));
                }
                let execution_view = self
                    .node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?;
                execution_view.cancellation = Some(NodeExecutionCancellationProjection {
                    attempt: None,
                    reason: reason.clone(),
                    sequence,
                });
                execution_view.state = NodeExecutionState::CancelledBeforeDispatch;
                self.eligible_executions.remove(execution);
                self.deactivate_execution(execution, event)?;
            }
            RunEventKind::NodeExecutionCancellationRequested {
                execution,
                attempt,
                reason,
            } => {
                let execution_view = self.execution(execution, event)?;
                let attempt_view = self.attempt(attempt, event)?;
                if execution_view.attempts.last() != Some(attempt)
                    || execution_view.cancellation.is_some()
                    || attempt_view.execution != *execution
                    || !matches!(
                        attempt_view.state,
                        AttemptState::Scheduled | AttemptState::Leased | AttemptState::Running
                    )
                    || !matches!(
                        execution_view.state,
                        NodeExecutionState::Scheduled(ref active)
                            | NodeExecutionState::Running(ref active)
                            if active == attempt
                    )
                    || !self.has_execution_cancellation_source(execution)
                {
                    return Err(invalid_at(
                        event,
                        "attempt cancellation must target the latest scheduled, leased, or running attempt with structured authority",
                    ));
                }
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .cancellation = Some(NodeExecutionCancellationProjection {
                    attempt: Some(attempt.clone()),
                    reason: reason.clone(),
                    sequence,
                });
            }
            RunEventKind::NodeScheduled {
                node,
                execution,
                attempt,
                invocation,
                idempotency_key,
                request,
            } => {
                if self.invocations.contains(invocation) {
                    return Err(invalid_at(
                        event,
                        "invocation identity was already scheduled",
                    ));
                }
                let execution_view = self.execution(execution, event)?;
                if execution_view.node != *node
                    || execution_view.mode != NodeExecutionMode::Executor
                    || request.invocation() != invocation
                    || request.idempotency_key() != idempotency_key.as_ref()
                {
                    return Err(invalid_at(
                        event,
                        "scheduled node differs from its execution or is runtime-owned",
                    ));
                }
                if execution_view.cancellation.is_some() {
                    return Err(invalid_at(
                        event,
                        "a cancelled execution cannot schedule another invocation",
                    ));
                }
                let is_first = execution_view.attempt_count == 0;
                if is_first {
                    if execution_view.state != NodeExecutionState::Eligible
                        || self.attempts.contains_key(attempt)
                    {
                        return Err(invalid_at(
                            event,
                            "first attempt is duplicate or out of state",
                        ));
                    }
                    self.attempts.insert(
                        attempt.clone(),
                        new_attempt(
                            attempt.clone(),
                            execution.clone(),
                            1,
                            AttemptState::Scheduled,
                        ),
                    );
                    self.node_executions
                        .get_mut(execution)
                        .ok_or_else(|| invalid_at(event, "unknown node execution"))?
                        .attempts
                        .push(attempt.clone());
                    self.node_executions
                        .get_mut(execution)
                        .ok_or_else(|| invalid_at(event, "unknown node execution"))?
                        .attempt_count = 1;
                } else {
                    let projected_attempt = self
                        .attempts
                        .get(attempt)
                        .ok_or_else(|| invalid_at(event, "retry attempt was not reserved"))?;
                    if projected_attempt.execution != *execution
                        || projected_attempt.state != AttemptState::ReadyToSchedule
                        || execution_view.attempts.last() != Some(attempt)
                    {
                        return Err(invalid_at(
                            event,
                            "retry attempt is not ready for this execution",
                        ));
                    }
                    let previous_request = execution_view
                        .attempts
                        .iter()
                        .rev()
                        .nth(1)
                        .and_then(|previous| self.attempts.get(previous))
                        .and_then(NodeAttemptProjection::request)
                        .ok_or_else(|| {
                            invalid_at(event, "retry has no prior persisted invocation request")
                        })?;
                    if !same_logical_invocation_request(previous_request, request) {
                        return Err(invalid_at(
                            event,
                            "retry changed immutable capability, provider, input, or extension facts",
                        ));
                    }
                    let timer = self
                        .retry_by_attempt
                        .get(attempt)
                        .ok_or_else(|| invalid_at(event, "retry attempt has no retry decision"))?
                        .clone();
                    self.retries
                        .get_mut(&timer)
                        .ok_or_else(|| invalid_at(event, "retry decision is missing"))?
                        .state = RetryState::Scheduled;
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "attempt was not created"))?;
                attempt_view.invocation = Some(invocation.clone());
                attempt_view.idempotency_key = idempotency_key.clone();
                attempt_view.request = Some(request.clone());
                attempt_view.scheduled_sequence = Some(sequence);
                attempt_view.state = AttemptState::Scheduled;
                self.active_attempt_ids.insert(attempt.clone());
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown node execution"))?
                    .state = NodeExecutionState::Scheduled(attempt.clone());
                self.eligible_executions.remove(execution);
                self.invocations.insert(invocation.clone());
            }
            RunEventKind::CapabilityResolved {
                execution,
                attempt,
                requirement,
                snapshot,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let request = attempt_view.request.as_ref().ok_or_else(|| {
                    invalid_at(
                        event,
                        "capability resolution has no persisted invocation request",
                    )
                })?;
                let execution_attempts = self
                    .node_executions
                    .get(&attempt_view.execution)
                    .map(NodeExecutionProjection::attempts)
                    .ok_or_else(|| invalid_at(event, "attempt has no owning execution"))?;
                let attempt_position = execution_attempts
                    .iter()
                    .position(|candidate| candidate == attempt);
                let stable_retry_snapshot = attempt_position.is_some_and(|position| {
                    position.checked_sub(1).is_none_or(|previous_position| {
                        execution_attempts
                            .get(previous_position)
                            .and_then(|previous| self.attempts.get(previous))
                            .is_some_and(|previous| {
                                let requires_stable_snapshot = previous.state
                                    == AttemptState::Uncertain
                                    || previous.side_effect.as_ref().is_some_and(
                                        |classification| {
                                            classification.side_effect
                                                == SideEffectClass::IdempotentWrite
                                        },
                                    );
                                !requires_stable_snapshot
                                    || previous
                                        .capability
                                        .as_ref()
                                        .is_some_and(|capability| capability.snapshot == *snapshot)
                            })
                    })
                });
                if attempt_view.execution != *execution
                    || attempt_view.state != AttemptState::Scheduled
                    || attempt_view.capability.is_some()
                    || requirement.operation() != snapshot.operation()
                    || requirement
                        .exact_capability()
                        .is_some_and(|required| required != snapshot.capability())
                    || requirement
                        .provider_profile_ref()
                        .is_some_and(|required| snapshot.provider_profile() != Some(required))
                    || snapshot.operation_contract().side_effect()
                        > requirement.maximum_side_effect_class()
                    || request.capability() != snapshot.capability()
                    || request.operation() != snapshot.operation()
                    || request.provider_profile() != snapshot.provider_profile()
                    || !stable_retry_snapshot
                {
                    return Err(invalid_at(
                        event,
                        "capability resolution is duplicate or incompatible",
                    ));
                }
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .capability = Some(CapabilityResolution {
                    requirement: requirement.clone(),
                    snapshot: snapshot.clone(),
                });
            }
            RunEventKind::SideEffectClassified {
                attempt,
                side_effect,
                idempotency,
                idempotency_key,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let capability = attempt_view.capability.as_ref().ok_or_else(|| {
                    invalid_at(
                        event,
                        "side-effect classification precedes capability resolution",
                    )
                })?;
                let contract = capability.snapshot.operation_contract();
                let key_shape_valid = match idempotency {
                    IdempotencyBehavior::Unsupported => idempotency_key.is_none(),
                    IdempotencyBehavior::CapabilityScoped
                    | IdempotencyBehavior::ProviderProfileScoped => idempotency_key.is_some(),
                };
                let execution_attempts = self
                    .node_executions
                    .get(&attempt_view.execution)
                    .map(NodeExecutionProjection::attempts)
                    .ok_or_else(|| invalid_at(event, "attempt has no owning execution"))?;
                let attempt_position = execution_attempts
                    .iter()
                    .position(|candidate| candidate == attempt);
                let stable_retry_key = if *side_effect == SideEffectClass::IdempotentWrite {
                    *idempotency != IdempotencyBehavior::Unsupported
                        && idempotency_key.is_some()
                        && attempt_position.is_some_and(|position| {
                            execution_attempts[..position].iter().all(|prior| {
                                self.attempts
                                    .get(prior)
                                    .and_then(|attempt| attempt.side_effect.as_ref())
                                    .and_then(|classification| {
                                        classification.idempotency_key.as_ref()
                                    })
                                    == idempotency_key.as_ref()
                            })
                        })
                } else {
                    true
                };
                if attempt_view.state != AttemptState::Scheduled
                    || attempt_view.side_effect.is_some()
                    || contract.side_effect() != *side_effect
                    || contract.idempotency() != *idempotency
                    || attempt_view.idempotency_key.as_ref() != idempotency_key.as_ref()
                    || !key_shape_valid
                    || !stable_retry_key
                {
                    return Err(invalid_at(
                        event,
                        "side-effect classification contradicts frozen dispatch facts",
                    ));
                }
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .side_effect = Some(SideEffectClassification {
                    side_effect: *side_effect,
                    idempotency: *idempotency,
                    idempotency_key: idempotency_key.clone(),
                });
            }
            RunEventKind::LeaseGranted {
                lease,
                execution,
                attempt,
                worker,
                expires_at,
            } => {
                if self.leases.contains_key(lease) || *expires_at <= event.occurred_at() {
                    return Err(invalid_at(
                        event,
                        "lease identity is duplicate or expiration is not future",
                    ));
                }
                let attempt_view = self.attempt(attempt, event)?;
                if attempt_view.execution != *execution
                    || attempt_view.state != AttemptState::Scheduled
                    || attempt_view.capability.is_none()
                    || attempt_view.side_effect.is_none()
                    || self.active_lease_for_attempt(attempt).is_some()
                {
                    return Err(invalid_at(
                        event,
                        "lease grant is out of state or lacks dispatch facts",
                    ));
                }
                self.leases.insert(
                    lease.clone(),
                    LeaseProjection {
                        lease: lease.clone(),
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        worker: worker.clone(),
                        expires_at: *expires_at,
                        state: LeaseState::Active,
                    },
                );
                if self
                    .active_lease_by_attempt
                    .insert(attempt.clone(), lease.clone())
                    .is_some()
                {
                    return Err(invalid_at(
                        event,
                        "lease grant replaced an active attempt lease",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.leases.push(lease.clone());
                attempt_view.lease_workers.insert(worker.clone());
                attempt_view.state = AttemptState::Leased;
            }
            RunEventKind::LeaseHeartbeatRecorded { lease, expires_at } => {
                let lease_view = self
                    .leases
                    .get_mut(lease)
                    .ok_or_else(|| invalid_at(event, "heartbeat references an unknown lease"))?;
                if !lease_view.is_active()
                    || event.occurred_at() >= lease_view.expires_at
                    || *expires_at <= lease_view.expires_at
                    || *expires_at <= event.occurred_at()
                {
                    return Err(invalid_at(
                        event,
                        "heartbeat requires a still-valid active lease and later expiration",
                    ));
                }
                lease_view.expires_at = *expires_at;
            }
            RunEventKind::LeaseExpired {
                lease,
                classification,
            } => {
                let lease_view = self
                    .leases
                    .get(lease)
                    .ok_or_else(|| invalid_at(event, "expiry references an unknown lease"))?;
                let lease_attempt = lease_view.attempt.clone();
                let attempt_view = self.attempt(&lease_attempt, event)?;
                let retry_safe = attempt_view
                    .side_effect
                    .as_ref()
                    .is_some_and(|classification| {
                        matches!(
                            classification.side_effect,
                            SideEffectClass::None | SideEffectClass::ReadOnly
                        ) || (classification.side_effect == SideEffectClass::IdempotentWrite
                            && classification.idempotency != IdempotencyBehavior::Unsupported
                            && classification.idempotency_key.is_some())
                    });
                let classification_valid = match classification {
                    RecoveryClassification::NotStarted => {
                        attempt_view.state == AttemptState::Leased
                    }
                    RecoveryClassification::Retryable => {
                        retry_safe
                            && matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            )
                    }
                    RecoveryClassification::Uncertain => {
                        !retry_safe
                            && matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            )
                    }
                    RecoveryClassification::LeaseStillValid
                    | RecoveryClassification::TerminalObserved => false,
                };
                if !lease_view.is_active()
                    || event.occurred_at() < lease_view.expires_at
                    || !classification_valid
                {
                    return Err(invalid_at(
                        event,
                        "lease expiry is early, duplicate, or contradicts immutable attempt facts",
                    ));
                }
                self.leases
                    .get_mut(lease)
                    .ok_or_else(|| invalid_at(event, "unknown lease"))?
                    .state = LeaseState::Expired(*classification);
                if self.active_lease_by_attempt.remove(&lease_attempt).as_ref() != Some(lease) {
                    return Err(invalid_at(
                        event,
                        "lease expiry disagrees with the active attempt lease",
                    ));
                }
            }
            RunEventKind::NodeReLeased {
                previous_lease,
                lease,
                attempt,
                worker,
                expires_at,
            } => {
                if self.leases.contains_key(lease) || *expires_at <= event.occurred_at() {
                    return Err(invalid_at(
                        event,
                        "replacement lease is duplicate or already expired",
                    ));
                }
                let prior = self.leases.get(previous_lease).ok_or_else(|| {
                    invalid_at(event, "replacement references an unknown prior lease")
                })?;
                let classification = match prior.state {
                    LeaseState::Expired(classification) => classification,
                    LeaseState::Active | LeaseState::Superseded(_) | LeaseState::Completed => {
                        return Err(invalid_at(event, "only an expired lease may be superseded"));
                    }
                };
                let execution = prior.execution.clone();
                let attempt_view = self.attempt(attempt, event)?;
                let execution_view = self.execution(&execution, event)?;
                let retry_safe = attempt_view
                    .side_effect
                    .as_ref()
                    .is_some_and(|classification| {
                        matches!(
                            classification.side_effect,
                            SideEffectClass::None | SideEffectClass::ReadOnly
                        ) || (classification.side_effect == SideEffectClass::IdempotentWrite
                            && classification.idempotency != IdempotencyBehavior::Unsupported
                            && classification.idempotency_key.is_some())
                    });
                let state_is_releasable = match classification {
                    RecoveryClassification::NotStarted => {
                        attempt_view.state == AttemptState::Leased
                    }
                    RecoveryClassification::Retryable => {
                        retry_safe
                            && matches!(
                                attempt_view.state,
                                AttemptState::Leased | AttemptState::Running
                            )
                    }
                    RecoveryClassification::LeaseStillValid
                    | RecoveryClassification::Uncertain
                    | RecoveryClassification::TerminalObserved => false,
                };
                let exact_recovery = attempt_view.recovery.last().is_some_and(|observation| {
                    observation.lease.as_ref() == Some(previous_lease)
                        && observation.classification == classification
                });
                if prior.attempt != *attempt
                    || attempt_view.leases.last() != Some(previous_lease)
                    || execution_view.attempts.last() != Some(attempt)
                    || execution_view.cancellation.is_some()
                    || !matches!(
                        execution_view.state,
                        NodeExecutionState::Scheduled(ref active)
                            | NodeExecutionState::Running(ref active)
                            if active == attempt
                    )
                    || !state_is_releasable
                    || !exact_recovery
                    || self.active_lease_for_attempt(attempt).is_some()
                {
                    return Err(invalid_at(
                        event,
                        "attempt is not safely eligible for re-lease",
                    ));
                }
                self.leases
                    .get_mut(previous_lease)
                    .ok_or_else(|| invalid_at(event, "unknown prior lease"))?
                    .state = LeaseState::Superseded(lease.clone());
                self.leases.insert(
                    lease.clone(),
                    LeaseProjection {
                        lease: lease.clone(),
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        worker: worker.clone(),
                        expires_at: *expires_at,
                        state: LeaseState::Active,
                    },
                );
                if self
                    .active_lease_by_attempt
                    .insert(attempt.clone(), lease.clone())
                    .is_some()
                {
                    return Err(invalid_at(
                        event,
                        "replacement lease displaced an active attempt lease",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.leases.push(lease.clone());
                attempt_view.lease_workers.insert(worker.clone());
                attempt_view.state = AttemptState::Leased;
                self.node_executions
                    .get_mut(&execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::Scheduled(attempt.clone());
            }
            RunEventKind::NodeStarted {
                execution,
                attempt,
                invocation,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                if attempt_view.execution != *execution
                    || attempt_view.invocation.as_ref() != Some(invocation)
                    || attempt_view.state != AttemptState::Leased
                    || self.active_lease_for_attempt(attempt).is_none()
                {
                    return Err(invalid_at(
                        event,
                        "node start does not match a leased scheduled invocation",
                    ));
                }
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .state = AttemptState::Running;
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::Running(attempt.clone());
            }
            RunEventKind::NodeProgressRecorded {
                attempt,
                report_sequence,
                detail,
                completed_units,
                total_units,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                if attempt_view.state != AttemptState::Running
                    || !attempt_view.expects_report_sequence(*report_sequence)
                {
                    return Err(invalid_at(
                        event,
                        "progress is out of state or not the exact next report",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.progress.push(ProgressObservation {
                    report_sequence: *report_sequence,
                    detail: detail.clone(),
                    completed_units: *completed_units,
                    total_units: *total_units,
                });
                attempt_view.last_report_sequence = Some(*report_sequence);
            }
            RunEventKind::AttemptUsageRecorded { attempt, usage } => {
                let attempt_view = self.attempt(attempt, event)?;
                if !matches!(
                    attempt_view.state,
                    AttemptState::Running | AttemptState::Terminal(_)
                ) || attempt_view.usage.is_some()
                {
                    return Err(invalid_at(
                        event,
                        "attempt usage is duplicate or out of state",
                    ));
                }
                self.accumulate_usage(usage, event)?;
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .usage = Some(usage.clone());
            }
            RunEventKind::InvocationCancellationAcknowledged {
                attempt,
                acknowledgement,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let cancellation_matches = self
                    .node_executions
                    .get(&attempt_view.execution)
                    .and_then(|execution| execution.cancellation.as_ref())
                    .and_then(NodeExecutionCancellationProjection::attempt)
                    == Some(attempt);
                if !cancellation_matches
                    || attempt_view.invocation.as_ref() != Some(acknowledgement.invocation())
                    || attempt_view.is_completed()
                    || attempt_view
                        .cancellation_acknowledgements
                        .last()
                        .is_some_and(|prior| {
                            acknowledgement.request_sequence() <= prior.request_sequence()
                        })
                {
                    return Err(invalid_at(
                        event,
                        "cancellation acknowledgement is stale or mismatched",
                    ));
                }
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .cancellation_acknowledgements
                    .push(acknowledgement.clone());
            }
            RunEventKind::NodeOutputPublished {
                execution,
                attempt,
                report_sequence,
                value,
                artifact,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let execution_view = self.execution(execution, event)?;
                if attempt_view.execution != *execution
                    || attempt_view.state != AttemptState::Running
                    || !attempt_view.expects_report_sequence(*report_sequence)
                    || value.scope() != &execution_view.scope
                    || self.workspace_values.contains(value)
                    || attempt_view
                        .outputs
                        .iter()
                        .any(|output| output.value == *value)
                {
                    return Err(invalid_at(
                        event,
                        "node output is duplicate, out of scope, or out of state",
                    ));
                }
                self.validate_workspace_value(value, event)?;
                if let Some(artifact) = artifact {
                    self.validate_published_artifact(artifact, event)?;
                }
                let output = PublishedNodeOutput {
                    report_sequence: Some(*report_sequence),
                    value: value.clone(),
                    artifact: artifact.clone(),
                    sequence,
                };
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.outputs.push(output.clone());
                attempt_view.last_report_sequence = Some(*report_sequence);
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .outputs
                    .push(output);
                self.record_workspace_value(value, event)?;
            }
            RunEventKind::DeterministicOutputPublished {
                execution,
                value,
                artifact,
            } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.state != NodeExecutionState::Eligible
                    || execution_view.mode != NodeExecutionMode::Runtime
                    || !execution_view.attempts.is_empty()
                    || value.scope() != &execution_view.scope
                    || self.workspace_values.contains(value)
                    || execution_view
                        .outputs
                        .iter()
                        .any(|output| output.value == *value)
                {
                    return Err(invalid_at(
                        event,
                        "deterministic output is duplicate, out of scope, or follows completion",
                    ));
                }
                self.validate_workspace_value(value, event)?;
                if let Some(artifact) = artifact {
                    self.validate_published_artifact(artifact, event)?;
                }
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .outputs
                    .push(PublishedNodeOutput {
                        report_sequence: None,
                        value: value.clone(),
                        artifact: artifact.clone(),
                        sequence,
                    });
                self.record_workspace_value(value, event)?;
            }
            RunEventKind::DeterministicNodeTerminal {
                execution,
                outcome,
                error_class,
                detail,
            } => {
                let execution_view = self.execution(execution, event)?;
                let failure_shape = matches!(outcome, NodeOutcome::Failed | NodeOutcome::Rejected);
                if execution_view.state != NodeExecutionState::Eligible
                    || execution_view.mode != NodeExecutionMode::Runtime
                    || !execution_view.attempts.is_empty()
                    || execution_view.cancellation.is_some()
                    || execution_view.deterministic_terminal.is_some()
                    || *outcome == NodeOutcome::Cancelled
                    || failure_shape != error_class.is_some()
                {
                    return Err(invalid_at(
                        event,
                        "deterministic terminal fact requires an attempt-free eligible execution and a valid non-cancellation outcome",
                    ));
                }
                let terminal = DeterministicNodeTerminalProjection {
                    outcome: *outcome,
                    error_class: *error_class,
                    detail: detail.clone(),
                    sequence,
                };
                let execution_view = self
                    .node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?;
                execution_view.deterministic_terminal = Some(terminal);
                execution_view.state = NodeExecutionState::Terminal(*outcome);
                self.eligible_executions.remove(execution);
                self.deactivate_execution(execution, event)?;
                if *outcome == NodeOutcome::Succeeded {
                    self.pending_successor_executions.insert(execution.clone());
                }
            }
            RunEventKind::NodePreDispatchFailed {
                execution,
                error_class,
                detail,
            } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.state != NodeExecutionState::Eligible
                    || execution_view.mode != NodeExecutionMode::Executor
                    || !execution_view.attempts.is_empty()
                    || execution_view.cancellation.is_some()
                    || execution_view.deterministic_terminal.is_some()
                {
                    return Err(invalid_at(
                        event,
                        "pre-dispatch failure requires an attempt-free eligible executor execution",
                    ));
                }
                let terminal = DeterministicNodeTerminalProjection {
                    outcome: NodeOutcome::Failed,
                    error_class: Some(*error_class),
                    detail: detail.clone(),
                    sequence,
                };
                let execution_view = self
                    .node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?;
                execution_view.deterministic_terminal = Some(terminal);
                execution_view.state = NodeExecutionState::Terminal(NodeOutcome::Failed);
                self.eligible_executions.remove(execution);
                self.deactivate_execution(execution, event)?;
            }
            RunEventKind::StructuredSuccessorScanCompleted { execution } => {
                if self.node_executions.get(execution).is_none_or(|execution| {
                    execution.state != NodeExecutionState::Terminal(NodeOutcome::Succeeded)
                }) || !self.pending_successor_executions.remove(execution)
                {
                    return Err(invalid_at(
                        event,
                        "successor scan marker must consume one pending successful execution",
                    ));
                }
            }
            RunEventKind::NodeTerminal {
                execution,
                attempt,
                report_sequence,
                outcome,
                error_class,
                detail,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let failure_shape = matches!(outcome, NodeOutcome::Failed | NodeOutcome::Rejected);
                let cancellation_matches = self
                    .node_executions
                    .get(execution)
                    .and_then(|execution| execution.cancellation.as_ref())
                    .and_then(NodeExecutionCancellationProjection::attempt)
                    == Some(attempt);
                if attempt_view.execution != *execution
                    || !matches!(
                        attempt_view.state,
                        AttemptState::Leased | AttemptState::Running
                    )
                    || attempt_view.capability.is_none()
                    || attempt_view.side_effect.is_none()
                    || attempt_view.leases.is_empty()
                    || !attempt_view.expects_report_sequence(*report_sequence)
                    || failure_shape != error_class.is_some()
                    || (*outcome == NodeOutcome::Cancelled && !cancellation_matches)
                {
                    return Err(invalid_at(
                        event,
                        "node terminal fact is duplicate, mismatched, or malformed",
                    ));
                }
                let safely_covered_uncertain = {
                    let current_request = attempt_view.request.as_ref();
                    let current_capability = attempt_view.capability.as_ref();
                    let current_side_effect = attempt_view.side_effect.as_ref();
                    self.node_executions
                        .get(execution)
                        .into_iter()
                        .flat_map(|execution| execution.attempts.iter())
                        .take_while(|candidate| *candidate != attempt)
                        .filter_map(|candidate| {
                            let prior = self.attempts.get(candidate)?;
                            let prior_side_effect = prior.side_effect.as_ref()?;
                            let retry_safe = matches!(
                                prior_side_effect.side_effect,
                                SideEffectClass::None | SideEffectClass::ReadOnly
                            ) || (prior_side_effect.side_effect
                                == SideEffectClass::IdempotentWrite
                                && prior_side_effect.idempotency
                                    != IdempotencyBehavior::Unsupported
                                && prior_side_effect.idempotency_key.is_some());
                            let terminal_covers = *outcome == NodeOutcome::Succeeded
                                || matches!(
                                    prior_side_effect.side_effect,
                                    SideEffectClass::None | SideEffectClass::ReadOnly
                                );
                            (prior.state == AttemptState::Uncertain
                                && prior.obligation.is_some()
                                && retry_safe
                                && terminal_covers
                                && prior_side_effect == current_side_effect?
                                && prior.request.as_ref().zip(current_request).is_some_and(
                                    |(prior, current)| {
                                        same_logical_invocation_request(prior, current)
                                    },
                                )
                                && prior.idempotency_key == attempt_view.idempotency_key
                                && prior
                                    .capability
                                    .as_ref()
                                    .zip(current_capability)
                                    .is_some_and(|(prior, current)| {
                                        prior.snapshot == current.snapshot
                                    }))
                            .then(|| candidate.clone())
                        })
                        .collect::<Vec<_>>()
                };
                let terminal = AttemptTerminal {
                    report_sequence: *report_sequence,
                    outcome: *outcome,
                    error_class: *error_class,
                    detail: detail.clone(),
                    sequence,
                };
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.state = AttemptState::Terminal(*outcome);
                attempt_view.last_report_sequence = Some(*report_sequence);
                attempt_view.terminal = Some(terminal);
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::Terminal(*outcome);
                self.active_attempt_ids.remove(attempt);
                self.deactivate_execution(execution, event)?;
                if *outcome == NodeOutcome::Succeeded {
                    self.pending_successor_executions.insert(execution.clone());
                }
                self.complete_attempt_leases(attempt);
                for covered in safely_covered_uncertain {
                    let covered_view = self
                        .attempts
                        .get_mut(&covered)
                        .ok_or_else(|| invalid_at(event, "superseded attempt is missing"))?;
                    covered_view.state = if *outcome == NodeOutcome::Cancelled {
                        AttemptState::UncertainAbandonedByCancellation {
                            cancelled_retry: attempt.clone(),
                        }
                    } else {
                        AttemptState::UncertainSupersededByRetry {
                            covering_attempt: attempt.clone(),
                        }
                    };
                    self.active_attempt_ids.remove(&covered);
                    self.complete_attempt_leases(&covered);
                }
            }
            RunEventKind::NodeRetryScheduled {
                execution,
                previous_attempt,
                next_attempt,
                attempt_number,
                timer,
                fire_at,
                error_class,
                reason,
            } => {
                if self.attempts.contains_key(next_attempt)
                    || self.timers.contains_key(timer)
                    || *fire_at < event.occurred_at()
                {
                    return Err(invalid_at(
                        event,
                        "retry identities are duplicate or deadline is in the past",
                    ));
                }
                let previous = self.attempt(previous_attempt, event)?;
                let retry_safe = previous.side_effect.as_ref().is_some_and(|classification| {
                    matches!(
                        classification.side_effect,
                        SideEffectClass::None | SideEffectClass::ReadOnly
                    ) || (classification.side_effect == SideEffectClass::IdempotentWrite
                        && classification.idempotency != IdempotencyBehavior::Unsupported
                        && classification.idempotency_key.is_some())
                });
                let retryable_terminal = matches!(
                    previous.state,
                    AttemptState::Terminal(NodeOutcome::Failed | NodeOutcome::Rejected)
                ) && previous
                    .terminal
                    .as_ref()
                    .is_some_and(|terminal| terminal.error_class == Some(*error_class))
                    && retry_safe;
                let retryable_uncertain = previous.state == AttemptState::Uncertain
                    && previous.obligation.as_ref().is_some_and(|obligation| {
                        obligation.side_effect
                            == previous
                                .side_effect
                                .as_ref()
                                .map_or(SideEffectClass::Unknown, |facts| facts.side_effect)
                    })
                    && retry_safe;
                let authority_retry = previous.obligation.as_ref().is_some_and(|obligation| {
                    obligation
                        .decisions
                        .last()
                        .is_some_and(|decision| decision.outcome == AuthorityDecision::Retry)
                }) && retry_safe;
                let execution_view = self.execution(execution, event)?;
                let expected_number = execution_view.attempt_count.checked_add(1);
                if previous.execution != *execution
                    || execution_view.attempts.last() != Some(previous_attempt)
                    || expected_number != Some(*attempt_number)
                    || *attempt_number > crate::scheduler::MAX_RETRY_ATTEMPTS
                    || (!retryable_terminal && !retryable_uncertain && !authority_retry)
                {
                    return Err(invalid_at(
                        event,
                        "retry does not follow the latest retryable attempt",
                    ));
                }
                self.attempts.insert(
                    next_attempt.clone(),
                    new_attempt(
                        next_attempt.clone(),
                        execution.clone(),
                        *attempt_number,
                        AttemptState::AwaitingRetryTimer,
                    ),
                );
                self.active_attempt_ids.insert(next_attempt.clone());
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .attempts
                    .push(next_attempt.clone());
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .attempt_count = *attempt_number;
                self.node_executions
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::RetryPending(next_attempt.clone());
                if !self.active_execution_ids.contains(execution) {
                    self.activate_execution(execution, event)?;
                }
                self.timers.insert(
                    timer.clone(),
                    TimerProjection {
                        timer: timer.clone(),
                        purpose: TimerPurpose::Retry {
                            attempt: next_attempt.clone(),
                        },
                        fire_at: *fire_at,
                        state: TimerState::Pending,
                        cancellation: None,
                    },
                );
                self.pending_timer_ids.insert(timer.clone());
                self.pending_timers_by_execution
                    .entry(execution.clone())
                    .or_default()
                    .insert(timer.clone());
                self.retries.insert(
                    timer.clone(),
                    RetryProjection {
                        execution: execution.clone(),
                        previous_attempt: previous_attempt.clone(),
                        next_attempt: next_attempt.clone(),
                        attempt_number: *attempt_number,
                        timer: timer.clone(),
                        fire_at: *fire_at,
                        error_class: *error_class,
                        reason: reason.clone(),
                        state: RetryState::Waiting,
                    },
                );
                self.retry_by_attempt
                    .insert(next_attempt.clone(), timer.clone());
                if retryable_uncertain {
                    self.complete_attempt_leases(previous_attempt);
                }
            }
            RunEventKind::ExternalOutcomeUncertain {
                attempt,
                report_sequence,
                side_effect,
                reason,
                evidence,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let classified = attempt_view.side_effect.as_ref().ok_or_else(|| {
                    invalid_at(event, "uncertainty lacks frozen side-effect facts")
                })?;
                if !matches!(
                    attempt_view.state,
                    AttemptState::Leased | AttemptState::Running
                ) || attempt_view.obligation.is_some()
                    || !attempt_view.expects_report_sequence(*report_sequence)
                    || classified.side_effect != *side_effect
                {
                    return Err(invalid_at(
                        event,
                        "uncertain outcome is duplicate or contradicts dispatch facts",
                    ));
                }
                let execution = attempt_view.execution.clone();
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.state = AttemptState::Uncertain;
                attempt_view.last_report_sequence = Some(*report_sequence);
                attempt_view.obligation = Some(ExternalOutcomeObligation {
                    report_sequence: *report_sequence,
                    side_effect: *side_effect,
                    reason: reason.clone(),
                    evidence: evidence.clone(),
                    uncertain_sequence: sequence,
                    retained: None,
                    decisions: Vec::new(),
                });
                self.node_executions
                    .get_mut(&execution)
                    .ok_or_else(|| invalid_at(event, "unknown execution"))?
                    .state = NodeExecutionState::Uncertain(attempt.clone());
                self.complete_attempt_leases(attempt);
            }
            RunEventKind::LateTerminalEvidenceRecorded {
                attempt,
                worker,
                report_sequence,
                terminal,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let obligation = attempt_view.obligation.as_ref().ok_or_else(|| {
                    invalid_at(
                        event,
                        "late terminal evidence requires an uncertainty obligation",
                    )
                })?;
                let classified = attempt_view.side_effect.as_ref().ok_or_else(|| {
                    invalid_at(
                        event,
                        "late terminal evidence lacks side-effect classification",
                    )
                })?;
                let historically_owned = attempt_view.lease_workers.contains(worker);
                if attempt_view.terminal.is_some()
                    || attempt_view.late_terminal_evidence.is_some()
                    || *report_sequence < obligation.report_sequence
                    || terminal.side_effect() > classified.side_effect
                    || !historically_owned
                {
                    return Err(invalid_at(
                        event,
                        "late terminal evidence contradicts attempt ownership or existing terminal facts",
                    ));
                }
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .late_terminal_evidence = Some(LateTerminalEvidence {
                    report_sequence: *report_sequence,
                    terminal: terminal.clone(),
                    worker: worker.clone(),
                    sequence,
                });
            }
            RunEventKind::ExternalOutcomeRetained {
                attempt,
                decision,
                reason,
            } => {
                let decision_view = self.recovery_decisions.get(decision);
                if decision_view != Some(&(attempt.clone(), AuthorityDecision::Retain)) {
                    return Err(invalid_at(
                        event,
                        "retention lacks its prior matching authority decision",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "retention references an unknown attempt"))?;
                let obligation = attempt_view.obligation.as_mut().ok_or_else(|| {
                    invalid_at(event, "retention requires an uncertain obligation")
                })?;
                if obligation.retained.is_some() {
                    return Err(invalid_at(event, "external outcome was already retained"));
                }
                obligation.retained = Some(RetainedExternalOutcome {
                    decision: decision.clone(),
                    reason: reason.clone(),
                    sequence,
                });
                attempt_view.state = AttemptState::Retained;
                self.complete_attempt_leases(attempt);
            }
            RunEventKind::ArtifactPublished { metadata } => {
                self.apply_artifact_publication(metadata, event)?;
            }
            _ => self.apply_structured_kind(event)?,
        }
        Ok(())
    }
}
