//! Single-request, atomic input uploads. No client-supplied host path or producer is accepted.
use crate::ProtocolError;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

/// Maximum decoded content in one public upload. Larger files must be reduced before submission.
pub const MAX_INPUT_UPLOAD_BYTES: usize = 524_288;

/// Exact replayable public input publication. Completion is atomic; a lost reply may be replayed.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputUploadRequest {
    /// Explicit target installation, checked against the authenticated endpoint's host.
    pub host: String,
    /// Caller-scoped immutable upload identity; reuse with different content conflicts.
    pub upload_id: String,
    /// Canonical media type without parameters.
    pub media_type: String,
    /// Explicit classification: restricted, internal, or public; authority must permit it.
    pub sensitivity: String,
    /// Expected lowercase BLAKE3 content digest.
    pub digest: String,
    /// Canonical padded standard-base64 content, bounded before decoding.
    pub content_base64: String,
}

impl InputUploadRequest {
    /// Verifies the complete size and digest before any publication is opened.
    pub fn content(&self) -> Result<Vec<u8>, ProtocolError> {
        if self.content_base64.len() > MAX_INPUT_UPLOAD_BYTES.div_ceil(3) * 4 {
            return Err(ProtocolError::Bounds(
                "input upload exceeds 524288 bytes".to_owned(),
            ));
        }
        let bytes = STANDARD
            .decode(&self.content_base64)
            .map_err(|_| ProtocolError::InvalidContract("invalid upload base64".to_owned()))?;
        if bytes.len() > MAX_INPUT_UPLOAD_BYTES
            || STANDARD.encode(&bytes) != self.content_base64
            || blake3::hash(&bytes).to_hex().as_str() != self.digest
        {
            return Err(ProtocolError::InvalidContract(
                "upload content, size or digest is inconsistent".to_owned(),
            ));
        }
        Ok(bytes)
    }

    /// Encodes bounded complete content. The server still establishes producer and authority.
    pub fn from_content(
        host: String,
        upload_id: String,
        media_type: String,
        sensitivity: String,
        bytes: &[u8],
    ) -> Result<Self, ProtocolError> {
        if bytes.len() > MAX_INPUT_UPLOAD_BYTES {
            return Err(ProtocolError::Bounds(
                "input upload exceeds 524288 bytes".to_owned(),
            ));
        }
        Ok(Self {
            host,
            upload_id,
            media_type,
            sensitivity,
            digest: blake3::hash(bytes).to_hex().to_string(),
            content_base64: STANDARD.encode(bytes),
        })
    }
}
