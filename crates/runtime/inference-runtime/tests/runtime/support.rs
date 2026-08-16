pub(crate) use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
pub(crate) use std::rc::Rc;
pub(crate) use std::thread;
pub(crate) use std::time::{Duration, Instant};

pub(crate) use domain_contracts::{
    BackendFailure, BackendFailureKind, BackendId, BackendSequence, ByteCount, CancellationReason,
    CapabilitySet, DecodeBufferRequirements, DecodeInput, DecodeOutcome, DeviceId, DeviceKind,
    DrainTimeout, ExecutionDevice, FailedLoad, FailedLoadOwner, FinishReason, LoadConfiguration,
    LoadError, LoadPlan, LoadedModel, MemoryBudget, MemoryFootprint, MemoryKind, ModelArchitecture,
    ModelCapabilities, ModelDescriptor, ModelError, ModelHandle, ModelId, ModelLoader,
    ModelMetadata, MonotonicMillis, PrefillBufferRequirements, PrefillInput, PrefillOutcome,
    PreparedDecodeBuffers, PreparedLoad, PreparedPrefillBuffers, QuantizationFormat, RequestId,
    ScalarType, ScalarTypeSet, SequenceConfiguration, SequenceError, SequenceId, SequencePlan,
    SequenceReservation, SequenceState, SynchronizationError, TokenId, UnloadPolicy,
};
pub(crate) use inference_runtime::{
    CleanupPoll, CleanupResource, CommandTicket, FailureDetail, HostedRuntime,
    HostedRuntimeConfiguration, InferenceRuntime, LoadReceipt, RetainedOwnership, RuntimeCommand,
    RuntimeError, RuntimeEvent, RuntimeLimits, RuntimeOperation, UnloadStatus,
    start_hosted_runtime,
};

pub(crate) const BACKEND_ID: BackendId = BackendId::new(91);

pub(crate) const fn cpu_device() -> ExecutionDevice {
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu)
}

#[derive(Clone, Copy)]
pub(crate) struct MockSource {
    pub(crate) model_bytes: u64,
    pub(crate) vocabulary_size: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct MockLoader;

pub(crate) struct MockPrepared {
    source: MockSource,
    configuration: LoadConfiguration,
    pub(crate) plan: LoadPlan,
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

pub(crate) struct MockModel {
    _thread_confined: Rc<()>,
    handle: ModelHandle,
    pub(crate) execution_device: ExecutionDevice,
    execution_scalar_type: ScalarType,
    pub(crate) descriptor: ModelDescriptor,
    reported_footprint: MemoryFootprint,
    unloading: bool,
    destroy_failure_consumed: bool,
}

pub(crate) struct MockSequence {
    pub(crate) id: SequenceId,
    pub(crate) state: SequenceState,
    pub(crate) position: usize,
    token_capacity: usize,
    pub(crate) plan: SequencePlan,
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
        if !configuration.memory_budget.host_bytes().contains(required) {
            return Err(LoadError::InsufficientMemory {
                kind: MemoryKind::Host,
                required_bytes: required,
                available_bytes: configuration.memory_budget.host_bytes(),
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
            reservation: SequenceReservation::checked(
                MemoryFootprint::host_working(ByteCount::from_u64(u64::from(
                    configuration.maximum_tokens.get(),
                ))),
                MemoryFootprint::ZERO,
            )
            .ok_or_else(|| ModelError::Backend(mock_failure(1)))?,
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

pub(crate) const fn descriptor(source: MockSource) -> ModelDescriptor {
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
        estimated_footprint: MemoryFootprint::host_weights(ByteCount::from_u64(source.model_bytes))
            .with_host_working_bytes(ByteCount::from_u64(10)),
    }
}

pub(crate) fn limits(models: u32, requests: u32, host_bytes: u64) -> RuntimeLimits {
    RuntimeLimits::new(
        NonZeroU32::new(models).unwrap_or(NonZeroU32::MIN),
        NonZeroU32::new(requests).unwrap_or(NonZeroU32::MIN),
        MemoryBudget::ZERO.with_host_bytes(ByteCount::from_u64(host_bytes)),
    )
}

pub(crate) fn sequence_configuration(
    maximum_tokens: u32,
    maximum_prefill_batch: u32,
) -> Result<SequenceConfiguration, String> {
    let tokens = NonZeroU32::new(maximum_tokens).ok_or("non-zero maximum tokens")?;
    let prefill = NonZeroU32::new(maximum_prefill_batch).ok_or("non-zero prefill batch")?;
    Ok(SequenceConfiguration::new(tokens, prefill))
}

pub(crate) fn non_zero_usize(value: usize) -> Result<NonZeroUsize, String> {
    NonZeroUsize::new(value).ok_or_else(|| "non-zero channel capacity".into())
}

pub(crate) fn wait_for_queue_state(
    hosted: &HostedRuntime<MockSource>,
    queued_events: usize,
    queued_commands: usize,
) -> Result<(), String> {
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(2))
        .ok_or("queue-state deadline overflow")?;
    while hosted.queued_events() != queued_events || hosted.queued_commands() != queued_commands {
        if Instant::now() >= deadline {
            return Err("hosted queues did not reach the expected state".into());
        }
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

pub(crate) fn receive_load_receipt(
    hosted: &HostedRuntime<MockSource>,
) -> Result<LoadReceipt, String> {
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

pub(crate) const fn mock_failure(code: u32) -> BackendFailure {
    BackendFailure::new(BACKEND_ID, BackendFailureKind::Internal, code)
}

pub(crate) fn debug_error<E: core::fmt::Debug>(error: E) -> String {
    format!("{error:?}")
}
