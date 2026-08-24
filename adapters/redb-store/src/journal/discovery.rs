use std::{cmp::Ordering, collections::BTreeMap, ops::Bound};

use super::append::RunnableHeadState;
use super::*;

pub(crate) fn apply_indexes(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(summary) = request.indexes().summary() else {
        return Ok(());
    };
    let summary_bytes = json::encode(summary, "run summary")?;
    let previous = {
        let mut summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        let previous = summaries
            .get(summary.run.as_str())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec());
        summaries
            .insert(summary.run.as_str(), summary_bytes.as_slice())
            .map_err(error::redb)?;
        previous
    };
    transition_nonterminal_membership(write, summary, previous.as_deref())?;
    apply_runnable_mutations(write, request.indexes().runnable())?;
    apply_timer_mutations(write, request.indexes().timers())?;
    apply_lease_mutations(write, request.indexes().leases())?;
    Ok(())
}

fn transition_nonterminal_membership(
    write: &redb::WriteTransaction,
    summary: &RunSummaryIndex,
    previous_summary_bytes: Option<&[u8]>,
) -> Result<(), PersistenceError> {
    let previous_marker = write
        .open_table(NONTERMINAL_RUNS)
        .map_err(error::redb)?
        .get(summary.run.as_str())
        .map_err(error::redb)?
        .map(|marker| marker.value());
    match previous_summary_bytes {
        None if previous_marker.is_some() => {
            return Err(error::corruption(
                "nonterminal discovery exists without a prior run summary",
            ));
        }
        Some(bytes) => {
            let previous: RunSummaryIndex = json::decode(bytes, "run summary")?;
            if previous.run != summary.run {
                return Err(error::corruption(
                    "prior run-summary key disagrees with its document",
                ));
            }
            match (previous.state, previous_marker) {
                (IndexedRunState::Terminal, None) | (_, Some(1)) => {}
                (IndexedRunState::Terminal, Some(_)) => {
                    return Err(error::corruption(
                        "prior terminal run remains in nonterminal discovery",
                    ));
                }
                (_, Some(_)) => {
                    return Err(error::corruption(
                        "prior nonterminal discovery marker is invalid",
                    ));
                }
                (_, None) => {
                    return Err(error::corruption(
                        "prior nonterminal run is missing from discovery",
                    ));
                }
            }
        }
        None => {}
    }
    let mut nonterminal = write.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
    let replaced = if summary.state == IndexedRunState::Terminal {
        nonterminal
            .remove(summary.run.as_str())
            .map_err(error::redb)?
    } else {
        nonterminal
            .insert(summary.run.as_str(), 1)
            .map_err(error::redb)?
    };
    if replaced.as_ref().map(|marker| marker.value()) != previous_marker {
        return Err(error::corruption(
            "nonterminal marker changed outside the command transaction",
        ));
    }
    Ok(())
}

fn apply_runnable_mutations(
    write: &redb::WriteTransaction,
    mutations: &[RunnableIndexMutation],
) -> Result<(), PersistenceError> {
    let mut heads = BTreeMap::new();
    for mutation in mutations {
        let run = match mutation {
            RunnableIndexMutation::Upsert { entry } => &entry.run,
            RunnableIndexMutation::Remove { run, .. } => run,
        };
        if !heads.contains_key(run) {
            let previous_bytes =
                load_runnable_head_in_transaction(write, run)?.map(|(_entry, bytes)| bytes);
            heads.insert(run.clone(), RunnableHeadState { previous_bytes });
        }
    }
    let mut entries = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let mut ordered = write.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    for mutation in mutations {
        let (run, execution) = match mutation {
            RunnableIndexMutation::Upsert { entry } => (&entry.run, &entry.execution),
            RunnableIndexMutation::Remove { run, execution } => (run, execution),
        };
        let identity = codec::pair(run.as_str(), execution.as_str())?;
        let previous = entries
            .get(identity.as_slice())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec());
        if let Some(previous_bytes) = previous.as_deref() {
            let previous: RunnableIndexEntry = json::decode(previous_bytes, "runnable index")?;
            if previous.run != *run || previous.execution != *execution {
                return Err(error::corruption(
                    "runnable identity key disagrees with its document",
                ));
            }
            let key = runnable_order_key(&previous)?;
            let removed = ordered.remove(key.as_slice()).map_err(error::redb)?;
            if removed.as_ref().map(|value| value.value()) != Some(previous_bytes) {
                return Err(error::corruption(
                    "runnable identity row is missing or mismatched in its ordered index",
                ));
            }
        }
        match mutation {
            RunnableIndexMutation::Upsert { entry } => {
                let bytes = json::encode(entry, "runnable index")?;
                let order_key = runnable_order_key(entry)?;
                let replaced = entries
                    .insert(identity.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
                if replaced.as_ref().map(|value| value.value()) != previous.as_deref() {
                    return Err(error::corruption(
                        "runnable identity changed outside the command transaction",
                    ));
                }
                if ordered
                    .insert(order_key.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?
                    .is_some()
                {
                    return Err(error::corruption(
                        "runnable ordered upsert would overwrite another row",
                    ));
                }
            }
            RunnableIndexMutation::Remove { .. } => {
                let removed = entries.remove(identity.as_slice()).map_err(error::redb)?;
                if removed.as_ref().map(|value| value.value()) != previous.as_deref() {
                    return Err(error::corruption(
                        "runnable identity changed outside the command transaction",
                    ));
                }
            }
        }
    }
    drop(ordered);
    drop(entries);
    for (run, state) in heads {
        let selected = first_runnable_for_run_in_transaction(write, &run)?;
        persist_runnable_head(
            write,
            &run,
            state.previous_bytes.as_deref(),
            selected.as_ref(),
        )?;
    }
    Ok(())
}

fn apply_timer_mutations(
    write: &redb::WriteTransaction,
    mutations: &[TimerIndexMutation],
) -> Result<(), PersistenceError> {
    let mut entries = write.open_table(TIMER_ENTRIES).map_err(error::redb)?;
    let mut ordered = write.open_table(TIMER_INDEX).map_err(error::redb)?;
    for mutation in mutations {
        let (run, timer) = match mutation {
            TimerIndexMutation::Upsert { entry } => (&entry.run, &entry.timer),
            TimerIndexMutation::Remove { run, timer } => (run, timer),
        };
        let identity = codec::pair(run.as_str(), timer.as_str())?;
        let previous = entries
            .get(identity.as_slice())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec());
        if let Some(previous_bytes) = previous.as_deref() {
            let previous: TimerIndexEntry = json::decode(previous_bytes, "timer index")?;
            if previous.run != *run || previous.timer != *timer {
                return Err(error::corruption("timer key disagrees with its document"));
            }
            let removed = ordered
                .remove(timer_order_key(&previous)?.as_slice())
                .map_err(error::redb)?;
            if removed.as_ref().map(|value| value.value()) != Some(previous_bytes) {
                return Err(error::corruption(
                    "timer identity row is missing or mismatched in its ordered index",
                ));
            }
        }
        mutate_ordered_entry(
            &mut entries,
            &mut ordered,
            identity.as_slice(),
            previous.as_deref(),
            match mutation {
                TimerIndexMutation::Upsert { entry } => {
                    Some((json::encode(entry, "timer index")?, timer_order_key(entry)?))
                }
                TimerIndexMutation::Remove { .. } => None,
            },
            "timer",
        )?;
    }
    Ok(())
}

fn apply_lease_mutations(
    write: &redb::WriteTransaction,
    mutations: &[LeaseIndexMutation],
) -> Result<(), PersistenceError> {
    let mut entries = write.open_table(LEASE_ENTRIES).map_err(error::redb)?;
    let mut ordered = write.open_table(LEASE_INDEX).map_err(error::redb)?;
    for mutation in mutations {
        let (run, lease) = match mutation {
            LeaseIndexMutation::Upsert { entry } => (&entry.run, &entry.lease),
            LeaseIndexMutation::Remove { run, lease } => (run, lease),
        };
        let identity = codec::pair(run.as_str(), lease.as_str())?;
        let previous = entries
            .get(identity.as_slice())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec());
        if let Some(previous_bytes) = previous.as_deref() {
            let previous: LeaseIndexEntry = json::decode(previous_bytes, "lease index")?;
            if previous.run != *run || previous.lease != *lease {
                return Err(error::corruption("lease key disagrees with its document"));
            }
            let removed = ordered
                .remove(lease_order_key(&previous)?.as_slice())
                .map_err(error::redb)?;
            if removed.as_ref().map(|value| value.value()) != Some(previous_bytes) {
                return Err(error::corruption(
                    "lease identity row is missing or mismatched in its ordered index",
                ));
            }
        }
        mutate_ordered_entry(
            &mut entries,
            &mut ordered,
            identity.as_slice(),
            previous.as_deref(),
            match mutation {
                LeaseIndexMutation::Upsert { entry } => {
                    Some((json::encode(entry, "lease index")?, lease_order_key(entry)?))
                }
                LeaseIndexMutation::Remove { .. } => None,
            },
            "lease",
        )?;
    }
    drop(ordered);
    drop(entries);
    if !mutations.is_empty() {
        advance_lease_set_revision(write)?;
    }
    Ok(())
}

fn mutate_ordered_entry(
    entries: &mut redb::Table<'_, &[u8], &[u8]>,
    ordered: &mut redb::Table<'_, &[u8], &[u8]>,
    identity: &[u8],
    previous: Option<&[u8]>,
    replacement: Option<(Vec<u8>, Vec<u8>)>,
    label: &'static str,
) -> Result<(), PersistenceError> {
    let replaced = if let Some((bytes, order_key)) = replacement {
        let replaced = entries
            .insert(identity, bytes.as_slice())
            .map_err(error::redb)?;
        if ordered
            .insert(order_key.as_slice(), bytes.as_slice())
            .map_err(error::redb)?
            .is_some()
        {
            return Err(error::corruption(format!(
                "{label} ordered upsert would overwrite another row"
            )));
        }
        replaced
    } else {
        entries.remove(identity).map_err(error::redb)?
    };
    if replaced.as_ref().map(|value| value.value()) != previous {
        return Err(error::corruption(format!(
            "{label} identity changed outside the command transaction"
        )));
    }
    Ok(())
}

fn runnable_precedes(left: &RunnableIndexEntry, right: &RunnableIndexEntry) -> bool {
    match left.eligible_at.cmp(&right.eligible_at) {
        Ordering::Less => true,
        Ordering::Greater => false,
        Ordering::Equal => match right.priority.cmp(&left.priority) {
            Ordering::Less => true,
            Ordering::Greater => false,
            Ordering::Equal => left.execution.as_str() < right.execution.as_str(),
        },
    }
}

type StoredRunnableHead = (RunnableIndexEntry, Vec<u8>);

fn load_runnable_head_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<Option<StoredRunnableHead>, PersistenceError> {
    let stored = write
        .open_table(RUNNABLE_RUN_HEADS)
        .map_err(error::redb)?
        .get(run.as_str())
        .map_err(error::redb)?
        .map(|bytes| bytes.value().to_vec());
    let selected = first_runnable_for_run_in_transaction(write, run)?;
    match (stored, selected) {
        (None, None) => Ok(None),
        (Some(bytes), Some(selected)) => {
            let entry: RunnableIndexEntry = json::decode(&bytes, "runnable run head")?;
            if entry.run != *run || entry != selected {
                return Err(error::corruption(
                    "runnable run head is not the canonical earliest eligible entry",
                ));
            }
            Ok(Some((entry, bytes)))
        }
        _ => Err(error::corruption(
            "runnable entries and their per-run head are inconsistent",
        )),
    }
}

fn persist_runnable_head(
    write: &redb::WriteTransaction,
    run: &RunId,
    previous_bytes: Option<&[u8]>,
    selected: Option<&RunnableIndexEntry>,
) -> Result<(), PersistenceError> {
    let new_bytes = selected
        .map(|entry| json::encode(entry, "runnable run head"))
        .transpose()?;
    let mut heads = write.open_table(RUNNABLE_RUN_HEADS).map_err(error::redb)?;
    let replaced = if let Some(bytes) = &new_bytes {
        heads
            .insert(run.as_str(), bytes.as_slice())
            .map_err(error::redb)?
    } else {
        heads.remove(run.as_str()).map_err(error::redb)?
    };
    if replaced.as_ref().map(|bytes| bytes.value()) != previous_bytes {
        return Err(error::corruption(
            "runnable run head changed outside its command transaction",
        ));
    }
    Ok(())
}

fn first_runnable_for_run_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<Option<RunnableIndexEntry>, PersistenceError> {
    let entries = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let ordered = write.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    first_runnable_for_run_tables(&entries, &ordered, run)
}

pub(crate) fn first_runnable_for_run(
    read: &redb::ReadTransaction,
    run: &RunId,
) -> Result<Option<RunnableIndexEntry>, PersistenceError> {
    let entries = read.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let ordered = read.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    first_runnable_for_run_tables(&entries, &ordered, run)
}

fn first_runnable_for_run_tables<I, O>(
    entries: &I,
    ordered: &O,
    run: &RunId,
) -> Result<Option<RunnableIndexEntry>, PersistenceError>
where
    I: ReadableTable<&'static [u8], &'static [u8]>,
    O: ReadableTable<&'static [u8], &'static [u8]>,
{
    let prefix = codec::component(run.as_str())?;
    let end = codec::prefix_end(prefix.clone())
        .ok_or_else(|| error::corruption("runnable run prefix has no range end"))?;
    let mut selected: Option<RunnableIndexEntry> = None;
    for row in entries
        .range::<&[u8]>((
            Bound::Included(prefix.as_slice()),
            Bound::Excluded(end.as_slice()),
        ))
        .map_err(error::redb)?
    {
        let (identity, bytes) = row.map_err(error::redb)?;
        let entry: RunnableIndexEntry = json::decode(bytes.value(), "runnable index")?;
        let expected_identity = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
        if entry.run != *run || identity.value() != expected_identity.as_slice() {
            return Err(error::corruption(
                "runnable identity key disagrees with its document",
            ));
        }
        let ordered_bytes = ordered
            .get(runnable_order_key(&entry)?.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("runnable ordered index is incomplete"))?;
        if ordered_bytes.value() != bytes.value() {
            return Err(error::corruption(
                "runnable identity and ordered rows disagree",
            ));
        }
        if selected
            .as_ref()
            .is_none_or(|current| runnable_precedes(&entry, current))
        {
            selected = Some(entry);
        }
    }
    Ok(selected)
}

pub(crate) fn validate_runnable_head(
    read: &redb::ReadTransaction,
    run_key: &str,
    bytes: &[u8],
) -> Result<RunnableIndexEntry, PersistenceError> {
    let run = RunId::new(run_key)
        .map_err(|cause| error::corruption(format!("invalid runnable run identity: {cause}")))?;
    let entry: RunnableIndexEntry = json::decode(bytes, "runnable run head")?;
    if entry.run != run {
        return Err(error::corruption(
            "runnable run-head key disagrees with its document",
        ));
    }
    if first_runnable_for_run(read, &run)?.as_ref() != Some(&entry) {
        return Err(error::corruption(
            "runnable run head is not the canonical earliest eligible entry",
        ));
    }
    Ok(entry)
}

pub(crate) fn runnable_order_key(entry: &RunnableIndexEntry) -> Result<Vec<u8>, PersistenceError> {
    codec::ordered_timestamp(
        entry.eligible_at.get(),
        &format!("{}\0{}", entry.run, entry.execution),
    )
}

pub(crate) fn timer_order_key(entry: &TimerIndexEntry) -> Result<Vec<u8>, PersistenceError> {
    codec::ordered_timestamp(
        entry.fire_at.get(),
        &format!("{}\0{}", entry.run, entry.timer),
    )
}

pub(crate) fn lease_order_key(entry: &LeaseIndexEntry) -> Result<Vec<u8>, PersistenceError> {
    codec::ordered_timestamp(
        entry.expires_at.get(),
        &format!("{}\0{}", entry.run, entry.lease),
    )
}

pub(crate) fn lease_set_revision_in_transaction(
    write: &redb::WriteTransaction,
) -> Result<IntegrityDigest, PersistenceError> {
    let metadata = write.open_table(METADATA).map_err(error::redb)?;
    let revision = metadata
        .get(LEASE_SET_REVISION_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("lease-set revision is missing"))?
        .value();
    Ok(lease_revision_digest(revision))
}

pub(crate) fn lease_set_revision(
    read: &redb::ReadTransaction,
) -> Result<IntegrityDigest, PersistenceError> {
    let metadata = read.open_table(METADATA).map_err(error::redb)?;
    let revision = metadata
        .get(LEASE_SET_REVISION_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("lease-set revision is missing"))?
        .value();
    Ok(lease_revision_digest(revision))
}

fn lease_revision_digest(revision: u64) -> IntegrityDigest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.redb.lease-set-revision.v1\0");
    hasher.update(&revision.to_be_bytes());
    IntegrityDigest::hash(hasher.finalize().as_bytes())
}

fn advance_lease_set_revision(write: &redb::WriteTransaction) -> Result<(), PersistenceError> {
    let mut metadata = write.open_table(METADATA).map_err(error::redb)?;
    let current = metadata
        .get(LEASE_SET_REVISION_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("lease-set revision is missing"))?
        .value();
    let next = current
        .checked_add(1)
        .ok_or_else(|| error::corruption("lease-set revision overflowed"))?;
    metadata
        .insert(LEASE_SET_REVISION_KEY, next)
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn record_artifact_references(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    crate::artifact::validate_artifact_state(write)?;
    for reference in request.required_artifacts() {
        let digest = reference.digest().to_hex();
        let key = codec::components(&[
            &digest,
            reference.artifact().as_str(),
            request.receipt().run().as_str(),
            request.receipt().command().as_str(),
        ])?;
        crate::artifact::persist_artifact_reference_occurrence(write, &key, reference)?;
        crate::artifact::persist_run_artifact_ownership(write, request.receipt().run(), reference)?;
    }
    crate::artifact::validate_artifact_state(write)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_ordered_index<T: for<'de> Deserialize<'de> + Serialize>(
    read: &redb::ReadTransaction,
    identity_definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    through: TimestampMillis,
    limit: PageSize,
    label: &'static str,
    timestamp: impl Fn(&T) -> TimestampMillis,
    order_key: impl Fn(&T) -> Result<Vec<u8>, PersistenceError>,
    identity_key: impl Fn(&T) -> Result<Vec<u8>, PersistenceError>,
) -> Result<Vec<T>, PersistenceError> {
    let identities = read.open_table(identity_definition).map_err(error::redb)?;
    let ordered = read.open_table(definition).map_err(error::redb)?;
    if identities.len().map_err(error::redb)? != ordered.len().map_err(error::redb)? {
        return Err(error::corruption(format!(
            "{label} identity and ordered indexes have different cardinality"
        )));
    }
    let mut results = Vec::with_capacity(limit.get() as usize);
    for row in ordered.iter().map_err(error::redb)? {
        let (stored_order_key, bytes) = row.map_err(error::redb)?;
        let entry: T = json::decode(bytes.value(), label)?;
        if stored_order_key.value() != order_key(&entry)?.as_slice() {
            return Err(error::corruption(format!(
                "{label} ordered key disagrees with its document"
            )));
        }
        let identity = identity_key(&entry)?;
        let identity_bytes = identities
            .get(identity.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption(format!("{label} identity row is missing")))?;
        if identity_bytes.value() != bytes.value() {
            return Err(error::corruption(format!(
                "{label} ordered and identity rows disagree"
            )));
        }
        if timestamp(&entry) > through {
            break;
        }
        results.push(entry);
        if results.len() == limit.get() as usize {
            break;
        }
    }
    Ok(results)
}
