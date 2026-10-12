//! Fast correctness assertions over every network-free operational fixture.

use milkdrift_evidence::{
    artifact_range_read, explicit_input_materialization, explicit_input_selection,
    measure_storage_growth, model_stream_parsers, peer_observation_paths, workflow_commit_batch,
    workflow_commit_one, workflow_read_page, workflow_receipt_paths, workflow_snapshot_reopen,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[test]
fn actual_child_capture_deadline_and_cleanup_are_bounded() -> TestResult {
    use milkdrift_evidence::application::{OwnedChild, run_command};
    use std::{process::Command, time::Duration};

    let helper = env!("CARGO_BIN_EXE_evidence-process-helper");
    let output = run_command(&mut Command::new(helper), None, Duration::from_secs(10))?;
    assert!(output.status.success());
    assert_eq!(output.stdout.as_bytes(), &[b'o'; 256 * 1024]);
    assert_eq!(output.stderr.as_bytes(), &[b'e'; 256 * 1024]);
    let result = run_command(
        Command::new(helper).arg("--deadline-fixture"),
        None,
        Duration::from_millis(50),
    );
    assert!(matches!(result, Err(error) if error.to_string().contains("deadline")));
    let result = run_command(
        Command::new(helper).arg("--overflow-fixture"),
        None,
        Duration::from_secs(5),
    );
    assert!(matches!(result, Err(error) if error.to_string().contains("output exceeds")));
    let mut child = OwnedChild::spawn(Command::new(helper).arg("--deadline-fixture"))?;
    child.terminate()?;
    assert!(child.try_wait()?.is_some());
    Ok(())
}

#[test]
fn persistence_replay_retention_and_recovery_contracts() -> TestResult {
    assert_eq!(workflow_commit_one()?.operations, 1);
    assert_eq!(workflow_commit_batch()?.operations, 32);
    assert_eq!(workflow_receipt_paths()?.operations, 35);
    assert_eq!(peer_observation_paths()?.operations, 72);
    let storage = measure_storage_growth(32)?;
    assert_eq!(storage.receipt_primary_count, 32);
    assert!(storage.receipt_primary_logical_bytes > 0);
    assert!(storage.oldest_receipt_replayed);
    assert!(storage.conflicting_receipt_refused);
    assert_eq!(storage.workflow_documents, 32);
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
    assert_eq!(workflow_read_page()?.operations, 64);
    assert_eq!(workflow_snapshot_reopen()?.operations, 128);
    assert_eq!(explicit_input_selection()?.operations, 64);
    assert_eq!(explicit_input_materialization()?.operations, 64);
    assert_eq!(artifact_range_read()?.operations, 17);
    assert_eq!(model_stream_parsers()?.operations, 2_048);
    Ok(())
}
