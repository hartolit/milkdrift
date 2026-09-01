//! Authorized artifact metadata and bounded content-release ownership.

use super::*;

impl Owner {
    pub(super) fn artifact_metadata(
        &mut self,
        session: &ActorSession,
        artifact: &str,
    ) -> Result<ArtifactMetadataRead, PublicFailure> {
        let artifact =
            ArtifactId::new(artifact.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        preauthorize_artifact_identity(
            session,
            &artifact,
            AuthorityOperation::ReadArtifactMetadata,
        )?;
        let metadata = self
            .store
            .metadata(&artifact)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        if !session
            .grant
            .resources()
            .artifacts
            .sensitivities()
            .is_some_and(|allowed| allowed.contains(&metadata.sensitivity()))
        {
            return Err(not_found());
        }
        let mut resources = RequestedResourceFacts::empty();
        resources.artifact = Some(artifact);
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        let decision = self.authorize(
            session,
            AuthorityOperation::ReadArtifactMetadata,
            resources,
            "read:artifact-metadata",
        )?;
        if metadata.sensitivity() != ArtifactSensitivity::Public {
            self.record_security_decision(&decision)?;
        }
        Ok(public_artifact_metadata(&metadata))
    }

    pub(super) fn artifact_range(
        &mut self,
        session: &ActorSession,
        artifact: &str,
        offset: u64,
        maximum: u32,
        evidence: &str,
    ) -> Result<ArtifactContentRead, PublicFailure> {
        let artifact_id =
            ArtifactId::new(artifact.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        preauthorize_artifact_identity(
            session,
            &artifact_id,
            AuthorityOperation::ReadArtifactContent,
        )?;
        let metadata = self
            .store
            .metadata(&artifact_id)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        if !session
            .grant
            .resources()
            .artifacts
            .sensitivities()
            .is_some_and(|allowed| allowed.contains(&metadata.sensitivity()))
        {
            return Err(not_found());
        }
        let mut resources = RequestedResourceFacts::empty();
        resources.artifact = Some(artifact_id);
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        let decision = self.authorize(
            session,
            AuthorityOperation::ReadArtifactContent,
            resources,
            "read:artifact-content",
        )?;
        let authority = ArtifactReadAuthority::Authorized {
            actor: session.actor.clone(),
            evidence: EvidenceId::new(format!("{evidence}-{}", decision.digest()))
                .map_err(public_persistence)?,
        };
        let chunk = self
            .store
            .read_chunk(
                &ArtifactReadRequest::new(metadata.reference().clone(), offset, maximum, authority)
                    .map_err(public_persistence)?,
            )
            .map_err(public_persistence)?;
        self.record_security_decision(&decision)?;
        Ok(ArtifactContentRead {
            metadata: public_artifact_metadata(&metadata),
            offset: chunk.offset,
            bytes: chunk.bytes,
            end: chunk.end_of_artifact,
        })
    }
}

pub(super) fn preauthorize_artifact_identity(
    session: &ActorSession,
    artifact: &ArtifactId,
    operation: AuthorityOperation,
) -> Result<(), PublicFailure> {
    let scope = &session.grant.resources().artifacts;
    if !session.grant.operations().contains(&operation)
        || !scope
            .identity_selection()
            .is_some_and(|selection| selection.matches(artifact))
    {
        return Err(unauthorized());
    }
    Ok(())
}
