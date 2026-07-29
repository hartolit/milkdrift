use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use domain_contracts::{CancellationReason, FinishReason, RequestId, TokenId};
use hf_hub_adapter::{ArtifactScalarType, ResolvedModelArtifacts};
use inference_runtime::{RuntimeCommand, RuntimeEvent};

use super::ApplicationRuntime;
use crate::{
    ApplicationActivity, ApplicationError, ApplicationEvent, ApplicationOutputRecordKind,
    ApplicationOutputState, ApplicationRuntimeConfiguration, ConversationRole, GenerationPhase,
    GenerationSeed, GenerationSettings, GenerationSettingsField, GenerationTerminal,
    GenerationTerminalKind, GenerationTerminalOutcome, LoadedModel, ModelUnloadBehavior,
    ResponseAttemptState,
};

const REPOSITORY: &str = "fixture/tiny-llama";
const CHAT_REPOSITORY: &str = "TinyLlama/TinyLlama-1.1B-Chat-v1.0";
const CHAT_COMMIT: &str = "fe8a4ea1ffedaf415f4da2f062534de366a451e6";
const REVISION: &str = "phase7";
const COMMIT: &str = "fixture";
const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const TEST_POLL: Duration = Duration::from_millis(1);

static NEXT_DATABASE_ID: AtomicU64 = AtomicU64::new(1);

type TestResult<T = ()> = Result<T, String>;

#[test]
fn generation_requires_a_loaded_model() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        assert_eq!(
            runtime.start_generation("prompt seed", deterministic_settings(1)),
            Err(ApplicationError::NoLoadedModel)
        );
        Ok(())
    })
}

#[test]
fn direct_completion_streams_text_and_releases_state() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, _loaded| {
        let request_id = runtime
            .start_generation("prompt seed", deterministic_settings(3))
            .map_err(application_error)?;
        assert_eq!(
            runtime
                .state()
                .active_generation()
                .map(|summary| summary.phase),
            Some(GenerationPhase::Starting)
        );

        wait_for_generation_started(runtime, request_id)?;
        assert_eq!(
            runtime
                .state()
                .active_generation()
                .map(|summary| summary.phase),
            Some(GenerationPhase::Running)
        );

        let result = collect_generation(runtime, request_id)?;
        assert_eq!(result.text, "seed seed seed");
        assert_eq!(
            result.terminal.outcome,
            GenerationTerminalOutcome::Finished(FinishReason::TokenLimit)
        );
        assert_eq!(result.terminal.usage.prompt_tokens, 2);
        assert_eq!(result.terminal.usage.generated_tokens, 3);
        assert!(result.states.contains(&ApplicationOutputState::Released(
            GenerationTerminalKind::Finished(FinishReason::TokenLimit)
        )));
        assert!(runtime.state().active_generation().is_none());
        assert_eq!(runtime.state().last_generation(), Some(&result.terminal));
        Ok(())
    })
}

#[test]
fn compatible_chat_plans_and_submits_the_rendered_prompt_with_profile_eos() -> TestResult {
    with_loaded_chat_runtime(default_test_configuration, |runtime, loaded| {
        assert!(runtime.can_submit_chat_message());
        let request_id = runtime
            .submit_user_message("hello", deterministic_settings(3))
            .map_err(application_error)?;
        assert!(!runtime.can_submit_chat_message());
        let diagnostics = runtime
            .context_diagnostics()
            .cloned()
            .ok_or_else(|| "chat context diagnostics were not published".to_owned())?;
        assert_eq!(diagnostics.selected.len(), 1);
        assert!(diagnostics.dropped.is_empty());
        assert!(
            diagnostics.actual_input_tokens + diagnostics.reserved_output_tokens
                <= loaded.maximum_context_tokens
        );
        assert!(diagnostics.actual_input_tokens > 1);

        wait_for_generation_started(runtime, request_id)?;
        let result = collect_generation(runtime, request_id)?;
        assert!(matches!(
            result.terminal.outcome,
            GenerationTerminalOutcome::Finished(_)
        ));
        assert_eq!(
            result.terminal.usage.prompt_tokens,
            u64::from(diagnostics.actual_input_tokens)
        );
        assert_eq!(runtime.conversation().len(), 2);
        let assistant = runtime
            .conversation()
            .get(1)
            .ok_or_else(|| "assistant attempt was not retained".to_owned())?;
        assert_eq!(assistant.role, ConversationRole::Assistant);
        assert!(assistant.is_active_context());
        assert!(matches!(
            assistant
                .response_attempt
                .as_ref()
                .map(|attempt| &attempt.state),
            Some(ResponseAttemptState::Completed(_))
        ));
        Ok(())
    })
}

#[test]
fn chat_attempt_becomes_terminal_before_backend_release() -> TestResult {
    with_loaded_chat_runtime(backpressure_test_configuration, |runtime, _loaded| {
        let request_id = runtime
            .submit_user_message("hello", deterministic_settings(3))
            .map_err(application_error)?;
        wait_for_generation_started(runtime, request_id)?;

        let deadline = deadline()?;
        loop {
            let mut states = Vec::new();
            runtime
                .pull_output(|batch| {
                    states.extend(batch.records().filter_map(|record| match record.kind {
                        ApplicationOutputRecordKind::State(state) => Some(state),
                        ApplicationOutputRecordKind::Text(_) => None,
                    }));
                })
                .map_err(application_error)?;

            if states.iter().any(|state| {
                matches!(
                    state,
                    ApplicationOutputState::Terminal(
                        GenerationTerminalKind::Finished(_) | GenerationTerminalKind::Failed
                    )
                )
            }) {
                let attempt = runtime
                    .conversation()
                    .last()
                    .and_then(|record| record.response_attempt.as_ref())
                    .ok_or_else(|| "terminal assistant attempt was not retained".to_owned())?;
                assert!(!matches!(&attempt.state, ResponseAttemptState::Streaming));
                assert_eq!(
                    runtime
                        .state()
                        .active_generation()
                        .map(|summary| summary.request_id),
                    Some(request_id)
                );
                assert_eq!(
                    runtime.clear_conversation(),
                    Err(ApplicationError::GenerationAlreadyActive(request_id))
                );
                break;
            }

            ensure_before_deadline(deadline, "generation terminal state before release")?;
            std::thread::sleep(TEST_POLL);
        }

        let _result = collect_generation(runtime, request_id)?;
        Ok(())
    })
}

#[test]
fn regeneration_preserves_superseded_attempt_and_clear_rejects_active_response() -> TestResult {
    with_loaded_chat_runtime(default_test_configuration, |runtime, _loaded| {
        let first_request = runtime
            .submit_user_message("hello", deterministic_settings(3))
            .map_err(application_error)?;
        assert_eq!(
            runtime.clear_conversation(),
            Err(ApplicationError::GenerationAlreadyActive(first_request))
        );
        wait_for_generation_started(runtime, first_request)?;
        let _first = collect_generation(runtime, first_request)?;

        let second_request = runtime
            .regenerate_last_response(deterministic_settings(3))
            .map_err(application_error)?;
        wait_for_generation_started(runtime, second_request)?;
        let _second = collect_generation(runtime, second_request)?;

        assert_eq!(runtime.conversation().len(), 3);
        let first_attempt = runtime
            .conversation()
            .get(1)
            .and_then(|record| record.response_attempt.as_ref())
            .ok_or_else(|| "first response attempt was not retained".to_owned())?;
        let second_attempt = runtime
            .conversation()
            .get(2)
            .and_then(|record| record.response_attempt.as_ref())
            .ok_or_else(|| "replacement response attempt was not retained".to_owned())?;
        assert!(first_attempt.superseded);
        assert!(!second_attempt.superseded);
        assert_ne!(first_attempt.id, second_attempt.id);
        assert!(
            !runtime
                .conversation()
                .get(1)
                .is_some_and(crate::ConversationRecord::is_active_context)
        );
        assert!(
            runtime
                .conversation()
                .get(2)
                .is_some_and(crate::ConversationRecord::is_active_context)
        );

        runtime.clear_conversation().map_err(application_error)?;
        assert!(runtime.conversation().is_empty());
        assert!(runtime.context_diagnostics().is_none());
        Ok(())
    })
}

#[test]
fn unanswered_committed_user_blocks_regeneration_of_the_previous_response() -> TestResult {
    with_loaded_chat_runtime(default_test_configuration, |runtime, loaded| {
        let first_request = runtime
            .submit_user_message("first", deterministic_settings(1))
            .map_err(application_error)?;
        wait_for_generation_started(runtime, first_request)?;
        let _first = collect_generation(runtime, first_request)?;
        assert!(runtime.can_regenerate_response());

        let oversized_output = loaded.maximum_context_tokens.saturating_add(1);
        let second = runtime.submit_user_message(
            "second unanswered message",
            deterministic_settings(oversized_output),
        );
        assert!(matches!(
            second,
            Err(ApplicationError::ContextCapacityExceeded { .. })
        ));
        assert_eq!(
            runtime.conversation().last().map(|record| record.role),
            Some(ConversationRole::User)
        );
        assert!(!runtime.can_regenerate_response());
        assert_eq!(
            runtime.regenerate_last_response(deterministic_settings(1)),
            Err(ApplicationError::NoRegenerableResponse)
        );
        Ok(())
    })
}

#[test]
fn pinned_overflow_keeps_committed_user_history_and_never_starts_an_attempt() -> TestResult {
    with_loaded_chat_runtime(default_test_configuration, |runtime, _loaded| {
        runtime
            .set_system_instruction(
                "old old old old old old old old old old old old old old old old old old",
            )
            .map_err(application_error)?;
        let result = runtime.submit_user_message("hello", deterministic_settings(3));

        assert!(matches!(
            result,
            Err(ApplicationError::PinnedBudgetExceeded { .. })
        ));
        assert_eq!(runtime.conversation().len(), 2);
        assert_eq!(
            runtime.conversation().get(1).map(|record| record.role),
            Some(ConversationRole::User)
        );
        assert!(runtime.state().active_generation().is_none());
        Ok(())
    })
}

#[test]
fn unknown_chat_compatibility_fails_without_guessing_a_template() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, _loaded| {
        assert!(!runtime.can_submit_chat_message());
        assert!(!runtime.can_regenerate_response());
        assert_eq!(
            runtime.submit_user_message("hello", deterministic_settings(1)),
            Err(ApplicationError::UnsupportedChatCompatibility)
        );
        assert!(runtime.conversation().is_empty());
        Ok(())
    })
}

#[test]
fn invalid_busy_and_eos_admission_are_normalized() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, _loaded| {
        let mut invalid = deterministic_settings(1);
        invalid.maximum_new_tokens = 0;
        assert_eq!(
            runtime.start_generation("prompt seed", invalid),
            Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::MaximumNewTokens
            ))
        );
        assert_eq!(
            runtime.start_generation("", deterministic_settings(1)),
            Err(ApplicationError::EmptyPrompt)
        );

        let mut eos = deterministic_settings(3);
        eos.eos_tokens.push(TokenId::new(2));
        let request_id = runtime
            .start_generation("prompt seed", eos)
            .map_err(application_error)?;
        assert_eq!(
            runtime.start_generation("prompt seed", deterministic_settings(1)),
            Err(ApplicationError::GenerationAlreadyActive(request_id))
        );

        wait_for_generation_started(runtime, request_id)?;
        let result = collect_generation(runtime, request_id)?;
        assert_eq!(result.text, "seed");
        assert_eq!(
            result.terminal.outcome,
            GenerationTerminalOutcome::Finished(FinishReason::EndOfSequence(TokenId::new(2)))
        );
        assert_eq!(result.terminal.usage.generated_tokens, 1);
        Ok(())
    })
}

#[test]
fn textual_stop_sequence_is_encoded_and_reported() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, _loaded| {
        let mut settings = deterministic_settings(4);
        settings.stop_sequences.push("seed".to_owned());
        let request_id = runtime
            .start_generation("prompt seed", settings)
            .map_err(application_error)?;
        wait_for_generation_started(runtime, request_id)?;

        let result = collect_generation(runtime, request_id)?;
        assert_eq!(result.text, "seed");
        assert_eq!(
            result.terminal.outcome,
            GenerationTerminalOutcome::Finished(FinishReason::StopCondition)
        );
        assert_eq!(result.terminal.usage.generated_tokens, 1);
        Ok(())
    })
}

#[test]
fn decoded_output_backpressure_resumes_without_loss() -> TestResult {
    with_loaded_runtime(backpressure_test_configuration, |runtime, _loaded| {
        let request_id = runtime
            .start_generation("prompt seed", deterministic_settings(3))
            .map_err(application_error)?;
        wait_for_generation_started(runtime, request_id)?;
        std::thread::sleep(Duration::from_millis(10));

        let result = collect_generation(runtime, request_id)?;
        assert_eq!(result.text, "seed seed seed");
        assert_eq!(result.terminal.usage.generated_tokens, 3);
        assert_eq!(
            result.terminal.outcome,
            GenerationTerminalOutcome::Finished(FinishReason::TokenLimit)
        );
        Ok(())
    })
}

#[test]
fn cancellation_remains_bounded_under_constrained_output_capacity() -> TestResult {
    with_loaded_runtime(backpressure_test_configuration, |runtime, _loaded| {
        let request_id = runtime
            .start_generation("prompt seed", deterministic_settings(12))
            .map_err(application_error)?;
        wait_for_generation_started(runtime, request_id)?;

        runtime
            .cancel_generation(request_id)
            .map_err(application_error)?;
        assert_eq!(
            runtime
                .state()
                .active_generation()
                .map(|summary| summary.phase),
            Some(GenerationPhase::Cancelling)
        );

        let result = collect_generation(runtime, request_id)?;
        assert_eq!(
            result.terminal.outcome,
            GenerationTerminalOutcome::Finished(FinishReason::Cancelled(
                CancellationReason::UserRequested
            ))
        );
        assert!(result.terminal.usage.generated_tokens < 12);
        Ok(())
    })
}

#[test]
fn reject_if_busy_preserves_active_generation_and_idle_unload_succeeds() -> TestResult {
    with_loaded_runtime(backpressure_test_configuration, |runtime, _loaded| {
        let request_id = runtime
            .start_generation("prompt seed", deterministic_settings(12))
            .map_err(application_error)?;
        wait_for_generation_started(runtime, request_id)?;

        runtime
            .unload_model_with_behavior(ModelUnloadBehavior::RejectIfBusy)
            .map_err(application_error)?;
        let event = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloadFailed { .. })
        })?;
        assert!(matches!(event, ApplicationEvent::ModelUnloadFailed { .. }));
        assert!(runtime.state().loaded().is_some());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        assert_eq!(
            runtime
                .state()
                .active_generation()
                .map(|summary| summary.request_id),
            Some(request_id)
        );

        runtime
            .cancel_generation(request_id)
            .map_err(application_error)?;
        let _result = collect_generation(runtime, request_id)?;

        runtime.unload_model().map_err(application_error)?;
        let unloaded = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        assert!(matches!(
            unloaded,
            ApplicationEvent::ModelUnloaded {
                cancelled_requests: 0,
                ..
            }
        ));
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn cancel_active_unload_cancels_and_releases_generation() -> TestResult {
    with_loaded_runtime(backpressure_test_configuration, |runtime, _loaded| {
        let request_id = runtime
            .start_generation("prompt seed", deterministic_settings(12))
            .map_err(application_error)?;
        wait_for_generation_started(runtime, request_id)?;

        runtime
            .unload_model_with_behavior(ModelUnloadBehavior::CancelActive)
            .map_err(application_error)?;
        let result = collect_generation_and_unload(runtime, request_id)?;

        assert_eq!(result.cancelled_requests, 1);
        assert_eq!(
            result.generation.terminal.outcome,
            GenerationTerminalOutcome::Finished(FinishReason::Cancelled(
                CancellationReason::ModelUnload
            ))
        );
        assert!(runtime.state().loaded().is_none());
        assert!(runtime.state().active_generation().is_none());
        Ok(())
    })
}

#[test]
fn drain_unload_allows_natural_completion_before_release() -> TestResult {
    with_loaded_runtime(backpressure_test_configuration, |runtime, _loaded| {
        let request_id = runtime
            .start_generation("prompt seed", deterministic_settings(3))
            .map_err(application_error)?;
        wait_for_generation_started(runtime, request_id)?;

        runtime
            .unload_model_with_behavior(ModelUnloadBehavior::Drain)
            .map_err(application_error)?;
        let result = collect_generation_and_unload(runtime, request_id)?;

        assert!(result.saw_draining);
        assert_eq!(result.cancelled_requests, 0);
        assert_eq!(
            result.generation.terminal.outcome,
            GenerationTerminalOutcome::Finished(FinishReason::TokenLimit)
        );
        assert_eq!(result.generation.text, "seed seed seed");
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn application_reports_inference_worker_disconnection() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let ticket = runtime.next_ticket().map_err(application_error)?;
        runtime
            .inference
            .try_submit(RuntimeCommand::Shutdown { ticket })
            .map_err(|error| format!("shutdown command rejected: {error:?}"))?;
        match runtime
            .inference
            .receive_timeout(TEST_TIMEOUT)
            .map_err(|error| format!("shutdown event failed: {error:?}"))?
        {
            RuntimeEvent::Shutdown {
                ticket: event_ticket,
                result: Ok(_),
            } if event_ticket == ticket => {}
            _ => return Err("unexpected shutdown event".to_owned()),
        }
        let thread = runtime
            .inference_thread
            .take()
            .ok_or_else(|| "inference thread was already absent".to_owned())?;
        thread
            .join()
            .map_err(|error| format!("inference worker join failed: {error:?}"))?;

        let event = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::RuntimeDisconnected)
        })?;
        assert_eq!(event, ApplicationEvent::RuntimeDisconnected);
        assert!(!runtime.state().inference_available());
        Ok(())
    })
}

#[test]
fn explicit_application_shutdown_disconnects_and_joins_workers() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        runtime.shutdown().map_err(application_error)?;
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::ShuttingDown
        );
        assert!(!runtime.state().hub_available());
        assert!(!runtime.state().inference_available());
        assert!(runtime.hub_thread.is_none());
        assert!(runtime.inference_thread.is_none());
        Ok(())
    })
}

struct CollectedGeneration {
    terminal: GenerationTerminal,
    text: String,
    states: Vec<ApplicationOutputState>,
}

struct GenerationAndUnload {
    generation: CollectedGeneration,
    cancelled_requests: u32,
    saw_draining: bool,
}

fn with_loaded_runtime<C, F>(configure: C, test: F) -> TestResult
where
    C: FnOnce(&mut ApplicationRuntimeConfiguration),
    F: FnOnce(&mut ApplicationRuntime, LoadedModel) -> TestResult,
{
    with_runtime(configure, |runtime| {
        let loaded = load_fixture(runtime)?;
        test(runtime, loaded)
    })
}

fn with_loaded_chat_runtime<C, F>(configure: C, test: F) -> TestResult
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

fn with_runtime<C, F>(configure: C, test: F) -> TestResult
where
    C: FnOnce(&mut ApplicationRuntimeConfiguration),
    F: FnOnce(&mut ApplicationRuntime) -> TestResult,
{
    let database_path = unique_database_path();
    let result = {
        let mut configuration = ApplicationRuntimeConfiguration::desktop(&database_path);
        configure(&mut configuration);
        match ApplicationRuntime::start(configuration) {
            Ok(mut runtime) => {
                let test_result = test(&mut runtime);
                let shutdown_result = runtime.shutdown().map_err(application_error);
                test_result.and(shutdown_result)
            }
            Err(error) => Err(application_error(error)),
        }
    };

    let cleanup_result = remove_database(&database_path);
    result.and(cleanup_result)
}

const fn default_test_configuration(configuration: &mut ApplicationRuntimeConfiguration) {
    configuration.defaults.maximum_host_memory_bytes = u64::MAX;
    configuration.defaults.drain_timeout_milliseconds = 5_000;
    configuration.timing.runtime_poll = TEST_POLL;
    configuration.timing.hub_worker_poll = TEST_POLL;
}

const fn backpressure_test_configuration(configuration: &mut ApplicationRuntimeConfiguration) {
    default_test_configuration(configuration);
    configuration.token_output_capacity = 1;
    configuration.token_output_record_capacity = 4;
    configuration.text_output_byte_capacity = 8;
    configuration.text_output_record_capacity = 1;
}

fn load_fixture(runtime: &mut ApplicationRuntime) -> TestResult<LoadedModel> {
    load_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")
}

fn load_fixture_with(
    runtime: &mut ApplicationRuntime,
    repository: &str,
    commit: &str,
    tokenizer_filename: &str,
) -> TestResult<LoadedModel> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candle = manifest.join("../inference-runtime/tests/fixtures/candle-llama");
    let artifacts = ResolvedModelArtifacts {
        repository: repository.to_owned(),
        revision: REVISION.to_owned(),
        commit: commit.to_owned(),
        declared_scalar_type: Some(ArtifactScalarType::F32),
        config_path: canonical(candle.join("config.json"))?,
        tokenizer_path: canonical(manifest.join("tests/fixtures").join(tokenizer_filename))?,
        weight_paths: vec![canonical(candle.join("model.safetensors"))?],
    };
    match runtime.accept_resolved_artifacts(artifacts) {
        ApplicationEvent::ModelResolved { .. } => {}
        event => return Err(format!("unexpected fixture-resolution event: {event:?}")),
    }

    runtime
        .load_model(repository, REVISION)
        .map_err(application_error)?;
    let event = wait_for_event(runtime, |event| {
        matches!(
            event,
            ApplicationEvent::ModelLoaded { .. }
                | ApplicationEvent::ModelLoadFailed { .. }
                | ApplicationEvent::ModelCompatibilityFailed { .. }
        )
    })?;
    match event {
        ApplicationEvent::ModelLoaded { model } => Ok(model),
        event => Err(format!("fixture model did not load: {event:?}")),
    }
}

fn wait_for_generation_started(
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

fn collect_generation(
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

fn collect_generation_and_unload(
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
                ApplicationEvent::ModelDraining { .. } => {
                    saw_draining = true;
                }
                ApplicationEvent::ModelUnloaded {
                    cancelled_requests: cancelled,
                    ..
                } => {
                    cancelled_requests = Some(cancelled);
                }
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

fn wait_for_event<F>(
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

const fn deterministic_settings(maximum_new_tokens: u32) -> GenerationSettings {
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

fn unique_database_path() -> PathBuf {
    let identifier = NEXT_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "llm-app-phase5-{}-{identifier}.redb",
        std::process::id()
    ))
}

fn remove_database(path: &Path) -> TestResult {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove test database: {error}")),
    }
}

fn canonical(path: impl AsRef<Path>) -> TestResult<PathBuf> {
    let path = path.as_ref();
    fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve fixture path {}: {error}", path.display()))
}

fn deadline() -> TestResult<Instant> {
    Instant::now()
        .checked_add(TEST_TIMEOUT)
        .ok_or_else(|| "test deadline overflow".to_owned())
}

fn ensure_before_deadline(deadline: Instant, context: &str) -> TestResult {
    if Instant::now() >= deadline {
        Err(format!("timed out waiting for {context}"))
    } else {
        Ok(())
    }
}

fn application_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
