use milkdrift_persistence::{
    BoundedDetail, IntegrityDigest, PersistenceError, RunEventEnvelope, RunSequence,
    SnapshotDocument, SnapshotId, SnapshotLoad, SnapshotStore,
};
use milkdrift_workspace::RunId;
use redb::{ReadableTable, ReadableTableMetadata};
use serde::{Deserialize, Serialize};

use crate::{
    RedbStore, codec, error, json,
    fault::FaultPoint,
    schema::{
        EVENT_HISTORY_DIGESTS, RUN_EVENTS, RUN_HEADS, RUN_HISTORY_ACCUMULATORS, SNAPSHOT_LATEST,
        SNAPSHOTS,
    },
};

const HISTORY_DIGEST_DOMAIN: &[u8] = b"milkdrift.run-history.v1\0";
const HISTORY_ACCUMULATOR_SCHEMA_VERSION: u32 = 1;
const HISTORY_CHECKPOINT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HistoryAccumulator {
    schema_version: u32,
    run: RunId,
    through_sequence: RunSequence,
    completed_chunks: u64,
    chaining_values: Vec<[u8; 32]>,
    pending: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HistoryCheckpoint {
    schema_version: u32,
    run: RunId,
    sequence: RunSequence,
    digest: IntegrityDigest,
}

fn snapshot_identity_path(key: &[u8]) -> [u8; 32] {
    crate::trie::hashed_path(crate::trie::CatalogFamily::SnapshotIdentity, key)
}

fn snapshot_ordered_path(
    snapshot: &SnapshotDocument,
    key: &[u8],
) -> Result<[u8; 32], PersistenceError> {
    let family = crate::trie::CatalogFamily::SnapshotOrdered;
    let run_hash = crate::trie::hashed_path(family, snapshot.run().as_str().as_bytes());
    let mut prefix = [0_u8; 24];
    prefix[..16].copy_from_slice(&run_hash[..16]);
    prefix[16..].copy_from_slice(&snapshot.covered_sequence().get().to_be_bytes());
    crate::trie::ordered_path(family, &prefix, key)
}

fn snapshot_latest_path(run: &RunId) -> [u8; 32] {
    crate::trie::hashed_path(
        crate::trie::CatalogFamily::SnapshotLatest,
        run.as_str().as_bytes(),
    )
}

pub(crate) fn validate_catalog_leaf(
    read: &redb::ReadTransaction,
    family: crate::trie::CatalogFamily,
    leaf: &crate::trie::TrieLeaf,
) -> Result<(), PersistenceError> {
    match family {
        crate::trie::CatalogFamily::HistoryAccumulator => {
            let run = std::str::from_utf8(&leaf.logical_key)
                .map_err(|_| error::corruption("history accumulator identity is not UTF-8"))?;
            let bytes = read
                .open_table(RUN_HISTORY_ACCUMULATORS)
                .map_err(error::redb)?
                .get(run)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("history accumulator catalog is dangling"))?
                .value()
                .to_vec();
            let accumulator: HistoryAccumulator = json::decode(&bytes, "history accumulator")?;
            accumulator.validate()?;
            if accumulator.run.as_str() != run
                || leaf.path != crate::trie::hashed_path(family, &leaf.logical_key)
                || leaf.payload_digest != crate::trie::digest_payload(family, &bytes)
            {
                return Err(error::corruption(
                    "history accumulator leaf disagrees with its document",
                ));
            }
            Ok(())
        }
        crate::trie::CatalogFamily::EventHistoryCheckpoint => {
            let bytes = read
                .open_table(EVENT_HISTORY_DIGESTS)
                .map_err(error::redb)?
                .get(leaf.logical_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("history checkpoint catalog is dangling"))?
                .value()
                .to_vec();
            let checkpoint: HistoryCheckpoint =
                json::decode(&bytes, "event history checkpoint")?;
            let expected_key = codec::run_sequence(
                checkpoint.run.as_str(),
                checkpoint.sequence,
            )?;
            if expected_key != leaf.logical_key
                || leaf.path
                    != history_checkpoint_path(
                        &checkpoint.run,
                        checkpoint.sequence,
                        &expected_key,
                    )?
                || leaf.payload_digest != crate::trie::digest_payload(family, &bytes)
            {
                return Err(error::corruption(
                    "history checkpoint leaf disagrees with its document",
                ));
            }
            Ok(())
        }
        crate::trie::CatalogFamily::SnapshotIdentity
        | crate::trie::CatalogFamily::SnapshotOrdered => {
            let document = read
                .open_table(SNAPSHOTS)
                .map_err(error::redb)?
                .get(leaf.logical_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("snapshot catalog leaf is dangling"))?
                .value()
                .to_vec();
            let snapshot = decode_stored_snapshot(&document)?;
            let expected_key = codec::pair(snapshot.run().as_str(), snapshot.snapshot().as_str())?;
            let expected_path = if family == crate::trie::CatalogFamily::SnapshotIdentity {
                snapshot_identity_path(&expected_key)
            } else {
                snapshot_ordered_path(&snapshot, &expected_key)?
            };
            if expected_key != leaf.logical_key
                || expected_path != leaf.path
                || leaf.payload_digest != crate::trie::digest_payload(family, &document)
            {
                return Err(error::corruption(
                    "snapshot catalog leaf disagrees with its document",
                ));
            }
            Ok(())
        }
        crate::trie::CatalogFamily::SnapshotLatest => {
            let run_text = std::str::from_utf8(&leaf.logical_key)
                .map_err(|_| error::corruption("snapshot latest identity is not UTF-8"))?;
            let run = RunId::new(run_text).map_err(|cause| {
                error::corruption(format!("invalid snapshot latest run identity: {cause}"))
            })?;
            let snapshot = read
                .open_table(SNAPSHOT_LATEST)
                .map_err(error::redb)?
                .get(run.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("snapshot latest catalog is dangling"))?
                .value()
                .to_owned();
            if leaf.path != snapshot_latest_path(&run)
                || leaf.payload_digest
                    != crate::trie::digest_payload(family, snapshot.as_bytes())
            {
                return Err(error::corruption(
                    "snapshot latest leaf disagrees with its pointer",
                ));
            }
            Ok(())
        }
        _ => Err(error::corruption(
            "snapshot catalog validator received another family's leaf",
        )),
    }
}

fn snapshot_group_predecessor(run: &RunId) -> Option<[u8; 32]> {
    let family = crate::trie::CatalogFamily::SnapshotOrdered;
    let run_hash = crate::trie::hashed_path(family, run.as_str().as_bytes());
    let mut first = [0_u8; 32];
    first[..16].copy_from_slice(&run_hash[..16]);
    for index in (0..first.len()).rev() {
        if first[index] != 0 {
            first[index] -= 1;
            first[index + 1..].fill(u8::MAX);
            return Some(first);
        }
    }
    None
}

fn snapshot_leaf_belongs_to_run(
    leaf: &crate::trie::TrieLeaf,
    run: &RunId,
) -> Result<bool, PersistenceError> {
    let family = crate::trie::CatalogFamily::SnapshotOrdered;
    let run_hash = crate::trie::hashed_path(family, run.as_str().as_bytes());
    if leaf.path[..16] != run_hash[..16] {
        return Ok(false);
    }
    let prefix = codec::component(run.as_str())?;
    if !leaf.logical_key.starts_with(&prefix) {
        return Err(error::corruption(
            "snapshot ordered catalog prefix collides across run identities",
        ));
    }
    Ok(true)
}

fn snapshots_exist_for_run_catalog(
    read: &redb::ReadTransaction,
    run: &RunId,
) -> Result<bool, PersistenceError> {
    let page = crate::trie::page(
        read,
        crate::trie::CatalogFamily::SnapshotOrdered,
        None,
        snapshot_group_predecessor(run),
        1,
    )?;
    page.leaves
        .first()
        .map_or(Ok(false), |leaf| snapshot_leaf_belongs_to_run(leaf, run))
}

fn snapshots_exist_for_run_catalog_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<bool, PersistenceError> {
    let page = crate::trie::page_in_transaction(
        write,
        crate::trie::CatalogFamily::SnapshotOrdered,
        None,
        snapshot_group_predecessor(run),
        1,
    )?;
    page.leaves
        .first()
        .map_or(Ok(false), |leaf| snapshot_leaf_belongs_to_run(leaf, run))
}

fn validate_snapshot_catalog_in_transaction(
    write: &redb::WriteTransaction,
    snapshot: &SnapshotDocument,
    key: &[u8],
    document: &[u8],
) -> Result<(), PersistenceError> {
    for (family, path) in [
        (
            crate::trie::CatalogFamily::SnapshotIdentity,
            snapshot_identity_path(key),
        ),
        (
            crate::trie::CatalogFamily::SnapshotOrdered,
            snapshot_ordered_path(snapshot, key)?,
        ),
    ] {
        let witness = crate::trie::verify_member_in_transaction(write, family, path, key)?;
        if witness != Some(crate::trie::digest_payload(family, document)) {
            return Err(error::corruption(
                "snapshot document disagrees with its authenticated catalog",
            ));
        }
    }
    Ok(())
}

fn validate_snapshot_catalog(
    read: &redb::ReadTransaction,
    snapshot: &SnapshotDocument,
    key: &[u8],
    document: &[u8],
) -> Result<(), PersistenceError> {
    for (family, path) in [
        (
            crate::trie::CatalogFamily::SnapshotIdentity,
            snapshot_identity_path(key),
        ),
        (
            crate::trie::CatalogFamily::SnapshotOrdered,
            snapshot_ordered_path(snapshot, key)?,
        ),
    ] {
        let witness = crate::trie::verify_member(read, family, path, key)?;
        if witness != Some(crate::trie::digest_payload(family, document)) {
            return Err(error::corruption(
                "snapshot document disagrees with its authenticated catalog",
            ));
        }
    }
    Ok(())
}

impl HistoryAccumulator {
    fn new(run: &RunId) -> Result<Self, PersistenceError> {
        let run_bytes = run.as_str().as_bytes();
        let run_length = u32::try_from(run_bytes.len()).map_err(|_| PersistenceError::Bounds {
            location: "history.run_id",
            reason: "run identity length does not fit u32".to_owned(),
        })?;
        let mut accumulator = Self {
            schema_version: HISTORY_ACCUMULATOR_SCHEMA_VERSION,
            run: run.clone(),
            through_sequence: RunSequence::ZERO,
            completed_chunks: 0,
            chaining_values: Vec::new(),
            pending: Vec::new(),
        };
        accumulator.update(HISTORY_DIGEST_DOMAIN)?;
        accumulator.update(&run_length.to_be_bytes())?;
        accumulator.update(run_bytes)?;
        Ok(accumulator)
    }

    fn validate(&self) -> Result<(), PersistenceError> {
        if self.schema_version != HISTORY_ACCUMULATOR_SCHEMA_VERSION {
            return Err(PersistenceError::UnsupportedVersion {
                document: "history_accumulator",
                found: self.schema_version,
                supported: HISTORY_ACCUMULATOR_SCHEMA_VERSION,
            });
        }
        if self.through_sequence == RunSequence::ZERO || self.pending.is_empty() {
            return Err(error::corruption(
                "stored history accumulator has no committed event prefix",
            ));
        }
        if self.pending.len() > blake3::CHUNK_LEN {
            return Err(error::corruption(
                "stored history accumulator exceeds one pending BLAKE3 chunk",
            ));
        }
        let expected_stack = usize::try_from(self.completed_chunks.count_ones()).map_err(|_| {
            error::corruption("history accumulator stack length does not fit this platform")
        })?;
        if self.chaining_values.len() != expected_stack {
            return Err(error::corruption(
                "stored history accumulator frontier has an invalid shape",
            ));
        }
        Ok(())
    }

    fn append_event(&mut self, event: &RunEventEnvelope) -> Result<IntegrityDigest, PersistenceError> {
        if event.run_id() != &self.run || event.sequence() != self.through_sequence.next()? {
            return Err(error::corruption(
                "history accumulator received a noncontiguous event",
            ));
        }
        let checksum = event.checksum().as_str().as_bytes();
        let checksum_length =
            u32::try_from(checksum.len()).map_err(|_| PersistenceError::Bounds {
                location: "history.checksum",
                reason: "checksum length does not fit u32".to_owned(),
            })?;
        self.update(&event.sequence().get().to_be_bytes())?;
        self.update(&checksum_length.to_be_bytes())?;
        self.update(checksum)?;
        self.through_sequence = event.sequence();
        self.digest()
    }

    fn update(&mut self, mut input: &[u8]) -> Result<(), PersistenceError> {
        use blake3::hazmat::HasherExt;

        while !input.is_empty() {
            if self.pending.len() == blake3::CHUNK_LEN {
                let offset = self
                    .completed_chunks
                    .checked_mul(u64::try_from(blake3::CHUNK_LEN).map_err(|_| {
                        error::corruption("BLAKE3 chunk length does not fit u64")
                    })?)
                    .ok_or_else(|| error::corruption("history byte offset overflowed"))?;
                let mut hasher = blake3::Hasher::new();
                hasher.set_input_offset(offset);
                hasher.update(&self.pending);
                let mut right = hasher.finalize_non_root();
                self.completed_chunks = self
                    .completed_chunks
                    .checked_add(1)
                    .ok_or_else(|| error::corruption("history chunk count overflowed"))?;
                let mut completed = self.completed_chunks;
                while completed & 1 == 0 {
                    let left = self.chaining_values.pop().ok_or_else(|| {
                        error::corruption("history accumulator frontier underflowed")
                    })?;
                    right = blake3::hazmat::merge_subtrees_non_root(
                        &left,
                        &right,
                        blake3::hazmat::Mode::Hash,
                    );
                    completed >>= 1;
                }
                self.chaining_values.push(right);
                self.pending.clear();
            }
            let available = blake3::CHUNK_LEN - self.pending.len();
            let take = available.min(input.len());
            self.pending.extend_from_slice(&input[..take]);
            input = &input[take..];
        }
        Ok(())
    }

    fn digest(&self) -> Result<IntegrityDigest, PersistenceError> {
        use blake3::hazmat::HasherExt;

        if self.pending.is_empty() {
            return Err(error::corruption(
                "history accumulator lost its rightmost BLAKE3 chunk",
            ));
        }
        let hash = if self.completed_chunks == 0 {
            blake3::hash(&self.pending)
        } else {
            let offset = self
                .completed_chunks
                .checked_mul(u64::try_from(blake3::CHUNK_LEN).map_err(|_| {
                    error::corruption("BLAKE3 chunk length does not fit u64")
                })?)
                .ok_or_else(|| error::corruption("history byte offset overflowed"))?;
            let mut hasher = blake3::Hasher::new();
            hasher.set_input_offset(offset);
            hasher.update(&self.pending);
            let mut right = hasher.finalize_non_root();
            for (index, left) in self.chaining_values.iter().enumerate().rev() {
                if index == 0 {
                    return IntegrityDigest::new(format!(
                        "b3_{}",
                        blake3::hazmat::merge_subtrees_root(
                            left,
                            &right,
                            blake3::hazmat::Mode::Hash,
                        )
                    ));
                }
                right = blake3::hazmat::merge_subtrees_non_root(
                    left,
                    &right,
                    blake3::hazmat::Mode::Hash,
                );
            }
            return Err(error::corruption(
                "history accumulator lost its completed BLAKE3 frontier",
            ));
        };
        IntegrityDigest::new(format!("b3_{hash}"))
    }
}

pub(crate) fn append_history_checkpoint(
    write: &redb::WriteTransaction,
    event: &RunEventEnvelope,
) -> Result<(), PersistenceError> {
    let run = event.run_id();
    let accumulator_family = crate::trie::CatalogFamily::HistoryAccumulator;
    let accumulator_path = crate::trie::hashed_path(accumulator_family, run.as_str().as_bytes());
    let mut accumulator = {
        let table = write
            .open_table(RUN_HISTORY_ACCUMULATORS)
            .map_err(error::redb)?;
        let stored = table
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec());
        drop(table);
        let witness = crate::trie::verify_member_in_transaction(
            write,
            accumulator_family,
            accumulator_path,
            run.as_str().as_bytes(),
        )?;
        match (stored, witness) {
            (None, None) if event.sequence() == RunSequence::FIRST => {
                HistoryAccumulator::new(run)?
            }
            (Some(bytes), Some(payload)) => {
                if payload != crate::trie::digest_payload(accumulator_family, &bytes) {
                    return Err(error::corruption(
                        "history accumulator document disagrees with its authenticated leaf",
                    ));
                }
                let accumulator: HistoryAccumulator =
                    json::decode(&bytes, "history accumulator")?;
                accumulator.validate()?;
                accumulator
            }
            (None, None) => {
                return Err(error::corruption(
                    "noninitial event is missing its history accumulator",
                ));
            }
            _ => {
                return Err(error::corruption(
                    "history accumulator document and authenticated catalog disagree",
                ));
            }
        }
    };
    let digest = accumulator.append_event(event)?;
    let checkpoint = HistoryCheckpoint {
        schema_version: HISTORY_CHECKPOINT_SCHEMA_VERSION,
        run: run.clone(),
        sequence: event.sequence(),
        digest,
    };
    let checkpoint_document = json::encode(&checkpoint, "event history checkpoint")?;
    let key = codec::run_sequence(run.as_str(), event.sequence())?;
    {
        let mut checkpoints = write
            .open_table(EVENT_HISTORY_DIGESTS)
            .map_err(error::redb)?;
        if checkpoints
            .insert(key.as_slice(), checkpoint_document.as_slice())
            .map_err(error::redb)?
            .is_some()
        {
            return Err(error::corruption(
                "event append overwrote a history digest checkpoint",
            ));
        }
    }
    let checkpoint_family = crate::trie::CatalogFamily::EventHistoryCheckpoint;
    if crate::trie::put(
        write,
        checkpoint_family,
        history_checkpoint_path(run, event.sequence(), &key)?,
        &key,
        crate::trie::digest_payload(checkpoint_family, &checkpoint_document),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "event append replaced an authenticated history checkpoint",
        ));
    }
    let accumulator_document = json::encode(&accumulator, "history accumulator")?;
    {
        let mut accumulators = write
            .open_table(RUN_HISTORY_ACCUMULATORS)
            .map_err(error::redb)?;
        accumulators
            .insert(run.as_str(), accumulator_document.as_slice())
            .map_err(error::redb)?;
    }
    crate::trie::put(
        write,
        accumulator_family,
        accumulator_path,
        run.as_str().as_bytes(),
        crate::trie::digest_payload(accumulator_family, &accumulator_document),
    )?;
    Ok(())
}

fn history_checkpoint_path(
    run: &RunId,
    sequence: RunSequence,
    key: &[u8],
) -> Result<[u8; 32], PersistenceError> {
    let family = crate::trie::CatalogFamily::EventHistoryCheckpoint;
    let run_hash = crate::trie::hashed_path(family, run.as_str().as_bytes());
    let mut prefix = [0; 24];
    prefix[..16].copy_from_slice(&run_hash[..16]);
    prefix[16..].copy_from_slice(&sequence.get().to_be_bytes());
    crate::trie::ordered_path(family, &prefix, key)
}

fn decode_history_checkpoint(
    bytes: &[u8],
    run: &RunId,
    sequence: RunSequence,
) -> Result<HistoryCheckpoint, PersistenceError> {
    let checkpoint: HistoryCheckpoint = json::decode(bytes, "event history checkpoint")?;
    if checkpoint.schema_version != HISTORY_CHECKPOINT_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "event_history_checkpoint",
            found: checkpoint.schema_version,
            supported: HISTORY_CHECKPOINT_SCHEMA_VERSION,
        });
    }
    if checkpoint.run != *run || checkpoint.sequence != sequence {
        return Err(error::corruption(
            "history checkpoint key disagrees with its document",
        ));
    }
    Ok(checkpoint)
}

fn history_digest_write(
    transaction: &redb::WriteTransaction,
    run: &RunId,
    through: RunSequence,
) -> Result<IntegrityDigest, PersistenceError> {
    if through == RunSequence::ZERO {
        return Err(PersistenceError::InvalidDocument(
            "snapshot history must cover at least one event".to_owned(),
        ));
    }
    let key = codec::run_sequence(run.as_str(), through)?;
    let table = transaction
        .open_table(EVENT_HISTORY_DIGESTS)
        .map_err(error::redb)?;
    let stored = table
        .get(key.as_slice())
        .map_err(error::redb)?
        .map(|bytes| bytes.value().to_vec());
    drop(table);
    let family = crate::trie::CatalogFamily::EventHistoryCheckpoint;
    let witness = crate::trie::verify_member_in_transaction(
        transaction,
        family,
        history_checkpoint_path(run, through, &key)?,
        &key,
    )?;
    validated_history_checkpoint(stored.as_deref(), witness, run, through)
}

fn history_digest_read(
    transaction: &redb::ReadTransaction,
    run: &RunId,
    through: RunSequence,
) -> Result<IntegrityDigest, PersistenceError> {
    if through == RunSequence::ZERO {
        return Err(PersistenceError::InvalidDocument(
            "snapshot history must cover at least one event".to_owned(),
        ));
    }
    let key = codec::run_sequence(run.as_str(), through)?;
    let table = transaction
        .open_table(EVENT_HISTORY_DIGESTS)
        .map_err(error::redb)?;
    let stored = table
        .get(key.as_slice())
        .map_err(error::redb)?
        .map(|bytes| bytes.value().to_vec());
    drop(table);
    let family = crate::trie::CatalogFamily::EventHistoryCheckpoint;
    let witness = crate::trie::verify_member(
        transaction,
        family,
        history_checkpoint_path(run, through, &key)?,
        &key,
    )?;
    validated_history_checkpoint(stored.as_deref(), witness, run, through)
}

fn validated_history_checkpoint(
    stored: Option<&[u8]>,
    witness: Option<[u8; 32]>,
    run: &RunId,
    through: RunSequence,
) -> Result<IntegrityDigest, PersistenceError> {
    let (Some(stored), Some(witness)) = (stored, witness) else {
        return Err(error::corruption(
            "snapshot history checkpoint is absent or unauthenticated",
        ));
    };
    let family = crate::trie::CatalogFamily::EventHistoryCheckpoint;
    if witness != crate::trie::digest_payload(family, stored) {
        return Err(error::corruption(
            "snapshot history checkpoint disagrees with its authenticated leaf",
        ));
    }
    Ok(decode_history_checkpoint(stored, run, through)?.digest)
}

pub(crate) fn migrate_snapshot_catalogs(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    let snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
    {
        let latest = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
        for item in latest.iter().map_err(error::redb)? {
            let (run, snapshot_id) = item.map_err(error::redb)?;
            let run = RunId::new(run.value()).map_err(|cause| {
                error::corruption(format!("legacy snapshot run identity is invalid: {cause}"))
            })?;
            let snapshot_id = SnapshotId::new(snapshot_id.value()).map_err(|cause| {
                error::corruption(format!("legacy snapshot latest identity is invalid: {cause}"))
            })?;
            let key = codec::pair(run.as_str(), snapshot_id.as_str())?;
            let document = snapshots
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("legacy snapshot latest pointer is dangling"))?;
            let snapshot = decode_stored_snapshot(document.value())?;
            if snapshot.run() != &run || snapshot.snapshot() != &snapshot_id {
                return Err(error::corruption(
                    "legacy snapshot latest pointer disagrees with its document",
                ));
            }
        }
    }
    let mut current: Option<(RunId, [u8; 32], SnapshotId)> = None;
    let mut run_groups = 0_u64;
    for item in snapshots.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let key = key.value().to_vec();
        let document = bytes.value().to_vec();
        let snapshot = decode_stored_snapshot(&document)?;
        let expected_key = codec::pair(snapshot.run().as_str(), snapshot.snapshot().as_str())?;
        if key != expected_key {
            return Err(error::corruption(
                "legacy snapshot key disagrees with its verified document",
            ));
        }
        if history_digest_write(write, snapshot.run(), snapshot.covered_sequence())?
            != *snapshot.history_digest()
        {
            return Err(error::corruption(
                "legacy snapshot digest disagrees with its authenticated history checkpoint",
            ));
        }
        for (family, path) in [
            (
                crate::trie::CatalogFamily::SnapshotIdentity,
                snapshot_identity_path(&key),
            ),
            (
                crate::trie::CatalogFamily::SnapshotOrdered,
                snapshot_ordered_path(&snapshot, &key)?,
            ),
        ] {
            if crate::trie::put(
                write,
                family,
                path,
                &key,
                crate::trie::digest_payload(family, &document),
            )?
            .is_some()
            {
                return Err(error::corruption(
                    "legacy snapshot catalog contains a duplicate leaf",
                ));
            }
        }
        let ordered_path = snapshot_ordered_path(&snapshot, &key)?;
        match current.as_mut() {
            Some((run, best_path, best_id)) if run == snapshot.run() => {
                if ordered_path > *best_path {
                    *best_path = ordered_path;
                    *best_id = snapshot.snapshot().clone();
                }
            }
            Some((run, _, best_id)) => {
                persist_migrated_snapshot_latest(write, run, best_id)?;
                run_groups = run_groups
                    .checked_add(1)
                    .ok_or_else(|| error::corruption("snapshot run group count overflowed"))?;
                current = Some((
                    snapshot.run().clone(),
                    ordered_path,
                    snapshot.snapshot().clone(),
                ));
            }
            None => {
                current = Some((
                    snapshot.run().clone(),
                    ordered_path,
                    snapshot.snapshot().clone(),
                ));
            }
        }
    }
    if let Some((run, _, best_id)) = current.as_ref() {
        persist_migrated_snapshot_latest(write, run, best_id)?;
        run_groups = run_groups
            .checked_add(1)
            .ok_or_else(|| error::corruption("snapshot run group count overflowed"))?;
    }
    drop(snapshots);
    let latest = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
    if latest.len().map_err(error::redb)? != run_groups {
        return Err(error::corruption(
            "legacy snapshot latest index and snapshot run groups disagree",
        ));
    }
    Ok(())
}

fn persist_migrated_snapshot_latest(
    write: &redb::WriteTransaction,
    run: &RunId,
    canonical_latest: &SnapshotId,
) -> Result<(), PersistenceError> {
    let previous = {
        let latest = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
        latest
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned())
    }
    .ok_or_else(|| error::corruption("legacy snapshot run has no latest pointer"))?;
    let _previous = SnapshotId::new(previous).map_err(|cause| {
        error::corruption(format!("legacy snapshot latest identity is invalid: {cause}"))
    })?;
    {
        let mut latest = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
        latest
            .insert(run.as_str(), canonical_latest.as_str())
            .map_err(error::redb)?;
    }
    let family = crate::trie::CatalogFamily::SnapshotLatest;
    if crate::trie::put(
        write,
        family,
        snapshot_latest_path(run),
        run.as_str().as_bytes(),
        crate::trie::digest_payload(family, canonical_latest.as_str().as_bytes()),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "legacy snapshot latest catalog contains a duplicate leaf",
        ));
    }
    Ok(())
}

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
        let actual_digest =
            history_digest_write(&write, snapshot.run(), snapshot.covered_sequence())?;
        if &actual_digest != snapshot.history_digest() {
            return Err(PersistenceError::InvalidDocument(
                "snapshot history digest does not match authoritative events".to_owned(),
            ));
        }

        let previous_latest = validated_latest_pointer(&write, snapshot.run())?;
        let mut inserted = false;
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
                inserted = true;
            }
        }
        if inserted {
            for (family, path) in [
                (
                    crate::trie::CatalogFamily::SnapshotIdentity,
                    snapshot_identity_path(&key),
                ),
                (
                    crate::trie::CatalogFamily::SnapshotOrdered,
                    snapshot_ordered_path(snapshot, &key)?,
                ),
            ] {
                if crate::trie::put(
                    &write,
                    family,
                    path,
                    &key,
                    crate::trie::digest_payload(family, &document),
                )?
                .is_some()
                {
                    return Err(error::corruption(
                        "snapshot insert replaced an authenticated catalog leaf",
                    ));
                }
            }
        } else {
            validate_snapshot_catalog_in_transaction(&write, snapshot, &key, &document)?;
        }

        let candidate_path = snapshot_ordered_path(snapshot, &key)?;
        let should_advance = if let Some(latest) = previous_latest.as_ref() {
            let latest_key = codec::pair(latest.run().as_str(), latest.snapshot().as_str());
            snapshot_ordered_path(latest, &latest_key?)? <= candidate_path
        } else {
            true
        };
        if should_advance {
            {
                let mut latest = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
                latest
                    .insert(snapshot.run().as_str(), snapshot.snapshot().as_str())
                    .map_err(error::redb)?;
            }
            let family = crate::trie::CatalogFamily::SnapshotLatest;
            crate::trie::put(
                &write,
                family,
                snapshot_latest_path(snapshot.run()),
                snapshot.run().as_str().as_bytes(),
                crate::trie::digest_payload(family, snapshot.snapshot().as_str().as_bytes()),
            )?;
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
        let _membership = crate::journal::validate_run_history_membership(&read, run, head)?;
        let latest = read.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
        let snapshot_id = latest
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned());
        drop(latest);
        let latest_family = crate::trie::CatalogFamily::SnapshotLatest;
        let latest_witness = crate::trie::verify_member(
            &read,
            latest_family,
            snapshot_latest_path(run),
            run.as_str().as_bytes(),
        )?;
        let Some(snapshot_id) = snapshot_id else {
            if latest_witness.is_some() {
                return Err(error::corruption(
                    "snapshot latest pointer is absent from its physical table",
                ));
            }
            let prefix = codec::component(run.as_str())?;
            let end = codec::prefix_end(prefix.clone())
                .ok_or_else(|| error::corruption("snapshot run prefix has no range end"))?;
            let snapshots = read.open_table(SNAPSHOTS).map_err(error::redb)?;
            let physical_exists = snapshots
                .range(prefix.as_slice()..end.as_slice())
                .map_err(error::redb)?
                .next()
                .transpose()
                .map_err(error::redb)?
                .is_some();
            drop(snapshots);
            let catalog_exists = snapshots_exist_for_run_catalog(&read, run)?;
            if physical_exists || catalog_exists {
                return Err(error::corruption(format!(
                    "run {run} retains snapshots without a latest-snapshot pointer"
                )));
            }
            return Ok(SnapshotLoad::Absent);
        };
        if latest_witness
            != Some(crate::trie::digest_payload(
                latest_family,
                snapshot_id.as_bytes(),
            ))
        {
            return Err(error::corruption(
                "snapshot latest pointer disagrees with its authenticated catalog",
            ));
        }
        let snapshot_id = SnapshotId::new(snapshot_id).map_err(|cause| {
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
        let document = bytes.value().to_vec();
        drop(bytes);
        drop(snapshots);
        let snapshot = match decode_stored_snapshot(&document) {
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
        if let Err(cause) = validate_snapshot_catalog(&read, &snapshot, &key, &document) {
            return rejected(Some(snapshot_id), cause.to_string());
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
        let head = crate::journal::validated_run_head_in_transaction(&write, run)?;
        let _membership =
            crate::journal::validate_run_history_membership_in_transaction(&write, run, head)?;
        let previous_latest = validated_latest_pointer(&write, run)?;
        let stored = {
            let snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
            snapshots
                .get(key.as_slice())
                .map_err(error::redb)?
                .map(|bytes| bytes.value().to_vec())
        };
        let identity_family = crate::trie::CatalogFamily::SnapshotIdentity;
        let identity_witness = crate::trie::verify_member_in_transaction(
            &write,
            identity_family,
            snapshot_identity_path(&key),
            &key,
        )?;
        let target = match (stored.as_deref(), identity_witness) {
            (None, None) => None,
            (Some(document), Some(witness)) => {
                if witness != crate::trie::digest_payload(identity_family, document) {
                    return Err(error::corruption(
                        "discarded snapshot disagrees with its authenticated identity",
                    ));
                }
                let target = decode_stored_snapshot(document)?;
                if target.run() != run || target.snapshot() != snapshot {
                    return Err(error::corruption(
                        "discarded snapshot key disagrees with its document",
                    ));
                }
                validate_snapshot_catalog_in_transaction(&write, &target, &key, document)?;
                Some(target)
            }
            _ => {
                return Err(error::corruption(
                    "discarded snapshot physical row and authenticated catalog disagree",
                ));
            }
        };
        let target_path = target
            .as_ref()
            .map(|target| snapshot_ordered_path(target, &key))
            .transpose()?;
        {
            let mut snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
            let _removed = snapshots.remove(key.as_slice()).map_err(error::redb)?;
        }
        if let (Some(_target), Some(target_path)) = (target.as_ref(), target_path) {
            if crate::trie::remove(
                &write,
                identity_family,
                snapshot_identity_path(&key),
                &key,
            )?
            .is_none()
                || crate::trie::remove(
                    &write,
                    crate::trie::CatalogFamily::SnapshotOrdered,
                    target_path,
                    &key,
                )?
                .is_none()
            {
                return Err(error::corruption(
                    "discarded snapshot was absent from an authenticated catalog",
                ));
            }
        }
        if previous_latest
            .as_ref()
            .is_some_and(|latest| latest.snapshot() == snapshot)
        {
            let before = target_path.ok_or_else(|| {
                error::corruption("latest snapshot is absent from its identity catalog")
            })?;
            let replacement = newest_snapshot_for_run(&write, run, before)?;
            {
                let mut latest = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
                if let Some(replacement) = replacement.as_ref() {
                    latest
                        .insert(run.as_str(), replacement.snapshot().as_str())
                        .map_err(error::redb)?;
                } else {
                    let _removed = latest.remove(run.as_str()).map_err(error::redb)?;
                }
            }
            let latest_family = crate::trie::CatalogFamily::SnapshotLatest;
            if let Some(replacement) = replacement {
                crate::trie::put(
                    &write,
                    latest_family,
                    snapshot_latest_path(run),
                    run.as_str().as_bytes(),
                    crate::trie::digest_payload(
                        latest_family,
                        replacement.snapshot().as_str().as_bytes(),
                    ),
                )?;
            } else {
                if crate::trie::remove(
                    &write,
                    latest_family,
                    snapshot_latest_path(run),
                    run.as_str().as_bytes(),
                )?
                .is_none()
                {
                    return Err(error::corruption(
                        "latest snapshot pointer was absent from its authenticated catalog",
                    ));
                }
            }
        }
        self.faults.check(FaultPoint::BeforeSnapshotDiscardCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterSnapshotDiscardCommit)
    }
}

fn validated_latest_pointer(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<Option<SnapshotDocument>, PersistenceError> {
    let latest = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
    let snapshot_id = latest
        .get(run.as_str())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    drop(latest);
    let latest_family = crate::trie::CatalogFamily::SnapshotLatest;
    let latest_witness = crate::trie::verify_member_in_transaction(
        write,
        latest_family,
        snapshot_latest_path(run),
        run.as_str().as_bytes(),
    )?;
    let Some(snapshot_id) = snapshot_id else {
        if latest_witness.is_some() {
            return Err(error::corruption(
                "snapshot latest pointer is absent from its physical table",
            ));
        }
        if snapshots_exist_for_run(write, run)?
            || snapshots_exist_for_run_catalog_in_transaction(write, run)?
        {
            return Err(error::corruption(format!(
                "run {run} retains snapshots without a latest-snapshot pointer"
            )));
        }
        return Ok(None);
    };
    if latest_witness
        != Some(crate::trie::digest_payload(
            latest_family,
            snapshot_id.as_bytes(),
        ))
    {
        return Err(error::corruption(
            "snapshot latest pointer disagrees with its authenticated catalog",
        ));
    }
    let snapshot_id = SnapshotId::new(snapshot_id)
        .map_err(|cause| error::corruption(format!("invalid latest snapshot identity: {cause}")))?;
    let key = codec::pair(run.as_str(), snapshot_id.as_str())?;
    let snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
    let bytes = snapshots
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("latest snapshot pointer is dangling"))?;
    let document = bytes.value().to_vec();
    drop(bytes);
    drop(snapshots);
    let snapshot = decode_stored_snapshot(&document)?;
    if snapshot.run() != run || snapshot.snapshot() != &snapshot_id {
        return Err(error::corruption(
            "latest snapshot key does not match its verified document",
        ));
    }
    validate_snapshot_catalog_in_transaction(write, &snapshot, &key, &document)?;
    Ok(Some(snapshot))
}

fn snapshots_exist_for_run(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<bool, PersistenceError> {
    let prefix = codec::component(run.as_str())?;
    let end = codec::prefix_end(prefix.clone())
        .ok_or_else(|| error::corruption("snapshot run prefix has no range end"))?;
    let snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
    Ok(snapshots
        .range(prefix.as_slice()..end.as_slice())
        .map_err(error::redb)?
        .next()
        .transpose()
        .map_err(error::redb)?
        .is_some())
}

fn newest_snapshot_for_run(
    write: &redb::WriteTransaction,
    run: &RunId,
    before: [u8; 32],
) -> Result<Option<SnapshotDocument>, PersistenceError> {
    let family = crate::trie::CatalogFamily::SnapshotOrdered;
    let Some(leaf) = crate::trie::predecessor_in_transaction(write, family, None, before)? else {
        return Ok(None);
    };
    if !snapshot_leaf_belongs_to_run(&leaf, run)? {
        return Ok(None);
    }
    let snapshots = write.open_table(SNAPSHOTS).map_err(error::redb)?;
    let document = snapshots
        .get(leaf.logical_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("snapshot ordered catalog is dangling"))?
        .value()
        .to_vec();
    drop(snapshots);
    if leaf.payload_digest != crate::trie::digest_payload(family, &document) {
        return Err(error::corruption(
            "snapshot ordered leaf disagrees with its document",
        ));
    }
    let candidate = decode_stored_snapshot(&document)?;
    let expected_key = codec::pair(run.as_str(), candidate.snapshot().as_str())?;
    if candidate.run() != run || leaf.logical_key != expected_key {
        return Err(error::corruption(
            "snapshot key does not match its verified document",
        ));
    }
    validate_snapshot_catalog_in_transaction(
        write,
        &candidate,
        &leaf.logical_key,
        &document,
    )?;
    Ok(Some(candidate))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_history_frontier_matches_flat_blake3_across_chunk_boundaries()
    -> Result<(), Box<dyn std::error::Error>> {
        let run = RunId::new("run_history_frontier_test")?;
        for payload_len in [1_usize, 700, 1_024, 1_025, 2_047, 2_048, 8_321] {
            let payload: Vec<u8> = (0..payload_len)
                .map(|index| u8::try_from(index % 251).unwrap_or_default())
                .collect();
            let mut accumulator = HistoryAccumulator::new(&run)?;
            for fragment in payload.chunks(137) {
                accumulator.update(fragment)?;
            }

            let run_bytes = run.as_str().as_bytes();
            let run_length = u32::try_from(run_bytes.len()).map_err(|_| {
                PersistenceError::InvalidDocument("test run identity is too long".to_owned())
            })?;
            let mut flat = blake3::Hasher::new();
            flat.update(HISTORY_DIGEST_DOMAIN);
            flat.update(&run_length.to_be_bytes());
            flat.update(run_bytes);
            flat.update(&payload);
            let expected = IntegrityDigest::new(format!("b3_{}", flat.finalize()))?;
            assert_eq!(accumulator.digest()?, expected);
        }
        Ok(())
    }
}
