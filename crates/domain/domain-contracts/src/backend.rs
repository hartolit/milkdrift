//! Statically dispatched contracts between inference engines and backends.

use crate::error::{LoadError, ModelError, SequenceError, SynchronizationError};
use crate::generation::{CancellationStatus, DecodeOutcome, FinishReason, PrefillOutcome};
use crate::model::{
    ExecutionDevice, LoadConfiguration, LoadPlan, MemoryFootprint, ModelDescriptor, ScalarType,
    SequenceConfiguration, SequencePlan,
};
use crate::sequence::{
    DecodeBufferRequirements, DecodeBuffers, DecodeInput, PrefillBufferRequirements,
    PrefillBuffers, PrefillInput, PreparedDecodeBuffers, PreparedPrefillBuffers, SequenceState,
};
use crate::{CapacityExhausted, CapacityResource, ModelHandle, SequenceId};

/// Backend-owned preparation for one exact model-load transaction.
///
/// The preparation owns the accepted source, configuration, and device
/// authority needed to execute its plan. [`Self::plan`] is stable for the
/// value's entire lifetime. Implementations must not clone or otherwise alias
/// the preparation's materialization authority.
///
/// This pre-attempt value is always ordinary-drop-safe. If materialization
/// acquires native resources and then fails, [`ModelLoader::load_prepared`]
/// consumes this value and returns the distinct [`Self::Failed`] owner instead.
/// That typestate separation prevents callers and runtimes from confusing an
/// unmaterialized plan with a resource-bearing failed attempt.
pub trait PreparedLoad: Sized {
    /// Resource-bearing owner returned if this preparation fails to materialize.
    type Failed: FailedLoadOwner;

    /// Returns the exact, lifetime-stable plan bound to this preparation.
    fn plan(&self) -> &LoadPlan;
}

/// Sole explicit cleanup owner after a failed materialization attempt.
///
/// A failed owner may retain source handles, host/device tensors, native model
/// state, or other backend resources acquired while consuming one
/// [`PreparedLoad`]. It must not be constructible by cloning or aliasing that
/// authority. Ordinary success from [`Self::cleanup`] is the only portable
/// proof that the retained resources were explicitly released. The public
/// [`FailedLoad`] guard encapsulates this raw owner and deliberately retains it
/// when the guard is abandoned before cleanup succeeds, so ordinary error
/// propagation cannot invoke unverified native destruction. Implementations
/// must keep the raw owner valid and retryable after every cleanup error.
pub trait FailedLoadOwner: Sized {
    /// Returns the exact plan whose materialization produced this owner.
    ///
    /// The report is lifetime-stable and allows E0 to verify that a backend did
    /// not substitute a different cleanup owner after consuming an accepted
    /// preparation.
    fn plan(&self) -> &LoadPlan;

    /// Completes explicit cleanup after failed materialization.
    ///
    /// Cleanup is all-or-nothing and retryable. An error must preserve every
    /// remaining resource and the sole cleanup authority unchanged so another
    /// attempt can retry safely. Implementations must not unwind.
    ///
    /// # Errors
    ///
    /// Returns [`SynchronizationError`] when cleanup cannot yet complete or
    /// backend synchronization or resource release fails.
    fn cleanup(&mut self) -> Result<(), SynchronizationError>;
}

/// A model-load failure encapsulating the sole partial-load cleanup owner.
///
/// The primary error describes why materialization failed. The raw backend
/// owner is never exposed: this guard is itself the only portable cleanup
/// authority. Successful [`FailedLoadOwner::cleanup`] authorizes ordinary drop
/// of the contained backend value. Dropping the guard before that proof fails
/// closed by deliberately retaining the unresolved owner until process exit
/// rather than invoking unverified native destruction.
///
/// The guard continues to report the backend owner's current plan before and
/// after every cleanup attempt. Runtimes can therefore detect substitution or
/// mutation instead of trusting the fallback plan captured at construction.
#[must_use = "a failed load retains a cleanup owner that must be handled"]
#[derive(Debug)]
pub struct FailedLoad<F: FailedLoadOwner> {
    primary: LoadError,
    fallback_plan: LoadPlan,
    cleanup_owner: Option<F>,
    cleanup_complete: bool,
}

impl<F: FailedLoadOwner> FailedLoad<F> {
    /// Creates the fail-closed guard for one failed materialization owner.
    pub fn new(primary: LoadError, cleanup_owner: F) -> Self {
        let fallback_plan = *cleanup_owner.plan();
        Self {
            primary,
            fallback_plan,
            cleanup_owner: Some(cleanup_owner),
            cleanup_complete: false,
        }
    }

    /// Returns the primary model-loading error.
    #[must_use]
    pub const fn primary(&self) -> LoadError {
        self.primary
    }

    /// Returns the backend owner's current plan report.
    ///
    /// The report is checked by E0 before and after every cleanup attempt. A
    /// conforming owner returns the same exact plan for its entire lifetime.
    #[must_use]
    pub fn plan(&self) -> &LoadPlan {
        match &self.cleanup_owner {
            Some(cleanup_owner) => cleanup_owner.plan(),
            None => &self.fallback_plan,
        }
    }

    /// Attempts explicit cleanup of the retained backend owner.
    ///
    /// Successful cleanup is idempotent and authorizes ordinary destruction of
    /// the raw backend value when this guard is dropped. A failure retains the
    /// same sole owner for a later retry.
    ///
    /// # Errors
    ///
    /// Returns [`SynchronizationError`] when cleanup cannot yet complete.
    pub fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        if self.cleanup_complete {
            return Ok(());
        }

        let Some(cleanup_owner) = self.cleanup_owner.as_mut() else {
            return Ok(());
        };
        cleanup_owner.cleanup()?;
        self.cleanup_complete = true;
        Ok(())
    }

    /// Returns whether explicit cleanup has completed successfully.
    #[must_use]
    pub const fn cleanup_complete(&self) -> bool {
        self.cleanup_complete
    }
}

impl<F: FailedLoadOwner> FailedLoadOwner for FailedLoad<F> {
    fn plan(&self) -> &LoadPlan {
        Self::plan(self)
    }

    fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        Self::cleanup(self)
    }
}

impl<F: FailedLoadOwner> Drop for FailedLoad<F> {
    fn drop(&mut self) {
        if self.cleanup_complete {
            return;
        }
        if let Some(cleanup_owner) = self.cleanup_owner.take() {
            core::mem::forget(cleanup_owner);
        }
    }
}

/// Cold-path model loader implemented by one concrete backend adapter.
///
/// Preparation binds one exact source, caller configuration, selected device,
/// and stable plan. Materialization consumes that authority exactly once: it
/// must not replan, inspect or consult a replacement source, or clone/alias the
/// cleanup authority. After native resources are acquired, implementations must
/// return failures rather than unwind.
///
/// # Generic trust boundary
///
/// The generic E0 boundary verifies portable reports, including handles,
/// descriptors, scalar/device identities, exact footprints, and component-wise
/// arithmetic. Those checks cannot prove physical allocation, actual device
/// placement, or the absence of hidden native aliases; those remain backend
/// implementation responsibilities.
pub trait ModelLoader {
    /// Backend-specific model source descriptor.
    type Source;
    /// Backend-owned exact load preparation.
    type Prepared: PreparedLoad<Failed = Self::FailedPreparation>;
    /// Sole cleanup owner returned after failed materialization.
    type FailedPreparation: FailedLoadOwner;
    /// Concrete loaded-model type produced by this loader.
    type Model: LoadedModel;

    /// Inspects model metadata without retaining loaded execution resources.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] when the source is invalid, unsupported, exceeds
    /// inspection capacity, or cannot be inspected by the backend.
    fn inspect(&self, source: &Self::Source) -> Result<ModelDescriptor, LoadError>;

    /// Creates one exact source-and-configuration-bound load preparation.
    ///
    /// The returned value owns the accepted source, configuration, selected
    /// device, and cleanup authority needed by its lifetime-stable plan. An
    /// unmaterialized preparation rejected by the caller is ordinary-drop-safe.
    /// An error must leave no explicit backend ownership created by this
    /// preparation attempt and therefore returns no cleanup owner.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] when inspection, validation, exact planning, or
    /// backend preparation fails.
    fn prepare_load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Prepared, LoadError>;

    /// Consumes the exact accepted preparation once, without replanning.
    ///
    /// Materialization must use only the preparation's retained authority; it
    /// must not consult a replacement source or substitute configuration/device
    /// state. On success, the model reports the plan's final ownership and the
    /// consumed preparation is fully released. On failure, [`FailedLoad`]
    /// retains the associated failed typestate as the sole cleanup owner;
    /// implementations must neither discard nor alias it while converting the
    /// primary error.
    ///
    /// # Errors
    ///
    /// Returns [`FailedLoad`] when validation, allocation, conversion,
    /// synchronization, cancellation, or backend materialization fails.
    #[expect(
        clippy::result_large_err,
        reason = "the failure guard must retain the concrete sole cleanup owner inline; boxing here would impose allocation and change the portable ownership contract"
    )]
    fn load_prepared(
        &mut self,
        prepared: Self::Prepared,
    ) -> Result<Self::Model, FailedLoad<Self::FailedPreparation>>;
}

/// Sequence-owned cache and position state that never owns model weights.
pub trait BackendSequence {
    /// Returns this sequence's stable identity.
    fn id(&self) -> SequenceId;

    /// Returns the current sequence lifecycle state.
    fn state(&self) -> SequenceState;

    /// Returns the number of token positions already consumed.
    fn position(&self) -> usize;

    /// Returns the fixed token capacity accepted by sequence admission.
    ///
    /// Backends may materialize cache payload incrementally during execution;
    /// this capacity is a logical plan bound, not proof of eager allocation.
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

    /// Returns the complete immutable descriptor retained by the loaded model.
    fn descriptor(&self) -> &ModelDescriptor;

    /// Returns the actual scalar type used by backend execution tensors.
    ///
    /// This may differ from the source scalar metadata in the descriptor and
    /// must equal the execution scalar accepted by the load plan.
    fn execution_scalar_type(&self) -> ScalarType;

    /// Returns the actual backend-visible device used by this loaded model.
    fn execution_device(&self) -> ExecutionDevice;

    /// Returns the backend's exact post-materialization ownership claim.
    ///
    /// This report is not already accepted accounting. E0 must verify it against
    /// the prepared plan before admission is committed. The portable equality
    /// check does not prove physical allocation, device placement, or absence of
    /// hidden native aliases.
    fn reported_footprint(&self) -> MemoryFootprint;

    /// Validates and reports sequence reservation requirements before creation.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError`] when the configuration or model state is invalid,
    /// the operation is unsupported, capacity is insufficient, or planning fails.
    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError>;

    /// Creates one sequence under the previously reported reservation plan.
    ///
    /// Creation establishes the sequence owner and its fixed logical capacities,
    /// but need not eagerly perform every future backend allocation. Execution-time
    /// logical tensor payloads and source transfers must remain covered by
    /// [`SequencePlan::expected_footprint`]; caller-owned logits buffers remain
    /// outside that backend reservation.
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
    /// If an error can occur after mutating sequence cache state, the backend must
    /// make the sequence non-retryable while preserving explicit destruction.
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
    /// If an error can occur after mutating sequence cache state, the backend must
    /// make the sequence non-retryable while preserving explicit destruction.
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
    /// native sequence state and return a backend slot. Implementations must not
    /// clone or alias cleanup authority and must not unwind after acquiring
    /// native resources. Failure is all-or-nothing: model and sequence ownership,
    /// lifecycle state, and portable reports remain stable and retryable. Success
    /// is the sole ordinary authorization to drop the sequence as fully released.
    /// Backends whose resources are entirely sequence-owned may return success
    /// without modifying it.
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
    /// Implementations must not clone or alias cleanup authority and must not
    /// unwind after acquiring native resources. Failure is all-or-nothing: it
    /// preserves model ownership, lifecycle state, and all portable reports for
    /// an identical explicit retry. Success is the sole ordinary authorization
    /// to drop or consume the model as fully released. Backends that cannot
    /// provide this contract must not advertise unload through this interface.
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
