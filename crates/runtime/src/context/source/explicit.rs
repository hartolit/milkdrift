use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::{ContextSemanticRole, RevisionId};
use milkdrift_capability::BoundedJson;
use milkdrift_model::{
    AuthorityFact, ContextEvidenceReference, ContextProducerFact, ContextSemanticKind,
    ContextSource,
};
use milkdrift_persistence::{AttemptId, NodeExecutionId};
use milkdrift_workspace::{
    ArtifactReference, ArtifactSensitivity, CausalReference, ContentDigest, ScopeReference,
    WorkspaceValueReference,
};

use super::{
    AttemptFact, ContextBuildError, ContextCandidate, ContextCandidateAvailability,
    ContextSourceRequest, DurableContextCandidateSource, ExecutionFact, event_actor, event_attempt,
    event_execution, event_semantics, exact_event, producer,
};

impl DurableContextCandidateSource<'_> {
    pub(super) fn explicit_workspace_candidate(
        &self,
        request: &ContextSourceRequest<'_>,
        reference: WorkspaceValueReference,
    ) -> Result<ContextCandidate, ContextBuildError> {
        let facts = self.workspace_facts(request, &reference)?;
        if let Some(artifact) = facts.artifact {
            let mut candidate = self.explicit_artifact_candidate(
                request,
                artifact,
                Some(reference.scope().clone()),
            )?;
            candidate
                .causal_parents
                .push(ContextEvidenceReference::Workspace {
                    reference: CausalReference::WorkspaceValue { reference },
                });
            return Ok(candidate);
        }
        Ok(ContextCandidate {
            kind: ContextSemanticKind::SuccessfulOutput,
            source: Some(ContextSource::WorkspaceValue {
                reference: reference.clone(),
            }),
            content_digest: facts.content_digest,
            source_revision: request.identity.revision.clone(),
            execution: None,
            attempt: None,
            source_sequence: None,
            occurred_at_ms: None,
            causal_distance: None,
            producer: ContextProducerFact::default(),
            node: None,
            roles: BTreeSet::from([ContextSemanticRole::Evidence]),
            scope: Some(reference.scope().clone()),
            exposed_across_scope: false,
            required: true,
            availability: facts.availability,
            selected_bytes: facts.selected_bytes,
            selected_artifact_bytes: 0,
            estimated_model_input_units: None,
            sensitivity: facts.sensitivity,
            authority: facts.authority,
            artifact: None,
            causal_parents: vec![ContextEvidenceReference::Workspace {
                reference: CausalReference::WorkspaceValue { reference },
            }],
        })
    }

    fn explicit_artifact_candidate(
        &self,
        request: &ContextSourceRequest<'_>,
        reference: ArtifactReference,
        scope: Option<ScopeReference>,
    ) -> Result<ContextCandidate, ContextBuildError> {
        let facts = self.artifact_facts(request, &reference, reference.artifact().as_str())?;
        Ok(ContextCandidate {
            kind: ContextSemanticKind::Artifact,
            source: Some(ContextSource::Artifact {
                reference: reference.clone(),
            }),
            content_digest: facts.content_digest,
            source_revision: request.identity.revision.clone(),
            execution: None,
            attempt: None,
            source_sequence: None,
            occurred_at_ms: None,
            causal_distance: None,
            producer: ContextProducerFact::default(),
            node: None,
            roles: BTreeSet::from([ContextSemanticRole::Evidence]),
            scope,
            exposed_across_scope: false,
            required: true,
            availability: facts.availability,
            selected_bytes: facts.selected_bytes,
            selected_artifact_bytes: facts.artifact_bytes,
            estimated_model_input_units: None,
            sensitivity: facts.sensitivity,
            authority: facts.authority,
            artifact: facts.selector,
            causal_parents: facts.causal_parents,
        })
    }

    pub(super) fn explicit_candidate(
        &self,
        request: &ContextSourceRequest<'_>,
        source: ContextSource,
        executions: &BTreeMap<NodeExecutionId, ExecutionFact>,
        attempts: &BTreeMap<AttemptId, AttemptFact>,
        distances: &mut BTreeMap<RevisionId, BTreeMap<milkdrift_blueprint::NodeId, u16>>,
    ) -> Result<ContextCandidate, ContextBuildError> {
        match source {
            ContextSource::WorkspaceValue { reference } => {
                self.explicit_workspace_candidate(request, reference)
            }
            ContextSource::Artifact { reference } => {
                self.explicit_artifact_candidate(request, reference, None)
            }
            ContextSource::Event { event, sequence } => {
                if sequence > request.through_sequence {
                    return Err(ContextBuildError::RequiredUnavailable(
                        "explicit future event",
                    ));
                }
                let envelope = exact_event(self.store, &request.identity.run, sequence, &event)?;
                let semantics = event_semantics(envelope.kind()).unwrap_or((
                    ContextSemanticKind::SuccessfulOutput,
                    BTreeSet::from([ContextSemanticRole::Evidence]),
                ));
                let attempt_id = event_attempt(envelope.kind());
                let attempt = attempt_id.and_then(|id| attempts.get(id));
                let execution = event_execution(envelope.kind())
                    .and_then(|id| executions.get(id))
                    .or_else(|| {
                        attempt
                            .and_then(|fact| fact.execution.as_ref())
                            .and_then(|id| executions.get(id))
                    });
                self.event_candidate(
                    request,
                    &envelope,
                    semantics.0,
                    semantics.1,
                    execution,
                    attempt_id,
                    attempt,
                    event_actor(envelope.kind()),
                    true,
                    distances,
                )
            }
            ContextSource::NodeExecution {
                node,
                execution,
                attempt,
                event_sequence,
            } => {
                let fact =
                    executions
                        .get(&execution)
                        .ok_or(ContextBuildError::RequiredUnavailable(
                            "explicit node execution",
                        ))?;
                if fact.node != node {
                    return Err(ContextBuildError::RequiredUnavailable(
                        "explicit node execution identity",
                    ));
                }
                let value = BoundedJson::new(serde_json::json!({
                    "node": node,
                    "execution": execution,
                    "attempt": attempt,
                    "event_sequence": event_sequence,
                }))
                .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
                let bytes = serde_json::to_vec(value.value())
                    .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
                Ok(ContextCandidate {
                    kind: ContextSemanticKind::SuccessfulOutput,
                    source: Some(ContextSource::NodeExecution {
                        node: node.clone(),
                        execution: execution.clone(),
                        attempt: attempt.clone(),
                        event_sequence,
                    }),
                    content_digest: ContentDigest::for_bytes(&bytes),
                    source_revision: fact.revision.clone(),
                    execution: Some(execution),
                    attempt: attempt.clone(),
                    source_sequence: event_sequence,
                    occurred_at_ms: None,
                    causal_distance: self.distance(request, fact, distances)?,
                    producer: producer(attempt.as_ref().and_then(|id| attempts.get(id)), None),
                    node: Some(node),
                    roles: self.output_roles(fact)?,
                    scope: Some(fact.scope.clone()),
                    exposed_across_scope: false,
                    required: true,
                    availability: ContextCandidateAvailability::Available,
                    selected_bytes: u64::try_from(bytes.len())
                        .map_err(|_| ContextBuildError::AccountingOverflow)?,
                    selected_artifact_bytes: 0,
                    estimated_model_input_units: None,
                    sensitivity: ArtifactSensitivity::Restricted,
                    authority: AuthorityFact {
                        required: false,
                        authorized: true,
                        authority_reference: Some(
                            request.authority.accepted_decision_digest().to_owned(),
                        ),
                    },
                    artifact: None,
                    causal_parents: Vec::new(),
                })
            }
            ContextSource::DirectInput { name, reference } => request
                .direct_inputs
                .iter()
                .find(|input| input.name() == name && input.value() == &reference)
                .ok_or(ContextBuildError::RequiredUnavailable(
                    "explicit direct input",
                ))
                .and_then(|input| self.direct_candidate(request, input)),
        }
    }
}
