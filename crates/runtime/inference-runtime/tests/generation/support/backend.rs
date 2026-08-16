use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ContractFaults(u16);

impl ContractFaults {
    pub(crate) const EMPTY: Self = Self(0);
    pub(crate) const SHORT_PREFILL_LOGITS: Self = Self(1 << 0);
    pub(crate) const SHORT_DECODE_LOGITS: Self = Self(1 << 1);
    pub(crate) const INVALID_PREFILL_POSITION: Self = Self(1 << 2);
    pub(crate) const INVALID_DECODE_POSITION: Self = Self(1 << 3);
    pub(crate) const INVALID_CONSUMED_TOKENS: Self = Self(1 << 4);
    pub(crate) const PREFILL_INVALID_STATE: Self = Self(1 << 5);
    pub(crate) const PREFILL_MUTATED_IDENTITY: Self = Self(1 << 6);
    pub(crate) const PREFILL_MUTATED_CAPACITY: Self = Self(1 << 7);
    pub(crate) const DECODE_INVALID_STATE: Self = Self(1 << 8);
    pub(crate) const DECODE_MUTATED_IDENTITY: Self = Self(1 << 9);
    pub(crate) const DECODE_MUTATED_CAPACITY: Self = Self(1 << 10);

    pub(crate) const fn contains(self, fault: Self) -> bool {
        self.0 & fault.0 != 0
    }
}
pub(crate) type HostedParts = (
    HostedRuntime<FakeSource>,
    RuntimeThread,
    Arc<Mutex<Counters>>,
    ModelHandle,
);
pub(crate) type SynchronousParts = (
    InferenceRuntime<FakeLoader>,
    Arc<Mutex<Counters>>,
    ModelHandle,
);

#[derive(Clone)]
pub(crate) struct FakeSource {
    pub(crate) script: [u32; 8],
    pub(crate) script_len: usize,
    pub(crate) uniform_logits: bool,
    pub(crate) no_candidate: bool,
    pub(crate) fail_prefill: bool,
    pub(crate) fail_decode_call: Option<u32>,
    pub(crate) failed_load_cleanup_failures: Option<u32>,
    pub(crate) destroy_failures: u32,
    pub(crate) unload_failures: u32,
    pub(crate) logits_capacity: usize,
    pub(crate) operations: CapabilitySet,
    pub(crate) contract_faults: ContractFaults,
    pub(crate) load_gate: Option<Arc<BlockingGate>>,
    pub(crate) prefill_gate: Option<Arc<BlockingGate>>,
}

impl FakeSource {
    pub(crate) const fn scripted(script: [u32; 8], script_len: usize) -> Self {
        Self {
            script,
            script_len,
            uniform_logits: false,
            no_candidate: false,
            fail_prefill: false,
            fail_decode_call: None,
            failed_load_cleanup_failures: None,
            destroy_failures: 0,
            unload_failures: 0,
            logits_capacity: 4,
            operations: GENERATION_OPERATIONS,
            contract_faults: ContractFaults::EMPTY,
            load_gate: None,
            prefill_gate: None,
        }
    }
}

pub(crate) struct BlockingGate {
    pub(crate) entered: mpsc::Sender<()>,
    pub(crate) release: Mutex<mpsc::Receiver<()>>,
}

pub(crate) fn blocking_gate() -> (Arc<BlockingGate>, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    (
        Arc::new(BlockingGate {
            entered: entered_sender,
            release: Mutex::new(release_receiver),
        }),
        entered_receiver,
        release_sender,
    )
}

#[derive(Default)]
pub(crate) struct Counters {
    pub(crate) loads: u32,
    pub(crate) unload_attempts: u32,
    pub(crate) sequence_creations: u32,
    pub(crate) destruction_attempts: u32,
    pub(crate) successful_destructions: u32,
    pub(crate) prefill_calls: u32,
    pub(crate) decode_calls: u32,
    pub(crate) sampling_opportunities: u32,
    pub(crate) active_sequences: u32,
    pub(crate) retained_memory_bytes: u64,
    pub(crate) failed_load_cleanup_attempts: u32,
    pub(crate) successful_failed_load_cleanups: u32,
    pub(crate) prepared_drops: u32,
    pub(crate) retained_prepared_drops: u32,
}

#[derive(Clone)]
pub(crate) struct FakeLoader {
    pub(crate) counters: Arc<Mutex<Counters>>,
}

pub(crate) struct FakePrepared {
    pub(crate) plan: LoadPlan,
    pub(crate) source: FakeSource,
    pub(crate) configuration: LoadConfiguration,
    pub(crate) counters: Arc<Mutex<Counters>>,
    pub(crate) remaining_failed_load_cleanup_failures: u32,
    pub(crate) partial_resources_retained: bool,
}

impl PreparedLoad for FakePrepared {
    type Failed = FakePrepared;

    fn plan(&self) -> &LoadPlan {
        &self.plan
    }
}

impl FailedLoadOwner for FakePrepared {
    fn plan(&self) -> &LoadPlan {
        &self.plan
    }

    fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        let mut counters = self
            .counters
            .lock()
            .map_err(|_| SynchronizationError::Backend(failure(16)))?;
        counters.failed_load_cleanup_attempts =
            counters.failed_load_cleanup_attempts.saturating_add(1);
        if self.remaining_failed_load_cleanup_failures > 0 {
            self.remaining_failed_load_cleanup_failures = self
                .remaining_failed_load_cleanup_failures
                .saturating_sub(1);
            return Err(SynchronizationError::Backend(failure(17)));
        }
        if self.partial_resources_retained {
            self.partial_resources_retained = false;
            counters.successful_failed_load_cleanups =
                counters.successful_failed_load_cleanups.saturating_add(1);
            counters.retained_memory_bytes = counters
                .retained_memory_bytes
                .saturating_sub(MODEL_HOST_BYTES);
        }
        Ok(())
    }
}

impl Drop for FakePrepared {
    fn drop(&mut self) {
        if let Ok(mut counters) = self.counters.lock() {
            counters.prepared_drops = counters.prepared_drops.saturating_add(1);
            if self.partial_resources_retained {
                counters.retained_prepared_drops =
                    counters.retained_prepared_drops.saturating_add(1);
            }
        }
    }
}

pub(crate) struct FakeModel {
    pub(crate) handle: ModelHandle,
    pub(crate) execution_device: ExecutionDevice,
    pub(crate) execution_scalar_type: ScalarType,
    pub(crate) descriptor: ModelDescriptor,
    pub(crate) reported_footprint: MemoryFootprint,
    pub(crate) source: FakeSource,
    pub(crate) counters: Arc<Mutex<Counters>>,
    pub(crate) remaining_destroy_failures: u32,
    pub(crate) remaining_unload_failures: u32,
    pub(crate) model_released: bool,
}

pub(crate) struct FakeSequence {
    pub(crate) id: SequenceId,
    pub(crate) state: SequenceState,
    pub(crate) position: usize,
    pub(crate) capacity: usize,
    pub(crate) generated: usize,
    pub(crate) plan: SequencePlan,
}

impl BackendSequence for FakeSequence {
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
        self.capacity
    }

    fn reported_plan(&self) -> SequencePlan {
        self.plan
    }
}

impl ModelLoader for FakeLoader {
    type Source = FakeSource;
    type Prepared = FakePrepared;
    type FailedPreparation = FakePrepared;
    type Model = FakeModel;

    fn inspect(&self, source: &Self::Source) -> Result<ModelDescriptor, LoadError> {
        Ok(descriptor(source.operations))
    }

    fn prepare_load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Prepared, LoadError> {
        Ok(FakePrepared {
            plan: LoadPlan {
                accepted_configuration: *configuration,
                descriptor: self.inspect(source)?,
                execution_scalar_type: ScalarType::F32,
                final_footprint: model_footprint(),
                loading_peak_footprint: model_footprint(),
            },
            source: source.clone(),
            configuration: *configuration,
            counters: Arc::clone(&self.counters),
            remaining_failed_load_cleanup_failures: source
                .failed_load_cleanup_failures
                .unwrap_or(0),
            partial_resources_retained: false,
        })
    }

    fn load_prepared(
        &mut self,
        mut prepared: Self::Prepared,
    ) -> Result<Self::Model, FailedLoad<Self::FailedPreparation>> {
        if prepared.source.failed_load_cleanup_failures.is_some() {
            let retained_bytes = MODEL_HOST_BYTES;
            let retained = (|| {
                let mut counters = self
                    .counters
                    .lock()
                    .map_err(|_| LoadError::Backend(BackendLoadFailure::new(failure(18))))?;
                counters.loads = counters.loads.saturating_add(1);
                counters.retained_memory_bytes = counters
                    .retained_memory_bytes
                    .saturating_add(retained_bytes);
                Ok::<(), LoadError>(())
            })();
            if let Err(primary) = retained {
                return Err(FailedLoad::new(primary, prepared));
            }
            prepared.partial_resources_retained = true;
            return Err(FailedLoad::new(
                LoadError::Backend(BackendLoadFailure::new(failure(19))),
                prepared,
            ));
        }

        let load_attempt = (|| {
            if let Some(gate) = &prepared.source.load_gate {
                gate.entered
                    .send(())
                    .map_err(|_| LoadError::Backend(BackendLoadFailure::new(failure(13))))?;
                gate.release
                    .lock()
                    .map_err(|_| LoadError::Backend(BackendLoadFailure::new(failure(14))))?
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|_| LoadError::Backend(BackendLoadFailure::new(failure(15))))?;
            }
            let mut counters = self
                .counters
                .lock()
                .map_err(|_| LoadError::Backend(BackendLoadFailure::new(failure(1))))?;
            counters.loads = counters.loads.saturating_add(1);
            counters.retained_memory_bytes = counters
                .retained_memory_bytes
                .saturating_add(MODEL_HOST_BYTES);
            Ok::<(), LoadError>(())
        })();
        if let Err(primary) = load_attempt {
            return Err(FailedLoad::new(primary, prepared));
        }

        let remaining_destroy_failures = prepared.source.destroy_failures;
        let remaining_unload_failures = prepared.source.unload_failures;
        Ok(FakeModel {
            handle: prepared.configuration.handle,
            execution_device: prepared.configuration.execution_device,
            execution_scalar_type: ScalarType::F32,
            descriptor: prepared.plan.descriptor,
            reported_footprint: prepared.plan.final_footprint,
            source: prepared.source.clone(),
            counters: Arc::clone(&self.counters),
            remaining_destroy_failures,
            remaining_unload_failures,
            model_released: false,
        })
    }
}

impl LoadedModel for FakeModel {
    type Sequence = FakeSequence;

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
        Ok(SequencePlan {
            configuration: *configuration,
            reservation: SequenceReservation::checked(sequence_footprint(), MemoryFootprint::ZERO)
                .ok_or(ModelError::Unsupported)?,
            logits_capacity: self.source.logits_capacity,
        })
    }

    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        configuration: &SequenceConfiguration,
    ) -> Result<Self::Sequence, ModelError> {
        let plan = self.plan_sequence(configuration)?;
        let mut counters = self
            .counters
            .lock()
            .map_err(|_| ModelError::Backend(failure(2)))?;
        counters.sequence_creations = counters.sequence_creations.saturating_add(1);
        counters.active_sequences = counters.active_sequences.saturating_add(1);
        counters.retained_memory_bytes = counters
            .retained_memory_bytes
            .saturating_add(SEQUENCE_HOST_BYTES);
        drop(counters);
        Ok(FakeSequence {
            id: sequence_id,
            state: SequenceState::Empty,
            position: 0,
            capacity: configuration.maximum_tokens.get() as usize,
            generated: 0,
            plan,
        })
    }

    fn prefill_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: &PrefillInput<'_>,
    ) -> PrefillBufferRequirements {
        PrefillBufferRequirements { logits: 4 }
    }

    fn decode_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: DecodeInput,
    ) -> DecodeBufferRequirements {
        DecodeBufferRequirements { logits: 4 }
    }

    fn prefill_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: PrefillInput<'_>,
        mut buffers: PreparedPrefillBuffers<'_>,
    ) -> Result<PrefillOutcome, SequenceError> {
        if let Some(gate) = &self.source.prefill_gate {
            gate.entered
                .send(())
                .map_err(|_| SequenceError::Backend(failure(16)))?;
            gate.release
                .lock()
                .map_err(|_| SequenceError::Backend(failure(17)))?
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| SequenceError::Backend(failure(18)))?;
        }
        self.counters
            .lock()
            .map_err(|_| SequenceError::Backend(failure(3)))?
            .prefill_calls += 1;
        if self.source.fail_prefill {
            return Err(SequenceError::Backend(failure(4)));
        }
        let consumed_position = if self
            .source
            .contract_faults
            .contains(ContractFaults::INVALID_PREFILL_POSITION)
        {
            input.tokens.len().saturating_add(1)
        } else {
            input.tokens.len()
        };
        sequence.position = sequence.position.saturating_add(consumed_position);
        sequence.state = SequenceState::Ready;
        if self
            .source
            .contract_faults
            .contains(ContractFaults::PREFILL_MUTATED_IDENTITY)
        {
            sequence.id = SequenceId::new(9_999);
        }
        if self
            .source
            .contract_faults
            .contains(ContractFaults::PREFILL_MUTATED_CAPACITY)
        {
            sequence.capacity = sequence.capacity.saturating_add(1);
        }
        if self
            .source
            .contract_faults
            .contains(ContractFaults::PREFILL_INVALID_STATE)
        {
            sequence.state = SequenceState::Transitioning;
        }
        write_logits(&self.source, sequence.generated, buffers.logits_mut());
        self.counters
            .lock()
            .map_err(|_| SequenceError::Backend(failure(10)))?
            .sampling_opportunities += 1;
        Ok(PrefillOutcome::Ready {
            consumed_tokens: if self
                .source
                .contract_faults
                .contains(ContractFaults::INVALID_CONSUMED_TOKENS)
            {
                input.tokens.len().saturating_add(1)
            } else {
                input.tokens.len()
            },
            position: sequence.position,
            logits_written: if self
                .source
                .contract_faults
                .contains(ContractFaults::SHORT_PREFILL_LOGITS)
            {
                buffers.required_logits().saturating_sub(1)
            } else {
                buffers.required_logits()
            },
        })
    }

    fn decode_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        _input: DecodeInput,
        mut buffers: PreparedDecodeBuffers<'_>,
    ) -> Result<DecodeOutcome, SequenceError> {
        let call = {
            let mut counters = self
                .counters
                .lock()
                .map_err(|_| SequenceError::Backend(failure(5)))?;
            counters.decode_calls = counters.decode_calls.saturating_add(1);
            counters.decode_calls
        };
        if self.source.fail_decode_call == Some(call) {
            return Err(SequenceError::Backend(failure(6)));
        }
        sequence.position = sequence.position.saturating_add(
            if self
                .source
                .contract_faults
                .contains(ContractFaults::INVALID_DECODE_POSITION)
            {
                2
            } else {
                1
            },
        );
        sequence.generated = sequence.generated.saturating_add(1);
        if self
            .source
            .contract_faults
            .contains(ContractFaults::DECODE_MUTATED_IDENTITY)
        {
            sequence.id = SequenceId::new(9_999);
        }
        if self
            .source
            .contract_faults
            .contains(ContractFaults::DECODE_MUTATED_CAPACITY)
        {
            sequence.capacity = sequence.capacity.saturating_add(1);
        }
        if self
            .source
            .contract_faults
            .contains(ContractFaults::DECODE_INVALID_STATE)
        {
            sequence.state = SequenceState::Transitioning;
        }
        write_logits(&self.source, sequence.generated, buffers.logits_mut());
        self.counters
            .lock()
            .map_err(|_| SequenceError::Backend(failure(11)))?
            .sampling_opportunities += 1;
        Ok(DecodeOutcome::Ready {
            position: sequence.position,
            logits_written: if self
                .source
                .contract_faults
                .contains(ContractFaults::SHORT_DECODE_LOGITS)
            {
                buffers.required_logits().saturating_sub(1)
            } else {
                buffers.required_logits()
            },
        })
    }

    fn destroy_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        let mut counters = self
            .counters
            .lock()
            .map_err(|_| SequenceError::Backend(failure(7)))?;
        counters.destruction_attempts = counters.destruction_attempts.saturating_add(1);
        if self.remaining_destroy_failures > 0 {
            self.remaining_destroy_failures = self.remaining_destroy_failures.saturating_sub(1);
            return Err(SequenceError::Backend(failure(8)));
        }
        if sequence.state != SequenceState::Finished {
            sequence.state = SequenceState::Finished;
            counters.successful_destructions = counters.successful_destructions.saturating_add(1);
            counters.active_sequences = counters.active_sequences.saturating_sub(1);
            counters.retained_memory_bytes = counters
                .retained_memory_bytes
                .saturating_sub(SEQUENCE_HOST_BYTES);
        }
        drop(counters);
        Ok(())
    }

    fn reset_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        sequence.state = SequenceState::Empty;
        sequence.position = 0;
        sequence.generated = 0;
        Ok(())
    }

    fn synchronize(&mut self) -> Result<(), SynchronizationError> {
        Ok(())
    }

    fn prepare_unload(&mut self) -> Result<(), SynchronizationError> {
        let mut counters = self
            .counters
            .lock()
            .map_err(|_| SynchronizationError::Backend(failure(9)))?;

        counters.unload_attempts = counters.unload_attempts.saturating_add(1);

        if self.remaining_unload_failures > 0 {
            self.remaining_unload_failures = self.remaining_unload_failures.saturating_sub(1);
            drop(counters);
            return Err(SynchronizationError::Backend(failure(12)));
        }

        if !self.model_released {
            self.model_released = true;
            counters.retained_memory_bytes = counters
                .retained_memory_bytes
                .saturating_sub(MODEL_HOST_BYTES);
        }

        drop(counters);
        Ok(())
    }
}
