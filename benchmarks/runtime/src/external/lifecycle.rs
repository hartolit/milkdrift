//! One authoritative public-E1 lifecycle for the pinned external CPU model.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationDevice, ApplicationEngine, ApplicationEvent,
    ApplicationModelFormat, ApplicationOutputRecordKind, ApplicationOutputState,
    ApplicationRuntime, ApplicationRuntimeConfiguration, ApplicationScalarType, ApplicationSource,
    ChatCompatibility, ConversationProvenance, ConversationRole, ConversationTokenEstimate,
    GenerationSeed, GenerationSettings, GenerationTerminal, GenerationTerminalKind,
    GenerationTerminalOutcome, LoadedModel, ModelSelection, ModelUnloadBehavior,
    PromptCompatibilityProfile, ResolvedModel, ResponseAttemptState,
};
use domain_contracts::{FinishReason, GenerationUsage, RequestId};

use crate::e1::cleanup_runtime_after_failure;
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::memory::{ProcessMemory, process_memory};
use crate::report::{
    ChatProofResult, ConversationProof, DirectCompletionResults, DirectCompletionSample,
    DirectCompletionSummary, DirectCompletionWarmupResult, ExternalMemoryCheckpoints,
    ExternalResults, ExternalShutdownResult, ExternalUnloadResult, GenerationOutcomeMatch,
    ShutdownOwnershipState, ShutdownWorkerState, duration_ns,
};

pub(super) const MODEL_REPOSITORY: &str = "TinyLlama/TinyLlama-1.1B-Chat-v1.0";
pub(super) const MODEL_REVISION: &str = "fe8a4ea1ffedaf415f4da2f062534de366a451e6";
pub(super) const MODEL_ARCHITECTURE: &str = "Llama";
pub(super) const CHAT_MESSAGE: &str =
    "Reply with one short sentence confirming that local inference is working.";
pub(super) const CHAT_MESSAGE_IDENTIFIER: &str = "tinyllama-local-inference-chat-proof-v1";
pub(super) const DIRECT_COMPLETION_PROMPT: &str =
    "The following is a concise explanation of deterministic resource cleanup in systems software:";
pub(super) const DIRECT_COMPLETION_PROMPT_IDENTIFIER: &str =
    "deterministic-resource-cleanup-completion-v1";
pub(super) const CHAT_MAXIMUM_NEW_TOKENS: u32 = 24;
pub(super) const DIRECT_MAXIMUM_NEW_TOKENS: u32 = 32;
pub(super) const WARMUP_COUNT: u32 = 1;
pub(super) const SAMPLE_COUNT: u32 = 3;
pub(super) const FIXED_SEED: u64 = 39;

const HUB_RETRIES: usize = 2;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const RESOLUTION_TIMEOUT: Duration = Duration::from_mins(30);
const LOAD_TIMEOUT: Duration = Duration::from_mins(10);
const GENERATION_TIMEOUT: Duration = Duration::from_mins(10);
const UNLOAD_TIMEOUT: Duration = Duration::from_mins(2);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct LifecycleEvidence {
    pub(super) resolved_commit: String,
    pub(super) scalar_type: &'static str,
    pub(super) vocabulary_size: u32,
    pub(super) maximum_context_tokens: u32,
    pub(super) maximum_prefill_batch: u32,
    pub(super) results: ExternalResults,
}

pub(super) fn run(
    database_path: PathBuf,
    cache_directory: &Path,
) -> BenchmarkResult<LifecycleEvidence> {
    let rss_before_start = process_memory()?;
    let configuration = application_configuration(database_path, cache_directory);
    eprintln!("starting ApplicationRuntime for the pinned external CPU baseline");
    let started = Instant::now();
    let runtime = ApplicationRuntime::start(configuration).map_err(|error| {
        BenchmarkError::new(format!(
            "external ApplicationRuntime startup failed: {error}"
        ))
    })?;
    let startup_elapsed = started.elapsed();
    finish_or_cleanup(runtime, startup_elapsed, rss_before_start)
}

fn finish_or_cleanup(
    mut runtime: ApplicationRuntime,
    startup_elapsed: Duration,
    rss_before_start: ProcessMemory,
) -> BenchmarkResult<LifecycleEvidence> {
    match execute_lifecycle(&mut runtime, startup_elapsed, rss_before_start) {
        Ok(evidence) => Ok(evidence),
        Err(error) => Err(cleanup_runtime_after_failure(runtime, error)),
    }
}

fn execute_lifecycle(
    runtime: &mut ApplicationRuntime,
    startup_elapsed: Duration,
    rss_before_start: ProcessMemory,
) -> BenchmarkResult<LifecycleEvidence> {
    validate_started(runtime)?;
    let rss_after_start = process_memory()?;
    let selection = ModelSelection::new(MODEL_REPOSITORY, MODEL_REVISION);

    eprintln!("resolving the exact immutable Hugging Face model revision");
    let (resolved, resolution_elapsed) = resolve_model(runtime, &selection)?;
    validate_resolved(runtime, &resolved, &selection)?;
    let rss_after_resolution = process_memory()?;

    eprintln!("loading the resolved model through public E1 on CPU");
    let (loaded, load_elapsed) = load_model(runtime, &selection)?;
    validate_loaded(runtime, &loaded, &resolved, &selection)?;
    let rss_after_load = process_memory()?;

    eprintln!("running the exact compatible-chat proof");
    let mut chat_compatibility = run_chat_proof(runtime, &loaded)?;
    runtime.clear_conversation().map_err(|error| {
        BenchmarkError::new(format!(
            "compatible-chat conversation could not be cleared after release: {error}"
        ))
    })?;
    if !runtime.conversation().is_empty() || runtime.context_diagnostics().is_some() {
        return Err(BenchmarkError::new(
            "compatible-chat conversation or diagnostics remained after public clear",
        ));
    }
    chat_compatibility.conversation.cleared = true;

    let direct_completion = run_direct_workload(runtime)?;

    eprintln!("unloading the model with RejectIfBusy after all releases");
    let unload = unload_model(runtime, &loaded)?;
    let rss_after_unload = process_memory()?;

    eprintln!("performing explicit bounded ApplicationRuntime shutdown");
    let shutdown_started = Instant::now();
    runtime.shutdown().map_err(|error| {
        BenchmarkError::new(format!(
            "explicit bounded external ApplicationRuntime shutdown failed: {error}"
        ))
    })?;
    let shutdown_elapsed = shutdown_started.elapsed();
    validate_stopped(runtime)?;
    let rss_after_shutdown = process_memory()?;

    Ok(LifecycleEvidence {
        resolved_commit: resolved.identity().commit().to_owned(),
        scalar_type: scalar_label(loaded.scalar_type()),
        vocabulary_size: loaded.vocabulary_size(),
        maximum_context_tokens: loaded.maximum_context_tokens(),
        maximum_prefill_batch: loaded.maximum_prefill_batch(),
        results: ExternalResults {
            application_startup_ns: duration_ns(startup_elapsed),
            resolution_ns: duration_ns(resolution_elapsed),
            load_ns: duration_ns(load_elapsed),
            chat_compatibility,
            direct_completion: direct_completion.results,
            unload,
            shutdown: ExternalShutdownResult {
                duration_ns: duration_ns(shutdown_elapsed),
                shutdown_returned_cleanly: true,
                workers: ShutdownWorkerState {
                    hub_unavailable: true,
                    inference_unavailable: true,
                },
                ownership: ShutdownOwnershipState {
                    loaded_model_absent: true,
                    active_generation_absent: true,
                },
                temporary_workspace_removed: false,
            },
            memory: ExternalMemoryCheckpoints {
                before_application_start: rss_before_start,
                after_application_start: rss_after_start,
                after_resolution: rss_after_resolution,
                after_load: rss_after_load,
                after_warmup_release: direct_completion.rss_after_warmup_release,
                after_unload: rss_after_unload,
                after_shutdown: rss_after_shutdown,
            },
        },
    })
}

fn application_configuration(
    database_path: PathBuf,
    cache_directory: &Path,
) -> ApplicationRuntimeConfiguration {
    let mut configuration = ApplicationRuntimeConfiguration::desktop(database_path);
    MODEL_REPOSITORY.clone_into(&mut configuration.defaults.default_repository);
    MODEL_REVISION.clone_into(&mut configuration.defaults.default_revision);
    configuration.defaults.drain_timeout_milliseconds = 10_000;
    configuration.hub.cache_directory = Some(cache_directory.to_path_buf());
    configuration.hub.maximum_retries = HUB_RETRIES;
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

fn validate_started(runtime: &ApplicationRuntime) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || !state.hub_available()
        || !state.inference_available()
        || state.resolved().is_some()
        || state.loaded().is_some()
        || state.active_generation().is_some()
        || state.last_generation().is_some()
        || !runtime.conversation().is_empty()
        || runtime.context_diagnostics().is_some()
    {
        return Err(BenchmarkError::new(
            "external ApplicationRuntime start returned non-clean initial E1 state",
        ));
    }
    Ok(())
}

fn resolve_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
) -> BenchmarkResult<(ResolvedModel, Duration)> {
    let started = Instant::now();
    runtime.resolve_model(selection.clone()).map_err(|error| {
        BenchmarkError::new(format!(
            "exact Hub resolution could not be submitted for {MODEL_REPOSITORY}@{MODEL_REVISION}: {error}"
        ))
    })?;
    let deadline = checked_deadline(RESOLUTION_TIMEOUT, "immutable Hub resolution")?;
    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelResolved {
                    model,
                    persistence_warning: None,
                } => return Ok((model, started.elapsed())),
                ApplicationEvent::ModelResolved {
                    persistence_warning: Some(warning),
                    ..
                } => {
                    return Err(BenchmarkError::new(format!(
                        "Hub resolution succeeded but immutable catalogue persistence reported a warning: {warning}"
                    )));
                }
                ApplicationEvent::ModelResolutionFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "exact Hub resolution failed for {MODEL_REPOSITORY}@{MODEL_REVISION}: {failure}"
                    )));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected during exact immutable resolution",
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during exact immutable resolution",
                    ));
                }
                unexpected => {
                    return Err(BenchmarkError::new(format!(
                        "unexpected application event during immutable resolution: {unexpected:?}"
                    )));
                }
            }
        }
        wait_for_next_poll(deadline, "immutable Hub resolution")?;
    }
}

fn validate_resolved(
    runtime: &ApplicationRuntime,
    model: &ResolvedModel,
    selection: &ModelSelection,
) -> BenchmarkResult {
    if model.selection() != selection
        || model.selection().repository() != MODEL_REPOSITORY
        || model.selection().revision() != MODEL_REVISION
        || model.identity().repository() != MODEL_REPOSITORY
        || model.identity().commit() != MODEL_REVISION
    {
        return Err(BenchmarkError::new(format!(
            "resolved model did not retain exact selection and immutable identity: selection={:?}, identity={}@{}",
            model.selection(),
            model.identity().repository(),
            model.identity().commit()
        )));
    }
    if model.engine() != ApplicationEngine::Candle
        || model.source() != ApplicationSource::HuggingFaceHub
        || model.device() != ApplicationDevice::Cpu
        || model.format() != ApplicationModelFormat::Safetensors
        || !model.is_loadable()
        || model.scalar_type().is_none()
        || model.vocabulary_size() == 0
        || model.chat_compatibility()
            != ChatCompatibility::Supported(PromptCompatibilityProfile::TinyLlamaChatV1)
    {
        return Err(BenchmarkError::new(format!(
            "resolved execution evidence did not match Candle/Hub/CPU/Safetensors/Llama with supported scalar, tokenizer vocabulary, loadability, and exact TinyLlama chat compatibility: {model:?}"
        )));
    }
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.resolved() != Some(model)
        || state.loaded().is_some()
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
    {
        return Err(BenchmarkError::new(
            "public E1 state did not retain the clean exact resolution",
        ));
    }
    Ok(())
}

fn load_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
) -> BenchmarkResult<(LoadedModel, Duration)> {
    let started = Instant::now();
    runtime.load_model(selection).map_err(|error| {
        BenchmarkError::new(format!(
            "exact Candle model load could not be submitted: {error}"
        ))
    })?;
    let deadline = checked_deadline(LOAD_TIMEOUT, "Candle model load")?;
    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelLoaded { model } => return Ok((model, started.elapsed())),
                ApplicationEvent::ModelLoadFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "exact Candle model load failed: {failure}"
                    )));
                }
                ApplicationEvent::ModelCompatibilityFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "resolved and loaded model compatibility failed: {failure}"
                    )));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected while the exact model was loading",
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected while the exact model was loading",
                    ));
                }
                unexpected => {
                    return Err(BenchmarkError::new(format!(
                        "unexpected application event during exact model load: {unexpected:?}"
                    )));
                }
            }
        }
        wait_for_next_poll(deadline, "Candle model load")?;
    }
}

fn validate_loaded(
    runtime: &ApplicationRuntime,
    loaded: &LoadedModel,
    resolved: &ResolvedModel,
    selection: &ModelSelection,
) -> BenchmarkResult {
    let resolved_scalar = resolved.scalar_type().ok_or_else(|| {
        BenchmarkError::new("resolved scalar evidence disappeared before loaded validation")
    })?;
    if loaded.selection() != selection
        || loaded.identity() != resolved.identity()
        || loaded.identity().repository() != MODEL_REPOSITORY
        || loaded.identity().commit() != MODEL_REVISION
        || loaded.engine() != ApplicationEngine::Candle
        || loaded.source() != ApplicationSource::HuggingFaceHub
        || loaded.device() != ApplicationDevice::Cpu
        || loaded.format() != ApplicationModelFormat::Safetensors
        || loaded.scalar_type() != resolved_scalar
        || loaded.vocabulary_size() == 0
        || loaded.vocabulary_size() != resolved.vocabulary_size()
        || loaded.maximum_context_tokens() == 0
        || loaded.maximum_prefill_batch() == 0
    {
        return Err(BenchmarkError::new(format!(
            "loaded execution evidence did not retain the exact resolution or expected Candle/Hub/CPU/Safetensors/Llama composition: {loaded:?}"
        )));
    }
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.resolved() != Some(resolved)
        || state.loaded() != Some(loaded)
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
        || !runtime.can_submit_chat_message()
    {
        return Err(BenchmarkError::new(
            "public E1 state did not retain the exact loaded model with compatible-chat admission",
        ));
    }
    Ok(())
}

struct DirectWorkloadEvidence {
    results: DirectCompletionResults,
    rss_after_warmup_release: ProcessMemory,
}

fn run_direct_workload(
    runtime: &mut ApplicationRuntime,
) -> BenchmarkResult<DirectWorkloadEvidence> {
    eprintln!("running one controlled direct-completion warmup");
    let warmup = run_direct_completion(runtime, ExpectedCompletion::DirectTokenLimit)?;
    let warmup = DirectCompletionWarmupResult {
        decoded_byte_count: warmup.decoded_byte_count,
        prompt_tokens: warmup.usage.prompt_tokens,
        generated_tokens: warmup.usage.generated_tokens,
        terminal_kind: warmup.terminal_kind,
        clean_release: true,
    };
    let rss_after_warmup_release = process_memory()?;

    eprintln!("running three sequential controlled direct-completion samples");
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(
            usize::try_from(SAMPLE_COUNT).map_err(|_| {
                BenchmarkError::new("external sample count conversion to usize failed")
            })?,
        )
        .map_err(|error| BenchmarkError::new(format!("sample allocation failed: {error}")))?;
    for ordinal in 1..=SAMPLE_COUNT {
        eprintln!("running direct-completion sample {ordinal} of {SAMPLE_COUNT}");
        let evidence = run_direct_completion(runtime, ExpectedCompletion::DirectTokenLimit)?;
        let process_memory_after_release = process_memory()?;
        samples.push(sample_record(
            ordinal,
            &evidence,
            process_memory_after_release,
        )?);
    }
    let summary = summarize_samples(&samples)?;
    Ok(DirectWorkloadEvidence {
        results: DirectCompletionResults {
            warmup,
            samples,
            summary,
        },
        rss_after_warmup_release,
    })
}

fn run_chat_proof(
    runtime: &mut ApplicationRuntime,
    loaded: &LoadedModel,
) -> BenchmarkResult<ChatProofResult> {
    let submitted = Instant::now();
    let request_id = runtime
        .submit_user_message(CHAT_MESSAGE, generation_settings(CHAT_MAXIMUM_NEW_TOKENS))
        .map_err(|error| {
            BenchmarkError::new(format!(
                "exact compatible-chat request could not be submitted: {error}"
            ))
        })?;
    let evidence = await_generation(runtime, request_id, submitted, ExpectedCompletion::Chat)?;
    validate_chat_conversation(runtime, loaded, &evidence)?;
    Ok(ChatProofResult {
        decoded_byte_count: evidence.decoded_byte_count,
        prompt_tokens: evidence.usage.prompt_tokens,
        generated_tokens: evidence.usage.generated_tokens,
        terminal_kind: evidence.terminal_kind,
        outcome_match: GenerationOutcomeMatch {
            terminal_state_matched: true,
            released_state_matched: true,
            terminal_event_matched: true,
        },
        conversation: ConversationProof {
            validated: true,
            cleared: false,
        },
    })
}

fn validate_chat_conversation(
    runtime: &ApplicationRuntime,
    loaded: &LoadedModel,
    evidence: &GenerationEvidence,
) -> BenchmarkResult {
    let records = runtime.conversation();
    if records.len() != 2 {
        return Err(BenchmarkError::new(format!(
            "compatible-chat proof retained {} conversation records instead of one user and one assistant record",
            records.len()
        )));
    }
    let user = records
        .first()
        .ok_or_else(|| BenchmarkError::new("compatible-chat user record disappeared"))?;
    let assistant = records
        .get(1)
        .ok_or_else(|| BenchmarkError::new("compatible-chat assistant record disappeared"))?;
    if user.role != ConversationRole::User
        || user.provenance != ConversationProvenance::User
        || user.content != CHAT_MESSAGE
        || user.response_attempt.is_some()
        || assistant.role != ConversationRole::Assistant
        || assistant.provenance != ConversationProvenance::Model
        || assistant.content.is_empty()
        || u64::try_from(assistant.content.len()).ok() != Some(evidence.decoded_byte_count)
        || assistant.token_estimate
            != ConversationTokenEstimate::Generated(
                u32::try_from(evidence.usage.generated_tokens).map_err(|_| {
                    BenchmarkError::new("chat generated usage could not fit its token estimate")
                })?,
            )
        || !assistant.is_active_context()
    {
        return Err(BenchmarkError::new(
            "compatible-chat conversation did not retain the expected user turn and non-empty active model response",
        ));
    }
    let attempt = assistant.response_attempt.as_ref().ok_or_else(|| {
        BenchmarkError::new("compatible-chat assistant record had no response-attempt provenance")
    })?;
    if attempt.responding_to != user.id
        || attempt.superseded
        || attempt.state != ResponseAttemptState::Completed(evidence.finish_reason)
    {
        return Err(BenchmarkError::new(format!(
            "compatible-chat assistant attempt did not match the released terminal outcome: {attempt:?}"
        )));
    }
    let diagnostics = runtime.context_diagnostics().ok_or_else(|| {
        BenchmarkError::new("compatible-chat context diagnostics were not retained")
    })?;
    if diagnostics.actual_input_tokens == 0
        || u64::from(diagnostics.actual_input_tokens) != evidence.usage.prompt_tokens
        || diagnostics.reserved_output_tokens != CHAT_MAXIMUM_NEW_TOKENS
        || diagnostics.maximum_context_tokens != loaded.maximum_context_tokens()
        || diagnostics
            .actual_input_tokens
            .checked_add(diagnostics.reserved_output_tokens)
            .is_none_or(|required| required > diagnostics.maximum_context_tokens)
    {
        return Err(BenchmarkError::new(format!(
            "compatible-chat context diagnostics were incomplete: {diagnostics:?}"
        )));
    }
    Ok(())
}

fn run_direct_completion(
    runtime: &mut ApplicationRuntime,
    expectation: ExpectedCompletion,
) -> BenchmarkResult<GenerationEvidence> {
    let submitted = Instant::now();
    let request_id = runtime
        .start_generation(
            DIRECT_COMPLETION_PROMPT,
            generation_settings(DIRECT_MAXIMUM_NEW_TOKENS),
        )
        .map_err(|error| {
            BenchmarkError::new(format!(
                "controlled direct completion could not be submitted: {error}"
            ))
        })?;
    await_generation(runtime, request_id, submitted, expectation)
}

fn generation_settings(maximum_new_tokens: u32) -> GenerationSettings {
    GenerationSettings {
        maximum_new_tokens,
        temperature: 1.0,
        top_k: 1,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        repetition_window: 0,
        seed: GenerationSeed::Fixed(FIXED_SEED),
        eos_tokens: Vec::new(),
        stop_sequences: Vec::new(),
    }
}

#[derive(Clone, Copy)]
enum ExpectedCompletion {
    Chat,
    DirectTokenLimit,
}

struct GenerationEvidence {
    finish_reason: FinishReason,
    terminal_kind: &'static str,
    usage: GenerationUsage,
    decoded_byte_count: u64,
    submission_to_started: Duration,
    submission_to_first_decoded: Duration,
    submission_to_terminal_event: Duration,
    submission_to_release: Duration,
}

struct GenerationObserver {
    request_id: RequestId,
    submitted: Instant,
    started_at: Option<Instant>,
    first_decoded_at: Option<Instant>,
    terminal_output: Option<GenerationTerminalKind>,
    released_output: Option<GenerationTerminalKind>,
    released_at: Option<Instant>,
    terminal_event: Option<GenerationTerminal>,
    terminal_event_at: Option<Instant>,
    decoded_byte_count: u64,
}

impl GenerationObserver {
    const fn new(request_id: RequestId, submitted: Instant) -> Self {
        Self {
            request_id,
            submitted,
            started_at: None,
            first_decoded_at: None,
            terminal_output: None,
            released_output: None,
            released_at: None,
            terminal_event: None,
            terminal_event_at: None,
            decoded_byte_count: 0,
        }
    }

    fn pull(&mut self, runtime: &mut ApplicationRuntime) -> BenchmarkResult {
        runtime
            .pull_output(|batch| {
                for record in batch.records() {
                    let fragment = match record.kind {
                        ApplicationOutputRecordKind::Text(_) => batch.text_for(record),
                        ApplicationOutputRecordKind::State(_) => None,
                    };
                    self.observe_output(record.request_id, record.kind, fragment)?;
                }
                Ok(())
            })
            .map_err(|error| {
                BenchmarkError::new(format!(
                    "bounded decoded application output could not be pulled: {error}"
                ))
            })?
    }

    fn observe_output(
        &mut self,
        request_id: RequestId,
        kind: ApplicationOutputRecordKind,
        fragment: Option<&str>,
    ) -> BenchmarkResult {
        if request_id != self.request_id {
            return Err(BenchmarkError::new(format!(
                "generation output addressed request {}, expected {}",
                request_id.get(),
                self.request_id.get()
            )));
        }
        let observed_at = Instant::now();
        match kind {
            ApplicationOutputRecordKind::Text(_) => {
                let fragment = fragment.ok_or_else(|| {
                    BenchmarkError::new("decoded output contained an invalid UTF-8 text range")
                })?;
                if !fragment.is_empty() {
                    let bytes = u64::try_from(fragment.len()).map_err(|_| {
                        BenchmarkError::new("decoded fragment length conversion failed")
                    })?;
                    self.decoded_byte_count = self
                        .decoded_byte_count
                        .checked_add(bytes)
                        .ok_or_else(|| BenchmarkError::new("decoded byte count overflowed"))?;
                    if self.first_decoded_at.is_none() {
                        self.first_decoded_at = Some(observed_at);
                    }
                }
            }
            ApplicationOutputRecordKind::State(ApplicationOutputState::Yielded(_)) => {}
            ApplicationOutputRecordKind::State(ApplicationOutputState::Terminal(kind)) => {
                if self.terminal_output.replace(kind).is_some() {
                    return Err(BenchmarkError::new(
                        "generation published more than one terminal output state",
                    ));
                }
            }
            ApplicationOutputRecordKind::State(ApplicationOutputState::CleanupPending) => {
                return Err(BenchmarkError::new(
                    "generation output entered cleanup-pending state",
                ));
            }
            ApplicationOutputRecordKind::State(ApplicationOutputState::CleanupExhausted) => {
                return Err(BenchmarkError::new(
                    "generation output exhausted cleanup while retaining ownership",
                ));
            }
            ApplicationOutputRecordKind::State(ApplicationOutputState::Released(kind)) => {
                if self.released_output.replace(kind).is_some() {
                    return Err(BenchmarkError::new(
                        "generation published more than one released output state",
                    ));
                }
                self.released_at = Some(observed_at);
            }
        }
        Ok(())
    }

    fn observe_started(&mut self, request_id: RequestId) -> BenchmarkResult {
        if request_id != self.request_id {
            return Err(BenchmarkError::new(format!(
                "GenerationStarted addressed request {}, expected {}",
                request_id.get(),
                self.request_id.get()
            )));
        }
        if self.started_at.replace(Instant::now()).is_some() {
            return Err(BenchmarkError::new(
                "generation published more than one matching GenerationStarted event",
            ));
        }
        Ok(())
    }

    fn observe_terminal(&mut self, terminal: GenerationTerminal) -> BenchmarkResult {
        if terminal.request_id != self.request_id {
            return Err(BenchmarkError::new(format!(
                "GenerationFinished addressed request {}, expected {}",
                terminal.request_id.get(),
                self.request_id.get()
            )));
        }
        if self.terminal_event.replace(terminal).is_some() {
            return Err(BenchmarkError::new(
                "generation published more than one matching terminal event",
            ));
        }
        self.terminal_event_at = Some(Instant::now());
        Ok(())
    }

    fn finish(
        self,
        runtime: &ApplicationRuntime,
        expectation: ExpectedCompletion,
    ) -> BenchmarkResult<GenerationEvidence> {
        let terminal = self
            .terminal_event
            .ok_or_else(|| BenchmarkError::new("matching generation terminal event was absent"))?;
        let finish_reason = match terminal.outcome {
            GenerationTerminalOutcome::Finished(reason) => reason,
            GenerationTerminalOutcome::Failed(failure) => {
                return Err(BenchmarkError::new(format!(
                    "generation terminal event reported failure: {failure}"
                )));
            }
        };
        let event_kind = GenerationTerminalKind::Finished(finish_reason);
        validate_terminal_consistency(self.terminal_output, self.released_output, event_kind)?;
        if terminal.usage.prompt_tokens == 0
            || terminal.usage.generated_tokens == 0
            || self.decoded_byte_count == 0
        {
            return Err(BenchmarkError::new(format!(
                "generation did not publish non-zero prompt usage, generated usage, and decoded bytes: usage={:?}, decoded_bytes={}",
                terminal.usage, self.decoded_byte_count
            )));
        }
        match expectation {
            ExpectedCompletion::Chat => match finish_reason {
                FinishReason::TokenLimit
                    if terminal.usage.generated_tokens == u64::from(CHAT_MAXIMUM_NEW_TOKENS) => {}
                FinishReason::EndOfSequence(_)
                    if terminal.usage.generated_tokens <= u64::from(CHAT_MAXIMUM_NEW_TOKENS) => {}
                _ => {
                    return Err(BenchmarkError::new(format!(
                        "compatible-chat proof returned a finish reason or usage inconsistent with its 24-token bound: {terminal:?}"
                    )));
                }
            },
            ExpectedCompletion::DirectTokenLimit => {
                if finish_reason != FinishReason::TokenLimit
                    || terminal.usage.generated_tokens != u64::from(DIRECT_MAXIMUM_NEW_TOKENS)
                {
                    return Err(BenchmarkError::new(format!(
                        "controlled direct completion did not reach the exact 32-token limit: {terminal:?}"
                    )));
                }
            }
        }
        if runtime.state().active_generation().is_some()
            || runtime.state().last_generation() != Some(&terminal)
            || !runtime.state().hub_available()
            || !runtime.state().inference_available()
        {
            return Err(BenchmarkError::new(
                "public E1 state did not retain the matching released terminal generation cleanly",
            ));
        }

        Ok(GenerationEvidence {
            finish_reason,
            terminal_kind: finish_reason_label(finish_reason),
            usage: terminal.usage,
            decoded_byte_count: self.decoded_byte_count,
            submission_to_started: elapsed_since(
                self.submitted,
                self.started_at,
                "GenerationStarted",
            )?,
            submission_to_first_decoded: elapsed_since(
                self.submitted,
                self.first_decoded_at,
                "first non-empty decoded output",
            )?,
            submission_to_terminal_event: elapsed_since(
                self.submitted,
                self.terminal_event_at,
                "terminal application event",
            )?,
            submission_to_release: elapsed_since(
                self.submitted,
                self.released_at,
                "observable Released output state",
            )?,
        })
    }
}

fn await_generation(
    runtime: &mut ApplicationRuntime,
    request_id: RequestId,
    submitted: Instant,
    expectation: ExpectedCompletion,
) -> BenchmarkResult<GenerationEvidence> {
    let deadline = checked_deadline(GENERATION_TIMEOUT, "generation terminal release")?;
    let mut observer = GenerationObserver::new(request_id, submitted);
    loop {
        observer.pull(runtime)?;
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::GenerationStarted { request_id } => {
                    observer.observe_started(request_id)?;
                }
                ApplicationEvent::GenerationFinished { terminal } => {
                    observer.observe_terminal(terminal)?;
                    observer.pull(runtime)?;
                    return observer.finish(runtime, expectation);
                }
                ApplicationEvent::GenerationCleanupPending {
                    request_id: cleanup_request_id,
                    exhausted,
                    failure,
                } => {
                    return Err(BenchmarkError::new(format!(
                        "generation cleanup remained pending for request {} while awaiting {} (exhausted={exhausted}): {failure}",
                        cleanup_request_id.get(),
                        request_id.get()
                    )));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected during generation",
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during generation",
                    ));
                }
                unexpected => {
                    return Err(BenchmarkError::new(format!(
                        "unexpected application event during generation: {unexpected:?}"
                    )));
                }
            }
        }
        wait_for_next_poll(deadline, "generation terminal release")?;
    }
}

fn validate_terminal_consistency(
    terminal_output: Option<GenerationTerminalKind>,
    released_output: Option<GenerationTerminalKind>,
    event_kind: GenerationTerminalKind,
) -> BenchmarkResult {
    if terminal_output != Some(event_kind) || released_output != Some(event_kind) {
        return Err(BenchmarkError::new(format!(
            "terminal, released, and terminal-event outcomes did not match: terminal={terminal_output:?}, released={released_output:?}, event={event_kind:?}"
        )));
    }
    Ok(())
}

fn sample_record(
    ordinal: u32,
    evidence: &GenerationEvidence,
    process_memory_after_release: ProcessMemory,
) -> BenchmarkResult<DirectCompletionSample> {
    let release_seconds = evidence.submission_to_release.as_secs_f64();
    if release_seconds <= 0.0 || !release_seconds.is_finite() {
        return Err(BenchmarkError::new(
            "submission-to-release duration could not support a finite throughput calculation",
        ));
    }
    let generated_tokens = u32::try_from(evidence.usage.generated_tokens).map_err(|_| {
        BenchmarkError::new("generated-token count was too large for exact f64 conversion")
    })?;
    let effective_generated_tokens_per_second = f64::from(generated_tokens) / release_seconds;
    if !effective_generated_tokens_per_second.is_finite() {
        return Err(BenchmarkError::new(
            "effective generated-token throughput was not finite",
        ));
    }
    Ok(DirectCompletionSample {
        ordinal,
        submission_to_generation_started_ns: duration_ns(evidence.submission_to_started),
        submission_to_first_decoded_output_ns: duration_ns(evidence.submission_to_first_decoded),
        submission_to_terminal_event_ns: duration_ns(evidence.submission_to_terminal_event),
        submission_to_release_ns: duration_ns(evidence.submission_to_release),
        prompt_tokens: evidence.usage.prompt_tokens,
        generated_tokens: evidence.usage.generated_tokens,
        decoded_byte_count: evidence.decoded_byte_count,
        terminal_kind: evidence.terminal_kind,
        terminal_state_matched: true,
        released_state_matched: true,
        terminal_event_matched: true,
        effective_generated_tokens_per_second,
        process_memory_after_release,
    })
}

fn summarize_samples(
    samples: &[DirectCompletionSample],
) -> BenchmarkResult<DirectCompletionSummary> {
    let sample_count = u32::try_from(samples.len())
        .map_err(|_| BenchmarkError::new("sample count conversion to u32 failed"))?;
    if sample_count != SAMPLE_COUNT {
        return Err(BenchmarkError::new(format!(
            "external baseline collected {sample_count} samples instead of {SAMPLE_COUNT}"
        )));
    }
    Ok(DirectCompletionSummary {
        sample_count,
        median_submission_to_generation_started_ns: median_u64(
            samples
                .iter()
                .map(|sample| sample.submission_to_generation_started_ns),
        )?,
        median_submission_to_first_decoded_output_ns: median_u64(
            samples
                .iter()
                .map(|sample| sample.submission_to_first_decoded_output_ns),
        )?,
        median_submission_to_terminal_event_ns: median_u64(
            samples
                .iter()
                .map(|sample| sample.submission_to_terminal_event_ns),
        )?,
        median_submission_to_release_ns: median_u64(
            samples.iter().map(|sample| sample.submission_to_release_ns),
        )?,
        median_effective_generated_tokens_per_second: median_f64(
            samples
                .iter()
                .map(|sample| sample.effective_generated_tokens_per_second),
        )?,
    })
}

fn median_u64(values: impl IntoIterator<Item = u64>) -> BenchmarkResult<u64> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        return Err(BenchmarkError::new("cannot summarize an empty sample set"));
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    let upper = values
        .get(middle)
        .copied()
        .ok_or_else(|| BenchmarkError::new("summary upper median disappeared"))?;
    if values.len() % 2 == 0 {
        let lower = values
            .get(middle.saturating_sub(1))
            .copied()
            .ok_or_else(|| BenchmarkError::new("summary lower median disappeared"))?;
        Ok(lower.saturating_add(upper) / 2)
    } else {
        Ok(upper)
    }
}

fn median_f64(values: impl IntoIterator<Item = f64>) -> BenchmarkResult<f64> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return Err(BenchmarkError::new(
            "cannot summarize empty or non-finite throughput samples",
        ));
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    let upper = values
        .get(middle)
        .copied()
        .ok_or_else(|| BenchmarkError::new("throughput upper median disappeared"))?;
    if values.len() % 2 == 0 {
        let lower = values
            .get(middle.saturating_sub(1))
            .copied()
            .ok_or_else(|| BenchmarkError::new("throughput lower median disappeared"))?;
        Ok(f64::midpoint(lower, upper))
    } else {
        Ok(upper)
    }
}

fn unload_model(
    runtime: &mut ApplicationRuntime,
    loaded: &LoadedModel,
) -> BenchmarkResult<ExternalUnloadResult> {
    let started = Instant::now();
    runtime
        .unload_model_with_behavior(ModelUnloadBehavior::RejectIfBusy)
        .map_err(|error| {
            BenchmarkError::new(format!(
                "RejectIfBusy model unload could not be submitted after release: {error}"
            ))
        })?;
    let deadline = checked_deadline(UNLOAD_TIMEOUT, "model unload")?;
    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelDraining { handle } if handle == loaded.handle() => {}
                ApplicationEvent::ModelDraining { handle } => {
                    return Err(BenchmarkError::new(format!(
                        "model unload began draining unexpected handle {handle:?}"
                    )));
                }
                ApplicationEvent::ModelUnloaded {
                    handle,
                    cancelled_requests,
                } => {
                    if handle != loaded.handle() || cancelled_requests != 0 {
                        return Err(BenchmarkError::new(format!(
                            "model unload receipt did not match the loaded handle with zero cancellations: handle={handle:?}, cancelled_requests={cancelled_requests}"
                        )));
                    }
                    let state = runtime.state();
                    if state.activity() != ApplicationActivity::Idle
                        || state.loaded().is_some()
                        || state.active_generation().is_some()
                        || !state.hub_available()
                        || !state.inference_available()
                    {
                        return Err(BenchmarkError::new(
                            "public E1 state retained loaded/active ownership or disconnected before explicit shutdown",
                        ));
                    }
                    return Ok(ExternalUnloadResult {
                        duration_ns: duration_ns(started.elapsed()),
                        cancelled_requests,
                        loaded_model_absent: true,
                        active_generation_absent: true,
                        runtime_connected: true,
                    });
                }
                ApplicationEvent::ModelUnloadFailed { failure } => {
                    return Err(BenchmarkError::new(format!(
                        "RejectIfBusy model unload failed: {failure}"
                    )));
                }
                ApplicationEvent::GenerationCleanupPending {
                    exhausted, failure, ..
                } => {
                    return Err(BenchmarkError::new(format!(
                        "generation cleanup remained pending during unload (exhausted={exhausted}): {failure}"
                    )));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(BenchmarkError::new(
                        "Hub worker disconnected before explicit shutdown",
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(BenchmarkError::new(
                        "inference worker disconnected during model unload",
                    ));
                }
                unexpected => {
                    return Err(BenchmarkError::new(format!(
                        "unexpected application event during model unload: {unexpected:?}"
                    )));
                }
            }
        }
        wait_for_next_poll(deadline, "model unload")?;
    }
}

fn validate_stopped(runtime: &ApplicationRuntime) -> BenchmarkResult {
    let state = runtime.state();
    let last_generation_succeeded = state
        .last_generation()
        .is_some_and(|terminal| matches!(terminal.outcome, GenerationTerminalOutcome::Finished(_)));
    if state.activity() != ApplicationActivity::ShuttingDown
        || state.hub_available()
        || state.inference_available()
        || state.loaded().is_some()
        || state.active_generation().is_some()
        || !last_generation_succeeded
        || !runtime.conversation().is_empty()
    {
        return Err(BenchmarkError::new(
            "explicit shutdown returned without clean terminal workers, released generation, unloaded model, and empty conversation state",
        ));
    }
    Ok(())
}

fn checked_deadline(timeout: Duration, operation: &'static str) -> BenchmarkResult<Instant> {
    Instant::now().checked_add(timeout).ok_or_else(|| {
        BenchmarkError::new(format!(
            "deadline overflow while preparing to wait for {operation}"
        ))
    })
}

fn wait_for_next_poll(deadline: Instant, operation: &'static str) -> BenchmarkResult {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| BenchmarkError::new(format!("timed out waiting for {operation}")))?;
    std::thread::sleep(POLL_INTERVAL.min(remaining));
    Ok(())
}

fn elapsed_since(
    submitted: Instant,
    observed: Option<Instant>,
    label: &'static str,
) -> BenchmarkResult<Duration> {
    observed
        .ok_or_else(|| BenchmarkError::new(format!("{label} was not observed")))?
        .checked_duration_since(submitted)
        .ok_or_else(|| BenchmarkError::new(format!("{label} preceded request submission")))
}

const fn scalar_label(scalar: ApplicationScalarType) -> &'static str {
    match scalar {
        ApplicationScalarType::F32 => "F32",
        ApplicationScalarType::F16 => "F16",
        ApplicationScalarType::Bf16 => "BF16",
    }
}

const fn finish_reason_label(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::EndOfSequence(_) => "end_of_sequence",
        FinishReason::TokenLimit => "token_limit",
        FinishReason::StopCondition => "stop_condition",
        FinishReason::BufferExhausted(_) => "buffer_exhausted",
        FinishReason::Cancelled(_) => "cancelled",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use application_runtime::{
        ApplicationOutputRecordKind, ApplicationOutputState, GenerationTerminalKind,
    };
    use domain_contracts::{FinishReason, RequestId};

    use super::{
        GenerationObserver, MODEL_REPOSITORY, MODEL_REVISION, validate_terminal_consistency,
    };

    #[test]
    fn exact_external_identity_is_constant_and_not_caller_controlled() {
        assert_eq!(MODEL_REPOSITORY, "TinyLlama/TinyLlama-1.1B-Chat-v1.0");
        assert_eq!(MODEL_REVISION, "fe8a4ea1ffedaf415f4da2f062534de366a451e6");
    }

    #[test]
    fn output_validation_rejects_a_mismatched_request_identity() {
        let mut observer = GenerationObserver::new(RequestId::new(1), Instant::now());
        let result = observer.observe_output(
            RequestId::new(2),
            ApplicationOutputRecordKind::State(ApplicationOutputState::Terminal(
                GenerationTerminalKind::Finished(FinishReason::TokenLimit),
            )),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn terminal_validation_rejects_mismatched_terminal_and_released_outcomes() {
        let terminal = GenerationTerminalKind::Finished(FinishReason::TokenLimit);
        let released = GenerationTerminalKind::Finished(FinishReason::StopCondition);
        assert!(validate_terminal_consistency(Some(terminal), Some(released), terminal).is_err());
        assert!(validate_terminal_consistency(Some(terminal), Some(terminal), released).is_err());
    }

    #[test]
    fn terminal_validation_accepts_one_matching_terminal_release_and_event() -> Result<(), String> {
        let expected = GenerationTerminalKind::Finished(FinishReason::TokenLimit);
        validate_terminal_consistency(Some(expected), Some(expected), expected)
            .map_err(|error| error.to_string())
    }
}
