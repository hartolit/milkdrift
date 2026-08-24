use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

use crate::{
    BoundedDetail, IntegrityDigest, PersistenceError, RunEventEnvelope, RunSequence, SnapshotId,
    document::canonical_json_bytes,
};

/// Current optional projection-snapshot envelope schema.
pub const SNAPSHOT_SCHEMA_VERSION_V1: u32 = 1;
/// Maximum serialized projection payload.
pub const MAX_SNAPSHOT_PAYLOAD_BYTES: usize = 4_194_304;
const MAX_SNAPSHOT_DOCUMENT_BYTES: usize = 25_165_824;
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
    let run_bytes = first.run_id().as_str().as_bytes();
    let run_length = u32::try_from(run_bytes.len()).map_err(|_| PersistenceError::Bounds {
        location: "history.run_id",
        reason: "run identity length does not fit u32".to_owned(),
    })?;
    let mut genesis = blake3::Hasher::new();
    genesis.update(HISTORY_GENESIS_DOMAIN);
    genesis.update(&run_length.to_be_bytes());
    genesis.update(run_bytes);
    genesis.update(&(HISTORY_GENESIS_MARKER.len() as u32).to_be_bytes());
    genesis.update(HISTORY_GENESIS_MARKER);
    let mut digest = IntegrityDigest::new(format!("b3_{}", genesis.finalize()))?;
    let mut expected = RunSequence::FIRST;
    for event in events {
        if event.run_id() != first.run_id() || event.sequence() != expected {
            return Err(PersistenceError::InvalidDocument(
                "snapshot history must be one run's contiguous prefix".to_owned(),
            ));
        }
        let checksum = event.checksum().as_str().as_bytes();
        let checksum_length =
            u32::try_from(checksum.len()).map_err(|_| PersistenceError::Bounds {
                location: "history.checksum",
                reason: "checksum length does not fit u32".to_owned(),
            })?;
        let mut link = blake3::Hasher::new();
        link.update(HISTORY_LINK_DOMAIN);
        link.update(&run_length.to_be_bytes());
        link.update(run_bytes);
        link.update(&event.sequence().get().to_be_bytes());
        let previous = digest.as_str().as_bytes();
        let previous_length = u32::try_from(previous.len()).map_err(|_| {
            PersistenceError::Bounds {
                location: "history.previous_digest",
                reason: "previous digest length does not fit u32".to_owned(),
            }
        })?;
        link.update(&previous_length.to_be_bytes());
        link.update(previous);
        link.update(&checksum_length.to_be_bytes());
        link.update(checksum);
        digest = IntegrityDigest::new(format!("b3_{}", link.finalize()))?;
        expected = expected.next()?;
    }
    Ok(digest)
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
        canonical_json_bytes(self, MAX_SNAPSHOT_DOCUMENT_BYTES)
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
    Ok(IntegrityDigest::hash(&canonical_json_bytes(
        &input,
        MAX_SNAPSHOT_DOCUMENT_BYTES,
    )?))
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
