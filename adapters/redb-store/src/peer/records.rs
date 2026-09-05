//! Physical peer document, ownership-index and observation-chain encodings.
use super::{
    LOCATION_ARCHIVED, LOCATION_HOT, OBSERVATION_DIGEST_DOMAIN, validation::validate_record,
    validation::validate_tombstone,
};
use crate::{
    codec, error, json, schema::PEER_ACTIVE_CLAIMS, schema::PEER_CATALOGS,
    schema::PEER_EXECUTION_LOCATIONS, schema::PEER_EXECUTION_TOMBSTONES, schema::PEER_EXECUTIONS,
    schema::PEER_RELATIONSHIPS, schema::PEER_TERMINAL_INDEX,
};
use milkdrift_capability::PeerId;
use milkdrift_peer_protocol::{PeerExecutionId, PeerRequestId};
use milkdrift_persistence::{
    PeerCatalogState, PeerDispatchClaim, PeerExecutionPhase, PeerExecutionRecord,
    PeerExecutionSnapshot, PeerExecutionTombstone, PeerRelationshipState, PersistenceError,
    StorageFailureClass, WorkerId,
};

pub(super) fn relationship_in_transaction(
    write: &redb::WriteTransaction,
    peer: &PeerId,
) -> Result<Option<PeerRelationshipState>, PersistenceError> {
    write
        .open_table(PEER_RELATIONSHIPS)
        .map_err(error::redb)?
        .get(peer.as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode(bytes.value(), "peer relationship"))
        .transpose()
}

pub(super) fn catalog_in_transaction(
    write: &redb::WriteTransaction,
    peer: &PeerId,
) -> Result<Option<PeerCatalogState>, PersistenceError> {
    write
        .open_table(PEER_CATALOGS)
        .map_err(error::redb)?
        .get(peer.as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode(bytes.value(), "peer catalog"))
        .transpose()
}

pub(super) fn execution_optional_in_transaction(
    write: &redb::WriteTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<PeerExecutionRecord>, PersistenceError> {
    let record = write
        .open_table(PEER_EXECUTIONS)
        .map_err(error::redb)?
        .get(execution.as_str())
        .map_err(error::redb)?
        .map(|bytes| decode_record(bytes.value()))
        .transpose()?;
    if record.is_some()
        && execution_location_in_transaction(write, execution)? != Some(LOCATION_HOT)
    {
        return Err(corruption(
            "hot peer execution disagrees with its location index",
        ));
    }
    Ok(record)
}

pub(super) fn execution_location_in_transaction(
    write: &redb::WriteTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<u8>, PersistenceError> {
    write
        .open_table(PEER_EXECUTION_LOCATIONS)
        .map_err(error::redb)?
        .get(execution.as_str())
        .map_err(error::redb)
        .map(|value| value.map(|value| value.value()))
}

pub(super) fn tombstone_optional_in_transaction(
    write: &redb::WriteTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<PeerExecutionTombstone>, PersistenceError> {
    write
        .open_table(PEER_EXECUTION_TOMBSTONES)
        .map_err(error::redb)?
        .get(execution.as_str())
        .map_err(error::redb)?
        .map(|bytes| decode_tombstone(bytes.value()))
        .transpose()
}

pub(super) fn execution_in_transaction_text(
    write: &redb::WriteTransaction,
    execution: &str,
) -> Result<PeerExecutionRecord, PersistenceError> {
    let execution_id = parse_execution_id(execution)?;
    execution_optional_in_transaction(write, &execution_id)?
        .ok_or_else(|| corruption("peer index points at a missing primary record"))
}

pub(super) fn snapshot_in_transaction_text(
    write: &redb::WriteTransaction,
    execution: &str,
) -> Result<PeerExecutionSnapshot, PersistenceError> {
    let execution_id = parse_execution_id(execution)?;
    snapshot_optional_in_transaction(write, &execution_id)?
        .ok_or_else(|| corruption("peer index points at a missing execution authority"))
}

pub(super) fn snapshot_in_read_transaction_text(
    read: &redb::ReadTransaction,
    execution: &str,
) -> Result<PeerExecutionSnapshot, PersistenceError> {
    let execution_id = parse_execution_id(execution)?;
    snapshot_optional_in_read_transaction(read, &execution_id)?
        .ok_or_else(|| corruption("peer index points at a missing execution authority"))
}

pub(super) fn parse_execution_id(execution: &str) -> Result<PeerExecutionId, PersistenceError> {
    PeerExecutionId::new(execution.to_owned()).map_err(|cause| {
        corruption(format!(
            "stored peer execution identity is invalid: {cause}"
        ))
    })
}

pub(super) fn owned_execution_in_transaction(
    write: &redb::WriteTransaction,
    owner: &PeerId,
    execution: &PeerExecutionId,
) -> Result<PeerExecutionRecord, PersistenceError> {
    execution_optional_in_transaction(write, execution)?
        .filter(|record| record.owner_peer == *owner)
        .ok_or_else(|| missing("peer_execution", execution.as_str()))
}

pub(super) fn put_execution(
    write: &redb::WriteTransaction,
    record: &PeerExecutionRecord,
) -> Result<(), PersistenceError> {
    validate_record(record)?;
    let bytes = json::encode(record, "peer execution")?;
    write
        .open_table(PEER_EXECUTIONS)
        .map_err(error::redb)?
        .insert(record.execution.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

pub(super) fn decode_record(bytes: &[u8]) -> Result<PeerExecutionRecord, PersistenceError> {
    let record = json::decode(bytes, "peer execution")?;
    validate_record(&record)?;
    Ok(record)
}

pub(super) fn put_tombstone(
    write: &redb::WriteTransaction,
    tombstone: &PeerExecutionTombstone,
) -> Result<(), PersistenceError> {
    validate_tombstone(tombstone)?;
    let bytes = json::encode(tombstone, "peer execution tombstone")?;
    if write
        .open_table(PEER_EXECUTION_TOMBSTONES)
        .map_err(error::redb)?
        .insert(tombstone.execution.as_str(), bytes.as_slice())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(corruption(
            "peer tombstone insertion replaced an existing authority",
        ));
    }
    Ok(())
}

pub(super) fn decode_tombstone(bytes: &[u8]) -> Result<PeerExecutionTombstone, PersistenceError> {
    let tombstone = json::decode(bytes, "peer execution tombstone")?;
    validate_tombstone(&tombstone)?;
    Ok(tombstone)
}

pub(super) fn exact_pre_entry_claim<'a>(
    record: &'a PeerExecutionRecord,
    worker: &WorkerId,
    generation: u64,
) -> Result<&'a PeerDispatchClaim, PersistenceError> {
    match &record.phase {
        PeerExecutionPhase::DispatchClaimed { claim }
            if claim.worker == *worker && claim.generation == generation =>
        {
            Ok(claim)
        }
        _ => Err(PersistenceError::ImmutableConflict {
            entity: "peer_dispatch_claim",
            identity: record.execution.to_string(),
        }),
    }
}

pub(super) fn exact_claim<'a>(
    record: &'a PeerExecutionRecord,
    worker: &WorkerId,
    generation: u64,
) -> Result<&'a PeerDispatchClaim, PersistenceError> {
    record
        .phase
        .claim()
        .filter(|claim| claim.worker == *worker && claim.generation == generation)
        .ok_or_else(|| PersistenceError::ImmutableConflict {
            entity: "peer_dispatch_claim",
            identity: record.execution.to_string(),
        })
}

pub(super) fn remove_claim_index(
    write: &redb::WriteTransaction,
    execution: &PeerExecutionId,
    claim: &PeerDispatchClaim,
) -> Result<(), PersistenceError> {
    let key = claim_key(execution, claim)?;
    let removed = write
        .open_table(PEER_ACTIVE_CLAIMS)
        .map_err(error::redb)?
        .remove(key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    if removed.as_deref() != Some(execution.as_str()) {
        return Err(corruption("peer claim index is missing or mismatched"));
    }
    Ok(())
}

pub(super) fn insert_terminal_index(
    write: &redb::WriteTransaction,
    record: &PeerExecutionRecord,
    terminal_at: u64,
) -> Result<(), PersistenceError> {
    let key = ordered_key(terminal_at, record.execution.as_str())?;
    write
        .open_table(PEER_TERMINAL_INDEX)
        .map_err(error::redb)?
        .insert(key.as_slice(), record.execution.as_str())
        .map_err(error::redb)?;
    Ok(())
}

pub(super) fn remove_terminal_index(
    write: &redb::WriteTransaction,
    execution: &PeerExecutionId,
    terminal_at: u64,
) -> Result<(), PersistenceError> {
    let key = ordered_key(terminal_at, execution.as_str())?;
    let removed = write
        .open_table(PEER_TERMINAL_INDEX)
        .map_err(error::redb)?
        .remove(key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    if removed.as_deref() != Some(execution.as_str()) {
        return Err(corruption("peer terminal index is missing or mismatched"));
    }
    Ok(())
}

pub(super) fn request_key(
    owner: &PeerId,
    request: &PeerRequestId,
) -> Result<Vec<u8>, PersistenceError> {
    codec::pair(owner.as_str(), request.as_str())
}

pub(super) fn observation_key(
    execution: &PeerExecutionId,
    sequence: u64,
) -> Result<Vec<u8>, PersistenceError> {
    if sequence == 0 {
        return Err(invalid("peer observation sequence must be nonzero"));
    }
    ordered_key(sequence, execution.as_str())
}

pub(super) fn available_key(record: &PeerExecutionRecord) -> Result<Vec<u8>, PersistenceError> {
    ordered_key(record.acceptance_sequence, record.execution.as_str())
}

pub(super) fn claim_key(
    execution: &PeerExecutionId,
    claim: &PeerDispatchClaim,
) -> Result<Vec<u8>, PersistenceError> {
    ordered_key(claim.lease_expires_at_unix_ms, execution.as_str())
}

pub(super) fn ordered_key(number: u64, identity: &str) -> Result<Vec<u8>, PersistenceError> {
    let mut key = number.to_be_bytes().to_vec();
    key.extend_from_slice(&codec::component(identity)?);
    Ok(key)
}

pub(super) fn decode_ordered_time(key: &[u8]) -> Result<u64, PersistenceError> {
    let bytes: [u8; 8] = key
        .get(..8)
        .ok_or_else(|| corruption("ordered peer index key is truncated"))?
        .try_into()
        .map_err(|_| corruption("ordered peer index key is malformed"))?;
    Ok(u64::from_be_bytes(bytes))
}

pub(super) fn bump_record(record: &mut PeerExecutionRecord) -> Result<(), PersistenceError> {
    record.revision = record
        .revision
        .checked_add(1)
        .ok_or_else(|| corruption("peer execution revision overflowed"))?;
    Ok(())
}

pub(super) fn observation_genesis_digest() -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(OBSERVATION_DIGEST_DOMAIN);
    hasher.update(b"genesis");
    format!("b3_{}", hasher.finalize().to_hex())
}

pub(super) fn observation_link_digest(
    previous: &str,
    observation_document: &[u8],
) -> Result<String, PersistenceError> {
    if !milkdrift_contracts::is_canonical_blake3_digest(previous) {
        return Err(corruption("peer observation history digest is invalid"));
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(OBSERVATION_DIGEST_DOMAIN);
    hasher.update(previous.as_bytes());
    hasher.update(observation_document);
    Ok(format!("b3_{}", hasher.finalize().to_hex()))
}

pub(super) fn invalid(message: &str) -> PersistenceError {
    PersistenceError::InvalidDocument(message.to_owned())
}

pub(super) fn missing(entity: &'static str, identity: &str) -> PersistenceError {
    PersistenceError::NotFound {
        entity,
        identity: identity.to_owned(),
    }
}

pub(super) fn corruption(message: impl Into<String>) -> PersistenceError {
    PersistenceError::Storage {
        class: StorageFailureClass::Corruption,
        message: message.into(),
    }
}
use redb::ReadableTable as _;

pub(super) fn snapshot_optional_in_transaction(
    transaction: &redb::WriteTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<PeerExecutionSnapshot>, PersistenceError> {
    read_snapshot(
        execution,
        &transaction
            .open_table(PEER_EXECUTION_LOCATIONS)
            .map_err(error::redb)?,
        &transaction
            .open_table(PEER_EXECUTIONS)
            .map_err(error::redb)?,
        &transaction
            .open_table(PEER_EXECUTION_TOMBSTONES)
            .map_err(error::redb)?,
    )
}

pub(super) fn snapshot_optional_in_read_transaction(
    transaction: &redb::ReadTransaction,
    execution: &PeerExecutionId,
) -> Result<Option<PeerExecutionSnapshot>, PersistenceError> {
    read_snapshot(
        execution,
        &transaction
            .open_table(PEER_EXECUTION_LOCATIONS)
            .map_err(error::redb)?,
        &transaction
            .open_table(PEER_EXECUTIONS)
            .map_err(error::redb)?,
        &transaction
            .open_table(PEER_EXECUTION_TOMBSTONES)
            .map_err(error::redb)?,
    )
}

// The location index selects exactly one durable authority in either transaction kind.
fn read_snapshot(
    execution: &PeerExecutionId,
    locations: &impl redb::ReadableTable<&'static str, u8>,
    hot: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    archived: &impl redb::ReadableTable<&'static str, &'static [u8]>,
) -> Result<Option<PeerExecutionSnapshot>, PersistenceError> {
    let key = execution.as_str();
    let location = locations
        .get(key)
        .map_err(error::redb)?
        .map(|value| value.value());
    match location {
        None => {
            let hot = hot.get(key).map_err(error::redb)?.is_some();
            let archived = archived.get(key).map_err(error::redb)?.is_some();
            if hot || archived {
                return Err(corruption("peer execution exists without a location index"));
            }
            Ok(None)
        }
        Some(LOCATION_HOT) => hot
            .get(key)
            .map_err(error::redb)?
            .map(|bytes| decode_record(bytes.value()))
            .transpose()?
            .map(|record| PeerExecutionSnapshot::Hot(Box::new(record)))
            .ok_or_else(|| corruption("peer hot location points at a missing record"))
            .map(Some),
        Some(LOCATION_ARCHIVED) => archived
            .get(key)
            .map_err(error::redb)?
            .map(|bytes| decode_tombstone(bytes.value()))
            .transpose()?
            .map(|tombstone| PeerExecutionSnapshot::Archived(Box::new(tombstone)))
            .ok_or_else(|| corruption("peer archived location points at a missing tombstone"))
            .map(Some),
        Some(_) => Err(corruption("peer execution location has an unknown value")),
    }
}

#[cfg(test)]
mod tests;
