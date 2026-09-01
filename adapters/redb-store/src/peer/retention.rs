//! Atomic compaction of terminal peer executions into durable tombstones.

use milkdrift_peer_protocol::PeerObservation;
use milkdrift_persistence::{
    PEER_EXECUTION_TOMBSTONE_SCHEMA_VERSION_V1, PeerAcceptedAuthoritySummary,
    PeerArchivedDisposition, PeerExecutionPhase, PeerExecutionRecord, PeerExecutionTombstone,
    PeerRetentionPage, PersistenceError,
};
use redb::ReadableTable;

use super::{
    LOCATION_ARCHIVED, LOCATION_HOT,
    accounting::{global_accounting, put_global_accounting},
    corruption, decode_ordered_time, execution_location_in_transaction,
    execution_optional_in_transaction, invalid, observation_key, parse_execution_id, put_tombstone,
    remove_terminal_index, request_key, tombstone_optional_in_transaction,
    validation::validate_tombstone,
};
use crate::{
    RedbStore, error,
    fault::FaultPoint,
    json,
    schema::{
        PEER_EXECUTION_LOCATIONS, PEER_EXECUTIONS, PEER_EXECUTIONS_BY_REQUEST,
        PEER_OBSERVATION_ARTIFACTS, PEER_OBSERVATIONS, PEER_TERMINAL_INDEX,
    },
};

pub(super) fn archive_eligible_in_transaction(
    store: &RedbStore,
    write: &redb::WriteTransaction,
    terminal_before_or_at: u64,
    archived_at: u64,
    limit: u32,
) -> Result<PeerRetentionPage, PersistenceError> {
    if terminal_before_or_at == 0 || archived_at == 0 || limit == 0 {
        return Err(invalid(
            "peer archival boundaries and limit must be nonzero",
        ));
    }
    let candidates = {
        let terminal = write.open_table(PEER_TERMINAL_INDEX).map_err(error::redb)?;
        terminal
            .iter()
            .map_err(error::redb)?
            .take(limit as usize + 1)
            .map(|row| {
                row.map(|(key, execution)| (key.value().to_vec(), execution.value().to_owned()))
                    .map_err(error::redb)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut archived = 0_u32;
    let mut more = false;
    for (terminal_key, execution_text) in candidates {
        let terminal_at = decode_ordered_time(&terminal_key)?;
        if terminal_at > terminal_before_or_at {
            break;
        }
        if archived >= limit {
            more = true;
            break;
        }
        let execution = parse_execution_id(&execution_text)?;
        if execution_location_in_transaction(write, &execution)? != Some(LOCATION_HOT) {
            return Err(corruption(
                "peer terminal index does not point at singular hot ownership",
            ));
        }
        if tombstone_optional_in_transaction(write, &execution)?.is_some() {
            return Err(corruption(
                "peer execution has hot and tombstone dual ownership",
            ));
        }
        let record = execution_optional_in_transaction(write, &execution)?
            .ok_or_else(|| corruption("peer terminal index points at a missing hot record"))?;
        if !matches!(
            record.phase,
            PeerExecutionPhase::Terminal { .. } | PeerExecutionPhase::Uncertain { .. }
        ) {
            return Err(corruption("peer terminal index points at an active record"));
        }
        let request_key = request_key(&record.owner_peer, &record.request.request_id)?;
        let indexed_execution = write
            .open_table(PEER_EXECUTIONS_BY_REQUEST)
            .map_err(error::redb)?
            .get(request_key.as_slice())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned());
        if indexed_execution.as_deref() != Some(record.execution.as_str()) {
            return Err(corruption(
                "peer request index disagrees before tombstone creation",
            ));
        }
        let tombstone = tombstone_from_record(write, &record, archived_at)?;
        put_tombstone(write, &tombstone)?;
        store.faults.check(FaultPoint::AfterPeerTombstoneInsert)?;
        let previous_location = write
            .open_table(PEER_EXECUTION_LOCATIONS)
            .map_err(error::redb)?
            .insert(execution.as_str(), LOCATION_ARCHIVED)
            .map_err(error::redb)?
            .map(|value| value.value());
        if previous_location != Some(LOCATION_HOT) {
            return Err(corruption(
                "peer archive location transition lost hot ownership",
            ));
        }
        compact_observation_rows(write, &record)?;
        store
            .faults
            .check(FaultPoint::AfterPeerObservationCleanup)?;
        remove_terminal_index(write, &record.execution, terminal_at)?;
        let mut hot = write.open_table(PEER_EXECUTIONS).map_err(error::redb)?;
        let removed = hot.remove(execution.as_str()).map_err(error::redb)?;
        if removed.is_none() {
            return Err(corruption("peer hot record disappeared during archival"));
        }
        store.faults.check(FaultPoint::AfterPeerHotRemove)?;
        let mut global = global_accounting(write)?;
        global.hot_terminal = global
            .hot_terminal
            .checked_sub(1)
            .ok_or_else(|| corruption("peer hot terminal count underflowed during archival"))?;
        global.tombstones = global
            .tombstones
            .checked_add(1)
            .ok_or_else(|| corruption("peer tombstone count overflowed"))?;
        put_global_accounting(write, global)?;
        store.faults.check(FaultPoint::AfterPeerArchiveAccounting)?;
        archived = archived.saturating_add(1);
    }
    if archived > 0 {
        let mut global = global_accounting(write)?;
        global.archive_generation = global
            .archive_generation
            .checked_add(1)
            .ok_or_else(|| corruption("peer archive generation overflowed"))?;
        global.last_archived_at_unix_ms = Some(archived_at);
        put_global_accounting(write, global)?;
    }
    Ok(PeerRetentionPage { archived, more })
}

fn tombstone_from_record(
    write: &redb::WriteTransaction,
    record: &PeerExecutionRecord,
    archived_at_unix_ms: u64,
) -> Result<PeerExecutionTombstone, PersistenceError> {
    let disposition = match &record.phase {
        PeerExecutionPhase::Terminal { sequence, .. } => {
            let key = observation_key(&record.execution, *sequence)?;
            let observations = write.open_table(PEER_OBSERVATIONS).map_err(error::redb)?;
            let observation = observations
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| corruption("terminal peer record lacks its final observation"))?;
            let observation: PeerObservation =
                json::decode(observation.value(), "peer observation")?;
            PeerArchivedDisposition::Terminal {
                observation: Box::new(observation),
            }
        }
        PeerExecutionPhase::Uncertain {
            uncertain_at_unix_ms,
            reason,
        } => PeerArchivedDisposition::Uncertain {
            uncertain_at_unix_ms: *uncertain_at_unix_ms,
            reason: reason.clone(),
        },
        _ => return Err(corruption("active peer record cannot become a tombstone")),
    };
    let operation = record.request.selection.operation_contract();
    let tombstone = PeerExecutionTombstone {
        schema_version: PEER_EXECUTION_TOMBSTONE_SCHEMA_VERSION_V1,
        owner_peer: record.owner_peer.clone(),
        target_peer: record.request.delegation.target_peer.clone(),
        delegation_ref: record.request.delegation.reference.clone(),
        relationship_generation: record.relationship_generation,
        request_id: record.request.request_id.clone(),
        request_digest: record.request.request_digest.clone(),
        execution: record.execution.clone(),
        acceptance_sequence: record.acceptance_sequence,
        accepted_at_unix_ms: record.accepted_at_unix_ms,
        catalog_generation: record.request.catalog_generation,
        catalog_digest: record.request.catalog_digest.as_str().to_owned(),
        capability: record.request.selection.capability().clone(),
        capability_generation: record.request.selection.descriptor_revision(),
        capability_digest: record.request.selection.digest().to_owned(),
        operation: record.request.selection.operation().clone(),
        side_effect: operation.side_effect(),
        idempotency: operation.idempotency(),
        authority: PeerAcceptedAuthoritySummary {
            decision: record.authority.request().decision.clone(),
            actor: record.authority.request().actor.clone(),
            grant: record.authority.request().grant.clone(),
            grant_revision: record.authority.request().grant_revision,
            grant_digest: record.authority.request().grant_digest.clone(),
            revocation_generation: record.authority.request().revocation_generation,
            policy: record.authority.policy().clone(),
            policy_version: record.authority.policy_version(),
            decision_digest: record.authority.digest().to_owned(),
        },
        provenance: record.request.delegation.provenance.clone(),
        disposition,
        cancellation: record.cancellation.clone(),
        last_observation_sequence: record.last_observation_sequence,
        observation_digest: record.observation_digest.clone(),
        accounting: record.accounting,
        compacted_through_sequence: record.last_observation_sequence,
        archived_at_unix_ms,
    };
    validate_tombstone(&tombstone)?;
    Ok(tombstone)
}

fn compact_observation_rows(
    write: &redb::WriteTransaction,
    record: &PeerExecutionRecord,
) -> Result<(), PersistenceError> {
    let mut observations = write.open_table(PEER_OBSERVATIONS).map_err(error::redb)?;
    let mut artifacts = write
        .open_table(PEER_OBSERVATION_ARTIFACTS)
        .map_err(error::redb)?;
    for sequence in 1..=record.last_observation_sequence {
        let key = observation_key(&record.execution, sequence)?;
        if observations
            .remove(key.as_slice())
            .map_err(error::redb)?
            .is_none()
        {
            return Err(corruption(
                "peer archival encountered a missing observation row",
            ));
        }
        artifacts.remove(key.as_slice()).map_err(error::redb)?;
    }
    Ok(())
}
