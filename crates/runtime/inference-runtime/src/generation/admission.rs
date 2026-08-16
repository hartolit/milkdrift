//! Cold generation validation, workspace reservation, and atomic visibility.

use std::mem::size_of;

use domain_contracts::{
    ByteCount, CapacityExhausted, CapacityResource, MemoryFootprint, ModelHandle, ModelLoader,
    RequestId, TokenId,
};
use host_runtime::TokenOutputProducer;
use sampling::Sampler;

use crate::{GenerationAdmission, InferenceRuntime, RuntimeError};

use super::transition::{GenerationPhase, PrefillPhase};
use super::{
    GenerationOutputState, GenerationRequest, GenerationScheduler, GenerationStopSequence,
    GenerationTask,
};

struct ValidatedGeneration {
    request: GenerationRequest,
    sampler: Sampler,
    maximum_generated_tokens: usize,
    required_sequence: usize,
    vocabulary_size: usize,
    workspace_footprint: MemoryFootprint,
}

struct GenerationWorkspace {
    sampler: Sampler,
    logits: Vec<f32>,
    sampling_indices: Vec<u32>,
    repetition_epochs: Vec<u32>,
    history: Vec<TokenId>,
    generated: Vec<TokenId>,
}

/// Owns every fallible generation-admission result until scheduler and runtime commit.
struct GenerationAdmissionTransaction<'runtime, L>
where
    L: ModelLoader,
{
    request_id: RequestId,
    task: GenerationTask,
    sequence: crate::runtime::SequenceAdmissionTransaction<'runtime, L>,
}

pub(super) fn admit<L: ModelLoader>(
    scheduler: &mut GenerationScheduler,
    runtime: &mut InferenceRuntime<L>,
    output: &TokenOutputProducer<GenerationOutputState>,
    handle: ModelHandle,
    request: GenerationRequest,
) -> Result<GenerationAdmission, RuntimeError> {
    let request_id = request.request_id;
    let validated = validate(scheduler, runtime, output, handle, request)?;
    let transaction = GenerationAdmissionTransaction::prepare(runtime, handle, validated)?;
    if scheduler.requests.contains_key(&request_id) {
        return Err(transaction
            .sequence
            .rollback(RuntimeError::RequestAlreadyActive(request_id)));
    }
    transaction.commit(scheduler)
}

fn validate<L: ModelLoader>(
    scheduler: &GenerationScheduler,
    runtime: &InferenceRuntime<L>,
    output: &TokenOutputProducer<GenerationOutputState>,
    handle: ModelHandle,
    request: GenerationRequest,
) -> Result<ValidatedGeneration, RuntimeError> {
    validate_request_shape(scheduler, output, &request)?;
    let maximum_generated_tokens = usize::try_from(request.maximum_generated_tokens.get())
        .map_err(|_| RuntimeError::BackendContractViolation)?;
    let sequence_capacity = usize::try_from(request.sequence.maximum_tokens.get())
        .map_err(|_| RuntimeError::BackendContractViolation)?;
    let required_sequence = request
        .prompt_tokens
        .len()
        .checked_add(maximum_generated_tokens)
        .ok_or(RuntimeError::MemoryArithmeticOverflow)?;
    if required_sequence > sequence_capacity {
        return Err(token_capacity(required_sequence, sequence_capacity));
    }

    let snapshot = runtime.exact_model_snapshot(handle)?;
    if snapshot.degraded {
        return Err(RuntimeError::ModelDegraded(handle.id));
    }
    let required_operations = domain_contracts::CapabilitySet::PREFILL
        .union(domain_contracts::CapabilitySet::INCREMENTAL_DECODE);
    if !snapshot
        .descriptor
        .capabilities
        .operations
        .contains(required_operations)
    {
        return Err(RuntimeError::Model(
            domain_contracts::ModelError::Unsupported,
        ));
    }
    let vocabulary_size = usize::try_from(snapshot.descriptor.metadata.vocabulary_size)
        .map_err(|_| RuntimeError::BackendContractViolation)?;
    let sampler = Sampler::new(request.sampling, request.seed);
    let workspace_footprint = generation_workspace_footprint(
        vocabulary_size,
        required_sequence,
        maximum_generated_tokens,
        request.prompt_tokens.len(),
        request.eos_tokens.len(),
        &request.stop_sequences,
    )?;

    // Aggregate backend plus caller-owned workspace capacity is admitted before
    // the first workspace allocation or native sequence creation.
    runtime.preflight_generation_resources(
        handle,
        request.request_id,
        request.sequence_id,
        request.sequence,
        workspace_footprint,
        vocabulary_size,
    )?;
    Ok(ValidatedGeneration {
        request,
        sampler,
        maximum_generated_tokens,
        required_sequence,
        vocabulary_size,
        workspace_footprint,
    })
}

fn validate_request_shape(
    scheduler: &GenerationScheduler,
    output: &TokenOutputProducer<GenerationOutputState>,
    request: &GenerationRequest,
) -> Result<(), RuntimeError> {
    if scheduler.requests.contains_key(&request.request_id) {
        return Err(RuntimeError::RequestAlreadyActive(request.request_id));
    }
    if request.prompt_tokens.is_empty() {
        return Err(token_capacity(1, 0));
    }
    let maximum_prefill_batch = usize::try_from(request.sequence.maximum_prefill_batch.get())
        .map_err(|_| RuntimeError::BackendContractViolation)?;
    if request.prompt_tokens.len() > maximum_prefill_batch {
        return Err(token_capacity(
            request.prompt_tokens.len(),
            maximum_prefill_batch,
        ));
    }
    let (token_output_capacity, record_output_capacity) = output.capacities();
    if request.output_capacity.minimum_tokens.get() > token_output_capacity {
        return Err(capacity_error(
            CapacityResource::Tokens,
            request.output_capacity.minimum_tokens.get(),
            token_output_capacity,
        ));
    }
    if request.output_capacity.minimum_records.get() > record_output_capacity {
        return Err(capacity_error(
            CapacityResource::OutputRecords,
            request.output_capacity.minimum_records.get(),
            record_output_capacity,
        ));
    }
    if request
        .stop_sequences
        .iter()
        .any(|stop| stop.tokens.is_empty())
    {
        return Err(token_capacity(1, 0));
    }
    Ok(())
}

impl GenerationWorkspace {
    fn reserve(
        vocabulary_size: usize,
        required_sequence: usize,
        maximum_generated_tokens: usize,
        prompt_tokens: &[TokenId],
        sampler: Sampler,
    ) -> Result<Self, RuntimeError> {
        let mut logits = reserved_f32(vocabulary_size, CapacityResource::Logits)?;
        logits.resize(vocabulary_size, 0.0);
        let mut sampling_indices =
            reserved_u32(vocabulary_size, CapacityResource::SamplingIndices)?;
        sampling_indices.resize(vocabulary_size, 0);
        let mut repetition_epochs = reserved_u32(vocabulary_size, CapacityResource::SamplingMask)?;
        repetition_epochs.resize(vocabulary_size, 0);
        let mut history = reserved_tokens(required_sequence, CapacityResource::RepetitionHistory)?;
        history.extend_from_slice(prompt_tokens);
        let generated = reserved_tokens(maximum_generated_tokens, CapacityResource::Tokens)?;
        Ok(Self {
            sampler,
            logits,
            sampling_indices,
            repetition_epochs,
            history,
            generated,
        })
    }
}

impl<'runtime, L> GenerationAdmissionTransaction<'runtime, L>
where
    L: ModelLoader,
{
    fn prepare(
        runtime: &'runtime mut InferenceRuntime<L>,
        handle: ModelHandle,
        validated: ValidatedGeneration,
    ) -> Result<Self, RuntimeError> {
        let ValidatedGeneration {
            request,
            sampler,
            maximum_generated_tokens,
            required_sequence,
            vocabulary_size,
            workspace_footprint,
        } = validated;
        let workspace = GenerationWorkspace::reserve(
            vocabulary_size,
            required_sequence,
            maximum_generated_tokens,
            &request.prompt_tokens,
            sampler,
        )?;
        let request_id = request.request_id;
        let sequence = runtime.prepare_generation_request(
            handle,
            request_id,
            request.sequence_id,
            request.sequence,
            workspace_footprint,
            vocabulary_size,
        )?;
        Ok(Self {
            request_id,
            task: GenerationTask {
                handle,
                workspace_footprint,
                prompt_tokens: request.prompt_tokens,
                maximum_generated_tokens,
                eos_tokens: request.eos_tokens,
                stop_sequences: request.stop_sequences,
                sampler: workspace.sampler,
                logits: workspace.logits,
                sampling_indices: workspace.sampling_indices,
                repetition_epochs: workspace.repetition_epochs,
                history: workspace.history,
                generated: workspace.generated,
                phase: GenerationPhase::Prefill(PrefillPhase),
                cancellation: None,
                pending_yield: None,
            },
            sequence,
        })
    }

    fn commit(
        self,
        scheduler: &mut GenerationScheduler,
    ) -> Result<GenerationAdmission, RuntimeError> {
        let Self {
            request_id,
            task,
            sequence,
        } = self;
        let receipt = sequence.commit()?;
        debug_assert_eq!(receipt.request_id, request_id);
        // The transaction revalidates scheduler identity immediately before
        // this commit. Runtime registry ownership is made visible first; the
        // crate allocator contract treats this final bounded insertion as
        // infallible after every recoverable admission step has completed.
        let replaced = scheduler.requests.insert(request_id, task);
        debug_assert!(replaced.is_none(), "scheduler request was preflighted");
        Ok(GenerationAdmission { request: receipt })
    }
}

fn generation_workspace_footprint(
    vocabulary_size: usize,
    history_capacity: usize,
    generated_capacity: usize,
    prompt_tokens: usize,
    eos_tokens: usize,
    stop_sequences: &[GenerationStopSequence],
) -> Result<MemoryFootprint, RuntimeError> {
    let logits = allocation_bytes::<f32>(vocabulary_size)?;
    let sampling_indices = allocation_bytes::<u32>(vocabulary_size)?;
    let repetition_epochs = allocation_bytes::<u32>(vocabulary_size)?;
    let history = allocation_bytes::<TokenId>(history_capacity)?;
    let generated = allocation_bytes::<TokenId>(generated_capacity)?;
    let prompt = allocation_bytes::<TokenId>(prompt_tokens)?;
    let eos = allocation_bytes::<TokenId>(eos_tokens)?;
    let stop_descriptors = allocation_bytes::<GenerationStopSequence>(stop_sequences.len())?;
    let stop_tokens = stop_sequences.iter().try_fold(
        ByteCount::ZERO,
        |total, stop| -> Result<ByteCount, RuntimeError> {
            total
                .checked_add(allocation_bytes::<TokenId>(stop.tokens.len())?)
                .ok_or(RuntimeError::MemoryArithmeticOverflow)
        },
    )?;
    let host_working_bytes = logits
        .checked_add(sampling_indices)
        .and_then(|value| value.checked_add(repetition_epochs))
        .and_then(|value| value.checked_add(history))
        .and_then(|value| value.checked_add(generated))
        .and_then(|value| value.checked_add(prompt))
        .and_then(|value| value.checked_add(eos))
        .and_then(|value| value.checked_add(stop_descriptors))
        .and_then(|value| value.checked_add(stop_tokens))
        .ok_or(RuntimeError::MemoryArithmeticOverflow)?;
    Ok(MemoryFootprint::host_working(host_working_bytes))
}

fn allocation_bytes<T>(length: usize) -> Result<ByteCount, RuntimeError> {
    let bytes = length
        .checked_mul(size_of::<T>())
        .ok_or(RuntimeError::MemoryArithmeticOverflow)?;
    u64::try_from(bytes)
        .map(ByteCount::from_u64)
        .map_err(|_| RuntimeError::MemoryArithmeticOverflow)
}

fn reserved_f32(length: usize, resource: CapacityResource) -> Result<Vec<f32>, RuntimeError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| allocation_capacity(resource, length))?;
    Ok(values)
}

fn reserved_u32(length: usize, resource: CapacityResource) -> Result<Vec<u32>, RuntimeError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| allocation_capacity(resource, length))?;
    Ok(values)
}

fn reserved_tokens(
    length: usize,
    resource: CapacityResource,
) -> Result<Vec<TokenId>, RuntimeError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| allocation_capacity(resource, length))?;
    Ok(values)
}

fn allocation_capacity(resource: CapacityResource, required: usize) -> RuntimeError {
    RuntimeError::CapacityExhausted(CapacityExhausted::new(
        resource,
        u64::try_from(required).unwrap_or(u64::MAX),
        0,
    ))
}

fn capacity_error(resource: CapacityResource, required: usize, available: usize) -> RuntimeError {
    RuntimeError::CapacityExhausted(CapacityExhausted::new(
        resource,
        u64::try_from(required).unwrap_or(u64::MAX),
        u64::try_from(available).unwrap_or(u64::MAX),
    ))
}

fn token_capacity(required: usize, available: usize) -> RuntimeError {
    capacity_error(CapacityResource::Tokens, required, available)
}
