use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::RevisionId;
use milkdrift_capability::InvocationRequest;
use milkdrift_persistence::{
    AttemptId, AttemptUsage, CorrelationKey, NodeExecutionId, RunEventEnvelope, RunSequence,
    SignalTypeId, TimerId, WaitCondition, WaitSatisfaction,
};
use milkdrift_workspace::{
    ArtifactMetadata, ArtifactReference, CausalReference, ScopeReference, WorkspaceScope,
    WorkspaceValueReference,
};

use crate::RuntimeError;

use super::node::{
    AttemptState, LeaseState, NodeAttemptProjection, NodeExecutionProjection, TimerProjection,
    TimerPurpose,
};
use super::run::RunProjection;
use super::structured::{BranchProjection, WaitProjection};

impl RunProjection {
    pub(super) fn execution<'a>(
        &'a self,
        execution: &NodeExecutionId,
        event: &RunEventEnvelope,
    ) -> Result<&'a NodeExecutionProjection, RuntimeError> {
        self.node_executions
            .get(execution)
            .ok_or_else(|| invalid_at(event, format!("unknown node execution '{execution}'")))
    }

    pub(super) fn attempt<'a>(
        &'a self,
        attempt: &AttemptId,
        event: &RunEventEnvelope,
    ) -> Result<&'a NodeAttemptProjection, RuntimeError> {
        self.attempts
            .get(attempt)
            .ok_or_else(|| invalid_at(event, format!("unknown attempt '{attempt}'")))
    }

    pub(super) fn validate_scope_reference(
        &self,
        scope: &ScopeReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        if scope.run() != event.run_id() || !self.scopes.contains_key(scope) {
            return Err(invalid_at(
                event,
                "workspace scope is unknown or belongs to another run",
            ));
        }
        Ok(())
    }

    pub(super) fn register_child_scope(
        &mut self,
        scope: &WorkspaceScope,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        if scope.reference().run() != event.run_id()
            || scope.kind().is_run_root()
            || scope
                .parent()
                .is_none_or(|parent| !self.scopes.contains_key(parent))
            || self.scopes.contains_key(scope.reference())
        {
            return Err(invalid_at(
                event,
                "child scope is duplicate, parentless, or has an unknown parent",
            ));
        }
        self.scopes.insert(scope.reference().clone(), scope.clone());
        Ok(())
    }

    pub(crate) fn scope_descends_from(
        &self,
        scope: &ScopeReference,
        ancestor: &ScopeReference,
    ) -> bool {
        let mut current = Some(scope);
        let mut remaining = self.scopes.len().saturating_add(1);
        while let Some(reference) = current {
            if reference == ancestor {
                return true;
            }
            if remaining == 0 {
                return false;
            }
            remaining -= 1;
            current = self.scopes.get(reference).and_then(WorkspaceScope::parent);
        }
        false
    }

    pub(super) fn validate_workspace_value(
        &self,
        value: &WorkspaceValueReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        self.validate_scope_reference(value.scope(), event)
    }

    pub(super) fn validate_known_workspace_value(
        &self,
        value: &WorkspaceValueReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        self.validate_workspace_value(value, event)?;
        if !self.workspace_values.contains(value) {
            return Err(invalid_at(
                event,
                "workspace value reference was not introduced by prior run history",
            ));
        }
        Ok(())
    }

    pub(super) fn record_workspace_value(
        &mut self,
        value: &WorkspaceValueReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        self.validate_workspace_value(value, event)?;
        if self.workspace_values.insert(value.clone()) {
            let next = self
                .resource_usage
                .workspace_value_references
                .checked_add(1)
                .ok_or_else(|| invalid_at(event, "workspace value-reference count overflow"))?;
            if self
                .workspace_budget
                .as_ref()
                .is_some_and(|budget| next > budget.max_value_versions())
            {
                return Err(invalid_at(
                    event,
                    "workspace value references exceed the pinned budget",
                ));
            }
            self.resource_usage.workspace_value_references = next;
        }
        Ok(())
    }

    pub(super) fn validate_published_artifact(
        &self,
        artifact: &ArtifactReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let metadata = self
            .artifacts
            .get(artifact.artifact())
            .ok_or_else(|| invalid_at(event, "artifact reference precedes publication metadata"))?;
        if metadata.reference() != artifact {
            return Err(invalid_at(
                event,
                "artifact reference contradicts published metadata",
            ));
        }
        Ok(())
    }

    pub(super) fn apply_artifact_publication(
        &mut self,
        metadata: &ArtifactMetadata,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let reference = metadata.reference();
        if self.artifacts.contains_key(reference.artifact()) {
            return Err(invalid_at(event, "artifact identity was already published"));
        }
        self.validate_causal_reference(metadata.provenance().producer(), event)?;
        for cause in metadata.provenance().causes() {
            self.validate_causal_reference(cause, event)?;
        }
        let budget = self
            .workspace_budget
            .as_ref()
            .ok_or_else(|| invalid_at(event, "artifact publication precedes run creation"))?;
        let artifacts = self
            .resource_usage
            .artifacts
            .checked_add(1)
            .ok_or_else(|| invalid_at(event, "artifact count overflow"))?;
        let artifact_bytes = self
            .resource_usage
            .artifact_bytes
            .checked_add(reference.size_bytes())
            .ok_or_else(|| invalid_at(event, "artifact byte accounting overflow"))?;
        if reference.size_bytes() > budget.max_bytes_per_artifact()
            || artifacts > budget.max_artifacts()
            || artifact_bytes > budget.max_total_artifact_bytes()
        {
            return Err(invalid_at(
                event,
                "artifact publication exceeds the pinned workspace budget",
            ));
        }
        self.resource_usage.artifacts = artifacts;
        self.resource_usage.artifact_bytes = artifact_bytes;
        self.artifacts
            .insert(reference.artifact().clone(), metadata.clone());
        Ok(())
    }

    pub(super) fn validate_causal_reference(
        &self,
        reference: &CausalReference,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        match reference {
            CausalReference::RunInput { run, .. } if run == event.run_id() => Ok(()),
            CausalReference::WorkspaceValue { reference } => {
                self.validate_known_workspace_value(reference, event)
            }
            CausalReference::Artifact { reference } => {
                self.validate_published_artifact(reference, event)
            }
            CausalReference::Invocation { invocation } if self.invocations.contains(invocation) => {
                Ok(())
            }
            CausalReference::External { .. } => Ok(()),
            CausalReference::RunInput { .. } | CausalReference::Invocation { .. } => {
                Err(invalid_at(
                    event,
                    "artifact provenance references an unknown or foreign fact",
                ))
            }
        }
    }

    pub(super) fn has_execution_cancellation_source(&self, execution: &NodeExecutionId) -> bool {
        self.cancellation.is_some()
            || self.termination.is_some()
            || self
                .branch_owner
                .get(execution)
                .and_then(|branch| self.branches.get(branch))
                .and_then(BranchProjection::cancellation_reason)
                .is_some()
            || self.reconciliation_cancellations.contains_key(execution)
            || self.subworkflows.values().any(|child| {
                child.parent_execution == *execution && child.cancellation_reason.is_some()
            })
    }

    pub(super) fn ensure_terminal_quiescent(
        &self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let outstanding = self.has_active_owned_work();
        if outstanding {
            return Err(invalid_at(
                event,
                "run terminal boundary would abandon active owned work or an unresolved obligation",
            ));
        }
        Ok(())
    }

    pub(super) fn adjust_scope_ownership(
        &mut self,
        scope: &ScopeReference,
        add: bool,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let mut current = Some(scope.clone());
        while let Some(reference) = current {
            let parent = self
                .scopes
                .get(&reference)
                .and_then(WorkspaceScope::parent)
                .cloned();
            if add {
                let count = self
                    .active_scope_ownership
                    .entry(reference.clone())
                    .or_default();
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| invalid_at(event, "active scope ownership overflow"))?;
            } else {
                let count = self
                    .active_scope_ownership
                    .get_mut(&reference)
                    .ok_or_else(|| invalid_at(event, "active scope ownership underflow"))?;
                *count = count
                    .checked_sub(1)
                    .ok_or_else(|| invalid_at(event, "active scope ownership underflow"))?;
                if *count == 0 {
                    self.active_scope_ownership.remove(&reference);
                }
            }
            current = parent;
        }
        Ok(())
    }

    pub(super) fn activate_execution(
        &mut self,
        execution: &NodeExecutionId,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let scope = self.execution(execution, event)?.scope.clone();
        if !self.active_execution_ids.insert(execution.clone()) {
            return Err(invalid_at(event, "execution was already active"));
        }
        let execution_view = self.execution(execution, event)?;
        let node = execution_view.node.clone();
        let mut current = Some(scope.clone());
        while let Some(reference) = current {
            let parent = self
                .scopes
                .get(&reference)
                .and_then(WorkspaceScope::parent)
                .cloned();
            self.latest_descendant_execution_by_scope_node
                .insert((reference, node.clone()), execution.clone());
            current = parent;
        }
        self.adjust_scope_ownership(&scope, true, event)
    }

    pub(super) fn deactivate_execution(
        &mut self,
        execution: &NodeExecutionId,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let scope = self.execution(execution, event)?.scope.clone();
        if !self.active_execution_ids.remove(execution) {
            return Err(invalid_at(event, "execution was not active"));
        }
        self.adjust_scope_ownership(&scope, false, event)
    }

    pub(super) fn adjust_structured_child_count(
        &mut self,
        execution: &NodeExecutionId,
        add: bool,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        if add {
            let count = self
                .active_structured_children_by_execution
                .entry(execution.clone())
                .or_default();
            *count = count
                .checked_add(1)
                .ok_or_else(|| invalid_at(event, "structured child ownership overflow"))?;
        } else {
            let count = self
                .active_structured_children_by_execution
                .get_mut(execution)
                .ok_or_else(|| invalid_at(event, "structured child ownership underflow"))?;
            *count = count
                .checked_sub(1)
                .ok_or_else(|| invalid_at(event, "structured child ownership underflow"))?;
            if *count == 0 {
                self.active_structured_children_by_execution
                    .remove(execution);
            }
        }
        Ok(())
    }

    pub(super) fn remove_pending_timer_owner(
        &mut self,
        timer: &TimerId,
        purpose: &TimerPurpose,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let owner = match purpose {
            TimerPurpose::Wait {
                execution: Some(execution),
            } => Some(execution.clone()),
            TimerPurpose::Retry { attempt } => self
                .attempts
                .get(attempt)
                .map(|attempt| attempt.execution.clone()),
            TimerPurpose::Wait { execution: None } => None,
        };
        if let Some(owner) = owner {
            let timers = self
                .pending_timers_by_execution
                .get_mut(&owner)
                .ok_or_else(|| invalid_at(event, "pending timer owner index is absent"))?;
            if !timers.remove(timer) {
                return Err(invalid_at(event, "pending timer owner index disagrees"));
            }
            if timers.is_empty() {
                self.pending_timers_by_execution.remove(&owner);
            }
        }
        Ok(())
    }

    pub(super) fn complete_attempt_leases(&mut self, attempt: &AttemptId) {
        if let Some(lease) = self.active_lease_by_attempt.remove(attempt)
            && let Some(lease) = self.leases.get_mut(&lease)
        {
            lease.state = LeaseState::Completed;
        }
    }

    pub(super) fn accumulate_usage(
        &mut self,
        usage: &AttemptUsage,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        add_optional_usage(
            &mut self.resource_usage.input_units,
            usage.input_units,
            event,
            "input units",
        )?;
        add_optional_usage(
            &mut self.resource_usage.output_units,
            usage.output_units,
            event,
            "output units",
        )?;
        add_optional_usage(
            &mut self.resource_usage.duration_ms,
            usage.duration_ms,
            event,
            "duration",
        )?;
        if let Some(cost) = &usage.cost {
            let total = self
                .resource_usage
                .cost_micros
                .entry(cost.currency.clone())
                .or_default();
            *total = total
                .checked_add(cost.micros)
                .ok_or_else(|| invalid_at(event, "monetary usage overflow"))?;
        }
        Ok(())
    }

    /// Returns the exact immutable revision governing a durable event sequence.
    ///
    /// The value is absent before run creation or beyond the projected journal
    /// head. Attempt provenance uses its recorded scheduling sequence here rather
    /// than assuming the run's current prospective pin.
    #[must_use]
    pub fn revision_at(&self, sequence: RunSequence) -> Option<&RevisionId> {
        if sequence == RunSequence::ZERO || sequence > self.sequence {
            return None;
        }
        self.pins
            .iter()
            .rev()
            .find(|pin| pin.effective_sequence <= sequence)
            .map(|pin| &pin.revision)
    }

    pub(super) fn wait_cause_matches(
        &self,
        wait: &WaitProjection,
        cause: &WaitSatisfaction,
    ) -> bool {
        match (wait.condition(), cause) {
            (WaitCondition::Timer { timer: expected }, WaitSatisfaction::Timer { timer })
            | (
                WaitCondition::SignalOrTimer {
                    timer: expected, ..
                },
                WaitSatisfaction::Timer { timer },
            ) if expected == timer => {
                self.timers
                    .get(timer)
                    .is_some_and(TimerProjection::is_completed)
                    && !self
                        .signals
                        .values()
                        .any(|signal| signal.consumed_by.contains(&wait.execution))
            }
            (
                WaitCondition::Signal {
                    signal_type,
                    correlation,
                }
                | WaitCondition::SignalOrTimer {
                    signal_type,
                    correlation,
                    ..
                },
                WaitSatisfaction::Signal { signal },
            ) => self.signals.get(signal).is_some_and(|received| {
                received.signal_type == *signal_type
                    && received.correlation == *correlation
                    && received.consumed_by.contains(&wait.execution)
            }),
            _ => false,
        }
    }
}

pub(super) fn new_attempt(
    attempt: AttemptId,
    execution: NodeExecutionId,
    attempt_number: u32,
    state: AttemptState,
) -> NodeAttemptProjection {
    NodeAttemptProjection {
        attempt,
        execution,
        attempt_number,
        invocation: None,
        idempotency_key: None,
        request: None,
        scheduled_sequence: None,
        state,
        capability: None,
        side_effect: None,
        leases: Vec::new(),
        progress: Vec::new(),
        last_report_sequence: None,
        usage: None,
        cancellation_acknowledgements: Vec::new(),
        outputs: Vec::new(),
        terminal: None,
        obligation: None,
        recovery: Vec::new(),
    }
}

pub(super) fn same_logical_invocation_request(
    left: &InvocationRequest,
    right: &InvocationRequest,
) -> bool {
    left.capability() == right.capability()
        && left.operation() == right.operation()
        && left.provider_profile() == right.provider_profile()
        && left.inputs() == right.inputs()
        && left.extensions() == right.extensions()
}

pub(super) fn wait_condition_timer(condition: &WaitCondition) -> Option<&TimerId> {
    match condition {
        WaitCondition::Timer { timer } | WaitCondition::SignalOrTimer { timer, .. } => Some(timer),
        WaitCondition::Signal { .. } => None,
    }
}

pub(super) fn wait_signal_projection_matches(
    condition: &WaitCondition,
    signal_type: &SignalTypeId,
    correlation: Option<&CorrelationKey>,
    timers: &BTreeMap<TimerId, TimerProjection>,
) -> bool {
    match condition {
        WaitCondition::Signal {
            signal_type: expected,
            correlation: expected_correlation,
        } => expected == signal_type && expected_correlation.as_ref() == correlation,
        WaitCondition::SignalOrTimer {
            timer,
            signal_type: expected,
            correlation: expected_correlation,
        } => {
            expected == signal_type
                && expected_correlation.as_ref() == correlation
                && timers.get(timer).is_some_and(TimerProjection::is_pending)
        }
        WaitCondition::Timer { .. } => false,
    }
}

pub(super) fn ensure_unique<T: Ord>(
    items: &[T],
    event: &RunEventEnvelope,
    kind: &str,
) -> Result<(), RuntimeError> {
    let mut unique = BTreeSet::new();
    if items.iter().all(|item| unique.insert(item)) {
        Ok(())
    } else {
        Err(invalid_at(event, format!("duplicate {kind}")))
    }
}

pub(super) fn ensure_unique_by<T, K: Ord, F: Fn(&T) -> K>(
    items: &[T],
    key: F,
    event: &RunEventEnvelope,
    kind: &str,
) -> Result<(), RuntimeError> {
    let mut unique = BTreeSet::new();
    if items.iter().all(|item| unique.insert(key(item))) {
        Ok(())
    } else {
        Err(invalid_at(event, format!("duplicate {kind}")))
    }
}

pub(super) fn add_optional_usage(
    total: &mut Option<u64>,
    observation: Option<u64>,
    event: &RunEventEnvelope,
    resource: &str,
) -> Result<(), RuntimeError> {
    if let Some(observation) = observation {
        *total = Some(
            total
                .unwrap_or(0)
                .checked_add(observation)
                .ok_or_else(|| invalid_at(event, format!("{resource} usage overflow")))?,
        );
    }
    Ok(())
}

pub(super) fn invalid(reason: impl Into<String>) -> RuntimeError {
    RuntimeError::InvalidHistory(reason.into())
}

pub(super) fn invalid_at(event: &RunEventEnvelope, reason: impl AsRef<str>) -> RuntimeError {
    invalid(format!(
        "event {} at sequence {}: {}",
        event.event_id(),
        event.sequence(),
        reason.as_ref()
    ))
}
