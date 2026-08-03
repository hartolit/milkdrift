use application_runtime::{ApplicationActivity, ApplicationRuntime};

use super::devices::DeviceSelectorModel;
use super::model::{
    ComposerMode, composer_mode, loaded_model_summary, resolved_model_summary, selected_model,
    selected_model_summary,
};
use crate::AppWindow;

#[expect(
    clippy::struct_excessive_bools,
    reason = "each field is one authoritative E1 admission or lifecycle flag"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct RuntimeAdmissions {
    pub(super) can_select_device: bool,
    pub(super) can_resolve: bool,
    pub(super) can_load: bool,
    pub(super) can_submit_chat: bool,
    pub(super) can_start_generation: bool,
    pub(super) can_regenerate: bool,
    pub(super) can_clear: bool,
    pub(super) can_cancel: bool,
    pub(super) can_unload: bool,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "each field maps one independent Slint control-enablement property"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ControlState {
    pub(super) can_select_device: bool,
    pub(super) can_resolve: bool,
    pub(super) can_load: bool,
    pub(super) can_edit_message: bool,
    pub(super) can_submit_message: bool,
    pub(super) can_regenerate: bool,
    pub(super) can_clear: bool,
    pub(super) can_cancel: bool,
    pub(super) can_unload: bool,
}

pub(super) fn control_state(
    admissions: RuntimeAdmissions,
    mode: ComposerMode,
    message: &str,
) -> ControlState {
    let can_edit_message = match mode {
        ComposerMode::Unavailable => false,
        ComposerMode::Chat => admissions.can_submit_chat,
        ComposerMode::DirectCompletion => admissions.can_start_generation,
    };
    ControlState {
        can_select_device: admissions.can_select_device,
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

pub(super) fn synchronize(
    window: &AppWindow,
    runtime: &ApplicationRuntime,
    device_selector: &mut DeviceSelectorModel,
) {
    let state = runtime.state();
    let selection = selected_model(window);
    let can_resolve = state.can_resolve(&selection);
    let can_load = state.can_load(&selection);
    let mode = composer_mode(runtime);
    let message = window.get_message_input().to_string();
    let controls = control_state(
        RuntimeAdmissions {
            can_select_device: state.can_select_device(),
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

    device_selector.synchronize(state.devices());
    window.set_selected_device_index(device_selector.selected_index(state.selected_device()));

    window.set_busy(state.activity() != ApplicationActivity::Idle);
    window.set_can_edit_selection(
        state.activity() == ApplicationActivity::Idle && state.loaded().is_none(),
    );
    window.set_can_select_device(controls.can_select_device);
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

    window.set_selected_model_summary(
        selected_model_summary(
            &selection,
            state.selected_device(),
            state.selected_device_available(),
        )
        .into(),
    );
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
