use std::ops::Bound;

use milkdrift_persistence::{
    BoundedDetail, IntegrityDigest, PersistenceError, RunEventEnvelope, RunSequence,
    SnapshotDocument, SnapshotId, SnapshotLoad, SnapshotStore, history_genesis_digest,
    history_link_digest,
};
use milkdrift_workspace::RunId;
use redb::ReadableTable;
use serde::{Deserialize, Serialize};

use crate::{
    RedbStore, codec, error,
    fault::FaultPoint,
    json,
    schema::{
        EVENT_HISTORY_DIGESTS, RUN_EVENTS, RUN_HEADS, RUN_HISTORY_HEADS, SNAPSHOT_LATEST, SNAPSHOTS,
    },
};

const HISTORY_CHAIN_SCHEMA_VERSION: u32 = 1;
const HISTORY_CHAIN_DOCUMENT_FAMILY: &str = "history-chain record";

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HistoryChainRecord {
    schema_version: u32,
    run: RunId,
    through_sequence: RunSequence,
    digest: IntegrityDigest,
}

fn decode_chain_record(
    bytes: &[u8],
    run: &RunId,
    sequence: RunSequence,
    label: &'static str,
) -> Result<HistoryChainRecord, PersistenceError> {
    let record: HistoryChainRecord = json::decode(bytes, HISTORY_CHAIN_DOCUMENT_FAMILY)?;
    if record.schema_version != HISTORY_CHAIN_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "history_chain_record",
            found: record.schema_version,
            supported: HISTORY_CHAIN_SCHEMA_VERSION,
        });
    }
    if record.run != *run || record.through_sequence != sequence {
        return Err(error::corruption(format!(
            "{label} key disagrees with its document"
        )));
    }
    Ok(record)
}

fn checkpoint_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    sequence: RunSequence,
) -> Result<HistoryChainRecord, PersistenceError> {
    let key = codec::run_sequence(run.as_str(), sequence)?;
    let table = write
        .open_table(EVENT_HISTORY_DIGESTS)
        .map_err(error::redb)?;
    let bytes = table
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("history-chain checkpoint is missing"))?
        .value()
        .to_vec();
    decode_chain_record(&bytes, run, sequence, "history-chain checkpoint")
}

fn checkpoint(
    read: &redb::ReadTransaction,
    run: &RunId,
    sequence: RunSequence,
) -> Result<HistoryChainRecord, PersistenceError> {
    let key = codec::run_sequence(run.as_str(), sequence)?;
    let table = read
        .open_table(EVENT_HISTORY_DIGESTS)
        .map_err(error::redb)?;
    let bytes = table
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("history-chain checkpoint is missing"))?
        .value()
        .to_vec();
    decode_chain_record(&bytes, run, sequence, "history-chain checkpoint")
}

pub(crate) fn append_history_checkpoint(
    write: &redb::WriteTransaction,
    event: &RunEventEnvelope,
) -> Result<(), PersistenceError> {
    let run = event.run_id();
    let previous_head_bytes = {
        let heads = write.open_table(RUN_HISTORY_HEADS).map_err(error::redb)?;
        heads
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec())
    };
    let previous = match previous_head_bytes.as_deref() {
        None if event.sequence() == RunSequence::FIRST => history_genesis_digest(run)?,
        None => {
            return Err(error::corruption(
                "noninitial event is missing its history-chain head",
            ));
        }
        Some(bytes) => {
            let prior_sequence =
                RunSequence::new(event.sequence().get().checked_sub(1).ok_or_else(|| {
                    error::corruption("initial event unexpectedly has a prior history-chain head")
                })?);
            let head = decode_chain_record(bytes, run, prior_sequence, "history-chain head")?;
            let prior = checkpoint_in_transaction(write, run, prior_sequence)?;
            if prior.digest != head.digest {
                return Err(error::corruption(
                    "history-chain head disagrees with its prior checkpoint",
                ));
            }
            head.digest
        }
    };
    let digest = history_link_digest(run, event.sequence(), &previous, event.checksum())?;
    let record = HistoryChainRecord {
        schema_version: HISTORY_CHAIN_SCHEMA_VERSION,
        run: run.clone(),
        through_sequence: event.sequence(),
        digest,
    };
    let document = json::encode(&record, HISTORY_CHAIN_DOCUMENT_FAMILY)?;
    let key = codec::run_sequence(run.as_str(), event.sequence())?;
    {
        let mut checkpoints = write
            .open_table(EVENT_HISTORY_DIGESTS)
            .map_err(error::redb)?;
        if checkpoints
            .insert(key.as_slice(), document.as_slice())
            .map_err(error::redb)?
            .is_some()
        {
            return Err(error::corruption(
                "event append overwrote a history-chain checkpoint",
            ));
        }
    }
    let mut heads = write.open_table(RUN_HISTORY_HEADS).map_err(error::redb)?;
    let replaced = heads
        .insert(run.as_str(), document.as_slice())
        .map_err(error::redb)?;
    if replaced.as_ref().map(|bytes| bytes.value()) != previous_head_bytes.as_deref() {
        return Err(error::corruption(
            "history-chain head changed outside the event transaction",
        ));
    }
    Ok(())
}

pub(crate) fn validate_history_link(
    read: &redb::ReadTransaction,
    event: &RunEventEnvelope,
) -> Result<IntegrityDigest, PersistenceError> {
    let previous = if event.sequence() == RunSequence::FIRST {
        history_genesis_digest(event.run_id())?
    } else {
        let previous_sequence =
            RunSequence::new(event.sequence().get().checked_sub(1).ok_or_else(|| {
                error::corruption("noninitial history link has no previous sequence")
            })?);
        checkpoint(read, event.run_id(), previous_sequence)?.digest
    };
    let expected = history_link_digest(
        event.run_id(),
        event.sequence(),
        &previous,
        event.checksum(),
    )?;
    let stored = checkpoint(read, event.run_id(), event.sequence())?;
    if stored.digest != expected {
        return Err(error::corruption(
            "history-chain checkpoint does not match its event and prior link",
        ));
    }
    Ok(expected)
}

pub(crate) fn validate_history_link_in_transaction(
    write: &redb::WriteTransaction,
    event: &RunEventEnvelope,
) -> Result<IntegrityDigest, PersistenceError> {
    let previous = if event.sequence() == RunSequence::FIRST {
        history_genesis_digest(event.run_id())?
    } else {
        let previous_sequence =
            RunSequence::new(event.sequence().get().checked_sub(1).ok_or_else(|| {
                error::corruption("noninitial history link has no previous sequence")
            })?);
        checkpoint_in_transaction(write, event.run_id(), previous_sequence)?.digest
    };
    let expected = history_link_digest(
        event.run_id(),
        event.sequence(),
        &previous,
        event.checksum(),
    )?;
    let stored = checkpoint_in_transaction(write, event.run_id(), event.sequence())?;
    if stored.digest != expected {
        return Err(error::corruption(
            "history-chain checkpoint does not match its event and prior link",
        ));
    }
    Ok(expected)
}

pub(crate) fn validate_history_head(
    read: &redb::ReadTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<(), PersistenceError> {
    let stored = {
        let heads = read.open_table(RUN_HISTORY_HEADS).map_err(error::redb)?;
        heads
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec())
    };
    if head == RunSequence::ZERO {
        if stored.is_some() {
            return Err(error::corruption("empty run retains a history-chain head"));
        }
        return Ok(());
    }
    let bytes = stored.ok_or_else(|| error::corruption("run is missing its history-chain head"))?;
    let chain_head = decode_chain_record(&bytes, run, head, "history-chain head")?;
    let checkpoint = checkpoint(read, run, head)?;
    if chain_head.digest != checkpoint.digest {
        return Err(error::corruption(
            "history-chain head disagrees with its final checkpoint",
        ));
    }
    Ok(())
}

pub(crate) fn validate_history_head_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<(), PersistenceError> {
    let stored = {
        let heads = write.open_table(RUN_HISTORY_HEADS).map_err(error::redb)?;
        heads
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec())
    };
    if head == RunSequence::ZERO {
        if stored.is_some() {
            return Err(error::corruption("empty run retains a history-chain head"));
        }
        return Ok(());
    }
    let bytes = stored.ok_or_else(|| error::corruption("run is missing its history-chain head"))?;
    let chain_head = decode_chain_record(&bytes, run, head, "history-chain head")?;
    let checkpoint = checkpoint_in_transaction(write, run, head)?;
    if chain_head.digest != checkpoint.digest {
        return Err(error::corruption(
            "history-chain head disagrees with its final checkpoint",
        ));
    }
    Ok(())
}

fn history_digest_write(
    write: &redb::WriteTransaction,
    run: &RunId,
    through: RunSequence,
) -> Result<IntegrityDigest, PersistenceError> {
    if through == RunSequence::ZERO {
        return Err(PersistenceError::InvalidDocument(
            "snapshot history must cover at least one event".to_owned(),
        ));
    }
    Ok(checkpoint_in_transaction(write, run, through)?.digest)
}

pub(crate) fn history_digest_read(
    read: &redb::ReadTransaction,
    run: &RunId,
    through: RunSequence,
) -> Result<IntegrityDigest, PersistenceError> {
    if through == RunSequence::ZERO {
        return Err(PersistenceError::InvalidDocument(
            "snapshot history must cover at least one event".to_owned(),
        ));
    }
    Ok(checkpoint(read, run, through)?.digest)
}

impl SnapshotStore for RedbStore {
    fn history_digest(
        &self,
        run: &RunId,
        through: RunSequence,
    ) -> Result<IntegrityDigest, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let head = {
            let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
            let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
            crate::journal::validated_run_head(&heads, &events, run)?
        };
        crate::journal::validate_run_history_membership(&read, run, head)?;
        if through == RunSequence::ZERO || through > head {
            return Err(PersistenceError::InvalidDocument(format!(
                "history digest sequence {through} is outside run {run} head {head}"
            )));
        }
        history_digest_read(&read, run, through)
    }

    #[tracing::instrument(
        name = "milkdrift.redb_store.put_snapshot",
        skip_all,
        fields(
            run = %snapshot.run(),
            snapshot = %snapshot.snapshot(),
            covered_sequence = snapshot.covered_sequence().get()
        )
    )]
    fn put_snapshot(&self, snapshot: &SnapshotDocument) -> Result<(), PersistenceError> {
        let document = snapshot.to_canonical_json()?;
        if &SnapshotDocument::from_json(&document)? != snapshot {
            return Err(PersistenceError::InvalidDocument(
                "snapshot did not round-trip through its canonical schema".to_owned(),
            ));
        }
        let key = codec::pair(snapshot.run().as_str(), snapshot.snapshot().as_str())?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let head = crate::journal::validated_run_head_in_transaction(&write, snapshot.run())?;
        if crate::journal::validate_run_history_membership_in_transaction(
            &write,
            snapshot.run(),
            head,
        )?
        .is_none()
        {
            return Err(PersistenceError::NotFound {
                entity: "run",
                identity: snapshot.run().to_string(),
            });
        }
        if snapshot.covered_sequence() > head {
            return Err(PersistenceError::InvalidDocument(format!(
                "snapshot covers sequence {} beyond journal head {head}",
                snapshot.covered_sequence()
            )));
        }
        if history_digest_write(&write, snapshot.run(), snapshot.covered_sequence())?
            != *snapshot.history_digest()
        {
            return Err(PersistenceError::InvalidDocument(
                "snapshot history digest does not match authoritative events".to_owned(),
            ));
        }
        let previous_latest = validated_latest_pointer(&write, snapshot.run())?;
        {
            let mut snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
            if let Some(existing) = snapshots.get(key.as_slice()).map_err(error::redb)? {
                if existing.value() != document.as_slice() {
                    return Err(PersistenceError::ImmutableConflict {
                        entity: "snapshot",
                        identity: snapshot.snapshot().to_string(),
                    });
                }
            } else {
                snapshots
                    .insert(key.as_slice(), document.as_slice())
                    .map_err(error::redb)?;
            }
        }
        let should_advance = previous_latest.as_ref().is_none_or(|latest| {
            latest.covered_sequence() < snapshot.covered_sequence()
                || latest.covered_sequence() == snapshot.covered_sequence()
                    && latest.snapshot().as_str() <= snapshot.snapshot().as_str()
        });
        if should_advance {
            write
                .open_table(SNAPSHOT_LATEST)
                .map_err(error::redb)?
                .insert(snapshot.run().as_str(), snapshot.snapshot().as_str())
                .map_err(error::redb)?;
        }
        self.faults.check(FaultPoint::BeforeSnapshotCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterSnapshotCommit)
    }

    fn latest_snapshot(&self, run: &RunId) -> Result<SnapshotLoad, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let head = {
            let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
            let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
            crate::journal::validated_run_head(&heads, &events, run)?
        };
        crate::journal::validate_run_history_membership(&read, run, head)?;
        let snapshot_id = read
            .open_table(SNAPSHOT_LATEST)
            .map_err(error::redb)?
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned());
        let Some(snapshot_id) = snapshot_id else {
            if snapshots_exist_for_run_read(&read, run)? {
                return rejected(
                    None,
                    format!("run {run} retains snapshots without a latest-snapshot pointer"),
                );
            }
            return Ok(SnapshotLoad::Absent);
        };
        let snapshot_id = match SnapshotId::new(snapshot_id) {
            Ok(snapshot_id) => snapshot_id,
            Err(cause) => {
                return rejected(None, format!("invalid latest snapshot identity: {cause}"));
            }
        };
        let key = codec::pair(run.as_str(), snapshot_id.as_str())?;
        let snapshots = read.open_table(SNAPSHOTS).map_err(error::redb)?;
        let Some(bytes) = snapshots.get(key.as_slice()).map_err(error::redb)? else {
            return rejected(
                Some(snapshot_id),
                "latest snapshot pointer is dangling".to_owned(),
            );
        };
        let document = bytes.value().to_vec();
        drop(bytes);
        drop(snapshots);
        let snapshot = match decode_stored_snapshot(&document) {
            Ok(snapshot) => snapshot,
            Err(cause) => return rejected(Some(snapshot_id), cause.to_string()),
        };
        if snapshot.run() != run || snapshot.snapshot() != &snapshot_id {
            return rejected(
                Some(snapshot_id),
                "snapshot key does not match its document".to_owned(),
            );
        }
        if snapshot.covered_sequence() > head {
            return rejected(
                Some(snapshot_id),
                "snapshot covers history beyond the journal head".to_owned(),
            );
        }
        let digest = match history_digest_read(&read, run, snapshot.covered_sequence()) {
            Ok(digest) => digest,
            Err(cause) => return rejected(Some(snapshot_id), cause.to_string()),
        };
        if digest != *snapshot.history_digest() {
            return rejected(
                Some(snapshot_id),
                "snapshot history digest no longer matches authoritative events".to_owned(),
            );
        }
        Ok(SnapshotLoad::Verified(snapshot))
    }

    fn discard_snapshot(&self, run: &RunId, snapshot: &SnapshotId) -> Result<(), PersistenceError> {
        let key = codec::pair(run.as_str(), snapshot.as_str())?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let head = crate::journal::validated_run_head_in_transaction(&write, run)?;
        crate::journal::validate_run_history_membership_in_transaction(&write, run, head)?;
        let previous_latest = latest_pointer_id_for_discard(&write, run)?;
        let target = {
            let snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
            snapshots
                .get(key.as_slice())
                .map_err(error::redb)?
                .map(|bytes| bytes.value().to_vec())
        };
        if let Some(document) = target.as_deref() {
            if let Ok(target) = decode_stored_snapshot(document)
                && (target.run() != run || target.snapshot() != snapshot)
            {
                return Err(error::corruption(
                    "discarded snapshot key disagrees with its document",
                ));
            }
            write
                .open_table(SNAPSHOTS)
                .map_err(error::redb)?
                .remove(key.as_slice())
                .map_err(error::redb)?;
        }
        if previous_latest
            .as_ref()
            .is_some_and(|latest| latest == snapshot)
        {
            let replacement = newest_snapshot_for_run(&write, run)?;
            let mut latest = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
            if let Some(replacement) = replacement {
                latest
                    .insert(run.as_str(), replacement.snapshot().as_str())
                    .map_err(error::redb)?;
            } else {
                latest.remove(run.as_str()).map_err(error::redb)?;
            }
        }
        self.faults.check(FaultPoint::BeforeSnapshotDiscardCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterSnapshotDiscardCommit)
    }
}

fn latest_pointer_id_for_discard(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<Option<SnapshotId>, PersistenceError> {
    let snapshot_id = write
        .open_table(SNAPSHOT_LATEST)
        .map_err(error::redb)?
        .get(run.as_str())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    let Some(snapshot_id) = snapshot_id else {
        if snapshots_exist_for_run(write, run)? {
            return Err(error::corruption(format!(
                "run {run} retains snapshots without a latest-snapshot pointer"
            )));
        }
        return Ok(None);
    };
    let snapshot_id = SnapshotId::new(snapshot_id)
        .map_err(|cause| error::corruption(format!("invalid latest snapshot identity: {cause}")))?;
    let key = codec::pair(run.as_str(), snapshot_id.as_str())?;
    if write
        .open_table(SNAPSHOTS)
        .map_err(error::redb)?
        .get(key.as_slice())
        .map_err(error::redb)?
        .is_none()
    {
        return Err(error::corruption("latest snapshot pointer is dangling"));
    }
    Ok(Some(snapshot_id))
}

fn validated_latest_pointer(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<Option<SnapshotDocument>, PersistenceError> {
    let snapshot_id = write
        .open_table(SNAPSHOT_LATEST)
        .map_err(error::redb)?
        .get(run.as_str())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    let Some(snapshot_id) = snapshot_id else {
        if snapshots_exist_for_run(write, run)? {
            return Err(error::corruption(format!(
                "run {run} retains snapshots without a latest-snapshot pointer"
            )));
        }
        return Ok(None);
    };
    let snapshot_id = SnapshotId::new(snapshot_id)
        .map_err(|cause| error::corruption(format!("invalid latest snapshot identity: {cause}")))?;
    let key = codec::pair(run.as_str(), snapshot_id.as_str())?;
    let snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
    let bytes = snapshots
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("latest snapshot pointer is dangling"))?
        .value()
        .to_vec();
    let snapshot = decode_stored_snapshot(&bytes)?;
    if snapshot.run() != run || snapshot.snapshot() != &snapshot_id {
        return Err(error::corruption(
            "latest snapshot key does not match its document",
        ));
    }
    Ok(Some(snapshot))
}

fn snapshot_range(run: &RunId) -> Result<(Vec<u8>, Vec<u8>), PersistenceError> {
    let prefix = codec::component(run.as_str())?;
    let end = codec::prefix_end(prefix.clone())
        .ok_or_else(|| error::corruption("snapshot run prefix has no range end"))?;
    Ok((prefix, end))
}

fn snapshots_exist_for_run(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<bool, PersistenceError> {
    let (prefix, end) = snapshot_range(run)?;
    let snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
    Ok(snapshots
        .range::<&[u8]>((
            Bound::Included(prefix.as_slice()),
            Bound::Excluded(end.as_slice()),
        ))
        .map_err(error::redb)?
        .next()
        .transpose()
        .map_err(error::redb)?
        .is_some())
}

fn snapshots_exist_for_run_read(
    read: &redb::ReadTransaction,
    run: &RunId,
) -> Result<bool, PersistenceError> {
    let (prefix, end) = snapshot_range(run)?;
    let snapshots = read.open_table(SNAPSHOTS).map_err(error::redb)?;
    Ok(snapshots
        .range::<&[u8]>((
            Bound::Included(prefix.as_slice()),
            Bound::Excluded(end.as_slice()),
        ))
        .map_err(error::redb)?
        .next()
        .transpose()
        .map_err(error::redb)?
        .is_some())
}

fn newest_snapshot_for_run(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<Option<SnapshotDocument>, PersistenceError> {
    let (prefix, end) = snapshot_range(run)?;
    let snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
    let mut newest: Option<SnapshotDocument> = None;
    for row in snapshots
        .range::<&[u8]>((
            Bound::Included(prefix.as_slice()),
            Bound::Excluded(end.as_slice()),
        ))
        .map_err(error::redb)?
    {
        let (key, value) = row.map_err(error::redb)?;
        let candidate = decode_stored_snapshot(value.value())?;
        let expected_key = codec::pair(run.as_str(), candidate.snapshot().as_str())?;
        if candidate.run() != run || key.value() != expected_key.as_slice() {
            return Err(error::corruption(
                "snapshot key does not match its document",
            ));
        }
        if newest.as_ref().is_none_or(|current| {
            current.covered_sequence() < candidate.covered_sequence()
                || current.covered_sequence() == candidate.covered_sequence()
                    && current.snapshot().as_str() < candidate.snapshot().as_str()
        }) {
            newest = Some(candidate);
        }
    }
    Ok(newest)
}

fn decode_stored_snapshot(bytes: &[u8]) -> Result<SnapshotDocument, PersistenceError> {
    SnapshotDocument::from_json(bytes).map_err(|cause| match cause {
        PersistenceError::UnsupportedVersion { .. } => cause,
        other => {
            PersistenceError::Corruption(format!("stored snapshot failed verification: {other}"))
        }
    })
}

fn rejected(
    snapshot: Option<SnapshotId>,
    reason: String,
) -> Result<SnapshotLoad, PersistenceError> {
    let mut reason: String = reason
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if reason.len() > milkdrift_persistence::MAX_DETAIL_BYTES {
        let mut boundary = milkdrift_persistence::MAX_DETAIL_BYTES;
        while !reason.is_char_boundary(boundary) {
            boundary -= 1;
        }
        reason.truncate(boundary);
    }
    Ok(SnapshotLoad::Rejected {
        snapshot,
        reason: BoundedDetail::new(reason)?,
    })
}
