use std::{fs, path::Path, time::Instant};

use milkdrift_authority::{ActorRef, GrantDigest, GrantId};
use milkdrift_blueprint::{ContentDigest as BlueprintDigest, RevisionId, WorkflowId};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    ApplicationCommandCommit, ApplicationCommandCommitOutcome, ApplicationCommandEffect,
    ApplicationCommandReceipt, ApplicationCommandResult, ApplicationCommandStore,
    ArtifactPublicationId, ArtifactReadAuthority, ArtifactReadRequest, ArtifactStore,
    AtomicRunCommitRequest, CommandDisposition, CommandId, CommandReceipt, CommandResultDocument,
    EventId, IndexedRunState, IntegrityDigest, Reason, RunEventEnvelope, RunEventKind,
    RunIndexUpdate, RunJournal, RunSequence, RunSummaryIndex, TimestampMillis, WorkspaceAccounting,
    WorkspaceMutation,
};
use milkdrift_redb_store::{RedbStore, RedbStoreConfig};
use milkdrift_runtime::RunProjection;
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType, RunId, ScopeId,
    WorkspaceBudget, WorkspaceScope, WorkspaceUsage,
};
use serde::Serialize;

use crate::{EvidenceResult, ScenarioMeasurement};

const HOT_BOUND: u32 = 8;
const ARCHIVE_BATCH: u32 = 3;

/// Machine-readable storage turnover and recovery report.
#[derive(Clone, Debug, Serialize)]
pub struct StorageEvidence {
    /// Unique application receipts admitted.
    pub receipt_operations: u32,
    /// Complete immutable receipt documents across hot and cold ownership.
    pub receipt_primary_count: u64,
    /// Final bounded hot receipt count.
    pub receipt_hot_count: u64,
    /// Final exact cold receipt count.
    pub receipt_cold_count: u64,
    /// Canonical JSON bytes for every primary receipt document.
    pub receipt_primary_logical_bytes: u64,
    /// Canonical JSON bytes owned by the final hot receipt tier.
    pub receipt_hot_logical_bytes: u64,
    /// Canonical JSON bytes owned by the final cold receipt tier.
    pub receipt_cold_logical_bytes: u64,
    /// Receipt archive transactions observed.
    pub receipt_archive_generation: u64,
    /// Durable store bytes before explicit final archival.
    pub bytes_before_final_archive: u64,
    /// Durable store bytes after explicit final archival.
    pub bytes_after_final_archive: u64,
    /// Store reopen and accounting recovery time.
    pub reopen_microseconds: u64,
    /// Exact replay of the first cold receipt succeeded after reopen.
    pub oldest_cold_replayed: bool,
    /// Journal/projection events used by the prompt-sequence-style history measurement.
    pub journal_events: u64,
    /// Bounded current projection node count after lifetime history replay.
    pub active_projection_nodes: u64,
    /// Peer executions driven through the complete state machine.
    pub peer_executions: u64,
    /// Final peer active execution count.
    pub peer_active_count: u32,
    /// Final peer dispatch-queue count.
    pub peer_dispatch_queued_count: u32,
    /// Final peer hot terminal count.
    pub peer_hot_count: u64,
    /// Peak active count observed during sequential turnover.
    pub peer_peak_active_count: u32,
    /// Peak hot-terminal count observed before bounded archival.
    pub peer_peak_hot_count: u64,
    /// Peer observations measured by the peer-specific scenario.
    pub peer_observations: u64,
    /// Peer tombstones measured by the peer-specific scenario.
    pub peer_tombstones: u64,
    /// Canonical JSON bytes of active snapshots observed before entry.
    pub peer_active_snapshot_logical_bytes: u64,
    /// Canonical JSON bytes of terminal hot snapshots observed before archival.
    pub peer_hot_snapshot_logical_bytes: u64,
    /// Canonical JSON bytes of compact tombstone snapshots after archival.
    pub peer_tombstone_snapshot_logical_bytes: u64,
    /// Canonical JSON bytes appended as peer observation rows.
    pub peer_observation_logical_bytes: u64,
}

/// Appends one accepted command/event transaction through the production redb journal.
pub fn journal_append_one() -> EvidenceResult<ScenarioMeasurement> {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    let request = journal_request("one", 1)?;
    let outcome = store.commit_command(&request)?;
    let bytes = serde_json::to_vec(outcome.result())?;
    Ok(ScenarioMeasurement::new(
        "persistence/journal_append_one",
        1,
        u64::try_from(bytes.len())?,
        &bytes,
    ))
}

/// Appends one maximum-representative bounded event batch in one transaction.
pub fn journal_append_batch() -> EvidenceResult<ScenarioMeasurement> {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    let request = journal_request("batch", 64)?;
    let outcome = store.commit_command(&request)?;
    let bytes = serde_json::to_vec(outcome.result())?;
    Ok(ScenarioMeasurement::new(
        "persistence/journal_append_batch_64",
        64,
        u64::try_from(bytes.len())?,
        &bytes,
    ))
}

/// Rebuilds a projection from representative bounded lifetime history.
pub fn projection_rebuild() -> EvidenceResult<ScenarioMeasurement> {
    let history = projection_history(4_096)?;
    let projection = RunProjection::replay(&history)?;
    let encoded = serde_json::to_vec(&projection)?;
    Ok(ScenarioMeasurement::new(
        "runtime/projection_rebuild_4096",
        u64::try_from(history.len())?,
        u64::try_from(encoded.len())?,
        &encoded,
    ))
}

/// Restores a serialized bounded projection and applies an authoritative tail.
pub fn projection_snapshot_tail() -> EvidenceResult<ScenarioMeasurement> {
    let history = projection_history(4_096)?;
    let split = history.len().saturating_sub(128);
    let checkpoint = RunProjection::replay(&history[..split])?;
    let payload = serde_json::to_vec(&checkpoint)?;
    let mut restored: RunProjection = serde_json::from_slice(&payload)?;
    for event in &history[split..] {
        restored.apply(event)?;
    }
    let encoded = serde_json::to_vec(&restored)?;
    Ok(ScenarioMeasurement::new(
        "runtime/projection_snapshot_plus_tail_128",
        128,
        u64::try_from(payload.len().saturating_add(encoded.len()))?,
        &encoded,
    ))
}

/// Exercises hot/cold exact receipt lookup, replay, conflict-safe commit, and archival.
pub fn application_receipt_paths() -> EvidenceResult<ScenarioMeasurement> {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_application_receipt_lifecycle(HOT_BOUND, ARCHIVE_BATCH),
    )?;
    let mut first = None;
    let mut last = None;
    for index in 0..32 {
        let receipt = application_receipt(index)?;
        if first.is_none() {
            first = Some(receipt.clone());
        }
        last = Some(receipt.clone());
        let outcome = store.commit_application_command(&ApplicationCommandCommit {
            receipt,
            effect: ApplicationCommandEffect::None,
        })?;
        if !matches!(outcome, ApplicationCommandCommitOutcome::Committed) {
            return Err(std::io::Error::other("fresh receipt unexpectedly replayed").into());
        }
    }
    let first = first.ok_or_else(|| std::io::Error::other("first receipt missing"))?;
    let last = last.ok_or_else(|| std::io::Error::other("last receipt missing"))?;
    let cold = store.application_command_receipt(first.actor(), first.command())?;
    let hot = store.application_command_receipt(last.actor(), last.command())?;
    if cold.as_ref() != Some(&first) || hot.as_ref() != Some(&last) {
        return Err(std::io::Error::other("hot/cold receipt lookup disagreed").into());
    }
    let replay = store.commit_application_command(&ApplicationCommandCommit {
        receipt: first,
        effect: ApplicationCommandEffect::None,
    })?;
    if !matches!(replay, ApplicationCommandCommitOutcome::Replayed(_)) {
        return Err(std::io::Error::other("cold receipt did not replay").into());
    }
    let status = store.application_receipt_status()?;
    let encoded = serde_json::to_vec(&(
        status.hot_count,
        status.cold_count,
        status.archive_generation,
    ))?;
    Ok(ScenarioMeasurement::new(
        "persistence/application_receipt_hot_cold_archive",
        35,
        u64::try_from(encoded.len())?,
        &encoded,
    ))
}

/// Publishes and range-reads one representative content-addressed artifact.
pub fn artifact_publication() -> EvidenceResult<ScenarioMeasurement> {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    let bytes = deterministic_bytes(1_048_576);
    let reference = ArtifactReference::new(
        ArtifactId::new("artifact-evidence-range")?,
        ContentDigest::for_bytes(&bytes),
        MediaType::new("application/octet-stream")?,
        u64::try_from(bytes.len())?,
    );
    let metadata = ArtifactMetadata::new(
        reference.clone(),
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("evidence-harness")?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = milkdrift_persistence::BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-evidence-range")?,
        RunId::new("run-evidence-artifact")?,
        metadata,
        WorkspaceBudget::new(0, 0, 0, 1, 2_097_152, 2_097_152)?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&publication)?;
    for (index, chunk) in bytes.chunks(65_536).enumerate() {
        store.write_chunk(
            publication.publication(),
            u64::try_from(index.saturating_mul(65_536))?,
            chunk,
        )?;
    }
    store.commit_publication(publication.publication())?;
    let read = store.read_chunk(&ArtifactReadRequest::new(
        reference,
        262_144,
        262_144,
        ArtifactReadAuthority::PublicOnly,
    )?)?;
    Ok(ScenarioMeasurement::new(
        "adapters/artifact_publish_and_range_read",
        17,
        u64::try_from(bytes.len().saturating_add(read.bytes.len()))?,
        &read.bytes,
    ))
}

/// Measures receipt turnover, durable growth, projection boundedness, and reopen recovery.
pub fn measure_storage_growth(operations: u32) -> EvidenceResult<StorageEvidence> {
    if operations < HOT_BOUND.saturating_mul(2) {
        return Err(
            std::io::Error::other("storage evidence needs at least two hot turnovers").into(),
        );
    }
    let directory = tempfile::tempdir()?;
    let root = directory.path().to_path_buf();
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(&root).with_application_receipt_lifecycle(HOT_BOUND, ARCHIVE_BATCH),
    )?;
    let receipts = (0..operations)
        .map(application_receipt)
        .collect::<EvidenceResult<Vec<_>>>()?;
    let first = receipts
        .first()
        .cloned()
        .ok_or_else(|| std::io::Error::other("storage receipt fixture is empty"))?;
    for receipt in &receipts {
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: receipt.clone(),
            effect: ApplicationCommandEffect::None,
        })?;
    }
    let before = store.application_receipt_status()?;
    let bytes_before_final_archive = directory_bytes(&root)?;
    let archived = store.archive_application_command_receipts(
        milkdrift_persistence::ApplicationReceiptArchiveRequest {
            expected_generation: before.archive_generation,
            archived_at: TimestampMillis::new(u64::from(operations).saturating_add(10_000)),
        },
    )?;
    let bytes_after_final_archive = directory_bytes(&root)?;
    drop(store);

    let reopen_started = Instant::now();
    let reopened = RedbStore::open_with_config(
        RedbStoreConfig::new(&root).with_application_receipt_lifecycle(HOT_BOUND, ARCHIVE_BATCH),
    )?;
    let status = reopened.application_receipt_status()?;
    let replay = reopened.commit_application_command(&ApplicationCommandCommit {
        receipt: first,
        effect: ApplicationCommandEffect::None,
    })?;
    let reopen_microseconds = u64::try_from(reopen_started.elapsed().as_micros())?;
    let history = projection_history(10_000)?;
    let projection = RunProjection::replay(&history)?;
    let hot_count = usize::try_from(status.hot_count)?;
    let hot_start = receipts.len().saturating_sub(hot_count);
    let receipt_bytes = receipts
        .iter()
        .map(serde_json::to_vec)
        .collect::<Result<Vec<_>, _>>()?;
    let receipt_primary_logical_bytes = receipt_bytes.iter().try_fold(0_u64, |total, bytes| {
        Ok::<_, std::num::TryFromIntError>(total.saturating_add(u64::try_from(bytes.len())?))
    })?;
    let receipt_cold_logical_bytes =
        receipt_bytes[..hot_start]
            .iter()
            .try_fold(0_u64, |total, bytes| {
                Ok::<_, std::num::TryFromIntError>(
                    total.saturating_add(u64::try_from(bytes.len())?),
                )
            })?;
    let receipt_hot_logical_bytes =
        receipt_bytes[hot_start..]
            .iter()
            .try_fold(0_u64, |total, bytes| {
                Ok::<_, std::num::TryFromIntError>(
                    total.saturating_add(u64::try_from(bytes.len())?),
                )
            })?;
    let peer = peer_operational_counts(operations)?;
    Ok(StorageEvidence {
        receipt_operations: operations,
        receipt_primary_count: status.hot_count.saturating_add(status.cold_count),
        receipt_hot_count: status.hot_count,
        receipt_cold_count: status.cold_count,
        receipt_primary_logical_bytes,
        receipt_hot_logical_bytes,
        receipt_cold_logical_bytes,
        receipt_archive_generation: archived.status.archive_generation,
        bytes_before_final_archive,
        bytes_after_final_archive,
        reopen_microseconds,
        oldest_cold_replayed: matches!(replay, ApplicationCommandCommitOutcome::Replayed(_)),
        journal_events: u64::try_from(history.len())?,
        active_projection_nodes: u64::try_from(
            projection
                .node_executions()
                .len()
                .saturating_add(projection.settled_node_executions().len()),
        )?,
        peer_executions: peer.executions,
        peer_active_count: peer.final_active,
        peer_dispatch_queued_count: peer.final_dispatch_queued,
        peer_hot_count: peer.final_hot,
        peer_peak_active_count: peer.peak_active,
        peer_peak_hot_count: peer.peak_hot,
        peer_observations: peer.observations,
        peer_tombstones: peer.final_tombstones,
        peer_active_snapshot_logical_bytes: peer.active_snapshot_logical_bytes,
        peer_hot_snapshot_logical_bytes: peer.hot_snapshot_logical_bytes,
        peer_tombstone_snapshot_logical_bytes: peer.tombstone_snapshot_logical_bytes,
        peer_observation_logical_bytes: peer.observation_logical_bytes,
    })
}

/// Runs peer append/page/resume and active/hot/tombstone exact-lookup scenarios.
pub fn peer_observation_paths() -> EvidenceResult<ScenarioMeasurement> {
    let peer = peer_operational_counts(4)?;
    let encoded = serde_json::to_vec(&peer)?;
    Ok(ScenarioMeasurement::new(
        "persistence/peer_observation_page_resume_and_tombstone",
        peer.observations.saturating_add(peer.final_tombstones),
        u64::try_from(encoded.len())?,
        &encoded,
    ))
}

fn journal_request(name: &str, count: u64) -> EvidenceResult<AtomicRunCommitRequest> {
    let run = RunId::new(format!("run-evidence-journal-{name}"))?;
    let command = CommandId::new(format!("command-evidence-journal-{name}"))?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor:evidence")?,
        RunSequence::ZERO,
        TimestampMillis::new(1),
        br#"{"schema_version":1,"type":"evidence"}"#.to_vec(),
    )?;
    let mut events = Vec::new();
    for sequence in 1..=count {
        events.push(RunEventEnvelope::new(
            EventId::new(format!("event-evidence-{name}-{sequence}"))?,
            run.clone(),
            RunSequence::new(sequence),
            TimestampMillis::new(sequence),
            RunEventKind::RunStarted,
        )?);
    }
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        RunSequence::new(count),
        events
            .iter()
            .map(|event| event.event_id().clone())
            .collect(),
        BoundedJson::new(serde_json::json!({"accepted": true}))?,
    )?;
    Ok(AtomicRunCommitRequest::new(
        receipt,
        events,
        Vec::<WorkspaceMutation>::new(),
        Some(WorkspaceAccounting {
            budget: WorkspaceBudget::new(0, 0, 0, 0, 0, 0)?,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: WorkspaceUsage::EMPTY,
        }),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-evidence-journal")?,
                revision: revision_id('0')?,
                state: IndexedRunState::Active,
                through_sequence: RunSequence::new(count),
                updated_at: TimestampMillis::new(count),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?)
}

fn application_receipt(index: u32) -> EvidenceResult<ApplicationCommandReceipt> {
    let command = CommandId::new(format!("command-evidence-receipt-{index:08}"))?;
    Ok(ApplicationCommandReceipt::new(
        ActorRef::new("actor:evidence")?,
        command,
        1,
        IntegrityDigest::hash(format!("application-command-{index}").as_bytes()),
        GrantId::new("grant:evidence")?,
        1,
        GrantDigest::new(format!("b3_{}", "1".repeat(64)))?,
        Some(format!("b3_{}", "2".repeat(64))),
        TimestampMillis::new(u64::from(index).saturating_add(1)),
        TimestampMillis::new(u64::from(index).saturating_add(2)),
        ApplicationCommandResult::Accepted {
            document: serde_json::to_vec(&serde_json::json!({"index": index}))?,
            effect: None,
        },
    )?)
}

fn projection_history(event_count: u64) -> EvidenceResult<Vec<RunEventEnvelope>> {
    let event_count = event_count.max(2);
    let run = RunId::new("run-evidence-projection")?;
    let scope = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
    let mut events = vec![RunEventEnvelope::new(
        EventId::new("event-projection-created")?,
        run.clone(),
        RunSequence::FIRST,
        TimestampMillis::new(1),
        RunEventKind::RunCreated {
            workflow: WorkflowId::new("workflow-evidence-projection")?,
            revision: revision_id('3')?,
            revision_digest: blueprint_digest('4')?,
            root_scope: scope,
            workspace_budget: WorkspaceBudget::new(16, 65_536, 65_536, 16, 65_536, 65_536)?,
            inputs: Vec::new(),
        },
    )?];
    events.push(RunEventEnvelope::new(
        EventId::new("event-projection-started")?,
        run.clone(),
        RunSequence::new(2),
        TimestampMillis::new(2),
        RunEventKind::RunStarted,
    )?);
    for sequence in 3..=event_count {
        let kind = if sequence % 2 == 1 {
            RunEventKind::RunPaused {
                reason: Reason::new("operational evidence pause")?,
                evidence: Vec::new(),
            }
        } else {
            RunEventKind::RunResumed {
                reason: Reason::new("operational evidence resume")?,
                evidence: Vec::new(),
            }
        };
        events.push(RunEventEnvelope::new(
            EventId::new(format!("event-projection-{sequence}"))?,
            run.clone(),
            RunSequence::new(sequence),
            TimestampMillis::new(sequence),
            kind,
        )?);
    }
    Ok(events)
}

fn revision_id(character: char) -> EvidenceResult<RevisionId> {
    Ok(serde_json::from_value(serde_json::json!(format!(
        "rev_{}",
        character.to_string().repeat(64)
    )))?)
}

fn blueprint_digest(character: char) -> EvidenceResult<BlueprintDigest> {
    Ok(serde_json::from_value(serde_json::json!(format!(
        "b3_{}",
        character.to_string().repeat(64)
    )))?)
}

fn deterministic_bytes(size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| u8::try_from(index % 251).unwrap_or(0))
        .collect()
}

fn directory_bytes(root: &Path) -> EvidenceResult<u64> {
    let mut total = 0_u64;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        total = if metadata.is_dir() {
            total.saturating_add(directory_bytes(&entry.path())?)
        } else {
            total.saturating_add(metadata.len())
        };
    }
    Ok(total)
}

fn peer_operational_counts(executions: u32) -> EvidenceResult<crate::peer::PeerTurnoverEvidence> {
    crate::adapters::peer_storage_turnover(executions)
}
