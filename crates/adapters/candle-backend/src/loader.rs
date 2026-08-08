//! Llama source inspection, exact load preparation, and transactional materialization.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Formatter;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::llama::{Config, Llama, LlamaConfig};
use domain_contracts::{
    BackendFailureKind, BackendId, CapabilitySet, CapacityExhausted, CapacityResource, DeviceKind,
    ExecutionDevice, FailedLoad, LoadConfiguration, LoadError, LoadPlan, MemoryBudget,
    MemoryFootprint, MemoryKind, ModelArchitecture, ModelCapabilities, ModelDescriptor,
    ModelLoader, ModelMetadata, PreparedLoad, QuantizationFormat, ScalarType, ScalarTypeSet,
    SynchronizationError,
};
use safetensors::tensor::{Dtype as SafeDtype, Metadata, TensorInfo};
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};

use crate::device::{CandleDeviceSummary, prepare_execution_device};
use crate::failure::{
    CODE_CONFIG_DECODE, CODE_CONFIG_READ, CODE_DUPLICATE_TENSOR, CODE_HEADER_ALLOCATION,
    CODE_HEADER_BOUNDS, CODE_HEADER_DECODE, CODE_LOAD_SYNCHRONIZE, CODE_MODEL_LOAD,
    CODE_MODEL_LOAD_PANIC, CODE_NUMERIC_OVERFLOW, CODE_PARTIAL_LOAD_SYNCHRONIZE, CODE_PAYLOAD_READ,
    CODE_PREPARED_PAYLOAD_CHANGED, CODE_REQUIRED_TENSOR, CODE_TENSOR_MATERIALIZE,
    CODE_TENSOR_TRANSFER, CODE_UNSUPPORTED_SCALAR, CODE_UNSUPPORTED_TENSOR_DTYPE,
    CODE_WEIGHT_METADATA, candle_cuda_failure_kind, failure,
};
use crate::model::{CandleLlamaModel, CandleLlamaModelParameters};
use crate::source::CandleLlamaSource;

const SAFETENSORS_PREFIX_BYTES: u64 = 8;
const MAX_SELECTED_SHARDS: usize = 256;
const MAX_SELECTED_SHARDS_U64: u64 = 256;
const MAX_AGGREGATE_HEADER_BYTES: u64 = 100_000_000;
const PAYLOAD_DIGEST_BYTES: usize = 32;
const PAYLOAD_DIGEST_BUFFER_BYTES: usize = 16 * 1024;
const PAYLOAD_DIGEST_BUFFER_BYTES_U64: u64 = 16 * 1024;

#[cfg(test)]
static TEST_CLEANUP_SYNCHRONIZATION_FAILURES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Cold-path loader for unquantized Hugging Face Llama Safetensors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandleLlamaLoader {
    backend: BackendId,
}

impl CandleLlamaLoader {
    /// Creates a loader with the stable backend identifier assigned by the app.
    #[must_use]
    pub const fn new(backend: BackendId) -> Self {
        Self { backend }
    }

    /// Returns this adapter's backend identifier.
    #[must_use]
    pub const fn backend_id(self) -> BackendId {
        self.backend
    }

    /// Initializes an explicitly selected device and returns stable observed facts.
    ///
    /// CPU discovery does not load a CUDA library in a default build. A CUDA
    /// request in a build without the `cuda` feature fails explicitly.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] when the identity is invalid, support was not
    /// compiled, or native device initialization or discovery fails.
    pub fn discover_device(
        self,
        execution_device: ExecutionDevice,
    ) -> Result<CandleDeviceSummary, LoadError> {
        prepare_execution_device(self.backend, execution_device).map(|prepared| prepared.summary)
    }

    fn inspect_source(self, source: &CandleLlamaSource) -> Result<InspectedSource, LoadError> {
        let config_bytes = fs::read(source.config_path())
            .map_err(|_| invalid_model_failure(self.backend, CODE_CONFIG_READ))?;
        let hugging_face: LlamaConfig = serde_json::from_slice(&config_bytes)
            .map_err(|_| invalid_model_failure(self.backend, CODE_CONFIG_DECODE))?;
        let config = hugging_face.into_config(false);
        validate_config(self.backend, &config)?;

        let mut shards = inspect_weight_shards(self.backend, source.weight_paths())?;
        let (observed_tensor_scalar_types, locations) =
            index_observed_tensors(self.backend, &shards)?;
        validate_required_llama_tensors(self.backend, &config, &mut shards, &locations)?;

        let configuration_declared_scalar_type = source.configuration_declared_scalar_type();
        let primary_scalar_type = select_primary_scalar(
            observed_tensor_scalar_types,
            configuration_declared_scalar_type,
        )?;
        let cpu_execution_dtype =
            select_execution_dtype(self.backend, primary_scalar_type, DeviceKind::Cpu, false)?;
        let cpu_footprints = calculate_footprints(
            self.backend,
            &config,
            &shards,
            DeviceKind::Cpu,
            cpu_execution_dtype,
        )?;

        let context_length = u32::try_from(config.max_position_embeddings)
            .map_err(|_| numeric_error(self.backend))?;
        let vocabulary_size =
            u32::try_from(config.vocab_size).map_err(|_| numeric_error(self.backend))?;
        let operations = CapabilitySet::PREFILL
            .union(CapabilitySet::INCREMENTAL_DECODE)
            .union(CapabilitySet::MULTIPLE_SEQUENCES)
            .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
        let descriptor = ModelDescriptor {
            backend: self.backend,
            metadata: ModelMetadata {
                architecture: ModelArchitecture::Llama,
                configuration_declared_scalar_type,
                observed_tensor_scalar_types,
                quantization: QuantizationFormat::None,
                vocabulary_size,
                context_length,
            },
            capabilities: ModelCapabilities {
                operations,
                maximum_context_tokens: context_length,
                maximum_sequences: u32::MAX,
                maximum_prefill_batch: context_length,
            },
            estimated_footprint: cpu_footprints.final_footprint,
        };

        Ok(InspectedSource {
            config,
            descriptor,
            primary_scalar_type,
            shards,
        })
    }
}

impl ModelLoader for CandleLlamaLoader {
    type Source = CandleLlamaSource;
    type Prepared = CandleLlamaPreparedLoad;
    type Model = CandleLlamaModel;

    fn inspect(&self, source: &Self::Source) -> Result<ModelDescriptor, LoadError> {
        self.inspect_source(source)
            .map(|inspected| inspected.descriptor)
    }

    fn prepare_load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Prepared, LoadError> {
        // Header, dtype, duplicate, and required-schema validation is complete
        // before this call can initialize the requested execution device.
        let inspected = self.inspect_source(source)?;
        let prepared_device =
            prepare_execution_device(self.backend, configuration.execution_device)?;
        let execution_dtype = select_execution_dtype(
            self.backend,
            inspected.primary_scalar_type,
            configuration.execution_device.kind,
            prepared_device.summary.supports_bf16,
        )?;
        let execution_scalar_type = execution_scalar_type(self.backend, execution_dtype)?;
        let footprints = calculate_footprints(
            self.backend,
            &inspected.config,
            &inspected.shards,
            configuration.execution_device.kind,
            execution_dtype,
        )?;
        validate_memory_plan(
            self.backend,
            footprints.loading_peak_footprint,
            configuration.memory_budget,
            prepared_device.summary.available_memory_bytes,
        )?;

        let plan = LoadPlan {
            accepted_configuration: *configuration,
            descriptor: inspected.descriptor,
            execution_scalar_type,
            expected_footprint: footprints.final_footprint,
            loading_peak_footprint: footprints.loading_peak_footprint,
        };

        Ok(CandleLlamaPreparedLoad {
            backend: self.backend,
            plan,
            config: Some(inspected.config),
            execution_dtype,
            device: Some(prepared_device.device),
            shards: inspected.shards,
            final_tensors: HashMap::new(),
            pending_source_tensor: None,
            pending_host_tensor: None,
            pending_device_tensor: None,
            constructed_model: None,
            cleanup_complete: false,
        })
    }

    fn load_prepared(
        &mut self,
        mut prepared: Self::Prepared,
    ) -> Result<Self::Model, FailedLoad<Self::Prepared>> {
        if let Err(error) = prepared.materialize() {
            return Err(FailedLoad::new(error, prepared));
        }

        if prepared.constructed_model.is_none()
            || prepared.config.is_none()
            || prepared.device.is_none()
            || prepared.pending_source_tensor.is_some()
            || prepared.pending_host_tensor.is_some()
            || prepared.pending_device_tensor.is_some()
        {
            return Err(FailedLoad::new(
                invalid_model_failure(self.backend, CODE_MODEL_LOAD),
                prepared,
            ));
        }

        let Some(loaded) = prepared.constructed_model.take() else {
            return Err(FailedLoad::new(
                invalid_model_failure(self.backend, CODE_MODEL_LOAD),
                prepared,
            ));
        };
        let Some(config) = prepared.config.take() else {
            prepared.constructed_model = Some(loaded);
            return Err(FailedLoad::new(
                invalid_model_failure(self.backend, CODE_MODEL_LOAD),
                prepared,
            ));
        };
        let Some(device) = prepared.device.take() else {
            prepared.constructed_model = Some(loaded);
            prepared.config = Some(config);
            return Err(FailedLoad::new(
                invalid_model_failure(self.backend, CODE_MODEL_LOAD),
                prepared,
            ));
        };

        // The constructed Llama owns shallow handles for every required tensor.
        // Clearing the load map now releases extras and duplicate handles without
        // duplicating or invalidating model-owned storage.
        prepared.final_tensors.clear();
        prepared.shards.clear();
        prepared.cleanup_complete = true;

        let configuration = prepared.plan.accepted_configuration;
        Ok(CandleLlamaModel::new(
            CandleLlamaModelParameters {
                backend: prepared.backend,
                handle: configuration.handle,
                execution_device: configuration.execution_device,
                descriptor: prepared.plan.descriptor,
                accounted_footprint: prepared.plan.expected_footprint,
                config,
                dtype: prepared.execution_dtype,
                execution_scalar_type: prepared.plan.execution_scalar_type,
                device,
            },
            loaded,
        ))
    }
}

/// Exact source-, device-, and plan-bound Candle load preparation.
///
/// The structure is intentionally opaque. Before materialization it owns only
/// inspected files, metadata, and the selected device and is ordinary-drop-safe.
/// After a failed materialization it is the sole owner of all completed and
/// pending tensors and must be retained until [`PreparedLoad::cleanup`] succeeds.
#[derive(Debug)]
pub struct CandleLlamaPreparedLoad {
    backend: BackendId,
    plan: LoadPlan,
    config: Option<Config>,
    execution_dtype: DType,
    device: Option<Device>,
    shards: Vec<InspectedShard>,
    final_tensors: HashMap<String, Tensor>,
    pending_source_tensor: Option<Tensor>,
    pending_host_tensor: Option<Tensor>,
    pending_device_tensor: Option<Tensor>,
    constructed_model: Option<Llama>,
    cleanup_complete: bool,
}

impl CandleLlamaPreparedLoad {
    fn materialize(&mut self) -> Result<(), LoadError> {
        if self.cleanup_complete
            || self.constructed_model.is_some()
            || !self.final_tensors.is_empty()
            || self.pending_source_tensor.is_some()
            || self.pending_host_tensor.is_some()
            || self.pending_device_tensor.is_some()
        {
            return Err(invalid_model_failure(self.backend, CODE_MODEL_LOAD));
        }

        for shard_index in 0..self.shards.len() {
            let current_length = self
                .shards
                .get(shard_index)
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
                .file
                .metadata()
                .map_err(|_| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
                .len();
            let inspected_length = self
                .shards
                .get(shard_index)
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
                .file_length;
            if current_length != inspected_length {
                return Err(invalid_model_failure(self.backend, CODE_HEADER_BOUNDS));
            }

            let tensor_count = self
                .shards
                .get(shard_index)
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
                .tensors
                .len();
            for tensor_index in 0..tensor_count {
                let tensor = self
                    .shards
                    .get(shard_index)
                    .and_then(|shard| shard.tensors.get(tensor_index))
                    .cloned()
                    .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
                self.materialize_tensor(shard_index, &tensor)?;
            }
        }

        let device = self
            .device
            .clone()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_MODEL_LOAD))?;
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_MODEL_LOAD))?;
        let variable_builder =
            VarBuilder::from_tensors(self.final_tensors.clone(), self.execution_dtype, &device);
        let loaded = catch_unwind(AssertUnwindSafe(|| Llama::load(variable_builder, config)))
            .map_err(|_| invalid_model_failure(self.backend, CODE_MODEL_LOAD_PANIC))?
            .map_err(|error| {
                map_candle_load_error(self.backend, &device, &error, CODE_MODEL_LOAD)
            })?;

        // Store the completed native model before the final synchronization so a
        // synchronization failure still returns one complete cleanup owner.
        self.constructed_model = Some(loaded);
        device.synchronize().map_err(|_| {
            LoadError::Backend(failure(
                self.backend,
                BackendFailureKind::Synchronization,
                CODE_LOAD_SYNCHRONIZE,
            ))
        })?;
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "per-tensor materialization keeps staging, conversion, transfer, synchronization, and ownership transfer in one auditable transaction"
    )]
    fn materialize_tensor(
        &mut self,
        shard_index: usize,
        inspected: &InspectedTensor,
    ) -> Result<(), LoadError> {
        if self.final_tensors.contains_key(&inspected.name) {
            return Err(invalid_model_failure(self.backend, CODE_DUPLICATE_TENSOR));
        }

        let payload = {
            let shard = self
                .shards
                .get_mut(shard_index)
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
            read_aligned_payload(self.backend, shard, inspected)?
        };
        let source_dtype = inspected.source_dtype.candle_dtype();
        let payload_bytes = payload.as_slice(self.backend)?;
        if digest_bytes(payload_bytes) != inspected.payload_digest {
            return Err(invalid_model_failure(
                self.backend,
                CODE_PREPARED_PAYLOAD_CHANGED,
            ));
        }
        let source_tensor =
            Tensor::from_raw_buffer(payload_bytes, source_dtype, &inspected.shape, &Device::Cpu)
                .map_err(|error| {
                    map_candle_load_error(
                        self.backend,
                        &Device::Cpu,
                        &error,
                        CODE_TENSOR_MATERIALIZE,
                    )
                })?;
        self.pending_source_tensor = Some(source_tensor);
        drop(payload);
        let retained_source = self
            .pending_source_tensor
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE))?;
        if retained_source.dtype() != source_dtype || retained_source.dims() != inspected.shape {
            return Err(invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE));
        }

        if source_dtype == self.execution_dtype {
            self.pending_host_tensor = self.pending_source_tensor.take();
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
            self.pending_source_tensor = None;
        }

        let host_tensor = self
            .pending_host_tensor
            .as_ref()
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE))?;
        if host_tensor.dtype() != self.execution_dtype || host_tensor.dims() != inspected.shape {
            return Err(invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE));
        }

        match self.plan.accepted_configuration.execution_device.kind {
            DeviceKind::Cpu => {
                let tensor = self
                    .pending_host_tensor
                    .take()
                    .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_MATERIALIZE))?;
                match self.final_tensors.entry(inspected.name.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(tensor);
                    }
                    Entry::Occupied(_) => {
                        self.pending_host_tensor = Some(tensor);
                        return Err(invalid_model_failure(self.backend, CODE_DUPLICATE_TENSOR));
                    }
                }
            }
            DeviceKind::Cuda => {
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
                // Both transfer endpoints become transaction-owned before any
                // post-transfer validation or asynchronous synchronization.
                self.pending_device_tensor = Some(transferred);
                let retained_device = self
                    .pending_device_tensor
                    .as_ref()
                    .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
                if retained_device.dtype() != self.execution_dtype
                    || retained_device.dims() != inspected.shape
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
                let tensor = self
                    .pending_device_tensor
                    .take()
                    .ok_or_else(|| invalid_model_failure(self.backend, CODE_TENSOR_TRANSFER))?;
                match self.final_tensors.entry(inspected.name.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(tensor);
                    }
                    Entry::Occupied(_) => {
                        self.pending_device_tensor = Some(tensor);
                        return Err(invalid_model_failure(self.backend, CODE_DUPLICATE_TENSOR));
                    }
                }
                self.pending_host_tensor = None;
            }
            _ => return Err(LoadError::InvalidConfiguration),
        }
        Ok(())
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
        if TEST_CLEANUP_SYNCHRONIZATION_FAILURES
            .fetch_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |remaining| remaining.checked_sub(1),
            )
            .is_ok()
        {
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
struct InspectedSource {
    config: Config,
    descriptor: ModelDescriptor,
    primary_scalar_type: ScalarType,
    shards: Vec<InspectedShard>,
}

#[derive(Debug)]
struct PreopenedShard {
    file: File,
    file_length: u64,
    header_length: usize,
    data_start: u64,
}

#[derive(Debug)]
struct InspectedShard {
    file: File,
    file_length: u64,
    data_start: u64,
    tensors: Vec<InspectedTensor>,
}

#[derive(Clone, Debug)]
struct InspectedTensor {
    name: String,
    source_dtype: SourceTensorDType,
    shape: Vec<usize>,
    data_start: u64,
    source_bytes: u64,
    element_count: u64,
    payload_digest: [u8; PAYLOAD_DIGEST_BYTES],
    required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceTensorDType {
    F32,
    F16,
    Bf16,
}

impl SourceTensorDType {
    const fn from_safetensors(dtype: SafeDtype) -> Option<Self> {
        match dtype {
            SafeDtype::F32 => Some(Self::F32),
            SafeDtype::F16 => Some(Self::F16),
            SafeDtype::BF16 => Some(Self::Bf16),
            _ => None,
        }
    }

    const fn scalar_type(self) -> ScalarType {
        match self {
            Self::F32 => ScalarType::F32,
            Self::F16 => ScalarType::F16,
            Self::Bf16 => ScalarType::Bf16,
        }
    }

    const fn candle_dtype(self) -> DType {
        match self {
            Self::F32 => DType::F32,
            Self::F16 => DType::F16,
            Self::Bf16 => DType::BF16,
        }
    }

    const fn bytes_per_element(self) -> u64 {
        match self {
            Self::F32 => 4,
            Self::F16 | Self::Bf16 => 2,
        }
    }

    const fn alignment(self) -> u64 {
        self.bytes_per_element()
    }
}

#[derive(Clone, Copy, Debug)]
struct TensorLocation {
    shard: usize,
    tensor: usize,
}

#[derive(Debug)]
struct ParsedHeader {
    metadata: Option<HashMap<String, String>>,
    tensors: Vec<(String, TensorInfo)>,
    duplicate_key: bool,
}

impl<'de> Deserialize<'de> for ParsedHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ParsedHeaderVisitor)
    }
}

struct ParsedHeaderVisitor;

impl<'de> Visitor<'de> for ParsedHeaderVisitor {
    type Value = ParsedHeader;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a Safetensors header object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut seen = BTreeSet::new();
        let mut metadata = None;
        let mut tensors = Vec::new();
        let mut duplicate_key = false;

        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                let _ignored = map.next_value::<IgnoredAny>()?;
                duplicate_key = true;
                continue;
            }
            if key == "__metadata__" {
                metadata = Some(map.next_value::<HashMap<String, String>>()?);
            } else {
                tensors.push((key, map.next_value::<TensorInfo>()?));
            }
        }

        Ok(ParsedHeader {
            metadata,
            tensors,
            duplicate_key,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CalculatedFootprints {
    final_footprint: MemoryFootprint,
    loading_peak_footprint: MemoryFootprint,
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

#[expect(
    clippy::too_many_lines,
    reason = "header inspection keeps bounded allocation, duplicate-aware parsing, parser validation, payload bounds, and checked tensor facts in one auditable pre-device pass"
)]
fn inspect_weight_shards(
    backend: BackendId,
    paths: &[std::path::PathBuf],
) -> Result<Vec<InspectedShard>, LoadError> {
    if paths.len() > MAX_SELECTED_SHARDS {
        let required = u64::try_from(paths.len()).map_err(|_| numeric_error(backend))?;
        return Err(LoadError::CapacityExhausted(CapacityExhausted::new(
            CapacityResource::BackendScratch,
            required,
            MAX_SELECTED_SHARDS_U64,
        )));
    }
    let mut sorted_paths = paths.to_vec();
    sorted_paths.sort();

    // Open every file and validate every length prefix before allocating any
    // header-sized buffer. The aggregate cap bounds even a large shard list.
    let mut preopened = Vec::new();
    preopened
        .try_reserve_exact(sorted_paths.len())
        .map_err(|_| host_memory_failure(backend, CODE_HEADER_ALLOCATION))?;
    let mut aggregate_header_bytes = 0_u64;
    for path in &sorted_paths {
        let mut file =
            File::open(path).map_err(|_| invalid_model_failure(backend, CODE_WEIGHT_METADATA))?;
        let file_length = file
            .metadata()
            .map_err(|_| invalid_model_failure(backend, CODE_WEIGHT_METADATA))?
            .len();
        let mut prefix = [0_u8; 8];
        file.read_exact(&mut prefix)
            .map_err(|_| invalid_model_failure(backend, CODE_HEADER_BOUNDS))?;
        let header_length_u64 = u64::from_le_bytes(prefix);
        if header_length_u64 == 0 || header_length_u64 > MAX_AGGREGATE_HEADER_BYTES {
            return Err(invalid_model_failure(backend, CODE_HEADER_BOUNDS));
        }
        aggregate_header_bytes = aggregate_header_bytes
            .checked_add(header_length_u64)
            .ok_or_else(|| numeric_error(backend))?;
        if aggregate_header_bytes > MAX_AGGREGATE_HEADER_BYTES {
            return Err(invalid_model_failure(backend, CODE_HEADER_BOUNDS));
        }
        let data_start = SAFETENSORS_PREFIX_BYTES
            .checked_add(header_length_u64)
            .ok_or_else(|| numeric_error(backend))?;
        if data_start > file_length {
            return Err(invalid_model_failure(backend, CODE_HEADER_BOUNDS));
        }
        let header_length = usize::try_from(header_length_u64)
            .map_err(|_| invalid_model_failure(backend, CODE_HEADER_BOUNDS))?;
        preopened.push(PreopenedShard {
            file,
            file_length,
            header_length,
            data_start,
        });
    }

    let mut shards = Vec::new();
    shards
        .try_reserve_exact(preopened.len())
        .map_err(|_| host_memory_failure(backend, CODE_HEADER_ALLOCATION))?;
    for mut shard in preopened {
        let mut header = Vec::new();
        header
            .try_reserve_exact(shard.header_length)
            .map_err(|_| host_memory_failure(backend, CODE_HEADER_ALLOCATION))?;
        header.resize(shard.header_length, 0);
        shard
            .file
            .read_exact(&mut header)
            .map_err(|_| invalid_model_failure(backend, CODE_HEADER_BOUNDS))?;
        if header.first().copied() != Some(b'{') {
            return Err(invalid_model_failure(backend, CODE_HEADER_DECODE));
        }

        let mut deserializer = serde_json::Deserializer::from_slice(&header);
        let mut parsed = ParsedHeader::deserialize(&mut deserializer)
            .map_err(|_| invalid_model_failure(backend, CODE_HEADER_DECODE))?;
        deserializer
            .end()
            .map_err(|_| invalid_model_failure(backend, CODE_HEADER_DECODE))?;
        if parsed.duplicate_key {
            return Err(invalid_model_failure(backend, CODE_DUPLICATE_TENSOR));
        }
        parsed.tensors.sort_by(|left, right| {
            left.1
                .data_offsets
                .cmp(&right.1.data_offsets)
                .then_with(|| left.0.cmp(&right.0))
        });

        let metadata = Metadata::new(parsed.metadata, parsed.tensors.clone())
            .map_err(|_| invalid_model_failure(backend, CODE_HEADER_BOUNDS))?;
        let declared_payload_bytes =
            u64::try_from(metadata.data_len()).map_err(|_| numeric_error(backend))?;
        let actual_payload_bytes = shard
            .file_length
            .checked_sub(shard.data_start)
            .ok_or_else(|| invalid_model_failure(backend, CODE_HEADER_BOUNDS))?;
        if declared_payload_bytes != actual_payload_bytes {
            return Err(invalid_model_failure(backend, CODE_HEADER_BOUNDS));
        }

        let mut tensors = Vec::new();
        tensors
            .try_reserve_exact(parsed.tensors.len())
            .map_err(|_| host_memory_failure(backend, CODE_HEADER_ALLOCATION))?;
        for (name, info) in parsed.tensors {
            let source_dtype =
                SourceTensorDType::from_safetensors(info.dtype).ok_or_else(|| {
                    LoadError::Backend(failure(
                        backend,
                        BackendFailureKind::Unsupported,
                        CODE_UNSUPPORTED_TENSOR_DTYPE,
                    ))
                })?;
            let element_count = checked_element_count(backend, &info.shape)?;
            let source_bytes = element_count
                .checked_mul(source_dtype.bytes_per_element())
                .ok_or_else(|| numeric_error(backend))?;
            let offset_start =
                u64::try_from(info.data_offsets.0).map_err(|_| numeric_error(backend))?;
            let offset_end =
                u64::try_from(info.data_offsets.1).map_err(|_| numeric_error(backend))?;
            let offset_bytes = offset_end
                .checked_sub(offset_start)
                .ok_or_else(|| invalid_model_failure(backend, CODE_HEADER_BOUNDS))?;
            if offset_bytes != source_bytes || offset_end > actual_payload_bytes {
                return Err(invalid_model_failure(backend, CODE_HEADER_BOUNDS));
            }
            let absolute_start = shard
                .data_start
                .checked_add(offset_start)
                .ok_or_else(|| numeric_error(backend))?;
            shard
                .data_start
                .checked_add(offset_end)
                .filter(|absolute_end| *absolute_end <= shard.file_length)
                .ok_or_else(|| invalid_model_failure(backend, CODE_HEADER_BOUNDS))?;
            let payload_digest =
                digest_file_range(backend, &mut shard.file, absolute_start, source_bytes)?;

            tensors.push(InspectedTensor {
                name,
                source_dtype,
                shape: info.shape,
                data_start: offset_start,
                source_bytes,
                element_count,
                payload_digest,
                required: false,
            });
        }
        shards.push(InspectedShard {
            file: shard.file,
            file_length: shard.file_length,
            data_start: shard.data_start,
            tensors,
        });
    }
    Ok(shards)
}

fn index_observed_tensors(
    backend: BackendId,
    shards: &[InspectedShard],
) -> Result<(ScalarTypeSet, BTreeMap<String, TensorLocation>), LoadError> {
    let mut observed = ScalarTypeSet::EMPTY;
    let mut locations = BTreeMap::new();
    for (shard_index, shard) in shards.iter().enumerate() {
        for (tensor_index, tensor) in shard.tensors.iter().enumerate() {
            observed.insert(tensor.source_dtype.scalar_type());
            if locations
                .insert(
                    tensor.name.clone(),
                    TensorLocation {
                        shard: shard_index,
                        tensor: tensor_index,
                    },
                )
                .is_some()
            {
                return Err(invalid_model_failure(backend, CODE_DUPLICATE_TENSOR));
            }
        }
    }
    Ok((observed, locations))
}

fn validate_required_llama_tensors(
    backend: BackendId,
    config: &Config,
    shards: &mut [InspectedShard],
    locations: &BTreeMap<String, TensorLocation>,
) -> Result<(), LoadError> {
    let base_tensor_count = if config.tie_word_embeddings { 2 } else { 3 };
    let required_tensor_count = config
        .num_hidden_layers
        .checked_mul(9)
        .and_then(|count| count.checked_add(base_tensor_count))
        .ok_or_else(|| numeric_error(backend))?;
    if required_tensor_count > locations.len() {
        return Err(invalid_model_failure(backend, CODE_REQUIRED_TENSOR));
    }

    mark_required_tensor(
        backend,
        shards,
        locations,
        "model.embed_tokens.weight",
        &[config.vocab_size, config.hidden_size],
    )?;
    if !config.tie_word_embeddings {
        mark_required_tensor(
            backend,
            shards,
            locations,
            "lm_head.weight",
            &[config.vocab_size, config.hidden_size],
        )?;
    }
    mark_required_tensor(
        backend,
        shards,
        locations,
        "model.norm.weight",
        &[config.hidden_size],
    )?;

    let head_dimension = config.hidden_size / config.num_attention_heads;
    let query_size = head_dimension
        .checked_mul(config.num_attention_heads)
        .ok_or_else(|| numeric_error(backend))?;
    let key_value_size = head_dimension
        .checked_mul(config.num_key_value_heads)
        .ok_or_else(|| numeric_error(backend))?;
    for layer in 0..config.num_hidden_layers {
        let prefix = format!("model.layers.{layer}");
        mark_required_tensor(
            backend,
            shards,
            locations,
            &format!("{prefix}.self_attn.q_proj.weight"),
            &[query_size, config.hidden_size],
        )?;
        for projection in ["k_proj", "v_proj"] {
            mark_required_tensor(
                backend,
                shards,
                locations,
                &format!("{prefix}.self_attn.{projection}.weight"),
                &[key_value_size, config.hidden_size],
            )?;
        }
        mark_required_tensor(
            backend,
            shards,
            locations,
            &format!("{prefix}.self_attn.o_proj.weight"),
            &[config.hidden_size, query_size],
        )?;
        for normalization in ["input_layernorm", "post_attention_layernorm"] {
            mark_required_tensor(
                backend,
                shards,
                locations,
                &format!("{prefix}.{normalization}.weight"),
                &[config.hidden_size],
            )?;
        }
        for projection in ["gate_proj", "up_proj"] {
            mark_required_tensor(
                backend,
                shards,
                locations,
                &format!("{prefix}.mlp.{projection}.weight"),
                &[config.intermediate_size, config.hidden_size],
            )?;
        }
        mark_required_tensor(
            backend,
            shards,
            locations,
            &format!("{prefix}.mlp.down_proj.weight"),
            &[config.hidden_size, config.intermediate_size],
        )?;
    }
    Ok(())
}

fn mark_required_tensor(
    backend: BackendId,
    shards: &mut [InspectedShard],
    locations: &BTreeMap<String, TensorLocation>,
    name: &str,
    expected_shape: &[usize],
) -> Result<(), LoadError> {
    let location = locations
        .get(name)
        .copied()
        .ok_or_else(|| invalid_model_failure(backend, CODE_REQUIRED_TENSOR))?;
    let tensor = shards
        .get_mut(location.shard)
        .and_then(|shard| shard.tensors.get_mut(location.tensor))
        .ok_or_else(|| invalid_model_failure(backend, CODE_REQUIRED_TENSOR))?;
    if tensor.shape != expected_shape {
        return Err(invalid_model_failure(backend, CODE_REQUIRED_TENSOR));
    }
    tensor.required = true;
    Ok(())
}

fn select_primary_scalar(
    observed: ScalarTypeSet,
    declaration: Option<ScalarType>,
) -> Result<ScalarType, LoadError> {
    let f32_set = ScalarTypeSet::from_scalar(ScalarType::F32);
    let f16_set = ScalarTypeSet::from_scalar(ScalarType::F16);
    let bf16_set = ScalarTypeSet::from_scalar(ScalarType::Bf16);
    let primary = if observed == f32_set {
        ScalarType::F32
    } else if observed == f16_set || observed == f16_set.union(f32_set) {
        ScalarType::F16
    } else if observed == bf16_set || observed == bf16_set.union(f32_set) {
        ScalarType::Bf16
    } else {
        return Err(LoadError::UnsupportedFormat);
    };

    match declaration {
        None => Ok(primary),
        Some(ScalarType::F32 | ScalarType::F16 | ScalarType::Bf16)
            if declaration == Some(primary) =>
        {
            Ok(primary)
        }
        Some(_) => Err(LoadError::UnsupportedFormat),
    }
}

fn select_execution_dtype(
    backend: BackendId,
    primary: ScalarType,
    device_kind: DeviceKind,
    supports_bf16: bool,
) -> Result<DType, LoadError> {
    match (primary, device_kind) {
        (ScalarType::F32, DeviceKind::Cpu | DeviceKind::Cuda)
        | (ScalarType::Bf16, DeviceKind::Cpu) => Ok(DType::F32),
        (ScalarType::F16, DeviceKind::Cpu | DeviceKind::Cuda) => Ok(DType::F16),
        (ScalarType::Bf16, DeviceKind::Cuda) if supports_bf16 => Ok(DType::BF16),
        (ScalarType::Bf16, DeviceKind::Cuda) => Err(unsupported_scalar(backend)),
        (_, DeviceKind::Cpu | DeviceKind::Cuda) => Err(LoadError::UnsupportedFormat),
        _ => Err(LoadError::InvalidConfiguration),
    }
}

fn execution_scalar_type(backend: BackendId, dtype: DType) -> Result<ScalarType, LoadError> {
    match dtype {
        DType::F32 => Ok(ScalarType::F32),
        DType::F16 => Ok(ScalarType::F16),
        DType::BF16 => Ok(ScalarType::Bf16),
        _ => Err(unsupported_scalar(backend)),
    }
}

fn calculate_footprints(
    backend: BackendId,
    config: &Config,
    shards: &[InspectedShard],
    device_kind: DeviceKind,
    execution_dtype: DType,
) -> Result<CalculatedFootprints, LoadError> {
    let execution_width =
        dtype_bytes(execution_dtype).ok_or_else(|| unsupported_scalar(backend))?;
    let cache_bytes_per_token = cache_bytes_per_token(backend, config, execution_width)?;
    let mut required_execution_bytes = 0_u64;
    let mut full_map_execution_bytes = 0_u64;
    let mut host_peak = 0_u64;

    for tensor in shards.iter().flat_map(|shard| &shard.tensors) {
        let execution_bytes = tensor
            .element_count
            .checked_mul(execution_width)
            .ok_or_else(|| numeric_error(backend))?;
        if tensor.required {
            required_execution_bytes = required_execution_bytes
                .checked_add(execution_bytes)
                .ok_or_else(|| numeric_error(backend))?;
        }
        let aligned_staging = tensor
            .source_bytes
            .checked_add(tensor.source_dtype.alignment().saturating_sub(1))
            .ok_or_else(|| numeric_error(backend))?;

        match device_kind {
            DeviceKind::Cpu => {
                let raw_peak = full_map_execution_bytes
                    .checked_add(aligned_staging)
                    .and_then(|bytes| bytes.checked_add(tensor.source_bytes))
                    .ok_or_else(|| numeric_error(backend))?;
                host_peak = host_peak.max(raw_peak);
                if tensor.source_dtype.candle_dtype() != execution_dtype {
                    let cast_peak = full_map_execution_bytes
                        .checked_add(tensor.source_bytes)
                        .and_then(|bytes| bytes.checked_add(execution_bytes))
                        .ok_or_else(|| numeric_error(backend))?;
                    host_peak = host_peak.max(cast_peak);
                }
            }
            DeviceKind::Cuda => {
                let raw_peak = aligned_staging
                    .checked_add(tensor.source_bytes)
                    .ok_or_else(|| numeric_error(backend))?;
                host_peak = host_peak.max(raw_peak).max(execution_bytes);
                if tensor.source_dtype.candle_dtype() != execution_dtype {
                    let cast_peak = tensor
                        .source_bytes
                        .checked_add(execution_bytes)
                        .ok_or_else(|| numeric_error(backend))?;
                    host_peak = host_peak.max(cast_peak);
                }
            }
            _ => return Err(LoadError::InvalidConfiguration),
        }
        full_map_execution_bytes = full_map_execution_bytes
            .checked_add(execution_bytes)
            .ok_or_else(|| numeric_error(backend))?;
    }

    match device_kind {
        DeviceKind::Cpu => {
            host_peak = host_peak.max(full_map_execution_bytes);
            let host_headroom = host_peak
                .checked_sub(required_execution_bytes)
                .ok_or_else(|| numeric_error(backend))?;
            Ok(CalculatedFootprints {
                final_footprint: MemoryFootprint {
                    host_weight_bytes: required_execution_bytes,
                    device_weight_bytes: 0,
                    host_working_bytes: 0,
                    device_working_bytes: 0,
                    cache_bytes_per_token,
                },
                loading_peak_footprint: MemoryFootprint {
                    host_weight_bytes: required_execution_bytes,
                    device_weight_bytes: 0,
                    host_working_bytes: host_headroom,
                    device_working_bytes: 0,
                    cache_bytes_per_token,
                },
            })
        }
        DeviceKind::Cuda => {
            let device_headroom = full_map_execution_bytes
                .checked_sub(required_execution_bytes)
                .ok_or_else(|| numeric_error(backend))?;
            Ok(CalculatedFootprints {
                final_footprint: MemoryFootprint {
                    host_weight_bytes: 0,
                    device_weight_bytes: required_execution_bytes,
                    host_working_bytes: 0,
                    device_working_bytes: 0,
                    cache_bytes_per_token,
                },
                loading_peak_footprint: MemoryFootprint {
                    host_weight_bytes: 0,
                    device_weight_bytes: required_execution_bytes,
                    host_working_bytes: host_peak,
                    device_working_bytes: device_headroom,
                    cache_bytes_per_token,
                },
            })
        }
        _ => Err(LoadError::InvalidConfiguration),
    }
}

fn digest_file_range(
    backend: BackendId,
    file: &mut File,
    absolute_start: u64,
    byte_count: u64,
) -> Result<[u8; PAYLOAD_DIGEST_BYTES], LoadError> {
    file.seek(SeekFrom::Start(absolute_start))
        .map_err(|_| invalid_model_failure(backend, CODE_PAYLOAD_READ))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; PAYLOAD_DIGEST_BUFFER_BYTES];
    let mut remaining = byte_count;
    while remaining > 0 {
        let chunk_length = usize::try_from(remaining.min(PAYLOAD_DIGEST_BUFFER_BYTES_U64))
            .map_err(|_| numeric_error(backend))?;
        let chunk = buffer
            .get_mut(..chunk_length)
            .ok_or_else(|| invalid_model_failure(backend, CODE_PAYLOAD_READ))?;
        file.read_exact(chunk)
            .map_err(|_| invalid_model_failure(backend, CODE_PAYLOAD_READ))?;
        hasher.update(chunk);
        remaining = remaining
            .checked_sub(u64::try_from(chunk_length).map_err(|_| numeric_error(backend))?)
            .ok_or_else(|| numeric_error(backend))?;
    }
    Ok(hasher.finalize().into())
}

fn digest_bytes(bytes: &[u8]) -> [u8; PAYLOAD_DIGEST_BYTES] {
    Sha256::digest(bytes).into()
}

fn read_aligned_payload(
    backend: BackendId,
    shard: &mut InspectedShard,
    tensor: &InspectedTensor,
) -> Result<AlignedPayload, LoadError> {
    let alignment_u64 = tensor.source_dtype.alignment();
    let allocation_bytes = tensor
        .source_bytes
        .checked_add(alignment_u64.saturating_sub(1))
        .ok_or_else(|| numeric_error(backend))?;
    let allocation_length =
        usize::try_from(allocation_bytes).map_err(|_| numeric_error(backend))?;
    let source_length = usize::try_from(tensor.source_bytes).map_err(|_| numeric_error(backend))?;
    let alignment = usize::try_from(alignment_u64).map_err(|_| numeric_error(backend))?;

    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(allocation_length)
        .map_err(|_| host_memory_failure(backend, CODE_TENSOR_MATERIALIZE))?;
    bytes.resize(allocation_length, 0);
    let start = bytes.as_ptr().align_offset(alignment);
    let end = start
        .checked_add(source_length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| invalid_model_failure(backend, CODE_TENSOR_MATERIALIZE))?;
    let absolute_start = shard
        .data_start
        .checked_add(tensor.data_start)
        .ok_or_else(|| numeric_error(backend))?;
    shard
        .file
        .seek(SeekFrom::Start(absolute_start))
        .map_err(|_| invalid_model_failure(backend, CODE_PAYLOAD_READ))?;
    let destination = bytes
        .get_mut(start..end)
        .ok_or_else(|| invalid_model_failure(backend, CODE_PAYLOAD_READ))?;
    shard
        .file
        .read_exact(destination)
        .map_err(|_| invalid_model_failure(backend, CODE_PAYLOAD_READ))?;
    Ok(AlignedPayload { bytes, start, end })
}

fn validate_config(backend: BackendId, config: &Config) -> Result<(), LoadError> {
    let required_non_zero = [
        config.hidden_size,
        config.intermediate_size,
        config.vocab_size,
        config.num_hidden_layers,
        config.num_attention_heads,
        config.num_key_value_heads,
        config.max_position_embeddings,
    ];
    let head_dimension = if required_non_zero.contains(&0) {
        return Err(LoadError::InvalidSource);
    } else {
        config.hidden_size / config.num_attention_heads
    };
    if !config
        .hidden_size
        .is_multiple_of(config.num_attention_heads)
        || !config
            .num_attention_heads
            .is_multiple_of(config.num_key_value_heads)
        || !head_dimension.is_multiple_of(2)
    {
        return Err(LoadError::InvalidSource);
    }
    config
        .num_hidden_layers
        .checked_mul(9)
        .and_then(|count| count.checked_add(3))
        .ok_or_else(|| numeric_error(backend))?;
    Ok(())
}

fn checked_element_count(backend: BackendId, shape: &[usize]) -> Result<u64, LoadError> {
    shape.iter().try_fold(1_u64, |total, dimension| {
        let dimension = u64::try_from(*dimension).map_err(|_| numeric_error(backend))?;
        total
            .checked_mul(dimension)
            .ok_or_else(|| numeric_error(backend))
    })
}

fn validate_memory_plan(
    backend: BackendId,
    footprint: MemoryFootprint,
    budget: MemoryBudget,
    currently_available_device_bytes: Option<u64>,
) -> Result<(), LoadError> {
    let required_host = footprint
        .checked_host_bytes()
        .ok_or_else(|| numeric_error(backend))?;
    if required_host > budget.host_bytes {
        return Err(LoadError::InsufficientMemory {
            kind: MemoryKind::Host,
            required_bytes: required_host,
            available_bytes: budget.host_bytes,
        });
    }
    let required_device = footprint
        .checked_device_bytes()
        .ok_or_else(|| numeric_error(backend))?;
    if required_device > budget.device_bytes {
        return Err(LoadError::InsufficientMemory {
            kind: MemoryKind::Device,
            required_bytes: required_device,
            available_bytes: budget.device_bytes,
        });
    }
    if let Some(available) = currently_available_device_bytes
        && required_device > available
    {
        return Err(LoadError::InsufficientMemory {
            kind: MemoryKind::Device,
            required_bytes: required_device,
            available_bytes: available,
        });
    }
    Ok(())
}

fn cache_bytes_per_token(
    backend: BackendId,
    config: &Config,
    scalar_bytes: u64,
) -> Result<u64, LoadError> {
    let head_dimension = config.hidden_size / config.num_attention_heads;
    let factors = [
        u64::try_from(config.num_hidden_layers),
        Ok(2_u64),
        u64::try_from(config.num_key_value_heads),
        u64::try_from(head_dimension),
        Ok(scalar_bytes),
    ];
    factors.into_iter().try_fold(1_u64, |total, factor| {
        let factor = factor.map_err(|_| numeric_error(backend))?;
        total
            .checked_mul(factor)
            .ok_or_else(|| numeric_error(backend))
    })
}

const fn dtype_bytes(dtype: DType) -> Option<u64> {
    match dtype {
        DType::F32 => Some(4),
        DType::F16 | DType::BF16 => Some(2),
        _ => None,
    }
}

const fn unsupported_scalar(backend: BackendId) -> LoadError {
    LoadError::Backend(failure(
        backend,
        BackendFailureKind::Unsupported,
        CODE_UNSUPPORTED_SCALAR,
    ))
}

const fn numeric_error(backend: BackendId) -> LoadError {
    invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW)
}

const fn invalid_model_failure(backend: BackendId, code: u32) -> LoadError {
    LoadError::Backend(failure(backend, BackendFailureKind::InvalidModel, code))
}

const fn host_memory_failure(backend: BackendId, code: u32) -> LoadError {
    LoadError::Backend(failure(backend, BackendFailureKind::HostMemory, code))
}

fn map_candle_load_error(
    backend: BackendId,
    device: &Device,
    error: &candle_core::Error,
    code: u32,
) -> LoadError {
    let kind = if device.is_cuda() {
        candle_cuda_failure_kind(error).unwrap_or(BackendFailureKind::InvalidModel)
    } else {
        BackendFailureKind::InvalidModel
    };
    LoadError::Backend(failure(backend, kind, code))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs::File;
    use std::sync::atomic::Ordering;

    use super::{
        CalculatedFootprints, CandleLlamaPreparedLoad, InspectedShard, InspectedTensor,
        PAYLOAD_DIGEST_BYTES, SourceTensorDType, TEST_CLEANUP_SYNCHRONIZATION_FAILURES,
        calculate_footprints, select_primary_scalar, validate_memory_plan,
    };
    use candle_core::{DType, Device, Tensor};
    use candle_transformers::models::llama::Config;
    use domain_contracts::{
        BackendId, CapabilitySet, DeviceId, DeviceKind, ExecutionDevice, LoadConfiguration,
        LoadError, LoadPlan, MemoryBudget, MemoryFootprint, MemoryKind, ModelArchitecture,
        ModelCapabilities, ModelDescriptor, ModelGeneration, ModelHandle, ModelId, ModelMetadata,
        PreparedLoad, QuantizationFormat, ScalarType, ScalarTypeSet,
    };

    #[test]
    fn exact_primary_sets_accept_only_reviewed_combinations() {
        let f32_set = ScalarTypeSet::from_scalar(ScalarType::F32);
        let f16_set = ScalarTypeSet::from_scalar(ScalarType::F16);
        let bf16_set = ScalarTypeSet::from_scalar(ScalarType::Bf16);
        assert_eq!(select_primary_scalar(f32_set, None), Ok(ScalarType::F32));
        assert_eq!(
            select_primary_scalar(f16_set.union(f32_set), Some(ScalarType::F16)),
            Ok(ScalarType::F16)
        );
        assert_eq!(
            select_primary_scalar(bf16_set.union(f32_set), Some(ScalarType::Bf16)),
            Ok(ScalarType::Bf16)
        );
        assert_eq!(
            select_primary_scalar(f16_set.union(bf16_set), None),
            Err(LoadError::UnsupportedFormat)
        );
        assert_eq!(
            select_primary_scalar(f16_set, Some(ScalarType::F32)),
            Err(LoadError::UnsupportedFormat)
        );
        assert_eq!(
            select_primary_scalar(f32_set, Some(ScalarType::I8)),
            Err(LoadError::UnsupportedFormat)
        );
    }

    #[test]
    fn memory_plan_rejections_use_checked_host_and_device_totals() {
        let backend = BackendId::new(1);
        let footprint = domain_contracts::MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 200,
            host_working_bytes: 80,
            device_working_bytes: 20,
            cache_bytes_per_token: 0,
        };
        assert!(matches!(
            validate_memory_plan(
                backend,
                footprint,
                MemoryBudget {
                    host_bytes: 79,
                    device_bytes: 220,
                },
                Some(220),
            ),
            Err(LoadError::InsufficientMemory {
                kind: MemoryKind::Host,
                required_bytes: 80,
                available_bytes: 79,
            })
        ));
        assert!(matches!(
            validate_memory_plan(
                backend,
                footprint,
                MemoryBudget {
                    host_bytes: 80,
                    device_bytes: 219,
                },
                Some(220),
            ),
            Err(LoadError::InsufficientMemory {
                kind: MemoryKind::Device,
                required_bytes: 220,
                available_bytes: 219,
            })
        ));
    }

    #[test]
    fn source_dtype_width_and_alignment_are_identical_for_supported_types() {
        for source_dtype in [
            SourceTensorDType::F32,
            SourceTensorDType::F16,
            SourceTensorDType::Bf16,
        ] {
            assert_eq!(source_dtype.bytes_per_element(), source_dtype.alignment());
        }
    }

    #[test]
    fn exact_cuda_footprints_cover_transfer_conversion_and_extra_tensor_headroom()
    -> Result<(), String> {
        let backend = BackendId::new(1);
        let config = test_config();
        let same_dtype_shard = calculation_shard(vec![calculation_tensor(
            "required.f32",
            SourceTensorDType::F32,
            10,
            true,
        )?])?;
        assert_eq!(
            calculate_footprints(
                backend,
                &config,
                &[same_dtype_shard],
                DeviceKind::Cuda,
                DType::F32,
            )
            .map_err(|error| format!("calculate same-dtype CUDA footprint: {error:?}"))?,
            CalculatedFootprints {
                final_footprint: MemoryFootprint {
                    host_weight_bytes: 0,
                    device_weight_bytes: 40,
                    host_working_bytes: 0,
                    device_working_bytes: 0,
                    cache_bytes_per_token: 64,
                },
                loading_peak_footprint: MemoryFootprint {
                    host_weight_bytes: 0,
                    device_weight_bytes: 40,
                    host_working_bytes: 83,
                    device_working_bytes: 0,
                    cache_bytes_per_token: 64,
                },
            }
        );

        for (execution_dtype, primary_dtype) in [
            (DType::F16, SourceTensorDType::F16),
            (DType::BF16, SourceTensorDType::Bf16),
        ] {
            let mixed_shard = calculation_shard(vec![
                calculation_tensor("required.primary", primary_dtype, 10, true)?,
                calculation_tensor("required.f32", SourceTensorDType::F32, 4, true)?,
                calculation_tensor("extra.f32", SourceTensorDType::F32, 2, false)?,
            ])?;
            assert_eq!(
                calculate_footprints(
                    backend,
                    &config,
                    &[mixed_shard],
                    DeviceKind::Cuda,
                    execution_dtype,
                )
                .map_err(|error| format!("calculate mixed CUDA footprint: {error:?}"))?,
                CalculatedFootprints {
                    final_footprint: MemoryFootprint {
                        host_weight_bytes: 0,
                        device_weight_bytes: 28,
                        host_working_bytes: 0,
                        device_working_bytes: 0,
                        cache_bytes_per_token: 32,
                    },
                    loading_peak_footprint: MemoryFootprint {
                        host_weight_bytes: 0,
                        device_weight_bytes: 28,
                        host_working_bytes: 41,
                        device_working_bytes: 4,
                        cache_bytes_per_token: 32,
                    },
                }
            );
        }
        Ok(())
    }

    #[test]
    fn footprint_calculator_rejects_unsupported_device_kind() {
        let config = test_config();
        assert_eq!(
            calculate_footprints(
                BackendId::new(1),
                &config,
                &[],
                DeviceKind::Metal,
                DType::F32,
            ),
            Err(LoadError::InvalidConfiguration)
        );
    }

    #[test]
    fn partial_cleanup_failure_retains_every_owner_and_retry_is_idempotent() -> Result<(), String> {
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
        };
        let plan = LoadPlan {
            accepted_configuration: LoadConfiguration {
                handle: ModelHandle::new(ModelId::new(1), ModelGeneration::new(1)),
                execution_device,
                memory_budget: MemoryBudget::default(),
            },
            descriptor,
            execution_scalar_type: ScalarType::F32,
            expected_footprint: MemoryFootprint::default(),
            loading_peak_footprint: MemoryFootprint::default(),
        };
        let pending = Tensor::ones(1, DType::F32, &Device::Cpu)
            .map_err(|error| format!("create pending cleanup tensor: {error}"))?;
        let mut prepared = CandleLlamaPreparedLoad {
            backend,
            plan,
            config: Some(test_config()),
            execution_dtype: DType::F32,
            device: Some(Device::Cpu),
            shards: Vec::new(),
            final_tensors: HashMap::new(),
            pending_source_tensor: Some(pending),
            pending_host_tensor: None,
            pending_device_tensor: None,
            constructed_model: None,
            cleanup_complete: false,
        };

        TEST_CLEANUP_SYNCHRONIZATION_FAILURES.store(1, Ordering::SeqCst);
        assert!(prepared.cleanup().is_err());
        assert!(prepared.pending_source_tensor.is_some());
        assert!(prepared.device.is_some());
        assert!(!prepared.cleanup_complete);

        prepared
            .cleanup()
            .map_err(|error| format!("retry partial cleanup: {error:?}"))?;
        assert!(prepared.pending_source_tensor.is_none());
        assert!(prepared.device.is_none());
        assert!(prepared.cleanup_complete);
        prepared
            .cleanup()
            .map_err(|error| format!("repeat completed cleanup: {error:?}"))?;
        assert_eq!(
            TEST_CLEANUP_SYNCHRONIZATION_FAILURES.load(Ordering::SeqCst),
            0
        );
        Ok(())
    }

    fn calculation_shard(tensors: Vec<InspectedTensor>) -> Result<InspectedShard, String> {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let file = File::open(executable).map_err(|error| error.to_string())?;
        Ok(InspectedShard {
            file,
            file_length: 0,
            data_start: 0,
            tensors,
        })
    }

    fn calculation_tensor(
        name: &str,
        source_dtype: SourceTensorDType,
        element_count: u64,
        required: bool,
    ) -> Result<InspectedTensor, String> {
        let dimension = usize::try_from(element_count).map_err(|error| error.to_string())?;
        let source_bytes = element_count
            .checked_mul(source_dtype.bytes_per_element())
            .ok_or_else(|| "calculation tensor source bytes overflow".to_owned())?;
        Ok(InspectedTensor {
            name: name.to_owned(),
            source_dtype,
            shape: vec![dimension],
            data_start: 0,
            source_bytes,
            element_count,
            payload_digest: [0; PAYLOAD_DIGEST_BYTES],
            required,
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
