//! Run creation/start and signal/timer command families.

use super::super::RuntimeService;
use super::super::support::{
    CommandPlan, entry_nodes, node_execution_mode, require_lifecycle, wait_signal_matches,
};
use crate::projection::{RunLifecycle, RunProjection, TimerPurpose};
use crate::{RunCommand, RunCommandDocument, RuntimeError};
use milkdrift_persistence::{
    MAX_WORKSPACE_MUTATIONS_PER_COMMIT, RunEventKind, SignalDeliveryMode, WaitSatisfaction,
    WorkspaceMutation,
};
use milkdrift_workspace::{ValueOrigin, ValueVersion};
use std::collections::BTreeSet;

impl RuntimeService {
    pub(super) fn plan_create_run(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
    ) -> Result<CommandPlan, RuntimeError> {
        let RunCommand::CreateRun {
            workflow,
            revision: revision_id,
            root_scope,
            workspace_budget: budget,
            inputs,
        } = document.command()
        else {
            return Err(RuntimeError::InvalidCommand(
                "create-run planner received another command family".to_owned(),
            ));
        };
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

    pub(super) fn plan_start_run(
        &self,
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

    pub(super) fn plan_signal(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
    ) -> Result<CommandPlan, RuntimeError> {
        let RunCommand::DeliverSignal {
            signal,
            signal_type,
            correlation,
            mode,
            payload,
        } = document.command()
        else {
            return Err(RuntimeError::InvalidCommand(
                "signal planner received another command family".to_owned(),
            ));
        };
        if !projection.lifecycle().is_active() {
            return Err(RuntimeError::InvalidTransition(
                "signals are accepted only for an active run".to_owned(),
            ));
        }
        if let Some(existing) = projection.signals().get(signal) {
            if existing.signal_type() != signal_type
                || existing.correlation() != correlation.as_ref()
                || existing.mode() != *mode
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
        if let Some(receipt) = self.store.signal_receipt(document.run_id(), signal)? {
            let RunEventKind::SignalReceived {
                signal: received,
                signal_type: received_type,
                correlation: received_correlation,
                mode: received_mode,
                payload: received_payload,
            } = receipt.kind()
            else {
                return Err(RuntimeError::InvalidHistory(
                    "signal receipt lookup returned a non-receipt event".to_owned(),
                ));
            };
            if received != signal
                || received_type != signal_type
                || received_correlation != correlation
                || received_mode != mode
                || received_payload != payload
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
        let retained_payload_bytes =
            projection
                .signals()
                .values()
                .try_fold(0_usize, |total, signal| {
                    let bytes = serde_json::to_vec(signal.payload())?;
                    total.checked_add(bytes.len()).ok_or_else(|| {
                        RuntimeError::InvalidTransition(
                            "pending signal payload byte count overflowed".to_owned(),
                        )
                    })
                })?;
        let payload_bytes = serde_json::to_vec(payload)?.len();
        if projection.signals().len() >= crate::projection::MAX_PENDING_SIGNAL_COUNT
            || retained_payload_bytes
                .checked_add(payload_bytes)
                .is_none_or(|bytes| bytes > crate::projection::MAX_PENDING_SIGNAL_PAYLOAD_BYTES)
        {
            return Err(RuntimeError::InvalidTransition(
                "pending signal count or aggregate payload-byte budget is exhausted".to_owned(),
            ));
        }
        let mut plan = CommandPlan::one(RunEventKind::SignalReceived {
            signal: signal.clone(),
            signal_type: signal_type.clone(),
            correlation: correlation.clone(),
            mode: *mode,
            payload: payload.clone(),
        });
        if *mode == SignalDeliveryMode::Broadcast || projection.lifecycle() == RunLifecycle::Paused
        {
            return Ok(plan);
        }
        let compatible = projection
            .waits()
            .values()
            .filter(|wait| {
                wait.is_pending()
                    && wait_signal_matches(wait.condition(), signal_type, correlation.as_ref())
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

    pub(super) fn plan_timer(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
    ) -> Result<CommandPlan, RuntimeError> {
        let RunCommand::FireTimer { timer } = document.command() else {
            return Err(RuntimeError::InvalidCommand(
                "timer planner received another command family".to_owned(),
            ));
        };
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
}
