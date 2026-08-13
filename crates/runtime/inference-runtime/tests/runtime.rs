//! Registry, admission, lifecycle, and bounded-host integration tests.

use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use domain_contracts::{
    BackendFailure, BackendFailureKind, BackendId, BackendSequence, CancellationReason,
    CapabilitySet, DecodeBufferRequirements, DecodeInput, DecodeOutcome, DeviceId, DeviceKind,
    DrainTimeout, ExecutionDevice, FailedLoad, FailedLoadOwner, FinishReason, LoadConfiguration,
    LoadError, LoadPlan, LoadedModel, MemoryBudget, MemoryFootprint, MemoryKind, ModelArchitecture,
    ModelCapabilities, ModelDescriptor, ModelError, ModelHandle, ModelId, ModelLoader,
    ModelMetadata, MonotonicMillis, PrefillBufferRequirements, PrefillInput, PrefillOutcome,
    PreparedDecodeBuffers, PreparedLoad, PreparedPrefillBuffers, QuantizationFormat, RequestId,
    ScalarType, ScalarTypeSet, SequenceConfiguration, SequenceError, SequenceId, SequencePlan,
    SequenceReservation, SequenceState, SynchronizationError, TokenId, UnloadPolicy,
};
use inference_runtime::{
    CleanupPoll, CleanupResource, CommandTicket, FailureDetail, HostedRuntime,
    HostedRuntimeConfiguration, InferenceRuntime, LoadReceipt, RetainedOwnership, RuntimeCommand,
    RuntimeError, RuntimeEvent, RuntimeLimits, RuntimeOperation, UnloadStatus,
    start_hosted_runtime,
};

const BACKEND_ID: BackendId = BackendId::new(91);

const fn cpu_device() -> ExecutionDevice {
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu)
}

#[derive(Clone, Copy)]
struct MockSource {
    model_bytes: u64,
    vocabulary_size: u32,
}

#[derive(Clone, Copy)]
struct MockLoader;

struct MockPrepared {
    source: MockSource,
    configuration: LoadConfiguration,
    plan: LoadPlan,
}

impl PreparedLoad for MockPrepared {
    type Failed = MockPrepared;

    fn plan(&self) -> &LoadPlan {
        &self.plan
    }
}

impl FailedLoadOwner for MockPrepared {
    fn plan(&self) -> &LoadPlan {
        &self.plan
    }

    fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        Ok(())
    }
}

struct MockModel {
    _thread_confined: Rc<()>,
    handle: ModelHandle,
    execution_device: ExecutionDevice,
    execution_scalar_type: ScalarType,
    descriptor: ModelDescriptor,
    reported_footprint: MemoryFootprint,
    unloading: bool,
    destroy_failure_consumed: bool,
}

struct MockSequence {
    id: SequenceId,
    state: SequenceState,
    position: usize,
    token_capacity: usize,
    plan: SequencePlan,
}

impl BackendSequence for MockSequence {
    fn id(&self) -> SequenceId {
        self.id
    }

    fn state(&self) -> SequenceState {
        self.state
    }

    fn position(&self) -> usize {
        self.position
    }

    fn token_capacity(&self) -> usize {
        self.token_capacity
    }

    fn reported_plan(&self) -> SequencePlan {
        self.plan
    }
}

impl ModelLoader for MockLoader {
    type Source = MockSource;
    type Prepared = MockPrepared;
    type FailedPreparation = MockPrepared;
    type Model = MockModel;

    fn inspect(&self, source: &Self::Source) -> Result<ModelDescriptor, LoadError> {
        Ok(descriptor(*source))
    }

    fn prepare_load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Prepared, LoadError> {
        let descriptor = self.inspect(source)?;
        let required = descriptor
            .estimated_footprint
            .checked_host_bytes()
            .ok_or(LoadError::InvalidSource)?;
        if required > configuration.memory_budget.host_bytes {
            return Err(LoadError::InsufficientMemory {
                kind: MemoryKind::Host,
                required_bytes: required,
                available_bytes: configuration.memory_budget.host_bytes,
            });
        }
        let plan = LoadPlan {
            accepted_configuration: *configuration,
            descriptor,
            execution_scalar_type: ScalarType::F32,
            final_footprint: descriptor.estimated_footprint,
            loading_peak_footprint: descriptor.estimated_footprint,
        };
        Ok(MockPrepared {
            source: *source,
            configuration: *configuration,
            plan,
        })
    }

    fn load_prepared(
        &mut self,
        prepared: Self::Prepared,
    ) -> Result<Self::Model, FailedLoad<Self::FailedPreparation>> {
        let descriptor = descriptor(prepared.source);
        Ok(MockModel {
            _thread_confined: Rc::new(()),
            handle: prepared.configuration.handle,
            execution_device: prepared.configuration.execution_device,
            execution_scalar_type: ScalarType::F32,
            descriptor,
            reported_footprint: prepared.plan.final_footprint,
            unloading: false,
            destroy_failure_consumed: false,
        })
    }
}

impl LoadedModel for MockModel {
    type Sequence = MockSequence;

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

    fn reported_footprint(&self) -> MemoryFootprint {
        self.reported_footprint
    }

    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError> {
        if self.unloading {
            return Err(ModelError::InvalidState);
        }
        Ok(SequencePlan {
            configuration: *configuration,
            reservation: SequenceReservation {
                persistent_footprint: MemoryFootprint {
                    host_weight_bytes: 0,
                    device_weight_bytes: 0,
                    host_working_bytes: u64::from(configuration.maximum_tokens.get()),
                    device_working_bytes: 0,
                },
                transient_footprint: MemoryFootprint::default(),
                total_footprint: MemoryFootprint {
                    host_weight_bytes: 0,
                    device_weight_bytes: 0,
                    host_working_bytes: u64::from(configuration.maximum_tokens.get()),
                    device_working_bytes: 0,
                },
            },
            logits_capacity: self.descriptor.metadata.vocabulary_size as usize,
        })
    }

    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        configuration: &SequenceConfiguration,
    ) -> Result<Self::Sequence, ModelError> {
        let plan = self.plan_sequence(configuration)?;
        let token_capacity = usize::try_from(configuration.maximum_tokens.get())
            .map_err(|_| ModelError::Backend(mock_failure(1)))?;
        Ok(MockSequence {
            id: sequence_id,
            state: SequenceState::Empty,
            position: 0,
            token_capacity,
            plan,
        })
    }

    fn prefill_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        input: &PrefillInput<'_>,
    ) -> PrefillBufferRequirements {
        PrefillBufferRequirements {
            logits: if input.emit_logits {
                self.descriptor.metadata.vocabulary_size as usize
            } else {
                0
            },
        }
    }

    fn decode_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: DecodeInput,
    ) -> DecodeBufferRequirements {
        DecodeBufferRequirements {
            logits: self.descriptor.metadata.vocabulary_size as usize,
        }
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "mock logits intentionally encode bounded test indices as f32"
    )]
    fn prefill_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: PrefillInput<'_>,
        mut buffers: PreparedPrefillBuffers<'_>,
    ) -> Result<PrefillOutcome, SequenceError> {
        if sequence.id == SequenceId::new(998) {
            return Err(SequenceError::Backend(mock_failure(3)));
        }
        if sequence.state == SequenceState::Finished || input.tokens.is_empty() {
            return Err(SequenceError::InvalidState);
        }
        let required = buffers.required_logits();
        let logits = buffers.logits_mut();
        for (index, logit) in logits.iter_mut().take(required).enumerate() {
            *logit = index as f32;
        }
        sequence.position = sequence.position.saturating_add(input.tokens.len());
        sequence.state = SequenceState::Ready;
        Ok(PrefillOutcome::Ready {
            consumed_tokens: input.tokens.len(),
            position: sequence.position,
            logits_written: required,
        })
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "mock logits intentionally encode bounded test tokens and indices as f32"
    )]
    fn decode_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: DecodeInput,
        mut buffers: PreparedDecodeBuffers<'_>,
    ) -> Result<DecodeOutcome, SequenceError> {
        if sequence.state != SequenceState::Ready {
            return Err(SequenceError::InvalidState);
        }
        let required = buffers.required_logits();
        let token_value = input.token.get() as f32;
        for (index, logit) in buffers.logits_mut().iter_mut().take(required).enumerate() {
            *logit = token_value + index as f32;
        }
        sequence.position = sequence.position.saturating_add(1);
        Ok(DecodeOutcome::Ready {
            position: sequence.position,
            logits_written: required,
        })
    }

    fn destroy_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        if matches!(sequence.id.get(), 998 | 999) && !self.destroy_failure_consumed {
            self.destroy_failure_consumed = true;
            return Err(SequenceError::Backend(mock_failure(2)));
        }
        sequence.state = SequenceState::Finished;
        Ok(())
    }

    fn reset_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        sequence.position = 0;
        sequence.state = SequenceState::Empty;
        Ok(())
    }

    fn synchronize(&mut self) -> Result<(), SynchronizationError> {
        Ok(())
    }

    fn prepare_unload(&mut self) -> Result<(), SynchronizationError> {
        if self.unloading {
            return Err(SynchronizationError::InvalidState);
        }
        self.unloading = true;
        Ok(())
    }
}

#[test]
fn registry_interleaves_sequences_and_reclaims_all_resources() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(4, 8, 10_000));
    let loaded = runtime
        .load_model(
            ModelId::new(1),
            &MockSource {
                model_bytes: 100,
                vocabulary_size: 8,
            },
            cpu_device(),
        )
        .map_err(debug_error)?;
    assert_eq!(loaded.execution_device, cpu_device());
    assert_eq!(loaded.execution_scalar_type, ScalarType::F32);
    assert_eq!(
        loaded.reserved_footprint,
        descriptor(MockSource {
            model_bytes: 100,
            vocabulary_size: 8,
        })
        .estimated_footprint
    );
    let loaded_snapshot = runtime
        .model_snapshot(loaded.handle)
        .ok_or_else(|| "loaded model snapshot missing".to_owned())?;
    assert_eq!(loaded_snapshot.execution_device, cpu_device());
    assert_eq!(loaded_snapshot.execution_scalar_type, ScalarType::F32);
    assert_eq!(
        loaded_snapshot.reserved_footprint,
        loaded.reserved_footprint
    );
    let configuration = sequence_configuration(16, 8)?;
    runtime
        .start_request(
            loaded.handle,
            RequestId::new(10),
            SequenceId::new(100),
            configuration,
        )
        .map_err(debug_error)?;
    runtime
        .start_request(
            loaded.handle,
            RequestId::new(11),
            SequenceId::new(101),
            configuration,
        )
        .map_err(debug_error)?;

    let mut logits_a = vec![0.0_f32; 8];
    let mut logits_b = vec![0.0_f32; 8];
    let prompt = [TokenId::new(1), TokenId::new(2)];
    runtime
        .prefill(RequestId::new(10), &prompt, true, &mut logits_a)
        .map_err(debug_error)?;
    runtime
        .prefill(RequestId::new(11), &prompt, true, &mut logits_b)
        .map_err(debug_error)?;
    runtime
        .decode(RequestId::new(10), TokenId::new(3), &mut logits_a)
        .map_err(debug_error)?;
    runtime
        .decode(RequestId::new(11), TokenId::new(4), &mut logits_b)
        .map_err(debug_error)?;

    runtime
        .cancel_request(RequestId::new(10), CancellationReason::UserRequested)
        .map_err(debug_error)?;
    runtime
        .complete_request(RequestId::new(11), FinishReason::TokenLimit)
        .map_err(debug_error)?;
    let unloaded = runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    if unloaded.status != UnloadStatus::Unloaded {
        return Err("ready model was not unloaded".into());
    }
    let snapshot = runtime.snapshot();
    if snapshot.loaded_models != 0
        || snapshot.active_requests != 0
        || snapshot.reserved_footprint != MemoryFootprint::default()
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("registry retained resources after unload".into());
    }
    Ok(())
}

#[test]
fn failed_sequence_release_preserves_request_for_retry() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 1, 10_000));
    let loaded = runtime
        .load_model(
            ModelId::new(7),
            &MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            cpu_device(),
        )
        .map_err(debug_error)?;
    runtime
        .start_request(
            loaded.handle,
            RequestId::new(70),
            SequenceId::new(999),
            sequence_configuration(8, 4)?,
        )
        .map_err(debug_error)?;

    let first = runtime.cancel_request(RequestId::new(70), CancellationReason::UserRequested);
    if !matches!(
        first,
        Err(inference_runtime::RuntimeError::CleanupFailed(state))
            if state.failure.primary_failure == inference_runtime::FailureClass::Cancellation
                && state.failure.cleanup_failure == inference_runtime::FailureClass::Sequence
    ) {
        return Err(format!("unexpected first release result: {first:?}"));
    }
    let snapshot = runtime.snapshot();
    if snapshot.active_requests != 0
        || snapshot.pending_cleanup_sequences != 1
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("failed sequence release did not quarantine ownership".into());
    }

    if !matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        inference_runtime::CleanupPoll::Released(state)
            if state.ownership == RetainedOwnership::Released && !state.exhausted()
    ) {
        return Err("pending cleanup was not retried".into());
    }
    let snapshot = runtime.snapshot();
    if snapshot.pending_cleanup_sequences != 0
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("successful release retry retained the sequence".into());
    }
    Ok(())
}

#[test]
fn direct_prefill_preserves_primary_and_exposes_retained_cleanup() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 1, 10_000));
    let loaded = runtime
        .load_model(
            ModelId::new(10),
            &MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            cpu_device(),
        )
        .map_err(debug_error)?;
    runtime
        .start_request(
            loaded.handle,
            RequestId::new(100),
            SequenceId::new(998),
            sequence_configuration(8, 4)?,
        )
        .map_err(debug_error)?;

    let mut logits = [0.0_f32; 4];
    let result = runtime.prefill(RequestId::new(100), &[TokenId::new(1)], true, &mut logits);
    if !matches!(
        result,
        Err(RuntimeError::Sequence(SequenceError::Backend(failure)))
            if failure == mock_failure(3)
    ) {
        return Err(format!(
            "primary prefill failure was not preserved: {result:?}"
        ));
    }

    let snapshot = runtime.snapshot();
    let cleanup = snapshot
        .last_cleanup
        .ok_or("failed cleanup was not exposed by the runtime snapshot")?;
    let resource = CleanupResource::Sequence {
        handle: loaded.handle,
        request_id: RequestId::new(100),
        sequence_id: SequenceId::new(998),
    };
    if snapshot.active_requests != 0
        || snapshot.pending_cleanup_sequences != 1
        || cleanup.resource != resource
        || cleanup.failure.primary_operation != RuntimeOperation::Prefill
        || cleanup.failure.primary_detail
            != FailureDetail::Sequence(SequenceError::Backend(mock_failure(3)))
        || cleanup.failure.cleanup_detail
            != FailureDetail::Sequence(SequenceError::Backend(mock_failure(2)))
        || cleanup.ownership
            != RetainedOwnership::Exact(MemoryFootprint {
                host_weight_bytes: 0,
                device_weight_bytes: 0,
                host_working_bytes: 8,
                device_working_bytes: 0,
            })
    {
        return Err("retained direct-prefill cleanup state lost structured identity".into());
    }

    if !matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Released(state)
            if state.resource == resource
                && state.ownership == RetainedOwnership::Released
                && !state.exhausted()
    ) {
        return Err("retained direct-prefill sequence was not released exactly once".into());
    }
    let model = runtime
        .model_snapshot(loaded.handle)
        .ok_or("model disappeared after sequence cleanup")?;
    if runtime.snapshot().pending_cleanup_sequences != 0 || model.degraded {
        return Err("successful cleanup did not clear retained sequence state".into());
    }
    runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    Ok(())
}

#[test]
fn drain_timeout_force_cancels_and_unloads() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 2, 10_000));
    let loaded = runtime
        .load_model(
            ModelId::new(2),
            &MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            cpu_device(),
        )
        .map_err(debug_error)?;
    runtime
        .start_request(
            loaded.handle,
            RequestId::new(20),
            SequenceId::new(200),
            sequence_configuration(8, 4)?,
        )
        .map_err(debug_error)?;
    let timeout = DrainTimeout::from_millis(10).map_err(debug_error)?;
    let receipt = runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::Drain { timeout },
            MonotonicMillis::new(100),
        )
        .map_err(debug_error)?;
    if receipt.status != UnloadStatus::Draining {
        return Err("active model did not enter draining".into());
    }
    if runtime
        .poll(MonotonicMillis::new(109))
        .map_err(debug_error)?
    {
        return Err("drain escalated before deadline".into());
    }
    if !runtime
        .poll(MonotonicMillis::new(110))
        .map_err(debug_error)?
    {
        return Err("drain did not escalate at deadline".into());
    }
    let snapshot = runtime.snapshot();
    if snapshot.loaded_models != 0
        || snapshot.active_requests != 0
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("timeout escalation did not reclaim registry state".into());
    }
    Ok(())
}

#[test]
fn undersized_logits_finish_request_without_backend_overwrite() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 1, 10_000));
    let loaded = runtime
        .load_model(
            ModelId::new(5),
            &MockSource {
                model_bytes: 100,
                vocabulary_size: 8,
            },
            cpu_device(),
        )
        .map_err(debug_error)?;
    runtime
        .start_request(
            loaded.handle,
            RequestId::new(50),
            SequenceId::new(500),
            sequence_configuration(8, 4)?,
        )
        .map_err(debug_error)?;

    let mut logits = [0.0_f32; 2];
    let receipt = runtime
        .prefill(RequestId::new(50), &[TokenId::new(1)], true, &mut logits)
        .map_err(debug_error)?;
    if !matches!(
        receipt.outcome,
        PrefillOutcome::Finished(FinishReason::BufferExhausted(_))
    ) {
        return Err("undersized logits did not finish with BufferExhausted".into());
    }
    let snapshot = runtime.snapshot();
    if snapshot.active_requests != 0
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("buffer-exhausted request remained active".into());
    }
    Ok(())
}

#[test]
fn aggregate_sequence_memory_is_admitted_before_allocation() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 1, 115));
    let loaded = runtime
        .load_model(
            ModelId::new(6),
            &MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            cpu_device(),
        )
        .map_err(debug_error)?;
    let error = runtime
        .start_request(
            loaded.handle,
            RequestId::new(60),
            SequenceId::new(600),
            sequence_configuration(8, 4)?,
        )
        .err()
        .ok_or("sequence admission unexpectedly succeeded")?;
    if !matches!(
        error,
        inference_runtime::RuntimeError::InsufficientMemory {
            kind: inference_runtime::MemoryKind::Host,
            ..
        }
    ) {
        return Err(format!("unexpected sequence admission error: {error:?}"));
    }
    let snapshot = runtime.snapshot();
    if snapshot.active_requests != 0
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("failed admission changed active-request state".into());
    }
    Ok(())
}

#[test]
fn reloading_increments_generation_and_rejects_stale_handles() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 1, 10_000));
    let source = MockSource {
        model_bytes: 100,
        vocabulary_size: 4,
    };
    let first = runtime
        .load_model(ModelId::new(3), &source, cpu_device())
        .map_err(debug_error)?;
    runtime
        .unload_model(
            first.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    let second = runtime
        .load_model(ModelId::new(3), &source, cpu_device())
        .map_err(debug_error)?;
    if second.handle.generation.get() != first.handle.generation.get() + 1 {
        return Err("model generation did not advance".into());
    }
    if runtime
        .unload_model(
            first.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .is_ok()
    {
        return Err("stale model handle was accepted".into());
    }
    Ok(())
}

#[test]
fn hosted_worker_retries_a_failed_forced_release() -> Result<(), String> {
    let hosted_configuration = HostedRuntimeConfiguration::new(
        non_zero_usize(4)?,
        non_zero_usize(4)?,
        NonZeroU64::new(1).ok_or("non-zero poll interval")?,
    );
    let (hosted, thread_handle) =
        start_hosted_runtime(MockLoader, limits(1, 1, 10_000), hosted_configuration)
            .map_err(|error| error.to_string())?;

    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(80),
            model_id: ModelId::new(8),
            source: MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            execution_device: cpu_device(),
        })
        .map_err(|_| "load command rejected")?;
    let loaded = receive_load_receipt(&hosted)?;

    hosted
        .try_submit(RuntimeCommand::StartRequest {
            ticket: CommandTicket::new(81),
            handle: loaded.handle,
            request_id: RequestId::new(80),
            sequence_id: SequenceId::new(999),
            configuration: sequence_configuration(8, 4)?,
        })
        .map_err(|_| "start command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("start event failed: {error:?}"))?
    {
        RuntimeEvent::RequestStarted { result: Ok(_), .. } => {}
        _ => return Err("unexpected request-start event".into()),
    }

    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(82),
            handle: loaded.handle,
            policy: UnloadPolicy::CancelActive,
        })
        .map_err(|_| "unload command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("initial unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Err(inference_runtime::RuntimeError::CleanupFailed(state)),
        } if state.failure.primary_failure == inference_runtime::FailureClass::Cancellation
            && state.failure.cleanup_failure == inference_runtime::FailureClass::Sequence
            && ticket == CommandTicket::new(82) => {}

        _ => return Err("missing initial sequence-release failure".into()),
    }

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("retry unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(82)
            && receipt.status == UnloadStatus::Unloaded
            && receipt.cancelled_requests == 1 => {}
        _ => return Err("failed sequence release was not retried".into()),
    }

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(83),
        })
        .map_err(|_| "shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        _ => return Err("unexpected shutdown event".into()),
    }
    thread_handle.join().map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the hosted regression keeps accepted, duplicate, completion, and terminal ticket ordering explicit"
)]
fn hosted_worker_reports_terminal_unload_after_natural_drain() -> Result<(), String> {
    let hosted_configuration = HostedRuntimeConfiguration::new(
        non_zero_usize(4)?,
        non_zero_usize(4)?,
        NonZeroU64::new(1).ok_or("non-zero poll interval")?,
    );
    let (hosted, thread_handle) =
        start_hosted_runtime(MockLoader, limits(1, 1, 10_000), hosted_configuration)
            .map_err(|error| error.to_string())?;

    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(90),
            model_id: ModelId::new(9),
            source: MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            execution_device: cpu_device(),
        })
        .map_err(|_| "load command rejected")?;
    let loaded = receive_load_receipt(&hosted)?;

    hosted
        .try_submit(RuntimeCommand::StartRequest {
            ticket: CommandTicket::new(91),
            handle: loaded.handle,
            request_id: RequestId::new(90),
            sequence_id: SequenceId::new(900),
            configuration: sequence_configuration(8, 4)?,
        })
        .map_err(|_| "start command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("start event failed: {error:?}"))?
    {
        RuntimeEvent::RequestStarted { result: Ok(_), .. } => {}
        _ => return Err("unexpected request-start event".into()),
    }

    let timeout = DrainTimeout::from_millis(5_000).map_err(debug_error)?;
    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(92),
            handle: loaded.handle,
            policy: UnloadPolicy::Drain { timeout },
        })
        .map_err(|_| "unload command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("drain event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(92) && receipt.status == UnloadStatus::Draining => {}
        _ => return Err("model did not enter draining".into()),
    }

    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(93),
            handle: loaded.handle,
            policy: UnloadPolicy::CancelActive,
        })
        .map_err(|_| "duplicate unload command rejected by the queue")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("duplicate unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Err(_),
        } if ticket == CommandTicket::new(93) => {}
        _ => return Err("duplicate unload was not rejected on its own ticket".into()),
    }

    hosted
        .try_submit(RuntimeCommand::CompleteRequest {
            ticket: CommandTicket::new(94),
            request_id: RequestId::new(90),
            reason: FinishReason::StopCondition,
        })
        .map_err(|_| "completion command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("request completion event failed: {error:?}"))?
    {
        RuntimeEvent::RequestFinished {
            ticket,
            result: Ok(FinishReason::StopCondition),
            ..
        } if ticket == CommandTicket::new(94) => {}
        _ => return Err("unexpected request completion event".into()),
    }

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("terminal natural-drain event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(92)
            && receipt.status == UnloadStatus::Unloaded
            && receipt.cancelled_requests == 0 => {}
        _ => return Err("natural drain did not emit terminal unload".into()),
    }

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(95),
        })
        .map_err(|_| "shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        _ => return Err("unexpected shutdown event".into()),
    }
    thread_handle.join().map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the bounded-queue deadline scenario keeps event backpressure and unload ordering explicit"
)]
fn hosted_worker_enforces_deadline_while_event_queue_is_full() -> Result<(), String> {
    let hosted_configuration = HostedRuntimeConfiguration::new(
        non_zero_usize(4)?,
        non_zero_usize(1)?,
        NonZeroU64::new(1).ok_or("non-zero poll interval")?,
    );
    let (hosted, thread_handle) =
        start_hosted_runtime(MockLoader, limits(1, 1, 10_000), hosted_configuration)
            .map_err(|error| error.to_string())?;

    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(1),
            model_id: ModelId::new(4),
            source: MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            execution_device: cpu_device(),
        })
        .map_err(|_| "load command rejected")?;
    let loaded = receive_load_receipt(&hosted)?;

    hosted
        .try_submit(RuntimeCommand::StartRequest {
            ticket: CommandTicket::new(2),
            handle: loaded.handle,
            request_id: RequestId::new(40),
            sequence_id: SequenceId::new(400),
            configuration: sequence_configuration(8, 4)?,
        })
        .map_err(|_| "start command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("start event failed: {error:?}"))?
    {
        RuntimeEvent::RequestStarted { result: Ok(_), .. } => {}
        _ => return Err("unexpected request-start event".into()),
    }

    let timeout = DrainTimeout::from_millis(2).map_err(debug_error)?;
    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(3),
            handle: loaded.handle,
            policy: UnloadPolicy::Drain { timeout },
        })
        .map_err(|_| "unload command rejected")?;
    thread::sleep(Duration::from_millis(20));
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            result: Ok(receipt),
            ..
        } if receipt.status == UnloadStatus::Draining => {}
        _ => return Err("unexpected unload event".into()),
    }

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("terminal unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(3)
            && receipt.status == UnloadStatus::Unloaded
            && receipt.cancelled_requests == 1 => {}
        _ => return Err("missing terminal unload event after drain timeout".into()),
    }

    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(4),
        })
        .map_err(|_| "snapshot command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            runtime,
            models,
            retained_models,
            ..
        } if runtime.loaded_models == 0
            && runtime.active_requests == 0
            && runtime.unverified_ownership.is_none()
            && !runtime.admission_blocked
            && models.is_empty()
            && retained_models.is_empty() => {}
        _ => return Err("deadline did not reclaim model under event backpressure".into()),
    }

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(5),
        })
        .map_err(|_| "shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        _ => return Err("unexpected shutdown event".into()),
    }
    thread_handle.join().map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn candle_loader_satisfies_runtime_loader_contract() {
    const fn assert_loader<L: ModelLoader>() {}
    assert_loader::<candle_backend::CandleLlamaLoader>();
}

const fn descriptor(source: MockSource) -> ModelDescriptor {
    ModelDescriptor {
        backend: BACKEND_ID,
        metadata: ModelMetadata {
            architecture: ModelArchitecture::Llama,
            configuration_declared_scalar_type: Some(ScalarType::F32),
            observed_tensor_scalar_types: ScalarTypeSet::from_scalar(ScalarType::F32),
            quantization: QuantizationFormat::None,
            vocabulary_size: source.vocabulary_size,
            context_length: 128,
        },
        capabilities: ModelCapabilities {
            operations: CapabilitySet::PREFILL
                .union(CapabilitySet::INCREMENTAL_DECODE)
                .union(CapabilitySet::MULTIPLE_SEQUENCES)
                .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION),
            maximum_context_tokens: 128,
            maximum_sequences: 4,
            maximum_prefill_batch: 128,
        },
        estimated_footprint: MemoryFootprint {
            host_weight_bytes: source.model_bytes,
            device_weight_bytes: 0,
            host_working_bytes: 10,
            device_working_bytes: 0,
        },
        sequence_cache_bytes_per_token: 1,
    }
}

fn limits(models: u32, requests: u32, host_bytes: u64) -> RuntimeLimits {
    RuntimeLimits::new(
        NonZeroU32::new(models).unwrap_or(NonZeroU32::MIN),
        NonZeroU32::new(requests).unwrap_or(NonZeroU32::MIN),
        MemoryBudget {
            host_bytes,
            device_bytes: 0,
        },
    )
}

fn sequence_configuration(
    maximum_tokens: u32,
    maximum_prefill_batch: u32,
) -> Result<SequenceConfiguration, String> {
    let tokens = NonZeroU32::new(maximum_tokens).ok_or("non-zero maximum tokens")?;
    let prefill = NonZeroU32::new(maximum_prefill_batch).ok_or("non-zero prefill batch")?;
    Ok(SequenceConfiguration::new(tokens, prefill))
}

fn non_zero_usize(value: usize) -> Result<NonZeroUsize, String> {
    NonZeroUsize::new(value).ok_or_else(|| "non-zero channel capacity".into())
}

fn receive_load_receipt(hosted: &HostedRuntime<MockSource>) -> Result<LoadReceipt, String> {
    let event = hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("load event failed: {error:?}"))?;
    let RuntimeEvent::ModelLoaded {
        result: Ok(receipt),
        ..
    } = event
    else {
        return Err("unexpected load event".into());
    };
    Ok(receipt)
}

const fn mock_failure(code: u32) -> BackendFailure {
    BackendFailure::new(BACKEND_ID, BackendFailureKind::Internal, code)
}

fn debug_error<E: core::fmt::Debug>(error: E) -> String {
    format!("{error:?}")
}
