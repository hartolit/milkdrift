//! Allocation enforcement for the prepared portable inference boundary.

#![forbid(unsafe_code)]

use std::alloc::System;
use std::process::ExitCode;

use domain_contracts::{
    BackendId, BackendSequence, CancellationStatus, CapabilitySet, DecodeBufferRequirements,
    DecodeBuffers, DecodeInput, DecodeOutcome, DeviceId, DeviceKind, ExecutionDevice, LoadedModel,
    MemoryFootprint, ModelArchitecture, ModelCapabilities, ModelDescriptor, ModelError,
    ModelGeneration, ModelHandle, ModelId, ModelMetadata, PrefillBufferRequirements,
    PrefillBuffers, PrefillInput, PrefillOutcome, PreparedDecodeBuffers, PreparedPrefillBuffers,
    QuantizationFormat, ScalarType, ScalarTypeSet, SequenceConfiguration, SequenceError,
    SequenceId, SequencePlan, SequenceReservation, SequenceState, SynchronizationError, TokenId,
    decode_checked, prefill_checked,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

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
    plan: SequencePlan,
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

    fn reported_plan(&self) -> SequencePlan {
        self.plan
    }
}

struct TestModel;

impl LoadedModel for TestModel {
    type Sequence = TestSequence;

    fn handle(&self) -> ModelHandle {
        ModelHandle::new(ModelId::new(1), ModelGeneration::new(1))
    }

    fn descriptor(&self) -> &ModelDescriptor {
        static DESCRIPTOR: ModelDescriptor = ModelDescriptor {
            backend: BackendId::new(1),
            metadata: ModelMetadata {
                architecture: ModelArchitecture::Llama,
                configuration_declared_scalar_type: Some(ScalarType::F32),
                observed_tensor_scalar_types: ScalarTypeSet::from_scalar(ScalarType::F32),
                quantization: QuantizationFormat::None,
                vocabulary_size: METADATA_VOCABULARY_SIZE,
                context_length: METADATA_CONTEXT_LENGTH,
            },
            capabilities: ModelCapabilities {
                operations: CapabilitySet::PREFILL.union(CapabilitySet::INCREMENTAL_DECODE),
                maximum_context_tokens: METADATA_CONTEXT_LENGTH,
                maximum_sequences: 1,
                maximum_prefill_batch: METADATA_CONTEXT_LENGTH,
            },
            estimated_footprint: MemoryFootprint::ZERO,
        };
        &DESCRIPTOR
    }

    fn execution_scalar_type(&self) -> ScalarType {
        ScalarType::F32
    }

    fn execution_device(&self) -> ExecutionDevice {
        ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu)
    }

    fn reported_footprint(&self) -> MemoryFootprint {
        MemoryFootprint::default()
    }

    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError> {
        Ok(SequencePlan {
            configuration: *configuration,
            reservation: SequenceReservation::default(),
            logits_capacity: VOCABULARY_SIZE,
        })
    }

    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        configuration: &SequenceConfiguration,
    ) -> Result<Self::Sequence, ModelError> {
        let plan = self.plan_sequence(configuration)?;
        Ok(TestSequence {
            id: sequence_id,
            position: 0,
            state: SequenceState::Empty,
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

type DecodeResult = Result<DecodeOutcome, SequenceError>;

fn main() -> ExitCode {
    let prompt = [TokenId::new(1); 8];
    let mut logits = [1.0_f32; VOCABULARY_SIZE];
    let mut model = TestModel;
    let mut sequence = TestSequence {
        id: SequenceId::new(1),
        position: 0,
        state: SequenceState::Empty,
        plan: SequencePlan {
            configuration: SequenceConfiguration::new(
                std::num::NonZeroU32::MIN,
                std::num::NonZeroU32::MIN,
            ),
            reservation: SequenceReservation::default(),
            logits_capacity: VOCABULARY_SIZE,
        },
    };

    let (prefill_result, prefill_allocation_change) =
        measure_prefill(&mut model, &mut sequence, &prompt, &mut logits);
    let expected_prefill_position = prompt.len();
    let prefill_result_is_correct = prefill_result_is_correct(&prefill_result, prompt.len());
    let prefill_state_is_correct = sequence.state == SequenceState::Ready
        && sequence.position == expected_prefill_position
        && logits_are_zero(&logits);
    let prefill_is_allocation_free = allocation_change_is_zero(&prefill_allocation_change);

    if !prefill_result_is_correct {
        eprintln!("prefill returned an unexpected outcome: {prefill_result:?}");
    }
    if !prefill_state_is_correct {
        eprintln!(
            "prefill produced invalid state: sequence_state={:?}, position={}, logits_zeroed={}",
            sequence.state,
            sequence.position,
            logits_are_zero(&logits)
        );
    }
    if !prefill_result_is_correct || !prefill_state_is_correct {
        return ExitCode::FAILURE;
    }

    logits.fill(1.0);
    let (decode_results, decode_allocation_change) =
        measure_decodes(&mut model, &mut sequence, &mut logits);
    let first_invalid_decode = first_invalid_decode(&decode_results, expected_prefill_position);
    let expected_final_position = expected_prefill_position + DECODE_STEPS;
    let decode_state_is_correct = sequence.state == SequenceState::Ready
        && sequence.position == expected_final_position
        && logits_are_zero(&logits);
    let decode_is_allocation_free = allocation_change_is_zero(&decode_allocation_change);

    if !prefill_is_allocation_free {
        eprintln!("prefill allocated after preparation: {prefill_allocation_change:?}");
    }
    if let Some((step, result)) = first_invalid_decode {
        eprintln!("decode step {step} returned an unexpected outcome: {result:?}");
    }
    if !decode_state_is_correct {
        eprintln!(
            "decode produced invalid state: sequence_state={:?}, position={}, logits_zeroed={}",
            sequence.state,
            sequence.position,
            logits_are_zero(&logits)
        );
    }
    if !decode_is_allocation_free {
        eprintln!("decode allocated after preparation: {decode_allocation_change:?}");
    }

    if prefill_is_allocation_free
        && first_invalid_decode.is_none()
        && decode_state_is_correct
        && decode_is_allocation_free
    {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn measure_prefill(
    model: &mut TestModel,
    sequence: &mut TestSequence,
    prompt: &[TokenId],
    logits: &mut [f32],
) -> (Result<PrefillOutcome, SequenceError>, Stats) {
    let region = Region::new(GLOBAL);
    let result = prefill_checked(
        model,
        sequence,
        PrefillInput::new(prompt, true),
        PrefillBuffers::new(logits),
        CancellationStatus::Running,
    );
    (result, region.change())
}

fn measure_decodes(
    model: &mut TestModel,
    sequence: &mut TestSequence,
    logits: &mut [f32],
) -> ([Option<DecodeResult>; DECODE_STEPS], Stats) {
    let mut results = std::array::from_fn(|_| None);
    let region = Region::new(GLOBAL);
    for result in &mut results {
        *result = Some(decode_checked(
            model,
            sequence,
            DecodeInput::new(TokenId::new(2)),
            DecodeBuffers::new(logits),
            CancellationStatus::Running,
        ));
    }
    (results, region.change())
}

fn prefill_result_is_correct(
    result: &Result<PrefillOutcome, SequenceError>,
    prompt_length: usize,
) -> bool {
    matches!(
        result,
        Ok(PrefillOutcome::Ready {
            consumed_tokens,
            position,
            logits_written,
        }) if *consumed_tokens == prompt_length
            && *position == prompt_length
            && *logits_written == VOCABULARY_SIZE
    )
}

fn first_invalid_decode(
    results: &[Option<DecodeResult>; DECODE_STEPS],
    initial_position: usize,
) -> Option<(usize, &Option<DecodeResult>)> {
    results.iter().enumerate().find_map(|(step, result)| {
        let expected_position = initial_position + step + 1;
        let result_is_correct = matches!(
            result,
            Some(Ok(DecodeOutcome::Ready {
                position,
                logits_written,
            })) if *position == expected_position && *logits_written == VOCABULARY_SIZE
        );
        (!result_is_correct).then_some((step, result))
    })
}

fn allocation_change_is_zero(change: &Stats) -> bool {
    change.allocations == 0 && change.reallocations == 0
}

fn logits_are_zero(logits: &[f32]) -> bool {
    logits.iter().all(|&value| value == 0.0)
}
