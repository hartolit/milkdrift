//! Direct invocation-input candidate discovery.

use milkdrift_capability::InvocationValueReference;

use super::{
    ArtifactSensitivity, AuthorityFact, BTreeSet, ContentDigest, ContextBuildError,
    ContextCandidate, ContextCandidateAvailability, ContextProducerFact, ContextSemanticKind,
    ContextSource, ContextSourceRequest, DurableContextCandidateSource, InputReference,
    WorkspaceValueReference, combined_authority, workspace_artifact,
};

impl DurableContextCandidateSource<'_> {
    pub(super) fn direct_candidate(
        &self,
        request: &ContextSourceRequest<'_>,
        input: &InputReference,
    ) -> Result<ContextCandidate, ContextBuildError> {
        let (
            digest,
            selected_bytes,
            artifact_bytes,
            sensitivity,
            authority,
            availability,
            artifact,
        ) = match input.value() {
            InvocationValueReference::Inline { value } => {
                let bytes = serde_json::to_vec(value.value())
                    .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
                (
                    ContentDigest::for_bytes(&bytes),
                    u64::try_from(bytes.len())
                        .map_err(|_| ContextBuildError::AccountingOverflow)?,
                    0,
                    ArtifactSensitivity::Restricted,
                    AuthorityFact {
                        required: false,
                        authorized: true,
                        authority_reference: Some(
                            request.authority.accepted_decision_digest().to_owned(),
                        ),
                    },
                    ContextCandidateAvailability::Available,
                    None,
                )
            }
            InvocationValueReference::Artifact { reference } => {
                let reference = workspace_artifact(reference)?;
                let facts = self.artifact_facts(request, &reference, input.name())?;
                (
                    facts.content_digest,
                    facts.selected_bytes,
                    facts.artifact_bytes,
                    facts.sensitivity,
                    facts.authority,
                    facts.availability,
                    facts.selector,
                )
            }
            InvocationValueReference::WorkspaceValue { identity, version } => {
                let reference: WorkspaceValueReference = serde_json::from_str(identity)
                    .map_err(|_| ContextBuildError::RequiredUnavailable("workspace input"))?;
                if version != &reference.version().get().to_string() {
                    return Err(ContextBuildError::RequiredUnavailable(
                        "workspace input version",
                    ));
                }
                let workspace = self.workspace_facts(request, &reference)?;
                if let Some(reference) = workspace.artifact {
                    let artifact = self.artifact_facts(request, &reference, input.name())?;
                    (
                        artifact.content_digest,
                        artifact.selected_bytes,
                        artifact.artifact_bytes,
                        artifact.sensitivity,
                        combined_authority(workspace.authority, artifact.authority),
                        artifact.availability,
                        artifact.selector,
                    )
                } else {
                    (
                        workspace.content_digest,
                        workspace.selected_bytes,
                        0,
                        workspace.sensitivity,
                        workspace.authority,
                        workspace.availability,
                        None,
                    )
                }
            }
        };
        Ok(ContextCandidate {
            kind: ContextSemanticKind::DirectInput,
            source: Some(ContextSource::DirectInput {
                name: input.name().to_owned(),
                reference: input.value().clone(),
            }),
            content_digest: digest,
            source_revision: request.identity.revision.clone(),
            execution: Some(request.identity.execution.clone()),
            attempt: Some(request.identity.attempt.clone()),
            source_sequence: None,
            occurred_at_ms: None,
            causal_distance: None,
            producer: ContextProducerFact::default(),
            node: Some(request.identity.node.clone()),
            roles: BTreeSet::new(),
            scope: Some(request.scope.clone()),
            exposed_across_scope: false,
            required: request.required_direct_inputs.contains(input.name()),
            availability,
            selected_bytes,
            selected_artifact_bytes: artifact_bytes,
            estimated_model_input_units: None,
            sensitivity,
            authority,
            artifact,
            causal_parents: Vec::new(),
        })
    }
}
