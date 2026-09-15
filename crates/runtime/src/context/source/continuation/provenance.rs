//! Journal-backed producer checks and publication of the resolved conversation selection.

use milkdrift_capability::{ArtifactReference, InvocationRequest};
use milkdrift_model::{ContextManifest, ContinuationHistory, ContinuationTurn};
use milkdrift_persistence::{
    EventCursor, EventPageQuery, NodeOutcome, PageSize, RunEventKind, RunSequence,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactRetention, ArtifactSensitivity,
    CausalId, CausalReference, ContentDigest, MediaType,
};
use std::collections::BTreeSet;

use super::{
    ContextBuildError, ContextSourceRequest, DurableContextCandidateSource, persistence,
    unavailable, workspace_artifact,
};

impl DurableContextCandidateSource<'_> {
    pub(super) fn find_continuation_turn(
        &self,
        request: &ContextSourceRequest<'_>,
        manifest: &ContextManifest,
        reference: &ArtifactReference,
        response: &ArtifactReference,
    ) -> Result<ContinuationTurn, ContextBuildError> {
        let maximum = u64::from(request.policy.budget().max_candidate_records);
        let start = request
            .through_sequence
            .get()
            .saturating_sub(maximum.saturating_sub(1))
            .max(1);
        let mut next = RunSequence::new(start);
        let mut events = [RunSequence::ZERO; 5];
        while next <= request.through_sequence {
            let page = self
                .store
                .events(
                    &EventPageQuery::new(
                        request.identity.run.clone(),
                        Some(EventCursor {
                            run: request.identity.run.clone(),
                            next_sequence: next,
                        }),
                        PageSize::new(256).map_err(persistence)?,
                    )
                    .map_err(persistence)?,
                )
                .map_err(persistence)?;
            if page.events.is_empty() {
                return Err(unavailable("predecessor journal is incomplete"));
            }
            for event in page.events {
                if event.sequence() > request.through_sequence {
                    break;
                }
                if event.sequence() != next {
                    return Err(unavailable("predecessor journal is not contiguous"));
                }
                let slot = match event.kind() {
                    RunEventKind::NodeBecameEligible { execution, .. }
                        if execution == manifest.execution() =>
                    {
                        Some(0)
                    }
                    RunEventKind::NodeScheduled { attempt, .. }
                        if attempt == manifest.attempt() =>
                    {
                        Some(1)
                    }
                    RunEventKind::CapabilityResolutionDecisionRecorded { attempt, .. }
                        if attempt == manifest.attempt() =>
                    {
                        Some(2)
                    }
                    RunEventKind::NodeOutputPublished {
                        attempt,
                        artifact: Some(artifact),
                        ..
                    } if attempt == manifest.attempt()
                        && artifact == &workspace_artifact(response)? =>
                    {
                        Some(3)
                    }
                    RunEventKind::NodeTerminal { attempt, .. } if attempt == manifest.attempt() => {
                        Some(4)
                    }
                    _ => None,
                };
                if let Some(slot) = slot {
                    events[slot] = event.sequence();
                }
                next = next.next().map_err(persistence)?;
            }
        }
        if events.contains(&RunSequence::ZERO) {
            return Err(unavailable(
                "predecessor proof is outside the bounded journal discovery window",
            ));
        }
        Ok(ContinuationTurn {
            manifest: reference.clone(),
            response: response.clone(),
            events,
            message_end: 0,
        })
    }

    pub(super) fn verify_continuation_turn(
        &self,
        request: &ContextSourceRequest<'_>,
        manifest: &ContextManifest,
        turn: &ContinuationTurn,
    ) -> Result<InvocationRequest, ContextBuildError> {
        if turn.response.media_type() != Some("application/vnd.milkdrift.model-response.v1+json") {
            return Err(unavailable("unsupported predecessor response family"));
        }
        let mut facts = Vec::with_capacity(5);
        for sequence in turn.events {
            facts.push(super::super::event_at(
                self.store,
                &request.identity.run,
                sequence,
                request.through_sequence,
            )?);
        }
        let RunEventKind::NodeBecameEligible {
            execution,
            node,
            scope,
            ..
        } = facts[0].kind()
        else {
            return Err(unavailable("predecessor eligibility proof missing"));
        };
        if execution != manifest.execution() || node != manifest.node() {
            return Err(unavailable("predecessor scope identity mismatch"));
        }
        let visible: BTreeSet<_> = self
            .store
            .scope_lineage(request.scope)
            .map_err(persistence)?
            .into_iter()
            .map(|scope| scope.reference().clone())
            .collect();
        if !visible.contains(scope) {
            return Err(unavailable("continuation predecessor is branch-private"));
        }
        // Exact references do not make unrelated work causal. The current revision must
        // expose the producing node through an incoming control/data path.
        let ancestors = super::super::super::ancestor_depths(
            request.revision.semantic(),
            &request.identity.node,
            Some(u16::MAX),
        );
        if !ancestors.contains_key(manifest.node()) {
            return Err(unavailable(
                "continuation predecessor is not causal in the current revision",
            ));
        }
        let RunEventKind::NodeScheduled {
            execution,
            attempt,
            node,
            request: invocation,
            ..
        } = facts[1].kind()
        else {
            return Err(unavailable("predecessor scheduling proof missing"));
        };
        if execution != manifest.execution()
            || attempt != manifest.attempt()
            || node != manifest.node()
            || invocation.context_manifest() != Some(&turn.manifest)
            || invocation.operation().as_str() != milkdrift_model::MODEL_GENERATE_OPERATION
        {
            return Err(unavailable(
                "predecessor invocation and manifest do not match",
            ));
        }
        let RunEventKind::CapabilityResolutionDecisionRecorded {
            execution,
            attempt,
            snapshot,
            authorization,
        } = facts[2].kind()
        else {
            return Err(unavailable("predecessor authority proof missing"));
        };
        if attempt != manifest.attempt()
            || execution != manifest.execution()
            || snapshot.category().is_some_and(|category| {
                category != &milkdrift_capability::CapabilityCategory::Model
            })
            || !authorization.is_allowed()
            || &authorization.request().actor != request.authority.actor()
            || &authorization.request().grant != request.authority.grant()
            || authorization.request().grant_revision != request.authority.grant_revision()
            || &authorization.request().grant_digest != request.authority.grant_digest()
            || authorization.request().provenance.revision.as_ref() != Some(manifest.revision())
            || authorization.request().provenance.node.as_ref() != Some(manifest.node())
        {
            return Err(unavailable("predecessor actor or authority mismatch"));
        }
        let RunEventKind::NodeOutputPublished {
            execution,
            attempt,
            artifact: Some(artifact),
            value,
            ..
        } = facts[3].kind()
        else {
            return Err(unavailable("predecessor response publication missing"));
        };
        if execution != manifest.execution()
            || attempt != manifest.attempt()
            || artifact != &workspace_artifact(&turn.response)?
            || value.scope() != scope
        {
            return Err(unavailable(
                "predecessor response publication does not match",
            ));
        }
        let RunEventKind::NodeTerminal {
            execution,
            attempt,
            outcome: NodeOutcome::Succeeded,
            ..
        } = facts[4].kind()
        else {
            return Err(unavailable("predecessor invocation did not succeed"));
        };
        if execution != manifest.execution()
            || attempt != manifest.attempt()
            || facts[4].sequence() <= facts[3].sequence()
        {
            return Err(unavailable("predecessor terminal does not match"));
        }
        let metadata = self
            .store
            .metadata(artifact.artifact())
            .map_err(persistence)?
            .ok_or(unavailable("predecessor response metadata missing"))?;
        if metadata.provenance().producer()
            != &(CausalReference::Invocation {
                invocation: invocation.invocation().clone(),
            })
            || !metadata
                .provenance()
                .causes()
                .contains(&CausalReference::Artifact {
                    reference: workspace_artifact(&turn.manifest)?,
                })
        {
            return Err(unavailable(
                "response producer or manifest linkage mismatch",
            ));
        }
        self.continuation_acceptance(request, turn.events[1], invocation)?;
        Ok(invocation.clone())
    }

    fn continuation_acceptance(
        &self,
        request: &ContextSourceRequest<'_>,
        terminal: RunSequence,
        producer: &InvocationRequest,
    ) -> Result<(), ContextBuildError> {
        use milkdrift_capability::{
            ACCEPTED_RESULT_OUTPUT, InvocationValueReference, RESULT_ACCEPTANCE_SUBJECT_INPUT,
            WORKFLOW_ACCEPT_RESULT_OPERATION,
        };
        if request
            .through_sequence
            .get()
            .saturating_sub(terminal.get())
            > u64::from(request.policy.budget().max_candidate_records)
        {
            return Err(ContextBuildError::RequiredBudget(
                "continuation acceptance discovery",
            ));
        }
        let mut evaluations = std::collections::BTreeMap::new();
        let mut next = terminal;
        while next <= request.through_sequence {
            let page = self
                .store
                .events(
                    &EventPageQuery::new(
                        request.identity.run.clone(),
                        Some(EventCursor {
                            run: request.identity.run.clone(),
                            next_sequence: next,
                        }),
                        PageSize::new(256).map_err(persistence)?,
                    )
                    .map_err(persistence)?,
                )
                .map_err(persistence)?;
            if page.events.is_empty() {
                return Err(unavailable("acceptance history missing"));
            }
            for event in page.events {
                if event.sequence() > request.through_sequence {
                    break;
                }
                if event.sequence() != next {
                    return Err(unavailable("acceptance history is not contiguous"));
                }
                match event.kind() {
                    RunEventKind::NodeScheduled {
                        attempt,
                        request: invocation,
                        ..
                    } if invocation.operation().as_str() == WORKFLOW_ACCEPT_RESULT_OPERATION => {
                        let subject = invocation
                            .inputs()
                            .iter()
                            .find(|input| input.name() == RESULT_ACCEPTANCE_SUBJECT_INPUT)
                            .map(|input| input.value());
                        let artifact = match subject {
                            Some(InvocationValueReference::Artifact { reference }) => {
                                Some(ArtifactId::new(reference.identity()).map_err(persistence)?)
                            }
                            Some(InvocationValueReference::WorkspaceValue {
                                identity,
                                version,
                            }) => {
                                let reference: milkdrift_workspace::WorkspaceValueReference =
                                    serde_json::from_str(identity).map_err(persistence)?;
                                if version != &reference.version().get().to_string() {
                                    return Err(unavailable("acceptance subject version mismatch"));
                                }
                                self.store
                                    .value(&reference)
                                    .map_err(persistence)?
                                    .ok_or(unavailable("acceptance subject missing"))?
                                    .value()
                                    .as_artifact()
                                    .map(|reference| reference.artifact().clone())
                            }
                            _ => None,
                        };
                        if let Some(artifact) = artifact {
                            let metadata = self.store.metadata(&artifact).map_err(persistence)?;
                            if metadata.is_some_and(|metadata| {
                                metadata.provenance().producer()
                                    == &(CausalReference::Invocation {
                                        invocation: producer.invocation().clone(),
                                    })
                            }) {
                                evaluations.insert(attempt.clone(), (false, false));
                            }
                        }
                    }
                    RunEventKind::NodeOutputPublished { attempt, value, .. }
                        if value.key().as_str() == ACCEPTED_RESULT_OUTPUT =>
                    {
                        if let Some(state) = evaluations.get_mut(attempt) {
                            state.0 = true;
                        }
                    }
                    RunEventKind::NodeTerminal {
                        attempt, outcome, ..
                    } => {
                        if let Some(state) = evaluations.get_mut(attempt) {
                            state.1 = *outcome == NodeOutcome::Succeeded;
                        }
                    }
                    _ => {}
                }
                next = next.next().map_err(persistence)?;
            }
        }
        if evaluations
            .values()
            .any(|&(accepted, completed)| !accepted || !completed)
        {
            return Err(unavailable(
                "prior result acceptance is rejected or incomplete; use explicit failed evidence for remediation",
            ));
        }
        Ok(())
    }

    pub(super) fn publish_continuation(
        &self,
        request: &ContextSourceRequest<'_>,
        manifest: ContextManifest,
        history: &ContinuationHistory,
        bytes: &[u8],
    ) -> Result<ContextManifest, ContextBuildError> {
        use milkdrift_model::{
            AuthorityFact, CONTINUATION_MEDIA_TYPE, ContextEvidenceReference,
            ContextInclusionReason, ContextManifestEntry, ContextProducerFact, ContextSemanticKind,
            ContextSource,
        };
        let identity = format!("continuation:{}", blake3::hash(bytes));
        let artifact = milkdrift_workspace::ArtifactReference::new(
            ArtifactId::new(identity.clone()).map_err(persistence)?,
            ContentDigest::for_bytes(bytes),
            MediaType::new(CONTINUATION_MEDIA_TYPE).map_err(persistence)?,
            bytes.len() as u64,
        );
        let causes = history
            .turns()
            .iter()
            .flat_map(|turn| [&turn.manifest, &turn.response])
            .map(|reference| {
                workspace_artifact(reference)
                    .map(|reference| CausalReference::Artifact { reference })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let metadata = ArtifactMetadata::new(
            artifact.clone(),
            ArtifactSensitivity::Restricted,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::External {
                    source: CausalId::new(identity.clone()).map_err(persistence)?,
                },
                causes.clone(),
            )
            .map_err(persistence)?,
        )
        .map_err(persistence)?;
        let mut totals = manifest.totals();
        totals.items = totals
            .items
            .checked_add(1)
            .ok_or(ContextBuildError::AccountingOverflow)?;
        totals.artifacts = totals
            .artifacts
            .checked_add(1)
            .ok_or(ContextBuildError::AccountingOverflow)?;
        totals.artifact_bytes = totals
            .artifact_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(ContextBuildError::AccountingOverflow)?;
        if bytes.len() as u64 > request.policy.budget().max_per_item_bytes {
            return Err(ContextBuildError::RequiredBudget("continuation artifact"));
        }
        if request.policy.budget().max_model_input_units.is_some() {
            return Err(unavailable(
                "continuation has no configured model-input unit estimator",
            ));
        }
        let mut entries = manifest.entries().to_vec();
        entries.push(ContextManifestEntry::new(
            entries.len() as u32 + 1,
            ContextSemanticKind::PriorPrompt,
            BTreeSet::new(),
            ContextSource::Artifact {
                reference: artifact.clone(),
            },
            artifact.digest(),
            request.identity.revision.clone(),
            Some(request.identity.execution.clone()),
            Some(request.identity.attempt.clone()),
            Some(request.scope.clone()),
            None,
            Some(request.through_sequence),
            None,
            ContextProducerFact {
                actor: Some(request.authority.actor().as_str().to_owned()),
                ..ContextProducerFact::default()
            },
            causes
                .into_iter()
                .map(|reference| ContextEvidenceReference::Workspace { reference })
                .collect(),
            true,
            0,
            bytes.len() as u64,
            None,
            ArtifactSensitivity::Restricted,
            AuthorityFact {
                required: true,
                authorized: true,
                authority_reference: Some(request.authority.accepted_decision_digest().to_owned()),
            },
            ContextInclusionReason::Continuation,
        )?);
        let result = ContextManifest::new(
            manifest.run().clone(),
            manifest.revision().clone(),
            manifest.node().clone(),
            manifest.execution().clone(),
            manifest.attempt().clone(),
            manifest.policy_version(),
            manifest.policy_digest().clone(),
            entries,
            manifest.omissions().to_vec(),
            totals,
            manifest.budget(),
        )?;
        if milkdrift_model::ContextManifestDocument::new(result.clone())
            .to_canonical_json()?
            .len() as u64
            > manifest.budget().max_manifest_bytes
        {
            return Err(ContextBuildError::RequiredBudget("manifest byte"));
        }
        let budget = request
            .projection
            .workspace_budget()
            .ok_or(unavailable("continuation workspace budget missing"))?;
        super::super::super::publish_context_artifact(
            self.store,
            &request.identity.run,
            metadata,
            milkdrift_persistence::ArtifactPublicationId::new(identity).map_err(persistence)?,
            bytes,
            budget.clone(),
            self.store
                .workspace_usage(&request.identity.run)
                .map_err(persistence)?,
        )?;
        Ok(result)
    }
}
