use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use milkdrift_authority::{ActorRef, PeerId};
use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision, ArtifactTransferDirection,
    TransferId,
};
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactReadAuthority, ArtifactReadRequest, ArtifactStore,
    BeginArtifactOutcome, BeginArtifactPublication, EvidenceId, WorkspaceStore,
};
use milkdrift_workspace::{
    ArtifactMetadata, ArtifactProvenance, CausalId, CausalReference, RunId, WorkspaceBudget,
    WorkspaceUsage,
};
use thiserror::Error;

const MAX_ACTIVE_TRANSFERS: usize = 1_024;

/// Safe core-publication or authorized-read failure for peer artifact transfer.
#[derive(Debug, Error)]
pub enum PeerArtifactError {
    /// Authority, metadata, content type, or quota rejected the transfer.
    #[error("peer artifact transfer rejected: {0}")]
    Rejected(String),
    /// Transfer identity or exact metadata conflicted.
    #[error("peer artifact transfer conflict: {0}")]
    Conflict(String),
    /// Sequential offset, size, or digest did not verify.
    #[error("peer artifact verification failed: {0}")]
    Verification(String),
    /// Core persistence failed without disclosing a host path.
    #[error("peer artifact core storage unavailable: {0}")]
    Persistence(String),
    /// Internal bounded transfer state is unavailable.
    #[error("peer artifact transfer state is unavailable")]
    Unavailable,
}

/// Narrow verified artifact transfer port. Durable bytes always belong to the core artifact port.
pub trait PeerArtifactStore: Send + Sync {
    /// Negotiates exact metadata before any bytes, returning deduplication or resume state.
    fn negotiate(
        &self,
        owner_peer: &PeerId,
        offer: &ArtifactMetadataOffer,
        maximum_artifact_bytes: u64,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError>;

    /// Appends one exact bounded chunk and publishes only through the core artifact authority.
    fn write_chunk(
        &self,
        owner_peer: &PeerId,
        chunk: &ArtifactChunk,
        maximum_chunk_bytes: u32,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError>;

    /// Reads one bounded verified range through the core authorized read port.
    fn read_chunk(
        &self,
        owner_peer: &PeerId,
        transfer: &TransferId,
        offset: u64,
        maximum_bytes: u32,
    ) -> Result<ArtifactChunk, PeerArtifactError>;

    /// Aborts an incomplete core publication.
    fn abort(&self, owner_peer: &PeerId, transfer: &TransferId) -> Result<(), PeerArtifactError>;
}

/// Trait-object boundary for the ordinary core artifact and workspace accounting ports.
pub trait PeerCoreArtifactStore: ArtifactStore + WorkspaceStore {}

impl<T> PeerCoreArtifactStore for T where T: ArtifactStore + WorkspaceStore {}

#[derive(Clone)]
struct TransferState {
    owner_peer: PeerId,
    offer: ArtifactMetadataOffer,
    publication: Option<ArtifactPublicationId>,
    next_offset: u64,
}

/// Bounded peer transfer adapter over Milkdrift's ordinary artifact publication/read authority.
pub struct CorePeerArtifactStore {
    core: Arc<dyn PeerCoreArtifactStore>,
    budget: WorkspaceBudget,
    transfers: Mutex<BTreeMap<TransferId, TransferState>>,
}

impl std::fmt::Debug for CorePeerArtifactStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CorePeerArtifactStore")
            .field("durable_owner", &"core artifact port")
            .finish_non_exhaustive()
    }
}

impl CorePeerArtifactStore {
    /// Constructs bounded staging over one ordinary core artifact owner.
    pub fn new(
        core: Arc<dyn PeerCoreArtifactStore>,
        maximum_artifact_bytes: u64,
        maximum_total_import_bytes: u64,
    ) -> Result<Self, PeerArtifactError> {
        let budget = WorkspaceBudget::new(
            0,
            0,
            0,
            1,
            maximum_artifact_bytes,
            maximum_total_import_bytes,
        )
        .map_err(|error| PeerArtifactError::Rejected(error.to_string()))?;
        Ok(Self {
            core,
            budget,
            transfers: Mutex::new(BTreeMap::new()),
        })
    }

    fn publication_request(
        &self,
        owner_peer: &PeerId,
        offer: &ArtifactMetadataOffer,
    ) -> Result<BeginArtifactPublication, PeerArtifactError> {
        let provenance = imported_provenance(owner_peer, offer)?;
        let metadata = ArtifactMetadata::new(
            offer.artifact.clone(),
            offer.sensitivity,
            offer.retention.clone(),
            provenance,
        )
        .map_err(|error| PeerArtifactError::Rejected(error.to_string()))?;
        BeginArtifactPublication::new(
            publication_id(&offer.transfer)?,
            import_run_id(&offer.transfer)?,
            metadata,
            self.budget.clone(),
            WorkspaceUsage::EMPTY,
        )
        .map_err(map_persistence)
    }
}

impl PeerArtifactStore for CorePeerArtifactStore {
    fn negotiate(
        &self,
        owner_peer: &PeerId,
        offer: &ArtifactMetadataOffer,
        maximum_artifact_bytes: u64,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError> {
        offer
            .validate()
            .map_err(|error| PeerArtifactError::Rejected(error.to_string()))?;
        if offer.artifact.size_bytes() > maximum_artifact_bytes {
            return Err(PeerArtifactError::Rejected(
                "artifact size exceeds relationship transfer authority".to_owned(),
            ));
        }
        match offer.direction {
            ArtifactTransferDirection::Upload if &offer.source_peer != owner_peer => {
                return Err(PeerArtifactError::Rejected(
                    "artifact source does not match authenticated peer".to_owned(),
                ));
            }
            ArtifactTransferDirection::Download if &offer.source_peer == owner_peer => {
                return Err(PeerArtifactError::Rejected(
                    "download source must be the serving peer".to_owned(),
                ));
            }
            ArtifactTransferDirection::Upload | ArtifactTransferDirection::Download => {}
        }

        let mut transfers = self
            .transfers
            .lock()
            .map_err(|_| PeerArtifactError::Unavailable)?;
        if let Some(existing) = transfers.get(&offer.transfer) {
            if existing.owner_peer != *owner_peer || existing.offer != *offer {
                return Err(PeerArtifactError::Conflict(
                    "transfer identity was reused with different metadata".to_owned(),
                ));
            }
            return Ok(ArtifactTransferDecision::Transfer {
                next_offset: existing.next_offset,
                maximum_chunk_bytes: milkdrift_peer_protocol::MAX_ARTIFACT_CHUNK_BYTES,
            });
        }

        let (publication, next_offset, already_present) = match offer.direction {
            ArtifactTransferDirection::Upload => {
                let request = self.publication_request(owner_peer, offer)?;
                if let Some(existing) = self
                    .core
                    .metadata(offer.artifact.artifact())
                    .map_err(map_persistence)?
                {
                    if &existing == request.metadata() {
                        return Ok(ArtifactTransferDecision::AlreadyPresent);
                    }
                    return Err(PeerArtifactError::Conflict(
                        "artifact identity already has different immutable core metadata"
                            .to_owned(),
                    ));
                }
                match self
                    .core
                    .begin_publication(&request)
                    .map_err(map_persistence)?
                {
                    BeginArtifactOutcome::Writable => {
                        (Some(request.publication().clone()), 0, false)
                    }
                    BeginArtifactOutcome::Resumed { next_offset } => {
                        (Some(request.publication().clone()), next_offset, false)
                    }
                    BeginArtifactOutcome::AlreadyCommitted(metadata) => {
                        if metadata.reference() != &offer.artifact {
                            return Err(PeerArtifactError::Verification(
                                "committed core artifact disagrees with transfer metadata"
                                    .to_owned(),
                            ));
                        }
                        (None, offer.artifact.size_bytes(), true)
                    }
                }
            }
            ArtifactTransferDirection::Download => {
                let metadata = self
                    .core
                    .metadata(offer.artifact.artifact())
                    .map_err(map_persistence)?
                    .ok_or_else(|| {
                        PeerArtifactError::Rejected(
                            "requested core artifact is not present".to_owned(),
                        )
                    })?;
                if metadata.reference() != &offer.artifact
                    || metadata.sensitivity() != offer.sensitivity
                    || metadata.retention() != &offer.retention
                    || metadata.provenance() != &offer.provenance
                {
                    return Err(PeerArtifactError::Conflict(
                        "download offer does not match immutable core metadata".to_owned(),
                    ));
                }
                (None, 0, false)
            }
        };
        if already_present {
            return Ok(ArtifactTransferDecision::AlreadyPresent);
        }
        if transfers.len() >= MAX_ACTIVE_TRANSFERS {
            if let Some(publication) = publication.as_ref() {
                self.core
                    .abort_publication(publication)
                    .map_err(map_persistence)?;
            }
            return Err(PeerArtifactError::Rejected(
                "active peer artifact transfer bound is full".to_owned(),
            ));
        }
        transfers.insert(
            offer.transfer.clone(),
            TransferState {
                owner_peer: owner_peer.clone(),
                offer: offer.clone(),
                publication,
                next_offset,
            },
        );
        Ok(ArtifactTransferDecision::Transfer {
            next_offset,
            maximum_chunk_bytes: milkdrift_peer_protocol::MAX_ARTIFACT_CHUNK_BYTES,
        })
    }

    fn write_chunk(
        &self,
        owner_peer: &PeerId,
        chunk: &ArtifactChunk,
        maximum_chunk_bytes: u32,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError> {
        chunk
            .validate(maximum_chunk_bytes)
            .map_err(|error| PeerArtifactError::Rejected(error.to_string()))?;
        let mut transfers = self
            .transfers
            .lock()
            .map_err(|_| PeerArtifactError::Unavailable)?;
        if transfers
            .get(&chunk.transfer)
            .is_some_and(|state| unix_millis() > state.offer.expires_at_unix_ms)
        {
            let expired = transfers.remove(&chunk.transfer);
            drop(transfers);
            if let Some(publication) = expired.and_then(|state| state.publication) {
                self.core
                    .abort_publication(&publication)
                    .map_err(map_persistence)?;
            }
            return Err(PeerArtifactError::Rejected(
                "artifact transfer authority expired".to_owned(),
            ));
        }
        let state = transfers.get_mut(&chunk.transfer).ok_or_else(|| {
            PeerArtifactError::Conflict("artifact transfer is not negotiated".to_owned())
        })?;
        if &state.owner_peer != owner_peer
            || state.offer.direction != ArtifactTransferDirection::Upload
            || chunk.offset != state.next_offset
        {
            return Err(PeerArtifactError::Conflict(
                "artifact transfer owner, direction, or offset mismatch".to_owned(),
            ));
        }
        let resulting = chunk
            .offset
            .checked_add(u64::try_from(chunk.bytes.len()).unwrap_or(u64::MAX))
            .ok_or_else(|| {
                PeerArtifactError::Verification("artifact offset overflowed".to_owned())
            })?;
        let exact_size = state.offer.artifact.size_bytes();
        if resulting > exact_size || chunk.final_chunk != (resulting == exact_size) {
            return Err(PeerArtifactError::Verification(
                "chunk exceeds size or final flag does not match exact size".to_owned(),
            ));
        }
        let publication = state.publication.as_ref().ok_or_else(|| {
            PeerArtifactError::Conflict("upload has no core publication owner".to_owned())
        })?;
        let progress = self
            .core
            .write_chunk(publication, chunk.offset, &chunk.bytes)
            .map_err(map_persistence)?;
        if progress.bytes_received != resulting || progress.complete_size != chunk.final_chunk {
            return Err(PeerArtifactError::Verification(
                "core publication progress disagrees with the transfer".to_owned(),
            ));
        }
        state.next_offset = resulting;
        if !chunk.final_chunk {
            return Ok(ArtifactTransferDecision::Transfer {
                next_offset: resulting,
                maximum_chunk_bytes,
            });
        }
        let committed = self
            .core
            .commit_publication(publication)
            .map_err(map_persistence)?;
        if committed.metadata().reference() != &state.offer.artifact {
            return Err(PeerArtifactError::Verification(
                "core publication returned different artifact facts".to_owned(),
            ));
        }
        transfers.remove(&chunk.transfer);
        Ok(ArtifactTransferDecision::AlreadyPresent)
    }

    fn read_chunk(
        &self,
        owner_peer: &PeerId,
        transfer: &TransferId,
        offset: u64,
        maximum_bytes: u32,
    ) -> Result<ArtifactChunk, PeerArtifactError> {
        let mut transfers = self
            .transfers
            .lock()
            .map_err(|_| PeerArtifactError::Unavailable)?;
        if transfers
            .get(transfer)
            .is_some_and(|state| unix_millis() > state.offer.expires_at_unix_ms)
        {
            transfers.remove(transfer);
            return Err(PeerArtifactError::Rejected(
                "artifact transfer authority expired".to_owned(),
            ));
        }
        let state = transfers.get(transfer).ok_or_else(|| {
            PeerArtifactError::Conflict("artifact transfer is not negotiated".to_owned())
        })?;
        if &state.owner_peer != owner_peer
            || state.offer.direction != ArtifactTransferDirection::Download
        {
            return Err(PeerArtifactError::Rejected(
                "artifact download owner or direction mismatch".to_owned(),
            ));
        }
        let actor = ActorRef::new(format!("peer:{}", owner_peer.as_str()))
            .map_err(|error| PeerArtifactError::Rejected(error.to_string()))?;
        let evidence = EvidenceId::new(format!("peer-artifact:{}", short_hash(transfer.as_str())))
            .map_err(|error| PeerArtifactError::Rejected(error.to_string()))?;
        let request = ArtifactReadRequest::new(
            state.offer.artifact.clone(),
            offset,
            maximum_bytes.min(milkdrift_peer_protocol::MAX_ARTIFACT_CHUNK_BYTES),
            ArtifactReadAuthority::Authorized { actor, evidence },
        )
        .map_err(map_persistence)?;
        let chunk = self.core.read_chunk(&request).map_err(map_persistence)?;
        Ok(ArtifactChunk {
            transfer: transfer.clone(),
            offset: chunk.offset,
            bytes: chunk.bytes,
            final_chunk: chunk.end_of_artifact,
        })
    }

    fn abort(&self, owner_peer: &PeerId, transfer: &TransferId) -> Result<(), PeerArtifactError> {
        let state = self
            .transfers
            .lock()
            .map_err(|_| PeerArtifactError::Unavailable)?
            .remove(transfer);
        let Some(state) = state else {
            return Ok(());
        };
        if state.owner_peer != *owner_peer {
            return Err(PeerArtifactError::Rejected(
                "artifact transfer owner mismatch".to_owned(),
            ));
        }
        if let Some(publication) = state.publication {
            self.core
                .abort_publication(&publication)
                .map_err(map_persistence)?;
        }
        Ok(())
    }
}

fn imported_provenance(
    owner_peer: &PeerId,
    offer: &ArtifactMetadataOffer,
) -> Result<ArtifactProvenance, PeerArtifactError> {
    let origin = CausalReference::External {
        source: origin_identity(owner_peer, offer)?,
    };
    let mut causes = offer.provenance.causes().to_vec();
    if offer.provenance.producer() != &origin && !causes.contains(&origin) {
        causes.push(origin);
    }
    ArtifactProvenance::new(offer.provenance.producer().clone(), causes)
        .map_err(|error| PeerArtifactError::Rejected(error.to_string()))
}

fn origin_identity(
    owner_peer: &PeerId,
    offer: &ArtifactMetadataOffer,
) -> Result<CausalId, PeerArtifactError> {
    let readable = format!(
        "peer:{}/execution:{}",
        owner_peer.as_str(),
        offer.execution.as_str()
    );
    CausalId::new(readable)
        .or_else(|_| {
            CausalId::new(format!(
                "peer-import:{}",
                short_hash(&format!(
                    "{}:{}:{}",
                    owner_peer.as_str(),
                    offer.execution.as_str(),
                    offer.artifact.digest()
                ))
            ))
        })
        .map_err(|error| PeerArtifactError::Rejected(error.to_string()))
}

fn publication_id(transfer: &TransferId) -> Result<ArtifactPublicationId, PeerArtifactError> {
    ArtifactPublicationId::new(format!(
        "peer-publication:{}",
        short_hash(transfer.as_str())
    ))
    .map_err(|error| PeerArtifactError::Rejected(error.to_string()))
}

fn import_run_id(transfer: &TransferId) -> Result<RunId, PeerArtifactError> {
    RunId::new(format!("peer-import:{}", short_hash(transfer.as_str())))
        .map_err(|error| PeerArtifactError::Rejected(error.to_string()))
}

fn short_hash(value: &str) -> String {
    blake3::hash(value.as_bytes()).to_hex()[..32].to_owned()
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn map_persistence(error: milkdrift_persistence::PersistenceError) -> PeerArtifactError {
    match error {
        milkdrift_persistence::PersistenceError::ImmutableConflict { .. } => {
            PeerArtifactError::Conflict(error.to_string())
        }
        milkdrift_persistence::PersistenceError::Bounds { .. }
        | milkdrift_persistence::PersistenceError::InvalidDocument(_)
        | milkdrift_persistence::PersistenceError::ArtifactAccessDenied(_) => {
            PeerArtifactError::Rejected(error.to_string())
        }
        milkdrift_persistence::PersistenceError::ArtifactNotCommitted(_) => {
            PeerArtifactError::Verification(error.to_string())
        }
        _ => PeerArtifactError::Persistence(error.to_string()),
    }
}
