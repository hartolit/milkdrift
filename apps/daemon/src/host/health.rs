//! Coherent, non-authoritative daemon health projection and queue accounting.

use std::sync::{Arc, Mutex, MutexGuard};

use milkdrift_control_protocol::{
    ApplicationReceiptHealthRead, DaemonState, HealthRead, PeerExecutionHealthRead,
};
use milkdrift_persistence::{ApplicationReceiptStatus, PeerExecutionStatus, TimestampMillis};

use crate::config::{PeerHostConfig, PeerServingConfig, StoragePlan};

use super::read_model::bounded;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Lifecycle {
    Starting,
    Ready,
    Draining,
    Stopped,
    Failed,
}

impl Lifecycle {
    const fn public(self) -> DaemonState {
        match self {
            Self::Starting => DaemonState::Starting,
            Self::Ready => DaemonState::Ready,
            Self::Draining => DaemonState::Draining,
            Self::Stopped => DaemonState::Stopped,
            Self::Failed => DaemonState::Failed,
        }
    }
}

struct ReceiptHealth {
    status: ApplicationReceiptStatus,
    archival_degraded: bool,
    last_archival_failure: Option<String>,
}

impl ReceiptHealth {
    fn new(storage: &StoragePlan) -> Self {
        Self {
            status: ApplicationReceiptStatus {
                hot_count: 0,
                cold_count: 0,
                hot_bound: storage.application_receipts.hot_receipt_bound,
                archive_batch_size: storage.application_receipts.archive_batch_size,
                archive_generation: 0,
                last_archived_at: None,
            },
            archival_degraded: false,
            last_archival_failure: None,
        }
    }

    fn read(&self) -> ApplicationReceiptHealthRead {
        ApplicationReceiptHealthRead {
            hot_count: self.status.hot_count,
            hot_bound: self.status.hot_bound,
            archive_batch_size: self.status.archive_batch_size,
            cold_count: self.status.cold_count,
            archive_generation: self.status.archive_generation,
            last_archived_at_unix_ms: self.status.last_archived_at.map(TimestampMillis::get),
            archival_degraded: self.archival_degraded,
            last_archival_failure: self.last_archival_failure.clone(),
        }
    }
}

struct PeerHealth {
    enabled: bool,
    active_bound: u32,
    dispatch_bound: u32,
    hot_terminal_bound: u64,
    archive_batch_size: u32,
    observation_hot_retention_ms: u64,
    status: PeerExecutionStatus,
    archival_degraded: bool,
    last_archival_failure: Option<String>,
}

impl PeerHealth {
    fn new(peers: &PeerHostConfig) -> Self {
        let serving = peer_serving(peers);
        Self {
            enabled: serving.is_some(),
            active_bound: serving.map_or(0, |policy| policy.maximum_global_active),
            dispatch_bound: serving.map_or(0, |policy| policy.maximum_dispatch_queue),
            hot_terminal_bound: serving.map_or(0, |policy| policy.maximum_hot_terminal_records),
            archive_batch_size: serving.map_or(0, |policy| policy.archive_batch_size),
            observation_hot_retention_ms: serving
                .map_or(0, |policy| policy.observation_hot_retention_ms),
            status: PeerExecutionStatus::default(),
            archival_degraded: false,
            last_archival_failure: None,
        }
    }

    fn read(&self) -> PeerExecutionHealthRead {
        PeerExecutionHealthRead {
            enabled: self.enabled,
            active_count: self.status.active,
            active_bound: self.active_bound,
            dispatch_queued: self.status.dispatch_queued,
            dispatch_bound: self.dispatch_bound,
            hot_terminal_count: self.status.hot_terminal,
            hot_terminal_bound: self.hot_terminal_bound,
            tombstone_count: self.status.tombstones,
            archive_batch_size: self.archive_batch_size,
            observation_hot_retention_ms: self.observation_hot_retention_ms,
            archive_generation: self.status.archive_generation,
            last_archived_at_unix_ms: self.status.last_archived_at_unix_ms,
            archival_degraded: self.archival_degraded,
            last_archival_failure: self.last_archival_failure.clone(),
        }
    }
}

struct HealthState {
    lifecycle: Lifecycle,
    queued_requests: u32,
    request_queue_capacity: u32,
    active_effects: u32,
    last_failure: Option<String>,
    application_receipts: ReceiptHealth,
    peer_executions: PeerHealth,
}

impl HealthState {
    fn read(&self) -> HealthRead {
        HealthRead {
            state: self.lifecycle.public(),
            live: !matches!(self.lifecycle, Lifecycle::Stopped | Lifecycle::Failed),
            ready: self.lifecycle == Lifecycle::Ready,
            draining: self.lifecycle == Lifecycle::Draining,
            queued_requests: self.queued_requests,
            request_queue_capacity: self.request_queue_capacity,
            active_effects: self.active_effects,
            last_failure: self.last_failure.clone(),
            application_receipts: self.application_receipts.read(),
            peer_executions: self.peer_executions.read(),
        }
    }
}

struct VersionedHealth {
    generation: u64,
    state: HealthState,
}

/// One synchronization owner for the complete operational health projection.
pub(super) struct SharedHealth {
    versioned: Mutex<VersionedHealth>,
}

impl SharedHealth {
    pub(super) fn new(
        request_queue_capacity: u32,
        storage: &StoragePlan,
        peers: &PeerHostConfig,
    ) -> Self {
        Self {
            versioned: Mutex::new(VersionedHealth {
                generation: 1,
                state: HealthState {
                    lifecycle: Lifecycle::Starting,
                    queued_requests: 0,
                    request_queue_capacity,
                    active_effects: 0,
                    last_failure: None,
                    application_receipts: ReceiptHealth::new(storage),
                    peer_executions: PeerHealth::new(peers),
                },
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, VersionedHealth> {
        match self.versioned.lock() {
            Ok(versioned) => versioned,
            // Health is a disposable operational projection. Preserve observability after a
            // panicking reader instead of turning mutex poison into a second failure authority.
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn update(&self, update: impl FnOnce(&mut HealthState) -> bool) {
        let mut versioned = self.lock();
        if update(&mut versioned.state) {
            versioned.generation = versioned.generation.saturating_add(1);
        }
    }

    pub(super) fn set_lifecycle(&self, lifecycle: Lifecycle) {
        self.update(|state| replace_if_changed(&mut state.lifecycle, lifecycle));
    }

    pub(super) fn is_ready(&self) -> bool {
        self.lock().state.lifecycle == Lifecycle::Ready
    }

    pub(super) fn failure(&self, message: &str) {
        let message = bounded(message);
        self.update(|state| replace_if_changed(&mut state.last_failure, Some(message)));
    }

    pub(super) fn receipt_status(&self, status: ApplicationReceiptStatus) {
        self.update(|state| {
            let changed = state.application_receipts.status != status
                || state.application_receipts.archival_degraded
                || state.application_receipts.last_archival_failure.is_some();
            state.application_receipts.status = status;
            state.application_receipts.archival_degraded = false;
            state.application_receipts.last_archival_failure = None;
            changed
        });
    }

    pub(super) fn receipt_failure(&self) {
        self.update(|state| {
            let failure = "application receipt archival/storage operation failed".to_owned();
            let changed = !state.application_receipts.archival_degraded
                || state.application_receipts.last_archival_failure.as_deref()
                    != Some(failure.as_str());
            state.application_receipts.archival_degraded = true;
            state.application_receipts.last_archival_failure = Some(failure);
            changed
        });
    }

    pub(super) fn peer_status(&self, status: PeerExecutionStatus) {
        self.update(|state| {
            let changed = state.peer_executions.status != status
                || state.peer_executions.archival_degraded
                || state.peer_executions.last_archival_failure.is_some();
            state.peer_executions.status = status;
            state.peer_executions.archival_degraded = false;
            state.peer_executions.last_archival_failure = None;
            changed
        });
    }

    pub(super) fn peer_failure(&self) {
        self.update(|state| {
            let failure = "peer execution archival/storage operation failed".to_owned();
            let changed = !state.peer_executions.archival_degraded
                || state.peer_executions.last_archival_failure.as_deref() != Some(failure.as_str());
            state.peer_executions.archival_degraded = true;
            state.peer_executions.last_archival_failure = Some(failure);
            changed
        });
    }

    pub(super) fn set_active_effects(&self, active_effects: u32) {
        self.update(|state| replace_if_changed(&mut state.active_effects, active_effects));
    }

    pub(super) fn track_queued_request(self: &Arc<Self>) -> QueuedRequestGuard {
        self.update(|state| {
            let queued_requests = state.queued_requests.saturating_add(1);
            replace_if_changed(&mut state.queued_requests, queued_requests)
        });
        QueuedRequestGuard {
            health: self.clone(),
            occupied: true,
        }
    }

    fn release_queued_request(&self) {
        self.update(|state| match state.queued_requests.checked_sub(1) {
            Some(queued) => {
                state.queued_requests = queued;
                true
            }
            None => replace_if_changed(
                &mut state.last_failure,
                Some("daemon request queue accounting underflowed".to_owned()),
            ),
        });
    }

    pub(super) fn read(&self) -> HealthRead {
        self.lock().state.read()
    }

    pub(super) fn snapshot(&self) -> (u64, HealthRead) {
        let versioned = self.lock();
        (versioned.generation, versioned.state.read())
    }
}

fn replace_if_changed<T: PartialEq>(current: &mut T, replacement: T) -> bool {
    if *current == replacement {
        false
    } else {
        *current = replacement;
        true
    }
}

/// Owned proof that one request currently contributes to queue occupancy.
pub(super) struct QueuedRequestGuard {
    health: Arc<SharedHealth>,
    occupied: bool,
}

impl QueuedRequestGuard {
    pub(super) fn release(mut self) {
        self.release_if_occupied();
    }

    fn release_if_occupied(&mut self) {
        if self.occupied {
            self.health.release_queued_request();
            self.occupied = false;
        }
    }
}

impl Drop for QueuedRequestGuard {
    fn drop(&mut self) {
        self.release_if_occupied();
    }
}

fn peer_serving(peers: &PeerHostConfig) -> Option<&PeerServingConfig> {
    match peers {
        PeerHostConfig::Disabled => None,
        PeerHostConfig::Enabled { serving, .. } => Some(serving),
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use milkdrift_control_protocol::DaemonState;
    use milkdrift_persistence::{ApplicationReceiptStatus, TimestampMillis};

    use super::{Lifecycle, SharedHealth};
    use crate::config::{ApplicationReceiptConfig, PeerHostConfig, StoragePlan};

    fn health() -> Arc<SharedHealth> {
        Arc::new(SharedHealth::new(
            4,
            &StoragePlan {
                data_root: PathBuf::from("unused-health-test-root"),
                application_receipts: ApplicationReceiptConfig {
                    hot_receipt_bound: 8,
                    archive_batch_size: 2,
                },
                security_audit_record_bound: 1,
            },
            &PeerHostConfig::Disabled,
        ))
    }

    #[test]
    fn coherent_updates_advance_generation_only_when_the_snapshot_changes() {
        let health = health();
        let (initial_generation, initial) = health.snapshot();
        assert_eq!(initial_generation, 1);
        assert_eq!(initial.state, DaemonState::Starting);

        health.set_lifecycle(Lifecycle::Ready);
        let (ready_generation, ready) = health.snapshot();
        assert_eq!(ready_generation, 2);
        assert!(ready.ready);
        assert!(ready.live);

        health.set_lifecycle(Lifecycle::Ready);
        assert_eq!(health.snapshot().0, ready_generation);

        health.receipt_status(ApplicationReceiptStatus {
            hot_count: 3,
            cold_count: 5,
            hot_bound: 8,
            archive_batch_size: 2,
            archive_generation: 1,
            last_archived_at: Some(TimestampMillis::new(42)),
        });
        let (receipt_generation, receipt) = health.snapshot();
        assert_eq!(receipt_generation, ready_generation + 1);
        assert_eq!(receipt.application_receipts.hot_count, 3);
        assert_eq!(receipt.application_receipts.cold_count, 5);
        assert_eq!(
            receipt.application_receipts.last_archived_at_unix_ms,
            Some(42)
        );
    }

    #[test]
    fn queue_occupancy_is_released_by_owned_guards_on_every_path() {
        let health = health();
        let initial_generation = health.snapshot().0;
        let first = health.track_queued_request();
        assert_eq!(health.read().queued_requests, 1);
        let second = health.track_queued_request();
        assert_eq!(health.read().queued_requests, 2);

        first.release();
        assert_eq!(health.read().queued_requests, 1);
        drop(second);
        let (generation, snapshot) = health.snapshot();
        assert_eq!(snapshot.queued_requests, 0);
        assert_eq!(generation, initial_generation + 4);
    }
}
