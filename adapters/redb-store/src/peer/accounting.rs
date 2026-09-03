//! Durable global and per-peer admission accounting.

use milkdrift_capability::PeerId;
use milkdrift_persistence::PersistenceError;
use redb::ReadableTable;
use serde::{Deserialize, Serialize};

use super::corruption;
use crate::{
    error, json,
    schema::{PEER_EXECUTION_ACCOUNTING, PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY},
};

pub(super) const PEER_ACCOUNTING_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GlobalPeerAccounting {
    pub(super) schema_version: u32,
    pub(super) next_acceptance_sequence: u64,
    pub(super) active: u32,
    pub(super) dispatch_queued: u32,
    pub(super) hot_terminal: u64,
    pub(super) tombstones: u64,
    pub(super) archive_generation: u64,
    pub(super) last_archived_at_unix_ms: Option<u64>,
    pub(super) admission_open: bool,
}

impl GlobalPeerAccounting {
    pub(crate) const EMPTY: Self = Self {
        schema_version: PEER_ACCOUNTING_SCHEMA_VERSION,
        next_acceptance_sequence: 1,
        active: 0,
        dispatch_queued: 0,
        hot_terminal: 0,
        tombstones: 0,
        archive_generation: 0,
        last_archived_at_unix_ms: None,
        admission_open: false,
    };
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PerPeerAccounting {
    pub(super) schema_version: u32,
    pub(super) peer: PeerId,
    pub(super) active: u32,
    pub(super) revision: u64,
}

impl PerPeerAccounting {
    fn empty(peer: &PeerId) -> Self {
        Self {
            schema_version: PEER_ACCOUNTING_SCHEMA_VERSION,
            peer: peer.clone(),
            active: 0,
            revision: 0,
        }
    }
}

pub(super) fn global_accounting(
    write: &redb::WriteTransaction,
) -> Result<GlobalPeerAccounting, PersistenceError> {
    let table = write
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?;
    let bytes = table
        .get(PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| corruption("peer global accounting is missing"))?;
    let value: GlobalPeerAccounting = json::decode(bytes.value(), "peer global accounting")?;
    validate_global_accounting(value)
}

pub(super) fn global_accounting_read(
    read: &redb::ReadTransaction,
) -> Result<GlobalPeerAccounting, PersistenceError> {
    let table = read
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?;
    let bytes = table
        .get(PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| corruption("peer global accounting is missing"))?;
    let value: GlobalPeerAccounting = json::decode(bytes.value(), "peer global accounting")?;
    validate_global_accounting(value)
}

fn validate_global_accounting(
    value: GlobalPeerAccounting,
) -> Result<GlobalPeerAccounting, PersistenceError> {
    if value.schema_version != PEER_ACCOUNTING_SCHEMA_VERSION
        || value.next_acceptance_sequence == 0
        || (value.archive_generation == 0) != value.last_archived_at_unix_ms.is_none()
    {
        return Err(corruption("peer global accounting schema is invalid"));
    }
    Ok(value)
}

pub(super) fn put_global_accounting(
    write: &redb::WriteTransaction,
    value: GlobalPeerAccounting,
) -> Result<(), PersistenceError> {
    let bytes = json::encode(&value, "peer global accounting")?;
    write
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?
        .insert(PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY, bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

pub(super) fn peer_accounting(
    write: &redb::WriteTransaction,
    peer: &PeerId,
) -> Result<PerPeerAccounting, PersistenceError> {
    let value = write
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?
        .get(peer.as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode(bytes.value(), "peer relationship accounting"))
        .transpose()?
        .unwrap_or_else(|| PerPeerAccounting::empty(peer));
    if value.schema_version != PEER_ACCOUNTING_SCHEMA_VERSION || value.peer != *peer {
        return Err(corruption("per-peer accounting schema or key is invalid"));
    }
    Ok(value)
}

pub(super) fn put_peer_accounting(
    write: &redb::WriteTransaction,
    value: &PerPeerAccounting,
) -> Result<(), PersistenceError> {
    let bytes = json::encode(value, "peer relationship accounting")?;
    write
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?
        .insert(value.peer.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

pub(super) fn release_active_accounting(
    write: &redb::WriteTransaction,
    owner: &PeerId,
    decrement_dispatch: bool,
) -> Result<(), PersistenceError> {
    let mut global = global_accounting(write)?;
    let mut peer = peer_accounting(write, owner)?;
    global.active = global
        .active
        .checked_sub(1)
        .ok_or_else(|| corruption("peer global active count underflowed"))?;
    if decrement_dispatch {
        global.dispatch_queued = global
            .dispatch_queued
            .checked_sub(1)
            .ok_or_else(|| corruption("peer dispatch count underflowed"))?;
    }
    global.hot_terminal = global
        .hot_terminal
        .checked_add(1)
        .ok_or_else(|| corruption("peer hot terminal count overflowed"))?;
    peer.active = peer
        .active
        .checked_sub(1)
        .ok_or_else(|| corruption("per-peer active count underflowed"))?;
    peer.revision = peer
        .revision
        .checked_add(1)
        .ok_or_else(|| corruption("per-peer accounting revision overflowed"))?;
    put_global_accounting(write, global)?;
    put_peer_accounting(write, &peer)
}
