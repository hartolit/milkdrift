//! Sequential selective materialization and retryable partial-load cleanup.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};

use candle_core::{DType, Device, Tensor};
use candle_transformers::models::llama::{Config, Llama};
use domain_contracts::{
    BackendFailureKind, BackendId, DeviceKind, LoadError, LoadPlan, PreparedLoad,
};
use sha2::{Digest, Sha256};

use crate::failure::{
    CODE_DUPLICATE_TENSOR, CODE_HEADER_IDENTITY_MISMATCH, CODE_LOAD_SYNCHRONIZE, CODE_MODEL_LOAD,
    CODE_MODEL_LOAD_PANIC, CODE_NUMERIC_OVERFLOW, CODE_PAYLOAD_READ, CODE_SOURCE_IDENTITY_LENGTH,
    CODE_SOURCE_IDENTITY_MISMATCH, CODE_TENSOR_MATERIALIZE, CODE_TENSOR_TRANSFER,
    CODE_WEIGHT_METADATA, failure,
};

use super::cleanup::CandleLlamaFailedPreparation;
use super::construction::construct_llama;
use super::identity::ContentIdentityEstablishment;
use super::manifest::{InspectedShard, SourceTensorDType, TensorShape};
use super::payload::{AlignedPayload, source_tensor, verification_buffer};
use super::transfer_batch::{TransferBatchEntry, TransferBatchOwner};
use super::transfer_plan::TransferPlan;
use super::{
    VERIFICATION_BUFFER_BYTES_U64, invalid_model_failure, map_candle_load_error, unsupported_scalar,
};

/// Exact source-, configuration-, device-, and plan-bound Candle preparation.
///
/// Its retained files, parsed config, selected device, and materialization
/// authority are not cloned or replaced during loading, and its plan remains
/// stable for the value's lifetime. This pre-attempt typestate is always
/// ordinary-drop-safe. A failed materialization consumes it into
/// [`CandleLlamaFailedPreparation`], which becomes the sole explicit cleanup
/// owner of every completed and in-flight tensor and any constructed model.
#[derive(Debug)]
pub struct CandleLlamaPreparedLoad {
    pub(super) backend: BackendId,
    pub(super) plan: LoadPlan,
    pub(super) config: Option<Config>,
    pub(super) execution_dtype: DType,
    pub(super) device: Option<Device>,
    pub(super) shards: Vec<InspectedShard>,
    pub(super) final_tensors: HashMap<String, Tensor>,
    pub(super) pending_source_tensor: Option<Tensor>,
    pub(super) pending_host_tensor: Option<Tensor>,
    pub(super) pending_device_tensor: Option<Tensor>,
    pub(super) transfer_plan: Option<TransferPlan>,
    pub(super) transfer_batch: Option<TransferBatchOwner>,
    pub(super) next_transfer_batch_index: usize,
    pub(super) next_transfer_entry_index: usize,
    pub(super) constructed_model: Option<Llama>,
    #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
    pub(super) load_observation: Option<crate::CandleLoadObservationRecorder>,
    pub(super) cleanup_complete: bool,
}

impl CandleLlamaPreparedLoad {
    pub(super) fn materialize(&mut self) -> Result<(), LoadError> {
        match catch_unwind(AssertUnwindSafe(|| {
            self.materialize_with_observer(&mut NoopMaterializationObserver)
        })) {
            Ok(result) => result,
            Err(_) => Err(invalid_model_failure(self.backend, CODE_MODEL_LOAD_PANIC)),
        }
    }

    fn materialize_with_observer<O: MaterializationObserver>(
        &mut self,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        self.validate_materialization_start()?;
        for shard_index in 0..self.shards.len() {
            self.materialize_shard(shard_index, observer)?;
        }
        self.validate_transfer_completion()?;
        self.construct_model(observer)
    }

    fn validate_materialization_start(&self) -> Result<(), LoadError> {
        if self.cleanup_complete
            || self.constructed_model.is_some()
            || !self.final_tensors.is_empty()
            || self.pending_source_tensor.is_some()
            || self.pending_host_tensor.is_some()
            || self.pending_device_tensor.is_some()
            || self.next_transfer_batch_index != 0
            || self.next_transfer_entry_index != 0
            || self
                .transfer_batch
                .as_ref()
                .is_some_and(|batch| !batch.is_empty())
        {
            Err(invalid_model_failure(self.backend, CODE_MODEL_LOAD))
        } else {
            Ok(())
        }
    }

    fn materialize_shard<O: MaterializationObserver>(
        &mut self,
        shard_index: usize,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let expected = self
            .shards
            .get(shard_index)
            .and_then(|shard| shard.established_content_identity)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        self.validate_shard_length(shard_index, expected.byte_length)?;
        self.seek_shard_start(shard_index)?;

        let mut hasher = Sha256::new();
        let mut verification_buffer = verification_buffer(self.backend)?;
        let header_bytes = self
            .shards
            .get(shard_index)
            .map(|shard| shard.data_start)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        self.read_ignored_range(
            shard_index,
            header_bytes,
            HashedRange::PrefixHeader,
            verification_buffer.as_mut_slice(),
            &mut hasher,
            observer,
        )?;
        let observed_header: [u8; 32] = hasher.clone().finalize().into();
        let expected_header = self
            .shards
            .get(shard_index)
            .map(|shard| shard.prefix_header_sha256)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        if observed_header != expected_header {
            return Err(invalid_model_failure(
                self.backend,
                CODE_HEADER_IDENTITY_MISMATCH,
            ));
        }

        let tensor_count = self
            .shards
            .get(shard_index)
            .map(|shard| shard.tensors.len())
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        for tensor_index in 0..tensor_count {
            let required = self
                .shards
                .get(shard_index)
                .and_then(|shard| shard.tensors.get(tensor_index))
                .map(|tensor| tensor.required)
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
            if required {
                let facts = self.required_tensor_facts(shard_index, tensor_index)?;
                let payload =
                    self.read_required_payload(shard_index, &facts, &mut hasher, observer)?;
                self.materialize_required_tensor(facts, payload, observer)?;
            } else {
                let source_bytes = self
                    .shards
                    .get(shard_index)
                    .and_then(|shard| shard.tensors.get(tensor_index))
                    .map(|tensor| tensor.source_bytes)
                    .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
                self.read_ignored_range(
                    shard_index,
                    source_bytes,
                    HashedRange::IgnoredTensor,
                    verification_buffer.as_mut_slice(),
                    &mut hasher,
                    observer,
                )?;
            }
        }

        self.verify_shard_eof(shard_index, expected.byte_length)?;
        let observed_sha256: [u8; 32] = hasher.finalize().into();
        if observed_sha256 != expected.sha256 {
            return Err(invalid_model_failure(
                self.backend,
                CODE_SOURCE_IDENTITY_MISMATCH,
            ));
        }
        observer.whole_shard_verified(expected.establishment);
        self.flush_shard_final_batch(shard_index, observer)
    }

    fn validate_shard_length(
        &self,
        shard_index: usize,
        expected_length: u64,
    ) -> Result<(), LoadError> {
        let shard = self
            .shards
            .get(shard_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        let current_length = shard
            .file
            .metadata()
            .map_err(|_| invalid_model_failure(self.backend, CODE_SOURCE_IDENTITY_LENGTH))?
            .len();
        if current_length != shard.file_length || current_length != expected_length {
            Err(invalid_model_failure(
                self.backend,
                CODE_SOURCE_IDENTITY_LENGTH,
            ))
        } else {
            Ok(())
        }
    }

    fn seek_shard_start(&mut self, shard_index: usize) -> Result<(), LoadError> {
        self.shards
            .get_mut(shard_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
            .file
            .seek(SeekFrom::Start(0))
            .map(|_| ())
            .map_err(|_| invalid_model_failure(self.backend, CODE_PAYLOAD_READ))
    }

    fn read_ignored_range<O: MaterializationObserver>(
        &mut self,
        shard_index: usize,
        byte_count: u64,
        range: HashedRange,
        buffer: &mut [u8],
        hasher: &mut Sha256,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let mut remaining = byte_count;
        while remaining > 0 {
            let chunk_length = usize::try_from(remaining.min(VERIFICATION_BUFFER_BYTES_U64))
                .map_err(|_| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
            let chunk = buffer
                .get_mut(..chunk_length)
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_PAYLOAD_READ))?;
            self.shards
                .get_mut(shard_index)
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
                .file
                .read_exact(chunk)
                .map_err(|_| invalid_model_failure(self.backend, CODE_SOURCE_IDENTITY_LENGTH))?;
            hasher.update(chunk);
            observer.hashed_range(range, chunk_length);
            self.record_verification_only_bytes(chunk_length);
            remaining = remaining
                .checked_sub(
                    u64::try_from(chunk_length)
                        .map_err(|_| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?,
                )
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
        }
        Ok(())
    }

    fn required_tensor_facts(
        &mut self,
        shard_index: usize,
        tensor_index: usize,
    ) -> Result<RequiredTensorFacts, LoadError> {
        let tensor = self
            .shards
            .get_mut(shard_index)
            .and_then(|shard| shard.tensors.get_mut(tensor_index))
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        Ok(RequiredTensorFacts {
            shard_index,
            tensor_index,
            name: std::mem::take(&mut tensor.name),
            source_dtype: tensor.source_dtype,
            shape: tensor.shape,
            source_bytes: tensor.source_bytes,
        })
    }

    fn read_required_payload<O: MaterializationObserver>(
        &mut self,
        shard_index: usize,
        facts: &RequiredTensorFacts,
        hasher: &mut Sha256,
        observer: &mut O,
    ) -> Result<AlignedPayload, LoadError> {
        let mut payload =
            AlignedPayload::allocate(self.backend, facts.source_dtype, facts.source_bytes)?;
        let destination = payload.as_mut_slice(self.backend)?;
        self.shards
            .get_mut(shard_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
            .file
            .read_exact(destination)
            .map_err(|_| invalid_model_failure(self.backend, CODE_SOURCE_IDENTITY_LENGTH))?;
        let hashed_bytes = destination.len();
        hasher.update(&*destination);
        observer.hashed_range(HashedRange::RequiredTensor, hashed_bytes);
        self.record_required_bytes(hashed_bytes);
        Ok(payload)
    }

    fn materialize_required_tensor<O: MaterializationObserver>(
        &mut self,
        facts: RequiredTensorFacts,
        payload: AlignedPayload,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        if self.final_tensors.contains_key(facts.name.as_str()) {
            return Err(invalid_model_failure(self.backend, CODE_DUPLICATE_TENSOR));
        }
        let source_dtype = facts
            .source_dtype
            .executable_dtype()
            .ok_or_else(|| unsupported_scalar(self.backend))?;
        let source_tensor = source_tensor(self.backend, &payload, source_dtype, facts.shape)?;
        self.pending_source_tensor = Some(source_tensor);
        observer.checkpoint(
            MaterializationCheckpoint::SourceOwned {
                shard_index: facts.shard_index,
                tensor_index: facts.tensor_index,
            },
            self.backend,
        )?;
        drop(payload);
        self.validate_pending_source(&facts, source_dtype)?;
        self.cast_if_required(&facts, source_dtype, observer)?;
        self.retain_on_execution_device(facts, observer)
    }

    fn validate_pending_source(
        &self,
        facts: &RequiredTensorFacts,
        source_dtype: DType,
    ) -> Result<(), LoadError> {
        let retained_source = self
            .pending_source_tensor
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE))?;
        if retained_source.dtype() == source_dtype
            && retained_source.dims() == facts.shape.as_slice()
        {
            Ok(())
        } else {
            Err(invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE))
        }
    }

    fn cast_if_required<O: MaterializationObserver>(
        &mut self,
        facts: &RequiredTensorFacts,
        source_dtype: DType,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let accelerator =
            self.plan.accepted_configuration.execution_device.kind == DeviceKind::Cuda;
        if source_dtype == self.execution_dtype {
            if accelerator {
                self.pending_host_tensor = None;
            } else {
                self.pending_host_tensor = self.pending_source_tensor.take();
            }
            observer.checkpoint(
                MaterializationCheckpoint::HostOwned {
                    shard_index: facts.shard_index,
                    tensor_index: facts.tensor_index,
                },
                self.backend,
            )?;
        } else {
            let converted = self
                .pending_source_tensor
                .as_ref()
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE))?
                .to_dtype(self.execution_dtype)
                .map_err(|error| {
                    map_candle_load_error(
                        self.backend,
                        &Device::Cpu,
                        &error,
                        CODE_TENSOR_MATERIALIZE,
                    )
                })?;
            self.pending_host_tensor = Some(converted);
            observer.checkpoint(
                MaterializationCheckpoint::CastOwned {
                    shard_index: facts.shard_index,
                    tensor_index: facts.tensor_index,
                },
                self.backend,
            )?;
            if !accelerator {
                self.pending_source_tensor = None;
            }
        }
        let host = self
            .pending_host_tensor
            .as_ref()
            .or(self.pending_source_tensor.as_ref())
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE))?;
        if host.dtype() != self.execution_dtype || host.dims() != facts.shape.as_slice() {
            return Err(invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE));
        }
        Ok(())
    }

    fn retain_on_execution_device<O: MaterializationObserver>(
        &mut self,
        facts: RequiredTensorFacts,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        match self.plan.accepted_configuration.execution_device.kind {
            DeviceKind::Cpu => self.insert_cpu_tensor(facts, observer),
            DeviceKind::Cuda => self.enqueue_transfer(facts, observer),
            _ => Err(LoadError::InvalidConfiguration),
        }
    }

    fn insert_cpu_tensor<O: MaterializationObserver>(
        &mut self,
        facts: RequiredTensorFacts,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        observer.checkpoint(
            MaterializationCheckpoint::BeforeCpuMapInsertion {
                shard_index: facts.shard_index,
                tensor_index: facts.tensor_index,
            },
            self.backend,
        )?;
        let tensor = self
            .pending_host_tensor
            .take()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE))?;
        match self.final_tensors.entry(facts.name) {
            Entry::Vacant(entry) => {
                entry.insert(tensor);
            }
            Entry::Occupied(entry) => {
                self.pending_host_tensor = Some(tensor);
                let _ = entry;
                return Err(invalid_model_failure(self.backend, CODE_DUPLICATE_TENSOR));
            }
        }
        observer.checkpoint(
            MaterializationCheckpoint::CpuMapOwned {
                shard_index: facts.shard_index,
                tensor_index: facts.tensor_index,
            },
            self.backend,
        )
    }

    fn enqueue_transfer<O: MaterializationObserver>(
        &mut self,
        facts: RequiredTensorFacts,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let (batch_index, entry_index, planned_entries, expected) =
            self.current_transfer_expectation(&facts)?;
        if entry_index == 0 {
            self.transfer_batch
                .as_mut()
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?
                .begin(self.backend, batch_index, planned_entries)?;
            self.record_transfer_batch_started();
        }
        let device = self
            .device
            .clone()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        let host = self
            .pending_host_tensor
            .as_ref()
            .or(self.pending_source_tensor.as_ref())
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        let next_batch_bytes = self
            .transfer_batch
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?
            .preflight_push(
                self.backend,
                expected.retained_host_bytes(),
                expected.execution_bytes(),
            )?;
        let transferred = self
            .pending_host_tensor
            .as_ref()
            .unwrap_or(host)
            .to_device(&device)
            .map_err(|error| {
                map_candle_load_error(self.backend, &device, &error, CODE_TENSOR_TRANSFER)
            })?;
        self.pending_device_tensor = Some(transferred);
        let retained_device = self
            .pending_device_tensor
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        if retained_device.dtype() != self.execution_dtype
            || retained_device.dims() != facts.shape.as_slice()
        {
            return Err(invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER));
        }
        let transfer_batch = self
            .transfer_batch
            .as_mut()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        let source_tensor = self.pending_source_tensor.take();
        let converted_host_tensor = self.pending_host_tensor.take();
        let device_tensor = self.pending_device_tensor.take();
        let (source_tensor, device_tensor) = match (source_tensor, device_tensor) {
            (Some(source_tensor), Some(device_tensor)) => (source_tensor, device_tensor),
            (source_tensor, device_tensor) => {
                self.pending_source_tensor = source_tensor;
                self.pending_host_tensor = converted_host_tensor;
                self.pending_device_tensor = device_tensor;
                return Err(invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER));
            }
        };
        let entry = TransferBatchEntry::new(
            facts.shard_index,
            facts.tensor_index,
            facts.name,
            source_tensor,
            converted_host_tensor,
            device_tensor,
            expected.retained_host_bytes(),
            expected.execution_bytes(),
        );
        // Preflight occurred before the asynchronous transfer. The owner is
        // still present and its maximum Vec capacity was reserved during
        // preparation, so moving the endpoints into it performs no fallible
        // work and cannot strand them in a temporary on an error path.
        transfer_batch.push_preflighted(entry, next_batch_bytes);
        self.next_transfer_entry_index = self
            .next_transfer_entry_index
            .checked_add(1)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
        observer.checkpoint(
            MaterializationCheckpoint::TransferEnqueued {
                batch_index,
                entry_index,
            },
            self.backend,
        )?;

        let is_complete = self.next_transfer_entry_index == planned_entries;
        let is_last_in_shard = self
            .transfer_plan
            .as_ref()
            .and_then(|plan| plan.batch(batch_index))
            .is_some_and(super::transfer_plan::TransferBatchPlan::is_last_in_shard);
        if is_complete && !is_last_in_shard {
            self.flush_transfer_batch(observer)?;
        }
        Ok(())
    }

    fn current_transfer_expectation(
        &self,
        facts: &RequiredTensorFacts,
    ) -> Result<(usize, usize, usize, super::transfer_plan::TransferEntryPlan), LoadError> {
        let batch_index = self.next_transfer_batch_index;
        let entry_index = self.next_transfer_entry_index;
        let plan = self
            .transfer_plan
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        let batch = plan
            .batch(batch_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        let expected = plan
            .entry(batch_index, entry_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        if expected.coordinate() != (facts.shard_index, facts.tensor_index)
            || expected.source_bytes() != facts.source_bytes
            || batch.shard_index() != facts.shard_index
        {
            return Err(invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER));
        }
        Ok((batch_index, entry_index, batch.entry_count(), expected))
    }

    fn flush_shard_final_batch<O: MaterializationObserver>(
        &mut self,
        shard_index: usize,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        if self.plan.accepted_configuration.execution_device.kind != DeviceKind::Cuda {
            return Ok(());
        }
        let Some(batch) = self
            .transfer_plan
            .as_ref()
            .and_then(|plan| plan.batch(self.next_transfer_batch_index))
        else {
            return Ok(());
        };
        if batch.shard_index() > shard_index {
            return Ok(());
        }
        if batch.shard_index() != shard_index
            || !batch.is_last_in_shard()
            || self.next_transfer_entry_index != batch.entry_count()
        {
            return Err(invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER));
        }
        self.flush_transfer_batch(observer)
    }

    fn flush_transfer_batch<O: MaterializationObserver>(
        &mut self,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let batch_index = self.next_transfer_batch_index;
        let entries = self.next_transfer_entry_index;
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        self.transfer_batch
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?
            .validate_ready(self.backend, self.execution_dtype)?;
        observer.checkpoint(
            MaterializationCheckpoint::BeforeBatchSynchronization {
                batch_index,
                entries,
            },
            self.backend,
        )?;
        self.record_loading_synchronization();
        observer.synchronize(
            LoadingSynchronization::TransferBatch { batch_index },
            self.backend,
            device,
        )?;
        self.transfer_batch
            .as_mut()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?
            .mark_synchronized(self.backend)?;
        observer.checkpoint(
            MaterializationCheckpoint::BatchSynchronized {
                batch_index,
                entries,
            },
            self.backend,
        )?;
        observer.checkpoint(
            MaterializationCheckpoint::BeforeBatchCommit {
                batch_index,
                entries,
            },
            self.backend,
        )?;
        for entry_index in 0..entries {
            let (shard_index, tensor_index) = self
                .transfer_batch
                .as_mut()
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?
                .commit_next(self.backend, &mut self.final_tensors)?;
            observer.checkpoint(
                MaterializationCheckpoint::BatchEntryCommitted {
                    batch_index,
                    entry_index,
                    shard_index,
                    tensor_index,
                },
                self.backend,
            )?;
        }
        observer.checkpoint(
            MaterializationCheckpoint::BatchCommitted {
                batch_index,
                entries,
            },
            self.backend,
        )?;
        self.transfer_batch
            .as_mut()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?
            .finish(self.backend)?;
        self.next_transfer_batch_index = self
            .next_transfer_batch_index
            .checked_add(1)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
        self.next_transfer_entry_index = 0;
        Ok(())
    }

    fn validate_transfer_completion(&self) -> Result<(), LoadError> {
        match self.plan.accepted_configuration.execution_device.kind {
            DeviceKind::Cpu => {
                if self.transfer_plan.is_none() && self.transfer_batch.is_none() {
                    Ok(())
                } else {
                    Err(invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))
                }
            }
            DeviceKind::Cuda => {
                let plan = self
                    .transfer_plan
                    .as_ref()
                    .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
                let batch = self
                    .transfer_batch
                    .as_ref()
                    .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
                if self.next_transfer_batch_index == plan.len()
                    && self.next_transfer_entry_index == 0
                    && batch.is_empty()
                {
                    Ok(())
                } else {
                    Err(invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))
                }
            }
            _ => Err(LoadError::InvalidConfiguration),
        }
    }

    fn record_verification_only_bytes(&self, bytes: usize) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.verification_only_bytes_read(u64::try_from(bytes).unwrap_or(u64::MAX));
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = (self, bytes);
    }

    fn record_required_bytes(&self, bytes: usize) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.required_and_verified_bytes_read(u64::try_from(bytes).unwrap_or(u64::MAX));
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = (self, bytes);
    }

    fn record_transfer_batch_started(&self) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.transfer_batches_started(1);
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = self;
    }

    fn record_loading_synchronization(&self) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.loading_device_synchronizations_started(1);
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = self;
    }

    pub(super) fn record_materialization_started(&self) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.materialization_started();
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = self;
    }

    pub(super) fn record_materialization_failed(&self) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.materialization_failed();
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = self;
    }

    pub(super) fn record_materialization_succeeded(&self) {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &self.load_observation {
            observation.materialization_succeeded();
        }
        #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
        let _ = self;
    }

    fn verify_shard_eof(
        &mut self,
        shard_index: usize,
        expected_length: u64,
    ) -> Result<(), LoadError> {
        let shard = self
            .shards
            .get_mut(shard_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        let mut trailing = [0_u8; 1];
        if shard
            .file
            .read(&mut trailing)
            .map_err(|_| invalid_model_failure(self.backend, CODE_PAYLOAD_READ))?
            != 0
        {
            return Err(invalid_model_failure(
                self.backend,
                CODE_SOURCE_IDENTITY_LENGTH,
            ));
        }
        let final_length = shard
            .file
            .metadata()
            .map_err(|_| invalid_model_failure(self.backend, CODE_SOURCE_IDENTITY_LENGTH))?
            .len();
        if final_length != shard.file_length || final_length != expected_length {
            return Err(invalid_model_failure(
                self.backend,
                CODE_SOURCE_IDENTITY_LENGTH,
            ));
        }
        Ok(())
    }

    fn construct_model<O: MaterializationObserver>(
        &mut self,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let device = self
            .device
            .clone()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_MODEL_LOAD))?;
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_MODEL_LOAD))?;
        let loaded = construct_llama(
            self.backend,
            &self.final_tensors,
            self.execution_dtype,
            &device,
            config,
        )?;
        self.constructed_model = Some(loaded);
        observer.checkpoint(MaterializationCheckpoint::ModelOwned, self.backend)?;
        Ok(())
    }
}

impl PreparedLoad for CandleLlamaPreparedLoad {
    type Failed = CandleLlamaFailedPreparation;

    fn plan(&self) -> &LoadPlan {
        &self.plan
    }
}

#[derive(Debug)]
struct RequiredTensorFacts {
    shard_index: usize,
    tensor_index: usize,
    name: String,
    source_dtype: SourceTensorDType,
    shape: TensorShape,
    source_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HashedRange {
    PrefixHeader,
    IgnoredTensor,
    RequiredTensor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MaterializationCheckpoint {
    SourceOwned {
        shard_index: usize,
        tensor_index: usize,
    },
    HostOwned {
        shard_index: usize,
        tensor_index: usize,
    },
    CastOwned {
        shard_index: usize,
        tensor_index: usize,
    },
    TransferEnqueued {
        batch_index: usize,
        entry_index: usize,
    },
    BeforeBatchSynchronization {
        batch_index: usize,
        entries: usize,
    },
    BatchSynchronized {
        batch_index: usize,
        entries: usize,
    },
    BeforeBatchCommit {
        batch_index: usize,
        entries: usize,
    },
    BatchEntryCommitted {
        batch_index: usize,
        entry_index: usize,
        shard_index: usize,
        tensor_index: usize,
    },
    BatchCommitted {
        batch_index: usize,
        entries: usize,
    },
    BeforeCpuMapInsertion {
        shard_index: usize,
        tensor_index: usize,
    },
    CpuMapOwned {
        shard_index: usize,
        tensor_index: usize,
    },
    ModelOwned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoadingSynchronization {
    TransferBatch { batch_index: usize },
}

trait MaterializationObserver {
    fn hashed_range(&mut self, _range: HashedRange, _bytes: usize) {}

    fn checkpoint(
        &mut self,
        _checkpoint: MaterializationCheckpoint,
        _backend: BackendId,
    ) -> Result<(), LoadError> {
        Ok(())
    }

    fn whole_shard_verified(&mut self, _establishment: ContentIdentityEstablishment) {}

    fn synchronize(
        &mut self,
        boundary: LoadingSynchronization,
        backend: BackendId,
        device: &Device,
    ) -> Result<(), LoadError> {
        let _ = boundary;
        device.synchronize().map_err(|_| {
            LoadError::Backend(failure(
                backend,
                BackendFailureKind::Synchronization,
                CODE_LOAD_SYNCHRONIZE,
            ))
        })
    }
}

struct NoopMaterializationObserver;

impl MaterializationObserver for NoopMaterializationObserver {}

#[cfg(test)]
mod tests;
