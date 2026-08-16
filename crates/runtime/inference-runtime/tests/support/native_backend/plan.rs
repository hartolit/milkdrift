use super::{
    BackendId, ByteCount, CandleLlamaLoader, CandleLlamaSource, CapabilitySet, DeviceId,
    DeviceKind, Duration, ExecutionDevice, LoadConfiguration, LoadPlan, LoadReceipt, MemoryBudget,
    MemoryFootprint, ModelArchitecture, ModelCapabilities, ModelDescriptor, ModelGeneration,
    ModelHandle, ModelId, ModelLoader, ModelMetadata, PreparedLoad, QuantizationFormat, ScalarType,
    ScalarTypeSet, TestResult, TokenId,
};

pub(crate) const CANDLE_BACKEND: BackendId = BackendId::new(41);
pub(crate) const MODEL: ModelId = ModelId::new(7);
pub(crate) const CPU_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
pub(crate) const VOCABULARY_SIZE: u32 = 16;
pub(crate) const CONTEXT_LENGTH: u32 = 16;
pub(crate) const EXPECTED_GREEDY_TOKEN: TokenId = TokenId::new(2);
pub(crate) const EVENT_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const OUTPUT_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const EXPECTED_CANDLE_OPERATIONS: CapabilitySet = CapabilitySet::PREFILL
    .union(CapabilitySet::INCREMENTAL_DECODE)
    .union(CapabilitySet::MULTIPLE_SEQUENCES)
    .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
pub(crate) const HOMOGENEOUS_F32_FINAL_FOOTPRINT: MemoryFootprint =
    MemoryFootprint::host_weights(ByteCount::from_u64(3_680));
pub(crate) const HOMOGENEOUS_F32_LOADING_PEAK_FOOTPRINT: MemoryFootprint =
    MemoryFootprint::host_weights(ByteCount::from_u64(3_680))
        .with_host_working_bytes(ByteCount::from_u64(65_763));
pub(crate) const HOMOGENEOUS_F32_DESCRIPTOR: ModelDescriptor = ModelDescriptor {
    backend: CANDLE_BACKEND,
    metadata: ModelMetadata {
        architecture: ModelArchitecture::Llama,
        configuration_declared_scalar_type: Some(ScalarType::F32),
        observed_tensor_scalar_types: ScalarTypeSet::from_scalar(ScalarType::F32),
        quantization: QuantizationFormat::None,
        vocabulary_size: VOCABULARY_SIZE,
        context_length: CONTEXT_LENGTH,
    },
    capabilities: ModelCapabilities {
        operations: EXPECTED_CANDLE_OPERATIONS,
        maximum_context_tokens: CONTEXT_LENGTH,
        maximum_sequences: u32::MAX,
        maximum_prefill_batch: CONTEXT_LENGTH,
    },
    estimated_footprint: HOMOGENEOUS_F32_FINAL_FOOTPRINT,
};
pub(crate) const MIXED_EXECUTION_WEIGHT_BYTES: u64 = 1_840;
pub(crate) const MIXED_CUDA_HOST_LOADING_PEAK_BYTES: u64 = 75_665;

pub(crate) fn prepare_plan(
    source: &CandleLlamaSource,
    execution_device: ExecutionDevice,
) -> TestResult<LoadPlan> {
    let mut loader = CandleLlamaLoader::new(CANDLE_BACKEND);
    let configuration = LoadConfiguration {
        handle: ModelHandle::new(MODEL, ModelGeneration::new(1)),
        execution_device,
        memory_budget: MemoryBudget::UNLIMITED,
    };
    let prepared = loader
        .prepare_load(source, &configuration)
        .map_err(|error| format!("prepare fixture: {error:?}"))?;
    let plan = *prepared.plan();
    drop(prepared);
    Ok(plan)
}

pub(crate) fn assert_mixed_f16_plan(
    plan: &LoadPlan,
    execution_device: ExecutionDevice,
) -> TestResult {
    assert_eq!(
        plan.accepted_configuration.execution_device,
        execution_device
    );
    assert_eq!(plan.execution_scalar_type, ScalarType::F16);
    assert_eq!(
        plan.descriptor.metadata.configuration_declared_scalar_type,
        Some(ScalarType::F16)
    );
    assert_eq!(
        plan.descriptor.metadata.observed_tensor_scalar_types,
        ScalarTypeSet::from_scalar(ScalarType::F16)
            .union(ScalarTypeSet::from_scalar(ScalarType::F32))
    );

    let expected_final = match execution_device.kind {
        DeviceKind::Cpu => {
            MemoryFootprint::host_weights(ByteCount::from_u64(MIXED_EXECUTION_WEIGHT_BYTES))
        }
        DeviceKind::Cuda => {
            MemoryFootprint::device_weights(ByteCount::from_u64(MIXED_EXECUTION_WEIGHT_BYTES))
        }
        _ => return Err("mixed fixture test selected an unsupported execution device".to_owned()),
    };
    assert_eq!(plan.final_footprint, expected_final);
    assert_eq!(
        plan.descriptor.estimated_footprint,
        MemoryFootprint::host_weights(ByteCount::from_u64(MIXED_EXECUTION_WEIGHT_BYTES))
    );

    match execution_device.kind {
        DeviceKind::Cpu => {
            assert_eq!(
                plan.loading_peak_footprint.host_weight_bytes(),
                ByteCount::from_u64(MIXED_EXECUTION_WEIGHT_BYTES)
            );
            assert_eq!(
                plan.loading_peak_footprint.device_weight_bytes(),
                ByteCount::ZERO
            );
            assert!(!plan.loading_peak_footprint.host_working_bytes().is_zero());
            assert_eq!(
                plan.loading_peak_footprint.device_working_bytes(),
                ByteCount::ZERO
            );
        }
        DeviceKind::Cuda => {
            assert_eq!(
                plan.loading_peak_footprint.host_weight_bytes(),
                ByteCount::ZERO
            );
            assert_eq!(
                plan.loading_peak_footprint.device_weight_bytes(),
                ByteCount::from_u64(MIXED_EXECUTION_WEIGHT_BYTES)
            );
            assert_eq!(
                plan.loading_peak_footprint.host_working_bytes(),
                ByteCount::from_u64(MIXED_CUDA_HOST_LOADING_PEAK_BYTES)
            );
            assert_eq!(
                plan.loading_peak_footprint.device_working_bytes(),
                ByteCount::ZERO
            );
        }
        _ => return Err("mixed fixture test selected an unsupported execution device".to_owned()),
    }
    Ok(())
}

pub(crate) fn assert_receipt_matches_plan(loaded: &LoadReceipt, plan: &LoadPlan) {
    assert_eq!(
        *loaded,
        LoadReceipt {
            handle: plan.accepted_configuration.handle,
            execution_device: plan.accepted_configuration.execution_device,
            execution_scalar_type: plan.execution_scalar_type,
            descriptor: plan.descriptor,
            reserved_footprint: plan.final_footprint,
        }
    );
}
