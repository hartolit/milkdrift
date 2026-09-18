//! Bounded-memory historical journal reconstruction into the canonical attempt read shape.

use milkdrift_blueprint::RevisionId;
use milkdrift_control_protocol::{ArtifactMetadataRead, AttemptOutputRead};
use milkdrift_persistence::{
    AttemptId, EventCursor, EventPage, EventPageQuery, NodeExecutionId, PageSize,
    ReconciliationPlanId, RunEventEnvelope, RunEventKind, RunQueryStore, RunSequence, TimerId,
};
use milkdrift_workspace::RunId;

use super::{LocatedAttempt, Owner};
use crate::host::{
    PublicFailure, corruption, empty_attempt_read, invalid, not_found, public_attempt_usage,
    public_authority_decision, public_capability_provenance, public_execution_authority,
    public_invocation_artifact, public_operation_contract, public_persistence, snake_debug,
};

#[cfg(test)]
mod tests;

struct HistoricalAttemptState {
    attempt: AttemptId,
    current_revision: Option<RevisionId>,
    execution_authority: Option<milkdrift_control_protocol::ExecutionAuthorityRead>,
    execution: NodeExecutionId,
    owner: Option<(String, String)>,
    remediation: Option<(ReconciliationPlanId, String)>,
    retry_timer: Option<TimerId>,
    located: Option<LocatedAttempt>,
}

impl HistoricalAttemptState {
    fn new(attempt: AttemptId, execution: NodeExecutionId) -> Self {
        Self {
            attempt,
            current_revision: None,
            execution_authority: None,
            execution,
            owner: None,
            remediation: None,
            retry_timer: None,
            located: None,
        }
    }

    #[expect(
        clippy::too_many_lines,
        clippy::cognitive_complexity,
        reason = "this exhaustive ordered reducer owns one historical attempt meaning across every relevant event family"
    )]
    fn fold(&mut self, event: &RunEventEnvelope) -> Result<(), PublicFailure> {
        let event_sequence = event.sequence().get();
        match event.kind() {
            RunEventKind::ExecutionAuthorityEstablished { basis } => {
                self.execution_authority = Some(public_execution_authority(basis));
            }
            RunEventKind::RunCreated { revision, .. } => {
                self.current_revision = Some(revision.clone());
            }
            RunEventKind::RevisionPinned { revision, plan, .. } => {
                // Reconciliation creates remediation before pinning its target revision.
                // Bind only that plan's pin; later adoptions cannot change this owner.
                if self
                    .remediation
                    .as_ref()
                    .is_some_and(|(pending, _)| pending == plan)
                    && let Some((_, node)) = self.remediation.take()
                {
                    self.owner = Some((node, revision.as_str().to_owned()));
                }
                self.current_revision = Some(revision.clone());
            }
            RunEventKind::NodeBecameEligible {
                node, execution, ..
            }
            | RunEventKind::RemediationWorkCreated {
                node, execution, ..
            } if execution == &self.execution => {
                if let Some(revision) = self.current_revision.as_ref() {
                    self.owner = Some((node.as_str().to_owned(), revision.as_str().to_owned()));
                }
            }
            RunEventKind::ReconciliationRemediationCreated {
                plan,
                node,
                execution,
                ..
            } if execution == &self.execution => {
                self.remediation = Some((plan.clone(), node.as_str().to_owned()));
            }
            RunEventKind::NodeRetryScheduled {
                execution,
                next_attempt,
                timer,
                ..
            } if next_attempt == &self.attempt && execution == &self.execution => {
                let (node_id, revision_id) = self
                    .owner
                    .clone()
                    .ok_or_else(|| corruption("retry attempt has no owning execution"))?;
                self.retry_timer = Some(timer.clone());
                self.located = Some(LocatedAttempt {
                    node_id,
                    revision_id,
                    value: empty_attempt_read(self.attempt.as_str(), "awaiting_retry_timer"),
                });
            }
            RunEventKind::TimerFired { timer, .. } if self.retry_timer.as_ref() == Some(timer) => {
                if let Some(located) = self.located.as_mut() {
                    located.value.state = "ready_to_schedule".to_owned();
                }
            }
            RunEventKind::NodeScheduled {
                node,
                execution,
                attempt,
                invocation,
                request,
                ..
            } if attempt == &self.attempt && execution == &self.execution => {
                let revision_id = self
                    .owner
                    .as_ref()
                    .map(|(_, revision)| revision.clone())
                    .ok_or_else(|| corruption("scheduled attempt has no owning execution"))?;
                let mut value = empty_attempt_read(self.attempt.as_str(), "scheduled");
                value.execution_authority = self.execution_authority.clone();
                value.invocation_id = Some(invocation.as_str().to_owned());
                value.context_manifest = request.context_manifest().map(public_invocation_artifact);
                value.context_access = if value.context_manifest.is_some() {
                    "metadata_only".to_owned()
                } else {
                    "absent".to_owned()
                };
                self.located = Some(LocatedAttempt {
                    node_id: node.as_str().to_owned(),
                    revision_id,
                    value,
                });
            }
            RunEventKind::CapabilityResolutionDecisionRecorded {
                attempt,
                authorization,
                ..
            } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.resolution_authorization =
                        Some(public_authority_decision(authorization));
                    located.value.peer_id = authorization
                        .request()
                        .resources
                        .peer
                        .as_ref()
                        .map(|peer| peer.as_str().to_owned())
                        .or_else(|| {
                            authorization
                                .request()
                                .provenance
                                .peer
                                .as_ref()
                                .map(|peer| peer.as_str().to_owned())
                        });
                }
            }
            RunEventKind::CapabilityEntryDecisionRecorded {
                attempt,
                authorization,
            } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.claim_authorization =
                        Some(public_authority_decision(authorization));
                }
            }
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt,
                authorization,
                controller_admission: _,
            } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.entry_authorization =
                        Some(public_authority_decision(authorization));
                }
            }
            RunEventKind::CapabilityResolved {
                attempt,
                snapshot,
                requirement,
                ..
            } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.requirement = Some(
                        serde_json::to_value(requirement)
                            .map_err(|_| super::super::read_model::internal())?,
                    );
                    located.value.capability_id = Some(snapshot.capability().as_str().to_owned());
                    located.value.descriptor_revision = Some(snapshot.descriptor_revision());
                    located.value.peer_id = snapshot.peer().map(|peer| peer.as_str().to_owned());
                    located.value.capability_provenance =
                        Some(public_capability_provenance(snapshot));
                    located.value.operation_contract = Some(public_operation_contract(
                        snapshot.operation(),
                        snapshot.operation_contract(),
                    ));
                    located.value.provider_profile = snapshot
                        .provider_profile()
                        .map(|profile| profile.as_str().to_owned());
                }
            }
            RunEventKind::SideEffectClassified {
                attempt,
                idempotency_key,
                ..
            } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.idempotency_key_present = idempotency_key.is_some();
                }
            }
            RunEventKind::LeaseGranted { attempt, .. } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.state = "leased".to_owned();
                }
            }
            RunEventKind::NodeStarted { attempt, .. } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.state = "running".to_owned();
                }
            }
            RunEventKind::NodeProgressRecorded {
                attempt, detail, ..
            } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.progress_observations =
                        located.value.progress_observations.saturating_add(1);
                    located.value.progress_bytes = located
                        .value
                        .progress_bytes
                        .saturating_add(u64::try_from(detail.as_str().len()).unwrap_or(u64::MAX));
                }
            }
            RunEventKind::AttemptUsageRecorded { attempt, usage } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.usage = Some(public_attempt_usage(usage));
                }
            }
            RunEventKind::NodeOutputPublished {
                attempt,
                report_sequence,
                value,
                artifact: Some(artifact),
                ..
            } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.outputs.push(AttemptOutputRead {
                        name: value.key().as_str().to_owned(),
                        report_sequence: Some(*report_sequence),
                        publication_sequence: event_sequence,
                        artifact: ArtifactMetadataRead {
                            artifact_id: artifact.artifact().as_str().to_owned(),
                            digest: artifact.digest().to_hex(),
                            size: artifact.size_bytes(),
                            content_type: artifact.media_type().as_str().to_owned(),
                            disposition_name: None,
                            sensitivity: "restricted".to_owned(),
                        },
                    });
                }
            }
            RunEventKind::NodeTerminal {
                attempt,
                outcome,
                detail,
                ..
            } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.state = "terminal".to_owned();
                    located.value.terminal = Some(snake_debug(outcome));
                    located.value.terminal_detail =
                        detail.as_ref().map(|detail| detail.as_str().to_owned());
                }
            }
            RunEventKind::ExternalOutcomeUncertain { attempt, .. } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.state = "uncertain".to_owned();
                    located.value.uncertain = true;
                }
            }
            RunEventKind::ExternalOutcomeRetained { attempt, .. } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.state = "retained".to_owned();
                    located.value.uncertain = true;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<LocatedAttempt, PublicFailure> {
        self.located.ok_or_else(not_found)
    }
}

impl Owner {
    pub(super) fn historical_attempt_read(
        &self,
        run: &str,
        attempt: &str,
    ) -> Result<LocatedAttempt, PublicFailure> {
        let run = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let attempt =
            AttemptId::new(attempt.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let projection = self.workflow()?.runtime.projection(&run).map_err(|error| {
            crate::host::read_model::public_control(milkdrift_control::ControlError::Runtime(error))
        })?;
        // These are verified operational anchors, not a second historical index. Settled
        // occurrences retain their original revision and exact terminal sequence.
        let current = projection
            .node_executions()
            .values()
            .find(|execution| execution.attempts().contains(&attempt))
            .map(|value| {
                (
                    value.execution(),
                    value.revision(),
                    value.created_sequence(),
                    projection.sequence(),
                )
            })
            .or_else(|| {
                projection
                    .settled_node_executions()
                    .values()
                    .find(|execution| execution.latest_attempt() == Some(&attempt))
                    .map(|value| {
                        (
                            value.execution(),
                            value.revision(),
                            value.created_sequence(),
                            value.terminal_sequence().unwrap_or(projection.sequence()),
                        )
                    })
            });
        let (mut state, start, end) = if let Some((execution, revision, start, end)) = current {
            let mut state = HistoricalAttemptState::new(attempt, execution.clone());
            state.current_revision = Some(revision.clone());
            state.execution_authority = projection
                .execution_authority()
                .map(public_execution_authority);
            (state, start, end)
        } else {
            // A retired occurrence has no retained anchor. First locate its identity,
            // then fold only that occurrence. Two bounded passes avoid retaining every
            // execution merely to recover one old attempt's original revision.
            let mut execution = None;
            scan_history(
                |query| self.store.events(query).map_err(public_persistence),
                &run,
                RunSequence::new(1),
                projection.sequence(),
                |event| {
                    match event.kind() {
                        RunEventKind::NodeScheduled {
                            attempt: id,
                            execution: owner,
                            ..
                        }
                        | RunEventKind::NodeRetryScheduled {
                            next_attempt: id,
                            execution: owner,
                            ..
                        } if id == &attempt => execution = Some(owner.clone()),
                        _ => {}
                    }
                    Ok(execution.is_some())
                },
            )?;
            (
                HistoricalAttemptState::new(attempt, execution.ok_or_else(not_found)?),
                RunSequence::new(1),
                projection.sequence(),
            )
        };
        scan_history(
            |query| self.store.events(query).map_err(public_persistence),
            &run,
            start,
            end,
            |event| {
                state.fold(event)?;
                Ok(false)
            },
        )?;
        state.finish()
    }
}

/// Visit a fixed journal prefix in bounded pages. The caller may stop after finding an identity.
fn scan_history(
    mut read_page: impl FnMut(&EventPageQuery) -> Result<EventPage, PublicFailure>,
    run: &RunId,
    start: RunSequence,
    end: RunSequence,
    mut visit: impl FnMut(&RunEventEnvelope) -> Result<bool, PublicFailure>,
) -> Result<(), PublicFailure> {
    let mut sequence = start;
    while sequence <= end {
        let limit = u32::try_from((end.get() - sequence.get() + 1).min(256))
            .map_err(|_| super::super::read_model::internal())?;
        let query = EventPageQuery::new(
            run.clone(),
            Some(EventCursor {
                run: run.clone(),
                next_sequence: sequence,
            }),
            PageSize::new(limit).map_err(public_persistence)?,
        )
        .map_err(public_persistence)?;
        let page = read_page(&query)?;
        if page.events.is_empty() {
            return Err(corruption(
                "historical attempt journal prefix is incomplete",
            ));
        }
        for event in &page.events {
            if event.sequence() != sequence {
                return Err(corruption(
                    "historical attempt journal prefix is discontinuous",
                ));
            }
            if visit(event)? || sequence == end {
                return Ok(());
            }
            sequence = sequence.next().map_err(public_persistence)?;
        }
    }
    Ok(())
}
