//! Allocation-free phase transitions for one scheduled generation opportunity.

use domain_contracts::{
    CapacityExhausted, CapacityResource, FinishReason, ModelLoader, RequestId, TokenId, YieldReason,
};
use host_runtime::{OutputPushError, TokenOutputProducer};
use sampling::SamplingWorkspace;

use crate::{
    CleanupRetryState, FailureClass, GenerationOutcome, InferenceRuntime, RuntimeError,
    RuntimeOperation,
};

use super::{GenerationOutputState, GenerationTask, SchedulerAdvance};

#[derive(Clone, Copy)]
#[expect(
    clippy::large_enum_variant,
    reason = "terminal publication retains bounded cleanup evidence inline until ordered release"
)]
pub(super) enum GenerationPhase {
    Prefill(PrefillPhase),
    PendingToken(PendingTokenPhase),
    Decode(DecodePhase),
    Terminal(TerminalPublication),
}

#[derive(Clone, Copy)]
pub(super) struct PrefillPhase;

#[derive(Clone, Copy)]
pub(super) struct PendingTokenPhase {
    token: TokenId,
}

#[derive(Clone, Copy)]
pub(super) struct DecodePhase;

#[derive(Clone, Copy)]
pub(super) struct TerminalPublication {
    outcome: GenerationOutcome,
    stage: TerminalPublicationStage,
}

#[derive(Clone, Copy)]
enum TerminalPublicationStage {
    Terminal {
        initial_cleanup: Option<CleanupRetryState>,
    },
    CleanupPending(CleanupRetryState),
    AwaitingCleanup,
    CleanupExhausted,
    Release,
}

enum PhaseTransition {
    Idle(GenerationPhase),
    Progressed(GenerationPhase),
    Completed,
    OutputPoisoned(GenerationPhase),
}

enum YieldPublication {
    Continue,
    Idle,
    OutputPoisoned,
}

pub(super) fn advance_task<L: ModelLoader>(
    runtime: &mut InferenceRuntime<L>,
    output: &TokenOutputProducer<GenerationOutputState>,
    request_id: RequestId,
    task: &mut GenerationTask,
) -> SchedulerAdvance {
    let phase = task.phase;
    if !matches!(phase, GenerationPhase::Terminal(_)) {
        if let Some(reason) = task.cancellation.take() {
            return cancellation_transition(runtime, request_id, reason).apply(task, request_id);
        }
        match publish_pending_yield(output, request_id, task) {
            YieldPublication::Continue => {}
            YieldPublication::Idle => return idle(),
            YieldPublication::OutputPoisoned => return output_poisoned(),
        }
    }

    let transition = match phase {
        GenerationPhase::Prefill(prefill) => advance_prefill(runtime, request_id, task, prefill),
        GenerationPhase::PendingToken(pending) => {
            advance_pending_token(runtime, output, request_id, task, pending)
        }
        GenerationPhase::Decode(decode) => advance_decode(runtime, request_id, task, decode),
        GenerationPhase::Terminal(terminal) => {
            advance_terminal(runtime, output, request_id, terminal)
        }
    };
    transition.apply(task, request_id)
}

impl PhaseTransition {
    fn apply(self, task: &mut GenerationTask, request_id: RequestId) -> SchedulerAdvance {
        match self {
            Self::Idle(phase) => {
                task.phase = phase;
                idle()
            }
            Self::Progressed(phase) => {
                task.phase = phase;
                progressed()
            }
            Self::Completed => SchedulerAdvance {
                progressed: true,
                completed: Some(request_id),
                output_poisoned: false,
            },
            Self::OutputPoisoned(phase) => {
                task.phase = phase;
                output_poisoned()
            }
        }
    }
}

fn cancellation_transition<L: ModelLoader>(
    runtime: &mut InferenceRuntime<L>,
    request_id: RequestId,
    reason: domain_contracts::CancellationReason,
) -> PhaseTransition {
    let cleanup_error = if runtime.is_request_active(request_id) {
        runtime.cancel_request(request_id, reason).err()
    } else {
        runtime
            .request_cleanup_state(request_id)
            .map(RuntimeError::CleanupFailed)
    };
    PhaseTransition::Progressed(terminal_phase(
        runtime,
        request_id,
        GenerationOutcome::Finished(FinishReason::Cancelled(reason)),
        cleanup_error,
    ))
}

fn publish_pending_yield(
    output: &TokenOutputProducer<GenerationOutputState>,
    request_id: RequestId,
    task: &mut GenerationTask,
) -> YieldPublication {
    let Some(reason) = task.pending_yield else {
        return YieldPublication::Continue;
    };
    match output.try_push_state(request_id, GenerationOutputState::Yielded(reason)) {
        Ok(()) => {
            task.pending_yield = None;
            YieldPublication::Continue
        }
        Err(OutputPushError::ConsumerBusy | OutputPushError::CapacityExhausted(_)) => {
            YieldPublication::Idle
        }
        Err(OutputPushError::Poisoned) => YieldPublication::OutputPoisoned,
    }
}

fn advance_prefill<L: ModelLoader>(
    runtime: &mut InferenceRuntime<L>,
    request_id: RequestId,
    task: &mut GenerationTask,
    _phase: PrefillPhase,
) -> PhaseTransition {
    let result = runtime.prefill(
        request_id,
        &task.prompt_tokens,
        true,
        task.logits.as_mut_slice(),
    );
    match result {
        Ok(receipt) => match receipt.outcome {
            domain_contracts::PrefillOutcome::Ready { logits_written, .. } => sample_pending_token(
                runtime,
                request_id,
                task,
                logits_written,
                RuntimeOperation::Prefill,
            ),
            domain_contracts::PrefillOutcome::Finished(reason) => {
                PhaseTransition::Progressed(terminal_phase(
                    runtime,
                    request_id,
                    GenerationOutcome::Finished(reason),
                    None,
                ))
            }
        },
        Err(error) => PhaseTransition::Progressed(terminal_phase_from_runtime_error(
            runtime, request_id, error,
        )),
    }
}

fn advance_pending_token<L: ModelLoader>(
    runtime: &mut InferenceRuntime<L>,
    output: &TokenOutputProducer<GenerationOutputState>,
    request_id: RequestId,
    task: &mut GenerationTask,
    phase: PendingTokenPhase,
) -> PhaseTransition {
    match output.try_push_token(request_id, phase.token) {
        Ok(()) => match task.finish_after_token(phase.token) {
            Some(reason) => {
                let cleanup = runtime.complete_request(request_id, reason).err();
                PhaseTransition::Progressed(terminal_phase(
                    runtime,
                    request_id,
                    GenerationOutcome::Finished(reason),
                    cleanup,
                ))
            }
            None => PhaseTransition::Progressed(GenerationPhase::Decode(DecodePhase)),
        },
        Err(OutputPushError::CapacityExhausted(capacity)) => {
            task.pending_yield = Some(YieldReason::OutputBackpressure(capacity));
            PhaseTransition::Idle(GenerationPhase::PendingToken(phase))
        }
        Err(OutputPushError::ConsumerBusy) => {
            task.pending_yield = Some(YieldReason::OutputBackpressure(CapacityExhausted::new(
                CapacityResource::OutputRecords,
                1,
                0,
            )));
            PhaseTransition::Idle(GenerationPhase::PendingToken(phase))
        }
        Err(OutputPushError::Poisoned) => {
            PhaseTransition::OutputPoisoned(GenerationPhase::PendingToken(phase))
        }
    }
}

fn advance_decode<L: ModelLoader>(
    runtime: &mut InferenceRuntime<L>,
    request_id: RequestId,
    task: &mut GenerationTask,
    _phase: DecodePhase,
) -> PhaseTransition {
    let Some(token) = task.generated.last().copied() else {
        return fail_backend_contract(runtime, request_id, RuntimeOperation::Decode);
    };
    let result = runtime.decode(request_id, token, task.logits.as_mut_slice());
    match result {
        Ok(receipt) => match receipt.outcome {
            domain_contracts::DecodeOutcome::Ready { logits_written, .. } => sample_pending_token(
                runtime,
                request_id,
                task,
                logits_written,
                RuntimeOperation::Decode,
            ),
            domain_contracts::DecodeOutcome::Finished(reason) => {
                PhaseTransition::Progressed(terminal_phase(
                    runtime,
                    request_id,
                    GenerationOutcome::Finished(reason),
                    None,
                ))
            }
        },
        Err(error) => PhaseTransition::Progressed(terminal_phase_from_runtime_error(
            runtime, request_id, error,
        )),
    }
}

fn sample_pending_token<L: ModelLoader>(
    runtime: &mut InferenceRuntime<L>,
    request_id: RequestId,
    task: &mut GenerationTask,
    logits_written: usize,
    operation: RuntimeOperation,
) -> PhaseTransition {
    if logits_written != task.logits.len() {
        return fail_backend_contract(runtime, request_id, operation);
    }
    let sample = task.sampler.sample(
        task.logits.as_mut_slice(),
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
            return PhaseTransition::Progressed(terminal_phase(
                runtime,
                request_id,
                GenerationOutcome::Failed(primary),
                cleanup,
            ));
        }
    };
    task.generated.push(token);
    task.history.push(token);
    PhaseTransition::Progressed(GenerationPhase::PendingToken(PendingTokenPhase { token }))
}

fn fail_backend_contract<L: ModelLoader>(
    runtime: &mut InferenceRuntime<L>,
    request_id: RequestId,
    operation: RuntimeOperation,
) -> PhaseTransition {
    let primary = RuntimeError::BackendContractViolation;
    let cleanup = runtime
        .fail_request(request_id, operation, primary.failure_class())
        .err();
    PhaseTransition::Progressed(terminal_phase(
        runtime,
        request_id,
        GenerationOutcome::Failed(primary),
        cleanup,
    ))
}

fn advance_terminal<L: ModelLoader>(
    runtime: &InferenceRuntime<L>,
    output: &TokenOutputProducer<GenerationOutputState>,
    request_id: RequestId,
    mut terminal: TerminalPublication,
) -> PhaseTransition {
    loop {
        let (state, next_stage) = match terminal.stage {
            TerminalPublicationStage::Terminal { initial_cleanup } => {
                let next = initial_cleanup.map_or(
                    TerminalPublicationStage::Release,
                    TerminalPublicationStage::CleanupPending,
                );
                (GenerationOutputState::Terminal(terminal.outcome), next)
            }
            TerminalPublicationStage::CleanupPending(initial_cleanup) => {
                let retry = runtime
                    .request_cleanup_state(request_id)
                    .unwrap_or(initial_cleanup);
                (
                    GenerationOutputState::CleanupPending {
                        outcome: terminal.outcome,
                        failure: retry.failure(),
                        retry,
                    },
                    TerminalPublicationStage::AwaitingCleanup,
                )
            }
            TerminalPublicationStage::AwaitingCleanup => {
                let Some(retry) = runtime.request_cleanup_state(request_id) else {
                    terminal.stage = TerminalPublicationStage::Release;
                    continue;
                };
                if !retry.exhausted() {
                    return PhaseTransition::Idle(GenerationPhase::Terminal(terminal));
                }
                (
                    GenerationOutputState::CleanupExhausted {
                        outcome: terminal.outcome,
                        failure: retry.failure(),
                        retry,
                    },
                    TerminalPublicationStage::CleanupExhausted,
                )
            }
            TerminalPublicationStage::CleanupExhausted => {
                if runtime.request_cleanup_state(request_id).is_some() {
                    return PhaseTransition::Idle(GenerationPhase::Terminal(terminal));
                }
                terminal.stage = TerminalPublicationStage::Release;
                continue;
            }
            TerminalPublicationStage::Release => (
                GenerationOutputState::Released(terminal.outcome),
                TerminalPublicationStage::Release,
            ),
        };

        match output.try_push_state(request_id, state) {
            Ok(()) => {}
            Err(OutputPushError::ConsumerBusy | OutputPushError::CapacityExhausted(_)) => {
                return PhaseTransition::Idle(GenerationPhase::Terminal(terminal));
            }
            Err(OutputPushError::Poisoned) => {
                return PhaseTransition::OutputPoisoned(GenerationPhase::Terminal(terminal));
            }
        }
        if matches!(terminal.stage, TerminalPublicationStage::Release) {
            return PhaseTransition::Completed;
        }
        terminal.stage = next_stage;
        return PhaseTransition::Progressed(GenerationPhase::Terminal(terminal));
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
        Some(RuntimeError::CleanupFailed(state)) => (outcome, Some(state)),
        Some(error @ RuntimeError::CleanupRetryExhausted(_)) => (
            outcome,
            retained_cleanup.or_else(|| cleanup_state_from_error(error)),
        ),
        Some(error) => (GenerationOutcome::Failed(error), retained_cleanup),
        None => (outcome, retained_cleanup),
    };
    terminal_publication(outcome, initial_cleanup)
}

fn terminal_phase_from_runtime_error<L: ModelLoader>(
    runtime: &InferenceRuntime<L>,
    request_id: RequestId,
    error: RuntimeError,
) -> GenerationPhase {
    terminal_publication(
        GenerationOutcome::Failed(error),
        runtime
            .request_cleanup_state(request_id)
            .or_else(|| cleanup_state_from_error(error)),
    )
}

const fn terminal_publication(
    outcome: GenerationOutcome,
    initial_cleanup: Option<CleanupRetryState>,
) -> GenerationPhase {
    GenerationPhase::Terminal(TerminalPublication {
        outcome,
        stage: TerminalPublicationStage::Terminal { initial_cleanup },
    })
}

const fn cleanup_state_from_error(error: RuntimeError) -> Option<CleanupRetryState> {
    match error {
        RuntimeError::CleanupFailed(state) | RuntimeError::CleanupRetryExhausted(state) => {
            Some(state)
        }
        _ => None,
    }
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

const fn progressed() -> SchedulerAdvance {
    SchedulerAdvance {
        progressed: true,
        completed: None,
        output_poisoned: false,
    }
}

const fn idle() -> SchedulerAdvance {
    SchedulerAdvance {
        progressed: false,
        completed: None,
        output_poisoned: false,
    }
}

const fn output_poisoned() -> SchedulerAdvance {
    SchedulerAdvance {
        progressed: false,
        completed: None,
        output_poisoned: true,
    }
}
