//! Count anchors make missing active-index rows a refusal instead of silently free capacity.
use crate::{error, schema::METADATA};
use milkdrift_persistence::PersistenceError;
use redb::{ReadableTable, ReadableTableMetadata, TableDefinition, WriteTransaction};

pub(crate) fn verify_read_count(
    read: &redb::ReadTransaction,
    index: TableDefinition<'static, &'static str, u64>,
    count_key: &str,
) -> Result<(), PersistenceError> {
    let count = read
        .open_table(METADATA)
        .map_err(error::redb)?
        .get(count_key)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("active capacity count is missing"))?
        .value();
    if read
        .open_table(index)
        .map_err(error::redb)?
        .len()
        .map_err(error::redb)?
        != count
    {
        return Err(error::corruption(
            "active capacity count differs from its index",
        ));
    }
    Ok(())
}

pub(crate) fn count(
    write: &WriteTransaction,
    index: TableDefinition<'static, &'static str, u64>,
    count_key: &str,
) -> Result<u64, PersistenceError> {
    let count = write
        .open_table(METADATA)
        .map_err(error::redb)?
        .get(count_key)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("active capacity count is missing"))?
        .value();
    if write
        .open_table(index)
        .map_err(error::redb)?
        .len()
        .map_err(error::redb)?
        != count
    {
        return Err(error::corruption(
            "active capacity count differs from its index",
        ));
    }
    Ok(count)
}

pub(crate) fn set_membership(
    write: &WriteTransaction,
    index: TableDefinition<'static, &'static str, u64>,
    count_key: &str,
    identity: &str,
    active: bool,
) -> Result<(), PersistenceError> {
    count(write, index, count_key)?;
    let mut table = write.open_table(index).map_err(error::redb)?;
    if active {
        table.insert(identity, 1).map_err(error::redb)?;
    } else {
        table.remove(identity).map_err(error::redb)?;
    }
    write
        .open_table(METADATA)
        .map_err(error::redb)?
        .insert(count_key, table.len().map_err(error::redb)?)
        .map_err(error::redb)?;
    Ok(())
}
