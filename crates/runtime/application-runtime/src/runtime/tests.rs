use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use domain_contracts::{CancellationReason, FinishReason, RequestId, TokenId};
use hf_hub_adapter::{ArtifactScalarType, ResolvedSafetensorsLlamaArtifacts};
use inference_runtime::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, FailureClass, RuntimeCommand,
    RuntimeError, RuntimeEvent, RuntimeOperation,
};

use super::{
    ApplicationRuntime, IncompatibleModelUnload, MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS,
};
use crate::shutdown::ShutdownStatus;
use crate::support::MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT;
use crate::{
    ApplicationActivity, ApplicationConfigurationField, ApplicationDevice, ApplicationEngine,
    ApplicationError, ApplicationEvent, ApplicationFailure, ApplicationFailureKind,
    ApplicationModelFormat, ApplicationOutputRecordKind, ApplicationOutputState,
    ApplicationRuntimeConfiguration, ApplicationScalarType, ApplicationSource, ApplicationWorker,
    ConversationRole, GenerationPhase, GenerationSeed, GenerationSettings, GenerationSettingsField,
    GenerationTerminal, GenerationTerminalKind, GenerationTerminalOutcome, LoadedModel,
    ModelSelection, ModelUnloadBehavior, ResolvedModel, ResponseAttemptState,
};

const REPOSITORY: &str = "fixture/tiny-llama";
const CHAT_REPOSITORY: &str = "TinyLlama/TinyLlama-1.1B-Chat-v1.0";
const CHAT_COMMIT: &str = "fe8a4ea1ffedaf415f4da2f062534de366a451e6";
const REVISION: &str = "phase7";
const COMMIT: &str = "fixture";
const TEST_TIMEOUT: Duration = Duration::from_secs(10);
const TEST_POLL: Duration = Duration::from_millis(1);

const fn terminal_cleanup_failure() -> RuntimeError {
    RuntimeError::CleanupRetryExhausted(CleanupRetryState {
        resource: CleanupResource::Model {
            model_id: domain_contracts::ModelId::new(1),
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
fn resolved_selection_persists_across_application_restart() -> TestResult {
    let database_path = unique_database_path();
    let result = with_runtime_at(&database_path, default_test_configuration, |runtime| {
        let (_selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        assert_eq!(runtime.preferences().default_repository, REPOSITORY);
        assert_eq!(runtime.preferences().default_revision, REVISION);
        Ok(())
    })
    .and_then(|()| {
        with_runtime_at(&database_path, default_test_configuration, |runtime| {
            assert_eq!(runtime.preferences().default_repository, REPOSITORY);
            assert_eq!(runtime.preferences().default_revision, REVISION);
            Ok(())
        })
    });

    let cleanup_result = remove_database(&database_path);
    result.and(cleanup_result)
}

#[test]
fn candle_runs_e1_direct_completion_scenario() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        run_e1_direct_completion_scenario(
            runtime,
            &loaded,
            DirectCompletionExpectation {
                prompt: "prompt seed",
                text: "seed seed seed",
                prompt_tokens: 2,
            },
        )
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
                <= loaded.maximum_context_tokens()
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

        let oversized_output = loaded.maximum_context_tokens().saturating_add(1);
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
fn repository_or_revision_change_is_rejected_after_resolution() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        let changed_repository = ModelSelection::new("fixture/other-model", REVISION);
        assert_eq!(
            runtime.load_model(&changed_repository),
            Err(ApplicationError::SelectionChanged)
        );
        let changed_revision = ModelSelection::new(REPOSITORY, "other-revision");
        assert_eq!(
            runtime.load_model(&changed_revision),
            Err(ApplicationError::SelectionChanged)
        );
        assert!(runtime.state().can_load(&selection));
        Ok(())
    })
}

#[test]
fn incompatible_scalar_evidence_unloads_without_publishing_loaded_state() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        runtime
            .pending_load
            .as_mut()
            .ok_or_else(|| "load admission evidence was not retained".to_owned())?
            .scalar_type = ApplicationScalarType::Bf16;

        let event = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelCompatibilityFailed { .. })
        })?;
        assert!(matches!(
            event,
            ApplicationEvent::ModelCompatibilityFailed { .. }
        ));
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);

        let event = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        assert!(matches!(
            event,
            ApplicationEvent::ModelUnloaded {
                cancelled_requests: 0,
                ..
            }
        ));
        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        Ok(())
    })
}

#[test]
fn incompatible_model_cleanup_retries_after_automatic_unload_submission_failure() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        runtime
            .pending_load
            .as_mut()
            .ok_or_else(|| "load admission evidence was not retained".to_owned())?
            .scalar_type = ApplicationScalarType::Bf16;
        runtime.forced_inference_busy_submissions = 1;

        let event = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelLoadFailed { .. })
        })?;
        assert!(matches!(event, ApplicationEvent::ModelLoadFailed { .. }));
        let retained = runtime
            .incompatible_model_cleanup
            .as_ref()
            .ok_or_else(|| "incompatible model ownership was not retained".to_owned())?;
        let retained_handle = retained.handle;
        assert!(
            retained
                .compatibility_failure
                .message
                .contains("compatibility")
        );
        assert!(matches!(
            retained.unload,
            IncompatibleModelUnload::PendingSubmission { attempts: 1, .. }
        ));
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert!(runtime.state().loaded().is_none());

        let event = wait_for_event(runtime, |event| {
            matches!(
                event,
                ApplicationEvent::ModelUnloaded { handle, .. } if *handle == retained_handle
            )
        })?;
        assert!(matches!(event, ApplicationEvent::ModelUnloaded { .. }));
        assert!(runtime.incompatible_model_cleanup.is_none());
        assert_eq!(runtime.state().activity(), ApplicationActivity::Idle);
        Ok(())
    })
}

#[test]
fn incompatible_model_cleanup_exhaustion_remains_accounted() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        runtime
            .pending_load
            .as_mut()
            .ok_or_else(|| "load admission evidence was not retained".to_owned())?
            .scalar_type = ApplicationScalarType::Bf16;
        runtime.forced_inference_busy_submissions =
            usize::from(MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS);

        let _initial_failure = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelLoadFailed { .. })
        })?;
        for _ in 1..MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS {
            let _retry_failure = wait_for_event(runtime, |event| {
                matches!(event, ApplicationEvent::ModelUnloadFailed { .. })
            })?;
        }

        let retained = runtime
            .incompatible_model_cleanup
            .as_ref()
            .ok_or_else(|| "exhausted incompatible model ownership was dropped".to_owned())?;
        assert!(
            retained
                .compatibility_failure
                .message
                .contains("compatibility")
        );
        assert!(matches!(
            retained.unload,
            IncompatibleModelUnload::RetryExhausted {
                attempts: MAXIMUM_INCOMPATIBLE_UNLOAD_SUBMISSION_ATTEMPTS,
                ..
            }
        ));
        assert_eq!(runtime.state().activity(), ApplicationActivity::Unloading);
        assert!(runtime.state().loaded().is_none());
        Ok(())
    })
}

#[test]
fn application_retains_inference_worker_disconnection_as_terminal() -> TestResult {
    let database_path = unique_database_path();
    let test_result = (|| {
        let mut configuration = ApplicationRuntimeConfiguration::desktop(&database_path);
        default_test_configuration(&mut configuration);
        let mut runtime = ApplicationRuntime::start(configuration).map_err(application_error)?;
        let ticket = runtime.next_ticket().map_err(application_error)?;
        runtime
            .submit_inference(RuntimeCommand::Shutdown { ticket })
            .map_err(application_error)?;
        match runtime
            .local
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
            .local
            .take_thread()
            .ok_or_else(|| "Candle inference thread was already absent".to_owned())?;
        thread
            .join()
            .map_err(|error| format!("inference worker join failed: {error:?}"))?;

        let event = wait_for_event(&mut runtime, |event| {
            matches!(event, ApplicationEvent::RuntimeDisconnected)
        })?;
        assert_eq!(event, ApplicationEvent::RuntimeDisconnected);
        assert!(!runtime.state().inference_available());
        assert_eq!(
            runtime.shutdown(),
            Err(ApplicationError::RuntimeDisconnected)
        );
        assert_eq!(
            runtime.shutdown_control.status,
            ShutdownStatus::TerminalFailure
        );
        assert_eq!(
            runtime.shutdown(),
            Err(ApplicationError::RuntimeDisconnected)
        );
        Ok(())
    })();

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
}

#[test]
fn shutdown_and_join_deadline_boundaries_are_validated_before_worker_start() -> TestResult {
    let mut maximum = ApplicationRuntimeConfiguration::desktop("unused.redb");
    default_test_configuration(&mut maximum);
    maximum.timing.runtime_shutdown_timeout = MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT;
    maximum.timing.runtime_join_timeout = MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT;
    maximum.timing.hub_shutdown_timeout = MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT;
    crate::support::validate_configuration(&maximum).map_err(application_error)?;

    assert_startup_deadline_duration_rejected(Duration::ZERO)?;
    assert_startup_deadline_duration_rejected(
        MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT + Duration::from_nanos(1),
    )?;
    assert_startup_deadline_duration_rejected(Duration::MAX)
}

#[test]
fn forced_hub_start_failure_stops_and_joins_started_inference_worker() -> TestResult {
    let database_path = unique_database_path();
    let mut configuration = ApplicationRuntimeConfiguration::desktop(&database_path);
    default_test_configuration(&mut configuration);
    let primary = ApplicationError::Failure(ApplicationFailure::new(
        ApplicationFailureKind::Hub,
        "forced Hub startup failure",
    ));

    let start_result =
        ApplicationRuntime::start_transaction(configuration, |_| Err(primary.clone()));
    let test_result = match start_result {
        Err(failure) => {
            assert_eq!(failure.primary, primary);
            assert_eq!(failure.inference_rollback, Some(Ok(())));
            Ok(())
        }
        Ok(mut runtime) => {
            runtime.shutdown().map_err(application_error)?;
            Err("forced Hub startup failure unexpectedly succeeded".to_owned())
        }
    };

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
}

#[test]
fn failed_startup_rollback_quarantines_and_later_reaps_inference_worker() -> TestResult {
    assert_eq!(super::startup_cleanup_quarantine_state(), (0, 0));
    let database_path = unique_database_path();
    let mut configuration = ApplicationRuntimeConfiguration::desktop(&database_path);
    default_test_configuration(&mut configuration);
    let primary = ApplicationError::Failure(ApplicationFailure::new(
        ApplicationFailureKind::Hub,
        "forced Hub startup failure with rollback timeout",
    ));
    let rollback_failure = ApplicationError::ShutdownTimeout(ApplicationWorker::Inference);

    let start_result = ApplicationRuntime::start_transaction_with_rollback(
        configuration,
        |_| Err(primary.clone()),
        |_local, _timing| {
            Err(ApplicationError::ShutdownTimeout(
                ApplicationWorker::Inference,
            ))
        },
    );
    let test_result = match start_result {
        Err(failure) => {
            assert_eq!(failure.primary, primary);
            assert_eq!(failure.inference_rollback, Some(Err(rollback_failure)));
            assert_eq!(super::startup_cleanup_quarantine_state(), (1, 1));

            let reap_result = super::reap_startup_cleanup_quarantine()
                .ok_or_else(|| "startup cleanup quarantine was unexpectedly empty".to_owned())?;
            reap_result.map_err(application_error)?;
            assert_eq!(super::startup_cleanup_quarantine_state(), (0, 0));
            Ok(())
        }
        Ok(mut runtime) => {
            runtime.shutdown().map_err(application_error)?;
            Err("forced Hub startup rollback failure unexpectedly succeeded".to_owned())
        }
    };

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
}

#[test]
fn shutdown_retries_retained_worker_joins_after_timeout() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        runtime.shutdown_control.forced_runtime_join_timeouts = 1;
        runtime.shutdown_control.forced_hub_join_timeouts = 1;

        assert_eq!(
            runtime.shutdown(),
            Err(ApplicationError::ShutdownTimeout(
                ApplicationWorker::Inference
            ))
        );
        assert_eq!(
            runtime.shutdown_control.status,
            ShutdownStatus::RetryableFailure
        );
        assert!(runtime.local.thread_is_present());
        assert!(runtime.hub_thread.is_some());
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::ShuttingDown
        );

        runtime.shutdown().map_err(application_error)?;
        assert_eq!(runtime.shutdown_control.status, ShutdownStatus::Stopped);
        assert!(!runtime.local.thread_is_present());
        assert!(runtime.hub_thread.is_none());

        runtime.shutdown().map_err(application_error)?;
        Ok(())
    })
}

#[test]
fn terminal_cleanup_failure_remains_sticky_after_worker_join() -> TestResult {
    let database_path = unique_database_path();
    let test_result = (|| {
        let mut configuration = ApplicationRuntimeConfiguration::desktop(&database_path);
        default_test_configuration(&mut configuration);
        let mut runtime = ApplicationRuntime::start(configuration).map_err(application_error)?;
        runtime.shutdown_control.forced_runtime_shutdown_failure = Some(terminal_cleanup_failure());
        runtime.shutdown_control.forced_runtime_join_timeouts = 1;

        let first_error = match runtime.shutdown() {
            Ok(()) => return Err("terminal cleanup failure was reported as success".to_owned()),
            Err(error) => error,
        };
        assert!(matches!(
            &first_error,
            ApplicationError::Failure(ApplicationFailure {
                kind: ApplicationFailureKind::Inference,
                message,
            }) if message.contains("CleanupRetryExhausted")
        ));
        assert_eq!(
            runtime.shutdown_control.status,
            ShutdownStatus::TerminalFailure
        );
        assert!(runtime.local.thread_is_present());

        assert_eq!(runtime.shutdown(), Err(first_error.clone()));
        assert_eq!(
            runtime.shutdown_control.status,
            ShutdownStatus::TerminalFailure
        );
        assert!(!runtime.local.thread_is_present());
        assert!(runtime.hub_thread.is_none());
        assert_eq!(runtime.shutdown(), Err(first_error));
        Ok(())
    })();

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
}

#[test]
fn explicit_application_shutdown_disconnects_and_joins_worker() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        runtime.shutdown().map_err(application_error)?;
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::ShuttingDown
        );
        assert!(!runtime.state().hub_available());
        assert!(!runtime.state().inference_available());
        assert!(runtime.hub_thread.is_none());
        assert!(!runtime.local.thread_is_present());
        assert_eq!(runtime.shutdown_control.status, ShutdownStatus::Stopped);
        runtime.shutdown().map_err(application_error)?;
        assert_eq!(runtime.shutdown_control.status, ShutdownStatus::Stopped);
        Ok(())
    })
}

#[derive(Clone, Copy)]
struct DirectCompletionExpectation {
    prompt: &'static str,
    text: &'static str,
    prompt_tokens: u64,
}

fn run_e1_direct_completion_scenario(
    runtime: &mut ApplicationRuntime,
    loaded: &LoadedModel,
    expected: DirectCompletionExpectation,
) -> TestResult {
    let request_id = runtime
        .start_generation(expected.prompt, deterministic_settings(3))
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
    assert_eq!(result.text, expected.text);
    assert_eq!(
        result.terminal.outcome,
        GenerationTerminalOutcome::Finished(FinishReason::TokenLimit)
    );
    assert_eq!(result.terminal.usage.prompt_tokens, expected.prompt_tokens);
    assert_eq!(result.terminal.usage.generated_tokens, 3);
    assert!(result.states.contains(&ApplicationOutputState::Released(
        GenerationTerminalKind::Finished(FinishReason::TokenLimit)
    )));
    assert!(runtime.state().active_generation().is_none());
    assert_eq!(runtime.state().last_generation(), Some(&result.terminal));

    runtime.unload_model().map_err(application_error)?;
    let event = wait_for_event(runtime, |event| {
        matches!(
            event,
            ApplicationEvent::ModelUnloaded { handle, .. } if *handle == loaded.handle()
        )
    })?;
    assert!(matches!(
        event,
        ApplicationEvent::ModelUnloaded {
            cancelled_requests: 0,
            ..
        }
    ));
    assert!(runtime.state().loaded().is_none());
    Ok(())
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
    let result = with_runtime_at(&database_path, configure, test);
    let cleanup_result = remove_database(&database_path);
    result.and(cleanup_result)
}

fn with_runtime_at<C, F>(database_path: &Path, configure: C, test: F) -> TestResult
where
    C: FnOnce(&mut ApplicationRuntimeConfiguration),
    F: FnOnce(&mut ApplicationRuntime) -> TestResult,
{
    let mut configuration = ApplicationRuntimeConfiguration::desktop(database_path);
    configure(&mut configuration);
    match ApplicationRuntime::start(configuration) {
        Ok(mut runtime) => {
            let test_result = test(&mut runtime);
            let shutdown_result = runtime.shutdown().map_err(application_error);
            test_result.and(shutdown_result)
        }
        Err(error) => Err(application_error(error)),
    }
}

fn assert_startup_deadline_duration_rejected(duration: Duration) -> TestResult {
    assert_startup_deadline_rejected(
        ApplicationConfigurationField::RuntimeShutdownTimeout,
        |configuration| configuration.timing.runtime_shutdown_timeout = duration,
    )?;
    assert_startup_deadline_rejected(
        ApplicationConfigurationField::RuntimeJoinTimeout,
        |configuration| configuration.timing.runtime_join_timeout = duration,
    )?;
    assert_startup_deadline_rejected(
        ApplicationConfigurationField::HubShutdownTimeout,
        |configuration| configuration.timing.hub_shutdown_timeout = duration,
    )
}

fn assert_startup_deadline_rejected<F>(
    field: ApplicationConfigurationField,
    configure: F,
) -> TestResult
where
    F: FnOnce(&mut ApplicationRuntimeConfiguration),
{
    let database_path = unique_database_path();
    let mut configuration = ApplicationRuntimeConfiguration::desktop(&database_path);
    default_test_configuration(&mut configuration);
    configure(&mut configuration);
    let hub_started = Cell::new(false);

    let start_result = ApplicationRuntime::start_transaction(configuration, |_| {
        hub_started.set(true);
        Err(ApplicationError::HubDisconnected)
    });
    let test_result = match start_result {
        Err(failure) => {
            assert_eq!(
                failure.primary,
                ApplicationError::InvalidConfiguration(field)
            );
            assert!(failure.inference_rollback.is_none());
            assert!(!hub_started.get());
            Ok(())
        }
        Ok(mut runtime) => {
            runtime.shutdown().map_err(application_error)?;
            Err(format!(
                "overflowing startup deadline was accepted for {field:?}"
            ))
        }
    };

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
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

fn resolve_fixture_with(
    runtime: &mut ApplicationRuntime,
    repository: &str,
    commit: &str,
    tokenizer_filename: &str,
) -> TestResult<(ModelSelection, ResolvedModel)> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candle = manifest.join("../inference-runtime/tests/fixtures/candle-llama");
    let artifacts = ResolvedSafetensorsLlamaArtifacts {
        repository: repository.to_owned(),
        revision: REVISION.to_owned(),
        commit: commit.to_owned(),
        declared_scalar_type: Some(ArtifactScalarType::F32),
        config_path: canonical(candle.join("config.json"))?,
        tokenizer_path: canonical(manifest.join("tests/fixtures").join(tokenizer_filename))?,
        weight_paths: vec![canonical(candle.join("model.safetensors"))?],
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
            assert_eq!(model.device(), ApplicationDevice::Cpu);
            assert_eq!(model.format(), ApplicationModelFormat::Safetensors);
            assert_eq!(model.scalar_type(), Some(ApplicationScalarType::F32));
            assert!(model.is_loadable());
            assert_eq!(model.identity().repository(), repository);
            assert_eq!(model.identity().commit(), commit);
            Ok((selection, model))
        }
        event => Err(format!("unexpected fixture-resolution event: {event:?}")),
    }
}

fn load_fixture_with(
    runtime: &mut ApplicationRuntime,
    repository: &str,
    commit: &str,
    tokenizer_filename: &str,
) -> TestResult<LoadedModel> {
    let (selection, _resolved) =
        resolve_fixture_with(runtime, repository, commit, tokenizer_filename)?;
    runtime.load_model(&selection).map_err(application_error)?;
    let event = wait_for_event(runtime, |event| {
        matches!(
            event,
            ApplicationEvent::ModelLoaded { .. }
                | ApplicationEvent::ModelLoadFailed { .. }
                | ApplicationEvent::ModelCompatibilityFailed { .. }
        )
    })?;
    match event {
        ApplicationEvent::ModelLoaded { model } => {
            assert_eq!(model.selection(), &selection);
            assert_eq!(model.engine(), ApplicationEngine::Candle);
            assert_eq!(model.source(), ApplicationSource::HuggingFaceHub);
            assert_eq!(model.device(), ApplicationDevice::Cpu);
            assert_eq!(model.format(), ApplicationModelFormat::Safetensors);
            assert_eq!(model.scalar_type(), ApplicationScalarType::F32);
            assert_eq!(model.identity().repository(), repository);
            assert_eq!(model.identity().commit(), commit);
            Ok(model)
        }
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
