//! Bounded broadcast-signal materialization and scan advancement.

use super::super::support::wait_signal_matches;
use super::super::transition::PlanTransition;
use super::super::{RuntimeService, STRUCTURED_EVENT_SOFT_LIMIT};
use crate::RuntimeError;
use crate::projection::RunProjection;
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    MAX_WORKSPACE_MUTATIONS_PER_COMMIT, NodeExecutionId, RunEventKind, WaitSatisfaction,
    WorkspaceMutation,
};
use milkdrift_workspace::{ValueKey, WorkspaceValue, WorkspaceValueEntry};

impl RuntimeService {
    pub(in crate::engine) fn signal_payload_entries(
        &self,
        projection: &RunProjection,
        execution: &NodeExecutionId,
        payload: &BoundedJson,
        pending_workspace: &[WorkspaceMutation],
    ) -> Result<Vec<WorkspaceValueEntry>, RuntimeError> {
        let execution_view = projection.node_executions().get(execution).ok_or_else(|| {
            RuntimeError::InvalidHistory("signal wait execution is absent".to_owned())
        })?;
        let revision = self.revision_for_execution(projection, execution)?;
        let node = revision
            .semantic()
            .nodes()
            .get(execution_view.node())
            .ok_or_else(|| RuntimeError::InvalidHistory("signal wait node is absent".to_owned()))?;
        let mut entries = Vec::with_capacity(node.data_outputs().len());
        for port in node.data_outputs().keys() {
            let key = ValueKey::new(port.as_str().to_owned())
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
            entries.push(self.projected_output_entry(
                projection,
                execution_view.scope(),
                key,
                WorkspaceValue::Json(payload.clone()),
                pending_workspace,
            )?);
        }
        Ok(entries)
    }

    pub(in crate::engine) fn drain_broadcast_signals(
        &self,
        transition: &mut PlanTransition<'_>,
    ) -> Result<(), RuntimeError> {
        if !transition.has_event_capacity(1, STRUCTURED_EVENT_SOFT_LIMIT) {
            return Ok(());
        }
        let Some((_, signal)) = transition
            .projection()
            .pending_broadcast_signals()
            .iter()
            .next()
            .cloned()
        else {
            return Ok(());
        };
        let signal_view = transition
            .projection()
            .signals()
            .get(&signal)
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "pending broadcast scan references an absent signal".to_owned(),
                )
            })?;
        let signal_type = signal_view.signal_type().clone();
        let correlation = signal_view.correlation().cloned();
        let received_sequence = signal_view.received_sequence();
        let payload = signal_view.payload().clone();
        let original_cursor = signal_view.broadcast_scan_through().cloned();
        let mut through = original_cursor.clone();
        let mut exhausted = false;
        let scan_limit = usize::from(self.config.maximum_tick_items);
        let mut scanned = 0_usize;

        while scanned < scan_limit {
            let lower = through
                .as_ref()
                .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
            let next_wait = transition
                .projection()
                .waits()
                .range((lower, std::ops::Bound::Unbounded))
                .next()
                .map(|(_, wait)| {
                    (
                        wait.execution().clone(),
                        wait.registered_sequence(),
                        wait.condition().clone(),
                        wait.is_pending(),
                    )
                });
            let Some((execution, registered_sequence, condition, pending)) = next_wait else {
                exhausted = true;
                break;
            };
            let consumed = transition
                .projection()
                .signals()
                .get(&signal)
                .is_some_and(|received| received.consumed_by().contains(&execution));
            let eligible = pending
                && registered_sequence < received_sequence
                && !consumed
                && wait_signal_matches(&condition, &signal_type, correlation.as_ref());
            if eligible {
                let entries = self.signal_payload_entries(
                    transition.projection(),
                    &execution,
                    &payload,
                    transition.workspace(),
                )?;
                let event_cost = entries.len().checked_add(2).ok_or_else(|| {
                    RuntimeError::Scheduling("broadcast signal event cost overflow".to_owned())
                })?;
                if event_cost.saturating_add(1) > STRUCTURED_EVENT_SOFT_LIMIT
                    || entries.len() > MAX_WORKSPACE_MUTATIONS_PER_COMMIT
                {
                    return Err(RuntimeError::InvalidHistory(
                        "one broadcast signal consumer exceeds atomic runtime bounds".to_owned(),
                    ));
                }
                if !transition
                    .has_event_capacity(event_cost.saturating_add(1), STRUCTURED_EVENT_SOFT_LIMIT)
                    || transition.workspace().len().saturating_add(entries.len())
                        > MAX_WORKSPACE_MUTATIONS_PER_COMMIT
                {
                    break;
                }
                transition.push_event(RunEventKind::SignalConsumed {
                    signal: signal.clone(),
                    execution: execution.clone(),
                })?;
                for entry in entries {
                    let value = entry.reference().clone();
                    transition.push_workspace(WorkspaceMutation::PutValue { entry })?;
                    transition.push_event(RunEventKind::DeterministicOutputPublished {
                        execution: execution.clone(),
                        value,
                        artifact: None,
                    })?;
                }
                transition.push_event(RunEventKind::WaitSatisfied {
                    execution: execution.clone(),
                    cause: WaitSatisfaction::Signal {
                        signal: signal.clone(),
                    },
                })?;
            }
            through = Some(execution);
            scanned = scanned.saturating_add(1);
        }
        if !exhausted && scanned == scan_limit {
            let lower = through
                .as_ref()
                .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
            exhausted = transition
                .projection()
                .waits()
                .range((lower, std::ops::Bound::Unbounded))
                .next()
                .is_none();
        }
        if through != original_cursor || exhausted {
            transition.push_event(RunEventKind::SignalBroadcastScanAdvanced {
                signal,
                through_execution: through,
                complete: exhausted,
            })?;
        }
        Ok(())
    }
}
