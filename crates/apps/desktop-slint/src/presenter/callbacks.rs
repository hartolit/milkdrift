use std::cell::RefCell;
use std::rc::Rc;

use application_runtime::{
    ApplicationEvent, ApplicationFailure, ApplicationRuntime, ChatCompatibility,
    GenerationSettings, ResolvedModel,
};
use slint::ComponentHandle;

use super::model::{
    ComposerMode, composer_mode, current_model_target_label, device_label, engine_label,
    model_target_label, scalar_type_label, selected_model,
};
use super::output::{
    PresentationState, format_terminal_outcome, render_generated_output_update,
    synchronize_conversation, synchronize_usage,
};
use super::synchronize_controls;
use crate::AppWindow;

pub(super) fn connect(
    window: &AppWindow,
    runtime: &Rc<RefCell<ApplicationRuntime>>,
    presentation: &Rc<RefCell<PresentationState>>,
) {
    connect_resolve(window, Rc::clone(runtime));
    connect_load(window, Rc::clone(runtime));
    connect_unload(window, Rc::clone(runtime));
    connect_submit_message(window, Rc::clone(runtime), Rc::clone(presentation));
    connect_regenerate(window, Rc::clone(runtime), Rc::clone(presentation));
    connect_cancel(window, Rc::clone(runtime));
    connect_clear_conversation(window, Rc::clone(runtime), Rc::clone(presentation));
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
    window.set_terminal_text(presentation.terminal_text().into());
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
    window.set_terminal_text(presentation.terminal_text().into());
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CancellationMessageState {
    Submitted,
    Accepted,
}

pub(super) fn cancellation_pending_message(
    request_id: u64,
    state: CancellationMessageState,
) -> String {
    match state {
        CancellationMessageState::Submitted => format!(
            "Cancellation for generation {request_id} is pending until a safe backend boundary."
        ),
        CancellationMessageState::Accepted => format!(
            "Cancellation for generation {request_id} was accepted and remains pending until completion."
        ),
    }
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
        window.set_terminal_text(presentation.terminal_text().into());
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

fn synchronize_conversation_if_allowed(
    window: &AppWindow,
    runtime: &Rc<RefCell<ApplicationRuntime>>,
    presentation: &Rc<RefCell<PresentationState>>,
) {
    if presentation.borrow().allows_conversation_snapshot() {
        synchronize_conversation(window, &runtime.borrow());
    }
}

pub(super) const fn event_requires_conversation_snapshot(event: &ApplicationEvent) -> bool {
    matches!(
        event,
        ApplicationEvent::GenerationCleanupPending { .. }
            | ApplicationEvent::GenerationFinished { .. }
            | ApplicationEvent::RuntimeDisconnected
    )
}

pub(super) fn apply_event(window: &AppWindow, event: ApplicationEvent) {
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
