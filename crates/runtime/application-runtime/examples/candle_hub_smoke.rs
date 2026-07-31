//! Opt-in E1 smoke for immutable Hub resolution and Candle CPU completion.
//!
//! Compilation performs no network access. Running the example requires
//! `LLM_APP_CANDLE_HUB_SMOKE=1`, because resolution may contact Hugging Face and
//! populate the configured cache.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use application_runtime::{
    ApplicationActivity, ApplicationDevice, ApplicationEngine, ApplicationEvent,
    ApplicationFailure, ApplicationModelFormat, ApplicationOutputRecordKind,
    ApplicationOutputState, ApplicationRuntime, ApplicationRuntimeConfiguration,
    ApplicationScalarType, ApplicationSource, GenerationSeed, GenerationSettings,
    GenerationTerminal, GenerationTerminalKind, GenerationTerminalOutcome, LoadedModel,
    ModelSelection, ModelUnloadBehavior, ResolvedModel,
};
use domain_contracts::{FinishReason, RequestId};

const OPT_IN_ENVIRONMENT: &str = "LLM_APP_CANDLE_HUB_SMOKE";
const MODEL_REPOSITORY: &str = "neubla/tiny-random-LlamaForCausalLM";
const MODEL_REVISION: &str = "1c81a3fba044af78df253edc66bdbab183184932";
const DIRECT_COMPLETION_PROMPT: &str = "Hello";
const GENERATED_TOKEN_LIMIT: u32 = 8;
const HUB_RETRIES: usize = 2;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const RESOLUTION_TIMEOUT: Duration = Duration::from_mins(3);
const LOAD_TIMEOUT: Duration = Duration::from_mins(1);
const GENERATION_TIMEOUT: Duration = Duration::from_mins(1);
const UNLOAD_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const TEMP_DIRECTORY_ATTEMPTS: u64 = 128;

static NEXT_TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);

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

fn run() -> Result<(), SmokeFailures> {
    require_explicit_opt_in().map_err(SmokeFailures::one)?;

    println!("Candle Hub smoke explicitly enabled by {OPT_IN_ENVIRONMENT}=1");
    println!("model repository: {MODEL_REPOSITORY}");
    println!("model revision: {MODEL_REVISION}");
    println!("expected architecture path: Hugging Face LlamaForCausalLM -> Candle Llama");
    print_hub_environment();

    let mut workspace = TemporaryWorkspace::create().map_err(SmokeFailures::one)?;
    println!(
        "temporary redb path: {}",
        workspace.database_path().display()
    );

    let configuration = application_configuration(&workspace.database_path());
    let mut failures = Vec::new();

    match ApplicationRuntime::start(configuration) {
        Ok(mut runtime) => {
            if let Err(error) = run_lifecycle(&mut runtime) {
                failures.push(error);
            }
            if let Err(error) = explicitly_shutdown(&mut runtime) {
                failures.push(error);
            }
            drop(runtime);
        }
        Err(error) => failures.push(SmokeError::new(
            SmokeStage::Runtime,
            format!(
                "ApplicationRuntime startup failed: {error}. Check that the temporary directory is \
                 writable; if the diagnostic names the Hub client, also inspect the authentication \
                 and cache guidance below.{}",
                hub_failure_guidance()
            ),
        )),
    }

    if let Err(error) = workspace.cleanup() {
        failures.push(error);
    }

    if failures.is_empty() {
        println!("temporary redb workspace removed");
        println!("Candle Hub smoke: PASS");
        Ok(())
    } else {
        Err(SmokeFailures(failures))
    }
}

fn application_configuration(database_path: &Path) -> ApplicationRuntimeConfiguration {
    let mut configuration = ApplicationRuntimeConfiguration::desktop(database_path);
    MODEL_REPOSITORY.clone_into(&mut configuration.defaults.default_repository);
    MODEL_REVISION.clone_into(&mut configuration.defaults.default_revision);
    configuration.defaults.drain_timeout_milliseconds = 10_000;
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

fn run_lifecycle(runtime: &mut ApplicationRuntime) -> SmokeResult {
    let selection = ModelSelection::new(MODEL_REPOSITORY, MODEL_REVISION);
    let resolved = resolve_model(runtime, &selection)?;
    verify_resolved_evidence(&resolved, &selection)?;
    print_resolved_evidence(&resolved);

    let loaded = load_model(runtime, &selection)?;
    verify_loaded_evidence(&loaded, &selection)?;
    print_loaded_evidence(&loaded);

    let generation = run_direct_completion(runtime)?;
    println!(
        "direct-completion request: {}",
        generation.terminal.request_id.get()
    );
    println!("decoded output: {:?}", generation.decoded_output);
    println!("terminal output state: {:?}", generation.terminal_state);
    println!("released output state: {:?}", generation.released_state);
    println!("terminal event: {:?}", generation.terminal);

    unload_model(runtime, &loaded)?;
    Ok(())
}

fn resolve_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
) -> SmokeResult<ResolvedModel> {
    runtime.resolve_model(selection.clone()).map_err(|error| {
        SmokeError::new(
            SmokeStage::Resolution,
            format!(
                "production Hub-worker resolution could not be submitted for \
                 {MODEL_REPOSITORY}@{MODEL_REVISION}: {error}.{}",
                hub_failure_guidance()
            ),
        )
    })?;

    let deadline = checked_deadline(
        RESOLUTION_TIMEOUT,
        SmokeStage::Resolution,
        "immutable Hub resolution",
    )?;
    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelResolved {
                    model,
                    persistence_warning,
                } => {
                    if let Some(warning) = persistence_warning {
                        return Err(SmokeError::new(
                            SmokeStage::Runtime,
                            format!(
                                "Hub artifacts resolved, but the production catalogue could not \
                                 persist the immutable result: {warning}"
                            ),
                        ));
                    }
                    return Ok(model);
                }
                ApplicationEvent::ModelResolutionFailed { failure } => {
                    return Err(hub_resolution_failure(&failure));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(SmokeError::new(
                        SmokeStage::Resolution,
                        format!(
                            "the production Hub worker disconnected while resolving \
                             {MODEL_REPOSITORY}@{MODEL_REVISION}.{}",
                            hub_failure_guidance()
                        ),
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        "the Candle inference worker disconnected during Hub resolution",
                    ));
                }
                unexpected => {
                    return Err(SmokeError::new(
                        SmokeStage::Resolution,
                        format!("unexpected event during Hub resolution: {unexpected:?}"),
                    ));
                }
            }
        }
        wait_for_next_poll(deadline, SmokeStage::Resolution, "immutable Hub resolution")?;
    }
}

fn verify_resolved_evidence(model: &ResolvedModel, selection: &ModelSelection) -> SmokeResult {
    if model.selection() != selection
        || model.selection().repository() != MODEL_REPOSITORY
        || model.selection().revision() != MODEL_REVISION
    {
        return Err(SmokeError::new(
            SmokeStage::Resolution,
            format!(
                "resolved selection mismatch: returned {:?}, expected \
                 {MODEL_REPOSITORY}@{MODEL_REVISION}",
                model.selection()
            ),
        ));
    }
    if model.identity().repository() != MODEL_REPOSITORY
        || model.identity().commit() != MODEL_REVISION
    {
        return Err(SmokeError::new(
            SmokeStage::Resolution,
            format!(
                "immutable Hub identity mismatch: returned {}@{}, expected \
                 {MODEL_REPOSITORY}@{MODEL_REVISION}",
                model.identity().repository(),
                model.identity().commit()
            ),
        ));
    }

    let evidence = (
        model.engine(),
        model.source(),
        model.device(),
        model.format(),
        model.scalar_type(),
    );
    let expected = (
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationDevice::Cpu,
        ApplicationModelFormat::Safetensors,
        Some(ApplicationScalarType::F32),
    );
    if evidence != expected || !model.is_loadable() {
        return Err(SmokeError::new(
            SmokeStage::Resolution,
            format!(
                "resolved execution evidence was {evidence:?}, expected Candle, Hugging Face Hub, \
                 CPU, Safetensors, and F32 with a loadable resolution"
            ),
        ));
    }
    Ok(())
}

fn print_resolved_evidence(model: &ResolvedModel) {
    println!(
        "resolved evidence: repository={} requested_revision={} commit={} engine={:?} source={:?} \
         device={:?} format={:?} scalar={:?} vocabulary_size={}",
        model.selection().repository(),
        model.selection().revision(),
        model.identity().commit(),
        model.engine(),
        model.source(),
        model.device(),
        model.format(),
        model.scalar_type(),
        model.vocabulary_size()
    );
}

fn load_model(
    runtime: &mut ApplicationRuntime,
    selection: &ModelSelection,
) -> SmokeResult<LoadedModel> {
    runtime.load_model(selection).map_err(|error| {
        SmokeError::new(
            SmokeStage::Runtime,
            format!("production Candle load could not be submitted: {error}"),
        )
    })?;

    let deadline = checked_deadline(LOAD_TIMEOUT, SmokeStage::Runtime, "Candle model load")?;
    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelLoaded { model } => return Ok(model),
                ApplicationEvent::ModelLoadFailed { failure } => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        format!("production Candle model load failed: {failure}"),
                    ));
                }
                ApplicationEvent::ModelCompatibilityFailed { failure } => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        format!(
                            "resolved Hub artifacts did not match the loaded Candle descriptor: \
                             {failure}"
                        ),
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        "the Candle inference worker disconnected during model load",
                    ));
                }
                ApplicationEvent::HubDisconnected => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        "the Hub worker disconnected after resolution and before model load completed",
                    ));
                }
                unexpected => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        format!("unexpected event during Candle model load: {unexpected:?}"),
                    ));
                }
            }
        }
        wait_for_next_poll(deadline, SmokeStage::Runtime, "Candle model load")?;
    }
}

fn verify_loaded_evidence(model: &LoadedModel, selection: &ModelSelection) -> SmokeResult {
    if model.selection() != selection
        || model.selection().repository() != MODEL_REPOSITORY
        || model.selection().revision() != MODEL_REVISION
        || model.identity().repository() != MODEL_REPOSITORY
        || model.identity().commit() != MODEL_REVISION
    {
        return Err(SmokeError::new(
            SmokeStage::Runtime,
            format!(
                "loaded model did not retain the exact resolved selection and immutable identity: \
                 selection={:?}, identity={}@{}",
                model.selection(),
                model.identity().repository(),
                model.identity().commit()
            ),
        ));
    }

    let evidence = (
        model.engine(),
        model.source(),
        model.device(),
        model.format(),
        model.scalar_type(),
    );
    let expected = (
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationDevice::Cpu,
        ApplicationModelFormat::Safetensors,
        ApplicationScalarType::F32,
    );
    if evidence != expected {
        return Err(SmokeError::new(
            SmokeStage::Runtime,
            format!(
                "loaded execution evidence was {evidence:?}, expected Candle, Hugging Face Hub, \
                 CPU, Safetensors, and F32"
            ),
        ));
    }
    Ok(())
}

fn print_loaded_evidence(model: &LoadedModel) {
    println!(
        "loaded evidence: repository={} commit={} engine={:?} source={:?} device={:?} format={:?} \
         scalar={:?} context_tokens={} prefill_tokens={}",
        model.identity().repository(),
        model.identity().commit(),
        model.engine(),
        model.source(),
        model.device(),
        model.format(),
        model.scalar_type(),
        model.maximum_context_tokens(),
        model.maximum_prefill_batch()
    );
}

fn run_direct_completion(runtime: &mut ApplicationRuntime) -> SmokeResult<GenerationEvidence> {
    let settings = GenerationSettings {
        maximum_new_tokens: GENERATED_TOKEN_LIMIT,
        temperature: 1.0,
        top_k: 1,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        repetition_window: 0,
        seed: GenerationSeed::Fixed(39),
        eos_tokens: Vec::new(),
        stop_sequences: Vec::new(),
    };
    let request_id = runtime
        .start_generation(DIRECT_COMPLETION_PROMPT, settings)
        .map_err(|error| {
            SmokeError::new(
                SmokeStage::Runtime,
                format!(
                    "bounded direct completion could not be submitted for prompt \
                     {DIRECT_COMPLETION_PROMPT:?}: {error}"
                ),
            )
        })?;
    let deadline = checked_deadline(
        GENERATION_TIMEOUT,
        SmokeStage::Runtime,
        "bounded direct completion release",
    )?;
    let mut collector = GenerationCollector::new(request_id);

    loop {
        collector.pull(runtime)?;
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::GenerationStarted {
                    request_id: started,
                } if started == request_id => collector.saw_started = true,
                ApplicationEvent::GenerationStarted {
                    request_id: unexpected,
                } => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        format!(
                            "generation admission returned request {}, expected {}",
                            unexpected.get(),
                            request_id.get()
                        ),
                    ));
                }
                ApplicationEvent::GenerationFinished { terminal }
                    if terminal.request_id == request_id =>
                {
                    collector.pull(runtime)?;
                    return collector.finish(runtime, terminal);
                }
                ApplicationEvent::GenerationFinished { terminal } => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        format!(
                            "generation terminal event addressed request {}, expected {}",
                            terminal.request_id.get(),
                            request_id.get()
                        ),
                    ));
                }
                ApplicationEvent::GenerationCleanupPending {
                    exhausted, failure, ..
                } => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        format!(
                            "generation cleanup remained pending (exhausted={exhausted}): {failure}"
                        ),
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        "the Candle inference worker disconnected during direct completion",
                    ));
                }
                unexpected => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        format!("unexpected event during direct completion: {unexpected:?}"),
                    ));
                }
            }
        }
        wait_for_next_poll(
            deadline,
            SmokeStage::Runtime,
            "bounded direct completion release",
        )?;
    }
}

struct GenerationCollector {
    request_id: RequestId,
    decoded_output: String,
    states: Vec<ApplicationOutputState>,
    text_records: usize,
    saw_started: bool,
}

impl GenerationCollector {
    const fn new(request_id: RequestId) -> Self {
        Self {
            request_id,
            decoded_output: String::new(),
            states: Vec::new(),
            text_records: 0,
            saw_started: false,
        }
    }

    fn pull(&mut self, runtime: &mut ApplicationRuntime) -> SmokeResult {
        runtime
            .pull_output(|batch| {
                for record in batch.records() {
                    if record.request_id != self.request_id {
                        return Err(SmokeError::new(
                            SmokeStage::Runtime,
                            format!(
                                "decoded output addressed request {}, expected {}",
                                record.request_id.get(),
                                self.request_id.get()
                            ),
                        ));
                    }
                    match record.kind {
                        ApplicationOutputRecordKind::Text(_) => {
                            let fragment = batch.text_for(record).ok_or_else(|| {
                                SmokeError::new(
                                    SmokeStage::Runtime,
                                    "decoded output contained an invalid UTF-8 text range",
                                )
                            })?;
                            self.decoded_output.push_str(fragment);
                            self.text_records = self.text_records.saturating_add(1);
                        }
                        ApplicationOutputRecordKind::State(state) => self.states.push(state),
                    }
                }
                Ok(())
            })
            .map_err(|error| {
                SmokeError::new(
                    SmokeStage::Runtime,
                    format!("decoded application output could not be pulled: {error}"),
                )
            })?
    }

    fn finish(
        self,
        runtime: &ApplicationRuntime,
        terminal: GenerationTerminal,
    ) -> SmokeResult<GenerationEvidence> {
        if !self.saw_started {
            return Err(SmokeError::new(
                SmokeStage::Runtime,
                "direct completion reached terminal release without a GenerationStarted event",
            ));
        }

        let terminal_state = self.states.iter().find_map(|state| match state {
            ApplicationOutputState::Terminal(kind) => Some(*kind),
            _ => None,
        });
        let released_state = self.states.iter().find_map(|state| match state {
            ApplicationOutputState::Released(kind) => Some(*kind),
            _ => None,
        });
        let expected = GenerationTerminalKind::Finished(FinishReason::TokenLimit);
        if terminal_state != Some(expected) || released_state != Some(expected) {
            return Err(SmokeError::new(
                SmokeStage::Runtime,
                format!(
                    "decoded output did not contain matching token-limit Terminal and Released \
                     states; collected states: {:?}",
                    self.states
                ),
            ));
        }
        if terminal.outcome != GenerationTerminalOutcome::Finished(FinishReason::TokenLimit)
            || terminal.usage.generated_tokens != u64::from(GENERATED_TOKEN_LIMIT)
            || terminal.usage.prompt_tokens == 0
        {
            return Err(SmokeError::new(
                SmokeStage::Runtime,
                format!(
                    "bounded direct completion returned unexpected terminal evidence: {terminal:?}"
                ),
            ));
        }
        if self.text_records == 0 || self.decoded_output.is_empty() {
            return Err(SmokeError::new(
                SmokeStage::Runtime,
                format!(
                    "direct completion generated {} tokens but published no decoded UTF-8 text",
                    terminal.usage.generated_tokens
                ),
            ));
        }
        if runtime.state().active_generation().is_some()
            || runtime.state().last_generation() != Some(&terminal)
        {
            return Err(SmokeError::new(
                SmokeStage::Runtime,
                "application state did not retain the released terminal generation cleanly",
            ));
        }

        Ok(GenerationEvidence {
            terminal,
            decoded_output: self.decoded_output,
            terminal_state: terminal_state.ok_or_else(|| {
                SmokeError::new(SmokeStage::Runtime, "terminal output state disappeared")
            })?,
            released_state: released_state.ok_or_else(|| {
                SmokeError::new(SmokeStage::Runtime, "released output state disappeared")
            })?,
        })
    }
}

struct GenerationEvidence {
    terminal: GenerationTerminal,
    decoded_output: String,
    terminal_state: GenerationTerminalKind,
    released_state: GenerationTerminalKind,
}

fn unload_model(runtime: &mut ApplicationRuntime, loaded: &LoadedModel) -> SmokeResult {
    runtime
        .unload_model_with_behavior(ModelUnloadBehavior::RejectIfBusy)
        .map_err(|error| {
            SmokeError::new(
                SmokeStage::Runtime,
                format!("model unload could not be submitted after release: {error}"),
            )
        })?;
    let deadline = checked_deadline(UNLOAD_TIMEOUT, SmokeStage::Runtime, "model unload")?;

    loop {
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::ModelUnloaded {
                    handle,
                    cancelled_requests,
                } => {
                    if handle != loaded.handle() || cancelled_requests != 0 {
                        return Err(SmokeError::new(
                            SmokeStage::Runtime,
                            format!(
                                "unexpected unload receipt: handle={handle:?}, \
                                 cancelled_requests={cancelled_requests}"
                            ),
                        ));
                    }
                    if runtime.state().loaded().is_some()
                        || runtime.state().active_generation().is_some()
                        || runtime.state().activity() != ApplicationActivity::Idle
                    {
                        return Err(SmokeError::new(
                            SmokeStage::Runtime,
                            "application retained loaded or active state after unload",
                        ));
                    }
                    println!(
                        "model unloaded: handle={handle:?} cancelled_requests={cancelled_requests}"
                    );
                    return Ok(());
                }
                ApplicationEvent::ModelDraining { handle } if handle == loaded.handle() => {}
                ApplicationEvent::ModelDraining { handle } => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        format!("unload began draining an unexpected handle: {handle:?}"),
                    ));
                }
                ApplicationEvent::ModelUnloadFailed { failure } => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        format!("production model unload failed: {failure}"),
                    ));
                }
                ApplicationEvent::RuntimeDisconnected => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        "the Candle inference worker disconnected during model unload",
                    ));
                }
                unexpected => {
                    return Err(SmokeError::new(
                        SmokeStage::Runtime,
                        format!("unexpected event during model unload: {unexpected:?}"),
                    ));
                }
            }
        }
        wait_for_next_poll(deadline, SmokeStage::Runtime, "model unload")?;
    }
}

fn explicitly_shutdown(runtime: &mut ApplicationRuntime) -> SmokeResult {
    runtime.shutdown().map_err(|error| {
        SmokeError::new(
            SmokeStage::Shutdown,
            format!("explicit bounded ApplicationRuntime shutdown failed: {error}"),
        )
    })?;
    if runtime.state().activity() != ApplicationActivity::ShuttingDown
        || runtime.state().hub_available()
        || runtime.state().inference_available()
    {
        return Err(SmokeError::new(
            SmokeStage::Shutdown,
            format!(
                "shutdown returned without terminal worker state: {:?}",
                runtime.state()
            ),
        ));
    }
    println!("explicit ApplicationRuntime shutdown: complete");
    Ok(())
}

fn require_explicit_opt_in() -> SmokeResult {
    match std::env::var(OPT_IN_ENVIRONMENT) {
        Ok(value) if value == "1" => Ok(()),
        Ok(value) => Err(SmokeError::new(
            SmokeStage::Configuration,
            format!(
                "{OPT_IN_ENVIRONMENT} must equal 1, not {value:?}; opt in with \
                 `{OPT_IN_ENVIRONMENT}=1 cargo run --locked -p application-runtime --example \
                 candle_hub_smoke`"
            ),
        )),
        Err(std::env::VarError::NotPresent) => Err(SmokeError::new(
            SmokeStage::Configuration,
            format!(
                "this example may access the network and Hub cache; opt in with \
                 `{OPT_IN_ENVIRONMENT}=1 cargo run --locked -p application-runtime --example \
                 candle_hub_smoke`"
            ),
        )),
        Err(std::env::VarError::NotUnicode(_)) => Err(SmokeError::new(
            SmokeStage::Configuration,
            format!("{OPT_IN_ENVIRONMENT} is not valid Unicode and must equal 1"),
        )),
    }
}

fn print_hub_environment() {
    let cache = std::env::var_os("HF_HOME").map_or_else(
        || "hf-hub environment default (HF_HOME is not set)".to_owned(),
        |value| PathBuf::from(value).display().to_string(),
    );
    let authentication = if std::env::var_os("HF_TOKEN").is_some() {
        "HF_TOKEN is set (value redacted)"
    } else {
        "HF_TOKEN is not set; anonymous or other environment-derived authentication will be used"
    };
    let offline = if std::env::var_os("HF_HUB_OFFLINE").is_some() {
        "set"
    } else {
        "not set"
    };
    println!("Hub cache: {cache}");
    println!("Hub authentication: {authentication}");
    println!("HF_HUB_OFFLINE: {offline}");
}

fn hub_resolution_failure(failure: &ApplicationFailure) -> SmokeError {
    SmokeError::new(
        SmokeStage::Resolution,
        format!(
            "production Hub-worker resolution failed for {MODEL_REPOSITORY}@{MODEL_REVISION} \
             ({:?}): {}.{}",
            failure.kind,
            failure.message,
            hub_failure_guidance()
        ),
    )
}

fn hub_failure_guidance() -> String {
    let cache = std::env::var_os("HF_HOME").map_or_else(
        || "the hf-hub environment default (HF_HOME is not set)".to_owned(),
        |value| format!("{} (from HF_HOME)", PathBuf::from(value).display()),
    );
    let token = if std::env::var_os("HF_TOKEN").is_some() {
        "set (value redacted)"
    } else {
        "not set"
    };
    let offline = if std::env::var_os("HF_HUB_OFFLINE").is_some() {
        "set; unset it when the exact revision is not already cached"
    } else {
        "not set"
    };
    format!(
        "\n  network: verify HTTPS/DNS access to huggingface.co and retry transient failures; \
         the smoke deadline is {RESOLUTION_TIMEOUT:?}.\n  authentication: HF_TOKEN is {token}; for \
         401/403 or gated-repository errors, provide an authorized token without printing it.\n  \
         cache: active cache is {cache}; ensure it exists or can be created, is writable, and has \
         enough space. HF_HUB_OFFLINE is {offline}.\n  artifacts: the pinned revision must expose \
         config.json, tokenizer.json, and model.safetensors (or a supported Safetensors index)."
    )
}

fn checked_deadline(
    timeout: Duration,
    stage: SmokeStage,
    operation: &'static str,
) -> SmokeResult<Instant> {
    Instant::now().checked_add(timeout).ok_or_else(|| {
        SmokeError::new(
            stage,
            format!("deadline overflow while preparing to wait for {operation}"),
        )
    })
}

fn wait_for_next_poll(
    deadline: Instant,
    stage: SmokeStage,
    operation: &'static str,
) -> SmokeResult {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| SmokeError::new(stage, format!("timed out waiting for {operation}")))?;
    std::thread::sleep(POLL_INTERVAL.min(remaining));
    Ok(())
}

struct TemporaryWorkspace {
    root: PathBuf,
    cleaned: bool,
}

impl TemporaryWorkspace {
    fn create() -> SmokeResult<Self> {
        let parent = std::env::temp_dir();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let first_identifier = NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);

        for attempt in 0..TEMP_DIRECTORY_ATTEMPTS {
            let identifier = first_identifier.wrapping_add(attempt);
            let root = parent.join(format!(
                "llm-app-candle-hub-smoke-{}-{timestamp}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    return Ok(Self {
                        root,
                        cleaned: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(SmokeError::new(
                        SmokeStage::Cleanup,
                        format!(
                            "could not create unique temporary workspace {}: {error}",
                            root.display()
                        ),
                    ));
                }
            }
        }

        Err(SmokeError::new(
            SmokeStage::Cleanup,
            format!(
                "could not create a unique temporary workspace under {} after \
                 {TEMP_DIRECTORY_ATTEMPTS} attempts",
                parent.display()
            ),
        ))
    }

    fn database_path(&self) -> PathBuf {
        self.root.join("application.redb")
    }

    fn cleanup(&mut self) -> SmokeResult {
        match fs::remove_dir_all(&self.root) {
            Ok(()) => {
                self.cleaned = true;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.cleaned = true;
                Ok(())
            }
            Err(error) => Err(SmokeError::new(
                SmokeStage::Cleanup,
                format!(
                    "could not remove temporary redb workspace {}: {error}; remove it manually \
                     after any detached worker exits",
                    self.root.display()
                ),
            )),
        }
    }
}

impl Drop for TemporaryWorkspace {
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        if let Err(error) = fs::remove_dir_all(&self.root)
            && error.kind() != io::ErrorKind::NotFound
        {
            eprintln!(
                "cleanup fallback could not remove temporary redb workspace {}: {error}",
                self.root.display()
            );
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum SmokeStage {
    Configuration,
    Resolution,
    Runtime,
    Shutdown,
    Cleanup,
}

impl SmokeStage {
    const fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration error",
            Self::Resolution => "Hub resolution error",
            Self::Runtime => "runtime error",
            Self::Shutdown => "shutdown error",
            Self::Cleanup => "cleanup error",
        }
    }
}

#[derive(Debug)]
struct SmokeError {
    stage: SmokeStage,
    message: String,
}

impl SmokeError {
    fn new(stage: SmokeStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }
}

impl Display for SmokeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage.label(), self.message)
    }
}

impl Error for SmokeError {}

#[derive(Debug)]
struct SmokeFailures(Vec<SmokeError>);

impl SmokeFailures {
    fn one(error: SmokeError) -> Self {
        Self(vec![error])
    }
}

impl Display for SmokeFailures {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        if let Some(error) = self.0.first()
            && self.0.len() == 1
        {
            return Display::fmt(error, formatter);
        }

        formatter.write_str("Candle Hub smoke failed:")?;
        for error in &self.0 {
            write!(formatter, "\n- {error}")?;
        }
        Ok(())
    }
}

impl Error for SmokeFailures {}
