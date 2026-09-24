//! Authorized peer artifact negotiation, transfer, and disabled-store behavior.

use milkdrift_authority::{AuthorityBudget, AuthorityOperation, RequestedResourceFacts};
use milkdrift_capability::PeerId;
use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision, ArtifactTransferDirection,
    TransferId,
};
use milkdrift_persistence::PeerExecutionSnapshot;

use super::artifact::{PeerArtifactError, PeerArtifactStore};
use super::{PeerService, ServingError, map_execution_persistence};

impl PeerService {
    /// Offers the exact artifact at one durable output observation to its authenticated owner.
    /// Metadata requires the same download authority as subsequent chunks.
    pub fn output_artifact_offer(
        &self,
        authenticated_peer: &PeerId,
        execution: &milkdrift_peer_protocol::PeerExecutionId,
        sequence: u64,
    ) -> Result<ArtifactMetadataOffer, ServingError> {
        let relationship = self.relationship(authenticated_peer)?;
        let snapshot = self
            .executions
            .peer_execution(&self.peer_caller(authenticated_peer), execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| ServingError::NotFound("execution output unavailable".to_owned()))?;
        self.require_execution_operation(
            &relationship,
            &snapshot,
            AuthorityOperation::InspectPeerExecution,
        )?;
        let (reference, deadline) = match &snapshot {
            PeerExecutionSnapshot::Hot(record) => (
                self.executions
                    .peer_observation_artifact(execution, sequence)
                    .map_err(map_execution_persistence)?,
                record.request.deadline_unix_ms,
            ),
            PeerExecutionSnapshot::Archived(record) => (
                record
                    .output_observations
                    .iter()
                    .find(|output| output.sequence == sequence)
                    .and_then(|output| output.event.kind().output())
                    .map(|(_, reference)| reference.clone()),
                record
                    .authorization
                    .delegation()
                    .map_or(0, |authorization| authorization.expires_at_unix_ms),
            ),
        };
        let reference = reference
            .ok_or_else(|| ServingError::NotFound("execution output unavailable".to_owned()))?;
        let metadata = self.artifacts.metadata(&reference)?;
        self.require_operation(
            &relationship,
            AuthorityOperation::PeerArtifactDownload,
            artifact_resource_facts(metadata.reference(), metadata.sensitivity()),
            AuthorityBudget {
                artifact_bytes: Some(metadata.reference().size_bytes()),
                ..AuthorityBudget::default()
            },
        )?;
        self.check_rate(&relationship, "artifact_metadata")?;
        let identity = serde_json::to_vec(&(authenticated_peer, execution, sequence))
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        Ok(ArtifactMetadataOffer {
            transfer: TransferId::new(format!("transfer:{}", blake3::hash(&identity)))
                .map_err(|error| ServingError::Protocol(error.to_string()))?,
            direction: ArtifactTransferDirection::Download,
            artifact: metadata.reference().clone(),
            sensitivity: metadata.sensitivity(),
            retention: metadata.retention().clone(),
            provenance: metadata.provenance().clone(),
            source_peer: self.config.local_peer.clone(),
            binding: milkdrift_peer_protocol::ArtifactTransferBinding::Execution {
                execution: execution.clone(),
            },
            expires_at_unix_ms: deadline.min(relationship.expires_at_unix_ms),
        })
    }

    /// Negotiates a metadata-first authorized upload or download.
    pub fn negotiate_artifact(
        &self,
        authenticated_peer: &PeerId,
        offer: &ArtifactMetadataOffer,
    ) -> Result<ArtifactTransferDecision, ServingError> {
        let relationship = self.relationship(authenticated_peer)?;
        offer
            .validate()
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        if let milkdrift_peer_protocol::ArtifactTransferBinding::Input { request } = &offer.binding
        {
            if offer.direction != ArtifactTransferDirection::Upload
                || &offer.source_peer != authenticated_peer
            {
                return Err(ServingError::Unauthorized(
                    "request-bound input staging requires an authenticated upload".to_owned(),
                ));
            }
            let now = self.now()?;
            if self.drain_state() != milkdrift_peer_protocol::DrainState::Ready
                || now > request.deadline_unix_ms
            {
                return Err(ServingError::Unavailable(
                    "input staging is draining or its request expired".to_owned(),
                ));
            }
            let catalog = self.catalog(authenticated_peer)?;
            if catalog.generation != request.catalog_generation
                || catalog.digest != request.catalog_digest
            {
                return Err(ServingError::Unauthorized(
                    "input staging requires the current exact catalog".to_owned(),
                ));
            }
            let generation = self.exact_generation(&relationship, request)?;
            self.authorize_invocation(
                &relationship,
                request,
                &generation.descriptor,
                &generation.authority_requirements,
                now,
            )?;
            self.require_operation(
                &relationship,
                AuthorityOperation::PeerArtifactUpload,
                artifact_resource_facts(&offer.artifact, offer.sensitivity),
                AuthorityBudget {
                    artifact_bytes: Some(offer.artifact.size_bytes()),
                    ..AuthorityBudget::default()
                },
            )?;
            self.check_rate(&relationship, "input_upload_negotiate")?;
            return self
                .artifacts
                .negotiate(
                    authenticated_peer,
                    offer,
                    relationship.maximum_artifact_bytes,
                    None,
                )
                .map_err(Into::into);
        }
        let operation = match offer.direction {
            ArtifactTransferDirection::Upload => AuthorityOperation::PeerArtifactUpload,
            ArtifactTransferDirection::Download => AuthorityOperation::PeerArtifactDownload,
        };
        let snapshot = self
            .executions
            .peer_execution(
                &self.peer_caller(authenticated_peer),
                offer.binding.execution().ok_or_else(|| {
                    ServingError::Protocol("execution transfer lacks an execution".to_owned())
                })?,
            )
            .map_err(map_execution_persistence)?
            .ok_or_else(|| {
                ServingError::Unauthorized(
                    "artifact is not bound to an execution owned by this peer".to_owned(),
                )
            })?;
        self.require_execution_operation(
            &relationship,
            &snapshot,
            AuthorityOperation::InspectPeerExecution,
        )?;
        if offer.direction == ArtifactTransferDirection::Download {
            if offer.source_peer != self.config.local_peer {
                return Err(ServingError::Unauthorized(
                    "download source is not the serving peer".to_owned(),
                ));
            }
            let produced = match &snapshot {
                PeerExecutionSnapshot::Archived(record) => record
                    .output_observations
                    .iter()
                    .filter_map(|output| output.event.kind().output())
                    .any(|(_, reference)| {
                        workspace_artifact_matches_capability(&offer.artifact, reference)
                    }),
                PeerExecutionSnapshot::Hot(record) => {
                    let mut produced = false;
                    for sequence in 1..=record.last_observation_sequence {
                        if self
                            .executions
                            .peer_observation_artifact(&record.execution, sequence)
                            .map_err(map_execution_persistence)?
                            .as_ref()
                            .is_some_and(|reference| {
                                workspace_artifact_matches_capability(&offer.artifact, reference)
                            })
                        {
                            produced = true;
                            break;
                        }
                    }
                    produced
                }
            };
            if !produced {
                return Err(ServingError::Unauthorized(
                    "artifact is not a durable output of the claimed execution".to_owned(),
                ));
            }
        } else if matches!(snapshot, PeerExecutionSnapshot::Archived(_)) {
            return Err(ServingError::Unauthorized(
                "artifact upload cannot target an archived execution".to_owned(),
            ));
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
        if self.now()? > offer.expires_at_unix_ms {
            return Err(ServingError::Unauthorized(
                "artifact transfer authority expired".to_owned(),
            ));
        }
        self.artifacts
            .negotiate(
                authenticated_peer,
                offer,
                relationship.maximum_artifact_bytes,
                None,
            )
            .map_err(Into::into)
    }

    /// Accepts one sequential bounded artifact chunk.
    pub fn write_artifact_chunk(
        &self,
        authenticated_peer: &PeerId,
        chunk: &ArtifactChunk,
    ) -> Result<ArtifactTransferDecision, ServingError> {
        let relationship = self.relationship(authenticated_peer)?;
        let facts = self
            .artifacts
            .transfer_facts(authenticated_peer, &chunk.transfer)?;
        if facts.direction != ArtifactTransferDirection::Upload {
            return Err(ServingError::Unauthorized(
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
    ) -> Result<ArtifactChunk, ServingError> {
        let relationship = self.relationship(authenticated_peer)?;
        let facts = self
            .artifacts
            .transfer_facts(authenticated_peer, transfer)?;
        if facts.direction != ArtifactTransferDirection::Download {
            return Err(ServingError::Unauthorized(
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
    ) -> Result<(), ServingError> {
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

impl From<PeerArtifactError> for ServingError {
    fn from(error: PeerArtifactError) -> Self {
        match error {
            PeerArtifactError::Rejected(message) => Self::Unauthorized(message),
            PeerArtifactError::Conflict(message) | PeerArtifactError::Verification(message) => {
                Self::Protocol(message)
            }
            PeerArtifactError::Persistence(message) => Self::Persistence(message),
            PeerArtifactError::Overloaded(message) => Self::Overloaded(message),
            PeerArtifactError::Unavailable => {
                Self::Unavailable("artifact transfer boundary unavailable".to_owned())
            }
        }
    }
}

pub(crate) struct DisabledArtifactStore;

impl PeerArtifactStore for DisabledArtifactStore {
    fn read_input_chunk(
        &self,
        _request: &milkdrift_persistence::ArtifactReadRequest,
    ) -> Result<milkdrift_persistence::ArtifactReadChunk, PeerArtifactError> {
        Err(PeerArtifactError::Rejected(
            "artifact transfer is disabled".to_owned(),
        ))
    }
    fn metadata(
        &self,
        _reference: &milkdrift_capability::ArtifactReference,
    ) -> Result<milkdrift_workspace::ArtifactMetadata, PeerArtifactError> {
        Err(PeerArtifactError::Rejected(
            "artifact transfer is disabled".to_owned(),
        ))
    }

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
        _controller: Option<&milkdrift_persistence::ControllerArtifactOwner>,
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
