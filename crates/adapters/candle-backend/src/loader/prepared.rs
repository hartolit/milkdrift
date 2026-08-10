//! Sequential selective materialization and retryable partial-load cleanup.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::llama::{Config, Llama};
use domain_contracts::{
    BackendFailureKind, BackendId, DeviceKind, LoadError, LoadPlan, PreparedLoad,
    SynchronizationError,
};
use sha2::{Digest, Sha256};

use crate::failure::{
    CODE_DUPLICATE_TENSOR, CODE_HEADER_IDENTITY_MISMATCH, CODE_INSPECTION_ALLOCATION,
    CODE_LOAD_SYNCHRONIZE, CODE_MODEL_LOAD, CODE_MODEL_LOAD_PANIC, CODE_NUMERIC_OVERFLOW,
    CODE_PARTIAL_LOAD_SYNCHRONIZE, CODE_PAYLOAD_READ, CODE_SOURCE_IDENTITY_LENGTH,
    CODE_SOURCE_IDENTITY_MISMATCH, CODE_TENSOR_MAP_ALLOCATION, CODE_TENSOR_MATERIALIZE,
    CODE_TENSOR_TRANSFER, CODE_WEIGHT_METADATA, failure,
};

use super::identity::EstablishedIdentityAuthority;
use super::manifest::{InspectedShard, SourceTensorDType, TensorShape};
use super::{
    host_memory_failure, invalid_model_failure, map_candle_load_error, unsupported_scalar,
};

const VERIFICATION_BUFFER_BYTES: usize = 64 * 1024;
const VERIFICATION_BUFFER_BYTES_U64: u64 = 64 * 1024;

#[cfg(test)]
thread_local! {
    static TEST_CLEANUP_SYNCHRONIZATION_FAILURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Exact source-, configuration-, device-, and plan-bound Candle preparation.
///
/// Its retained files, parsed config, selected device, and cleanup authority are
/// not cloned or replaced during loading, and its plan remains stable for the
/// value's lifetime. Before materialization it is ordinary-drop-safe. After any
/// materialization failure it is the sole cleanup owner of every completed and
/// in-flight tensor and any constructed model; [`PreparedLoad::cleanup`] is
/// all-or-nothing, retryable, and idempotent.
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
    pub(super) constructed_model: Option<Llama>,
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
        self.construct_model(observer)
    }

    fn validate_materialization_start(&self) -> Result<(), LoadError> {
        if self.cleanup_complete
            || self.constructed_model.is_some()
            || !self.final_tensors.is_empty()
            || self.pending_source_tensor.is_some()
            || self.pending_host_tensor.is_some()
            || self.pending_device_tensor.is_some()
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
            .and_then(|shard| shard.established_identity)
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
        observer.whole_shard_verified(expected.authority);
        Ok(())
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
        let file = &mut self
            .shards
            .get_mut(shard_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
            .file;
        let mut remaining = byte_count;
        while remaining > 0 {
            let chunk_length = usize::try_from(remaining.min(VERIFICATION_BUFFER_BYTES_U64))
                .map_err(|_| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
            let chunk = buffer
                .get_mut(..chunk_length)
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_PAYLOAD_READ))?;
            file.read_exact(chunk)
                .map_err(|_| invalid_model_failure(self.backend, CODE_SOURCE_IDENTITY_LENGTH))?;
            hasher.update(chunk);
            observer.hashed_range(range, chunk_length);
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
        &self,
        shard_index: usize,
        tensor_index: usize,
    ) -> Result<RequiredTensorFacts, LoadError> {
        let tensor = self
            .shards
            .get(shard_index)
            .and_then(|shard| shard.tensors.get(tensor_index))
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        let mut name = String::new();
        name.try_reserve_exact(tensor.name.len())
            .map_err(|_| host_memory_failure(self.backend, CODE_TENSOR_MAP_ALLOCATION))?;
        name.push_str(tensor.name.as_str());
        Ok(RequiredTensorFacts {
            name,
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
        let alignment = facts
            .source_dtype
            .alignment()
            .ok_or_else(|| unsupported_scalar(self.backend))?;
        let alignment_padding = alignment
            .checked_sub(1)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
        let allocation_bytes = facts
            .source_bytes
            .checked_add(alignment_padding)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
        let allocation_length = usize::try_from(allocation_bytes)
            .map_err(|_| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
        let source_length = usize::try_from(facts.source_bytes)
            .map_err(|_| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
        let alignment = usize::try_from(alignment)
            .map_err(|_| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;

        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(allocation_length)
            .map_err(|_| host_memory_failure(self.backend, CODE_TENSOR_MATERIALIZE))?;
        bytes.resize(allocation_length, 0);
        let start = bytes.as_ptr().align_offset(alignment);
        let end = start
            .checked_add(source_length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE))?;
        let destination = bytes
            .get_mut(start..end)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_PAYLOAD_READ))?;
        self.shards
            .get_mut(shard_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
            .file
            .read_exact(destination)
            .map_err(|_| invalid_model_failure(self.backend, CODE_SOURCE_IDENTITY_LENGTH))?;
        let hashed_bytes = destination.len();
        hasher.update(&*destination);
        observer.hashed_range(HashedRange::RequiredTensor, hashed_bytes);
        Ok(AlignedPayload { bytes, start, end })
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
        let payload_bytes = payload.as_slice(self.backend)?;
        let source_tensor = Tensor::from_raw_buffer(
            payload_bytes,
            source_dtype,
            facts.shape.as_slice(),
            &Device::Cpu,
        )
        .map_err(|error| {
            map_candle_load_error(self.backend, &Device::Cpu, &error, CODE_TENSOR_MATERIALIZE)
        })?;
        self.pending_source_tensor = Some(source_tensor);
        observer.checkpoint(MaterializationCheckpoint::SourceOwned, self.backend)?;
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
        if source_dtype == self.execution_dtype {
            self.pending_host_tensor = self.pending_source_tensor.take();
            observer.checkpoint(MaterializationCheckpoint::HostOwned, self.backend)?;
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
            observer.checkpoint(MaterializationCheckpoint::CastOwned, self.backend)?;
            self.pending_source_tensor = None;
        }
        let host = self
            .pending_host_tensor
            .as_ref()
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
            DeviceKind::Cpu => self.insert_cpu_tensor(facts.name, observer),
            DeviceKind::Cuda => self.transfer_and_insert(facts, observer),
            _ => Err(LoadError::InvalidConfiguration),
        }
    }

    fn insert_cpu_tensor<O: MaterializationObserver>(
        &mut self,
        name: String,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        observer.checkpoint(MaterializationCheckpoint::BeforeMapInsertion, self.backend)?;
        let tensor = self
            .pending_host_tensor
            .take()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE))?;
        match self.final_tensors.entry(name) {
            Entry::Vacant(entry) => {
                entry.insert(tensor);
            }
            Entry::Occupied(entry) => {
                self.pending_host_tensor = Some(tensor);
                let _ = entry;
                return Err(invalid_model_failure(self.backend, CODE_DUPLICATE_TENSOR));
            }
        }
        observer.checkpoint(MaterializationCheckpoint::MapOwned, self.backend)
    }

    fn transfer_and_insert<O: MaterializationObserver>(
        &mut self,
        facts: RequiredTensorFacts,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let device = self
            .device
            .clone()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        let transferred = self
            .pending_host_tensor
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?
            .to_device(&device)
            .map_err(|error| {
                map_candle_load_error(self.backend, &device, &error, CODE_TENSOR_TRANSFER)
            })?;
        self.pending_device_tensor = Some(transferred);
        observer.checkpoint(MaterializationCheckpoint::TransferOwned, self.backend)?;
        let retained_device = self
            .pending_device_tensor
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        if retained_device.dtype() != self.execution_dtype
            || retained_device.dims() != facts.shape.as_slice()
        {
            return Err(invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER));
        }
        device.synchronize().map_err(|_| {
            LoadError::Backend(failure(
                self.backend,
                BackendFailureKind::Synchronization,
                CODE_LOAD_SYNCHRONIZE,
            ))
        })?;
        observer.checkpoint(MaterializationCheckpoint::BeforeMapInsertion, self.backend)?;
        let tensor = self
            .pending_device_tensor
            .take()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
        match self.final_tensors.entry(facts.name) {
            Entry::Vacant(entry) => {
                entry.insert(tensor);
            }
            Entry::Occupied(entry) => {
                self.pending_device_tensor = Some(tensor);
                let _ = entry;
                return Err(invalid_model_failure(self.backend, CODE_DUPLICATE_TENSOR));
            }
        }
        observer.checkpoint(MaterializationCheckpoint::MapOwned, self.backend)?;
        self.pending_host_tensor = None;
        Ok(())
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
        let builder_tensors = checked_tensor_map_clone(self.backend, &self.final_tensors)?;
        let variable_builder =
            VarBuilder::from_tensors(builder_tensors, self.execution_dtype, &device);
        let loaded = catch_unwind(AssertUnwindSafe(|| Llama::load(variable_builder, config)))
            .map_err(|_| invalid_model_failure(self.backend, CODE_MODEL_LOAD_PANIC))?
            .map_err(|error| {
                map_candle_load_error(self.backend, &device, &error, CODE_MODEL_LOAD)
            })?;
        self.constructed_model = Some(loaded);
        observer.checkpoint(MaterializationCheckpoint::ModelOwned, self.backend)?;
        observer.checkpoint(MaterializationCheckpoint::BeforeFinalSync, self.backend)?;
        device.synchronize().map_err(|_| {
            LoadError::Backend(failure(
                self.backend,
                BackendFailureKind::Synchronization,
                CODE_LOAD_SYNCHRONIZE,
            ))
        })?;
        observer.checkpoint(MaterializationCheckpoint::FinalSynchronized, self.backend)
    }
}

impl PreparedLoad for CandleLlamaPreparedLoad {
    fn plan(&self) -> &LoadPlan {
        &self.plan
    }

    fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        if self.cleanup_complete {
            return Ok(());
        }
        #[cfg(test)]
        if TEST_CLEANUP_SYNCHRONIZATION_FAILURES.with(|remaining| {
            let value = remaining.get();
            if value == 0 {
                false
            } else {
                remaining.set(value - 1);
                true
            }
        }) {
            return Err(SynchronizationError::Backend(failure(
                self.backend,
                BackendFailureKind::Synchronization,
                CODE_PARTIAL_LOAD_SYNCHRONIZE,
            )));
        }
        if let Some(device) = &self.device {
            device.synchronize().map_err(|_| {
                SynchronizationError::Backend(failure(
                    self.backend,
                    BackendFailureKind::Synchronization,
                    CODE_PARTIAL_LOAD_SYNCHRONIZE,
                ))
            })?;
        }

        self.constructed_model = None;
        self.final_tensors.clear();
        self.pending_device_tensor = None;
        self.pending_host_tensor = None;
        self.pending_source_tensor = None;
        self.shards.clear();
        self.config = None;
        self.device = None;
        self.cleanup_complete = true;
        Ok(())
    }
}

#[derive(Debug)]
struct RequiredTensorFacts {
    name: String,
    source_dtype: SourceTensorDType,
    shape: TensorShape,
    source_bytes: u64,
}

#[derive(Debug)]
struct AlignedPayload {
    bytes: Vec<u8>,
    start: usize,
    end: usize,
}

impl AlignedPayload {
    fn as_slice(&self, backend: BackendId) -> Result<&[u8], LoadError> {
        self.bytes
            .get(self.start..self.end)
            .ok_or_else(|| invalid_model_failure(backend, CODE_PAYLOAD_READ))
    }
}

fn verification_buffer(backend: BackendId) -> Result<Vec<u8>, LoadError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(VERIFICATION_BUFFER_BYTES)
        .map_err(|_| host_memory_failure(backend, CODE_INSPECTION_ALLOCATION))?;
    buffer.resize(VERIFICATION_BUFFER_BYTES, 0);
    Ok(buffer)
}

fn checked_tensor_map_clone(
    backend: BackendId,
    tensors: &HashMap<String, Tensor>,
) -> Result<HashMap<String, Tensor>, LoadError> {
    let mut cloned = HashMap::new();
    cloned
        .try_reserve(tensors.len())
        .map_err(|_| host_memory_failure(backend, CODE_TENSOR_MAP_ALLOCATION))?;
    for (name, tensor) in tensors {
        let mut cloned_name = String::new();
        cloned_name
            .try_reserve_exact(name.len())
            .map_err(|_| host_memory_failure(backend, CODE_TENSOR_MAP_ALLOCATION))?;
        cloned_name.push_str(name);
        cloned.insert(cloned_name, tensor.clone());
    }
    Ok(cloned)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HashedRange {
    PrefixHeader,
    IgnoredTensor,
    RequiredTensor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MaterializationCheckpoint {
    SourceOwned,
    HostOwned,
    CastOwned,
    TransferOwned,
    BeforeMapInsertion,
    MapOwned,
    ModelOwned,
    BeforeFinalSync,
    FinalSynchronized,
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

    fn whole_shard_verified(&mut self, _authority: EstablishedIdentityAuthority) {}
}

struct NoopMaterializationObserver;

impl MaterializationObserver for NoopMaterializationObserver {}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs::{self, File, OpenOptions};
    use std::io::{Seek, SeekFrom, Write};
    use std::sync::atomic::{AtomicU64, Ordering};

    use candle_core::{DType, Device, Tensor};
    use candle_transformers::models::llama::Config;
    use domain_contracts::{
        BackendId, CapabilitySet, DeviceId, DeviceKind, ExecutionDevice, LoadConfiguration,
        LoadError, LoadPlan, MemoryBudget, MemoryFootprint, ModelArchitecture, ModelCapabilities,
        ModelDescriptor, ModelGeneration, ModelHandle, ModelId, ModelMetadata, PreparedLoad,
        QuantizationFormat, ScalarType, ScalarTypeSet,
    };
    use sha2::{Digest, Sha256};

    use super::{
        CandleLlamaPreparedLoad, EstablishedIdentityAuthority, HashedRange,
        MaterializationCheckpoint, MaterializationObserver, TEST_CLEANUP_SYNCHRONIZATION_FAILURES,
    };
    use crate::failure::{
        CODE_HEADER_IDENTITY_MISMATCH, CODE_SOURCE_IDENTITY_LENGTH, CODE_SOURCE_IDENTITY_MISMATCH,
        CODE_TENSOR_MATERIALIZE,
    };
    use crate::loader::identity::EstablishedShardIdentity;
    use crate::loader::manifest::{
        InspectedShard, InspectedTensor, SourceTensorDType, TensorShape,
    };
    use crate::source::CandleShardIdentity;

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
        verified_authorities: Vec<EstablishedIdentityAuthority>,
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
                MaterializationCheckpoint::SourceOwned => self.source_owned_count += 1,
                MaterializationCheckpoint::CastOwned => self.cast_owned_count += 1,
                MaterializationCheckpoint::TransferOwned => self.transfer_owned_count += 1,
                MaterializationCheckpoint::MapOwned => self.map_owned_count += 1,
                _ => {}
            }
            Ok(())
        }

        fn whole_shard_verified(&mut self, authority: EstablishedIdentityAuthority) {
            self.verified_authorities.push(authority);
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
                Err(super::invalid_model_failure(
                    backend,
                    CODE_TENSOR_MATERIALIZE,
                ))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn ignored_ranges_are_hashed_without_materialization_or_transfer() -> Result<(), String> {
        let header = br#"{"ignored":{"dtype":"U8","shape":[3],"data_offsets":[0,3]},"required":{"dtype":"F32","shape":[1],"data_offsets":[3,7]}}"#;
        let payload = [9_u8, 8, 7, 0, 0, 128, 63];

        for (device_kind, expected_transfer_count) in [(DeviceKind::Cpu, 0), (DeviceKind::Cuda, 1)]
        {
            let tensors = vec![
                inspected_tensor("ignored", SourceTensorDType::U8, &[3], 0, 3, false)?,
                inspected_tensor("required", SourceTensorDType::F32, &[1], 3, 4, true)?,
            ];
            let shard = inspected_shard(header, &payload, tensors)?;
            let mut prepared = test_prepared(vec![shard], DType::F32)?;
            prepared.plan.accepted_configuration.execution_device =
                ExecutionDevice::new(DeviceId::new(0), device_kind);
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
            assert_eq!(
                events.verified_authorities.as_slice(),
                &[EstablishedIdentityAuthority::ProjectEstablished]
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

        let payload_tensor =
            inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
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

        let truncated_tensor =
            inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
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
    fn source_cast_and_map_faults_retain_real_owners() -> Result<(), String> {
        for (checkpoint, source_dtype, execution_dtype) in [
            (
                MaterializationCheckpoint::SourceOwned,
                SourceTensorDType::F32,
                DType::F32,
            ),
            (
                MaterializationCheckpoint::HostOwned,
                SourceTensorDType::F32,
                DType::F32,
            ),
            (
                MaterializationCheckpoint::CastOwned,
                SourceTensorDType::F32,
                DType::F16,
            ),
            (
                MaterializationCheckpoint::BeforeMapInsertion,
                SourceTensorDType::F32,
                DType::F32,
            ),
            (
                MaterializationCheckpoint::MapOwned,
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
            assert!(matches!(error, LoadError::Backend(_)));
            match checkpoint {
                MaterializationCheckpoint::SourceOwned => {
                    assert!(prepared.pending_source_tensor.is_some());
                }
                MaterializationCheckpoint::HostOwned => {
                    assert!(prepared.pending_source_tensor.is_none());
                    assert!(prepared.pending_host_tensor.is_some());
                }
                MaterializationCheckpoint::CastOwned => {
                    assert!(prepared.pending_source_tensor.is_some());
                    assert!(prepared.pending_host_tensor.is_some());
                }
                MaterializationCheckpoint::BeforeMapInsertion => {
                    assert!(prepared.pending_host_tensor.is_some());
                }
                MaterializationCheckpoint::MapOwned => {
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
        prepared.plan.accepted_configuration.execution_device =
            ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);

        let error = required_error(
            prepared.materialize_shard(0, &mut FailAt(MaterializationCheckpoint::TransferOwned)),
            "simulated transfer ownership checkpoint must fail",
        )?;
        assert!(matches!(error, LoadError::Backend(_)));
        assert!(prepared.pending_host_tensor.is_some());
        assert!(prepared.pending_device_tensor.is_some());
        assert!(prepared.final_tensors.is_empty());
        Ok(())
    }

    #[test]
    fn model_construction_and_final_sync_faults_retain_the_owner() -> Result<(), String> {
        let mut missing = test_prepared(Vec::new(), DType::F32)?;
        populate_required_model_tensors(&mut missing)?;
        missing.final_tensors.remove("lm_head.weight");
        required_error(
            missing.construct_model(&mut super::NoopMaterializationObserver),
            "missing native model tensor must fail construction",
        )?;
        assert!(missing.constructed_model.is_none());
        assert!(!missing.final_tensors.is_empty());

        for checkpoint in [
            MaterializationCheckpoint::ModelOwned,
            MaterializationCheckpoint::BeforeFinalSync,
            MaterializationCheckpoint::FinalSynchronized,
        ] {
            let mut prepared = test_prepared(Vec::new(), DType::F32)?;
            populate_required_model_tensors(&mut prepared)?;
            required_error(
                prepared.construct_model(&mut FailAt(checkpoint)),
                "post-construction checkpoint must fail",
            )?;
            assert!(prepared.constructed_model.is_some());
            assert_eq!(prepared.final_tensors.len(), 12);
            prepared
                .cleanup()
                .map_err(|error| format!("cleanup constructed model: {error:?}"))?;
        }
        Ok(())
    }

    #[test]
    fn cleanup_failure_retains_all_handles_and_retry_is_idempotent() -> Result<(), String> {
        let shard = inspected_shard(br"{}", &[], Vec::new())?;
        let mut prepared = test_prepared(vec![shard], DType::F32)?;
        let stable_plan = *prepared.plan();
        let tensor = Tensor::ones(1, DType::F32, &Device::Cpu)
            .map_err(|error| format!("create cleanup tensor: {error}"))?;
        prepared
            .final_tensors
            .insert("final".to_owned(), tensor.clone());
        prepared.pending_source_tensor = Some(tensor.clone());
        prepared.pending_host_tensor = Some(tensor.clone());
        prepared.pending_device_tensor = Some(tensor);

        TEST_CLEANUP_SYNCHRONIZATION_FAILURES.with(|remaining| remaining.set(1));
        assert!(prepared.cleanup().is_err());
        assert_eq!(prepared.final_tensors.len(), 1);
        assert!(prepared.pending_source_tensor.is_some());
        assert!(prepared.pending_host_tensor.is_some());
        assert!(prepared.pending_device_tensor.is_some());
        assert_eq!(prepared.shards.len(), 1);
        assert!(prepared.config.is_some());
        assert!(prepared.device.is_some());
        assert!(!prepared.cleanup_complete);
        assert_eq!(prepared.plan(), &stable_plan);

        prepared
            .cleanup()
            .map_err(|error| format!("retry cleanup: {error:?}"))?;
        assert!(prepared.final_tensors.is_empty());
        assert!(prepared.pending_source_tensor.is_none());
        assert!(prepared.pending_host_tensor.is_none());
        assert!(prepared.pending_device_tensor.is_none());
        assert!(prepared.shards.is_empty());
        assert!(prepared.config.is_none());
        assert!(prepared.device.is_none());
        assert!(prepared.cleanup_complete);
        assert_eq!(prepared.plan(), &stable_plan);
        prepared
            .cleanup()
            .map_err(|error| format!("idempotent cleanup: {error:?}"))?;
        Ok(())
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
            source_identity: CandleShardIdentity::ProjectEstablished {
                byte_length: file_length,
                sha256: whole_sha256,
            },
            established_identity: Some(EstablishedShardIdentity {
                byte_length: file_length,
                sha256: whole_sha256,
                authority: EstablishedIdentityAuthority::ProjectEstablished,
            }),
            tensors,
        })
    }

    fn populate_required_model_tensors(
        prepared: &mut CandleLlamaPreparedLoad,
    ) -> Result<(), String> {
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
            LoadError::Backend(failure) => Some(failure.code),
            _ => None,
        }
    }

    fn required_error<T>(result: Result<T, LoadError>, context: &str) -> Result<LoadError, String> {
        result.err().ok_or_else(|| context.to_owned())
    }

    fn first_shard_mut(
        prepared: &mut CandleLlamaPreparedLoad,
    ) -> Result<&mut InspectedShard, String> {
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
            constructed_model: None,
            cleanup_complete: false,
        })
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
}
