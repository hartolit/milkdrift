//! Slint-specific presentation mapping over the reusable application runtime.

mod callbacks;
mod controls;
mod devices;
mod model;
mod output;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use application_runtime::ApplicationRuntime;
use slint::ComponentHandle;

use self::devices::DeviceSelectorModel;
use self::output::PresentationState;
use crate::AppWindow;

const UI_FRAME_MILLISECONDS: u64 = 16;
const MAXIMUM_EVENTS_PER_FRAME: usize = 64;

/// Owns presentation-only generation state and the Slint callback bindings.
pub struct Presenter {
    state: Rc<RefCell<PresentationState>>,
    device_selector: Rc<RefCell<DeviceSelectorModel>>,
}

impl Presenter {
    /// Connects UI intents to the frontend-neutral application runtime.
    pub fn connect(window: &AppWindow, runtime: &Rc<RefCell<ApplicationRuntime>>) -> Self {
        let state = Rc::new(RefCell::new(PresentationState::default()));
        let device_selector = Rc::new(RefCell::new(DeviceSelectorModel::new(
            runtime.borrow().state().devices(),
        )));
        window.set_device_model(device_selector.borrow().slint_model());
        callbacks::connect(window, runtime, &state, &device_selector);
        Self {
            state,
            device_selector,
        }
    }

    /// Synchronizes every control and summary from authoritative E1 state.
    pub fn synchronize_controls(&self, window: &AppWindow, runtime: &ApplicationRuntime) {
        synchronize_controls(window, runtime, &self.device_selector);
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
        let device_selector = Rc::clone(&self.device_selector);
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
                        refresh_conversation |=
                            callbacks::event_requires_conversation_snapshot(&event);
                        callbacks::apply_event(&window, event);
                    }

                    let presentation = presentation.borrow();
                    let displayed_request = presentation.displayed_request();
                    let refresh_conversation =
                        refresh_conversation && presentation.allows_conversation_snapshot();
                    drop(presentation);
                    let output = runtime_ref.pull_output(|batch| {
                        output::collect_output_batch(&batch, displayed_request)
                    });
                    (output, refresh_conversation)
                };

                match output {
                    Ok(delta) => {
                        let mut presentation = presentation.borrow_mut();
                        let mut update = presentation.apply_delta(delta);
                        if refresh_conversation {
                            update.output = None;
                        }
                        output::render_presentation_update(&window, &presentation, update);
                    }
                    Err(error) => {
                        window.set_status_text(
                            format!("Generated output pull failed: {error}").into(),
                        );
                    }
                }
                if refresh_conversation {
                    let runtime_ref = runtime.borrow();
                    output::synchronize_conversation(&window, &runtime_ref);
                }
                let runtime_ref = runtime.borrow();
                synchronize_controls(&window, &runtime_ref, &device_selector);
                output::synchronize_usage(
                    &window,
                    &runtime_ref,
                    presentation.borrow().displayed_request(),
                );
            },
        );
        timer
    }
}

/// Synchronizes every control, model summary, mode, and usage value from authoritative state.
fn synchronize_controls(
    window: &AppWindow,
    runtime: &ApplicationRuntime,
    device_selector: &Rc<RefCell<DeviceSelectorModel>>,
) {
    controls::synchronize(window, runtime, &mut device_selector.borrow_mut());
}
