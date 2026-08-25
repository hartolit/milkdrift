use std::collections::BTreeSet;

use milkdrift_persistence::{
    MAX_PAGE_SIZE, RunEventEnvelope, RunEventKind, SignalDeliveryMode, WaitCondition,
};

use crate::RuntimeError;

use super::helpers::{invalid_at, wait_condition_timer, wait_signal_projection_matches};
use super::node::{NodeExecutionProjection, TimerProjection, TimerPurpose};
use super::run::RunProjection;
use super::structured::{MAX_PENDING_SIGNAL_COUNT, MAX_PENDING_SIGNAL_PAYLOAD_BYTES};
use super::structured::{SignalProjection, WaitCancellationProjection, WaitProjection};

impl RunProjection {
    pub(super) fn apply_wait_signal_kind(
        &mut self,
        event: &RunEventEnvelope,
    ) -> Result<(), RuntimeError> {
        let sequence = event.sequence();
        match event.kind() {
            RunEventKind::WaitRegistered {
                execution,
                condition,
            } => {
                let execution_view = self.execution(execution, event)?;
                if execution_view.is_completed() || self.waits.contains_key(execution) {
                    return Err(invalid_at(
                        event,
                        "wait is duplicate or follows terminal execution",
                    ));
                }
                if let Some(timer) = wait_condition_timer(condition) {
                    let timer_view = self
                        .timers
                        .get(timer)
                        .ok_or_else(|| invalid_at(event, "wait references an unknown timer"))?;
                    if !matches!(
                        &timer_view.purpose,
                        TimerPurpose::Wait { execution: Some(owner) } if owner == execution
                    ) {
                        return Err(invalid_at(event, "wait timer belongs to another execution"));
                    }
                }
                self.waits.insert(
                    execution.clone(),
                    WaitProjection {
                        execution: execution.clone(),
                        condition: condition.clone(),
                        registered_sequence: sequence,
                        satisfaction: None,
                        cancellation: None,
                    },
                );
                self.pending_wait_execution_ids.insert(execution.clone());
            }
            RunEventKind::WaitSatisfied { execution, cause } => {
                let wait = self
                    .waits
                    .get(execution)
                    .ok_or_else(|| invalid_at(event, "satisfaction references an unknown wait"))?;
                if !wait.is_pending() || !self.wait_cause_matches(wait, cause) {
                    return Err(invalid_at(
                        event,
                        "wait cause is duplicate, incompatible, or not yet durable",
                    ));
                }
                self.waits
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown wait"))?
                    .satisfaction = Some(cause.clone());
                self.pending_wait_execution_ids.remove(execution);
            }
            RunEventKind::WaitCancelled { execution, reason } => {
                let wait = self.waits.get(execution).ok_or_else(|| {
                    invalid_at(event, "wait cancellation references an unknown wait")
                })?;
                let authorized = self
                    .node_executions
                    .get(execution)
                    .and_then(NodeExecutionProjection::cancellation)
                    .is_some()
                    || self.has_execution_cancellation_source(execution);
                if !wait.is_pending() || !authorized {
                    return Err(invalid_at(
                        event,
                        "wait cancellation requires a pending wait and structured owner cancellation",
                    ));
                }
                self.waits
                    .get_mut(execution)
                    .ok_or_else(|| invalid_at(event, "unknown wait"))?
                    .cancellation = Some(WaitCancellationProjection {
                    reason: reason.clone(),
                    sequence,
                });
                self.pending_wait_execution_ids.remove(execution);
            }
            RunEventKind::SignalReceived {
                signal,
                signal_type,
                correlation,
                mode,
                payload,
            } => {
                if self.signals.contains_key(signal) {
                    return Err(invalid_at(event, "signal identity was already received"));
                }
                let retained_count = self.signals.len();
                let retained_payload_bytes =
                    self.signals.values().try_fold(0_usize, |total, signal| {
                        let bytes = serde_json::to_vec(signal.payload()).map_err(|_| {
                            invalid_at(event, "pending signal payload could not be serialized")
                        })?;
                        total.checked_add(bytes.len()).ok_or_else(|| {
                            invalid_at(event, "pending signal payload byte count overflowed")
                        })
                    })?;
                let payload_bytes = serde_json::to_vec(payload)
                    .map_err(|_| invalid_at(event, "signal payload could not be serialized"))?
                    .len();
                if retained_count >= MAX_PENDING_SIGNAL_COUNT
                    || retained_payload_bytes
                        .checked_add(payload_bytes)
                        .is_none_or(|bytes| bytes > MAX_PENDING_SIGNAL_PAYLOAD_BYTES)
                {
                    return Err(invalid_at(
                        event,
                        "pending signal count or aggregate payload-byte budget is exhausted",
                    ));
                }
                self.signals.insert(
                    signal.clone(),
                    SignalProjection {
                        signal: signal.clone(),
                        signal_type: signal_type.clone(),
                        correlation: correlation.clone(),
                        mode: *mode,
                        payload: payload.clone(),
                        received_sequence: sequence,
                        consumed_by: BTreeSet::new(),
                        broadcast_scan_through: None,
                        broadcast_scan_complete: false,
                        duplicate_commands: Vec::new(),
                    },
                );
                if *mode == SignalDeliveryMode::Broadcast {
                    self.pending_broadcast_signals
                        .insert((sequence, signal.clone()));
                }
            }
            RunEventKind::SignalBroadcastScanAdvanced {
                signal,
                through_execution,
                complete,
            } => {
                let signal_view = self.signals.get(signal).ok_or_else(|| {
                    invalid_at(event, "broadcast scan references an unknown signal")
                })?;
                if signal_view.mode != SignalDeliveryMode::Broadcast
                    || signal_view.broadcast_scan_complete
                {
                    return Err(invalid_at(
                        event,
                        "broadcast scan requires an incomplete broadcast signal",
                    ));
                }
                let previous = signal_view.broadcast_scan_through.as_ref();
                let cursor_valid = match (previous, through_execution.as_ref()) {
                    (None, None) => *complete,
                    (None, Some(next)) => self.waits.contains_key(next),
                    (Some(_), None) => false,
                    (Some(previous), Some(next)) => {
                        self.waits.contains_key(next)
                            && (next > previous || (*complete && next == previous))
                    }
                };
                if !cursor_valid {
                    return Err(invalid_at(
                        event,
                        "broadcast scan cursor did not advance monotonically through known waits",
                    ));
                }
                let lower = previous.map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
                let upper = if *complete {
                    std::ops::Bound::Unbounded
                } else {
                    std::ops::Bound::Included(through_execution.as_ref().ok_or_else(|| {
                        invalid_at(event, "incomplete broadcast scan has no cursor")
                    })?)
                };
                let mut scanned = 0_u32;
                for (_, wait) in self.waits.range((lower, upper)) {
                    scanned = scanned.saturating_add(1);
                    if scanned > MAX_PAGE_SIZE {
                        return Err(invalid_at(
                            event,
                            "one broadcast scan event exceeds the durable wait-page bound",
                        ));
                    }
                    let eligible = wait.is_pending()
                        && wait.registered_sequence() < signal_view.received_sequence
                        && !signal_view.consumed_by.contains(wait.execution())
                        && wait_signal_projection_matches(
                            wait.condition(),
                            &signal_view.signal_type,
                            signal_view.correlation.as_ref(),
                            &self.timers,
                        );
                    if eligible {
                        return Err(invalid_at(
                            event,
                            "broadcast scan cannot advance past an eligible unconsumed wait",
                        ));
                    }
                }
                let signal_view = self.signals.get_mut(signal).ok_or_else(|| {
                    invalid_at(event, "broadcast scan references an unknown signal")
                })?;
                signal_view.broadcast_scan_through = through_execution.clone();
                signal_view.broadcast_scan_complete = *complete;
                if *complete {
                    self.pending_broadcast_signals
                        .remove(&(signal_view.received_sequence, signal.clone()));
                }
            }
            RunEventKind::SignalDeduplicated {
                signal,
                duplicate_command,
            } => {
                if self
                    .signals
                    .values()
                    .any(|received| received.duplicate_commands.contains(duplicate_command))
                {
                    return Err(invalid_at(
                        event,
                        "duplicate signal command identity was already recorded",
                    ));
                }
                if let Some(signal_view) = self.signals.get_mut(signal) {
                    signal_view
                        .duplicate_commands
                        .push(duplicate_command.clone());
                }
            }
            RunEventKind::SignalConsumed { signal, execution } => {
                let execution_view = self.execution(execution, event)?;
                let wait_view = self
                    .waits
                    .get(execution)
                    .ok_or_else(|| invalid_at(event, "signal consumer has no registered wait"))?;
                let signal_view = self
                    .signals
                    .get(signal)
                    .ok_or_else(|| invalid_at(event, "consumption references an unknown signal"))?;
                let compatible_wait = match wait_view.condition() {
                    WaitCondition::Signal {
                        signal_type,
                        correlation,
                    } => {
                        signal_view.signal_type == *signal_type
                            && signal_view.correlation == *correlation
                    }
                    WaitCondition::SignalOrTimer {
                        timer,
                        signal_type,
                        correlation,
                    } => {
                        signal_view.signal_type == *signal_type
                            && signal_view.correlation == *correlation
                            && self
                                .timers
                                .get(timer)
                                .is_some_and(TimerProjection::is_pending)
                    }
                    WaitCondition::Timer { .. } => false,
                };
                if execution_view.is_completed()
                    || wait_view.is_completed()
                    || !compatible_wait
                    || signal_view.consumed_by.contains(execution)
                    || (signal_view.mode == SignalDeliveryMode::OneShot
                        && !signal_view.consumed_by.is_empty())
                    || (signal_view.mode == SignalDeliveryMode::Broadcast
                        && wait_view.registered_sequence >= signal_view.received_sequence)
                {
                    return Err(invalid_at(
                        event,
                        "signal consumption is duplicate, incompatible, or violates delivery mode",
                    ));
                }
                self.signals
                    .get_mut(signal)
                    .ok_or_else(|| invalid_at(event, "unknown signal"))?
                    .consumed_by
                    .insert(execution.clone());
            }
            _ => unreachable!("structured dispatch owns wait and signal correlation routing"),
        }
        Ok(())
    }
}
