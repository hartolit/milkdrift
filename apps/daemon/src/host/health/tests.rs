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
