//! One installation record and a finite, append-only set of exact rename acceptances.
#![deny(clippy::arithmetic_side_effects)]

use milkdrift_authority::ActorRef;
use milkdrift_capability::PeerId;
use milkdrift_persistence::{
    DaemonDisplayName, DaemonMetadata, DaemonMetadataStore, DaemonRename, DaemonRenameOutcome,
    DaemonRenameReceipt, MAX_DAEMON_RENAME_RECEIPT_BYTES, MAX_DAEMON_RENAME_RECEIPTS,
    PersistenceError,
};
use redb::{ReadableTable, ReadableTableMetadata};

use crate::{
    RedbStore, error,
    fault::FaultPoint,
    json,
    schema::{DAEMON_METADATA, DAEMON_RENAME_RECEIPTS, SERVING_HOST_IDENTITY},
};

const METADATA_KEY: &str = "installation";

impl DaemonMetadataStore for RedbStore {
    fn initialize_daemon_metadata(
        &self,
        installation: &PeerId,
        seed: &DaemonDisplayName,
    ) -> Result<DaemonMetadata, PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        crate::peer::bind_host_in_transaction(&write, installation)?;
        let mut table = write.open_table(DAEMON_METADATA).map_err(error::redb)?;
        if let Some(metadata) = current_metadata(&table)? {
            require_installation(&metadata.installation, installation)?;
            return Ok(metadata);
        }
        if !write
            .open_table(DAEMON_RENAME_RECEIPTS)
            .map_err(error::redb)?
            .is_empty()
            .map_err(error::redb)?
        {
            return Err(error::corruption("daemon receipts exist without metadata"));
        }
        let metadata = DaemonMetadata {
            installation: installation.clone(),
            display_name: seed.clone(),
            generation: 1,
        };
        let bytes = json::encode(&metadata, "daemon metadata")?;
        table
            .insert(METADATA_KEY, bytes.as_slice())
            .map_err(error::redb)?;
        drop(table);
        self.faults.check(FaultPoint::BeforeDaemonMetadataCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterDaemonMetadataCommit)?;
        Ok(metadata)
    }

    fn daemon_metadata(
        &self,
        installation: &PeerId,
    ) -> Result<Option<DaemonMetadata>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        require_bound_installation(&read, installation)?;
        let metadata = current_metadata(&read.open_table(DAEMON_METADATA).map_err(error::redb)?)?;
        if let Some(metadata) = &metadata {
            require_installation(&metadata.installation, installation)?;
        }
        Ok(metadata)
    }

    fn daemon_rename_receipt(
        &self,
        installation: &PeerId,
        actor: &ActorRef,
        key: &str,
    ) -> Result<Option<DaemonRenameReceipt>, PersistenceError> {
        let storage_key = receipt_key(actor, key)?;
        let read = self.database().begin_read().map_err(error::redb)?;
        require_bound_installation(&read, installation)?;
        let table = read
            .open_table(DAEMON_RENAME_RECEIPTS)
            .map_err(error::redb)?;
        table
            .get(storage_key.as_str())
            .map_err(error::redb)?
            .map(|bytes| {
                let receipt = decode_receipt(storage_key.as_str(), bytes.value())?;
                require_installation(&receipt.installation, installation)?;
                Ok(receipt)
            })
            .transpose()
    }

    fn rename_daemon(&self, rename: DaemonRename) -> Result<DaemonRenameOutcome, PersistenceError> {
        let storage_key = receipt_key(&rename.actor, &rename.key)?;
        let write = self.database().begin_write().map_err(error::redb)?;
        crate::peer::bind_host_in_transaction(&write, &rename.installation)?;
        let mut table = write.open_table(DAEMON_METADATA).map_err(error::redb)?;
        let current = current_metadata(&table)?.ok_or_else(|| PersistenceError::NotFound {
            entity: "daemon metadata",
            identity: rename.installation.to_string(),
        })?;
        require_installation(&current.installation, &rename.installation)?;
        let mut receipts = write
            .open_table(DAEMON_RENAME_RECEIPTS)
            .map_err(error::redb)?;
        if let Some(bytes) = receipts.get(storage_key.as_str()).map_err(error::redb)? {
            let receipt = decode_receipt(&storage_key, bytes.value())?;
            require_installation(&receipt.installation, &rename.installation)?;
            if receipt.canonical_request != rename.canonical_request {
                return Err(PersistenceError::DaemonIdempotencyConflict {
                    actor: rename.actor,
                    key: rename.key,
                });
            }
            return Ok(DaemonRenameOutcome::Replay(receipt));
        }
        if current.generation != rename.expected_generation {
            return Err(PersistenceError::DaemonGenerationConflict {
                expected: rename.expected_generation,
                actual: current.generation,
            });
        }
        let count = receipts.len().map_err(error::redb)?;
        if count.checked_add(1) != Some(current.generation) {
            return Err(error::corruption(
                "daemon receipt count differs from metadata generation",
            ));
        }
        if count >= MAX_DAEMON_RENAME_RECEIPTS {
            return Err(PersistenceError::Storage {
                class: milkdrift_persistence::StorageFailureClass::ResourceExhausted,
                message: "rename receipt capacity is full; existing names and exact replay remain available".to_owned(),
            });
        }
        let result = DaemonMetadata {
            installation: rename.installation.clone(),
            display_name: rename.display_name,
            generation: current
                .generation
                .checked_add(1)
                .ok_or_else(|| bounds("daemon name generation exhausted"))?,
        };
        let receipt = DaemonRenameReceipt {
            installation: rename.installation,
            actor: rename.actor,
            key: rename.key,
            canonical_request: rename.canonical_request,
            result,
        };
        validate_receipt(&receipt)?;
        let bytes = json::encode(&receipt.result, "daemon metadata")?;
        let receipt_bytes = json::encode(&receipt, "daemon rename receipt")?;
        table
            .insert(METADATA_KEY, bytes.as_slice())
            .map_err(error::redb)?;
        receipts
            .insert(storage_key.as_str(), receipt_bytes.as_slice())
            .map_err(error::redb)?;
        drop(receipts);
        drop(table);
        self.faults.check(FaultPoint::BeforeDaemonMetadataCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterDaemonMetadataCommit)?;
        Ok(DaemonRenameOutcome::Applied(receipt))
    }
}

fn current_metadata(
    table: &impl ReadableTable<&'static str, &'static [u8]>,
) -> Result<Option<DaemonMetadata>, PersistenceError> {
    let bytes = table.get(METADATA_KEY).map_err(error::redb)?;
    if table.len().map_err(error::redb)? != u64::from(bytes.is_some()) {
        return Err(error::corruption(
            "daemon metadata must contain one installation record",
        ));
    }
    bytes
        .map(|bytes| {
            let metadata: DaemonMetadata = json::decode(bytes.value(), "daemon metadata")?;
            if metadata.generation == 0
                || metadata.generation
                    > MAX_DAEMON_RENAME_RECEIPTS
                        .checked_add(1)
                        .ok_or_else(|| bounds("daemon receipt limit overflow"))?
            {
                return Err(error::corruption(
                    "daemon metadata generation is outside its receipt bound",
                ));
            }
            Ok(metadata)
        })
        .transpose()
}

fn require_installation(actual: &PeerId, expected: &PeerId) -> Result<(), PersistenceError> {
    if actual != expected {
        return Err(PersistenceError::InvalidDocument(
            "configured host_id differs from this store's daemon installation; use its original identity".to_owned(),
        ));
    }
    Ok(())
}

fn require_bound_installation(
    read: &redb::ReadTransaction,
    expected: &PeerId,
) -> Result<(), PersistenceError> {
    let table = read
        .open_table(SERVING_HOST_IDENTITY)
        .map_err(error::redb)?;
    let host = table.get("host").map_err(error::redb)?;
    if table.len().map_err(error::redb)? != u64::from(host.is_some()) {
        return Err(error::corruption(
            "serving installation identity table is invalid",
        ));
    }
    if let Some(host) = host {
        let actual = PeerId::new(host.value())
            .map_err(|_| error::corruption("invalid stored installation identity"))?;
        require_installation(&actual, expected)?;
    }
    Ok(())
}

fn receipt_key(actor: &ActorRef, key: &str) -> Result<String, PersistenceError> {
    if key.is_empty()
        || matches!(key, "." | "..")
        || key.len() > 512
        || key.chars().any(char::is_control)
    {
        return Err(bounds(
            "rename request key must contain 1–512 bytes without control characters and cannot be a standalone dot segment",
        ));
    }
    Ok(format!("{}:{actor}{key}", actor.as_str().len()))
}

fn validate_receipt(receipt: &DaemonRenameReceipt) -> Result<(), PersistenceError> {
    receipt_key(&receipt.actor, &receipt.key)?;
    if receipt.installation != receipt.result.installation || receipt.result.generation < 2 {
        return Err(error::corruption(
            "daemon receipt result identity or generation is invalid",
        ));
    }
    let result_bytes = serde_json::to_vec(&receipt.result)?.len();
    if receipt
        .canonical_request
        .len()
        .checked_add(result_bytes)
        .is_none_or(|bytes| bytes > MAX_DAEMON_RENAME_RECEIPT_BYTES)
    {
        return Err(bounds("canonical rename request and result exceed 16 KiB"));
    }
    let request: serde_json::Value = serde_json::from_slice(&receipt.canonical_request)?;
    if serde_json::to_vec(&request)? != receipt.canonical_request {
        return Err(PersistenceError::InvalidDocument(
            "daemon rename request is not canonical JSON".to_owned(),
        ));
    }
    Ok(())
}

fn decode_receipt(key: &str, bytes: &[u8]) -> Result<DaemonRenameReceipt, PersistenceError> {
    let receipt: DaemonRenameReceipt = json::decode(bytes, "daemon rename receipt")?;
    validate_receipt(&receipt)?;
    if receipt_key(&receipt.actor, &receipt.key)? != key {
        return Err(error::corruption(
            "daemon receipt key differs from its actor and request",
        ));
    }
    Ok(receipt)
}

pub(crate) fn validate(read: &redb::ReadTransaction) -> Result<(), PersistenceError> {
    let metadata = current_metadata(&read.open_table(DAEMON_METADATA).map_err(error::redb)?)?;
    let receipts = read
        .open_table(DAEMON_RENAME_RECEIPTS)
        .map_err(error::redb)?;
    let count = receipts.len().map_err(error::redb)?;
    let Some(metadata) = metadata else {
        return if count == 0 {
            Ok(())
        } else {
            Err(error::corruption("daemon receipts exist without metadata"))
        };
    };
    require_bound_installation(read, &metadata.installation)?;
    if read
        .open_table(SERVING_HOST_IDENTITY)
        .map_err(error::redb)?
        .is_empty()
        .map_err(error::redb)?
    {
        return Err(error::corruption(
            "daemon metadata has no stable installation binding",
        ));
    }
    if count > MAX_DAEMON_RENAME_RECEIPTS || count.checked_add(1) != Some(metadata.generation) {
        return Err(error::corruption(
            "daemon receipt count differs from metadata generation",
        ));
    }
    let mut generations = std::collections::BTreeSet::new();
    for row in receipts.iter().map_err(error::redb)? {
        let (key, bytes) = row.map_err(error::redb)?;
        let receipt = decode_receipt(key.value(), bytes.value())?;
        if receipt.installation != metadata.installation
            || receipt.result.generation > metadata.generation
            || !generations.insert(receipt.result.generation)
            || (receipt.result.generation == metadata.generation && receipt.result != metadata)
        {
            return Err(error::corruption(
                "daemon receipt history contradicts current metadata",
            ));
        }
    }
    Ok(())
}

fn bounds(reason: &str) -> PersistenceError {
    PersistenceError::Bounds {
        location: "daemon_metadata",
        reason: reason.to_owned(),
    }
}
