//! One retained-history allowance covers task facts, pending writes and artifact content.

use super::{Accounting, WorkflowSnapshot, add, bounds, replace};
use crate::{
    error, json,
    schema::{TASK_ACCOUNTING, TASK_WORKFLOWS},
};
use milkdrift_blueprint::WorkflowId;
use milkdrift_persistence::PersistenceError;
use milkdrift_workspace::{ArtifactOwner, RunId};
use redb::ReadableTable;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskOutputUsage {
    limit: u64,
    bytes: u64,
    count: u64,
}

fn output_key(
    owner: &ArtifactOwner,
    metadata: &milkdrift_workspace::ArtifactMetadata,
) -> Result<String, PersistenceError> {
    let Some(run) = owner.run() else {
        return Err(bounds("task output has no workflow owner"));
    };
    let milkdrift_workspace::CausalReference::Invocation { invocation } =
        metadata.provenance().producer()
    else {
        return Err(bounds("task output has no invocation producer"));
    };
    Ok(format!("{}:{run}{invocation}", run.as_str().len()))
}

pub(crate) fn reserve_task_output(
    write: &redb::WriteTransaction,
    owner: &ArtifactOwner,
    metadata: &milkdrift_workspace::ArtifactMetadata,
    limit: Option<u64>,
) -> Result<(), PersistenceError> {
    let Some(limit) = limit else {
        return Ok(());
    };
    let key = output_key(owner, metadata)?;
    let mut table = write
        .open_table(crate::schema::TASK_OUTPUT_USAGE)
        .map_err(error::redb)?;
    let mut usage: TaskOutputUsage = table
        .get(key.as_str())
        .map_err(error::redb)?
        .map(|value| json::decode(value.value(), "task output usage"))
        .transpose()?
        .unwrap_or(TaskOutputUsage {
            limit,
            bytes: 0,
            count: 0,
        });
    if usage.limit != limit {
        return Err(bounds(
            "task output ceiling changed after first publication",
        ));
    }
    usage.bytes = add(usage.bytes, metadata.reference().size_bytes())?;
    usage.count = add(usage.count, 1)?;
    if usage.bytes > limit || usage.count > 64 {
        return Err(bounds("task cumulative output allowance exhausted"));
    }
    let bytes = json::encode(&usage, "task output usage")?;
    table
        .insert(key.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn release_task_output(
    write: &redb::WriteTransaction,
    owner: &ArtifactOwner,
    metadata: &milkdrift_workspace::ArtifactMetadata,
    limit: Option<u64>,
) -> Result<(), PersistenceError> {
    if limit.is_none() {
        return Ok(());
    }
    let key = output_key(owner, metadata)?;
    let mut table = write
        .open_table(crate::schema::TASK_OUTPUT_USAGE)
        .map_err(error::redb)?;
    let mut usage: TaskOutputUsage = table
        .get(key.as_str())
        .map_err(error::redb)?
        .map(|value| json::decode(value.value(), "task output usage"))
        .transpose()?
        .ok_or_else(|| error::corruption("task output reservation disappeared"))?;
    usage.bytes = replace(usage.bytes, metadata.reference().size_bytes(), 0)?;
    usage.count = replace(usage.count, 1, 0)?;
    let bytes = json::encode(&usage, "task output usage")?;
    table
        .insert(key.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

pub(super) fn workflow_output_bytes(
    write: &redb::WriteTransaction,
    workflow: &WorkflowId,
) -> Result<u64, PersistenceError> {
    let run = RunId::new(workflow.as_str())
        .map_err(|error| PersistenceError::InvalidDocument(error.to_string()))?;
    let owner = ArtifactOwner::from(&run);
    add(
        committed_owner_bytes(write, &owner)?,
        crate::artifact::pending_owner_bytes(write, &owner)?,
    )
}

fn committed_owner_bytes(
    write: &redb::WriteTransaction,
    owner: &ArtifactOwner,
) -> Result<u64, PersistenceError> {
    Ok(
        crate::artifact::domain::workspace_domain_in_transaction(write, owner)?
            .map_or(0, |(_, usage)| usage.artifact_bytes()),
    )
}

fn owner_capacity(
    write: &redb::WriteTransaction,
    owner: &ArtifactOwner,
) -> Result<u64, PersistenceError> {
    let Some(run) = owner.run() else {
        return Ok(0);
    };
    let table = write.open_table(TASK_WORKFLOWS).map_err(error::redb)?;
    table
        .get(run.as_str())
        .map_err(error::redb)?
        .map(|value| {
            json::decode::<WorkflowSnapshot>(value.value(), "task workflow")
                .map(|snapshot| snapshot.artifact_capacity)
        })
        .transpose()
        .map(|capacity| capacity.unwrap_or(0))
}

pub(super) fn require_history_capacity(
    write: &redb::WriteTransaction,
    accounting: &Accounting,
    limit: u64,
) -> Result<(), PersistenceError> {
    let physical = crate::artifact::validate_artifact_state(write)?.committed_content_bytes;
    let used = [
        accounting.history_bytes,
        accounting.authoring_replacement_bytes,
        accounting.reserved_bytes,
        accounting.artifact_reserved_bytes,
        accounting.pending_artifact_bytes,
        physical,
    ]
    .into_iter()
    .try_fold(0, add)?;
    if used > limit {
        return Err(bounds(
            "retained history, artifacts and accepted work reservations fill the storage allowance",
        ));
    }
    Ok(())
}

fn update(
    write: &redb::WriteTransaction,
    change: impl FnOnce(&mut Accounting) -> Result<(), PersistenceError>,
    limit: Option<u64>,
) -> Result<(), PersistenceError> {
    let mut table = write.open_table(TASK_ACCOUNTING).map_err(error::redb)?;
    let mut accounting: Accounting = table
        .get("global")
        .map_err(error::redb)?
        .map(|value| json::decode(value.value(), "task accounting"))
        .transpose()?
        .ok_or_else(|| error::corruption("task accounting is missing"))?;
    change(&mut accounting)?;
    if let Some(limit) = limit {
        require_history_capacity(write, &accounting, limit)?;
    }
    let bytes = json::encode(&accounting, "task accounting")?;
    table
        .insert("global", bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

/// Charge pending bytes before creating a temporary file; output reservations cover their owner.
pub(crate) fn begin_artifact(
    write: &redb::WriteTransaction,
    owner: &ArtifactOwner,
    bytes: u64,
    limit: u64,
) -> Result<(), PersistenceError> {
    let remaining =
        owner_capacity(write, owner)?.saturating_sub(committed_owner_bytes(write, owner)?);
    update(
        write,
        |accounting| {
            accounting.pending_artifact_bytes = add(accounting.pending_artifact_bytes, bytes)?;
            accounting.artifact_reserved_bytes =
                replace(accounting.artifact_reserved_bytes, remaining.min(bytes), 0)?;
            Ok(())
        },
        Some(limit),
    )
}

/// The existing artifact accounting now owns committed bytes; never charge the same write twice.
pub(crate) fn commit_artifact(
    write: &redb::WriteTransaction,
    bytes: u64,
) -> Result<(), PersistenceError> {
    update(
        write,
        |accounting| {
            accounting.pending_artifact_bytes =
                replace(accounting.pending_artifact_bytes, bytes, 0)?;
            Ok(())
        },
        None,
    )
}

/// Aborting releases pending bytes and restores any still-promised output reservation.
pub(crate) fn release_artifact(
    write: &redb::WriteTransaction,
    owner: &ArtifactOwner,
    bytes: u64,
) -> Result<(), PersistenceError> {
    let restored = owner_capacity(write, owner)?
        .saturating_sub(committed_owner_bytes(write, owner)?)
        .min(bytes);
    update(
        write,
        |accounting| {
            accounting.pending_artifact_bytes =
                replace(accounting.pending_artifact_bytes, bytes, 0)?;
            accounting.artifact_reserved_bytes = add(accounting.artifact_reserved_bytes, restored)?;
            Ok(())
        },
        None,
    )
}

pub(crate) fn validate_output_usage(bytes: &[u8]) -> Result<(), PersistenceError> {
    let usage: TaskOutputUsage = json::decode(bytes, "task output usage")?;
    if usage.limit == 0 || usage.bytes > usage.limit || usage.count > 64 {
        return Err(error::corruption(
            "task output usage exceeds frozen allowance",
        ));
    }
    Ok(())
}
