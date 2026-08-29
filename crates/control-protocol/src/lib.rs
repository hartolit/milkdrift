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

/// Reference to bounded external evidence retained elsewhere.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    /// Stable evidence identity.
    pub id: String,
    /// Stable evidence category.
    pub kind: String,
}

/// External mutating request. Actor identity is intentionally absent.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    /// Requested protocol version.
    pub protocol: ProtocolVersion,
    /// Stable client-owned idempotency identity.
    pub command_id: String,
    /// Optional aggregate sequence guard.
    pub expected_sequence: Option<u64>,
    /// Optional exact semantic revision guard.
    pub expected_revision: Option<String>,
    /// Bounded human/operator reason.
    pub reason: String,
    /// Bounded references, never inline evidence blobs.
    pub evidence: Vec<EvidenceRef>,
    /// Closed command body.
    pub command: Command,
}

impl CommandRequest {
    /// Validates common envelope bounds and version support.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.protocol.negotiate()?;
        validate_identifier("command_id", &self.command_id, 192)?;
        if self.reason.is_empty() || self.reason.len() > MAX_REASON_BYTES {
            return Err(ProtocolError::Bounds(format!(
                "reason must contain 1..={MAX_REASON_BYTES} bytes"
            )));
        }
        if self.evidence.len() > MAX_EVIDENCE_ITEMS {
            return Err(ProtocolError::Bounds(format!(
                "at most {MAX_EVIDENCE_ITEMS} evidence references are allowed"
            )));
        }
        let mut identities = BTreeSet::new();
        for evidence in &self.evidence {
            validate_identifier("evidence.id", &evidence.id, 192)?;
            validate_identifier("evidence.kind", &evidence.kind, 64)?;
            if !identities.insert(&evidence.id) {
                return Err(ProtocolError::InvalidContract(
                    "evidence identities must be distinct".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

/// Closed version-one mutation vocabulary.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
#[allow(missing_docs)] // Variant prose documents each compact operation payload.
pub enum Command {
    /// Store a validated immutable blueprint document.
    ImportBlueprint { document: Value },
    /// Validate an immutable blueprint document without storing it.
    ValidateBlueprint { document: Value },
    /// Create and start a run at one exact revision.
    StartRun {
        run_id: String,
        workflow_id: String,
        revision_id: String,
    },
    /// Pause new work for a run.
    PauseRun { run_id: String },
    /// Resume a paused run.
    ResumeRun { run_id: String },
    /// Request durable cancellation.
    CancelRun { run_id: String },
    /// Deliver a typed signal with a bounded JSON payload.
    SignalRun {
        run_id: String,
        signal_id: String,
        signal_type: String,
        correlation: Option<String>,
        broadcast: bool,
        payload: Value,
    },
    /// Resolve retained/uncertain external work.
    ResolveWork {
        run_id: String,
        attempt_id: String,
        decision_id: String,
        action: ResolveAction,
        remediation_node: Option<String>,
    },
    /// Submit a versioned workflow proposal document.
    SubmitProposal { document: Value },
    /// Decide an exact proposal/reconciliation plan.
    DecideProposal {
        run_id: String,
        proposal_id: String,
        proposal_digest: String,
        proposed_revision: String,
        decision_id: String,
        decision: ProposalDecision,
    },
    /// Apply an approved exact proposal.
    ApplyProposal {
        run_id: String,
        proposal_id: String,
        proposal_digest: String,
        proposed_revision: String,
    },
    /// Optimistically replace presentation-only layout state.
    PutLayout { layout: LayoutDocument },
}

/// Public resolution choice for retained external work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveAction {
    /// Query external truth through a separately authorized capability.
    Query,
    /// Retry only under runtime idempotency policy.
    Retry,
    /// Create explicit compensation.
    Compensate,
    /// Keep the obligation visible.
    Retain,
    /// Resolve as succeeded from evidence.
    ResolveSucceeded,
    /// Resolve as failed from evidence.
    ResolveFailed,
}

/// Public proposal decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalDecision {
    /// Authorize application.
    Approve,
    /// Reject application.
    Reject,
}

/// Durable command acceptance response.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandAccepted {
    /// Echoed idempotency identity.
    pub command_id: String,
    /// True when a previously committed result was returned.
    pub replayed: bool,
    /// Resulting aggregate sequence, when a run was mutated.
    pub resulting_sequence: Option<u64>,
    /// Stable result category.
    pub result_type: String,
    /// Bounded operation-specific result.
    pub value: Value,
}

/// Daemon liveness/readiness/draining state.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthRead {
    /// Stable lifecycle state.
    pub state: DaemonState,
    /// Process is alive.
    pub live: bool,
    /// Recovery completed and commands may be considered.
    pub ready: bool,
    /// New mutations are refused.
    pub draining: bool,
    /// Current request queue occupancy.
    pub queued_requests: u32,
    /// Fixed request queue bound.
    pub request_queue_capacity: u32,
    /// Current effect workers executing external work.
    pub active_effects: u32,
    /// Redacted last startup/worker failure.
    pub last_failure: Option<String>,
}

/// Stable daemon lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonState {
    /// Configuration and resources are being initialized.
    Starting,
    /// Recovery and adapter initialization completed.
    Ready,
    /// Shutdown has closed admission.
    Draining,
    /// Host has stopped its owned services.
    Stopped,
    /// Startup failed before readiness.
    Failed,
}

/// Compact immutable revision summary.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionSummary {
    /// Revision identity.
    pub revision_id: String,
    /// Workflow lineage.
    pub workflow_id: String,
    /// User-facing lineage sequence.
    pub lineage_sequence: u64,
    /// Semantic digest.
    pub semantic_digest: String,
    /// Exact parents.
    pub parents: Vec<String>,
}

/// Bounded immutable revision inspection.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRead {
    /// Compact summary.
    pub summary: RevisionSummary,
    /// Bounded author reference.
    pub author: String,
    /// Bounded provenance reason.
    pub reason: String,
    /// Semantic node count.
    pub node_count: u32,
    /// Semantic edge count.
    pub edge_count: u32,
    /// Canonical portable document when explicitly requested.
    pub document: Option<Value>,
}

/// Structured bounded semantic difference.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionDiffRead {
    /// Left revision.
    pub from_revision: String,
    /// Right revision.
    pub to_revision: String,
    /// Stable bounded changes.
    pub changes: Vec<RevisionChange>,
    /// True if further changes were omitted at the response bound.
    pub truncated: bool,
}

/// One stable semantic revision change.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionChange {
    /// Added, removed, or changed.
    pub change: String,
    /// Node, edge, interface, or metadata.
    pub subject: String,
    /// Stable semantic identity where applicable.
    pub identity: Option<String>,
    /// Bounded structured summary.
    pub detail: Value,
}

/// Compact current run status.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunRead {
    /// Run aggregate.
    pub run_id: String,
    /// Authoritative sequence.
    pub sequence: u64,
    /// Stable lifecycle label.
    pub lifecycle: String,
    /// Stable terminal outcome when lifecycle is terminal.
    pub terminal: Option<String>,
    /// Workflow lineage.
    pub workflow_id: Option<String>,
    /// Exact current revision.
    pub revision_id: Option<String>,
    /// Current semantic digest.
    pub semantic_digest: Option<String>,
    /// Compact execution frontier.
    pub nodes: Vec<NodeRead>,
    /// Unresolved side-effect obligations.
    pub uncertainty_count: u32,
}

/// Compact node-execution and latest-attempt state.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeRead {
    /// Logical execution identity.
    pub execution_id: String,
    /// Semantic node identity.
    pub node_id: String,
    /// Governing revision.
    pub revision_id: String,
    /// Stable execution state label.
    pub state: String,
    /// Total attempts.
    pub attempt_count: u32,
    /// Latest retained attempt.
    pub latest_attempt: Option<AttemptRead>,
}

/// Bounded attempt status and provenance.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptRead {
    /// Attempt identity.
    pub attempt_id: String,
    /// Invocation identity after scheduling.
    pub invocation_id: Option<String>,
    /// Stable state label.
    pub state: String,
    /// Resolved capability identity.
    pub capability_id: Option<String>,
    /// Exact resolved capability descriptor revision.
    pub descriptor_revision: Option<u64>,
    /// Exact frozen capability-generation and safe implementation provenance.
    pub capability_provenance: Option<CapabilityProvenanceRead>,
    /// Exact provider profile selected for this attempt.
    pub provider_profile: Option<String>,
    /// Authenticated peer identity when remote execution recorded one.
    pub peer_id: Option<String>,
    /// Exact immutable context manifest.
    pub context_manifest: Option<ArtifactMetadataRead>,
    /// Authorized bounded contents of the exact context manifest.
    pub context: Option<ContextManifestRead>,
    /// `absent`, `metadata_only`, `authorized`, or `denied` without leaking contents.
    pub context_access: String,
    /// Stable terminal summary.
    pub terminal: Option<String>,
    /// Whether external outcome remains unresolved.
    pub uncertain: bool,
}

/// Safe immutable capability-generation facts retained with an attempt.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityProvenanceRead {
    /// Domain-separated digest of the complete resolved capability snapshot.
    pub snapshot_digest: String,
    /// Exact execution isolation/trust class selected for the attempt.
    pub execution_trust: String,
    /// Local-process implementation identity, including safe path digests, when applicable.
    pub implementation_identity: Option<String>,
    /// Exact executable content digest, when the generation is a byte-pinned local process.
    pub implementation_content_digest: Option<String>,
    /// Exact executable byte size, when the generation is a byte-pinned local process.
    pub implementation_size_bytes: Option<u64>,
    /// Digest of the complete local-process profile, when applicable.
    pub process_profile_digest: Option<String>,
    /// Digest of execution-semantic local-process policy, when applicable.
    pub execution_policy_digest: Option<String>,
    /// Bounded operator-declared package/deployment revision, when supplied.
    pub package_revision: Option<String>,
    /// Bounded operator-declared implementation documentation reference, when supplied.
    pub documentation_reference: Option<String>,
}

/// Bounded authorized context policy, selection, omissions, and accounting.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextManifestRead {
    /// Inner context-manifest schema.
    pub schema_version: u32,
    /// Domain-separated canonical manifest digest.
    pub digest: String,
    /// Exact immutable task context policy.
    pub policy: Value,
    /// Selected entry metadata and causal provenance, capped by the read model.
    pub entries: Vec<Value>,
    /// Stable omission/denial/missing records, capped by the read model.
    pub omissions: Vec<Value>,
    /// Applied totals.
    pub totals: Value,
    /// Applied deterministic limits.
    pub budget: Value,
    /// Whether entry or omission detail exceeded the response cap.
    pub truncated: bool,
}

/// Public timeline category; never an internal event enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineCategory {
    /// Run lifecycle changed.
    Lifecycle,
    /// Node became eligible/scheduled/started/terminal.
    Execution,
    /// Progress was durably observed.
    Progress,
    /// Artifact/output publication changed.
    Artifact,
    /// Signal or timer interaction.
    Coordination,
    /// Authority or approval decision.
    Authority,
    /// Recovery classified durable work.
    Recovery,
    /// Revision reconciliation changed.
    Reconciliation,
    /// External outcome remains uncertain.
    Uncertainty,
}

/// One bounded externally stable timeline entry.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineEntry {
    /// Exact journal provenance sequence.
    pub sequence: u64,
    /// Boundary timestamp.
    pub timestamp_ms: u64,
    /// Stable projected category.
    pub category: TimelineCategory,
    /// Bounded actor reference.
    pub actor: String,
    /// Run aggregate.
    pub run_id: String,
    /// Optional related node.
    pub node_id: Option<String>,
    /// Optional related attempt.
    pub attempt_id: Option<String>,
    /// Optional related revision.
    pub revision_id: Option<String>,
    /// Stable non-secret summary.
    pub summary: String,
    /// Bounded category-specific detail.
    pub detail: Value,
}

/// Capability generation and health observation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRead {
    /// Capability identity.
    pub capability_id: String,
    /// Descriptor revision.
    pub generation: u64,
    /// Descriptor digest.
    pub descriptor_digest: String,
    /// Stable discovery category.
    pub category: String,
    /// Exact advertised operation identities.
    pub operations: Vec<String>,
    /// Optional provider/model profile identity.
    pub provider_profile: Option<String>,
    /// Stable execution-locality label.
    pub locality: String,
    /// Authenticated remote peer identity, when peer-owned.
    pub peer_id: Option<String>,
    /// Complete advertised trust-zone labels.
    pub trust_zones: Vec<String>,
    /// Exact execution-isolation/trust class.
    pub execution_trust: String,
    /// Whether this generation is selected for new work.
    pub current: bool,
    /// Whether new work is refused.
    pub draining: bool,
    /// Stable health label.
    pub health: String,
    /// Last known availability.
    pub available: Option<bool>,
    /// Active/maximum permits.
    pub active_permits: u32,
    /// Admission permit bound.
    pub permit_limit: u32,
}

/// Safe authenticated peer/catalog status without credentials or transport internals.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerRead {
    /// Configured authenticated remote peer identity.
    pub peer_id: String,
    /// Whether an authenticated session and live verified catalog are current.
    pub connected: bool,
    /// Stable redacted health classification.
    pub health: String,
    /// Current authenticated remote daemon session identity.
    pub session_id: Option<String>,
    /// Current verified catalog generation.
    pub catalog_generation: Option<u64>,
    /// Current verified catalog digest.
    pub catalog_digest: Option<String>,
    /// Current ordinary capability-host registration count.
    pub registered_capabilities: usize,
    /// Current catalog hard expiry, when connected.
    pub catalog_expires_at_unix_ms: Option<u64>,
    /// Whether the live relationship was administratively revoked.
    pub revoked: bool,
}

/// Safe artifact metadata without a filesystem path.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMetadataRead {
    /// Artifact identity.
    pub artifact_id: String,
    /// Content digest.
    pub digest: String,
    /// Exact byte size.
    pub size: u64,
    /// Declared media type.
    pub content_type: String,
    /// Safe suggested download name.
    pub disposition_name: Option<String>,
    /// Stable sensitivity label.
    pub sensitivity: String,
}

/// Public actor/grant observation returned only under inspect authority.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityRead {
    /// Server-owned actor identity.
    pub actor: String,
    /// Exact configured grant identity.
    pub grant_id: String,
    /// Immutable grant revision.
    pub grant_revision: u64,
    /// Current revocation generation.
    pub revocation_generation: u64,
    /// Stable operation names allowed by configuration.
    pub operations: Vec<String>,
}

/// Workflow-controller proposal status.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalRead {
    /// Proposal identity.
    pub proposal_id: String,
    /// Exact prospective revision.
    pub proposed_revision: String,
    /// Stable reconciliation status.
    pub status: String,
    /// Whether approval is durably recorded.
    pub approved: bool,
    /// Optional application sequence.
    pub applied_sequence: Option<u64>,
}

/// A resumable, externally projected stream item.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationEnvelope {
    /// Protocol version.
    pub protocol: ProtocolVersion,
    /// Feed-bound monotonically ordered cursor.
    pub cursor: Cursor,
    /// Observation timestamp, independent from heartbeats.
    pub observed_at_ms: u64,
    /// Exact filtered feed identity.
    pub feed: String,
    /// External read-model payload.
    pub observation: Observation,
}

/// Closed stream observation vocabulary.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "value",
    deny_unknown_fields
)]
#[allow(missing_docs)] // Variant prose documents each compact observation payload.
pub enum Observation {
    /// Projected timeline fact.
    Timeline(TimelineEntry),
    /// Compact current run state.
    RunStatus(RunRead),
    /// Capability generation/health change.
    Capability(CapabilityRead),
    /// Daemon lifecycle/health change.
    DaemonHealth(HealthRead),
    /// Server is shutting down this stream.
    StreamClosing { reason: String },
    /// Subscriber fell outside retained stream history.
    ResyncRequired { reason: String },
}

/// Presentation-only layout coordinate.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPoint {
    /// Horizontal canvas coordinate.
    pub x: f64,
    /// Vertical canvas coordinate.
    pub y: f64,
    /// Optional width.
    pub width: Option<f64>,
    /// Optional height.
    pub height: Option<f64>,
}

/// Optional viewport preference.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutViewport {
    /// Horizontal pan.
    pub x: f64,
    /// Vertical pan.
    pub y: f64,
    /// Positive zoom factor.
    pub zoom: f64,
}

/// Independent versioned presentation layout.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutDocument {
    /// Layout schema version, independent from the control protocol.
    pub schema_version: u32,
    /// Workflow association.
    pub workflow_id: String,
    /// Exact revision association.
    pub revision_id: String,
    /// Optimistic update generation.
    pub generation: u64,
    /// Bounded author reference supplied from authenticated context on write.
    pub author: String,
    /// Digest over the complete document with an empty digest field.
    pub digest: String,
    /// Node positions/dimensions keyed by semantic node identity.
    pub nodes: BTreeMap<String, LayoutPoint>,
    /// Collapsed presentation group identities.
    pub collapsed_groups: BTreeSet<String>,
    /// Short non-executable annotations keyed by stable presentation identity.
    pub annotations: BTreeMap<String, String>,
    /// Optional canvas viewport preference.
    pub viewport: Option<LayoutViewport>,
}

impl LayoutDocument {
    /// Validates associations, finite coordinates, counts, byte size, and digest.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.schema_version != LAYOUT_SCHEMA_VERSION {
            return Err(ProtocolError::UnsupportedMajor {
                found: u16::try_from(self.schema_version).unwrap_or(u16::MAX),
                supported: u16::try_from(LAYOUT_SCHEMA_VERSION).unwrap_or(u16::MAX),
            });
        }
        validate_identifier("layout.workflow_id", &self.workflow_id, 192)?;
        validate_identifier("layout.revision_id", &self.revision_id, 192)?;
        validate_identifier("layout.author", &self.author, 192)?;
        if self.generation == 0
            || self.nodes.len() > 4_096
            || self.collapsed_groups.len() > 1_024
            || self.annotations.len() > 1_024
        {
            return Err(ProtocolError::Bounds(
                "layout generation/count bounds are invalid".to_owned(),
            ));
        }
        for (node, point) in &self.nodes {
            validate_identifier("layout.node", node, 192)?;
            if !point.x.is_finite()
                || !point.y.is_finite()
                || point
                    .width
                    .is_some_and(|value| !value.is_finite() || value <= 0.0)
                || point
                    .height
                    .is_some_and(|value| !value.is_finite() || value <= 0.0)
            {
                return Err(ProtocolError::InvalidContract(
                    "layout coordinates must be finite and dimensions positive".to_owned(),
                ));
            }
        }
        for (identity, annotation) in &self.annotations {
            validate_identifier("layout.annotation", identity, 192)?;
            if annotation.len() > 4_096 {
                return Err(ProtocolError::Bounds(
                    "layout annotation exceeds 4096 bytes".to_owned(),
                ));
            }
        }
        if let Some(viewport) = self.viewport
            && (!viewport.x.is_finite()
                || !viewport.y.is_finite()
                || !viewport.zoom.is_finite()
                || !(0.01..=100.0).contains(&viewport.zoom))
        {
            return Err(ProtocolError::InvalidContract(
                "layout viewport is invalid".to_owned(),
            ));
        }
        let encoded = encode_json(self)?;
        if encoded.len() > MAX_LAYOUT_BYTES {
            return Err(ProtocolError::Bounds(format!(
                "layout exceeds {MAX_LAYOUT_BYTES} bytes"
            )));
        }
        let expected = self.computed_digest()?;
        if self.digest != expected {
            return Err(ProtocolError::InvalidContract(
                "layout digest does not match its content".to_owned(),
            ));
        }
        Ok(())
    }

    /// Computes the domain-separated content digest without semantic blueprint data.
    pub fn computed_digest(&self) -> Result<String, ProtocolError> {
        let mut unsigned = self.clone();
        unsigned.digest.clear();
        let bytes = serde_json::to_vec(&unsigned)
            .map_err(|error| ProtocolError::InvalidJson(error.to_string()))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.layout.v1\0");
        hasher.update(&bytes);
        Ok(format!("b3_{}", hasher.finalize()))
    }

    /// Replaces the digest with the value computed from current content.
    pub fn seal(mut self) -> Result<Self, ProtocolError> {
        self.digest = self.computed_digest()?;
        self.validate()?;
        Ok(self)
    }
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
