//! Exact requested-device policy, context-free NVIDIA observation, and public E1 validation.

use std::process::Command;

use application_runtime::{
    ApplicationComputeCapability, ApplicationDevice, ApplicationDeviceSummary, ApplicationRuntime,
};

use super::super::cli::RequestedDevice;
use super::super::report::{CudaComputeCapability, CudaDeviceMetadata};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::evidence::application_device_record;
use crate::report::DeviceIdentity;

const CUDA_ZERO_APPLICATION_DEVICE: ApplicationDevice = ApplicationDevice::Cuda { ordinal: 0 };
pub(super) const REQUIRED_CUDA_NAME: &str = "NVIDIA GeForce RTX 5070 Ti";
const REQUIRED_CUDA_COMPUTE_CAPABILITY: CudaComputeCapability = CudaComputeCapability {
    major: 12,
    minor: 0,
};
const REQUIRED_CUDA_COMPUTE_CAPABILITY_TEXT: &str = "12.0";
const MEBIBYTE_BYTES: u64 = 1024 * 1024;
const NVIDIA_SMI_QUERY: &str =
    "--query-gpu=index,name,driver_version,compute_cap,memory.total,memory.free";
const NVIDIA_SMI_FORMAT: &str = "--format=csv,noheader,nounits";

pub(super) struct DeviceState {
    requested: RequestedDevice,
    cuda: Option<CudaState>,
}

struct CudaState {
    metadata: CudaDeviceMetadata,
    driver_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ValidatedCudaObservation {
    pub(super) name: String,
    pub(super) driver_version: String,
    pub(super) compute_capability: CudaComputeCapability,
    pub(super) total_bytes: u64,
    pub(super) free_bytes: u64,
}

impl DeviceState {
    pub(super) fn new(requested: RequestedDevice) -> BenchmarkResult<Self> {
        let cuda = match requested {
            RequestedDevice::Cpu => None,
            RequestedDevice::Cuda0 => Some(CudaState::new(observe_cuda_zero()?)),
        };
        Ok(Self { requested, cuda })
    }

    #[cfg(test)]
    pub(super) fn from_validated_observation(observation: ValidatedCudaObservation) -> Self {
        Self {
            requested: RequestedDevice::Cuda0,
            cuda: Some(CudaState::new(observation)),
        }
    }

    pub(super) const fn requested(&self) -> RequestedDevice {
        self.requested
    }

    pub(super) fn requested_identity(&self) -> DeviceIdentity {
        application_device_record(requested_application_device(self.requested))
    }

    pub(super) fn cuda_device_metadata(&self) -> Option<CudaDeviceMetadata> {
        self.cuda.as_ref().map(|cuda| cuda.metadata.clone())
    }

    pub(super) fn validated_cuda_observation(&self) -> BenchmarkResult<ValidatedCudaObservation> {
        let cuda = self.cuda_state()?;
        let observation = observe_cuda_zero()?;
        cuda.validate_stable(&observation)?;
        Ok(observation)
    }

    pub(super) fn validate_selected_e1(&self, runtime: &ApplicationRuntime) -> BenchmarkResult {
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
                "E1 selected {expected:?}, but its latest bounded device observation marked it unavailable ({:?}); resolve CUDA feature, driver, or device initialization diagnostics before running the external workload",
                summary.unavailable_reason()
            )));
        }
        if summary.unavailable_reason().is_some() {
            return Err(BenchmarkError::new(format!(
                "E1 selected {expected:?} as available but retained an unavailable reason"
            )));
        }

        match self.requested {
            RequestedDevice::Cpu => validate_e1_cpu_summary(summary),
            RequestedDevice::Cuda0 => validate_e1_cuda_summary(summary),
        }
    }

    pub(super) fn validate_actual_loaded(&self, actual: ApplicationDevice) -> BenchmarkResult {
        let expected = requested_application_device(self.requested);
        if actual != expected {
            return Err(BenchmarkError::new(format!(
                "public E1 loaded-model evidence reported actual device {actual:?}, expected the explicitly requested {expected:?}; refusing to record a substituted execution device"
            )));
        }
        Ok(())
    }

    fn cuda_state(&self) -> BenchmarkResult<&CudaState> {
        self.cuda.as_ref().ok_or_else(|| {
            BenchmarkError::new(
                "internal external-runner invariant failed: CUDA was requested but no validated nvidia-smi observation is available",
            )
        })
    }
}

impl CudaState {
    fn new(observation: ValidatedCudaObservation) -> Self {
        let ValidatedCudaObservation {
            name,
            driver_version,
            compute_capability,
            total_bytes,
            free_bytes: _,
        } = observation;
        Self {
            metadata: CudaDeviceMetadata {
                name,
                compute_capability,
                total_memory_bytes: total_bytes,
            },
            driver_version,
        }
    }

    fn validate_stable(&self, observation: &ValidatedCudaObservation) -> BenchmarkResult {
        if observation.name != self.metadata.name {
            return Err(BenchmarkError::new(format!(
                "physical GPU index 0 name changed across nvidia-smi observations: initial {:?}, current {:?}; stop the run and verify stable device identity",
                self.metadata.name, observation.name
            )));
        }
        if observation.driver_version != self.driver_version {
            return Err(BenchmarkError::new(format!(
                "physical GPU index 0 driver version changed across nvidia-smi observations: initial {:?}, current {:?}; stop the run and verify a stable driver environment",
                self.driver_version, observation.driver_version
            )));
        }
        if observation.compute_capability != self.metadata.compute_capability {
            return Err(BenchmarkError::new(format!(
                "physical GPU index 0 compute capability changed across nvidia-smi observations: initial {}.{}, current {}.{}; stop the run and verify stable device identity",
                self.metadata.compute_capability.major,
                self.metadata.compute_capability.minor,
                observation.compute_capability.major,
                observation.compute_capability.minor
            )));
        }
        if observation.total_bytes != self.metadata.total_memory_bytes {
            return Err(BenchmarkError::new(format!(
                "physical GPU index 0 total memory changed across nvidia-smi observations: initial {} bytes, current {} bytes; stop the run and verify stable device identity",
                self.metadata.total_memory_bytes, observation.total_bytes
            )));
        }
        Ok(())
    }
}

impl ValidatedCudaObservation {
    pub(super) fn used_bytes(&self) -> BenchmarkResult<u64> {
        self.total_bytes.checked_sub(self.free_bytes).ok_or_else(|| {
            BenchmarkError::new(format!(
                "physical GPU index 0 used-memory calculation underflowed: total {} bytes, free {} bytes",
                self.total_bytes, self.free_bytes
            ))
        })
    }
}

const fn requested_application_device(requested: RequestedDevice) -> ApplicationDevice {
    match requested {
        RequestedDevice::Cpu => ApplicationDevice::Cpu,
        RequestedDevice::Cuda0 => CUDA_ZERO_APPLICATION_DEVICE,
    }
}

fn observe_cuda_zero() -> BenchmarkResult<ValidatedCudaObservation> {
    let output = Command::new("nvidia-smi")
        .arg(NVIDIA_SMI_QUERY)
        .arg(NVIDIA_SMI_FORMAT)
        .output()
        .map_err(|error| {
            BenchmarkError::new(format!(
                "could not execute the fixed nvidia-smi whole-device query; ensure NVIDIA driver tools are installed and on PATH: {error}"
            ))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let stderr = if stderr.is_empty() {
            "no stderr output"
        } else {
            stderr
        };
        return Err(BenchmarkError::new(format!(
            "fixed nvidia-smi whole-device query exited with status {} ({stderr}); verify that the NVIDIA driver can query physical GPU index 0",
            output.status
        )));
    }
    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        BenchmarkError::new(format!(
            "fixed nvidia-smi whole-device query returned non-UTF-8 stdout; verify the NVIDIA driver installation: {error}"
        ))
    })?;
    parse_nvidia_smi_cuda_zero(&stdout)
}

fn parse_nvidia_smi_cuda_zero(output: &str) -> BenchmarkResult<ValidatedCudaObservation> {
    let mut selected = None;
    for (line_index, line) in output.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let line_number = line_index
            .checked_add(1)
            .ok_or_else(|| BenchmarkError::new("nvidia-smi output line number overflowed"))?;
        let mut fields = line.split(',').map(str::trim);
        let index = fields.next().ok_or_else(|| {
            BenchmarkError::new(format!(
                "nvidia-smi line {line_number} omitted the GPU index"
            ))
        })?;
        let name = fields.next().ok_or_else(|| {
            BenchmarkError::new(format!(
                "nvidia-smi line {line_number} omitted the GPU name"
            ))
        })?;
        let driver_version = fields.next().ok_or_else(|| {
            BenchmarkError::new(format!(
                "nvidia-smi line {line_number} omitted the driver version"
            ))
        })?;
        let compute_capability = fields.next().ok_or_else(|| {
            BenchmarkError::new(format!(
                "nvidia-smi line {line_number} omitted compute capability"
            ))
        })?;
        let total_mib = fields.next().ok_or_else(|| {
            BenchmarkError::new(format!(
                "nvidia-smi line {line_number} omitted total memory"
            ))
        })?;
        let free_mib = fields.next().ok_or_else(|| {
            BenchmarkError::new(format!("nvidia-smi line {line_number} omitted free memory"))
        })?;
        if fields.next().is_some() {
            return Err(BenchmarkError::new(format!(
                "nvidia-smi line {line_number} must contain exactly index, name, driver_version, compute_cap, memory.total, and memory.free"
            )));
        }

        let index = index.parse::<u32>().map_err(|error| {
            BenchmarkError::new(format!(
                "nvidia-smi line {line_number} has a nonnumeric GPU index: {error}"
            ))
        })?;
        if index != 0 {
            continue;
        }
        if selected.is_some() {
            return Err(BenchmarkError::new(
                "nvidia-smi output contains more than one row for physical GPU index 0",
            ));
        }

        selected = Some(validate_cuda_zero_fields(
            name,
            driver_version,
            compute_capability,
            total_mib,
            free_mib,
            line_number,
        )?);
    }
    selected.ok_or_else(|| {
        BenchmarkError::new(
            "fixed nvidia-smi whole-device query returned no row for physical GPU index 0",
        )
    })
}

fn validate_cuda_zero_fields(
    name: &str,
    driver_version: &str,
    compute_capability: &str,
    total_mib: &str,
    free_mib: &str,
    line_number: usize,
) -> BenchmarkResult<ValidatedCudaObservation> {
    if name != REQUIRED_CUDA_NAME {
        return Err(BenchmarkError::new(format!(
            "nvidia-smi identifies physical GPU index 0 as {name:?}, but the required executed matrix is exactly {REQUIRED_CUDA_NAME:?}"
        )));
    }
    if !is_dotted_decimal_version(driver_version) {
        return Err(BenchmarkError::new(format!(
            "nvidia-smi returned invalid or empty dotted driver version {driver_version:?} for physical GPU index 0"
        )));
    }
    if compute_capability != REQUIRED_CUDA_COMPUTE_CAPABILITY_TEXT {
        return Err(BenchmarkError::new(format!(
            "nvidia-smi returned compute capability {compute_capability:?} for physical GPU index 0, but the required executed matrix is exactly {REQUIRED_CUDA_COMPUTE_CAPABILITY_TEXT}"
        )));
    }

    let total_bytes = checked_mib_to_bytes(total_mib, "total", line_number)?;
    let free_bytes = checked_mib_to_bytes(free_mib, "free", line_number)?;
    if free_bytes > total_bytes {
        return Err(BenchmarkError::new(format!(
            "nvidia-smi reported free memory ({free_bytes} bytes) greater than total memory ({total_bytes} bytes) for physical GPU index 0"
        )));
    }

    Ok(ValidatedCudaObservation {
        name: name.to_owned(),
        driver_version: driver_version.to_owned(),
        compute_capability: REQUIRED_CUDA_COMPUTE_CAPABILITY,
        total_bytes,
        free_bytes,
    })
}

fn checked_mib_to_bytes(value: &str, field: &str, line_number: usize) -> BenchmarkResult<u64> {
    let mebibytes = value.parse::<u64>().map_err(|error| {
        BenchmarkError::new(format!(
            "nvidia-smi line {line_number} has nonnumeric {field} memory in MiB: {error}"
        ))
    })?;
    mebibytes.checked_mul(MEBIBYTE_BYTES).ok_or_else(|| {
        BenchmarkError::new(format!(
            "nvidia-smi line {line_number} {field} memory overflowed while converting {mebibytes} MiB to bytes"
        ))
    })
}

fn is_dotted_decimal_version(value: &str) -> bool {
    value.contains('.')
        && value.split('.').all(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn validate_e1_cpu_summary(summary: &ApplicationDeviceSummary) -> BenchmarkResult {
    if summary.display_name().is_some()
        || summary.total_memory_bytes().is_some()
        || summary.available_memory_bytes().is_some()
        || summary.compute_capability().is_some()
    {
        return Err(BenchmarkError::new(
            "E1 CPU summary unexpectedly published CUDA-only name, memory, or compute-capability fields",
        ));
    }
    Ok(())
}

fn validate_e1_cuda_summary(summary: &ApplicationDeviceSummary) -> BenchmarkResult {
    validate_e1_cuda_fields(
        summary.display_name(),
        summary.total_memory_bytes(),
        summary.available_memory_bytes(),
        summary.compute_capability(),
    )
}

fn validate_e1_cuda_fields(
    display_name: Option<&str>,
    total_bytes: Option<u64>,
    free_bytes: Option<u64>,
    compute_capability: Option<ApplicationComputeCapability>,
) -> BenchmarkResult {
    let name = display_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            BenchmarkError::new(
                "E1 selected CUDA ordinal 0 but its available public summary omitted a nonempty device name",
            )
        })?;
    if name != REQUIRED_CUDA_NAME {
        return Err(BenchmarkError::new(format!(
            "E1 selected CUDA ordinal 0 with public name {name:?}, but the required executed matrix is exactly {REQUIRED_CUDA_NAME:?}"
        )));
    }

    let total_bytes = total_bytes.ok_or_else(|| {
        BenchmarkError::new(
            "E1 selected CUDA ordinal 0 but its available public summary omitted total device memory",
        )
    })?;
    let free_bytes = free_bytes.ok_or_else(|| {
        BenchmarkError::new(
            "E1 selected CUDA ordinal 0 but its available public summary omitted free device memory",
        )
    })?;
    if free_bytes > total_bytes {
        return Err(BenchmarkError::new(format!(
            "E1 CUDA ordinal 0 public summary reported free memory ({free_bytes} bytes) greater than total memory ({total_bytes} bytes)"
        )));
    }

    let compute_capability = compute_capability.ok_or_else(|| {
        BenchmarkError::new(
            "E1 selected CUDA ordinal 0 but its available public summary omitted compute capability",
        )
    })?;
    if compute_capability.major != REQUIRED_CUDA_COMPUTE_CAPABILITY.major
        || compute_capability.minor != REQUIRED_CUDA_COMPUTE_CAPABILITY.minor
    {
        return Err(BenchmarkError::new(format!(
            "E1 CUDA ordinal 0 public summary reported compute capability {}.{}, but the required executed matrix is exactly 12.0",
            compute_capability.major, compute_capability.minor
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use application_runtime::ApplicationComputeCapability;

    use super::{
        CudaState, MEBIBYTE_BYTES, REQUIRED_CUDA_NAME, ValidatedCudaObservation,
        parse_nvidia_smi_cuda_zero, validate_e1_cuda_fields,
    };
    use crate::external::report::CudaComputeCapability;

    const VALID_ROW: &str = "0, NVIDIA GeForce RTX 5070 Ti, 575.57.08, 12.0, 16000, 12000\n";

    fn valid_observation() -> Result<ValidatedCudaObservation, String> {
        parse_nvidia_smi_cuda_zero(VALID_ROW).map_err(|error| error.to_string())
    }

    #[test]
    fn nvidia_smi_parser_selects_and_converts_exact_physical_cuda_zero() -> Result<(), String> {
        let parsed = parse_nvidia_smi_cuda_zero(
            "1, NVIDIA Other GPU, 575.57.08, 8.9, 24000, 22000\n0, NVIDIA GeForce RTX 5070 Ti, 575.57.08, 12.0, 16000, 12000\n",
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(
            parsed,
            ValidatedCudaObservation {
                name: REQUIRED_CUDA_NAME.to_owned(),
                driver_version: "575.57.08".to_owned(),
                compute_capability: CudaComputeCapability {
                    major: 12,
                    minor: 0,
                },
                total_bytes: 16_000 * MEBIBYTE_BYTES,
                free_bytes: 12_000 * MEBIBYTE_BYTES,
            }
        );
        assert_eq!(
            parsed.used_bytes().map_err(|error| error.to_string())?,
            4_000 * MEBIBYTE_BYTES
        );
        Ok(())
    }

    #[test]
    fn nvidia_smi_parser_rejects_invalid_zero_rows_and_nonexact_field_counts() {
        for output in [
            "0, NVIDIA GeForce RTX 4090, 575.57.08, 12.0, 16000, 12000\n",
            "1, NVIDIA GeForce RTX 5070 Ti, 575.57.08, 12.0, 16000, 12000\n",
            "0, NVIDIA GeForce RTX 5070 Ti, 575.57.08, 12.0, 16000, 12000\n0, NVIDIA GeForce RTX 5070 Ti, 575.57.08, 12.0, 16000, 12000\n",
            "0, NVIDIA GeForce RTX 5070 Ti, unknown, 12.0, 16000, 12000\n",
            "0, NVIDIA GeForce RTX 5070 Ti, 575.57.08, 12.1, 16000, 12000\n",
            "0, NVIDIA GeForce RTX 5070 Ti, 575.57.08, 12.0, 16000\n",
            "0, NVIDIA GeForce RTX 5070 Ti, 575.57.08, 12.0, 16000, 12000, extra\n",
            "0, NVIDIA GeForce RTX 5070 Ti, 575.57.08, 12.0, 12000, 16000\n",
            "0, NVIDIA GeForce RTX 5070 Ti, 575.57.08, 12.0, unknown, 12000\n",
            "0, NVIDIA GeForce RTX 5070 Ti, 575.57.08, 12.0, 18446744073709551615, 12000\n",
        ] {
            assert!(parse_nvidia_smi_cuda_zero(output).is_err(), "{output:?}");
        }
    }

    #[test]
    fn repeated_observations_require_stable_static_facts_but_allow_free_memory_to_vary()
    -> Result<(), String> {
        let observation = valid_observation()?;
        let state = CudaState::new(observation.clone());

        let mut changed_free = observation.clone();
        changed_free.free_bytes = 11_000 * MEBIBYTE_BYTES;
        state
            .validate_stable(&changed_free)
            .map_err(|error| error.to_string())?;

        let mut changed_name = observation.clone();
        changed_name.name = "NVIDIA GeForce RTX 5070".to_owned();
        assert!(state.validate_stable(&changed_name).is_err());

        let mut changed_driver = observation.clone();
        changed_driver.driver_version = "576.0".to_owned();
        assert!(state.validate_stable(&changed_driver).is_err());

        let mut changed_compute = observation.clone();
        changed_compute.compute_capability = CudaComputeCapability {
            major: 12,
            minor: 1,
        };
        assert!(state.validate_stable(&changed_compute).is_err());

        let mut changed_total = observation;
        changed_total.total_bytes = 16_001 * MEBIBYTE_BYTES;
        assert!(state.validate_stable(&changed_total).is_err());
        Ok(())
    }

    #[test]
    fn public_e1_cuda_summary_fields_are_validated_independently() -> Result<(), String> {
        let compute = ApplicationComputeCapability {
            major: 12,
            minor: 0,
        };
        validate_e1_cuda_fields(
            Some(REQUIRED_CUDA_NAME),
            Some(16_000),
            Some(12_000),
            Some(compute),
        )
        .map_err(|error| error.to_string())?;

        assert!(
            validate_e1_cuda_fields(
                Some("NVIDIA GeForce RTX 4090"),
                Some(16_000),
                Some(12_000),
                Some(compute),
            )
            .is_err()
        );
        assert!(
            validate_e1_cuda_fields(
                Some(REQUIRED_CUDA_NAME),
                Some(16_000),
                Some(16_001),
                Some(compute),
            )
            .is_err()
        );
        assert!(
            validate_e1_cuda_fields(
                Some(REQUIRED_CUDA_NAME),
                Some(16_000),
                Some(12_000),
                Some(ApplicationComputeCapability {
                    major: 12,
                    minor: 1,
                }),
            )
            .is_err()
        );
        Ok(())
    }
}
