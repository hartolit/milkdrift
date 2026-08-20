use super::*;
use super::{
    append::{
        RunnableHeadState, nonterminal_membership_path, nonterminal_membership_payload,
        predecessor_path, run_membership_payload,
    },
    queries::page_size_usize,
};
pub(crate) fn apply_indexes(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(summary) = &request.indexes.summary else {
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
    transition_nonterminal_membership(write, summary, &summary_bytes, previous.as_deref())?;
    apply_runnable_mutations(write, &request.indexes.runnable)?;
    apply_timer_mutations(write, &request.indexes.timers)?;
    apply_lease_mutations(write, &request.indexes.leases)?;
    Ok(())
}

pub(crate) fn transition_nonterminal_membership(
    write: &redb::WriteTransaction,
    summary: &RunSummaryIndex,
    summary_bytes: &[u8],
    previous_summary_bytes: Option<&[u8]>,
) -> Result<(), PersistenceError> {
    let family = crate::trie::CatalogFamily::NonterminalRun;
    let path = nonterminal_membership_path(&summary.run);
    let previous_witness = crate::trie::verify_member_in_transaction(
        write,
        family,
        path,
        summary.run.as_str().as_bytes(),
    )?;
    let previous_marker = {
        let nonterminal = write.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
        nonterminal
            .get(summary.run.as_str())
            .map_err(error::redb)?
            .map(|marker| marker.value())
    };
    let previous_expected = match previous_summary_bytes {
        None => {
            if previous_marker.is_some() || previous_witness.is_some() {
                return Err(error::corruption(
                    "nonterminal discovery exists without a prior run summary",
                ));
            }
            None
        }
        Some(bytes) => {
            let previous: RunSummaryIndex = json::decode(bytes, "run summary")?;
            if previous.run != summary.run {
                return Err(error::corruption(
                    "prior run-summary key disagrees with its document",
                ));
            }
            match (previous.state, previous_marker) {
                (IndexedRunState::Terminal, None) => None,
                (IndexedRunState::Terminal, Some(_)) => {
                    return Err(error::corruption(
                        "prior terminal run remains in nonterminal discovery",
                    ));
                }
                (_, Some(1)) => Some(nonterminal_membership_payload(run_membership_payload(
                    &previous.run,
                    previous.through_sequence,
                    bytes,
                ))),
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
    };
    if previous_witness != previous_expected {
        return Err(error::corruption(
            "prior nonterminal discovery disagrees with its authenticated catalog",
        ));
    }

    {
        let mut nonterminal = write.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
        if summary.state == IndexedRunState::Terminal {
            let removed = nonterminal
                .remove(summary.run.as_str())
                .map_err(error::redb)?;
            if removed.as_ref().map(|marker| marker.value()) != previous_marker {
                return Err(error::corruption(
                    "nonterminal marker changed outside the command transaction",
                ));
            }
        } else {
            let replaced = nonterminal
                .insert(summary.run.as_str(), 1)
                .map_err(error::redb)?;
            if replaced.as_ref().map(|marker| marker.value()) != previous_marker {
                return Err(error::corruption(
                    "nonterminal marker changed outside the command transaction",
                ));
            }
        }
    }
    let replaced = if summary.state == IndexedRunState::Terminal {
        crate::trie::remove(write, family, path, summary.run.as_str().as_bytes())?
    } else {
        let run_payload =
            run_membership_payload(&summary.run, summary.through_sequence, summary_bytes);
        crate::trie::put(
            write,
            family,
            path,
            summary.run.as_str().as_bytes(),
            nonterminal_membership_payload(run_payload),
        )?
    };
    if replaced != previous_witness {
        return Err(error::corruption(
            "nonterminal witness changed outside the command transaction",
        ));
    }
    Ok(())
}

pub(crate) fn apply_runnable_mutations(
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
            let loaded = load_runnable_head_in_transaction(write, run)?;
            let state = match loaded {
                Some((_entry, bytes, witness)) => RunnableHeadState {
                    previous_bytes: Some(bytes),
                    previous_witness: Some(witness),
                },
                None => RunnableHeadState {
                    previous_bytes: None,
                    previous_witness: None,
                },
            };
            heads.insert(run.clone(), state);
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
        let had_previous =
            if let Some(previous) = entries.get(identity.as_slice()).map_err(error::redb)? {
                let previous_bytes = previous.value().to_vec();
                let previous: RunnableIndexEntry = json::decode(&previous_bytes, "runnable index")?;
                remove_runnable_catalog_entry(write, &identity, &previous, &previous_bytes)?;
                let key = runnable_order_key(&previous)?;
                let removed = ordered.remove(key.as_slice()).map_err(error::redb)?;
                if removed.as_ref().map(|value| value.value()) != Some(previous_bytes.as_slice()) {
                    return Err(error::corruption(
                        "runnable identity row is missing or mismatched in its ordered index",
                    ));
                }
                true
            } else {
                ensure_catalog_absent(
                    write,
                    crate::trie::CatalogFamily::RunnableIdentity,
                    runnable_catalog_identity_path(run, &identity)?,
                    &identity,
                    "runnable",
                )?;
                false
            };
        match mutation {
            RunnableIndexMutation::Upsert { entry } => {
                let bytes = json::encode(entry, "runnable index")?;
                let order_key = runnable_order_key(entry)?;
                let replaced_identity = entries
                    .insert(identity.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
                if replaced_identity.is_some() != had_previous {
                    return Err(error::corruption(
                        "runnable identity upsert found unexpected physical state",
                    ));
                }
                if ordered
                    .insert(order_key.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?
                    .is_some()
                {
                    return Err(error::corruption(
                        "runnable ordered upsert would overwrite an unauthenticated row",
                    ));
                }
                insert_runnable_catalog_entry(write, &identity, entry, &bytes)?;
            }
            RunnableIndexMutation::Remove { .. } => {
                let _removed = entries.remove(identity.as_slice()).map_err(error::redb)?;
            }
        }
    }
    drop(ordered);
    drop(entries);
    for (run, state) in heads {
        let selected = best_runnable_for_run_in_transaction(write, &run, None)?;
        persist_runnable_head(
            write,
            &run,
            state.previous_bytes.as_deref(),
            state.previous_witness,
            selected.as_ref(),
        )?;
    }
    Ok(())
}

pub(crate) fn apply_timer_mutations(
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
        let had_previous =
            if let Some(previous) = entries.get(identity.as_slice()).map_err(error::redb)? {
                let previous_bytes = previous.value().to_vec();
                let previous: TimerIndexEntry = json::decode(&previous_bytes, "timer index")?;
                remove_timer_catalog_entry(write, &identity, &previous, &previous_bytes)?;
                let key = timer_order_key(&previous)?;
                let removed = ordered.remove(key.as_slice()).map_err(error::redb)?;
                if removed.as_ref().map(|value| value.value()) != Some(previous_bytes.as_slice()) {
                    return Err(error::corruption(
                        "timer identity row is missing or mismatched in its ordered index",
                    ));
                }
                true
            } else {
                ensure_catalog_absent(
                    write,
                    crate::trie::CatalogFamily::TimerIdentity,
                    crate::trie::hashed_path(crate::trie::CatalogFamily::TimerIdentity, &identity),
                    &identity,
                    "timer",
                )?;
                false
            };
        match mutation {
            TimerIndexMutation::Upsert { entry } => {
                let bytes = json::encode(entry, "timer index")?;
                let order_key = timer_order_key(entry)?;
                let replaced_identity = entries
                    .insert(identity.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
                if replaced_identity.is_some() != had_previous {
                    return Err(error::corruption(
                        "timer identity upsert found unexpected physical state",
                    ));
                }
                if ordered
                    .insert(order_key.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?
                    .is_some()
                {
                    return Err(error::corruption(
                        "timer ordered upsert would overwrite an unauthenticated row",
                    ));
                }
                insert_timer_catalog_entry(write, &identity, entry, &bytes)?;
            }
            TimerIndexMutation::Remove { .. } => {
                let _removed = entries.remove(identity.as_slice()).map_err(error::redb)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn apply_lease_mutations(
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
        let had_previous =
            if let Some(previous) = entries.get(identity.as_slice()).map_err(error::redb)? {
                let previous_bytes = previous.value().to_vec();
                let previous: LeaseIndexEntry = json::decode(&previous_bytes, "lease index")?;
                remove_lease_catalog_entry(write, &identity, &previous, &previous_bytes)?;
                let key = lease_order_key(&previous)?;
                let removed = ordered.remove(key.as_slice()).map_err(error::redb)?;
                if removed.as_ref().map(|value| value.value()) != Some(previous_bytes.as_slice()) {
                    return Err(error::corruption(
                        "lease identity row is missing or mismatched in its ordered index",
                    ));
                }
                true
            } else {
                ensure_catalog_absent(
                    write,
                    crate::trie::CatalogFamily::LeaseIdentity,
                    crate::trie::hashed_path(crate::trie::CatalogFamily::LeaseIdentity, &identity),
                    &identity,
                    "lease",
                )?;
                false
            };
        match mutation {
            LeaseIndexMutation::Upsert { entry } => {
                let bytes = json::encode(entry, "lease index")?;
                let order_key = lease_order_key(entry)?;
                let replaced_identity = entries
                    .insert(identity.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
                if replaced_identity.is_some() != had_previous {
                    return Err(error::corruption(
                        "lease identity upsert found unexpected physical state",
                    ));
                }
                if ordered
                    .insert(order_key.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?
                    .is_some()
                {
                    return Err(error::corruption(
                        "lease ordered upsert would overwrite an unauthenticated row",
                    ));
                }
                insert_lease_catalog_entry(write, &identity, entry, &bytes)?;
            }
            LeaseIndexMutation::Remove { .. } => {
                let _removed = entries.remove(identity.as_slice()).map_err(error::redb)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn runnable_catalog_identity_path(
    _run: &RunId,
    identity: &[u8],
) -> Result<[u8; 32], PersistenceError> {
    Ok(crate::trie::hashed_path(
        crate::trie::CatalogFamily::RunnableIdentity,
        identity,
    ))
}

pub(crate) fn runnable_group(family: crate::trie::CatalogFamily, key: &[u8]) -> [u8; 16] {
    let run_hash = crate::trie::hashed_path(family, key);
    let mut group = [0_u8; 16];
    group.copy_from_slice(&run_hash[..16]);
    group
}

pub(crate) fn runnable_bucket_key(
    run: &RunId,
    eligible_at: TimestampMillis,
) -> Result<Vec<u8>, PersistenceError> {
    codec::components(&[run.as_str(), &eligible_at.get().to_string()])
}

pub(crate) fn runnable_bucket_path(
    run: &RunId,
    eligible_at: TimestampMillis,
    key: &[u8],
) -> Result<[u8; 32], PersistenceError> {
    let family = crate::trie::CatalogFamily::RunnableBucket;
    let mut prefix = [0_u8; 24];
    prefix[..16].copy_from_slice(&runnable_group(family, run.as_str().as_bytes()));
    prefix[16..].copy_from_slice(&eligible_at.get().to_be_bytes());
    crate::trie::ordered_path(family, &prefix, key)
}

pub(crate) fn runnable_bucket_entry_path(
    bucket_key: &[u8],
    identity: &[u8],
    priority: u16,
) -> Result<[u8; 32], PersistenceError> {
    let family = crate::trie::CatalogFamily::RunnableBucketEntry;
    let mut prefix = [0_u8; 18];
    prefix[..16].copy_from_slice(&runnable_group(family, bucket_key));
    prefix[16..].copy_from_slice(&(u16::MAX - priority).to_be_bytes());
    crate::trie::ordered_path(family, &prefix, identity)
}

pub(crate) fn first_path_in_group(group: [u8; 16]) -> Option<[u8; 32]> {
    let mut first = [0_u8; 32];
    first[..16].copy_from_slice(&group);
    predecessor_path(first)
}

pub(crate) fn runnable_head_path(run: &RunId) -> [u8; 32] {
    let family = crate::trie::CatalogFamily::RunnableRunHead;
    crate::trie::hashed_path(family, run.as_str().as_bytes())
}

type StoredRunnableHead = (RunnableIndexEntry, Vec<u8>, [u8; 32]);

pub(crate) fn load_runnable_head_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<Option<StoredRunnableHead>, PersistenceError> {
    let stored = {
        let table = write.open_table(RUNNABLE_RUN_HEADS).map_err(error::redb)?;
        table
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec())
    };
    let family = crate::trie::CatalogFamily::RunnableRunHead;
    let witness = crate::trie::verify_member_in_transaction(
        write,
        family,
        runnable_head_path(run),
        run.as_str().as_bytes(),
    )?;
    match (stored, witness) {
        (None, None) => Ok(None),
        (Some(bytes), Some(witness)) if witness == crate::trie::digest_payload(family, &bytes) => {
            let entry: RunnableIndexEntry = json::decode(&bytes, "runnable run head")?;
            if entry.run != *run {
                return Err(error::corruption(
                    "runnable run-head key disagrees with its document",
                ));
            }
            validate_runnable_entry_in_transaction(write, &entry)?;
            if first_runnable_for_run_in_transaction(write, run)?.as_ref() != Some(&entry) {
                return Err(error::corruption(
                    "runnable run head is not the canonical earliest eligible entry",
                ));
            }
            Ok(Some((entry, bytes, witness)))
        }
        _ => Err(error::corruption(
            "runnable run head disagrees with its authenticated catalog",
        )),
    }
}

pub(crate) fn persist_runnable_head(
    write: &redb::WriteTransaction,
    run: &RunId,
    previous_bytes: Option<&[u8]>,
    previous_witness: Option<[u8; 32]>,
    selected: Option<&RunnableIndexEntry>,
) -> Result<(), PersistenceError> {
    let family = crate::trie::CatalogFamily::RunnableRunHead;
    let new_bytes = selected
        .map(|entry| {
            if entry.run != *run {
                return Err(error::corruption(
                    "runnable run head belongs to another run",
                ));
            }
            validate_runnable_entry_in_transaction(write, entry)?;
            json::encode(entry, "runnable run head")
        })
        .transpose()?;
    {
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
    }
    let replaced = if let Some(bytes) = &new_bytes {
        crate::trie::put(
            write,
            family,
            runnable_head_path(run),
            run.as_str().as_bytes(),
            crate::trie::digest_payload(family, bytes),
        )?
    } else {
        crate::trie::remove(
            write,
            family,
            runnable_head_path(run),
            run.as_str().as_bytes(),
        )?
    };
    if replaced != previous_witness {
        return Err(error::corruption(
            "runnable run-head witness changed outside its command transaction",
        ));
    }
    Ok(())
}

pub(crate) fn validate_runnable_entry_in_transaction(
    write: &redb::WriteTransaction,
    entry: &RunnableIndexEntry,
) -> Result<(), PersistenceError> {
    let identity = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
    let bytes = {
        let entries = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
        entries
            .get(identity.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("runnable run head names a missing identity row"))?
            .value()
            .to_vec()
    };
    let stored: RunnableIndexEntry = json::decode(&bytes, "runnable index")?;
    if &stored != entry {
        return Err(error::corruption(
            "runnable run head disagrees with its identity row",
        ));
    }
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    let identity_witness = crate::trie::verify_member_in_transaction(
        write,
        identity_family,
        runnable_catalog_identity_path(&entry.run, &identity)?,
        &identity,
    )?;
    if identity_witness != Some(crate::trie::digest_payload(identity_family, &bytes)) {
        return Err(error::corruption(
            "runnable identity row disagrees with its authenticated catalog",
        ));
    }
    let order_key = runnable_order_key(entry)?;
    let ordered = write.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let ordered_bytes = ordered
        .get(order_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable run head has no ordered row"))?;
    if ordered_bytes.value() != bytes.as_slice() {
        return Err(error::corruption(
            "runnable identity and ordered rows disagree",
        ));
    }
    let ordered_family = crate::trie::CatalogFamily::RunnableOrdered;
    let ordered_witness = crate::trie::verify_member_in_transaction(
        write,
        ordered_family,
        runnable_catalog_ordered_path(&identity, entry)?,
        &identity,
    )?;
    if ordered_witness != Some(crate::trie::digest_payload(ordered_family, &bytes)) {
        return Err(error::corruption(
            "runnable ordered row disagrees with its authenticated catalog",
        ));
    }
    Ok(())
}

pub(crate) fn best_runnable_for_run_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    _eligible_through: Option<TimestampMillis>,
) -> Result<Option<RunnableIndexEntry>, PersistenceError> {
    first_runnable_for_run_in_transaction(write, run)
}

pub(crate) fn first_runnable_for_run_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<Option<RunnableIndexEntry>, PersistenceError> {
    let bucket_family = crate::trie::CatalogFamily::RunnableBucket;
    let run_group = runnable_group(bucket_family, run.as_str().as_bytes());
    let buckets = crate::trie::page_in_transaction(
        write,
        bucket_family,
        None,
        first_path_in_group(run_group),
        1,
    )?;
    let Some(bucket) = buckets.leaves.first() else {
        return Ok(None);
    };
    if bucket.path[..16] != run_group {
        return Ok(None);
    }
    let components = codec::decode_components(&bucket.logical_key, 2)?;
    let bucket_run = RunId::new(components[0]).map_err(|cause| {
        error::corruption(format!(
            "runnable bucket has an invalid run identity: {cause}"
        ))
    })?;
    let eligible_at = components[1]
        .parse::<u64>()
        .map_err(|_| error::corruption("runnable bucket has an invalid eligibility timestamp"))?;
    let eligible_at = TimestampMillis::new(eligible_at);
    if bucket_run != *run
        || bucket.path != runnable_bucket_path(run, eligible_at, &bucket.logical_key)?
        || bucket.payload_digest != crate::trie::digest_payload(bucket_family, &bucket.logical_key)
    {
        return Err(error::corruption(
            "runnable eligible bucket disagrees with its authenticated key",
        ));
    }

    let entry_family = crate::trie::CatalogFamily::RunnableBucketEntry;
    let entry_group = runnable_group(entry_family, &bucket.logical_key);
    let entries = crate::trie::page_in_transaction(
        write,
        entry_family,
        None,
        first_path_in_group(entry_group),
        1,
    )?;
    let entry_leaf = entries
        .leaves
        .first()
        .ok_or_else(|| error::corruption("runnable eligible bucket is empty"))?;
    if entry_leaf.path[..16] != entry_group {
        return Err(error::corruption(
            "runnable eligible bucket has no authenticated entries",
        ));
    }
    let identities = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let ordered = write.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let identity_bytes = identities
        .get(entry_leaf.logical_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable bucket entry is dangling"))?
        .value()
        .to_vec();
    if entry_leaf.payload_digest != crate::trie::digest_payload(entry_family, &identity_bytes) {
        return Err(error::corruption(
            "runnable bucket entry disagrees with its physical document",
        ));
    }
    let entry: RunnableIndexEntry = json::decode(&identity_bytes, "runnable index")?;
    if entry.run != *run
        || entry.eligible_at != eligible_at
        || entry_leaf.path
            != runnable_bucket_entry_path(
                &bucket.logical_key,
                &entry_leaf.logical_key,
                entry.priority,
            )?
    {
        return Err(error::corruption(
            "runnable bucket entry disagrees with its checked document",
        ));
    }
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    let identity_witness = crate::trie::verify_member_in_transaction(
        write,
        identity_family,
        runnable_catalog_identity_path(run, &entry_leaf.logical_key)?,
        &entry_leaf.logical_key,
    )?;
    let entry = validate_runnable_leaf_in_transaction(
        write,
        &identities,
        &ordered,
        &crate::trie::TrieLeaf {
            path: runnable_catalog_identity_path(run, &entry_leaf.logical_key)?,
            logical_key: entry_leaf.logical_key.clone(),
            payload_digest: identity_witness.ok_or_else(|| {
                error::corruption("runnable bucket entry has no authenticated identity")
            })?,
        },
    )?;
    Ok(Some(entry))
}

pub(crate) fn first_runnable_for_run(
    read: &redb::ReadTransaction,
    run: &RunId,
) -> Result<Option<RunnableIndexEntry>, PersistenceError> {
    let bucket_family = crate::trie::CatalogFamily::RunnableBucket;
    let run_group = runnable_group(bucket_family, run.as_str().as_bytes());
    let buckets = crate::trie::page(read, bucket_family, None, first_path_in_group(run_group), 1)?;
    let Some(bucket) = buckets.leaves.first() else {
        return Ok(None);
    };
    if bucket.path[..16] != run_group {
        return Ok(None);
    }
    let components = codec::decode_components(&bucket.logical_key, 2)?;
    let bucket_run = RunId::new(components[0]).map_err(|cause| {
        error::corruption(format!(
            "runnable bucket has an invalid run identity: {cause}"
        ))
    })?;
    let eligible_at =
        TimestampMillis::new(components[1].parse::<u64>().map_err(|_| {
            error::corruption("runnable bucket has an invalid eligibility timestamp")
        })?);
    if bucket_run != *run
        || bucket.path != runnable_bucket_path(run, eligible_at, &bucket.logical_key)?
        || bucket.payload_digest != crate::trie::digest_payload(bucket_family, &bucket.logical_key)
    {
        return Err(error::corruption(
            "runnable eligible bucket disagrees with its authenticated key",
        ));
    }
    let entry_family = crate::trie::CatalogFamily::RunnableBucketEntry;
    let entry_group = runnable_group(entry_family, &bucket.logical_key);
    let entries_page = crate::trie::page(
        read,
        entry_family,
        None,
        first_path_in_group(entry_group),
        1,
    )?;
    let entry_leaf = entries_page
        .leaves
        .first()
        .ok_or_else(|| error::corruption("runnable eligible bucket is empty"))?;
    if entry_leaf.path[..16] != entry_group {
        return Err(error::corruption(
            "runnable eligible bucket has no authenticated entries",
        ));
    }
    let identities = read.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let ordered = read.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let identity_bytes = identities
        .get(entry_leaf.logical_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable bucket entry is dangling"))?
        .value()
        .to_vec();
    if entry_leaf.payload_digest != crate::trie::digest_payload(entry_family, &identity_bytes) {
        return Err(error::corruption(
            "runnable bucket entry disagrees with its physical document",
        ));
    }
    let entry: RunnableIndexEntry = json::decode(&identity_bytes, "runnable index")?;
    if entry.run != *run
        || entry.eligible_at != eligible_at
        || entry_leaf.path
            != runnable_bucket_entry_path(
                &bucket.logical_key,
                &entry_leaf.logical_key,
                entry.priority,
            )?
    {
        return Err(error::corruption(
            "runnable bucket entry disagrees with its checked document",
        ));
    }
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    let identity_witness = crate::trie::verify_member(
        read,
        identity_family,
        runnable_catalog_identity_path(run, &entry_leaf.logical_key)?,
        &entry_leaf.logical_key,
    )?;
    Ok(Some(validate_runnable_leaf(
        read,
        &identities,
        &ordered,
        &crate::trie::TrieLeaf {
            path: runnable_catalog_identity_path(run, &entry_leaf.logical_key)?,
            logical_key: entry_leaf.logical_key.clone(),
            payload_digest: identity_witness.ok_or_else(|| {
                error::corruption("runnable bucket entry has no authenticated identity")
            })?,
        },
    )?))
}

pub(crate) fn migrate_runnable_run_heads(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    let heads = write.open_table(RUNNABLE_RUN_HEADS).map_err(error::redb)?;
    if heads.len().map_err(error::redb)? != 0 {
        return Err(error::corruption(
            "legacy storage unexpectedly contains runnable run heads",
        ));
    }
    drop(heads);
    let entries = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let mut current_run: Option<RunId> = None;
    let mut best: Option<RunnableIndexEntry> = None;
    for item in entries.iter().map_err(error::redb)? {
        let (identity, bytes) = item.map_err(error::redb)?;
        let entry: RunnableIndexEntry = json::decode(bytes.value(), "runnable index")?;
        if current_run.as_ref().is_some_and(|run| run != &entry.run) {
            let run = current_run
                .as_ref()
                .ok_or_else(|| error::corruption("legacy runnable run tracker is empty"))?;
            let selected = best
                .as_ref()
                .ok_or_else(|| error::corruption("legacy runnable run has no selected head"))?;
            persist_migrated_runnable_head(write, run, selected)?;
            best = None;
        }
        current_run = Some(entry.run.clone());
        insert_runnable_bucket_entry(write, identity.value(), &entry, bytes.value())?;
        if best
            .as_ref()
            .is_none_or(|selected| runnable_precedes(&entry, selected))
        {
            best = Some(entry);
        }
    }
    if let Some(run) = &current_run {
        let selected = best
            .as_ref()
            .ok_or_else(|| error::corruption("legacy runnable run has no selected head"))?;
        persist_migrated_runnable_head(write, run, selected)?;
    }
    Ok(())
}

pub(crate) fn persist_migrated_runnable_head(
    write: &redb::WriteTransaction,
    run: &RunId,
    selected: &RunnableIndexEntry,
) -> Result<(), PersistenceError> {
    if selected.run != *run {
        return Err(error::corruption(
            "legacy runnable head belongs to another run",
        ));
    }
    let bytes = json::encode(selected, "runnable run head")?;
    if write
        .open_table(RUNNABLE_RUN_HEADS)
        .map_err(error::redb)?
        .insert(run.as_str(), bytes.as_slice())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(error::corruption(
            "legacy runnable head migration replaced an existing row",
        ));
    }
    let family = crate::trie::CatalogFamily::RunnableRunHead;
    if crate::trie::put(
        write,
        family,
        runnable_head_path(run),
        run.as_str().as_bytes(),
        crate::trie::digest_payload(family, &bytes),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "legacy runnable head migration replaced an authenticated leaf",
        ));
    }
    Ok(())
}

pub(crate) fn runnable_catalog_ordered_path(
    identity: &[u8],
    entry: &RunnableIndexEntry,
) -> Result<[u8; 32], PersistenceError> {
    crate::trie::ordered_path(
        crate::trie::CatalogFamily::RunnableOrdered,
        &entry.eligible_at.get().to_be_bytes(),
        identity,
    )
}

pub(crate) fn insert_runnable_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &RunnableIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    insert_discovery_catalog_entry(
        write,
        identity,
        bytes,
        crate::trie::CatalogFamily::RunnableIdentity,
        runnable_catalog_identity_path(&entry.run, identity)?,
        crate::trie::CatalogFamily::RunnableOrdered,
        runnable_catalog_ordered_path(identity, entry)?,
        "runnable",
    )?;
    insert_runnable_bucket_entry(write, identity, entry, bytes)
}

pub(crate) fn remove_runnable_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &RunnableIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    remove_runnable_bucket_entry(write, identity, entry, bytes)?;
    remove_discovery_catalog_entry(
        write,
        identity,
        bytes,
        crate::trie::CatalogFamily::RunnableIdentity,
        runnable_catalog_identity_path(&entry.run, identity)?,
        crate::trie::CatalogFamily::RunnableOrdered,
        runnable_catalog_ordered_path(identity, entry)?,
        "runnable",
    )
}

pub(crate) fn insert_runnable_bucket_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &RunnableIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let bucket_key = runnable_bucket_key(&entry.run, entry.eligible_at)?;
    let bucket_family = crate::trie::CatalogFamily::RunnableBucket;
    let bucket_path = runnable_bucket_path(&entry.run, entry.eligible_at, &bucket_key)?;
    let bucket_witness =
        crate::trie::verify_member_in_transaction(write, bucket_family, bucket_path, &bucket_key)?;
    let expected_bucket = crate::trie::digest_payload(bucket_family, &bucket_key);
    match bucket_witness {
        Some(witness) if witness == expected_bucket => {}
        Some(_) => {
            return Err(error::corruption(
                "runnable eligible bucket has an invalid authenticated payload",
            ));
        }
        None => {
            if crate::trie::put(
                write,
                bucket_family,
                bucket_path,
                &bucket_key,
                expected_bucket,
            )?
            .is_some()
            {
                return Err(error::corruption(
                    "runnable eligible bucket appeared during insertion",
                ));
            }
        }
    }
    let entry_family = crate::trie::CatalogFamily::RunnableBucketEntry;
    if crate::trie::put(
        write,
        entry_family,
        runnable_bucket_entry_path(&bucket_key, identity, entry.priority)?,
        identity,
        crate::trie::digest_payload(entry_family, bytes),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "runnable eligible bucket would overwrite an authenticated entry",
        ));
    }
    Ok(())
}

pub(crate) fn remove_runnable_bucket_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &RunnableIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let bucket_key = runnable_bucket_key(&entry.run, entry.eligible_at)?;
    let bucket_family = crate::trie::CatalogFamily::RunnableBucket;
    let bucket_path = runnable_bucket_path(&entry.run, entry.eligible_at, &bucket_key)?;
    let bucket_witness =
        crate::trie::verify_member_in_transaction(write, bucket_family, bucket_path, &bucket_key)?;
    if bucket_witness != Some(crate::trie::digest_payload(bucket_family, &bucket_key)) {
        return Err(error::corruption(
            "runnable entry is missing its authenticated eligible bucket",
        ));
    }
    let entry_family = crate::trie::CatalogFamily::RunnableBucketEntry;
    let removed = crate::trie::remove(
        write,
        entry_family,
        runnable_bucket_entry_path(&bucket_key, identity, entry.priority)?,
        identity,
    )?;
    if removed != Some(crate::trie::digest_payload(entry_family, bytes)) {
        return Err(error::corruption(
            "runnable entry disagrees with its authenticated eligible bucket",
        ));
    }
    let group = runnable_group(entry_family, &bucket_key);
    let page =
        crate::trie::page_in_transaction(write, entry_family, None, first_path_in_group(group), 1)?;
    let bucket_empty = page
        .leaves
        .first()
        .is_none_or(|leaf| leaf.path[..16] != group);
    if bucket_empty
        && crate::trie::remove(write, bucket_family, bucket_path, &bucket_key)?
            != Some(crate::trie::digest_payload(bucket_family, &bucket_key))
    {
        return Err(error::corruption(
            "empty runnable eligible bucket disappeared unexpectedly",
        ));
    }
    Ok(())
}

pub(crate) fn timer_catalog_ordered_path(
    identity: &[u8],
    entry: &TimerIndexEntry,
) -> Result<[u8; 32], PersistenceError> {
    crate::trie::ordered_path(
        crate::trie::CatalogFamily::TimerOrdered,
        &entry.fire_at.get().to_be_bytes(),
        identity,
    )
}

pub(crate) fn insert_timer_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &TimerIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let identity_family = crate::trie::CatalogFamily::TimerIdentity;
    insert_discovery_catalog_entry(
        write,
        identity,
        bytes,
        identity_family,
        crate::trie::hashed_path(identity_family, identity),
        crate::trie::CatalogFamily::TimerOrdered,
        timer_catalog_ordered_path(identity, entry)?,
        "timer",
    )
}

pub(crate) fn remove_timer_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &TimerIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let identity_family = crate::trie::CatalogFamily::TimerIdentity;
    remove_discovery_catalog_entry(
        write,
        identity,
        bytes,
        identity_family,
        crate::trie::hashed_path(identity_family, identity),
        crate::trie::CatalogFamily::TimerOrdered,
        timer_catalog_ordered_path(identity, entry)?,
        "timer",
    )
}

pub(crate) fn lease_catalog_ordered_path(
    identity: &[u8],
    entry: &LeaseIndexEntry,
) -> Result<[u8; 32], PersistenceError> {
    crate::trie::ordered_path(
        crate::trie::CatalogFamily::LeaseOrdered,
        &entry.expires_at.get().to_be_bytes(),
        identity,
    )
}

pub(crate) fn insert_lease_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &LeaseIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let identity_family = crate::trie::CatalogFamily::LeaseIdentity;
    insert_discovery_catalog_entry(
        write,
        identity,
        bytes,
        identity_family,
        crate::trie::hashed_path(identity_family, identity),
        crate::trie::CatalogFamily::LeaseOrdered,
        lease_catalog_ordered_path(identity, entry)?,
        "lease",
    )
}

pub(crate) fn remove_lease_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &LeaseIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let identity_family = crate::trie::CatalogFamily::LeaseIdentity;
    remove_discovery_catalog_entry(
        write,
        identity,
        bytes,
        identity_family,
        crate::trie::hashed_path(identity_family, identity),
        crate::trie::CatalogFamily::LeaseOrdered,
        lease_catalog_ordered_path(identity, entry)?,
        "lease",
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_discovery_catalog_entry(
    write: &redb::WriteTransaction,
    logical_key: &[u8],
    bytes: &[u8],
    identity_family: crate::trie::CatalogFamily,
    identity_path: [u8; 32],
    ordered_family: crate::trie::CatalogFamily,
    ordered_path: [u8; 32],
    label: &'static str,
) -> Result<(), PersistenceError> {
    for (family, path) in [
        (identity_family, identity_path),
        (ordered_family, ordered_path),
    ] {
        let digest = crate::trie::digest_payload(family, bytes);
        if crate::trie::put(write, family, path, logical_key, digest)?.is_some() {
            return Err(error::corruption(format!(
                "{label} authenticated catalog unexpectedly replaced a committed leaf"
            )));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn remove_discovery_catalog_entry(
    write: &redb::WriteTransaction,
    logical_key: &[u8],
    bytes: &[u8],
    identity_family: crate::trie::CatalogFamily,
    identity_path: [u8; 32],
    ordered_family: crate::trie::CatalogFamily,
    ordered_path: [u8; 32],
    label: &'static str,
) -> Result<(), PersistenceError> {
    for (family, path) in [
        (identity_family, identity_path),
        (ordered_family, ordered_path),
    ] {
        let expected = crate::trie::digest_payload(family, bytes);
        if crate::trie::remove(write, family, path, logical_key)? != Some(expected) {
            return Err(error::corruption(format!(
                "{label} row disagrees with its authenticated catalog"
            )));
        }
    }
    Ok(())
}

pub(crate) fn ensure_catalog_absent(
    write: &redb::WriteTransaction,
    family: crate::trie::CatalogFamily,
    path: [u8; 32],
    logical_key: &[u8],
    label: &'static str,
) -> Result<(), PersistenceError> {
    if crate::trie::verify_member_in_transaction(write, family, path, logical_key)?.is_some() {
        return Err(error::corruption(format!(
            "{label} authenticated catalog names a missing primary row"
        )));
    }
    Ok(())
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

pub(crate) fn record_artifact_references(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    crate::artifact::validate_artifact_catalog(write)?;
    for reference in &request.required_artifacts {
        let digest = reference.digest().to_hex();
        let key = codec::components(&[
            &digest,
            reference.artifact().as_str(),
            request.receipt.run().as_str(),
            request.receipt.command().as_str(),
        ])?;
        crate::artifact::persist_artifact_reference_occurrence(write, &key, reference)?;
        crate::artifact::persist_run_artifact_ownership(write, request.receipt.run(), reference)?;
    }
    crate::artifact::validate_artifact_catalog(write)?;
    Ok(())
}

pub(crate) fn validate_runnable_leaf_in_transaction<I, O>(
    write: &redb::WriteTransaction,
    identities: &I,
    ordered: &O,
    leaf: &crate::trie::TrieLeaf,
) -> Result<RunnableIndexEntry, PersistenceError>
where
    I: ReadableTable<&'static [u8], &'static [u8]>,
    O: ReadableTable<&'static [u8], &'static [u8]>,
{
    let bytes = identities
        .get(leaf.logical_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable authenticated catalog is dangling"))?;
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    if leaf.payload_digest != crate::trie::digest_payload(identity_family, bytes.value()) {
        return Err(error::corruption(
            "runnable identity row disagrees with its authenticated catalog",
        ));
    }
    let entry: RunnableIndexEntry = json::decode(bytes.value(), "runnable index")?;
    let expected_identity = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
    if leaf.logical_key != expected_identity
        || leaf.path != runnable_catalog_identity_path(&entry.run, &expected_identity)?
    {
        return Err(error::corruption(
            "runnable identity leaf disagrees with its checked document",
        ));
    }
    let ordered_key = runnable_order_key(&entry)?;
    let ordered_bytes = ordered
        .get(ordered_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable ordered index is incomplete"))?;
    if ordered_bytes.value() != bytes.value() {
        return Err(error::corruption(
            "runnable identity and ordered rows disagree",
        ));
    }
    let ordered_family = crate::trie::CatalogFamily::RunnableOrdered;
    let ordered_witness = crate::trie::verify_member_in_transaction(
        write,
        ordered_family,
        runnable_catalog_ordered_path(&expected_identity, &entry)?,
        &expected_identity,
    )?;
    if ordered_witness != Some(crate::trie::digest_payload(ordered_family, bytes.value())) {
        return Err(error::corruption(
            "runnable ordered row disagrees with its authenticated catalog",
        ));
    }
    Ok(entry)
}

pub(crate) fn validate_runnable_leaf<I, O>(
    read: &redb::ReadTransaction,
    identities: &I,
    ordered: &O,
    leaf: &crate::trie::TrieLeaf,
) -> Result<RunnableIndexEntry, PersistenceError>
where
    I: ReadableTable<&'static [u8], &'static [u8]>,
    O: ReadableTable<&'static [u8], &'static [u8]>,
{
    let bytes = identities
        .get(leaf.logical_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable authenticated catalog is dangling"))?;
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    if leaf.payload_digest != crate::trie::digest_payload(identity_family, bytes.value()) {
        return Err(error::corruption(
            "runnable identity row disagrees with its authenticated catalog",
        ));
    }
    let entry: RunnableIndexEntry = json::decode(bytes.value(), "runnable index")?;
    let expected_identity = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
    if leaf.logical_key != expected_identity
        || leaf.path != runnable_catalog_identity_path(&entry.run, &expected_identity)?
    {
        return Err(error::corruption(
            "runnable identity leaf disagrees with its checked document",
        ));
    }
    let ordered_key = runnable_order_key(&entry)?;
    let ordered_bytes = ordered
        .get(ordered_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable ordered index is incomplete"))?;
    if ordered_bytes.value() != bytes.value() {
        return Err(error::corruption(
            "runnable identity and ordered rows disagree",
        ));
    }
    let ordered_family = crate::trie::CatalogFamily::RunnableOrdered;
    let ordered_witness = crate::trie::verify_member(
        read,
        ordered_family,
        runnable_catalog_ordered_path(&expected_identity, &entry)?,
        &expected_identity,
    )?;
    if ordered_witness != Some(crate::trie::digest_payload(ordered_family, bytes.value())) {
        return Err(error::corruption(
            "runnable ordered row disagrees with its authenticated catalog",
        ));
    }
    Ok(entry)
}

pub(crate) fn validate_runnable_head_leaf(
    read: &redb::ReadTransaction,
    leaf: &crate::trie::TrieLeaf,
) -> Result<RunnableIndexEntry, PersistenceError> {
    let run_text = std::str::from_utf8(&leaf.logical_key)
        .map_err(|_| error::corruption("runnable run-head identity is not UTF-8"))?;
    let run = RunId::new(run_text)
        .map_err(|cause| error::corruption(format!("invalid runnable run identity: {cause}")))?;
    if leaf.path != runnable_head_path(&run) {
        return Err(error::corruption(
            "runnable run-head path disagrees with its run identity",
        ));
    }
    let bytes = read
        .open_table(RUNNABLE_RUN_HEADS)
        .map_err(error::redb)?
        .get(run.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable run-head catalog is dangling"))?
        .value()
        .to_vec();
    let family = crate::trie::CatalogFamily::RunnableRunHead;
    if leaf.payload_digest != crate::trie::digest_payload(family, &bytes) {
        return Err(error::corruption(
            "runnable run-head row disagrees with its authenticated catalog",
        ));
    }
    let entry: RunnableIndexEntry = json::decode(&bytes, "runnable run head")?;
    if entry.run != run {
        return Err(error::corruption(
            "runnable run-head key disagrees with its document",
        ));
    }
    let identity = codec::pair(run.as_str(), entry.execution.as_str())?;
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    let identity_witness = crate::trie::verify_member(
        read,
        identity_family,
        runnable_catalog_identity_path(&run, &identity)?,
        &identity,
    )?
    .ok_or_else(|| error::corruption("runnable run head names an unauthenticated entry"))?;
    let identities = read.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let ordered = read.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let active = validate_runnable_leaf(
        read,
        &identities,
        &ordered,
        &crate::trie::TrieLeaf {
            path: runnable_catalog_identity_path(&run, &identity)?,
            logical_key: identity,
            payload_digest: identity_witness,
        },
    )?;
    if active != entry {
        return Err(error::corruption(
            "runnable run head disagrees with its active entry",
        ));
    }
    drop(ordered);
    drop(identities);
    if first_runnable_for_run(read, &run)?.as_ref() != Some(&entry) {
        return Err(error::corruption(
            "runnable run head is not the canonical earliest eligible entry",
        ));
    }
    Ok(entry)
}

pub(crate) fn runnable_precedes(
    candidate: &RunnableIndexEntry,
    selected: &RunnableIndexEntry,
) -> bool {
    candidate.eligible_at < selected.eligible_at
        || (candidate.eligible_at == selected.eligible_at
            && (candidate.priority > selected.priority
                || (candidate.priority == selected.priority
                    && candidate.execution < selected.execution)))
}

#[allow(clippy::too_many_arguments)] // Closed table/key functions keep one verifier shared.
pub(crate) fn read_ordered_index<T: for<'de> Deserialize<'de> + Serialize>(
    store: &RedbStore,
    identity_definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    through: TimestampMillis,
    limit: PageSize,
    family: &'static str,
    identity_family: crate::trie::CatalogFamily,
    ordered_family: crate::trie::CatalogFamily,
    timestamp: impl Fn(&T) -> TimestampMillis,
    order_key: impl Fn(&T) -> Result<Vec<u8>, PersistenceError>,
    identity_key: impl Fn(&T) -> Result<Vec<u8>, PersistenceError>,
    identity_path: impl Fn(&[u8], &T) -> Result<[u8; 32], PersistenceError>,
    ordered_path: impl Fn(&[u8], &T) -> Result<[u8; 32], PersistenceError>,
) -> Result<(Vec<T>, [u8; 32]), PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    let identities = read.open_table(identity_definition).map_err(error::redb)?;
    let table = read.open_table(definition).map_err(error::redb)?;
    let identity_root = crate::trie::family_root(&read, identity_family)?;
    let catalog = crate::trie::page(&read, ordered_family, None, None, page_size_usize(limit)?)?;
    if catalog.leaves.is_empty()
        && (identities.len().map_err(error::redb)? != 0 || table.len().map_err(error::redb)? != 0)
    {
        return Err(error::corruption(format!(
            "{family} derived rows exist without an authenticated catalog"
        )));
    }
    let mut results = Vec::with_capacity(limit.get() as usize);
    for leaf in catalog.leaves {
        let value = identities
            .get(leaf.logical_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| {
                error::corruption(format!(
                    "{family} authenticated catalog names a missing identity row"
                ))
            })?;
        let value_bytes = value.value().to_vec();
        let entry: T = json::decode(&value_bytes, family)?;
        let expected_identity = identity_key(&entry)?;
        if expected_identity != leaf.logical_key {
            return Err(error::corruption(format!(
                "{family} identity key does not match its document"
            )));
        }
        let expected_order_key = order_key(&entry)?;
        if ordered_path(&expected_identity, &entry)? != leaf.path {
            return Err(error::corruption(format!(
                "{family} authenticated order path does not match its document"
            )));
        }
        if leaf.payload_digest != crate::trie::digest_payload(ordered_family, &value_bytes) {
            return Err(error::corruption(format!(
                "{family} ordered catalog payload disagrees with its identity row"
            )));
        }
        match table
            .get(expected_order_key.as_slice())
            .map_err(error::redb)?
        {
            Some(ordered_value) if ordered_value.value() == value_bytes.as_slice() => {}
            _ => {
                return Err(error::corruption(format!(
                    "{family} authenticated catalog is missing its ordered row"
                )));
            }
        }
        let authenticated_identity = crate::trie::verify_member(
            &read,
            identity_family,
            identity_path(&expected_identity, &entry)?,
            &expected_identity,
        )?;
        if authenticated_identity
            != Some(crate::trie::digest_payload(identity_family, &value_bytes))
        {
            return Err(error::corruption(format!(
                "{family} identity row disagrees with its authenticated catalog"
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
    Ok((results, identity_root))
}
