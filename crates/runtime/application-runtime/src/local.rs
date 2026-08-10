//! Private concrete composition for the local Candle E0 worker.

use candle_backend::{CandleDeviceSummary, CandleLlamaLoader, CandleLlamaSource};
use domain_contracts::{
    BackendFailureKind, BackendId, DeviceId, DeviceKind, ExecutionDevice, LoadError,
};
use host_runtime::{OutputPullError, TokenOutputBatch};
use inference_runtime::{
    GenerationOutputState, HostedRuntime, HostedRuntimeConfiguration, RuntimeCommand, RuntimeEvent,
    RuntimeLimits, RuntimeReceiveError, RuntimeThread, start_hosted_runtime,
};

use crate::{
    ApplicationComputeCapability, ApplicationDevice, ApplicationDeviceDiscoveryFailure,
    ApplicationDeviceDiscoveryFailureKind, ApplicationDeviceSummary, ApplicationError,
    ApplicationFailure, ApplicationFailureKind,
};

pub const CANDLE_BACKEND_ID: BackendId = BackendId::new(1);

pub(crate) type DeviceProbe = fn(ApplicationDevice) -> DeviceProbeResult;
pub(crate) type DeviceProbeResult = Result<ApplicationDeviceSummary, DeviceProbeFailure>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeviceProbeFailure {
    #[cfg(not(feature = "cuda"))]
    SupportNotCompiled,
    Discovery(ApplicationDeviceDiscoveryFailure),
}

pub(crate) fn probe_application_device(device: ApplicationDevice) -> DeviceProbeResult {
    match device {
        ApplicationDevice::Cpu => CandleLlamaLoader::new(CANDLE_BACKEND_ID)
            .discover_device(execution_device(device))
            .map_err(|error| discovery_failure(device, error))
            .and_then(|summary| translate_device_summary(device, &summary)),
        ApplicationDevice::Cuda { ordinal } => probe_cuda_device(ordinal),
    }
}

#[cfg(feature = "cuda")]
fn probe_cuda_device(ordinal: u32) -> DeviceProbeResult {
    let device = ApplicationDevice::Cuda { ordinal };
    let execution_device = execution_device(device);
    CandleLlamaLoader::new(CANDLE_BACKEND_ID)
        .discover_device(execution_device)
        .map_err(|error| discovery_failure(device, error))
        .and_then(|summary| translate_device_summary(device, &summary))
}

#[cfg(not(feature = "cuda"))]
const fn probe_cuda_device(_ordinal: u32) -> DeviceProbeResult {
    Err(DeviceProbeFailure::SupportNotCompiled)
}

pub(crate) fn execution_device(device: ApplicationDevice) -> ExecutionDevice {
    match device {
        ApplicationDevice::Cpu => ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu),
        ApplicationDevice::Cuda { ordinal } => {
            ExecutionDevice::new(DeviceId::new(u64::from(ordinal)), DeviceKind::Cuda)
        }
    }
}

pub(crate) fn application_device(device: ExecutionDevice) -> Option<ApplicationDevice> {
    match device.kind {
        DeviceKind::Cpu if device.id.get() == 0 => Some(ApplicationDevice::Cpu),
        DeviceKind::Cuda => u32::try_from(device.id.get())
            .ok()
            .map(|ordinal| ApplicationDevice::Cuda { ordinal }),
        _ => None,
    }
}

fn translate_device_summary(
    expected: ApplicationDevice,
    summary: &CandleDeviceSummary,
) -> DeviceProbeResult {
    if application_device(summary.execution_device) != Some(expected)
        || summary.ordinal
            != match expected {
                ApplicationDevice::Cpu => None,
                ApplicationDevice::Cuda { ordinal } => Some(u64::from(ordinal)),
            }
    {
        return Err(DeviceProbeFailure::Discovery(
            ApplicationDeviceDiscoveryFailure::new(
                expected,
                ApplicationDeviceDiscoveryFailureKind::Other,
                "device discovery returned inconsistent identity facts".to_owned(),
            ),
        ));
    }

    let display_name = summary
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    Ok(ApplicationDeviceSummary::discovered(
        expected,
        display_name,
        summary.total_memory_bytes,
        summary.available_memory_bytes,
        summary
            .compute_capability
            .map(|capability| ApplicationComputeCapability {
                major: capability.major,
                minor: capability.minor,
            }),
    ))
}

fn discovery_failure(device: ApplicationDevice, error: LoadError) -> DeviceProbeFailure {
    let kind = match error {
        LoadError::InvalidConfiguration => {
            ApplicationDeviceDiscoveryFailureKind::InvalidConfiguration
        }
        LoadError::Backend(failure) => match failure.kind {
            BackendFailureKind::Unsupported => ApplicationDeviceDiscoveryFailureKind::Unsupported,
            BackendFailureKind::DeviceInitialization => {
                ApplicationDeviceDiscoveryFailureKind::Initialization
            }
            _ => ApplicationDeviceDiscoveryFailureKind::Other,
        },
        _ => ApplicationDeviceDiscoveryFailureKind::Other,
    };
    DeviceProbeFailure::Discovery(ApplicationDeviceDiscoveryFailure::new(
        device,
        kind,
        format!("device discovery failed: {error:?}"),
    ))
}

/// Private owner of the concrete, monomorphized Candle E0 endpoint.
pub struct LocalInference {
    runtime: HostedRuntime<CandleLlamaSource>,
    thread: Option<RuntimeThread>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalSubmitError {
    Full,
    Disconnected,
}

impl LocalInference {
    pub(crate) fn start(
        limits: RuntimeLimits,
        hosted: HostedRuntimeConfiguration,
    ) -> Result<Self, ApplicationError> {
        let (runtime, thread) =
            start_hosted_runtime(CandleLlamaLoader::new(CANDLE_BACKEND_ID), limits, hosted)
                .map_err(worker_start_failure)?;
        Ok(Self {
            runtime,
            thread: Some(thread),
        })
    }

    pub(crate) fn submit(
        &self,
        command: RuntimeCommand<CandleLlamaSource>,
    ) -> Result<(), LocalSubmitError> {
        match self.runtime.try_submit(command) {
            Ok(()) => Ok(()),
            Err(inference_runtime::RuntimeSubmitError::Full(_)) => Err(LocalSubmitError::Full),
            Err(inference_runtime::RuntimeSubmitError::Disconnected(_)) => {
                Err(LocalSubmitError::Disconnected)
            }
        }
    }

    pub(crate) fn try_receive(&self) -> Result<RuntimeEvent, RuntimeReceiveError> {
        self.runtime.try_receive()
    }

    #[cfg(test)]
    pub(crate) fn receive_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<RuntimeEvent, RuntimeReceiveError> {
        self.runtime.receive_timeout(timeout)
    }

    pub(crate) fn pull_token_output<R, F>(&self, consume: F) -> Result<R, OutputPullError>
    where
        F: for<'batch> FnOnce(TokenOutputBatch<'batch, GenerationOutputState>) -> R,
    {
        self.runtime.pull_token_output(consume)
    }

    pub(crate) const fn runtime(&self) -> &HostedRuntime<CandleLlamaSource> {
        &self.runtime
    }

    pub(crate) const fn thread_slot(&mut self) -> &mut Option<RuntimeThread> {
        &mut self.thread
    }

    #[cfg(test)]
    pub(crate) const fn take_thread(&mut self) -> Option<RuntimeThread> {
        self.thread.take()
    }

    pub(crate) const fn thread_is_present(&self) -> bool {
        self.thread.is_some()
    }
}

fn worker_start_failure(error: inference_runtime::HostedRuntimeStartError) -> ApplicationError {
    ApplicationFailure::new(ApplicationFailureKind::Worker, error).into()
}

#[cfg(test)]
mod tests {
    use candle_backend::{CandleDeviceSummary, CudaComputeCapability};
    use domain_contracts::{DeviceId, DeviceKind, ExecutionDevice};

    use super::{application_device, execution_device, translate_device_summary};
    use crate::{ApplicationDevice, ApplicationDeviceSummary};

    #[test]
    fn application_device_identity_round_trips_without_vendor_types() {
        for device in [
            ApplicationDevice::Cpu,
            ApplicationDevice::Cuda { ordinal: 7 },
        ] {
            assert_eq!(application_device(execution_device(device)), Some(device));
        }
        assert_eq!(
            application_device(ExecutionDevice::new(DeviceId::new(1), DeviceKind::Cpu)),
            None
        );
        assert_eq!(
            application_device(ExecutionDevice::new(DeviceId::new(0), DeviceKind::Metal)),
            None
        );
    }

    #[test]
    fn cuda_summary_translation_produces_application_owned_facts() -> Result<(), String> {
        let device = ApplicationDevice::Cuda { ordinal: 3 };
        let translated: ApplicationDeviceSummary = translate_device_summary(
            device,
            &CandleDeviceSummary {
                execution_device: execution_device(device),
                ordinal: Some(3),
                display_name: Some("NVIDIA Test Device".to_owned()),
                compute_capability: Some(CudaComputeCapability {
                    major: 12,
                    minor: 0,
                }),
                total_memory_bytes: Some(16_000),
                available_memory_bytes: Some(12_000),
                supports_bf16: true,
            },
        )
        .map_err(|failure| format!("summary translation failed: {failure:?}"))?;

        assert_eq!(translated.device(), device);
        assert_eq!(translated.display_name(), Some("NVIDIA Test Device"));
        assert!(translated.available());
        assert_eq!(translated.total_memory_bytes(), Some(16_000));
        assert_eq!(translated.available_memory_bytes(), Some(12_000));
        assert_eq!(
            translated.compute_capability(),
            Some(crate::ApplicationComputeCapability {
                major: 12,
                minor: 0
            })
        );
        Ok(())
    }

    #[test]
    fn cpu_summary_needs_no_discovered_display_name() {
        let cpu = ApplicationDeviceSummary::cpu();
        assert_eq!(cpu.device(), ApplicationDevice::Cpu);
        assert_eq!(cpu.display_name(), None);
        assert!(cpu.available());
    }
}
