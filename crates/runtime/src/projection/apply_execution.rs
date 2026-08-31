use milkdrift_capability::{CapabilityCategory, IdempotencyBehavior, SideEffectClass};
use milkdrift_persistence::{NodeExecutionMode, RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::helpers::{invalid_at, new_attempt, same_logical_invocation_request};
use super::node::{
    AttemptState, CapabilityResolution, NodeAttemptProjection, NodeExecutionProjection,
    NodeExecutionState, RetryState, SideEffectClassification,
};
use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_execution_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
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
            RunEventKind::CapabilityResolutionDecisionRecorded {
                execution,
                attempt,
                snapshot,
                authorization,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let basis = self.execution_authority.as_ref().ok_or_else(|| {
                    invalid_at(event, "capability resolution has no run authority basis")
                })?;
                if attempt_view.execution != *execution
                    || attempt_view.state != AttemptState::Scheduled
                    || attempt_view.capability.is_some()
                    || attempt_view.resolution_authorization.is_some()
                    || !authorization.is_allowed()
                    || authorization.policy() != basis.policy()
                    || authorization.policy_version() != basis.policy_version()
                    || authorization.request().actor != *basis.actor()
                    || authorization.request().grant != *basis.grant()
                    || authorization.request().grant_revision != basis.grant_revision()
                    || authorization.request().grant_digest != *basis.grant_digest()
                    || authorization.request().revocation_generation
                        != basis.revocation_generation()
                    || authorization.request().resources.capability.as_ref()
                        != Some(snapshot.capability())
                    || authorization
                        .request()
                        .resources
                        .capability_operation
                        .as_ref()
                        != Some(snapshot.operation())
                    || authorization.request().provenance.attempt.as_deref()
                        != Some(attempt.as_str())
                {
                    return Err(invalid_at(
                        event,
                        "capability resolution decision is duplicate or incompatible",
                    ));
                }
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .resolution_authorization = Some(authorization.clone());
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
                let resolution_authorization = attempt_view.resolution_authorization.clone();
                self.attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?
                    .capability = Some(CapabilityResolution {
                    requirement: requirement.clone(),
                    snapshot: snapshot.clone(),
                    authorization: resolution_authorization,
                });
                let (count_process, count_model, controller_metered) = match snapshot.category() {
                    Some(CapabilityCategory::Model) => (false, true, true),
                    Some(CapabilityCategory::Process) => (true, false, true),
                    Some(
                        CapabilityCategory::Tool
                        | CapabilityCategory::Human
                        | CapabilityCategory::Peer
                        | CapabilityCategory::Custom(_),
                    ) => (false, false, false),
                    // Schema-v1 snapshots written before exact category freezing retain
                    // their original digest. Count both resource-bearing categories so
                    // controller replay cannot turn missing historical classification
                    // into a bypass.
                    None => (true, true, true),
                };
                if count_process {
                    self.resource_usage.process_invocations = self
                        .resource_usage
                        .process_invocations
                        .checked_add(1)
                        .ok_or_else(|| {
                            invalid_at(event, "controller process invocation accounting overflow")
                        })?;
                }
                if count_model {
                    self.resource_usage.model_invocations = self
                        .resource_usage
                        .model_invocations
                        .checked_add(1)
                        .ok_or_else(|| {
                            invalid_at(event, "controller model invocation accounting overflow")
                        })?;
                }
                if controller_metered {
                    self.resource_usage.unknown_input_usage = self
                        .resource_usage
                        .unknown_input_usage
                        .checked_add(1)
                        .ok_or_else(|| invalid_at(event, "unknown input accounting overflow"))?;
                    self.resource_usage.unknown_output_usage = self
                        .resource_usage
                        .unknown_output_usage
                        .checked_add(1)
                        .ok_or_else(|| invalid_at(event, "unknown output accounting overflow"))?;
                    self.resource_usage.unknown_cost_usage = self
                        .resource_usage
                        .unknown_cost_usage
                        .checked_add(1)
                        .ok_or_else(|| invalid_at(event, "unknown cost accounting overflow"))?;
                }
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
            _ => unreachable!("central projection dispatch owns dispatch contract routing"),
        }
        Ok(())
    }
}
