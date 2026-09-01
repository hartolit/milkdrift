//! Daemon-owned clock boundary and narrow adapters for inward runtime, peer, and storage ports.

use std::{
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use milkdrift_peer_http::{PeerClock, PeerClockError};
use milkdrift_persistence::{
    ClockWatermarkObservation, ClockWatermarkStore, PersistenceError, StorageFailureClass,
    TimestampMillis,
};
use milkdrift_redb_store::{ArtifactClock, RedbStore};
use milkdrift_runtime::{BoundaryClock, RuntimeError};
use thiserror::Error;
use tracing::{info, warn};

use super::{OwnerCallFailure, OwnerQueue, SharedHealth};

pub(super) trait DaemonClockSource: Send + Sync {
    fn now_unix_ms(&self) -> Result<u64, DaemonClockError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SystemDaemonClock;

impl DaemonClockSource for SystemDaemonClock {
    fn now_unix_ms(&self) -> Result<u64, DaemonClockError> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| DaemonClockError::BeforeUnixEpoch)?;
        u64::try_from(elapsed.as_millis()).map_err(|_| DaemonClockError::MillisecondOverflow)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub(super) enum DaemonClockError {
    #[error("system clock precedes the Unix epoch")]
    BeforeUnixEpoch,
    #[error("system clock exceeds the timestamp representation")]
    MillisecondOverflow,
    #[error("system clock moved behind durable high-water evidence")]
    MovedBackwards,
    #[error("system clock is unavailable")]
    Unavailable,
}

struct DurableClockInner {
    source: Arc<dyn DaemonClockSource>,
    queue: OwnerQueue,
    store: Weak<RedbStore>,
    health: Arc<SharedHealth>,
    failed: AtomicBool,
}

/// One application clock whose observations are serialized and durably monotonic.
#[derive(Clone)]
pub(super) struct DurableClock {
    inner: Arc<DurableClockInner>,
}

impl DurableClock {
    pub(super) fn new(
        source: Arc<dyn DaemonClockSource>,
        queue: OwnerQueue,
        store: Weak<RedbStore>,
        health: Arc<SharedHealth>,
    ) -> Self {
        Self {
            inner: Arc::new(DurableClockInner {
                source,
                queue,
                store,
                health,
                failed: AtomicBool::new(false),
            }),
        }
    }

    pub(super) fn now(&self) -> Result<TimestampMillis, DaemonClockError> {
        let source = self.inner.source.clone();
        let store = self.inner.store.clone();
        let result = self.inner.queue.call(
            move || {
                let observed = TimestampMillis::new(source.now_unix_ms()?);
                let store = store.upgrade().ok_or(DaemonClockError::Unavailable)?;
                match store
                    .observe_clock(observed)
                    .map_err(|_| DaemonClockError::Unavailable)?
                {
                    ClockWatermarkObservation::Advanced | ClockWatermarkObservation::Unchanged => {
                        Ok(observed)
                    }
                    ClockWatermarkObservation::RejectedRollback { .. } => {
                        Err(DaemonClockError::MovedBackwards)
                    }
                }
            },
            owner_failure,
        );
        match result {
            Ok(observed) => {
                if self.inner.failed.swap(false, Ordering::SeqCst) {
                    info!(
                        outcome = "recovered",
                        code = "daemon_clock_boundary",
                        "daemon clock boundary recovered"
                    );
                }
                Ok(observed)
            }
            Err(error) => {
                self.inner
                    .health
                    .failure("daemon clock boundary is unavailable or untrustworthy");
                if !self.inner.failed.swap(true, Ordering::SeqCst) {
                    warn!(
                        outcome = "failed",
                        code = "daemon_clock_boundary",
                        reason = clock_reason(error),
                        "daemon clock boundary rejected an observation"
                    );
                }
                Err(error)
            }
        }
    }

    pub(super) fn peer_adapter(&self) -> Arc<dyn PeerClock> {
        Arc::new(PeerClockAdapter(self.clone()))
    }

    pub(super) fn runtime_adapter(&self) -> Arc<dyn BoundaryClock> {
        Arc::new(RuntimeClockAdapter(self.clone()))
    }
}

fn owner_failure(_failure: OwnerCallFailure) -> DaemonClockError {
    DaemonClockError::Unavailable
}

const fn clock_reason(error: DaemonClockError) -> &'static str {
    match error {
        DaemonClockError::BeforeUnixEpoch => "before_unix_epoch",
        DaemonClockError::MillisecondOverflow => "millisecond_overflow",
        DaemonClockError::MovedBackwards => "moved_backwards",
        DaemonClockError::Unavailable => "unavailable",
    }
}

struct PeerClockAdapter(DurableClock);

impl PeerClock for PeerClockAdapter {
    fn now_unix_ms(&self) -> Result<u64, PeerClockError> {
        self.0
            .now()
            .map(TimestampMillis::get)
            .map_err(|error| match error {
                DaemonClockError::BeforeUnixEpoch => PeerClockError::BeforeUnixEpoch,
                DaemonClockError::MillisecondOverflow => PeerClockError::MillisecondOverflow,
                DaemonClockError::MovedBackwards => PeerClockError::MovedBackwards,
                DaemonClockError::Unavailable => PeerClockError::Unavailable,
            })
    }
}

struct RuntimeClockAdapter(DurableClock);

impl BoundaryClock for RuntimeClockAdapter {
    fn now(&self) -> Result<TimestampMillis, RuntimeError> {
        self.0.now().map_err(|_| {
            RuntimeError::Persistence(PersistenceError::Storage {
                class: StorageFailureClass::Unavailable,
                message: "daemon clock boundary is unavailable or untrustworthy".to_owned(),
            })
        })
    }
}

/// Raw source adapter used only where redb advances the watermark in the same write transaction.
pub(super) struct ArtifactClockAdapter(pub(super) Arc<dyn DaemonClockSource>);

impl ArtifactClock for ArtifactClockAdapter {
    fn now(&self) -> Result<TimestampMillis, PersistenceError> {
        self.0
            .now_unix_ms()
            .map(TimestampMillis::new)
            .map_err(|_| PersistenceError::Storage {
                class: StorageFailureClass::Unavailable,
                message: "daemon clock source is unavailable or untrustworthy".to_owned(),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;

    use milkdrift_peer_http::PeerClockError;
    use tempfile::TempDir;

    use super::*;
    use crate::config::{ApplicationReceiptConfig, PeerHostConfig, StoragePlan};

    struct ControlledSource {
        now: AtomicU64,
        available: AtomicBool,
    }

    impl ControlledSource {
        const fn new(now: u64) -> Self {
            Self {
                now: AtomicU64::new(now),
                available: AtomicBool::new(true),
            }
        }

        fn set(&self, now: u64) {
            self.now.store(now, Ordering::SeqCst);
        }

        fn set_available(&self, available: bool) {
            self.available.store(available, Ordering::SeqCst);
        }
    }

    impl DaemonClockSource for ControlledSource {
        fn now_unix_ms(&self) -> Result<u64, DaemonClockError> {
            if !self.available.load(Ordering::SeqCst) {
                return Err(DaemonClockError::Unavailable);
            }
            Ok(self.now.load(Ordering::SeqCst))
        }
    }

    #[test]
    fn daemon_clock_routes_peer_observations_through_durable_owner_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = TempDir::new()?;
        let source = Arc::new(ControlledSource::new(100));
        let store = Arc::new(RedbStore::open_with_config(
            milkdrift_redb_store::RedbStoreConfig::new(directory.path())
                .with_artifact_clock(Arc::new(ArtifactClockAdapter(source.clone()))),
        )?);
        let storage = StoragePlan {
            data_root: directory.path().to_path_buf(),
            application_receipts: ApplicationReceiptConfig::default(),
            security_audit_record_bound: 10,
        };
        let health = Arc::new(SharedHealth::new(4, &storage, &PeerHostConfig::default()));
        let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
        let sender = Arc::new(sender);
        let queue = OwnerQueue::new(
            Arc::downgrade(&sender),
            health.clone(),
            std::thread::current().id(),
        );
        let clock = DurableClock::new(
            source.clone(),
            queue,
            Arc::downgrade(&store),
            health.clone(),
        );
        let peer = clock.peer_adapter();

        assert_eq!(peer.now_unix_ms()?, 100);
        source.set(120);
        assert_eq!(peer.now_unix_ms()?, 120);
        assert_eq!(store.clock_watermark()?, Some(TimestampMillis::new(120)));

        source.set(119);
        assert_eq!(peer.now_unix_ms(), Err(PeerClockError::MovedBackwards));
        assert_eq!(
            health.read().last_failure.as_deref(),
            Some("daemon clock boundary is unavailable or untrustworthy")
        );

        source.set(120);
        assert_eq!(peer.now_unix_ms()?, 120);
        source.set_available(false);
        assert_eq!(peer.now_unix_ms(), Err(PeerClockError::Unavailable));
        Ok(())
    }
}
