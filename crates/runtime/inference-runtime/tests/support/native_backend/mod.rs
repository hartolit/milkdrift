//! Download-free Candle real-fixture coverage for E0 generation and lifecycle.

pub(crate) use std::collections::HashMap;
pub(crate) use std::fs;
pub(crate) use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) use candle_backend::{CandleLlamaLoader, CandleLlamaSource};
pub(crate) use candle_core::{DType, Device, Tensor};
pub(crate) use domain_contracts::{
    BackendId, ByteCount, CancellationReason, CapabilitySet, DeviceId, DeviceKind, ExecutionDevice,
    FinishReason, LoadConfiguration, LoadPlan, MemoryBudget, MemoryFootprint, ModelArchitecture,
    ModelCapabilities, ModelDescriptor, ModelGeneration, ModelHandle, ModelId, ModelLoader,
    ModelMetadata, PreparedLoad, QuantizationFormat, RequestId, ScalarType, ScalarTypeSet,
    SequenceConfiguration, SequenceId, TokenId, UnloadPolicy, YieldReason,
};

pub(crate) use host_runtime::TokenOutputRecordKind;
pub(crate) use inference_runtime::{
    CommandTicket, GenerationOutcome, GenerationOutputCapacityPolicy, GenerationOutputState,
    GenerationRequest, HostedRuntime, HostedRuntimeConfiguration, LoadReceipt, RuntimeCommand,
    RuntimeEvent, RuntimeLimits, RuntimeThread, UnloadStatus, start_hosted_runtime,
};
pub(crate) use sampling::SamplingConfig;

mod fixture;
mod generation;
mod host;
mod lifecycle;
mod plan;

#[cfg(feature = "cuda-hardware-tests")]
mod cuda;

#[cfg(feature = "cuda-hardware-tests")]
pub(crate) use cuda::candle_mixed_cuda_fixture_covers_e0_generation_accounting_and_lifecycle;

pub(crate) use fixture::*;
pub(crate) use generation::*;
pub(crate) use host::*;
pub(crate) use lifecycle::*;
pub(crate) use plan::*;

pub(crate) type TestResult<T = ()> = Result<T, String>;
pub(crate) type CandleRuntime = HostedRuntime<CandleLlamaSource>;

pub(crate) fn candle_fixture_covers_generation_sampling_eos_and_lifecycle() -> TestResult {
    let source = candle_fixture_source()?;
    let execution_device = CPU_EXECUTION_DEVICE;
    let plan = prepare_plan(&source, execution_device)?;
    assert_homogeneous_f32_plan(&plan, execution_device);

    let (hosted, thread) = hosted_runtime(execution_device, 16, 64)?;
    let loaded = load_model(&hosted, source, execution_device)?;
    assert_receipt_matches_plan(&loaded, &plan);
    let handle = loaded.handle;

    // Three generated tokens require one prompt prefill and two incremental
    // decode calls inside RuntimeCommand::Generate.
    let greedy = generation_request(1, 101, 3, SamplingConfig::greedy(), 17, Box::new([]))?;
    submit_generation(&hosted, handle, CommandTicket::new(10), &greedy)?;
    let greedy_output = collect_until_released(
        &hosted,
        greedy.request_id,
        OUTPUT_TIMEOUT,
        CollectedOutput::default(),
    )?;
    assert_eq!(greedy_output.tokens, vec![EXPECTED_GREEDY_TOKEN; 3]);
    assert_finished(&greedy_output, FinishReason::TokenLimit);
    assert_released_snapshot(&hosted, &loaded, CommandTicket::new(20))?;

    let sampling = stochastic_sampling()?;
    let first_seeded = generation_request(2, 102, 5, sampling, 0x5eed, Box::new([]))?;
    submit_generation(&hosted, handle, CommandTicket::new(11), &first_seeded)?;
    let first_seeded_output = collect_until_released(
        &hosted,
        first_seeded.request_id,
        OUTPUT_TIMEOUT,
        CollectedOutput::default(),
    )?;
    assert_eq!(first_seeded_output.tokens.len(), 5);
    assert!(
        first_seeded_output
            .tokens
            .iter()
            .all(|token| token.get() < VOCABULARY_SIZE)
    );
    assert_finished(&first_seeded_output, FinishReason::TokenLimit);
    assert_released_snapshot(&hosted, &loaded, CommandTicket::new(21))?;

    let second_seeded = generation_request(3, 103, 5, sampling, 0x5eed, Box::new([]))?;
    submit_generation(&hosted, handle, CommandTicket::new(12), &second_seeded)?;
    let second_seeded_output = collect_until_released(
        &hosted,
        second_seeded.request_id,
        OUTPUT_TIMEOUT,
        CollectedOutput::default(),
    )?;
    assert_eq!(second_seeded_output.tokens.len(), 5);
    assert_eq!(first_seeded_output.tokens, second_seeded_output.tokens);
    assert_finished(&second_seeded_output, FinishReason::TokenLimit);
    assert_released_snapshot(&hosted, &loaded, CommandTicket::new(22))?;

    let eos = generation_request(
        4,
        104,
        3,
        SamplingConfig::greedy(),
        19,
        Box::new([EXPECTED_GREEDY_TOKEN]),
    )?;
    submit_generation(&hosted, handle, CommandTicket::new(13), &eos)?;
    let eos_output = collect_until_released(
        &hosted,
        eos.request_id,
        OUTPUT_TIMEOUT,
        CollectedOutput::default(),
    )?;
    assert_eq!(eos_output.tokens, vec![EXPECTED_GREEDY_TOKEN]);
    assert_finished(
        &eos_output,
        FinishReason::EndOfSequence(EXPECTED_GREEDY_TOKEN),
    );
    assert_released_snapshot(&hosted, &loaded, CommandTicket::new(23))?;

    unload_model(&hosted, handle)?;
    shutdown(hosted, thread)
}

pub(crate) fn mixed_f16_f32_fixture_covers_e0_generation_accounting_and_lifecycle() -> TestResult {
    mixed_f16_f32_fixture_covers_generation_accounting_and_lifecycle(CPU_EXECUTION_DEVICE)
}

fn mixed_f16_f32_fixture_covers_generation_accounting_and_lifecycle(
    execution_device: ExecutionDevice,
) -> TestResult {
    let converted = ConvertedFixture::create(DType::F16, true)?;
    let source = mixed_fixture_source(&converted)?;
    let plan = prepare_plan(&source, execution_device)?;
    assert_mixed_f16_plan(&plan, execution_device)?;

    let (hosted, thread) = hosted_runtime(execution_device, 16, 64)?;
    let loaded = load_model(&hosted, source, execution_device)?;
    assert_receipt_matches_plan(&loaded, &plan);
    let handle = loaded.handle;

    let request = generation_request(5, 105, 3, SamplingConfig::greedy(), 37, Box::new([]))?;
    submit_generation(&hosted, handle, CommandTicket::new(14), &request)?;
    let output = collect_until_released(
        &hosted,
        request.request_id,
        OUTPUT_TIMEOUT,
        CollectedOutput::default(),
    )?;
    assert_eq!(output.tokens, vec![EXPECTED_GREEDY_TOKEN; 3]);
    assert_finished(&output, FinishReason::TokenLimit);
    assert_released_snapshot(&hosted, &loaded, CommandTicket::new(24))?;

    unload_model(&hosted, handle)?;
    shutdown(hosted, thread)
}

pub(crate) fn candle_fixture_covers_output_backpressure_and_cancellation() -> TestResult {
    let source = candle_fixture_source()?;
    let execution_device = CPU_EXECUTION_DEVICE;
    let plan = prepare_plan(&source, execution_device)?;
    assert_homogeneous_f32_plan(&plan, execution_device);

    let (hosted, thread) = hosted_runtime(execution_device, 1, 64)?;
    let loaded = load_model(&hosted, source, execution_device)?;
    assert_receipt_matches_plan(&loaded, &plan);
    let handle = loaded.handle;

    let backpressured = generation_request(10, 110, 4, SamplingConfig::greedy(), 23, Box::new([]))?;
    submit_generation(&hosted, handle, CommandTicket::new(30), &backpressured)?;
    let observed = collect_until_backpressure(&hosted, backpressured.request_id, OUTPUT_TIMEOUT)?;
    let backpressured_output =
        collect_until_released(&hosted, backpressured.request_id, OUTPUT_TIMEOUT, observed)?;
    assert_eq!(backpressured_output.tokens, vec![EXPECTED_GREEDY_TOKEN; 4]);
    assert_output_backpressure(&backpressured_output);
    assert_finished(&backpressured_output, FinishReason::TokenLimit);
    assert_released_snapshot(&hosted, &loaded, CommandTicket::new(40))?;

    let cancelled = generation_request(11, 111, 12, SamplingConfig::greedy(), 29, Box::new([]))?;
    submit_generation(&hosted, handle, CommandTicket::new(31), &cancelled)?;
    let observed = collect_until_backpressure(&hosted, cancelled.request_id, OUTPUT_TIMEOUT)?;
    request_cancellation(&hosted, cancelled.request_id, CommandTicket::new(32))?;
    let cancelled_output =
        collect_until_released(&hosted, cancelled.request_id, OUTPUT_TIMEOUT, observed)?;
    assert!(!cancelled_output.tokens.is_empty());
    assert!(cancelled_output.tokens.len() < 12);
    assert!(
        cancelled_output
            .tokens
            .iter()
            .all(|token| *token == EXPECTED_GREEDY_TOKEN)
    );
    assert_output_backpressure(&cancelled_output);
    assert_finished(
        &cancelled_output,
        FinishReason::Cancelled(CancellationReason::UserRequested),
    );
    assert_released_snapshot(&hosted, &loaded, CommandTicket::new(41))?;

    unload_model(&hosted, handle)?;
    shutdown(hosted, thread)
}
