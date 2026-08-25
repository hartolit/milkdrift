use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    BoundedDetail, IntegrityDigest, PersistenceError, RunEventEnvelope, RunSequence, SnapshotId,
};

/// Current optional projection-snapshot envelope schema.
pub const SNAPSHOT_SCHEMA_VERSION_V1: u32 = 1;
/// Maximum serialized projection payload.
pub const MAX_SNAPSHOT_PAYLOAD_BYTES: usize = 4_194_304;
const MAX_SNAPSHOT_DOCUMENT_BYTES: usize = 25_165_824;
const MAX_SNAPSHOT_JSON_DEPTH: usize = 64;
const MAX_SNAPSHOT_JSON_STRING_BYTES: usize = 65_536;
const MAX_SNAPSHOT_JSON_CONTAINER_ITEMS: usize = 4_096;
const SNAPSHOT_CHECKSUM_DOMAIN: &str = "milkdrift.projection-snapshot.v1";
const HISTORY_GENESIS_DOMAIN: &[u8] = b"milkdrift.run-history-chain.genesis.v1\0";
const HISTORY_LINK_DOMAIN: &[u8] = b"milkdrift.run-history-chain.link.v1\0";
const HISTORY_GENESIS_MARKER: &[u8] = b"genesis";

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
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotDocument {
    schema_version: u32,
    snapshot: SnapshotId,
    run: RunId,
    covered_sequence: RunSequence,
    history_digest: IntegrityDigest,
    projection_schema: u32,
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
    projection_schema: u32,
    payload: Vec<u8>,
    checksum: IntegrityDigest,
}

#[derive(Serialize)]
struct SnapshotChecksumInput<'a> {
    domain: &'static str,
    schema_version: u32,
    snapshot: &'a SnapshotId,
    run: &'a RunId,
    covered_sequence: RunSequence,
    history_digest: &'a IntegrityDigest,
    projection_schema: u32,
    payload: &'a [u8],
}

impl SnapshotDocument {
    /// Creates and checksums a bounded snapshot.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        snapshot: SnapshotId,
        run: RunId,
        covered_sequence: RunSequence,
        history_digest: IntegrityDigest,
        projection_schema: u32,
        payload: Vec<u8>,
    ) -> Result<Self, PersistenceError> {
        validate_snapshot_fields(covered_sequence, projection_schema, &payload)?;
        let checksum = snapshot_checksum(
            &snapshot,
            &run,
            covered_sequence,
            &history_digest,
            projection_schema,
            &payload,
        )?;
        Ok(Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION_V1,
            snapshot,
            run,
            covered_sequence,
            history_digest,
            projection_schema,
            payload,
            checksum,
        })
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
    pub const fn projection_schema(&self) -> u32 {
        self.projection_schema
    }

    /// Bounded runtime-owned projection bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Snapshot integrity checksum.
    #[must_use]
    pub const fn checksum(&self) -> &IntegrityDigest {
        &self.checksum
    }

    /// Encodes deterministic compact canonical JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, PersistenceError> {
        snapshot_canonical_json_bytes(self)
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
        validate_snapshot_json_value(&value, "$", 0)?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                PersistenceError::InvalidDocument(
                    "snapshot requires a numeric u32 schema_version".to_owned(),
                )
            })?;
        if version != SNAPSHOT_SCHEMA_VERSION_V1 {
            return Err(PersistenceError::UnsupportedVersion {
                document: "snapshot",
                found: version,
                supported: SNAPSHOT_SCHEMA_VERSION_V1,
            });
        }
        let wire: SnapshotWire = serde_json::from_value(value)?;
        validate_snapshot_fields(wire.covered_sequence, wire.projection_schema, &wire.payload)?;
        let expected = snapshot_checksum(
            &wire.snapshot,
            &wire.run,
            wire.covered_sequence,
            &wire.history_digest,
            wire.projection_schema,
            &wire.payload,
        )?;
        if expected != wire.checksum {
            return Err(PersistenceError::Corruption(format!(
                "snapshot {} checksum mismatch",
                wire.snapshot
            )));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            snapshot: wire.snapshot,
            run: wire.run,
            covered_sequence: wire.covered_sequence,
            history_digest: wire.history_digest,
            projection_schema: wire.projection_schema,
            payload: wire.payload,
            checksum: wire.checksum,
        })
    }
}

fn validate_snapshot_fields(
    covered_sequence: RunSequence,
    projection_schema: u32,
    payload: &[u8],
) -> Result<(), PersistenceError> {
    if covered_sequence == RunSequence::ZERO || projection_schema == 0 || payload.is_empty() {
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
    projection_schema: u32,
    payload: &[u8],
) -> Result<IntegrityDigest, PersistenceError> {
    let input = SnapshotChecksumInput {
        domain: SNAPSHOT_CHECKSUM_DOMAIN,
        schema_version: SNAPSHOT_SCHEMA_VERSION_V1,
        snapshot,
        run,
        covered_sequence,
        history_digest,
        projection_schema,
        payload,
    };
    Ok(IntegrityDigest::hash(&snapshot_canonical_json_bytes(
        &input,
    )?))
}

fn snapshot_canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, PersistenceError> {
    let mut value = serde_json::to_value(value)?;
    validate_snapshot_json_value(&value, "$", 0)?;
    sort_snapshot_json_value(&mut value);
    let bytes = serde_json::to_vec(&value)?;
    if bytes.len() > MAX_SNAPSHOT_DOCUMENT_BYTES {
        return Err(PersistenceError::Bounds {
            location: "snapshot.document",
            reason: format!("canonical JSON exceeds {MAX_SNAPSHOT_DOCUMENT_BYTES} bytes"),
        });
    }
    Ok(bytes)
}

fn validate_snapshot_json_value(
    value: &Value,
    path: &str,
    depth: usize,
) -> Result<(), PersistenceError> {
    if depth > MAX_SNAPSHOT_JSON_DEPTH {
        return Err(snapshot_json_bound(
            "snapshot.document.depth",
            format!("{path} exceeds depth {MAX_SNAPSHOT_JSON_DEPTH}"),
        ));
    }
    match value {
        Value::String(text) if text.len() > MAX_SNAPSHOT_JSON_STRING_BYTES => {
            Err(snapshot_json_bound(
                "snapshot.document.string",
                format!("{path} exceeds {MAX_SNAPSHOT_JSON_STRING_BYTES} bytes"),
            ))
        }
        Value::Array(values) => {
            let maximum = if path == "$.payload" {
                MAX_SNAPSHOT_PAYLOAD_BYTES
            } else {
                MAX_SNAPSHOT_JSON_CONTAINER_ITEMS
            };
            if values.len() > maximum {
                return Err(snapshot_json_bound(
                    "snapshot.document.array",
                    format!("{path} exceeds {maximum} entries"),
                ));
            }
            for (index, child) in values.iter().enumerate() {
                validate_snapshot_json_value(child, &format!("{path}[{index}]"), depth + 1)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            if map.len() > MAX_SNAPSHOT_JSON_CONTAINER_ITEMS {
                return Err(snapshot_json_bound(
                    "snapshot.document.object",
                    format!("{path} exceeds {MAX_SNAPSHOT_JSON_CONTAINER_ITEMS} entries"),
                ));
            }
            for (key, child) in map {
                if key.len() > MAX_SNAPSHOT_JSON_STRING_BYTES {
                    return Err(snapshot_json_bound(
                        "snapshot.document.key",
                        format!("key below {path} exceeds {MAX_SNAPSHOT_JSON_STRING_BYTES} bytes"),
                    ));
                }
                validate_snapshot_json_value(child, &format!("{path}.{key}"), depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn snapshot_json_bound(location: &'static str, reason: String) -> PersistenceError {
    PersistenceError::Bounds { location, reason }
}

fn sort_snapshot_json_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for child in map.values_mut() {
                sort_snapshot_json_value(child);
            }
            let previous = std::mem::take(map);
            let mut entries = previous.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            map.extend(entries);
        }
        Value::Array(values) => {
            for child in values {
                sort_snapshot_json_value(child);
            }
        }
        _ => {}
    }
}

/// Result of loading the latest snapshot optimization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotLoad {
    /// No snapshot optimization exists.
    Absent,
    /// Integrity and covered-history digest were verified by storage.
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
    /// Implementations verify `covered_sequence` is not beyond the journal head and
    /// recompute `history_digest` from exact authoritative event envelopes.
    fn put_snapshot(&self, snapshot: &SnapshotDocument) -> Result<(), PersistenceError>;

    /// Loads and validates the latest candidate. A rejected snapshot is never used to
    /// repair or replace history; callers replay authoritative events from sequence one.
    fn latest_snapshot(&self, run: &RunId) -> Result<SnapshotLoad, PersistenceError>;

    /// Removes only an optional optimization. Authoritative events are untouched.
    fn discard_snapshot(&self, run: &RunId, snapshot: &SnapshotId) -> Result<(), PersistenceError>;
}
