//! One shared real-model E0 generation contract instantiated for Candle and GGUF.

use std::num::{NonZeroI32, NonZeroU32, NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use candle_backend::{CandleLlamaLoader, CandleLlamaSource, CandleScalarType};
use domain_contracts::{
    BackendId, CancellationReason, CapabilitySet, DeviceId, DeviceKind, FinishReason, MemoryBudget,
    MemoryFootprint, ModelArchitecture, ModelHandle, ModelId, ModelLoader, RequestId, ScalarType,
    SequenceConfiguration, SequenceId, TokenId, UnloadPolicy, YieldReason,
};
use gguf_backend::{GgufBackendRuntime, GgufExecutionConfiguration, GgufLoader, GgufSource};
use host_runtime::TokenOutputRecordKind;
use inference_runtime::{
    CommandTicket, GenerationOutcome, GenerationOutputCapacityPolicy, GenerationOutputState,
    GenerationRequest, HostedRuntime, HostedRuntimeConfiguration, LoadReceipt, RuntimeCommand,
    RuntimeEvent, RuntimeLimits, RuntimeThread, UnloadStatus, start_hosted_runtime,
};
use sampling::SamplingConfig;

const CANDLE_BACKEND: BackendId = BackendId::new(41);
const GGUF_BACKEND: BackendId = BackendId::new(42);
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

type TestResult<T = ()> = Result<T, String>;

type BackendSource<B> = <<B as NativeBackend>::Loader as ModelLoader>::Source;

trait NativeBackend {
    type Loader: ModelLoader + Send + 'static;

    fn name(&self) -> &'static str;
    fn backend_id(&self) -> BackendId;
    fn loader(&self) -> Self::Loader;
    fn source(&self) -> TestResult<BackendSource<Self>>;
}

struct CandleBackend;

impl NativeBackend for CandleBackend {
    type Loader = CandleLlamaLoader;

    fn name(&self) -> &'static str {
        "Candle"
    }

    fn backend_id(&self) -> BackendId {
        CANDLE_BACKEND
    }

    fn loader(&self) -> Self::Loader {
        CandleLlamaLoader::new(CANDLE_BACKEND)
    }

    fn source(&self) -> TestResult<CandleLlamaSource> {
        let directory = fixture_directory("candle-llama");
        CandleLlamaSource::new(
            directory.join("config.json"),
            vec![directory.join("model.safetensors")],
            CandleScalarType::F32,
        )
        .map_err(|error| error.to_string())
    }
}

struct GgufBackend {
    runtime: GgufBackendRuntime,
}

impl NativeBackend for GgufBackend {
    type Loader = GgufLoader;

    fn name(&self) -> &'static str {
        "GGUF/llama.cpp"
    }

    fn backend_id(&self) -> BackendId {
        GGUF_BACKEND
    }

    fn loader(&self) -> Self::Loader {
        GgufLoader::new(GGUF_BACKEND, self.runtime.clone())
    }

    fn source(&self) -> TestResult<GgufSource> {
        let execution = GgufExecutionConfiguration::new(
            nonzero_u32(CONTEXT_LENGTH)?,
            nonzero_u32(8)?,
            nonzero_u32(8)?,
            NonZeroU32::MIN,
            nonzero_i32(1)?,
            nonzero_i32(1)?,
        )
        .map_err(|error| error.to_string())?
        .with_mmap(false);
        Ok(GgufSource::new(
            fixture_directory("gguf-llama").join("tiny-llama-f32.gguf"),
            execution,
        ))
    }
}

#[test]
fn candle_runs_shared_native_backend_suite() -> TestResult {
    run_shared_native_backend_suite(&CandleBackend)
}

#[test]
fn gguf_runs_shared_native_backend_suite() -> TestResult {
    // llama.cpp permits one process-level initialization. This token remains
    // alive while every loader/model scenario in the GGUF suite runs.
    let runtime = GgufBackendRuntime::initialize().map_err(|error| error.to_string())?;
    run_shared_native_backend_suite(&GgufBackend { runtime })
}

fn run_shared_native_backend_suite<B>(backend: &B) -> TestResult
where
    B: NativeBackend,
    BackendSource<B>: Send + 'static,
{
    run_generation_lifecycle_scenario(backend)
        .map_err(|error| format!("{} lifecycle scenario: {error}", backend.name()))?;
    run_backpressure_and_cancellation_scenario(backend)
        .map_err(|error| format!("{} backpressure scenario: {error}", backend.name()))
}

fn run_generation_lifecycle_scenario<B>(backend: &B) -> TestResult
where
    B: NativeBackend,
    BackendSource<B>: Send + 'static,
{
    let (hosted, thread) = hosted_runtime(backend.loader(), 16, 64)?;
    let loaded = load_model(&hosted, backend.source()?)?;
    assert_loaded_fixture(&loaded, backend.backend_id());
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
    assert_released_snapshot(&hosted, handle, CommandTicket::new(20))?;

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
    assert_released_snapshot(&hosted, handle, CommandTicket::new(21))?;

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
    assert_released_snapshot(&hosted, handle, CommandTicket::new(22))?;

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
    assert_released_snapshot(&hosted, handle, CommandTicket::new(23))?;

    unload_model(&hosted, handle)?;
    shutdown(hosted, thread)
}

fn run_backpressure_and_cancellation_scenario<B>(backend: &B) -> TestResult
where
    B: NativeBackend,
    BackendSource<B>: Send + 'static,
{
    let (hosted, thread) = hosted_runtime(backend.loader(), 1, 64)?;
    let loaded = load_model(&hosted, backend.source()?)?;
    assert_loaded_fixture(&loaded, backend.backend_id());
    let handle = loaded.handle;

    let backpressured = generation_request(10, 110, 4, SamplingConfig::greedy(), 23, Box::new([]))?;
    submit_generation(&hosted, handle, CommandTicket::new(30), &backpressured)?;
    let observed = collect_until_backpressure(&hosted, backpressured.request_id, OUTPUT_TIMEOUT)?;
    let backpressured_output =
        collect_until_released(&hosted, backpressured.request_id, OUTPUT_TIMEOUT, observed)?;
    assert_eq!(backpressured_output.tokens, vec![EXPECTED_GREEDY_TOKEN; 4]);
    assert_output_backpressure(&backpressured_output);
    assert_finished(&backpressured_output, FinishReason::TokenLimit);
    assert_released_snapshot(&hosted, handle, CommandTicket::new(40))?;

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
    assert_released_snapshot(&hosted, handle, CommandTicket::new(41))?;

    unload_model(&hosted, handle)?;
    shutdown(hosted, thread)
}

fn hosted_runtime<L>(
    loader: L,
    token_capacity: usize,
    record_capacity: usize,
) -> TestResult<(HostedRuntime<L::Source>, RuntimeThread)>
where
    L: ModelLoader + Send + 'static,
    L::Source: Send + 'static,
{
    let configuration =
        HostedRuntimeConfiguration::new(nonzero_usize(8)?, nonzero_usize(8)?, NonZeroU64::MIN)
            .with_token_output_capacity(
                nonzero_usize(token_capacity)?,
                nonzero_usize(record_capacity)?,
            );
    start_hosted_runtime(
        loader,
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

fn load_model<S>(hosted: &HostedRuntime<S>, source: S) -> TestResult<LoadReceipt> {
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: LOAD_TICKET,
            model_id: MODEL,
            source,
            device: DeviceId::new(0),
            device_kind: DeviceKind::Cpu,
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

fn assert_loaded_fixture(loaded: &LoadReceipt, backend: BackendId) {
    let descriptor = loaded.descriptor;
    assert_eq!(descriptor.backend, backend);
    assert_eq!(descriptor.metadata.architecture, ModelArchitecture::Llama);
    assert_eq!(descriptor.metadata.scalar_type, ScalarType::F32);
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

fn submit_generation<S>(
    hosted: &HostedRuntime<S>,
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

fn request_cancellation<S>(
    hosted: &HostedRuntime<S>,
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

fn pull_output<S>(
    hosted: &HostedRuntime<S>,
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

fn collect_until_backpressure<S>(
    hosted: &HostedRuntime<S>,
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

fn collect_until_released<S>(
    hosted: &HostedRuntime<S>,
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

fn assert_released_snapshot<S>(
    hosted: &HostedRuntime<S>,
    handle: ModelHandle,
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

fn unload_model<S>(hosted: &HostedRuntime<S>, handle: ModelHandle) -> TestResult {
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

fn assert_unloaded_snapshot<S>(hosted: &HostedRuntime<S>) -> TestResult {
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
fn shutdown<S>(hosted: HostedRuntime<S>, thread: RuntimeThread) -> TestResult {
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

fn fixture_directory(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn deadline(timeout: Duration) -> TestResult<Instant> {
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| "test deadline overflow".into())
}

fn nonzero_u32(value: u32) -> TestResult<NonZeroU32> {
    NonZeroU32::new(value).ok_or_else(|| "value must be a non-zero u32".into())
}

fn nonzero_i32(value: i32) -> TestResult<NonZeroI32> {
    NonZeroI32::new(value).ok_or_else(|| "value must be a non-zero i32".into())
}

fn nonzero_usize(value: usize) -> TestResult<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| "capacity must be non-zero".into())
}
