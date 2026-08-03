//! Shared fixture load, direct request, prefill/decode, completion, and unload logic.

use std::num::NonZeroU32;
use std::time::Duration;

use domain_contracts::{
    CapabilitySet, DecodeOutcome, DeviceId, DeviceKind, ExecutionDevice, FinishReason,
    MemoryFootprint, ModelArchitecture, PrefillOutcome, QuantizationFormat, RequestId, ScalarType,
    SequenceConfiguration, TokenId, UnloadPolicy,
};
use inference_runtime::{LoadReceipt, RuntimeCommand, RuntimeEvent, UnloadStatus};

use super::harness::{CANDLE_BACKEND, FIXTURE_MODEL_ID, HostedE0Harness, TimedEvent};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::fixture::{CONTEXT_CAPACITY, VOCABULARY_SIZE, VerifiedFixture};

const CPU_DEVICE: DeviceId = DeviceId::new(0);
pub(crate) const CHECKED_PREFILL_TOKEN_COUNT: u32 = 4;
pub(crate) const GENERATION_PROMPT_TOKEN_COUNT: u32 = 2;
/// Reusable logits length required by the deterministic Criterion fixture.
#[doc(hidden)]
pub const CRITERION_VOCABULARY_SIZE: usize = 16;

pub(super) fn load_fixture(
    harness: &mut HostedE0Harness,
    fixture: &VerifiedFixture,
) -> BenchmarkResult<(LoadReceipt, Duration)> {
    let source = fixture.source()?;
    let ticket = harness.ticket()?;
    let command = RuntimeCommand::LoadModel {
        ticket,
        model_id: FIXTURE_MODEL_ID,
        source,
        execution_device: ExecutionDevice::new(CPU_DEVICE, DeviceKind::Cpu),
    };
    let TimedEvent { event, elapsed } =
        harness.timed_exchange(ticket, command, "fixture model load")?;
    let loaded = loaded_receipt(&event)?;
    validate_loaded_fixture(&loaded)?;
    harness.record_loaded_model(loaded.handle)?;
    Ok((loaded, elapsed))
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
            "fixture model load command returned an unexpected event",
        )),
    }
}

fn validate_loaded_fixture(loaded: &LoadReceipt) -> BenchmarkResult {
    let descriptor = loaded.descriptor;
    let required = CapabilitySet::PREFILL.union(CapabilitySet::INCREMENTAL_DECODE);
    if loaded.handle.id != FIXTURE_MODEL_ID
        || descriptor.backend != CANDLE_BACKEND
        || descriptor.metadata.architecture != ModelArchitecture::Llama
        || descriptor.metadata.scalar_type != ScalarType::F32
        || descriptor.metadata.quantization != QuantizationFormat::None
        || descriptor.metadata.vocabulary_size != VOCABULARY_SIZE
        || descriptor.metadata.context_length != CONTEXT_CAPACITY
        || !descriptor.capabilities.operations.contains(required)
        || descriptor.capabilities.maximum_context_tokens < CONTEXT_CAPACITY
        || descriptor.capabilities.maximum_prefill_batch < CHECKED_PREFILL_TOKEN_COUNT
        || descriptor.capabilities.maximum_sequences < 1
        || loaded.reserved_footprint == MemoryFootprint::default()
    {
        return Err(BenchmarkError::new(
            "loaded public E0 descriptor does not match the reviewed Candle/Llama/Safetensors/F32/vocab16/context16 fixture",
        ));
    }
    Ok(())
}

/// Starts one loaded fixture harness for a Criterion target.
///
/// Fixture verification, worker start, and model load all occur before Criterion
/// measurement. A setup failure after worker start still performs bounded cleanup.
#[doc(hidden)]
pub fn criterion_harness() -> BenchmarkResult<HostedE0Harness> {
    let fixture = VerifiedFixture::verify()?;
    let (mut harness, _) = HostedE0Harness::start(16, 64)?;
    match load_fixture(&mut harness, &fixture) {
        Ok(_) => Ok(harness),
        Err(error) => match harness.finish::<()>(Err(error)) {
            Err(error) => Err(error),
            Ok(_) => Err(BenchmarkError::new(
                "failed Criterion setup unexpectedly finalized as success",
            )),
        },
    }
}

/// Runs one checked four-token prefill iteration and returns only the named duration.
///
/// Request setup, prompt construction, event validation, request completion, and
/// reusable-logits restoration are outside the returned duration.
#[doc(hidden)]
pub fn criterion_checked_prefill_iteration(
    harness: &mut HostedE0Harness,
    logits: &mut Vec<f32>,
) -> BenchmarkResult<Duration> {
    checked_prefill_iteration(harness, logits)
}

pub(super) fn checked_prefill_iteration(
    harness: &mut HostedE0Harness,
    logits: &mut Vec<f32>,
) -> BenchmarkResult<Duration> {
    let request = start_request(
        harness,
        CONTEXT_CAPACITY,
        CHECKED_PREFILL_TOKEN_COUNT,
        "checked-prefill request setup",
    )?;
    require_logits_length(logits, request.logits_capacity, "checked prefill")?;
    let prompt = vec![
        TokenId::new(1),
        TokenId::new(2),
        TokenId::new(3),
        TokenId::new(4),
    ]
    .into_boxed_slice();
    let (returned, elapsed) = prefill_round_trip(
        harness,
        request.request_id,
        prompt,
        std::mem::take(logits),
        CHECKED_PREFILL_TOKEN_COUNT,
        request.logits_capacity,
        "checked prompt prefill",
    )?;
    *logits = returned;
    complete_request(
        harness,
        request.request_id,
        "checked-prefill request completion",
    )?;
    Ok(elapsed)
}

/// Runs one decode after an untimed two-token setup prefill.
///
/// Only decode command submission through its matching completion event is
/// returned. All setup, validation, completion, and logits ownership movement
/// remain outside that duration.
#[doc(hidden)]
pub fn criterion_incremental_decode_iteration(
    harness: &mut HostedE0Harness,
    logits: &mut Vec<f32>,
) -> BenchmarkResult<Duration> {
    let request = start_request(
        harness,
        CONTEXT_CAPACITY,
        CHECKED_PREFILL_TOKEN_COUNT,
        "decode request setup",
    )?;
    require_logits_length(logits, request.logits_capacity, "decode")?;
    let (returned, _) = prefill_round_trip(
        harness,
        request.request_id,
        vec![TokenId::new(1), TokenId::new(2)].into_boxed_slice(),
        std::mem::take(logits),
        GENERATION_PROMPT_TOKEN_COUNT,
        request.logits_capacity,
        "untimed decode setup prefill",
    )?;
    *logits = returned;

    let ticket = harness.ticket()?;
    let command = RuntimeCommand::Decode {
        ticket,
        request_id: request.request_id,
        token: TokenId::new(2),
        logits: std::mem::take(logits),
    };
    let TimedEvent { event, elapsed } =
        harness.timed_exchange(ticket, command, "timed incremental decode")?;
    *logits = validate_decode_event(
        event,
        request.request_id,
        request.logits_capacity,
        "timed incremental decode",
    )?;
    complete_request(harness, request.request_id, "decode request completion")?;
    Ok(elapsed)
}

struct StartedRequest {
    request_id: RequestId,
    logits_capacity: usize,
}

fn start_request(
    harness: &mut HostedE0Harness,
    maximum_tokens: u32,
    maximum_prefill_batch: u32,
    operation: &str,
) -> BenchmarkResult<StartedRequest> {
    let handle = harness.loaded_model()?;
    let (request_id, sequence_id) = harness.request_identity()?;
    let ticket = harness.ticket()?;
    harness.submit(
        RuntimeCommand::StartRequest {
            ticket,
            handle,
            request_id,
            sequence_id,
            configuration: sequence_configuration(maximum_tokens, maximum_prefill_batch)?,
        },
        operation,
    )?;
    match harness.receive(ticket, operation)? {
        RuntimeEvent::RequestStarted {
            result: Ok(receipt),
            ..
        } if receipt.request_id == request_id
            && receipt.sequence_id == sequence_id
            && receipt.logits_capacity == usize_from_u32(VOCABULARY_SIZE)? =>
        {
            Ok(StartedRequest {
                request_id,
                logits_capacity: receipt.logits_capacity,
            })
        }
        RuntimeEvent::RequestStarted {
            result: Err(error), ..
        } => Err(BenchmarkError::new(format!(
            "{operation} failed: {error:?}"
        ))),
        _ => Err(BenchmarkError::new(format!(
            "{operation} returned an unexpected identity, logits capacity, or event"
        ))),
    }
}

fn prefill_round_trip(
    harness: &mut HostedE0Harness,
    request_id: RequestId,
    tokens: Box<[TokenId]>,
    logits: Vec<f32>,
    expected_prompt_tokens: u32,
    expected_logits: usize,
    operation: &str,
) -> BenchmarkResult<(Vec<f32>, Duration)> {
    let ticket = harness.ticket()?;
    let command = RuntimeCommand::Prefill {
        ticket,
        request_id,
        tokens,
        emit_logits: true,
        logits,
    };
    let TimedEvent { event, elapsed } = harness.timed_exchange(ticket, command, operation)?;
    let logits = validate_prefill_event(
        event,
        request_id,
        expected_prompt_tokens,
        expected_logits,
        operation,
    )?;
    Ok((logits, elapsed))
}

fn validate_prefill_event(
    event: RuntimeEvent,
    expected_request: RequestId,
    expected_prompt_tokens: u32,
    expected_logits: usize,
    operation: &str,
) -> BenchmarkResult<Vec<f32>> {
    let expected_tokens = usize_from_u32(expected_prompt_tokens)?;
    match event {
        RuntimeEvent::PrefillCompleted {
            request_id,
            result: Ok(receipt),
            logits,
            ..
        } if request_id == expected_request && logits.len() == expected_logits => {
            match receipt.outcome {
                PrefillOutcome::Ready {
                    consumed_tokens,
                    position,
                    logits_written,
                } if consumed_tokens == expected_tokens
                    && position == expected_tokens
                    && logits_written == expected_logits
                    && receipt.usage.prompt_tokens == u64::from(expected_prompt_tokens)
                    && receipt.usage.generated_tokens == 0 =>
                {
                    Ok(logits)
                }
                _ => Err(BenchmarkError::new(format!(
                    "{operation} returned unexpected outcome or usage"
                ))),
            }
        }
        RuntimeEvent::PrefillCompleted {
            result: Err(error), ..
        } => Err(BenchmarkError::new(format!(
            "{operation} failed: {error:?}"
        ))),
        _ => Err(BenchmarkError::new(format!(
            "{operation} returned an unexpected event, request, or logits length"
        ))),
    }
}

fn validate_decode_event(
    event: RuntimeEvent,
    expected_request: RequestId,
    expected_logits: usize,
    operation: &str,
) -> BenchmarkResult<Vec<f32>> {
    match event {
        RuntimeEvent::DecodeCompleted {
            request_id,
            result: Ok(receipt),
            logits,
            ..
        } if request_id == expected_request && logits.len() == expected_logits => {
            match receipt.outcome {
                DecodeOutcome::Ready {
                    position,
                    logits_written,
                } if position == 3
                    && logits_written == expected_logits
                    && receipt.usage.prompt_tokens == u64::from(GENERATION_PROMPT_TOKEN_COUNT)
                    && receipt.usage.generated_tokens == 1 =>
                {
                    Ok(logits)
                }
                _ => Err(BenchmarkError::new(format!(
                    "{operation} returned unexpected outcome or usage"
                ))),
            }
        }
        RuntimeEvent::DecodeCompleted {
            result: Err(error), ..
        } => Err(BenchmarkError::new(format!(
            "{operation} failed: {error:?}"
        ))),
        _ => Err(BenchmarkError::new(format!(
            "{operation} returned an unexpected event, request, or logits length"
        ))),
    }
}

fn complete_request(
    harness: &mut HostedE0Harness,
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

pub(super) fn unload_loaded_model(harness: &mut HostedE0Harness) -> BenchmarkResult<Duration> {
    let handle = harness.loaded_model()?;
    let ticket = harness.ticket()?;
    let command = RuntimeCommand::UnloadModel {
        ticket,
        handle,
        policy: UnloadPolicy::RejectIfBusy,
    };
    let TimedEvent { event, elapsed } = harness.timed_exchange(ticket, command, "model unload")?;
    match event {
        RuntimeEvent::ModelUnload {
            result: Ok(receipt),
            ..
        } if receipt.handle == handle
            && receipt.status == UnloadStatus::Unloaded
            && receipt.cancelled_requests == 0 =>
        {
            harness.record_unloaded_model(handle)?;
            Ok(elapsed)
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

pub(super) fn sequence_configuration(
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

fn require_logits_length(logits: &[f32], expected: usize, operation: &str) -> BenchmarkResult {
    if logits.len() != expected {
        return Err(BenchmarkError::new(format!(
            "{operation} reusable logits length changed from {expected} to {}",
            logits.len()
        )));
    }
    Ok(())
}

fn usize_from_u32(value: u32) -> BenchmarkResult<usize> {
    usize::try_from(value)
        .map_err(|_| BenchmarkError::new("u32-to-usize capacity conversion failed"))
}
