//! Bounded-memory historical journal reconstruction into the canonical attempt read shape.

use std::collections::BTreeMap;

use milkdrift_blueprint::RevisionId;
use milkdrift_control_protocol::{ArtifactMetadataRead, AttemptOutputRead};
use milkdrift_persistence::{
    AttemptId, EventPageQuery, NodeExecutionId, PageSize, RunEventEnvelope, RunEventKind,
    RunQueryStore, TimerId,
};
use milkdrift_workspace::RunId;

use super::{LocatedAttempt, Owner};
use crate::host::{
    PublicFailure, corruption, empty_attempt_read, invalid, not_found, public_attempt_usage,
    public_authority_decision, public_capability_provenance, public_execution_authority,
    public_invocation_artifact, public_operation_contract, public_persistence, snake_debug,
};

struct HistoricalAttemptState {
    attempt: AttemptId,
    current_revision: Option<RevisionId>,
    execution_authority: Option<milkdrift_control_protocol::ExecutionAuthorityRead>,
    executions: BTreeMap<NodeExecutionId, (String, String)>,
    retry_timer: Option<TimerId>,
    located: Option<LocatedAttempt>,
}

impl HistoricalAttemptState {
    fn new(attempt: AttemptId) -> Self {
        Self {
            attempt,
            current_revision: None,
            execution_authority: None,
            executions: BTreeMap::new(),
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
            RunEventKind::RunCreated { revision, .. }
            | RunEventKind::RevisionPinned { revision, .. } => {
                self.current_revision = Some(revision.clone());
            }
            RunEventKind::NodeBecameEligible {
                node, execution, ..
            } => {
                if let Some(revision) = self.current_revision.as_ref() {
                    self.executions.insert(
                        execution.clone(),
                        (node.as_str().to_owned(), revision.as_str().to_owned()),
                    );
                }
            }
            RunEventKind::NodeRetryScheduled {
                execution,
                next_attempt,
                timer,
                ..
            } if next_attempt == &self.attempt => {
                let (node_id, revision_id) = self
                    .executions
                    .get(execution)
                    .cloned()
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
            } if attempt == &self.attempt => {
                let revision_id = self
                    .executions
                    .get(execution)
                    .map(|(_, revision)| revision.clone())
                    .or_else(|| {
                        self.current_revision
                            .as_ref()
                            .map(|revision| revision.as_str().to_owned())
                    })
                    .ok_or_else(|| corruption("scheduled attempt has no revision"))?;
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
                attempt, snapshot, ..
            } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.capability_id = Some(snapshot.capability().as_str().to_owned());
                    located.value.descriptor_revision = Some(snapshot.descriptor_revision());
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
                attempt, outcome, ..
            } if attempt == &self.attempt => {
                if let Some(located) = self.located.as_mut() {
                    located.value.state = "terminal".to_owned();
                    located.value.terminal = Some(snake_debug(outcome));
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
        let page_size = PageSize::new(256).map_err(public_persistence)?;
        let mut cursor = None;
        let mut state = HistoricalAttemptState::new(attempt);
        loop {
            let query =
                EventPageQuery::new(run.clone(), cursor, page_size).map_err(public_persistence)?;
            let page = self.store.events(&query).map_err(public_persistence)?;
            for event in &page.events {
                state.fold(event)?;
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        state.finish()
    }
}
