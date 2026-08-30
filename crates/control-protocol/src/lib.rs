//! Pure versioned wire contracts for the local Milkdrift control plane.
//!
//! This crate deliberately contains no HTTP, asynchronous runtime, database, process,
//! provider, or UI types. Identities are opaque strings at this boundary and internal
//! durable event variants are projected into the stable [`TimelineCategory`] vocabulary.

use std::collections::{BTreeMap, BTreeSet};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use milkdrift_contracts::{JsonLimits, parse_json_without_duplicates, validate_json_value};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;

mod command;
mod layout;
mod read;

pub use command::*;
pub use layout::*;
pub use read::*;

/// Supported control protocol major version.
pub const PROTOCOL_MAJOR: u16 = 1;
/// Supported control protocol minor version.
pub const PROTOCOL_MINOR: u16 = 0;
/// Independent presentation-layout document version.
pub const LAYOUT_SCHEMA_VERSION: u32 = 1;
/// Maximum JSON request or response envelope size.
pub const MAX_DOCUMENT_BYTES: usize = 1_310_720;
/// Maximum returned items in a single page.
pub const MAX_PAGE_ITEMS: u32 = 1_024;
/// Maximum reason length in UTF-8 bytes.
pub const MAX_REASON_BYTES: usize = 2_048;
/// Maximum evidence references on one command.
pub const MAX_EVIDENCE_ITEMS: usize = 32;
/// Maximum independently persisted layout bytes.
pub const MAX_LAYOUT_BYTES: usize = 262_144;

const JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: 72,
    maximum_string_bytes: 1_048_576,
    maximum_key_bytes: 256,
    maximum_container_items: 8_192,
};

/// Protocol or document validation failure before application dispatch.
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// JSON syntax or typed decoding was invalid.
    #[error("invalid JSON document: {0}")]
    InvalidJson(String),
    /// A configured wire bound was exceeded.
    #[error("document bound exceeded: {0}")]
    Bounds(String),
    /// A protocol major version cannot be served.
    #[error("unsupported protocol major version {found}; supported version is {supported}")]
    UnsupportedMajor {
        /// Requested major version.
        found: u16,
        /// Supported major version.
        supported: u16,
    },
    /// A cursor is malformed or is not valid for the selected feed.
    #[error("invalid cursor: {0}")]
    InvalidCursor(String),
    /// A semantic document invariant was violated.
    #[error("invalid contract: {0}")]
    InvalidContract(String),
}

/// Explicit major/minor version carried by every JSON envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolVersion {
    /// Breaking contract generation.
    pub major: u16,
    /// Backward-compatible feature generation.
    pub minor: u16,
}

impl ProtocolVersion {
    /// Current server/client contract version.
    pub const CURRENT: Self = Self {
        major: PROTOCOL_MAJOR,
        minor: PROTOCOL_MINOR,
    };

    /// Rejects unsupported majors and negotiates the lower minor.
    pub fn negotiate(self) -> Result<Self, ProtocolError> {
        if self.major != PROTOCOL_MAJOR {
            return Err(ProtocolError::UnsupportedMajor {
                found: self.major,
                supported: PROTOCOL_MAJOR,
            });
        }
        Ok(Self {
            major: PROTOCOL_MAJOR,
            minor: PROTOCOL_MINOR,
        })
    }
}

/// Version negotiation request.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VersionRequest {
    /// Client's highest supported version.
    pub protocol: ProtocolVersion,
}

/// Version negotiation response.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VersionResponse {
    /// Negotiated version.
    pub protocol: ProtocolVersion,
    /// Stable daemon implementation name.
    pub service: String,
}

/// Stable configuration-independent failure classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Authentication was absent or invalid.
    Unauthenticated,
    /// The authenticated actor lacks authority.
    Unauthorized,
    /// Input or a resource contract was invalid.
    InvalidInput,
    /// An optimistic guard or idempotency identity conflicted.
    Conflict,
    /// The requested object was not found.
    NotFound,
    /// A bounded queue or output limit was exceeded.
    Overload,
    /// A required service or adapter is temporarily unavailable.
    Unavailable,
    /// Durable state failed integrity verification.
    Corruption,
    /// External side-effect truth is deliberately unresolved.
    Uncertain,
    /// The requested protocol or operation is unsupported.
    UnsupportedVersion,
    /// A deadline elapsed.
    Timeout,
    /// A non-redacted internal failure occurred.
    Internal,
}

/// Bounded redacted public error body.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorEnvelope {
    /// Protocol version used for the response.
    pub protocol: ProtocolVersion,
    /// Optional caller-visible request correlation identity.
    pub request_id: Option<String>,
    /// Stable machine-readable code.
    pub code: ErrorCode,
    /// Bounded non-secret diagnostic.
    pub message: String,
    /// Whether the exact request may succeed when repeated later.
    pub retryable: bool,
    /// Small stable redacted facts such as actual sequence.
    pub details: BTreeMap<String, String>,
}

impl ErrorEnvelope {
    /// Builds a bounded current-protocol error.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        let mut message = message.into();
        truncate_utf8(&mut message, MAX_REASON_BYTES);
        Self {
            protocol: ProtocolVersion::CURRENT,
            request_id: None,
            code,
            message,
            retryable,
            details: BTreeMap::new(),
        }
    }
}

/// Success envelope used for typed JSON responses.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseEnvelope<T> {
    /// Negotiated protocol.
    pub protocol: ProtocolVersion,
    /// Request correlation identity.
    pub request_id: String,
    /// Typed bounded result.
    pub value: T,
}

/// Stable opaque pagination or stream continuation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Cursor(String);

/// Server-verified authority and filter identity for one continuation family.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CursorBinding {
    /// Authenticated actor identity.
    pub actor: String,
    /// Exact immutable grant lineage.
    pub grant_id: String,
    /// Exact immutable grant revision.
    pub grant_revision: u64,
    /// Digest of the exact immutable grant document.
    pub grant_digest: String,
    /// Domain-separated digest of every resource and query filter.
    pub scope_digest: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CursorWire {
    version: u8,
    feed: String,
    position: CursorPosition,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BoundCursorWire {
    version: u8,
    feed: String,
    position: CursorPosition,
    binding: CursorBinding,
    decision_digest: String,
    mac: String,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct CursorMacDocument<'a> {
    version: u8,
    feed: &'a str,
    position: &'a CursorPosition,
    binding: &'a CursorBinding,
    decision_digest: &'a str,
}

#[derive(Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "value",
    deny_unknown_fields
)]
enum CursorPosition {
    Sequence(u64),
    Key(String),
}

impl Cursor {
    /// Creates an authenticated sequence continuation bound to one actor, exact grant, and scope.
    pub fn new_bound(
        feed: &str,
        position: u64,
        binding: CursorBinding,
        decision_digest: &str,
        key: &[u8; 32],
    ) -> Result<Self, ProtocolError> {
        Self::new_bound_position(
            feed,
            CursorPosition::Sequence(position),
            binding,
            decision_digest,
            key,
        )
    }

    /// Creates an authenticated key continuation bound to one actor, exact grant, and scope.
    pub fn new_bound_key(
        feed: &str,
        position: &str,
        binding: CursorBinding,
        decision_digest: &str,
        key: &[u8; 32],
    ) -> Result<Self, ProtocolError> {
        validate_identifier("cursor.key", position, 256)?;
        Self::new_bound_position(
            feed,
            CursorPosition::Key(position.to_owned()),
            binding,
            decision_digest,
            key,
        )
    }

    fn new_bound_position(
        feed: &str,
        position: CursorPosition,
        binding: CursorBinding,
        decision_digest: &str,
        key: &[u8; 32],
    ) -> Result<Self, ProtocolError> {
        validate_identifier("feed", feed, 256)?;
        validate_cursor_binding(&binding)?;
        validate_identifier("cursor.decision_digest", decision_digest, 256)?;
        let mac = cursor_mac(feed, &position, &binding, decision_digest, key)?;
        let bytes = serde_json::to_vec(&BoundCursorWire {
            version: 2,
            feed: feed.to_owned(),
            position,
            binding,
            decision_digest: decision_digest.to_owned(),
            mac,
        })
        .map_err(|error| ProtocolError::InvalidCursor(error.to_string()))?;
        Ok(Self(URL_SAFE_NO_PAD.encode(bytes)))
    }

    /// Verifies an authenticated sequence continuation against the current request boundary.
    pub fn position_for_bound(
        &self,
        expected_feed: &str,
        expected_binding: &CursorBinding,
        key: &[u8; 32],
    ) -> Result<u64, ProtocolError> {
        match self.bound_position(expected_feed, expected_binding, key)? {
            CursorPosition::Sequence(position) => Ok(position),
            CursorPosition::Key(_) => Err(ProtocolError::InvalidCursor(
                "cursor is not a sequence continuation".to_owned(),
            )),
        }
    }

    /// Verifies an authenticated key continuation against the current request boundary.
    pub fn key_for_bound(
        &self,
        expected_feed: &str,
        expected_binding: &CursorBinding,
        key: &[u8; 32],
    ) -> Result<String, ProtocolError> {
        match self.bound_position(expected_feed, expected_binding, key)? {
            CursorPosition::Key(position) => Ok(position),
            CursorPosition::Sequence(_) => Err(ProtocolError::InvalidCursor(
                "cursor is not an identity continuation".to_owned(),
            )),
        }
    }

    fn bound_position(
        &self,
        expected_feed: &str,
        expected_binding: &CursorBinding,
        key: &[u8; 32],
    ) -> Result<CursorPosition, ProtocolError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(&self.0)
            .map_err(|_| ProtocolError::InvalidCursor("malformed base64url".to_owned()))?;
        if bytes.len() > 2_048 {
            return Err(ProtocolError::InvalidCursor(
                "cursor is too large".to_owned(),
            ));
        }
        let value = parse_json_without_duplicates(&bytes)
            .map_err(|_| ProtocolError::InvalidCursor("malformed payload".to_owned()))?;
        let wire: BoundCursorWire = serde_json::from_value(value)
            .map_err(|_| ProtocolError::InvalidCursor("malformed fields".to_owned()))?;
        let expected_mac = cursor_mac(
            &wire.feed,
            &wire.position,
            &wire.binding,
            &wire.decision_digest,
            key,
        )?;
        if wire.version != 2
            || wire.feed != expected_feed
            || &wire.binding != expected_binding
            || wire.mac != expected_mac
        {
            return Err(ProtocolError::InvalidCursor(
                "cursor authority, scope, feed, or integrity check failed".to_owned(),
            ));
        }
        Ok(wire.position)
    }

    /// Creates a cursor bound to one exact feed and monotonic position.
    pub fn new(feed: &str, position: u64) -> Result<Self, ProtocolError> {
        validate_identifier("feed", feed, 256)?;
        let bytes = serde_json::to_vec(&CursorWire {
            version: 1,
            feed: feed.to_owned(),
            position: CursorPosition::Sequence(position),
        })
        .map_err(|error| ProtocolError::InvalidCursor(error.to_string()))?;
        Ok(Self(URL_SAFE_NO_PAD.encode(bytes)))
    }

    /// Decodes the position only when the cursor belongs to `expected_feed`.
    pub fn position_for(&self, expected_feed: &str) -> Result<u64, ProtocolError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(&self.0)
            .map_err(|_| ProtocolError::InvalidCursor("malformed base64url".to_owned()))?;
        if bytes.len() > 2_048 {
            return Err(ProtocolError::InvalidCursor(
                "cursor is too large".to_owned(),
            ));
        }
        let value = parse_json_without_duplicates(&bytes)
            .map_err(|_| ProtocolError::InvalidCursor("malformed payload".to_owned()))?;
        let position = match value.get("version").and_then(serde_json::Value::as_u64) {
            Some(1) => {
                let wire: CursorWire = serde_json::from_value(value)
                    .map_err(|_| ProtocolError::InvalidCursor("malformed fields".to_owned()))?;
                if wire.feed != expected_feed {
                    return Err(ProtocolError::InvalidCursor(
                        "cursor belongs to another feed or version".to_owned(),
                    ));
                }
                wire.position
            }
            Some(2) => {
                let wire: BoundCursorWire = serde_json::from_value(value)
                    .map_err(|_| ProtocolError::InvalidCursor("malformed fields".to_owned()))?;
                if wire.feed != expected_feed {
                    return Err(ProtocolError::InvalidCursor(
                        "cursor belongs to another feed or version".to_owned(),
                    ));
                }
                wire.position
            }
            _ => {
                return Err(ProtocolError::InvalidCursor(
                    "cursor belongs to another feed or version".to_owned(),
                ));
            }
        };
        match position {
            CursorPosition::Sequence(position) => Ok(position),
            CursorPosition::Key(_) => Err(ProtocolError::InvalidCursor(
                "cursor is not a sequence continuation".to_owned(),
            )),
        }
    }

    /// Creates a cursor bound to one exact feed and stable identity resume key.
    pub fn new_key(feed: &str, key: &str) -> Result<Self, ProtocolError> {
        validate_identifier("feed", feed, 256)?;
        validate_identifier("cursor.key", key, 256)?;
        let bytes = serde_json::to_vec(&CursorWire {
            version: 1,
            feed: feed.to_owned(),
            position: CursorPosition::Key(key.to_owned()),
        })
        .map_err(|error| ProtocolError::InvalidCursor(error.to_string()))?;
        Ok(Self(URL_SAFE_NO_PAD.encode(bytes)))
    }

    /// Decodes a stable identity resume key only for the exact selected feed.
    pub fn key_for(&self, expected_feed: &str) -> Result<String, ProtocolError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(&self.0)
            .map_err(|_| ProtocolError::InvalidCursor("malformed base64url".to_owned()))?;
        if bytes.len() > 2_048 {
            return Err(ProtocolError::InvalidCursor(
                "cursor is too large".to_owned(),
            ));
        }
        let value = parse_json_without_duplicates(&bytes)
            .map_err(|_| ProtocolError::InvalidCursor("malformed payload".to_owned()))?;
        let position = match value.get("version").and_then(serde_json::Value::as_u64) {
            Some(1) => {
                let wire: CursorWire = serde_json::from_value(value)
                    .map_err(|_| ProtocolError::InvalidCursor("malformed fields".to_owned()))?;
                if wire.feed != expected_feed {
                    return Err(ProtocolError::InvalidCursor(
                        "cursor belongs to another feed or version".to_owned(),
                    ));
                }
                wire.position
            }
            Some(2) => {
                let wire: BoundCursorWire = serde_json::from_value(value)
                    .map_err(|_| ProtocolError::InvalidCursor("malformed fields".to_owned()))?;
                if wire.feed != expected_feed {
                    return Err(ProtocolError::InvalidCursor(
                        "cursor belongs to another feed or version".to_owned(),
                    ));
                }
                wire.position
            }
            _ => {
                return Err(ProtocolError::InvalidCursor(
                    "cursor belongs to another feed or version".to_owned(),
                ));
            }
        };
        match position {
            CursorPosition::Key(key) => Ok(key),
            CursorPosition::Sequence(_) => Err(ProtocolError::InvalidCursor(
                "cursor is not an identity continuation".to_owned(),
            )),
        }
    }

    /// Opaque transport text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_cursor_binding(binding: &CursorBinding) -> Result<(), ProtocolError> {
    validate_identifier("cursor.actor", &binding.actor, 256)?;
    validate_identifier("cursor.grant_id", &binding.grant_id, 256)?;
    validate_identifier("cursor.grant_digest", &binding.grant_digest, 256)?;
    validate_identifier("cursor.scope_digest", &binding.scope_digest, 256)?;
    if binding.grant_revision == 0 {
        return Err(ProtocolError::InvalidCursor(
            "cursor grant revision must be nonzero".to_owned(),
        ));
    }
    Ok(())
}

fn cursor_mac(
    feed: &str,
    position: &CursorPosition,
    binding: &CursorBinding,
    decision_digest: &str,
    key: &[u8; 32],
) -> Result<String, ProtocolError> {
    let bytes = serde_json::to_vec(&CursorMacDocument {
        version: 2,
        feed,
        position,
        binding,
        decision_digest,
    })
    .map_err(|error| ProtocolError::InvalidCursor(error.to_string()))?;
    Ok(blake3::keyed_hash(key, &bytes).to_hex().to_string())
}

/// Explicit bounded page request.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    /// Optional stable continuation.
    pub cursor: Option<Cursor>,
    /// Maximum number of returned items.
    pub limit: u32,
}

impl PageRequest {
    /// Validates the nonzero global page bound.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.limit == 0 || self.limit > MAX_PAGE_ITEMS {
            return Err(ProtocolError::Bounds(format!(
                "page limit must be in 1..={MAX_PAGE_ITEMS}"
            )));
        }
        Ok(())
    }
}

/// One bounded stable-cursor page.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Page<T> {
    /// Returned items.
    pub items: Vec<T>,
    /// Continuation, absent at end of feed.
    pub next_cursor: Option<Cursor>,
    /// Feed head observed while reading this page.
    pub observed_cursor: Option<Cursor>,
}

/// Strictly duplicate-checks, bounds-checks, and decodes one protocol JSON document.
pub fn decode_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProtocolError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(ProtocolError::Bounds(format!(
            "document exceeds {MAX_DOCUMENT_BYTES} bytes"
        )));
    }
    milkdrift_contracts::preflight_json_structure(bytes, JSON_LIMITS)
        .map_err(|error| ProtocolError::Bounds(format!("{error:?}")))?;
    let value = parse_json_without_duplicates(bytes)
        .map_err(|error| ProtocolError::InvalidJson(error.to_string()))?;
    validate_json_value(&value, JSON_LIMITS)
        .map_err(|error| ProtocolError::Bounds(format!("{error:?}")))?;
    serde_json::from_value(value).map_err(|error| ProtocolError::InvalidJson(error.to_string()))
}

/// Encodes one protocol JSON document within the global byte bound.
pub fn encode_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| ProtocolError::InvalidJson(error.to_string()))?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(ProtocolError::Bounds(format!(
            "document exceeds {MAX_DOCUMENT_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn validate_identifier(
    location: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), ProtocolError> {
    if value.is_empty()
        || value.len() > maximum
        || !value.is_ascii()
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(ProtocolError::InvalidContract(format!(
            "{location} must be 1..={maximum} printable ASCII bytes"
        )));
    }
    Ok(())
}

fn truncate_utf8(value: &mut String, maximum: usize) {
    if value.len() <= maximum {
        return;
    }
    let mut boundary = maximum;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_and_cursor_are_explicit_and_feed_bound() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            ProtocolVersion::CURRENT.negotiate()?,
            ProtocolVersion::CURRENT
        );
        assert!(matches!(
            ProtocolVersion { major: 2, minor: 0 }.negotiate(),
            Err(ProtocolError::UnsupportedMajor { .. })
        ));
        let cursor = Cursor::new("run:alpha", 42)?;
        assert_eq!(cursor.position_for("run:alpha")?, 42);
        assert!(cursor.position_for("run:beta").is_err());
        assert!(
            Cursor("not-base64!".to_owned())
                .position_for("run:alpha")
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn bound_cursor_rejects_other_actor_grant_scope_and_rotated_credential()
    -> Result<(), Box<dyn std::error::Error>> {
        let binding = CursorBinding {
            actor: "human:alice".to_owned(),
            grant_id: "grant:alice".to_owned(),
            grant_revision: 3,
            grant_digest: format!("b3_{}", "1".repeat(64)),
            scope_digest: format!("b3_{}", "2".repeat(64)),
        };
        let key = [7_u8; 32];
        let cursor = Cursor::new_bound(
            "runs:active:workflow-a",
            42,
            binding.clone(),
            &format!("b3_{}", "3".repeat(64)),
            &key,
        )?;
        assert_eq!(
            cursor.position_for_bound("runs:active:workflow-a", &binding, &key)?,
            42
        );
        let mut other_actor = binding.clone();
        other_actor.actor = "ai:alice".to_owned();
        assert!(
            cursor
                .position_for_bound("runs:active:workflow-a", &other_actor, &key)
                .is_err()
        );
        let mut narrower_scope = binding.clone();
        narrower_scope.scope_digest = format!("b3_{}", "4".repeat(64));
        assert!(
            cursor
                .position_for_bound("runs:active:workflow-a", &narrower_scope, &key)
                .is_err()
        );
        assert!(
            cursor
                .position_for_bound("runs:active:workflow-a", &binding, &[8_u8; 32])
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn duplicate_json_keys_and_unbounded_pages_are_rejected() {
        assert!(
            decode_json::<VersionRequest>(br#"{"protocol":{"major":1,"major":1,"minor":0}}"#)
                .is_err()
        );
        assert!(
            PageRequest {
                cursor: None,
                limit: MAX_PAGE_ITEMS + 1
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn layout_digest_is_independent_and_tamper_evident() -> Result<(), Box<dyn std::error::Error>> {
        let layout = LayoutDocument {
            schema_version: LAYOUT_SCHEMA_VERSION,
            workflow_id: "workflow-a".to_owned(),
            revision_id: "revision-a".to_owned(),
            generation: 1,
            author: "human:operator".to_owned(),
            digest: String::new(),
            nodes: BTreeMap::from([(
                "node-a".to_owned(),
                LayoutPoint {
                    x: 1.0,
                    y: 2.0,
                    width: None,
                    height: None,
                },
            )]),
            collapsed_groups: BTreeSet::new(),
            annotations: BTreeMap::new(),
            viewport: None,
        }
        .seal()?;
        layout.validate()?;
        let mut tampered = layout.clone();
        tampered.nodes.get_mut("node-a").ok_or("missing node")?.x = 9.0;
        assert!(tampered.validate().is_err());
        Ok(())
    }

    #[test]
    fn public_timeline_has_no_internal_event_variant_field()
    -> Result<(), Box<dyn std::error::Error>> {
        let entry = TimelineEntry {
            sequence: 1,
            timestamp_ms: 2,
            category: TimelineCategory::Lifecycle,
            actor: "human:operator".to_owned(),
            run_id: "run-a".to_owned(),
            node_id: None,
            attempt_id: None,
            revision_id: None,
            summary: "run created".to_owned(),
            detail: Value::Null,
        };
        let encoded = String::from_utf8(encode_json(&entry)?)?;
        assert!(!encoded.contains("RunEventKind"));
        assert!(!encoded.contains("run_event_kind"));
        Ok(())
    }
}
