use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::{
    AuthorityBudget, AuthorityExecutionProvenance, AuthorityOperation, BoundaryTimeMillis,
    DecisionId, RequestedResourceFacts,
};
use milkdrift_blueprint::{ContextSemanticRole, RevisionId};
use milkdrift_model::{
    AuthorityFact, ContextEvidenceReference, ContextProducerFact, ContextSemanticKind,
    ContextSource,
};
use milkdrift_persistence::{AttemptId, RunEventEnvelope};
use milkdrift_workspace::{
    ArtifactReference, ArtifactSensitivity, CausalReference, ContentDigest, WorkspaceValue,
    WorkspaceValueReference,
};

use super::{
    ArtifactCandidateFacts, AttemptFact, ContextBuildError, ContextCandidate,
    ContextCandidateArtifactFacts, ContextCandidateAvailability, ContextSourceRequest,
    DurableContextCandidateSource, ExecutionFact, WorkspaceCandidateFacts, ancestor_depths,
    artifact_causes, bounded_name, combined_authority, persistence, producer, provenance,
    retention, sensitivity, summarize_context_event,
};

impl DurableContextCandidateSource<'_> {
    fn authority(
        &self,
        request: &ContextSourceRequest<'_>,
        operation: AuthorityOperation,
        mut resources: RequestedResourceFacts,
        budget: AuthorityBudget,
        source_key: &str,
    ) -> Result<AuthorityFact, ContextBuildError> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.context-source-authority.v1\0");
        hasher.update(request.authority.digest().as_bytes());
        hasher.update(request.identity.attempt.as_str().as_bytes());
        hasher.update(source_key.as_bytes());
        resources.revision = Some(request.identity.revision.clone());
        let decision = request.authority.request(
            DecisionId::new(format!("decision:{}", hasher.finalize()))
                .map_err(|error| ContextBuildError::Policy(error.to_string()))?,
            operation,
            resources,
            budget,
            BoundaryTimeMillis::new(request.evaluated_at_ms),
            AuthorityExecutionProvenance {
                revision: Some(request.identity.revision.clone()),
                node: Some(request.identity.node.clone()),
                execution: Some(request.identity.execution.as_str().to_owned()),
                attempt: Some(request.identity.attempt.as_str().to_owned()),
                ..AuthorityExecutionProvenance::default()
            },
        );
        let decision = self
            .authority
            .evaluate(&decision)
            .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
        Ok(AuthorityFact {
            required: true,
            authorized: decision.is_allowed(),
            authority_reference: Some(decision.digest().to_owned()),
        })
    }

    pub(super) fn artifact_facts(
        &self,
        request: &ContextSourceRequest<'_>,
        reference: &ArtifactReference,
        name: &str,
    ) -> Result<ArtifactCandidateFacts, ContextBuildError> {
        let metadata = self
            .store
            .metadata(reference.artifact())
            .map_err(persistence)?;
        let Some(metadata) = metadata.filter(|metadata| metadata.reference() == reference) else {
            return Ok(ArtifactCandidateFacts::missing(reference));
        };
        let authority = if metadata.sensitivity() == ArtifactSensitivity::Public {
            AuthorityFact {
                required: false,
                authorized: true,
                authority_reference: Some(request.authority.accepted_decision_digest().to_owned()),
            }
        } else {
            let mut resources = RequestedResourceFacts::empty();
            resources.artifact = Some(reference.artifact().clone());
            resources.artifact_sensitivity = Some(metadata.sensitivity());
            self.authority(
                request,
                AuthorityOperation::ReadArtifactContent,
                resources,
                AuthorityBudget {
                    artifact_bytes: Some(reference.size_bytes()),
                    ..AuthorityBudget::default()
                },
                reference.artifact().as_str(),
            )?
        };
        Ok(ArtifactCandidateFacts {
            content_digest: reference.digest(),
            selected_bytes: 0,
            artifact_bytes: reference.size_bytes(),
            sensitivity: metadata.sensitivity(),
            authority,
            availability: ContextCandidateAvailability::Available,
            selector: Some(ContextCandidateArtifactFacts {
                name: bounded_name(name),
                media_type: reference.media_type().as_str().to_owned(),
                sensitivity: sensitivity(metadata.sensitivity()),
                retention: retention(metadata.retention()),
                provenance: provenance(metadata.provenance()),
            }),
            causal_parents: artifact_causes(&metadata),
        })
    }

    pub(super) fn workspace_facts(
        &self,
        request: &ContextSourceRequest<'_>,
        reference: &WorkspaceValueReference,
    ) -> Result<WorkspaceCandidateFacts, ContextBuildError> {
        let mut resources = RequestedResourceFacts::empty();
        resources.workspace_scope = Some(reference.scope().scope().clone());
        let authority = self.authority(
            request,
            AuthorityOperation::ReadWorkspaceValue,
            resources,
            AuthorityBudget::default(),
            &serde_json::to_string(reference)
                .map_err(|error| ContextBuildError::Policy(error.to_string()))?,
        )?;
        if !authority.authorized {
            return Ok(WorkspaceCandidateFacts {
                content_digest: ContentDigest::for_bytes(&[]),
                selected_bytes: 0,
                artifact: None,
                sensitivity: ArtifactSensitivity::Restricted,
                authority,
                availability: ContextCandidateAvailability::Available,
            });
        }
        let entry = self.store.value(reference).map_err(persistence)?;
        let Some(entry) = entry else {
            return Ok(WorkspaceCandidateFacts::missing());
        };
        match entry.value() {
            WorkspaceValue::Json(value) => {
                let bytes = serde_json::to_vec(value.value())
                    .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
                Ok(WorkspaceCandidateFacts {
                    content_digest: ContentDigest::for_bytes(&bytes),
                    selected_bytes: u64::try_from(bytes.len())
                        .map_err(|_| ContextBuildError::AccountingOverflow)?,
                    artifact: None,
                    sensitivity: ArtifactSensitivity::Restricted,
                    authority,
                    availability: ContextCandidateAvailability::Available,
                })
            }
            WorkspaceValue::Artifact(reference) => Ok(WorkspaceCandidateFacts {
                content_digest: reference.digest(),
                selected_bytes: 0,
                artifact: Some(reference.clone()),
                sensitivity: ArtifactSensitivity::Restricted,
                authority,
                availability: ContextCandidateAvailability::Available,
            }),
        }
    }

    #[allow(clippy::too_many_arguments)] // One exact durable output occurrence has these facts.
    pub(super) fn output_candidate(
        &self,
        request: &ContextSourceRequest<'_>,
        execution: &ExecutionFact,
        attempt: Option<&AttemptId>,
        value: &WorkspaceValueReference,
        artifact: Option<&ArtifactReference>,
        event: &RunEventEnvelope,
        producer: ContextProducerFact,
        exposed_across_scope: bool,
        distances: &mut BTreeMap<RevisionId, BTreeMap<milkdrift_blueprint::NodeId, u16>>,
    ) -> Result<ContextCandidate, ContextBuildError> {
        let workspace = self.workspace_facts(request, value)?;
        let exact_artifact = artifact.cloned().or(workspace.artifact);
        let (
            kind,
            source,
            digest,
            bytes,
            artifact_bytes,
            sensitivity,
            authority,
            availability,
            selector,
            mut parents,
        ) = if let Some(reference) = exact_artifact {
            let mut facts = self.artifact_facts(request, &reference, value.key().as_str())?;
            facts.authority = combined_authority(workspace.authority, facts.authority);
            (
                ContextSemanticKind::Artifact,
                ContextSource::Artifact { reference },
                facts.content_digest,
                facts.selected_bytes,
                facts.artifact_bytes,
                facts.sensitivity,
                facts.authority,
                facts.availability,
                facts.selector,
                facts.causal_parents,
            )
        } else {
            (
                ContextSemanticKind::SuccessfulOutput,
                ContextSource::WorkspaceValue {
                    reference: value.clone(),
                },
                workspace.content_digest,
                workspace.selected_bytes,
                0,
                workspace.sensitivity,
                workspace.authority,
                workspace.availability,
                None,
                vec![ContextEvidenceReference::Workspace {
                    reference: CausalReference::WorkspaceValue {
                        reference: value.clone(),
                    },
                }],
            )
        };
        parents.push(ContextEvidenceReference::Workspace {
            reference: CausalReference::WorkspaceValue {
                reference: value.clone(),
            },
        });
        parents.push(ContextEvidenceReference::Execution {
            execution: execution.execution.clone(),
        });
        parents.push(ContextEvidenceReference::Event {
            event: event.event_id().clone(),
            sequence: event.sequence(),
        });
        parents.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
        parents.dedup();
        let distance = self.distance(request, execution, distances)?;
        let serialized = serde_json::to_string(value)
            .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
        let roles = self.output_roles(execution)?;
        Ok(ContextCandidate {
            kind,
            source: Some(source),
            content_digest: digest,
            source_revision: execution.revision.clone(),
            execution: Some(execution.execution.clone()),
            attempt: attempt.cloned(),
            source_sequence: Some(event.sequence()),
            occurred_at_ms: Some(event.occurred_at().get()),
            causal_distance: distance,
            producer,
            node: Some(execution.node.clone()),
            roles,
            scope: Some(value.scope().clone()),
            exposed_across_scope,
            required: request
                .policy
                .selected_executions()
                .contains(execution.execution.as_str())
                || request
                    .policy
                    .selected_workspace_values()
                    .contains(&serialized),
            availability,
            selected_bytes: bytes,
            selected_artifact_bytes: artifact_bytes,
            estimated_model_input_units: None,
            sensitivity,
            authority,
            artifact: selector,
            causal_parents: parents,
        })
    }

    #[allow(clippy::too_many_arguments)] // One exact journal evidence occurrence has these facts.
    pub(super) fn event_candidate(
        &self,
        request: &ContextSourceRequest<'_>,
        event: &RunEventEnvelope,
        kind: ContextSemanticKind,
        roles: BTreeSet<ContextSemanticRole>,
        execution: Option<&ExecutionFact>,
        attempt_id: Option<&AttemptId>,
        attempt: Option<&AttemptFact>,
        actor: Option<String>,
        required: bool,
        distances: &mut BTreeMap<RevisionId, BTreeMap<milkdrift_blueprint::NodeId, u16>>,
    ) -> Result<ContextCandidate, ContextBuildError> {
        let summary = summarize_context_event(event)?;
        let bytes = serde_json::to_vec(summary.value())
            .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
        let revision = execution
            .map(|execution| execution.revision.clone())
            .unwrap_or_else(|| request.identity.revision.clone());
        let distance = execution
            .map(|execution| self.distance(request, execution, distances))
            .transpose()?
            .flatten();
        let mut parents = vec![ContextEvidenceReference::Event {
            event: event.event_id().clone(),
            sequence: event.sequence(),
        }];
        if let Some(execution) = execution {
            parents.push(ContextEvidenceReference::Execution {
                execution: execution.execution.clone(),
            });
        }
        Ok(ContextCandidate {
            kind,
            source: Some(ContextSource::Event {
                event: event.event_id().clone(),
                sequence: event.sequence(),
            }),
            content_digest: ContentDigest::for_bytes(&bytes),
            source_revision: revision,
            execution: execution.map(|execution| execution.execution.clone()),
            attempt: attempt_id.cloned(),
            source_sequence: Some(event.sequence()),
            occurred_at_ms: Some(event.occurred_at().get()),
            causal_distance: distance,
            producer: producer(attempt, actor),
            node: execution.map(|execution| execution.node.clone()),
            roles,
            scope: execution.map(|execution| execution.scope.clone()),
            exposed_across_scope: false,
            required,
            availability: ContextCandidateAvailability::Available,
            selected_bytes: u64::try_from(bytes.len())
                .map_err(|_| ContextBuildError::AccountingOverflow)?,
            selected_artifact_bytes: 0,
            estimated_model_input_units: None,
            sensitivity: ArtifactSensitivity::Restricted,
            authority: AuthorityFact {
                required: false,
                authorized: true,
                authority_reference: Some(request.authority.accepted_decision_digest().to_owned()),
            },
            artifact: None,
            causal_parents: parents,
        })
    }

    pub(super) fn distance(
        &self,
        request: &ContextSourceRequest<'_>,
        execution: &ExecutionFact,
        cache: &mut BTreeMap<RevisionId, BTreeMap<milkdrift_blueprint::NodeId, u16>>,
    ) -> Result<Option<u16>, ContextBuildError> {
        if request.policy.ancestor_depth().is_none() {
            return Ok(None);
        }
        if !cache.contains_key(&execution.revision) {
            let revision = self
                .store
                .revision(&execution.revision)
                .map_err(persistence)?
                .ok_or_else(|| ContextBuildError::MissingNode(execution.node.clone()))?;
            cache.insert(
                execution.revision.clone(),
                ancestor_depths(
                    revision.semantic(),
                    &request.identity.node,
                    request.policy.ancestor_depth(),
                ),
            );
        }
        Ok(cache
            .get(&execution.revision)
            .and_then(|distances| distances.get(&execution.node).copied()))
    }

    pub(super) fn output_roles(
        &self,
        execution: &ExecutionFact,
    ) -> Result<BTreeSet<ContextSemanticRole>, ContextBuildError> {
        let revision = self
            .store
            .revision(&execution.revision)
            .map_err(persistence)?
            .ok_or_else(|| ContextBuildError::MissingNode(execution.node.clone()))?;
        let node = revision
            .semantic()
            .nodes()
            .get(&execution.node)
            .ok_or_else(|| ContextBuildError::MissingNode(execution.node.clone()))?;
        let mut roles = match node.kind() {
            milkdrift_blueprint::NodeKind::Task { config } => config.output_context_roles().clone(),
            _ => BTreeSet::new(),
        };
        roles.insert(ContextSemanticRole::Evidence);
        Ok(roles)
    }
}
