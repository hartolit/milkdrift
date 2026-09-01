use super::super::{
    LEASE_ENTRIES, LEASE_INDEX, LeaseIndexEntry, PersistenceError, RUNNABLE_ENTRIES,
    RUNNABLE_INDEX, RUNNABLE_RUN_HEADS, RunnableIndexEntry, TIMER_ENTRIES, TIMER_INDEX,
    TimerIndexEntry, codec, error, json,
};
use super::{ScanContext, phase};

pub(super) fn scan_ordered(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let runnable_identities = read.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let runnable_ordered = read.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let timer_identities = read.open_table(TIMER_ENTRIES).map_err(error::redb)?;
    let timer_ordered = read.open_table(TIMER_INDEX).map_err(error::redb)?;
    let lease_identities = read.open_table(LEASE_ENTRIES).map_err(error::redb)?;
    let lease_ordered = read.open_table(LEASE_INDEX).map_err(error::redb)?;

    context.binary_bytes(
        phase::RUNNABLE_IDENTITIES,
        &runnable_identities,
        "runnable_indexes",
        |key, bytes| {
            let entry: RunnableIndexEntry = json::decode(bytes, "runnable index")?;
            let identity = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
            let ordered = crate::journal::runnable_order_key(&entry)?;
            validate_paired_index_row(
                key,
                bytes,
                &identity,
                &ordered,
                &runnable_ordered,
                "runnable",
            )
        },
    )?;
    context.binary_bytes(
        phase::RUNNABLE_ORDERED,
        &runnable_ordered,
        "runnable_indexes",
        |key, bytes| {
            let entry: RunnableIndexEntry = json::decode(bytes, "runnable index")?;
            let ordered = crate::journal::runnable_order_key(&entry)?;
            let identity = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
            validate_paired_index_row(
                key,
                bytes,
                &ordered,
                &identity,
                &runnable_identities,
                "runnable",
            )
        },
    )?;
    context.binary_bytes(
        phase::TIMER_IDENTITIES,
        &timer_identities,
        "timer_indexes",
        |key, bytes| {
            let entry: TimerIndexEntry = json::decode(bytes, "timer index")?;
            let identity = codec::pair(entry.run.as_str(), entry.timer.as_str())?;
            let ordered = crate::journal::timer_order_key(&entry)?;
            validate_paired_index_row(key, bytes, &identity, &ordered, &timer_ordered, "timer")
        },
    )?;
    context.binary_bytes(
        phase::TIMER_ORDERED,
        &timer_ordered,
        "timer_indexes",
        |key, bytes| {
            let entry: TimerIndexEntry = json::decode(bytes, "timer index")?;
            let ordered = crate::journal::timer_order_key(&entry)?;
            let identity = codec::pair(entry.run.as_str(), entry.timer.as_str())?;
            validate_paired_index_row(key, bytes, &ordered, &identity, &timer_identities, "timer")
        },
    )?;
    context.binary_bytes(
        phase::LEASE_IDENTITIES,
        &lease_identities,
        "lease_indexes",
        |key, bytes| {
            let entry: LeaseIndexEntry = json::decode(bytes, "lease index")?;
            let identity = codec::pair(entry.run.as_str(), entry.lease.as_str())?;
            let ordered = crate::journal::lease_order_key(&entry)?;
            validate_paired_index_row(key, bytes, &identity, &ordered, &lease_ordered, "lease")
        },
    )?;
    context.binary_bytes(
        phase::LEASE_ORDERED,
        &lease_ordered,
        "lease_indexes",
        |key, bytes| {
            let entry: LeaseIndexEntry = json::decode(bytes, "lease index")?;
            let ordered = crate::journal::lease_order_key(&entry)?;
            let identity = codec::pair(entry.run.as_str(), entry.lease.as_str())?;
            validate_paired_index_row(key, bytes, &ordered, &identity, &lease_identities, "lease")
        },
    )
}

pub(super) fn scan_run_heads(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let runnable_run_heads = read.open_table(RUNNABLE_RUN_HEADS).map_err(error::redb)?;
    context.string_bytes(
        phase::RUNNABLE_RUN_HEADS,
        &runnable_run_heads,
        "runnable_indexes",
        |run, bytes| crate::journal::validate_runnable_head(read, run, bytes).map(|_| ()),
    )
}

fn validate_paired_index_row(
    actual_key: &[u8],
    actual_value: &[u8],
    expected_key: &[u8],
    paired_key: &[u8],
    paired: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    family: &str,
) -> Result<(), PersistenceError> {
    if actual_key != expected_key {
        return Err(error::corruption(format!(
            "{family} index key does not match its checked document"
        )));
    }
    match paired.get(paired_key).map_err(error::redb)? {
        Some(value) if value.value() == actual_value => Ok(()),
        _ => Err(error::corruption(format!(
            "{family} identity/ordered index pair is missing or mismatched"
        ))),
    }
}
