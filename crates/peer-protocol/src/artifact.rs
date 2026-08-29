use milkdrift_authority::PeerId;
use milkdrift_workspace::{
    ArtifactProvenance, ArtifactReference, ArtifactRetention, ArtifactSensitivity,
};
use serde::{Deserialize, Serialize};

use crate::{PeerExecutionId, PeerProtocolError, TransferId};

/// Protocol ceiling for one artifact chunk or range.
pub const MAX_ARTIFACT_CHUNK_BYTES: u32 = 1_048_576;

/// Direction requested for a verified content-addressed transfer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactTransferDirection {
    /// Caller will upload bytes to the receiver.
    Upload,
    /// Caller requests authorized bytes from the receiver.
    Download,
}

/// Metadata-first transfer offer. No filename or host path is accepted.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMetadataOffer {
    /// Idempotent transfer identity.
    pub transfer: TransferId,
    /// Transfer direction.
    pub direction: ArtifactTransferDirection,
    /// Exact digest, size, media type, and opaque artifact identity.
    pub artifact: ArtifactReference,
    /// Source sensitivity classification preserved at import.
    pub sensitivity: ArtifactSensitivity,
    /// Source retention floor preserved at import.
    pub retention: ArtifactRetention,
    /// Source causal provenance, augmented with peer/execution origin on import.
    pub provenance: ArtifactProvenance,
    /// Authenticated source peer retained in provenance.
    pub source_peer: PeerId,
    /// Exact remote execution that produced or consumes the artifact.
    pub execution: PeerExecutionId,
    /// Expiry of this narrow transfer authority.
    pub expires_at_unix_ms: u64,
}

impl ArtifactMetadataOffer {
    /// Requires exact media type/size and a nonzero transfer expiry.
    pub fn validate(&self) -> Result<(), PeerProtocolError> {
        if self.expires_at_unix_ms == 0 {
            return Err(PeerProtocolError::InvalidContract(
                "artifact transfer requires exact size, content type, and expiry".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Receiver decision after checking digest, authority, content type, and budget.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ArtifactTransferDecision {
    /// Verified identical content is already authorized and present.
    AlreadyPresent,
    /// Transfer may resume at the exact temporary byte offset.
    Transfer {
        /// First byte offset accepted by the next chunk.
        next_offset: u64,
        /// Enforced maximum chunk size.
        maximum_chunk_bytes: u32,
    },
    /// Transfer was rejected before publishing any content.
    Rejected {
        /// Bounded non-secret reason.
        reason: String,
    },
}

/// One bounded sequential or ranged artifact chunk.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactChunk {
    /// Exact transfer session.
    pub transfer: TransferId,
    /// Exact byte offset.
    pub offset: u64,
    /// Raw bytes, bounded before acceptance.
    pub bytes: Vec<u8>,
    /// True only when this chunk reaches the declared exact total size.
    pub final_chunk: bool,
}

impl ArtifactChunk {
    /// Enforces nonempty package and negotiated chunk limits before use.
    pub fn validate(&self, negotiated_maximum: u32) -> Result<(), PeerProtocolError> {
        let limit = negotiated_maximum.min(MAX_ARTIFACT_CHUNK_BYTES);
        if self.bytes.is_empty() || self.bytes.len() > usize::try_from(limit).unwrap_or(usize::MAX)
        {
            return Err(PeerProtocolError::Bounds {
                location: "artifact.chunk",
                reason: "chunk is empty or exceeds the negotiated bound".to_owned(),
            });
        }
        Ok(())
    }
}
