//! Authenticate the actual structured response and its selected context before admitting a lesson.
use super::{
    ActorSession, CommandRequest, LearningDeclaration, Owner, PublicFailure, authorize_artifact,
    internal, invalid, not_found, public_control, public_persistence,
};
use milkdrift_control::{
    ControlCommand, ControlResult, ProposalProvenance, WorkflowProposal, WorkflowProposalDocument,
};
use milkdrift_persistence::{ArtifactStore, PageSize, RunEventKind, RunSequence};
use milkdrift_workspace::ArtifactReference;

pub(super) fn bytes(
    owner: &mut Owner,
    session: &ActorSession,
    reference: &ArtifactReference,
) -> Result<Vec<u8>, PublicFailure> {
    if reference.size_bytes() > milkdrift_model::MAX_MODEL_DOCUMENT_BYTES as u64 {
        return Err(invalid("learning artifact exceeds document bound"));
    }
    authorize_artifact(owner, session, reference)?;
    let mut bytes = Vec::new();
    while bytes.len() as u64 != reference.size_bytes() {
        let chunk = owner.artifact_range(
            session,
            reference.artifact().as_str(),
            bytes.len() as u64,
            (reference.size_bytes() - bytes.len() as u64).min(16_384) as u32,
            "learning-model",
        )?;
        if chunk.bytes.is_empty() {
            return Err(invalid("learning artifact made no read progress"));
        }
        bytes.extend(chunk.bytes);
    }
    if !reference.verifies(&bytes) {
        return Err(invalid("learning artifact integrity differs"));
    }
    Ok(bytes)
}
fn reference(
    owner: &Owner,
    reference: &milkdrift_capability::ArtifactReference,
) -> Result<ArtifactReference, PublicFailure> {
    let id = milkdrift_workspace::ArtifactId::new(reference.identity())
        .map_err(|e| invalid(&e.to_string()))?;
    let metadata = owner
        .store
        .metadata(&id)
        .map_err(public_persistence)?
        .ok_or_else(not_found)?;
    let actual = metadata.reference();
    if actual.digest().to_string() != reference.digest()
        || Some(actual.size_bytes()) != reference.size_bytes()
        || Some(actual.media_type().as_str()) != reference.media_type()
    {
        return Err(invalid("proposal artifact differs from retained reference"));
    }
    Ok(actual.clone())
}

#[allow(clippy::too_many_arguments)] // Check the independent declaration and selected source boundary together.
pub(super) fn verify(
    owner: &mut Owner,
    session: &ActorSession,
    request: &CommandRequest,
    proposal: &WorkflowProposal,
    selection: &ArtifactReference,
    sources: &super::KnowledgeSelection,
    declaration: &LearningDeclaration,
    declared_at: u64,
) -> Result<(), PublicFailure> {
    let ProposalProvenance::Model {
        capability,
        invocation,
        model_profile,
        context_manifest,
        response_artifact,
    } = proposal.provenance()
    else {
        return Err(invalid(
            "learning candidate requires model response provenance",
        ));
    };
    let manifest_ref = reference(owner, context_manifest)?;
    let manifest =
        milkdrift_model::ContextManifestDocument::from_json(&bytes(owner, session, &manifest_ref)?)
            .map_err(|e| invalid(&e.to_string()))?;
    let manifest = manifest.body();
    if manifest.run() != &declaration.proposal_run {
        return Err(invalid("proposal did not use the predeclared model run"));
    }
    let response_ref = reference(owner, response_artifact)?;
    let response =
        milkdrift_model::ModelResponseDocument::from_json(&bytes(owner, session, &response_ref)?)
            .map_err(|e| invalid(&e.to_string()))?;
    let actual =
        WorkflowProposalDocument::from_model_response(response.body()).map_err(public_control)?;
    // The producer cannot predict its eventual output artifact reference. The ordinary caller
    // may attach authenticated provenance and identities, but cannot replace the model's mutation,
    // rationale, evidence, risks, assumptions, scope, requested actions or application policy.
    let semantic = |p: &WorkflowProposal| -> Result<serde_json::Value, PublicFailure> {
        let mut value = serde_json::to_value(p).map_err(|_| internal())?;
        let object = value.as_object_mut().ok_or_else(internal)?;
        for key in ["identity", "proposer", "provenance", "digest"] {
            object.remove(key);
        }
        Ok(value)
    };
    if semantic(actual.proposal())? != semantic(proposal)? {
        return Err(invalid(
            "candidate differs from the actual structured model proposal",
        ));
    }
    let allowed = sources
        .artifacts
        .iter()
        .chain([&sources.guidance, selection])
        .map(|a| a.artifact().as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let held_out = declaration
        .pairs
        .iter()
        .map(|pair| pair.input.artifact().as_str())
        .collect();
    verify_sources(
        manifest.entries().iter().map(|entry| entry.source()),
        allowed,
        held_out,
    )?;
    let mut next = Some(RunSequence::FIRST);
    let (mut resolved, mut scheduled, mut output) = (false, false, false);
    for _ in 0..64 {
        let result = owner.execute_control_result(
            session,
            request,
            None,
            None,
            ControlCommand::InspectTimeline {
                run: manifest.run().clone(),
                after: next,
                limit: PageSize::new(64).map_err(public_persistence)?,
            },
            "learning-model-history",
        )?;
        let ControlResult::Timeline { value: page } = result else {
            return Err(internal());
        };
        for event in &page.events {
            match event.kind() {
                RunEventKind::CapabilityResolved {
                    attempt, snapshot, ..
                } if attempt == manifest.attempt() => {
                    resolved = snapshot.capability() == capability
                        && snapshot.provider_profile() == Some(model_profile)
                        && snapshot.category() == &milkdrift_capability::CapabilityCategory::Model;
                }
                RunEventKind::NodeScheduled {
                    attempt, request, ..
                } if attempt == manifest.attempt() => {
                    scheduled = request.invocation() == invocation
                        && request.context_manifest() == Some(context_manifest)
                        && event.occurred_at().get() >= declared_at;
                }
                RunEventKind::NodeOutputPublished {
                    attempt,
                    artifact: Some(artifact),
                    ..
                } if attempt == manifest.attempt() => {
                    output |= artifact == &response_ref;
                }
                _ => {}
            }
        }
        if resolved && scheduled && output {
            return Ok(());
        }
        next = page.next_sequence;
        if next.is_none() {
            break;
        }
    }
    Err(invalid(
        "no exact post-declaration model entry and structured output in the bounded journal selection",
    ))
}

fn verify_sources<'a>(
    entries: impl Iterator<Item = &'a milkdrift_model::ContextSource>,
    allowed: std::collections::BTreeSet<&str>,
    held_out: std::collections::BTreeSet<&str>,
) -> Result<(), PublicFailure> {
    let mut selected = std::collections::BTreeSet::new();
    let mut materialized = std::collections::BTreeSet::new();
    for source in entries {
        if matches!(
            source,
            milkdrift_model::ContextSource::NodeExecution { .. }
                | milkdrift_model::ContextSource::Event { .. }
                | milkdrift_model::ContextSource::WorkspaceValue { .. }
                | milkdrift_model::ContextSource::DirectInput {
                    reference: milkdrift_capability::InvocationValueReference::WorkspaceValue { .. },
                    ..
                }
        ) {
            return Err(invalid(
                "learning history and workspace context must come through the frozen selected artifact boundary",
            ));
        }
        if let milkdrift_model::ContextSource::Artifact { reference } = source {
            selected.insert(reference.artifact().as_str().to_owned());
            materialized.insert(reference.artifact().as_str());
        }
        if let milkdrift_model::ContextSource::DirectInput {
            reference: milkdrift_capability::InvocationValueReference::Artifact { reference },
            ..
        } = source
        {
            selected.insert(reference.identity().to_owned());
        }
    }
    if held_out.iter().any(|id| selected.contains(*id)) {
        return Err(invalid(
            "proposal context contains a held-out evaluation input",
        ));
    }
    if selected.iter().any(|id| !allowed.contains(id.as_str())) {
        return Err(invalid(
            "proposal context selected an artifact outside the declared source boundary",
        ));
    }
    if allowed.iter().any(|id| !materialized.contains(id)) {
        return Err(invalid(
            "model context must materialize every selected source artifact; direct input metadata alone is insufficient",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::verify_sources;
    use milkdrift_model::ContextSource;
    use milkdrift_workspace::{ArtifactId, ArtifactReference, ContentDigest, MediaType};

    type Result = std::result::Result<(), Box<dyn std::error::Error>>;

    fn source(id: &str) -> std::result::Result<ContextSource, Box<dyn std::error::Error>> {
        Ok(ContextSource::Artifact {
            reference: ArtifactReference::new(
                ArtifactId::new(id)?,
                ContentDigest::for_bytes(id.as_bytes()),
                MediaType::new("text/plain")?,
                id.len() as u64,
            ),
        })
    }

    #[test]
    fn selected_sources_require_materialized_content() -> Result {
        let source = source("source")?;
        let ContextSource::Artifact { reference } = &source else {
            unreachable!()
        };
        let metadata: ContextSource = serde_json::from_value(serde_json::json!({
            "type":"direct_input", "name":"source", "reference":{
                "type":"artifact", "reference":{
                    "identity":reference.artifact(), "digest":reference.digest(),
                    "media_type":reference.media_type(), "size_bytes":reference.size_bytes()
                }
            }
        }))?;
        assert!(verify_sources([&metadata].into_iter(), ["source"].into(), [].into()).is_err());
        assert!(
            verify_sources(
                [&source, &metadata].into_iter(),
                ["source"].into(),
                [].into()
            )
            .is_ok()
        );
        assert!(
            verify_sources(
                [&source].into_iter(),
                ["source", "missing"].into(),
                [].into()
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn held_out_and_unselected_content_are_refused() -> Result {
        let source = source("source")?;
        let other = self::source("other")?;
        assert!(
            verify_sources([&source, &other].into_iter(), ["source"].into(), [].into()).is_err()
        );
        assert!(
            verify_sources([&source].into_iter(), ["source"].into(), ["source"].into()).is_err()
        );
        let event = serde_json::from_value(
            serde_json::json!({"type":"event","event":"event:outside-source","sequence":1}),
        )?;
        assert!(
            verify_sources([&source, &event].into_iter(), ["source"].into(), [].into()).is_err()
        );
        Ok(())
    }
}
