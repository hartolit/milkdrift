//! Independent pinned E1 product lifecycle through public `ApplicationRuntime` APIs.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationDevice, ApplicationEngine, ApplicationEvent,
    ApplicationModelFormat, ApplicationOutputRecordKind, ApplicationOutputState,
    ApplicationRuntime, ApplicationRuntimeConfiguration, ApplicationScalarType, ApplicationSource,
    GenerationSeed, GenerationSettings, GenerationTerminal, GenerationTerminalKind,
    GenerationTerminalOutcome, LoadedModel, ModelSelection, ModelUnloadBehavior, ResolvedModel,
};
use domain_contracts::{FinishReason, GenerationUsage, RequestId};

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::memory::{ProcessMemory, process_memory};
use crate::report::{
    FirstDecodedOutputMeasurement, ProxyThroughputMeasurement, RealGenerationEvidence,
    RealModelEvidence, RealProcessMemory, RealProductCycle, UsageRecord, duration_ns, throughput,
};
use crate::workspace::OutputWorkspace;

use super::shutdown_for_cleanup;

pub(crate) const REAL_PRODUCT_REPOSITORY: &str = "neubla/tiny-random-LlamaForCausalLM";
pub(crate) const REAL_PRODUCT_REVISION: &str = "1c81a3fba044af78df253edc66bdbab183184932";
pub(crate) const REAL_GENERATION_TOKEN_COUNT: u32 = 8;
pub(crate) const REAL_POST_FIRST_TOKEN_WINDOW: u32 = 4;
const DIRECT_COMPLETION_PROMPT: &str = "Hello";
const GENERATION_SEED: u64 = 39;
const POLL_INTERVAL: Duration = Duration::from_millis(1);
const RESOLUTION_TIMEOUT: Duration = Duration::from_mins(3);
const LOAD_TIMEOUT: Duration = Duration::from_mins(1);
const GENERATION_TIMEOUT: Duration = Duration::from_mins(1);
const UNLOAD_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct RealCycles {
    pub(crate) warmups: Vec<RealProductCycle>,
    pub(crate) samples: Vec<RealProductCycle>,
}

pub(crate) fn run_real_cycles(
    cache_directory: &Path,
    warmup_cycles: u32,
    sample_cycles: u32,
) -> BenchmarkResult<RealCycles> {
    let mut workspace = OutputWorkspace::create("real-product")?;
    let result = run_in_workspace(&workspace, cache_directory, warmup_cycles, sample_cycles);
    let cleanup = workspace.cleanup();
    match result {
        Ok(cycles) => {
            cleanup?;
            Ok(cycles)
        }
        Err(error) => Err(error.with_cleanup(cleanup)),
    }
}

fn run_in_workspace(
    workspace: &OutputWorkspace,
    cache_directory: &Path,
    warmup_cycles: u32,
    sample_cycles: u32,
) -> BenchmarkResult<RealCycles> {
    let mut warmups = Vec::new();
    let mut samples = Vec::new();
    warmups
        .try_reserve_exact(usize_from_u32(warmup_cycles)?)
        .map_err(|error| BenchmarkError::new(format!("real warmup allocation failed: {error}")))?;
    samples
        .try_reserve_exact(usize_from_u32(sample_cycles)?)
        .map_err(|error| BenchmarkError::new(format!("real sample allocation failed: {error}")))?;
    for ordinal in 1..=warmup_cycles {
        warmups.push(run_cycle(
            workspace.database_path("warmup", ordinal),
            cache_directory,
            ordinal,
        )?);
    }
    for ordinal in 1..=sample_cycles {
        samples.push(run_cycle(
            workspace.database_path("sample", ordinal),
            cache_directory,
            ordinal,
        )?);
    }
    validate_cycle_consistency(&warmups, &samples)?;
    Ok(RealCycles { warmups, samples })
}

fn run_cycle(
    database_path: PathBuf,
    cache_directory: &Path,
    ordinal: u32,
) -> BenchmarkResult<RealProductCycle> {
    let before_start = process_memory()?;
    let configuration = application_configuration(database_path, cache_directory);
    let start_started = Instant::now();
    let mut runtime = ApplicationRuntime::start(configuration).map_err(|error| {
        BenchmarkError::new(format!(
            "real-product ApplicationRuntime start failed: {error}"
        ))
    })?;
    let application_start_ns = duration_ns(start_started.elapsed());
    let result = finish_cycle(&mut runtime, ordinal, application_start_ns, before_start);
    match result {
        Ok(cycle) => {
            drop(runtime);
            Ok(cycle)
        }
        Err(error) => Err(error.with_cleanup(shutdown_for_cleanup(&mut runtime))),
    }
}

fn finish_cycle(
    runtime: &mut ApplicationRuntime,
    ordinal: u32,
    application_start_ns: u64,
    before_start: ProcessMemory,
) -> BenchmarkResult<RealProductCycle> {
    let after_start = process_memory()?;
    validate_started(runtime)?;
    let body = run_loaded_lifecycle(runtime)?;
    let shutdown_started = Instant::now();
    runtime.shutdown().map_err(|error| {
        BenchmarkError::new(format!(
            "real-product ApplicationRuntime shutdown failed: {error}"
        ))
    })?;
    let application_shutdown_ns = duration_ns(shutdown_started.elapsed());
    validate_stopped(runtime)?;
    let after_shutdown = process_memory()?;
    Ok(RealProductCycle {
        ordinal,
        application_start_ns,
        resolution_or_download_ns: body.resolution_or_download_ns,
        model_load_ns: body.model_load_ns,
        first_decoded_output: body.first_decoded_output,
        post_first_generated_token_proxy: body.post_first_generated_token_proxy,
        generation: body.generation,
        model_unload_ns: body.model_unload_ns,
        application_shutdown_ns,
        model: body.model,
        process_memory: RealProcessMemory {
            before_start,
            after_start,
            after_resolution: body.after_resolution,
            after_load: body.after_load,
            after_generation_release: body.after_generation_release,
            after_unload: body.after_unload,
            after_shutdown,
        },
    })
}

struct LoadedLifecycle {
    resolution_or_download_ns: u64,
    model_load_ns: u64,
    first_decoded_output: FirstDecodedOutputMeasurement,
    post_first_generated_token_proxy: Option<ProxyThroughputMeasurement>,
    generation: RealGenerationEvidence,
    model_unload_ns: u64,
    model: RealModelEvidence,
    after_resolution: ProcessMemory,
    after_load: ProcessMemory,
    after_generation_release: ProcessMemory,
    after_unload: ProcessMemory,
}

fn run_loaded_lifecycle(runtime: &mut ApplicationRuntime) -> BenchmarkResult<LoadedLifecycle> {
    let selection = ModelSelection::new(REAL_PRODUCT_REPOSITORY, REAL_PRODUCT_REVISION);
    let resolution_started = Instant::now();
    runtime.resolve_model(selection.clone()).map_err(|error| {
        BenchmarkError::new(format!(
            "real-product resolution submission failed: {error}"
        ))
    })?;
    let resolved = await_resolution(runtime, resolution_started)?;
    let resolution_or_download_ns = duration_ns(resolution_started.elapsed());
    validate_resolved(&resolved, &selection)?;
    let after_resolution = process_memory()?;

    let load_started = Instant::now();
    runtime.load_model(&selection).map_err(|error| {
        BenchmarkError::new(format!(
            "real-product model-load submission failed: {error}"
        ))
    })?;
    let loaded = await_load(runtime, load_started)?;
    let model_load_ns = duration_ns(load_started.elapsed());
    let model = validate_loaded(&resolved, &loaded, &selection)?;
    let after_load = process_memory()?;

    let generation = run_generation(runtime)?;
    let after_generation_release = process_memory()?;
    let model_unload_ns = unload(runtime, &loaded)?;
    let after_unload = process_memory()?;

    Ok(LoadedLifecycle {
        resolution_or_download_ns,
        model_load_ns,
        first_decoded_output: generation.first_decoded_output,
        post_first_generated_token_proxy: generation.post_first_generated_token_proxy,
        generation: generation.evidence,
        model_unload_ns,
        model,
        after_resolution,
        after_load,
        after_generation_release,
        after_unload,
    })
}

fn application_configuration(
    database_path: PathBuf,
    cache_directory: &Path,
) -> ApplicationRuntimeConfiguration {
    let mut configuration = ApplicationRuntimeConfiguration::desktop(database_path);
    REAL_PRODUCT_REPOSITORY.clone_into(&mut configuration.defaults.default_repository);
    REAL_PRODUCT_REVISION.clone_into(&mut configuration.defaults.default_revision);
    configuration.defaults.drain_timeout_milliseconds = 10_000;
    configuration.hub.cache_directory = Some(cache_directory.to_path_buf());
    configuration.hub.maximum_retries = 2;
    configuration.token_output_capacity = 1;
    configuration.token_output_record_capacity = 128;
    configuration.timing.runtime_poll = POLL_INTERVAL;
    configuration.timing.hub_worker_poll = POLL_INTERVAL;
    configuration.timing.hub_event_send_timeout = Duration::from_secs(1);
    configuration.timing.hub_command_shutdown_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.runtime_shutdown_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.runtime_shutdown_event_poll = POLL_INTERVAL;
    configuration.timing.runtime_join_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.runtime_join_poll = POLL_INTERVAL;
    configuration.timing.hub_shutdown_timeout = SHUTDOWN_TIMEOUT;
    configuration.timing.hub_shutdown_poll = POLL_INTERVAL;
    configuration
}

fn await_resolution(
    runtime: &mut ApplicationRuntime,
    started: Instant,
) -> BenchmarkResult<ResolvedModel> {
    let deadline = checked_deadline(started, RESOLUTION_TIMEOUT, "real-product resolution")?;
    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelResolved {
                    model,
                    persistence_warning: None,
                } => return Ok(model),
                ApplicationEvent::ModelResolved {
                    persistence_warning: Some(warning),
                    ..
                } => {
                    return Err(BenchmarkError::new(format!(
                        "real-product resolution persistence failed: {warning}"
                    )));
                }
                ApplicationEvent::ModelResolutionFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "real-product resolution failed: {failure}"
                    )));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected during real-product resolution",
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during real-product resolution",
                    ));
                }
                _ => {
                    return Err(BenchmarkError::new(
                        "unexpected application event during real-product resolution",
                    ));
                }
            }
        }
        wait_until(deadline, "real-product resolution/download")?;
    }
}

fn await_load(runtime: &mut ApplicationRuntime, started: Instant) -> BenchmarkResult<LoadedModel> {
    let deadline = checked_deadline(started, LOAD_TIMEOUT, "real-product model load")?;
    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelLoaded { model } => return Ok(model),
                ApplicationEvent::ModelLoadFailed { failure }
                | ApplicationEvent::ModelCompatibilityFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "real-product model load failed: {failure}"
                    )));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during real-product model load",
                    ));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected after real-product resolution",
                    ));
                }
                _ => {
                    return Err(BenchmarkError::new(
                        "unexpected application event during real-product model load",
                    ));
                }
            }
        }
        wait_until(deadline, "real-product model load")?;
    }
}

fn validate_resolved(model: &ResolvedModel, selection: &ModelSelection) -> BenchmarkResult {
    if model.selection() != selection
        || model.selection().repository() != REAL_PRODUCT_REPOSITORY
        || model.selection().revision() != REAL_PRODUCT_REVISION
        || model.identity().repository() != REAL_PRODUCT_REPOSITORY
        || model.identity().commit() != REAL_PRODUCT_REVISION
        || model.engine() != ApplicationEngine::Candle
        || model.source() != ApplicationSource::HuggingFaceHub
        || model.device() != ApplicationDevice::Cpu
        || model.format() != ApplicationModelFormat::Safetensors
        || model.scalar_type() != Some(ApplicationScalarType::F32)
        || !model.is_loadable()
    {
        return Err(BenchmarkError::new(
            "resolved product evidence did not match the hardcoded immutable Candle/Safetensors/F32 selection",
        ));
    }
    Ok(())
}

fn validate_loaded(
    resolved: &ResolvedModel,
    loaded: &LoadedModel,
    selection: &ModelSelection,
) -> BenchmarkResult<RealModelEvidence> {
    if loaded.selection() != selection
        || loaded.identity().repository() != REAL_PRODUCT_REPOSITORY
        || loaded.identity().commit() != REAL_PRODUCT_REVISION
        || loaded.engine() != ApplicationEngine::Candle
        || loaded.source() != ApplicationSource::HuggingFaceHub
        || loaded.device() != ApplicationDevice::Cpu
        || loaded.format() != ApplicationModelFormat::Safetensors
        || loaded.scalar_type() != ApplicationScalarType::F32
        || loaded.vocabulary_size() != resolved.vocabulary_size()
    {
        return Err(BenchmarkError::new(
            "loaded product evidence did not retain the exact immutable resolved identity",
        ));
    }
    Ok(RealModelEvidence {
        repository: loaded.identity().repository().to_owned(),
        requested_revision: loaded.selection().revision().to_owned(),
        immutable_commit: loaded.identity().commit().to_owned(),
        engine: "Candle",
        source: "HuggingFaceHub",
        device: "CPU",
        format: "Safetensors",
        scalar_type: "F32",
        vocabulary_size: loaded.vocabulary_size(),
        maximum_context_tokens: loaded.maximum_context_tokens(),
        maximum_prefill_batch: loaded.maximum_prefill_batch(),
    })
}

struct GenerationMeasurements {
    first_decoded_output: FirstDecodedOutputMeasurement,
    post_first_generated_token_proxy: Option<ProxyThroughputMeasurement>,
    evidence: RealGenerationEvidence,
}

#[expect(
    clippy::too_many_lines,
    reason = "the E1 loop keeps public output, event, usage, cleanup, and timing evidence ordered"
)]
fn run_generation(runtime: &mut ApplicationRuntime) -> BenchmarkResult<GenerationMeasurements> {
    let settings = GenerationSettings {
        maximum_new_tokens: REAL_GENERATION_TOKEN_COUNT,
        temperature: 1.0,
        top_k: 1,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        repetition_window: 0,
        seed: GenerationSeed::Fixed(GENERATION_SEED),
        eos_tokens: Vec::new(),
        stop_sequences: Vec::new(),
    };
    let started = Instant::now();
    let request_id = runtime
        .start_generation(DIRECT_COMPLETION_PROMPT, settings)
        .map_err(|error| {
            BenchmarkError::new(format!(
                "real-product generation submission failed: {error}"
            ))
        })?;
    let deadline = checked_deadline(started, GENERATION_TIMEOUT, "real-product generation")?;
    let mut output = ApplicationOutputObservation::new(request_id);
    let mut saw_started = false;
    let mut terminal_event = None;
    let mut first_decoded_output = None;
    let mut post_window_started = None;
    let mut post_window_initial_usage = None;
    let mut post_first_generated_token_proxy = None;

    while terminal_event.is_none() {
        let new_text_bytes = output.pull(runtime)?;
        if first_decoded_output.is_none() && new_text_bytes > 0 {
            let observed = Instant::now();
            let usage = current_usage(runtime).ok_or_else(|| {
                BenchmarkError::new(
                    "first decoded output was observed without public generation usage",
                )
            })?;
            first_decoded_output = Some(FirstDecodedOutputMeasurement {
                duration_ns: duration_ns(observed.saturating_duration_since(started)),
                first_fragment_bytes: new_text_bytes,
                usage_at_observation: usage_record(usage),
            });
            post_window_started = Some(observed);
            post_window_initial_usage = Some(usage.generated_tokens);
        }
        if post_first_generated_token_proxy.is_none()
            && let (Some(window_started), Some(initial_usage), Some(usage)) = (
                post_window_started,
                post_window_initial_usage,
                current_usage(runtime),
            )
        {
            let target = initial_usage.saturating_add(u64::from(REAL_POST_FIRST_TOKEN_WINDOW));
            if usage.generated_tokens >= target {
                let elapsed = window_started.elapsed();
                post_first_generated_token_proxy = Some(ProxyThroughputMeasurement {
                    label: "real-product public-E1 post-first-output short-window integration proxy; not production steady state",
                    duration_ns: duration_ns(elapsed),
                    token_count: REAL_POST_FIRST_TOKEN_WINDOW,
                    tokens_per_second: throughput(REAL_POST_FIRST_TOKEN_WINDOW, elapsed),
                });
            }
        }

        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::GenerationStarted {
                    request_id: event_request,
                } if event_request == request_id => saw_started = true,
                ApplicationEvent::GenerationFinished { terminal }
                    if terminal.request_id == request_id =>
                {
                    terminal_event = Some(terminal);
                }
                ApplicationEvent::GenerationCleanupPending {
                    exhausted, failure, ..
                } => {
                    return Err(BenchmarkError::new(format!(
                        "real-product generation cleanup remained pending (exhausted={exhausted}): {failure}"
                    )));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during real-product generation",
                    ));
                }
                ApplicationEvent::GenerationCancellationRequested { .. }
                | ApplicationEvent::GenerationCancellationFailed { .. } => {
                    return Err(BenchmarkError::new(
                        "normal real-product generation unexpectedly entered cancellation control",
                    ));
                }
                _ => {
                    return Err(BenchmarkError::new(
                        "unexpected application event during real-product generation",
                    ));
                }
            }
        }
        if terminal_event.is_none() {
            wait_until(deadline, "real-product generation release")?;
        }
    }

    let _remaining_text_bytes = output.pull(runtime)?;
    let terminal = terminal_event
        .ok_or_else(|| BenchmarkError::new("real-product terminal event disappeared"))?;
    let first_decoded_output = first_decoded_output.ok_or_else(|| {
        BenchmarkError::new("real-product generation produced no decoded public output")
    })?;
    if post_first_generated_token_proxy.is_none()
        && let (Some(window_started), Some(initial_usage)) =
            (post_window_started, post_window_initial_usage)
    {
        let delta = terminal
            .usage
            .generated_tokens
            .saturating_sub(initial_usage);
        if delta > 0 {
            let observed_tokens = u32::try_from(delta)
                .map_err(|_| BenchmarkError::new("post-first generated usage conversion failed"))?;
            let elapsed = window_started.elapsed();
            post_first_generated_token_proxy = Some(ProxyThroughputMeasurement {
                label: "real-product public-E1 observable post-first-output integration proxy; not production steady state",
                duration_ns: duration_ns(elapsed),
                token_count: observed_tokens,
                tokens_per_second: throughput(observed_tokens, elapsed),
            });
        }
    }
    output.validate(&terminal, saw_started)?;
    if runtime.state().active_generation().is_some()
        || runtime.state().last_generation() != Some(&terminal)
    {
        return Err(BenchmarkError::new(
            "application state did not retain the released real-product terminal cleanly",
        ));
    }

    Ok(GenerationMeasurements {
        first_decoded_output,
        post_first_generated_token_proxy,
        evidence: RealGenerationEvidence {
            decoded_byte_count: output.decoded_byte_count,
            decoded_text_record_count: output.decoded_text_record_count,
            terminal: "finished:token-limit",
            released: "finished:token-limit",
            terminal_usage: usage_record(terminal.usage),
            cleanup_pending_observed: output.cleanup_pending_observed,
            cleanup_exhausted_observed: output.cleanup_exhausted_observed,
        },
    })
}

struct ApplicationOutputObservation {
    request_id: RequestId,
    decoded_byte_count: usize,
    decoded_text_record_count: u32,
    terminal: Option<GenerationTerminalKind>,
    released: Option<GenerationTerminalKind>,
    cleanup_pending_observed: bool,
    cleanup_exhausted_observed: bool,
}

impl ApplicationOutputObservation {
    const fn new(request_id: RequestId) -> Self {
        Self {
            request_id,
            decoded_byte_count: 0,
            decoded_text_record_count: 0,
            terminal: None,
            released: None,
            cleanup_pending_observed: false,
            cleanup_exhausted_observed: false,
        }
    }

    fn pull(&mut self, runtime: &mut ApplicationRuntime) -> BenchmarkResult<usize> {
        runtime
            .pull_output(|batch| {
                let mut new_text_bytes = 0_usize;
                for record in batch.records() {
                    if record.request_id != self.request_id {
                        return Err(BenchmarkError::new(
                            "decoded product output addressed an unexpected request",
                        ));
                    }
                    match record.kind {
                        ApplicationOutputRecordKind::Text(_) => {
                            let fragment = batch.text_for(record).ok_or_else(|| {
                                BenchmarkError::new(
                                    "decoded product output contained an invalid UTF-8 range",
                                )
                            })?;
                            let bytes = fragment.len();
                            new_text_bytes = new_text_bytes.saturating_add(bytes);
                            self.decoded_byte_count = self.decoded_byte_count.saturating_add(bytes);
                            self.decoded_text_record_count = self
                                .decoded_text_record_count
                                .checked_add(1)
                                .ok_or_else(|| {
                                    BenchmarkError::new("decoded text record count overflowed")
                                })?;
                        }
                        ApplicationOutputRecordKind::State(state) => match state {
                            ApplicationOutputState::Terminal(kind) => {
                                record_terminal_kind(&mut self.terminal, kind, "Terminal")?;
                            }
                            ApplicationOutputState::Released(kind) => {
                                record_terminal_kind(&mut self.released, kind, "Released")?;
                            }
                            ApplicationOutputState::CleanupPending => {
                                self.cleanup_pending_observed = true;
                                return Err(BenchmarkError::new(
                                    "real-product output entered CleanupPending",
                                ));
                            }
                            ApplicationOutputState::CleanupExhausted => {
                                self.cleanup_exhausted_observed = true;
                                return Err(BenchmarkError::new(
                                    "real-product output entered CleanupExhausted",
                                ));
                            }
                            ApplicationOutputState::Yielded(_) => {}
                        },
                    }
                }
                Ok(new_text_bytes)
            })
            .map_err(|error| {
                BenchmarkError::new(format!("decoded product output pull failed: {error}"))
            })?
    }

    fn validate(&self, terminal: &GenerationTerminal, saw_started: bool) -> BenchmarkResult {
        let expected = GenerationTerminalKind::Finished(FinishReason::TokenLimit);
        if !saw_started
            || self.terminal != Some(expected)
            || self.released != Some(expected)
            || self.cleanup_pending_observed
            || self.cleanup_exhausted_observed
            || self.decoded_byte_count == 0
            || self.decoded_text_record_count == 0
            || terminal.outcome != GenerationTerminalOutcome::Finished(FinishReason::TokenLimit)
            || terminal.usage.prompt_tokens == 0
            || terminal.usage.generated_tokens != u64::from(REAL_GENERATION_TOKEN_COUNT)
        {
            return Err(BenchmarkError::new(
                "real-product generation lacked matching started, decoded, usage, Terminal, Released, or clean-accounting evidence",
            ));
        }
        Ok(())
    }
}

fn record_terminal_kind(
    destination: &mut Option<GenerationTerminalKind>,
    kind: GenerationTerminalKind,
    label: &str,
) -> BenchmarkResult {
    if destination.is_some_and(|existing| existing != kind) {
        return Err(BenchmarkError::new(format!(
            "real-product output published inconsistent {label} states"
        )));
    }
    *destination = Some(kind);
    Ok(())
}

fn current_usage(runtime: &ApplicationRuntime) -> Option<GenerationUsage> {
    runtime
        .state()
        .active_generation()
        .map(|summary| summary.usage)
        .or_else(|| {
            runtime
                .state()
                .last_generation()
                .map(|terminal| terminal.usage)
        })
}

const fn usage_record(usage: GenerationUsage) -> UsageRecord {
    UsageRecord {
        prompt_tokens: usage.prompt_tokens,
        generated_tokens: usage.generated_tokens,
    }
}

fn unload(runtime: &mut ApplicationRuntime, loaded: &LoadedModel) -> BenchmarkResult<u64> {
    let started = Instant::now();
    runtime
        .unload_model_with_behavior(ModelUnloadBehavior::RejectIfBusy)
        .map_err(|error| {
            BenchmarkError::new(format!("real-product unload submission failed: {error}"))
        })?;
    let deadline = checked_deadline(started, UNLOAD_TIMEOUT, "real-product unload")?;
    loop {
        if let Some(event) = runtime.poll_event() {
            let event_elapsed = started.elapsed();
            match event {
                ApplicationEvent::ModelUnloaded {
                    handle,
                    cancelled_requests: 0,
                } if handle == loaded.handle() => {
                    if runtime.state().loaded().is_some()
                        || runtime.state().active_generation().is_some()
                        || runtime.state().activity() != ApplicationActivity::Idle
                    {
                        return Err(BenchmarkError::new(
                            "real-product unload retained loaded or active application state",
                        ));
                    }
                    return Ok(duration_ns(event_elapsed));
                }
                ApplicationEvent::ModelDraining { handle } if handle == loaded.handle() => {}
                ApplicationEvent::ModelUnloadFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "real-product model unload failed: {failure}"
                    )));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during real-product unload",
                    ));
                }
                _ => {
                    return Err(BenchmarkError::new(
                        "unexpected application event or unload accounting during real-product unload",
                    ));
                }
            }
        }
        wait_until(deadline, "real-product unload")?;
    }
}

fn validate_started(runtime: &ApplicationRuntime) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || !state.hub_available()
        || !state.inference_available()
        || state.loaded().is_some()
        || state.active_generation().is_some()
    {
        return Err(BenchmarkError::new(
            "real-product ApplicationRuntime did not start in clean idle state",
        ));
    }
    Ok(())
}

fn validate_stopped(runtime: &ApplicationRuntime) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::ShuttingDown
        || state.hub_available()
        || state.inference_available()
        || state.loaded().is_some()
        || state.active_generation().is_some()
    {
        return Err(BenchmarkError::new(
            "real-product ApplicationRuntime shutdown did not stop both workers",
        ));
    }
    Ok(())
}

fn validate_cycle_consistency(
    warmups: &[RealProductCycle],
    samples: &[RealProductCycle],
) -> BenchmarkResult {
    let reference = samples
        .first()
        .or_else(|| warmups.first())
        .ok_or_else(|| BenchmarkError::new("real-product cycle set is empty"))?;
    for cycle in warmups.iter().chain(samples) {
        if cycle.model.repository != reference.model.repository
            || cycle.model.requested_revision != reference.model.requested_revision
            || cycle.model.immutable_commit != reference.model.immutable_commit
            || cycle.model.vocabulary_size != reference.model.vocabulary_size
            || cycle.model.maximum_context_tokens != reference.model.maximum_context_tokens
            || cycle.generation.terminal_usage.prompt_tokens
                != reference.generation.terminal_usage.prompt_tokens
        {
            return Err(BenchmarkError::new(
                "real-product cycles did not preserve identical model identity and prompt usage",
            ));
        }
    }
    Ok(())
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
                "{operation} exceeded its hard operational timeout; no timing threshold was applied"
            ))
        })?;
    std::thread::sleep(POLL_INTERVAL.min(remaining));
    Ok(())
}

fn usize_from_u32(value: u32) -> BenchmarkResult<usize> {
    usize::try_from(value)
        .map_err(|_| BenchmarkError::new("cycle count conversion to usize failed"))
}
