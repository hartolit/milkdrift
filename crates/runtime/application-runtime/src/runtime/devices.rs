//! Application-owned device catalogue, selection, and load-time refresh policy.

use crate::local::{DeviceProbe, DeviceProbeFailure};
use crate::support::{storage_failure, stored_settings};
use crate::{
    ApplicationDevice, ApplicationDeviceDiscoveryFailure, ApplicationDeviceSummary,
    ApplicationDeviceUnavailableReason, ApplicationError, ApplicationRuntime,
};

pub(super) struct DeviceCatalogue {
    pub(super) summaries: Vec<ApplicationDeviceSummary>,
    pub(super) failures: Vec<ApplicationDeviceDiscoveryFailure>,
}

pub(super) fn discover_device_catalogue(
    selected_device: ApplicationDevice,
    device_probe: DeviceProbe,
) -> DeviceCatalogue {
    let mut summaries = vec![ApplicationDeviceSummary::cpu()];
    let mut failures = Vec::new();
    let mut cuda_devices = vec![ApplicationDevice::Cuda { ordinal: 0 }];
    if matches!(selected_device, ApplicationDevice::Cuda { .. })
        && selected_device != (ApplicationDevice::Cuda { ordinal: 0 })
    {
        cuda_devices.push(selected_device);
    }

    for device in cuda_devices {
        let (summary, failure) = probe_device(device, device_probe);
        if summary.available() || device == selected_device {
            summaries.push(summary);
        }
        if let Some(failure) = failure {
            failures.push(failure);
        }
    }
    summaries.sort_by_key(ApplicationDeviceSummary::device);
    DeviceCatalogue {
        summaries,
        failures,
    }
}

fn probe_device(
    device: ApplicationDevice,
    device_probe: DeviceProbe,
) -> (
    ApplicationDeviceSummary,
    Option<ApplicationDeviceDiscoveryFailure>,
) {
    if device == ApplicationDevice::Cpu {
        return (ApplicationDeviceSummary::cpu(), None);
    }
    match device_probe(device) {
        Ok(summary) => (summary, None),
        #[cfg(not(feature = "cuda"))]
        Err(DeviceProbeFailure::SupportNotCompiled) => (
            ApplicationDeviceSummary::unavailable(
                device,
                ApplicationDeviceUnavailableReason::SupportNotCompiled,
            ),
            None,
        ),
        Err(DeviceProbeFailure::Discovery(failure)) => (
            ApplicationDeviceSummary::unavailable(
                device,
                ApplicationDeviceUnavailableReason::DiscoveryFailed,
            ),
            Some(failure),
        ),
    }
}

impl ApplicationRuntime {
    /// Selects the exact device used by the next model load and persists that intent.
    ///
    /// An unavailable CUDA choice remains selected and is never replaced with CPU.
    ///
    /// # Errors
    ///
    /// Returns an error when model or generation ownership locks selection, the device is not in
    /// the bounded catalogue, or persistence fails.
    pub fn select_device(&mut self, device: ApplicationDevice) -> Result<(), ApplicationError> {
        if !self.state.can_select_device() {
            return Err(ApplicationError::DeviceSelectionLocked);
        }
        if !self
            .state
            .devices()
            .iter()
            .any(|summary| summary.device() == device)
        {
            return Err(ApplicationError::DeviceNotInCatalogue(device));
        }

        let (summary, failure) = probe_device(device, self.device_probe);
        let mut candidate = self.preferences.clone();
        candidate.selected_device = device;
        self.storage
            .save_settings(&stored_settings(&candidate))
            .map_err(storage_failure)?;

        self.preferences = candidate;
        self.state.replace_device_summary(summary, failure);
        self.state.set_selected_device(device);
        Ok(())
    }

    pub(super) fn refresh_selected_device(&mut self) -> Result<(), ApplicationError> {
        let selected_device = self.state.selected_device();
        let (summary, failure) = probe_device(selected_device, self.device_probe);
        let unavailable_reason = summary.unavailable_reason();
        self.state.replace_device_summary(summary, failure);
        if let Some(reason) = unavailable_reason {
            return Err(ApplicationError::SelectedDeviceUnavailable {
                device: selected_device,
                reason,
            });
        }
        if !self.state.selected_device_memory_budget_available() {
            return Err(ApplicationError::SelectedDeviceMemoryBudgetUnavailable {
                device: selected_device,
                budget_bytes: self.memory_budget.device_bytes,
                total_memory_bytes: self
                    .state
                    .selected_device_summary()
                    .and_then(ApplicationDeviceSummary::total_memory_bytes),
            });
        }
        Ok(())
    }
}
