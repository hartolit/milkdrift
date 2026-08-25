use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use milkdrift_contracts::{
    CanonicalJsonError, JsonBoundKind, JsonBoundViolation, JsonLimits,
    canonical_json_bytes as encode_canonical_json,
};
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

use crate::{
    BoundedDetail, IntegrityDigest, PersistenceError, RunEventEnvelope, RunSequence, SnapshotId,
};

/// Current optional projection-snapshot envelope schema.
pub const SNAPSHOT_ENVELOPE_SCHEMA_VERSION_V2: u32 = 2;
/// Maximum decoded runtime-owned projection payload.
pub const MAX_SNAPSHOT_PAYLOAD_BYTES: usize = 4_194_304;
/// Maximum RFC 4648 padded Base64 characters for the decoded payload bound.
pub const MAX_SNAPSHOT_ENCODED_PAYLOAD_BYTES: usize = MAX_SNAPSHOT_PAYLOAD_BYTES.div_ceil(3) * 4;
// The non-payload wire consists of fixed field names/punctuation, two bounded ASCII
// identities (192 + 128 bytes), two 67-byte digests, and three bounded decimal
// integers. Those values require fewer than 1,024 bytes even at every maximum.
const MAX_SNAPSHOT_DOCUMENT_METADATA_BYTES: usize = 1_024;
/// Maximum encoded snapshot envelope: maximum Base64 payload plus bounded metadata.
pub const MAX_SNAPSHOT_DOCUMENT_BYTES: usize =
    MAX_SNAPSHOT_ENCODED_PAYLOAD_BYTES + MAX_SNAPSHOT_DOCUMENT_METADATA_BYTES;
const SNAPSHOT_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: 2,
    maximum_string_bytes: MAX_SNAPSHOT_ENCODED_PAYLOAD_BYTES,
    maximum_key_bytes: 64,
    maximum_container_items: 16,
};
const SNAPSHOT_CHECKSUM_DOMAIN: &[u8] = b"milkdrift.projection-snapshot-envelope.checksum.v2\0";
const PROJECTION_CHECKPOINT_DOMAIN: &[u8] = b"milkdrift.projection-payload-checkpoint.v1\0";
const HISTORY_GENESIS_DOMAIN: &[u8] = b"milkdrift.run-history-chain.genesis.v1\0";
const HISTORY_LINK_DOMAIN: &[u8] = b"milkdrift.run-history-chain.link.v1\0";
const HISTORY_GENESIS_MARKER: &[u8] = b"genesis";

/// Durable commitment to runtime projection bytes derived at an accepted journal head.
///
/// The commitment is adapter-neutral but is not an independently writable projection:
/// storage associates it atomically with the authoritative event append that produced it.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionCheckpoint {
    payload_schema: u32,
    payload_digest: IntegrityDigest,
}

impl ProjectionCheckpoint {
    /// Commits to one exact, bounded, non-empty projection payload.
    pub fn new(payload_schema: u32, payload: &[u8]) -> Result<Self, PersistenceError> {
        if payload_schema == 0 || payload.is_empty() {
            return Err(PersistenceError::InvalidDocument(
                "projection checkpoint schema and payload must be non-zero/non-empty".to_owned(),
            ));
        }
        if payload.len() > MAX_SNAPSHOT_PAYLOAD_BYTES {
            return Err(PersistenceError::Bounds {
                location: "projection_checkpoint.payload",
                reason: format!("exceeds {MAX_SNAPSHOT_PAYLOAD_BYTES} bytes"),
            });
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update(PROJECTION_CHECKPOINT_DOMAIN);
        hasher.update(&payload_schema.to_be_bytes());
        let length = u64::try_from(payload.len()).map_err(|_| PersistenceError::Bounds {
            location: "projection_checkpoint.payload",
            reason: "payload length exceeds u64".to_owned(),
        })?;
        hasher.update(&length.to_be_bytes());
        hasher.update(payload);
        Ok(Self {
            payload_schema,
            payload_digest: IntegrityDigest::new(format!("b3_{}", hasher.finalize()))?,
        })
    }

    /// Runtime-owned payload schema included in the commitment.
    #[must_use]
    pub const fn payload_schema(&self) -> u32 {
        self.payload_schema
    }

    /// Domain-separated digest of the exact payload bytes.
    #[must_use]
    pub const fn payload_digest(&self) -> &IntegrityDigest {
        &self.payload_digest
    }
}

/// Computes the documented digest of one complete contiguous event prefix.
///
/// The input must start at sequence one, belong to one run, and be contiguous. Each
/// versioned, domain-separated link binds the run identity, sequence, previous digest,
/// and canonical envelope checksum.
pub fn history_digest(events: &[RunEventEnvelope]) -> Result<IntegrityDigest, PersistenceError> {
    let first = events.first().ok_or_else(|| {
        PersistenceError::InvalidDocument(
            "a snapshot history digest requires at least one event".to_owned(),
        )
    })?;
    if first.sequence() != RunSequence::FIRST {
        return Err(PersistenceError::InvalidDocument(
            "snapshot history must begin at sequence one".to_owned(),
        ));
    }
    let mut digest = history_genesis_digest(first.run_id())?;
    let mut expected = RunSequence::FIRST;
    for event in events {
        if event.run_id() != first.run_id() || event.sequence() != expected {
            return Err(PersistenceError::InvalidDocument(
                "snapshot history must be one run's contiguous prefix".to_owned(),
            ));
        }
        digest = history_link_digest(event.run_id(), event.sequence(), &digest, event.checksum())?;
        expected = expected.next()?;
    }
    Ok(digest)
}

fn framed_history_length(
    bytes: &[u8],
    location: &'static str,
) -> Result<[u8; 4], PersistenceError> {
    u32::try_from(bytes.len())
        .map(u32::to_be_bytes)
        .map_err(|_| PersistenceError::Bounds {
            location,
            reason: "framed history-chain value exceeds u32".to_owned(),
        })
}

/// Computes the domain-separated genesis digest for one run history chain.
pub fn history_genesis_digest(run: &RunId) -> Result<IntegrityDigest, PersistenceError> {
    let run_bytes = run.as_str().as_bytes();
    let mut hasher = blake3::Hasher::new();
    hasher.update(HISTORY_GENESIS_DOMAIN);
    hasher.update(&framed_history_length(run_bytes, "history.run_id")?);
    hasher.update(run_bytes);
    hasher.update(&framed_history_length(
        HISTORY_GENESIS_MARKER,
        "history.genesis_marker",
    )?);
    hasher.update(HISTORY_GENESIS_MARKER);
    IntegrityDigest::new(format!("b3_{}", hasher.finalize()))
}

/// Computes one domain-separated link in a run history chain.
pub fn history_link_digest(
    run: &RunId,
    sequence: RunSequence,
    previous: &IntegrityDigest,
    checksum: &IntegrityDigest,
) -> Result<IntegrityDigest, PersistenceError> {
    let run_bytes = run.as_str().as_bytes();
    let previous_bytes = previous.as_str().as_bytes();
    let checksum_bytes = checksum.as_str().as_bytes();
    let mut hasher = blake3::Hasher::new();
    hasher.update(HISTORY_LINK_DOMAIN);
    hasher.update(&framed_history_length(run_bytes, "history.run_id")?);
    hasher.update(run_bytes);
    hasher.update(&sequence.get().to_be_bytes());
    hasher.update(&framed_history_length(
        previous_bytes,
        "history.previous_digest",
    )?);
    hasher.update(previous_bytes);
    hasher.update(&framed_history_length(checksum_bytes, "history.checksum")?);
    hasher.update(checksum_bytes);
    IntegrityDigest::new(format!("b3_{}", hasher.finalize()))
}

/// Optional optimization over authoritative events; never independently writable state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotDocument {
    envelope_schema_version: u32,
    snapshot: SnapshotId,
    run: RunId,
    covered_sequence: RunSequence,
    history_digest: IntegrityDigest,
    projection_payload_schema: u32,
    payload: Vec<u8>,
    checksum: IntegrityDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWire {
    schema_version: u32,
    snapshot: SnapshotId,
    run: RunId,
    covered_sequence: RunSequence,
    history_digest: IntegrityDigest,
    projection_payload_schema: u32,
    encoded_payload: String,
    checksum: IntegrityDigest,
}

#[derive(Serialize)]
struct SnapshotWireRef<'a> {
    schema_version: u32,
    snapshot: &'a SnapshotId,
    run: &'a RunId,
    covered_sequence: RunSequence,
    history_digest: &'a IntegrityDigest,
    projection_payload_schema: u32,
    encoded_payload: &'a str,
    checksum: &'a IntegrityDigest,
}

impl SnapshotDocument {
    /// Creates and checksums a bounded snapshot.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        snapshot: SnapshotId,
        run: RunId,
        covered_sequence: RunSequence,
        history_digest: IntegrityDigest,
        projection_payload_schema: u32,
        payload: Vec<u8>,
    ) -> Result<Self, PersistenceError> {
        validate_snapshot_fields(covered_sequence, projection_payload_schema, &payload)?;
        let checksum = snapshot_checksum(
            &snapshot,
            &run,
            covered_sequence,
            &history_digest,
            projection_payload_schema,
            &payload,
        )?;
        Ok(Self {
            envelope_schema_version: SNAPSHOT_ENVELOPE_SCHEMA_VERSION_V2,
            snapshot,
            run,
            covered_sequence,
            history_digest,
            projection_payload_schema,
            payload,
            checksum,
        })
    }

    /// Snapshot-envelope wire schema, independent of the runtime payload schema.
    #[must_use]
    pub const fn envelope_schema_version(&self) -> u32 {
        self.envelope_schema_version
    }

    /// Snapshot identity.
    #[must_use]
    pub const fn snapshot(&self) -> &SnapshotId {
        &self.snapshot
    }

    /// Owning run.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }

    /// Exact event prefix covered.
    #[must_use]
    pub const fn covered_sequence(&self) -> RunSequence {
        self.covered_sequence
    }

    /// Digest of the exact checksummed event prefix.
    #[must_use]
    pub const fn history_digest(&self) -> &IntegrityDigest {
        &self.history_digest
    }

    /// Runtime-owned projection schema.
    #[must_use]
    pub const fn projection_payload_schema(&self) -> u32 {
        self.projection_payload_schema
    }

    /// Bounded runtime-owned projection bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Computes the append-time commitment that must authorize this payload.
    pub fn payload_checkpoint(&self) -> Result<ProjectionCheckpoint, PersistenceError> {
        ProjectionCheckpoint::new(self.projection_payload_schema, &self.payload)
    }

    /// Snapshot integrity checksum.
    #[must_use]
    pub const fn checksum(&self) -> &IntegrityDigest {
        &self.checksum
    }

    /// Encodes deterministic compact canonical JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, PersistenceError> {
        let encoded_payload = snapshot_base64().encode(&self.payload);
        debug_assert!(encoded_payload.len() <= MAX_SNAPSHOT_ENCODED_PAYLOAD_BYTES);
        let bytes = encode_canonical_json(
            &SnapshotWireRef {
                schema_version: self.envelope_schema_version,
                snapshot: &self.snapshot,
                run: &self.run,
                covered_sequence: self.covered_sequence,
                history_digest: &self.history_digest,
                projection_payload_schema: self.projection_payload_schema,
                encoded_payload: &encoded_payload,
                checksum: &self.checksum,
            },
            SNAPSHOT_JSON_LIMITS,
        )
        .map_err(|error| match error {
            CanonicalJsonError::Json(error) => PersistenceError::Json(error),
            CanonicalJsonError::Bounds(violation) => snapshot_json_bound(violation),
        })?;
        if bytes.len() > MAX_SNAPSHOT_DOCUMENT_BYTES {
            return Err(PersistenceError::Bounds {
                location: "snapshot.document",
                reason: format!("canonical JSON exceeds {MAX_SNAPSHOT_DOCUMENT_BYTES} bytes"),
            });
        }
        Ok(bytes)
    }

    /// Decodes and verifies a persisted snapshot document.
    pub fn from_json(bytes: &[u8]) -> Result<Self, PersistenceError> {
        if bytes.len() > MAX_SNAPSHOT_DOCUMENT_BYTES {
            return Err(PersistenceError::Bounds {
                location: "snapshot.document",
                reason: format!("exceeds {MAX_SNAPSHOT_DOCUMENT_BYTES} bytes"),
            });
        }
        let value = crate::document::parse_json_without_duplicates(bytes)?;
        milkdrift_contracts::validate_json_value(&value, SNAPSHOT_JSON_LIMITS)
            .map_err(snapshot_json_bound)?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                PersistenceError::InvalidDocument(
                    "snapshot requires a numeric u32 schema_version".to_owned(),
                )
            })?;
        if version != SNAPSHOT_ENVELOPE_SCHEMA_VERSION_V2 {
            return Err(PersistenceError::UnsupportedVersion {
                document: "snapshot_envelope",
                found: version,
                supported: SNAPSHOT_ENVELOPE_SCHEMA_VERSION_V2,
            });
        }
        let wire: SnapshotWire = serde_json::from_value(value)?;
        if wire.encoded_payload.len() > MAX_SNAPSHOT_ENCODED_PAYLOAD_BYTES {
            return Err(PersistenceError::Bounds {
                location: "snapshot.encoded_payload",
                reason: format!("exceeds {MAX_SNAPSHOT_ENCODED_PAYLOAD_BYTES} bytes"),
            });
        }
        let payload = snapshot_base64()
            .decode(&wire.encoded_payload)
            .map_err(|cause| {
                PersistenceError::InvalidDocument(format!(
                    "snapshot encoded_payload is not canonical padded standard Base64: {cause}"
                ))
            })?;
        validate_snapshot_fields(
            wire.covered_sequence,
            wire.projection_payload_schema,
            &payload,
        )?;
        let expected = snapshot_checksum(
            &wire.snapshot,
            &wire.run,
            wire.covered_sequence,
            &wire.history_digest,
            wire.projection_payload_schema,
            &payload,
        )?;
        if expected != wire.checksum {
            return Err(PersistenceError::Corruption(format!(
                "snapshot {} checksum mismatch",
                wire.snapshot
            )));
        }
        Ok(Self {
            envelope_schema_version: wire.schema_version,
            snapshot: wire.snapshot,
            run: wire.run,
            covered_sequence: wire.covered_sequence,
            history_digest: wire.history_digest,
            projection_payload_schema: wire.projection_payload_schema,
            payload,
            checksum: wire.checksum,
        })
    }
}

fn validate_snapshot_fields(
    covered_sequence: RunSequence,
    projection_payload_schema: u32,
    payload: &[u8],
) -> Result<(), PersistenceError> {
    if covered_sequence == RunSequence::ZERO || projection_payload_schema == 0 || payload.is_empty()
    {
        return Err(PersistenceError::InvalidDocument(
            "snapshot sequence, projection schema, and payload must be non-zero/non-empty"
                .to_owned(),
        ));
    }
    if payload.len() > MAX_SNAPSHOT_PAYLOAD_BYTES {
        return Err(PersistenceError::Bounds {
            location: "snapshot.payload",
            reason: format!("exceeds {MAX_SNAPSHOT_PAYLOAD_BYTES} bytes"),
        });
    }
    Ok(())
}

fn snapshot_checksum(
    snapshot: &SnapshotId,
    run: &RunId,
    covered_sequence: RunSequence,
    history_digest: &IntegrityDigest,
    projection_payload_schema: u32,
    payload: &[u8],
) -> Result<IntegrityDigest, PersistenceError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(SNAPSHOT_CHECKSUM_DOMAIN);
    hasher.update(&SNAPSHOT_ENVELOPE_SCHEMA_VERSION_V2.to_be_bytes());
    snapshot_checksum_frame(&mut hasher, snapshot.as_str().as_bytes())?;
    snapshot_checksum_frame(&mut hasher, run.as_str().as_bytes())?;
    hasher.update(&covered_sequence.get().to_be_bytes());
    snapshot_checksum_frame(&mut hasher, history_digest.as_str().as_bytes())?;
    hasher.update(&projection_payload_schema.to_be_bytes());
    snapshot_checksum_frame(&mut hasher, payload)?;
    IntegrityDigest::new(format!("b3_{}", hasher.finalize()))
}

fn snapshot_base64() -> GeneralPurpose {
    GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_encode_padding(true)
            .with_decode_padding_mode(DecodePaddingMode::RequireCanonical)
            .with_decode_allow_trailing_bits(false),
    )
}

fn snapshot_checksum_frame(
    hasher: &mut blake3::Hasher,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let length = u64::try_from(bytes.len()).map_err(|_| PersistenceError::Bounds {
        location: "snapshot.checksum",
        reason: "checksum field length exceeds u64".to_owned(),
    })?;
    hasher.update(&length.to_be_bytes());
    hasher.update(bytes);
    Ok(())
}

fn snapshot_json_bound(violation: JsonBoundViolation) -> PersistenceError {
    let kind = match violation.kind() {
        JsonBoundKind::Depth => "depth",
        JsonBoundKind::String => "string",
        JsonBoundKind::Key => "key",
        JsonBoundKind::Array => "array",
        JsonBoundKind::Object => "object",
    };
    PersistenceError::Bounds {
        location: "snapshot.document",
        reason: format!(
            "{} exceeds {kind} limit {}",
            violation.path(),
            violation.maximum()
        ),
    }
}

/// Result of loading the latest snapshot optimization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotLoad {
    /// No snapshot optimization exists.
    Absent,
    /// Integrity, covered-history digest, and append-time projection commitment were verified.
    Verified(SnapshotDocument),
    /// Snapshot bytes/index/history linkage were invalid; caller must replay events.
    Rejected {
        /// Snapshot identity when recoverable from its index.
        snapshot: Option<SnapshotId>,
        /// Bounded diagnostic suitable for health reporting.
        reason: BoundedDetail,
    },
}

/// Optional snapshot optimization port.
pub trait SnapshotStore: Send + Sync {
    /// Returns the cumulative digest of one exact authoritative event prefix.
    ///
    /// Adapters should use their append-time chain checkpoint rather than
    /// replaying the prefix. The digest is an input to a runtime-owned snapshot;
    /// authoritative events remain the sole source of truth.
    fn history_digest(
        &self,
        run: &RunId,
        through: RunSequence,
    ) -> Result<IntegrityDigest, PersistenceError>;

    /// Stores an immutable snapshot and advances the latest pointer atomically.
    ///
    /// Implementations verify `covered_sequence` is not beyond the journal head,
    /// recompute `history_digest` from exact authoritative event envelopes, and require
    /// an equal projection commitment recorded atomically at that journal sequence.
    fn put_snapshot(&self, snapshot: &SnapshotDocument) -> Result<(), PersistenceError>;

    /// Loads and validates the latest candidate. A rejected snapshot is never used to
    /// repair or replace history; callers replay authoritative events from sequence one.
    fn latest_snapshot(&self, run: &RunId) -> Result<SnapshotLoad, PersistenceError>;

    /// Removes only an optional optimization. Authoritative events are untouched.
    fn discard_snapshot(&self, run: &RunId, snapshot: &SnapshotId) -> Result<(), PersistenceError>;
}
