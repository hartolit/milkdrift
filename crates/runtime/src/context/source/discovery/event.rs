//! Event classification and candidate folding for one journal page.

use milkdrift_model::{ContextProducerFact, ContextSemanticKind};
use milkdrift_persistence::{RunEventEnvelope, RunEventKind};

use super::DiscoveryState;
use crate::context::source::{
    AttemptFact, ContextBuildError, ExecutionFact, event_actor, event_attempt, event_execution,
    event_semantics, producer,
};

impl DiscoveryState<'_, '_, '_> {
    #[expect(
        clippy::too_many_lines,
        reason = "this exhaustive reducer is the single owner of journal-event context classification"
    )]
    pub(super) fn fold_event(&mut self, event: &RunEventEnvelope) -> Result<(), ContextBuildError> {
        match event.kind() {
            RunEventKind::RunCreated { revision, .. }
            | RunEventKind::RevisionPinned { revision, .. } => {
                self.current_revision = revision.clone();
            }
            RunEventKind::NodeBecameEligible {
                node,
                execution,
                scope,
                ..
            } => {
                self.executions
                    .entry(execution.clone())
                    .or_insert_with(|| ExecutionFact {
                        execution: execution.clone(),
                        node: node.clone(),
                        scope: scope.clone(),
                        revision: self.current_revision.clone(),
                    });
            }
            RunEventKind::NodeScheduled {
                execution,
                attempt,
                invocation,
                ..
            } => {
                self.attempts.insert(
                    attempt.clone(),
                    AttemptFact {
                        execution: Some(execution.clone()),
                        invocation: Some(invocation.as_str().to_owned()),
                        ..AttemptFact::default()
                    },
                );
            }
            RunEventKind::CapabilityResolved {
                attempt, snapshot, ..
            } => {
                let fact = self.attempts.entry(attempt.clone()).or_default();
                fact.capability = Some(snapshot.capability().as_str().to_owned());
                fact.descriptor_revision = Some(snapshot.descriptor_revision());
                fact.provider_profile = snapshot
                    .provider_profile()
                    .map(|profile| profile.as_str().to_owned());
            }
            RunEventKind::CapabilityResolutionDecisionRecorded {
                attempt,
                authorization,
                ..
            } => {
                let fact = self.attempts.entry(attempt.clone()).or_default();
                fact.peer = authorization
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
            RunEventKind::NodeOutputPublished {
                execution,
                attempt,
                value,
                artifact,
                ..
            } => {
                if self.indexed_sequences.contains(&event.sequence()) {
                    return Ok(());
                }
                if let Some(execution_fact) = self.executions.get(execution) {
                    self.candidates.push(self.source.output_candidate(
                        &self.request,
                        execution_fact,
                        Some(attempt),
                        value,
                        artifact.as_ref(),
                        event,
                        producer(self.attempts.get(attempt), None),
                        false,
                        &mut self.distances,
                    )?);
                }
            }
            RunEventKind::DeterministicOutputPublished {
                execution,
                value,
                artifact,
            } => {
                if self.indexed_sequences.contains(&event.sequence()) {
                    return Ok(());
                }
                if let Some(execution_fact) = self.executions.get(execution) {
                    self.candidates.push(self.source.output_candidate(
                        &self.request,
                        execution_fact,
                        None,
                        value,
                        artifact.as_ref(),
                        event,
                        ContextProducerFact::default(),
                        false,
                        &mut self.distances,
                    )?);
                }
            }
            RunEventKind::SubworkflowCreated {
                subworkflow,
                parent_execution,
                ..
            } => self.record_subworkflow_parent(subworkflow, parent_execution)?,
            RunEventKind::SubworkflowOutputImported {
                subworkflow,
                parent_value,
                ..
            } => self.import_subworkflow_output(event, subworkflow, parent_value)?,
            RunEventKind::JoinSatisfied {
                execution,
                branches,
                ..
            } => {
                self.expose_join_outputs(execution, branches);
                if self.can_summarize_event() {
                    self.candidates.push(
                        self.source.event_candidate(
                            &self.request,
                            event,
                            ContextSemanticKind::SuccessfulOutput,
                            std::collections::BTreeSet::from([
                                milkdrift_blueprint::ContextSemanticRole::Evidence,
                            ]),
                            event_execution(event.kind())
                                .and_then(|execution| self.executions.get(execution)),
                            None,
                            None,
                            None,
                            false,
                            &mut self.distances,
                        )?,
                    );
                    self.count_event_summary()?;
                }
            }
            kind => self.fold_semantic_event(event, kind)?,
        }
        Ok(())
    }

    fn fold_semantic_event(
        &mut self,
        event: &RunEventEnvelope,
        kind: &RunEventKind,
    ) -> Result<(), ContextBuildError> {
        if self.indexed_sequences.contains(&event.sequence()) || !self.can_summarize_event() {
            return Ok(());
        }
        let Some((semantic_kind, roles)) = event_semantics(kind) else {
            return Ok(());
        };
        let attempt_id = event_attempt(kind);
        let attempt = attempt_id.and_then(|attempt| self.attempts.get(attempt));
        let execution = event_execution(kind)
            .and_then(|execution| self.executions.get(execution))
            .or_else(|| {
                attempt
                    .and_then(|attempt| attempt.execution.as_ref())
                    .and_then(|execution| self.executions.get(execution))
            });
        self.candidates.push(self.source.event_candidate(
            &self.request,
            event,
            semantic_kind,
            roles,
            execution,
            attempt_id,
            attempt,
            event_actor(kind),
            false,
            &mut self.distances,
        )?);
        self.count_event_summary()
    }
}
