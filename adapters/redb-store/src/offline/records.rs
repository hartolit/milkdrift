//! Bounded, redacted physical diagnostics. Durable decoders retain semantic ownership.
use super::OfflineStore;
use crate::{error, json, schema};
use milkdrift_persistence::{
    ClockWatermarkStore, PageSize, PeerExecutionSnapshot, PersistenceError,
};
use milkdrift_workspace::ArtifactMetadata;
use redb::ReadableTableMetadata;
use serde::{Deserialize, Serialize};
use std::ops::Bound;

/// Closed record families available to file-owner inspection. These are diagnostics,
/// not a general raw-table export.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InspectionFamily {
    /// Task snapshot identity and retained obligation counters, excluding task inputs.
    Workflows,
    /// Exact command replay ownership, excluding requests and responses.
    Receipts,
    /// Serving execution identity and phase, excluding requests and grants.
    Peers,
    /// Permanent peer request replay ownership.
    PeerTombstones,
    /// Artifact identity, size and digest; content is never exported here.
    Artifacts,
    /// Installation state and blockers without executable configuration or secret values.
    Managed,
}

/// One decoded observation; error records retain their cursor position so an operator
/// can continue past damage without claiming the omitted record was healthy.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InspectionRecord {
    /// Verified resource inventory projection; platform data remains outside store backups.
    Managed {
        /// Durable installation state without live platform inspection.
        installation: milkdrift_capability::managed::ManagedResponse,
    },
    /// Runtime snapshot metadata without frozen inputs or capability requests.
    Workflow {
        /// Durable workflow identity.
        workflow: String,
        /// Current compare-and-swap generation.
        generation: u64,
        /// Authenticated immutable owner.
        actor: String,
        /// Guaranteed remaining execution-history growth.
        reserved_bytes: u64,
        /// Guaranteed remaining control receipt count.
        reserved_receipts: u32,
    },
    /// Digests permit identification without exposing canonical request/result content.
    Receipt {
        /// Original idempotent command identity.
        command: String,
        /// Actor that owns the command's receipt scope.
        actor: String,
        /// Fingerprint of the canonical accepted request.
        request_digest: String,
        /// Fingerprint of the canonical retained result.
        result_digest: String,
        /// Durable completion observation, without sampling current time.
        generation: u64,
    },
    /// Durable remote execution/request link and conservative uncertainty classification.
    Peer {
        /// Durable remote execution identity.
        execution: String,
        /// Caller that owns the retained request scope.
        caller: String,
        /// Original request identity bound to this execution.
        request: String,
        /// Fingerprint used for exact request replay/conflict checks.
        request_digest: String,
        /// Whether this projection comes from the retained tombstone.
        archived: bool,
        /// Retained execution/uncertainty classification.
        phase: String,
        /// Last durable observation position.
        observation_sequence: u64,
    },
    /// Names, provenance, causal links, and artifact content are deliberately omitted.
    Artifact {
        /// Logical artifact identity without its provenance or content.
        artifact: String,
        /// Expected immutable content fingerprint.
        digest: String,
        /// Expected content length.
        bytes: u64,
        /// Whether content was checked and its verification outcome.
        content_health: String,
    },
    /// Failed record at an opaque key; details cannot echo corrupt protected payloads.
    Failure {
        /// Opaque position of the damaged record.
        key: String,
        /// Safe failure category without raw protected payloads.
        classification: String,
    },
}

/// One bounded diagnostic page. An empty page with a continuation is not exhaustion.
#[derive(Debug, Serialize)]
pub struct InspectionPage {
    /// Family visited.
    pub family: InspectionFamily,
    /// At most the requested records and 8 MiB of decodable source data. An oversized
    /// value yields one failure without decoding; its full length is still accounted.
    pub records: Vec<InspectionRecord>,
    /// Exact source bytes occupied by visited values, including failed records.
    pub encoded_bytes_visited: u64,
    /// Source-fingerprint/family/mode-bound continuation.
    pub next: Option<String>,
}

/// Cheap storage facts. Counts are physical table lengths, not a health verdict.
#[derive(Debug, Serialize)]
pub struct StorageOverview {
    /// Exact supported physical schema.
    pub storage_schema: u64,
    /// Exact supported internal record format.
    pub document_format: u64,
    /// Source database bytes before private-copy housekeeping.
    pub database_bytes: u64,
    /// Preserved durable clock boundary, without observing current time.
    pub clock_high_water_unix_ms: u64,
    /// Per-family physical row counts. Byte footprint is supplied per inspected page.
    pub row_counts: Vec<(InspectionFamily, u64)>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    source: String,
    family: InspectionFamily,
    hash_artifacts: bool,
    after: Vec<u8>,
}

impl OfflineStore {
    /// Reads schema, clock and constant-time table counts without a history scan.
    ///
    /// # Errors
    /// Returns database/table read failures or a missing/corrupt durable clock watermark.
    pub fn overview(&self) -> Result<StorageOverview, PersistenceError> {
        let read = self.store.database().begin_read().map_err(error::redb)?;
        let mut row_counts = Vec::new();
        macro_rules! count {
            ($family:ident, $table:ident) => {
                row_counts.push((
                    InspectionFamily::$family,
                    read.open_table(schema::$table)
                        .map_err(error::redb)?
                        .len()
                        .map_err(error::redb)?,
                ));
            };
        }
        count!(Workflows, TASK_WORKFLOWS);
        count!(Receipts, TASK_RECEIPTS);
        count!(Peers, PEER_EXECUTIONS);
        count!(PeerTombstones, PEER_EXECUTION_TOMBSTONES);
        count!(Artifacts, ARTIFACT_METADATA);
        count!(Managed, MANAGED_INSTALLATIONS);
        Ok(StorageOverview {
            storage_schema: schema::STORAGE_SCHEMA_VERSION,
            document_format: schema::INTERNAL_DOCUMENT_FORMAT_VERSION,
            database_bytes: self.database_bytes,
            clock_high_water_unix_ms: self
                .store
                .clock_watermark()?
                .ok_or_else(|| error::corruption("clock watermark missing"))?
                .get(),
            row_counts,
        })
    }

    /// Reads at most 128 records, retaining corruption explicitly. Artifact hashing
    /// is opt-in and cursor-bound; other families refuse that option.
    ///
    /// # Errors
    /// Refuses more than 128 records, hashing outside the artifact family, and malformed,
    /// oversized or source/family/mode-incompatible cursors. Returns table traversal failures;
    /// individual unreadable records are represented by [`InspectionRecord::Failure`].
    pub fn inspect(
        &self,
        family: InspectionFamily,
        limit: PageSize,
        after: Option<&str>,
        hash_artifacts: bool,
    ) -> Result<InspectionPage, PersistenceError> {
        if limit.get() > 128 || (hash_artifacts && family != InspectionFamily::Artifacts) {
            return Err(error::corruption(
                "inspection requires 1..128 records and artifact hashing only for the artifact family",
            ));
        }
        let cursor = after
            .map(|value| -> Result<Cursor, PersistenceError> {
                if value.len() > 8192 {
                    return Err(PersistenceError::InvalidCursor(
                        "inspection cursor exceeds 8192 bytes".into(),
                    ));
                }
                let cursor: Cursor = serde_json::from_str(value)?;
                if cursor.source != self.database_digest
                    || cursor.family != family
                    || cursor.hash_artifacts != hash_artifacts
                    || cursor.after.len() > 512
                    || cursor.after.is_empty()
                {
                    return Err(PersistenceError::InvalidCursor(
                        "inspection cursor belongs to another source/family/mode".into(),
                    ));
                }
                Ok(cursor)
            })
            .transpose()?;
        let lower = cursor.as_ref().map(|c| c.after.as_slice());
        let read = self.store.database().begin_read().map_err(error::redb)?;
        let mut result = InspectionPage {
            family,
            records: Vec::new(),
            encoded_bytes_visited: 0,
            next: None,
        };
        let mut last = None;
        let mut visit = |key: &[u8], bytes: &[u8]| -> Result<bool, PersistenceError> {
            if key.len() > 512 {
                return Err(PersistenceError::Bounds {
                    location: "offline_record_key",
                    reason: "record key exceeds 512 bytes".into(),
                });
            }
            if result.records.len() == limit.get() as usize
                || (!result.records.is_empty()
                    && result.encoded_bytes_visited + bytes.len() as u64 > 8 * 1024 * 1024)
            {
                result.next = Some(serde_json::to_string(&Cursor {
                    source: self.database_digest.clone(),
                    family,
                    hash_artifacts,
                    after: last
                        .take()
                        .ok_or_else(|| error::corruption("inspection made no progress"))?,
                })?);
                return Ok(false);
            }
            last = Some(key.to_vec());
            result.encoded_bytes_visited += bytes.len() as u64;
            let decoded = if bytes.len() > 4 * 1024 * 1024 {
                Err(PersistenceError::Bounds {
                    location: "offline_record",
                    reason: "record exceeds 4 MiB diagnostic bound".into(),
                })
            } else {
                self.decode_inspection(family, key, bytes, hash_artifacts)
            };
            result.records.push(decoded.unwrap_or_else(|cause| {
                InspectionRecord::Failure {
                    key: key.iter().map(|b| format!("{b:02x}")).collect(),
                    classification: match cause {
                        PersistenceError::UnsupportedVersion { .. } => "unsupported_schema",
                        PersistenceError::Bounds { .. } => "diagnostic_bound_exceeded",
                        _ => "corrupt_record",
                    }
                    .into(),
                }
            }));
            Ok(true)
        };
        macro_rules! text_table {
            ($table:ident) => {{
                let lower = lower
                    .map(std::str::from_utf8)
                    .transpose()
                    .map_err(|_| PersistenceError::InvalidCursor("invalid text key".into()))?;
                let table = read.open_table(schema::$table).map_err(error::redb)?;
                for row in table
                    .range::<&str>((
                        lower.map_or(Bound::Unbounded, Bound::Excluded),
                        Bound::Unbounded,
                    ))
                    .map_err(error::redb)?
                {
                    let (key, bytes) = row.map_err(error::redb)?;
                    if !visit(key.value().as_bytes(), bytes.value())? {
                        break;
                    }
                }
            }};
        }
        match family {
            InspectionFamily::Workflows => text_table!(TASK_WORKFLOWS),
            InspectionFamily::Receipts => text_table!(TASK_RECEIPTS),
            InspectionFamily::Peers => text_table!(PEER_EXECUTIONS),
            InspectionFamily::PeerTombstones => text_table!(PEER_EXECUTION_TOMBSTONES),
            InspectionFamily::Artifacts => text_table!(ARTIFACT_METADATA),
            InspectionFamily::Managed => text_table!(MANAGED_INSTALLATIONS),
        }
        Ok(result)
    }

    fn decode_inspection(
        &self,
        family: InspectionFamily,
        key: &[u8],
        bytes: &[u8],
        hash_artifacts: bool,
    ) -> Result<InspectionRecord, PersistenceError> {
        let text_key =
            || std::str::from_utf8(key).map_err(|_| error::corruption("invalid record key"));
        match family {
            InspectionFamily::Managed => {
                let record = crate::managed::decode_record(bytes)?;
                if record.name.as_str() != text_key()? {
                    return Err(error::corruption("managed inventory key differs"));
                }
                Ok(InspectionRecord::Managed {
                    installation: record.view(),
                })
            }
            InspectionFamily::Workflows => {
                crate::task::validate_snapshot_row(text_key()?, bytes)?;
                let snapshot: milkdrift_persistence::WorkflowSnapshot =
                    json::decode(bytes, "task workflow")?;
                Ok(InspectionRecord::Workflow {
                    workflow: snapshot.workflow.to_string(),
                    generation: snapshot.generation,
                    actor: snapshot.owner_actor,
                    reserved_bytes: snapshot.reserved_bytes,
                    reserved_receipts: snapshot.reserved_receipts,
                })
            }
            InspectionFamily::Receipts => {
                crate::task::validate_receipt_row(text_key()?, bytes)?;
                let receipt: milkdrift_persistence::WorkflowCommandReceipt =
                    json::decode(bytes, "task receipt")?;
                Ok(InspectionRecord::Receipt {
                    command: receipt.acceptance.key,
                    actor: receipt.acceptance.actor,
                    request_digest: blake3::hash(&receipt.acceptance.canonical_request)
                        .to_hex()
                        .to_string(),
                    result_digest: blake3::hash(&serde_json::to_vec(&receipt.acceptance.result)?)
                        .to_hex()
                        .to_string(),
                    generation: receipt.generation,
                })
            }
            InspectionFamily::Peers | InspectionFamily::PeerTombstones => {
                let read = self.store.database().begin_read().map_err(error::redb)?;
                let snapshot = crate::peer::snapshot_in_read_transaction_text(&read, text_key()?)?;
                let archived = matches!(&snapshot, PeerExecutionSnapshot::Archived(_));
                if archived != (family == InspectionFamily::PeerTombstones) {
                    return Err(error::corruption("peer ownership mismatch"));
                }
                let phase = match &snapshot {
                    PeerExecutionSnapshot::Hot(record) => match &record.phase {
                        milkdrift_persistence::PeerExecutionPhase::Uncertain { .. } => "uncertain",
                        phase if phase.is_active() => "active",
                        _ => "terminal",
                    },
                    PeerExecutionSnapshot::Archived(record) => {
                        if record.disposition.is_uncertain() {
                            "uncertain"
                        } else {
                            "terminal"
                        }
                    }
                };
                Ok(InspectionRecord::Peer {
                    execution: snapshot.execution().to_string(),
                    caller: snapshot.caller().to_string(),
                    request: snapshot.request_id().to_string(),
                    request_digest: snapshot.request_digest().to_string(),
                    archived,
                    phase: phase.into(),
                    observation_sequence: snapshot.last_observation_sequence(),
                })
            }
            InspectionFamily::Artifacts => {
                let metadata: ArtifactMetadata = json::decode(bytes, "artifact metadata")?;
                let reference = metadata.reference();
                if reference.artifact().as_str() != text_key()? {
                    return Err(error::corruption("artifact key mismatch"));
                }
                let path = self.store.content_path(reference.digest());
                let content_health = match paths_health(
                    &path,
                    reference,
                    hash_artifacts,
                    self.store.max_artifact_bytes,
                ) {
                    Ok(status) => status,
                    Err(PersistenceError::Storage {
                        class: milkdrift_persistence::StorageFailureClass::Unavailable,
                        ..
                    }) => "unavailable_content",
                    Err(_) => "corrupt_content_path",
                };
                Ok(InspectionRecord::Artifact {
                    artifact: reference.artifact().to_string(),
                    digest: reference.digest().to_hex(),
                    bytes: reference.size_bytes(),
                    content_health: content_health.into(),
                })
            }
        }
    }
}

fn paths_health(
    path: &std::path::Path,
    reference: &milkdrift_workspace::ArtifactReference,
    hash: bool,
    maximum_hash_bytes: u64,
) -> Result<&'static str, PersistenceError> {
    let file = super::paths::read_file(path)?;
    if file.metadata().map_err(error::io)?.len() != reference.size_bytes() {
        return Ok("corrupt_size");
    }
    if !hash {
        return Ok("present_digest_unchecked");
    }
    if reference.size_bytes() > maximum_hash_bytes {
        return Ok("hash_bound_exceeded");
    }
    let (_, digest) = super::paths::hash_file(path)?;
    Ok(if digest == reference.digest().to_hex() {
        "verified"
    } else {
        "corrupt_digest"
    })
}
