use std::collections::BTreeMap;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{PeerProtocolError, ProtocolVersion, session::PROTOCOL_MAJOR_V1};

/// Maximum encoded bytes for one peer control document. Artifact bytes use chunk routes.
pub const MAX_PEER_DOCUMENT_BYTES: usize = 1_048_576;
pub(crate) const MAX_CONTAINER_ITEMS: usize = 512;
const MAX_EXTENSION_ITEMS: usize = 32;

/// Defensive JSON decoder bounds applied before domain deserialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    /// Maximum complete encoded body.
    pub bytes: usize,
    /// Maximum container nesting depth.
    pub depth: usize,
    /// Maximum entries in any array or object.
    pub items: usize,
    /// Maximum decoded string bytes.
    pub string_bytes: usize,
    /// Maximum object key bytes.
    pub key_bytes: usize,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            bytes: MAX_PEER_DOCUMENT_BYTES,
            depth: 32,
            items: MAX_CONTAINER_ITEMS,
            string_bytes: 262_144,
            key_bytes: 192,
        }
    }
}

/// Versioned peer message envelope with bounded namespaced optional extensions.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolEnvelope<T> {
    /// Protocol version selected for this session.
    pub protocol: ProtocolVersion,
    /// Typed family payload.
    pub message: T,
    /// Explicitly ignorable, bounded, namespaced forward-compatible fields.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

impl<T> ProtocolEnvelope<T> {
    /// Wraps one v1.0 message without optional extensions.
    #[must_use]
    pub fn v1(message: T) -> Self {
        Self {
            protocol: ProtocolVersion::V1_0,
            message,
            extensions: BTreeMap::new(),
        }
    }
}

/// Canonically encodes an envelope after applying structural and extension bounds.
pub fn encode_envelope<T: Serialize>(
    envelope: &ProtocolEnvelope<T>,
) -> Result<Vec<u8>, PeerProtocolError> {
    validate_envelope(envelope.protocol, &envelope.extensions)?;
    let limits = json_limits(DecodeLimits::default());
    let bytes = milkdrift_contracts::canonical_json_bytes(envelope, limits)
        .map_err(|error| PeerProtocolError::Json(format!("{error:?}")))?;
    if bytes.len() > MAX_PEER_DOCUMENT_BYTES {
        return Err(PeerProtocolError::Bounds {
            location: "envelope",
            reason: format!("document exceeds {MAX_PEER_DOCUMENT_BYTES} bytes"),
        });
    }
    Ok(bytes)
}

/// Preflights, duplicate-checks, structurally bounds, and decodes one envelope.
pub fn decode_envelope<T: DeserializeOwned>(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<ProtocolEnvelope<T>, PeerProtocolError> {
    if limits.bytes == 0 || limits.bytes > MAX_PEER_DOCUMENT_BYTES || bytes.len() > limits.bytes {
        return Err(PeerProtocolError::Bounds {
            location: "envelope",
            reason: "encoded document exceeds the negotiated byte limit".to_owned(),
        });
    }
    let json_limits = json_limits(limits);
    milkdrift_contracts::preflight_json_structure(bytes, json_limits)
        .map_err(|error| PeerProtocolError::Json(format!("preflight bound: {error:?}")))?;
    let value = milkdrift_contracts::parse_json_without_duplicates(bytes)
        .map_err(|error| PeerProtocolError::Json(error.to_string()))?;
    milkdrift_contracts::validate_json_value(&value, json_limits)
        .map_err(|error| PeerProtocolError::Json(format!("structural bound: {error:?}")))?;
    let envelope: ProtocolEnvelope<T> = serde_json::from_value(value)
        .map_err(|error| PeerProtocolError::Json(error.to_string()))?;
    validate_envelope(envelope.protocol, &envelope.extensions)?;
    Ok(envelope)
}

fn validate_envelope(
    protocol: ProtocolVersion,
    extensions: &BTreeMap<String, Value>,
) -> Result<(), PeerProtocolError> {
    if protocol.major != PROTOCOL_MAJOR_V1 {
        return Err(PeerProtocolError::IncompatibleVersion);
    }
    if extensions.len() > MAX_EXTENSION_ITEMS
        || extensions.keys().any(|key| {
            key.len() > 192
                || !key.split_once('/').is_some_and(|(namespace, name)| {
                    namespace.contains('.') && !namespace.is_empty() && !name.is_empty()
                })
        })
    {
        return Err(PeerProtocolError::Bounds {
            location: "envelope.extensions",
            reason: "extensions must be bounded and use a DNS-like namespace".to_owned(),
        });
    }
    Ok(())
}

fn json_limits(limits: DecodeLimits) -> milkdrift_contracts::JsonLimits {
    milkdrift_contracts::JsonLimits {
        maximum_depth: limits.depth.min(32),
        maximum_string_bytes: limits.string_bytes.min(262_144),
        maximum_key_bytes: limits.key_bytes.min(192),
        maximum_container_items: limits.items.min(MAX_CONTAINER_ITEMS),
    }
}
