//! Slint-specific presentation mapping over the reusable application runtime.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use application_runtime::{ApplicationActivity, ApplicationEvent, ApplicationRuntime, ScalarType};
use slint::ComponentHandle;

use crate::AppWindow;

const UI_FRAME_MILLISECONDS: u64 = 16;
const MAXIMUM_EVENTS_PER_FRAME: usize = 64;

pub fn connect(window: &AppWindow, runtime: Rc<RefCell<ApplicationRuntime>>) {
    connect_resolve(window, Rc::clone(&runtime));
    connect_load(window, Rc::clone(&runtime));
    connect_unload(window, runtime);
}

pub fn start_frame_timer(
    window: &AppWindow,
    runtime: Rc<RefCell<ApplicationRuntime>>,
) -> slint::Timer {
    let timer = slint::Timer::default();
    let weak = window.as_weak();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(UI_FRAME_MILLISECONDS),
        move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let mut runtime = runtime.borrow_mut();
            for _ in 0..MAXIMUM_EVENTS_PER_FRAME {
                let Some(event) = runtime.poll_event() else {
                    break;
                };
                apply_event(&window, event);
            }
            synchronize_controls(&window, &runtime);
        },
    );
    timer
}

pub fn synchronize_controls(window: &AppWindow, runtime: &ApplicationRuntime) {
    let state = runtime.state();
    let repository = window.get_repository().to_string();
    let revision = window.get_revision().to_string();
    window.set_busy(state.activity() != ApplicationActivity::Idle);
    window.set_can_resolve(state.can_resolve());
    window.set_can_load(state.can_load(&repository, &revision));
    window.set_can_unload(state.can_unload());
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

fn apply_event(window: &AppWindow, event: ApplicationEvent) {
    match event {
        ApplicationEvent::ModelResolved {
            model,
            persistence_warning,
        } => {
            window.set_resolved_commit(model.commit.clone().into());
            let scalar = model.scalar_type.map_or("unknown", scalar_type_name);
            let message = match persistence_warning {
                Some(warning) => format!(
                    "Artifacts and tokenizer ({} tokens, {scalar}) are ready; catalogue \
                     persistence failed: {warning}",
                    model.vocabulary_size,
                ),
                None => format!(
                    "Artifacts and tokenizer ({} tokens, {scalar}) are ready for CPU loading.",
                    model.vocabulary_size,
                ),
            };
            window.set_status_text(message.into());
        }
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
            window.set_status_text(format!("Generation {} started.", request_id.get()).into());
        }
        ApplicationEvent::GenerationCancellationRequested { request_id } => {
            window.set_status_text(
                format!(
                    "Cancellation requested for generation {}.",
                    request_id.get()
                )
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
                    "Generation {} finished: {:?}",
                    terminal.request_id.get(),
                    terminal.outcome
                )
                .into(),
            );
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
                    "Model resources were unloaded after cancelling {cancelled_requests} active \
                     requests."
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
