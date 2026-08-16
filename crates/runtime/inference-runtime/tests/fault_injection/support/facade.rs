use super::*;

pub(crate) fn assert_model_admission_mismatch_is_cleaned(faults: Faults) {
    assert_model_admission_mismatch_for_source_is_cleaned(faults, DEFAULT_SOURCE);
}

pub(crate) fn assert_model_admission_mismatch_for_source_is_cleaned(
    faults: Faults,
    source: FaultSource,
) {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(faults, Rc::clone(&counts));

    assert_eq!(
        load_source(&mut runtime, source),
        Err(RuntimeError::BackendContractViolation)
    );
    assert_eq!(counts.model_loads.get(), 1);
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
}

pub(crate) fn assert_sequence_contract_rollback(faults: Faults) -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(faults, Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;

    assert_eq!(
        start(&mut runtime, loaded.handle, 10, 100),
        Err(RuntimeError::BackendContractViolation)
    );
    assert_eq!(counts.sequence_creations.get(), 1);
    assert_eq!(counts.sequence_destructions.get(), 1);
    assert_only_model_reserved(&runtime);
    Ok(())
}

pub(crate) fn runtime(faults: Faults, counts: Rc<CleanupCounts>) -> InferenceRuntime<FaultLoader> {
    runtime_with_cleanup_attempts(faults, counts, 3)
}

pub(crate) fn runtime_with_cleanup_attempts(
    faults: Faults,
    counts: Rc<CleanupCounts>,
    maximum_attempts: u32,
) -> InferenceRuntime<FaultLoader> {
    runtime_with_limits(faults, counts, maximum_attempts, 1_024)
}

pub(crate) fn runtime_with_host_budget(
    faults: Faults,
    counts: Rc<CleanupCounts>,
    host_bytes: u64,
) -> InferenceRuntime<FaultLoader> {
    runtime_with_limits(faults, counts, 3, host_bytes)
}

pub(crate) fn runtime_with_limits(
    faults: Faults,
    counts: Rc<CleanupCounts>,
    maximum_attempts: u32,
    host_bytes: u64,
) -> InferenceRuntime<FaultLoader> {
    runtime_with_resources(
        faults,
        counts,
        maximum_attempts,
        1,
        2,
        MemoryBudget::ZERO.with_host_bytes(ByteCount::from_u64(host_bytes)),
    )
}

pub(crate) fn runtime_with_resources(
    faults: Faults,
    counts: Rc<CleanupCounts>,
    maximum_attempts: u32,
    maximum_models: u32,
    maximum_requests: u32,
    memory_budget: MemoryBudget,
) -> InferenceRuntime<FaultLoader> {
    let maximum_attempts = NonZeroU32::new(maximum_attempts).unwrap_or(NonZeroU32::MIN);
    InferenceRuntime::new(
        FaultLoader { faults, counts },
        RuntimeLimits::new(
            NonZeroU32::new(maximum_models).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(maximum_requests).unwrap_or(NonZeroU32::MIN),
            memory_budget,
        )
        .with_cleanup_retry_policy(CleanupRetryPolicy::new(maximum_attempts)),
    )
}

pub(crate) fn load(
    runtime: &mut InferenceRuntime<FaultLoader>,
) -> Result<inference_runtime::LoadReceipt, RuntimeError> {
    load_source(runtime, DEFAULT_SOURCE)
}

pub(crate) fn load_source(
    runtime: &mut InferenceRuntime<FaultLoader>,
    source: FaultSource,
) -> Result<inference_runtime::LoadReceipt, RuntimeError> {
    load_model_id(runtime, 1, source)
}

pub(crate) fn load_model_id(
    runtime: &mut InferenceRuntime<FaultLoader>,
    model_id: u64,
    source: FaultSource,
) -> Result<inference_runtime::LoadReceipt, RuntimeError> {
    runtime.load_model(
        ModelId::new(model_id),
        &source,
        ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu),
    )
}

pub(crate) fn start(
    runtime: &mut InferenceRuntime<FaultLoader>,
    handle: ModelHandle,
    request: u64,
    sequence: u64,
) -> Result<inference_runtime::RequestStartReceipt, RuntimeError> {
    runtime.start_request(
        handle,
        RequestId::new(request),
        SequenceId::new(sequence),
        SequenceConfiguration::new(
            NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
        ),
    )
}

pub(crate) fn assert_empty(runtime: &InferenceRuntime<FaultLoader>) {
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.loaded_models, 0);
    assert_eq!(snapshot.active_requests, 0);
    assert_eq!(snapshot.pending_cleanup_models, 0);
    assert_eq!(snapshot.pending_cleanup_sequences, 0);
    assert_eq!(snapshot.exhausted_cleanup_models, 0);
    assert_eq!(snapshot.exhausted_cleanup_sequences, 0);
    assert_eq!(snapshot.generation_workspaces, 0);
    assert_eq!(
        snapshot.reserved_generation_workspace,
        MemoryFootprint::default()
    );
    assert_eq!(snapshot.reserved_footprint, MemoryFootprint::default());
    assert!(runtime.model_snapshots().is_empty());
}

pub(crate) fn assert_only_model_reserved(runtime: &InferenceRuntime<FaultLoader>) {
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.loaded_models, 1);
    assert_eq!(snapshot.active_requests, 0);
    assert_eq!(snapshot.pending_cleanup_models, 0);
    assert_eq!(snapshot.pending_cleanup_sequences, 0);
    assert_eq!(snapshot.exhausted_cleanup_models, 0);
    assert_eq!(snapshot.exhausted_cleanup_sequences, 0);
    assert_eq!(snapshot.generation_workspaces, 0);
    assert_eq!(
        snapshot.reserved_generation_workspace,
        MemoryFootprint::default()
    );
    assert_eq!(snapshot.reserved_footprint, model_footprint());
    let models = runtime.model_snapshots();
    assert_eq!(models.len(), 1);
    assert_eq!(
        models.first().map(|model| model.execution_scalar_type),
        Some(ScalarType::F32)
    );
    assert_eq!(models.first().map(|model| model.active_requests), Some(0));
    assert_eq!(
        models.first().map(|model| model.reserved_footprint),
        Some(model_footprint())
    );
}

pub(crate) const fn source_with_faults(faults: Faults) -> FaultSource {
    FaultSource {
        faults,
        ..DEFAULT_SOURCE
    }
}

pub(crate) const fn descriptor(source_scalar_type: ScalarType) -> ModelDescriptor {
    ModelDescriptor {
        backend: BACKEND_ID,
        metadata: ModelMetadata {
            architecture: ModelArchitecture::Llama,
            configuration_declared_scalar_type: Some(source_scalar_type),
            observed_tensor_scalar_types: ScalarTypeSet::from_scalar(source_scalar_type),
            quantization: QuantizationFormat::None,
            vocabulary_size: 4,
            context_length: 16,
        },
        capabilities: ModelCapabilities {
            operations: CapabilitySet::PREFILL
                .union(CapabilitySet::INCREMENTAL_DECODE)
                .union(CapabilitySet::MULTIPLE_SEQUENCES)
                .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION),
            maximum_context_tokens: 16,
            maximum_sequences: 2,
            maximum_prefill_batch: 4,
        },
        estimated_footprint: model_footprint(),
    }
}

pub(crate) const fn model_footprint() -> MemoryFootprint {
    footprint(100, 0, 10, 0)
}

pub(crate) const fn loading_peak_footprint() -> MemoryFootprint {
    footprint(100, 0, 40, 0)
}

pub(crate) const fn loading_peak_host_bytes() -> u64 {
    140
}

pub(crate) const fn checked_total_footprint() -> MemoryFootprint {
    footprint(100, 0, 18, 0)
}

pub(crate) const fn sequence_footprint() -> MemoryFootprint {
    footprint(0, 0, 8, 0)
}

pub(crate) const fn sequence_persistent_footprint() -> MemoryFootprint {
    footprint(0, 0, 3, 0)
}

pub(crate) const fn sequence_transient_footprint() -> MemoryFootprint {
    footprint(0, 0, 5, 0)
}

pub(crate) fn sequence_reservation() -> SequenceReservation {
    SequenceReservation::checked(
        sequence_persistent_footprint(),
        sequence_transient_footprint(),
    )
    .unwrap_or_default()
}

pub(crate) fn sequence_report_reservation(
    host_bytes: u64,
    device_bytes: u64,
) -> SequenceReservation {
    SequenceReservation::checked(
        footprint(0, 0, host_bytes, device_bytes),
        MemoryFootprint::ZERO,
    )
    .unwrap_or_default()
}

pub(crate) const fn footprint(
    host_weights: u64,
    device_weights: u64,
    host_working: u64,
    device_working: u64,
) -> MemoryFootprint {
    MemoryFootprint::ZERO
        .with_host_weight_bytes(ByteCount::from_u64(host_weights))
        .with_device_weight_bytes(ByteCount::from_u64(device_weights))
        .with_host_working_bytes(ByteCount::from_u64(host_working))
        .with_device_working_bytes(ByteCount::from_u64(device_working))
}

pub(crate) fn complete_report_cases() -> [(Faults, MemoryFootprint, ConservativeFootprint); 8] {
    [
        (
            Faults::REPORTED_LARGER_THAN_PEAK,
            footprint(200, 0, 100, 0),
            ConservativeFootprint::Known(footprint(200, 0, 100, 0)),
        ),
        (
            Faults::REPORTED_RECLASSIFIED_TO_DEVICE,
            footprint(0, 100, 0, 10),
            ConservativeFootprint::Known(footprint(100, 100, 40, 10)),
        ),
        (
            Faults::REPORTED_OVERFLOWING_HOST,
            footprint(u64::MAX, 0, 1, 0),
            ConservativeFootprint::Overflow,
        ),
        (
            Faults::REPORTED_OVERFLOWING_DEVICE,
            footprint(100, u64::MAX, 10, 1),
            ConservativeFootprint::Overflow,
        ),
        (
            Faults::REPORTED_SMALLER_THAN_FINAL,
            footprint(50, 0, 0, 0),
            ConservativeFootprint::Known(loading_peak_footprint()),
        ),
        (
            Faults::WRONG_DEVICE_ID,
            model_footprint(),
            ConservativeFootprint::Known(loading_peak_footprint()),
        ),
        (
            Faults::WRONG_EXECUTION_SCALAR,
            model_footprint(),
            ConservativeFootprint::Known(loading_peak_footprint()),
        ),
        (
            Faults::MISMATCHED_DESCRIPTOR,
            model_footprint(),
            ConservativeFootprint::Known(loading_peak_footprint()),
        ),
    ]
}

pub(crate) const fn expected_handle(model_id: u64) -> ModelHandle {
    ModelHandle::new(ModelId::new(model_id), ModelGeneration::new(1))
}

pub(crate) fn debug_error(error: impl core::fmt::Debug) -> String {
    format!("{error:?}")
}
