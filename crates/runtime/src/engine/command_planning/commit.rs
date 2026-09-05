//! Atomic accepted/rejected command commit assembly.

use super::super::RuntimeService;
use super::super::support::{CommandPlan, collect_required_artifacts, event_kind_name};
use crate::projection::{RunLifecycle, RunProjection};
use crate::query::{RUN_PROJECTION_SNAPSHOT_SCHEMA_V4, encode_projection_snapshot};
use crate::{RunCommandDocument, RuntimeError};
use milkdrift_authority::AuthorityDecisionSnapshot;
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    AtomicRunCommitOutcome, AtomicRunCommitRequest, CommandDisposition, CommandReceipt,
    CommandResultDocument, ControllerAccountAction, ControllerAccountTransaction,
    ControllerAssessmentBoundary, ControllerTransitionId, ProjectionCheckpoint, RunEventEnvelope,
    RunEventKind, RunIndexUpdate, WorkspaceAccounting,
};
use serde_json::json;
use tracing::{debug, warn};

impl RuntimeService {
    pub(in crate::engine) fn commit_accepted(
        &self,
        document: &RunCommandDocument,
        receipt: CommandReceipt,
        projection: RunProjection,
        mut plan: CommandPlan,
        authorization: Option<AuthorityDecisionSnapshot>,
    ) -> Result<AtomicRunCommitOutcome, RuntimeError> {
        if plan.events.is_empty() {
            return Err(RuntimeError::InvalidTransition(
                "an accepted transition must emit at least one event".to_owned(),
            ));
        }
        let (mut candidate, mut envelopes) =
            self.project_planned_events(document, &projection, &mut plan)?;
        if candidate.revision().is_some() && candidate.lifecycle().is_active() {
            let revision = self.current_revision(&candidate)?;
            self.extend_structured_progress(
                document.run_id(),
                document.issued_at(),
                &revision,
                &mut candidate,
                &mut envelopes,
                &mut plan.workspace,
            )?;
        }
        self.extend_controller_actions(document, &envelopes, &mut plan)?;

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
        let result = match authorization {
            Some(decision) => CommandResultDocument::new_authorized(
                document.command_id().clone(),
                document.run_id().clone(),
                receipt.fingerprint().clone(),
                CommandDisposition::Accepted,
                resulting_sequence,
                event_ids,
                result_payload,
                decision,
            )?,
            None => CommandResultDocument::new(
                document.command_id().clone(),
                document.run_id().clone(),
                receipt.fingerprint().clone(),
                CommandDisposition::Accepted,
                resulting_sequence,
                event_ids,
                result_payload,
            )?,
        };

        let required_artifacts = collect_required_artifacts(&envelopes, &plan.workspace)?;
        self.validate_planned_artifacts(&required_artifacts, &plan)?;
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
        let should_checkpoint =
            self.should_checkpoint_projection(projection.sequence(), &candidate);
        let projection_payload = should_checkpoint
            .then(|| encode_projection_snapshot(&candidate))
            .transpose()?;
        let projection_checkpoint = projection_payload
            .as_deref()
            .map(|payload| ProjectionCheckpoint::new(RUN_PROJECTION_SNAPSHOT_SCHEMA_V4, payload))
            .transpose()?;
        let mut request = AtomicRunCommitRequest::new(
            receipt,
            envelopes,
            plan.workspace,
            Some(accounting),
            required_artifacts.into_iter().collect(),
            newly_referenced_artifacts.into_iter().collect(),
            plan.expected_lease_revision,
            result,
            indexes,
        )?;
        if !plan.controller_actions.is_empty() {
            request =
                request.with_controller_account_transaction(ControllerAccountTransaction::new(
                    ControllerTransitionId::new(format!(
                        "controller-transition:{}",
                        document.command_id()
                    ))?,
                    plan.expected_controller_revision,
                    plan.controller_actions,
                )?)?;
        }
        if let Some(checkpoint) = projection_checkpoint {
            request = request.with_projection_checkpoint(checkpoint)?;
        }
        let outcome = self.store.commit_command(&request)?;
        if should_checkpoint
            && matches!(&outcome, AtomicRunCommitOutcome::Committed(_))
            && let Some(payload) = projection_payload
            && let Err(error) =
                self.persist_projection_snapshot(document.run_id(), &candidate, payload)
        {
            warn!(
                run = %document.run_id(),
                sequence = candidate.sequence().get(),
                reason = %error,
                "optional projection checkpoint could not be persisted"
            );
        }
        if matches!(candidate.lifecycle(), RunLifecycle::Terminal(_)) {
            self.clear_run_scan_cursors(document.run_id());
        }
        Ok(outcome)
    }

    fn project_planned_events(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        plan: &mut CommandPlan,
    ) -> Result<(RunProjection, Vec<RunEventEnvelope>), RuntimeError> {
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
        Ok((candidate, envelopes))
    }

    fn extend_controller_actions(
        &self,
        document: &RunCommandDocument,
        envelopes: &[RunEventEnvelope],
        plan: &mut CommandPlan,
    ) -> Result<(), RuntimeError> {
        let mut bound_account = self.store.controller_account_binding(document.run_id())?;
        let mut child_runs = Vec::new();
        for event in envelopes {
            match event.kind() {
                RunEventKind::ControllerAssessmentRecorded {
                    boundary,
                    account_declaration: Some(declaration),
                    ..
                } => match bound_account.as_ref() {
                    Some(account) if account == declaration.account() => {
                        let state = self.store.controller_account(account)?.ok_or_else(|| {
                            RuntimeError::InvalidHistory(format!(
                                "controller binding references missing account {account}"
                            ))
                        })?;
                        if state.declaration() != declaration {
                            return Err(RuntimeError::InvalidHistory(
                                "controller assessment declaration differs from the durable account"
                                    .to_owned(),
                            ));
                        }
                    }
                    Some(_) => {
                        return Err(RuntimeError::InvalidHistory(
                            "nested or conflicting controller account is unsupported".to_owned(),
                        ));
                    }
                    None if *boundary == ControllerAssessmentBoundary::Activation => {
                        plan.controller_actions
                            .push(ControllerAccountAction::Establish {
                                declaration: declaration.clone(),
                                bind_run: document.run_id().clone(),
                            });
                        bound_account = Some(declaration.account().clone());
                    }
                    None => {
                        return Err(RuntimeError::InvalidHistory(
                            "non-activation controller assessment has no durable account binding"
                                .to_owned(),
                        ));
                    }
                },
                RunEventKind::ControllerAssessmentRecorded {
                    account_declaration: None,
                    ..
                } => {
                    return Err(RuntimeError::InvalidHistory(
                        "current controller assessment omitted its account declaration".to_owned(),
                    ));
                }
                RunEventKind::SubworkflowCreated { child_run, .. } => {
                    child_runs.push(child_run.clone());
                }
                _ => {}
            }
        }
        if let Some(account) = bound_account {
            plan.controller_actions
                .extend(
                    child_runs
                        .into_iter()
                        .map(|run| ControllerAccountAction::BindRun {
                            account: account.clone(),
                            run,
                        }),
                );
        }
        Ok(())
    }

    fn validate_planned_artifacts(
        &self,
        required_artifacts: &std::collections::BTreeSet<milkdrift_workspace::ArtifactReference>,
        plan: &CommandPlan,
    ) -> Result<(), RuntimeError> {
        for artifact in required_artifacts {
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
        Ok(())
    }

    pub(in crate::engine) fn commit_rejected(
        &self,
        document: &RunCommandDocument,
        receipt: CommandReceipt,
        detail: &str,
        authorization: Option<AuthorityDecisionSnapshot>,
    ) -> Result<AtomicRunCommitOutcome, RuntimeError> {
        let payload = BoundedJson::new(json!({
            "status": "rejected",
            "reason": detail,
        }))
        .map_err(|error| RuntimeError::InvalidCommand(error.to_string()))?;
        let result = match authorization {
            Some(decision) => CommandResultDocument::new_authorized(
                document.command_id().clone(),
                document.run_id().clone(),
                receipt.fingerprint().clone(),
                CommandDisposition::Rejected,
                document.expected_sequence(),
                Vec::new(),
                payload,
                decision,
            )?,
            None => CommandResultDocument::new(
                document.command_id().clone(),
                document.run_id().clone(),
                receipt.fingerprint().clone(),
                CommandDisposition::Rejected,
                document.expected_sequence(),
                Vec::new(),
                payload,
            )?,
        };
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
