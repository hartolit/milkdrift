//! Loaded llama.cpp model and logical sequence implementation.

use domain_contracts::{
    BackendFailureKind, BackendId, BackendSequence, CapacityExhausted, CapacityResource,
    DecodeBufferRequirements, DecodeInput, DecodeOutcome, LoadedModel, MemoryFootprint,
    ModelDescriptor, ModelError, ModelHandle, PrefillBufferRequirements, PrefillInput,
    PrefillOutcome, PreparedDecodeBuffers, PreparedPrefillBuffers, SequenceConfiguration,
    SequenceError, SequenceId, SequencePlan, SequenceState, SynchronizationError,
};
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::token::LlamaToken;

use crate::digest::Sha256Digest;
use crate::failure::{
    CODE_BATCH_ADD, CODE_DECODE, CODE_KV_CLEAR, CODE_NUMERIC_OVERFLOW, CODE_SEQUENCE_SLOT, failure,
};
use crate::loader::GgufBackendRuntime;
use crate::source::GgufExecutionConfiguration;

const SLOT_FREE: u8 = 0;
const SLOT_OCCUPIED: u8 = 1;

// Field order is intentional. The native model must be dropped before the
// final backend initialization token is released.
struct NativeOwner {
    model: LlamaModel,
    runtime: GgufBackendRuntime,
}

struct NativeContext<'model> {
    context: LlamaContext<'model>,
}

#[allow(missing_docs, unsafe_code)]
mod generated_cell {
    use self_cell::self_cell;

    use super::{NativeContext, NativeOwner};

    self_cell!(
        pub(super) struct NativeModelCell {
            owner: NativeOwner,

            #[not_covariant]
            dependent: NativeContext,
        }
    );
}

use generated_cell::NativeModelCell;

/// Loaded GGUF model with one shared llama.cpp context and bounded sequence slots.
pub struct GgufModel {
    backend: BackendId,
    handle: ModelHandle,
    descriptor: ModelDescriptor,
    content_digest: Option<Sha256Digest>,
    execution: GgufExecutionConfiguration,
    vocabulary_size: usize,
    // Batch resources are freed before the native context and backend token.
    batch: LlamaBatch<'static>,
    native: NativeModelCell,
    occupied_slots: Box<[u8]>,
    unloading: bool,
}

impl GgufModel {
    pub(crate) fn new(
        backend: BackendId,
        handle: ModelHandle,
        descriptor: ModelDescriptor,
        runtime: GgufBackendRuntime,
        model: LlamaModel,
        context_params: LlamaContextParams,
        execution: GgufExecutionConfiguration,
    ) -> Result<Self, ModelConstructionError> {
        let vocabulary_size = usize::try_from(descriptor.metadata.vocabulary_size)
            .map_err(|_| ModelConstructionError)?;
        let batch_capacity = usize::try_from(execution.maximum_prefill_batch().get())
            .map_err(|_| ModelConstructionError)?;
        let maximum_sequences = usize::try_from(execution.maximum_sequences().get())
            .map_err(|_| ModelConstructionError)?;
        // Each submitted token belongs to one logical sequence. Native sequence
        // identifiers may vary between calls, but the per-token sequence-id
        // array therefore needs exactly one slot.
        let batch = LlamaBatch::new(batch_capacity, 1);
        let owner = NativeOwner { model, runtime };
        let native = NativeModelCell::try_new(owner, move |owner| {
            owner
                .model
                .new_context(owner.runtime.native.as_ref(), context_params)
                .map(|context| NativeContext { context })
                .map_err(|_| ModelConstructionError)
        })?;
        let expected_context = execution
            .total_context_tokens()
            .map_err(|_| ModelConstructionError)?
            .get();
        let context_matches = native.with_dependent(|_, dependent| {
            dependent.context.n_ctx() >= expected_context
                && dependent.context.n_batch() >= execution.maximum_prefill_batch().get()
                && dependent.context.n_ubatch() >= execution.micro_batch_tokens().get()
        });
        if !context_matches {
            return Err(ModelConstructionError);
        }

        let mut occupied_slots = Vec::new();
        occupied_slots
            .try_reserve_exact(maximum_sequences)
            .map_err(|_| ModelConstructionError)?;
        occupied_slots.resize(maximum_sequences, SLOT_FREE);

        Ok(Self {
            backend,
            handle,
            descriptor,
            content_digest: None,
            execution,
            vocabulary_size,
            batch,
            native,
            occupied_slots: occupied_slots.into_boxed_slice(),
            unloading: false,
        })
    }

    /// Returns the complete inspected descriptor retained by the loaded model.
    #[must_use]
    pub const fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    pub(crate) const fn with_content_digest(
        mut self,
        content_digest: Option<Sha256Digest>,
    ) -> Self {
        self.content_digest = content_digest;
        self
    }

    /// Returns the required GGUF identity verified while loading, when supplied.
    #[must_use]
    pub const fn content_digest(&self) -> Option<Sha256Digest> {
        self.content_digest
    }

    fn allocate_slot(&mut self) -> Result<u32, ModelError> {
        let Some(index) = self
            .occupied_slots
            .iter()
            .position(|state| *state == SLOT_FREE)
        else {
            let available = usize_to_u64(self.occupied_slots.len());
            return Err(ModelError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::ActiveSequences,
                available.saturating_add(1),
                available,
            )));
        };
        let Some(occupied) = self.occupied_slots.get_mut(index) else {
            return Err(numeric_model_error(self.backend));
        };
        *occupied = SLOT_OCCUPIED;
        u32::try_from(index).map_err(|_| numeric_model_error(self.backend))
    }

    fn validate_native_slot(&self, native_slot: u32) -> Result<usize, SequenceError> {
        let index =
            usize::try_from(native_slot).map_err(|_| numeric_sequence_error(self.backend))?;
        let state = self
            .occupied_slots
            .get(index)
            .copied()
            .ok_or_else(|| sequence_slot_error(self.backend))?;
        if state != SLOT_OCCUPIED {
            return Err(sequence_slot_error(self.backend));
        }
        Ok(index)
    }

    fn release_native_slot(&mut self, native_slot: u32) -> Result<(), SequenceError> {
        let index = self.validate_native_slot(native_slot)?;

        let cleared = self.native.with_dependent_mut(|_, native| {
            native
                .context
                .clear_kv_cache_seq(Some(native_slot), None, None)
        });
        match cleared {
            Ok(true) => {
                let Some(occupied) = self.occupied_slots.get_mut(index) else {
                    return Err(sequence_slot_error(self.backend));
                };
                *occupied = SLOT_FREE;
                Ok(())
            }
            Ok(false) | Err(_) => Err(kv_clear_error(self.backend)),
        }
    }

    fn execute_prefill(
        &mut self,
        sequence: &GgufSequence,
        input: PrefillInput<'_>,
        logits: &mut [f32],
    ) -> Result<usize, SequenceError> {
        self.validate_native_slot(sequence.native_slot)?;
        self.batch.clear();
        let sequence_id = i32::try_from(sequence.native_slot)
            .map_err(|_| numeric_sequence_error(self.backend))?;
        let last_index = input.tokens.len().saturating_sub(1);
        for (offset, token) in input.tokens.iter().enumerate() {
            let token = token_to_native(*token, self.backend)?;
            let absolute_position = sequence
                .position
                .checked_add(offset)
                .ok_or_else(|| numeric_sequence_error(self.backend))?;
            let position = i32::try_from(absolute_position)
                .map_err(|_| numeric_sequence_error(self.backend))?;
            self.batch
                .add(
                    token,
                    position,
                    &[sequence_id],
                    input.emit_logits && offset == last_index,
                )
                .map_err(|_| batch_sequence_error(self.backend))?;
        }

        let backend = self.backend;
        let batch = &mut self.batch;
        let native = &mut self.native;
        native.with_dependent_mut(|_, dependent| {
            dependent
                .context
                .decode(batch)
                .map_err(|_| decode_sequence_error(backend))?;
            if input.emit_logits {
                copy_logits(dependent.context.get_logits(), logits)
            } else {
                Ok(0)
            }
        })
    }

    fn execute_decode(
        &mut self,
        sequence: &GgufSequence,
        input: DecodeInput,
        logits: &mut [f32],
    ) -> Result<usize, SequenceError> {
        self.validate_native_slot(sequence.native_slot)?;
        self.batch.clear();
        let sequence_id = i32::try_from(sequence.native_slot)
            .map_err(|_| numeric_sequence_error(self.backend))?;
        let position =
            i32::try_from(sequence.position).map_err(|_| numeric_sequence_error(self.backend))?;
        self.batch
            .add(
                token_to_native(input.token, self.backend)?,
                position,
                &[sequence_id],
                true,
            )
            .map_err(|_| batch_sequence_error(self.backend))?;

        let backend = self.backend;
        let batch = &mut self.batch;
        let native = &mut self.native;
        native.with_dependent_mut(|_, dependent| {
            dependent
                .context
                .decode(batch)
                .map_err(|_| decode_sequence_error(backend))?;
            copy_logits(dependent.context.get_logits(), logits)
        })
    }
}

impl LoadedModel for GgufModel {
    type Sequence = GgufSequence;

    fn handle(&self) -> ModelHandle {
        self.handle
    }

    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError> {
        if self.unloading {
            return Err(ModelError::InvalidState);
        }
        if configuration.maximum_tokens.get() > self.execution.context_tokens_per_sequence().get()
            || configuration.maximum_prefill_batch.get()
                > self.execution.maximum_prefill_batch().get()
            || configuration.maximum_prefill_batch.get() > configuration.maximum_tokens.get()
        {
            return Err(ModelError::Unsupported);
        }
        Ok(SequencePlan {
            configuration: *configuration,
            // llama.cpp allocates the shared KV arena with the model context.
            // Sequence creation only reserves a logical slot in that arena.
            expected_footprint: MemoryFootprint::default(),
            logits_capacity: self.vocabulary_size,
        })
    }

    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        configuration: &SequenceConfiguration,
    ) -> Result<Self::Sequence, ModelError> {
        let _plan = self.plan_sequence(configuration)?;
        let token_capacity = usize::try_from(configuration.maximum_tokens.get())
            .map_err(|_| numeric_model_error(self.backend))?;
        let maximum_prefill = usize::try_from(configuration.maximum_prefill_batch.get())
            .map_err(|_| numeric_model_error(self.backend))?;
        let native_slot = self.allocate_slot()?;
        Ok(GgufSequence {
            id: sequence_id,
            native_slot,
            state: SequenceState::Empty,
            position: 0,
            token_capacity,
            maximum_prefill,
        })
    }

    fn prefill_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        input: &PrefillInput<'_>,
    ) -> PrefillBufferRequirements {
        PrefillBufferRequirements {
            logits: if input.emit_logits {
                self.vocabulary_size
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
            logits: self.vocabulary_size,
        }
    }

    fn prefill_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: PrefillInput<'_>,
        mut buffers: PreparedPrefillBuffers<'_>,
    ) -> Result<PrefillOutcome, SequenceError> {
        if self.unloading || sequence.state == SequenceState::Finished || input.tokens.is_empty() {
            return Err(SequenceError::InvalidState);
        }
        if input.tokens.len() > sequence.maximum_prefill {
            return Err(SequenceError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::PrefillBatch,
                usize_to_u64(input.tokens.len()),
                usize_to_u64(sequence.maximum_prefill),
            )));
        }

        let logits_written = match self.execute_prefill(sequence, input, buffers.logits_mut()) {
            Ok(written) => written,
            Err(error) => {
                sequence.state = SequenceState::Finished;
                return Err(error);
            }
        };
        sequence.position = sequence
            .position
            .checked_add(input.tokens.len())
            .ok_or_else(|| numeric_sequence_error(self.backend))?;
        sequence.state = SequenceState::Ready;
        Ok(PrefillOutcome::Ready {
            consumed_tokens: input.tokens.len(),
            position: sequence.position,
            logits_written,
        })
    }

    fn decode_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: DecodeInput,
        mut buffers: PreparedDecodeBuffers<'_>,
    ) -> Result<DecodeOutcome, SequenceError> {
        if self.unloading || sequence.state != SequenceState::Ready {
            return Err(SequenceError::InvalidState);
        }

        let logits_written = match self.execute_decode(sequence, input, buffers.logits_mut()) {
            Ok(written) => written,
            Err(error) => {
                sequence.state = SequenceState::Finished;
                return Err(error);
            }
        };
        sequence.position = sequence
            .position
            .checked_add(1)
            .ok_or_else(|| numeric_sequence_error(self.backend))?;
        Ok(DecodeOutcome::Ready {
            position: sequence.position,
            logits_written,
        })
    }

    fn destroy_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        self.release_native_slot(sequence.native_slot)?;
        sequence.position = 0;
        sequence.state = SequenceState::Finished;
        Ok(())
    }

    fn reset_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        if self.unloading || sequence.state == SequenceState::Finished {
            return Err(SequenceError::InvalidState);
        }
        self.validate_native_slot(sequence.native_slot)?;
        let cleared = self.native.with_dependent_mut(|_, native| {
            native
                .context
                .clear_kv_cache_seq(Some(sequence.native_slot), None, None)
        });
        match cleared {
            Ok(true) => {
                sequence.position = 0;
                sequence.state = SequenceState::Empty;
                Ok(())
            }
            Ok(false) | Err(_) => Err(kv_clear_error(self.backend)),
        }
    }

    fn synchronize(&mut self) -> Result<(), SynchronizationError> {
        if self.unloading {
            return Err(SynchronizationError::InvalidState);
        }
        // llama.cpp's CPU decode call completes before returning.
        Ok(())
    }

    fn prepare_unload(&mut self) -> Result<(), SynchronizationError> {
        if self.unloading || self.occupied_slots.contains(&SLOT_OCCUPIED) {
            return Err(SynchronizationError::InvalidState);
        }
        self.native.with_dependent_mut(|_, native| {
            native.context.clear_kv_cache();
        });
        self.unloading = true;
        Ok(())
    }
}

/// Logical sequence state assigned to one native llama.cpp sequence slot.
pub struct GgufSequence {
    id: SequenceId,
    native_slot: u32,
    state: SequenceState,
    position: usize,
    token_capacity: usize,
    maximum_prefill: usize,
}

impl BackendSequence for GgufSequence {
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelConstructionError;

fn token_to_native(
    token: domain_contracts::TokenId,
    backend: BackendId,
) -> Result<LlamaToken, SequenceError> {
    let value = i32::try_from(token.get()).map_err(|_| numeric_sequence_error(backend))?;
    Ok(LlamaToken::new(value))
}

fn copy_logits(source: &[f32], destination: &mut [f32]) -> Result<usize, SequenceError> {
    if destination.len() < source.len() {
        return Err(SequenceError::CapacityExhausted(CapacityExhausted::new(
            CapacityResource::Logits,
            usize_to_u64(source.len()),
            usize_to_u64(destination.len()),
        )));
    }
    let Some(target) = destination.get_mut(..source.len()) else {
        return Err(SequenceError::CapacityExhausted(CapacityExhausted::new(
            CapacityResource::Logits,
            usize_to_u64(source.len()),
            usize_to_u64(destination.len()),
        )));
    };
    target.copy_from_slice(source);
    Ok(source.len())
}

const fn batch_sequence_error(backend: BackendId) -> SequenceError {
    SequenceError::Backend(failure(
        backend,
        BackendFailureKind::InvalidState,
        CODE_BATCH_ADD,
    ))
}

const fn decode_sequence_error(backend: BackendId) -> SequenceError {
    SequenceError::Backend(failure(
        backend,
        BackendFailureKind::DeviceExecution,
        CODE_DECODE,
    ))
}

const fn kv_clear_error(backend: BackendId) -> SequenceError {
    SequenceError::Backend(failure(
        backend,
        BackendFailureKind::ForeignFunction,
        CODE_KV_CLEAR,
    ))
}

const fn sequence_slot_error(backend: BackendId) -> SequenceError {
    SequenceError::Backend(failure(
        backend,
        BackendFailureKind::InvalidState,
        CODE_SEQUENCE_SLOT,
    ))
}

const fn numeric_model_error(backend: BackendId) -> ModelError {
    ModelError::Backend(failure(
        backend,
        BackendFailureKind::InvalidModel,
        CODE_NUMERIC_OVERFLOW,
    ))
}

const fn numeric_sequence_error(backend: BackendId) -> SequenceError {
    SequenceError::Backend(failure(
        backend,
        BackendFailureKind::InvalidModel,
        CODE_NUMERIC_OVERFLOW,
    ))
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
