use std::sync::Arc;

use milkdrift_control_protocol::DaemonState;
use milkdrift_persistence::PeerExecutionStatus;

use super::{Lifecycle, SharedHealth};

fn health() -> Arc<SharedHealth> {
    Arc::new(SharedHealth::new(
        4,
        None,
        milkdrift_control_protocol::HostRole::WorkflowEnabled,
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

    health.peer_status(PeerExecutionStatus {
        active: 3,
        ..PeerExecutionStatus::default()
    });
    let (generation, snapshot) = health.snapshot();
    assert_eq!(generation, ready_generation + 1);
    assert_eq!(snapshot.peer_executions.active_count, 3);
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
