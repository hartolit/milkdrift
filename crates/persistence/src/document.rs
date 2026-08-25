use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use milkdrift_contracts::{
    CanonicalJsonError, JsonBoundKind, JsonBoundViolation, JsonLimits,
    canonical_json_bytes as encode_canonical_json,
};

use crate::{
    EventId, IntegrityDigest, PersistenceError, RunEventKind, RunSequence, TimestampMillis,
};

/// Current run-event envelope schema.
pub const RUN_EVENT_SCHEMA_VERSION_V1: u32 = 1;
/// Maximum encoded event size. Larger content belongs in workspace/artifact storage.
pub const MAX_EVENT_DOCUMENT_BYTES: usize = 1_048_576;
const MAX_DOCUMENT_DEPTH: usize = 64;
const MAX_CONTAINER_ITEMS: usize = 4_096;
const MAX_STRING_BYTES: usize = 65_536;
const EVENT_CHECKSUM_DOMAIN: &str = "milkdrift.run-event-envelope.v1";
const PERSISTENCE_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: MAX_DOCUMENT_DEPTH,
    maximum_string_bytes: MAX_STRING_BYTES,
    maximum_key_bytes: MAX_STRING_BYTES,
    maximum_container_items: MAX_CONTAINER_ITEMS,
};

/// Explicit, checksummed schema-v1 append-only event envelope.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunEventEnvelope {
    schema_version: u32,
    event_id: EventId,
    run_id: RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: RunEventKind,
    checksum: IntegrityDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunEventEnvelopeWire {
    schema_version: u32,
    event_id: EventId,
    run_id: RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: RunEventKind,
    checksum: IntegrityDigest,
}

#[derive(Serialize)]
struct EventChecksumInput<'a> {
    domain: &'static str,
    schema_version: u32,
    event_id: &'a EventId,
    run_id: &'a RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: &'a RunEventKind,
}

impl RunEventEnvelope {
    /// Constructs a validated envelope and calculates its canonical checksum.
    pub fn new(
        event_id: EventId,
        run_id: RunId,
        sequence: RunSequence,
        occurred_at: TimestampMillis,
        kind: RunEventKind,
    ) -> Result<Self, PersistenceError> {
        if sequence == RunSequence::ZERO {
            return Err(PersistenceError::InvalidDocument(
                "event sequence must be greater than zero".to_owned(),
            ));
        }
        kind.validate_for_run(&run_id)?;
        let checksum = calculate_event_checksum(
            RUN_EVENT_SCHEMA_VERSION_V1,
            &event_id,
            &run_id,
            sequence,
            occurred_at,
            &kind,
        )?;
        let envelope = Self {
            schema_version: RUN_EVENT_SCHEMA_VERSION_V1,
            event_id,
            run_id,
            sequence,
            occurred_at,
            kind,
            checksum,
        };
        // Enforce the encoded bound at the trusted construction boundary too.
        let _ = envelope.to_canonical_json()?;
        Ok(envelope)
    }

    /// Returns the explicit envelope schema.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the stable event identity.
    #[must_use]
    pub const fn event_id(&self) -> &EventId {
        &self.event_id
    }

    /// Returns the owning run aggregate.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the authoritative per-run event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }

    /// Returns the boundary-clock timestamp recorded with the fact.
    #[must_use]
    pub const fn occurred_at(&self) -> TimestampMillis {
        self.occurred_at
    }

    /// Returns the closed event fact.
    #[must_use]
    pub const fn kind(&self) -> &RunEventKind {
        &self.kind
    }

    /// Returns the canonical envelope checksum.
    #[must_use]
    pub const fn checksum(&self) -> &IntegrityDigest {
        &self.checksum
    }

    /// Encodes deterministic recursively key-sorted compact JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, PersistenceError> {
        canonical_json_bytes(self, MAX_EVENT_DOCUMENT_BYTES)
    }

    /// Bounds-checks, duplicate-key-checks, version-checks, decodes, and verifies JSON.
    ///
    /// Future schemas, malformed fields, and checksum failures are returned explicitly;
    /// none are interpreted as an absent event.
    pub fn from_json(bytes: &[u8]) -> Result<Self, PersistenceError> {
        if bytes.len() > MAX_EVENT_DOCUMENT_BYTES {
            return Err(PersistenceError::Bounds {
                location: "event_document",
                reason: format!("exceeds {MAX_EVENT_DOCUMENT_BYTES} bytes"),
            });
        }
        let value = parse_json_without_duplicates(bytes)?;
        validate_value(&value, "$", 0)?;
        let version = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                PersistenceError::InvalidDocument(
                    "run event requires a numeric u32 schema_version".to_owned(),
                )
            })?;
        if version != RUN_EVENT_SCHEMA_VERSION_V1 {
            return Err(PersistenceError::UnsupportedVersion {
                document: "run_event",
                found: version,
                supported: RUN_EVENT_SCHEMA_VERSION_V1,
            });
        }
        let wire: RunEventEnvelopeWire = serde_json::from_value(value)?;
        wire.kind.validate_for_run(&wire.run_id)?;
        if wire.sequence == RunSequence::ZERO {
            return Err(PersistenceError::InvalidDocument(
                "event sequence must be greater than zero".to_owned(),
            ));
        }
        let expected = calculate_event_checksum(
            wire.schema_version,
            &wire.event_id,
            &wire.run_id,
            wire.sequence,
            wire.occurred_at,
            &wire.kind,
        )?;
        if expected != wire.checksum {
            return Err(PersistenceError::Corruption(format!(
                "event {} checksum mismatch: expected {expected}, stored {}",
                wire.event_id, wire.checksum
            )));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            event_id: wire.event_id,
            run_id: wire.run_id,
            sequence: wire.sequence,
            occurred_at: wire.occurred_at,
            kind: wire.kind,
            checksum: wire.checksum,
        })
    }
}

fn calculate_event_checksum(
    schema_version: u32,
    event_id: &EventId,
    run_id: &RunId,
    sequence: RunSequence,
    occurred_at: TimestampMillis,
    kind: &RunEventKind,
) -> Result<IntegrityDigest, PersistenceError> {
    let input = EventChecksumInput {
        domain: EVENT_CHECKSUM_DOMAIN,
        schema_version,
        event_id,
        run_id,
        sequence,
        occurred_at,
        kind,
    };
    let bytes = canonical_json_bytes(&input, MAX_EVENT_DOCUMENT_BYTES)?;
    Ok(IntegrityDigest::hash(&bytes))
}

pub(crate) fn canonical_json_bytes<T: Serialize>(
    value: &T,
    maximum_bytes: usize,
) -> Result<Vec<u8>, PersistenceError> {
    let bytes =
        encode_canonical_json(value, PERSISTENCE_JSON_LIMITS).map_err(|error| match error {
            CanonicalJsonError::Json(error) => PersistenceError::Json(error),
            CanonicalJsonError::Bounds(violation) => persistence_bound(violation),
        })?;
    if bytes.len() > maximum_bytes {
        return Err(PersistenceError::Bounds {
            location: "document",
            reason: format!("canonical JSON exceeds {maximum_bytes} bytes"),
        });
    }
    Ok(bytes)
}

fn validate_value(value: &Value, location: &str, depth: usize) -> Result<(), PersistenceError> {
    debug_assert_eq!(location, "$", "persistence validation starts at the root");
    debug_assert_eq!(depth, 0, "persistence validation starts at depth zero");
    milkdrift_contracts::validate_json_value(value, PERSISTENCE_JSON_LIMITS)
        .map_err(persistence_bound)
}

pub(crate) fn parse_json_without_duplicates(bytes: &[u8]) -> Result<Value, PersistenceError> {
    milkdrift_contracts::parse_json_without_duplicates(bytes).map_err(PersistenceError::Json)
}

fn persistence_bound(violation: JsonBoundViolation) -> PersistenceError {
    let (location, reason) = match violation.kind() {
        JsonBoundKind::Depth => (
            "document.depth",
            format!("{} exceeds depth {}", violation.path(), violation.maximum()),
        ),
        JsonBoundKind::String => (
            "document.string",
            format!("{} exceeds {} bytes", violation.path(), violation.maximum()),
        ),
        JsonBoundKind::Key => (
            "document.key",
            format!(
                "key below {} exceeds {} bytes",
                violation.path(),
                violation.maximum()
            ),
        ),
        JsonBoundKind::Array => (
            "document.array",
            format!(
                "{} exceeds {} entries",
                violation.path(),
                violation.maximum()
            ),
        ),
        JsonBoundKind::Object => (
            "document.object",
            format!(
                "{} exceeds {} entries",
                violation.path(),
                violation.maximum()
            ),
        ),
    };
    PersistenceError::Bounds { location, reason }
}
