pub(super) use std::collections::HashMap;
pub(super) use std::fs::{self, File, OpenOptions};
pub(super) use std::io::{Seek, SeekFrom, Write};
pub(super) use std::sync::atomic::{AtomicU64, Ordering};

pub(super) use candle_core::{DType, Device, Tensor};
pub(super) use candle_transformers::models::llama::Config;
pub(super) use domain_contracts::{
    BackendFailureKind, BackendId, BackendLoadFailure, CapabilitySet, DeviceId, DeviceKind,
    ExecutionDevice, FailedLoadOwner, LoadConfiguration, LoadError, LoadFailureContext,
    LoadFailureStage, LoadPlan, MemoryBudget, MemoryFootprint, ModelArchitecture,
    ModelCapabilities, ModelDescriptor, ModelGeneration, ModelHandle, ModelId, ModelLoader,
    ModelMetadata, PreparedLoad, QuantizationFormat, ScalarType, ScalarTypeSet,
    TensorFailureLocation,
};
pub(super) use sha2::{Digest, Sha256};

pub(super) use super::CandleLlamaPreparedLoad;
pub(super) use crate::failure::{
    CODE_HEADER_IDENTITY_MISMATCH, CODE_LOAD_SYNCHRONIZE, CODE_SOURCE_IDENTITY_LENGTH,
    CODE_SOURCE_IDENTITY_MISMATCH, CODE_TENSOR_MAP_ALLOCATION, CODE_TENSOR_MATERIALIZE,
    CODE_TENSOR_TRANSFER, failure, tensor_name_fingerprint,
};
pub(super) use crate::loader::cleanup::TEST_CLEANUP_SYNCHRONIZATION_FAILURES;
pub(super) use crate::loader::identity::{
    ContentIdentityEstablishment, EstablishedContentIdentity,
};
pub(super) use crate::loader::manifest::{
    InspectedShard, InspectedTensor, SourceTensorDType, TensorShape,
};
pub(super) use crate::loader::observer::{
    HashedRange, LoadingSynchronization, MaterializationCheckpoint, MaterializationObserver,
    NoopMaterializationObserver, TEST_MATERIALIZATION_CHECKPOINT_FAILURE,
};
pub(super) use crate::loader::shard_stream::TEST_REQUIRED_PAYLOAD_READ_FAILURES;
pub(super) use crate::loader::transfer_batch::{
    TransferBatchEndpoints, TransferBatchEntry, TransferBatchOwner,
};
pub(super) use crate::loader::transfer_plan::{MAXIMUM_BATCH_ENTRIES, TransferPlan};
pub(super) use crate::source::CandleExpectedContentIdentity;

pub(crate) static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub(crate) struct Events {
    pub(crate) prefix_header_bytes: usize,
    pub(crate) ignored_bytes: usize,
    pub(crate) required_bytes: usize,
    pub(crate) source_owned_count: usize,
    pub(crate) cast_owned_count: usize,
    pub(crate) transfer_owned_count: usize,
    pub(crate) map_owned_count: usize,
    pub(crate) batch_synchronizations: usize,
    pub(crate) verified_establishments: Vec<ContentIdentityEstablishment>,
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

pub(crate) struct FailAt(pub(crate) MaterializationCheckpoint);

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

pub(crate) struct FailSynchronizationAt(pub(crate) usize);

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

pub(crate) fn required_f32_shard(count: usize) -> Result<InspectedShard, String> {
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

pub(crate) fn inspected_tensor(
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

pub(crate) fn inspected_shard(
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

pub(crate) fn populate_required_model_tensors(
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

pub(crate) fn failure_code(error: LoadError) -> Option<u32> {
    match error {
        LoadError::Backend(failure) => Some(failure.failure.code),
        _ => None,
    }
}

pub(crate) fn assert_tensor_context(
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

pub(crate) fn required_error<T>(
    result: Result<T, LoadError>,
    context: &str,
) -> Result<LoadError, String> {
    result.err().ok_or_else(|| context.to_owned())
}

pub(crate) fn first_shard_mut(
    prepared: &mut CandleLlamaPreparedLoad,
) -> Result<&mut InspectedShard, String> {
    prepared
        .shards
        .first_mut()
        .ok_or_else(|| "prepared load has no retained shard".to_owned())
}

pub(crate) fn test_prepared(
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

pub(crate) fn configure_test_device(
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

pub(crate) fn test_config() -> Config {
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
