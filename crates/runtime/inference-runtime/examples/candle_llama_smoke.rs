//! Opt-in E0 diagnostic for already-resolved local Candle Llama artifacts.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use candle_backend::{CandleLlamaLoader, CandleLlamaSource};
use domain_contracts::{
    BackendId, CancellationReason, DeviceId, DeviceKind, ExecutionDevice, FinishReason,
    MemoryBudget, MemoryFootprint, ModelArchitecture, ModelHandle, ModelId, RequestId, ScalarType,
    SequenceConfiguration, SequenceId, TokenId, UnloadPolicy,
};
use host_runtime::TokenOutputRecordKind;
use inference_runtime::{
    CommandTicket, GenerationOutcome, GenerationOutputCapacityPolicy, GenerationOutputState,
    GenerationRequest, HostedRuntime, HostedRuntimeConfiguration, RuntimeCommand, RuntimeEvent,
    RuntimeLimits, RuntimeThread, UnloadStatus, start_hosted_runtime,
};
use sampling::SamplingConfig;

const MODEL_REPOSITORY: &str = "neubla/tiny-random-LlamaForCausalLM";
const MODEL_REVISION: &str = "1c81a3fba044af78df253edc66bdbab183184932";
const EXPECTED_ARCHITECTURE: &str = "LlamaForCausalLM/Llama";
const MAXIMUM_SEQUENCE_TOKENS: u32 = 64;
const MAXIMUM_PREFILL_TOKENS: u32 = 32;
const BACKEND: BackendId = BackendId::new(401);
const MODEL: ModelId = ModelId::new(1);
const GENERATED_TOKEN_LIMIT: u32 = 8;
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

type SmokeResult<T = ()> = Result<T, SmokeError>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> SmokeResult {
    let configuration = SmokeConfiguration::from_environment()?;
    println!("model repository: {MODEL_REPOSITORY}");
    println!("model revision: {MODEL_REVISION}");
    println!("expected architecture: {EXPECTED_ARCHITECTURE}");
    println!("prompt token count: {}", configuration.prompt_tokens.len());

    let rss_before_load = resident_set_kib();
    let (hosted, thread) = hosted_runtime()?;
    let load_started = Instant::now();
    let handle = load_model(&hosted, configuration.source)?;
    let load_duration = load_started.elapsed();
    let rss_after_load = resident_set_kib();

    let generation_started = Instant::now();
    submit_generation(
        &hosted,
        handle,
        generation_request(
            RequestId::new(1),
            SequenceId::new(1),
            configuration.prompt_tokens.clone(),
            GENERATED_TOKEN_LIMIT,
        ),
        CommandTicket::new(2),
    )?;
    let completed = collect_generation(&hosted, RequestId::new(1), generation_started)?;
    if completed.tokens.is_empty() {
        return Err(SmokeError::runtime(
            "Candle generation released without publishing a token",
        ));
    }
    let completed_outcome = GenerationOutcome::Finished(FinishReason::TokenLimit);
    for expected_state in [
        GenerationOutputState::Terminal(completed_outcome),
        GenerationOutputState::Released(completed_outcome),
    ] {
        if !completed.states.contains(&expected_state) {
            return Err(SmokeError::runtime(format!(
                "completed generation did not publish {expected_state:?}"
            )));
        }
    }
    let rss_during_generation = completed.rss_at_first_token;

    submit_generation(
        &hosted,
        handle,
        generation_request(
            RequestId::new(2),
            SequenceId::new(2),
            configuration.prompt_tokens,
            GENERATED_TOKEN_LIMIT,
        ),
        CommandTicket::new(3),
    )?;
    wait_for_first_token(&hosted, RequestId::new(2))?;
    let cancellation_started = Instant::now();
    request_cancellation(&hosted, RequestId::new(2))?;
    let cancelled = collect_released(&hosted, RequestId::new(2))?;
    let cancellation_latency = cancellation_started.elapsed();
    let expected_cancellation =
        GenerationOutcome::Finished(FinishReason::Cancelled(CancellationReason::UserRequested));
    for expected_state in [
        GenerationOutputState::Terminal(expected_cancellation),
        GenerationOutputState::Released(expected_cancellation),
    ] {
        if !cancelled.states.contains(&expected_state) {
            return Err(SmokeError::runtime(format!(
                "cancelled generation did not publish {expected_state:?}"
            )));
        }
    }

    assert_released_snapshot(&hosted)?;
    let unload_started = Instant::now();
    unload_model(&hosted, handle)?;
    let unload_duration = unload_started.elapsed();
    assert_unloaded_snapshot(&hosted)?;
    let rss_after_unload = resident_set_kib();
    shutdown(hosted, thread)?;

    println!("generated token ids: {:?}", completed.tokens);
    println!("model load duration: {:.6} s", load_duration.as_secs_f64());
    println!(
        "time to first generated token: {:.6} s",
        completed.time_to_first_token.as_secs_f64()
    );
    println!(
        "decode tokens per second: {:.3}",
        completed.decode_tokens_per_second
    );
    println!(
        "cancellation latency: {:.6} s",
        cancellation_latency.as_secs_f64()
    );
    println!(
        "model unload duration: {:.6} s",
        unload_duration.as_secs_f64()
    );
    print_rss("process RSS before load", rss_before_load);
    print_rss("process RSS after load", rss_after_load);
    print_rss("process RSS during generation", rss_during_generation);
    print_rss("process RSS after unload", rss_after_unload);
    Ok(())
}

struct SmokeConfiguration {
    source: CandleLlamaSource,
    prompt_tokens: Box<[TokenId]>,
}

impl SmokeConfiguration {
    fn from_environment() -> SmokeResult<Self> {
        let model_directory = required_environment_path("LLM_APP_CANDLE_MODEL_DIR")?;
        let revision = required_environment("LLM_APP_CANDLE_MODEL_REVISION")?;
        if revision != MODEL_REVISION {
            return Err(SmokeError::configuration(format!(
                "LLM_APP_CANDLE_MODEL_REVISION must equal pinned revision {MODEL_REVISION}"
            )));
        }
        let config_path = model_directory.join("config.json");
        let weight_path = model_directory.join("model.safetensors");
        require_file(&config_path, "config.json")?;
        require_file(&weight_path, "model.safetensors")?;
        let prompt_tokens = parse_prompt_tokens(
            &std::env::var("LLM_APP_CANDLE_PROMPT_TOKENS").unwrap_or_else(|_| "1,2,3".to_owned()),
        )?;
        let maximum_prefill_tokens = usize::try_from(MAXIMUM_PREFILL_TOKENS)
            .map_err(|_| SmokeError::configuration("smoke prefill limit exceeds usize"))?;
        if prompt_tokens.len() > maximum_prefill_tokens {
            return Err(SmokeError::configuration(format!(
                "LLM_APP_CANDLE_PROMPT_TOKENS exceeds the smoke prefill limit of \
                 {MAXIMUM_PREFILL_TOKENS} tokens"
            )));
        }
        let source = CandleLlamaSource::new(config_path, vec![weight_path], Some(ScalarType::F32))
            .map_err(|error| SmokeError::configuration(error.to_string()))?;
        Ok(Self {
            source,
            prompt_tokens,
        })
    }
}

struct CompletedGeneration {
    tokens: Vec<TokenId>,
    states: Vec<GenerationOutputState>,
    time_to_first_token: Duration,
    decode_tokens_per_second: f64,
    rss_at_first_token: Option<u64>,
}

struct ReleasedGeneration {
    states: Vec<GenerationOutputState>,
}

fn hosted_runtime() -> SmokeResult<(HostedRuntime<CandleLlamaSource>, RuntimeThread)> {
    let configuration = HostedRuntimeConfiguration::new(
        nonzero_usize(16, "command capacity")?,
        nonzero_usize(16, "event capacity")?,
        NonZeroU64::MIN,
    )
    .with_token_output_capacity(
        NonZeroUsize::MIN,
        nonzero_usize(32, "output record capacity")?,
    );
    start_hosted_runtime(
        CandleLlamaLoader::new(BACKEND),
        RuntimeLimits::new(
            NonZeroU32::MIN,
            NonZeroU32::new(2).ok_or_else(|| SmokeError::runtime("request limit is zero"))?,
            MemoryBudget {
                host_bytes: u64::MAX,
                device_bytes: 0,
            },
        ),
        configuration,
    )
    .map_err(|error| SmokeError::runtime(error.to_string()))
}

fn load_model(
    hosted: &HostedRuntime<CandleLlamaSource>,
    source: CandleLlamaSource,
) -> SmokeResult<ModelHandle> {
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(1),
            model_id: MODEL,
            source,
            execution_device: ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu),
        })
        .map_err(|error| SmokeError::runtime(format!("load command rejected: {error:?}")))?;
    match receive(hosted, "model load")? {
        RuntimeEvent::ModelLoaded {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(1) => {
            if receipt.descriptor.metadata.architecture != ModelArchitecture::Llama {
                return Err(SmokeError::runtime(format!(
                    "loaded model architecture was {:?}, expected Llama",
                    receipt.descriptor.metadata.architecture
                )));
            }
            if receipt.descriptor.metadata.context_length < MAXIMUM_SEQUENCE_TOKENS {
                return Err(SmokeError::runtime(format!(
                    "loaded model context length {} is below smoke requirement {}",
                    receipt.descriptor.metadata.context_length, MAXIMUM_SEQUENCE_TOKENS
                )));
            }
            Ok(receipt.handle)
        }
        RuntimeEvent::ModelLoaded {
            result: Err(error), ..
        } => Err(SmokeError::runtime(format!("model load failed: {error:?}"))),
        _ => Err(SmokeError::runtime("unexpected model load event")),
    }
}

fn submit_generation(
    hosted: &HostedRuntime<CandleLlamaSource>,
    handle: ModelHandle,
    request: GenerationRequest,
    ticket: CommandTicket,
) -> SmokeResult {
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket,
            handle,
            request,
        })
        .map_err(|error| SmokeError::runtime(format!("generation command rejected: {error:?}")))?;
    match receive(hosted, "generation admission")? {
        RuntimeEvent::GenerationAdmitted {
            ticket: event_ticket,
            result: Ok(_),
        } if event_ticket == ticket => Ok(()),
        RuntimeEvent::GenerationAdmitted {
            result: Err(error), ..
        } => Err(SmokeError::runtime(format!(
            "generation admission failed: {error:?}"
        ))),
        _ => Err(SmokeError::runtime("unexpected generation admission event")),
    }
}

fn generation_request(
    request_id: RequestId,
    sequence_id: SequenceId,
    prompt_tokens: Box<[TokenId]>,
    maximum_generated_tokens: u32,
) -> GenerationRequest {
    GenerationRequest {
        request_id,
        sequence_id,
        prompt_tokens,
        sequence: SequenceConfiguration::new(
            NonZeroU32::new(MAXIMUM_SEQUENCE_TOKENS).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(MAXIMUM_PREFILL_TOKENS).unwrap_or(NonZeroU32::MIN),
        ),
        maximum_generated_tokens: NonZeroU32::new(maximum_generated_tokens)
            .unwrap_or(NonZeroU32::MIN),
        sampling: SamplingConfig::greedy(),
        seed: 17,
        eos_tokens: Box::new([]),
        stop_sequences: Box::new([]),
        scheduler_quantum: NonZeroU32::MIN,
        output_capacity: GenerationOutputCapacityPolicy::default(),
    }
}

fn collect_generation(
    hosted: &HostedRuntime<CandleLlamaSource>,
    request_id: RequestId,
    started: Instant,
) -> SmokeResult<CompletedGeneration> {
    let deadline = started
        .checked_add(EVENT_TIMEOUT)
        .ok_or_else(|| SmokeError::runtime("generation deadline overflow"))?;
    let mut tokens = Vec::new();
    let mut states = Vec::new();
    let mut first_token_at = None;
    let mut rss_at_first_token = None;
    loop {
        pull_output(hosted, request_id, &mut tokens, &mut states)?;
        if first_token_at.is_none() && !tokens.is_empty() {
            first_token_at = Some(Instant::now());
            rss_at_first_token = resident_set_kib();
        }
        if states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
        {
            let finished = Instant::now();
            let first = first_token_at
                .ok_or_else(|| SmokeError::runtime("generation released before first token"))?;
            let decode_duration = finished.saturating_duration_since(first);
            let decoded_after_first = u32::try_from(tokens.len().saturating_sub(1))
                .map_err(|_| SmokeError::runtime("generated token count exceeded u32"))?;
            let decode_tokens_per_second = if decode_duration.is_zero() {
                0.0
            } else {
                f64::from(decoded_after_first) / decode_duration.as_secs_f64()
            };
            return Ok(CompletedGeneration {
                tokens,
                states,
                time_to_first_token: first.saturating_duration_since(started),
                decode_tokens_per_second,
                rss_at_first_token,
            });
        }
        if Instant::now() >= deadline {
            return Err(SmokeError::runtime(format!(
                "generation timed out with states {states:?}"
            )));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn wait_for_first_token(
    hosted: &HostedRuntime<CandleLlamaSource>,
    request_id: RequestId,
) -> SmokeResult {
    let started = Instant::now();
    let deadline = started
        .checked_add(EVENT_TIMEOUT)
        .ok_or_else(|| SmokeError::runtime("first-token deadline overflow"))?;
    let mut tokens = Vec::new();
    let mut states = Vec::new();
    loop {
        pull_output(hosted, request_id, &mut tokens, &mut states)?;
        if !tokens.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(SmokeError::runtime(
                "cancellation fixture did not publish a first token",
            ));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn request_cancellation(
    hosted: &HostedRuntime<CandleLlamaSource>,
    request_id: RequestId,
) -> SmokeResult {
    hosted
        .try_submit(RuntimeCommand::CancelRequest {
            ticket: CommandTicket::new(4),
            request_id,
            reason: CancellationReason::UserRequested,
        })
        .map_err(|error| SmokeError::runtime(format!("cancel command rejected: {error:?}")))?;
    match receive(hosted, "generation cancellation")? {
        RuntimeEvent::GenerationCancellationRequested {
            ticket,
            request_id: event_request,
            result: Ok(()),
        } if ticket == CommandTicket::new(4) && event_request == request_id => Ok(()),
        RuntimeEvent::GenerationCancellationRequested {
            result: Err(error), ..
        } => Err(SmokeError::runtime(format!(
            "generation cancellation failed: {error:?}"
        ))),
        _ => Err(SmokeError::runtime(
            "unexpected generation cancellation event",
        )),
    }
}

fn collect_released(
    hosted: &HostedRuntime<CandleLlamaSource>,
    request_id: RequestId,
) -> SmokeResult<ReleasedGeneration> {
    let deadline = Instant::now()
        .checked_add(EVENT_TIMEOUT)
        .ok_or_else(|| SmokeError::runtime("release deadline overflow"))?;
    let mut tokens = Vec::new();
    let mut states = Vec::new();
    loop {
        pull_output(hosted, request_id, &mut tokens, &mut states)?;
        if states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
        {
            return Ok(ReleasedGeneration { states });
        }
        if Instant::now() >= deadline {
            return Err(SmokeError::runtime(format!(
                "cancelled generation release timed out with states {states:?}"
            )));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn pull_output(
    hosted: &HostedRuntime<CandleLlamaSource>,
    request_id: RequestId,
    tokens: &mut Vec<TokenId>,
    states: &mut Vec<GenerationOutputState>,
) -> SmokeResult {
    hosted
        .pull_token_output(|batch| {
            for record in batch.records {
                if record.request_id != request_id {
                    continue;
                }
                match record.kind {
                    TokenOutputRecordKind::Tokens(range) => {
                        if let Some(published) = batch.tokens_for(range) {
                            tokens.extend_from_slice(published);
                        }
                    }
                    TokenOutputRecordKind::State(state) => states.push(state),
                }
            }
        })
        .map_err(|error| SmokeError::runtime(format!("output pull failed: {error:?}")))
}

fn assert_released_snapshot(hosted: &HostedRuntime<CandleLlamaSource>) -> SmokeResult {
    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(5),
        })
        .map_err(|error| SmokeError::runtime(format!("snapshot command rejected: {error:?}")))?;
    match receive(hosted, "runtime snapshot")? {
        RuntimeEvent::Snapshot {
            ticket,
            runtime,
            models,
        } if ticket == CommandTicket::new(5) => {
            if runtime.loaded_models != 1
                || runtime.active_requests != 0
                || runtime.generation_workspaces != 0
                || runtime.reserved_generation_workspace != MemoryFootprint::default()
                || runtime.pending_cleanup_models != 0
                || runtime.pending_cleanup_sequences != 0
                || runtime.exhausted_cleanup_models != 0
                || runtime.exhausted_cleanup_sequences != 0
                || runtime.maintenance_error.is_some()
                || models.len() != 1
            {
                return Err(SmokeError::runtime(format!(
                    "generation ownership remained after release: {runtime:?}"
                )));
            }
            Ok(())
        }
        _ => Err(SmokeError::runtime("unexpected runtime snapshot event")),
    }
}

fn unload_model(hosted: &HostedRuntime<CandleLlamaSource>, handle: ModelHandle) -> SmokeResult {
    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(6),
            handle,
            policy: UnloadPolicy::RejectIfBusy,
        })
        .map_err(|error| SmokeError::runtime(format!("unload command rejected: {error:?}")))?;
    match receive(hosted, "model unload")? {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(6) && receipt.status == UnloadStatus::Unloaded => Ok(()),
        RuntimeEvent::ModelUnload {
            result: Err(error), ..
        } => Err(SmokeError::runtime(format!(
            "model unload failed: {error:?}"
        ))),
        _ => Err(SmokeError::runtime("unexpected model unload event")),
    }
}

fn assert_unloaded_snapshot(hosted: &HostedRuntime<CandleLlamaSource>) -> SmokeResult {
    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(7),
        })
        .map_err(|error| {
            SmokeError::runtime(format!("post-unload snapshot command rejected: {error:?}"))
        })?;
    match receive(hosted, "post-unload runtime snapshot")? {
        RuntimeEvent::Snapshot {
            ticket,
            runtime,
            models,
        } if ticket == CommandTicket::new(7) => {
            if runtime.loaded_models != 0
                || runtime.active_requests != 0
                || runtime.reserved_footprint != MemoryFootprint::default()
                || runtime.generation_workspaces != 0
                || runtime.reserved_generation_workspace != MemoryFootprint::default()
                || runtime.pending_cleanup_models != 0
                || runtime.pending_cleanup_sequences != 0
                || runtime.exhausted_cleanup_models != 0
                || runtime.exhausted_cleanup_sequences != 0
                || runtime.maintenance_error.is_some()
                || !models.is_empty()
            {
                return Err(SmokeError::runtime(format!(
                    "runtime retained ownership after unload: {runtime:?}"
                )));
            }
            Ok(())
        }
        _ => Err(SmokeError::runtime(
            "unexpected post-unload runtime snapshot event",
        )),
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the helper owns both runtime endpoints through worker join"
)]
fn shutdown(hosted: HostedRuntime<CandleLlamaSource>, thread: RuntimeThread) -> SmokeResult {
    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(8),
        })
        .map_err(|error| SmokeError::runtime(format!("shutdown command rejected: {error:?}")))?;
    match receive(&hosted, "runtime shutdown")? {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        RuntimeEvent::Shutdown {
            result: Err(error), ..
        } => {
            return Err(SmokeError::runtime(format!(
                "runtime shutdown failed: {error:?}"
            )));
        }
        _ => return Err(SmokeError::runtime("unexpected runtime shutdown event")),
    }
    thread
        .join()
        .map_err(|error| SmokeError::runtime(error.to_string()))
}

fn receive(
    hosted: &HostedRuntime<CandleLlamaSource>,
    operation: &str,
) -> SmokeResult<RuntimeEvent> {
    hosted.receive_timeout(EVENT_TIMEOUT).map_err(|error| {
        SmokeError::runtime(format!("{operation} event was not received: {error:?}"))
    })
}

fn required_environment(name: &str) -> SmokeResult<String> {
    std::env::var(name).map_err(|_| {
        SmokeError::configuration(format!("required environment variable {name} is missing"))
    })
}

fn required_environment_path(name: &str) -> SmokeResult<PathBuf> {
    required_environment(name).map(PathBuf::from)
}

fn require_file(path: &Path, label: &str) -> SmokeResult {
    if path.is_file() {
        Ok(())
    } else {
        Err(SmokeError::configuration(format!(
            "required {label} is missing at {}",
            path.display()
        )))
    }
}

fn parse_prompt_tokens(value: &str) -> SmokeResult<Box<[TokenId]>> {
    let tokens = value
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| {
            token.parse::<u32>().map(TokenId::new).map_err(|_| {
                SmokeError::configuration(format!(
                    "LLM_APP_CANDLE_PROMPT_TOKENS contains invalid token {token:?}"
                ))
            })
        })
        .collect::<SmokeResult<Vec<_>>>()?;
    if tokens.is_empty() {
        return Err(SmokeError::configuration(
            "LLM_APP_CANDLE_PROMPT_TOKENS must contain at least one token",
        ));
    }
    Ok(tokens.into_boxed_slice())
}

fn nonzero_usize(value: usize, label: &str) -> SmokeResult<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| SmokeError::runtime(format!("{label} must be non-zero")))
}

fn resident_set_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

fn print_rss(label: &str, value: Option<u64>) {
    match value {
        Some(kibibytes) => println!("{label}: {kibibytes} KiB"),
        None => println!("{label}: unavailable on this platform"),
    }
}

#[derive(Debug)]
enum SmokeError {
    Configuration(String),
    Runtime(String),
}

impl SmokeError {
    fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self::Runtime(message.into())
    }
}

impl Display for SmokeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message) => write!(formatter, "configuration error: {message}"),
            Self::Runtime(message) => write!(formatter, "runtime error: {message}"),
        }
    }
}

impl Error for SmokeError {}
