use milkdrift_persistence::{
    BoundedDetail, IntegrityDigest, PersistenceError, RunEventEnvelope, RunSequence,
    SnapshotDocument, SnapshotId, SnapshotLoad, SnapshotStore,
};
use milkdrift_workspace::RunId;
use redb::ReadableTable;

use crate::{
    RedbStore, codec, error,
    fault::FaultPoint,
    schema::{EVENT_CHECKSUMS, RUN_EVENTS, RUN_HEADS, SNAPSHOT_LATEST, SNAPSHOTS},
};

const HISTORY_DIGEST_DOMAIN: &[u8] = b"milkdrift.run-history.v1\0";

impl SnapshotStore for RedbStore {
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
        let verified = SnapshotDocument::from_json(&document)?;
        if &verified != snapshot {
            return Err(PersistenceError::InvalidDocument(
                "snapshot did not round-trip through its canonical schema".to_owned(),
            ));
        }
        let key = codec::pair(snapshot.run().as_str(), snapshot.snapshot().as_str())?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let head = {
            let heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
            heads
                .get(snapshot.run().as_str())
                .map_err(error::redb)?
                .map_or(RunSequence::ZERO, |value| RunSequence::new(value.value()))
        };
        if snapshot.covered_sequence() > head {
            return Err(PersistenceError::InvalidDocument(format!(
                "snapshot covers sequence {} beyond journal head {head}",
                snapshot.covered_sequence()
            )));
        }
        let actual_digest =
            history_digest_write(&write, snapshot.run(), snapshot.covered_sequence())?;
        if &actual_digest != snapshot.history_digest() {
            return Err(PersistenceError::InvalidDocument(
                "snapshot history digest does not match authoritative events".to_owned(),
            ));
        }

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

        let should_advance = {
            let latest = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
            match latest.get(snapshot.run().as_str()).map_err(error::redb)? {
                None => true,
                Some(latest_id) => {
                    let latest_key = codec::pair(snapshot.run().as_str(), latest_id.value())?;
                    let snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
                    let bytes = snapshots
                        .get(latest_key.as_slice())
                        .map_err(error::redb)?
                        .ok_or_else(|| error::corruption("latest snapshot pointer is dangling"))?;
                    let latest = decode_stored_snapshot(bytes.value())?;
                    latest.covered_sequence() <= snapshot.covered_sequence()
                }
            }
        };
        if should_advance {
            let mut latest = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
            latest
                .insert(snapshot.run().as_str(), snapshot.snapshot().as_str())
                .map_err(error::redb)?;
        }
        self.faults.check(FaultPoint::BeforeSnapshotCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterSnapshotCommit)
    }

    fn latest_snapshot(&self, run: &RunId) -> Result<SnapshotLoad, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let latest = read.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
        let Some(snapshot_id) = latest.get(run.as_str()).map_err(error::redb)? else {
            return Ok(SnapshotLoad::Absent);
        };
        let snapshot_id = SnapshotId::new(snapshot_id.value()).map_err(|cause| {
            error::corruption(format!("invalid latest snapshot identity: {cause}"))
        })?;
        let key = codec::pair(run.as_str(), snapshot_id.as_str())?;
        let snapshots = read.open_table(SNAPSHOTS).map_err(error::redb)?;
        let Some(bytes) = snapshots.get(key.as_slice()).map_err(error::redb)? else {
            return rejected(
                Some(snapshot_id),
                "latest snapshot pointer is dangling".to_owned(),
            );
        };
        let snapshot = match decode_stored_snapshot(bytes.value()) {
            Ok(snapshot) => snapshot,
            Err(cause @ PersistenceError::UnsupportedVersion { .. }) => return Err(cause),
            Err(cause) => return rejected(Some(snapshot_id), cause.to_string()),
        };
        if snapshot.run() != run || snapshot.snapshot() != &snapshot_id {
            return rejected(
                Some(snapshot_id),
                "snapshot key does not match its document".to_owned(),
            );
        }
        let head = {
            let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
            heads
                .get(run.as_str())
                .map_err(error::redb)?
                .map_or(RunSequence::ZERO, |value| RunSequence::new(value.value()))
        };
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
        if &digest != snapshot.history_digest() {
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
        {
            let mut snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
            let _removed = snapshots.remove(key.as_slice()).map_err(error::redb)?;
        }
        {
            let mut latest = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
            if latest
                .get(run.as_str())
                .map_err(error::redb)?
                .is_some_and(|value| value.value() == snapshot.as_str())
            {
                let _removed = latest.remove(run.as_str()).map_err(error::redb)?;
            }
        }
        self.faults.check(FaultPoint::BeforeSnapshotDiscardCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterSnapshotDiscardCommit)
    }
}

fn history_digest_write(
    transaction: &redb::WriteTransaction,
    run: &RunId,
    through: RunSequence,
) -> Result<IntegrityDigest, PersistenceError> {
    let events = transaction.open_table(RUN_EVENTS).map_err(error::redb)?;
    let checksums = transaction
        .open_table(EVENT_CHECKSUMS)
        .map_err(error::redb)?;
    calculate_history(run, through, |sequence| {
        read_event(&events, &checksums, run, sequence)
    })
}

fn history_digest_read(
    transaction: &redb::ReadTransaction,
    run: &RunId,
    through: RunSequence,
) -> Result<IntegrityDigest, PersistenceError> {
    let events = transaction.open_table(RUN_EVENTS).map_err(error::redb)?;
    let checksums = transaction
        .open_table(EVENT_CHECKSUMS)
        .map_err(error::redb)?;
    calculate_history(run, through, |sequence| {
        read_event(&events, &checksums, run, sequence)
    })
}

fn read_event<E, C>(
    events: &E,
    checksums: &C,
    run: &RunId,
    sequence: RunSequence,
) -> Result<RunEventEnvelope, PersistenceError>
where
    E: ReadableTable<&'static [u8], &'static [u8]>,
    C: ReadableTable<&'static str, &'static str>,
{
    let key = codec::run_sequence(run.as_str(), sequence)?;
    let bytes = events
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| {
            error::corruption(format!(
                "run {run} is missing event sequence {sequence} for snapshot verification"
            ))
        })?;
    let event = crate::journal::decode_stored_event(bytes.value())?;
    if event.run_id() != run || event.sequence() != sequence {
        return Err(error::corruption(
            "snapshot history event key does not match its envelope",
        ));
    }
    let checksum = checksums
        .get(event.event_id().as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("snapshot history event checksum is missing"))?;
    if checksum.value() != event.checksum().as_str() {
        return Err(error::corruption(
            "snapshot history checksum index mismatch",
        ));
    }
    Ok(event)
}

fn calculate_history(
    run: &RunId,
    through: RunSequence,
    mut event_at: impl FnMut(RunSequence) -> Result<RunEventEnvelope, PersistenceError>,
) -> Result<IntegrityDigest, PersistenceError> {
    if through == RunSequence::ZERO {
        return Err(PersistenceError::InvalidDocument(
            "snapshot history must cover at least one event".to_owned(),
        ));
    }
    let run_bytes = run.as_str().as_bytes();
    let run_length = u32::try_from(run_bytes.len()).map_err(|_| PersistenceError::Bounds {
        location: "history.run_id",
        reason: "run identity length does not fit u32".to_owned(),
    })?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(HISTORY_DIGEST_DOMAIN);
    hasher.update(&run_length.to_be_bytes());
    hasher.update(run_bytes);
    let mut sequence = RunSequence::FIRST;
    loop {
        let event = event_at(sequence)?;
        let checksum = event.checksum().as_str().as_bytes();
        let checksum_length =
            u32::try_from(checksum.len()).map_err(|_| PersistenceError::Bounds {
                location: "history.checksum",
                reason: "checksum length does not fit u32".to_owned(),
            })?;
        hasher.update(&sequence.get().to_be_bytes());
        hasher.update(&checksum_length.to_be_bytes());
        hasher.update(checksum);
        if sequence == through {
            break;
        }
        sequence = sequence.next()?;
    }
    IntegrityDigest::new(format!("b3_{}", hasher.finalize()))
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
    let reason = BoundedDetail::new(reason)?;
    Ok(SnapshotLoad::Rejected { snapshot, reason })
}
