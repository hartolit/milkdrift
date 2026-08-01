//! Deterministic synthetic lifecycle measurements through public hosted E0 APIs.

use std::num::{NonZeroU32, NonZeroUsize};
use std::time::{Duration, Instant};

use domain_contracts::{
    CancellationReason, CapabilitySet, DeviceId, DeviceKind, FinishReason, MemoryFootprint,
    ModelArchitecture, ModelHandle, ModelId, ModelLifecycleState, PrefillOutcome,
    QuantizationFormat, RequestId, ScalarType, SequenceConfiguration, SequenceId, TokenId,
    UnloadPolicy, YieldReason,
};
use host_runtime::TokenOutputRecordKind;
use inference_runtime::{
    CommandTicket, GenerationAdmission, GenerationOutcome, GenerationOutputCapacityPolicy,
    GenerationOutputState, GenerationRequest, LoadReceipt, RuntimeCommand, RuntimeEvent,
    RuntimeReceiveError, SamplingConfig, UnloadStatus,
};

use super::harness::{CANDLE_BACKEND, CapturedSnapshot, E0Harness};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::fixture::{CONTEXT_CAPACITY, VOCABULARY_SIZE, VerifiedFixture};
use crate::report::{
    BackpressureMeasurement, CancellationMeasurement, GenerationValidation,
    ProxyThroughputMeasurement, SyntheticCycle, SyntheticGenerationEvidence, ThroughputMeasurement,
    duration_ns, throughput,
};

const MODEL_ID: ModelId = ModelId::new(7);
const CPU_DEVICE: DeviceId = DeviceId::new(0);
const EXPECTED_GREEDY_TOKEN: TokenId = TokenId::new(2);
const PREFILL_PROMPT_TOKENS: u32 = 4;
const GENERATION_PROMPT_TOKENS: u32 = 2;
pub(crate) const GENERATION_TOKEN_COUNT: u32 = 6;
pub(crate) const POST_FIRST_TOKEN_WINDOW: u32 = 4;
const BACKPRESSURE_GENERATED_TOKENS: u32 = 4;
const CANCELLATION_GENERATED_TOKEN_LIMIT: u32 = 12;
const BACKPRESSURE_HOLD: Duration = Duration::from_millis(100);
const CANCELLATION_HOLD: Duration = Duration::from_millis(25);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(1);

pub(crate) struct SyntheticCycles {
    pub(crate) warmups: Vec<SyntheticCycle>,
    pub(crate) samples: Vec<SyntheticCycle>,
}

pub(crate) fn run_cycles(
    fixture: &VerifiedFixture,
    warmup_cycles: u32,
    sample_cycles: u32,
) -> BenchmarkResult<SyntheticCycles> {
    let mut warmups = Vec::new();
    let mut samples = Vec::new();
    warmups
        .try_reserve_exact(usize_from_u32(warmup_cycles)?)
        .map_err(|error| {
            BenchmarkError::new(format!("warmup record allocation failed: {error}"))
        })?;
    samples
        .try_reserve_exact(usize_from_u32(sample_cycles)?)
        .map_err(|error| {
            BenchmarkError::new(format!("sample record allocation failed: {error}"))
        })?;

    for ordinal in 1..=warmup_cycles {
        warmups.push(run_cycle(fixture, ordinal)?);
    }
    for ordinal in 1..=sample_cycles {
        samples.push(run_cycle(fixture, ordinal)?);
    }
    Ok(SyntheticCycles { warmups, samples })
}

fn run_cycle(fixture: &VerifiedFixture, ordinal: u32) -> BenchmarkResult<SyntheticCycle> {
    let (mut harness, start_duration) = E0Harness::start()?;
    let body = run_cycle_body(&mut harness, fixture);
    match body {
        Ok(body) => {
            let shutdown = harness.shutdown(true)?;
            Ok(SyntheticCycle {
                ordinal,
                e0_start_ns: duration_ns(start_duration),
                model_load_ns: body.model_load_ns,
                checked_prefill: body.checked_prefill,
                first_token_ns: body.first_token_ns,
                post_first_token_proxy: body.post_first_token_proxy,
                backpressure: body.backpressure,
                cancellation: body.cancellation,
                model_unload_ns: body.model_unload_ns,
                shutdown,
                generations: body.generations,
                snapshots: body.snapshots,
            })
        }
        Err(error) => Err(error.with_cleanup(harness.shutdown(false).map(|_| ()))),
    }
}

struct CycleBody {
    model_load_ns: u64,
    checked_prefill: ThroughputMeasurement,
    first_token_ns: u64,
    post_first_token_proxy: ProxyThroughputMeasurement,
    backpressure: BackpressureMeasurement,
    cancellation: CancellationMeasurement,
    model_unload_ns: u64,
    generations: SyntheticGenerationEvidence,
    snapshots: Vec<crate::report::SnapshotCheckpoint>,
}

fn run_cycle_body(
    harness: &mut E0Harness,
    fixture: &VerifiedFixture,
) -> BenchmarkResult<CycleBody> {
    let mut snapshots = Vec::new();

    let before_load = harness.snapshot("before-load")?;
    validate_empty_snapshot(&before_load, "before load")?;
    snapshots.push(before_load.record);

    let source = fixture.source()?;
    let load_ticket = harness.ticket()?;
    let load_command = RuntimeCommand::LoadModel {
        ticket: load_ticket,
        model_id: MODEL_ID,
        source,
        device: CPU_DEVICE,
        device_kind: DeviceKind::Cpu,
    };
    let load_started = Instant::now();
    harness.submit(load_command, "model load")?;
    let load_event = harness.receive(load_ticket, "model load")?;
    let model_load_ns = duration_ns(load_started.elapsed());
    let loaded = loaded_receipt(&load_event)?;
    validate_loaded_fixture(&loaded)?;
    let handle = loaded.handle;

    let after_load = harness.snapshot("after-load")?;
    validate_loaded_idle_snapshot(&after_load, handle, loaded.reserved_footprint, "after load")?;
    snapshots.push(after_load.record);

    let checked_prefill = measure_checked_prefill(harness, handle)?;
    let after_prefill = harness.snapshot("after-checked-prefill-release")?;
    validate_loaded_idle_snapshot(
        &after_prefill,
        handle,
        loaded.reserved_footprint,
        "after checked prefill release",
    )?;
    snapshots.push(after_prefill.record);

    let first = measure_first_token_and_proxy(harness, handle)?;
    let after_first_release = harness.snapshot("after-first-token-proxy-release")?;
    validate_loaded_idle_snapshot(
        &after_first_release,
        handle,
        loaded.reserved_footprint,
        "after first-token proxy release",
    )?;
    snapshots.push(after_first_release.record);

    let backpressure =
        measure_backpressure(harness, handle, loaded.reserved_footprint, &mut snapshots)?;
    let after_backpressure_release = harness.snapshot("after-backpressure-release")?;
    validate_loaded_idle_snapshot(
        &after_backpressure_release,
        handle,
        loaded.reserved_footprint,
        "after backpressure release",
    )?;
    snapshots.push(after_backpressure_release.record);

    let cancellation = measure_cancellation(harness, handle)?;
    let after_release = harness.snapshot("after-cancellation-release")?;
    validate_loaded_idle_snapshot(
        &after_release,
        handle,
        loaded.reserved_footprint,
        "after cancellation release",
    )?;
    snapshots.push(after_release.record);

    let model_unload_ns = unload_model(harness, handle)?;
    let after_unload = harness.snapshot("after-unload")?;
    validate_empty_snapshot(&after_unload, "after unload")?;
    snapshots.push(after_unload.record);

    Ok(CycleBody {
        model_load_ns,
        checked_prefill,
        first_token_ns: first.first_token_ns,
        post_first_token_proxy: first.proxy,
        backpressure: backpressure.measurement,
        cancellation: cancellation.measurement,
        model_unload_ns,
        generations: SyntheticGenerationEvidence {
            first_token_and_proxy: first.validation,
            backpressure: backpressure.validation,
            cancellation: cancellation.validation,
        },
        snapshots,
    })
}

fn loaded_receipt(event: &RuntimeEvent) -> BenchmarkResult<LoadReceipt> {
    match event {
        RuntimeEvent::ModelLoaded {
            result: Ok(receipt),
            ..
        } => Ok(*receipt),
        RuntimeEvent::ModelLoaded {
            result: Err(error), ..
        } => Err(BenchmarkError::new(format!(
            "fixture model load failed: {error:?}"
        ))),
        _ => Err(BenchmarkError::new(
            "model load command returned an unexpected event",
        )),
    }
}

fn validate_loaded_fixture(loaded: &LoadReceipt) -> BenchmarkResult {
    let descriptor = loaded.descriptor;
    let required = CapabilitySet::PREFILL.union(CapabilitySet::INCREMENTAL_DECODE);
    if loaded.handle.id != MODEL_ID
        || descriptor.backend != CANDLE_BACKEND
        || descriptor.metadata.architecture != ModelArchitecture::Llama
        || descriptor.metadata.scalar_type != ScalarType::F32
        || descriptor.metadata.quantization != QuantizationFormat::None
        || descriptor.metadata.vocabulary_size != VOCABULARY_SIZE
        || descriptor.metadata.context_length != CONTEXT_CAPACITY
        || !descriptor.capabilities.operations.contains(required)
        || descriptor.capabilities.maximum_context_tokens < CONTEXT_CAPACITY
        || descriptor.capabilities.maximum_prefill_batch < PREFILL_PROMPT_TOKENS
        || descriptor.capabilities.maximum_sequences < 1
        || loaded.reserved_footprint == MemoryFootprint::default()
    {
        return Err(BenchmarkError::new(
            "loaded public E0 handle or descriptor does not match the requested model and Llama/Safetensors/F32/vocab16/context16 fixture identity",
        ));
    }
    Ok(())
}

fn measure_checked_prefill(
    harness: &mut E0Harness,
    handle: ModelHandle,
) -> BenchmarkResult<ThroughputMeasurement> {
    let setup = setup_checked_prefill_request(harness, handle)?;
    let logits = vec![0.0_f32; setup.logits_capacity];
    let tokens = vec![
        TokenId::new(1),
        TokenId::new(2),
        TokenId::new(3),
        TokenId::new(4),
    ]
    .into_boxed_slice();
    let ticket = harness.ticket()?;
    let command = RuntimeCommand::Prefill {
        ticket,
        request_id: setup.request_id,
        tokens,
        emit_logits: true,
        logits,
    };
    let started = Instant::now();
    harness.submit(command, "checked prompt prefill")?;
    let event = harness.receive(ticket, "checked prompt prefill")?;
    let elapsed = started.elapsed();
    validate_checked_prefill_event(&event, &setup)?;
    complete_request(
        harness,
        setup.request_id,
        "checked-prefill request completion",
    )?;
    Ok(ThroughputMeasurement {
        duration_ns: duration_ns(elapsed),
        token_count: PREFILL_PROMPT_TOKENS,
        tokens_per_second: throughput(PREFILL_PROMPT_TOKENS, elapsed),
    })
}

struct CheckedPrefillSetup {
    request_id: RequestId,
    logits_capacity: usize,
}

fn setup_checked_prefill_request(
    harness: &mut E0Harness,
    handle: ModelHandle,
) -> BenchmarkResult<CheckedPrefillSetup> {
    let request_id = RequestId::new(10);
    let sequence_id = SequenceId::new(110);
    let ticket = harness.ticket()?;
    harness.submit(
        RuntimeCommand::StartRequest {
            ticket,
            handle,
            request_id,
            sequence_id,
            configuration: sequence_configuration(CONTEXT_CAPACITY, PREFILL_PROMPT_TOKENS)?,
        },
        "checked-prefill request setup",
    )?;
    let event = harness.receive(ticket, "checked-prefill request setup")?;
    let logits_capacity = match &event {
        RuntimeEvent::RequestStarted {
            result: Ok(receipt),
            ..
        } if receipt.request_id == request_id && receipt.sequence_id == sequence_id => {
            receipt.logits_capacity
        }
        RuntimeEvent::RequestStarted {
            result: Err(error), ..
        } => {
            return Err(BenchmarkError::new(format!(
                "checked-prefill request setup failed: {error:?}"
            )));
        }
        _ => {
            return Err(BenchmarkError::new(
                "checked-prefill request setup returned an unexpected event",
            ));
        }
    };
    if logits_capacity != usize_from_u32(VOCABULARY_SIZE)? {
        return Err(BenchmarkError::new(
            "checked-prefill request reported an unexpected logits capacity",
        ));
    }
    Ok(CheckedPrefillSetup {
        request_id,
        logits_capacity,
    })
}

fn validate_checked_prefill_event(
    event: &RuntimeEvent,
    setup: &CheckedPrefillSetup,
) -> BenchmarkResult {
    let expected_tokens = usize_from_u32(PREFILL_PROMPT_TOKENS)?;
    match event {
        RuntimeEvent::PrefillCompleted {
            request_id,
            result: Ok(receipt),
            logits,
            ..
        } if *request_id == setup.request_id && logits.len() == setup.logits_capacity => {
            match receipt.outcome {
                PrefillOutcome::Ready {
                    consumed_tokens,
                    position,
                    logits_written,
                } if consumed_tokens == expected_tokens
                    && position == expected_tokens
                    && logits_written == setup.logits_capacity
                    && receipt.usage.prompt_tokens == u64::from(PREFILL_PROMPT_TOKENS)
                    && receipt.usage.generated_tokens == 0 =>
                {
                    Ok(())
                }
                _ => Err(BenchmarkError::new(
                    "checked prompt prefill returned unexpected outcome or usage",
                )),
            }
        }
        RuntimeEvent::PrefillCompleted {
            result: Err(error), ..
        } => Err(BenchmarkError::new(format!(
            "checked prompt prefill failed: {error:?}"
        ))),
        _ => Err(BenchmarkError::new(
            "checked prompt prefill returned an unexpected event",
        )),
    }
}

fn complete_request(
    harness: &mut E0Harness,
    request_id: RequestId,
    operation: &str,
) -> BenchmarkResult {
    let ticket = harness.ticket()?;
    harness.submit(
        RuntimeCommand::CompleteRequest {
            ticket,
            request_id,
            reason: FinishReason::TokenLimit,
        },
        operation,
    )?;
    match harness.receive(ticket, operation)? {
        RuntimeEvent::RequestFinished {
            request_id: event_request,
            result: Ok(FinishReason::TokenLimit),
            ..
        } if event_request == request_id => Ok(()),
        RuntimeEvent::RequestFinished {
            result: Err(error), ..
        } => Err(BenchmarkError::new(format!(
            "{operation} failed: {error:?}"
        ))),
        _ => Err(BenchmarkError::new(format!(
            "{operation} returned an unexpected event"
        ))),
    }
}

struct FirstTokenMeasurement {
    first_token_ns: u64,
    proxy: ProxyThroughputMeasurement,
    validation: GenerationValidation,
}

fn measure_first_token_and_proxy(
    harness: &mut E0Harness,
    handle: ModelHandle,
) -> BenchmarkResult<FirstTokenMeasurement> {
    let request = generation_request(20, 120, GENERATION_TOKEN_COUNT)?;
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
    let first_token_ns = duration_ns(first_observed.saturating_duration_since(started));
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
    let proxy_elapsed = first_observed.elapsed();
    collect_until_released(harness, request_id, deadline, &mut output)?;
    let validation = validate_generation(
        &output,
        GENERATION_TOKEN_COUNT,
        GenerationOutcome::Finished(FinishReason::TokenLimit),
        "finished:token-limit",
    )?;
    Ok(FirstTokenMeasurement {
        first_token_ns,
        proxy: ProxyThroughputMeasurement {
            label: "synthetic short-window integration proxy; not representative production steady state",
            duration_ns: duration_ns(proxy_elapsed),
            token_count: POST_FIRST_TOKEN_WINDOW,
            tokens_per_second: throughput(POST_FIRST_TOKEN_WINDOW, proxy_elapsed),
        },
        validation,
    })
}

struct BackpressureResult {
    measurement: BackpressureMeasurement,
    validation: GenerationValidation,
}

fn measure_backpressure(
    harness: &mut E0Harness,
    handle: ModelHandle,
    loaded_footprint: MemoryFootprint,
    snapshots: &mut Vec<crate::report::SnapshotCheckpoint>,
) -> BenchmarkResult<BackpressureResult> {
    let request = generation_request(30, 130, BACKPRESSURE_GENERATED_TOKENS)?;
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
    let during = harness.snapshot("during-generation-backpressure")?;
    validate_active_snapshot(
        &during,
        handle,
        loaded_footprint,
        admission.request.reserved_footprint,
        "during generation backpressure",
    )?;
    snapshots.push(during.record);

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
    let recovery = recovery_started.elapsed();
    collect_until_released(harness, request_id, deadline, &mut output)?;
    let validation = validate_generation(
        &output,
        BACKPRESSURE_GENERATED_TOKENS,
        GenerationOutcome::Finished(FinishReason::TokenLimit),
        "finished:token-limit",
    )?;
    Ok(BackpressureResult {
        measurement: BackpressureMeasurement {
            controlled_hold_ns: duration_ns(controlled_hold),
            recovery_to_next_token_ns: duration_ns(recovery),
            output_backpressure_observed: output.output_backpressure_observed,
        },
        validation,
    })
}

struct CancellationResult {
    measurement: CancellationMeasurement,
    validation: GenerationValidation,
}

fn measure_cancellation(
    harness: &mut E0Harness,
    handle: ModelHandle,
) -> BenchmarkResult<CancellationResult> {
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
    let validation = validate_generation(
        &output,
        output.generated_tokens,
        expected,
        "finished:cancelled:user-requested",
    )?;
    if output.generated_tokens == 0 || output.generated_tokens >= CANCELLATION_GENERATED_TOKEN_LIMIT
    {
        return Err(BenchmarkError::new(
            "controlled cancellation did not stop after a non-zero partial generation window",
        ));
    }
    Ok(CancellationResult {
        measurement: CancellationMeasurement {
            acknowledgement_ns: duration_ns(observations.acknowledgement),
            terminal_ns: duration_ns(observations.terminal),
            released_ns: duration_ns(observations.released),
        },
        validation,
    })
}

fn setup_cancellation_generation(
    harness: &mut E0Harness,
    handle: ModelHandle,
) -> BenchmarkResult<(RequestId, OutputObservation)> {
    let request = generation_request(40, 140, CANCELLATION_GENERATED_TOKEN_LIMIT)?;
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

struct CancellationObservations {
    acknowledgement: Duration,
    terminal: Duration,
    released: Duration,
}

fn observe_cancellation(
    harness: &E0Harness,
    request_id: RequestId,
    cancellation_ticket: CommandTicket,
    started: Instant,
    deadline: Instant,
    expected: GenerationOutcome,
    output: &mut OutputObservation,
) -> BenchmarkResult<CancellationObservations> {
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
    Ok(CancellationObservations {
        acknowledgement: required_duration(acknowledgement, "cancellation acknowledgement")?,
        terminal: required_duration(terminal, "cancellation Terminal")?,
        released: required_duration(released, "cancellation Released")?,
    })
}

fn poll_cancellation_acknowledgement(
    harness: &E0Harness,
    request_id: RequestId,
    cancellation_ticket: CommandTicket,
    started: Instant,
    acknowledgement: &mut Option<Duration>,
) -> BenchmarkResult {
    match harness.runtime.try_receive() {
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
    harness: &E0Harness,
    request_id: RequestId,
    output: &mut OutputObservation,
) -> BenchmarkResult {
    harness
        .runtime
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
                            .ok_or_else(|| BenchmarkError::new("generated token count overflowed"))?;
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
        .map_err(|error| BenchmarkError::new(format!("public token output pull failed: {error:?}")))??;
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
    harness: &E0Harness,
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
    label: &'static str,
) -> BenchmarkResult<GenerationValidation> {
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
    Ok(GenerationValidation {
        generated_token_count: output.generated_tokens,
        terminal: label,
        released: label,
        cleanup_pending_observed: false,
        cleanup_exhausted_observed: false,
    })
}

fn unload_model(harness: &mut E0Harness, handle: ModelHandle) -> BenchmarkResult<u64> {
    let ticket = harness.ticket()?;
    let command = RuntimeCommand::UnloadModel {
        ticket,
        handle,
        policy: UnloadPolicy::RejectIfBusy,
    };
    let started = Instant::now();
    harness.submit(command, "model unload")?;
    let event = harness.receive(ticket, "model unload")?;
    let elapsed = started.elapsed();
    match event {
        RuntimeEvent::ModelUnload {
            result: Ok(receipt),
            ..
        } if receipt.handle == handle
            && receipt.status == UnloadStatus::Unloaded
            && receipt.cancelled_requests == 0 =>
        {
            Ok(duration_ns(elapsed))
        }
        RuntimeEvent::ModelUnload {
            result: Err(error), ..
        } => Err(BenchmarkError::new(format!(
            "model unload failed: {error:?}"
        ))),
        _ => Err(BenchmarkError::new(
            "model unload returned unexpected status or accounting",
        )),
    }
}

fn generation_request(
    request: u64,
    sequence: u64,
    maximum_generated_tokens: u32,
) -> BenchmarkResult<GenerationRequest> {
    let maximum_generated_tokens = NonZeroU32::new(maximum_generated_tokens)
        .ok_or_else(|| BenchmarkError::new("generation token count must be non-zero"))?;
    Ok(GenerationRequest {
        request_id: RequestId::new(request),
        sequence_id: SequenceId::new(sequence),
        prompt_tokens: vec![TokenId::new(1), TokenId::new(2)].into_boxed_slice(),
        sequence: sequence_configuration(CONTEXT_CAPACITY, GENERATION_PROMPT_TOKENS)?,
        maximum_generated_tokens,
        sampling: SamplingConfig::greedy(),
        seed: 17,
        eos_tokens: Box::new([]),
        stop_sequences: Box::new([]),
        scheduler_quantum: NonZeroU32::MIN,
        output_capacity: GenerationOutputCapacityPolicy::new(NonZeroUsize::MIN, NonZeroUsize::MIN),
    })
}

fn sequence_configuration(
    maximum_tokens: u32,
    maximum_prefill_batch: u32,
) -> BenchmarkResult<SequenceConfiguration> {
    Ok(SequenceConfiguration::new(
        NonZeroU32::new(maximum_tokens)
            .ok_or_else(|| BenchmarkError::new("sequence capacity must be non-zero"))?,
        NonZeroU32::new(maximum_prefill_batch)
            .ok_or_else(|| BenchmarkError::new("prefill capacity must be non-zero"))?,
    ))
}

fn validate_empty_snapshot(snapshot: &CapturedSnapshot, checkpoint: &str) -> BenchmarkResult {
    let runtime = snapshot.raw;
    if runtime.loaded_models != 0
        || runtime.active_requests != 0
        || runtime.reserved_footprint != MemoryFootprint::default()
        || runtime.generation_workspaces != 0
        || runtime.reserved_generation_workspace != MemoryFootprint::default()
        || runtime.pending_cleanup_models != 0
        || runtime.pending_cleanup_sequences != 0
        || runtime.exhausted_cleanup_models != 0
        || runtime.exhausted_cleanup_sequences != 0
        || runtime.last_cleanup.is_some()
        || runtime.maintenance_error.is_some()
        || runtime.shutting_down
        || !snapshot.models.is_empty()
    {
        return Err(BenchmarkError::new(format!(
            "{checkpoint} did not have exact empty E0 accounting"
        )));
    }
    Ok(())
}

fn validate_loaded_idle_snapshot(
    snapshot: &CapturedSnapshot,
    handle: ModelHandle,
    expected_footprint: MemoryFootprint,
    checkpoint: &str,
) -> BenchmarkResult {
    let runtime = snapshot.raw;
    let model = only_model(snapshot, checkpoint)?;
    if runtime.loaded_models != 1
        || runtime.active_requests != 0
        || runtime.reserved_footprint != expected_footprint
        || runtime.generation_workspaces != 0
        || runtime.reserved_generation_workspace != MemoryFootprint::default()
        || runtime.last_cleanup.is_some()
        || runtime.shutting_down
        || model.handle != handle
        || model.lifecycle != ModelLifecycleState::Ready
        || model.reserved_footprint != expected_footprint
        || model.active_requests != 0
        || model.pending_cleanup_sequences != 0
        || model.exhausted_cleanup_sequences != 0
        || model.degraded
    {
        return Err(BenchmarkError::new(format!(
            "{checkpoint} did not have exact loaded-idle E0 accounting"
        )));
    }
    Ok(())
}

fn validate_active_snapshot(
    snapshot: &CapturedSnapshot,
    handle: ModelHandle,
    loaded_footprint: MemoryFootprint,
    request_footprint: MemoryFootprint,
    checkpoint: &str,
) -> BenchmarkResult {
    let runtime = snapshot.raw;
    let model = only_model(snapshot, checkpoint)?;
    let expected_footprint =
        checked_add_footprints(loaded_footprint, request_footprint, checkpoint)?;
    let generation_workspace = runtime.reserved_generation_workspace;
    let exact_generation_workspace = runtime.generation_workspaces == 1
        && generation_workspace.host_weight_bytes == 0
        && generation_workspace.device_weight_bytes == 0
        && generation_workspace.host_working_bytes != 0
        && generation_workspace.device_working_bytes == 0
        && generation_workspace.cache_bytes_per_token == 0
        && footprint_contains(request_footprint, generation_workspace);
    if runtime.loaded_models != 1
        || runtime.active_requests != 1
        || runtime.reserved_footprint != expected_footprint
        || !exact_generation_workspace
        || runtime.last_cleanup.is_some()
        || runtime.shutting_down
        || model.handle != handle
        || !matches!(
            model.lifecycle,
            ModelLifecycleState::Active { active_requests: 1 }
        )
        || model.reserved_footprint != expected_footprint
        || model.active_requests != 1
        || model.pending_cleanup_sequences != 0
        || model.exhausted_cleanup_sequences != 0
        || model.degraded
    {
        return Err(BenchmarkError::new(format!(
            "{checkpoint} did not have exact active-request, footprint, lifecycle, and generation-workspace accounting"
        )));
    }
    Ok(())
}

fn checked_add_footprints(
    left: MemoryFootprint,
    right: MemoryFootprint,
    checkpoint: &str,
) -> BenchmarkResult<MemoryFootprint> {
    Ok(MemoryFootprint {
        host_weight_bytes: left
            .host_weight_bytes
            .checked_add(right.host_weight_bytes)
            .ok_or_else(|| footprint_overflow(checkpoint))?,
        device_weight_bytes: left
            .device_weight_bytes
            .checked_add(right.device_weight_bytes)
            .ok_or_else(|| footprint_overflow(checkpoint))?,
        host_working_bytes: left
            .host_working_bytes
            .checked_add(right.host_working_bytes)
            .ok_or_else(|| footprint_overflow(checkpoint))?,
        device_working_bytes: left
            .device_working_bytes
            .checked_add(right.device_working_bytes)
            .ok_or_else(|| footprint_overflow(checkpoint))?,
        cache_bytes_per_token: left
            .cache_bytes_per_token
            .checked_add(right.cache_bytes_per_token)
            .ok_or_else(|| footprint_overflow(checkpoint))?,
    })
}

fn footprint_overflow(checkpoint: &str) -> BenchmarkError {
    BenchmarkError::new(format!("{checkpoint} active footprint addition overflowed"))
}

const fn footprint_contains(available: MemoryFootprint, required: MemoryFootprint) -> bool {
    available.host_weight_bytes >= required.host_weight_bytes
        && available.device_weight_bytes >= required.device_weight_bytes
        && available.host_working_bytes >= required.host_working_bytes
        && available.device_working_bytes >= required.device_working_bytes
        && available.cache_bytes_per_token >= required.cache_bytes_per_token
}

fn only_model<'a>(
    snapshot: &'a CapturedSnapshot,
    checkpoint: &str,
) -> BenchmarkResult<&'a inference_runtime::ModelSnapshot> {
    if snapshot.models.len() != 1 {
        return Err(BenchmarkError::new(format!(
            "{checkpoint} expected exactly one model snapshot"
        )));
    }
    snapshot
        .models
        .first()
        .ok_or_else(|| BenchmarkError::new(format!("{checkpoint} model snapshot disappeared")))
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
