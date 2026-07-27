//! Slint-specific presentation mapping over the reusable application runtime.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use application_runtime::{
    ApplicationActivity, ApplicationEvent, ApplicationFailure, ApplicationOutputBatch,
    ApplicationOutputRecordKind, ApplicationOutputState, ApplicationRuntime, GenerationSettings,
    GenerationTerminalKind, GenerationTerminalOutcome, ResolvedModel, ScalarType,
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
        connect_generate(window, Rc::clone(runtime), Rc::clone(&state));
        connect_cancel(window, Rc::clone(runtime));
        connect_clear_output(window);
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
    let prompt = window.get_prompt().to_string();
    let controls = control_state(
        state.can_resolve(),
        state.can_load(&repository, &revision),
        state.can_start_generation(),
        state.can_cancel_generation(),
        state.can_unload(),
        &prompt,
    );

    window.set_busy(state.activity() != ApplicationActivity::Idle);
    window.set_can_resolve(controls.can_resolve);
    window.set_can_load(controls.can_load);
    window.set_can_edit_prompt(controls.can_edit_prompt);
    window.set_can_generate(controls.can_generate);
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
    can_edit_prompt: bool,
    can_generate: bool,
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
    can_cancel: bool,
    can_unload: bool,
    prompt: &str,
) -> ControlState {
    ControlState {
        can_resolve,
        can_load,
        can_edit_prompt: can_start_generation,
        can_generate: can_start_generation && !prompt.trim().is_empty(),
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
    Reset,
}

#[derive(Debug, PartialEq, Eq)]
struct PresentationUpdate {
    output: Option<GeneratedOutputUpdate>,
    terminal_changed: bool,
    invalid_text_record: bool,
}

const fn reset_generated_output_update() -> GeneratedOutputUpdate {
    GeneratedOutputUpdate::Reset
}

fn render_generated_output_update(window: &AppWindow, update: GeneratedOutputUpdate) {
    match update {
        GeneratedOutputUpdate::Append(text) => window.invoke_append_generated_output(text.into()),
        GeneratedOutputUpdate::Reset => window.invoke_reset_generated_output(),
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

fn connect_generate(
    window: &AppWindow,
    runtime: Rc<RefCell<ApplicationRuntime>>,
    presentation: Rc<RefCell<PresentationState>>,
) {
    let weak = window.as_weak();
    window.on_generate(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let prompt = window.get_prompt().to_string();
        match runtime
            .borrow_mut()
            .start_generation(&prompt, GenerationSettings::default())
        {
            Ok(request_id) => {
                let mut presentation = presentation.borrow_mut();
                presentation.begin_request(request_id.get());
                render_generated_output_update(&window, reset_generated_output_update());
                window.set_terminal_text(presentation.terminal_text.clone().into());
                drop(presentation);
                window.set_status_text(
                    format!(
                        "Generation {} submitted to Candle on CPU.",
                        request_id.get()
                    )
                    .into(),
                );
            }
            Err(error) => {
                window.set_status_text(format!("Generation could not start: {error}").into());
            }
        }
        synchronize_controls(&window, &runtime.borrow());
    });
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

fn connect_clear_output(window: &AppWindow) {
    let weak = window.as_weak();
    window.on_clear_output(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        render_generated_output_update(&window, reset_generated_output_update());
    });
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
    let message = match persistence_warning {
        Some(warning) => format!(
            "Artifacts and tokenizer ({} tokens, {scalar}) are ready; catalogue persistence failed: {warning}",
            model.vocabulary_size,
        ),
        None => format!(
            "Artifacts and tokenizer ({} tokens, {scalar}) are ready for CPU loading.",
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
        TerminalPresentation, cancellation_pending_message, control_state, format_terminal_outcome,
        output_state_message, released_terminal_message, reset_generated_output_update,
    };
    use application_runtime::{
        ApplicationFailure, ApplicationFailureKind, ApplicationOutputState, GenerationTerminalKind,
        GenerationTerminalOutcome,
    };

    #[test]
    fn running_generation_exposes_cancellation() {
        let controls = control_state(false, false, false, true, true, "prompt");

        assert!(!controls.can_generate);
        assert!(controls.can_cancel);
        assert!(controls.can_unload);
        assert!(!controls.can_edit_prompt);
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
    fn generation_requires_a_nonempty_visible_prompt() {
        let empty = control_state(false, false, true, false, true, "  \n ");
        let populated = control_state(false, false, true, false, true, "Hello");

        assert!(empty.can_edit_prompt);
        assert!(!empty.can_generate);
        assert!(populated.can_generate);
    }

    #[test]
    fn released_generation_reenables_generate_and_keeps_unload_available() {
        let controls = control_state(false, false, true, false, true, "Next prompt");

        assert!(controls.can_edit_prompt);
        assert!(controls.can_generate);
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
    fn clear_output_resets_only_widget_output_state() {
        let presentation = PresentationState {
            displayed_request: Some(7),
            terminal_text: "Still running".to_owned(),
        };

        assert_eq!(
            reset_generated_output_update(),
            GeneratedOutputUpdate::Reset
        );
        assert_eq!(presentation.displayed_request, Some(7));
        assert_eq!(presentation.terminal_text, "Still running");
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
