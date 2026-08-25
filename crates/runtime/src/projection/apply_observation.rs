use milkdrift_persistence::{NodeExecutionMode, RunEventEnvelope, RunEventKind};

use crate::RuntimeError;

use super::helpers::invalid_at;
use super::node::{
    AttemptState, NodeExecutionCancellationProjection, NodeExecutionState, ProgressObservation,
    PublishedNodeOutput,
};
use super::run::RunProjection;

impl RunProjection {
    pub(super) fn apply_observation_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::NodeStarted {
                execution,
                attempt,
                invocation,
            } => {
                let attempt_view = self.attempt(attempt, event)?;
                let execution_view = self.execution(execution, event)?;
                let started_worker = self
                    .active_lease_for_attempt(attempt)
                    .filter(|lease| {
                        lease.execution() == execution && lease.expires_at() > event.occurred_at()
                    })
                    .map(|lease| lease.worker().clone());
                if attempt_view.execution != *execution
                    || attempt_view.invocation.as_ref() != Some(invocation)
                    || attempt_view.state != AttemptState::Leased
                    || execution_view.state != NodeExecutionState::Scheduled(attempt.clone())
                    || execution_view.cancellation.is_some()
                    || self.has_execution_cancellation_source(execution)
                    || started_worker.is_none()
                {
                    return Err(invalid_at(
                        event,
                        "node start does not match a leased scheduled invocation",
                    ));
                }
                let attempt_view = self
                    .attempts
                    .get_mut(attempt)
                    .ok_or_else(|| invalid_at(event, "unknown attempt"))?;
                attempt_view.state = AttemptState::Running;
                attempt_view.lease_workers.clear();
                attempt_view.lease_workers.insert(
                    started_worker
                        .ok_or_else(|| invalid_at(event, "active lease worker disappeared"))?,
                );
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
            _ => unreachable!(
                "central projection dispatch owns executor observations and outputs routing"
            ),
        }
        Ok(())
    }
}
