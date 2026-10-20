//! Nested process calls use the same bounded generation holds as direct task execution.
use super::{execution, invalid, uses};
use crate::{RedbStore, error};
use milkdrift_blueprint::WorkflowId;
use milkdrift_capability::{InvocationId, InvocationRequest, ResolvedCapabilitySnapshot};
use milkdrift_persistence::{
    PersistenceError, WorkflowSnapshot,
    managed::{ManagedExecution, ManagedUse},
};
use redb::ReadableTable;

pub(super) fn reserve(
    store: &RedbStore,
    workflow: &WorkflowId,
    parent: &InvocationId,
    request: &InvocationRequest,
    selection: &ResolvedCapabilitySnapshot,
) -> Result<ManagedUse, PersistenceError> {
    if request.capability() != selection.capability()
        || request.operation() != selection.operation()
        || !matches!(
            selection.category(),
            milkdrift_capability::CapabilityCategory::Process
        )
        || !(1..=16).any(|call| {
            milkdrift_persistence::managed::task_tool_invocation(parent, call)
                .is_ok_and(|expected| request.invocation() == &expected)
        })
    {
        return Err(invalid(
            "managed model tool requires one exact bounded process child",
        ));
    }
    let request_bytes = crate::json::encode(&(request, selection), "managed model tool request")?;
    let request_digest = format!("b3_{}", blake3::hash(&request_bytes));
    let write = store.database().begin_write().map_err(error::redb)?;
    let table = write
        .open_table(crate::schema::TASK_WORKFLOWS)
        .map_err(error::redb)?;
    let bytes = table
        .get(workflow.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| invalid("managed model tool owning workflow is absent"))?;
    let snapshot: WorkflowSnapshot = crate::json::decode(bytes.value(), "task workflow")?;
    if !snapshot.entered_invocations.contains(parent) {
        return Err(invalid("managed model tool parent has no durable entry"));
    }
    drop(bytes);
    drop(table);
    let execution = ManagedExecution::TaskTool {
        workflow: workflow.clone(),
        parent_invocation: parent.clone(),
        invocation: request.invocation().clone(),
        request_digest,
    };
    let id = execution.use_id();
    execution::acquire(&write, selection, execution)?;
    execution::entry(&write, &id)?;
    let (record, index) = uses::locate(&write, &id)?
        .ok_or_else(|| invalid("managed model tool selection has no managed binding"))?;
    let usage = record
        .uses
        .get(index)
        .ok_or_else(|| invalid("managed model tool use index missing"))?
        .clone();
    write.commit().map_err(error::redb)?;
    Ok(usage)
}
