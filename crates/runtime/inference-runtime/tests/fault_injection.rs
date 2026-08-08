//! Transaction rollback tests using a deliberately nonconforming backend.

use std::cell::Cell;
use std::num::NonZeroU32;
use std::rc::Rc;

use domain_contracts::{
    BackendFailure, BackendFailureKind, BackendId, BackendSequence, CancellationReason,
    CapabilitySet, DecodeBufferRequirements, DecodeInput, DecodeOutcome, DeviceId, DeviceKind,
    ExecutionDevice, FailedLoad, LoadConfiguration, LoadError, LoadPlan, LoadedModel, MemoryBudget,
    MemoryFootprint, MemoryKind, ModelArchitecture, ModelCapabilities, ModelDescriptor, ModelError,
    ModelHandle, ModelId, ModelLoader, ModelMetadata, MonotonicMillis, PrefillBufferRequirements,
    PrefillInput, PrefillOutcome, PreparedDecodeBuffers, PreparedLoad, PreparedPrefillBuffers,
    QuantizationFormat, RequestId, ScalarType, ScalarTypeSet, SequenceConfiguration, SequenceError,
    SequenceId, SequencePlan, SequenceState, SynchronizationError, UnloadPolicy,
};
use inference_runtime::{
    CleanupPoll, CleanupResource, CleanupRetryPolicy, FailureClass, InferenceRuntime, RuntimeError,
    RuntimeLimits, RuntimeOperation,
};

const BACKEND_ID: BackendId = BackendId::new(92);

type TestResult = Result<(), String>;

#[derive(Clone, Copy, Default)]
struct Faults(u64);

impl Faults {
    const WRONG_MODEL_HANDLE: Self = Self(1 << 0);
    const MISMATCHED_METADATA: Self = Self(1 << 1);
    const FAIL_MODEL_CLEANUP: Self = Self(1 << 2);
    const CONTRADICTORY_SEQUENCE_PLAN: Self = Self(1 << 3);
    const WRONG_SEQUENCE_ID: Self = Self(1 << 4);
    const WRONG_SEQUENCE_CAPACITY: Self = Self(1 << 5);
    const FAIL_SEQUENCE_DESTRUCTION: Self = Self(1 << 6);
    const MISMATCHED_DESCRIPTOR: Self = Self(1 << 7);
    const MISSING_MULTIPLE_SEQUENCES: Self = Self(1 << 8);
    const ZERO_VOCABULARY: Self = Self(1 << 9);
    const ZERO_CONTEXT_LENGTH: Self = Self(1 << 10);
    const ZERO_MAXIMUM_CONTEXT: Self = Self(1 << 11);
    const ZERO_MAXIMUM_SEQUENCES: Self = Self(1 << 12);
    const ZERO_MAXIMUM_PREFILL: Self = Self(1 << 13);
    const CONTEXT_EXCEEDS_METADATA: Self = Self(1 << 14);
    const PREFILL_EXCEEDS_CONTEXT: Self = Self(1 << 15);
    const WRONG_DEVICE_ID: Self = Self(1 << 16);
    const WRONG_DEVICE_KIND: Self = Self(1 << 17);
    const WRONG_MODEL_FOOTPRINT: Self = Self(1 << 18);
    const WRONG_EXECUTION_SCALAR: Self = Self(1 << 19);
    const SOURCE_SCALAR_AS_EXECUTION_SCALAR: Self = Self(1 << 20);
    const UNSUPPORTED_ACTUAL_EXECUTION_SCALAR: Self = Self(1 << 21);
    const FAIL_MODEL_CLEANUP_ONCE: Self = Self(1 << 22);
    const WRONG_ACCEPTED_CONFIGURATION: Self = Self(1 << 23);
    const EMPTY_OBSERVED_TENSOR_SET: Self = Self(1 << 24);
    const OVERFLOWING_FINAL_FOOTPRINT: Self = Self(1 << 25);
    const LOADING_PEAK_BELOW_FINAL: Self = Self(1 << 26);
    const MISMATCHED_LOADING_CACHE: Self = Self(1 << 27);
    const FAIL_LOAD: Self = Self(1 << 28);
    const FAIL_FAILED_LOAD_CLEANUP: Self = Self(1 << 29);
    const FAIL_FAILED_LOAD_CLEANUP_ONCE: Self = Self(1 << 30);
    const OVERFLOWING_LOADING_PEAK: Self = Self(1 << 31);
    const RECLASSIFIED_LOADING_PEAK: Self = Self(1 << 32);

    const fn contains(self, fault: Self) -> bool {
        self.0 & fault.0 != 0
    }

    const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Default)]
struct CleanupCounts {
    preparations: Cell<u32>,
    model_loads: Cell<u32>,
    model_cleanups: Cell<u32>,
    failed_load_cleanups: Cell<u32>,
    successful_failed_load_cleanups: Cell<u32>,
    retained_partial_load_bytes: Cell<u64>,
    sequence_creations: Cell<u32>,
    sequence_destructions: Cell<u32>,
}

#[derive(Clone, Copy)]
struct FaultSource {
    source_scalar_type: ScalarType,
    planned_execution_scalar_type: ScalarType,
}

const DEFAULT_SOURCE: FaultSource = FaultSource {
    source_scalar_type: ScalarType::F32,
    planned_execution_scalar_type: ScalarType::F32,
};
const BF16_SOURCE_WITH_F32_EXECUTION: FaultSource = FaultSource {
    source_scalar_type: ScalarType::Bf16,
    planned_execution_scalar_type: ScalarType::F32,
};

struct FaultLoader {
    faults: Faults,
    counts: Rc<CleanupCounts>,
}

struct FaultPrepared {
    plan: LoadPlan,
    source: FaultSource,
    faults: Faults,
    counts: Rc<CleanupCounts>,
    remaining_cleanup_failures: u32,
    partial_resources_retained: bool,
}

impl PreparedLoad for FaultPrepared {
    fn plan(&self) -> &LoadPlan {
        &self.plan
    }

    fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        self.counts
            .failed_load_cleanups
            .set(self.counts.failed_load_cleanups.get().saturating_add(1));
        if self.faults.contains(Faults::FAIL_FAILED_LOAD_CLEANUP)
            || self.remaining_cleanup_failures > 0
        {
            self.remaining_cleanup_failures = self.remaining_cleanup_failures.saturating_sub(1);
            return Err(SynchronizationError::Backend(backend_failure(4)));
        }
        if !self.partial_resources_retained {
            return Err(SynchronizationError::InvalidState);
        }
        self.partial_resources_retained = false;
        self.counts.successful_failed_load_cleanups.set(
            self.counts
                .successful_failed_load_cleanups
                .get()
                .saturating_add(1),
        );
        self.counts.retained_partial_load_bytes.set(
            self.counts
                .retained_partial_load_bytes
                .get()
                .saturating_sub(loading_peak_host_bytes()),
        );
        Ok(())
    }
}

struct FaultModel {
    handle: ModelHandle,
    execution_device: ExecutionDevice,
    execution_scalar_type: ScalarType,
    descriptor: ModelDescriptor,
    accounted_footprint: MemoryFootprint,
    remaining_model_cleanup_failures: u32,
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
    type Prepared = FaultPrepared;
    type Model = FaultModel;

    fn inspect(&self, source: &Self::Source) -> Result<ModelDescriptor, LoadError> {
        let mut descriptor = descriptor(source.source_scalar_type);
        if self.faults.contains(Faults::MISSING_MULTIPLE_SEQUENCES) {
            descriptor.capabilities.operations = CapabilitySet::PREFILL
                .union(CapabilitySet::INCREMENTAL_DECODE)
                .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
        }
        if self.faults.contains(Faults::ZERO_VOCABULARY) {
            descriptor.metadata.vocabulary_size = 0;
        }
        if self.faults.contains(Faults::ZERO_CONTEXT_LENGTH) {
            descriptor.metadata.context_length = 0;
        }
        if self.faults.contains(Faults::ZERO_MAXIMUM_CONTEXT) {
            descriptor.capabilities.maximum_context_tokens = 0;
        }
        if self.faults.contains(Faults::ZERO_MAXIMUM_SEQUENCES) {
            descriptor.capabilities.maximum_sequences = 0;
        }
        if self.faults.contains(Faults::ZERO_MAXIMUM_PREFILL) {
            descriptor.capabilities.maximum_prefill_batch = 0;
        }
        if self.faults.contains(Faults::CONTEXT_EXCEEDS_METADATA) {
            descriptor.capabilities.maximum_context_tokens =
                descriptor.metadata.context_length.saturating_add(1);
        }
        if self.faults.contains(Faults::PREFILL_EXCEEDS_CONTEXT) {
            descriptor.capabilities.maximum_prefill_batch = descriptor
                .capabilities
                .maximum_context_tokens
                .saturating_add(1);
        }
        if self.faults.contains(Faults::EMPTY_OBSERVED_TENSOR_SET) {
            descriptor.metadata.observed_tensor_scalar_types = ScalarTypeSet::EMPTY;
        }
        Ok(descriptor)
    }

    fn prepare_load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Prepared, LoadError> {
        self.counts
            .preparations
            .set(self.counts.preparations.get().saturating_add(1));
        let descriptor = self.inspect(source)?;
        let mut accepted_configuration = *configuration;
        if self.faults.contains(Faults::WRONG_ACCEPTED_CONFIGURATION) {
            accepted_configuration.execution_device.id = DeviceId::new(
                accepted_configuration
                    .execution_device
                    .id
                    .get()
                    .saturating_add(1),
            );
        }
        let mut expected_footprint = descriptor.estimated_footprint;
        if self.faults.contains(Faults::OVERFLOWING_FINAL_FOOTPRINT) {
            expected_footprint.host_weight_bytes = u64::MAX;
            expected_footprint.host_working_bytes = 1;
        }
        let mut loading_peak_footprint = loading_peak_footprint();
        if self.faults.contains(Faults::OVERFLOWING_LOADING_PEAK) {
            loading_peak_footprint.host_weight_bytes = u64::MAX;
            loading_peak_footprint.host_working_bytes = 1;
        }
        if self.faults.contains(Faults::LOADING_PEAK_BELOW_FINAL) {
            loading_peak_footprint.host_working_bytes = 0;
        }
        if self.faults.contains(Faults::MISMATCHED_LOADING_CACHE) {
            loading_peak_footprint.cache_bytes_per_token = loading_peak_footprint
                .cache_bytes_per_token
                .saturating_add(1);
        }
        if self.faults.contains(Faults::RECLASSIFIED_LOADING_PEAK) {
            loading_peak_footprint.host_working_bytes = loading_peak_footprint
                .host_working_bytes
                .saturating_add(loading_peak_footprint.host_weight_bytes);
            loading_peak_footprint.host_weight_bytes = 0;
        }
        Ok(FaultPrepared {
            plan: LoadPlan {
                accepted_configuration,
                descriptor,
                execution_scalar_type: source.planned_execution_scalar_type,
                expected_footprint,
                loading_peak_footprint,
            },
            source: *source,
            faults: self.faults,
            counts: Rc::clone(&self.counts),
            remaining_cleanup_failures: u32::from(
                self.faults.contains(Faults::FAIL_FAILED_LOAD_CLEANUP_ONCE),
            ),
            partial_resources_retained: false,
        })
    }

    fn load_prepared(
        &mut self,
        mut prepared: Self::Prepared,
    ) -> Result<Self::Model, FailedLoad<Self::Prepared>> {
        self.counts
            .model_loads
            .set(self.counts.model_loads.get().saturating_add(1));
        if self.faults.contains(Faults::FAIL_LOAD) {
            prepared.partial_resources_retained = true;
            self.counts.retained_partial_load_bytes.set(
                self.counts
                    .retained_partial_load_bytes
                    .get()
                    .saturating_add(loading_peak_host_bytes()),
            );
            return Err(FailedLoad::new(
                LoadError::Backend(backend_failure(5)),
                prepared,
            ));
        }

        let source = prepared.source;
        let configuration = prepared.plan.accepted_configuration;
        let mut descriptor = prepared.plan.descriptor;
        if self.faults.contains(Faults::MISMATCHED_METADATA) {
            descriptor.metadata.vocabulary_size =
                descriptor.metadata.vocabulary_size.saturating_add(1);
        }
        if self.faults.contains(Faults::MISMATCHED_DESCRIPTOR) {
            descriptor.capabilities.maximum_prefill_batch = descriptor
                .capabilities
                .maximum_prefill_batch
                .saturating_add(1);
        }
        let handle = if self.faults.contains(Faults::WRONG_MODEL_HANDLE) {
            ModelHandle::new(ModelId::new(999), configuration.handle.generation)
        } else {
            configuration.handle
        };
        let mut execution_device = configuration.execution_device;
        if self.faults.contains(Faults::WRONG_DEVICE_ID) {
            execution_device.id = DeviceId::new(execution_device.id.get().saturating_add(1));
        }
        if self.faults.contains(Faults::WRONG_DEVICE_KIND) {
            execution_device.kind = DeviceKind::Cuda;
        }
        let execution_scalar_type = if self
            .faults
            .contains(Faults::SOURCE_SCALAR_AS_EXECUTION_SCALAR)
        {
            descriptor
                .metadata
                .configuration_declared_scalar_type
                .unwrap_or(source.source_scalar_type)
        } else if self
            .faults
            .contains(Faults::UNSUPPORTED_ACTUAL_EXECUTION_SCALAR)
        {
            ScalarType::Other(u16::MAX)
        } else if self.faults.contains(Faults::WRONG_EXECUTION_SCALAR) {
            ScalarType::F16
        } else {
            source.planned_execution_scalar_type
        };
        let mut accounted_footprint = prepared.plan.expected_footprint;
        if self.faults.contains(Faults::WRONG_MODEL_FOOTPRINT) {
            accounted_footprint.host_working_bytes =
                accounted_footprint.host_working_bytes.saturating_add(1);
        }
        Ok(FaultModel {
            handle,
            execution_device,
            execution_scalar_type,
            descriptor,
            accounted_footprint,
            remaining_model_cleanup_failures: u32::from(
                self.faults.contains(Faults::FAIL_MODEL_CLEANUP_ONCE),
            ),
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

    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn execution_device(&self) -> ExecutionDevice {
        self.execution_device
    }

    fn execution_scalar_type(&self) -> ScalarType {
        self.execution_scalar_type
    }

    fn accounted_footprint(&self) -> MemoryFootprint {
        self.accounted_footprint
    }

    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError> {
        let accepted = if self.faults.contains(Faults::CONTRADICTORY_SEQUENCE_PLAN) {
            SequenceConfiguration::new(
                NonZeroU32::new(17).unwrap_or(NonZeroU32::MIN),
                configuration.maximum_prefill_batch,
            )
        } else {
            *configuration
        };
        Ok(SequencePlan {
            configuration: accepted,
            expected_footprint: sequence_footprint(),
            logits_capacity: self.descriptor.metadata.vocabulary_size as usize,
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
        if self.faults.contains(Faults::FAIL_MODEL_CLEANUP)
            || self.remaining_model_cleanup_failures > 0
        {
            self.remaining_model_cleanup_failures =
                self.remaining_model_cleanup_failures.saturating_sub(1);
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
fn wrong_device_id_after_native_load_is_cleaned_without_publication() {
    assert_model_admission_mismatch_is_cleaned(Faults::WRONG_DEVICE_ID);
}

#[test]
fn wrong_device_kind_after_native_load_is_cleaned_without_publication() {
    assert_model_admission_mismatch_is_cleaned(Faults::WRONG_DEVICE_KIND);
}

#[test]
fn correct_execution_scalar_wrong_accounted_footprint_is_cleaned_without_publication() {
    assert_model_admission_mismatch_is_cleaned(Faults::WRONG_MODEL_FOOTPRINT);
}

#[test]
fn correct_device_wrong_execution_scalar_is_cleaned_without_publication() {
    assert_model_admission_mismatch_is_cleaned(Faults::WRONG_EXECUTION_SCALAR);
}

#[test]
fn source_scalar_mistaken_for_execution_scalar_is_cleaned_without_publication() {
    assert_model_admission_mismatch_for_source_is_cleaned(
        Faults::SOURCE_SCALAR_AS_EXECUTION_SCALAR,
        BF16_SOURCE_WITH_F32_EXECUTION,
    );
}

#[test]
fn unsupported_actual_execution_scalar_is_cleaned_without_publication() {
    assert_model_admission_mismatch_is_cleaned(Faults::UNSUPPORTED_ACTUAL_EXECUTION_SCALAR);
}

#[test]
fn planned_execution_scalar_is_published_independently_from_source_scalar() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::default(), Rc::clone(&counts));

    let loaded = load_source(&mut runtime, BF16_SOURCE_WITH_F32_EXECUTION).map_err(debug_error)?;
    assert_eq!(
        loaded
            .descriptor
            .metadata
            .configuration_declared_scalar_type,
        Some(ScalarType::Bf16)
    );
    assert_eq!(loaded.execution_scalar_type, ScalarType::F32);
    assert_eq!(
        loaded.execution_device,
        ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu)
    );
    assert_eq!(loaded.reserved_footprint, model_footprint());
    let snapshot = runtime
        .model_snapshot(loaded.handle)
        .ok_or_else(|| "loaded model snapshot missing".to_owned())?;
    assert_eq!(
        snapshot
            .descriptor
            .metadata
            .configuration_declared_scalar_type,
        Some(ScalarType::Bf16)
    );
    assert_eq!(snapshot.execution_scalar_type, ScalarType::F32);
    assert_eq!(snapshot.reserved_footprint, model_footprint());

    runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn wrong_execution_scalar_cleanup_failure_retains_accounting_until_successful_retry() -> TestResult
{
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::WRONG_EXECUTION_SCALAR.union(Faults::FAIL_MODEL_CLEANUP_ONCE);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(report))
            if report.primary_operation == RuntimeOperation::ModelAdmission
                && report.primary_failure == FailureClass::BackendContract
                && report.cleanup_operation == RuntimeOperation::ModelUnload
                && report.cleanup_failure == FailureClass::Synchronization
    ));
    assert_eq!(counts.model_loads.get(), 1);
    assert_eq!(counts.model_cleanups.get(), 1);
    let retained = runtime.snapshot();
    assert_eq!(retained.loaded_models, 0);
    assert_eq!(retained.pending_cleanup_models, 1);
    assert_eq!(retained.reserved_footprint, loading_peak_footprint());
    assert!(runtime.model_snapshots().is_empty());
    assert!(matches!(
        runtime.model_cleanup_state(ModelId::new(1)),
        Some(state) if state.attempts == 1 && !state.exhausted()
    ));

    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Released(state) if state.attempts == 2 && !state.exhausted()
    ));
    assert_eq!(counts.model_cleanups.get(), 2);
    assert_empty(&runtime);
    Ok(())
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
fn mismatched_loaded_descriptor_is_explicitly_cleaned_without_publication() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::MISMATCHED_DESCRIPTOR, Rc::clone(&counts));

    let result = load(&mut runtime);
    assert_eq!(result, Err(RuntimeError::BackendContractViolation));
    assert_eq!(counts.model_loads.get(), 1);
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
}

#[test]
fn multiple_sequences_requires_the_matching_capability() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::MISSING_MULTIPLE_SEQUENCES, Rc::clone(&counts));

    let result = load(&mut runtime);
    assert_eq!(result, Err(RuntimeError::BackendContractViolation));
    assert_eq!(counts.model_loads.get(), 0);
    assert_eq!(counts.model_cleanups.get(), 0);
    assert_empty(&runtime);
}

#[test]
fn descriptor_numeric_fields_must_be_nonzero_and_consistent() {
    for fault in [
        Faults::ZERO_VOCABULARY,
        Faults::ZERO_CONTEXT_LENGTH,
        Faults::ZERO_MAXIMUM_CONTEXT,
        Faults::ZERO_MAXIMUM_SEQUENCES,
        Faults::ZERO_MAXIMUM_PREFILL,
        Faults::CONTEXT_EXCEEDS_METADATA,
        Faults::PREFILL_EXCEEDS_CONTEXT,
    ] {
        let counts = Rc::new(CleanupCounts::default());
        let mut runtime = runtime(fault, Rc::clone(&counts));

        assert_eq!(
            load(&mut runtime),
            Err(RuntimeError::BackendContractViolation)
        );
        assert_eq!(counts.model_loads.get(), 0);
        assert_eq!(counts.model_cleanups.get(), 0);
        assert_empty(&runtime);
    }
}

#[test]
fn device_mismatch_cleanup_failure_preserves_primary_error_ownership_and_accounting() {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::WRONG_DEVICE_ID.union(Faults::FAIL_MODEL_CLEANUP);
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
    assert_eq!(snapshot.reserved_footprint, loading_peak_footprint());
}

#[test]
fn exact_preparation_is_consumed_once_without_replanning() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::default(), Rc::clone(&counts));

    let loaded = load(&mut runtime).map_err(debug_error)?;
    assert_eq!(counts.preparations.get(), 1);
    assert_eq!(counts.model_loads.get(), 1);
    assert_eq!(counts.failed_load_cleanups.get(), 0);
    assert_eq!(runtime.snapshot().reserved_footprint, model_footprint());

    runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn invalid_prepared_plans_are_rejected_before_materialization() {
    for fault in [
        Faults::WRONG_ACCEPTED_CONFIGURATION,
        Faults::EMPTY_OBSERVED_TENSOR_SET,
        Faults::OVERFLOWING_FINAL_FOOTPRINT,
        Faults::OVERFLOWING_LOADING_PEAK,
        Faults::LOADING_PEAK_BELOW_FINAL,
        Faults::MISMATCHED_LOADING_CACHE,
        Faults::RECLASSIFIED_LOADING_PEAK,
    ] {
        let counts = Rc::new(CleanupCounts::default());
        let mut runtime = runtime(fault, Rc::clone(&counts));

        assert_eq!(
            load(&mut runtime),
            Err(RuntimeError::BackendContractViolation)
        );
        assert_eq!(counts.preparations.get(), 1);
        assert_eq!(counts.model_loads.get(), 0);
        assert_eq!(counts.failed_load_cleanups.get(), 0);
        assert_eq!(counts.model_cleanups.get(), 0);
        assert_empty(&runtime);
    }
}

#[test]
fn aggregate_loading_peak_budget_rejection_precedes_materialization() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime_with_host_budget(
        Faults::default(),
        Rc::clone(&counts),
        loading_peak_host_bytes() - 1,
    );

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Host,
            required_bytes,
            available_bytes,
        }) if required_bytes == loading_peak_host_bytes()
            && available_bytes == loading_peak_host_bytes() - 1
    ));
    assert_eq!(counts.preparations.get(), 1);
    assert_eq!(counts.model_loads.get(), 0);
    assert_eq!(counts.failed_load_cleanups.get(), 0);
    assert_eq!(counts.model_cleanups.get(), 0);
    assert_empty(&runtime);
}

#[test]
fn failed_load_immediate_cleanup_returns_exact_primary_and_restores_accounting() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::FAIL_LOAD, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::Load(LoadError::Backend(failure))) if failure.code == 5
    ));
    assert_eq!(counts.preparations.get(), 1);
    assert_eq!(counts.model_loads.get(), 1);
    assert_eq!(counts.failed_load_cleanups.get(), 1);
    assert_eq!(counts.successful_failed_load_cleanups.get(), 1);
    assert_eq!(counts.retained_partial_load_bytes.get(), 0);
    assert_empty(&runtime);
}

#[test]
fn failed_load_cleanup_failure_retains_owner_and_full_loading_peak() {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::FAIL_LOAD.union(Faults::FAIL_FAILED_LOAD_CLEANUP_ONCE);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(report))
            if report.primary_operation == RuntimeOperation::ModelLoad
                && report.primary_failure == FailureClass::Load
                && report.cleanup_operation == RuntimeOperation::FailedLoadCleanup
                && report.cleanup_failure == FailureClass::Synchronization
    ));
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.loaded_models, 0);
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.reserved_footprint, loading_peak_footprint());
    assert!(runtime.model_snapshots().is_empty());
    assert!(matches!(
        runtime.model_cleanup_state(ModelId::new(1)),
        Some(state)
            if state.attempts == 1
                && !state.exhausted()
                && state.resource
                    == (CleanupResource::FailedLoad {
                        model_id: ModelId::new(1),
                    })
    ));
    assert_eq!(counts.failed_load_cleanups.get(), 1);
    assert_eq!(counts.successful_failed_load_cleanups.get(), 0);
    assert_eq!(
        counts.retained_partial_load_bytes.get(),
        loading_peak_host_bytes()
    );
    assert_eq!(
        load(&mut runtime),
        Err(RuntimeError::ModelAlreadyLoaded(ModelId::new(1)))
    );
}

#[test]
fn failed_load_cleanup_retry_releases_owner_and_accounting_once() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::FAIL_LOAD.union(Faults::FAIL_FAILED_LOAD_CLEANUP_ONCE);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(_))
    ));
    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Released(state)
            if state.attempts == 2
                && state.resource
                    == (CleanupResource::FailedLoad {
                        model_id: ModelId::new(1),
                    })
    ));
    assert_eq!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Idle
    );
    assert_eq!(counts.failed_load_cleanups.get(), 2);
    assert_eq!(counts.successful_failed_load_cleanups.get(), 1);
    assert_eq!(counts.retained_partial_load_bytes.get(), 0);
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn shutdown_releases_retryable_failed_load_without_counting_an_unloaded_model() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::FAIL_LOAD.union(Faults::FAIL_FAILED_LOAD_CLEANUP_ONCE);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(_))
    ));
    let receipt = runtime.shutdown().map_err(debug_error)?;
    assert_eq!(receipt.unloaded_models, 0);
    assert_eq!(receipt.cancelled_requests, 0);
    assert_eq!(counts.failed_load_cleanups.get(), 2);
    assert_eq!(counts.successful_failed_load_cleanups.get(), 1);
    assert_eq!(counts.retained_partial_load_bytes.get(), 0);
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn failed_load_cleanup_exhaustion_survives_shutdown_accounted_and_owned() {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::FAIL_LOAD.union(Faults::FAIL_FAILED_LOAD_CLEANUP);
    let mut runtime = runtime_with_cleanup_attempts(faults, Rc::clone(&counts), 3);

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(_))
    ));
    assert!(matches!(
        runtime.shutdown(),
        Err(RuntimeError::CleanupRetryExhausted(state))
            if state.attempts == 3
                && state.exhausted()
                && state.resource
                    == (CleanupResource::FailedLoad {
                        model_id: ModelId::new(1),
                    })
                && state.failure.primary_operation == RuntimeOperation::ModelLoad
                && state.failure.primary_failure == FailureClass::Load
                && state.failure.cleanup_operation == RuntimeOperation::FailedLoadCleanup
    ));
    assert_eq!(counts.failed_load_cleanups.get(), 3);
    assert_eq!(counts.successful_failed_load_cleanups.get(), 0);
    assert_eq!(
        counts.retained_partial_load_bytes.get(),
        loading_peak_host_bytes()
    );
    let snapshot = runtime.snapshot();
    assert!(snapshot.shutting_down);
    assert_eq!(snapshot.loaded_models, 0);
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.exhausted_cleanup_models, 1);
    assert_eq!(snapshot.reserved_footprint, loading_peak_footprint());
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
fn over_advertised_sequence_plan_is_rejected_before_native_creation() -> TestResult {
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
fn direct_sequence_configuration_respects_advertised_limits() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::default(), Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;
    let configurations = [
        SequenceConfiguration::new(
            NonZeroU32::new(17).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
        ),
        SequenceConfiguration::new(
            NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(5).unwrap_or(NonZeroU32::MIN),
        ),
        SequenceConfiguration::new(
            NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
        ),
    ];

    for (offset, configuration) in configurations.into_iter().enumerate() {
        let offset = u64::try_from(offset).map_err(debug_error)?;
        assert_eq!(
            runtime.start_request(
                loaded.handle,
                RequestId::new(20_u64.saturating_add(offset)),
                SequenceId::new(200_u64.saturating_add(offset)),
                configuration,
            ),
            Err(RuntimeError::Model(ModelError::Unsupported))
        );
    }
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
fn ordinary_unload_releases_all_runtime_ownership_and_accounting() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::default(), Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;
    start(&mut runtime, loaded.handle, 10, 100).map_err(debug_error)?;

    let receipt = runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::CancelActive,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    assert_eq!(receipt.cancelled_requests, 1);
    assert_eq!(counts.sequence_destructions.get(), 1);
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
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

fn assert_model_admission_mismatch_is_cleaned(faults: Faults) {
    assert_model_admission_mismatch_for_source_is_cleaned(faults, DEFAULT_SOURCE);
}

fn assert_model_admission_mismatch_for_source_is_cleaned(faults: Faults, source: FaultSource) {
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
    runtime_with_limits(faults, counts, maximum_attempts, 1_024)
}

fn runtime_with_host_budget(
    faults: Faults,
    counts: Rc<CleanupCounts>,
    host_bytes: u64,
) -> InferenceRuntime<FaultLoader> {
    runtime_with_limits(faults, counts, 3, host_bytes)
}

fn runtime_with_limits(
    faults: Faults,
    counts: Rc<CleanupCounts>,
    maximum_attempts: u32,
    host_bytes: u64,
) -> InferenceRuntime<FaultLoader> {
    let maximum_attempts = NonZeroU32::new(maximum_attempts).unwrap_or(NonZeroU32::MIN);
    InferenceRuntime::new(
        FaultLoader { faults, counts },
        RuntimeLimits::new(
            NonZeroU32::MIN,
            NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            MemoryBudget {
                host_bytes,
                device_bytes: 0,
            },
        )
        .with_cleanup_retry_policy(CleanupRetryPolicy::new(maximum_attempts)),
    )
}

fn load(
    runtime: &mut InferenceRuntime<FaultLoader>,
) -> Result<inference_runtime::LoadReceipt, RuntimeError> {
    load_source(runtime, DEFAULT_SOURCE)
}

fn load_source(
    runtime: &mut InferenceRuntime<FaultLoader>,
    source: FaultSource,
) -> Result<inference_runtime::LoadReceipt, RuntimeError> {
    runtime.load_model(
        ModelId::new(1),
        &source,
        ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu),
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

const fn descriptor(source_scalar_type: ScalarType) -> ModelDescriptor {
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

const fn model_footprint() -> MemoryFootprint {
    MemoryFootprint {
        host_weight_bytes: 100,
        device_weight_bytes: 0,
        host_working_bytes: 10,
        device_working_bytes: 0,
        cache_bytes_per_token: 0,
    }
}

const fn loading_peak_footprint() -> MemoryFootprint {
    MemoryFootprint {
        host_weight_bytes: 100,
        device_weight_bytes: 0,
        host_working_bytes: 40,
        device_working_bytes: 0,
        cache_bytes_per_token: 0,
    }
}

const fn loading_peak_host_bytes() -> u64 {
    140
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
