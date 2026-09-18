//! Bounded authenticated input publication through the ordinary artifact owner.
use super::{
    Owner, PublicFailure,
    read_model::{invalid, public_artifact_metadata, public_persistence, unauthorized_decision},
};
use crate::auth::ActorSession;
use milkdrift_authority::{AuthorityBudget, AuthorityOperation, RequestedResourceFacts};
use milkdrift_control_protocol::{ArtifactMetadataRead, ErrorCode, InputUploadRequest};
use milkdrift_persistence::{ArtifactPublicationId, ArtifactStore, BeginArtifactPublication};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactOwner, ArtifactProvenance, ArtifactReference,
    ArtifactRetention, ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType,
};

impl Owner {
    pub(super) fn upload_input(
        &mut self,
        session: &ActorSession,
        request: &InputUploadRequest,
    ) -> Result<ArtifactMetadataRead, PublicFailure> {
        if self.recovery_controls || request.host != self.host_id.as_str() {
            return Err(invalid(
                "upload requires this exact serving host in normal mode",
            ));
        }
        let upload =
            CausalId::new(&request.upload_id).map_err(|error| invalid(&error.to_string()))?;
        let client =
            CausalId::new(session.actor.as_str()).map_err(|error| invalid(&error.to_string()))?;
        let owner = ArtifactOwner::ClientInput {
            host: self.host_id.clone(),
            client: client.clone(),
        };
        let identity = serde_json::to_vec(&(&owner, &upload))
            .map_err(|_| invalid("upload identity could not be encoded"))?;
        let digest = blake3::hash(&identity);
        let artifact = ArtifactId::new(format!("input:{digest}"))
            .map_err(|error| invalid(&error.to_string()))?;
        let sensitivity = match request.sensitivity.as_str() {
            "restricted" => ArtifactSensitivity::Restricted,
            "internal" => ArtifactSensitivity::Internal,
            "public" => ArtifactSensitivity::Public,
            _ => return Err(invalid("unsupported artifact sensitivity")),
        };
        let bytes = request
            .content()
            .map_err(|error| invalid(&error.to_string()))?;
        let mut resources = RequestedResourceFacts::empty();
        resources.artifact = Some(artifact.clone());
        resources.artifact_sensitivity = Some(sensitivity);
        let decision = self.evaluate_authority_with_budget(
            session,
            AuthorityOperation::PublishArtifact,
            resources.clone(),
            AuthorityBudget {
                artifact_bytes: Some(bytes.len() as u64),
                ..AuthorityBudget::default()
            },
            "publish:client-input",
        )?;
        if !decision.is_allowed() {
            return Err(unauthorized_decision(&decision));
        }
        self.authorize(
            session,
            AuthorityOperation::ReadArtifactMetadata,
            resources,
            "read:uploaded-input",
        )?;
        let metadata = ArtifactMetadata::new(
            ArtifactReference::new(
                artifact.clone(),
                ContentDigest::for_bytes(&bytes),
                MediaType::new(&request.media_type).map_err(|error| invalid(&error.to_string()))?,
                bytes.len() as u64,
            ),
            sensitivity,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::ClientUpload {
                    host: self.host_id.clone(),
                    client: client.clone(),
                    upload,
                },
                Vec::new(),
            )
            .map_err(|error| invalid(&error.to_string()))?,
        )
        .map_err(|error| invalid(&error.to_string()))?;
        if let Some(existing) = self.store.metadata(&artifact).map_err(public_persistence)? {
            if existing != metadata {
                return Err(PublicFailure::new(
                    ErrorCode::Conflict,
                    "upload identity already names different content or metadata",
                    false,
                ));
            }
            if !self
                .store
                .is_committed(metadata.reference())
                .map_err(public_persistence)?
            {
                return Err(PublicFailure::new(
                    ErrorCode::Corruption,
                    "uploaded artifact is not completely committed",
                    false,
                ));
            }
            self.record_security_decision(&decision)?;
            return Ok(public_artifact_metadata(&existing));
        }
        let usage = self
            .store
            .artifact_usage(&owner)
            .map_err(public_persistence)?;
        let publication =
            ArtifactPublicationId::new(format!("input:{digest}")).map_err(public_persistence)?;
        let begin = BeginArtifactPublication::for_client_input(
            publication.clone(),
            self.host_id.clone(),
            client,
            metadata,
            self.input_budget.clone(),
            usage,
        )
        .map_err(public_persistence)?;
        self.record_security_decision(&decision)?;
        let mut cleanup = UploadCleanup {
            store: self.store.as_ref(),
            publication: &publication,
            committed: false,
        };
        let state = self
            .store
            .begin_publication(&begin)
            .map_err(public_persistence)?;
        let offset = usize::try_from(state.next_offset().unwrap_or(0))
            .map_err(|_| invalid("upload offset exceeds platform"))?;
        let remaining = bytes
            .get(offset..)
            .ok_or_else(|| invalid("upload offset exceeds exact content"))?;
        if !remaining.is_empty() {
            self.store
                .write_chunk(&publication, offset as u64, remaining)
                .map_err(public_persistence)?;
        }
        let outcome = self
            .store
            .commit_publication(&publication)
            .map_err(public_persistence)?;
        cleanup.committed = true;
        Ok(public_artifact_metadata(outcome.metadata()))
    }
}

// Also runs during panic unwinding. Crash leftovers use the store's bounded orphan cleanup.
struct UploadCleanup<'a> {
    store: &'a dyn ArtifactStore,
    publication: &'a ArtifactPublicationId,
    committed: bool,
}
impl Drop for UploadCleanup<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.store.abort_publication(self.publication);
        }
    }
}
