use super::support::*;

#[test]
fn cleanup_failure_retains_all_handles_and_retry_is_idempotent() -> Result<(), String> {
    let mut prepared = test_prepared(vec![required_f32_shard(2)?], DType::F32)?;
    configure_test_device(&mut prepared, DeviceKind::Cuda)?;
    required_error(
        prepared.materialize_shard(
            0,
            &mut FailAt(MaterializationCheckpoint::BeforeBatchSynchronization {
                batch_index: 0,
                entries: 2,
            }),
        ),
        "populated batch checkpoint must fail",
    )?;
    let stable_plan = *prepared.plan();
    let stable_transfer_plan_batches = prepared
        .transfer_plan
        .as_ref()
        .map(TransferPlan::len)
        .ok_or_else(|| "missing stable transfer plan".to_owned())?;
    let tensor = Tensor::ones(1, DType::F32, &Device::Cpu)
        .map_err(|error| format!("create cleanup tensor: {error}"))?;
    prepared
        .final_tensors
        .insert("final".to_owned(), tensor.clone());
    prepared.pending_source_tensor = Some(tensor);

    let mut failed = prepared.into_failed();
    TEST_CLEANUP_SYNCHRONIZATION_FAILURES.with(|remaining| remaining.set(1));
    assert!(failed.cleanup().is_err());
    let retained = failed
        .prepared
        .as_ref()
        .ok_or_else(|| "cleanup failure discarded the retained owner".to_owned())?;
    assert_eq!(retained.final_tensors.len(), 1);
    assert!(retained.pending_source_tensor.is_some());
    assert!(retained.pending_host_tensor.is_none());
    assert!(retained.pending_device_tensor.is_none());
    let retained_batch = retained
        .transfer_batch
        .as_ref()
        .ok_or_else(|| "cleanup failure discarded transfer batch".to_owned())?;
    assert_eq!(retained_batch.active_batch_index(), Some(0));
    assert_eq!(retained_batch.len(), 2);
    assert_eq!(retained_batch.retained_host_bytes(), 8);
    assert_eq!(retained_batch.transferred_device_bytes(), 8);
    assert!(!retained_batch.synchronized());
    assert_eq!(retained_batch.committed_entries(), 0);
    assert_eq!(
        retained.transfer_plan.as_ref().map(TransferPlan::len),
        Some(stable_transfer_plan_batches)
    );
    assert_eq!(retained.shards.len(), 1);
    assert!(retained.config.is_some());
    assert!(retained.device.is_some());
    assert!(!retained.cleanup_complete);
    assert_eq!(failed.plan(), &stable_plan);

    failed
        .cleanup()
        .map_err(|error| format!("retry cleanup: {error:?}"))?;
    assert!(failed.prepared.is_none());
    assert_eq!(failed.plan(), &stable_plan);
    failed
        .cleanup()
        .map_err(|error| format!("idempotent cleanup: {error:?}"))?;
    drop(failed);
    Ok(())
}

#[test]
fn public_prepared_load_preserves_primary_tensor_context_across_cleanup_retry() -> Result<(), String>
{
    let header = br#"{"required":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
    let payload = [0_u8, 0, 128, 63];
    let tensor = inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
    let shard = inspected_shard(header, &payload, vec![tensor])?;
    let prepared = test_prepared(vec![shard], DType::F32)?;
    let expected = TensorFailureLocation::new(
        0,
        0,
        tensor_name_fingerprint("required"),
        Some(ScalarType::F32),
    );
    TEST_MATERIALIZATION_CHECKPOINT_FAILURE.with(|selected| {
        selected.set(Some(MaterializationCheckpoint::SourceOwned {
            shard_index: 0,
            tensor_index: 0,
        }));
    });
    let mut loader = crate::CandleLlamaLoader::new(prepared.backend);
    let mut failed = loader
        .load_prepared(prepared)
        .err()
        .ok_or_else(|| "injected public prepared load unexpectedly succeeded".to_owned())?;
    TEST_MATERIALIZATION_CHECKPOINT_FAILURE.with(|selected| selected.set(None));
    assert_tensor_context(
        failed.primary(),
        LoadFailureStage::HostMaterialization,
        expected,
    );

    TEST_CLEANUP_SYNCHRONIZATION_FAILURES.with(|remaining| remaining.set(1));
    let cleanup_error = failed
        .cleanup()
        .err()
        .ok_or_else(|| "injected cleanup unexpectedly succeeded".to_owned())?;
    assert!(matches!(
        cleanup_error,
        domain_contracts::SynchronizationError::Backend(failure)
            if failure.code == crate::failure::CODE_PARTIAL_LOAD_SYNCHRONIZE
    ));
    assert_tensor_context(
        failed.primary(),
        LoadFailureStage::HostMaterialization,
        expected,
    );
    failed
        .cleanup()
        .map_err(|error| format!("retry cleanup: {error:?}"))?;
    assert_tensor_context(
        failed.primary(),
        LoadFailureStage::HostMaterialization,
        expected,
    );
    Ok(())
}
