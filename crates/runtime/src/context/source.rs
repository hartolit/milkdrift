//! Authoritative metadata discovery and selected-only context materialization.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::{
    AuthorityBudget, AuthorityEvaluator, AuthorityExecutionProvenance, AuthorityOperation,
    BoundaryTimeMillis, DecisionId, ExecutionAuthorityBasis, RequestedResourceFacts,
};
use milkdrift_blueprint::{
    BlueprintRevision, ContextArtifactRetention, ContextArtifactSensitivity,
    ContextProvenanceClass, ContextSemanticRole, RevisionId, TaskContextPolicy,
};
use milkdrift_capability::{
    ArtifactReference as CapabilityArtifactReference, BoundedJson, InputReference,
};
use milkdrift_model::{
    AuthorityFact, ContextEvidenceReference, ContextProducerFact, ContextSemanticKind,
    ContextSource,
};
use milkdrift_persistence::{
    AttemptId, EventCursor, EventId, EventPageQuery, NodeExecutionId, PageSize, RunEventEnvelope,
    RunEventKind, RunQueryStore, RunSequence,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, CausalReference, ContentDigest, RunId, ScopeReference, SubworkflowId,
    WorkspaceValue, WorkspaceValueReference,
};

use super::{
    ContextBuildError, ContextBuildIdentity, ContextCandidate, ContextCandidateArtifactFacts,
    ContextCandidateAvailability, ancestor_depths,
};

const SOURCE_PAGE_SIZE: u32 = 256;

mod direct;
mod materialize;

pub use materialize::{materialize_selected_context, read_context_manifest};

/// Frozen facts used by the canonical runtime candidate source.
pub struct ContextSourceRequest<'a> {
    /// Exact current attempt identity.
    pub identity: ContextBuildIdentity,
    /// Immutable current revision.
    pub revision: &'a BlueprintRevision,
    /// Immutable task policy under that revision.
    pub policy: &'a TaskContextPolicy,
    /// Current execution scope.
    pub scope: &'a ScopeReference,
    /// Exact already-resolved direct task inputs.
    pub direct_inputs: &'a [InputReference],
    /// Names of workflow-declared direct inputs that are required.
    pub required_direct_inputs: &'a BTreeSet<String>,
    /// Exact journal head covered by the dispatch projection.
    pub through_sequence: RunSequence,
    /// Already-validated bounded projection at the same frozen journal boundary.
    pub projection: &'a crate::RunProjection,
    /// Frozen initiating authority basis.
    pub authority: &'a ExecutionAuthorityBasis,
    /// Caller-supplied boundary time for fresh read decisions.
    pub evaluated_at_ms: u64,
}

/// Narrow runtime boundary that discovers bounded metadata candidates from durable state.
pub trait ContextCandidateSource {
    /// Discovers metadata only; large artifact content is not read by this method.
    fn discover(
        &self,
        request: ContextSourceRequest<'_>,
    ) -> Result<Vec<ContextCandidate>, ContextBuildError>;
}

/// Production source over journal, workspace, revision, artifact, and authority ports.
pub struct DurableContextCandidateSource<'a> {
    store: &'a dyn crate::RuntimeStore,
    authority: &'a dyn AuthorityEvaluator,
}

impl<'a> DurableContextCandidateSource<'a> {
    /// Binds the existing authoritative ports without introducing duplicate storage.
    #[must_use]
    pub const fn new(
        store: &'a dyn crate::RuntimeStore,
        authority: &'a dyn AuthorityEvaluator,
    ) -> Self {
        Self { store, authority }
    }

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

    fn artifact_facts(
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

    fn workspace_facts(
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
    fn output_candidate(
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
    fn event_candidate(
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

    fn distance(
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

    fn output_roles(
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

impl ContextCandidateSource for DurableContextCandidateSource<'_> {
    fn discover(
        &self,
        request: ContextSourceRequest<'_>,
    ) -> Result<Vec<ContextCandidate>, ContextBuildError> {
        let mut candidates = Vec::new();
        for input in request.direct_inputs {
            candidates.push(self.direct_candidate(&request, input)?);
        }

        let maximum_records = request.policy.budget().max_candidate_records;
        let first_sequence = candidate_tail_start(request.through_sequence.get(), maximum_records);
        let mut cursor = (first_sequence > 1).then(|| EventCursor {
            run: request.identity.run.clone(),
            next_sequence: RunSequence::new(first_sequence),
        });
        let mut scanned = 0_u32;
        let mut event_summaries = 0_u32;
        let mut current_revision = request.revision.id().clone();
        let mut executions = request
            .projection
            .current_node_executions()
            .map(|execution| {
                (
                    execution.execution().clone(),
                    ExecutionFact {
                        execution: execution.execution().clone(),
                        node: execution.node().clone(),
                        scope: execution.scope().clone(),
                        revision: execution.revision().clone(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut attempts = request
            .projection
            .attempts()
            .iter()
            .map(|(attempt, fact)| (attempt.clone(), projected_attempt_fact(fact)))
            .collect::<BTreeMap<_, _>>();
        let mut distances = BTreeMap::new();
        let mut subworkflow_parents = request
            .projection
            .subworkflows()
            .values()
            .map(|child| {
                (
                    child.subworkflow().clone(),
                    child.parent_execution().clone(),
                )
            })
            .collect::<BTreeMap<SubworkflowId, NodeExecutionId>>();
        let all_ancestors = ancestor_depths(
            request.revision.semantic(),
            &request.identity.node,
            Some(u16::MAX),
        );
        let mut join_exposed_values = BTreeSet::new();
        for join in request.projection.joins().values() {
            if executions.get(join.execution()).is_some_and(|execution| {
                execution.revision == *request.revision.id()
                    && all_ancestors.contains_key(&execution.node)
            }) {
                join_exposed_values.extend(
                    join.branches()
                        .iter()
                        .flat_map(|branch| branch.outputs.iter().cloned()),
                );
            }
        }
        let mut indexed_sequences = BTreeSet::new();
        for execution in request.projection.current_node_executions() {
            let Some(execution_fact) = executions.get(execution.execution()) else {
                continue;
            };
            for output in execution.outputs() {
                let event = event_at(
                    self.store,
                    &request.identity.run,
                    output.sequence(),
                    request.through_sequence,
                )?;
                let attempt = event_attempt(event.kind());
                candidates.push(self.output_candidate(
                    &request,
                    execution_fact,
                    attempt,
                    output.value(),
                    output.artifact(),
                    &event,
                    producer(attempt.and_then(|attempt| attempts.get(attempt)), None),
                    false,
                    &mut distances,
                )?);
                indexed_sequences.insert(output.sequence());
            }
        }
        for execution in request.projection.node_executions().values() {
            let sequence = execution
                .deterministic_terminal()
                .map(|terminal| terminal.sequence())
                .or_else(|| {
                    execution
                        .attempts()
                        .last()
                        .and_then(|attempt| request.projection.attempts().get(attempt))
                        .and_then(|attempt| attempt.terminal())
                        .map(|terminal| terminal.sequence())
                });
            let Some(sequence) = sequence else {
                continue;
            };
            if event_summaries >= request.policy.budget().max_event_summaries {
                break;
            }
            let event = event_at(
                self.store,
                &request.identity.run,
                sequence,
                request.through_sequence,
            )?;
            let Some((kind, roles)) = event_semantics(event.kind()) else {
                continue;
            };
            let execution_fact = executions.get(execution.execution());
            let attempt_id = event_attempt(event.kind());
            let attempt = attempt_id.and_then(|attempt| attempts.get(attempt));
            candidates.push(self.event_candidate(
                &request,
                &event,
                kind,
                roles,
                execution_fact,
                attempt_id,
                attempt,
                event_actor(event.kind()),
                false,
                &mut distances,
            )?);
            event_summaries += 1;
            indexed_sequences.insert(sequence);
        }
        for execution in request.projection.settled_node_executions().values() {
            let Some(sequence) = execution.terminal_sequence() else {
                continue;
            };
            if event_summaries >= request.policy.budget().max_event_summaries {
                break;
            }
            let event = event_at(
                self.store,
                &request.identity.run,
                sequence,
                request.through_sequence,
            )?;
            let Some((kind, roles)) = event_semantics(event.kind()) else {
                continue;
            };
            let execution_fact = executions.get(execution.execution());
            let attempt_id = event_attempt(event.kind());
            let attempt = attempt_id.and_then(|attempt| attempts.get(attempt));
            candidates.push(self.event_candidate(
                &request,
                &event,
                kind,
                roles,
                execution_fact,
                attempt_id,
                attempt,
                event_actor(event.kind()),
                false,
                &mut distances,
            )?);
            event_summaries += 1;
            indexed_sequences.insert(sequence);
        }
        'pages: loop {
            let remaining = maximum_records - scanned;
            if remaining == 0 {
                break;
            }
            let page_size = SOURCE_PAGE_SIZE.min(remaining);
            let page = self
                .store
                .events(
                    &EventPageQuery::new(
                        request.identity.run.clone(),
                        cursor,
                        PageSize::new(page_size).map_err(persistence)?,
                    )
                    .map_err(persistence)?,
                )
                .map_err(persistence)?;
            scanned = scanned
                .checked_add(
                    u32::try_from(page.events.len())
                        .map_err(|_| ContextBuildError::AccountingOverflow)?,
                )
                .ok_or(ContextBuildError::AccountingOverflow)?;
            for event in &page.events {
                if event.sequence() > request.through_sequence {
                    break 'pages;
                }
                match event.kind() {
                    RunEventKind::RunCreated { revision, .. }
                    | RunEventKind::RevisionPinned { revision, .. } => {
                        current_revision = revision.clone();
                    }
                    RunEventKind::NodeBecameEligible {
                        node,
                        execution,
                        scope,
                        ..
                    } => {
                        executions
                            .entry(execution.clone())
                            .or_insert_with(|| ExecutionFact {
                                execution: execution.clone(),
                                node: node.clone(),
                                scope: scope.clone(),
                                revision: current_revision.clone(),
                            });
                    }
                    RunEventKind::NodeScheduled {
                        execution,
                        attempt,
                        invocation,
                        ..
                    } => {
                        attempts.insert(
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
                        let fact = attempts.entry(attempt.clone()).or_default();
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
                        let fact = attempts.entry(attempt.clone()).or_default();
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
                        if indexed_sequences.contains(&event.sequence()) {
                            continue;
                        }
                        if let Some(execution_fact) = executions.get(execution) {
                            candidates.push(self.output_candidate(
                                &request,
                                execution_fact,
                                Some(attempt),
                                value,
                                artifact.as_ref(),
                                event,
                                producer(attempts.get(attempt), None),
                                false,
                                &mut distances,
                            )?);
                        }
                    }
                    RunEventKind::DeterministicOutputPublished {
                        execution,
                        value,
                        artifact,
                    } => {
                        if indexed_sequences.contains(&event.sequence()) {
                            continue;
                        }
                        if let Some(execution_fact) = executions.get(execution) {
                            candidates.push(self.output_candidate(
                                &request,
                                execution_fact,
                                None,
                                value,
                                artifact.as_ref(),
                                event,
                                ContextProducerFact::default(),
                                false,
                                &mut distances,
                            )?);
                        }
                    }
                    RunEventKind::SubworkflowCreated {
                        subworkflow,
                        parent_execution,
                        ..
                    } => {
                        record_subworkflow_parent(
                            &mut subworkflow_parents,
                            subworkflow,
                            parent_execution,
                        )?;
                    }
                    RunEventKind::SubworkflowOutputImported {
                        subworkflow,
                        parent_value,
                        ..
                    } => {
                        let execution_fact = subworkflow_parents
                            .get(subworkflow)
                            .and_then(|execution| executions.get(execution))
                            .ok_or(ContextBuildError::RequiredUnavailable(
                                "subworkflow parent provenance",
                            ))?;
                        candidates.push(self.output_candidate(
                            &request,
                            execution_fact,
                            None,
                            parent_value,
                            None,
                            event,
                            ContextProducerFact::default(),
                            true,
                            &mut distances,
                        )?);
                    }
                    RunEventKind::JoinSatisfied {
                        execution,
                        branches,
                        ..
                    } => {
                        if executions.get(execution).is_some_and(|execution| {
                            execution.revision == *request.revision.id()
                                && all_ancestors.contains_key(&execution.node)
                        }) {
                            join_exposed_values.extend(
                                branches
                                    .iter()
                                    .flat_map(|branch| branch.outputs.iter().cloned()),
                            );
                        }
                        if event_summaries < request.policy.budget().max_event_summaries {
                            event_summaries += 1;
                            candidates.push(
                                self.event_candidate(
                                    &request,
                                    event,
                                    ContextSemanticKind::SuccessfulOutput,
                                    BTreeSet::from([ContextSemanticRole::Evidence]),
                                    event_execution(event.kind())
                                        .and_then(|execution| executions.get(execution)),
                                    None,
                                    None,
                                    None,
                                    false,
                                    &mut distances,
                                )?,
                            );
                        }
                    }
                    kind if !indexed_sequences.contains(&event.sequence())
                        && event_semantics(kind).is_some()
                        && event_summaries < request.policy.budget().max_event_summaries =>
                    {
                        event_summaries += 1;
                        let semantics = event_semantics(kind).ok_or_else(|| {
                            ContextBuildError::Policy("event semantics disappeared".to_owned())
                        })?;
                        let attempt_id = event_attempt(kind);
                        let attempt = attempt_id.and_then(|attempt| attempts.get(attempt));
                        let execution = event_execution(kind)
                            .and_then(|execution| executions.get(execution))
                            .or_else(|| {
                                attempt
                                    .and_then(|attempt| attempt.execution.as_ref())
                                    .and_then(|execution| executions.get(execution))
                            });
                        candidates.push(self.event_candidate(
                            &request,
                            event,
                            semantics.0,
                            semantics.1,
                            execution,
                            attempt_id,
                            attempt,
                            event_actor(kind),
                            false,
                            &mut distances,
                        )?);
                    }
                    _ => {}
                }
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        for selector in request.policy.selected_workspace_values() {
            let reference: WorkspaceValueReference =
                serde_json::from_str(selector).map_err(|_| {
                    ContextBuildError::RequiredUnavailable("selected workspace reference")
                })?;
            let source = ContextSource::WorkspaceValue {
                reference: reference.clone(),
            };
            if !candidates
                .iter()
                .any(|candidate| candidate.source.as_ref() == Some(&source))
            {
                candidates.push(self.explicit_workspace_candidate(&request, reference)?);
            }
        }
        for selector in request.policy.explicit_evidence() {
            let source: ContextSource = serde_json::from_str(selector).map_err(|_| {
                ContextBuildError::RequiredUnavailable("explicit evidence reference")
            })?;
            if candidates
                .iter()
                .any(|candidate| candidate.source.as_ref() == Some(&source))
            {
                continue;
            }
            candidates.push(self.explicit_candidate(
                &request,
                source,
                &executions,
                &attempts,
                &mut distances,
            )?);
        }
        for candidate in &mut candidates {
            if candidate_references_join_output(candidate, &join_exposed_values) {
                candidate.exposed_across_scope = true;
            }
        }
        for selected in request.policy.selected_executions() {
            if !candidates.iter().any(|candidate| {
                candidate
                    .execution
                    .as_ref()
                    .is_some_and(|execution| execution.as_str() == selected)
            }) {
                return Err(ContextBuildError::RequiredUnavailable(
                    "selected execution has no durable evidence",
                ));
            }
        }
        if candidates.len()
            > usize::try_from(request.policy.budget().max_candidate_records)
                .map_err(|_| ContextBuildError::AccountingOverflow)?
        {
            return Err(ContextBuildError::RequiredBudget("candidate count"));
        }
        Ok(candidates)
    }
}

impl DurableContextCandidateSource<'_> {
    fn explicit_workspace_candidate(
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

    fn explicit_candidate(
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

#[derive(Clone)]
struct ExecutionFact {
    execution: NodeExecutionId,
    node: milkdrift_blueprint::NodeId,
    scope: ScopeReference,
    revision: RevisionId,
}

#[derive(Clone, Default)]
struct AttemptFact {
    execution: Option<NodeExecutionId>,
    invocation: Option<String>,
    capability: Option<String>,
    descriptor_revision: Option<u64>,
    provider_profile: Option<String>,
    peer: Option<String>,
}

struct ArtifactCandidateFacts {
    content_digest: ContentDigest,
    selected_bytes: u64,
    artifact_bytes: u64,
    sensitivity: ArtifactSensitivity,
    authority: AuthorityFact,
    availability: ContextCandidateAvailability,
    selector: Option<ContextCandidateArtifactFacts>,
    causal_parents: Vec<ContextEvidenceReference>,
}

impl ArtifactCandidateFacts {
    fn missing(reference: &ArtifactReference) -> Self {
        Self {
            content_digest: reference.digest(),
            selected_bytes: 0,
            artifact_bytes: reference.size_bytes(),
            sensitivity: ArtifactSensitivity::Restricted,
            authority: AuthorityFact {
                required: true,
                authorized: false,
                authority_reference: None,
            },
            availability: ContextCandidateAvailability::MissingOrCorrupt,
            selector: None,
            causal_parents: Vec::new(),
        }
    }
}

struct WorkspaceCandidateFacts {
    content_digest: ContentDigest,
    selected_bytes: u64,
    artifact: Option<ArtifactReference>,
    sensitivity: ArtifactSensitivity,
    authority: AuthorityFact,
    availability: ContextCandidateAvailability,
}

impl WorkspaceCandidateFacts {
    fn missing() -> Self {
        Self {
            content_digest: ContentDigest::for_bytes(&[]),
            selected_bytes: 0,
            artifact: None,
            sensitivity: ArtifactSensitivity::Restricted,
            authority: AuthorityFact {
                required: true,
                authorized: false,
                authority_reference: None,
            },
            availability: ContextCandidateAvailability::MissingOrCorrupt,
        }
    }
}

fn producer(attempt: Option<&AttemptFact>, actor: Option<String>) -> ContextProducerFact {
    ContextProducerFact {
        actor,
        capability: attempt.and_then(|attempt| attempt.capability.clone()),
        descriptor_revision: attempt.and_then(|attempt| attempt.descriptor_revision),
        provider_profile: attempt.and_then(|attempt| attempt.provider_profile.clone()),
        peer: attempt.and_then(|attempt| attempt.peer.clone()),
        invocation: attempt.and_then(|attempt| attempt.invocation.clone()),
    }
}

fn event_semantics(
    kind: &RunEventKind,
) -> Option<(ContextSemanticKind, BTreeSet<ContextSemanticRole>)> {
    match kind {
        RunEventKind::NodeTerminal { outcome, .. }
            if *outcome != milkdrift_persistence::NodeOutcome::Succeeded =>
        {
            Some((
                ContextSemanticKind::Failure,
                BTreeSet::from([
                    ContextSemanticRole::FailureEvidence,
                    ContextSemanticRole::Verification,
                ]),
            ))
        }
        RunEventKind::DeterministicNodeTerminal { outcome, .. }
            if *outcome != milkdrift_persistence::NodeOutcome::Succeeded =>
        {
            Some((
                ContextSemanticKind::Failure,
                BTreeSet::from([ContextSemanticRole::FailureEvidence]),
            ))
        }
        RunEventKind::NodePreDispatchFailed { .. }
        | RunEventKind::ExternalOutcomeUncertain { .. }
        | RunEventKind::RunCancellationRequested { .. }
        | RunEventKind::NodeExecutionCancellationRequested { .. }
        | RunEventKind::NodeExecutionCancelledBeforeDispatch { .. } => Some((
            ContextSemanticKind::Failure,
            BTreeSet::from([ContextSemanticRole::FailureEvidence]),
        )),
        RunEventKind::ReconciliationDecisionRecorded { .. }
        | RunEventKind::RecoveryDecisionRecorded { .. }
        | RunEventKind::RepeatContinuationDecided { .. } => Some((
            ContextSemanticKind::Decision,
            BTreeSet::from([ContextSemanticRole::Decision]),
        )),
        RunEventKind::JoinSatisfied { .. } | RunEventKind::SubworkflowTerminal { .. } => Some((
            ContextSemanticKind::SuccessfulOutput,
            BTreeSet::from([ContextSemanticRole::Evidence]),
        )),
        _ => None,
    }
}

fn event_attempt(kind: &RunEventKind) -> Option<&AttemptId> {
    match kind {
        RunEventKind::NodeTerminal { attempt, .. }
        | RunEventKind::ExternalOutcomeUncertain { attempt, .. }
        | RunEventKind::NodeExecutionCancellationRequested { attempt, .. }
        | RunEventKind::CapabilityResolutionDecisionRecorded { attempt, .. }
        | RunEventKind::CapabilityAdapterEntryDecisionRecorded { attempt, .. }
        | RunEventKind::CapabilityEntryDecisionRecorded { attempt, .. }
        | RunEventKind::RecoveryDecisionRecorded { attempt, .. } => Some(attempt),
        _ => None,
    }
}

fn event_execution(kind: &RunEventKind) -> Option<&NodeExecutionId> {
    match kind {
        RunEventKind::NodeTerminal { execution, .. }
        | RunEventKind::DeterministicNodeTerminal { execution, .. }
        | RunEventKind::NodePreDispatchFailed { execution, .. }
        | RunEventKind::NodeExecutionCancellationRequested { execution, .. }
        | RunEventKind::NodeExecutionCancelledBeforeDispatch { execution, .. }
        | RunEventKind::JoinSatisfied { execution, .. } => Some(execution),
        _ => None,
    }
}

fn event_actor(kind: &RunEventKind) -> Option<String> {
    match kind {
        RunEventKind::ReconciliationDecisionRecorded { actor, .. }
        | RunEventKind::RecoveryDecisionRecorded { actor, .. }
        | RunEventKind::RepeatContinuationDecided { actor, .. } => Some(actor.as_str().to_owned()),
        _ => None,
    }
}

/// Produces the one bounded deterministic content representation for a selected event.
pub(crate) fn summarize_context_event(
    event: &RunEventEnvelope,
) -> Result<BoundedJson, ContextBuildError> {
    let value = serde_json::to_value(event.kind())
        .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
    BoundedJson::new(serde_json::json!({
        "event_id": event.event_id(),
        "sequence": event.sequence(),
        "occurred_at_ms": event.occurred_at().get(),
        "fact": value,
    }))
    .map_err(|error| ContextBuildError::Policy(error.to_string()))
}

fn projected_attempt_fact(attempt: &crate::NodeAttemptProjection) -> AttemptFact {
    let snapshot = attempt.capability().map(|resolution| resolution.snapshot());
    let authorization = attempt.resolution_authorization();
    AttemptFact {
        execution: Some(attempt.execution().clone()),
        invocation: attempt
            .invocation()
            .map(|invocation| invocation.as_str().to_owned()),
        capability: snapshot.map(|snapshot| snapshot.capability().as_str().to_owned()),
        descriptor_revision: snapshot.map(|snapshot| snapshot.descriptor_revision()),
        provider_profile: snapshot.and_then(|snapshot| {
            snapshot
                .provider_profile()
                .map(|profile| profile.as_str().to_owned())
        }),
        peer: authorization.and_then(|authorization| {
            authorization
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
                })
        }),
    }
}

fn candidate_references_join_output(
    candidate: &ContextCandidate,
    exposed: &BTreeSet<WorkspaceValueReference>,
) -> bool {
    candidate.source.as_ref().is_some_and(|source| {
        matches!(
            source,
            ContextSource::WorkspaceValue { reference } if exposed.contains(reference)
        )
    }) || candidate.causal_parents.iter().any(|parent| {
        matches!(
            parent,
            ContextEvidenceReference::Workspace {
                reference: CausalReference::WorkspaceValue { reference }
            } if exposed.contains(reference)
        )
    })
}

fn event_at(
    store: &dyn RunQueryStore,
    run: &RunId,
    sequence: RunSequence,
    through_sequence: RunSequence,
) -> Result<RunEventEnvelope, ContextBuildError> {
    if sequence > through_sequence {
        return Err(ContextBuildError::RequiredUnavailable(
            "context event is beyond the frozen boundary",
        ));
    }
    let page = store
        .events(
            &EventPageQuery::new(
                run.clone(),
                Some(EventCursor {
                    run: run.clone(),
                    next_sequence: sequence,
                }),
                PageSize::new(1).map_err(persistence)?,
            )
            .map_err(persistence)?,
        )
        .map_err(persistence)?;
    page.events
        .into_iter()
        .find(|event| event.sequence() == sequence)
        .ok_or(ContextBuildError::RequiredUnavailable(
            "context event changed or disappeared",
        ))
}

fn exact_event(
    store: &dyn RunQueryStore,
    run: &RunId,
    sequence: RunSequence,
    event: &EventId,
) -> Result<RunEventEnvelope, ContextBuildError> {
    let page = store
        .events(
            &EventPageQuery::new(
                run.clone(),
                Some(EventCursor {
                    run: run.clone(),
                    next_sequence: sequence,
                }),
                PageSize::new(1).map_err(persistence)?,
            )
            .map_err(persistence)?,
        )
        .map_err(persistence)?;
    page.events
        .into_iter()
        .find(|candidate| candidate.sequence() == sequence && candidate.event_id() == event)
        .ok_or(ContextBuildError::RequiredUnavailable(
            "selected event changed or disappeared",
        ))
}

fn workspace_artifact(
    reference: &CapabilityArtifactReference,
) -> Result<ArtifactReference, ContextBuildError> {
    Ok(ArtifactReference::new(
        ArtifactId::new(reference.identity()).map_err(persistence)?,
        ContentDigest::from_hex(reference.digest()).map_err(persistence)?,
        milkdrift_workspace::MediaType::new(reference.media_type().ok_or(
            ContextBuildError::RequiredUnavailable("artifact media type"),
        )?)
        .map_err(persistence)?,
        reference
            .size_bytes()
            .ok_or(ContextBuildError::RequiredUnavailable("artifact byte size"))?,
    ))
}

fn artifact_causes(metadata: &ArtifactMetadata) -> Vec<ContextEvidenceReference> {
    std::iter::once(metadata.provenance().producer())
        .chain(metadata.provenance().causes())
        .cloned()
        .map(|reference| ContextEvidenceReference::Workspace { reference })
        .collect()
}

const fn sensitivity(value: ArtifactSensitivity) -> ContextArtifactSensitivity {
    match value {
        ArtifactSensitivity::Public => ContextArtifactSensitivity::Public,
        ArtifactSensitivity::Internal => ContextArtifactSensitivity::Internal,
        ArtifactSensitivity::Restricted => ContextArtifactSensitivity::Restricted,
    }
}

const fn retention(value: &ArtifactRetention) -> ContextArtifactRetention {
    match value {
        ArtifactRetention::WhileReferenced => ContextArtifactRetention::WhileReferenced,
        ArtifactRetention::Until { .. } => ContextArtifactRetention::Until,
        ArtifactRetention::Indefinite => ContextArtifactRetention::Indefinite,
    }
}

const fn provenance(value: &ArtifactProvenance) -> ContextProvenanceClass {
    match value.producer() {
        CausalReference::RunInput { .. } => ContextProvenanceClass::RunInput,
        CausalReference::WorkspaceValue { .. } => ContextProvenanceClass::WorkspaceValue,
        CausalReference::Artifact { .. } => ContextProvenanceClass::Artifact,
        CausalReference::Invocation { .. } => ContextProvenanceClass::Invocation,
        CausalReference::External { .. } => ContextProvenanceClass::External,
    }
}

fn bounded_name(value: &str) -> String {
    if value.len() <= 255 {
        value.to_owned()
    } else {
        format!("artifact:{}", blake3::hash(value.as_bytes()).to_hex())
    }
}

fn combined_authority(left: AuthorityFact, right: AuthorityFact) -> AuthorityFact {
    let authorized = left.authorized && right.authorized;
    AuthorityFact {
        required: left.required || right.required,
        authorized,
        authority_reference: if !left.authorized {
            left.authority_reference
        } else if !right.authorized {
            right.authority_reference
        } else {
            right.authority_reference.or(left.authority_reference)
        },
    }
}

fn persistence(error: impl std::fmt::Display) -> ContextBuildError {
    ContextBuildError::Persistence(error.to_string())
}

fn candidate_tail_start(through_sequence: u64, maximum_records: u32) -> u64 {
    through_sequence
        .saturating_sub(u64::from(maximum_records).saturating_sub(1))
        .max(1)
}

fn record_subworkflow_parent(
    parents: &mut BTreeMap<SubworkflowId, NodeExecutionId>,
    subworkflow: &SubworkflowId,
    parent_execution: &NodeExecutionId,
) -> Result<(), ContextBuildError> {
    if parents
        .insert(subworkflow.clone(), parent_execution.clone())
        .is_some_and(|existing| existing != *parent_execution)
    {
        return Err(ContextBuildError::Policy(
            "subworkflow identity has conflicting parent provenance".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::candidate_tail_start;

    #[test]
    fn candidate_scan_budget_bounds_the_recent_tail_not_the_lifetime_prefix() {
        assert_eq!(candidate_tail_start(10_000, 4_096), 5_905);
        assert!(9_999 >= candidate_tail_start(10_000, 4_096));
        assert_eq!(candidate_tail_start(512, 4_096), 1);
    }
}
