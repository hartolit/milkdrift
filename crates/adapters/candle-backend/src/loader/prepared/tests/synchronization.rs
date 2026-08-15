use super::support::*;

#[test]
fn synchronization_failure_retains_unsynchronized_populated_batch() -> Result<(), String> {
    let mut prepared = test_prepared(vec![required_f32_shard(2)?], DType::F32)?;
    configure_test_device(&mut prepared, DeviceKind::Cuda)?;
    let error = required_error(
        prepared.materialize_shard(0, &mut FailSynchronizationAt(0)),
        "batch synchronization failure must surface",
    )?;
    assert_eq!(failure_code(error), Some(CODE_LOAD_SYNCHRONIZE));
    let batch = prepared
        .transfer_batch
        .as_ref()
        .ok_or_else(|| "missing retained failed batch".to_owned())?;
    assert_eq!(batch.len(), 2);
    assert!(!batch.synchronized());
    assert_eq!(batch.committed_entries(), 0);
    assert!(prepared.final_tensors.is_empty());
    Ok(())
}
