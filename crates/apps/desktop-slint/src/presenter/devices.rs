use std::rc::Rc;

use application_runtime::{ApplicationDevice, ApplicationDeviceSummary};
use slint::{Model, ModelRc, SharedString, VecModel};

use super::model::device_label;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DeviceChoice {
    device: ApplicationDevice,
    label: String,
    available: bool,
}

impl DeviceChoice {
    pub(super) fn new(
        device: ApplicationDevice,
        label: impl Into<String>,
        available: bool,
    ) -> Self {
        Self {
            device,
            label: label.into(),
            available,
        }
    }

    fn from_summary(summary: &ApplicationDeviceSummary) -> Self {
        Self::new(summary.device(), summary.label(), summary.available())
    }

    fn display_label(&self) -> SharedString {
        let runtime_label = self.label.trim();
        let mut label = if runtime_label.is_empty() {
            device_label(self.device)
        } else {
            runtime_label.to_owned()
        };
        if !self.available {
            label.push_str(" (unavailable)");
        }
        label.into()
    }
}

#[derive(Default)]
pub(super) struct DeviceSelectorModel {
    identities: Vec<ApplicationDevice>,
    labels: Rc<VecModel<SharedString>>,
}

impl DeviceSelectorModel {
    pub(super) fn new(summaries: &[ApplicationDeviceSummary]) -> Self {
        let mut model = Self::default();
        model.synchronize(summaries);
        model
    }

    pub(super) fn slint_model(&self) -> ModelRc<SharedString> {
        Rc::clone(&self.labels).into()
    }

    pub(super) fn synchronize(&mut self, summaries: &[ApplicationDeviceSummary]) {
        let choices = summaries
            .iter()
            .map(DeviceChoice::from_summary)
            .collect::<Vec<_>>();
        self.synchronize_choices(&choices);
    }

    pub(super) fn synchronize_choices(&mut self, choices: &[DeviceChoice]) {
        let identities = choices
            .iter()
            .map(|choice| choice.device)
            .collect::<Vec<_>>();
        let labels = choices
            .iter()
            .map(DeviceChoice::display_label)
            .collect::<Vec<_>>();

        if identities != self.identities || self.labels.row_count() != labels.len() {
            self.identities = identities;
            self.labels.set_vec(labels);
            return;
        }

        for (index, label) in labels.into_iter().enumerate() {
            if self.labels.row_data(index).as_ref() != Some(&label) {
                self.labels.set_row_data(index, label);
            }
        }
    }

    pub(super) fn device_at_checked_index(&self, index: i32) -> Option<ApplicationDevice> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.identities.get(index))
            .copied()
    }

    pub(super) fn selected_index(&self, device: ApplicationDevice) -> i32 {
        self.identities
            .iter()
            .position(|candidate| *candidate == device)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(-1)
    }
}
