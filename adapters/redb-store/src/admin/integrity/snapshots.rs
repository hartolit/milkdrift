use super::super::{
    PersistenceError, RUN_EVENTS, RUN_HEADS, RunId, SNAPSHOT_LATEST, SNAPSHOTS, SnapshotDocument,
    SnapshotId, codec, error,
};
use super::{ScanContext, phase};

pub(super) fn scan(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let snapshots = read.open_table(SNAPSHOTS).map_err(error::redb)?;
    let snapshot_latest = read.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
    context.binary_bytes(
        phase::SNAPSHOTS,
        &snapshots,
        "snapshot_indexes",
        |key, bytes| {
            let components = codec::decode_components(key, 2)?;
            let snapshot = SnapshotDocument::from_json(bytes)?;
            if snapshot.run().as_str() != components[0]
                || snapshot.snapshot().as_str() != components[1]
            {
                return Err(error::corruption(
                    "snapshot key does not match its checked document",
                ));
            }
            let run = snapshot.run();
            let head = crate::journal::validated_run_head(&heads, &events, run)?;
            crate::journal::validate_run_history_membership(read, run, head)?;
            if snapshot.covered_sequence() > head
                || crate::snapshot::history_digest_read(read, run, snapshot.covered_sequence())?
                    != *snapshot.history_digest()
            {
                return Err(error::corruption(
                    "snapshot does not match authoritative history",
                ));
            }
            let latest_id = snapshot_latest
                .get(run.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("snapshot has no latest pointer"))?;
            let latest_key = codec::pair(run.as_str(), latest_id.value())?;
            let latest = snapshots
                .get(latest_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("latest snapshot pointer is dangling"))?;
            let latest = SnapshotDocument::from_json(latest.value())?;
            if snapshot.covered_sequence() > latest.covered_sequence()
                || (snapshot.covered_sequence() == latest.covered_sequence()
                    && snapshot.snapshot().as_str() > latest.snapshot().as_str())
            {
                return Err(error::corruption(
                    "latest snapshot pointer does not name the newest snapshot",
                ));
            }
            Ok(())
        },
    )?;
    context.string_string(
        phase::SNAPSHOT_LATEST,
        &snapshot_latest,
        "snapshot_indexes",
        |run, snapshot| {
            let run = RunId::new(run).map_err(|cause| {
                error::corruption(format!("invalid latest-snapshot run: {cause}"))
            })?;
            let snapshot = SnapshotId::new(snapshot).map_err(|cause| {
                error::corruption(format!("invalid latest-snapshot identity: {cause}"))
            })?;
            let key = codec::pair(run.as_str(), snapshot.as_str())?;
            let bytes = snapshots
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("latest snapshot pointer is dangling"))?;
            let document = SnapshotDocument::from_json(bytes.value())?;
            if document.run() != &run || document.snapshot() != &snapshot {
                return Err(error::corruption(
                    "latest snapshot pointer disagrees with its document",
                ));
            }
            Ok(())
        },
    )
}
