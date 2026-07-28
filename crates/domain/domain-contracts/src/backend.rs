//! Statically dispatched contracts between inference engines and backends.

use crate::error::{LoadError, ModelError, SequenceError, SynchronizationError};
use crate::generation::{CancellationStatus, DecodeOutcome, FinishReason, PrefillOutcome};
use crate::model::{
    LoadConfiguration, LoadPlan, ModelDescriptor, ModelMetadata, SequenceConfiguration,
    SequencePlan,
};
use crate::sequence::{
    DecodeBufferRequirements, DecodeBuffers, DecodeInput, PrefillBufferRequirements,
    PrefillBuffers, PrefillInput, PreparedDecodeBuffers, PreparedPrefillBuffers, SequenceState,
};
use crate::{CapacityExhausted, CapacityResource, ModelHandle, SequenceId};

/// Cold-path model loader implemented by one concrete backend adapter.
pub trait ModelLoader {
    /// Backend-specific model source descriptor.
    type Source;
    /// Concrete loaded-model type produced by this loader.
    type Model: LoadedModel;

    /// Inspects model metadata without retaining loaded execution resources.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] when the source is invalid, unsupported, exceeds
    /// inspection capacity, or cannot be inspected by the backend.
    fn inspect(&self, source: &Self::Source) -> Result<ModelDescriptor, LoadError>;

    /// Validates configuration and reports resource requirements before loading.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] when the source or configuration is invalid or
    /// unsupported, required capacity is unavailable, or backend planning fails.
    fn plan_load(
        &self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<LoadPlan, LoadError>;

    /// Loads the model according to a previously validated configuration.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] when validation or allocation fails, loading is
    /// cancelled, or the backend cannot load the model.
    fn load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Model, LoadError>;
}

/// Sequence-owned cache and position state that never owns model weights.
pub trait BackendSequence {
    /// Returns this sequence's stable identity.
    fn id(&self) -> SequenceId;

    /// Returns the current sequence lifecycle state.
    fn state(&self) -> SequenceState;

    /// Returns the number of token positions already consumed.
    fn position(&self) -> usize;

    /// Returns the fixed token capacity allocated during sequence creation.
    fn token_capacity(&self) -> usize;
}

/// Loaded backend model exclusively owned by the inference runtime registry.
///
/// The model owns weights and device execution resources. Associated sequences
/// own only request-specific cache and position state. All prefill and decode
/// operations therefore execute through `&mut self`, so no `Arc<Model>` or
/// model-weight clone is required to keep a sequence alive.
pub trait LoadedModel {
    /// Concrete sequence state operated on by this model.
    type Sequence: BackendSequence;

    /// Returns the runtime handle assigned to this loaded model generation.
    fn handle(&self) -> ModelHandle;

    /// Returns immutable model metadata.
    fn metadata(&self) -> &ModelMetadata;

    /// Validates and reports sequence resource requirements before allocation.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError`] when the configuration or model state is invalid,
    /// the operation is unsupported, capacity is insufficient, or planning fails.
    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError>;

    /// Creates all sequence-owned allocations before the generation hot path starts.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError`] when the model state or configuration is invalid,
    /// the operation is unsupported, capacity is insufficient, or creation fails.
    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        configuration: &SequenceConfiguration,
    ) -> Result<Self::Sequence, ModelError>;

    /// Returns required caller-owned buffers for the given prefill input.
    fn prefill_buffer_requirements(
        &self,
        sequence: &Self::Sequence,
        input: &PrefillInput<'_>,
    ) -> PrefillBufferRequirements;

    /// Returns required caller-owned buffers for one decode step.
    fn decode_buffer_requirements(
        &self,
        sequence: &Self::Sequence,
        input: DecodeInput,
    ) -> DecodeBufferRequirements;

    /// Executes prefill after all caller-owned capacities have been validated.
    ///
    /// # Errors
    ///
    /// Returns [`SequenceError`] when the sequence state is invalid, the operation
    /// is unsupported or cancelled, capacity is exhausted, or backend execution fails.
    fn prefill_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: PrefillInput<'_>,
        buffers: PreparedPrefillBuffers<'_>,
    ) -> Result<PrefillOutcome, SequenceError>;

    /// Executes one incremental decode step after capacities have been validated.
    ///
    /// # Errors
    ///
    /// Returns [`SequenceError`] when the sequence state is invalid, the operation
    /// is unsupported or cancelled, capacity is exhausted, or backend execution fails.
    fn decode_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: DecodeInput,
        buffers: PreparedDecodeBuffers<'_>,
    ) -> Result<DecodeOutcome, SequenceError>;

    /// Releases backend-owned resources before a sequence value is dropped.
    ///
    /// Backends with model-owned or shared cache arenas use this hook to clear
    /// native sequence state and return a backend slot. The sequence is borrowed
    /// so a failed release leaves the runtime-owned value available for a later
    /// retry instead of losing the only cleanup handle. Backends whose resources
    /// are entirely sequence-owned may return success without modifying it.
    ///
    /// # Errors
    ///
    /// Returns [`SequenceError`] when the sequence cannot be destroyed in its
    /// current state or backend resource release fails.
    fn destroy_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError>;

    /// Resets sequence-owned state without reallocating its prepared buffers.
    ///
    /// # Errors
    ///
    /// Returns [`SequenceError`] when the sequence cannot be reset in its current
    /// state, reset is unsupported, or the backend reset fails.
    fn reset_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError>;

    /// Completes pending device work at a coarse lifecycle boundary.
    ///
    /// # Errors
    ///
    /// Returns [`SynchronizationError`] when synchronization is invalid or
    /// cancelled, or when the backend cannot complete pending work.
    fn synchronize(&mut self) -> Result<(), SynchronizationError>;

    /// Prepares deterministic resource destruction after all sequences are gone.
    ///
    /// A failure must leave the model value valid for a later explicit retry.
    /// Success is the only transition that permits the owner to drop or consume
    /// the model as fully released. Backends that cannot provide this retry
    /// contract must not advertise unload through this interface.
    ///
    /// # Errors
    ///
    /// Returns [`SynchronizationError`] when unloading is invalid or cancelled,
    /// or when backend synchronization fails.
    fn prepare_unload(&mut self) -> Result<(), SynchronizationError>;
}

/// Performs one checked prefill operation.
///
/// This function is generic only over the concrete loaded backend model. Its
/// associated sequence type does not add a second independent generic axis.
/// Sampling, stop matching, tokenization, and output batching remain concrete
/// engine operations over flat slices, preventing combinatorial
/// monomorphization of the complete generation loop.
///
/// # Errors
///
/// Returns [`SequenceError`] when the sequence is already finished or when the
/// backend reports an unrecoverable prefill failure. Capacity exhaustion and
/// cancellation are returned as normal [`PrefillOutcome::Finished`] values.
pub fn prefill_checked<M: LoadedModel>(
    model: &mut M,
    sequence: &mut M::Sequence,
    input: PrefillInput<'_>,
    buffers: PrefillBuffers<'_>,
    cancellation: CancellationStatus,
) -> Result<PrefillOutcome, SequenceError> {
    if let Some(reason) = cancellation.reason() {
        return Ok(PrefillOutcome::Finished(FinishReason::Cancelled(reason)));
    }

    if sequence.state() == SequenceState::Finished {
        return Err(SequenceError::InvalidState);
    }

    let available_tokens = sequence
        .token_capacity()
        .saturating_sub(sequence.position());
    if input.tokens.len() > available_tokens {
        return Ok(PrefillOutcome::Finished(FinishReason::BufferExhausted(
            CapacityExhausted::new(
                CapacityResource::Tokens,
                input.tokens.len() as u64,
                available_tokens as u64,
            ),
        )));
    }

    let requirements = model.prefill_buffer_requirements(sequence, &input);
    let prepared = match buffers.prepare(requirements) {
        Ok(prepared) => prepared,
        Err(capacity) => {
            return Ok(PrefillOutcome::Finished(FinishReason::BufferExhausted(
                capacity,
            )));
        }
    };

    match model.prefill_prepared(sequence, input, prepared) {
        Ok(outcome) => Ok(outcome),
        Err(SequenceError::CapacityExhausted(capacity)) => Ok(PrefillOutcome::Finished(
            FinishReason::BufferExhausted(capacity),
        )),
        Err(SequenceError::Cancelled(reason)) => {
            Ok(PrefillOutcome::Finished(FinishReason::Cancelled(reason)))
        }
        Err(error) => Err(error),
    }
}

/// Performs one checked incremental decode operation.
///
/// Capacity exhaustion is converted into a normal finish reason before backend
/// execution whenever possible. This prevents slice growth, unchecked writes,
/// and panic-based control flow in the generation hot path.
///
/// # Errors
///
/// Returns [`SequenceError`] when the sequence is not ready or when the backend
/// reports an unrecoverable decode failure. Capacity exhaustion and cancellation
/// are returned as normal [`DecodeOutcome::Finished`] values.
pub fn decode_checked<M: LoadedModel>(
    model: &mut M,
    sequence: &mut M::Sequence,
    input: DecodeInput,
    buffers: DecodeBuffers<'_>,
    cancellation: CancellationStatus,
) -> Result<DecodeOutcome, SequenceError> {
    if let Some(reason) = cancellation.reason() {
        return Ok(DecodeOutcome::Finished(FinishReason::Cancelled(reason)));
    }

    if sequence.state() != SequenceState::Ready {
        return Err(SequenceError::InvalidState);
    }

    let available_tokens = sequence
        .token_capacity()
        .saturating_sub(sequence.position());
    if available_tokens == 0 {
        return Ok(DecodeOutcome::Finished(FinishReason::BufferExhausted(
            CapacityExhausted::new(CapacityResource::Tokens, 1, 0),
        )));
    }

    let requirements = model.decode_buffer_requirements(sequence, input);
    let prepared = match buffers.prepare(requirements) {
        Ok(prepared) => prepared,
        Err(capacity) => {
            return Ok(DecodeOutcome::Finished(FinishReason::BufferExhausted(
                capacity,
            )));
        }
    };

    match model.decode_prepared(sequence, input, prepared) {
        Ok(outcome) => Ok(outcome),
        Err(SequenceError::CapacityExhausted(capacity)) => Ok(DecodeOutcome::Finished(
            FinishReason::BufferExhausted(capacity),
        )),
        Err(SequenceError::Cancelled(reason)) => {
            Ok(DecodeOutcome::Finished(FinishReason::Cancelled(reason)))
        }
        Err(error) => Err(error),
    }
}
