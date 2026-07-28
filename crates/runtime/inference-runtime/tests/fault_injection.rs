//! Transaction rollback tests using a deliberately nonconforming backend.

use std::cell::Cell;
use std::num::NonZeroU32;
use std::rc::Rc;

use domain_contracts::{
    BackendFailure, BackendFailureKind, BackendId, BackendSequence, CancellationReason,
    CapabilitySet, DecodeBufferRequirements, DecodeInput, DecodeOutcome, DeviceId, DeviceKind,
    LoadConfiguration, LoadError, LoadPlan, LoadedModel, MemoryBudget, MemoryFootprint,
    ModelArchitecture, ModelCapabilities, ModelDescriptor, ModelError, ModelHandle, ModelId,
    ModelLoader, ModelMetadata, MonotonicMillis, PrefillBufferRequirements, PrefillInput,
    PrefillOutcome, PreparedDecodeBuffers, PreparedPrefillBuffers, QuantizationFormat, RequestId,
    ScalarType, SequenceConfiguration, SequenceError, SequenceId, SequencePlan, SequenceState,
    SynchronizationError, UnloadPolicy,
};
use inference_runtime::{
    CleanupPoll, CleanupResource, CleanupRetryPolicy, FailureClass, InferenceRuntime, RuntimeError,
    RuntimeLimits, RuntimeOperation,
};

const BACKEND_ID: BackendId = BackendId::new(92);

type TestResult = Result<(), String>;

#[derive(Clone, Copy, Default)]
struct Faults(u16);

impl Faults {
    const WRONG_MODEL_HANDLE: Self = Self(1 << 0);
    const MISMATCHED_METADATA: Self = Self(1 << 1);
    const FAIL_MODEL_CLEANUP: Self = Self(1 << 2);
    const CONTRADICTORY_SEQUENCE_PLAN: Self = Self(1 << 3);
    const WRONG_SEQUENCE_ID: Self = Self(1 << 4);
    const WRONG_SEQUENCE_CAPACITY: Self = Self(1 << 5);
    const FAIL_SEQUENCE_DESTRUCTION: Self = Self(1 << 6);

    const fn contains(self, fault: Self) -> bool {
        self.0 & fault.0 != 0
    }

    const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Default)]
struct CleanupCounts {
    model_loads: Cell<u32>,
    model_cleanups: Cell<u32>,
    sequence_creations: Cell<u32>,
    sequence_destructions: Cell<u32>,
}

#[derive(Clone, Copy)]
struct FaultSource;

struct FaultLoader {
    faults: Faults,
    counts: Rc<CleanupCounts>,
}

struct FaultModel {
    handle: ModelHandle,
    metadata: ModelMetadata,
    faults: Faults,
    counts: Rc<CleanupCounts>,
}

struct FaultSequence {
    id: SequenceId,
    state: SequenceState,
    token_capacity: usize,
}

impl BackendSequence for FaultSequence {
    fn id(&self) -> SequenceId {
        self.id
    }

    fn state(&self) -> SequenceState {
        self.state
    }

    fn position(&self) -> usize {
        0
    }

    fn token_capacity(&self) -> usize {
        self.token_capacity
    }
}

impl ModelLoader for FaultLoader {
    type Source = FaultSource;
    type Model = FaultModel;

    fn inspect(&self, _source: &Self::Source) -> Result<ModelDescriptor, LoadError> {
        Ok(descriptor())
    }

    fn plan_load(
        &self,
        source: &Self::Source,
        _configuration: &LoadConfiguration,
    ) -> Result<LoadPlan, LoadError> {
        let descriptor = self.inspect(source)?;
        Ok(LoadPlan {
            descriptor,
            expected_footprint: descriptor.estimated_footprint,
        })
    }

    fn load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Model, LoadError> {
        self.counts
            .model_loads
            .set(self.counts.model_loads.get().saturating_add(1));
        let descriptor = self.inspect(source)?;
        let mut metadata = descriptor.metadata;
        if self.faults.contains(Faults::MISMATCHED_METADATA) {
            metadata.vocabulary_size = metadata.vocabulary_size.saturating_add(1);
        }
        let handle = if self.faults.contains(Faults::WRONG_MODEL_HANDLE) {
            ModelHandle::new(ModelId::new(999), configuration.handle.generation)
        } else {
            configuration.handle
        };
        Ok(FaultModel {
            handle,
            metadata,
            faults: self.faults,
            counts: Rc::clone(&self.counts),
        })
    }
}

impl LoadedModel for FaultModel {
    type Sequence = FaultSequence;

    fn handle(&self) -> ModelHandle {
        self.handle
    }

    fn metadata(&self) -> &ModelMetadata {
        &self.metadata
    }

    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError> {
        let accepted = if self.faults.contains(Faults::CONTRADICTORY_SEQUENCE_PLAN) {
            SequenceConfiguration::new(NonZeroU32::MIN, configuration.maximum_prefill_batch)
        } else {
            *configuration
        };
        Ok(SequencePlan {
            configuration: accepted,
            expected_footprint: sequence_footprint(),
            logits_capacity: self.metadata.vocabulary_size as usize,
        })
    }

    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        configuration: &SequenceConfiguration,
    ) -> Result<Self::Sequence, ModelError> {
        self.counts
            .sequence_creations
            .set(self.counts.sequence_creations.get().saturating_add(1));
        let id = if self.faults.contains(Faults::WRONG_SEQUENCE_ID) {
            SequenceId::new(999)
        } else {
            sequence_id
        };
        let token_capacity = if self.faults.contains(Faults::WRONG_SEQUENCE_CAPACITY) {
            1
        } else {
            usize::try_from(configuration.maximum_tokens.get())
                .map_err(|_| ModelError::Backend(backend_failure(1)))?
        };
        Ok(FaultSequence {
            id,
            state: SequenceState::Empty,
            token_capacity,
        })
    }

    fn prefill_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: &PrefillInput<'_>,
    ) -> PrefillBufferRequirements {
        PrefillBufferRequirements { logits: 0 }
    }

    fn decode_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: DecodeInput,
    ) -> DecodeBufferRequirements {
        DecodeBufferRequirements { logits: 0 }
    }

    fn prefill_prepared(
        &mut self,
        _sequence: &mut Self::Sequence,
        _input: PrefillInput<'_>,
        _buffers: PreparedPrefillBuffers<'_>,
    ) -> Result<PrefillOutcome, SequenceError> {
        Err(SequenceError::Unsupported)
    }

    fn decode_prepared(
        &mut self,
        _sequence: &mut Self::Sequence,
        _input: DecodeInput,
        _buffers: PreparedDecodeBuffers<'_>,
    ) -> Result<DecodeOutcome, SequenceError> {
        Err(SequenceError::Unsupported)
    }

    fn destroy_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        self.counts
            .sequence_destructions
            .set(self.counts.sequence_destructions.get().saturating_add(1));
        if self.faults.contains(Faults::FAIL_SEQUENCE_DESTRUCTION) {
            return Err(SequenceError::Backend(backend_failure(2)));
        }
        sequence.state = SequenceState::Finished;
        Ok(())
    }

    fn reset_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        sequence.state = SequenceState::Empty;
        Ok(())
    }

    fn synchronize(&mut self) -> Result<(), SynchronizationError> {
        Ok(())
    }

    fn prepare_unload(&mut self) -> Result<(), SynchronizationError> {
        self.counts
            .model_cleanups
            .set(self.counts.model_cleanups.get().saturating_add(1));
        if self.faults.contains(Faults::FAIL_MODEL_CLEANUP) {
            Err(SynchronizationError::Backend(backend_failure(3)))
        } else {
            Ok(())
        }
    }
}

#[test]
fn wrong_model_handle_is_explicitly_cleaned_without_publication() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::WRONG_MODEL_HANDLE, Rc::clone(&counts));

    let result = load(&mut runtime);
    assert_eq!(result, Err(RuntimeError::BackendContractViolation));
    assert_eq!(counts.model_loads.get(), 1);
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
}

#[test]
fn mismatched_metadata_is_explicitly_cleaned_without_publication() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::MISMATCHED_METADATA, Rc::clone(&counts));

    let result = load(&mut runtime);
    assert_eq!(result, Err(RuntimeError::BackendContractViolation));
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
}

#[test]
fn model_cleanup_failure_preserves_primary_error_ownership_and_accounting() {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::WRONG_MODEL_HANDLE.union(Faults::FAIL_MODEL_CLEANUP);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    let result = load(&mut runtime);
    assert!(matches!(
        result,
        Err(RuntimeError::CleanupFailed(report))
            if report.primary_failure == inference_runtime::FailureClass::BackendContract
                && report.cleanup_failure == inference_runtime::FailureClass::Synchronization
    ));
    assert_eq!(counts.model_cleanups.get(), 1);
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.loaded_models, 0);
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.reserved_footprint, model_footprint());
}

#[test]
fn wrong_sequence_identity_is_destroyed_without_registry_mutation() -> TestResult {
    assert_sequence_contract_rollback(Faults::WRONG_SEQUENCE_ID)
}

#[test]
fn wrong_sequence_capacity_is_destroyed_without_registry_mutation() -> TestResult {
    assert_sequence_contract_rollback(Faults::WRONG_SEQUENCE_CAPACITY)
}

#[test]
fn failed_sequence_rollback_is_reported_without_registry_mutation() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::WRONG_SEQUENCE_ID.union(Faults::FAIL_SEQUENCE_DESTRUCTION);
    let mut runtime = runtime(faults, Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;

    let result = start(&mut runtime, loaded.handle, 10, 100);
    assert!(matches!(
        result,
        Err(RuntimeError::CleanupFailed(report))
            if report.primary_failure == inference_runtime::FailureClass::BackendContract
                && report.cleanup_failure == inference_runtime::FailureClass::Sequence
    ));
    assert_eq!(counts.sequence_creations.get(), 1);
    assert_eq!(counts.sequence_destructions.get(), 1);
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.active_requests, 0);
    assert_eq!(snapshot.pending_cleanup_sequences, 1);
    assert_eq!(snapshot.reserved_footprint, checked_total_footprint());
    assert!(
        runtime
            .model_snapshots()
            .first()
            .is_some_and(|model| model.degraded)
    );
    Ok(())
}

#[test]
fn contradictory_sequence_plan_is_rejected_before_native_creation() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::CONTRADICTORY_SEQUENCE_PLAN, Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;

    assert_eq!(
        start(&mut runtime, loaded.handle, 10, 100),
        Err(RuntimeError::BackendContractViolation)
    );
    assert_eq!(counts.sequence_creations.get(), 0);
    assert_eq!(counts.sequence_destructions.get(), 0);
    assert_only_model_reserved(&runtime);
    Ok(())
}

#[test]
fn occupied_request_and_sequence_indexes_fail_before_native_creation() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::default(), Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;
    start(&mut runtime, loaded.handle, 10, 100).map_err(debug_error)?;

    assert_eq!(
        start(&mut runtime, loaded.handle, 10, 101),
        Err(RuntimeError::RequestAlreadyActive(RequestId::new(10)))
    );
    assert_eq!(
        start(&mut runtime, loaded.handle, 11, 100),
        Err(RuntimeError::SequenceAlreadyActive(SequenceId::new(100)))
    );
    assert_eq!(counts.sequence_creations.get(), 1);
    assert_eq!(counts.sequence_destructions.get(), 0);
    assert_eq!(runtime.snapshot().active_requests, 1);

    runtime
        .cancel_request(RequestId::new(10), CancellationReason::UserRequested)
        .map_err(debug_error)?;
    runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    assert_eq!(counts.sequence_destructions.get(), 1);
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn repeated_sequence_cleanup_failure_exhausts_without_releasing_accounting() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime =
        runtime_with_cleanup_attempts(Faults::FAIL_SEQUENCE_DESTRUCTION, Rc::clone(&counts), 3);
    let loaded = load(&mut runtime).map_err(debug_error)?;
    start(&mut runtime, loaded.handle, 10, 100).map_err(debug_error)?;

    let initial = runtime.cancel_request(RequestId::new(10), CancellationReason::UserRequested);
    assert!(matches!(
        initial,
        Err(RuntimeError::CleanupFailed(report))
            if report.primary_failure == FailureClass::Cancellation
                && report.cleanup_failure == FailureClass::Sequence
    ));
    assert_eq!(counts.sequence_destructions.get(), 1);
    assert_eq!(
        start(&mut runtime, loaded.handle, 11, 101),
        Err(RuntimeError::ModelDegraded(loaded.handle.id))
    );
    assert_eq!(counts.sequence_creations.get(), 1);

    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::RetryFailed(state)
            if state.attempts == 2 && !state.exhausted()
    ));
    let exhausted = runtime.poll_cleanup().map_err(debug_error)?;
    assert!(matches!(
        exhausted,
        CleanupPoll::Exhausted(state)
            if state.attempts == 3
                && state.exhausted()
                && matches!(
                    state.resource,
                    CleanupResource::Sequence {
                        model_id,
                        request_id,
                        sequence_id,
                    } if model_id == loaded.handle.id
                        && request_id == RequestId::new(10)
                        && sequence_id == SequenceId::new(100)
                )
    ));
    assert_eq!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Idle
    );
    assert_eq!(counts.sequence_destructions.get(), 3);

    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.active_requests, 0);
    assert_eq!(snapshot.pending_cleanup_sequences, 1);
    assert_eq!(snapshot.exhausted_cleanup_sequences, 1);
    assert_eq!(snapshot.reserved_footprint, checked_total_footprint());
    assert!(matches!(
        runtime.shutdown(),
        Err(RuntimeError::CleanupRetryExhausted(state))
            if state.attempts == 3 && state.exhausted()
    ));
    assert_eq!(counts.sequence_destructions.get(), 3);
    Ok(())
}

#[test]
fn normal_model_unload_failure_uses_the_bounded_cleanup_state_machine() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime =
        runtime_with_cleanup_attempts(Faults::FAIL_MODEL_CLEANUP, Rc::clone(&counts), 3);
    let loaded = load(&mut runtime).map_err(debug_error)?;

    let initial = runtime.unload_model(
        loaded.handle,
        UnloadPolicy::RejectIfBusy,
        MonotonicMillis::new(0),
    );
    assert!(matches!(
        initial,
        Err(RuntimeError::CleanupFailed(report))
            if report.primary_operation == RuntimeOperation::ModelUnload
                && report.primary_failure == FailureClass::Completion
                && report.cleanup_failure == FailureClass::Synchronization
    ));
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.loaded_models, 0);
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.reserved_footprint, model_footprint());

    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::RetryFailed(state) if state.attempts == 2
    ));
    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Exhausted(state)
            if state.attempts == 3
                && matches!(
                    state.resource,
                    CleanupResource::Model { model_id } if model_id == loaded.handle.id
                )
    ));
    assert_eq!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Idle
    );
    assert_eq!(counts.model_cleanups.get(), 3);

    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.exhausted_cleanup_models, 1);
    assert!(matches!(
        runtime.unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(1),
        ),
        Err(RuntimeError::CleanupRetryExhausted(state))
            if state.attempts == 3 && state.exhausted()
    ));
    Ok(())
}

#[test]
fn shutdown_reports_model_cleanup_exhaustion_with_shutdown_as_primary() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime =
        runtime_with_cleanup_attempts(Faults::FAIL_MODEL_CLEANUP, Rc::clone(&counts), 3);
    let loaded = load(&mut runtime).map_err(debug_error)?;

    assert!(matches!(
        runtime.shutdown(),
        Err(RuntimeError::CleanupRetryExhausted(state))
            if state.attempts == 3
                && state.failure.primary_operation == RuntimeOperation::Shutdown
                && state.failure.primary_failure == FailureClass::Shutdown
                && matches!(
                    state.resource,
                    CleanupResource::Model { model_id } if model_id == loaded.handle.id
                )
    ));
    assert_eq!(counts.model_cleanups.get(), 3);
    let snapshot = runtime.snapshot();
    assert!(snapshot.shutting_down);
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.exhausted_cleanup_models, 1);
    assert_eq!(snapshot.reserved_footprint, model_footprint());
    Ok(())
}

fn assert_sequence_contract_rollback(faults: Faults) -> TestResult {
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

fn runtime(faults: Faults, counts: Rc<CleanupCounts>) -> InferenceRuntime<FaultLoader> {
    runtime_with_cleanup_attempts(faults, counts, 3)
}

fn runtime_with_cleanup_attempts(
    faults: Faults,
    counts: Rc<CleanupCounts>,
    maximum_attempts: u32,
) -> InferenceRuntime<FaultLoader> {
    let maximum_attempts = NonZeroU32::new(maximum_attempts).unwrap_or(NonZeroU32::MIN);
    InferenceRuntime::new(
        FaultLoader { faults, counts },
        RuntimeLimits::new(
            NonZeroU32::MIN,
            NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            MemoryBudget {
                host_bytes: 1_024,
                device_bytes: 0,
            },
        )
        .with_cleanup_retry_policy(CleanupRetryPolicy::new(maximum_attempts)),
    )
}

fn load(
    runtime: &mut InferenceRuntime<FaultLoader>,
) -> Result<inference_runtime::LoadReceipt, RuntimeError> {
    runtime.load_model(
        ModelId::new(1),
        &FaultSource,
        DeviceId::new(0),
        DeviceKind::Cpu,
    )
}

fn start(
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

fn assert_empty(runtime: &InferenceRuntime<FaultLoader>) {
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

fn assert_only_model_reserved(runtime: &InferenceRuntime<FaultLoader>) {
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
    assert_eq!(models.first().map(|model| model.active_requests), Some(0));
    assert_eq!(
        models.first().map(|model| model.reserved_footprint),
        Some(model_footprint())
    );
}

const fn descriptor() -> ModelDescriptor {
    ModelDescriptor {
        backend: BACKEND_ID,
        metadata: ModelMetadata {
            architecture: ModelArchitecture::Llama,
            scalar_type: ScalarType::F32,
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

const fn model_footprint() -> MemoryFootprint {
    MemoryFootprint {
        host_weight_bytes: 100,
        device_weight_bytes: 0,
        host_working_bytes: 10,
        device_working_bytes: 0,
        cache_bytes_per_token: 0,
    }
}

const fn checked_total_footprint() -> MemoryFootprint {
    MemoryFootprint {
        host_weight_bytes: 100,
        device_weight_bytes: 0,
        host_working_bytes: 18,
        device_working_bytes: 0,
        cache_bytes_per_token: 0,
    }
}

const fn sequence_footprint() -> MemoryFootprint {
    MemoryFootprint {
        host_weight_bytes: 0,
        device_weight_bytes: 0,
        host_working_bytes: 8,
        device_working_bytes: 0,
        cache_bytes_per_token: 0,
    }
}

const fn backend_failure(code: u32) -> BackendFailure {
    BackendFailure::new(BACKEND_ID, BackendFailureKind::Internal, code)
}

fn debug_error(error: impl core::fmt::Debug) -> String {
    format!("{error:?}")
}
