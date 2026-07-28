//! Backend-independent generation admission and bounded scheduler state.

use std::collections::BTreeMap;
use std::mem::size_of;
use std::num::{NonZeroU32, NonZeroUsize};

use domain_contracts::{
    CapacityExhausted, CapacityResource, FinishReason, MemoryFootprint, ModelHandle, ModelLoader,
    RequestId, SequenceConfiguration, SequenceId, TokenId, YieldReason,
};
use host_runtime::{OutputPushError, TokenOutputProducer};
use sampling::{Sampler, SamplingConfig, SamplingWorkspace};

use crate::{
    CleanupFailureReport, CleanupRetryState, FailureClass, InferenceRuntime, RequestStartReceipt,
    RuntimeError, RuntimeOperation,
};

/// One owned token stop pattern validated before generation begins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationStopSequence {
    /// Stable caller-defined stop code.
    pub code: u32,
    /// Non-empty token pattern.
    pub tokens: Box<[TokenId]>,
}

/// Minimum shared pull-accumulator capacity required by one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationOutputCapacityPolicy {
    /// Minimum token identifiers that must fit before the consumer pulls.
    pub minimum_tokens: NonZeroUsize,
    /// Minimum token/state records that must fit before the consumer pulls.
    pub minimum_records: NonZeroUsize,
}

impl GenerationOutputCapacityPolicy {
    /// Creates an explicit output-capacity requirement.
    #[must_use]
    pub const fn new(minimum_tokens: NonZeroUsize, minimum_records: NonZeroUsize) -> Self {
        Self {
            minimum_tokens,
            minimum_records,
        }
    }
}

impl Default for GenerationOutputCapacityPolicy {
    fn default() -> Self {
        Self::new(NonZeroUsize::MIN, NonZeroUsize::MIN)
    }
}

/// Runtime-level generation request with no frontend or tokenizer state.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerationRequest {
    /// Generation request identity.
    pub request_id: RequestId,
    /// Backend sequence identity.
    pub sequence_id: SequenceId,
    /// Already-tokenized direct-completion prompt.
    pub prompt_tokens: Box<[TokenId]>,
    /// Model sequence bounds used for backend allocation.
    pub sequence: SequenceConfiguration,
    /// Maximum number of sampled output tokens.
    pub maximum_generated_tokens: NonZeroU32,
    /// Immutable sampling policy.
    pub sampling: SamplingConfig,
    /// Deterministic sampler seed.
    pub seed: u64,
    /// Tokens that terminate generation after being published.
    pub eos_tokens: Box<[TokenId]>,
    /// Token suffix patterns that terminate generation after being published.
    pub stop_sequences: Box<[GenerationStopSequence]>,
    /// Maximum backend steps for one scheduler opportunity.
    pub scheduler_quantum: NonZeroU32,
    /// Minimum capacity required from the shared pull accumulator.
    pub output_capacity: GenerationOutputCapacityPolicy,
}

/// Stable generation outcome retained independently from cleanup disposition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationOutcome {
    /// Generation reached a graceful terminal reason.
    Finished(FinishReason),
    /// Generation failed in the backend, sampler, or runtime.
    Failed(RuntimeError),
}

/// State payload published beside token ranges in the pull accumulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationOutputState {
    /// Generation yielded while retaining all request-owned state.
    Yielded(YieldReason),
    /// Generation work ended; explicit sequence cleanup is ordered after this record.
    Terminal(GenerationOutcome),
    /// Explicit sequence destruction failed and ownership remains quarantined.
    CleanupPending {
        /// Original generation outcome.
        outcome: GenerationOutcome,
        /// Primary and cleanup failure classifications.
        failure: CleanupFailureReport,
        /// Current bounded retry state.
        retry: CleanupRetryState,
    },
    /// Automatic cleanup attempts are exhausted and ownership remains retained.
    CleanupExhausted {
        /// Original generation outcome.
        outcome: GenerationOutcome,
        /// Primary and cleanup failure classifications.
        failure: CleanupFailureReport,
        /// Exhausted bounded retry state.
        retry: CleanupRetryState,
    },
    /// Sequence cleanup completed and request accounting was released.
    Released(GenerationOutcome),
}

/// Successful cold admission of a scheduled generation request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationAdmission {
    /// Backend sequence admission receipt.
    pub request: RequestStartReceipt,
    /// Scheduler quantum retained by the worker.
    pub scheduler_quantum: NonZeroU32,
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "the private generation module exposes scheduler state only to the sibling \
              worker module"
)]
pub(super) struct GenerationScheduler {
    requests: BTreeMap<RequestId, GenerationTask>,
    cursor: Option<RequestId>,
}

struct GenerationTask {
    handle: ModelHandle,
    workspace_footprint: MemoryFootprint,
    prompt_tokens: Box<[TokenId]>,
    maximum_generated_tokens: usize,
    eos_tokens: Box<[TokenId]>,
    stop_sequences: Box<[GenerationStopSequence]>,
    scheduler_quantum: NonZeroU32,
    sampler: Sampler,
    logits: Vec<f32>,
    sampling_indices: Vec<u32>,
    repetition_epochs: Vec<u32>,
    history: Vec<TokenId>,
    generated: Vec<TokenId>,
    phase: GenerationPhase,
    pending_token: Option<TokenId>,
    cancellation: Option<domain_contracts::CancellationReason>,
    pending_yield: Option<YieldReason>,
}

#[derive(Clone, Copy)]
enum GenerationPhase {
    Prefill,
    Decode,
    Terminal(TerminalPublication),
}

#[derive(Clone, Copy)]
struct TerminalPublication {
    outcome: GenerationOutcome,
    initial_cleanup: Option<CleanupRetryState>,
    terminal_published: bool,
    cleanup_published: bool,
    exhaustion_published: bool,
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "the private generation module exposes scheduler progress only to the sibling \
              worker module"
)]
pub(super) struct SchedulerAdvance {
    pub(super) progressed: bool,
    pub(super) completed: Option<RequestId>,
}

impl GenerationScheduler {
    pub(super) const fn new() -> Self {
        Self {
            requests: BTreeMap::new(),
            cursor: None,
        }
    }

    pub(super) fn contains(&self, request_id: RequestId) -> bool {
        self.requests.contains_key(&request_id)
    }

    pub(super) fn request_cancellation(
        &mut self,
        request_id: RequestId,
        reason: domain_contracts::CancellationReason,
    ) -> Result<(), RuntimeError> {
        let task = self
            .requests
            .get_mut(&request_id)
            .ok_or(RuntimeError::RequestNotActive(request_id))?;
        task.cancellation = Some(reason);
        Ok(())
    }

    pub(super) fn request_model_cancellation(
        &mut self,
        model_id: domain_contracts::ModelId,
        reason: domain_contracts::CancellationReason,
    ) {
        for task in self.requests.values_mut() {
            if task.handle.id == model_id {
                task.cancellation = Some(reason);
            }
        }
    }

    pub(super) fn discard_all<L: ModelLoader>(
        &mut self,
        runtime: &mut InferenceRuntime<L>,
    ) -> Result<(), RuntimeError> {
        self.cursor = None;
        let tasks = std::mem::take(&mut self.requests);
        let mut first_error = None;
        for task in tasks.into_values() {
            if let Err(error) = runtime.release_generation_workspace(task.workspace_footprint)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }

        first_error.map_or(Ok(()), Err)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "cold admission keeps validation, allocation, backend creation, rollback, and publication contiguous"
    )]
    pub(super) fn admit<L: ModelLoader>(
        &mut self,
        runtime: &mut InferenceRuntime<L>,
        output: &TokenOutputProducer<GenerationOutputState>,
        handle: ModelHandle,
        request: GenerationRequest,
    ) -> Result<GenerationAdmission, RuntimeError> {
        if self.requests.contains_key(&request.request_id) {
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
        for stop in &request.stop_sequences {
            if stop.tokens.is_empty() {
                return Err(token_capacity(1, 0));
            }
        }

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
        let vocabulary_size = usize::try_from(snapshot.descriptor.metadata.vocabulary_size)
            .map_err(|_| RuntimeError::BackendContractViolation)?;
        let sampler = Sampler::new(request.sampling, request.seed)
            .map_err(|error| RuntimeError::Sampling(error.into()))?;
        let workspace_footprint = generation_workspace_footprint(
            vocabulary_size,
            required_sequence,
            maximum_generated_tokens,
            request.prompt_tokens.len(),
            request.eos_tokens.len(),
            &request.stop_sequences,
        )?;
        runtime.preflight_generation_resources(
            handle,
            request.request_id,
            request.sequence_id,
            request.sequence,
            workspace_footprint,
            vocabulary_size,
        )?;

        let mut logits = reserved_f32(vocabulary_size, CapacityResource::Logits)?;
        logits.resize(vocabulary_size, 0.0);
        let mut sampling_indices =
            reserved_u32(vocabulary_size, CapacityResource::SamplingIndices)?;
        sampling_indices.resize(vocabulary_size, 0);
        let mut repetition_epochs = reserved_u32(vocabulary_size, CapacityResource::SamplingMask)?;
        repetition_epochs.resize(vocabulary_size, 0);
        let mut history = reserved_tokens(required_sequence, CapacityResource::RepetitionHistory)?;
        history.extend_from_slice(&request.prompt_tokens);
        let generated = reserved_tokens(maximum_generated_tokens, CapacityResource::Tokens)?;

        let receipt = runtime.start_generation_request(
            handle,
            request.request_id,
            request.sequence_id,
            request.sequence,
            workspace_footprint,
            vocabulary_size,
        )?;
        if receipt.logits_capacity != vocabulary_size {
            let primary = RuntimeError::BackendContractViolation;
            let cleanup = runtime.fail_request(
                request.request_id,
                RuntimeOperation::SequenceAdmission,
                primary.failure_class(),
            );
            runtime.release_generation_workspace(workspace_footprint)?;
            cleanup?;
            return Err(primary);
        }

        let admission = GenerationAdmission {
            request: receipt,
            scheduler_quantum: request.scheduler_quantum,
        };
        self.requests.insert(
            request.request_id,
            GenerationTask {
                handle,
                workspace_footprint,
                prompt_tokens: request.prompt_tokens,
                maximum_generated_tokens,
                eos_tokens: request.eos_tokens,
                stop_sequences: request.stop_sequences,
                scheduler_quantum: request.scheduler_quantum,
                sampler,
                logits,
                sampling_indices,
                repetition_epochs,
                history,
                generated,
                phase: GenerationPhase::Prefill,
                pending_token: None,
                cancellation: None,
                pending_yield: None,
            },
        );
        Ok(admission)
    }

    pub(super) fn advance<L: ModelLoader>(
        &mut self,
        runtime: &mut InferenceRuntime<L>,
        output: &TokenOutputProducer<GenerationOutputState>,
    ) -> SchedulerAdvance {
        let Some(request_id) = self.next_request() else {
            return SchedulerAdvance {
                progressed: false,
                completed: None,
            };
        };
        self.cursor = Some(request_id);
        let result = {
            let Some(task) = self.requests.get_mut(&request_id) else {
                return SchedulerAdvance {
                    progressed: false,
                    completed: None,
                };
            };
            advance_task(runtime, output, request_id, task)
        };
        if result.completed == Some(request_id) {
            if let Some(task) = self.requests.remove(&request_id) {
                let workspace_footprint = task.workspace_footprint;
                drop(task);
                if let Err(error) = runtime.release_generation_workspace(workspace_footprint) {
                    runtime.record_maintenance_error(error);
                }
            } else {
                runtime.record_maintenance_error(RuntimeError::BackendContractViolation);
            }
        }
        result
    }

    fn next_request(&self) -> Option<RequestId> {
        if let Some(cursor) = self.cursor
            && let Some((request_id, _)) = self
                .requests
                .range((
                    std::ops::Bound::Excluded(cursor),
                    std::ops::Bound::Unbounded,
                ))
                .next()
        {
            return Some(*request_id);
        }
        self.requests
            .first_key_value()
            .map(|(request_id, _)| *request_id)
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the explicit generation state transition is kept contiguous for invariant review"
)]
fn advance_task<L: ModelLoader>(
    runtime: &mut InferenceRuntime<L>,
    output: &TokenOutputProducer<GenerationOutputState>,
    request_id: RequestId,
    task: &mut GenerationTask,
) -> SchedulerAdvance {
    if let GenerationPhase::Terminal(mut terminal) = task.phase {
        let result = publish_terminal(runtime, output, request_id, &mut terminal);
        task.phase = GenerationPhase::Terminal(terminal);
        return result;
    }

    if let Some(reason) = task.cancellation.take() {
        let finish = FinishReason::Cancelled(reason);
        let cleanup_error = if runtime.is_request_active(request_id) {
            runtime.cancel_request(request_id, reason).err()
        } else {
            runtime
                .request_cleanup_failure(request_id)
                .map(RuntimeError::CleanupFailed)
        };
        task.phase = terminal_phase(
            runtime,
            request_id,
            GenerationOutcome::Finished(finish),
            cleanup_error,
        );
        return progressed();
    }

    if let Some(reason) = task.pending_yield {
        match output.try_push_state(request_id, GenerationOutputState::Yielded(reason)) {
            Ok(()) => task.pending_yield = None,
            Err(_) => return idle(),
        }
    }

    if let Some(token) = task.pending_token {
        match output.try_push_token(request_id, token) {
            Ok(()) => {
                task.pending_token = None;
                if let Some(reason) = task.finish_after_token(token) {
                    let cleanup = runtime.complete_request(request_id, reason).err();
                    task.phase = terminal_phase(
                        runtime,
                        request_id,
                        GenerationOutcome::Finished(reason),
                        cleanup,
                    );
                } else {
                    task.phase = GenerationPhase::Decode;
                }
                return progressed();
            }
            Err(error) => {
                task.pending_yield = Some(output_yield(error));
                return idle();
            }
        }
    }

    let quantum = task.scheduler_quantum.get();
    let mut steps = 0_u32;
    while steps < quantum && task.pending_token.is_none() {
        if task.cancellation.is_some() {
            break;
        }
        let backend_result = match task.phase {
            GenerationPhase::Prefill => runtime
                .prefill(
                    request_id,
                    &task.prompt_tokens,
                    true,
                    task.logits.as_mut_slice(),
                )
                .map(|receipt| match receipt.outcome {
                    domain_contracts::PrefillOutcome::Ready { logits_written, .. } => {
                        Ok(logits_written)
                    }
                    domain_contracts::PrefillOutcome::Finished(reason) => {
                        Err(GenerationOutcome::Finished(reason))
                    }
                }),
            GenerationPhase::Decode => {
                let Some(token) = task.generated.last().copied() else {
                    task.phase = terminal_phase(
                        runtime,
                        request_id,
                        GenerationOutcome::Failed(RuntimeError::BackendContractViolation),
                        None,
                    );
                    return progressed();
                };
                runtime
                    .decode(request_id, token, task.logits.as_mut_slice())
                    .map(|receipt| match receipt.outcome {
                        domain_contracts::DecodeOutcome::Ready { logits_written, .. } => {
                            Ok(logits_written)
                        }
                        domain_contracts::DecodeOutcome::Finished(reason) => {
                            Err(GenerationOutcome::Finished(reason))
                        }
                    })
            }
            GenerationPhase::Terminal(_) => return progressed(),
        };

        let logits_written = match backend_result {
            Ok(Ok(written)) => written,
            Ok(Err(outcome)) => {
                task.phase = terminal_phase(runtime, request_id, outcome, None);
                return progressed();
            }
            Err(error) => {
                task.phase = terminal_phase_from_runtime_error(runtime, request_id, error);
                return progressed();
            }
        };
        let Some(sample_logits) = task.logits.get_mut(..logits_written) else {
            let primary = RuntimeError::BackendContractViolation;
            let cleanup = runtime
                .fail_request(
                    request_id,
                    RuntimeOperation::Decode,
                    primary.failure_class(),
                )
                .err();
            task.phase = terminal_phase(
                runtime,
                request_id,
                GenerationOutcome::Failed(primary),
                cleanup,
            );
            return progressed();
        };

        let sample = task.sampler.sample(
            sample_logits,
            &task.history,
            SamplingWorkspace {
                indices: task.sampling_indices.as_mut_slice(),
                seen_tokens: task.repetition_epochs.as_mut_slice(),
            },
        );
        let token = match sample {
            Ok(sample) => sample.token,
            Err(error) => {
                let primary = RuntimeError::Sampling(error.into());
                let cleanup = runtime
                    .fail_request(
                        request_id,
                        RuntimeOperation::Sampling,
                        FailureClass::Sampling,
                    )
                    .err();
                task.phase = terminal_phase(
                    runtime,
                    request_id,
                    GenerationOutcome::Failed(primary),
                    cleanup,
                );
                return progressed();
            }
        };
        task.generated.push(token);
        task.history.push(token);
        task.pending_token = Some(token);
        steps = steps.saturating_add(1);
    }
    progressed()
}

impl GenerationTask {
    fn finish_after_token(&self, token: TokenId) -> Option<FinishReason> {
        if self.eos_tokens.contains(&token) {
            return Some(FinishReason::EndOfSequence(token));
        }
        for stop in &self.stop_sequences {
            if stop.tokens.len() <= self.generated.len()
                && self
                    .generated
                    .get(self.generated.len().saturating_sub(stop.tokens.len())..)
                    == Some(stop.tokens.as_ref())
            {
                return Some(FinishReason::StopCondition);
            }
        }
        (self.generated.len() >= self.maximum_generated_tokens).then_some(FinishReason::TokenLimit)
    }
}

fn publish_terminal<L: ModelLoader>(
    runtime: &InferenceRuntime<L>,
    output: &TokenOutputProducer<GenerationOutputState>,
    request_id: RequestId,
    terminal: &mut TerminalPublication,
) -> SchedulerAdvance {
    if !terminal.terminal_published {
        if output
            .try_push_state(
                request_id,
                GenerationOutputState::Terminal(terminal.outcome),
            )
            .is_err()
        {
            return idle();
        }
        terminal.terminal_published = true;
        return progressed();
    }

    if let Some(initial_cleanup) = terminal.initial_cleanup {
        if !terminal.cleanup_published {
            let retry = runtime
                .request_cleanup_state(request_id)
                .unwrap_or(initial_cleanup);
            if output
                .try_push_state(
                    request_id,
                    GenerationOutputState::CleanupPending {
                        outcome: terminal.outcome,
                        failure: retry.failure,
                        retry,
                    },
                )
                .is_err()
            {
                return idle();
            }
            terminal.cleanup_published = true;
            return progressed();
        }

        if let Some(retry) = runtime.request_cleanup_state(request_id) {
            if retry.exhausted() && !terminal.exhaustion_published {
                if output
                    .try_push_state(
                        request_id,
                        GenerationOutputState::CleanupExhausted {
                            outcome: terminal.outcome,
                            failure: retry.failure,
                            retry,
                        },
                    )
                    .is_err()
                {
                    return idle();
                }
                terminal.exhaustion_published = true;
                return progressed();
            }
            return idle();
        }
    }

    if output
        .try_push_state(
            request_id,
            GenerationOutputState::Released(terminal.outcome),
        )
        .is_err()
    {
        return idle();
    }
    SchedulerAdvance {
        progressed: true,
        completed: Some(request_id),
    }
}

fn terminal_phase<L: ModelLoader>(
    runtime: &InferenceRuntime<L>,
    request_id: RequestId,
    outcome: GenerationOutcome,
    cleanup_error: Option<RuntimeError>,
) -> GenerationPhase {
    let retained_cleanup = runtime.request_cleanup_state(request_id);
    let (outcome, initial_cleanup) = match cleanup_error {
        Some(error @ RuntimeError::CleanupFailed(_)) => {
            let cleanup =
                retained_cleanup.or_else(|| cleanup_state_from_error(runtime, request_id, error));
            if cleanup.is_some() {
                (outcome, cleanup)
            } else {
                (GenerationOutcome::Failed(error), None)
            }
        }
        Some(error @ RuntimeError::CleanupRetryExhausted(_)) => (
            outcome,
            retained_cleanup.or_else(|| cleanup_state_from_error(runtime, request_id, error)),
        ),
        Some(error) => (GenerationOutcome::Failed(error), retained_cleanup),
        None => (outcome, retained_cleanup),
    };
    GenerationPhase::Terminal(TerminalPublication {
        outcome,
        initial_cleanup,
        terminal_published: false,
        cleanup_published: false,
        exhaustion_published: false,
    })
}

fn terminal_phase_from_runtime_error<L: ModelLoader>(
    runtime: &InferenceRuntime<L>,
    request_id: RequestId,
    error: RuntimeError,
) -> GenerationPhase {
    GenerationPhase::Terminal(TerminalPublication {
        outcome: GenerationOutcome::Failed(error),
        initial_cleanup: runtime
            .request_cleanup_state(request_id)
            .or_else(|| cleanup_state_from_error(runtime, request_id, error)),
        terminal_published: false,
        cleanup_published: false,
        exhaustion_published: false,
    })
}

fn cleanup_state_from_error<L: ModelLoader>(
    runtime: &InferenceRuntime<L>,
    request_id: RequestId,
    error: RuntimeError,
) -> Option<CleanupRetryState> {
    match error {
        RuntimeError::CleanupFailed(_) => runtime.request_cleanup_state(request_id),
        RuntimeError::CleanupRetryExhausted(state) => Some(state),
        _ => None,
    }
}

const fn output_yield(error: OutputPushError) -> YieldReason {
    let capacity = match error {
        OutputPushError::CapacityExhausted(capacity) => capacity,
        OutputPushError::ConsumerBusy
        | OutputPushError::Poisoned
        | OutputPushError::InvalidRecordKind => {
            CapacityExhausted::new(CapacityResource::OutputRecords, 1, 0)
        }
    };
    YieldReason::OutputBackpressure(capacity)
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
    let stop_tokens =
        stop_sequences
            .iter()
            .try_fold(0_u64, |total, stop| -> Result<u64, RuntimeError> {
                total
                    .checked_add(allocation_bytes::<TokenId>(stop.tokens.len())?)
                    .ok_or(RuntimeError::MemoryArithmeticOverflow)
            })?;
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
    Ok(MemoryFootprint {
        host_weight_bytes: 0,
        device_weight_bytes: 0,
        host_working_bytes,
        device_working_bytes: 0,
        cache_bytes_per_token: 0,
    })
}

fn allocation_bytes<T>(length: usize) -> Result<u64, RuntimeError> {
    let bytes = length
        .checked_mul(size_of::<T>())
        .ok_or(RuntimeError::MemoryArithmeticOverflow)?;
    u64::try_from(bytes).map_err(|_| RuntimeError::MemoryArithmeticOverflow)
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
    RuntimeError::CapacityExhausted(CapacityExhausted::new(
        CapacityResource::Tokens,
        u64::try_from(required).unwrap_or(u64::MAX),
        u64::try_from(available).unwrap_or(u64::MAX),
    ))
}

const fn progressed() -> SchedulerAdvance {
    SchedulerAdvance {
        progressed: true,
        completed: None,
    }
}

const fn idle() -> SchedulerAdvance {
    SchedulerAdvance {
        progressed: false,
        completed: None,
    }
}
