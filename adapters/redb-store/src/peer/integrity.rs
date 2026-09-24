//! Whole-store peer execution integrity verification.

use std::collections::BTreeMap;

use milkdrift_peer_protocol::PeerObservation;
use milkdrift_peer_protocol::ServingCaller;
use milkdrift_persistence::{
    PeerDispatchClaim, PeerExecutionPhase, PeerExecutionRecord, PeerExecutionSnapshot,
    PersistenceError,
};
use redb::{ReadableTable, ReadableTableMetadata};

use super::{
    LOCATION_ARCHIVED, LOCATION_HOT,
    accounting::{PEER_ACCOUNTING_SCHEMA_VERSION, PerPeerAccounting, global_accounting_read},
    available_key, claim_key, corruption, decode_record, decode_tombstone,
    observation_genesis_digest, observation_key, observation_link_digest, ordered_key, request_key,
    snapshot_in_read_transaction_text,
};
use crate::{
    RedbStore, error, json,
    schema::{
        PEER_ACTIVE_CLAIMS, PEER_DISPATCH_AVAILABLE, PEER_EXECUTION_ACCOUNTING,
        PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY, PEER_EXECUTION_LOCATIONS, PEER_EXECUTION_TOMBSTONES,
        PEER_EXECUTIONS, PEER_EXECUTIONS_BY_REQUEST, PEER_OBSERVATION_ARTIFACTS, PEER_OBSERVATIONS,
        PEER_TERMINAL_INDEX,
    },
};

pub(super) fn verify(store: &RedbStore) -> Result<(), PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    let identity = read
        .open_table(crate::schema::SERVING_HOST_IDENTITY)
        .map_err(error::redb)?;
    let host = identity
        .get("host")
        .map_err(error::redb)?
        .map(|value| {
            milkdrift_capability::PeerId::new(value.value())
                .map_err(|_| corruption("invalid serving host identity"))
        })
        .transpose()?;
    if identity.len().map_err(error::redb)? != u64::from(host.is_some()) {
        return Err(corruption("invalid serving host identity membership"));
    }
    let callers = read
        .open_table(crate::schema::PEER_RELATIONSHIPS)
        .map_err(error::redb)?;
    for row in callers.iter().map_err(error::redb)? {
        let (key, bytes) = row.map_err(error::redb)?;
        let state: milkdrift_persistence::ServingCallerState =
            json::decode(bytes.value(), "peer relationship")?;
        super::validate_relationship(&state)?;
        if host.as_ref() != Some(&state.caller.host) || key.value() != state.caller.storage_key() {
            return Err(corruption(
                "serving caller belongs to another installation or key",
            ));
        }
    }
    let catalogs = read
        .open_table(crate::schema::PEER_CATALOGS)
        .map_err(error::redb)?;
    for row in catalogs.iter().map_err(error::redb)? {
        let (key, bytes) = row.map_err(error::redb)?;
        let state: milkdrift_persistence::ServingCatalogState =
            json::decode(bytes.value(), "peer catalog")?;
        super::validate_catalog(&state)?;
        if host.as_ref() != Some(&state.caller.host)
            || key.value() != state.caller.storage_key()
            || callers.get(key.value()).map_err(error::redb)?.is_none()
        {
            return Err(corruption(
                "serving catalog has no exact caller installation",
            ));
        }
    }
    let global = global_accounting_read(&read)?;
    let hot = read.open_table(PEER_EXECUTIONS).map_err(error::redb)?;
    let tombstones = read
        .open_table(PEER_EXECUTION_TOMBSTONES)
        .map_err(error::redb)?;
    let locations = read
        .open_table(PEER_EXECUTION_LOCATIONS)
        .map_err(error::redb)?;
    let requests = read
        .open_table(PEER_EXECUTIONS_BY_REQUEST)
        .map_err(error::redb)?;
    let available = read
        .open_table(PEER_DISPATCH_AVAILABLE)
        .map_err(error::redb)?;
    let claims = read.open_table(PEER_ACTIVE_CLAIMS).map_err(error::redb)?;
    let terminal = read.open_table(PEER_TERMINAL_INDEX).map_err(error::redb)?;
    let observations = read.open_table(PEER_OBSERVATIONS).map_err(error::redb)?;
    let observation_artifacts = read
        .open_table(PEER_OBSERVATION_ARTIFACTS)
        .map_err(error::redb)?;

    let mut active_by_peer = BTreeMap::<ServingCaller, u32>::new();
    let mut dispatch_count = 0_u32;
    let mut hot_terminal_count = 0_u64;
    let mut available_index_count = 0_u64;
    let mut claim_index_count = 0_u64;
    let mut terminal_index_count = 0_u64;
    let mut observation_count = 0_u64;
    let mut observation_artifact_count = 0_u64;
    for row in hot.iter().map_err(error::redb)? {
        let (key, bytes) = row.map_err(error::redb)?;
        let record = decode_record(bytes.value())?;
        if host.as_ref() != Some(&record.caller.host) {
            return Err(corruption(
                "accepted invocation belongs to another serving installation",
            ));
        }
        if key.value() != record.execution.as_str()
            || locations
                .get(record.execution.as_str())
                .map_err(error::redb)?
                .map(|value| value.value())
                != Some(LOCATION_HOT)
            || tombstones
                .get(record.execution.as_str())
                .map_err(error::redb)?
                .is_some()
        {
            return Err(corruption(
                "peer hot record key/location/tombstone ownership is inconsistent",
            ));
        }
        let request_index_key = request_key(&record.caller, &record.request.request_id)?;
        if requests
            .get(request_index_key.as_slice())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned())
            .as_deref()
            != Some(record.execution.as_str())
        {
            return Err(corruption("peer hot record request index is inconsistent"));
        }
        let mut recomputed_digest = observation_genesis_digest();
        let mut outputs = 0;
        observation_count = observation_count
            .checked_add(record.last_observation_sequence)
            .ok_or_else(|| corruption("peer integrity observation count overflowed"))?;
        for sequence in 1..=record.last_observation_sequence {
            let observation_key = observation_key(&record.execution, sequence)?;
            let stored = observations
                .get(observation_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| corruption("peer hot observation history has a missing row"))?;
            let observation: PeerObservation = json::decode(stored.value(), "peer observation")?;
            if observation.execution != record.execution || observation.sequence != sequence {
                return Err(corruption(
                    "peer observation row key/document integrity is inconsistent",
                ));
            }
            let stored_artifact = observation_artifacts
                .get(observation_key.as_slice())
                .map_err(error::redb)?;
            match (observation.event.kind().output(), stored_artifact) {
                (Some((_name, expected)), Some(stored)) => {
                    outputs += 1;
                    let stored: milkdrift_capability::ArtifactReference =
                        json::decode(stored.value(), "peer observation artifact")?;
                    if stored != *expected {
                        return Err(corruption(
                            "peer observation artifact mapping is inconsistent",
                        ));
                    }
                    observation_artifact_count =
                        observation_artifact_count.checked_add(1).ok_or_else(|| {
                            corruption("peer integrity artifact mapping count overflowed")
                        })?;
                }
                (None, None) => {}
                (Some(_), None) | (None, Some(_)) => {
                    return Err(corruption(
                        "peer observation artifact mapping presence is inconsistent",
                    ));
                }
            }
            recomputed_digest = observation_link_digest(&recomputed_digest, stored.value())?;
        }
        if outputs != record.accounting.outputs {
            return Err(corruption(
                "peer output count differs from retained observations",
            ));
        }
        if recomputed_digest != record.observation_digest {
            return Err(corruption(
                "peer observation history digest is inconsistent",
            ));
        }
        if record.phase.is_active() {
            let peer_active = active_by_peer.entry(record.caller.clone()).or_default();
            *peer_active = peer_active
                .checked_add(1)
                .ok_or_else(|| corruption("peer integrity per-peer count overflowed"))?;
        }
        match &record.phase {
            PeerExecutionPhase::AwaitingWorkflow { .. }
            | PeerExecutionPhase::CancellationRequested {
                claim: None,
                evidence: Some(_),
            } if record.published_invocation.is_some() => {}
            PeerExecutionPhase::AwaitingWorkflow { .. } => {
                return Err(corruption("pending workflow has no saved association"));
            }
            PeerExecutionPhase::DispatchAvailable { .. }
            | PeerExecutionPhase::CancellationRequested {
                claim: None,
                evidence: None,
            } => {
                dispatch_count = dispatch_count.saturating_add(1);
                available_index_count = available_index_count.saturating_add(1);
                let key = available_key(&record)?;
                if available
                    .get(key.as_slice())
                    .map_err(error::redb)?
                    .map(|value| value.value().to_owned())
                    .as_deref()
                    != Some(record.execution.as_str())
                {
                    return Err(corruption("peer dispatch-available index is inconsistent"));
                }
            }
            PeerExecutionPhase::DispatchClaimed { claim } => {
                dispatch_count = dispatch_count.saturating_add(1);
                claim_index_count = claim_index_count.saturating_add(1);
                verify_claim_index(&claims, &record, claim)?;
            }
            PeerExecutionPhase::Entered { claim, .. }
            | PeerExecutionPhase::CancellationRequested {
                claim: Some(claim), ..
            } => {
                claim_index_count = claim_index_count.saturating_add(1);
                verify_claim_index(&claims, &record, claim)?;
            }
            PeerExecutionPhase::Terminal {
                terminal_at_unix_ms,
                ..
            } => {
                hot_terminal_count = hot_terminal_count.saturating_add(1);
                terminal_index_count = terminal_index_count.saturating_add(1);
                verify_terminal_index(&terminal, &record, *terminal_at_unix_ms)?;
            }
            PeerExecutionPhase::Uncertain {
                uncertain_at_unix_ms,
                ..
            } => {
                hot_terminal_count = hot_terminal_count.saturating_add(1);
                terminal_index_count = terminal_index_count.saturating_add(1);
                verify_terminal_index(&terminal, &record, *uncertain_at_unix_ms)?;
            }
            PeerExecutionPhase::CancellationRequested { .. } => {
                return Err(corruption(
                    "peer cancellation phase has inconsistent claim evidence",
                ));
            }
        }
    }

    let mut tombstone_count = 0_u64;
    for row in tombstones.iter().map_err(error::redb)? {
        let (key, bytes) = row.map_err(error::redb)?;
        let tombstone = decode_tombstone(bytes.value())?;
        if host.as_ref() != Some(&tombstone.caller.host) {
            return Err(corruption(
                "archived invocation belongs to another serving installation",
            ));
        }
        tombstone_count = tombstone_count.saturating_add(1);
        if key.value() != tombstone.execution.as_str()
            || locations
                .get(tombstone.execution.as_str())
                .map_err(error::redb)?
                .map(|value| value.value())
                != Some(LOCATION_ARCHIVED)
            || hot
                .get(tombstone.execution.as_str())
                .map_err(error::redb)?
                .is_some()
        {
            return Err(corruption(
                "peer tombstone key/location/hot ownership is inconsistent",
            ));
        }
        let request_index_key = request_key(&tombstone.caller, &tombstone.request_id)?;
        if requests
            .get(request_index_key.as_slice())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned())
            .as_deref()
            != Some(tombstone.execution.as_str())
        {
            return Err(corruption("peer tombstone request index is inconsistent"));
        }
        if tombstone.last_observation_sequence > 0 {
            let last = observation_key(&tombstone.execution, tombstone.last_observation_sequence)?;
            if observations
                .get(last.as_slice())
                .map_err(error::redb)?
                .is_some()
                || observation_artifacts
                    .get(last.as_slice())
                    .map_err(error::redb)?
                    .is_some()
            {
                return Err(corruption(
                    "archived peer execution retained a compacted hot observation row",
                ));
            }
        }
    }

    let active_count = active_by_peer.values().try_fold(0_u32, |total, count| {
        total
            .checked_add(*count)
            .ok_or_else(|| corruption("peer integrity active count overflowed"))
    })?;
    if active_count != global.active
        || dispatch_count != global.dispatch_queued
        || hot_terminal_count != global.hot_terminal
        || tombstone_count != global.tombstones
        || available.len().map_err(error::redb)? != available_index_count
        || claims.len().map_err(error::redb)? != claim_index_count
        || terminal.len().map_err(error::redb)? != terminal_index_count
        || observations.len().map_err(error::redb)? != observation_count
        || observation_artifacts.len().map_err(error::redb)? != observation_artifact_count
        || hot.len().map_err(error::redb)?
            != u64::from(global.active).saturating_add(global.hot_terminal)
        || locations.len().map_err(error::redb)?
            != hot
                .len()
                .map_err(error::redb)?
                .saturating_add(tombstone_count)
        || requests.len().map_err(error::redb)? != locations.len().map_err(error::redb)?
    {
        return Err(corruption(
            "peer global counters or primary index cardinality drifted",
        ));
    }

    verify_peer_indexes(&read)?;
    verify_per_peer_accounting(&read, active_by_peer)?;
    Ok(())
}

fn verify_claim_index(
    claims: &impl ReadableTable<&'static [u8], &'static str>,
    record: &PeerExecutionRecord,
    claim: &PeerDispatchClaim,
) -> Result<(), PersistenceError> {
    let key = claim_key(&record.execution, claim)?;
    if claims
        .get(key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned())
        .as_deref()
        != Some(record.execution.as_str())
    {
        return Err(corruption("peer active-claim index is inconsistent"));
    }
    Ok(())
}

fn verify_terminal_index(
    terminal: &impl ReadableTable<&'static [u8], &'static str>,
    record: &PeerExecutionRecord,
    terminal_at: u64,
) -> Result<(), PersistenceError> {
    let key = ordered_key(terminal_at, record.execution.as_str())?;
    if terminal
        .get(key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned())
        .as_deref()
        != Some(record.execution.as_str())
    {
        return Err(corruption("peer hot-terminal index is inconsistent"));
    }
    Ok(())
}

fn verify_per_peer_accounting(
    read: &redb::ReadTransaction,
    mut active_by_peer: BTreeMap<ServingCaller, u32>,
) -> Result<(), PersistenceError> {
    let accounting = read
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?;
    for row in accounting.iter().map_err(error::redb)? {
        let (key, bytes) = row.map_err(error::redb)?;
        if key.value() == PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY {
            continue;
        }
        let value: PerPeerAccounting = json::decode(bytes.value(), "peer relationship accounting")?;
        if value.peer.storage_key().as_str() != key.value()
            || value.schema_version != PEER_ACCOUNTING_SCHEMA_VERSION
        {
            return Err(corruption(
                "per-peer accounting row key/schema is inconsistent",
            ));
        }
        let actual = active_by_peer.remove(&value.peer).unwrap_or(0);
        if actual != value.active {
            return Err(corruption("per-peer active accounting drifted"));
        }
    }
    if !active_by_peer.is_empty() {
        return Err(corruption("active peer record has no per-peer accounting"));
    }
    Ok(())
}

fn verify_peer_indexes(read: &redb::ReadTransaction) -> Result<(), PersistenceError> {
    let requests = read
        .open_table(PEER_EXECUTIONS_BY_REQUEST)
        .map_err(error::redb)?;
    for row in requests.iter().map_err(error::redb)? {
        let (stored_key, execution) = row.map_err(error::redb)?;
        let snapshot = snapshot_in_read_transaction_text(read, execution.value())?;
        if request_key(snapshot.caller(), snapshot.request_id())? != stored_key.value() {
            return Err(corruption(
                "peer request index key disagrees with its authority",
            ));
        }
    }
    let locations = read
        .open_table(PEER_EXECUTION_LOCATIONS)
        .map_err(error::redb)?;
    for row in locations.iter().map_err(error::redb)? {
        let (execution, location) = row.map_err(error::redb)?;
        let snapshot = snapshot_in_read_transaction_text(read, execution.value())?;
        if !matches!(
            (location.value(), snapshot),
            (LOCATION_HOT, PeerExecutionSnapshot::Hot(_))
                | (LOCATION_ARCHIVED, PeerExecutionSnapshot::Archived(_))
        ) {
            return Err(corruption(
                "peer execution location points at the wrong authority",
            ));
        }
    }
    Ok(())
}
