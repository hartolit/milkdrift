use application_runtime::{
    ApplicationConservativeFootprint, ApplicationDevice, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationGenerationMode, ApplicationModelCleanupDisposition,
    ApplicationOutputState, ApplicationRetainedModelResource, ApplicationRetainedOwnership,
    ApplicationScalarType, ChatCompatibility, ConversationProvenance, ConversationRecord,
    ConversationRecordId, ConversationRetention, ConversationRole, ConversationTokenEstimate,
    GenerationTerminalKind, GenerationTerminalOutcome, MemoryFootprint, ResponseAttempt,
    ResponseAttemptId, ResponseAttemptState,
};
use slint::Model;

use super::callbacks::{
    CancellationMessageState, cancellation_pending_message, event_requires_conversation_snapshot,
};
use super::controls::{RuntimeAdmissions, control_state};
use super::devices::{DeviceChoice, DeviceSelectorModel};
use super::model::{
    ComposerMode, composer_mode_from_generation_mode, device_label, loaded_model_facts_summary,
    map_model_selection, model_residency_facts_summary, resolved_model_facts_summary,
    retained_model_facts_summary, selected_model_summary,
};
use super::output::{
    FrameOutputDelta, GeneratedOutputUpdate, PresentationState, TerminalPresentation,
    format_conversation, format_terminal_outcome, output_state_message, released_terminal_message,
    replace_conversation_update,
};
use super::{MAXIMUM_EVENTS_PER_FRAME, UI_FRAME_MILLISECONDS};

#[test]
fn repository_revision_and_selected_device_are_projected_without_backend_details() {
    let selection = map_model_selection(" owner/model ", " main ");

    assert_eq!(selection.repository(), "owner/model");
    assert_eq!(selection.revision(), "main");

    let summary = selected_model_summary(&selection, ApplicationDevice::Cpu, true);
    assert!(summary.contains("owner/model"));
    assert!(summary.contains("main"));
    assert!(summary.contains("CPU"));
    assert!(summary.contains("available"));
    for implementation_detail in ["Engine", "Source", "Format", "Identity", "commit"] {
        assert!(!summary.contains(implementation_detail));
    }
}

#[test]
fn resolved_summary_projects_public_declaration_and_compatibility_facts_only() {
    let summary = resolved_model_facts_summary(
        Some(ApplicationScalarType::F32),
        ChatCompatibility::Supported,
    );

    assert!(summary.contains("Resolved"));
    assert!(summary.contains("Recognized"));
    assert!(summary.contains("F32"));
    assert!(summary.contains("supported"));
    for implementation_detail in [
        "Engine",
        "Source",
        "Format",
        "Identity",
        "commit",
        "vocabulary",
    ] {
        assert!(!summary.contains(implementation_detail));
    }
    assert!(!summary.contains("Execution device"));
    assert!(!summary.contains("Execution scalar"));
}

#[test]
fn device_labels_are_owned_and_stable_for_cpu_and_cuda_ordinals() {
    assert_eq!(device_label(ApplicationDevice::Cpu), "CPU");
    assert_eq!(
        device_label(ApplicationDevice::Cuda { ordinal: 7 }),
        "CUDA 7"
    );
}

#[test]
fn checked_device_rows_round_trip_rust_owned_application_identities() {
    let cpu = ApplicationDevice::Cpu;
    let cuda_zero = ApplicationDevice::Cuda { ordinal: 0 };
    let cuda_four = ApplicationDevice::Cuda { ordinal: 4 };
    let devices = [cpu, cuda_zero, cuda_four];
    let mut selector = DeviceSelectorModel::default();
    selector.synchronize_choices(&[
        DeviceChoice::new(cpu, "CPU", true),
        DeviceChoice::new(cuda_zero, "CUDA 0", true),
        DeviceChoice::new(cuda_four, "CUDA 4", true),
    ]);

    for device in devices {
        let selected_row = selector.selected_index(device);
        assert!(selected_row >= 0);
        assert_eq!(selector.device_at_checked_index(selected_row), Some(device));
    }
    assert_eq!(selector.device_at_checked_index(-1), None);
    assert_eq!(selector.device_at_checked_index(i32::MAX), None);
    assert_eq!(
        selector.selected_index(ApplicationDevice::Cuda { ordinal: 9 }),
        -1
    );
}

#[test]
fn unavailable_selected_cuda_identity_remains_visible_and_selectable() {
    let cuda = ApplicationDevice::Cuda { ordinal: 2 };
    let mut selector = DeviceSelectorModel::default();
    selector.synchronize_choices(&[
        DeviceChoice::new(ApplicationDevice::Cpu, "CPU", true),
        DeviceChoice::new(cuda, "CUDA 2 — Test GPU", true),
    ]);
    let labels = selector.slint_model();

    selector.synchronize_choices(&[
        DeviceChoice::new(ApplicationDevice::Cpu, "CPU", true),
        DeviceChoice::new(cuda, "CUDA 2", false),
    ]);

    let selected_row = selector.selected_index(cuda);
    let selected_label = usize::try_from(selected_row)
        .ok()
        .and_then(|index| labels.row_data(index))
        .map(|label| label.to_string());

    assert_eq!(labels, selector.slint_model());
    assert!(
        selected_label
            .as_deref()
            .is_some_and(|label| label.contains("CUDA 2"))
    );
    assert!(
        selected_label
            .as_deref()
            .is_some_and(|label| label.contains("unavailable"))
    );
    assert_eq!(selector.device_at_checked_index(selected_row), Some(cuda));
}

#[test]
fn resolved_summary_labels_a_recognized_configuration_declaration() {
    let resolved = resolved_model_facts_summary(
        Some(ApplicationScalarType::Bf16),
        ChatCompatibility::Supported,
    );

    assert!(resolved.contains("Resolved"));
    assert!(resolved.contains("Recognized"));
    assert!(resolved.contains("BF16"));
    assert!(resolved.contains("supported"));
    assert!(!resolved.contains("Execution scalar"));
    assert!(!resolved.contains("Execution device"));
}

#[test]
fn resolved_summary_omits_an_absent_configuration_declaration() {
    let resolved = resolved_model_facts_summary(None, ChatCompatibility::Unsupported);

    assert!(resolved.contains("Resolved"));
    assert!(resolved.contains("unsupported"));
    assert!(!resolved.contains("Recognized"));
    assert!(!resolved.contains("scalar"));
    assert!(!resolved.contains("device"));
}

#[test]
fn recognized_declaration_and_loaded_execution_are_presented_independently() {
    let resolved = resolved_model_facts_summary(
        Some(ApplicationScalarType::Bf16),
        ChatCompatibility::Supported,
    );
    let loaded = loaded_model_facts_summary(ApplicationScalarType::F32, ApplicationDevice::Cpu);

    assert!(resolved.contains("Recognized"));
    assert!(resolved.contains("BF16"));
    assert!(!resolved.contains("Execution scalar"));
    assert!(!resolved.contains("Execution device"));
    assert!(loaded.contains("Execution scalar: F32"));
    assert!(loaded.contains("Execution device: CPU"));
    assert!(!loaded.contains("declaration"));
    assert!(!loaded.contains("BF16"));
}

#[test]
fn loaded_summary_reports_only_actual_bf16_cuda_execution_facts() {
    let loaded = loaded_model_facts_summary(
        ApplicationScalarType::Bf16,
        ApplicationDevice::Cuda { ordinal: 2 },
    );

    assert!(loaded.contains("Execution scalar: BF16"));
    assert!(loaded.contains("Execution device: CUDA 2"));
    for non_execution_fact in [
        "declaration",
        "Engine",
        "Source",
        "Format",
        "Identity",
        "commit",
    ] {
        assert!(!loaded.contains(non_execution_fact));
    }
}

#[test]
fn selected_and_loaded_execution_devices_remain_distinct_projected_facts() {
    let selection = map_model_selection("owner/model", "main");
    let selected =
        selected_model_summary(&selection, ApplicationDevice::Cuda { ordinal: 3 }, false);
    let loaded = loaded_model_facts_summary(ApplicationScalarType::F32, ApplicationDevice::Cpu);

    assert!(selected.contains("Selected device: CUDA 3"));
    assert!(selected.contains("unavailable"));
    assert!(!selected.contains("Execution device"));
    assert!(loaded.contains("Execution device: CPU"));
    assert!(!loaded.contains("Selected device"));
    assert!(!loaded.contains("CUDA 3"));
}

#[test]
fn released_residency_clears_execution_facts_without_changing_selected_device() {
    let selection = map_model_selection("owner/model", "main");
    let selected_before =
        selected_model_summary(&selection, ApplicationDevice::Cuda { ordinal: 3 }, false);
    let loaded_before =
        loaded_model_facts_summary(ApplicationScalarType::F32, ApplicationDevice::Cpu);
    let released = model_residency_facts_summary(None, None);
    let selected_after =
        selected_model_summary(&selection, ApplicationDevice::Cuda { ordinal: 3 }, false);

    assert!(loaded_before.contains("Execution scalar: F32"));
    assert!(loaded_before.contains("Execution device: CPU"));
    assert!(released.contains("No loaded"));
    assert!(released.contains("retained"));
    assert!(!released.contains("Execution scalar"));
    assert!(!released.contains("Execution device"));
    assert_eq!(selected_after, selected_before);
    assert!(selected_after.contains("Selected device: CUDA 3"));
}

#[test]
fn exact_retained_ownership_replaces_the_unloaded_placeholder() {
    let retained = retained_model_facts_summary(
        ApplicationRetainedModelResource::UnconfirmedModel,
        ApplicationRetainedOwnership::Exact(MemoryFootprint::default()),
        ApplicationModelCleanupDisposition::Pending,
    );
    let residency = model_residency_facts_summary(None, Some(&retained));

    assert!(residency.contains("Retained"));
    assert!(residency.contains("unconfirmed-model"));
    assert!(residency.contains("exact"));
    assert!(residency.contains("pending"));
    assert!(!residency.contains("No loaded"));
    assert!(!residency.contains("Not loaded"));
}

#[test]
fn unverified_retained_ownership_projects_its_retryable_disposition() {
    let retained = retained_model_facts_summary(
        ApplicationRetainedModelResource::UnconfirmedLoad,
        ApplicationRetainedOwnership::Unverified {
            accepted_loading_peak: MemoryFootprint::default(),
            reported_footprint: MemoryFootprint::default(),
            conservative_footprint: ApplicationConservativeFootprint::Overflow,
        },
        ApplicationModelCleanupDisposition::LowerRetryable {
            attempts: 2,
            maximum_attempts: 3,
        },
    );

    assert!(retained.contains("unverified"));
    assert!(retained.contains("retryable"));
    assert!(retained.contains('2'));
    assert!(retained.contains('3'));
}

#[test]
fn unknown_retained_ownership_projects_unconfirmed_release() {
    let retained = retained_model_facts_summary(
        ApplicationRetainedModelResource::UnconfirmedModel,
        ApplicationRetainedOwnership::Unknown,
        ApplicationModelCleanupDisposition::WorkerDisconnected,
    );

    assert!(retained.contains("unknown"));
    assert!(retained.contains("worker disconnected"));
    assert!(retained.contains("without confirmed release"));
    assert!(!retained.contains("released"));
}

#[test]
fn control_state_uses_the_e1_device_selection_flag_unchanged() {
    for expected in [false, true] {
        let controls = control_state(
            RuntimeAdmissions {
                can_select_device: expected,
                ..RuntimeAdmissions::default()
            },
            ComposerMode::Unavailable,
            "",
        );

        assert_eq!(controls.can_select_device, expected);
    }
}

#[test]
fn cleanup_retry_control_is_a_direct_e1_admission_projection() {
    for expected in [false, true] {
        let controls = control_state(
            RuntimeAdmissions {
                can_retry_model_cleanup: expected,
                ..RuntimeAdmissions::default()
            },
            ComposerMode::Unavailable,
            "",
        );

        assert_eq!(controls.can_retry_model_cleanup, expected);
    }
}

#[test]
fn changing_visible_selection_fields_never_reuses_the_stale_selection() {
    let selection = map_model_selection("a/model", "main");
    let changed_repository = map_model_selection("b/model", "main");
    let changed_revision = map_model_selection("a/model", "v2");

    assert_ne!(selection, changed_repository);
    assert_ne!(selection, changed_revision);
}

#[test]
fn composer_mode_is_a_direct_projection_of_application_generation_mode() {
    for (generation_mode, composer_mode) in [
        (
            ApplicationGenerationMode::Unavailable,
            ComposerMode::Unavailable,
        ),
        (
            ApplicationGenerationMode::DirectCompletion,
            ComposerMode::DirectCompletion,
        ),
        (ApplicationGenerationMode::Chat, ComposerMode::Chat),
    ] {
        assert_eq!(
            composer_mode_from_generation_mode(generation_mode),
            composer_mode
        );
    }
}

#[test]
fn unavailable_composer_guidance_distinguishes_retained_ownership() {
    assert!(ComposerMode::Unavailable.guidance(false).contains("Load"));
    assert!(
        ComposerMode::Unavailable
            .guidance(true)
            .contains("retained")
    );
    assert!(ComposerMode::Unavailable.guidance(true).contains("release"));
}

#[test]
fn direct_completion_uses_generation_admission_and_disables_regeneration() {
    let controls = control_state(
        RuntimeAdmissions {
            can_start_generation: true,
            can_regenerate: true,
            can_clear: true,
            can_unload: true,
            ..RuntimeAdmissions::default()
        },
        ComposerMode::DirectCompletion,
        "prompt",
    );

    assert!(controls.can_edit_message);
    assert!(controls.can_submit_message);
    assert!(!controls.can_regenerate);
    assert!(controls.can_clear);
    assert!(controls.can_unload);
}

#[test]
fn chat_controls_use_only_chat_admission() {
    let controls = control_state(
        RuntimeAdmissions {
            can_submit_chat: true,
            can_start_generation: true,
            can_regenerate: true,
            can_clear: true,
            ..RuntimeAdmissions::default()
        },
        ComposerMode::Chat,
        "message",
    );

    assert!(controls.can_edit_message);
    assert!(controls.can_submit_message);
    assert!(controls.can_regenerate);
}

#[test]
fn running_generation_exposes_cancellation() {
    let controls = control_state(
        RuntimeAdmissions {
            can_cancel: true,
            can_unload: true,
            ..RuntimeAdmissions::default()
        },
        ComposerMode::DirectCompletion,
        "prompt",
    );

    assert!(!controls.can_submit_message);
    assert!(controls.can_cancel);
    assert!(controls.can_unload);
    assert!(!controls.can_edit_message);
}

#[test]
fn message_submission_requires_nonempty_visible_input() {
    let admissions = RuntimeAdmissions {
        can_submit_chat: true,
        ..RuntimeAdmissions::default()
    };
    let empty = control_state(admissions, ComposerMode::Chat, "  \n ");
    let populated = control_state(admissions, ComposerMode::Chat, "Hello");

    assert!(empty.can_edit_message);
    assert!(!empty.can_submit_message);
    assert!(populated.can_submit_message);
}

#[test]
fn cancellation_pending_is_explicit_before_and_after_backend_acceptance() {
    let submitted = cancellation_pending_message(7, CancellationMessageState::Submitted);
    let accepted = cancellation_pending_message(7, CancellationMessageState::Accepted);

    assert!(submitted.contains("generation 7"));
    assert!(submitted.contains("pending until a safe backend boundary"));
    assert!(accepted.contains("generation 7"));
    assert!(accepted.contains("accepted"));
    assert!(accepted.contains("remains pending"));
}

#[test]
fn frame_work_remains_explicitly_bounded() {
    assert_eq!(UI_FRAME_MILLISECONDS, 16);
    assert_eq!(MAXIMUM_EVENTS_PER_FRAME, 64);
}

#[test]
fn text_delta_contains_only_the_new_frame_fragment() {
    let mut presentation = PresentationState::default();
    presentation.begin_chat_request(7);
    let delta = FrameOutputDelta {
        text: "new frame text".to_owned(),
        terminal_text: None,
        invalid_text_record: false,
    };

    let update = presentation.apply_delta(delta);

    assert_eq!(
        update.output,
        Some(GeneratedOutputUpdate::Append("new frame text".to_owned()))
    );
    assert!(!update.terminal_changed);
    assert_eq!(presentation.displayed_request(), Some(7));
}

#[test]
fn replacing_transcript_is_one_batched_presentation_update() {
    assert_eq!(
        replace_conversation_update("User: hello".to_owned()),
        GeneratedOutputUpdate::Replace("User: hello".to_owned())
    );
}

#[test]
fn direct_completion_presents_one_prompt_and_completion_without_chat_roles() {
    let mut presentation = PresentationState::default();
    let replacement = presentation.begin_direct_request(11, "Explain Rust ownership.");

    assert_eq!(
        replacement,
        GeneratedOutputUpdate::Replace(
            "Prompt:\nExplain Rust ownership.\n\nCompletion:\n".to_owned()
        )
    );
    assert!(!presentation.allows_conversation_snapshot());

    let update = presentation.apply_delta(FrameOutputDelta {
        text: "Ownership tracks values.".to_owned(),
        terminal_text: Some("Generation finished.".to_owned()),
        invalid_text_record: false,
    });
    assert_eq!(
        update.output,
        Some(GeneratedOutputUpdate::Append(
            "Ownership tracks values.".to_owned()
        ))
    );
    assert!(!presentation.allows_conversation_snapshot());
}

#[test]
fn transcript_mode_changes_only_on_explicit_presentation_actions() {
    let mut presentation = PresentationState::default();
    presentation.begin_direct_request(3, "prompt");
    presentation.apply_delta(FrameOutputDelta {
        text: String::new(),
        terminal_text: Some("terminal".to_owned()),
        invalid_text_record: false,
    });
    assert!(!presentation.allows_conversation_snapshot());

    let cleared = presentation.clear(ComposerMode::DirectCompletion);
    assert_eq!(cleared, GeneratedOutputUpdate::Replace(String::new()));
    assert!(!presentation.allows_conversation_snapshot());
    assert_eq!(presentation.displayed_request(), None);

    presentation.begin_chat_request(4);
    assert!(presentation.allows_conversation_snapshot());
}

#[test]
fn transcript_preserves_failed_partial_attempt_provenance() {
    let record = ConversationRecord {
        id: ConversationRecordId::new(2),
        ordinal: 2,
        role: ConversationRole::Assistant,
        content: "partial".to_owned(),
        provenance: ConversationProvenance::Model,
        retention: ConversationRetention::Retained,
        token_estimate: ConversationTokenEstimate::Measured(1),
        response_attempt: Some(ResponseAttempt {
            id: ResponseAttemptId::new(1),
            responding_to: ConversationRecordId::new(1),
            state: ResponseAttemptState::Failed(ApplicationFailure::new(
                ApplicationFailureKind::Inference,
                "decode failed",
            )),
            superseded: false,
        }),
    };

    let transcript = format_conversation(&[record]);

    assert!(transcript.contains("Assistant: partial"));
    assert!(transcript.contains("failed: decode failed"));
}

#[test]
fn terminal_lifecycle_events_request_a_conversation_snapshot() {
    assert!(event_requires_conversation_snapshot(
        &ApplicationEvent::RuntimeDisconnected
    ));
    assert!(!event_requires_conversation_snapshot(
        &ApplicationEvent::HubDisconnected
    ));
}

#[test]
fn successful_terminal_release_is_presented_as_released() {
    let message =
        released_terminal_message(&TerminalPresentation::Finished("TokenLimit".to_owned()));

    assert!(message.contains("Generation finished: TokenLimit"));
    assert!(message.contains("resources were released"));
}

#[test]
fn generation_failure_preserves_the_diagnostic_and_release_state() {
    let outcome = GenerationTerminalOutcome::Failed(ApplicationFailure::new(
        ApplicationFailureKind::Inference,
        "decode failed",
    ));
    let terminal = format_terminal_outcome(&outcome);
    let released = output_state_message(ApplicationOutputState::Released(
        GenerationTerminalKind::Failed,
    ));

    assert!(terminal.contains("Generation failed: decode failed"));
    assert!(terminal.contains("resources were released"));
    assert!(
        released
            .as_deref()
            .is_some_and(|value| value.contains("Generation failed"))
    );
    assert!(
        released
            .as_deref()
            .is_some_and(|value| value.contains("resources were released"))
    );
}

#[test]
fn cleanup_states_are_not_presented_as_released() {
    let pending = output_state_message(ApplicationOutputState::CleanupPending);
    let exhausted = output_state_message(ApplicationOutputState::CleanupExhausted);

    assert!(
        pending
            .as_deref()
            .is_some_and(|value| value.contains("pending"))
    );
    assert!(
        pending
            .as_deref()
            .is_some_and(|value| !value.contains("released"))
    );
    assert!(
        exhausted
            .as_deref()
            .is_some_and(|value| value.contains("exhausted"))
    );
    assert!(
        exhausted
            .as_deref()
            .is_some_and(|value| value.contains("remain retained"))
    );
}
