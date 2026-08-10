//! Safe host/CUDA resource observation and exact external-runner device validation.

mod device;
mod environment;
mod resources;

use application_runtime::{ApplicationDevice, ApplicationRuntime};

use self::device::DeviceState;
use self::resources::ResourceState;
use super::cli::RequestedDevice;
use super::report::{
    CudaDeviceMetadata, CudaEnvironmentMetadata, ExecutionMetadata, ResourceCheckpoint,
};
use crate::error::BenchmarkResult;
use crate::report::DeviceIdentity;

pub(super) use resources::{
    CycleStabilityObservation, record_owner_drop, stability_after_unload, summarize_stability,
    validate_pre_load_checkpoint,
};

pub(super) struct DeviceObserver {
    device: DeviceState,
    resources: ResourceState,
}

impl DeviceObserver {
    pub(super) fn new(requested: RequestedDevice) -> BenchmarkResult<Self> {
        if requested == RequestedDevice::Cuda0 {
            environment::validate_cuda_build_configuration()?;
        }
        Ok(Self {
            device: DeviceState::new(requested)?,
            resources: ResourceState::default(),
        })
    }

    pub(super) fn begin_cycle(&mut self) {
        self.resources.begin_cycle();
    }

    pub(super) fn requested_identity(&self) -> DeviceIdentity {
        self.device.requested_identity()
    }

    pub(super) fn cuda_device_metadata(&self) -> Option<CudaDeviceMetadata> {
        self.device.cuda_device_metadata()
    }

    pub(super) fn capture(&self, checkpoint: &'static str) -> BenchmarkResult<ResourceCheckpoint> {
        self.resources.capture(&self.device, checkpoint)
    }

    pub(super) fn capture_pre_load(
        &mut self,
        checkpoint: &'static str,
    ) -> BenchmarkResult<ResourceCheckpoint> {
        self.resources.capture_pre_load(&self.device, checkpoint)
    }

    pub(super) fn validate_selected_e1(&self, runtime: &ApplicationRuntime) -> BenchmarkResult {
        self.device.validate_selected_e1(runtime)
    }

    pub(super) fn validate_actual_loaded(&self, actual: ApplicationDevice) -> BenchmarkResult {
        self.device.validate_actual_loaded(actual)
    }

    pub(super) fn execution_metadata(
        &self,
        actual_execution_scalar: &'static str,
    ) -> ExecutionMetadata {
        ExecutionMetadata {
            cuda_enabled: cfg!(feature = "cuda"),
            requested_device: self.requested_identity(),
            cuda_device: self.cuda_device_metadata(),
            actual_execution_scalar,
        }
    }

    pub(super) fn collect_cuda_environment(
        &self,
    ) -> BenchmarkResult<Option<CudaEnvironmentMetadata>> {
        environment::collect_cuda_environment(&self.device)
    }
}

#[cfg(test)]
mod tests {
    use application_runtime::ApplicationDevice;
    use candle_backend::{CandleDeviceSummary, CudaComputeCapability as CandleComputeCapability};
    use domain_contracts::DeviceKind;

    use super::DeviceObserver;
    use super::device::{
        CPU_EXECUTION_DEVICE, CUDA_ZERO_EXECUTION_DEVICE, DeviceState, REQUIRED_CUDA_NAME,
        ValidatedCudaProbe,
    };
    use super::resources::ResourceState;
    use crate::external::cli::RequestedDevice;
    use crate::external::report::{CudaComputeCapability, CudaDeviceMetadata};
    use crate::report::DeviceIdentity;

    fn valid_probe() -> Result<ValidatedCudaProbe, String> {
        ValidatedCudaProbe::from_summary(&CandleDeviceSummary {
            execution_device: CUDA_ZERO_EXECUTION_DEVICE,
            ordinal: Some(0),
            display_name: Some(REQUIRED_CUDA_NAME.to_owned()),
            compute_capability: Some(CandleComputeCapability {
                major: 12,
                minor: 0,
            }),
            total_memory_bytes: Some(16_000),
            available_memory_bytes: Some(12_000),
            supports_bf16: true,
        })
        .map_err(|error| error.to_string())
    }

    fn synthetic_cuda_observer() -> Result<DeviceObserver, String> {
        Ok(DeviceObserver {
            device: DeviceState::from_validated_probe(valid_probe()?),
            resources: ResourceState::default(),
        })
    }

    #[test]
    fn cpu_observer_records_only_the_explicit_cpu_identity() -> Result<(), String> {
        let observer =
            DeviceObserver::new(RequestedDevice::Cpu).map_err(|error| error.to_string())?;
        assert_eq!(
            observer.requested_identity(),
            DeviceIdentity {
                kind: "cpu",
                id: 0,
                ordinal: None,
            }
        );
        assert_eq!(CPU_EXECUTION_DEVICE.id.get(), 0);
        assert_eq!(CPU_EXECUTION_DEVICE.kind, DeviceKind::Cpu);
        assert_eq!(observer.cuda_device_metadata(), None);

        let execution = observer.execution_metadata("F32");
        assert_eq!(execution.cuda_enabled, cfg!(feature = "cuda"));
        assert_eq!(execution.requested_device, observer.requested_identity());
        assert_eq!(execution.cuda_device, None);
        assert_eq!(execution.actual_execution_scalar, "F32");
        assert_eq!(
            observer
                .collect_cuda_environment()
                .map_err(|error| error.to_string())?,
            None
        );
        observer
            .validate_actual_loaded(ApplicationDevice::Cpu)
            .map_err(|error| error.to_string())?;
        assert!(
            observer
                .validate_actual_loaded(ApplicationDevice::Cuda { ordinal: 0 })
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn cuda_observer_reports_exact_identity_and_static_metadata_without_hardware()
    -> Result<(), String> {
        let observer = synthetic_cuda_observer()?;
        assert_eq!(
            observer.requested_identity(),
            DeviceIdentity {
                kind: "cuda",
                id: 0,
                ordinal: Some(0),
            }
        );
        assert_eq!(
            observer.cuda_device_metadata(),
            Some(CudaDeviceMetadata {
                name: REQUIRED_CUDA_NAME.to_owned(),
                compute_capability: CudaComputeCapability {
                    major: 12,
                    minor: 0,
                },
                total_memory_bytes: 16_000,
            })
        );

        let execution = observer.execution_metadata("BF16");
        assert_eq!(execution.cuda_enabled, cfg!(feature = "cuda"));
        assert_eq!(execution.actual_execution_scalar, "BF16");
        observer
            .validate_actual_loaded(ApplicationDevice::Cuda { ordinal: 0 })
            .map_err(|error| error.to_string())?;
        assert!(
            observer
                .validate_actual_loaded(ApplicationDevice::Cpu)
                .is_err()
        );
        Ok(())
    }
}
