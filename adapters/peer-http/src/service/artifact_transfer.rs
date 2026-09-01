//! Authorized peer artifact negotiation, transfer, and disabled-store behavior.

use milkdrift_authority::{AuthorityBudget, AuthorityOperation, PeerId, RequestedResourceFacts};
use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision, ArtifactTransferDirection,
    TransferId,
};
use milkdrift_persistence::PeerExecutionSnapshot;

use super::{PeerHttpError, PeerService, map_execution_persistence};
use crate::artifact::{PeerArtifactError, PeerArtifactStore};

impl PeerService {
    /// Negotiates a metadata-first authorized upload or download.
    pub fn negotiate_artifact(
        &self,
        authenticated_peer: &PeerId,
        offer: &ArtifactMetadataOffer,
    ) -> Result<ArtifactTransferDecision, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        let operation = match offer.direction {
            ArtifactTransferDirection::Upload => AuthorityOperation::PeerArtifactUpload,
            ArtifactTransferDirection::Download => AuthorityOperation::PeerArtifactDownload,
        };
        let snapshot = self
            .executions
            .peer_execution(authenticated_peer, &offer.execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| {
                PeerHttpError::Unauthorized(
                    "artifact is not bound to an execution owned by this peer".to_owned(),
                )
            })?;
        let record = match snapshot {
            PeerExecutionSnapshot::Hot(record) => record,
            PeerExecutionSnapshot::Archived(_)
                if offer.direction == ArtifactTransferDirection::Download =>
            {
                return Err(PeerHttpError::NotFound(
                    "archived execution observation-to-artifact history was compacted; core artifact retention is unchanged"
                        .to_owned(),
                ));
            }
            PeerExecutionSnapshot::Archived(tombstone) => {
                return Err(PeerHttpError::Unauthorized(format!(
                    "artifact upload cannot target archived execution {}",
                    tombstone.execution
                )));
            }
        };
        if offer.direction == ArtifactTransferDirection::Download {
            if offer.source_peer != self.config.local_peer {
                return Err(PeerHttpError::Unauthorized(
                    "download source is not the serving peer".to_owned(),
                ));
            }
            let mut produced = false;
            for sequence in 1..=record.last_observation_sequence {
                if self
                    .executions
                    .peer_observation_artifact(&record.execution, sequence)
                    .map_err(map_execution_persistence)?
                    .as_ref()
                    .is_some_and(|artifact| {
                        workspace_artifact_matches_capability(&offer.artifact, artifact)
                    })
                {
                    produced = true;
                    break;
                }
            }
            if !produced {
                return Err(PeerHttpError::Unauthorized(
                    "artifact is not a durable output of the claimed execution".to_owned(),
                ));
            }
        }
        self.require_operation(
            &relationship,
            operation,
            artifact_resource_facts(&offer.artifact, offer.sensitivity),
            AuthorityBudget {
                artifact_bytes: Some(offer.artifact.size_bytes()),
                ..AuthorityBudget::default()
            },
        )?;
        self.check_rate(
            &relationship,
            match offer.direction {
                ArtifactTransferDirection::Upload => "artifact_upload_negotiate",
                ArtifactTransferDirection::Download => "artifact_download_negotiate",
            },
        )?;
        if self.clock.now_unix_ms() > offer.expires_at_unix_ms {
            return Err(PeerHttpError::Unauthorized(
                "artifact transfer authority expired".to_owned(),
            ));
        }
        self.artifacts
            .negotiate(
                authenticated_peer,
                offer,
                relationship.maximum_artifact_bytes,
            )
            .map_err(Into::into)
    }

    /// Accepts one sequential bounded artifact chunk.
    pub fn write_artifact_chunk(
        &self,
        authenticated_peer: &PeerId,
        chunk: &ArtifactChunk,
    ) -> Result<ArtifactTransferDecision, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        let facts = self
            .artifacts
            .transfer_facts(authenticated_peer, &chunk.transfer)?;
        if facts.direction != ArtifactTransferDirection::Upload {
            return Err(PeerHttpError::Unauthorized(
                "artifact transfer direction is not upload".to_owned(),
            ));
        }
        self.require_operation(
            &relationship,
            AuthorityOperation::PeerArtifactUpload,
            artifact_resource_facts(&facts.artifact, facts.sensitivity),
            AuthorityBudget {
                artifact_bytes: Some(u64::try_from(chunk.bytes.len()).unwrap_or(u64::MAX)),
                ..AuthorityBudget::default()
            },
        )?;
        self.check_rate(&relationship, "artifact_upload_chunk")?;
        self.artifacts
            .write_chunk(
                authenticated_peer,
                chunk,
                self.config.limits.artifact_chunk_bytes,
            )
            .map_err(Into::into)
    }

    /// Returns one authorized verified artifact range.
    pub fn read_artifact_chunk(
        &self,
        authenticated_peer: &PeerId,
        transfer: &TransferId,
        offset: u64,
        maximum_bytes: u32,
    ) -> Result<ArtifactChunk, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        let facts = self
            .artifacts
            .transfer_facts(authenticated_peer, transfer)?;
        if facts.direction != ArtifactTransferDirection::Download {
            return Err(PeerHttpError::Unauthorized(
                "artifact transfer direction is not download".to_owned(),
            ));
        }
        self.require_operation(
            &relationship,
            AuthorityOperation::PeerArtifactDownload,
            artifact_resource_facts(&facts.artifact, facts.sensitivity),
            AuthorityBudget {
                artifact_bytes: Some(u64::from(maximum_bytes)),
                ..AuthorityBudget::default()
            },
        )?;
        self.check_rate(&relationship, "artifact_download_chunk")?;
        self.artifacts
            .read_chunk(
                authenticated_peer,
                transfer,
                offset,
                maximum_bytes.min(self.config.limits.artifact_chunk_bytes),
            )
            .map_err(Into::into)
    }

    /// Aborts an incomplete artifact transfer and removes temporary bytes.
    pub fn abort_artifact(
        &self,
        authenticated_peer: &PeerId,
        transfer: &TransferId,
    ) -> Result<(), PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        let facts = self
            .artifacts
            .transfer_facts(authenticated_peer, transfer)?;
        let operation = match facts.direction {
            ArtifactTransferDirection::Upload => AuthorityOperation::PeerArtifactUpload,
            ArtifactTransferDirection::Download => AuthorityOperation::PeerArtifactDownload,
        };
        self.require_operation(
            &relationship,
            operation,
            artifact_resource_facts(&facts.artifact, facts.sensitivity),
            AuthorityBudget::default(),
        )?;
        self.check_rate(&relationship, "artifact_abort")?;
        self.artifacts.abort(authenticated_peer, transfer)?;
        Ok(())
    }
}

fn artifact_resource_facts(
    artifact: &milkdrift_workspace::ArtifactReference,
    sensitivity: milkdrift_workspace::ArtifactSensitivity,
) -> RequestedResourceFacts {
    let mut resources = RequestedResourceFacts::empty();
    resources.artifact = Some(artifact.artifact().clone());
    resources.artifact_sensitivity = Some(sensitivity);
    resources
}

fn workspace_artifact_matches_capability(
    workspace: &milkdrift_workspace::ArtifactReference,
    capability: &milkdrift_capability::ArtifactReference,
) -> bool {
    capability.identity() == workspace.artifact().as_str()
        && capability.digest() == workspace.digest().to_string()
        && capability.media_type() == Some(workspace.media_type().as_str())
        && capability.size_bytes() == Some(workspace.size_bytes())
}

impl From<PeerArtifactError> for PeerHttpError {
    fn from(error: PeerArtifactError) -> Self {
        match error {
            PeerArtifactError::Rejected(message) => Self::Unauthorized(message),
            PeerArtifactError::Conflict(message) | PeerArtifactError::Verification(message) => {
                Self::Protocol(message)
            }
            PeerArtifactError::Persistence(message) => Self::Persistence(message),
            PeerArtifactError::Overloaded(message) => Self::Overloaded(message),
            PeerArtifactError::Unavailable => {
                Self::Persistence("artifact state unavailable".to_owned())
            }
        }
    }
}

pub(super) struct DisabledArtifactStore;

impl PeerArtifactStore for DisabledArtifactStore {
    fn transfer_facts(
        &self,
        _owner_peer: &PeerId,
        _transfer: &TransferId,
    ) -> Result<crate::PeerArtifactTransferFacts, PeerArtifactError> {
        Err(PeerArtifactError::Rejected(
            "peer artifact transfer is disabled".to_owned(),
        ))
    }

    fn negotiate(
        &self,
        _owner_peer: &PeerId,
        _offer: &ArtifactMetadataOffer,
        _maximum_artifact_bytes: u64,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError> {
        Err(PeerArtifactError::Rejected(
            "peer artifact storage is not configured".to_owned(),
        ))
    }

    fn write_chunk(
        &self,
        _owner_peer: &PeerId,
        _chunk: &ArtifactChunk,
        _maximum_chunk_bytes: u32,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError> {
        Err(PeerArtifactError::Rejected(
            "peer artifact storage is not configured".to_owned(),
        ))
    }

    fn read_chunk(
        &self,
        _owner_peer: &PeerId,
        _transfer: &TransferId,
        _offset: u64,
        _maximum_bytes: u32,
    ) -> Result<ArtifactChunk, PeerArtifactError> {
        Err(PeerArtifactError::Rejected(
            "peer artifact storage is not configured".to_owned(),
        ))
    }

    fn abort(&self, _owner_peer: &PeerId, _transfer: &TransferId) -> Result<(), PeerArtifactError> {
        Ok(())
    }
}
