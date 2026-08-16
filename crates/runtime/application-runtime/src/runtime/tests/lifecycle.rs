use std::time::Duration;

use domain_contracts::{CancellationReason, FinishReason, TokenId};
use inference_runtime::{RuntimeCommand, RuntimeEvent};

use super::support::*;
use crate::shutdown::ShutdownStatus;
use crate::{
    ApplicationActivity, ApplicationError, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationOutputRecordKind, ApplicationOutputState,
    ApplicationRuntime, ApplicationRuntimeConfiguration, ApplicationWorker, ConversationRole,
    GenerationPhase, GenerationSettingsField, GenerationTerminalKind, GenerationTerminalOutcome,
    LoadedModel, ModelUnloadBehavior, ResponseAttemptState,
};

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
fn terminal_generation_phases_reject_late_cancellation() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, _loaded| {
        let request_id = runtime
            .start_generation("prompt seed", deterministic_settings(3))
            .map_err(application_error)?;
        for phase in [
            GenerationPhase::Finishing,
            GenerationPhase::CleanupPending,
            GenerationPhase::CleanupExhausted,
        ] {
            runtime
                .state
                .transition_generation(request_id, phase)
                .map_err(|error| format!("generation phase transition failed: {error:?}"))?;
            assert!(!runtime.state().can_cancel_generation());
            assert_eq!(
                runtime.cancel_generation(request_id),
                Err(ApplicationError::GenerationNotCancellable { request_id, phase })
            );
        }
        Ok(())
    })
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
fn generation_submission_is_transactional_and_commits_correlated_state() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, _loaded| {
        runtime.forced_inference_busy_submissions = 1;
        assert_eq!(
            runtime.start_generation("prompt seed", deterministic_settings(1)),
            Err(ApplicationError::RuntimeBusy)
        );
        assert!(runtime.state().active_generation().is_none());
        assert!(runtime.generation.session_correlation().is_none());

        runtime.forced_unsent_command_disconnects = 1;
        assert_eq!(
            runtime.start_generation("prompt seed", deterministic_settings(1)),
            Err(ApplicationError::RuntimeDisconnected)
        );
        assert!(runtime.state().active_generation().is_none());
        assert!(runtime.generation.session_correlation().is_none());

        let request_id = runtime
            .start_generation("prompt seed", deterministic_settings(1))
            .map_err(application_error)?;
        let (session_request, admission_ticket) = runtime
            .generation
            .session_correlation()
            .ok_or_else(|| "successful submission omitted its generation session".to_owned())?;
        assert_eq!(session_request, request_id);
        assert_eq!(admission_ticket.get(), request_id.get());
        assert_eq!(
            runtime
                .state()
                .active_generation()
                .map(|summary| summary.request_id),
            Some(request_id)
        );
        let _result = collect_generation(runtime, request_id)?;
        Ok(())
    })
}

#[test]
fn active_generation_precondition_precedes_additional_prompt_encoding() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        let request_id = runtime
            .start_generation("prompt seed", deterministic_settings(1))
            .map_err(application_error)?;
        let oversized_prompt = "word ".repeat(
            usize::try_from(loaded.maximum_context_tokens())
                .unwrap_or(usize::MAX)
                .saturating_add(1),
        );
        assert_eq!(
            runtime.start_generation(oversized_prompt.as_str(), deterministic_settings(1)),
            Err(ApplicationError::GenerationAlreadyActive(request_id))
        );
        let _result = collect_generation(runtime, request_id)?;
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
        assert_eq!(
            runtime.generation.session_correlation(),
            Some((
                request_id,
                inference_runtime::CommandTicket::new(request_id.get())
            ))
        );
        assert!(runtime.conversation.has_active_response());
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
fn full_chat_submission_does_not_publish_or_supersede_a_provisional_response() -> TestResult {
    with_loaded_chat_runtime(default_test_configuration, |runtime, _loaded| {
        let first_request = runtime
            .submit_user_message("hello", deterministic_settings(1))
            .map_err(application_error)?;
        wait_for_generation_started(runtime, first_request)?;
        let _first = collect_generation(runtime, first_request)?;
        let records_before = runtime.conversation().to_vec();
        let diagnostics_before = runtime.context_diagnostics().cloned();

        runtime.forced_inference_busy_submissions = 1;
        assert_eq!(
            runtime.regenerate_last_response(deterministic_settings(1)),
            Err(ApplicationError::RuntimeBusy)
        );
        assert_eq!(runtime.conversation(), records_before.as_slice());
        assert!(runtime.conversation().get(1).is_some_and(|record| {
            record
                .response_attempt
                .as_ref()
                .is_some_and(|attempt| !attempt.superseded)
        }));
        assert!(!runtime.conversation.has_active_response());
        assert!(runtime.state().active_generation().is_none());
        assert!(runtime.generation.session_correlation().is_none());
        assert_eq!(runtime.context_diagnostics(), diagnostics_before.as_ref());
        Ok(())
    })
}

#[test]
fn unsent_disconnected_chat_submission_retains_user_without_publishing_response() -> TestResult {
    with_loaded_chat_runtime(default_test_configuration, |runtime, _loaded| {
        let first_request = runtime
            .submit_user_message("hello", deterministic_settings(1))
            .map_err(application_error)?;
        wait_for_generation_started(runtime, first_request)?;
        let _first = collect_generation(runtime, first_request)?;
        let records_before = runtime.conversation().len();
        let diagnostics_before = runtime.context_diagnostics().cloned();

        runtime.forced_unsent_command_disconnects = 1;
        assert_eq!(
            runtime.submit_user_message("committed question", deterministic_settings(1)),
            Err(ApplicationError::RuntimeDisconnected)
        );
        assert_eq!(runtime.conversation().len(), records_before + 1);
        assert_eq!(
            runtime.conversation().last().map(|record| record.role),
            Some(ConversationRole::User)
        );
        assert!(runtime.conversation().get(1).is_some_and(|record| {
            record
                .response_attempt
                .as_ref()
                .is_some_and(|attempt| !attempt.superseded)
        }));
        assert!(!runtime.conversation.has_active_response());
        assert!(runtime.state().active_generation().is_none());
        assert!(runtime.generation.session_correlation().is_none());
        assert_eq!(runtime.context_diagnostics(), diagnostics_before.as_ref());
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
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        let mut invalid = deterministic_settings(1);
        invalid.maximum_new_tokens = 0;
        assert_eq!(
            runtime.start_generation("", invalid),
            Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::MaximumNewTokens
            ))
        );
        assert_eq!(
            runtime.start_generation("", deterministic_settings(1)),
            Err(ApplicationError::EmptyPrompt)
        );
        let mut empty_with_unencodable_stop = deterministic_settings(1);
        empty_with_unencodable_stop.stop_sequences.push(
            "word ".repeat(
                usize::try_from(loaded.maximum_context_tokens())
                    .unwrap_or(usize::MAX)
                    .saturating_add(1),
            ),
        );
        assert_eq!(
            runtime.start_generation("", empty_with_unencodable_stop),
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
        assert_cancelled_by_user(&result);
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
fn application_retains_inference_worker_disconnection_as_terminal() -> TestResult {
    let database_path = unique_database_path();
    let test_result = (|| {
        let mut configuration = ApplicationRuntimeConfiguration::new(&database_path);
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
        let mut configuration = ApplicationRuntimeConfiguration::new(&database_path);
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
                ..
            }) if message.contains("TerminalCleanupRetention")
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
    assert_released_token_limit(&result);
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
