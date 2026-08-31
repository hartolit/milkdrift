//! Fast correctness assertions over every network-free operational fixture.

use milkdrift_evidence::{
    application_receipt_paths, artifact_range_read, context_discovery_and_selection,
    context_materialization, journal_append_batch, journal_append_one, measure_storage_growth,
    model_stream_parsers, peer_observation_paths, projection_rebuild, projection_snapshot_tail,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[test]
fn persistence_replay_retention_and_recovery_contracts() -> TestResult {
    assert_eq!(journal_append_one()?.operations, 1);
    assert_eq!(journal_append_batch()?.operations, 64);
    assert_eq!(application_receipt_paths()?.operations, 35);
    assert_eq!(peer_observation_paths()?.operations, 72);
    let storage = measure_storage_growth(32)?;
    assert_eq!(storage.receipt_primary_count, 32);
    assert_eq!(
        storage.receipt_primary_count,
        storage
            .receipt_hot_count
            .saturating_add(storage.receipt_cold_count)
    );
    assert!(storage.receipt_hot_count <= 8);
    assert!(storage.receipt_cold_count >= 24);
    assert_eq!(
        storage.receipt_primary_logical_bytes,
        storage
            .receipt_hot_logical_bytes
            .saturating_add(storage.receipt_cold_logical_bytes)
    );
    assert!(storage.oldest_cold_replayed);
    assert_eq!(storage.peer_executions, 32);
    assert_eq!(storage.peer_observations, 32 * 17);
    assert_eq!(storage.peer_tombstones, 32);
    assert_eq!(storage.peer_active_count, 0);
    assert_eq!(storage.peer_dispatch_queued_count, 0);
    assert_eq!(storage.peer_hot_count, 0);
    assert_eq!(storage.peer_peak_active_count, 1);
    assert_eq!(storage.peer_peak_hot_count, 1);
    assert!(storage.peer_active_snapshot_logical_bytes > 0);
    assert!(storage.peer_hot_snapshot_logical_bytes > 0);
    assert!(storage.peer_tombstone_snapshot_logical_bytes > 0);
    assert!(storage.peer_observation_logical_bytes > 0);
    Ok(())
}

#[test]
fn runtime_context_and_adapter_contracts() -> TestResult {
    assert_eq!(projection_rebuild()?.operations, 4_096);
    assert_eq!(projection_snapshot_tail()?.operations, 128);
    assert_eq!(context_discovery_and_selection()?.operations, 2_048);
    assert_eq!(context_materialization()?.operations, 64);
    assert_eq!(artifact_range_read()?.operations, 17);
    assert_eq!(model_stream_parsers()?.operations, 2_048);
    Ok(())
}
