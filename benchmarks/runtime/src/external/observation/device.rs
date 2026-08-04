//! Exact requested-device policy, CUDA matrix validation, and stable identity.

use std::cell::Cell;

use application_runtime::{ApplicationDevice, ApplicationRuntime};
use candle_backend::{CandleDeviceSummary, CandleLlamaLoader};
use domain_contracts::{BackendId, DeviceId, DeviceKind, ExecutionDevice};

use super::super::cli::RequestedDevice;
use super::super::report::{CudaComputeCapability, CudaDeviceMetadata, DeviceIdentity};
use crate::error::{BenchmarkError, BenchmarkResult};

const OBSERVATION_BACKEND: BackendId = BackendId::new(10_002);
pub(super) const CPU_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
pub(super) const CUDA_ZERO_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);
const CUDA_ZERO_APPLICATION_DEVICE: ApplicationDevice = ApplicationDevice::Cuda { ordinal: 0 };
pub(super) const REQUIRED_CUDA_NAME: &str = "NVIDIA GeForce RTX 5070 Ti";
const REQUIRED_CUDA_COMPUTE_CAPABILITY: CudaComputeCapability = CudaComputeCapability {
    major: 12,
    minor: 0,
};
pub(super) const CUDA_LOGITS_TO_HOST_LIMITATION: &str = "CUDA vocabulary logits are transferred to host F32 before sampling; GPU-side sampling is not implemented";
pub(super) const CUDA_CONTEXT_INITIALIZATION_SCOPE: &str = "each counted safe Candle discover_device call constructs a temporary Candle CUDA device and cudarc context at a cold identity or resource checkpoint; never per token; retaining a reusable context would require unsupported ownership exposure or a new lower-level benchmark dependency";

pub(super) struct DiscoveryCounter {
    calls: Cell<u64>,
}

impl DiscoveryCounter {
    pub(super) const fn new() -> Self {
        Self {
            calls: Cell::new(0),
        }
    }

    pub(super) fn record_call(&self) -> BenchmarkResult {
        let calls =
            self.calls.get().checked_add(1).ok_or_else(|| {
                BenchmarkError::new("CUDA device-discovery call count overflowed")
            })?;
        self.calls.set(calls);
        Ok(())
    }

    pub(super) fn count(&self) -> u64 {
        self.calls.get()
    }
}

pub(super) struct DeviceState {
    requested: RequestedDevice,
    cuda: Option<CudaState>,
}

struct CudaState {
    metadata: CudaDeviceMetadata,
    supports_bf16: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ValidatedCudaProbe {
    pub(super) name: String,
    pub(super) compute_capability: CudaComputeCapability,
    pub(super) total_bytes: u64,
    pub(super) free_bytes: u64,
    pub(super) supports_bf16: bool,
}

impl DeviceState {
    pub(super) fn new(
        requested: RequestedDevice,
        discovery_counter: &DiscoveryCounter,
    ) -> BenchmarkResult<Self> {
        let cuda = match requested {
            RequestedDevice::Cpu => None,
            RequestedDevice::Cuda0 => Some(CudaState::new(discover_cuda_zero(discovery_counter)?)),
        };
        Ok(Self { requested, cuda })
    }

    #[cfg(test)]
    pub(super) fn from_validated_probe(probe: ValidatedCudaProbe) -> Self {
        Self {
            requested: RequestedDevice::Cuda0,
            cuda: Some(CudaState::new(probe)),
        }
    }

    pub(super) const fn requested(&self) -> RequestedDevice {
        self.requested
    }

    pub(super) fn requested_identity(&self) -> DeviceIdentity {
        let execution_device = requested_execution_device(self.requested);
        DeviceIdentity {
            kind: match self.requested {
                RequestedDevice::Cpu => "cpu",
                RequestedDevice::Cuda0 => "cuda",
            },
            id: execution_device.id.get(),
            ordinal: match self.requested {
                RequestedDevice::Cpu => None,
                RequestedDevice::Cuda0 => Some(0),
            },
        }
    }

    pub(super) fn cuda_device_metadata(&self) -> Option<CudaDeviceMetadata> {
        self.cuda.as_ref().map(|cuda| cuda.metadata.clone())
    }

    pub(super) fn validated_cuda_probe(
        &self,
        discovery_counter: &DiscoveryCounter,
    ) -> BenchmarkResult<ValidatedCudaProbe> {
        let cuda = self.cuda_state()?;
        let probe = discover_cuda_zero(discovery_counter)?;
        cuda.validate_stable(&probe)?;
        Ok(probe)
    }

    pub(super) fn validate_selected_e1(
        &self,
        runtime: &ApplicationRuntime,
        discovery_counter: &DiscoveryCounter,
    ) -> BenchmarkResult {
        let expected = requested_application_device(self.requested);
        let persisted = runtime.preferences().selected_device;
        if persisted != expected {
            return Err(BenchmarkError::new(format!(
                "external runner requested {expected:?}, but ApplicationRuntime preferences selected {persisted:?}; configure the exact requested device before startup and use the intended settings database"
            )));
        }

        let state = runtime.state();
        let selected = state.selected_device();
        if selected != expected {
            return Err(BenchmarkError::new(format!(
                "external runner requested {expected:?}, but E1 selected {selected:?}; the runner does not permit device substitution or CPU fallback"
            )));
        }
        let summary = state.selected_device_summary().ok_or_else(|| {
            BenchmarkError::new(format!(
                "E1 selected {expected:?} but published no summary for that exact device"
            ))
        })?;
        if summary.device() != expected {
            return Err(BenchmarkError::new(format!(
                "E1 selected-device summary reported {:?}, expected {expected:?}",
                summary.device()
            )));
        }
        if !summary.available() {
            return Err(BenchmarkError::new(format!(
                "E1 selected {expected:?}, but its latest bounded device probe marked it unavailable ({:?}); resolve CUDA feature, driver, or device initialization diagnostics before running the external workload",
                summary.unavailable_reason()
            )));
        }

        if self.requested == RequestedDevice::Cuda0 {
            let cuda = self.cuda_state()?;
            self.validated_cuda_probe(discovery_counter)?;
            validate_e1_cuda_summary(summary, cuda)?;
        }
        Ok(())
    }

    pub(super) fn validate_actual_loaded(&self, actual: ApplicationDevice) -> BenchmarkResult {
        let expected = requested_application_device(self.requested);
        if actual != expected {
            return Err(BenchmarkError::new(format!(
                "E0 load receipt reported actual device {actual:?}, expected the explicitly requested {expected:?}; refusing to record a substituted execution device"
            )));
        }
        Ok(())
    }

    fn cuda_state(&self) -> BenchmarkResult<&CudaState> {
        self.cuda.as_ref().ok_or_else(|| {
            BenchmarkError::new(
                "internal external-runner invariant failed: CUDA was requested but no validated CUDA device metadata is available",
            )
        })
    }
}

impl CudaState {
    fn new(probe: ValidatedCudaProbe) -> Self {
        Self {
            metadata: CudaDeviceMetadata {
                name: probe.name,
                compute_capability: probe.compute_capability,
                total_memory_bytes: probe.total_bytes,
            },
            supports_bf16: probe.supports_bf16,
        }
    }

    fn validate_stable(&self, probe: &ValidatedCudaProbe) -> BenchmarkResult {
        if probe.name != self.metadata.name {
            return Err(BenchmarkError::new(format!(
                "CUDA ordinal 0 device name changed across probes: initial {:?}, current {:?}; stop the run and verify stable CUDA device visibility",
                self.metadata.name, probe.name
            )));
        }
        if probe.total_bytes != self.metadata.total_memory_bytes {
            return Err(BenchmarkError::new(format!(
                "CUDA ordinal 0 total memory changed across probes: initial {} bytes, current {} bytes; stop the run and verify stable device identity",
                self.metadata.total_memory_bytes, probe.total_bytes
            )));
        }
        if probe.compute_capability != self.metadata.compute_capability {
            return Err(BenchmarkError::new(format!(
                "CUDA ordinal 0 compute capability changed across probes: initial {}.{}, current {}.{}; stop the run and verify stable device identity",
                self.metadata.compute_capability.major,
                self.metadata.compute_capability.minor,
                probe.compute_capability.major,
                probe.compute_capability.minor
            )));
        }
        if probe.supports_bf16 != self.supports_bf16 {
            return Err(BenchmarkError::new(format!(
                "CUDA ordinal 0 BF16 support changed across probes: initial {}, current {}; stop the run and verify stable Candle/CUDA device initialization",
                self.supports_bf16, probe.supports_bf16
            )));
        }
        Ok(())
    }
}

impl ValidatedCudaProbe {
    pub(super) fn from_summary(summary: &CandleDeviceSummary) -> BenchmarkResult<Self> {
        if summary.execution_device != CUDA_ZERO_EXECUTION_DEVICE {
            return Err(BenchmarkError::new(format!(
                "Candle CUDA discovery returned execution identity {:?} id {}, expected exactly CUDA id 0; the external runner does not permit ordinal substitution",
                summary.execution_device.kind,
                summary.execution_device.id.get()
            )));
        }
        if summary.ordinal != Some(0) {
            return Err(BenchmarkError::new(format!(
                "Candle CUDA discovery returned ordinal {:?}, expected exactly Some(0)",
                summary.ordinal
            )));
        }

        let name = summary
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                BenchmarkError::new(
                    "Candle CUDA discovery returned no nonempty device name for ordinal 0; verify the NVIDIA driver can identify the GPU",
                )
            })?;
        if name != REQUIRED_CUDA_NAME {
            return Err(BenchmarkError::new(format!(
                "CUDA ordinal 0 is {name:?}, but this executed matrix requires exactly {REQUIRED_CUDA_NAME:?}; select the required host/device rather than recording incomparable evidence"
            )));
        }

        let compute_capability = summary.compute_capability.ok_or_else(|| {
            BenchmarkError::new(
                "Candle CUDA discovery omitted compute capability for ordinal 0; verify driver device-attribute discovery",
            )
        })?;
        let compute_capability = CudaComputeCapability {
            major: compute_capability.major,
            minor: compute_capability.minor,
        };
        if compute_capability != REQUIRED_CUDA_COMPUTE_CAPABILITY {
            return Err(BenchmarkError::new(format!(
                "CUDA ordinal 0 compute capability is {}.{}, but this executed matrix requires exactly 12.0",
                compute_capability.major, compute_capability.minor
            )));
        }

        let total_bytes = summary.total_memory_bytes.ok_or_else(|| {
            BenchmarkError::new(
                "Candle CUDA discovery omitted total device memory for ordinal 0; the external runner cannot establish a resource baseline",
            )
        })?;
        let free_bytes = summary.available_memory_bytes.ok_or_else(|| {
            BenchmarkError::new(
                "Candle CUDA discovery omitted free device memory for ordinal 0; the external runner cannot establish a resource checkpoint",
            )
        })?;
        if free_bytes > total_bytes {
            return Err(BenchmarkError::new(format!(
                "Candle CUDA discovery reported free memory ({free_bytes} bytes) greater than total memory ({total_bytes} bytes) for ordinal 0"
            )));
        }
        if !summary.supports_bf16 {
            return Err(BenchmarkError::new(
                "CUDA ordinal 0 does not report BF16 support through Candle, but the required external CUDA matrix executes the BF16 source as BF16",
            ));
        }

        Ok(Self {
            name: name.to_owned(),
            compute_capability,
            total_bytes,
            free_bytes,
            supports_bf16: summary.supports_bf16,
        })
    }

    pub(super) fn used_bytes(&self) -> BenchmarkResult<u64> {
        self.total_bytes.checked_sub(self.free_bytes).ok_or_else(|| {
            BenchmarkError::new(format!(
                "CUDA ordinal 0 used-memory calculation underflowed: total {} bytes, free {} bytes",
                self.total_bytes, self.free_bytes
            ))
        })
    }
}

const fn requested_execution_device(requested: RequestedDevice) -> ExecutionDevice {
    match requested {
        RequestedDevice::Cpu => CPU_EXECUTION_DEVICE,
        RequestedDevice::Cuda0 => CUDA_ZERO_EXECUTION_DEVICE,
    }
}

const fn requested_application_device(requested: RequestedDevice) -> ApplicationDevice {
    match requested {
        RequestedDevice::Cpu => ApplicationDevice::Cpu,
        RequestedDevice::Cuda0 => CUDA_ZERO_APPLICATION_DEVICE,
    }
}

fn discover_cuda_zero(discovery_counter: &DiscoveryCounter) -> BenchmarkResult<ValidatedCudaProbe> {
    discovery_counter.record_call()?;
    let summary = CandleLlamaLoader::new(OBSERVATION_BACKEND)
        .discover_device(CUDA_ZERO_EXECUTION_DEVICE)
        .map_err(|error| {
            BenchmarkError::new(format!(
                "safe Candle CUDA ordinal 0 discovery failed ({error:?}); verify the runtime-benchmarks `cuda` feature, NVIDIA driver, CUDA visibility, and device availability"
            ))
        })?;
    ValidatedCudaProbe::from_summary(&summary)
}

fn validate_e1_cuda_summary(
    summary: &application_runtime::ApplicationDeviceSummary,
    cuda: &CudaState,
) -> BenchmarkResult {
    let total_bytes = summary.total_memory_bytes().ok_or_else(|| {
        BenchmarkError::new(
            "E1 selected CUDA ordinal 0 but its available summary omitted total device memory",
        )
    })?;
    if total_bytes != cuda.metadata.total_memory_bytes {
        return Err(BenchmarkError::new(format!(
            "E1 CUDA ordinal 0 total memory is {total_bytes} bytes, but the observer's stable adapter probe reported {} bytes",
            cuda.metadata.total_memory_bytes
        )));
    }
    let free_bytes = summary.available_memory_bytes().ok_or_else(|| {
        BenchmarkError::new(
            "E1 selected CUDA ordinal 0 but its available summary omitted free device memory",
        )
    })?;
    if free_bytes > total_bytes {
        return Err(BenchmarkError::new(format!(
            "E1 CUDA ordinal 0 summary reported free memory ({free_bytes} bytes) greater than total memory ({total_bytes} bytes)"
        )));
    }
    let compute_capability = summary.compute_capability().ok_or_else(|| {
        BenchmarkError::new(
            "E1 selected CUDA ordinal 0 but its available summary omitted compute capability",
        )
    })?;
    if compute_capability.major != REQUIRED_CUDA_COMPUTE_CAPABILITY.major
        || compute_capability.minor != REQUIRED_CUDA_COMPUTE_CAPABILITY.minor
    {
        return Err(BenchmarkError::new(format!(
            "E1 CUDA ordinal 0 compute capability is {}.{}, but the required executed matrix is 12.0",
            compute_capability.major, compute_capability.minor
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use candle_backend::{CandleDeviceSummary, CudaComputeCapability as CandleComputeCapability};
    use domain_contracts::{DeviceId, DeviceKind, ExecutionDevice};

    use super::{CUDA_ZERO_EXECUTION_DEVICE, CudaState, REQUIRED_CUDA_NAME, ValidatedCudaProbe};
    use crate::external::report::CudaComputeCapability;

    fn valid_cuda_summary() -> CandleDeviceSummary {
        CandleDeviceSummary {
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
        }
    }

    #[test]
    fn cuda_probe_validation_requires_exact_identity_and_matrix() -> Result<(), String> {
        let probe = ValidatedCudaProbe::from_summary(&valid_cuda_summary())
            .map_err(|error| error.to_string())?;
        assert_eq!(probe.name, REQUIRED_CUDA_NAME);
        assert_eq!(
            probe.compute_capability,
            CudaComputeCapability {
                major: 12,
                minor: 0
            }
        );
        assert_eq!(
            probe.used_bytes().map_err(|error| error.to_string())?,
            4_000
        );

        let mut wrong_identity = valid_cuda_summary();
        wrong_identity.execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
        assert!(ValidatedCudaProbe::from_summary(&wrong_identity).is_err());

        let mut wrong_ordinal = valid_cuda_summary();
        wrong_ordinal.ordinal = Some(1);
        assert!(ValidatedCudaProbe::from_summary(&wrong_ordinal).is_err());

        let mut impossible_memory = valid_cuda_summary();
        impossible_memory.available_memory_bytes = Some(16_001);
        assert!(ValidatedCudaProbe::from_summary(&impossible_memory).is_err());

        let mut no_bf16 = valid_cuda_summary();
        no_bf16.supports_bf16 = false;
        assert!(ValidatedCudaProbe::from_summary(&no_bf16).is_err());
        Ok(())
    }

    #[test]
    fn cuda_static_facts_must_remain_stable_across_probes() -> Result<(), String> {
        let probe = ValidatedCudaProbe::from_summary(&valid_cuda_summary())
            .map_err(|error| error.to_string())?;
        let state = CudaState::new(probe.clone());
        state
            .validate_stable(&probe)
            .map_err(|error| error.to_string())?;

        let mut changed_total = probe;
        changed_total.total_bytes += 1;
        let Err(error) = state.validate_stable(&changed_total) else {
            return Err("changed total memory unexpectedly passed".to_owned());
        };
        let error = error.to_string();
        assert!(error.contains("total memory changed"), "{error}");
        Ok(())
    }
}
