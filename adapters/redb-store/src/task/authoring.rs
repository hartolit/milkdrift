//! Conditional latest-content replacement shares task capacity without consuming command receipts.
#![deny(clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;

use milkdrift_blueprint::WorkflowId;
use milkdrift_persistence::{
    MAX_WORKFLOW_AUTHORING_BYTES, MAX_WORKFLOW_POSITIONS, MAX_WORKFLOW_PROTECTED_SOURCES,
    PersistenceError, WorkflowAuthoringCommit, WorkflowAuthoringSnapshot, WorkflowPositionsCommit,
    WorkflowPositionsSnapshot,
};
use milkdrift_workspace::ArtifactMetadata;
use redb::ReadableTable;

use super::{Accounting, bounds, capacity, replace};
use crate::{
    RedbStore, error, json,
    schema::{
        ARTIFACT_MANIFEST, ARTIFACT_METADATA, TASK_ACCOUNTING, TASK_AUTHORING, TASK_POSITIONS,
        TASK_WORKFLOWS,
    },
};

fn digest(value: &impl serde::Serialize) -> Result<String, PersistenceError> {
    Ok(format!("b3_{}", blake3::hash(&serde_json::to_vec(value)?)))
}

fn empty_authoring(workflow: &WorkflowId) -> Result<WorkflowAuthoringSnapshot, PersistenceError> {
    Ok(WorkflowAuthoringSnapshot {
        workflow: workflow.clone(),
        generation: 0,
        digest: digest(&serde_json::Value::Null)?,
        document: serde_json::Value::Null,
        protected_sources: Vec::new(),
    })
}

fn empty_positions(workflow: &WorkflowId) -> Result<WorkflowPositionsSnapshot, PersistenceError> {
    let positions = BTreeMap::new();
    Ok(WorkflowPositionsSnapshot {
        workflow: workflow.clone(),
        generation: 0,
        digest: digest(&positions)?,
        positions,
    })
}

pub(super) fn read_authoring(
    store: &RedbStore,
    workflow: &WorkflowId,
) -> Result<Option<WorkflowAuthoringSnapshot>, PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    if read
        .open_table(TASK_WORKFLOWS)
        .map_err(error::redb)?
        .get(workflow.as_str())
        .map_err(error::redb)?
        .is_none()
    {
        return Ok(None);
    }
    let table = read.open_table(TASK_AUTHORING).map_err(error::redb)?;
    let snapshot = match table.get(workflow.as_str()).map_err(error::redb)? {
        Some(bytes) => {
            validate_authoring_row(workflow.as_str(), bytes.value())?;
            json::decode(bytes.value(), "workflow authoring")?
        }
        None => empty_authoring(workflow)?,
    };
    Ok(Some(snapshot))
}

pub(super) fn read_positions(
    store: &RedbStore,
    workflow: &WorkflowId,
) -> Result<Option<WorkflowPositionsSnapshot>, PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    if read
        .open_table(TASK_WORKFLOWS)
        .map_err(error::redb)?
        .get(workflow.as_str())
        .map_err(error::redb)?
        .is_none()
    {
        return Ok(None);
    }
    let table = read.open_table(TASK_POSITIONS).map_err(error::redb)?;
    let snapshot = match table.get(workflow.as_str()).map_err(error::redb)? {
        Some(bytes) => {
            validate_positions_row(workflow.as_str(), bytes.value())?;
            json::decode(bytes.value(), "workflow positions")?
        }
        None => empty_positions(workflow)?,
    };
    Ok(Some(snapshot))
}

fn require_workflow(
    write: &redb::WriteTransaction,
    workflow: &WorkflowId,
) -> Result<(), PersistenceError> {
    if write
        .open_table(TASK_WORKFLOWS)
        .map_err(error::redb)?
        .get(workflow.as_str())
        .map_err(error::redb)?
        .is_none()
    {
        return Err(PersistenceError::NotFound {
            entity: "workflow",
            identity: workflow.to_string(),
        });
    }
    Ok(())
}

fn accounting(write: &redb::WriteTransaction) -> Result<Accounting, PersistenceError> {
    write
        .open_table(TASK_ACCOUNTING)
        .map_err(error::redb)?
        .get("global")
        .map_err(error::redb)?
        .map(|bytes| json::decode(bytes.value(), "task accounting"))
        .transpose()?
        .ok_or_else(|| error::corruption("task accounting is missing"))
}

fn finish_checkpoint(
    store: &RedbStore,
    write: redb::WriteTransaction,
    accounting: &Accounting,
) -> Result<(), PersistenceError> {
    capacity::require_history_capacity(&write, accounting, store.workflow_limits.history_bytes)?;
    let bytes = json::encode(accounting, "task accounting")?;
    write
        .open_table(TASK_ACCOUNTING)
        .map_err(error::redb)?
        .insert("global", bytes.as_slice())
        .map_err(error::redb)?;
    store
        .faults
        .check(crate::fault::FaultPoint::BeforeAuthoringCommit)?;
    write.commit().map_err(error::redb)?;
    store
        .faults
        .check(crate::fault::FaultPoint::AfterAuthoringCommit)
}

pub(super) fn checkpoint_authoring(
    store: &RedbStore,
    workflow: &WorkflowId,
    checkpoint: WorkflowAuthoringCommit,
) -> Result<WorkflowAuthoringSnapshot, PersistenceError> {
    let write = store.database().begin_write().map_err(error::redb)?;
    require_workflow(&write, workflow)?;
    crate::clock::require_clock_in_transaction(&write, store.clock.now()?)?;
    let mut accounting = accounting(&write)?;
    let snapshot = replace_authoring(&write, workflow, checkpoint, &mut accounting)?;
    finish_checkpoint(store, write, &accounting)?;
    Ok(snapshot)
}

pub(super) fn replace_authoring(
    write: &redb::WriteTransaction,
    workflow: &WorkflowId,
    checkpoint: WorkflowAuthoringCommit,
    accounting: &mut Accounting,
) -> Result<WorkflowAuthoringSnapshot, PersistenceError> {
    validate_document(&checkpoint.document, &checkpoint.protected_sources)?;
    let mut table = write.open_table(TASK_AUTHORING).map_err(error::redb)?;
    let old_bytes = table
        .get(workflow.as_str())
        .map_err(error::redb)?
        .map(|bytes| bytes.value().to_vec());
    let old = match old_bytes.as_deref() {
        Some(bytes) => {
            validate_authoring_row(workflow.as_str(), bytes)?;
            json::decode(bytes, "workflow authoring")?
        }
        None => empty_authoring(workflow)?,
    };
    if checkpoint.expected_generation != old.generation {
        return Err(PersistenceError::AuthoringGenerationConflict {
            workflow: workflow.clone(),
            expected: checkpoint.expected_generation,
            actual: old.generation,
        });
    }
    // Protection belongs to the latest durable content. Existing committed metadata/ownership
    // already protects these bytes from orphan cleanup; removal here releases only this draft's
    // reason to retain them, never a producer, accepted task or exact command's ownership.
    for source in &checkpoint.protected_sources {
        let metadata = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
        let manifest = write.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
        let bytes = metadata
            .get(source.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| PersistenceError::NotFound {
                entity: "authoring source artifact",
                identity: source.clone(),
            })?;
        let value: ArtifactMetadata = json::decode(bytes.value(), "artifact metadata")?;
        let retained = manifest
            .get(source.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authoring source has no authoritative manifest"))?;
        let retained: ArtifactMetadata = json::decode(retained.value(), "artifact manifest")?;
        if value.reference().artifact().as_str() != source || value != retained {
            return Err(error::corruption(
                "authoring source differs from authoritative artifact metadata",
            ));
        }
    }
    let snapshot = WorkflowAuthoringSnapshot {
        workflow: workflow.clone(),
        generation: old
            .generation
            .checked_add(1)
            .ok_or_else(|| bounds("authoring generation exhausted"))?,
        digest: digest(&checkpoint.document)?,
        document: checkpoint.document,
        protected_sources: checkpoint.protected_sources,
    };
    let bytes = json::encode(&snapshot, "workflow authoring")?;
    charge_replacement(
        accounting,
        old_bytes.as_ref().map_or(0, Vec::len),
        bytes.len(),
    )?;
    table
        .insert(workflow.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(snapshot)
}

pub(super) fn checkpoint_positions(
    store: &RedbStore,
    workflow: &WorkflowId,
    checkpoint: WorkflowPositionsCommit,
) -> Result<WorkflowPositionsSnapshot, PersistenceError> {
    validate_positions(&checkpoint.positions)?;
    let write = store.database().begin_write().map_err(error::redb)?;
    require_workflow(&write, workflow)?;
    crate::clock::require_clock_in_transaction(&write, store.clock.now()?)?;
    let mut accounting = accounting(&write)?;
    let mut table = write.open_table(TASK_POSITIONS).map_err(error::redb)?;
    let old_bytes = table
        .get(workflow.as_str())
        .map_err(error::redb)?
        .map(|bytes| bytes.value().to_vec());
    let old = match old_bytes.as_deref() {
        Some(bytes) => {
            validate_positions_row(workflow.as_str(), bytes)?;
            json::decode(bytes, "workflow positions")?
        }
        None => empty_positions(workflow)?,
    };
    if checkpoint.expected_generation != old.generation {
        return Err(PersistenceError::PositionsGenerationConflict {
            workflow: workflow.clone(),
            expected: checkpoint.expected_generation,
            actual: old.generation,
        });
    }
    let snapshot = WorkflowPositionsSnapshot {
        workflow: workflow.clone(),
        generation: old
            .generation
            .checked_add(1)
            .ok_or_else(|| bounds("position generation exhausted"))?,
        digest: digest(&checkpoint.positions)?,
        positions: checkpoint.positions,
    };
    let bytes = json::encode(&snapshot, "workflow positions")?;
    charge_replacement(
        &mut accounting,
        old_bytes.as_ref().map_or(0, Vec::len),
        bytes.len(),
    )?;
    table
        .insert(workflow.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    drop(table);
    finish_checkpoint(store, write, &accounting)?;
    Ok(snapshot)
}

fn charge_replacement(
    accounting: &mut Accounting,
    old: usize,
    new: usize,
) -> Result<(), PersistenceError> {
    let old = u64::try_from(old).map_err(|_| bounds("authoring byte size overflow"))?;
    let new = u64::try_from(new).map_err(|_| bounds("authoring byte size overflow"))?;
    accounting.history_bytes = replace(accounting.history_bytes, old, new)?;
    accounting.authoring_bytes = replace(accounting.authoring_bytes, old, new)?;
    // Retaining one replacement's worth beside the latest bytes covers the transaction's old
    // and new authoring content. It cannot spend reserves promised to accepted execution.
    accounting.authoring_replacement_bytes =
        replace(accounting.authoring_replacement_bytes, old, new)?;
    Ok(())
}

fn validate_document(
    document: &serde_json::Value,
    sources: &[String],
) -> Result<(), PersistenceError> {
    if serde_json::to_vec(document)?.len() > MAX_WORKFLOW_AUTHORING_BYTES {
        return Err(bounds("saved authoring exceeds its 2 MiB document bound"));
    }
    if sources.len() > MAX_WORKFLOW_PROTECTED_SOURCES {
        return Err(bounds(&format!(
            "Saved canvas protects at most {MAX_WORKFLOW_PROTECTED_SOURCES} retained files and results. Remove unused sources or result connections before saving."
        )));
    }
    if sources
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != sources.len()
        || sources.iter().any(|source| {
            source.is_empty() || source.len() > 1024 || source.chars().any(char::is_control)
        })
    {
        return Err(bounds(
            "saved authoring has invalid or duplicate protected sources",
        ));
    }
    Ok(())
}

fn validate_positions(
    positions: &BTreeMap<String, milkdrift_persistence::WorkflowPosition>,
) -> Result<(), PersistenceError> {
    if positions.len() > MAX_WORKFLOW_POSITIONS
        || positions.iter().any(|(id, position)| {
            id.is_empty()
                || id.len() > 1024
                || id.chars().any(char::is_control)
                || !position.x.is_finite()
                || !position.y.is_finite()
        })
    {
        return Err(bounds(
            "saved positions exceed their bound or have invalid coordinates",
        ));
    }
    Ok(())
}

pub(crate) fn validate_authoring_row(key: &str, bytes: &[u8]) -> Result<(), PersistenceError> {
    let snapshot: WorkflowAuthoringSnapshot = json::decode(bytes, "workflow authoring")?;
    if snapshot.workflow.as_str() != key
        || snapshot.generation == 0
        || snapshot.digest != digest(&snapshot.document)?
    {
        return Err(error::corruption(
            "authoring identity, generation or content digest differs",
        ));
    }
    validate_document(&snapshot.document, &snapshot.protected_sources)
}

pub(crate) fn validate_positions_row(key: &str, bytes: &[u8]) -> Result<(), PersistenceError> {
    let snapshot: WorkflowPositionsSnapshot = json::decode(bytes, "workflow positions")?;
    if snapshot.workflow.as_str() != key
        || snapshot.generation == 0
        || snapshot.digest != digest(&snapshot.positions)?
    {
        return Err(error::corruption(
            "position identity, generation or content digest differs",
        ));
    }
    validate_positions(&snapshot.positions)
}
