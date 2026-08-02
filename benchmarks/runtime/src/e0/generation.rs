//! Scheduled generation issuance, public-output observation, and correctness validation.

use std::num::{NonZeroU32, NonZeroUsize};
use std::time::{Duration, Instant};

use domain_contracts::{
    CancellationReason, FinishReason, MemoryFootprint, ModelHandle, RequestId, SequenceId, TokenId,
    YieldReason,
};
use host_runtime::TokenOutputRecordKind;
use inference_runtime::{
    CommandTicket, GenerationAdmission, GenerationOutcome, GenerationOutputCapacityPolicy,
    GenerationOutputState, GenerationRequest, RuntimeCommand, RuntimeEvent, RuntimeReceiveError,
    SamplingConfig,
};

use super::harness::HostedE0Harness;
use super::lifecycle::{GENERATION_PROMPT_TOKEN_COUNT, sequence_configuration};
use super::observation::{CapturedSnapshot, capture_snapshot, validate_active_snapshot};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::fixture::{CONTEXT_CAPACITY, VOCABULARY_SIZE};

const EXPECTED_GREEDY_TOKEN: TokenId = TokenId::new(2);
pub(crate) const FIRST_TOKEN_GENERATION_LIMIT: u32 = 6;
pub(crate) const POST_FIRST_TOKEN_WINDOW: u32 = 4;
pub(crate) const BACKPRESSURE_GENERATION_LIMIT: u32 = 4;
pub(crate) const CANCELLATION_GENERATION_LIMIT: u32 = 12;
pub(crate) const BACKPRESSURE_HOLD_MILLISECONDS: u64 = 100;
pub(crate) const CANCELLATION_HOLD_MILLISECONDS: u64 = 25;
const BACKPRESSURE_HOLD: Duration = Duration::from_millis(BACKPRESSURE_HOLD_MILLISECONDS);
const CANCELLATION_HOLD: Duration = Duration::from_millis(CANCELLATION_HOLD_MILLISECONDS);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(1);

pub(super) struct FirstTokenMeasurement {
    pub(super) first_token: Duration,
    pub(super) post_first_proxy: Duration,
}

pub(super) struct BackpressureObservation {
    pub(super) controlled_hold: Duration,
    pub(super) recovery_to_next_token: Duration,
    pub(super) during_backpressure: CapturedSnapshot,
}

pub(super) struct CancellationObservation {
    pub(super) generated_tokens: u32,
    pub(super) acknowledgement: Duration,
    pub(super) terminal: Duration,
    pub(super) released: Duration,
}

pub(super) fn measure_first_token_and_proxy(
    harness: &mut HostedE0Harness,
    handle: ModelHandle,
) -> BenchmarkResult<FirstTokenMeasurement> {
    let request = generation_request(harness, FIRST_TOKEN_GENERATION_LIMIT)?;
    let request_id = request.request_id;
    let sequence_id = request.sequence_id;
    let scheduler_quantum = request.scheduler_quantum;
    let ticket = harness.ticket()?;
    let command = RuntimeCommand::Generate {
        ticket,
        handle,
        request,
    };
    let started = Instant::now();
    harness.submit(command, "first-token generation")?;
    let admission_event = harness.receive(ticket, "first-token generation admission")?;
    validate_admission(
        &admission_event,
        ticket,
        request_id,
        sequence_id,
        scheduler_quantum,
    )?;
    let deadline = checked_deadline(started, OPERATION_TIMEOUT, "first-token generation")?;
    let mut output = OutputObservation::default();
    while output.generated_tokens == 0 {
        pull_output(harness, request_id, &mut output)?;
        if output.generated_tokens != 0 {
            break;
        }
        if output.released.is_some() {
            return Err(BenchmarkError::new(
                "first-token generation released before a token was observed",
            ));
        }
        wait_until(deadline, "first token at the public pull boundary")?;
    }
    let first_observed = Instant::now();
    let first_token = first_observed.saturating_duration_since(started);
    let target = 1_u32
        .checked_add(POST_FIRST_TOKEN_WINDOW)
        .ok_or_else(|| BenchmarkError::new("post-first-token target overflowed"))?;
    while output.generated_tokens < target {
        pull_output(harness, request_id, &mut output)?;
        if output.generated_tokens >= target {
            break;
        }
        if output.released.is_some() {
            return Err(BenchmarkError::new(
                "generation released before the fixed post-first-token proxy window completed",
            ));
        }
        wait_until(deadline, "post-first-token proxy window")?;
    }
    let post_first_proxy = first_observed.elapsed();
    collect_until_released(harness, request_id, deadline, &mut output)?;
    validate_generation(
        &output,
        FIRST_TOKEN_GENERATION_LIMIT,
        GenerationOutcome::Finished(FinishReason::TokenLimit),
    )?;
    Ok(FirstTokenMeasurement {
        first_token,
        post_first_proxy,
    })
}

pub(super) fn measure_backpressure(
    harness: &mut HostedE0Harness,
    handle: ModelHandle,
    loaded_footprint: MemoryFootprint,
) -> BenchmarkResult<BackpressureObservation> {
    let request = generation_request(harness, BACKPRESSURE_GENERATION_LIMIT)?;
    let request_id = request.request_id;
    let sequence_id = request.sequence_id;
    let scheduler_quantum = request.scheduler_quantum;
    let ticket = harness.ticket()?;
    harness.submit(
        RuntimeCommand::Generate {
            ticket,
            handle,
            request,
        },
        "backpressure generation",
    )?;
    let admission_event = harness.receive(ticket, "backpressure generation admission")?;
    let admission = validate_admission(
        &admission_event,
        ticket,
        request_id,
        sequence_id,
        scheduler_quantum,
    )?;
    let hold_started = Instant::now();
    std::thread::sleep(BACKPRESSURE_HOLD);
    let controlled_hold = hold_started.elapsed();
    let during_backpressure = capture_snapshot(harness, "during-generation-backpressure")?;
    validate_active_snapshot(
        &during_backpressure,
        handle,
        loaded_footprint,
        admission.request.reserved_footprint,
        "during generation backpressure",
    )?;

    let mut output = OutputObservation::default();
    let recovery_started = Instant::now();
    pull_output(harness, request_id, &mut output)?;
    if output.generated_tokens != 1 || !output.output_backpressure_observed {
        return Err(BenchmarkError::new(
            "controlled hold did not expose one retained token and explicit output backpressure",
        ));
    }
    let deadline = checked_deadline(recovery_started, OPERATION_TIMEOUT, "backpressure recovery")?;
    while output.generated_tokens < 2 {
        pull_output(harness, request_id, &mut output)?;
        if output.generated_tokens >= 2 {
            break;
        }
        wait_until(deadline, "next token after freeing public pull output")?;
    }
    let recovery_to_next_token = recovery_started.elapsed();
    collect_until_released(harness, request_id, deadline, &mut output)?;
    validate_generation(
        &output,
        BACKPRESSURE_GENERATION_LIMIT,
        GenerationOutcome::Finished(FinishReason::TokenLimit),
    )?;
    Ok(BackpressureObservation {
        controlled_hold,
        recovery_to_next_token,
        during_backpressure,
    })
}

pub(super) fn measure_cancellation(
    harness: &mut HostedE0Harness,
    handle: ModelHandle,
) -> BenchmarkResult<CancellationObservation> {
    let (request_id, mut output) = setup_cancellation_generation(harness, handle)?;
    let cancellation_ticket = harness.ticket()?;
    let command = RuntimeCommand::CancelRequest {
        ticket: cancellation_ticket,
        request_id,
        reason: CancellationReason::UserRequested,
    };
    let started = Instant::now();
    harness.submit(command, "generation cancellation")?;
    let deadline = checked_deadline(started, OPERATION_TIMEOUT, "generation cancellation")?;
    let expected =
        GenerationOutcome::Finished(FinishReason::Cancelled(CancellationReason::UserRequested));
    let observations = observe_cancellation(
        harness,
        request_id,
        cancellation_ticket,
        started,
        deadline,
        expected,
        &mut output,
    )?;
    validate_generation(&output, output.generated_tokens, expected)?;
    if output.generated_tokens == 0 || output.generated_tokens >= CANCELLATION_GENERATION_LIMIT {
        return Err(BenchmarkError::new(
            "controlled cancellation did not stop after a non-zero partial generation window",
        ));
    }
    Ok(observations)
}

fn setup_cancellation_generation(
    harness: &mut HostedE0Harness,
    handle: ModelHandle,
) -> BenchmarkResult<(RequestId, OutputObservation)> {
    let request = generation_request(harness, CANCELLATION_GENERATION_LIMIT)?;
    let request_id = request.request_id;
    let sequence_id = request.sequence_id;
    let scheduler_quantum = request.scheduler_quantum;
    let ticket = harness.ticket()?;
    harness.submit(
        RuntimeCommand::Generate {
            ticket,
            handle,
            request,
        },
        "cancellation generation",
    )?;
    let admission_event = harness.receive(ticket, "cancellation generation admission")?;
    validate_admission(
        &admission_event,
        ticket,
        request_id,
        sequence_id,
        scheduler_quantum,
    )?;
    let started = Instant::now();
    let deadline = checked_deadline(
        started,
        OPERATION_TIMEOUT,
        "cancellation precondition token",
    )?;
    let mut output = OutputObservation::default();
    while output.generated_tokens == 0 {
        pull_output(harness, request_id, &mut output)?;
        if output.generated_tokens != 0 {
            break;
        }
        if output.terminal.is_some() || output.released.is_some() {
            return Err(BenchmarkError::new(
                "cancellation precondition generation ended before its first token",
            ));
        }
        wait_until(deadline, "cancellation precondition token")?;
    }
    std::thread::sleep(CANCELLATION_HOLD);
    Ok((request_id, output))
}

fn observe_cancellation(
    harness: &HostedE0Harness,
    request_id: RequestId,
    cancellation_ticket: CommandTicket,
    started: Instant,
    deadline: Instant,
    expected: GenerationOutcome,
    output: &mut OutputObservation,
) -> BenchmarkResult<CancellationObservation> {
    let mut acknowledgement = None;
    let mut terminal = None;
    let mut released = None;
    while acknowledgement.is_none() || terminal.is_none() || released.is_none() {
        poll_cancellation_acknowledgement(
            harness,
            request_id,
            cancellation_ticket,
            started,
            &mut acknowledgement,
        )?;
        pull_output(harness, request_id, output)?;
        validate_cancellation_outcome(output, expected)?;
        if terminal.is_none() && output.terminal == Some(expected) {
            terminal = Some(started.elapsed());
        }
        if released.is_none() && output.released == Some(expected) {
            released = Some(started.elapsed());
        }
        if acknowledgement.is_some() && terminal.is_some() && released.is_some() {
            break;
        }
        wait_until(
            deadline,
            "cancellation Terminal, acknowledgement, and Released",
        )?;
    }
    Ok(CancellationObservation {
        generated_tokens: output.generated_tokens,
        acknowledgement: required_duration(acknowledgement, "cancellation acknowledgement")?,
        terminal: required_duration(terminal, "cancellation Terminal")?,
        released: required_duration(released, "cancellation Released")?,
    })
}

fn poll_cancellation_acknowledgement(
    harness: &HostedE0Harness,
    request_id: RequestId,
    cancellation_ticket: CommandTicket,
    started: Instant,
    acknowledgement: &mut Option<Duration>,
) -> BenchmarkResult {
    match harness.runtime()?.try_receive() {
        Ok(RuntimeEvent::GenerationCancellationRequested {
            ticket,
            request_id: event_request,
            result: Ok(()),
        }) if ticket == cancellation_ticket && event_request == request_id => {
            if acknowledgement.is_none() {
                *acknowledgement = Some(started.elapsed());
            }
            Ok(())
        }
        Ok(RuntimeEvent::GenerationCancellationRequested {
            result: Err(error), ..
        }) => Err(BenchmarkError::new(format!(
            "generation cancellation acknowledgement failed: {error:?}"
        ))),
        Ok(event) => Err(BenchmarkError::new(format!(
            "unexpected event while awaiting cancellation acknowledgement: ticket {}",
            event.ticket().get()
        ))),
        Err(RuntimeReceiveError::Timeout) => Ok(()),
        Err(RuntimeReceiveError::Disconnected) => Err(BenchmarkError::new(
            "runtime disconnected while awaiting cancellation acknowledgement",
        )),
    }
}

fn validate_cancellation_outcome(
    output: &OutputObservation,
    expected: GenerationOutcome,
) -> BenchmarkResult {
    if output.terminal.is_some_and(|outcome| outcome != expected)
        || output.released.is_some_and(|outcome| outcome != expected)
    {
        return Err(BenchmarkError::new(
            "controlled cancellation published an unexpected terminal outcome",
        ));
    }
    Ok(())
}

fn required_duration(value: Option<Duration>, label: &str) -> BenchmarkResult<Duration> {
    value.ok_or_else(|| BenchmarkError::new(format!("{label} was not observed")))
}

fn validate_admission(
    event: &RuntimeEvent,
    ticket: CommandTicket,
    request_id: RequestId,
    sequence_id: SequenceId,
    scheduler_quantum: NonZeroU32,
) -> BenchmarkResult<GenerationAdmission> {
    match event {
        RuntimeEvent::GenerationAdmitted {
            ticket: event_ticket,
            result: Ok(admission),
        } if *event_ticket == ticket
            && admission.request.request_id == request_id
            && admission.request.sequence_id == sequence_id
            && admission.request.logits_capacity == usize_from_u32(VOCABULARY_SIZE)?
            && admission.request.reserved_footprint != MemoryFootprint::default()
            && admission.scheduler_quantum == scheduler_quantum =>
        {
            Ok(*admission)
        }
        RuntimeEvent::GenerationAdmitted {
            result: Err(error), ..
        } => Err(BenchmarkError::new(format!(
            "generation admission failed: {error:?}"
        ))),
        _ => Err(BenchmarkError::new(
            "generation admission returned unexpected ticket, identity, logits capacity, footprint, or scheduler quantum",
        )),
    }
}

#[derive(Default)]
struct OutputObservation {
    generated_tokens: u32,
    terminal: Option<GenerationOutcome>,
    released: Option<GenerationOutcome>,
    output_backpressure_observed: bool,
    cleanup_pending_observed: bool,
    cleanup_exhausted_observed: bool,
}

fn pull_output(
    harness: &HostedE0Harness,
    request_id: RequestId,
    output: &mut OutputObservation,
) -> BenchmarkResult {
    harness
        .runtime()?
        .pull_token_output(|batch| {
            for record in batch.records {
                if record.request_id != request_id {
                    return Err(BenchmarkError::new(
                        "public token output addressed an unexpected request",
                    ));
                }
                match record.kind {
                    TokenOutputRecordKind::Tokens(range) => {
                        let tokens = batch.tokens_for(range).ok_or_else(|| {
                            BenchmarkError::new("public token output contained an invalid range")
                        })?;
                        if tokens.iter().any(|token| {
                            token.get() >= VOCABULARY_SIZE || *token != EXPECTED_GREEDY_TOKEN
                        }) {
                            return Err(BenchmarkError::new(
                                "synthetic generation produced a token outside its reviewed deterministic expectation",
                            ));
                        }
                        let additional = u32::try_from(tokens.len()).map_err(|_| {
                            BenchmarkError::new("generated token count conversion failed")
                        })?;
                        output.generated_tokens = output
                            .generated_tokens
                            .checked_add(additional)
                            .ok_or_else(|| {
                                BenchmarkError::new("generated token count overflowed")
                            })?;
                    }
                    TokenOutputRecordKind::State(state) => match state {
                        GenerationOutputState::Yielded(YieldReason::OutputBackpressure(_)) => {
                            output.output_backpressure_observed = true;
                        }
                        GenerationOutputState::Yielded(_) => {}
                        GenerationOutputState::Terminal(outcome) => {
                            record_outcome(&mut output.terminal, outcome, "Terminal")?;
                        }
                        GenerationOutputState::Released(outcome) => {
                            record_outcome(&mut output.released, outcome, "Released")?;
                        }
                        GenerationOutputState::CleanupPending { .. } => {
                            output.cleanup_pending_observed = true;
                            return Err(BenchmarkError::new(
                                "generation entered CleanupPending; sample is invalid",
                            ));
                        }
                        GenerationOutputState::CleanupExhausted { .. } => {
                            output.cleanup_exhausted_observed = true;
                            return Err(BenchmarkError::new(
                                "generation entered CleanupExhausted; sample is invalid",
                            ));
                        }
                    },
                }
            }
            Ok(())
        })
        .map_err(|error| {
            BenchmarkError::new(format!("public token output pull failed: {error:?}"))
        })??;
    Ok(())
}

fn record_outcome(
    destination: &mut Option<GenerationOutcome>,
    outcome: GenerationOutcome,
    label: &str,
) -> BenchmarkResult {
    if destination.is_some_and(|existing| existing != outcome) {
        return Err(BenchmarkError::new(format!(
            "generation published inconsistent {label} outcomes"
        )));
    }
    *destination = Some(outcome);
    Ok(())
}

fn collect_until_released(
    harness: &HostedE0Harness,
    request_id: RequestId,
    deadline: Instant,
    output: &mut OutputObservation,
) -> BenchmarkResult {
    while output.released.is_none() {
        pull_output(harness, request_id, output)?;
        if output.released.is_some() {
            break;
        }
        wait_until(deadline, "generation Released state")?;
    }
    Ok(())
}

fn validate_generation(
    output: &OutputObservation,
    expected_tokens: u32,
    expected_outcome: GenerationOutcome,
) -> BenchmarkResult {
    if output.generated_tokens != expected_tokens
        || output.terminal != Some(expected_outcome)
        || output.released != Some(expected_outcome)
        || output.cleanup_pending_observed
        || output.cleanup_exhausted_observed
    {
        return Err(BenchmarkError::new(format!(
            "generation validation failed: observed {} tokens, Terminal={}, Released={}, cleanup_pending={}, cleanup_exhausted={}",
            output.generated_tokens,
            output.terminal.is_some(),
            output.released.is_some(),
            output.cleanup_pending_observed,
            output.cleanup_exhausted_observed
        )));
    }
    Ok(())
}

fn generation_request(
    harness: &mut HostedE0Harness,
    maximum_generated_tokens: u32,
) -> BenchmarkResult<GenerationRequest> {
    let (request_id, sequence_id) = harness.request_identity()?;
    let maximum_generated_tokens = NonZeroU32::new(maximum_generated_tokens)
        .ok_or_else(|| BenchmarkError::new("generation token count must be non-zero"))?;
    Ok(GenerationRequest {
        request_id,
        sequence_id,
        prompt_tokens: vec![TokenId::new(1), TokenId::new(2)].into_boxed_slice(),
        sequence: sequence_configuration(CONTEXT_CAPACITY, GENERATION_PROMPT_TOKEN_COUNT)?,
        maximum_generated_tokens,
        sampling: SamplingConfig::greedy(),
        seed: 17,
        eos_tokens: Box::new([]),
        stop_sequences: Box::new([]),
        scheduler_quantum: NonZeroU32::MIN,
        output_capacity: GenerationOutputCapacityPolicy::new(NonZeroUsize::MIN, NonZeroUsize::MIN),
    })
}

fn checked_deadline(
    started: Instant,
    timeout: Duration,
    operation: &str,
) -> BenchmarkResult<Instant> {
    started
        .checked_add(timeout)
        .ok_or_else(|| BenchmarkError::new(format!("{operation} deadline overflowed")))
}

fn wait_until(deadline: Instant, operation: &str) -> BenchmarkResult {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| {
            BenchmarkError::new(format!(
                "{operation} exceeded the hard operational timeout; no performance threshold was applied"
            ))
        })?;
    std::thread::sleep(POLL_INTERVAL.min(remaining));
    Ok(())
}

fn usize_from_u32(value: u32) -> BenchmarkResult<usize> {
    usize::try_from(value)
        .map_err(|_| BenchmarkError::new("u32-to-usize capacity conversion failed"))
}
