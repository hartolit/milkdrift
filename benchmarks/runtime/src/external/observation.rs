//! Safe host/CUDA resource observation and exact external-runner device validation.

use std::env::{self, VarError};
use std::process::{Command, Output};

use application_runtime::{ApplicationDevice, ApplicationRuntime};
use candle_backend::{CandleDeviceSummary, CandleLlamaLoader};
use domain_contracts::{BackendId, DeviceId, DeviceKind, ExecutionDevice};

use super::cli::RequestedDevice;
use super::report::{
    CudaComputeCapability, CudaDeviceMetadata, CudaEnvironmentMetadata, CudaMemoryObservation,
    DeviceIdentity, ExecutionMetadata, ResourceCheckpoint,
};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::memory::process_memory;

const OBSERVATION_BACKEND: BackendId = BackendId::new(10_002);
const CPU_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
const CUDA_ZERO_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);
const CUDA_ZERO_APPLICATION_DEVICE: ApplicationDevice = ApplicationDevice::Cuda { ordinal: 0 };
const REQUIRED_CUDA_NAME: &str = "NVIDIA GeForce RTX 5070 Ti";
const REQUIRED_CUDA_COMPUTE_CAPABILITY: CudaComputeCapability = CudaComputeCapability {
    major: 12,
    minor: 0,
};
#[cfg(feature = "cuda")]
const REQUIRED_BUILD_COMPUTE_CAPABILITY: &str = "120";
const CUDA_LOGITS_TO_HOST_LIMITATION: &str = "CUDA vocabulary logits are transferred to host F32 before sampling; GPU-side sampling is not implemented";
const CUDA_MEMORY_OBSERVATION_SCOPE: &str = "safe CUDA driver total/free observations for the whole device, not process-attributed usage; desktop and other GPU processes can affect absolute values and deltas";

pub(super) struct DeviceObserver {
    requested: RequestedDevice,
    cuda: Option<CudaState>,
}

struct CudaState {
    metadata: CudaDeviceMetadata,
    supports_bf16: bool,
    pre_load_used_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ValidatedCudaProbe {
    name: String,
    compute_capability: CudaComputeCapability,
    total_bytes: u64,
    free_bytes: u64,
    supports_bf16: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NvidiaSmiMetadata {
    name: String,
    driver_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NvccMetadata {
    toolkit_release: String,
    compiler_version: String,
}

impl DeviceObserver {
    pub(super) fn new(requested: RequestedDevice) -> BenchmarkResult<Self> {
        let cuda = match requested {
            RequestedDevice::Cpu => None,
            RequestedDevice::Cuda0 => {
                validate_cuda_build_configuration()?;
                Some(CudaState::new(discover_cuda_zero()?))
            }
        };
        Ok(Self { requested, cuda })
    }

    pub(super) fn begin_cycle(&mut self) {
        if let Some(cuda) = self.cuda.as_mut() {
            cuda.pre_load_used_bytes = None;
        }
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

    pub(super) fn capture(&self, checkpoint: &'static str) -> BenchmarkResult<ResourceCheckpoint> {
        let host_memory = process_memory()?;
        let cuda_memory = match self.requested {
            RequestedDevice::Cpu => None,
            RequestedDevice::Cuda0 => {
                let cuda = self.cuda_state()?;
                let probe = Self::validated_cuda_probe(cuda)?;
                Some(cuda_memory_observation(&probe, cuda.pre_load_used_bytes)?)
            }
        };
        Ok(ResourceCheckpoint {
            checkpoint,
            host_memory,
            cuda_memory,
        })
    }

    pub(super) fn capture_pre_load(
        &mut self,
        checkpoint: &'static str,
    ) -> BenchmarkResult<ResourceCheckpoint> {
        let host_memory = process_memory()?;
        let cuda_memory = match self.requested {
            RequestedDevice::Cpu => None,
            RequestedDevice::Cuda0 => {
                let probe = {
                    let cuda = self.cuda_state()?;
                    Self::validated_cuda_probe(cuda)?
                };
                let used_bytes = probe.used_bytes()?;
                let observation = CudaMemoryObservation {
                    total_bytes: probe.total_bytes,
                    free_bytes: probe.free_bytes,
                    used_bytes,
                    used_delta_from_pre_load_bytes: Some(0),
                };
                self.cuda_state_mut()?.pre_load_used_bytes = Some(used_bytes);
                Some(observation)
            }
        };
        Ok(ResourceCheckpoint {
            checkpoint,
            host_memory,
            cuda_memory,
        })
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
                "E1 selected {expected:?}, but its latest bounded device probe marked it unavailable ({:?}); resolve CUDA feature, driver, or device initialization diagnostics before running the external workload",
                summary.unavailable_reason()
            )));
        }

        if self.requested == RequestedDevice::Cuda0 {
            let cuda = self.cuda_state()?;
            Self::validated_cuda_probe(cuda)?;
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

    pub(super) fn execution_metadata(&self, execution_dtype: &'static str) -> ExecutionMetadata {
        let cuda_enabled = cfg!(feature = "cuda");
        let cuda_requested = self.requested == RequestedDevice::Cuda0;
        ExecutionMetadata {
            cuda_enabled,
            requested_device: self.requested_identity(),
            cuda_device: self.cuda_device_metadata(),
            execution_dtype,
            host_sampling: true,
            cuda_logits_to_host_limitation: cuda_requested
                .then_some(CUDA_LOGITS_TO_HOST_LIMITATION),
            cuda_memory_observation_scope: cuda_requested.then_some(CUDA_MEMORY_OBSERVATION_SCOPE),
        }
    }

    pub(super) fn collect_cuda_environment(
        &self,
    ) -> BenchmarkResult<Option<CudaEnvironmentMetadata>> {
        if self.requested == RequestedDevice::Cpu {
            return Ok(None);
        }

        let build_compute_capability = validate_cuda_build_configuration()?;
        let cuda_visible_devices = collect_cuda_visible_devices()?;
        let cuda = self.cuda_state()?;
        let probe = Self::validated_cuda_probe(cuda)?;

        let nvidia_smi = query_nvidia_smi()?;
        if nvidia_smi.name != probe.name {
            return Err(BenchmarkError::new(format!(
                "fixed nvidia-smi metadata query identified CUDA index 0 as {:?}, but Candle discovered backend CUDA ordinal 0 as {:?}; ensure CUDA_VISIBLE_DEVICES is unset or exactly `0` and that both tools address the required device",
                nvidia_smi.name, probe.name
            )));
        }
        let nvcc = query_nvcc()?;
        validate_minimum_toolkit_release(&nvcc.toolkit_release)?;

        Ok(Some(CudaEnvironmentMetadata {
            driver_version: nvidia_smi.driver_version,
            toolkit_release: nvcc.toolkit_release,
            toolkit_compiler_version: nvcc.compiler_version,
            build_compute_capability: build_compute_capability.to_owned(),
            cuda_visible_devices,
        }))
    }

    fn cuda_state(&self) -> BenchmarkResult<&CudaState> {
        self.cuda.as_ref().ok_or_else(|| {
            BenchmarkError::new(
                "internal external-runner invariant failed: CUDA was requested but no validated CUDA device metadata is available",
            )
        })
    }

    fn cuda_state_mut(&mut self) -> BenchmarkResult<&mut CudaState> {
        self.cuda.as_mut().ok_or_else(|| {
            BenchmarkError::new(
                "internal external-runner invariant failed: CUDA was requested but no mutable CUDA observation state is available",
            )
        })
    }

    fn validated_cuda_probe(cuda: &CudaState) -> BenchmarkResult<ValidatedCudaProbe> {
        let probe = discover_cuda_zero()?;
        cuda.validate_stable(&probe)?;
        Ok(probe)
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
            pre_load_used_bytes: None,
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
    fn from_summary(summary: &CandleDeviceSummary) -> BenchmarkResult<Self> {
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

    fn used_bytes(&self) -> BenchmarkResult<u64> {
        self.total_bytes.checked_sub(self.free_bytes).ok_or_else(|| {
            BenchmarkError::new(format!(
                "CUDA ordinal 0 used-memory calculation underflowed: total {} bytes, free {} bytes",
                self.total_bytes, self.free_bytes
            ))
        })
    }
}

fn requested_execution_device(requested: RequestedDevice) -> ExecutionDevice {
    match requested {
        RequestedDevice::Cpu => CPU_EXECUTION_DEVICE,
        RequestedDevice::Cuda0 => CUDA_ZERO_EXECUTION_DEVICE,
    }
}

fn requested_application_device(requested: RequestedDevice) -> ApplicationDevice {
    match requested {
        RequestedDevice::Cpu => ApplicationDevice::Cpu,
        RequestedDevice::Cuda0 => CUDA_ZERO_APPLICATION_DEVICE,
    }
}

fn discover_cuda_zero() -> BenchmarkResult<ValidatedCudaProbe> {
    let summary = CandleLlamaLoader::new(OBSERVATION_BACKEND)
        .discover_device(CUDA_ZERO_EXECUTION_DEVICE)
        .map_err(|error| {
            BenchmarkError::new(format!(
                "safe Candle CUDA ordinal 0 discovery failed ({error:?}); verify the runtime-benchmarks `cuda` feature, NVIDIA driver, CUDA visibility, and device availability"
            ))
        })?;
    ValidatedCudaProbe::from_summary(&summary)
}

fn cuda_memory_observation(
    probe: &ValidatedCudaProbe,
    pre_load_used_bytes: Option<u64>,
) -> BenchmarkResult<CudaMemoryObservation> {
    let used_bytes = probe.used_bytes()?;
    let used_delta_from_pre_load_bytes = pre_load_used_bytes
        .map(|baseline| signed_used_delta(used_bytes, baseline))
        .transpose()?;
    Ok(CudaMemoryObservation {
        total_bytes: probe.total_bytes,
        free_bytes: probe.free_bytes,
        used_bytes,
        used_delta_from_pre_load_bytes,
    })
}

fn signed_used_delta(current: u64, pre_load: u64) -> BenchmarkResult<i64> {
    let delta = i128::from(current) - i128::from(pre_load);
    i64::try_from(delta).map_err(|_| {
        BenchmarkError::new(format!(
            "CUDA used-memory delta does not fit i64: current {current} bytes, pre-load baseline {pre_load} bytes"
        ))
    })
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

fn validate_cuda_build_configuration() -> BenchmarkResult<&'static str> {
    #[cfg(not(feature = "cuda"))]
    {
        Err(BenchmarkError::new(
            "CUDA observation requires the runtime-benchmarks `cuda` feature; rebuild with `--features cuda` and CUDA_COMPUTE_CAP=120",
        ))
    }
    #[cfg(feature = "cuda")]
    {
        match option_env!("CUDA_COMPUTE_CAP") {
            Some(REQUIRED_BUILD_COMPUTE_CAPABILITY) => Ok(REQUIRED_BUILD_COMPUTE_CAPABILITY),
            Some(value) => Err(BenchmarkError::new(format!(
                "runtime-benchmarks was compiled with CUDA_COMPUTE_CAP={value:?}; the required RTX 5070 Ti matrix must be rebuilt with CUDA_COMPUTE_CAP=120"
            ))),
            None => Err(BenchmarkError::new(
                "runtime-benchmarks was compiled without CUDA_COMPUTE_CAP; rebuild the CUDA target with CUDA_COMPUTE_CAP=120",
            )),
        }
    }
}

fn collect_cuda_visible_devices() -> BenchmarkResult<Option<String>> {
    match env::var("CUDA_VISIBLE_DEVICES") {
        Ok(value) if value == "0" => Ok(Some(value)),
        Ok(_) => Err(BenchmarkError::new(
            "CUDA_VISIBLE_DEVICES must be unset or exactly `0` for the required physical-index-0 external CUDA matrix",
        )),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(BenchmarkError::new(
            "CUDA_VISIBLE_DEVICES is not valid Unicode; unset it or set it to exactly `0`",
        )),
    }
}

fn query_nvidia_smi() -> BenchmarkResult<NvidiaSmiMetadata> {
    let output = Command::new("nvidia-smi")
        .arg("--query-gpu=index,name,driver_version")
        .arg("--format=csv,noheader,nounits")
        .output()
        .map_err(|error| {
            BenchmarkError::new(format!(
                "could not execute the fixed nvidia-smi GPU identity/driver query; ensure NVIDIA driver tools are installed and on PATH: {error}"
            ))
        })?;
    let stdout = successful_stdout(
        "fixed nvidia-smi GPU identity/driver query",
        output,
        "verify that the NVIDIA driver can query physical GPU index 0",
    )?;
    parse_nvidia_smi_cuda_zero(&stdout)
}

fn query_nvcc() -> BenchmarkResult<NvccMetadata> {
    let output = Command::new("nvcc")
        .arg("--version")
        .output()
        .map_err(|error| {
            BenchmarkError::new(format!(
                "could not execute fixed `nvcc --version`; ensure the CUDA Toolkit compiler is installed and on PATH: {error}"
            ))
        })?;
    let stdout = successful_stdout(
        "fixed `nvcc --version` query",
        output,
        "verify that the intended CUDA Toolkit compiler is installed and runnable",
    )?;
    parse_nvcc_version(&stdout)
}

fn successful_stdout(
    command_description: &str,
    output: Output,
    remediation: &str,
) -> BenchmarkResult<String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let stderr = if stderr.is_empty() {
            "no stderr output"
        } else {
            stderr
        };
        return Err(BenchmarkError::new(format!(
            "{command_description} exited with status {} ({stderr}); {remediation}",
            output.status
        )));
    }
    String::from_utf8(output.stdout).map_err(|error| {
        BenchmarkError::new(format!(
            "{command_description} returned non-UTF-8 stdout; {remediation}: {error}"
        ))
    })
}

fn parse_nvidia_smi_cuda_zero(output: &str) -> BenchmarkResult<NvidiaSmiMetadata> {
    let mut selected = None;
    for (line_index, line) in output.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let line_number = line_index + 1;
        let mut fields = line.split(',');
        let index = fields.next().map(str::trim).ok_or_else(|| {
            BenchmarkError::new(format!(
                "nvidia-smi metadata line {line_number} omitted the GPU index"
            ))
        })?;
        let name = fields.next().map(str::trim).ok_or_else(|| {
            BenchmarkError::new(format!(
                "nvidia-smi metadata line {line_number} omitted the GPU name"
            ))
        })?;
        let driver_version = fields.next().map(str::trim).ok_or_else(|| {
            BenchmarkError::new(format!(
                "nvidia-smi metadata line {line_number} omitted the driver version"
            ))
        })?;
        if fields.next().is_some() {
            return Err(BenchmarkError::new(format!(
                "nvidia-smi metadata line {line_number} must contain exactly index, name, and driver_version"
            )));
        }
        let index = index.parse::<u32>().map_err(|error| {
            BenchmarkError::new(format!(
                "nvidia-smi metadata line {line_number} has a nonnumeric GPU index: {error}"
            ))
        })?;
        if index != 0 {
            continue;
        }
        if selected.is_some() {
            return Err(BenchmarkError::new(
                "nvidia-smi metadata output contains more than one row for GPU index 0",
            ));
        }
        if name != REQUIRED_CUDA_NAME {
            return Err(BenchmarkError::new(format!(
                "nvidia-smi identifies physical GPU index 0 as {name:?}, but the required executed matrix is exactly {REQUIRED_CUDA_NAME:?}"
            )));
        }
        if !is_dotted_decimal_version(driver_version) {
            return Err(BenchmarkError::new(format!(
                "nvidia-smi returned invalid or empty driver version {driver_version:?} for GPU index 0"
            )));
        }
        selected = Some(NvidiaSmiMetadata {
            name: name.to_owned(),
            driver_version: driver_version.to_owned(),
        });
    }
    selected.ok_or_else(|| {
        BenchmarkError::new(
            "fixed nvidia-smi metadata query returned no row for physical GPU index 0",
        )
    })
}

fn parse_nvcc_version(output: &str) -> BenchmarkResult<NvccMetadata> {
    const PREFIX: &str = "Cuda compilation tools,";

    let mut parsed = None;
    for line in output.lines().map(str::trim) {
        let Some(version_fields) = line.strip_prefix(PREFIX) else {
            continue;
        };
        if parsed.is_some() {
            return Err(BenchmarkError::new(
                "`nvcc --version` output contains more than one `Cuda compilation tools` version line",
            ));
        }
        let (release, compiler_version) = version_fields.split_once(',').ok_or_else(|| {
            BenchmarkError::new(
                "`nvcc --version` CUDA compilation-tools line omitted the V compiler version",
            )
        })?;
        if compiler_version.contains(',') {
            return Err(BenchmarkError::new(
                "`nvcc --version` CUDA compilation-tools line contains unexpected extra fields",
            ));
        }
        let release = release.trim().strip_prefix("release ").ok_or_else(|| {
            BenchmarkError::new(
                "`nvcc --version` CUDA compilation-tools line omitted the `release` field",
            )
        })?;
        let compiler_version = compiler_version.trim();
        let compiler_numeric = compiler_version.strip_prefix('V').ok_or_else(|| {
            BenchmarkError::new(
                "`nvcc --version` CUDA compiler version must use the expected V-prefixed form",
            )
        })?;
        if !is_dotted_decimal_version(release) {
            return Err(BenchmarkError::new(format!(
                "`nvcc --version` returned invalid toolkit release {release:?}"
            )));
        }
        if !is_dotted_decimal_version(compiler_numeric) {
            return Err(BenchmarkError::new(format!(
                "`nvcc --version` returned invalid V compiler version {compiler_version:?}"
            )));
        }
        parsed = Some(NvccMetadata {
            toolkit_release: release.to_owned(),
            compiler_version: compiler_version.to_owned(),
        });
    }
    parsed.ok_or_else(|| {
        BenchmarkError::new(
            "`nvcc --version` output contained no `Cuda compilation tools, release ..., V...` line",
        )
    })
}

fn is_dotted_decimal_version(value: &str) -> bool {
    value.contains('.')
        && value.split('.').all(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn validate_minimum_toolkit_release(release: &str) -> BenchmarkResult {
    let mut components = release.split('.');
    let major = components
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| BenchmarkError::new("CUDA Toolkit release has no numeric major version"))?;
    let minor = components
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| BenchmarkError::new("CUDA Toolkit release has no numeric minor version"))?;
    if major < 12 || (major == 12 && minor < 8) {
        return Err(BenchmarkError::new(format!(
            "CUDA Toolkit release {release} is older than the required 12.8 minimum"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use application_runtime::ApplicationDevice;
    use candle_backend::{CandleDeviceSummary, CudaComputeCapability as CandleComputeCapability};
    use domain_contracts::{DeviceId, DeviceKind, ExecutionDevice};

    use super::{
        CPU_EXECUTION_DEVICE, CUDA_LOGITS_TO_HOST_LIMITATION, CUDA_MEMORY_OBSERVATION_SCOPE,
        CUDA_ZERO_EXECUTION_DEVICE, CudaState, DeviceObserver, NvccMetadata, NvidiaSmiMetadata,
        REQUIRED_CUDA_NAME, ValidatedCudaProbe, cuda_memory_observation, parse_nvcc_version,
        parse_nvidia_smi_cuda_zero, signed_used_delta,
    };
    use crate::external::cli::RequestedDevice;
    use crate::external::report::{
        CudaComputeCapability, CudaDeviceMetadata, CudaMemoryObservation, DeviceIdentity,
    };

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

    fn valid_probe() -> Result<ValidatedCudaProbe, String> {
        ValidatedCudaProbe::from_summary(&valid_cuda_summary()).map_err(|error| error.to_string())
    }

    fn synthetic_cuda_observer() -> Result<DeviceObserver, String> {
        Ok(DeviceObserver {
            requested: RequestedDevice::Cuda0,
            cuda: Some(CudaState::new(valid_probe()?)),
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
        assert_eq!(
            execution.requested_device,
            DeviceIdentity {
                kind: "cpu",
                id: 0,
                ordinal: None,
            }
        );
        assert_eq!(execution.cuda_device, None);
        assert_eq!(execution.execution_dtype, "F32");
        assert!(execution.host_sampling);
        assert_eq!(execution.cuda_logits_to_host_limitation, None);
        assert_eq!(execution.cuda_memory_observation_scope, None);
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
        assert_eq!(execution.execution_dtype, "BF16");
        assert_eq!(
            execution.cuda_logits_to_host_limitation,
            Some(CUDA_LOGITS_TO_HOST_LIMITATION)
        );
        assert_eq!(
            execution.cuda_memory_observation_scope,
            Some(CUDA_MEMORY_OBSERVATION_SCOPE)
        );
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
    fn cuda_probe_validation_requires_exact_identity_and_matrix() -> Result<(), String> {
        let probe = valid_probe()?;
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
        let probe = valid_probe()?;
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

    #[test]
    fn cycle_reset_clears_the_previous_pre_load_baseline() -> Result<(), String> {
        let mut observer = synthetic_cuda_observer()?;
        observer
            .cuda
            .as_mut()
            .ok_or_else(|| "synthetic CUDA state disappeared".to_owned())?
            .pre_load_used_bytes = Some(4_000);
        observer.begin_cycle();
        assert_eq!(
            observer
                .cuda
                .as_ref()
                .ok_or_else(|| "synthetic CUDA state disappeared".to_owned())?
                .pre_load_used_bytes,
            None
        );
        Ok(())
    }

    #[test]
    fn cuda_memory_delta_is_signed_and_pre_load_can_be_exactly_zero() -> Result<(), String> {
        assert_eq!(
            signed_used_delta(12_000, 10_000).map_err(|error| error.to_string())?,
            2_000
        );
        assert_eq!(
            signed_used_delta(8_000, 10_000).map_err(|error| error.to_string())?,
            -2_000
        );
        assert_eq!(
            signed_used_delta(10_000, 10_000).map_err(|error| error.to_string())?,
            0
        );
        assert!(signed_used_delta(u64::MAX, 0).is_err());

        let probe = valid_probe()?;
        assert_eq!(
            cuda_memory_observation(&probe, Some(4_000)).map_err(|error| error.to_string())?,
            CudaMemoryObservation {
                total_bytes: 16_000,
                free_bytes: 12_000,
                used_bytes: 4_000,
                used_delta_from_pre_load_bytes: Some(0),
            }
        );
        Ok(())
    }

    #[test]
    fn nvidia_smi_parser_selects_exact_physical_cuda_zero() -> Result<(), String> {
        let parsed = parse_nvidia_smi_cuda_zero(
            "1, NVIDIA Other GPU, 575.57.08\n0, NVIDIA GeForce RTX 5070 Ti, 575.57.08\n",
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(
            parsed,
            NvidiaSmiMetadata {
                name: REQUIRED_CUDA_NAME.to_owned(),
                driver_version: "575.57.08".to_owned(),
            }
        );
        Ok(())
    }

    #[test]
    fn nvidia_smi_parser_rejects_wrong_missing_duplicate_and_malformed_zero_rows() {
        for output in [
            "0, NVIDIA GeForce RTX 4090, 575.57.08\n",
            "1, NVIDIA GeForce RTX 5070 Ti, 575.57.08\n",
            "0, NVIDIA GeForce RTX 5070 Ti, 575.57.08\n0, NVIDIA GeForce RTX 5070 Ti, 575.57.08\n",
            "0, NVIDIA GeForce RTX 5070 Ti, unknown\n",
            "0, NVIDIA GeForce RTX 5070 Ti\n",
        ] {
            assert!(parse_nvidia_smi_cuda_zero(output).is_err(), "{output:?}");
        }
    }

    #[test]
    fn nvcc_parser_extracts_release_and_v_compiler_version() -> Result<(), String> {
        let parsed = parse_nvcc_version(
            "nvcc: NVIDIA (R) Cuda compiler driver\nCopyright (c) NVIDIA\nCuda compilation tools, release 12.8, V12.8.93\nBuild cuda_12.8.r12.8/compiler.35583870_0\n",
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(
            parsed,
            NvccMetadata {
                toolkit_release: "12.8".to_owned(),
                compiler_version: "V12.8.93".to_owned(),
            }
        );
        Ok(())
    }

    #[test]
    fn nvcc_parser_rejects_absent_duplicate_and_non_v_versions() {
        for output in [
            "nvcc: NVIDIA (R) Cuda compiler driver\n",
            "Cuda compilation tools, release 12.8, 12.8.93\n",
            "Cuda compilation tools, release unknown, V12.8.93\n",
            "Cuda compilation tools, release 12.8, V12.8.93\nCuda compilation tools, release 12.8, V12.8.93\n",
        ] {
            assert!(parse_nvcc_version(output).is_err(), "{output:?}");
        }
    }

    #[test]
    fn cuda_execution_metadata_documents_host_sampling_boundary() {
        assert!(CUDA_LOGITS_TO_HOST_LIMITATION.contains("host F32"));
        assert!(CUDA_LOGITS_TO_HOST_LIMITATION.contains("GPU-side sampling"));
    }
}
