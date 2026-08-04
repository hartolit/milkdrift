//! Safe host/CUDA resource observation and exact external-runner device validation.

mod device;
mod environment;
mod resources;

use application_runtime::{ApplicationDevice, ApplicationRuntime};

use self::device::{CUDA_LOGITS_TO_HOST_LIMITATION, DeviceState, DiscoveryCounter};
use self::resources::{CUDA_MEMORY_OBSERVATION_SCOPE, ResourceState};
use super::cli::RequestedDevice;
use super::report::{
    CudaContextObservation, CudaDeviceMetadata, CudaEnvironmentMetadata, DeviceIdentity,
    ExecutionMetadata, ResourceCheckpoint,
};
use crate::error::BenchmarkResult;

pub(super) use resources::{
    CycleStabilityObservation, record_owner_drop, stability_after_unload, summarize_stability,
    validate_pre_load_checkpoint,
};

pub(super) struct DeviceObserver {
    device: DeviceState,
    resources: ResourceState,
    discovery_counter: DiscoveryCounter,
}

impl DeviceObserver {
    pub(super) fn new(requested: RequestedDevice) -> BenchmarkResult<Self> {
        if requested == RequestedDevice::Cuda0 {
            environment::validate_cuda_build_configuration()?;
        }
        let discovery_counter = DiscoveryCounter::new();
        let device = DeviceState::new(requested, &discovery_counter)?;
        Ok(Self {
            device,
            resources: ResourceState::default(),
            discovery_counter,
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
        self.resources
            .capture(&self.device, &self.discovery_counter, checkpoint)
    }

    pub(super) fn capture_pre_load(
        &mut self,
        checkpoint: &'static str,
    ) -> BenchmarkResult<ResourceCheckpoint> {
        self.resources
            .capture_pre_load(&self.device, &self.discovery_counter, checkpoint)
    }

    pub(super) fn validate_selected_e1(&self, runtime: &ApplicationRuntime) -> BenchmarkResult {
        self.device
            .validate_selected_e1(runtime, &self.discovery_counter)
    }

    pub(super) fn validate_actual_loaded(&self, actual: ApplicationDevice) -> BenchmarkResult {
        self.device.validate_actual_loaded(actual)
    }

    pub(super) fn execution_metadata(&self, execution_scalar: &'static str) -> ExecutionMetadata {
        let cuda_enabled = cfg!(feature = "cuda");
        let cuda_requested = self.device.requested() == RequestedDevice::Cuda0;
        ExecutionMetadata {
            cuda_enabled,
            requested_device: self.requested_identity(),
            cuda_device: self.cuda_device_metadata(),
            execution_scalar,
            host_sampling: true,
            cuda_logits_to_host_limitation: cuda_requested
                .then_some(CUDA_LOGITS_TO_HOST_LIMITATION),
            cuda_memory_observation_scope: cuda_requested.then_some(CUDA_MEMORY_OBSERVATION_SCOPE),
            cuda_context_observation: cuda_requested.then_some(CudaContextObservation {
                device_discovery_calls: self.cuda_discovery_count(),
                initialization_scope: device::CUDA_CONTEXT_INITIALIZATION_SCOPE,
            }),
        }
    }

    pub(super) fn collect_cuda_environment(
        &self,
    ) -> BenchmarkResult<Option<CudaEnvironmentMetadata>> {
        environment::collect_cuda_environment(&self.device, &self.discovery_counter)
    }

    pub(super) fn cuda_discovery_count(&self) -> u64 {
        self.discovery_counter.count()
    }
}

#[cfg(test)]
mod tests {
    use application_runtime::ApplicationDevice;
    use candle_backend::{CandleDeviceSummary, CudaComputeCapability as CandleComputeCapability};

    use super::DeviceObserver;
    use super::device::{
        CPU_EXECUTION_DEVICE, CUDA_LOGITS_TO_HOST_LIMITATION, CUDA_ZERO_EXECUTION_DEVICE,
        DeviceState, DiscoveryCounter, REQUIRED_CUDA_NAME, ValidatedCudaProbe,
    };
    use super::resources::{CUDA_MEMORY_OBSERVATION_SCOPE, ResourceState};
    use crate::external::cli::RequestedDevice;
    use crate::external::report::{CudaComputeCapability, CudaDeviceMetadata, DeviceIdentity};
    use domain_contracts::DeviceKind;

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
            discovery_counter: DiscoveryCounter::new(),
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
        assert_eq!(observer.cuda_discovery_count(), 0);

        let execution = observer.execution_metadata("F32");
        assert_eq!(execution.cuda_enabled, cfg!(feature = "cuda"));
        assert_eq!(
            execution.requested_device,
            DeviceIdentity {
                kind: "cpu",
                id: 0,
                ordinal: None,
            }
        );
        assert_eq!(execution.cuda_device, None);
        assert_eq!(execution.execution_scalar, "F32");
        assert!(execution.host_sampling);
        assert_eq!(execution.cuda_logits_to_host_limitation, None);
        assert_eq!(execution.cuda_memory_observation_scope, None);
        assert_eq!(execution.cuda_context_observation, None);
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
        assert_eq!(execution.execution_scalar, "BF16");
        assert_eq!(
            execution.cuda_logits_to_host_limitation,
            Some(CUDA_LOGITS_TO_HOST_LIMITATION)
        );
        assert_eq!(
            execution.cuda_memory_observation_scope,
            Some(CUDA_MEMORY_OBSERVATION_SCOPE)
        );
        let context = execution
            .cuda_context_observation
            .ok_or_else(|| "CUDA context observation disappeared".to_owned())?;
        assert_eq!(context.device_discovery_calls, 0);
        assert!(context.initialization_scope.contains("never per token"));
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

    #[test]
    fn discovery_count_bookkeeping_is_per_observer_and_hardware_free() -> Result<(), String> {
        let observer = synthetic_cuda_observer()?;
        assert_eq!(observer.cuda_discovery_count(), 0);
        observer
            .discovery_counter
            .record_call()
            .map_err(|error| error.to_string())?;
        observer
            .discovery_counter
            .record_call()
            .map_err(|error| error.to_string())?;
        assert_eq!(observer.cuda_discovery_count(), 2);

        let other = synthetic_cuda_observer()?;
        assert_eq!(other.cuda_discovery_count(), 0);
        Ok(())
    }

    #[test]
    fn cuda_execution_metadata_documents_host_sampling_boundary() {
        assert!(CUDA_LOGITS_TO_HOST_LIMITATION.contains("host F32"));
        assert!(CUDA_LOGITS_TO_HOST_LIMITATION.contains("GPU-side sampling"));
        assert!(CUDA_MEMORY_OBSERVATION_SCOPE.contains("whole device"));
        assert!(CUDA_MEMORY_OBSERVATION_SCOPE.contains("not process-attributed"));
    }
}
