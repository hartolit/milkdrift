//! Real Candle CPU coverage through the backend-independent E0 generation scheduler.

use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use candle_backend::{CandleLlamaLoader, CandleLlamaSource, CandleScalarType};
use domain_contracts::{
    BackendId, CancellationReason, DeviceId, DeviceKind, FinishReason, MemoryBudget,
    MemoryFootprint, ModelHandle, ModelId, RequestId, SequenceConfiguration, SequenceId, TokenId,
    UnloadPolicy,
};
use host_runtime::TokenOutputRecordKind;
use inference_runtime::{
    CommandTicket, GenerationOutcome, GenerationOutputCapacityPolicy, GenerationOutputState,
    GenerationRequest, HostedRuntime, HostedRuntimeConfiguration, RuntimeCommand, RuntimeEvent,
    RuntimeLimits, RuntimeThread, UnloadStatus, start_hosted_runtime,
};
use sampling::SamplingConfig;

const BACKEND: BackendId = BackendId::new(41);
const MODEL: ModelId = ModelId::new(7);

type TestResult<T = ()> = Result<T, String>;

#[test]
fn candle_llama_generates_through_e0_and_unloads() -> TestResult {
    let fixture = TinyLlamaFixture::create();
    let (hosted, thread) = hosted_runtime(8, 16)?;
    let handle = load_model(&hosted, fixture.source()?)?;
    let request_id = RequestId::new(1);
    submit_generation(
        &hosted,
        handle,
        generation_request(request_id, 1, 3, Box::new([])),
    )?;

    let output = collect_until_released(&hosted, request_id, Duration::from_secs(5))?;
    assert_eq!(output.tokens, vec![TokenId::new(2); 3]);
    assert!(output.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::TokenLimit)
    )));
    assert!(output.states.contains(&GenerationOutputState::Released(
        GenerationOutcome::Finished(FinishReason::TokenLimit)
    )));

    let eos_request = RequestId::new(3);
    submit_generation(
        &hosted,
        handle,
        generation_request(eos_request, 3, 3, Box::new([TokenId::new(2)])),
    )?;
    let eos_output = collect_until_released(&hosted, eos_request, Duration::from_secs(5))?;
    assert_eq!(eos_output.tokens, vec![TokenId::new(2)]);
    let eos_outcome = GenerationOutcome::Finished(FinishReason::EndOfSequence(TokenId::new(2)));
    assert!(
        eos_output
            .states
            .contains(&GenerationOutputState::Terminal(eos_outcome))
    );
    assert!(
        eos_output
            .states
            .contains(&GenerationOutputState::Released(eos_outcome))
    );

    assert_released_snapshot(&hosted)?;
    unload_model(&hosted, handle)?;
    shutdown(hosted, thread)
}

#[test]
fn candle_llama_cancels_at_a_backend_boundary_and_releases() -> TestResult {
    let fixture = TinyLlamaFixture::create();
    let (hosted, thread) = hosted_runtime(1, 16)?;
    let handle = load_model(&hosted, fixture.source()?)?;
    let request_id = RequestId::new(2);
    submit_generation(
        &hosted,
        handle,
        generation_request(request_id, 2, 12, Box::new([])),
    )?;

    let first = collect_until_token(&hosted, request_id, Duration::from_secs(5))?;
    assert_eq!(first, vec![TokenId::new(2)]);
    hosted
        .try_submit(RuntimeCommand::CancelRequest {
            ticket: CommandTicket::new(3),
            request_id,
            reason: CancellationReason::UserRequested,
        })
        .map_err(|error| format!("cancel command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(Duration::from_secs(5))
        .map_err(|error| format!("cancellation event failed: {error:?}"))?
    {
        RuntimeEvent::GenerationCancellationRequested {
            ticket,
            request_id: event_request,
            result: Ok(()),
        } if ticket == CommandTicket::new(3) && event_request == request_id => {}
        _ => return Err("unexpected cancellation event".into()),
    }

    let output = collect_until_released(&hosted, request_id, Duration::from_secs(5))?;
    let cancelled =
        GenerationOutcome::Finished(FinishReason::Cancelled(CancellationReason::UserRequested));
    assert!(
        output
            .states
            .contains(&GenerationOutputState::Terminal(cancelled))
    );
    assert!(
        output
            .states
            .contains(&GenerationOutputState::Released(cancelled))
    );
    assert_released_snapshot(&hosted)?;
    unload_model(&hosted, handle)?;
    shutdown(hosted, thread)
}

fn hosted_runtime(
    token_capacity: usize,
    record_capacity: usize,
) -> TestResult<(HostedRuntime<CandleLlamaSource>, RuntimeThread)> {
    let configuration =
        HostedRuntimeConfiguration::new(nonzero_usize(8)?, nonzero_usize(8)?, NonZeroU64::MIN)
            .with_token_output_capacity(
                nonzero_usize(token_capacity)?,
                nonzero_usize(record_capacity)?,
            );
    start_hosted_runtime(
        CandleLlamaLoader::new(BACKEND),
        RuntimeLimits::new(
            NonZeroU32::MIN,
            NonZeroU32::new(2).ok_or("active request limit")?,
            MemoryBudget {
                host_bytes: u64::MAX,
                device_bytes: 0,
            },
        ),
        configuration,
    )
    .map_err(|error| error.to_string())
}

fn load_model(
    hosted: &HostedRuntime<CandleLlamaSource>,
    source: CandleLlamaSource,
) -> TestResult<ModelHandle> {
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(1),
            model_id: MODEL,
            source,
            device: DeviceId::new(0),
            device_kind: DeviceKind::Cpu,
        })
        .map_err(|error| format!("load command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(Duration::from_secs(5))
        .map_err(|error| format!("load event failed: {error:?}"))?
    {
        RuntimeEvent::ModelLoaded {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(1) => Ok(receipt.handle),
        RuntimeEvent::ModelLoaded {
            result: Err(error), ..
        } => Err(format!("model load failed: {error:?}")),
        _ => Err("unexpected model load event".into()),
    }
}

fn submit_generation(
    hosted: &HostedRuntime<CandleLlamaSource>,
    handle: ModelHandle,
    request: GenerationRequest,
) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket: CommandTicket::new(2),
            handle,
            request,
        })
        .map_err(|error| format!("generation command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(Duration::from_secs(5))
        .map_err(|error| format!("generation event failed: {error:?}"))?
    {
        RuntimeEvent::GenerationAdmitted {
            ticket,
            result: Ok(_),
        } if ticket == CommandTicket::new(2) => Ok(()),
        RuntimeEvent::GenerationAdmitted {
            result: Err(error), ..
        } => Err(format!("generation admission failed: {error:?}")),
        _ => Err("unexpected generation event".into()),
    }
}

fn generation_request(
    request_id: RequestId,
    sequence: u64,
    maximum_generated_tokens: u32,
    eos_tokens: Box<[TokenId]>,
) -> GenerationRequest {
    GenerationRequest {
        request_id,
        sequence_id: SequenceId::new(sequence),
        prompt_tokens: vec![TokenId::new(1), TokenId::new(2)].into_boxed_slice(),
        sequence: SequenceConfiguration::new(
            NonZeroU32::new(16).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
        ),
        maximum_generated_tokens: NonZeroU32::new(maximum_generated_tokens)
            .unwrap_or(NonZeroU32::MIN),
        sampling: SamplingConfig::greedy(),
        seed: 17,
        eos_tokens,
        stop_sequences: Box::new([]),
        scheduler_quantum: NonZeroU32::MIN,
        output_capacity: GenerationOutputCapacityPolicy::default(),
    }
}

struct CollectedOutput {
    tokens: Vec<TokenId>,
    states: Vec<GenerationOutputState>,
}

fn collect_until_token(
    hosted: &HostedRuntime<CandleLlamaSource>,
    request_id: RequestId,
    timeout: Duration,
) -> TestResult<Vec<TokenId>> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("token deadline overflow")?;
    loop {
        let tokens = hosted
            .pull_token_output(|batch| {
                let mut tokens = Vec::new();
                for record in batch.records {
                    if record.request_id != request_id {
                        continue;
                    }
                    if let TokenOutputRecordKind::Tokens(range) = record.kind
                        && let Some(published) = batch.tokens_for(range)
                    {
                        tokens.extend_from_slice(published);
                    }
                }
                tokens
            })
            .map_err(|error| format!("token pull failed: {error:?}"))?;
        if !tokens.is_empty() {
            return Ok(tokens);
        }
        if Instant::now() >= deadline {
            return Err("first Candle token timed out".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn collect_until_released(
    hosted: &HostedRuntime<CandleLlamaSource>,
    request_id: RequestId,
    timeout: Duration,
) -> TestResult<CollectedOutput> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("release deadline overflow")?;
    let mut output = CollectedOutput {
        tokens: Vec::new(),
        states: Vec::new(),
    };
    loop {
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
            .map_err(|error| format!("output pull failed: {error:?}"))?;
        if output
            .states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
        {
            return Ok(output);
        }
        if Instant::now() >= deadline {
            return Err(format!("Candle release timed out: {:?}", output.states));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn assert_released_snapshot(hosted: &HostedRuntime<CandleLlamaSource>) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(4),
        })
        .map_err(|error| format!("snapshot command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(Duration::from_secs(5))
        .map_err(|error| format!("snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            ticket,
            runtime,
            models,
        } if ticket == CommandTicket::new(4) => {
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
            Ok(())
        }
        _ => Err("unexpected snapshot event".into()),
    }
}

fn unload_model(hosted: &HostedRuntime<CandleLlamaSource>, handle: ModelHandle) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(5),
            handle,
            policy: UnloadPolicy::RejectIfBusy,
        })
        .map_err(|error| format!("unload command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(Duration::from_secs(5))
        .map_err(|error| format!("unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            ticket,
            result: Ok(receipt),
        } if ticket == CommandTicket::new(5) && receipt.status == UnloadStatus::Unloaded => Ok(()),
        RuntimeEvent::ModelUnload {
            result: Err(error), ..
        } => Err(format!("model unload failed: {error:?}")),
        _ => Err("unexpected model unload event".into()),
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the helper owns both runtime endpoints through worker join"
)]
fn shutdown(hosted: HostedRuntime<CandleLlamaSource>, thread: RuntimeThread) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(6),
        })
        .map_err(|error| format!("shutdown command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(Duration::from_secs(5))
        .map_err(|error| format!("shutdown event failed: {error:?}"))?
    {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        RuntimeEvent::Shutdown {
            result: Err(error), ..
        } => return Err(format!("runtime shutdown failed: {error:?}")),
        _ => return Err("unexpected shutdown event".into()),
    }
    thread.join().map_err(|error| error.to_string())
}

fn nonzero_usize(value: usize) -> TestResult<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| "capacity must be non-zero".into())
}

struct TinyLlamaFixture {
    config_path: PathBuf,
    weight_path: PathBuf,
}

impl TinyLlamaFixture {
    fn create() -> Self {
        let directory =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/candle-llama");
        Self {
            config_path: directory.join("config.json"),
            weight_path: directory.join("model.safetensors"),
        }
    }

    fn source(&self) -> TestResult<CandleLlamaSource> {
        CandleLlamaSource::new(
            self.config_path.clone(),
            vec![self.weight_path.clone()],
            CandleScalarType::F32,
        )
        .map_err(|error| error.to_string())
    }
}
