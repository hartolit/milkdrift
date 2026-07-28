//! Allocation enforcement for the prepared portable inference boundary.

#![forbid(unsafe_code)]

use std::alloc::System;

use domain_contracts::{
    BackendSequence, CancellationStatus, DecodeBufferRequirements, DecodeBuffers, DecodeInput,
    DecodeOutcome, LoadedModel, MemoryFootprint, ModelArchitecture, ModelError, ModelGeneration,
    ModelHandle, ModelId, ModelMetadata, PrefillBufferRequirements, PrefillBuffers, PrefillInput,
    PrefillOutcome, PreparedDecodeBuffers, PreparedPrefillBuffers, QuantizationFormat, ScalarType,
    SequenceConfiguration, SequenceError, SequenceId, SequencePlan, SequenceState,
    SynchronizationError, TokenId, decode_checked, prefill_checked,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const VOCABULARY_SIZE: usize = 32;
const TOKEN_CAPACITY: usize = 512;
const METADATA_VOCABULARY_SIZE: u32 = 32;
const METADATA_CONTEXT_LENGTH: u32 = 512;
const DECODE_STEPS: usize = 128;

struct TestSequence {
    id: SequenceId,
    position: usize,
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
        TOKEN_CAPACITY
    }
}

struct TestModel;

impl LoadedModel for TestModel {
    type Sequence = TestSequence;

    fn handle(&self) -> ModelHandle {
        ModelHandle::new(ModelId::new(1), ModelGeneration::new(1))
    }

    fn metadata(&self) -> &ModelMetadata {
        static METADATA: ModelMetadata = ModelMetadata {
            architecture: ModelArchitecture::Llama,
            scalar_type: ScalarType::F32,
            quantization: QuantizationFormat::None,
            vocabulary_size: METADATA_VOCABULARY_SIZE,
            context_length: METADATA_CONTEXT_LENGTH,
        };
        &METADATA
    }

    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError> {
        Ok(SequencePlan {
            configuration: *configuration,
            expected_footprint: MemoryFootprint::default(),
            logits_capacity: VOCABULARY_SIZE,
        })
    }

    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        _configuration: &SequenceConfiguration,
    ) -> Result<Self::Sequence, ModelError> {
        Ok(TestSequence {
            id: sequence_id,
            position: 0,
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
                VOCABULARY_SIZE
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
            logits: VOCABULARY_SIZE,
        }
    }

    fn prefill_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: PrefillInput<'_>,
        mut buffers: PreparedPrefillBuffers<'_>,
    ) -> Result<PrefillOutcome, SequenceError> {
        sequence.position += input.tokens.len();
        sequence.state = SequenceState::Ready;
        buffers.logits_mut().fill(0.0);
        Ok(PrefillOutcome::Ready {
            consumed_tokens: input.tokens.len(),
            position: sequence.position,
            logits_written: buffers.required_logits(),
        })
    }

    fn decode_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        _input: DecodeInput,
        mut buffers: PreparedDecodeBuffers<'_>,
    ) -> Result<DecodeOutcome, SequenceError> {
        sequence.position += 1;
        buffers.logits_mut().fill(0.0);
        Ok(DecodeOutcome::Ready {
            position: sequence.position,
            logits_written: buffers.required_logits(),
        })
    }

    fn destroy_sequence(&mut self, _sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
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
        Ok(())
    }
}

#[test]
fn checked_prefill_and_decode_do_not_allocate_after_preparation() {
    let prompt = [TokenId::new(1); 8];
    let mut logits = [0.0_f32; VOCABULARY_SIZE];
    let mut model = TestModel;
    let mut sequence = TestSequence {
        id: SequenceId::new(1),
        position: 0,
        state: SequenceState::Empty,
    };

    let region = Region::new(GLOBAL);
    let prefill_ready = matches!(
        prefill_checked(
            &mut model,
            &mut sequence,
            PrefillInput::new(&prompt, true),
            PrefillBuffers::new(&mut logits),
            CancellationStatus::Running,
        ),
        Ok(PrefillOutcome::Ready { .. })
    );
    let mut every_decode_ready = true;
    for _ in 0..DECODE_STEPS {
        every_decode_ready &= matches!(
            decode_checked(
                &mut model,
                &mut sequence,
                DecodeInput::new(TokenId::new(2)),
                DecodeBuffers::new(&mut logits),
                CancellationStatus::Running,
            ),
            Ok(DecodeOutcome::Ready { .. })
        );
    }
    let allocation_change = region.change();

    assert!(prefill_ready);
    assert!(every_decode_ready);
    assert_eq!(allocation_change.allocations, 0, "{allocation_change:?}");
    assert_eq!(allocation_change.reallocations, 0, "{allocation_change:?}");
}
