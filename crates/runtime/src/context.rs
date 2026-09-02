//! Pure deterministic construction of exact causal context manifests.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use milkdrift_blueprint::{
    ContextArtifactRetention, ContextArtifactSensitivity, ContextCategory, ContextProvenanceClass,
    ContextSemanticRole, ContextTruncation, EdgeKind, NodeId, RevisionId, SemanticBlueprint,
    TaskContextPolicy,
};
use milkdrift_model::{
    AuthorityFact, ContextEvidenceReference, ContextInclusionReason, ContextManifest,
    ContextManifestEntry, ContextOmission, ContextOmissionReason, ContextProducerFact,
    ContextSemanticKind, ContextSource, ContextTotals, ModelContractError,
};
use milkdrift_persistence::{AttemptId, NodeExecutionId};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance,
    ArtifactReference as WorkspaceArtifactReference, ArtifactRetention, ArtifactSensitivity,
    CausalId, CausalReference, ContentDigest, MediaType, RunId, ScopeReference, WorkspaceBudget,
    WorkspaceUsage,
};
use thiserror::Error;

mod source;

pub use source::{
    ContextCandidateSource, ContextSourceRequest, DurableContextCandidateSource,
    materialize_selected_context, read_context_manifest,
};

/// Immutable identities frozen into one manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextBuildIdentity {
    /// Owning run.
    pub run: RunId,
    /// Exact governing revision.
    pub revision: RevisionId,
    /// Current semantic task node.
    pub node: NodeId,
    /// Current node execution.
    pub execution: NodeExecutionId,
    /// Frozen attempt.
    pub attempt: AttemptId,
}

/// Metadata-only artifact facts evaluated before any bytes are read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextCandidateArtifactFacts {
    /// Declared artifact name.
    pub name: String,
    /// Exact media type.
    pub media_type: String,
    /// Sensitivity class.
    pub sensitivity: ContextArtifactSensitivity,
    /// Retention class.
    pub retention: ContextArtifactRetention,
    /// Producer provenance class.
    pub provenance: ContextProvenanceClass,
}

/// One bounded metadata candidate from the projection, paged journal, workspace, or inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextCandidate {
    /// Semantic category.
    pub kind: ContextSemanticKind,
    /// Exact immutable source reference.
    pub source: Option<ContextSource>,
    /// Digest of exact bytes that will be loaded only if selected.
    pub content_digest: ContentDigest,
    /// Immutable revision governing the source occurrence.
    pub source_revision: RevisionId,
    /// Exact source execution when execution-backed.
    pub execution: Option<NodeExecutionId>,
    /// Exact source attempt when the evidence is attempt-specific.
    pub attempt: Option<AttemptId>,
    /// Source journal sequence.
    pub source_sequence: Option<milkdrift_persistence::RunSequence>,
    /// Source timestamp when semantically relevant.
    pub occurred_at_ms: Option<u64>,
    /// Causal distance computed under the source occurrence's immutable revision.
    pub causal_distance: Option<u16>,
    /// Exact bounded producer provenance.
    pub producer: ContextProducerFact,
    /// Source node when graph causality applies.
    pub node: Option<NodeId>,
    /// Tagged known roles.
    pub roles: BTreeSet<ContextSemanticRole>,
    /// Source scope when workspace branch isolation applies.
    pub scope: Option<ScopeReference>,
    /// True only when an explicit edge/join/reducer exposed a non-lineage scope.
    pub exposed_across_scope: bool,
    /// Whether policy resolution requires this candidate.
    pub required: bool,
    /// Availability/integrity classification determined without loading large content.
    pub availability: ContextCandidateAvailability,
    /// Estimated small/reference bytes.
    pub selected_bytes: u64,
    /// Exact referenced artifact bytes from metadata.
    pub selected_artifact_bytes: u64,
    /// Optional provider-neutral unit estimate supplied or derived without a tokenizer.
    pub estimated_model_input_units: Option<u64>,
    /// Sensitivity propagated to model output handling.
    pub sensitivity: ArtifactSensitivity,
    /// Authority decision facts, never a secret value.
    pub authority: AuthorityFact,
    /// Optional artifact metadata used by the safe selector.
    pub artifact: Option<ContextCandidateArtifactFacts>,
    /// Exact causal/evidence parents.
    pub causal_parents: Vec<ContextEvidenceReference>,
}

/// Stable metadata-only availability classification used before selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextCandidateAvailability {
    /// Exact source and immutable metadata are available.
    Available,
    /// Source is missing or contradicts its durable reference.
    MissingOrCorrupt,
    /// Source kind cannot be represented by the configured invocation boundary.
    Unsupported,
    /// A later exact fact superseded this optional candidate.
    Superseded,
}

/// Complete immutable input to the pure builder.
pub struct ContextBuildRequest<'a> {
    /// Frozen manifest identities.
    pub identity: ContextBuildIdentity,
    /// Exact semantic revision body matching `identity.revision`.
    pub semantic: &'a SemanticBlueprint,
    /// Immutable task policy under that revision.
    pub policy: &'a TaskContextPolicy,
    /// Root-to-leaf visible workspace scopes plus explicit join imports.
    pub visible_scopes: BTreeSet<ScopeReference>,
    /// Candidates may arrive in any page/chunk order.
    pub candidates: Vec<ContextCandidate>,
}

/// Deterministic pre-dispatch context failure.
#[derive(Debug, Error)]
pub enum ContextBuildError {
    /// Current task or a selected exact node is absent from the governing revision.
    #[error("context policy references missing semantic node {0}")]
    MissingNode(NodeId),
    /// Required evidence could not be resolved.
    #[error("required context is unavailable: {0}")]
    RequiredUnavailable(&'static str),
    /// Required restricted evidence lacked exact authority.
    #[error("required context authority was denied")]
    AuthorityDenied,
    /// Required evidence cannot fit the declared budget.
    #[error("required context exceeds the {0} budget")]
    RequiredBudget(&'static str),
    /// Manifest contract construction failed.
    #[error(transparent)]
    Contract(#[from] ModelContractError),
    /// Policy digest computation failed.
    #[error("context policy digest failed: {0}")]
    Policy(String),
    /// Integer accounting overflowed.
    #[error("context accounting overflow")]
    AccountingOverflow,
    /// Durable manifest publication failed before external dispatch.
    #[error("context manifest persistence failed: {0}")]
    Persistence(String),
}

/// Stateless deterministic context planner.
#[derive(Clone, Copy, Debug, Default)]
pub struct CausalContextBuilder;

impl CausalContextBuilder {
    /// Builds one exact manifest. Stable order is causal depth, semantic kind,
    /// source node, then canonical source-reference bytes.
    pub fn build(request: ContextBuildRequest<'_>) -> Result<ContextManifest, ContextBuildError> {
        if !request
            .semantic
            .nodes()
            .contains_key(&request.identity.node)
        {
            return Err(ContextBuildError::MissingNode(request.identity.node));
        }
        for node in request.policy.selected_nodes() {
            if !request.semantic.nodes().contains_key(node) {
                return Err(ContextBuildError::MissingNode(node.clone()));
            }
        }
        let ancestor_depths = ancestor_depths(
            request.semantic,
            &request.identity.node,
            request.policy.ancestor_depth(),
        );
        let mut ranked = Vec::with_capacity(request.candidates.len());
        for candidate in request.candidates {
            let (eligible, reason, omission) = eligibility(
                request.policy,
                &request.visible_scopes,
                &ancestor_depths,
                &candidate,
            );
            let depth = candidate
                .causal_distance
                .or_else(|| {
                    candidate
                        .node
                        .as_ref()
                        .and_then(|node| ancestor_depths.get(node).copied())
                })
                .unwrap_or(u16::MAX);
            let source_key = candidate
                .source
                .as_ref()
                .map(serde_json::to_vec)
                .transpose()
                .map_err(|error| ModelContractError::Invalid(error.to_string()))?
                .unwrap_or_default();
            ranked.push((
                depth,
                candidate.kind,
                candidate.node.clone(),
                source_key,
                eligible,
                reason,
                omission,
                candidate,
            ));
        }
        ranked.sort_by(|left, right| {
            (&left.0, &left.1, &left.2, &left.3).cmp(&(&right.0, &right.1, &right.2, &right.3))
        });

        let budget = request.policy.budget();
        let mut entries = Vec::new();
        let mut omissions = Vec::new();
        let mut totals = ContextTotals::default();
        let mut stopped = false;
        for (_, _, _, _, eligible, reason, omission_reason, candidate) in ranked {
            if !eligible || stopped {
                if !eligible
                    && candidate.required
                    && request.policy.fail_closed()
                    && exact_source_requested(request.policy, &candidate)
                {
                    return Err(ContextBuildError::RequiredUnavailable(
                        "exact context source is excluded or not visible",
                    ));
                }
                omissions.push(omission(
                    &candidate,
                    if stopped {
                        ContextOmissionReason::SelectionStopped
                    } else {
                        omission_reason.unwrap_or(ContextOmissionReason::NotSelected)
                    },
                ));
                continue;
            }
            if candidate.availability != ContextCandidateAvailability::Available {
                if candidate.required && request.policy.fail_closed() {
                    return Err(ContextBuildError::RequiredUnavailable(
                        "missing exact source",
                    ));
                }
                let reason = match candidate.availability {
                    ContextCandidateAvailability::Available => unreachable!(),
                    ContextCandidateAvailability::MissingOrCorrupt => {
                        ContextOmissionReason::MissingOrCorrupt
                    }
                    ContextCandidateAvailability::Unsupported => ContextOmissionReason::Unsupported,
                    ContextCandidateAvailability::Superseded => ContextOmissionReason::Superseded,
                };
                omissions.push(omission(&candidate, reason));
                continue;
            }
            if candidate.authority.required && !candidate.authority.authorized {
                if candidate.required && request.policy.fail_closed() {
                    return Err(ContextBuildError::AuthorityDenied);
                }
                omissions.push(omission(&candidate, ContextOmissionReason::AuthorityDenied));
                continue;
            }
            let overflow = budget_overflow(budget, totals, &candidate)?;
            if let Some((omission_reason, budget_name)) = overflow {
                if candidate.required && request.policy.fail_closed() {
                    return Err(ContextBuildError::RequiredBudget(budget_name));
                }
                omissions.push(omission(&candidate, omission_reason));
                stopped = request.policy.truncation() == ContextTruncation::StopAtFirstOverflow;
                continue;
            }
            totals.items = totals
                .items
                .checked_add(1)
                .ok_or(ContextBuildError::AccountingOverflow)?;
            totals.bytes = totals
                .bytes
                .checked_add(candidate.selected_bytes)
                .ok_or(ContextBuildError::AccountingOverflow)?;
            totals.artifact_bytes = totals
                .artifact_bytes
                .checked_add(candidate.selected_artifact_bytes)
                .ok_or(ContextBuildError::AccountingOverflow)?;
            let selected_artifact = candidate_is_artifact(&candidate);
            if selected_artifact {
                totals.artifacts = totals
                    .artifacts
                    .checked_add(1)
                    .ok_or(ContextBuildError::AccountingOverflow)?;
            }
            if let Some(units) = candidate.estimated_model_input_units {
                totals.model_input_units = Some(
                    totals
                        .model_input_units
                        .unwrap_or(0)
                        .checked_add(units)
                        .ok_or(ContextBuildError::AccountingOverflow)?,
                );
            }
            let source = candidate.source.ok_or({
                if candidate.required {
                    ContextBuildError::RequiredUnavailable("missing source reference")
                } else {
                    ContextBuildError::RequiredUnavailable("eligible source reference")
                }
            })?;
            let ordinal = totals.items;
            entries.push(ContextManifestEntry::new(
                ordinal,
                candidate.kind,
                candidate.roles,
                source,
                candidate.content_digest,
                candidate.source_revision,
                candidate.execution,
                candidate.attempt,
                candidate.scope,
                candidate.causal_distance,
                candidate.source_sequence,
                candidate.occurred_at_ms,
                candidate.producer,
                candidate.causal_parents,
                selected_artifact,
                candidate.selected_bytes,
                candidate.selected_artifact_bytes,
                candidate.estimated_model_input_units,
                candidate.sensitivity,
                candidate.authority,
                reason.unwrap_or(ContextInclusionReason::IncludedCategory),
            )?);
        }
        let digest = request
            .policy
            .digest()
            .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
        let manifest = ContextManifest::new(
            request.identity.run,
            request.identity.revision,
            request.identity.node,
            request.identity.execution,
            request.identity.attempt,
            1,
            digest,
            entries,
            omissions,
            totals,
            budget,
        )?;
        let encoded =
            milkdrift_model::ContextManifestDocument::new(manifest.clone()).to_canonical_json()?;
        if u64::try_from(encoded.len()).map_or(true, |size| size > budget.max_manifest_bytes) {
            return Err(ContextBuildError::RequiredBudget(
                "serialized manifest byte",
            ));
        }
        Ok(manifest)
    }
}

/// Persists the exact canonical manifest as a restricted immutable artifact before dispatch.
///
/// Publication identity and artifact identity derive from the manifest digest, so retrying the
/// same frozen attempt is idempotent and cannot substitute revised context.
pub fn persist_context_manifest(
    store: &dyn milkdrift_persistence::ArtifactStore,
    manifest: &ContextManifest,
    workspace_budget: WorkspaceBudget,
    expected_usage: WorkspaceUsage,
) -> Result<milkdrift_capability::ArtifactReference, ContextBuildError> {
    use milkdrift_persistence::{
        ArtifactPublicationId, BeginArtifactOutcome, BeginArtifactPublication,
        MAX_ARTIFACT_CHUNK_BYTES,
    };
    let bytes =
        milkdrift_model::ContextManifestDocument::new(manifest.clone()).to_canonical_json()?;
    let identity = format!("context-manifest:{}", manifest.digest().as_str());
    let reference = WorkspaceArtifactReference::new(
        ArtifactId::new(identity.clone())
            .map_err(|error| ContextBuildError::Persistence(error.to_string()))?,
        ContentDigest::for_bytes(&bytes),
        MediaType::new("application/vnd.milkdrift.context-manifest.v2+json")
            .map_err(|error| ContextBuildError::Persistence(error.to_string()))?,
        u64::try_from(bytes.len()).map_err(|_| ContextBuildError::AccountingOverflow)?,
    );
    let provenance = ArtifactProvenance::new(
        CausalReference::External {
            source: CausalId::new(identity)
                .map_err(|error| ContextBuildError::Persistence(error.to_string()))?,
        },
        Vec::new(),
    )
    .map_err(|error| ContextBuildError::Persistence(error.to_string()))?;
    let metadata = ArtifactMetadata::new(
        reference.clone(),
        ArtifactSensitivity::Restricted,
        ArtifactRetention::WhileReferenced,
        provenance,
    )
    .map_err(|error| ContextBuildError::Persistence(error.to_string()))?;
    let publication = ArtifactPublicationId::new(format!(
        "context-publication:{}",
        manifest.digest().as_str()
    ))
    .map_err(|error| ContextBuildError::Persistence(error.to_string()))?;
    let request = BeginArtifactPublication::new(
        publication.clone(),
        manifest.run().clone(),
        metadata,
        workspace_budget,
        expected_usage,
    )
    .map_err(|error| ContextBuildError::Persistence(error.to_string()))?;
    let offset = match store
        .begin_publication(&request)
        .map_err(|error| ContextBuildError::Persistence(error.to_string()))?
    {
        BeginArtifactOutcome::Writable => 0,
        BeginArtifactOutcome::Resumed { next_offset } => next_offset,
        BeginArtifactOutcome::AlreadyCommitted(metadata) => {
            return capability_artifact(metadata.reference());
        }
    };
    let start = usize::try_from(offset).map_err(|_| ContextBuildError::AccountingOverflow)?;
    if start > bytes.len() {
        let _abort = store.abort_publication(&publication);
        return Err(ContextBuildError::Persistence(
            "resumed manifest publication exceeds its exact content size".to_owned(),
        ));
    }
    for (index, chunk) in bytes[start..].chunks(MAX_ARTIFACT_CHUNK_BYTES).enumerate() {
        let chunk_offset = offset
            .checked_add(
                u64::try_from(index)
                    .ok()
                    .and_then(|value| value.checked_mul(MAX_ARTIFACT_CHUNK_BYTES as u64))
                    .ok_or(ContextBuildError::AccountingOverflow)?,
            )
            .ok_or(ContextBuildError::AccountingOverflow)?;
        if let Err(error) = store.write_chunk(&publication, chunk_offset, chunk) {
            let _abort = store.abort_publication(&publication);
            return Err(ContextBuildError::Persistence(error.to_string()));
        }
    }
    let outcome = match store.commit_publication(&publication) {
        Ok(outcome) => outcome,
        Err(error) => {
            let _abort = store.abort_publication(&publication);
            return Err(ContextBuildError::Persistence(error.to_string()));
        }
    };
    capability_artifact(outcome.metadata().reference())
}

fn capability_artifact(
    reference: &WorkspaceArtifactReference,
) -> Result<milkdrift_capability::ArtifactReference, ContextBuildError> {
    milkdrift_capability::ArtifactReference::new(
        reference.artifact().as_str(),
        reference.digest().to_hex(),
        Some(reference.media_type().as_str().to_owned()),
        Some(reference.size_bytes()),
    )
    .map_err(|error| ContextBuildError::Persistence(error.to_string()))
}

fn ancestor_depths(
    semantic: &SemanticBlueprint,
    target: &NodeId,
    maximum: Option<u16>,
) -> BTreeMap<NodeId, u16> {
    let Some(maximum) = maximum else {
        return BTreeMap::new();
    };
    let mut incoming: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
    for edge in semantic.edges().values() {
        if matches!(edge.kind(), EdgeKind::Control | EdgeKind::Data) {
            incoming
                .entry(edge.target_node().clone())
                .or_default()
                .insert(edge.source_node().clone());
        }
    }
    let mut result = BTreeMap::new();
    let mut queue = VecDeque::from([(target.clone(), 0u16)]);
    while let Some((node, depth)) = queue.pop_front() {
        if depth >= maximum {
            continue;
        }
        if let Some(parents) = incoming.get(&node) {
            for parent in parents {
                let next = depth.saturating_add(1);
                if result.get(parent).is_none_or(|known| next < *known) {
                    result.insert(parent.clone(), next);
                    queue.push_back((parent.clone(), next));
                }
            }
        }
    }
    result
}

fn eligibility(
    policy: &TaskContextPolicy,
    visible: &BTreeSet<ScopeReference>,
    ancestors: &BTreeMap<NodeId, u16>,
    candidate: &ContextCandidate,
) -> (
    bool,
    Option<ContextInclusionReason>,
    Option<ContextOmissionReason>,
) {
    let category = category(candidate.kind);
    if policy.exclude_categories().contains(&category) {
        return (false, None, Some(ContextOmissionReason::ExcludedCategory));
    }
    if candidate
        .scope
        .as_ref()
        .is_some_and(|scope| !visible.contains(scope))
        && !candidate.exposed_across_scope
    {
        return (false, None, Some(ContextOmissionReason::BranchIsolated));
    }
    if candidate.kind == ContextSemanticKind::Artifact && !artifact_matches(policy, candidate) {
        return (false, None, Some(ContextOmissionReason::NotSelected));
    }
    if candidate.kind == ContextSemanticKind::DirectInput && policy.include_direct_inputs() {
        return (true, Some(ContextInclusionReason::DirectInput), None);
    }
    if candidate
        .node
        .as_ref()
        .is_some_and(|node| policy.selected_nodes().contains(node))
    {
        return (true, Some(ContextInclusionReason::SelectedNode), None);
    }
    if candidate
        .execution
        .as_ref()
        .is_some_and(|execution| policy.selected_executions().contains(execution.as_str()))
    {
        return (true, Some(ContextInclusionReason::SelectedNode), None);
    }
    if candidate.source.as_ref().is_some_and(|source| {
        policy.explicit_evidence().iter().any(|selector| {
            serde_json::from_str::<ContextSource>(selector)
                .is_ok_and(|selected| selected == *source)
        })
    }) {
        return (true, Some(ContextInclusionReason::ExplicitEvidence), None);
    }
    if candidate.source.as_ref().is_some_and(|source| {
        let reference = match source {
            ContextSource::WorkspaceValue { reference } => Some(reference),
            _ => None,
        };
        reference.is_some_and(|reference| {
            serde_json::to_string(reference)
                .is_ok_and(|reference| policy.selected_workspace_values().contains(&reference))
        })
    }) || candidate.causal_parents.iter().any(|parent| {
        let ContextEvidenceReference::Workspace {
            reference: CausalReference::WorkspaceValue { reference },
        } = parent
        else {
            return false;
        };
        serde_json::to_string(reference)
            .is_ok_and(|reference| policy.selected_workspace_values().contains(&reference))
    }) {
        return (true, Some(ContextInclusionReason::IncludedCategory), None);
    }
    if candidate
        .roles
        .iter()
        .any(|role| policy.selected_roles().contains(role))
    {
        return (true, Some(ContextInclusionReason::SelectedRole), None);
    }
    if candidate.kind != ContextSemanticKind::DirectInput
        && policy.ancestor_depth().is_some()
        && (candidate
            .causal_distance
            .is_some_and(|distance| distance > 0)
            || (candidate.causal_distance.is_none()
                && candidate
                    .node
                    .as_ref()
                    .is_some_and(|node| ancestors.contains_key(node))))
    {
        return (true, Some(ContextInclusionReason::CausalAncestor), None);
    }
    if policy.include_categories().contains(&category) {
        if candidate.kind == ContextSemanticKind::Artifact {
            return (true, Some(ContextInclusionReason::ArtifactSelector), None);
        }
        return (true, Some(ContextInclusionReason::IncludedCategory), None);
    }
    (false, None, Some(ContextOmissionReason::NotSelected))
}

fn artifact_matches(policy: &TaskContextPolicy, candidate: &ContextCandidate) -> bool {
    let Some(selector) = policy.artifact_selector() else {
        return true;
    };
    let Some(facts) = &candidate.artifact else {
        return false;
    };
    (selector.names().is_empty() || selector.names().contains(&facts.name))
        && (selector.media_types().is_empty() || selector.media_types().contains(&facts.media_type))
        && (selector.sensitivities().is_empty()
            || selector.sensitivities().contains(&facts.sensitivity))
        && (selector.retentions().is_empty() || selector.retentions().contains(&facts.retention))
        && (selector.provenance().is_empty() || selector.provenance().contains(&facts.provenance))
}

fn exact_source_requested(policy: &TaskContextPolicy, candidate: &ContextCandidate) -> bool {
    candidate
        .execution
        .as_ref()
        .is_some_and(|execution| policy.selected_executions().contains(execution.as_str()))
        || candidate.source.as_ref().is_some_and(|source| {
            policy.explicit_evidence().iter().any(|selector| {
                serde_json::from_str::<ContextSource>(selector)
                    .is_ok_and(|selected| selected == *source)
            })
        })
        || candidate.source.as_ref().is_some_and(|source| {
            let ContextSource::WorkspaceValue { reference } = source else {
                return false;
            };
            serde_json::to_string(reference)
                .is_ok_and(|reference| policy.selected_workspace_values().contains(&reference))
        })
        || candidate.causal_parents.iter().any(|parent| {
            let ContextEvidenceReference::Workspace {
                reference: CausalReference::WorkspaceValue { reference },
            } = parent
            else {
                return false;
            };
            serde_json::to_string(reference)
                .is_ok_and(|reference| policy.selected_workspace_values().contains(&reference))
        })
}

fn budget_overflow(
    budget: milkdrift_blueprint::ContextBudget,
    totals: ContextTotals,
    candidate: &ContextCandidate,
) -> Result<Option<(ContextOmissionReason, &'static str)>, ContextBuildError> {
    let item_bytes = candidate
        .selected_bytes
        .checked_add(candidate.selected_artifact_bytes)
        .ok_or(ContextBuildError::AccountingOverflow)?;
    if item_bytes > budget.max_per_item_bytes {
        return Ok(Some((
            ContextOmissionReason::PerItemByteBudget,
            "per-item byte",
        )));
    }
    if totals
        .items
        .checked_add(1)
        .ok_or(ContextBuildError::AccountingOverflow)?
        > budget.max_items
    {
        return Ok(Some((ContextOmissionReason::ItemBudget, "item")));
    }
    if candidate_is_artifact(candidate)
        && totals
            .artifacts
            .checked_add(1)
            .ok_or(ContextBuildError::AccountingOverflow)?
            > budget.max_artifacts
    {
        return Ok(Some((
            ContextOmissionReason::ArtifactItemBudget,
            "artifact item",
        )));
    }
    if totals
        .bytes
        .checked_add(candidate.selected_bytes)
        .ok_or(ContextBuildError::AccountingOverflow)?
        > budget.max_bytes
    {
        return Ok(Some((ContextOmissionReason::ByteBudget, "byte")));
    }
    if totals
        .artifact_bytes
        .checked_add(candidate.selected_artifact_bytes)
        .ok_or(ContextBuildError::AccountingOverflow)?
        > budget.max_artifact_bytes
    {
        return Ok(Some((
            ContextOmissionReason::ArtifactByteBudget,
            "artifact-byte",
        )));
    }
    if let Some(max) = budget.max_model_input_units {
        let used = totals.model_input_units.unwrap_or(0);
        if used
            .checked_add(candidate.estimated_model_input_units.unwrap_or(0))
            .ok_or(ContextBuildError::AccountingOverflow)?
            > max
        {
            return Ok(Some((
                ContextOmissionReason::ModelInputUnitBudget,
                "model-input-unit",
            )));
        }
    }
    Ok(None)
}

fn candidate_is_artifact(candidate: &ContextCandidate) -> bool {
    candidate.artifact.is_some()
        || candidate.kind == ContextSemanticKind::Artifact
        || matches!(
            candidate.source.as_ref(),
            Some(ContextSource::DirectInput {
                reference: milkdrift_capability::InvocationValueReference::Artifact { .. },
                ..
            })
        )
}

fn omission(candidate: &ContextCandidate, reason: ContextOmissionReason) -> ContextOmission {
    let redacted = matches!(
        reason,
        ContextOmissionReason::BranchIsolated | ContextOmissionReason::AuthorityDenied
    );
    ContextOmission {
        source: (!redacted).then(|| candidate.source.clone()).flatten(),
        kind: candidate.kind,
        reason,
        required: candidate.required,
        omitted_bytes: if redacted {
            0
        } else {
            candidate.selected_bytes
        },
        omitted_artifact_bytes: if redacted {
            0
        } else {
            candidate.selected_artifact_bytes
        },
    }
}

const fn category(kind: ContextSemanticKind) -> ContextCategory {
    match kind {
        ContextSemanticKind::DirectInput => ContextCategory::DirectInput,
        ContextSemanticKind::SuccessfulOutput => ContextCategory::SuccessfulOutput,
        ContextSemanticKind::Failure => ContextCategory::Failure,
        ContextSemanticKind::Decision => ContextCategory::Decision,
        ContextSemanticKind::Artifact => ContextCategory::Artifact,
        ContextSemanticKind::RawProgress => ContextCategory::RawProgress,
        ContextSemanticKind::ToolTrace => ContextCategory::ToolTrace,
        ContextSemanticKind::VerboseCommandOutput => ContextCategory::VerboseCommandOutput,
        ContextSemanticKind::PriorPrompt => ContextCategory::PriorPrompt,
        ContextSemanticKind::FinalOutput => ContextCategory::FinalOutput,
    }
}
