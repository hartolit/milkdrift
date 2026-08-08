//! Integration tests for backend contracts, lifecycle transitions, and capacity guards.

use core::num::NonZeroU32;

use domain_contracts::{
    BackendSequence, CancellationReason, CancellationStatus, CapacityResource,
    DecodeBufferRequirements, DecodeBuffers, DecodeInput, DecodeOutcome, DrainTimeout, FailedLoad,
    FinishReason, LifecycleAction, LoadConfiguration, LoadError, LoadPlan, MemoryBudget,
    MemoryFootprint, ModelLifecycle, ModelLifecycleState, MonotonicMillis,
    PrefillBufferRequirements, PrefillBuffers, PrefillInput, PrefillOutcome, PreparedDecodeBuffers,
    PreparedLoad, PreparedPrefillBuffers, ScalarType, ScalarTypeSet, SequenceId, SequenceState,
    SynchronizationError, TokenId, UnloadPolicy, decode_checked, prefill_checked,
};

struct TestSequence {
    id: SequenceId,
    position: usize,
    capacity: usize,
    state: SequenceState,
}

impl BackendSequence for TestSequence {
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
}

struct TestModel {
    vocabulary: usize,
}

impl domain_contracts::LoadedModel for TestModel {
    type Sequence = TestSequence;

    fn handle(&self) -> domain_contracts::ModelHandle {
        domain_contracts::ModelHandle::new(
            domain_contracts::ModelId::new(1),
            domain_contracts::ModelGeneration::new(1),
        )
    }

    fn descriptor(&self) -> &domain_contracts::ModelDescriptor {
        static DESCRIPTOR: domain_contracts::ModelDescriptor = domain_contracts::ModelDescriptor {
            backend: domain_contracts::BackendId::new(1),
            metadata: domain_contracts::ModelMetadata {
                architecture: domain_contracts::ModelArchitecture::Llama,
                configuration_declared_scalar_type: Some(domain_contracts::ScalarType::F32),
                observed_tensor_scalar_types: domain_contracts::ScalarTypeSet::from_scalar(
                    domain_contracts::ScalarType::F32,
                ),
                quantization: domain_contracts::QuantizationFormat::None,
                vocabulary_size: 16,
                context_length: 8,
            },
            capabilities: domain_contracts::ModelCapabilities {
                operations: domain_contracts::CapabilitySet::PREFILL
                    .union(domain_contracts::CapabilitySet::INCREMENTAL_DECODE),
                maximum_context_tokens: 8,
                maximum_sequences: 1,
                maximum_prefill_batch: 8,
            },
            estimated_footprint: domain_contracts::MemoryFootprint {
                host_weight_bytes: 0,
                device_weight_bytes: 0,
                host_working_bytes: 0,
                device_working_bytes: 0,
                cache_bytes_per_token: 0,
            },
        };
        &DESCRIPTOR
    }

    fn execution_scalar_type(&self) -> domain_contracts::ScalarType {
        domain_contracts::ScalarType::F32
    }

    fn execution_device(&self) -> domain_contracts::ExecutionDevice {
        domain_contracts::ExecutionDevice::new(
            domain_contracts::DeviceId::new(0),
            domain_contracts::DeviceKind::Cpu,
        )
    }

    fn accounted_footprint(&self) -> domain_contracts::MemoryFootprint {
        domain_contracts::MemoryFootprint::default()
    }

    fn plan_sequence(
        &self,
        configuration: &domain_contracts::SequenceConfiguration,
    ) -> Result<domain_contracts::SequencePlan, domain_contracts::ModelError> {
        Ok(domain_contracts::SequencePlan {
            configuration: *configuration,
            expected_footprint: domain_contracts::MemoryFootprint::default(),
            logits_capacity: self.vocabulary,
        })
    }

    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        configuration: &domain_contracts::SequenceConfiguration,
    ) -> Result<Self::Sequence, domain_contracts::ModelError> {
        Ok(TestSequence {
            id: sequence_id,
            position: 0,
            capacity: configuration.maximum_tokens.get() as usize,
            state: SequenceState::Empty,
        })
    }

    fn prefill_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        input: &PrefillInput<'_>,
    ) -> PrefillBufferRequirements {
        PrefillBufferRequirements {
            logits: if input.emit_logits {
                self.vocabulary
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
            logits: self.vocabulary,
        }
    }

    fn prefill_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: PrefillInput<'_>,
        mut buffers: PreparedPrefillBuffers<'_>,
    ) -> Result<PrefillOutcome, domain_contracts::SequenceError> {
        sequence.position += input.tokens.len();
        sequence.state = SequenceState::Ready;
        let logits_written = buffers.required_logits();
        for value in buffers.logits_mut().iter_mut().take(logits_written) {
            *value = 0.0;
        }
        Ok(PrefillOutcome::Ready {
            consumed_tokens: input.tokens.len(),
            position: sequence.position,
            logits_written,
        })
    }

    fn decode_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        _input: DecodeInput,
        mut buffers: PreparedDecodeBuffers<'_>,
    ) -> Result<DecodeOutcome, domain_contracts::SequenceError> {
        sequence.position += 1;
        let logits_written = buffers.required_logits();
        for value in buffers.logits_mut().iter_mut().take(logits_written) {
            *value = 0.0;
        }
        Ok(DecodeOutcome::Ready {
            position: sequence.position,
            logits_written,
        })
    }

    fn destroy_sequence(
        &mut self,
        _sequence: &mut Self::Sequence,
    ) -> Result<(), domain_contracts::SequenceError> {
        Ok(())
    }

    fn reset_sequence(
        &mut self,
        sequence: &mut Self::Sequence,
    ) -> Result<(), domain_contracts::SequenceError> {
        sequence.position = 0;
        sequence.state = SequenceState::Empty;
        Ok(())
    }

    fn synchronize(&mut self) -> Result<(), domain_contracts::SynchronizationError> {
        Ok(())
    }

    fn prepare_unload(&mut self) -> Result<(), domain_contracts::SynchronizationError> {
        Ok(())
    }
}

struct RetryablePreparation {
    plan: LoadPlan,
    cleanup_should_fail: bool,
}

impl PreparedLoad for RetryablePreparation {
    fn plan(&self) -> &LoadPlan {
        &self.plan
    }

    fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        if self.cleanup_should_fail {
            self.cleanup_should_fail = false;
            Err(SynchronizationError::InvalidState)
        } else {
            Ok(())
        }
    }
}

#[test]
fn scalar_type_set_tracks_all_portable_categories() {
    let mut observed = ScalarTypeSet::default();
    assert_eq!(observed, ScalarTypeSet::EMPTY);
    assert!(observed.is_empty());

    for scalar_type in [
        ScalarType::F32,
        ScalarType::F16,
        ScalarType::Bf16,
        ScalarType::I8,
        ScalarType::U8,
        ScalarType::Other(7),
    ] {
        observed.insert(scalar_type);
        assert!(observed.contains(scalar_type));
    }

    assert_eq!(observed.bits(), 0b00_111111);
    assert!(observed.contains(ScalarType::Other(u16::MAX)));

    let floating = ScalarTypeSet::from_scalar(ScalarType::F32)
        .union(ScalarTypeSet::from_scalar(ScalarType::F16))
        .union(ScalarTypeSet::from_scalar(ScalarType::Bf16));
    assert!(floating.is_subset_of(observed));
    assert!(!observed.is_subset_of(floating));
}

#[test]
fn memory_footprint_checked_totals_detect_overflow() {
    let footprint = MemoryFootprint {
        host_weight_bytes: 11,
        device_weight_bytes: 13,
        host_working_bytes: 17,
        device_working_bytes: 19,
        cache_bytes_per_token: 23,
    };
    assert_eq!(footprint.checked_host_bytes(), Some(28));
    assert_eq!(footprint.checked_device_bytes(), Some(32));

    let overflowing = MemoryFootprint {
        host_weight_bytes: u64::MAX,
        device_weight_bytes: u64::MAX - 1,
        host_working_bytes: 1,
        device_working_bytes: 2,
        cache_bytes_per_token: 0,
    };
    assert_eq!(overflowing.checked_host_bytes(), None);
    assert_eq!(overflowing.checked_device_bytes(), None);
    assert_eq!(overflowing.host_bytes(), u64::MAX);
    assert_eq!(overflowing.device_bytes(), u64::MAX);
}

#[test]
fn failed_load_preserves_retryable_cleanup_ownership() {
    let model = TestModel { vocabulary: 16 };
    let plan = LoadPlan {
        accepted_configuration: LoadConfiguration {
            handle: domain_contracts::LoadedModel::handle(&model),
            execution_device: domain_contracts::LoadedModel::execution_device(&model),
            memory_budget: MemoryBudget {
                host_bytes: 1,
                device_bytes: 1,
            },
        },
        descriptor: *domain_contracts::LoadedModel::descriptor(&model),
        execution_scalar_type: ScalarType::F32,
        expected_footprint: MemoryFootprint::default(),
        loading_peak_footprint: MemoryFootprint::default(),
    };
    let mut failed = FailedLoad::new(
        LoadError::InvalidSource,
        RetryablePreparation {
            plan,
            cleanup_should_fail: true,
        },
    );

    assert_eq!(failed.primary(), LoadError::InvalidSource);
    assert_eq!(failed.cleanup_owner().plan(), &plan);
    assert_eq!(
        failed.cleanup_owner_mut().cleanup(),
        Err(SynchronizationError::InvalidState)
    );
    assert_eq!(failed.cleanup_owner_mut().cleanup(), Ok(()));

    let (primary, cleanup_owner) = failed.into_parts();
    assert_eq!(primary, LoadError::InvalidSource);
    assert_eq!(cleanup_owner.plan(), &plan);
}

#[test]
fn drain_timeout_escalates_to_forced_cancellation() -> Result<(), &'static str> {
    let mut lifecycle = ModelLifecycle::new();
    assert_eq!(lifecycle.begin_load(), Ok(LifecycleAction::None));
    assert_eq!(lifecycle.complete_load(), Ok(LifecycleAction::None));
    assert_eq!(lifecycle.start_request(), Ok(LifecycleAction::None));

    let timeout = DrainTimeout::from_millis(25).map_err(|_| "non-zero timeout rejected")?;
    assert_eq!(
        lifecycle.request_unload(UnloadPolicy::Drain { timeout }, MonotonicMillis::new(100),),
        Ok(LifecycleAction::None)
    );
    assert!(matches!(
        lifecycle.state(),
        ModelLifecycleState::Draining { .. }
    ));

    assert_eq!(
        lifecycle.poll(MonotonicMillis::new(124)),
        Ok(LifecycleAction::None)
    );
    assert_eq!(
        lifecycle.poll(MonotonicMillis::new(125)),
        Ok(LifecycleAction::CancelActive {
            reason: CancellationReason::DrainTimeout,
        })
    );
    Ok(())
}

#[test]
fn decode_capacity_exhaustion_finishes_without_backend_entry() {
    let mut model = TestModel { vocabulary: 16 };
    let mut sequence = TestSequence {
        id: SequenceId::new(1),
        position: 2,
        capacity: 8,
        state: SequenceState::Ready,
    };
    let mut logits = [0.0_f32; 8];

    let outcome = decode_checked(
        &mut model,
        &mut sequence,
        DecodeInput::new(TokenId::new(7)),
        DecodeBuffers::new(&mut logits),
        CancellationStatus::Running,
    );

    assert!(matches!(
        outcome,
        Ok(DecodeOutcome::Finished(FinishReason::BufferExhausted(
            domain_contracts::CapacityExhausted {
                resource: CapacityResource::Logits,
                required: 16,
                available: 8,
            }
        )))
    ));
    assert_eq!(sequence.position, 2);
}

#[test]
fn prefill_token_capacity_exhaustion_finishes_without_backend_entry() {
    let mut model = TestModel { vocabulary: 4 };
    let mut sequence = TestSequence {
        id: SequenceId::new(1),
        position: 3,
        capacity: 4,
        state: SequenceState::Ready,
    };
    let tokens = [TokenId::new(1), TokenId::new(2)];
    let mut logits = [0.0_f32; 4];

    let outcome = prefill_checked(
        &mut model,
        &mut sequence,
        PrefillInput::new(&tokens, true),
        PrefillBuffers::new(&mut logits),
        CancellationStatus::Running,
    );

    assert!(matches!(
        outcome,
        Ok(PrefillOutcome::Finished(FinishReason::BufferExhausted(
            domain_contracts::CapacityExhausted {
                resource: CapacityResource::Tokens,
                required: 2,
                available: 1,
            }
        )))
    ));
    assert_eq!(sequence.position, 3);
}

#[test]
fn sequence_configuration_requires_non_zero_bounds() -> Result<(), &'static str> {
    let maximum_tokens = NonZeroU32::new(4096).ok_or("maximum tokens must be non-zero")?;
    let maximum_prefill_batch = NonZeroU32::new(512).ok_or("prefill batch must be non-zero")?;
    let configuration =
        domain_contracts::SequenceConfiguration::new(maximum_tokens, maximum_prefill_batch);
    assert_eq!(configuration.maximum_tokens.get(), 4096);
    assert_eq!(configuration.maximum_prefill_batch.get(), 512);
    Ok(())
}
