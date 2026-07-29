//! Slint-specific presentation mapping over the reusable application runtime.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use application_runtime::{
    ApplicationActivity, ApplicationEvent, ApplicationFailure, ApplicationOutputBatch,
    ApplicationOutputRecordKind, ApplicationOutputState, ApplicationRuntime, ChatCompatibility,
    ConversationRecord, ConversationRole, GenerationSettings, GenerationTerminalKind,
    GenerationTerminalOutcome, ResolvedModel, ResponseAttemptState, ScalarType,
};
use slint::ComponentHandle;

use crate::AppWindow;

const UI_FRAME_MILLISECONDS: u64 = 16;
const MAXIMUM_EVENTS_PER_FRAME: usize = 64;

/// Owns presentation-only generation state and the Slint callback bindings.
pub struct Presenter {
    state: Rc<RefCell<PresentationState>>,
}

impl Presenter {
    /// Connects UI intents to the frontend-neutral application runtime.
    pub fn connect(window: &AppWindow, runtime: &Rc<RefCell<ApplicationRuntime>>) -> Self {
        let state = Rc::new(RefCell::new(PresentationState::default()));
        connect_resolve(window, Rc::clone(runtime));
        connect_load(window, Rc::clone(runtime));
        connect_unload(window, Rc::clone(runtime));
        connect_submit_message(window, Rc::clone(runtime), Rc::clone(&state));
        connect_regenerate(window, Rc::clone(runtime), Rc::clone(&state));
        connect_cancel(window, Rc::clone(runtime));
        connect_clear_conversation(window, Rc::clone(runtime));
        Self { state }
    }

    /// Starts bounded event and decoded-output polling on the UI frame cadence.
    pub fn start_frame_timer(
        &self,
        window: &AppWindow,
        runtime: Rc<RefCell<ApplicationRuntime>>,
    ) -> slint::Timer {
        let timer = slint::Timer::default();
        let weak = window.as_weak();
        let presentation = Rc::clone(&self.state);
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(UI_FRAME_MILLISECONDS),
            move || {
                let Some(window) = weak.upgrade() else {
                    return;
                };
                let output = {
                    let mut runtime_ref = runtime.borrow_mut();
                    for _ in 0..MAXIMUM_EVENTS_PER_FRAME {
                        let Some(event) = runtime_ref.poll_event() else {
                            break;
                        };
                        apply_event(&window, event);
                    }

                    let displayed_request = presentation.borrow().displayed_request;
                    runtime_ref.pull_output(|batch| collect_output_batch(&batch, displayed_request))
                };

                match output {
                    Ok(delta) => {
                        let mut presentation = presentation.borrow_mut();
                        let update = presentation.apply_delta(delta);
                        render_presentation_update(&window, &presentation, update);
                    }
                    Err(error) => {
                        window.set_status_text(
                            format!("Generated output pull failed: {error}").into(),
                        );
                    }
                }
                synchronize_controls(&window, &runtime.borrow());
            },
        );
        timer
    }
}

/// Synchronizes every control and usage value from authoritative application state.
pub fn synchronize_controls(window: &AppWindow, runtime: &ApplicationRuntime) {
    let state = runtime.state();
    let repository = window.get_repository().to_string();
    let revision = window.get_revision().to_string();
    let message = window.get_message_input().to_string();
    let controls = control_state(
        state.can_resolve(),
        state.can_load(&repository, &revision),
        state.can_start_generation(),
        runtime.can_regenerate_response(),
        state.can_cancel_generation(),
        state.can_unload(),
        &message,
    );

    window.set_busy(state.activity() != ApplicationActivity::Idle);
    window.set_can_resolve(controls.can_resolve);
    window.set_can_load(controls.can_load);
    window.set_can_edit_message(controls.can_edit_message);
    window.set_can_submit_message(controls.can_submit_message);
    window.set_can_regenerate(controls.can_regenerate);
    window.set_can_clear_conversation(state.active_generation().is_none());
    window.set_can_cancel(controls.can_cancel);
    window.set_can_unload(controls.can_unload);

    let usage = state
        .active_generation()
        .map(|summary| summary.usage)
        .or_else(|| state.last_generation().map(|terminal| terminal.usage));
    let (prompt_tokens, generated_tokens) = usage.map_or_else(
        || ("0".to_owned(), "0".to_owned()),
        |usage| {
            (
                usage.prompt_tokens.to_string(),
                usage.generated_tokens.to_string(),
            )
        },
    );
    window.set_prompt_token_count(prompt_tokens.into());
    window.set_generated_token_count(generated_tokens.into());

    if let Some(terminal) = state.last_generation() {
        window.set_terminal_text(format_terminal_outcome(&terminal.outcome).into());
    }
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "each field maps one independent Slint control-enablement property"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ControlState {
    can_resolve: bool,
    can_load: bool,
    can_edit_message: bool,
    can_submit_message: bool,
    can_regenerate: bool,
    can_cancel: bool,
    can_unload: bool,
}

#[expect(
    clippy::fn_params_excessive_bools,
    reason = "the pure mapping accepts the authoritative E1 admission flags used by the controls"
)]
fn control_state(
    can_resolve: bool,
    can_load: bool,
    can_start_generation: bool,
    can_regenerate: bool,
    can_cancel: bool,
    can_unload: bool,
    message: &str,
) -> ControlState {
    ControlState {
        can_resolve,
        can_load,
        can_edit_message: can_start_generation,
        can_submit_message: can_start_generation && !message.trim().is_empty(),
        can_regenerate,
        can_cancel,
        can_unload,
    }
}

#[derive(Default)]
struct PresentationState {
    displayed_request: Option<u64>,
    terminal_text: String,
}

impl PresentationState {
    fn begin_request(&mut self, request_id: u64) {
        self.displayed_request = Some(request_id);
        self.terminal_text.clear();
        self.terminal_text
            .push_str("Generation submitted; waiting for admission.");
    }

    fn apply_delta(&mut self, delta: FrameOutputDelta) -> PresentationUpdate {
        let terminal_changed = if let Some(terminal_text) = delta.terminal_text {
            self.terminal_text = terminal_text;
            true
        } else {
            false
        };
        PresentationUpdate {
            output: (!delta.text.is_empty()).then_some(GeneratedOutputUpdate::Append(delta.text)),
            terminal_changed,
            invalid_text_record: delta.invalid_text_record,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum GeneratedOutputUpdate {
    Append(String),
    Replace(String),
}

#[derive(Debug, PartialEq, Eq)]
struct PresentationUpdate {
    output: Option<GeneratedOutputUpdate>,
    terminal_changed: bool,
    invalid_text_record: bool,
}

const fn replace_conversation_update(transcript: String) -> GeneratedOutputUpdate {
    GeneratedOutputUpdate::Replace(transcript)
}

fn render_generated_output_update(window: &AppWindow, update: GeneratedOutputUpdate) {
    match update {
        GeneratedOutputUpdate::Append(text) => window.invoke_append_assistant_text(text.into()),
        GeneratedOutputUpdate::Replace(text) => {
            window.invoke_replace_conversation_transcript(text.into());
        }
    }
}

fn render_presentation_update(
    window: &AppWindow,
    presentation: &PresentationState,
    update: PresentationUpdate,
) {
    if let Some(output) = update.output {
        render_generated_output_update(window, output);
    }
    if update.terminal_changed {
        window.set_terminal_text(presentation.terminal_text.clone().into());
    }
    if update.invalid_text_record {
        window.set_status_text(
            "Generated output contained an invalid UTF-8 range; the affected fragment was skipped."
                .into(),
        );
    }
}

#[derive(Default)]
struct FrameOutputDelta {
    text: String,
    terminal_text: Option<String>,
    invalid_text_record: bool,
}

fn collect_output_batch(
    batch: &ApplicationOutputBatch<'_>,
    displayed_request: Option<u64>,
) -> FrameOutputDelta {
    let mut delta = FrameOutputDelta::default();
    for record in batch.records() {
        if displayed_request != Some(record.request_id.get()) {
            continue;
        }
        match record.kind {
            ApplicationOutputRecordKind::Text(_) => match batch.text_for(record) {
                Some(text) => delta.text.push_str(text),
                None => delta.invalid_text_record = true,
            },
            ApplicationOutputRecordKind::State(state) => {
                if let Some(message) = output_state_message(state) {
                    delta.terminal_text = Some(message);
                }
            }
        }
    }
    delta
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CancellationMessageState {
    Submitted,
    Accepted,
}

fn cancellation_pending_message(request_id: u64, state: CancellationMessageState) -> String {
    match state {
        CancellationMessageState::Submitted => format!(
            "Cancellation for generation {request_id} is pending until a safe backend boundary."
        ),
        CancellationMessageState::Accepted => format!(
            "Cancellation for generation {request_id} was accepted and remains pending until completion."
        ),
    }
}

fn output_state_message(state: ApplicationOutputState) -> Option<String> {
    match state {
        ApplicationOutputState::Yielded(_) => None,
        ApplicationOutputState::Terminal(kind) => Some(format!(
            "{} Backend cleanup is in progress.",
            terminal_kind_message(kind)
        )),
        ApplicationOutputState::CleanupPending => {
            Some("Generation ended; backend cleanup is pending and will be retried.".to_owned())
        }
        ApplicationOutputState::CleanupExhausted => Some(
            "Generation ended, but backend cleanup retries were exhausted; resources remain retained."
                .to_owned(),
        ),
        ApplicationOutputState::Released(kind) => {
            Some(released_terminal_message(&terminal_presentation(kind)))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TerminalPresentation {
    Finished(String),
    Failed,
}

fn terminal_presentation(kind: GenerationTerminalKind) -> TerminalPresentation {
    match kind {
        GenerationTerminalKind::Finished(reason) => {
            TerminalPresentation::Finished(format!("{reason:?}"))
        }
        GenerationTerminalKind::Failed => TerminalPresentation::Failed,
    }
}

fn terminal_kind_message(kind: GenerationTerminalKind) -> String {
    terminal_presentation_message(&terminal_presentation(kind))
}

fn terminal_presentation_message(presentation: &TerminalPresentation) -> String {
    match presentation {
        TerminalPresentation::Finished(reason) => format!("Generation finished: {reason}."),
        TerminalPresentation::Failed => "Generation failed.".to_owned(),
    }
}

fn released_terminal_message(presentation: &TerminalPresentation) -> String {
    format!(
        "{} Backend resources were released.",
        terminal_presentation_message(presentation)
    )
}

fn format_terminal_outcome(outcome: &GenerationTerminalOutcome) -> String {
    match outcome {
        GenerationTerminalOutcome::Finished(reason) => {
            format!("Generation finished: {reason:?}. Backend resources were released.")
        }
        GenerationTerminalOutcome::Failed(failure) => {
            format!("Generation failed: {failure}. Backend resources were released.")
        }
    }
}

fn connect_resolve(window: &AppWindow, runtime: Rc<RefCell<ApplicationRuntime>>) {
    let weak = window.as_weak();
    window.on_resolve_model(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        window.set_status_text(
            "Resolving repository metadata and immutable cached artifacts…".into(),
        );
        let result = runtime.borrow_mut().resolve_model(
            window.get_repository().to_string(),
            window.get_revision().to_string(),
        );
        if let Err(error) = result {
            window.set_status_text(error.to_string().into());
        }
        synchronize_controls(&window, &runtime.borrow());
    });
}

fn connect_load(window: &AppWindow, runtime: Rc<RefCell<ApplicationRuntime>>) {
    let weak = window.as_weak();
    window.on_load_model(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        window.set_status_text("Loading model weights on the CPU runtime…".into());
        let repository = window.get_repository().to_string();
        let revision = window.get_revision().to_string();
        let result = runtime.borrow_mut().load_model(&repository, &revision);
        if let Err(error) = result {
            window.set_status_text(error.to_string().into());
        }
        synchronize_controls(&window, &runtime.borrow());
    });
}

fn connect_unload(window: &AppWindow, runtime: Rc<RefCell<ApplicationRuntime>>) {
    let weak = window.as_weak();
    window.on_unload_model(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        window.set_status_text("Draining active work before deterministic unload…".into());
        if let Err(error) = runtime.borrow_mut().unload_model() {
            window.set_status_text(error.to_string().into());
        }
        synchronize_controls(&window, &runtime.borrow());
    });
}

fn connect_submit_message(
    window: &AppWindow,
    runtime: Rc<RefCell<ApplicationRuntime>>,
    presentation: Rc<RefCell<PresentationState>>,
) {
    let weak = window.as_weak();
    window.on_submit_message(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let message = window.get_message_input().to_string();
        let result = runtime
            .borrow_mut()
            .submit_user_message(&message, GenerationSettings::default());
        match result {
            Ok(request_id) => {
                begin_presented_request(&window, &runtime, &presentation, request_id.get());
                window.set_message_input("".into());
            }
            Err(error) => {
                window.set_status_text(format!("Message could not be submitted: {error}").into());
            }
        }
        synchronize_controls(&window, &runtime.borrow());
    });
}

fn connect_regenerate(
    window: &AppWindow,
    runtime: Rc<RefCell<ApplicationRuntime>>,
    presentation: Rc<RefCell<PresentationState>>,
) {
    let weak = window.as_weak();
    window.on_regenerate_response(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let result = runtime
            .borrow_mut()
            .regenerate_last_response(GenerationSettings::default());
        match result {
            Ok(request_id) => {
                begin_presented_request(&window, &runtime, &presentation, request_id.get());
            }
            Err(error) => {
                window
                    .set_status_text(format!("Response could not be regenerated: {error}").into());
            }
        }
        synchronize_controls(&window, &runtime.borrow());
    });
}

fn begin_presented_request(
    window: &AppWindow,
    runtime: &Rc<RefCell<ApplicationRuntime>>,
    presentation: &Rc<RefCell<PresentationState>>,
    request_id: u64,
) {
    let transcript = format_conversation(runtime.borrow().conversation());
    let mut presentation = presentation.borrow_mut();
    presentation.begin_request(request_id);
    render_generated_output_update(window, replace_conversation_update(transcript));
    window.set_terminal_text(presentation.terminal_text.clone().into());
    drop(presentation);
    window.set_status_text(
        format!("Response {request_id} submitted to TinyLlama Chat on Candle CPU.").into(),
    );
}

fn connect_cancel(window: &AppWindow, runtime: Rc<RefCell<ApplicationRuntime>>) {
    let weak = window.as_weak();
    window.on_cancel_generation(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let request_id = runtime
            .borrow()
            .state()
            .active_generation()
            .map(|summary| summary.request_id);
        let Some(request_id) = request_id else {
            window.set_status_text("No active generation can be cancelled.".into());
            return;
        };
        match runtime.borrow_mut().cancel_generation(request_id) {
            Ok(()) => window.set_status_text(
                cancellation_pending_message(request_id.get(), CancellationMessageState::Submitted)
                    .into(),
            ),
            Err(error) => {
                window.set_status_text(
                    format!("Cancellation could not be requested: {error}").into(),
                );
            }
        }
        synchronize_controls(&window, &runtime.borrow());
    });
}

fn connect_clear_conversation(window: &AppWindow, runtime: Rc<RefCell<ApplicationRuntime>>) {
    let weak = window.as_weak();
    window.on_clear_conversation(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        match runtime.borrow_mut().clear_conversation() {
            Ok(()) => {
                render_generated_output_update(&window, replace_conversation_update(String::new()));
                window.set_terminal_text("No response has completed.".into());
                window.set_status_text("Conversation cleared.".into());
            }
            Err(error) => {
                window
                    .set_status_text(format!("Conversation could not be cleared: {error}").into());
            }
        }
        synchronize_controls(&window, &runtime.borrow());
    });
}

fn format_conversation(records: &[ConversationRecord]) -> String {
    let mut transcript = String::new();
    for record in records {
        let label = match record.role {
            ConversationRole::System => "System",
            ConversationRole::User => "User",
            ConversationRole::Assistant => "Assistant",
        };
        transcript.push_str(label);
        transcript.push_str(": ");
        transcript.push_str(record.content.as_str());
        if let Some(attempt) = record.response_attempt.as_ref() {
            match &attempt.state {
                ResponseAttemptState::Streaming => {}
                ResponseAttemptState::Completed(reason) => {
                    transcript.push_str(format!("\n[completed: {reason:?}]").as_str());
                }
                ResponseAttemptState::Cancelled(reason) => {
                    transcript.push_str(format!("\n[cancelled: {reason:?}]").as_str());
                }
                ResponseAttemptState::Failed(failure) => {
                    transcript.push_str(format!("\n[failed: {failure}]").as_str());
                }
            }
            if attempt.superseded {
                transcript.push_str("\n[superseded by regeneration]");
            }
        }
        if !matches!(
            record
                .response_attempt
                .as_ref()
                .map(|attempt| &attempt.state),
            Some(ResponseAttemptState::Streaming)
        ) {
            transcript.push_str("\n\n");
        }
    }
    transcript
}

fn apply_event(window: &AppWindow, event: ApplicationEvent) {
    match event {
        ApplicationEvent::ModelResolved {
            model,
            persistence_warning,
        } => apply_model_resolved(window, model, persistence_warning),
        ApplicationEvent::ModelResolutionFailed { failure } => {
            window.set_resolved_commit("Not resolved".into());
            window.set_status_text(format!("Model resolution failed: {failure}").into());
        }
        ApplicationEvent::ModelLoaded { model } => {
            window.set_status_text(
                format!(
                    "Loaded generation {} with {} vocabulary entries.",
                    model.handle.generation.get(),
                    model.vocabulary_size,
                )
                .into(),
            );
        }
        ApplicationEvent::ModelLoadFailed { failure } => {
            window.set_status_text(format!("Model load failed: {failure}").into());
        }
        ApplicationEvent::ModelCompatibilityFailed { failure } => {
            window.set_status_text(format!("Model compatibility check failed: {failure}").into());
        }
        ApplicationEvent::GenerationStarted { request_id } => {
            window.set_status_text(format!("Generation {} is running.", request_id.get()).into());
        }
        ApplicationEvent::GenerationCancellationRequested { request_id } => {
            window.set_status_text(
                cancellation_pending_message(request_id.get(), CancellationMessageState::Accepted)
                    .into(),
            );
        }
        ApplicationEvent::GenerationCancellationFailed {
            request_id,
            failure,
        } => {
            window.set_status_text(
                format!(
                    "Cancellation failed for generation {}: {failure}",
                    request_id.get()
                )
                .into(),
            );
        }
        ApplicationEvent::GenerationCleanupPending {
            request_id,
            exhausted,
            failure,
        } => {
            let state = if exhausted { "exhausted" } else { "pending" };
            window.set_status_text(
                format!("Generation {} cleanup {state}: {failure}", request_id.get()).into(),
            );
        }
        ApplicationEvent::GenerationFinished { terminal } => {
            window.set_status_text(
                format!(
                    "Generation {} reached terminal release.",
                    terminal.request_id.get()
                )
                .into(),
            );
            window.set_terminal_text(format_terminal_outcome(&terminal.outcome).into());
        }
        ApplicationEvent::ModelDraining { .. } => {
            window.set_status_text("Model is draining active work.".into());
        }
        ApplicationEvent::ModelUnloaded {
            cancelled_requests, ..
        } => {
            let message = if cancelled_requests == 0 {
                "Model resources were unloaded.".to_owned()
            } else {
                format!(
                    "Model resources were unloaded after cancelling {cancelled_requests} active requests."
                )
            };
            window.set_status_text(message.into());
        }
        ApplicationEvent::ModelUnloadFailed { failure } => {
            window.set_status_text(format!("Model unload failed: {failure}").into());
        }
        ApplicationEvent::HubDisconnected => {
            window.set_status_text("Hub resolver disconnected".into());
        }
        ApplicationEvent::RuntimeDisconnected => {
            window.set_status_text("Inference runtime disconnected".into());
        }
    }
}

fn apply_model_resolved(
    window: &AppWindow,
    model: ResolvedModel,
    persistence_warning: Option<ApplicationFailure>,
) {
    window.set_resolved_commit(model.commit.into());
    let scalar = model.scalar_type.map_or("unknown", scalar_type_name);
    let chat = match model.chat_compatibility {
        ChatCompatibility::Supported(_) => "verified TinyLlama Chat v1",
        ChatCompatibility::Unsupported => "direct completion only; chat compatibility unknown",
    };
    let message = match persistence_warning {
        Some(warning) => format!(
            "Artifacts and tokenizer ({} tokens, {scalar}, {chat}) are ready; catalogue persistence failed: {warning}",
            model.vocabulary_size,
        ),
        None => format!(
            "Artifacts and tokenizer ({} tokens, {scalar}, {chat}) are ready for CPU loading.",
            model.vocabulary_size,
        ),
    };
    window.set_status_text(message.into());
}

const fn scalar_type_name(value: ScalarType) -> &'static str {
    match value {
        ScalarType::F32 => "F32",
        ScalarType::F16 => "F16",
        ScalarType::Bf16 => "BF16",
        ScalarType::I8 => "I8",
        ScalarType::U8 => "U8",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CancellationMessageState, FrameOutputDelta, GeneratedOutputUpdate, PresentationState,
        TerminalPresentation, cancellation_pending_message, control_state, format_conversation,
        format_terminal_outcome, output_state_message, released_terminal_message,
        replace_conversation_update,
    };
    use application_runtime::{
        ApplicationFailure, ApplicationFailureKind, ApplicationOutputState, ConversationProvenance,
        ConversationRecord, ConversationRecordId, ConversationRetention, ConversationRole,
        ConversationTokenEstimate, GenerationTerminalKind, GenerationTerminalOutcome,
        ResponseAttempt, ResponseAttemptId, ResponseAttemptState,
    };

    #[test]
    fn running_generation_exposes_cancellation() {
        let controls = control_state(false, false, false, false, true, true, "message");

        assert!(!controls.can_submit_message);
        assert!(controls.can_cancel);
        assert!(controls.can_unload);
        assert!(!controls.can_edit_message);
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
    fn message_submission_requires_nonempty_visible_input() {
        let empty = control_state(false, false, true, false, false, true, "  \n ");
        let populated = control_state(false, false, true, false, false, true, "Hello");

        assert!(empty.can_edit_message);
        assert!(!empty.can_submit_message);
        assert!(populated.can_submit_message);
    }

    #[test]
    fn released_response_reenables_submit_and_regeneration() {
        let controls = control_state(false, false, true, true, false, true, "Next message");

        assert!(controls.can_edit_message);
        assert!(controls.can_submit_message);
        assert!(controls.can_regenerate);
        assert!(!controls.can_cancel);
        assert!(controls.can_unload);
    }

    #[test]
    fn text_delta_contains_only_the_new_fragment() {
        let mut presentation = PresentationState {
            displayed_request: Some(7),
            terminal_text: String::new(),
        };
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
        assert_eq!(presentation.displayed_request, Some(7));
    }

    #[test]
    fn replacing_transcript_is_one_batched_presentation_update() {
        assert_eq!(
            replace_conversation_update("User: hello".to_owned()),
            GeneratedOutputUpdate::Replace("User: hello".to_owned())
        );
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
}
