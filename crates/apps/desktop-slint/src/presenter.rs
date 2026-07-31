//! Slint-specific presentation mapping over the reusable application runtime.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use application_runtime::{
    ApplicationActivity, ApplicationDevice, ApplicationEngine, ApplicationEvent,
    ApplicationFailure, ApplicationModelFormat, ApplicationOutputBatch,
    ApplicationOutputRecordKind, ApplicationOutputState, ApplicationRuntime, ApplicationScalarType,
    ApplicationSource, ChatCompatibility, ConversationRecord, ConversationRole, GenerationSettings,
    GenerationTerminalKind, GenerationTerminalOutcome, ImmutableModelIdentity, LoadedModel,
    ModelSelection, ResolvedModel, ResponseAttemptState,
};
use slint::ComponentHandle;

use crate::AppWindow;

const UI_FRAME_MILLISECONDS: u64 = 16;
const MAXIMUM_EVENTS_PER_FRAME: usize = 64;
const DEFAULT_TERMINAL_TEXT: &str = "No response has completed.";

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
        connect_clear_conversation(window, Rc::clone(runtime), Rc::clone(&state));
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
                let (output, refresh_conversation) = {
                    let mut runtime_ref = runtime.borrow_mut();
                    let mut refresh_conversation = false;
                    for _ in 0..MAXIMUM_EVENTS_PER_FRAME {
                        let Some(event) = runtime_ref.poll_event() else {
                            break;
                        };
                        refresh_conversation |= event_requires_conversation_snapshot(&event);
                        apply_event(&window, event);
                    }

                    let presentation = presentation.borrow();
                    let displayed_request = presentation.displayed_request;
                    let refresh_conversation =
                        refresh_conversation && presentation.allows_conversation_snapshot();
                    drop(presentation);
                    let output = runtime_ref
                        .pull_output(|batch| collect_output_batch(&batch, displayed_request));
                    (output, refresh_conversation)
                };

                match output {
                    Ok(delta) => {
                        let mut presentation = presentation.borrow_mut();
                        let mut update = presentation.apply_delta(delta);
                        if refresh_conversation {
                            update.output = None;
                        }
                        render_presentation_update(&window, &presentation, update);
                    }
                    Err(error) => {
                        window.set_status_text(
                            format!("Generated output pull failed: {error}").into(),
                        );
                    }
                }
                if refresh_conversation {
                    let runtime_ref = runtime.borrow();
                    synchronize_conversation(&window, &runtime_ref);
                }
                let runtime_ref = runtime.borrow();
                synchronize_controls(&window, &runtime_ref);
                synchronize_usage(
                    &window,
                    &runtime_ref,
                    presentation.borrow().displayed_request,
                );
            },
        );
        timer
    }
}

fn map_model_selection(repository: &str, revision: &str) -> ModelSelection {
    ModelSelection::new(repository, revision)
}

fn selected_model(window: &AppWindow) -> ModelSelection {
    map_model_selection(
        window.get_repository().as_str(),
        window.get_revision().as_str(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerMode {
    Unavailable,
    Chat,
    DirectCompletion,
}

impl ComposerMode {
    const fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "No model loaded",
            Self::Chat => "Chat",
            Self::DirectCompletion => "Direct completion",
        }
    }

    const fn guidance(self) -> &'static str {
        match self {
            Self::Unavailable => "Load a model to enable generation.",
            Self::Chat => {
                "Verified chat compatibility: E1 owns conversation history, prompt rendering, and regeneration."
            }
            Self::DirectCompletion => {
                "No verified chat profile: input is submitted as an E1 direct-completion prompt without inferred history or template semantics."
            }
        }
    }

    const fn input_label(self) -> &'static str {
        match self {
            Self::Unavailable | Self::Chat => "Message",
            Self::DirectCompletion => "Prompt",
        }
    }

    const fn submit_label(self) -> &'static str {
        match self {
            Self::Unavailable | Self::Chat => "Send",
            Self::DirectCompletion => "Complete",
        }
    }
}

const fn composer_mode_from_evidence(
    has_loaded_model: bool,
    has_verified_chat_compatibility: bool,
) -> ComposerMode {
    if !has_loaded_model {
        ComposerMode::Unavailable
    } else if has_verified_chat_compatibility {
        ComposerMode::Chat
    } else {
        ComposerMode::DirectCompletion
    }
}

fn composer_mode(runtime: &ApplicationRuntime) -> ComposerMode {
    let state = runtime.state();
    let Some(loaded) = state.loaded() else {
        return ComposerMode::Unavailable;
    };
    let has_verified_chat_compatibility = state.resolved().is_some_and(|resolved| {
        resolved.selection() == loaded.selection()
            && resolved.identity() == loaded.identity()
            && matches!(
                resolved.chat_compatibility(),
                ChatCompatibility::Supported(_)
            )
    });
    composer_mode_from_evidence(true, has_verified_chat_compatibility)
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "each field is one authoritative E1 admission or lifecycle flag"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RuntimeAdmissions {
    can_resolve: bool,
    can_load: bool,
    can_submit_chat: bool,
    can_start_generation: bool,
    can_regenerate: bool,
    can_clear: bool,
    can_cancel: bool,
    can_unload: bool,
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
    can_clear: bool,
    can_cancel: bool,
    can_unload: bool,
}

fn control_state(admissions: RuntimeAdmissions, mode: ComposerMode, message: &str) -> ControlState {
    let can_edit_message = match mode {
        ComposerMode::Unavailable => false,
        ComposerMode::Chat => admissions.can_submit_chat,
        ComposerMode::DirectCompletion => admissions.can_start_generation,
    };
    ControlState {
        can_resolve: admissions.can_resolve,
        can_load: admissions.can_load,
        can_edit_message,
        can_submit_message: can_edit_message && !message.trim().is_empty(),
        can_regenerate: mode == ComposerMode::Chat && admissions.can_regenerate,
        can_clear: admissions.can_clear,
        can_cancel: admissions.can_cancel,
        can_unload: admissions.can_unload,
    }
}

/// Synchronizes every control, model summary, mode, and usage value from authoritative state.
pub fn synchronize_controls(window: &AppWindow, runtime: &ApplicationRuntime) {
    let state = runtime.state();
    let selection = selected_model(window);
    let can_resolve = state.can_resolve(&selection);
    let can_load = state.can_load(&selection);
    let mode = composer_mode(runtime);
    let message = window.get_message_input().to_string();
    let controls = control_state(
        RuntimeAdmissions {
            can_resolve,
            can_load,
            can_submit_chat: runtime.can_submit_chat_message(),
            can_start_generation: state.can_start_generation(),
            can_regenerate: runtime.can_regenerate_response(),
            can_clear: state.active_generation().is_none(),
            can_cancel: state.can_cancel_generation(),
            can_unload: state.can_unload(),
        },
        mode,
        &message,
    );

    window.set_busy(state.activity() != ApplicationActivity::Idle);
    window.set_can_edit_selection(
        state.activity() == ApplicationActivity::Idle && state.loaded().is_none(),
    );
    window.set_can_resolve(controls.can_resolve);
    window.set_can_load(controls.can_load);
    window.set_can_edit_message(controls.can_edit_message);
    window.set_can_submit_message(controls.can_submit_message);
    window.set_can_regenerate(controls.can_regenerate);
    window.set_can_clear_conversation(controls.can_clear);
    window.set_can_cancel(controls.can_cancel);
    window.set_can_unload(controls.can_unload);
    window.set_composer_mode(mode.label().into());
    window.set_composer_guidance(mode.guidance().into());
    window.set_composer_input_label(mode.input_label().into());
    window.set_composer_submit_label(mode.submit_label().into());

    window.set_selected_model_summary(selected_model_summary(&selection).into());
    window.set_resolved_model_summary(
        state
            .resolved()
            .map_or_else(|| "Not resolved.".to_owned(), resolved_model_summary)
            .into(),
    );
    window.set_loaded_model_summary(
        state
            .loaded()
            .map_or_else(|| "Not loaded.".to_owned(), loaded_model_summary)
            .into(),
    );
}

fn synchronize_usage(
    window: &AppWindow,
    runtime: &ApplicationRuntime,
    displayed_request: Option<u64>,
) {
    let state = runtime.state();
    let usage = state
        .active_generation()
        .filter(|summary| displayed_request == Some(summary.request_id.get()))
        .map(|summary| summary.usage)
        .or_else(|| {
            state
                .last_generation()
                .filter(|terminal| displayed_request == Some(terminal.request_id.get()))
                .map(|terminal| terminal.usage)
        });
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
}

fn selected_model_summary(selection: &ModelSelection) -> String {
    format!(
        "{} • Repository: {} • Revision: {} • Scalar: pending resolution • Identity: pending resolution",
        current_model_target_label(),
        selection.repository(),
        selection.revision(),
    )
}

fn resolved_model_summary(model: &ResolvedModel) -> String {
    detailed_model_summary(
        model.engine(),
        model.source(),
        model.device(),
        model.format(),
        model.scalar_type(),
        model.identity(),
    )
}

fn loaded_model_summary(model: &LoadedModel) -> String {
    detailed_model_summary(
        model.engine(),
        model.source(),
        model.device(),
        model.format(),
        Some(model.scalar_type()),
        model.identity(),
    )
}

fn detailed_model_summary(
    engine: ApplicationEngine,
    source: ApplicationSource,
    device: ApplicationDevice,
    format: ApplicationModelFormat,
    scalar_type: Option<ApplicationScalarType>,
    identity: &ImmutableModelIdentity,
) -> String {
    let scalar = scalar_type.map_or("Unknown", scalar_type_label);
    format!(
        "{} • Scalar: {scalar} • Identity: {}",
        model_target_label(engine, source, device, format),
        immutable_identity_label(identity)
    )
}

fn current_model_target_label() -> String {
    model_target_label(
        ApplicationEngine::Candle,
        ApplicationSource::HuggingFaceHub,
        ApplicationDevice::Cpu,
        ApplicationModelFormat::Safetensors,
    )
}

fn model_target_label(
    engine: ApplicationEngine,
    source: ApplicationSource,
    device: ApplicationDevice,
    format: ApplicationModelFormat,
) -> String {
    format!(
        "Engine: {} • Source: {} • Device: {} • Format: {}",
        engine_label(engine),
        source_label(source),
        device_label(device),
        model_format_label(format)
    )
}

const fn engine_label(engine: ApplicationEngine) -> &'static str {
    match engine {
        ApplicationEngine::Candle => "Candle",
    }
}

const fn source_label(source: ApplicationSource) -> &'static str {
    match source {
        ApplicationSource::HuggingFaceHub => "Hugging Face Hub",
    }
}

const fn device_label(device: ApplicationDevice) -> &'static str {
    match device {
        ApplicationDevice::Cpu => "CPU",
    }
}

const fn model_format_label(format: ApplicationModelFormat) -> &'static str {
    match format {
        ApplicationModelFormat::Safetensors => "Safetensors",
    }
}

const fn scalar_type_label(value: ApplicationScalarType) -> &'static str {
    match value {
        ApplicationScalarType::F32 => "F32",
        ApplicationScalarType::F16 => "F16",
        ApplicationScalarType::Bf16 => "BF16",
    }
}

fn immutable_identity_label(identity: &ImmutableModelIdentity) -> String {
    format!(
        "Hub commit {} ({})",
        identity.commit(),
        identity.repository()
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TranscriptPresentation {
    #[default]
    Conversation,
    DirectCompletion,
}

#[derive(Default)]
struct PresentationState {
    displayed_request: Option<u64>,
    terminal_text: String,
    transcript: TranscriptPresentation,
}

impl PresentationState {
    fn begin_request(&mut self, request_id: u64) {
        self.displayed_request = Some(request_id);
        self.terminal_text.clear();
        self.terminal_text
            .push_str("Generation submitted; waiting for admission.");
    }

    fn begin_chat_request(&mut self, request_id: u64) {
        self.transcript = TranscriptPresentation::Conversation;
        self.begin_request(request_id);
    }

    fn begin_direct_request(&mut self, request_id: u64, prompt: &str) -> GeneratedOutputUpdate {
        self.transcript = TranscriptPresentation::DirectCompletion;
        self.begin_request(request_id);
        GeneratedOutputUpdate::Replace(format_direct_completion_transcript(prompt))
    }

    fn clear(&mut self, mode: ComposerMode) -> GeneratedOutputUpdate {
        self.displayed_request = None;
        self.terminal_text.clear();
        self.terminal_text.push_str(DEFAULT_TERMINAL_TEXT);
        self.transcript = if mode == ComposerMode::DirectCompletion {
            TranscriptPresentation::DirectCompletion
        } else {
            TranscriptPresentation::Conversation
        };
        GeneratedOutputUpdate::Replace(String::new())
    }

    const fn allows_conversation_snapshot(&self) -> bool {
        matches!(self.transcript, TranscriptPresentation::Conversation)
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

fn format_direct_completion_transcript(prompt: &str) -> String {
    format!("Prompt:\n{prompt}\n\nCompletion:\n")
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
        GeneratedOutputUpdate::Append(text) => window.invoke_append_generated_text(text.into()),
        GeneratedOutputUpdate::Replace(text) => {
            window.invoke_replace_transcript(text.into());
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
        let selection = selected_model(&window);
        window.set_status_text(resolution_progress_message().into());
        if let Err(error) = runtime.borrow_mut().resolve_model(selection) {
            window.set_status_text(error.to_string().into());
        }
        synchronize_controls(&window, &runtime.borrow());
    });
}

const fn resolution_progress_message() -> &'static str {
    "Resolving Hub metadata and immutable cached Safetensors artifacts…"
}

fn connect_load(window: &AppWindow, runtime: Rc<RefCell<ApplicationRuntime>>) {
    let weak = window.as_weak();
    window.on_load_model(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let selection = selected_model(&window);
        window.set_status_text(format!("Loading {}.", current_model_target_label()).into());
        if let Err(error) = runtime.borrow_mut().load_model(&selection) {
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
        let mode = {
            let runtime_ref = runtime.borrow();
            composer_mode(&runtime_ref)
        };
        match mode {
            ComposerMode::Chat => {
                submit_chat_message(&window, &runtime, &presentation, &message);
            }
            ComposerMode::DirectCompletion => {
                submit_direct_completion(&window, &runtime, &presentation, &message);
            }
            ComposerMode::Unavailable => {
                window.set_status_text("Load a model before submitting input.".into());
            }
        }
        synchronize_controls(&window, &runtime.borrow());
    });
}

fn submit_chat_message(
    window: &AppWindow,
    runtime: &Rc<RefCell<ApplicationRuntime>>,
    presentation: &Rc<RefCell<PresentationState>>,
    message: &str,
) {
    let result = runtime
        .borrow_mut()
        .submit_user_message(message, GenerationSettings::default());
    match result {
        Ok(request_id) => {
            begin_presented_chat_request(window, runtime, presentation, request_id.get());
            window.set_message_input("".into());
        }
        Err(error) => {
            synchronize_conversation_if_allowed(window, runtime, presentation);
            window.set_status_text(format!("Message could not be submitted: {error}").into());
        }
    }
}

fn submit_direct_completion(
    window: &AppWindow,
    runtime: &Rc<RefCell<ApplicationRuntime>>,
    presentation: &Rc<RefCell<PresentationState>>,
    prompt: &str,
) {
    let result = runtime
        .borrow_mut()
        .start_generation(prompt, GenerationSettings::default());
    match result {
        Ok(request_id) => {
            begin_presented_direct_request(window, presentation, request_id.get(), prompt);
            window.set_status_text(
                generation_submission_message(
                    &runtime.borrow(),
                    ComposerMode::DirectCompletion,
                    request_id.get(),
                )
                .into(),
            );
            window.set_message_input("".into());
        }
        Err(error) => {
            window.set_status_text(
                format!("Direct completion could not be submitted: {error}").into(),
            );
        }
    }
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
        if composer_mode(&runtime.borrow()) != ComposerMode::Chat {
            window.set_status_text("Regeneration is available only in Chat mode.".into());
            synchronize_controls(&window, &runtime.borrow());
            return;
        }
        let result = runtime
            .borrow_mut()
            .regenerate_last_response(GenerationSettings::default());
        match result {
            Ok(request_id) => {
                begin_presented_chat_request(&window, &runtime, &presentation, request_id.get());
            }
            Err(error) => {
                window
                    .set_status_text(format!("Response could not be regenerated: {error}").into());
            }
        }
        synchronize_controls(&window, &runtime.borrow());
    });
}

fn begin_presented_chat_request(
    window: &AppWindow,
    runtime: &Rc<RefCell<ApplicationRuntime>>,
    presentation: &Rc<RefCell<PresentationState>>,
    request_id: u64,
) {
    let mut presentation = presentation.borrow_mut();
    presentation.begin_chat_request(request_id);
    window.set_terminal_text(presentation.terminal_text.clone().into());
    drop(presentation);
    let runtime_ref = runtime.borrow();
    synchronize_conversation(window, &runtime_ref);
    window.set_status_text(
        generation_submission_message(&runtime_ref, ComposerMode::Chat, request_id).into(),
    );
}

fn begin_presented_direct_request(
    window: &AppWindow,
    presentation: &Rc<RefCell<PresentationState>>,
    request_id: u64,
    prompt: &str,
) {
    let mut presentation = presentation.borrow_mut();
    let update = presentation.begin_direct_request(request_id, prompt);
    render_generated_output_update(window, update);
    window.set_terminal_text(presentation.terminal_text.clone().into());
}

fn generation_submission_message(
    runtime: &ApplicationRuntime,
    mode: ComposerMode,
    request_id: u64,
) -> String {
    let target = runtime.state().loaded().map_or_else(
        || "the loaded model".to_owned(),
        |loaded| {
            format!(
                "{} on {}",
                engine_label(loaded.engine()),
                device_label(loaded.device())
            )
        },
    );
    match mode {
        ComposerMode::Chat => format!("Chat response {request_id} submitted to {target}."),
        ComposerMode::DirectCompletion => {
            format!("Direct completion {request_id} submitted to {target}.")
        }
        ComposerMode::Unavailable => {
            format!("Generation {request_id} submitted to {target}.")
        }
    }
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

fn connect_clear_conversation(
    window: &AppWindow,
    runtime: Rc<RefCell<ApplicationRuntime>>,
    presentation: Rc<RefCell<PresentationState>>,
) {
    let weak = window.as_weak();
    window.on_clear_conversation(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if runtime.borrow().state().active_generation().is_some() {
            window.set_status_text(
                "Cancel the active generation and wait for release before clearing.".into(),
            );
            synchronize_controls(&window, &runtime.borrow());
            return;
        }

        let clear_result = runtime.borrow_mut().clear_conversation();
        let mode = composer_mode(&runtime.borrow());
        let mut presentation = presentation.borrow_mut();
        let update = presentation.clear(mode);
        render_generated_output_update(&window, update);
        window.set_terminal_text(presentation.terminal_text.clone().into());
        drop(presentation);

        match clear_result {
            Ok(()) => {
                let message = match mode {
                    ComposerMode::Chat => "Conversation cleared.",
                    ComposerMode::DirectCompletion => "Direct-completion presentation cleared.",
                    ComposerMode::Unavailable => "Presentation cleared.",
                };
                window.set_status_text(message.into());
            }
            Err(error) => {
                window.set_status_text(
                    format!("Presentation cleared; E1 conversation clear failed: {error}").into(),
                );
            }
        }
        let runtime_ref = runtime.borrow();
        synchronize_controls(&window, &runtime_ref);
        synchronize_usage(&window, &runtime_ref, None);
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

fn synchronize_conversation(window: &AppWindow, runtime: &ApplicationRuntime) {
    let transcript = format_conversation(runtime.conversation());
    render_generated_output_update(window, replace_conversation_update(transcript));
}

fn synchronize_conversation_if_allowed(
    window: &AppWindow,
    runtime: &Rc<RefCell<ApplicationRuntime>>,
    presentation: &Rc<RefCell<PresentationState>>,
) {
    if presentation.borrow().allows_conversation_snapshot() {
        synchronize_conversation(window, &runtime.borrow());
    }
}

const fn event_requires_conversation_snapshot(event: &ApplicationEvent) -> bool {
    matches!(
        event,
        ApplicationEvent::GenerationCleanupPending { .. }
            | ApplicationEvent::GenerationFinished { .. }
            | ApplicationEvent::RuntimeDisconnected
    )
}

fn apply_event(window: &AppWindow, event: ApplicationEvent) {
    match event {
        ApplicationEvent::ModelResolved {
            model,
            persistence_warning,
        } => apply_model_resolved(window, &model, persistence_warning),
        ApplicationEvent::ModelResolutionFailed { failure } => {
            window.set_status_text(format!("Model resolution failed: {failure}").into());
        }
        ApplicationEvent::ModelLoaded { model } => {
            window.set_status_text(
                format!(
                    "Loaded {} as generation {} with {} vocabulary entries.",
                    model_target_label(
                        model.engine(),
                        model.source(),
                        model.device(),
                        model.format(),
                    ),
                    model.handle().generation.get(),
                    model.vocabulary_size(),
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
    model: &ResolvedModel,
    persistence_warning: Option<ApplicationFailure>,
) {
    let scalar = model.scalar_type().map_or("unknown", scalar_type_label);
    let mode = match model.chat_compatibility() {
        ChatCompatibility::Supported(_) => "verified Chat mode",
        ChatCompatibility::Unsupported => "Direct completion mode; chat is not verified",
    };
    let target = model_target_label(
        model.engine(),
        model.source(),
        model.device(),
        model.format(),
    );
    let message = persistence_warning.map_or_else(
        || {
            format!(
                "Resolved {target} ({} vocabulary entries, {scalar}, {mode}) and ready for loading.",
                model.vocabulary_size(),
            )
        },
        |warning| {
            format!(
                "Resolved {target} ({} vocabulary entries, {scalar}, {mode}); catalogue persistence failed: {warning}",
                model.vocabulary_size(),
            )
        },
    );
    window.set_status_text(message.into());
}

#[cfg(test)]
mod tests {
    use super::{
        CancellationMessageState, ComposerMode, FrameOutputDelta, GeneratedOutputUpdate,
        MAXIMUM_EVENTS_PER_FRAME, PresentationState, RuntimeAdmissions, TerminalPresentation,
        UI_FRAME_MILLISECONDS, cancellation_pending_message, composer_mode_from_evidence,
        control_state, event_requires_conversation_snapshot, format_conversation,
        format_terminal_outcome, map_model_selection, model_target_label, output_state_message,
        released_terminal_message, replace_conversation_update, selected_model_summary,
    };
    use application_runtime::{
        ApplicationDevice, ApplicationEngine, ApplicationEvent, ApplicationFailure,
        ApplicationFailureKind, ApplicationModelFormat, ApplicationOutputState, ApplicationSource,
        ConversationProvenance, ConversationRecord, ConversationRecordId, ConversationRetention,
        ConversationRole, ConversationTokenEstimate, GenerationTerminalKind,
        GenerationTerminalOutcome, ResponseAttempt, ResponseAttemptId, ResponseAttemptState,
    };

    #[test]
    fn repository_and_revision_map_to_model_selection() {
        let selection = map_model_selection(" owner/model ", " main ");

        assert_eq!(selection.repository(), "owner/model");
        assert_eq!(selection.revision(), "main");

        let summary = selected_model_summary(&selection);
        assert!(summary.contains("Engine: Candle"));
        assert!(summary.contains("Repository: owner/model"));
        assert!(summary.contains("Revision: main"));
    }

    #[test]
    fn target_summary_reports_orthogonal_current_facts() {
        let target = model_target_label(
            ApplicationEngine::Candle,
            ApplicationSource::HuggingFaceHub,
            ApplicationDevice::Cpu,
            ApplicationModelFormat::Safetensors,
        );

        assert!(target.contains("Engine: Candle"));
        assert!(target.contains("Source: Hugging Face Hub"));
        assert!(target.contains("Device: CPU"));
        assert!(target.contains("Format: Safetensors"));
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
        let mut presentation = PresentationState {
            displayed_request: Some(7),
            ..PresentationState::default()
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
        assert_eq!(presentation.displayed_request, None);

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
}
