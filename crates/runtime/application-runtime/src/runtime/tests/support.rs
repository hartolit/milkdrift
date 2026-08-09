use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use domain_contracts::{CancellationReason, FinishReason, ModelId, RequestId};
use hf_hub_adapter::{
    ArtifactContentIdentity, ArtifactContentIdentityAuthority, ArtifactScalarType,
    ResolvedSafetensorsLlamaArtifacts, ResolvedSafetensorsShard,
};
use inference_runtime::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, CommandTicket, FailureClass,
    LoadReceipt, RuntimeError, RuntimeEvent, RuntimeOperation,
};

use super::super::ApplicationRuntime;
use crate::{
    ApplicationDevice, ApplicationEngine, ApplicationEvent, ApplicationModelFormat,
    ApplicationOutputRecordKind, ApplicationOutputState, ApplicationRuntimeConfiguration,
    ApplicationScalarType, ApplicationSource, GenerationSeed, GenerationSettings,
    GenerationTerminal, GenerationTerminalKind, GenerationTerminalOutcome, LoadedModel,
    ModelSelection, ResolvedModel,
};

pub(super) const REPOSITORY: &str = "fixture/tiny-llama";
pub(super) const CHAT_REPOSITORY: &str = "TinyLlama/TinyLlama-1.1B-Chat-v1.0";
pub(super) const CHAT_COMMIT: &str = "fe8a4ea1ffedaf415f4da2f062534de366a451e6";
pub(super) const REVISION: &str = "phase7";
pub(super) const COMMIT: &str = "fixture";
pub(super) const TEST_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const TEST_POLL: Duration = Duration::from_millis(1);
pub(super) const TEST_CUDA_MEMORY_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub(super) const CUDA_ZERO: ApplicationDevice = ApplicationDevice::Cuda { ordinal: 0 };

const CANDLE_FIXTURE_WEIGHT_BYTES: u64 = 4_800;
const CANDLE_FIXTURE_WEIGHT_SHA256: [u8; 32] = [
    0xcc, 0x47, 0x98, 0xaf, 0x93, 0x48, 0x8b, 0x4f, 0xb2, 0xae, 0x05, 0x48, 0xc2, 0xb2, 0x8a, 0xce,
    0x60, 0x05, 0x21, 0x73, 0x2b, 0x52, 0x02, 0x3a, 0x77, 0x86, 0xc3, 0x22, 0x7d, 0x72, 0xd6, 0x72,
];

static NEXT_DATABASE_ID: AtomicU64 = AtomicU64::new(1);

pub(super) type TestResult<T = ()> = Result<T, String>;

pub(super) const fn retryable_failed_load_cleanup_failure() -> RuntimeError {
    RuntimeError::CleanupFailed(CleanupFailureReport::new(
        RuntimeOperation::ModelLoad,
        FailureClass::Load,
        RuntimeOperation::FailedLoadCleanup,
        FailureClass::Synchronization,
    ))
}

pub(super) const fn retryable_model_unload_cleanup_failure() -> RuntimeError {
    RuntimeError::CleanupFailed(CleanupFailureReport::new(
        RuntimeOperation::ModelUnload,
        FailureClass::Completion,
        RuntimeOperation::ModelUnload,
        FailureClass::Synchronization,
    ))
}

pub(super) const fn exhausted_failed_load_cleanup_failure() -> RuntimeError {
    RuntimeError::CleanupRetryExhausted(CleanupRetryState {
        resource: CleanupResource::FailedLoad {
            model_id: ModelId::new(1),
        },
        failure: CleanupFailureReport::new(
            RuntimeOperation::ModelLoad,
            FailureClass::Load,
            RuntimeOperation::FailedLoadCleanup,
            FailureClass::Synchronization,
        ),
        attempts: 3,
        maximum_attempts: 3,
    })
}

pub(super) const fn terminal_cleanup_failure() -> RuntimeError {
    RuntimeError::CleanupRetryExhausted(CleanupRetryState {
        resource: CleanupResource::Model {
            model_id: ModelId::new(1),
        },
        failure: CleanupFailureReport::new(
            RuntimeOperation::Shutdown,
            FailureClass::Shutdown,
            RuntimeOperation::ModelUnload,
            FailureClass::Synchronization,
        ),
        attempts: 3,
        maximum_attempts: 3,
    })
}

pub(super) fn with_loaded_runtime<C, F>(configure: C, test: F) -> TestResult
where
    C: FnOnce(&mut ApplicationRuntimeConfiguration),
    F: FnOnce(&mut ApplicationRuntime, LoadedModel) -> TestResult,
{
    with_runtime(configure, |runtime| {
        let loaded = load_fixture(runtime)?;
        test(runtime, loaded)
    })
}

pub(super) fn with_loaded_chat_runtime<C, F>(configure: C, test: F) -> TestResult
where
    C: FnOnce(&mut ApplicationRuntimeConfiguration),
    F: FnOnce(&mut ApplicationRuntime, LoadedModel) -> TestResult,
{
    with_runtime(configure, |runtime| {
        let loaded =
            load_fixture_with(runtime, CHAT_REPOSITORY, CHAT_COMMIT, "chat-tokenizer.json")?;
        test(runtime, loaded)
    })
}

pub(super) fn with_runtime<C, F>(configure: C, test: F) -> TestResult
where
    C: FnOnce(&mut ApplicationRuntimeConfiguration),
    F: FnOnce(&mut ApplicationRuntime) -> TestResult,
{
    with_runtime_and_probe(configure, crate::local::probe_application_device, test)
}

pub(super) fn with_runtime_and_probe<C, F>(
    configure: C,
    device_probe: crate::local::DeviceProbe,
    test: F,
) -> TestResult
where
    C: FnOnce(&mut ApplicationRuntimeConfiguration),
    F: FnOnce(&mut ApplicationRuntime) -> TestResult,
{
    let database_path = unique_database_path();
    let result = with_runtime_at_with_probe(&database_path, configure, device_probe, test);
    let cleanup_result = remove_database(&database_path);
    result.and(cleanup_result)
}

pub(super) fn with_runtime_at<C, F>(database_path: &Path, configure: C, test: F) -> TestResult
where
    C: FnOnce(&mut ApplicationRuntimeConfiguration),
    F: FnOnce(&mut ApplicationRuntime) -> TestResult,
{
    with_runtime_at_with_probe(
        database_path,
        configure,
        crate::local::probe_application_device,
        test,
    )
}

pub(super) fn with_runtime_at_with_probe<C, F>(
    database_path: &Path,
    configure: C,
    device_probe: crate::local::DeviceProbe,
    test: F,
) -> TestResult
where
    C: FnOnce(&mut ApplicationRuntimeConfiguration),
    F: FnOnce(&mut ApplicationRuntime) -> TestResult,
{
    let mut configuration = ApplicationRuntimeConfiguration::desktop(database_path);
    configure(&mut configuration);
    match ApplicationRuntime::start_with_device_probe(configuration, device_probe) {
        Ok(mut runtime) => {
            let test_result = test(&mut runtime);
            let shutdown_result = runtime.shutdown().map_err(application_error);
            test_result.and(shutdown_result)
        }
        Err(error) => Err(application_error(error)),
    }
}

pub(super) const fn default_test_configuration(
    configuration: &mut ApplicationRuntimeConfiguration,
) {
    configuration.defaults.maximum_host_memory_bytes = u64::MAX;
    configuration.defaults.drain_timeout_milliseconds = 5_000;
    configuration.timing.runtime_poll = TEST_POLL;
    configuration.timing.hub_worker_poll = TEST_POLL;
}

pub(super) const fn backpressure_test_configuration(
    configuration: &mut ApplicationRuntimeConfiguration,
) {
    default_test_configuration(configuration);
    configuration.token_output_capacity = 1;
    configuration.token_output_record_capacity = 4;
    configuration.text_output_byte_capacity = 8;
    configuration.text_output_record_capacity = 1;
}

pub(super) fn load_fixture(runtime: &mut ApplicationRuntime) -> TestResult<LoadedModel> {
    load_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")
}

pub(super) fn resolve_fixture_with(
    runtime: &mut ApplicationRuntime,
    repository: &str,
    commit: &str,
    tokenizer_filename: &str,
) -> TestResult<(ModelSelection, ResolvedModel)> {
    resolve_fixture_with_configuration(
        runtime,
        repository,
        commit,
        tokenizer_filename,
        &candle_fixture_configuration_path(),
        Some(ArtifactScalarType::F32),
    )
}

pub(super) fn resolve_fixture_with_configuration(
    runtime: &mut ApplicationRuntime,
    repository: &str,
    commit: &str,
    tokenizer_filename: &str,
    config_path: &Path,
    configuration_declared_scalar_type: Option<ArtifactScalarType>,
) -> TestResult<(ModelSelection, ResolvedModel)> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candle = manifest.join("../inference-runtime/tests/fixtures/candle-llama");
    let artifacts = ResolvedSafetensorsLlamaArtifacts {
        repository: repository.to_owned(),
        revision: REVISION.to_owned(),
        commit: commit.to_owned(),
        configuration_declared_scalar_type,
        config_path: canonical(config_path)?,
        tokenizer_path: canonical(manifest.join("tests/fixtures").join(tokenizer_filename))?,
        weight_shards: vec![ResolvedSafetensorsShard {
            path: canonical(candle.join("model.safetensors"))?,
            content_identity: ArtifactContentIdentity {
                byte_length: CANDLE_FIXTURE_WEIGHT_BYTES,
                sha256: CANDLE_FIXTURE_WEIGHT_SHA256,
                authority: ArtifactContentIdentityAuthority::ProjectEstablished,
            },
        }],
    };
    let selection = ModelSelection::new(repository, REVISION);
    match runtime.accept_resolved_artifacts(artifacts) {
        ApplicationEvent::ModelResolved {
            model,
            persistence_warning,
        } => {
            assert!(persistence_warning.is_none());
            assert_eq!(model.selection(), &selection);
            assert_eq!(model.engine(), ApplicationEngine::Candle);
            assert_eq!(model.source(), ApplicationSource::HuggingFaceHub);
            assert_eq!(model.format(), ApplicationModelFormat::Safetensors);
            assert_eq!(
                model.configuration_declared_scalar_type(),
                configuration_declared_scalar_type
                    .map(crate::support::application_configuration_declared_scalar_type)
            );
            assert_eq!(model.identity().repository(), repository);
            assert_eq!(model.identity().commit(), commit);
            Ok((selection, model))
        }
        event => Err(format!("unexpected fixture-resolution event: {event:?}")),
    }
}

pub(super) fn load_fixture_with(
    runtime: &mut ApplicationRuntime,
    repository: &str,
    commit: &str,
    tokenizer_filename: &str,
) -> TestResult<LoadedModel> {
    let (selection, _resolved) =
        resolve_fixture_with(runtime, repository, commit, tokenizer_filename)?;
    let expected_device = runtime.state().selected_device();
    runtime.load_model(&selection).map_err(application_error)?;
    let event = wait_for_event(runtime, |event| {
        matches!(
            event,
            ApplicationEvent::ModelLoaded { .. }
                | ApplicationEvent::ModelLoadFailed { .. }
                | ApplicationEvent::ModelCleanupPending { .. }
                | ApplicationEvent::ModelCompatibilityFailed { .. }
        )
    })?;
    match event {
        ApplicationEvent::ModelLoaded { model } => {
            assert_eq!(model.selection(), &selection);
            assert_eq!(model.engine(), ApplicationEngine::Candle);
            assert_eq!(model.source(), ApplicationSource::HuggingFaceHub);
            assert_eq!(model.device(), expected_device);
            assert_eq!(
                runtime.state().loaded().map(LoadedModel::device),
                Some(expected_device)
            );
            assert_eq!(model.format(), ApplicationModelFormat::Safetensors);
            assert_eq!(model.execution_scalar_type(), ApplicationScalarType::F32);
            assert_eq!(model.identity().repository(), repository);
            assert_eq!(model.identity().commit(), commit);
            Ok(model)
        }
        event => Err(format!("fixture model did not load: {event:?}")),
    }
}

pub(super) fn receive_successful_load_receipt(
    runtime: &mut ApplicationRuntime,
) -> TestResult<(CommandTicket, LoadReceipt)> {
    let event = runtime
        .local
        .receive_timeout(TEST_TIMEOUT)
        .map_err(|error| format!("model load event failed: {error:?}"))?;
    let RuntimeEvent::ModelLoaded {
        ticket,
        result: Ok(receipt),
    } = event
    else {
        return Err("unexpected model load event".to_owned());
    };
    Ok((ticket, receipt))
}

pub(super) fn wait_for_generation_started(
    runtime: &mut ApplicationRuntime,
    request_id: RequestId,
) -> TestResult {
    let event = wait_for_event(runtime, |event| {
        matches!(
            event,
            ApplicationEvent::GenerationStarted {
                request_id: event_request
            } if *event_request == request_id
        ) || matches!(event, ApplicationEvent::GenerationFinished { .. })
    })?;
    match event {
        ApplicationEvent::GenerationStarted {
            request_id: event_request,
        } if event_request == request_id => Ok(()),
        event => Err(format!("generation was not admitted: {event:?}")),
    }
}

pub(super) struct CollectedGeneration {
    pub(super) terminal: GenerationTerminal,
    pub(super) text: String,
    pub(super) states: Vec<ApplicationOutputState>,
}

pub(super) struct GenerationAndUnload {
    pub(super) generation: CollectedGeneration,
    pub(super) cancelled_requests: u32,
    pub(super) saw_draining: bool,
}

pub(super) fn collect_generation(
    runtime: &mut ApplicationRuntime,
    request_id: RequestId,
) -> TestResult<CollectedGeneration> {
    let deadline = deadline()?;
    let mut text = String::new();
    let mut states = Vec::new();
    loop {
        pull_output(runtime, &mut text, &mut states)?;
        if let Some(event) = runtime.poll_event()
            && let ApplicationEvent::GenerationFinished { terminal } = event
            && terminal.request_id == request_id
        {
            pull_output(runtime, &mut text, &mut states)?;
            return Ok(CollectedGeneration {
                terminal,
                text,
                states,
            });
        }
        ensure_before_deadline(deadline, "generation completion")?;
        std::thread::sleep(TEST_POLL);
    }
}

pub(super) fn collect_generation_and_unload(
    runtime: &mut ApplicationRuntime,
    request_id: RequestId,
) -> TestResult<GenerationAndUnload> {
    let deadline = deadline()?;
    let mut text = String::new();
    let mut states = Vec::new();
    let mut terminal = None;
    let mut cancelled_requests = None;
    let mut saw_draining = false;

    loop {
        pull_output(runtime, &mut text, &mut states)?;
        if let Some(event) = runtime.poll_event() {
            match event {
                ApplicationEvent::GenerationFinished { terminal: finished }
                    if finished.request_id == request_id =>
                {
                    terminal = Some(finished);
                }
                ApplicationEvent::ModelDraining { .. } => saw_draining = true,
                ApplicationEvent::ModelUnloaded {
                    cancelled_requests: cancelled,
                    ..
                } => cancelled_requests = Some(cancelled),
                ApplicationEvent::ModelUnloadFailed { failure } => {
                    return Err(format!("model unload failed: {failure}"));
                }
                _ => {}
            }
        }

        if let Some(cancelled_requests) = cancelled_requests
            && let Some(terminal) = terminal.take()
        {
            pull_output(runtime, &mut text, &mut states)?;
            return Ok(GenerationAndUnload {
                generation: CollectedGeneration {
                    terminal,
                    text,
                    states,
                },
                cancelled_requests,
                saw_draining,
            });
        }
        ensure_before_deadline(deadline, "generation and unload completion")?;
        std::thread::sleep(TEST_POLL);
    }
}

fn pull_output(
    runtime: &mut ApplicationRuntime,
    text: &mut String,
    states: &mut Vec<ApplicationOutputState>,
) -> TestResult {
    runtime
        .pull_output(|batch| {
            for record in batch.records() {
                match record.kind {
                    ApplicationOutputRecordKind::Text(_) => {
                        let fragment = batch
                            .text_for(record)
                            .ok_or_else(|| "application text range was invalid".to_owned())?;
                        text.push_str(fragment);
                    }
                    ApplicationOutputRecordKind::State(state) => states.push(state),
                }
            }
            Ok::<(), String>(())
        })
        .map_err(application_error)??;
    Ok(())
}

pub(super) fn wait_for_event<F>(
    runtime: &mut ApplicationRuntime,
    mut matches: F,
) -> TestResult<ApplicationEvent>
where
    F: FnMut(&ApplicationEvent) -> bool,
{
    let deadline = deadline()?;
    loop {
        if let Some(event) = runtime.poll_event()
            && matches(&event)
        {
            return Ok(event);
        }
        ensure_before_deadline(deadline, "application event")?;
        std::thread::sleep(TEST_POLL);
    }
}

pub(super) const fn deterministic_settings(maximum_new_tokens: u32) -> GenerationSettings {
    GenerationSettings {
        maximum_new_tokens,
        temperature: 1.0,
        top_k: 1,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        repetition_window: 0,
        seed: GenerationSeed::Fixed(7),
        eos_tokens: Vec::new(),
        stop_sequences: Vec::new(),
    }
}

pub(super) fn unique_database_path() -> PathBuf {
    let identifier = NEXT_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "llm-app-phase5-{}-{identifier}.redb",
        std::process::id()
    ))
}

pub(super) fn write_fixture_configuration_without_declaration(destination: &Path) -> TestResult {
    const DECLARATION: &str = "  \"dtype\": \"float32\",\n";

    let source = candle_fixture_configuration_path();
    let configuration = fs::read_to_string(&source).map_err(|error| {
        format!(
            "failed to read fixture configuration {}: {error}",
            source.display()
        )
    })?;
    if configuration.matches(DECLARATION).count() != 1 {
        return Err("fixture configuration has an unexpected dtype declaration".to_owned());
    }
    fs::write(destination, configuration.replacen(DECLARATION, "", 1)).map_err(|error| {
        format!(
            "failed to write declaration-free fixture configuration {}: {error}",
            destination.display()
        )
    })
}

pub(super) fn remove_test_file(path: &Path) -> TestResult {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to remove test file {}: {error}",
            path.display()
        )),
    }
}

pub(super) fn remove_database(path: &Path) -> TestResult {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove test database: {error}")),
    }
}

fn candle_fixture_configuration_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../inference-runtime/tests/fixtures/candle-llama/config.json")
}

fn canonical(path: impl AsRef<Path>) -> TestResult<PathBuf> {
    let path = path.as_ref();
    fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve fixture path {}: {error}", path.display()))
}

pub(super) fn deadline() -> TestResult<Instant> {
    Instant::now()
        .checked_add(TEST_TIMEOUT)
        .ok_or_else(|| "test deadline overflow".to_owned())
}

pub(super) fn ensure_before_deadline(deadline: Instant, context: &str) -> TestResult {
    if Instant::now() >= deadline {
        Err(format!("timed out waiting for {context}"))
    } else {
        Ok(())
    }
}

pub(super) fn application_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub(super) fn assert_cancelled_by_user(result: &CollectedGeneration) {
    assert_eq!(
        result.terminal.outcome,
        GenerationTerminalOutcome::Finished(FinishReason::Cancelled(
            CancellationReason::UserRequested
        ))
    );
}

pub(super) fn assert_released_token_limit(result: &CollectedGeneration) {
    assert!(result.states.contains(&ApplicationOutputState::Released(
        GenerationTerminalKind::Finished(FinishReason::TokenLimit)
    )));
}
