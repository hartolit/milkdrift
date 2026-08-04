//! CUDA build and process environment validation plus direct metadata commands.

use std::env::{self, VarError};
use std::process::{Command, Output};

use super::super::cli::RequestedDevice;
use super::super::report::CudaEnvironmentMetadata;
use super::device::{DeviceState, DiscoveryCounter, REQUIRED_CUDA_NAME};
use crate::error::{BenchmarkError, BenchmarkResult};

#[cfg(feature = "cuda")]
const REQUIRED_BUILD_COMPUTE_CAPABILITY: &str = "120";

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

pub(super) fn collect_cuda_environment(
    device: &DeviceState,
    discovery_counter: &DiscoveryCounter,
) -> BenchmarkResult<Option<CudaEnvironmentMetadata>> {
    if device.requested() == RequestedDevice::Cpu {
        return Ok(None);
    }

    let build_compute_capability = validate_cuda_build_configuration()?;
    let cuda_visible_devices = collect_cuda_visible_devices()?;
    let probe = device.validated_cuda_probe(discovery_counter)?;

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

pub(super) fn validate_cuda_build_configuration() -> BenchmarkResult<&'static str> {
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
    use super::{
        NvccMetadata, NvidiaSmiMetadata, parse_nvcc_version, parse_nvidia_smi_cuda_zero,
        validate_minimum_toolkit_release,
    };
    use crate::external::observation::device::REQUIRED_CUDA_NAME;

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
    fn toolkit_release_enforces_the_12_8_minimum() -> Result<(), String> {
        validate_minimum_toolkit_release("12.8").map_err(|error| error.to_string())?;
        validate_minimum_toolkit_release("13.0").map_err(|error| error.to_string())?;
        assert!(validate_minimum_toolkit_release("12.7").is_err());
        assert!(validate_minimum_toolkit_release("11.9").is_err());
        Ok(())
    }
}
