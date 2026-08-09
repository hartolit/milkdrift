//! Download-free Candle real-fixture coverage for E0 generation and lifecycle.

use std::collections::HashMap;
use std::fs;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use candle_backend::{CandleLlamaLoader, CandleLlamaSource};
use candle_core::{DType, Device, Tensor};
use domain_contracts::{
    BackendId, CancellationReason, CapabilitySet, DeviceId, DeviceKind, ExecutionDevice,
    FinishReason, LoadConfiguration, LoadPlan, MemoryBudget, MemoryFootprint, ModelArchitecture,
    ModelGeneration, ModelHandle, ModelId, ModelLoader, PreparedLoad, RequestId, ScalarType,
    ScalarTypeSet, SequenceConfiguration, SequenceId, TokenId, UnloadPolicy, YieldReason,
};

use host_runtime::TokenOutputRecordKind;
use inference_runtime::{
    CommandTicket, GenerationOutcome, GenerationOutputCapacityPolicy, GenerationOutputState,
    GenerationRequest, HostedRuntime, HostedRuntimeConfiguration, LoadReceipt, RuntimeCommand,
    RuntimeEvent, RuntimeLimits, RuntimeThread, UnloadStatus, start_hosted_runtime,
};
use sampling::SamplingConfig;

const CANDLE_BACKEND: BackendId = BackendId::new(41);
const MODEL: ModelId = ModelId::new(7);
const VOCABULARY_SIZE: u32 = 16;
const CONTEXT_LENGTH: u32 = 16;
const EXPECTED_GREEDY_TOKEN: TokenId = TokenId::new(2);
const EVENT_TIMEOUT: Duration = Duration::from_secs(10);
const OUTPUT_TIMEOUT: Duration = Duration::from_secs(10);

const LOAD_TICKET: CommandTicket = CommandTicket::new(1);
const UNLOAD_TICKET: CommandTicket = CommandTicket::new(90);
const UNLOADED_SNAPSHOT_TICKET: CommandTicket = CommandTicket::new(91);
const SHUTDOWN_TICKET: CommandTicket = CommandTicket::new(92);

const REQUIRED_GENERATION_OPERATIONS: CapabilitySet =
    CapabilitySet::PREFILL.union(CapabilitySet::INCREMENTAL_DECODE);
const MIXED_EXECUTION_WEIGHT_BYTES: u64 = 1_840;
const MIXED_CACHE_BYTES_PER_TOKEN: u64 = 32;
const MIXED_CUDA_HOST_LOADING_PEAK_BYTES: u64 = 513;

type TestResult<T = ()> = Result<T, String>;
type CandleRuntime = HostedRuntime<CandleLlamaSource>;

#[test]
fn candle_fixture_covers_generation_sampling_eos_and_lifecycle() -> TestResult {
    let (hosted, thread) = hosted_runtime(16, 64)?;
    let loaded = load_model(&hosted, candle_fixture_source()?)?;
    assert_loaded_fixture(&loaded);
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
    assert_released_snapshot(
        &hosted,
        handle,
        loaded.execution_scalar_type,
        loaded.reserved_footprint,
        CommandTicket::new(20),
    )?;

    let sampling = stochastic_sampling();
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
    assert_released_snapshot(
        &hosted,
        handle,
        loaded.execution_scalar_type,
        loaded.reserved_footprint,
        CommandTicket::new(21),
    )?;

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
    assert_released_snapshot(
        &hosted,
        handle,
        loaded.execution_scalar_type,
        loaded.reserved_footprint,
        CommandTicket::new(22),
    )?;

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
    assert_released_snapshot(
        &hosted,
        handle,
        loaded.execution_scalar_type,
        loaded.reserved_footprint,
        CommandTicket::new(23),
    )?;

    unload_model(&hosted, handle)?;
    shutdown(hosted, thread)
}

#[test]
fn mixed_f16_f32_fixture_covers_e0_generation_accounting_and_lifecycle() -> TestResult {
    let converted = ConvertedFixture::create(DType::F16, true)?;
    let source = mixed_fixture_source(&converted)?;
    let execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
    let plan = prepare_plan(&source, execution_device)?;
    assert_mixed_f16_plan(&plan, execution_device)?;

    let (hosted, thread) = hosted_runtime(16, 64)?;
    let loaded = load_model(&hosted, source)?;
    assert_mixed_f16_receipt(&loaded, &plan, execution_device);
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
    assert_released_snapshot(
        &hosted,
        handle,
        ScalarType::F16,
        plan.expected_footprint,
        CommandTicket::new(24),
    )?;

    unload_model(&hosted, handle)?;
    shutdown(hosted, thread)
}

#[test]
fn candle_fixture_covers_output_backpressure_and_cancellation() -> TestResult {
    let (hosted, thread) = hosted_runtime(1, 64)?;
    let loaded = load_model(&hosted, candle_fixture_source()?)?;
    assert_loaded_fixture(&loaded);
    let handle = loaded.handle;

    let backpressured = generation_request(10, 110, 4, SamplingConfig::greedy(), 23, Box::new([]))?;
    submit_generation(&hosted, handle, CommandTicket::new(30), &backpressured)?;
    let observed = collect_until_backpressure(&hosted, backpressured.request_id, OUTPUT_TIMEOUT)?;
    let backpressured_output =
        collect_until_released(&hosted, backpressured.request_id, OUTPUT_TIMEOUT, observed)?;
    assert_eq!(backpressured_output.tokens, vec![EXPECTED_GREEDY_TOKEN; 4]);
    assert_output_backpressure(&backpressured_output);
    assert_finished(&backpressured_output, FinishReason::TokenLimit);
    assert_released_snapshot(
        &hosted,
        handle,
        loaded.execution_scalar_type,
        loaded.reserved_footprint,
        CommandTicket::new(40),
    )?;

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
    assert_released_snapshot(
        &hosted,
        handle,
        loaded.execution_scalar_type,
        loaded.reserved_footprint,
        CommandTicket::new(41),
    )?;

    unload_model(&hosted, handle)?;
    shutdown(hosted, thread)
}

#[cfg(feature = "cuda")]
#[test]
#[ignore = "requires MILKDRIFT_CUDA_TEST=1 and CUDA ordinal 0"]
fn candle_mixed_cuda_fixture_covers_e0_generation_accounting_and_lifecycle() -> TestResult {
    require_cuda_opt_in()?;
    let converted = ConvertedFixture::create(DType::F16, true)?;
    let source = mixed_fixture_source(&converted)?;
    let execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);
    let plan = prepare_plan(&source, execution_device)?;
    assert_mixed_f16_plan(&plan, execution_device)?;

    let (hosted, thread) = hosted_cuda_runtime()?;
    let loaded = load_cuda_model(&hosted, source)?;
    assert_mixed_f16_receipt(&loaded, &plan, execution_device);
    let handle = loaded.handle;

    let request = generation_request(50, 150, 3, SamplingConfig::greedy(), 31, Box::new([]))?;
    submit_generation(&hosted, handle, CommandTicket::new(50), &request)?;
    let output = collect_until_released(
        &hosted,
        request.request_id,
        OUTPUT_TIMEOUT,
        CollectedOutput::default(),
    )?;
    assert_eq!(output.tokens, vec![EXPECTED_GREEDY_TOKEN; 3]);
    assert_finished(&output, FinishReason::TokenLimit);
    assert_cuda_released_snapshot(&hosted, handle, ScalarType::F16, loaded.reserved_footprint)?;

    unload_model(&hosted, handle)?;
    shutdown(hosted, thread)
}

fn candle_fixture_source() -> TestResult<CandleLlamaSource> {
    let directory = candle_fixture_directory();
    CandleLlamaSource::from_local_files(
        directory.join("config.json"),
        vec![directory.join("model.safetensors")],
    )
    .map_err(|error| error.to_string())
}

fn mixed_fixture_source(fixture: &ConvertedFixture) -> TestResult<CandleLlamaSource> {
    CandleLlamaSource::from_local_files(
        fixture.config_path.clone(),
        vec![fixture.weight_path.clone()],
    )
    .map_err(|error| error.to_string())
}

fn prepare_plan(
    source: &CandleLlamaSource,
    execution_device: ExecutionDevice,
) -> TestResult<LoadPlan> {
    let mut loader = CandleLlamaLoader::new(CANDLE_BACKEND);
    let configuration = LoadConfiguration {
        handle: ModelHandle::new(MODEL, ModelGeneration::new(1)),
        execution_device,
        memory_budget: MemoryBudget {
            host_bytes: u64::MAX,
            device_bytes: u64::MAX,
        },
    };
    let prepared = loader
        .prepare_load(source, &configuration)
        .map_err(|error| format!("prepare mixed fixture: {error:?}"))?;
    let plan = *prepared.plan();
    drop(prepared);
    Ok(plan)
}

fn assert_mixed_f16_plan(plan: &LoadPlan, execution_device: ExecutionDevice) -> TestResult {
    assert_eq!(
        plan.accepted_configuration.execution_device,
        execution_device
    );
    assert_eq!(plan.execution_scalar_type, ScalarType::F16);
    assert_eq!(
        plan.descriptor.metadata.configuration_declared_scalar_type,
        Some(ScalarType::F16)
    );
    assert_eq!(
        plan.descriptor.metadata.observed_tensor_scalar_types,
        ScalarTypeSet::from_scalar(ScalarType::F16)
            .union(ScalarTypeSet::from_scalar(ScalarType::F32))
    );

    let expected_final = match execution_device.kind {
        DeviceKind::Cpu => MemoryFootprint {
            host_weight_bytes: MIXED_EXECUTION_WEIGHT_BYTES,
            device_weight_bytes: 0,
            host_working_bytes: 0,
            device_working_bytes: 0,
            cache_bytes_per_token: MIXED_CACHE_BYTES_PER_TOKEN,
        },
        DeviceKind::Cuda => MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: MIXED_EXECUTION_WEIGHT_BYTES,
            host_working_bytes: 0,
            device_working_bytes: 0,
            cache_bytes_per_token: MIXED_CACHE_BYTES_PER_TOKEN,
        },
        _ => return Err("mixed fixture test selected an unsupported execution device".to_owned()),
    };
    assert_eq!(plan.expected_footprint, expected_final);
    assert_eq!(
        plan.descriptor.estimated_footprint,
        MemoryFootprint {
            host_weight_bytes: MIXED_EXECUTION_WEIGHT_BYTES,
            device_weight_bytes: 0,
            host_working_bytes: 0,
            device_working_bytes: 0,
            cache_bytes_per_token: MIXED_CACHE_BYTES_PER_TOKEN,
        }
    );
    assert_eq!(
        plan.loading_peak_footprint.cache_bytes_per_token,
        MIXED_CACHE_BYTES_PER_TOKEN
    );

    match execution_device.kind {
        DeviceKind::Cpu => {
            assert_eq!(
                plan.loading_peak_footprint.host_weight_bytes,
                MIXED_EXECUTION_WEIGHT_BYTES
            );
            assert_eq!(plan.loading_peak_footprint.device_weight_bytes, 0);
            assert!(plan.loading_peak_footprint.host_working_bytes > 0);
            assert_eq!(plan.loading_peak_footprint.device_working_bytes, 0);
        }
        DeviceKind::Cuda => {
            assert_eq!(plan.loading_peak_footprint.host_weight_bytes, 0);
            assert_eq!(
                plan.loading_peak_footprint.device_weight_bytes,
                MIXED_EXECUTION_WEIGHT_BYTES
            );
            assert_eq!(
                plan.loading_peak_footprint.host_working_bytes,
                MIXED_CUDA_HOST_LOADING_PEAK_BYTES
            );
            assert_eq!(plan.loading_peak_footprint.device_working_bytes, 0);
        }
        _ => return Err("mixed fixture test selected an unsupported execution device".to_owned()),
    }
    Ok(())
}

fn assert_mixed_f16_receipt(
    loaded: &LoadReceipt,
    plan: &LoadPlan,
    execution_device: ExecutionDevice,
) {
    assert_eq!(loaded.execution_device, execution_device);
    assert_eq!(loaded.execution_scalar_type, ScalarType::F16);
    assert_eq!(loaded.descriptor, plan.descriptor);
    assert_eq!(loaded.reserved_footprint, plan.expected_footprint);
}

fn hosted_runtime(
    token_capacity: usize,
    record_capacity: usize,
) -> TestResult<(CandleRuntime, RuntimeThread)> {
    let configuration =
        HostedRuntimeConfiguration::new(nonzero_usize(8)?, nonzero_usize(8)?, NonZeroU64::MIN)
            .with_token_output_capacity(
                nonzero_usize(token_capacity)?,
                nonzero_usize(record_capacity)?,
            );
    start_hosted_runtime(
        CandleLlamaLoader::new(CANDLE_BACKEND),
        RuntimeLimits::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            MemoryBudget {
                host_bytes: u64::MAX,
                device_bytes: 0,
            },
        ),
        configuration,
    )
    .map_err(|error| error.to_string())
}

#[cfg(feature = "cuda")]
fn hosted_cuda_runtime() -> TestResult<(CandleRuntime, RuntimeThread)> {
    let configuration =
        HostedRuntimeConfiguration::new(nonzero_usize(8)?, nonzero_usize(8)?, NonZeroU64::MIN)
            .with_token_output_capacity(nonzero_usize(16)?, nonzero_usize(64)?);
    start_hosted_runtime(
        CandleLlamaLoader::new(CANDLE_BACKEND),
        RuntimeLimits::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            MemoryBudget {
                host_bytes: u64::MAX,
                device_bytes: u64::MAX,
            },
        ),
        configuration,
    )
    .map_err(|error| error.to_string())
}

fn load_model(hosted: &CandleRuntime, source: CandleLlamaSource) -> TestResult<LoadReceipt> {
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: LOAD_TICKET,
            model_id: MODEL,
            source,
            execution_device: ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu),
        })
        .map_err(|error| format!("load command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("load event failed: {error:?}"))?
    {
        RuntimeEvent::ModelLoaded {
            ticket,
            result: Ok(receipt),
        } if ticket == LOAD_TICKET => Ok(receipt),
        RuntimeEvent::ModelLoaded {
            result: Err(error), ..
        } => Err(format!("model load failed: {error:?}")),
        event => Err(format!(
            "unexpected load event for ticket {:?}",
            event.ticket()
        )),
    }
}

#[cfg(feature = "cuda")]
fn load_cuda_model(hosted: &CandleRuntime, source: CandleLlamaSource) -> TestResult<LoadReceipt> {
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: LOAD_TICKET,
            model_id: MODEL,
            source,
            execution_device: ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda),
        })
        .map_err(|error| format!("CUDA load command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("CUDA load event failed: {error:?}"))?
    {
        RuntimeEvent::ModelLoaded {
            ticket,
            result: Ok(receipt),
        } if ticket == LOAD_TICKET => Ok(receipt),
        RuntimeEvent::ModelLoaded {
            result: Err(error), ..
        } => Err(format!("CUDA model load failed: {error:?}")),
        event => Err(format!(
            "unexpected CUDA load event for ticket {:?}",
            event.ticket()
        )),
    }
}

fn assert_loaded_fixture(loaded: &LoadReceipt) {
    assert_eq!(
        loaded.execution_device,
        ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu)
    );
    assert_eq!(loaded.execution_scalar_type, ScalarType::F32);
    let descriptor = loaded.descriptor;
    assert_eq!(descriptor.backend, CANDLE_BACKEND);
    assert_eq!(descriptor.metadata.architecture, ModelArchitecture::Llama);
    assert_eq!(
        descriptor.metadata.configuration_declared_scalar_type,
        Some(ScalarType::F32)
    );
    assert!(
        descriptor
            .metadata
            .observed_tensor_scalar_types
            .contains(ScalarType::F32)
    );
    assert_eq!(descriptor.metadata.vocabulary_size, VOCABULARY_SIZE);
    assert_eq!(descriptor.metadata.context_length, CONTEXT_LENGTH);
    assert!(
        descriptor
            .capabilities
            .operations
            .contains(REQUIRED_GENERATION_OPERATIONS)
    );
    assert!(descriptor.capabilities.maximum_context_tokens >= CONTEXT_LENGTH);
    assert!(descriptor.capabilities.maximum_prefill_batch >= 2);
    assert!(descriptor.capabilities.maximum_sequences >= 1);
    assert_ne!(loaded.reserved_footprint, MemoryFootprint::default());
}

fn submit_generation(
    hosted: &CandleRuntime,
    handle: ModelHandle,
    ticket: CommandTicket,
    request: &GenerationRequest,
) -> TestResult {
    let request_id = request.request_id;
    let sequence_id = request.sequence_id;
    let scheduler_quantum = request.scheduler_quantum;
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket,
            handle,
            request: request.clone(),
        })
        .map_err(|error| format!("generation command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("generation admission event failed: {error:?}"))?
    {
        RuntimeEvent::GenerationAdmitted {
            ticket: event_ticket,
            result: Ok(admission),
        } if event_ticket == ticket => {
            assert_eq!(admission.request.request_id, request_id);
            assert_eq!(admission.request.sequence_id, sequence_id);
            assert_eq!(admission.request.logits_capacity, VOCABULARY_SIZE as usize);
            assert_eq!(admission.scheduler_quantum, scheduler_quantum);
            Ok(())
        }
        RuntimeEvent::GenerationAdmitted {
            result: Err(error), ..
        } => Err(format!("generation admission failed: {error:?}")),
        event => Err(format!(
            "unexpected generation event for ticket {:?}",
            event.ticket()
        )),
    }
}

fn request_cancellation(
    hosted: &CandleRuntime,
    request_id: RequestId,
    ticket: CommandTicket,
) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::CancelRequest {
            ticket,
            request_id,
            reason: CancellationReason::UserRequested,
        })
        .map_err(|error| format!("cancellation command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("cancellation event failed: {error:?}"))?
    {
        RuntimeEvent::GenerationCancellationRequested {
            ticket: event_ticket,
            request_id: event_request,
            result: Ok(()),
        } if event_ticket == ticket && event_request == request_id => Ok(()),
        RuntimeEvent::GenerationCancellationRequested {
            result: Err(error), ..
        } => Err(format!("generation cancellation failed: {error:?}")),
        event => Err(format!(
            "unexpected cancellation event for ticket {:?}",
            event.ticket()
        )),
    }
}

fn generation_request(
    request: u64,
    sequence: u64,
    maximum_generated_tokens: u32,
    sampling: SamplingConfig,
    seed: u64,
    eos_tokens: Box<[TokenId]>,
) -> TestResult<GenerationRequest> {
    Ok(GenerationRequest {
        request_id: RequestId::new(request),
        sequence_id: SequenceId::new(sequence),
        prompt_tokens: vec![TokenId::new(1), EXPECTED_GREEDY_TOKEN].into_boxed_slice(),
        sequence: SequenceConfiguration::new(nonzero_u32(CONTEXT_LENGTH)?, nonzero_u32(8)?),
        maximum_generated_tokens: nonzero_u32(maximum_generated_tokens)?,
        sampling,
        seed,
        eos_tokens,
        stop_sequences: Box::new([]),
        scheduler_quantum: NonZeroU32::MIN,
        output_capacity: GenerationOutputCapacityPolicy::new(NonZeroUsize::MIN, NonZeroUsize::MIN),
    })
}

const fn stochastic_sampling() -> SamplingConfig {
    SamplingConfig {
        temperature: 8.0,
        top_k: 0,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        repetition_window: 0,
    }
}

#[derive(Default)]
struct CollectedOutput {
    tokens: Vec<TokenId>,
    states: Vec<GenerationOutputState>,
}

fn pull_output(
    hosted: &CandleRuntime,
    request_id: RequestId,
    output: &mut CollectedOutput,
) -> TestResult {
    hosted
        .pull_token_output(|batch| {
            for record in batch.records {
                if record.request_id != request_id {
                    continue;
                }
                match record.kind {
                    TokenOutputRecordKind::Tokens(range) => {
                        if let Some(tokens) = batch.tokens_for(range) {
                            output.tokens.extend_from_slice(tokens);
                        }
                    }
                    TokenOutputRecordKind::State(state) => output.states.push(state),
                }
            }
        })
        .map_err(|error| format!("output pull failed: {error:?}"))
}

fn collect_until_backpressure(
    hosted: &CandleRuntime,
    request_id: RequestId,
    timeout: Duration,
) -> TestResult<CollectedOutput> {
    let deadline = deadline(timeout)?;
    let mut output = CollectedOutput::default();
    loop {
        // Leave the one-token accumulator full long enough for the scheduler to
        // attempt the next publication and emit an observable yield record.
        std::thread::sleep(Duration::from_millis(100));
        pull_output(hosted, request_id, &mut output)?;
        if has_output_backpressure(&output) {
            return Ok(output);
        }
        if output
            .states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
        {
            return Err("request released before output backpressure was observed".into());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "output backpressure timed out after {} tokens and states {:?}",
                output.tokens.len(),
                output.states
            ));
        }
    }
}

fn collect_until_released(
    hosted: &CandleRuntime,
    request_id: RequestId,
    timeout: Duration,
    mut output: CollectedOutput,
) -> TestResult<CollectedOutput> {
    let deadline = deadline(timeout)?;
    loop {
        pull_output(hosted, request_id, &mut output)?;
        if output
            .states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
        {
            return Ok(output);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "generation release timed out after {} tokens and states {:?}",
                output.tokens.len(),
                output.states
            ));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn assert_finished(output: &CollectedOutput, reason: FinishReason) {
    let outcome = GenerationOutcome::Finished(reason);
    assert!(
        output
            .states
            .contains(&GenerationOutputState::Terminal(outcome))
    );
    assert!(
        output
            .states
            .contains(&GenerationOutputState::Released(outcome))
    );
    assert!(!output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::CleanupPending { .. }
            | GenerationOutputState::CleanupExhausted { .. }
    )));
}

fn has_output_backpressure(output: &CollectedOutput) -> bool {
    output.states.iter().any(|state| {
        matches!(
            state,
            GenerationOutputState::Yielded(YieldReason::OutputBackpressure(_))
        )
    })
}

fn assert_output_backpressure(output: &CollectedOutput) {
    assert!(
        has_output_backpressure(output),
        "shared output accumulator never reported an explicit backpressure yield"
    );
}

fn assert_released_snapshot(
    hosted: &CandleRuntime,
    handle: ModelHandle,
    execution_scalar_type: ScalarType,
    model_footprint: MemoryFootprint,
    ticket: CommandTicket,
) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Snapshot { ticket })
        .map_err(|error| format!("snapshot command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            ticket: event_ticket,
            runtime,
            models,
        } if event_ticket == ticket => {
            assert_eq!(runtime.loaded_models, 1);
            assert_eq!(runtime.active_requests, 0);
            assert_eq!(runtime.reserved_footprint, model_footprint);
            assert_eq!(runtime.generation_workspaces, 0);
            assert_eq!(
                runtime.reserved_generation_workspace,
                MemoryFootprint::default()
            );
            assert_eq!(runtime.pending_cleanup_models, 0);
            assert_eq!(runtime.pending_cleanup_sequences, 0);
            assert_eq!(runtime.exhausted_cleanup_models, 0);
            assert_eq!(runtime.exhausted_cleanup_sequences, 0);
            assert!(runtime.maintenance_error.is_none());
            assert_eq!(models.len(), 1);
            let model = models.first().ok_or("loaded model snapshot missing")?;
            assert_eq!(model.handle, handle);
            assert_eq!(
                model.execution_device,
                ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu)
            );
            assert_eq!(model.execution_scalar_type, execution_scalar_type);
            assert_eq!(model.reserved_footprint, model_footprint);
            assert_eq!(model.active_requests, 0);
            assert_eq!(model.pending_cleanup_sequences, 0);
            assert_eq!(model.exhausted_cleanup_sequences, 0);
            assert!(!model.degraded);
            Ok(())
        }
        event => Err(format!(
            "unexpected released snapshot event for ticket {:?}",
            event.ticket()
        )),
    }
}

#[cfg(feature = "cuda")]
fn assert_cuda_released_snapshot(
    hosted: &CandleRuntime,
    handle: ModelHandle,
    execution_scalar_type: ScalarType,
    model_footprint: MemoryFootprint,
) -> TestResult {
    let ticket = CommandTicket::new(51);
    hosted
        .try_submit(RuntimeCommand::Snapshot { ticket })
        .map_err(|error| format!("CUDA snapshot command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("CUDA snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            ticket: event_ticket,
            runtime,
            models,
        } if event_ticket == ticket => {
            assert_eq!(runtime.loaded_models, 1);
            assert_eq!(runtime.active_requests, 0);
            assert_eq!(runtime.reserved_footprint, model_footprint);
            assert_eq!(runtime.generation_workspaces, 0);
            assert_eq!(
                runtime.reserved_generation_workspace,
                MemoryFootprint::default()
            );
            assert_eq!(runtime.pending_cleanup_models, 0);
            assert_eq!(runtime.pending_cleanup_sequences, 0);
            assert_eq!(models.len(), 1);
            let model = models.first().ok_or("CUDA model snapshot missing")?;
            assert_eq!(model.handle, handle);
            assert_eq!(
                model.execution_device,
                ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda)
            );
            assert_eq!(model.execution_scalar_type, execution_scalar_type);
            assert_eq!(model.reserved_footprint, model_footprint);
            assert_eq!(model.active_requests, 0);
            assert!(!model.degraded);
            Ok(())
        }
        event => Err(format!(
            "unexpected CUDA snapshot event for ticket {:?}",
            event.ticket()
        )),
    }
}

fn unload_model(hosted: &CandleRuntime, handle: ModelHandle) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: UNLOAD_TICKET,
            handle,
            policy: UnloadPolicy::RejectIfBusy,
        })
        .map_err(|error| format!("unload command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == UNLOAD_TICKET && receipt.status == UnloadStatus::Unloaded => {
            assert_eq!(receipt.handle, handle);
            assert_eq!(receipt.cancelled_requests, 0);
            assert_unloaded_snapshot(hosted)
        }
        RuntimeEvent::ModelUnload {
            result: Err(error), ..
        } => Err(format!("model unload failed: {error:?}")),
        event => Err(format!(
            "unexpected unload event for ticket {:?}",
            event.ticket()
        )),
    }
}

fn assert_unloaded_snapshot(hosted: &CandleRuntime) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: UNLOADED_SNAPSHOT_TICKET,
        })
        .map_err(|error| format!("post-unload snapshot command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("post-unload snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            ticket,
            runtime,
            models,
        } if ticket == UNLOADED_SNAPSHOT_TICKET => {
            assert_eq!(runtime.loaded_models, 0);
            assert_eq!(runtime.active_requests, 0);
            assert_eq!(runtime.reserved_footprint, MemoryFootprint::default());
            assert_eq!(runtime.generation_workspaces, 0);
            assert_eq!(
                runtime.reserved_generation_workspace,
                MemoryFootprint::default()
            );
            assert_eq!(runtime.pending_cleanup_models, 0);
            assert_eq!(runtime.pending_cleanup_sequences, 0);
            assert_eq!(runtime.exhausted_cleanup_models, 0);
            assert_eq!(runtime.exhausted_cleanup_sequences, 0);
            assert!(runtime.maintenance_error.is_none());
            assert!(models.is_empty());
            Ok(())
        }
        event => Err(format!(
            "unexpected post-unload snapshot event for ticket {:?}",
            event.ticket()
        )),
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the helper owns both runtime endpoints through worker join"
)]
fn shutdown(hosted: CandleRuntime, thread: RuntimeThread) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: SHUTDOWN_TICKET,
        })
        .map_err(|error| format!("shutdown command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown {
            ticket,
            result: Ok(receipt),
        } if ticket == SHUTDOWN_TICKET => {
            assert_eq!(receipt.unloaded_models, 0);
            assert_eq!(receipt.cancelled_requests, 0);
        }
        RuntimeEvent::Shutdown {
            result: Err(error), ..
        } => return Err(format!("runtime shutdown failed: {error:?}")),
        event => {
            return Err(format!(
                "unexpected shutdown event for ticket {:?}",
                event.ticket()
            ));
        }
    }
    thread.join().map_err(|error| error.to_string())
}

#[cfg(feature = "cuda")]
fn require_cuda_opt_in() -> TestResult {
    if matches!(std::env::var("MILKDRIFT_CUDA_TEST").as_deref(), Ok("1")) {
        Ok(())
    } else {
        Err("set MILKDRIFT_CUDA_TEST=1 to execute CUDA hardware tests".to_owned())
    }
}

fn candle_fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/candle-llama")
}

struct ConvertedFixture {
    directory: PathBuf,
    config_path: PathBuf,
    weight_path: PathBuf,
}

impl ConvertedFixture {
    fn create(primary_dtype: DType, mixed_f32: bool) -> TestResult<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "milkdrift-e0-phase12-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let config_path = directory.join("config.json");
        let weight_path = directory.join("model.safetensors");
        write_converted_configuration(&config_path, primary_dtype)?;
        convert_weights(
            &candle_fixture_directory().join("model.safetensors"),
            &weight_path,
            primary_dtype,
            mixed_f32,
        )?;
        Ok(Self {
            directory,
            config_path,
            weight_path,
        })
    }
}

impl Drop for ConvertedFixture {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.directory);
    }
}

fn write_converted_configuration(destination: &Path, primary_dtype: DType) -> TestResult {
    let declaration = match primary_dtype {
        DType::F16 => "float16",
        DType::BF16 => "bfloat16",
        DType::F32 => "float32",
        _ => {
            return Err(format!(
                "unsupported converted fixture dtype: {primary_dtype:?}"
            ));
        }
    };
    let source = candle_fixture_directory().join("config.json");
    let configuration = fs::read_to_string(&source)
        .map_err(|error| format!("read converted fixture configuration source: {error}"))?;
    let expected = "\"dtype\": \"float32\"";
    if configuration.matches(expected).count() != 1 {
        return Err("committed fixture configuration has unexpected dtype declaration".to_owned());
    }
    let replacement = format!("\"dtype\": \"{declaration}\"");
    let converted = configuration.replacen(expected, replacement.as_str(), 1);
    fs::write(destination, converted)
        .map_err(|error| format!("write converted fixture configuration: {error}"))
}

fn convert_weights(
    source: &Path,
    destination: &Path,
    primary_dtype: DType,
    mixed_f32: bool,
) -> TestResult {
    let tensors = candle_core::safetensors::load(source, &Device::Cpu)
        .map_err(|error| format!("load F32 conversion source: {error}"))?;
    let converted = tensors
        .into_iter()
        .map(|(name, tensor)| {
            let dtype = if mixed_f32 && name == "model.norm.weight" {
                DType::F32
            } else {
                primary_dtype
            };
            let tensor = tensor
                .to_dtype(dtype)
                .map_err(|error| format!("convert {name} to {dtype:?}: {error}"))?;
            Ok((name, tensor))
        })
        .collect::<TestResult<HashMap<String, Tensor>>>()?;
    candle_core::safetensors::save(&converted, destination)
        .map_err(|error| format!("save converted fixture: {error}"))
}

fn deadline(timeout: Duration) -> TestResult<Instant> {
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| "test deadline overflow".into())
}

fn nonzero_u32(value: u32) -> TestResult<NonZeroU32> {
    NonZeroU32::new(value).ok_or_else(|| "value must be a non-zero u32".into())
}

fn nonzero_usize(value: usize) -> TestResult<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| "capacity must be non-zero".into())
}
