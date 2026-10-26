use super::ScanContext;
use crate::{
    error, json,
    schema::{
        NATIVE_PATH_CLAIMS, PEER_EXECUTION_TOMBSTONES, PEER_EXECUTIONS, TASK_ACCOUNTING,
        TASK_ACTION_OWNERS, TASK_AUTHORING, TASK_OUTPUT_USAGE, TASK_POSITIONS, TASK_RECEIPTS,
        TASK_WORKFLOWS, WORKSPACE_BUDGETS, WORKSPACE_USAGE,
    },
};
use milkdrift_persistence::PersistenceError;

pub(super) fn scan(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    context.string_bytes(
        0,
        &read.open_table(TASK_RECEIPTS).map_err(error::redb)?,
        "task_receipts",
        crate::task::validate_receipt_row,
    )?;
    context.string_bytes(
        1,
        &read.open_table(TASK_ACCOUNTING).map_err(error::redb)?,
        "task_accounting",
        crate::task::validate_accounting_row,
    )?;
    context.string_string(
        2,
        &read.open_table(TASK_ACTION_OWNERS).map_err(error::redb)?,
        "task_action_owners",
        |action, actor| {
            if !action.starts_with("stop:") || actor.is_empty() {
                return Err(error::corruption("mandatory action has invalid owner"));
            }
            let receipts = read.open_table(TASK_RECEIPTS).map_err(error::redb)?;
            let key = format!("{}:{actor}{action}", actor.len());
            if receipts.get(key.as_str()).map_err(error::redb)?.is_none() {
                return Err(error::corruption("mandatory action lost exact receipt"));
            }
            Ok(())
        },
    )?;
    context.string_bytes(
        3,
        &read.open_table(NATIVE_PATH_CLAIMS).map_err(error::redb)?,
        "native_path_claims",
        |key, bytes| {
            let claim: milkdrift_persistence::NativePathClaim =
                json::decode(bytes, "native path claim")?;
            crate::native_paths::validate_claim(key, &claim)
        },
    )?;
    context.string_bytes(
        4,
        &read.open_table(TASK_OUTPUT_USAGE).map_err(error::redb)?,
        "task_output_usage",
        |_, bytes| crate::task::validate_output_usage(bytes),
    )?;
    let authoring_validator: fn(&str, &[u8]) -> Result<(), PersistenceError> =
        crate::task::validate_authoring_row;
    for (phase, table, component, validate) in [
        (5, TASK_AUTHORING, "task_authoring", authoring_validator),
        (
            6,
            TASK_POSITIONS,
            "task_positions",
            crate::task::validate_positions_row,
        ),
    ] {
        context.string_bytes(
            phase,
            &read.open_table(table).map_err(error::redb)?,
            component,
            |key, bytes| {
                validate(key, bytes)?;
                if read
                    .open_table(TASK_WORKFLOWS)
                    .map_err(error::redb)?
                    .get(key)
                    .map_err(error::redb)?
                    .is_none()
                {
                    return Err(error::corruption("saved authoring lost its workflow"));
                }
                if phase == 5 {
                    let saved: milkdrift_persistence::WorkflowAuthoringSnapshot =
                        json::decode(bytes, "workflow authoring")?;
                    let metadata = read
                        .open_table(crate::schema::ARTIFACT_METADATA)
                        .map_err(error::redb)?;
                    for source in saved.protected_sources {
                        if metadata
                            .get(source.as_str())
                            .map_err(error::redb)?
                            .is_none()
                        {
                            return Err(error::corruption(
                                "saved authoring lost its protected source",
                            ));
                        }
                    }
                }
                Ok(())
            },
        )?;
    }
    for (phase, table, component) in [
        (12, WORKSPACE_USAGE, "artifact_owner_usage"),
        (13, WORKSPACE_BUDGETS, "artifact_owner_budget"),
    ] {
        context.string_bytes(
            phase,
            &read.open_table(table).map_err(error::redb)?,
            component,
            |key, _| {
                let owner = crate::artifact::owner::domain_owner(key)?;
                crate::artifact::domain::validated_workspace_domain(read, &owner)?
                    .ok_or_else(|| error::corruption("artifact domain is missing"))?;
                Ok(())
            },
        )?;
    }
    Ok(())
}

pub(super) fn scan_peers(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    for (phase, table, component) in [
        (62, PEER_EXECUTIONS, "peer_executions"),
        (63, PEER_EXECUTION_TOMBSTONES, "peer_tombstones"),
    ] {
        context.string_bytes(
            phase,
            &read.open_table(table).map_err(error::redb)?,
            component,
            |key, _| crate::peer::snapshot_in_read_transaction_text(read, key).map(|_| ()),
        )?;
    }
    Ok(())
}
