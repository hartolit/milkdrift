//! Atomic runtime snapshots, exact receipts and capacity protected for accepted work.

use milkdrift_blueprint::WorkflowId;
use milkdrift_persistence::{
    MAX_WORKFLOW_DOCUMENT_BYTES, MAX_WORKFLOW_REQUEST_BYTES, PersistenceError,
    WorkflowCommandReceipt, WorkflowCommit, WorkflowCommitOutcome, WorkflowPage, WorkflowSnapshot,
    WorkflowStore,
};
use redb::{ReadableTable, ReadableTableMetadata};
use serde::{Deserialize, Serialize};

use crate::{
    RedbStore, error, json,
    schema::{TASK_ACCOUNTING, TASK_RECEIPTS, TASK_WORKFLOWS},
};

/// A control action's reserved receipt cannot consume an unbounded request/result body.
const ACTION_RECEIPT_BYTES: u64 = 8192;
const MAX_RECEIPT_RESULT_BYTES: usize = 64 * 1024;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Accounting {
    history_bytes: u64,
    receipt_bytes: u64,
    receipt_entries: u64,
    reserved_bytes: u64,
    reserved_receipts: u64,
}

impl WorkflowStore for RedbStore {
    fn workflow(
        &self,
        workflow: &WorkflowId,
    ) -> Result<Option<WorkflowSnapshot>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(TASK_WORKFLOWS).map_err(error::redb)?;
        table
            .get(workflow.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "task workflow"))
            .transpose()
    }

    fn workflows(&self, after: Option<&str>, limit: u16) -> Result<WorkflowPage, PersistenceError> {
        if limit == 0 || limit > 200 {
            return Err(bounds("workflow page must contain 1..=200 records"));
        }
        if let Some(cursor) = after {
            identity(cursor)?;
        }
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(TASK_WORKFLOWS).map_err(error::redb)?;
        let mut items = Vec::new();
        let mut next = None;
        let start = after.unwrap_or("");
        for row in table.range(start..).map_err(error::redb)? {
            let (key, value) = row.map_err(error::redb)?;
            if after.is_some_and(|after| key.value() <= after) {
                continue;
            }
            if items.len() == usize::from(limit) {
                next = items
                    .last()
                    .map(|item: &WorkflowSnapshot| item.workflow.to_string());
                break;
            }
            items.push(json::decode(value.value(), "task workflow")?);
        }
        Ok(WorkflowPage { items, next })
    }

    fn workflow_receipt(
        &self,
        actor: &str,
        key: &str,
    ) -> Result<Option<WorkflowCommandReceipt>, PersistenceError> {
        let key = receipt_key(actor, key)?;
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(TASK_RECEIPTS).map_err(error::redb)?;
        table
            .get(key.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "task receipt"))
            .transpose()
    }

    fn commit_workflow(
        &self,
        commit: WorkflowCommit,
    ) -> Result<WorkflowCommitOutcome, PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        let receipt_key = commit
            .command
            .as_ref()
            .map(|command| receipt_key(&command.actor, &command.key))
            .transpose()?;
        let mut receipts = write.open_table(TASK_RECEIPTS).map_err(error::redb)?;
        if let Some(key) = &receipt_key {
            if let Some(bytes) = receipts.get(key.as_str()).map_err(error::redb)? {
                let receipt: WorkflowCommandReceipt = json::decode(bytes.value(), "task receipt")?;
                let request = commit
                    .command
                    .as_ref()
                    .ok_or_else(|| error::corruption("receipt key has no command"))?;
                if receipt.acceptance.canonical_request != request.canonical_request {
                    return Err(PersistenceError::WorkflowIdempotencyConflict {
                        actor: request.actor.clone(),
                        key: request.key.clone(),
                    });
                }
                return Ok(WorkflowCommitOutcome::Replay(receipt));
            }
        }
        validate(&commit)?;
        crate::clock::require_clock_in_transaction(&write, self.clock.now()?)?;
        let mut workflows = write.open_table(TASK_WORKFLOWS).map_err(error::redb)?;
        let old_bytes = workflows
            .get(commit.snapshot.workflow.as_str())
            .map_err(error::redb)?
            .map(|value| value.value().to_vec());
        let old: Option<WorkflowSnapshot> = old_bytes
            .as_deref()
            .map(|bytes| json::decode(bytes, "task workflow"))
            .transpose()?;
        let actual = old.as_ref().map(|snapshot| snapshot.generation);
        if actual != commit.expected_generation {
            return Err(PersistenceError::WorkflowGenerationConflict {
                workflow: commit.snapshot.workflow.clone(),
                expected: commit.expected_generation,
                actual,
            });
        }
        if old
            .as_ref()
            .is_some_and(|old| old.owner_actor != commit.snapshot.owner_actor)
        {
            return Err(PersistenceError::InvalidDocument(
                "workflow owner is immutable".to_owned(),
            ));
        }
        if old.as_ref().is_some_and(|old| {
            old.entered_invocations
                .iter()
                .any(|invocation| !commit.snapshot.entered_invocations.contains(invocation))
        }) {
            return Err(PersistenceError::InvalidDocument(
                "workflow entered invocation evidence cannot be removed".to_owned(),
            ));
        }
        let bytes = json::encode(&commit.snapshot, "task workflow")?;
        let receipt = commit.command.map(|acceptance| WorkflowCommandReceipt {
            workflow: commit.snapshot.workflow.clone(),
            generation: commit.snapshot.generation,
            acceptance,
        });
        let receipt_bytes = receipt
            .as_ref()
            .map(|receipt| json::encode(receipt, "task receipt"))
            .transpose()?;
        let mut accounting_table = write.open_table(TASK_ACCOUNTING).map_err(error::redb)?;
        let mut accounting: Accounting = accounting_table
            .get("global")
            .map_err(error::redb)?
            .map(|value| json::decode(value.value(), "task accounting"))
            .transpose()?
            .unwrap_or_default();
        if accounting.receipt_entries != receipts.len().map_err(error::redb)? {
            return Err(error::corruption(
                "task receipt count differs from accounting",
            ));
        }
        accounting.history_bytes = replace(
            accounting.history_bytes,
            old_bytes.as_ref().map_or(0, |bytes| bytes.len() as u64),
            bytes.len() as u64,
        )?;
        accounting.reserved_bytes = replace(
            accounting.reserved_bytes,
            old.as_ref().map_or(0, |old| old.reserved_bytes),
            commit.snapshot.reserved_bytes,
        )?;
        accounting.reserved_receipts = replace(
            accounting.reserved_receipts,
            old.as_ref()
                .map_or(0, |old| u64::from(old.reserved_receipts)),
            u64::from(commit.snapshot.reserved_receipts),
        )?;
        accounting.receipt_bytes = add(
            accounting.receipt_bytes,
            receipt_bytes.as_ref().map_or(0, |bytes| bytes.len() as u64),
        )?;
        accounting.receipt_entries = add(accounting.receipt_entries, u64::from(receipt.is_some()))?;
        let reserved_receipt_bytes = accounting
            .reserved_receipts
            .checked_mul(ACTION_RECEIPT_BYTES)
            .ok_or_else(|| bounds("reserved action capacity overflow"))?;
        if add(accounting.history_bytes, accounting.reserved_bytes)?
            > self.workflow_limits.history_bytes
            || add(accounting.receipt_bytes, reserved_receipt_bytes)?
                > self.workflow_limits.receipt_bytes
            || add(accounting.receipt_entries, accounting.reserved_receipts)?
                > u64::from(self.workflow_limits.receipt_entries)
        {
            return Err(bounds(
                "task storage is full; accepted work and its settlement/control reserve remain protected",
            ));
        }
        workflows
            .insert(commit.snapshot.workflow.as_str(), bytes.as_slice())
            .map_err(error::redb)?;
        if let (Some(key), Some(bytes)) = (receipt_key, receipt_bytes) {
            receipts
                .insert(key.as_str(), bytes.as_slice())
                .map_err(error::redb)?;
        }
        let accounting_bytes = json::encode(&accounting, "task accounting")?;
        accounting_table
            .insert("global", accounting_bytes.as_slice())
            .map_err(error::redb)?;
        drop(accounting_table);
        drop(workflows);
        drop(receipts);
        self.faults
            .check(crate::fault::FaultPoint::BeforeCommandCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults
            .check(crate::fault::FaultPoint::AfterCommandCommit)?;
        Ok(WorkflowCommitOutcome::Applied(receipt))
    }
}

fn validate(commit: &WorkflowCommit) -> Result<(), PersistenceError> {
    let invocations = &commit.snapshot.entered_invocations;
    if invocations.len() > 4096
        || invocations
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != invocations.len()
    {
        return Err(bounds(
            "workflow entered invocation evidence must be unique and bounded",
        ));
    }
    let expected = commit
        .expected_generation
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| bounds("workflow generation exhausted"))?;
    if commit.snapshot.generation != expected {
        return Err(PersistenceError::InvalidDocument(
            "next workflow generation must increment exactly once".to_owned(),
        ));
    }
    identity(&commit.snapshot.owner_actor)?;
    if commit.snapshot.title.is_empty()
        || commit.snapshot.title.len() > 512
        || commit.snapshot.description.len() > 64 * 1024
        || serde_json::to_vec(&commit.snapshot.document)?.len() > MAX_WORKFLOW_DOCUMENT_BYTES
    {
        return Err(bounds(
            "workflow document or discovery text exceeds its bound",
        ));
    }
    if let Some(command) = &commit.command {
        identity(&command.actor)?;
        identity(&command.key)?;
        if command.canonical_request.len() > MAX_WORKFLOW_REQUEST_BYTES
            || serde_json::to_vec(&command.result)?.len() > MAX_RECEIPT_RESULT_BYTES
        {
            return Err(bounds(
                "workflow request or compact result exceeds its bound",
            ));
        }
        let request: serde_json::Value = serde_json::from_slice(&command.canonical_request)?;
        if serde_json::to_vec(&request)? != command.canonical_request {
            return Err(PersistenceError::InvalidDocument(
                "workflow request is not canonical JSON".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Task entry evidence lives in the same atomic snapshot as the frozen runtime start.
/// A task workflow never falls back to the retired journal when its proof is absent.
pub(crate) fn validate_task_invocation_in_transaction(
    write: &redb::WriteTransaction,
    run: &milkdrift_workspace::RunId,
    invocation: &milkdrift_capability::InvocationId,
) -> Result<bool, PersistenceError> {
    let workflows = write.open_table(TASK_WORKFLOWS).map_err(error::redb)?;
    let Some(bytes) = workflows.get(run.as_str()).map_err(error::redb)? else {
        return Ok(false);
    };
    let snapshot: WorkflowSnapshot = json::decode(bytes.value(), "task workflow")?;
    if snapshot.workflow.as_str() != run.as_str() {
        return Err(error::corruption(
            "task workflow key differs from its provenance owner",
        ));
    }
    if !snapshot.entered_invocations.contains(invocation) {
        return Err(PersistenceError::NotFound {
            entity: "task_artifact_provenance_invocation",
            identity: invocation.to_string(),
        });
    }
    Ok(true)
}

fn receipt_key(actor: &str, key: &str) -> Result<String, PersistenceError> {
    identity(actor)?;
    identity(key)?;
    Ok(format!("{}:{actor}{key}", actor.len()))
}

fn identity(value: &str) -> Result<(), PersistenceError> {
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(bounds(
            "task identity must contain 1..=512 bytes without control characters",
        ));
    }
    Ok(())
}

fn add(left: u64, right: u64) -> Result<u64, PersistenceError> {
    left.checked_add(right)
        .ok_or_else(|| bounds("task storage accounting overflow"))
}

fn replace(total: u64, old: u64, new: u64) -> Result<u64, PersistenceError> {
    add(
        total
            .checked_sub(old)
            .ok_or_else(|| error::corruption("task storage accounting underflow"))?,
        new,
    )
}

fn bounds(reason: &str) -> PersistenceError {
    PersistenceError::Bounds {
        location: "task_storage",
        reason: reason.to_owned(),
    }
}
