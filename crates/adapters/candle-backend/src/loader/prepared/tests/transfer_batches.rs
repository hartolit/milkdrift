use super::support::*;

#[test]
fn transfer_fault_retains_both_endpoints_without_cuda_hardware() -> Result<(), String> {
    let header = br#"{"required":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
    let payload = [0_u8, 0, 128, 63];
    let tensor = inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
    let shard = inspected_shard(header, &payload, vec![tensor])?;
    let mut prepared = test_prepared(vec![shard], DType::F32)?;
    configure_test_device(&mut prepared, DeviceKind::Cuda)?;

    let error = required_error(
        prepared.materialize_shard(
            0,
            &mut FailAt(MaterializationCheckpoint::TransferEnqueued {
                batch_index: 0,
                entry_index: 0,
            }),
        ),
        "simulated transfer ownership checkpoint must fail",
    )?;
    assert_tensor_context(
        error,
        LoadFailureStage::DeviceTransfer,
        TensorFailureLocation::new(
            0,
            0,
            tensor_name_fingerprint("required"),
            Some(ScalarType::F32),
        ),
    );
    assert!(prepared.pending_host_tensor.is_none());
    assert!(prepared.pending_device_tensor.is_none());
    assert_eq!(
        prepared
            .transfer_batch
            .as_ref()
            .map(TransferBatchOwner::len),
        Some(1)
    );
    assert!(prepared.final_tensors.is_empty());
    Ok(())
}

#[test]
fn planned_multi_entry_batch_uses_one_synchronization_and_commits_all_entries() -> Result<(), String>
{
    let shard = required_f32_shard(3)?;
    let mut prepared = test_prepared(vec![shard], DType::F32)?;
    configure_test_device(&mut prepared, DeviceKind::Cuda)?;
    let planned_batches = prepared
        .transfer_plan
        .as_ref()
        .map(TransferPlan::len)
        .ok_or_else(|| "missing transfer plan".to_owned())?;
    assert_eq!(planned_batches, 1);

    let mut events = Events::default();
    prepared
        .materialize_shard(0, &mut events)
        .map_err(|error| format!("materialize batch: {error:?}"))?;
    assert_eq!(events.transfer_owned_count, 3);
    assert_eq!(events.batch_synchronizations, planned_batches);
    assert_eq!(events.map_owned_count, 3);
    assert_eq!(prepared.final_tensors.len(), 3);
    assert!(
        prepared
            .transfer_batch
            .as_ref()
            .is_some_and(TransferBatchOwner::is_empty)
    );
    Ok(())
}

#[test]
fn planned_and_owned_batch_accounting_must_match_before_synchronization() -> Result<(), String> {
    let mut prepared = test_prepared(vec![required_f32_shard(1)?], DType::F32)?;
    configure_test_device(&mut prepared, DeviceKind::Cuda)?;
    let source = Tensor::ones(1, DType::F32, &Device::Cpu)
        .map_err(|error| format!("create accounting source: {error}"))?;
    let backend = prepared.backend;
    let owner = prepared
        .transfer_batch
        .as_mut()
        .ok_or_else(|| "missing transfer owner".to_owned())?;
    owner
        .begin(backend, 0, 1)
        .map_err(|error| format!("begin transfer owner: {error:?}"))?;
    let next_bytes = owner
        .preflight_push(backend, 5, 4)
        .map_err(|error| format!("preflight mismatched entry: {error:?}"))?;
    owner.push_preflighted(
        TransferBatchEntry::new(
            (0, 0),
            "required.0".to_owned(),
            TensorFailureLocation::new(0, 0, 0, Some(ScalarType::F32)),
            TransferBatchEndpoints {
                source: source.clone(),
                converted_host: None,
                device: source,
            },
            5,
            4,
        ),
        next_bytes,
    );
    prepared.next_transfer_entry_index = 1;

    let mut events = Events::default();
    let error = required_error(
        prepared.flush_transfer_batch(&mut events),
        "plan/owner byte-accounting drift must fail before synchronization",
    )?;
    assert_eq!(failure_code(error), Some(CODE_TENSOR_TRANSFER));
    assert_eq!(events.batch_synchronizations, 0);
    let retained = prepared
        .transfer_batch
        .as_ref()
        .ok_or_else(|| "mismatch discarded transfer owner".to_owned())?;
    assert_eq!(retained.len(), 1);
    assert_eq!(retained.retained_host_bytes(), 5);
    assert_eq!(retained.transferred_device_bytes(), 4);
    assert!(!retained.synchronized());
    Ok(())
}

#[test]
fn batch_fault_boundaries_retain_every_endpoint_and_commit_alias() -> Result<(), String> {
    for checkpoint in [
        MaterializationCheckpoint::BeforeBatchSynchronization {
            batch_index: 0,
            entries: 2,
        },
        MaterializationCheckpoint::BatchSynchronized {
            batch_index: 0,
            entries: 2,
        },
        MaterializationCheckpoint::BeforeBatchCommit {
            batch_index: 0,
            entries: 2,
        },
        MaterializationCheckpoint::BatchEntryCommitted {
            batch_index: 0,
            entry_index: 0,
            shard_index: 0,
            tensor_index: 0,
        },
    ] {
        let mut prepared = test_prepared(vec![required_f32_shard(2)?], DType::F32)?;
        configure_test_device(&mut prepared, DeviceKind::Cuda)?;
        required_error(
            prepared.materialize_shard(0, &mut FailAt(checkpoint)),
            "batch checkpoint must fail",
        )?;
        let batch = prepared
            .transfer_batch
            .as_ref()
            .ok_or_else(|| "missing retained batch".to_owned())?;
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.retained_host_bytes(), 8);
        assert_eq!(batch.transferred_device_bytes(), 8);
        match checkpoint {
            MaterializationCheckpoint::BeforeBatchSynchronization { .. } => {
                assert!(!batch.synchronized());
                assert_eq!(batch.committed_entries(), 0);
                assert!(prepared.final_tensors.is_empty());
            }
            MaterializationCheckpoint::BatchSynchronized { .. }
            | MaterializationCheckpoint::BeforeBatchCommit { .. } => {
                assert!(batch.synchronized());
                assert_eq!(batch.committed_entries(), 0);
                assert!(prepared.final_tensors.is_empty());
            }
            MaterializationCheckpoint::BatchEntryCommitted { .. } => {
                assert!(batch.synchronized());
                assert_eq!(batch.committed_entries(), 1);
                assert_eq!(prepared.final_tensors.len(), 1);
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

#[test]
fn later_batch_failure_keeps_earlier_commits_and_current_source() -> Result<(), String> {
    let mut prepared = test_prepared(vec![required_f32_shard(3)?], DType::F32)?;
    configure_test_device(&mut prepared, DeviceKind::Cuda)?;
    prepared.transfer_plan = Some(
        TransferPlan::build_with_test_limits(
            prepared.backend,
            &prepared.shards,
            prepared.execution_dtype,
            u64::MAX,
            2,
        )
        .map_err(|error| format!("split test plan: {error:?}"))?,
    );
    let error = required_error(
        prepared.materialize_shard(
            0,
            &mut FailAt(MaterializationCheckpoint::SourceOwned {
                shard_index: 0,
                tensor_index: 2,
            }),
        ),
        "later batch source checkpoint must fail",
    )?;
    assert_tensor_context(
        error,
        LoadFailureStage::HostMaterialization,
        TensorFailureLocation::new(
            0,
            2,
            tensor_name_fingerprint("required.2"),
            Some(ScalarType::F32),
        ),
    );
    assert_eq!(prepared.final_tensors.len(), 2);
    assert!(prepared.pending_source_tensor.is_some());
    assert!(
        prepared
            .transfer_batch
            .as_ref()
            .is_some_and(TransferBatchOwner::is_empty)
    );
    assert_eq!(prepared.next_transfer_batch_index, 1);
    Ok(())
}

#[test]
fn late_same_inode_mutation_keeps_prior_commit_and_final_batch_owner() -> Result<(), String> {
    let mut prepared = test_prepared(vec![required_f32_shard(3)?], DType::F32)?;
    configure_test_device(&mut prepared, DeviceKind::Cuda)?;
    prepared.transfer_plan = Some(
        TransferPlan::build_with_test_limits(
            prepared.backend,
            &prepared.shards,
            prepared.execution_dtype,
            u64::MAX,
            2,
        )
        .map_err(|error| format!("split test plan: {error:?}"))?,
    );
    let final_byte = prepared
        .shards
        .first()
        .ok_or_else(|| "missing retained shard".to_owned())?
        .file_length
        .checked_sub(1)
        .ok_or_else(|| "cannot mutate an empty shard".to_owned())?;
    let shard = prepared
        .shards
        .first_mut()
        .ok_or_else(|| "missing retained shard".to_owned())?;
    shard
        .file
        .seek(SeekFrom::Start(final_byte))
        .map_err(|error| error.to_string())?;
    shard
        .file
        .write_all(&[0])
        .map_err(|error| error.to_string())?;

    let error = required_error(
        prepared.materialize_shard(0, &mut Events::default()),
        "late same-inode mutation must fail whole-shard verification",
    )?;
    assert_eq!(failure_code(error), Some(CODE_SOURCE_IDENTITY_MISMATCH));
    assert_eq!(prepared.final_tensors.len(), 2);
    let batch = prepared
        .transfer_batch
        .as_ref()
        .ok_or_else(|| "missing final retained batch".to_owned())?;
    assert_eq!(batch.active_batch_index(), Some(1));
    assert_eq!(batch.len(), 1);
    assert!(!batch.synchronized());
    assert_eq!(batch.committed_entries(), 0);
    Ok(())
}
