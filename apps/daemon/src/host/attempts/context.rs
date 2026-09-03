//! Context-manifest authorization, verification, and bounded public attachment.

use super::{LocatedAttempt, Owner};
use crate::host::{
    ActorSession, ArtifactId, ArtifactReadAuthority, ArtifactStore, AuthorityOperation,
    ContextManifestRead, ErrorCode, EvidenceId, PublicFailure, RequestedResourceFacts,
    RevisionStore, internal, invalid, not_found, parse_revision_id, public_persistence,
};

impl Owner {
    pub(super) fn attach_context(
        &mut self,
        session: &ActorSession,
        attempt: &str,
        located: &mut LocatedAttempt,
    ) -> Result<(), PublicFailure> {
        let Some(reference) = located.value.context_manifest.as_ref() else {
            return Ok(());
        };
        let artifact = ArtifactId::new(reference.artifact_id.clone())
            .map_err(|error| invalid(&error.to_string()))?;
        crate::host::artifacts::preauthorize_artifact_identity(
            session,
            &artifact,
            AuthorityOperation::ReadArtifactContent,
        )?;
        let metadata = self
            .store
            .metadata(&artifact)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let mut resources = RequestedResourceFacts::empty();
        resources.artifact = Some(artifact);
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        let decision = self.evaluate_authority(
            session,
            AuthorityOperation::ReadArtifactContent,
            resources,
            "read:attempt-context-manifest",
        )?;
        self.record_security_decision(&decision)?;
        if !decision.is_allowed() {
            located.value.context_access = "denied".to_owned();
            return Ok(());
        }
        let reference = milkdrift_capability::ArtifactReference::new(
            reference.artifact_id.clone(),
            reference.digest.clone(),
            Some(reference.content_type.clone()),
            Some(reference.size),
        )
        .map_err(|error| invalid(&error.to_string()))?;
        let manifest = milkdrift_runtime::read_context_manifest(
            self.store.as_ref(),
            &reference,
            ArtifactReadAuthority::Authorized {
                actor: session.actor.clone(),
                evidence: EvidenceId::new(format!("attempt-context:{}", &decision.digest()[..32]))
                    .map_err(public_persistence)?,
            },
        )
        .map_err(|error| PublicFailure::new(ErrorCode::Corruption, error.to_string(), false))?;
        if manifest.attempt().as_str() != attempt {
            return Err(PublicFailure::new(
                ErrorCode::Corruption,
                "context manifest is bound to another attempt",
                false,
            ));
        }
        let revision = parse_revision_id(&located.revision_id)?;
        let stored = self
            .store
            .revision(&revision)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let policy = stored
            .semantic()
            .nodes()
            .get(
                &milkdrift_blueprint::NodeId::new(located.node_id.clone())
                    .map_err(|error| invalid(&error.to_string()))?,
            )
            .and_then(|node| match node.kind() {
                milkdrift_blueprint::NodeKind::Task { config } => Some(config.context_policy()),
                _ => None,
            })
            .ok_or_else(not_found)?;
        const MAX_CONTEXT_READ_ITEMS: usize = 256;
        let truncated = manifest.entries().len() > MAX_CONTEXT_READ_ITEMS
            || manifest.omissions().len() > MAX_CONTEXT_READ_ITEMS;
        located.value.context = Some(ContextManifestRead {
            schema_version: manifest.schema_version(),
            digest: manifest.digest().as_str().to_owned(),
            policy: serde_json::to_value(policy).map_err(|_| internal())?,
            entries: manifest
                .entries()
                .iter()
                .take(MAX_CONTEXT_READ_ITEMS)
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()
                .map_err(|_| internal())?,
            omissions: manifest
                .omissions()
                .iter()
                .take(MAX_CONTEXT_READ_ITEMS)
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()
                .map_err(|_| internal())?,
            totals: serde_json::to_value(manifest.totals()).map_err(|_| internal())?,
            budget: serde_json::to_value(manifest.budget()).map_err(|_| internal())?,
            truncated,
        });
        located.value.context_access = "authorized".to_owned();
        Ok(())
    }
}
