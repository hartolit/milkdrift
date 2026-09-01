//! Authoritative metadata discovery and selected-only context materialization.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::{AuthorityEvaluator, ExecutionAuthorityBasis};
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
    WorkspaceValueReference,
};

use super::{
    ContextBuildError, ContextBuildIdentity, ContextCandidate, ContextCandidateArtifactFacts,
    ContextCandidateAvailability, ancestor_depths,
};

const SOURCE_PAGE_SIZE: u32 = 256;

mod candidate;
mod direct;
mod discovery;
mod explicit;
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
