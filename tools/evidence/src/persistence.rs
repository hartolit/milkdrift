//! Measure atomic task snapshots, exact receipts, artifact I/O and independent peer retention.
//! Logical bytes and filesystem allocation answer different questions. Reopen checks retain
//! software facts; these measurements make no filesystem power-loss claim.
use crate::{EvidenceResult, ScenarioMeasurement};
use milkdrift_blueprint::WorkflowId;
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactReadAuthority, ArtifactReadRequest, ArtifactStore,
    PersistenceError, TimestampMillis, WorkflowCommandAcceptance, WorkflowCommit,
    WorkflowCommitOutcome, WorkflowSnapshot, WorkflowStore,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType, RunId,
    WorkspaceBudget, WorkspaceUsage,
};
use serde::Serialize;
use std::{fs, path::Path, time::Instant};

/// Machine-readable storage, receipt and peer retention measurements.
#[derive(Clone, Debug, Serialize)]
pub struct StorageEvidence {
    /// Unique task command receipts admitted.
    pub receipt_operations: u32,
    /// Complete immutable receipt documents.
    pub receipt_primary_count: u64,
    /// Canonical JSON bytes for every primary receipt document.
    pub receipt_primary_logical_bytes: u64,
    /// Filesystem allocation after the initial store creation.
    pub bytes_before_commands: u64,
    /// Filesystem allocation after all accepted commands.
    pub bytes_after_commands: u64,
    /// Store reopen and accounting recovery time.
    pub reopen_microseconds: u64,
    /// Exact replay of the first accepted receipt succeeded after reopen.
    pub oldest_receipt_replayed: bool,
    /// A changed canonical request with the retained key was refused after reopen.
    pub conflicting_receipt_refused: bool,
    /// Atomic task-workflow snapshots advanced by the storage measurement.
    pub workflow_transitions: u64,
    /// Exact task-workflow documents recovered through the stable list port.
    pub workflow_documents: u64,
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

fn workflow_snapshot(name: &str, generation: u64, items: u32) -> EvidenceResult<WorkflowSnapshot> {
    Ok(WorkflowSnapshot {
        workflow: WorkflowId::new(format!("workflow-evidence-{name}"))?,
        generation,
        title: "Task storage measurement".into(),
        description: "Bounded synthetic document through the ordinary opaque storage port".into(),
        owner_actor: "actor:evidence".into(),
        updated_at: TimestampMillis::new(generation),
        document: serde_json::json!({"generation":generation,"facts":(0..items).map(|index| serde_json::json!({"index":index,"title":"Task evidence"})).collect::<Vec<_>>()}),
        entered_invocations: Vec::new(),
        managed_executions: Vec::new(),
        artifact_capacity: 0,
        reserved_bytes: 65_536,
        reserved_receipts: 0,
    })
}

fn commit_measurement(items: u32) -> EvidenceResult<ScenarioMeasurement> {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    let snapshot = workflow_snapshot("commit", 1, items)?;
    store.commit_workflow(WorkflowCommit {
        authoring: None,
        project_guard: None,
        project_id: None,
        expected_generation: None,
        snapshot: snapshot.clone(),
        command: None,
    })?;
    let retained = store
        .workflow(&snapshot.workflow)?
        .ok_or("snapshot disappeared")?;
    if retained != snapshot {
        return Err("stored task document differs from accepted bytes".into());
    }
    let bytes = serde_json::to_vec(&retained)?;
    Ok(ScenarioMeasurement::new(
        if items == 1 {
            "persistence/task_commit_one"
        } else {
            "persistence/task_commit_batch_32"
        },
        u64::from(items),
        u64::try_from(bytes.len())?,
        &bytes,
    ))
}

/// Commit and read one workflow document through production redb.
/// # Errors
/// Returns storage admission, read or encoding failures.
pub fn workflow_commit_one() -> EvidenceResult<ScenarioMeasurement> {
    commit_measurement(1)
}
/// Commit one bounded 32-item initial task document atomically.
/// # Errors
/// Returns storage admission, read or encoding failures.
pub fn workflow_commit_batch() -> EvidenceResult<ScenarioMeasurement> {
    commit_measurement(32)
}
/// Discover 64 retained workflows through stable bounded pagination.
/// # Errors
/// Returns storage or page ordering failures.
pub fn workflow_read_page() -> EvidenceResult<ScenarioMeasurement> {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    for index in 0..64 {
        store.commit_workflow(WorkflowCommit {
            authoring: None,
            project_guard: None,
            project_id: None,
            expected_generation: None,
            snapshot: workflow_snapshot(&format!("{index:04}"), 1, 1)?,
            command: None,
        })?;
    }
    let mut cursor = None;
    let mut identities = Vec::new();
    loop {
        let page = store.workflows(cursor.as_deref(), 16)?;
        identities.extend(page.items.into_iter().map(|item| item.workflow));
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    if identities.len() != 64
        || identities
            .iter()
            .zip(identities.iter().skip(1))
            .any(|(prior, next)| prior >= next)
    {
        return Err("workflow page coverage/order differs".into());
    }
    let bytes = serde_json::to_vec(&identities)?;
    Ok(ScenarioMeasurement::new(
        "persistence/task_discovery_64",
        64,
        u64::try_from(bytes.len())?,
        &bytes,
    ))
}
/// Reopen the exact snapshot after 128 generation-checked updates.
/// # Errors
/// Returns CAS, open, read or canonical-byte mismatches.
pub fn workflow_snapshot_reopen() -> EvidenceResult<ScenarioMeasurement> {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    let mut snapshot = workflow_snapshot("reopen", 1, 32)?;
    store.commit_workflow(WorkflowCommit {
        authoring: None,
        project_guard: None,
        project_id: None,
        expected_generation: None,
        snapshot: snapshot.clone(),
        command: None,
    })?;
    for generation in 2..=128 {
        snapshot.generation = generation;
        *snapshot
            .document
            .get_mut("generation")
            .ok_or("workflow snapshot generation absent")? = serde_json::json!(generation);
        store.commit_workflow(WorkflowCommit {
            authoring: None,
            project_guard: None,
            project_id: None,
            expected_generation: Some(generation.saturating_sub(1)),
            snapshot: snapshot.clone(),
            command: None,
        })?;
    }
    drop(store);
    let store = RedbStore::open(directory.path())?;
    if store.workflow(&snapshot.workflow)? != Some(snapshot.clone()) {
        return Err("reopened task snapshot changed".into());
    }
    let bytes = serde_json::to_vec(&snapshot)?;
    Ok(ScenarioMeasurement::new(
        "persistence/task_snapshot_reopen_128",
        128,
        u64::try_from(bytes.len())?,
        &bytes,
    ))
}

/// Exercise exact task receipts, changed-request conflicts, and replay after reopen.
/// # Errors
/// Returns command admission, lookup, reopen, or canonical-result failures.
pub fn workflow_receipt_paths() -> EvidenceResult<ScenarioMeasurement> {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    for index in 0..32 {
        let outcome = store.commit_workflow(receipt_commit(index)?)?;
        if !matches!(outcome, WorkflowCommitOutcome::Applied(Some(_))) {
            return Err("fresh task receipt unexpectedly replayed".into());
        }
    }
    drop(store);
    let store = RedbStore::open(directory.path())?;
    let first = receipt_commit(0)?;
    let recovered = store
        .workflow_receipt("actor:evidence", "command-evidence-00000000")?
        .ok_or("retained task receipt absent")?;
    let replay = store.commit_workflow(first.clone())?;
    if replay != WorkflowCommitOutcome::Replay(recovered.clone()) {
        return Err("task receipt exact replay changed".into());
    }
    let mut conflict = first;
    conflict
        .command
        .as_mut()
        .ok_or("command absent")?
        .canonical_request = serde_json::to_vec(&serde_json::json!({"index":99}))?;
    if !matches!(
        store.commit_workflow(conflict),
        Err(PersistenceError::WorkflowIdempotencyConflict { .. })
    ) {
        return Err("changed task command reused retained key".into());
    }
    let encoded = serde_json::to_vec(&recovered)?;
    Ok(ScenarioMeasurement::new(
        "persistence/task_receipt_replay_conflict_reopen",
        35,
        u64::try_from(encoded.len())?,
        &encoded,
    ))
}

/// Publishes and range-reads one representative content-addressed artifact.
///
/// # Errors
/// Store creation, metadata/budget validation, staged publication, commit, or authorized range
/// reading can fail; incomplete work is retained only in the temporary evidence store.
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

/// Measures exact task receipt growth, bounded snapshot recovery, and independent peer retention.
/// # Errors
/// Refuses an empty scenario and returns storage, replay, conflict, or accounting failures.
pub fn measure_storage_growth(operations: u32) -> EvidenceResult<StorageEvidence> {
    if operations == 0 {
        return Err("storage evidence requires at least one command".into());
    }
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let store = RedbStore::open(root)?;
    let bytes_before_commands = directory_bytes(root)?;
    let mut receipt_primary_logical_bytes = 0_u64;
    for index in 0..operations {
        let outcome = store.commit_workflow(receipt_commit(index)?)?;
        let WorkflowCommitOutcome::Applied(Some(receipt)) = outcome else {
            return Err("fresh task command did not create exact receipt".into());
        };
        receipt_primary_logical_bytes = receipt_primary_logical_bytes
            .checked_add(u64::try_from(serde_json::to_vec(&receipt)?.len())?)
            .ok_or("receipt measurement overflow")?;
    }
    let bytes_after_commands = directory_bytes(root)?;
    drop(store);
    let reopen_started = Instant::now();
    let reopened = RedbStore::open(root)?;
    let replay = reopened.commit_workflow(receipt_commit(0)?)?;
    let oldest_receipt_replayed = matches!(replay, WorkflowCommitOutcome::Replay(_));
    let mut changed = receipt_commit(0)?;
    changed
        .command
        .as_mut()
        .ok_or("command absent")?
        .canonical_request = b"{\"changed\":true}".to_vec();
    let conflicting_receipt_refused = matches!(
        reopened.commit_workflow(changed),
        Err(PersistenceError::WorkflowIdempotencyConflict { .. })
    );
    let reopen_microseconds = u64::try_from(reopen_started.elapsed().as_micros())?;
    let mut cursor = None;
    let mut workflow_documents = 0_u64;
    loop {
        let page = reopened.workflows(cursor.as_deref(), 200)?;
        workflow_documents = workflow_documents
            .checked_add(u64::try_from(page.items.len())?)
            .ok_or("workflow count overflow")?;
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    let usage = reopened.workflow_storage_usage()?;
    let peer = peer_operational_counts(operations)?;
    Ok(StorageEvidence {
        receipt_operations: operations,
        receipt_primary_count: usage.receipt_entries,
        receipt_primary_logical_bytes,
        bytes_before_commands,
        bytes_after_commands,
        reopen_microseconds,
        oldest_receipt_replayed,
        conflicting_receipt_refused,
        workflow_transitions: u64::from(operations),
        workflow_documents,
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
///
/// # Errors
/// Returns serving-store admission, observation, paging, archival/replay, or report-encoding
/// failure from the controlled peer lifecycle.
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

fn receipt_commit(index: u32) -> EvidenceResult<WorkflowCommit> {
    Ok(WorkflowCommit {
        authoring: None,
        project_guard: None,
        project_id: None,
        expected_generation: None,
        snapshot: workflow_snapshot(&format!("receipt-{index:08}"), 1, 1)?,
        command: Some(WorkflowCommandAcceptance {
            actor: "actor:evidence".into(),
            key: format!("command-evidence-{index:08}"),
            reserved_action: None,
            canonical_request: serde_json::to_vec(&serde_json::json!({"index":index}))?,
            result: serde_json::json!({"accepted":index}),
        }),
    })
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
