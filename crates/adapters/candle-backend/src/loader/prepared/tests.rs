use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use candle_core::{DType, Device, Tensor};
use candle_transformers::models::llama::Config;
use domain_contracts::{
    BackendFailureKind, BackendId, BackendLoadFailure, CapabilitySet, DeviceId, DeviceKind,
    ExecutionDevice, FailedLoadOwner, LoadConfiguration, LoadError, LoadFailureContext,
    LoadFailureStage, LoadPlan, MemoryBudget, MemoryFootprint, ModelArchitecture,
    ModelCapabilities, ModelDescriptor, ModelGeneration, ModelHandle, ModelId, ModelLoader,
    ModelMetadata, PreparedLoad, QuantizationFormat, ScalarType, ScalarTypeSet,
    TensorFailureLocation,
};
use sha2::{Digest, Sha256};

use super::CandleLlamaPreparedLoad;
use crate::failure::{
    CODE_HEADER_IDENTITY_MISMATCH, CODE_LOAD_SYNCHRONIZE, CODE_SOURCE_IDENTITY_LENGTH,
    CODE_SOURCE_IDENTITY_MISMATCH, CODE_TENSOR_MAP_ALLOCATION, CODE_TENSOR_MATERIALIZE,
    CODE_TENSOR_TRANSFER, failure, tensor_name_fingerprint,
};
use crate::loader::cleanup::TEST_CLEANUP_SYNCHRONIZATION_FAILURES;
use crate::loader::identity::{ContentIdentityEstablishment, EstablishedContentIdentity};
use crate::loader::manifest::{InspectedShard, InspectedTensor, SourceTensorDType, TensorShape};
use crate::loader::observer::{
    HashedRange, LoadingSynchronization, MaterializationCheckpoint, MaterializationObserver,
    NoopMaterializationObserver, TEST_MATERIALIZATION_CHECKPOINT_FAILURE,
};
use crate::loader::shard_stream::TEST_REQUIRED_PAYLOAD_READ_FAILURES;
use crate::loader::transfer_batch::{
    TransferBatchEndpoints, TransferBatchEntry, TransferBatchOwner,
};
use crate::loader::transfer_plan::{MAXIMUM_BATCH_ENTRIES, TransferPlan};
use crate::source::CandleExpectedContentIdentity;

static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct Events {
    prefix_header_bytes: usize,
    ignored_bytes: usize,
    required_bytes: usize,
    source_owned_count: usize,
    cast_owned_count: usize,
    transfer_owned_count: usize,
    map_owned_count: usize,
    batch_synchronizations: usize,
    verified_establishments: Vec<ContentIdentityEstablishment>,
}

impl MaterializationObserver for Events {
    fn hashed_range(&mut self, range: HashedRange, bytes: usize) {
        match range {
            HashedRange::PrefixHeader => self.prefix_header_bytes += bytes,
            HashedRange::IgnoredTensor => self.ignored_bytes += bytes,
            HashedRange::RequiredTensor => self.required_bytes += bytes,
        }
    }

    fn checkpoint(
        &mut self,
        checkpoint: MaterializationCheckpoint,
        _backend: BackendId,
    ) -> Result<(), LoadError> {
        match checkpoint {
            MaterializationCheckpoint::SourceOwned { .. } => self.source_owned_count += 1,
            MaterializationCheckpoint::CastOwned { .. } => self.cast_owned_count += 1,
            MaterializationCheckpoint::TransferEnqueued { .. } => {
                self.transfer_owned_count += 1;
            }
            MaterializationCheckpoint::CpuMapOwned { .. }
            | MaterializationCheckpoint::BatchEntryCommitted { .. } => {
                self.map_owned_count += 1;
            }
            _ => {}
        }
        Ok(())
    }

    fn whole_shard_verified(&mut self, establishment: ContentIdentityEstablishment) {
        self.verified_establishments.push(establishment);
    }

    fn synchronize(
        &mut self,
        _boundary: LoadingSynchronization,
        _backend: BackendId,
        _device: &Device,
    ) -> Result<(), LoadError> {
        self.batch_synchronizations += 1;
        Ok(())
    }
}

struct FailAt(MaterializationCheckpoint);

impl MaterializationObserver for FailAt {
    fn checkpoint(
        &mut self,
        checkpoint: MaterializationCheckpoint,
        backend: BackendId,
    ) -> Result<(), LoadError> {
        if checkpoint == self.0 {
            let code = match checkpoint {
                MaterializationCheckpoint::BeforeCpuMapInsertion { .. }
                | MaterializationCheckpoint::CpuMapOwned { .. }
                | MaterializationCheckpoint::BatchEntryCommitted { .. } => {
                    CODE_TENSOR_MAP_ALLOCATION
                }
                MaterializationCheckpoint::TransferEnqueued { .. } => CODE_TENSOR_TRANSFER,
                _ => CODE_TENSOR_MATERIALIZE,
            };
            Err(super::invalid_model_failure(backend, code))
        } else {
            Ok(())
        }
    }
}

struct FailSynchronizationAt(usize);

impl MaterializationObserver for FailSynchronizationAt {
    fn synchronize(
        &mut self,
        boundary: LoadingSynchronization,
        backend: BackendId,
        _device: &Device,
    ) -> Result<(), LoadError> {
        let LoadingSynchronization::TransferBatch { batch_index } = boundary;
        if batch_index == self.0 {
            Err(LoadError::Backend(BackendLoadFailure::new(failure(
                backend,
                BackendFailureKind::Synchronization,
                CODE_LOAD_SYNCHRONIZE,
            ))))
        } else {
            Ok(())
        }
    }
}

#[test]
fn ignored_ranges_are_hashed_without_materialization_or_transfer() -> Result<(), String> {
    let header = br#"{"ignored":{"dtype":"U8","shape":[3],"data_offsets":[0,3]},"required":{"dtype":"F32","shape":[1],"data_offsets":[3,7]}}"#;
    let payload = [9_u8, 8, 7, 0, 0, 128, 63];

    for (device_kind, expected_transfer_count) in [(DeviceKind::Cpu, 0), (DeviceKind::Cuda, 1)] {
        let tensors = vec![
            inspected_tensor("ignored", SourceTensorDType::U8, &[3], 0, 3, false)?,
            inspected_tensor("required", SourceTensorDType::F32, &[1], 3, 4, true)?,
        ];
        let shard = inspected_shard(header, &payload, tensors)?;
        let mut prepared = test_prepared(vec![shard], DType::F32)?;
        configure_test_device(&mut prepared, device_kind)?;
        let mut events = Events::default();
        prepared
            .materialize_shard(0, &mut events)
            .map_err(|error| format!("materialize shard: {error:?}"))?;

        assert_eq!(events.prefix_header_bytes, 8 + header.len());
        assert_eq!(events.ignored_bytes, 3);
        assert_eq!(events.required_bytes, 4);
        assert_eq!(events.source_owned_count, 1);
        assert_eq!(events.cast_owned_count, 0);
        assert_eq!(events.transfer_owned_count, expected_transfer_count);
        assert_eq!(events.map_owned_count, 1);
        assert_eq!(events.batch_synchronizations, expected_transfer_count);
        assert_eq!(
            events.verified_establishments.as_slice(),
            &[ContentIdentityEstablishment::SuppliedExpectation]
        );
        assert!(prepared.final_tensors.contains_key("required"));
        assert!(!prepared.final_tensors.contains_key("ignored"));
    }
    Ok(())
}

#[test]
fn header_payload_and_truncation_mutations_fail_from_retained_files() -> Result<(), String> {
    let header = br#"{"required":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
    let payload = [0_u8, 0, 128, 63];

    let header_tensor = inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
    let header_shard = inspected_shard(header, &payload, vec![header_tensor])?;
    let mut header_prepared = test_prepared(vec![header_shard], DType::F32)?;
    {
        let shard = first_shard_mut(&mut header_prepared)?;
        shard
            .file
            .seek(SeekFrom::Start(8))
            .map_err(|error| error.to_string())?;
        shard
            .file
            .write_all(b"[")
            .map_err(|error| error.to_string())?;
    }
    let error = required_error(
        header_prepared.materialize_shard(0, &mut Events::default()),
        "header mutation must fail before payload processing",
    )?;
    assert_eq!(failure_code(error), Some(CODE_HEADER_IDENTITY_MISMATCH));
    assert!(header_prepared.final_tensors.is_empty());

    let payload_tensor = inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
    let payload_shard = inspected_shard(header, &payload, vec![payload_tensor])?;
    let mut payload_prepared = test_prepared(vec![payload_shard], DType::F32)?;
    let last_byte = first_shard_mut(&mut payload_prepared)?
        .file_length
        .checked_sub(1)
        .ok_or_else(|| "missing payload byte".to_owned())?;
    {
        let shard = first_shard_mut(&mut payload_prepared)?;
        shard
            .file
            .seek(SeekFrom::Start(last_byte))
            .map_err(|error| error.to_string())?;
        shard
            .file
            .write_all(&[0_u8])
            .map_err(|error| error.to_string())?;
    }
    let error = required_error(
        payload_prepared.materialize_shard(0, &mut Events::default()),
        "payload mutation must fail at whole-shard verification",
    )?;
    assert_eq!(failure_code(error), Some(CODE_SOURCE_IDENTITY_MISMATCH));
    assert!(payload_prepared.final_tensors.contains_key("required"));

    let truncated_tensor = inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
    let truncated_shard = inspected_shard(header, &payload, vec![truncated_tensor])?;
    let mut truncated_prepared = test_prepared(vec![truncated_shard], DType::F32)?;
    let truncated_length = first_shard_mut(&mut truncated_prepared)?
        .file_length
        .checked_sub(1)
        .ok_or_else(|| "cannot truncate empty file".to_owned())?;
    first_shard_mut(&mut truncated_prepared)?
        .file
        .set_len(truncated_length)
        .map_err(|error| error.to_string())?;
    let error = required_error(
        truncated_prepared.materialize_shard(0, &mut Events::default()),
        "truncation must fail before streaming",
    )?;
    assert_eq!(failure_code(error), Some(CODE_SOURCE_IDENTITY_LENGTH));
    Ok(())
}

#[test]
fn concrete_required_payload_read_failure_has_exact_tensor_context() -> Result<(), String> {
    let header = br#"{"required":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
    let payload = [0_u8, 0, 128, 63];
    let tensor = inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
    let shard = inspected_shard(header, &payload, vec![tensor])?;
    let mut prepared = test_prepared(vec![shard], DType::F32)?;
    TEST_REQUIRED_PAYLOAD_READ_FAILURES.with(|remaining| remaining.set(1));
    let error = required_error(
        prepared.materialize_shard(0, &mut Events::default()),
        "injected required payload read must fail",
    )?;
    assert_tensor_context(
        error,
        LoadFailureStage::PayloadRead,
        TensorFailureLocation::new(
            0,
            0,
            tensor_name_fingerprint("required"),
            Some(ScalarType::F32),
        ),
    );
    Ok(())
}

#[test]
fn source_cast_and_map_faults_retain_real_owners() -> Result<(), String> {
    for (checkpoint, source_dtype, execution_dtype) in [
        (
            MaterializationCheckpoint::SourceOwned {
                shard_index: 0,
                tensor_index: 0,
            },
            SourceTensorDType::F32,
            DType::F32,
        ),
        (
            MaterializationCheckpoint::HostOwned {
                shard_index: 0,
                tensor_index: 0,
            },
            SourceTensorDType::F32,
            DType::F32,
        ),
        (
            MaterializationCheckpoint::CastOwned {
                shard_index: 0,
                tensor_index: 0,
            },
            SourceTensorDType::F32,
            DType::F16,
        ),
        (
            MaterializationCheckpoint::BeforeCpuMapInsertion {
                shard_index: 0,
                tensor_index: 0,
            },
            SourceTensorDType::F32,
            DType::F32,
        ),
        (
            MaterializationCheckpoint::CpuMapOwned {
                shard_index: 0,
                tensor_index: 0,
            },
            SourceTensorDType::F32,
            DType::F32,
        ),
    ] {
        let header = br#"{"required":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
        let payload = [0_u8, 0, 128, 63];
        let tensor = inspected_tensor("required", source_dtype, &[1], 0, 4, true)?;
        let shard = inspected_shard(header, &payload, vec![tensor])?;
        let mut prepared = test_prepared(vec![shard], execution_dtype)?;
        let error = required_error(
            prepared.materialize_shard(0, &mut FailAt(checkpoint)),
            "injected ownership checkpoint must fail",
        )?;
        let expected_stage = match checkpoint {
            MaterializationCheckpoint::CastOwned { .. } => LoadFailureStage::ScalarConversion,
            MaterializationCheckpoint::BeforeCpuMapInsertion { .. }
            | MaterializationCheckpoint::CpuMapOwned { .. } => LoadFailureStage::RetainedPlacement,
            _ => LoadFailureStage::HostMaterialization,
        };
        assert_tensor_context(
            error,
            expected_stage,
            TensorFailureLocation::new(
                0,
                0,
                tensor_name_fingerprint("required"),
                Some(ScalarType::F32),
            ),
        );
        match checkpoint {
            MaterializationCheckpoint::SourceOwned { .. } => {
                assert!(prepared.pending_source_tensor.is_some());
            }
            MaterializationCheckpoint::HostOwned { .. } => {
                assert!(prepared.pending_source_tensor.is_none());
                assert!(prepared.pending_host_tensor.is_some());
            }
            MaterializationCheckpoint::CastOwned { .. } => {
                assert!(prepared.pending_source_tensor.is_some());
                assert!(prepared.pending_host_tensor.is_some());
            }
            MaterializationCheckpoint::BeforeCpuMapInsertion { .. } => {
                assert!(prepared.pending_host_tensor.is_some());
            }
            MaterializationCheckpoint::CpuMapOwned { .. } => {
                assert!(prepared.final_tensors.contains_key("required"));
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

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

#[test]
fn model_construction_is_handle_only_and_fault_retains_the_owner() -> Result<(), String> {
    let mut missing = test_prepared(Vec::new(), DType::F32)?;
    populate_required_model_tensors(&mut missing)?;
    missing.final_tensors.remove("lm_head.weight");
    required_error(
        missing.construct_model(&mut NoopMaterializationObserver),
        "missing native model tensor must fail construction",
    )?;
    assert!(missing.constructed_model.is_none());
    assert!(!missing.final_tensors.is_empty());

    {
        let checkpoint = MaterializationCheckpoint::ModelOwned;
        let mut prepared = test_prepared(Vec::new(), DType::F32)?;
        populate_required_model_tensors(&mut prepared)?;
        required_error(
            prepared.construct_model(&mut FailAt(checkpoint)),
            "post-construction checkpoint must fail",
        )?;
        assert!(prepared.constructed_model.is_some());
        assert_eq!(prepared.final_tensors.len(), 12);
        let mut failed = prepared.into_failed();
        failed
            .cleanup()
            .map_err(|error| format!("cleanup constructed model: {error:?}"))?;
    }

    let mut prepared = test_prepared(Vec::new(), DType::F32)?;
    populate_required_model_tensors(&mut prepared)?;
    let mut events = Events::default();
    prepared
        .construct_model(&mut events)
        .map_err(|error| format!("construct handle-only model: {error:?}"))?;
    assert_eq!(events.batch_synchronizations, 0);
    assert!(prepared.constructed_model.is_some());
    Ok(())
}

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

fn required_f32_shard(count: usize) -> Result<InspectedShard, String> {
    let scalar = [0_u8, 0, 128, 63];
    let mut payload = Vec::new();
    let mut tensors = Vec::new();
    for index in 0..count {
        payload.extend_from_slice(&scalar);
        let data_start = u64::try_from(index)
            .map_err(|error| error.to_string())?
            .checked_mul(4)
            .ok_or_else(|| "test data offset overflow".to_owned())?;
        tensors.push(inspected_tensor(
            format!("required.{index}").as_str(),
            SourceTensorDType::F32,
            &[1],
            data_start,
            4,
            true,
        )?);
    }
    inspected_shard(br"{}", payload.as_slice(), tensors)
}

fn inspected_tensor(
    name: &str,
    source_dtype: SourceTensorDType,
    shape: &[usize],
    data_start: u64,
    source_bytes: u64,
    required: bool,
) -> Result<InspectedTensor, String> {
    let element_count = shape.iter().try_fold(1_u64, |total, dimension| {
        total
            .checked_mul(u64::try_from(*dimension).map_err(|error| error.to_string())?)
            .ok_or_else(|| "element count overflow".to_owned())
    })?;
    Ok(InspectedTensor {
        name: name.to_owned(),
        source_dtype,
        shape: TensorShape::from_slice(shape)
            .ok_or_else(|| "test shape exceeds fixed rank".to_owned())?,
        data_start,
        source_bytes,
        element_count,
        required,
    })
}

fn inspected_shard(
    header: &[u8],
    payload: &[u8],
    tensors: Vec<InspectedTensor>,
) -> Result<InspectedShard, String> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(
        &u64::try_from(header.len())
            .map_err(|error| error.to_string())?
            .to_le_bytes(),
    );
    bytes.extend_from_slice(header);
    bytes.extend_from_slice(payload);
    let data_start = 8_u64
        .checked_add(u64::try_from(header.len()).map_err(|error| error.to_string())?)
        .ok_or_else(|| "data start overflow".to_owned())?;
    let prefix_length = usize::try_from(data_start).map_err(|error| error.to_string())?;
    let prefix_header_sha256: [u8; 32] = Sha256::digest(
        bytes
            .get(..prefix_length)
            .ok_or_else(|| "prefix range missing".to_owned())?,
    )
    .into();
    let whole_sha256: [u8; 32] = Sha256::digest(bytes.as_slice()).into();
    let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "milkdrift-candle-materialize-{}-{sequence}.safetensors",
        std::process::id()
    ));
    let mut created = File::create(&path).map_err(|error| error.to_string())?;
    created
        .write_all(bytes.as_slice())
        .map_err(|error| error.to_string())?;
    created.sync_all().map_err(|error| error.to_string())?;
    drop(created);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    fs::remove_file(path).map_err(|error| error.to_string())?;
    let file_length = u64::try_from(bytes.len()).map_err(|error| error.to_string())?;
    Ok(InspectedShard {
        file,
        file_length,
        data_start,
        prefix_header_sha256,
        source_expected_content: Some(CandleExpectedContentIdentity::new(
            file_length,
            whole_sha256,
        )),
        established_content_identity: Some(EstablishedContentIdentity {
            byte_length: file_length,
            sha256: whole_sha256,
            establishment: ContentIdentityEstablishment::SuppliedExpectation,
        }),
        tensors,
    })
}

fn populate_required_model_tensors(prepared: &mut CandleLlamaPreparedLoad) -> Result<(), String> {
    for (name, shape) in [
        ("model.embed_tokens.weight", &[16, 8][..]),
        ("lm_head.weight", &[16, 8][..]),
        ("model.norm.weight", &[8][..]),
        ("model.layers.0.self_attn.q_proj.weight", &[8, 8][..]),
        ("model.layers.0.self_attn.k_proj.weight", &[8, 8][..]),
        ("model.layers.0.self_attn.v_proj.weight", &[8, 8][..]),
        ("model.layers.0.self_attn.o_proj.weight", &[8, 8][..]),
        ("model.layers.0.input_layernorm.weight", &[8][..]),
        ("model.layers.0.post_attention_layernorm.weight", &[8][..]),
        ("model.layers.0.mlp.gate_proj.weight", &[16, 8][..]),
        ("model.layers.0.mlp.up_proj.weight", &[16, 8][..]),
        ("model.layers.0.mlp.down_proj.weight", &[8, 16][..]),
    ] {
        let tensor = Tensor::ones(shape, DType::F32, &Device::Cpu)
            .map_err(|error| format!("create required model tensor {name}: {error}"))?;
        prepared.final_tensors.insert(name.to_owned(), tensor);
    }
    Ok(())
}

fn failure_code(error: LoadError) -> Option<u32> {
    match error {
        LoadError::Backend(failure) => Some(failure.failure.code),
        _ => None,
    }
}

fn assert_tensor_context(
    error: LoadError,
    expected_stage: LoadFailureStage,
    expected_location: TensorFailureLocation,
) {
    assert!(matches!(
        error,
        LoadError::Backend(failure)
            if failure.context
                == Some(LoadFailureContext::tensor(expected_stage, expected_location))
    ));
}

fn required_error<T>(result: Result<T, LoadError>, context: &str) -> Result<LoadError, String> {
    result.err().ok_or_else(|| context.to_owned())
}

fn first_shard_mut(prepared: &mut CandleLlamaPreparedLoad) -> Result<&mut InspectedShard, String> {
    prepared
        .shards
        .first_mut()
        .ok_or_else(|| "prepared load has no retained shard".to_owned())
}

fn test_prepared(
    shards: Vec<InspectedShard>,
    execution_dtype: DType,
) -> Result<CandleLlamaPreparedLoad, String> {
    let backend = BackendId::new(7);
    let execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
    let descriptor = ModelDescriptor {
        backend,
        metadata: ModelMetadata {
            architecture: ModelArchitecture::Llama,
            configuration_declared_scalar_type: Some(ScalarType::F32),
            observed_tensor_scalar_types: ScalarTypeSet::from_scalar(ScalarType::F32),
            quantization: QuantizationFormat::None,
            vocabulary_size: 16,
            context_length: 16,
        },
        capabilities: ModelCapabilities {
            operations: CapabilitySet::EXPLICIT_SYNCHRONIZATION,
            maximum_context_tokens: 16,
            maximum_sequences: 1,
            maximum_prefill_batch: 16,
        },
        estimated_footprint: MemoryFootprint::default(),
        sequence_cache_bytes_per_token: 0,
    };
    let plan = LoadPlan {
        accepted_configuration: LoadConfiguration {
            handle: ModelHandle::new(ModelId::new(1), ModelGeneration::new(1)),
            execution_device,
            memory_budget: MemoryBudget::default(),
        },
        descriptor,
        execution_scalar_type: ScalarType::F32,
        final_footprint: MemoryFootprint::default(),
        loading_peak_footprint: MemoryFootprint::default(),
    };
    let mut final_tensors = HashMap::new();
    final_tensors
        .try_reserve(16)
        .map_err(|error| error.to_string())?;
    Ok(CandleLlamaPreparedLoad {
        backend,
        plan,
        config: Some(test_config()),
        execution_dtype,
        device: Some(Device::Cpu),
        shards,
        final_tensors,
        pending_source_tensor: None,
        pending_host_tensor: None,
        pending_device_tensor: None,
        transfer_plan: None,
        transfer_batch: None,
        next_transfer_batch_index: 0,
        next_transfer_entry_index: 0,
        constructed_model: None,
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        load_observation: None,
        #[cfg(feature = "cuda-hardware-tests")]
        hardware_load_fault: None,
        cleanup_complete: false,
    })
}

fn configure_test_device(
    prepared: &mut CandleLlamaPreparedLoad,
    device_kind: DeviceKind,
) -> Result<(), String> {
    prepared.plan.accepted_configuration.execution_device =
        ExecutionDevice::new(DeviceId::new(0), device_kind);
    match device_kind {
        DeviceKind::Cpu => {
            prepared.transfer_plan = None;
            prepared.transfer_batch = None;
        }
        DeviceKind::Cuda => {
            prepared.transfer_plan = Some(
                TransferPlan::build(prepared.backend, &prepared.shards, prepared.execution_dtype)
                    .map_err(|error| format!("test transfer plan: {error:?}"))?,
            );
            prepared.transfer_batch = Some(
                TransferBatchOwner::allocate(prepared.backend, MAXIMUM_BATCH_ENTRIES)
                    .map_err(|error| format!("test transfer owner: {error:?}"))?,
            );
        }
        _ => return Err("unsupported test device".to_owned()),
    }
    prepared.next_transfer_batch_index = 0;
    prepared.next_transfer_entry_index = 0;
    Ok(())
}

fn test_config() -> Config {
    Config {
        hidden_size: 8,
        intermediate_size: 16,
        vocab_size: 16,
        num_hidden_layers: 1,
        num_attention_heads: 2,
        num_key_value_heads: 2,
        use_flash_attn: false,
        rms_norm_eps: 1e-5,
        rope_theta: 10_000.0,
        bos_token_id: None,
        eos_token_id: None,
        rope_scaling: None,
        max_position_embeddings: 16,
        tie_word_embeddings: false,
    }
}
