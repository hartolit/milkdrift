use application_runtime::{
    ApplicationDevice, ApplicationEngine, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationModelFormat, ApplicationOutputState, ApplicationScalarType,
    ApplicationSource, ConversationProvenance, ConversationRecord, ConversationRecordId,
    ConversationRetention, ConversationRole, ConversationTokenEstimate, GenerationTerminalKind,
    GenerationTerminalOutcome, ResponseAttempt, ResponseAttemptId, ResponseAttemptState,
};
use slint::Model;

use super::callbacks::{
    CancellationMessageState, cancellation_pending_message, event_requires_conversation_snapshot,
};
use super::controls::{RuntimeAdmissions, control_state};
use super::devices::{DeviceChoice, DeviceSelectorModel};
use super::model::{
    ComposerMode, UNLOADED_MODEL_SUMMARY, artifact_target_label, composer_mode_from_evidence,
    device_label, loaded_model_facts_summary, map_model_selection, resolved_model_facts_summary,
    selected_model_summary,
};
use super::output::{
    FrameOutputDelta, GeneratedOutputUpdate, PresentationState, TerminalPresentation,
    format_conversation, format_terminal_outcome, output_state_message, released_terminal_message,
    replace_conversation_update,
};
use super::{MAXIMUM_EVENTS_PER_FRAME, UI_FRAME_MILLISECONDS};

#[test]
fn repository_and_revision_map_to_model_selection() {
    let selection = map_model_selection(" owner/model ", " main ");

    assert_eq!(selection.repository(), "owner/model");
    assert_eq!(selection.revision(), "main");

    let summary = selected_model_summary(&selection, ApplicationDevice::Cpu, true);
    assert!(summary.contains("Engine: Candle"));
    assert!(summary.contains("Repository: owner/model"));
    assert!(summary.contains("Revision: main"));
    assert!(summary.contains("Selected device: CPU"));
    assert!(!summary.contains("Configuration-declared scalar"));
    assert!(!summary.contains("Source scalar"));
    assert!(!summary.contains("Execution scalar"));
    assert!(!summary.contains("Execution device"));
}

#[test]
fn resolved_artifact_target_reports_only_artifact_facts() {
    let target = artifact_target_label(
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationModelFormat::Safetensors,
    );

    assert!(target.contains("Engine: Candle"));
    assert!(target.contains("Source: Hugging Face Hub"));
    assert!(target.contains("Format: Safetensors"));
    assert!(!target.contains("device"));
    assert!(!target.contains("CPU"));
    assert!(!target.contains("CUDA"));
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
fn checked_device_indices_map_to_the_rust_owned_e1_order() {
    let mut selector = DeviceSelectorModel::default();
    selector.synchronize_choices(&[
        DeviceChoice::new(ApplicationDevice::Cpu, "CPU", true),
        DeviceChoice::new(ApplicationDevice::Cuda { ordinal: 0 }, "CUDA 0", true),
        DeviceChoice::new(ApplicationDevice::Cuda { ordinal: 4 }, "CUDA 4", true),
    ]);

    assert_eq!(selector.device_at_checked_index(-1), None);
    assert_eq!(
        selector.device_at_checked_index(0),
        Some(ApplicationDevice::Cpu)
    );
    assert_eq!(
        selector.device_at_checked_index(1),
        Some(ApplicationDevice::Cuda { ordinal: 0 })
    );
    assert_eq!(
        selector.device_at_checked_index(2),
        Some(ApplicationDevice::Cuda { ordinal: 4 })
    );
    assert_eq!(selector.device_at_checked_index(3), None);
    assert_eq!(
        selector.selected_index(ApplicationDevice::Cuda { ordinal: 9 }),
        -1
    );
}

#[test]
fn unavailable_selected_cuda_row_is_retained_at_its_stable_index() {
    let mut selector = DeviceSelectorModel::default();
    selector.synchronize_choices(&[
        DeviceChoice::new(ApplicationDevice::Cpu, "CPU", true),
        DeviceChoice::new(
            ApplicationDevice::Cuda { ordinal: 2 },
            "CUDA 2 — Test GPU",
            true,
        ),
    ]);
    let labels = selector.slint_model();

    selector.synchronize_choices(&[
        DeviceChoice::new(ApplicationDevice::Cpu, "CPU", true),
        DeviceChoice::new(ApplicationDevice::Cuda { ordinal: 2 }, "CUDA 2", false),
    ]);

    assert_eq!(labels, selector.slint_model());
    assert_eq!(labels.row_count(), 2);
    assert_eq!(
        labels.row_data(1).map(|label| label.to_string()),
        Some("CUDA 2 (unavailable)".to_owned())
    );
    assert_eq!(
        selector.device_at_checked_index(1),
        Some(ApplicationDevice::Cuda { ordinal: 2 })
    );
    assert_eq!(
        selector.selected_index(ApplicationDevice::Cuda { ordinal: 2 }),
        1
    );
}

#[test]
fn resolved_summary_labels_configuration_declared_scalar() {
    let target = artifact_target_label(
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationModelFormat::Safetensors,
    );
    let resolved = resolved_model_facts_summary(
        &target,
        Some(ApplicationScalarType::Bf16),
        "Hub commit abc123 (owner/model)",
    );

    assert_eq!(
        resolved,
        "Engine: Candle • Source: Hugging Face Hub • Format: Safetensors • Configuration-declared scalar: BF16 • Identity: Hub commit abc123 (owner/model)"
    );
    assert!(!resolved.contains("Source scalar"));
    assert!(!resolved.contains("Execution scalar"));
    assert!(!resolved.contains("Execution device"));
}

#[test]
fn resolved_summary_omits_absent_configuration_declaration() {
    let target = artifact_target_label(
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationModelFormat::Safetensors,
    );
    let resolved = resolved_model_facts_summary(&target, None, "Hub commit abc123 (owner/model)");

    assert_eq!(
        resolved,
        "Engine: Candle • Source: Hugging Face Hub • Format: Safetensors • Identity: Hub commit abc123 (owner/model)"
    );
    assert!(!resolved.contains("scalar"));
    assert!(!resolved.contains("device"));
}

#[test]
fn configuration_declaration_and_loaded_execution_are_presented_independently() {
    let target = artifact_target_label(
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationModelFormat::Safetensors,
    );
    let resolved = resolved_model_facts_summary(
        &target,
        Some(ApplicationScalarType::Bf16),
        "Hub commit abc123 (owner/model)",
    );
    let loaded = loaded_model_facts_summary(
        &target,
        ApplicationScalarType::F32,
        ApplicationDevice::Cpu,
        "Hub commit abc123 (owner/model)",
    );

    assert!(resolved.contains("Configuration-declared scalar: BF16"));
    assert!(!resolved.contains("Execution scalar"));
    assert_eq!(
        loaded,
        "Engine: Candle • Source: Hugging Face Hub • Format: Safetensors • Execution scalar: F32 • Execution device: CPU • Identity: Hub commit abc123 (owner/model)"
    );
    assert!(!loaded.contains("Configuration-declared scalar"));
    assert!(!loaded.contains("Source scalar"));
}

#[test]
fn loaded_summary_reports_only_bf16_cuda_execution_facts() {
    let target = artifact_target_label(
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationModelFormat::Safetensors,
    );
    let loaded = loaded_model_facts_summary(
        &target,
        ApplicationScalarType::Bf16,
        ApplicationDevice::Cuda { ordinal: 2 },
        "Hub commit abc123 (owner/model)",
    );

    assert!(loaded.contains("Execution scalar: BF16"));
    assert!(loaded.contains("Execution device: CUDA 2"));
    assert!(!loaded.contains("Configuration-declared scalar"));
    assert!(!loaded.contains("Source scalar"));
}

#[test]
fn selected_and_loaded_execution_device_formatting_remain_distinct() {
    let selection = map_model_selection("owner/model", "main");
    let selected =
        selected_model_summary(&selection, ApplicationDevice::Cuda { ordinal: 3 }, false);
    let target = artifact_target_label(
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationModelFormat::Safetensors,
    );
    let loaded = loaded_model_facts_summary(
        &target,
        ApplicationScalarType::F32,
        ApplicationDevice::Cpu,
        "Hub commit abc123 (owner/model)",
    );

    assert!(selected.contains("Selected device: CUDA 3"));
    assert!(selected.contains("Availability: unavailable"));
    assert!(!selected.contains("Execution device"));
    assert!(loaded.contains("Execution device: CPU"));
    assert!(!loaded.contains("Selected device"));
    assert!(!loaded.contains("CUDA 3"));
}

#[test]
fn unload_display_clears_loaded_execution_facts_without_changing_selected_identity() {
    let selection = map_model_selection("owner/model", "main");
    let selected_before =
        selected_model_summary(&selection, ApplicationDevice::Cuda { ordinal: 3 }, false);
    let target = artifact_target_label(
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationModelFormat::Safetensors,
    );
    let loaded_before = loaded_model_facts_summary(
        &target,
        ApplicationScalarType::F32,
        ApplicationDevice::Cpu,
        "Hub commit abc123 (owner/model)",
    );
    let selected_after =
        selected_model_summary(&selection, ApplicationDevice::Cuda { ordinal: 3 }, false);

    assert!(loaded_before.contains("Execution scalar: F32"));
    assert!(loaded_before.contains("Execution device: CPU"));
    assert_eq!(UNLOADED_MODEL_SUMMARY, "Not loaded.");
    assert!(!UNLOADED_MODEL_SUMMARY.contains("scalar"));
    assert!(!UNLOADED_MODEL_SUMMARY.contains("device"));
    assert_eq!(selected_after, selected_before);
    assert!(selected_after.contains("Selected device: CUDA 3"));
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
fn changing_visible_selection_fields_never_reuses_the_stale_selection() {
    let selection = map_model_selection("a/model", "main");
    let changed_repository = map_model_selection("b/model", "main");
    let changed_revision = map_model_selection("a/model", "v2");

    assert_ne!(selection, changed_repository);
    assert_ne!(selection, changed_revision);
}

#[test]
fn composer_mode_requires_a_loaded_model_and_verified_chat_evidence() {
    assert_eq!(
        composer_mode_from_evidence(false, false),
        ComposerMode::Unavailable
    );
    assert_eq!(composer_mode_from_evidence(true, true), ComposerMode::Chat);
    assert_eq!(
        composer_mode_from_evidence(true, false),
        ComposerMode::DirectCompletion
    );
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
