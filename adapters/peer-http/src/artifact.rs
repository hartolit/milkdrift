use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use milkdrift_authority::PeerId;
use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision, ArtifactTransferDirection,
    TransferId,
};
use thiserror::Error;

/// Safe staging/publication failure for peer artifact transfer.
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
    /// Owned storage I/O failed without disclosing paths.
    #[error("peer artifact storage unavailable: {0}")]
    Io(String),
    /// Internal transfer state lock is unavailable.
    #[error("peer artifact storage state is unavailable")]
    Unavailable,
}

/// Exact publication boundaries used by deterministic crash/failure tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerArtifactFaultPoint {
    /// After verified bytes are durable but before the content-addressed blob rename.
    BlobBeforePublication,
    /// After the blob is durable but before metadata makes it visible to transfer readers.
    MetadataBeforePublication,
}

/// Optional deterministic artifact publication fault injector.
pub trait PeerArtifactFaultInjector: Send + Sync {
    /// Returns an injected failure at one exact publication boundary.
    fn check(&self, point: PeerArtifactFaultPoint) -> Result<(), PeerArtifactError>;
}

#[derive(Default)]
struct NoArtifactFaults;

impl PeerArtifactFaultInjector for NoArtifactFaults {
    fn check(&self, _point: PeerArtifactFaultPoint) -> Result<(), PeerArtifactError> {
        Ok(())
    }
}

/// Narrow verified artifact transfer port owned by the peer adapter layer.
pub trait PeerArtifactStore: Send + Sync {
    /// Negotiates metadata before any bytes, returning deduplication or resume state.
    fn negotiate(
        &self,
        owner_peer: &PeerId,
        offer: &ArtifactMetadataOffer,
        maximum_artifact_bytes: u64,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError>;

    /// Appends one exact bounded chunk and atomically publishes only after verification.
    fn write_chunk(
        &self,
        owner_peer: &PeerId,
        chunk: &ArtifactChunk,
        maximum_chunk_bytes: u32,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError>;

    /// Reads one bounded verified range for a previously negotiated download.
    fn read_chunk(
        &self,
        owner_peer: &PeerId,
        transfer: &TransferId,
        offset: u64,
        maximum_bytes: u32,
    ) -> Result<ArtifactChunk, PeerArtifactError>;

    /// Aborts an incomplete upload, leaving no published artifact.
    fn abort(&self, owner_peer: &PeerId, transfer: &TransferId) -> Result<(), PeerArtifactError>;
}

#[derive(Clone)]
struct TransferState {
    owner_peer: PeerId,
    offer: ArtifactMetadataOffer,
    next_offset: u64,
}

/// Owned content-addressed peer staging store with verified atomic publication.
pub struct FilePeerArtifactStore {
    temporary: PathBuf,
    blobs: PathBuf,
    metadata: PathBuf,
    transfers: Mutex<BTreeMap<TransferId, TransferState>>,
    faults: Arc<dyn PeerArtifactFaultInjector>,
}

impl std::fmt::Debug for FilePeerArtifactStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilePeerArtifactStore")
            .field("root", &"[owned peer artifact directory]")
            .finish_non_exhaustive()
    }
}

impl FilePeerArtifactStore {
    /// Opens or creates owned temporary, blob, and metadata directories.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, PeerArtifactError> {
        Self::open_with_faults(root, Arc::new(NoArtifactFaults))
    }

    /// Opens with deterministic publication fault injection.
    pub fn open_with_faults(
        root: impl Into<PathBuf>,
        faults: Arc<dyn PeerArtifactFaultInjector>,
    ) -> Result<Self, PeerArtifactError> {
        let root = root.into();
        let temporary = root.join("temporary");
        let blobs = root.join("blobs");
        let metadata = root.join("metadata");
        for directory in [&root, &temporary, &blobs, &metadata] {
            fs::create_dir_all(directory).map_err(io_error)?;
        }
        Ok(Self {
            temporary,
            blobs,
            metadata,
            transfers: Mutex::new(BTreeMap::new()),
            faults,
        })
    }

    fn transfer_path(&self, transfer: &TransferId) -> PathBuf {
        self.temporary
            .join(format!("{}.part", key_hash(transfer.as_str())))
    }

    fn blob_path(&self, digest: &str) -> PathBuf {
        self.blobs.join(digest)
    }

    fn metadata_path(&self, digest: &str) -> PathBuf {
        self.metadata.join(format!("{digest}.json"))
    }

    fn verify_blob(&self, offer: &ArtifactMetadataOffer) -> Result<bool, PeerArtifactError> {
        let path = self.blob_path(offer.artifact.digest());
        if !path.exists() {
            return Ok(false);
        }
        let metadata = fs::metadata(&path).map_err(io_error)?;
        if Some(metadata.len()) != offer.artifact.size_bytes() {
            return Err(PeerArtifactError::Verification(
                "existing content size does not match metadata".to_owned(),
            ));
        }
        let digest = digest_file(&path)?;
        if digest != offer.artifact.digest() {
            return Err(PeerArtifactError::Verification(
                "existing content digest does not match metadata".to_owned(),
            ));
        }
        let metadata_path = self.metadata_path(offer.artifact.digest());
        if !metadata_path.exists() {
            return Ok(false);
        }
        let bytes = fs::read(metadata_path).map_err(io_error)?;
        if bytes.len() > milkdrift_peer_protocol::MAX_PEER_DOCUMENT_BYTES {
            return Err(PeerArtifactError::Verification(
                "published artifact metadata exceeds its bound".to_owned(),
            ));
        }
        let published: ArtifactMetadataOffer = serde_json::from_slice(&bytes)
            .map_err(|error| PeerArtifactError::Verification(error.to_string()))?;
        if published.artifact.digest() != offer.artifact.digest()
            || published.artifact.size_bytes() != offer.artifact.size_bytes()
            || published.artifact.media_type() != offer.artifact.media_type()
        {
            return Err(PeerArtifactError::Verification(
                "published artifact metadata does not match requested content".to_owned(),
            ));
        }
        Ok(true)
    }

    fn publish_metadata(&self, offer: &ArtifactMetadataOffer) -> Result<(), PeerArtifactError> {
        let bytes = serde_json::to_vec(offer)
            .map_err(|error| PeerArtifactError::Conflict(error.to_string()))?;
        let destination = self.metadata_path(offer.artifact.digest());
        let temporary = destination.with_extension("json.tmp");
        let mut file = File::create(&temporary).map_err(io_error)?;
        file.write_all(&bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        self.faults
            .check(PeerArtifactFaultPoint::MetadataBeforePublication)?;
        fs::rename(&temporary, &destination).map_err(io_error)?;
        File::open(&self.metadata)
            .and_then(|directory| directory.sync_all())
            .map_err(io_error)
    }
}

impl PeerArtifactStore for FilePeerArtifactStore {
    fn negotiate(
        &self,
        owner_peer: &PeerId,
        offer: &ArtifactMetadataOffer,
        maximum_artifact_bytes: u64,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError> {
        offer
            .validate()
            .map_err(|error| PeerArtifactError::Rejected(error.to_string()))?;
        let size = offer.artifact.size_bytes().ok_or_else(|| {
            PeerArtifactError::Rejected("exact artifact size is required".to_owned())
        })?;
        let media_type = offer.artifact.media_type().ok_or_else(|| {
            PeerArtifactError::Rejected("exact artifact content type is required".to_owned())
        })?;
        if size > maximum_artifact_bytes
            || media_type.contains("secret")
            || media_type.contains("x-milkdrift-config")
        {
            return Err(PeerArtifactError::Rejected(
                "artifact size, content type, or forwarding policy rejected metadata".to_owned(),
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
        if self.verify_blob(offer)? {
            if offer.direction == ArtifactTransferDirection::Upload {
                return Ok(ArtifactTransferDecision::AlreadyPresent);
            }
        } else if offer.direction == ArtifactTransferDirection::Upload
            && self.blob_path(offer.artifact.digest()).exists()
        {
            self.publish_metadata(offer)?;
            return Ok(ArtifactTransferDecision::AlreadyPresent);
        } else if offer.direction == ArtifactTransferDirection::Download {
            return Err(PeerArtifactError::Rejected(
                "requested verified artifact is not present".to_owned(),
            ));
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
        let next_offset = if offer.direction == ArtifactTransferDirection::Upload {
            let path = self.transfer_path(&offer.transfer);
            if path.exists() {
                fs::metadata(&path).map_err(io_error)?.len()
            } else {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(io_error)?;
                0
            }
        } else {
            0
        };
        if next_offset > size {
            return Err(PeerArtifactError::Verification(
                "temporary transfer exceeds declared size".to_owned(),
            ));
        }
        transfers.insert(
            offer.transfer.clone(),
            TransferState {
                owner_peer: owner_peer.clone(),
                offer: offer.clone(),
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
        let exact_size = state.offer.artifact.size_bytes().ok_or_else(|| {
            PeerArtifactError::Verification("exact artifact size is absent".to_owned())
        })?;
        let resulting = chunk
            .offset
            .saturating_add(u64::try_from(chunk.bytes.len()).unwrap_or(u64::MAX));
        if resulting > exact_size || chunk.final_chunk != (resulting == exact_size) {
            return Err(PeerArtifactError::Verification(
                "chunk exceeds size or final flag does not match exact size".to_owned(),
            ));
        }
        let path = self.transfer_path(&chunk.transfer);
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(io_error)?;
        file.write_all(&chunk.bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        state.next_offset = resulting;
        if !chunk.final_chunk {
            return Ok(ArtifactTransferDecision::Transfer {
                next_offset: resulting,
                maximum_chunk_bytes,
            });
        }
        let offer = state.offer.clone();
        let actual = digest_file(&path)?;
        if actual != offer.artifact.digest() {
            let _ = fs::remove_file(&path);
            transfers.remove(&chunk.transfer);
            return Err(PeerArtifactError::Verification(
                "completed artifact digest mismatch".to_owned(),
            ));
        }
        let destination = self.blob_path(offer.artifact.digest());
        if destination.exists() {
            if !self.verify_blob(&offer)? {
                return Err(PeerArtifactError::Verification(
                    "deduplicated destination failed verification".to_owned(),
                ));
            }
            fs::remove_file(&path).map_err(io_error)?;
        } else {
            self.faults
                .check(PeerArtifactFaultPoint::BlobBeforePublication)?;
            fs::rename(&path, &destination).map_err(io_error)?;
            File::open(&self.blobs)
                .and_then(|directory| directory.sync_all())
                .map_err(io_error)?;
        }
        self.publish_metadata(&offer)?;
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
        let transfers = self
            .transfers
            .lock()
            .map_err(|_| PeerArtifactError::Unavailable)?;
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
        let size = state.offer.artifact.size_bytes().ok_or_else(|| {
            PeerArtifactError::Verification("exact artifact size is absent".to_owned())
        })?;
        if offset > size || maximum_bytes == 0 {
            return Err(PeerArtifactError::Rejected(
                "artifact range is outside exact bounds".to_owned(),
            ));
        }
        let limit = maximum_bytes.min(milkdrift_peer_protocol::MAX_ARTIFACT_CHUNK_BYTES);
        let mut file =
            File::open(self.blob_path(state.offer.artifact.digest())).map_err(io_error)?;
        file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
        let remaining = size.saturating_sub(offset);
        let count = usize::try_from(remaining.min(u64::from(limit))).unwrap_or(usize::MAX);
        let mut bytes = vec![0_u8; count];
        file.read_exact(&mut bytes).map_err(io_error)?;
        Ok(ArtifactChunk {
            transfer: transfer.clone(),
            offset,
            final_chunk: offset.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
                == size,
            bytes,
        })
    }

    fn abort(&self, owner_peer: &PeerId, transfer: &TransferId) -> Result<(), PeerArtifactError> {
        let mut transfers = self
            .transfers
            .lock()
            .map_err(|_| PeerArtifactError::Unavailable)?;
        if let Some(state) = transfers.get(transfer) {
            if &state.owner_peer != owner_peer {
                return Err(PeerArtifactError::Rejected(
                    "artifact transfer is owned by another peer".to_owned(),
                ));
            }
            if state.offer.direction == ArtifactTransferDirection::Upload {
                let path = self.transfer_path(transfer);
                if path.exists() {
                    fs::remove_file(path).map_err(io_error)?;
                }
            }
        }
        transfers.remove(transfer);
        Ok(())
    }
}

fn key_hash(value: &str) -> String {
    blake3::hash(value.as_bytes()).to_hex().to_string()
}

fn digest_file(path: &PathBuf) -> Result<String, PeerArtifactError> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 65_536];
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn io_error(error: std::io::Error) -> PeerArtifactError {
    PeerArtifactError::Io(format!("{:?}", error.kind()))
}
