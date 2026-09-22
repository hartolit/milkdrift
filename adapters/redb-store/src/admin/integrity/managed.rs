//! Resource records use the same bounded physical scan as every other durable owner.
use super::{ScanContext, phase};
use crate::{
    error,
    schema::{
        MANAGED_INSTALLATIONS, MANAGED_LINKS, MANAGED_LOCAL_USES, MANAGED_RECEIPTS,
        MANAGED_TRANSITIONS, MANAGED_USES,
    },
};
use milkdrift_persistence::PersistenceError;

pub(super) fn scan(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    context.string_bytes(
        phase::MANAGED_INSTALLATIONS,
        &read
            .open_table(MANAGED_INSTALLATIONS)
            .map_err(error::redb)?,
        "managed_installations",
        |key, bytes| crate::managed::integrity::record(read, key, bytes),
    )?;
    context.string_string(
        phase::MANAGED_USES,
        &read.open_table(MANAGED_USES).map_err(error::redb)?,
        "managed_uses",
        |id, name| crate::managed::integrity::use_index(read, id, name),
    )?;
    context.binary_bytes(
        phase::MANAGED_RECEIPTS,
        &read.open_table(MANAGED_RECEIPTS).map_err(error::redb)?,
        "managed_receipts",
        crate::managed::integrity::receipt,
    )?;
    context.string_bytes(
        phase::MANAGED_TRANSITIONS,
        &read.open_table(MANAGED_TRANSITIONS).map_err(error::redb)?,
        "managed_transitions",
        crate::managed::integrity::transition,
    )?;
    context.string_string(phase::MANAGED_LOCAL_USES, &read.open_table(MANAGED_LOCAL_USES).map_err(error::redb)?, "managed_local_uses", |key, id| {
        let uses = read.open_table(MANAGED_USES).map_err(error::redb)?;
        let name = uses.get(id).map_err(error::redb)?.ok_or_else(|| error::corruption("local use index lost lifetime hold"))?;
        let installations = read.open_table(MANAGED_INSTALLATIONS).map_err(error::redb)?;
        let bytes = installations.get(name.value()).map_err(error::redb)?.ok_or_else(|| error::corruption("local use inventory absent"))?;
        let record = crate::managed::decode_record(bytes.value())?;
        if !record.uses.iter().any(|usage| usage.id == id && matches!(&usage.execution, milkdrift_persistence::managed::ManagedExecution::Local { run, attempt, .. } if key == format!("local:{run}:{attempt}"))) { return Err(error::corruption("local use index conflicts with acceptance")); }
        Ok(())
    })?;
    context.string_bytes(
        phase::MANAGED_LINKS,
        &read.open_table(MANAGED_LINKS).map_err(error::redb)?,
        "managed_links",
        |key, bytes| crate::managed::verify_link(read, key, bytes),
    )
}
