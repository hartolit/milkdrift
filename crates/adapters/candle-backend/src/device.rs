//! Explicit Candle execution-device construction and discovery.

use candle_core::Device;
use domain_contracts::{
    BackendFailureKind, BackendId, ByteCount, DeviceKind, ExecutionDevice, LoadError,
    LoadFailureStage,
};

#[cfg(not(feature = "cuda"))]
use crate::failure::CODE_CUDA_NOT_COMPILED;
#[cfg(feature = "cuda")]
use crate::failure::{CODE_CUDA_DISCOVERY, CODE_CUDA_INITIALIZATION};
use crate::failure::{CODE_UNSUPPORTED_DEVICE, load_failure};

/// CUDA compute capability reported by the selected driver device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CudaComputeCapability {
    /// CUDA compute-capability major version.
    pub major: u32,
    /// CUDA compute-capability minor version.
    pub minor: u32,
}

/// Stable adapter-owned physical facts observed while initializing a device.
///
/// Total and available memory are driver observations made at probe time. They
/// are distinct from the planned and retained accounting quantities represented
/// by `MemoryFootprint`. CUDA ordinals are process-local backend selectors, not
/// permanent hardware identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandleDeviceSummary {
    /// Backend-visible identity accepted by Candle.
    pub execution_device: ExecutionDevice,
    /// CUDA ordinal when this is a CUDA device.
    pub ordinal: Option<u64>,
    /// Driver-provided display name when available.
    pub display_name: Option<String>,
    /// CUDA compute capability when this is a CUDA device.
    pub compute_capability: Option<CudaComputeCapability>,
    /// Physical device-local memory capacity reported during this probe.
    pub total_memory_bytes: Option<ByteCount>,
    /// Moment-in-time available device-local memory reported during this probe.
    pub available_memory_bytes: Option<ByteCount>,
    /// Whether Candle reports BF16 execution support for this device.
    pub supports_bf16: bool,
}

pub(crate) struct PreparedExecutionDevice {
    pub(crate) device: Device,
    pub(crate) summary: CandleDeviceSummary,
}

/// Independently initializes one cold-path execution-device probe.
///
/// Discovery, load planning, and loading each call this function and retain no
/// shared probe cache. Only the load path keeps the returned Candle device.
pub(crate) fn prepare_execution_device(
    backend: BackendId,
    execution_device: ExecutionDevice,
) -> Result<PreparedExecutionDevice, LoadError> {
    match execution_device.kind {
        DeviceKind::Cpu if execution_device.id.get() == 0 => {
            let device = Device::Cpu;
            Ok(PreparedExecutionDevice {
                summary: CandleDeviceSummary {
                    execution_device,
                    ordinal: None,
                    display_name: Some("CPU".to_owned()),
                    compute_capability: None,
                    total_memory_bytes: None,
                    available_memory_bytes: None,
                    supports_bf16: device.supports_bf16(),
                },
                device,
            })
        }
        DeviceKind::Cpu => Err(LoadError::InvalidConfiguration),
        DeviceKind::Cuda => prepare_cuda_device(backend, execution_device),
        _ => Err(load_failure(
            backend,
            BackendFailureKind::Unsupported,
            CODE_UNSUPPORTED_DEVICE,
            LoadFailureStage::ConfigurationInspection,
        )),
    }
}

#[cfg(feature = "cuda")]
fn prepare_cuda_device(
    backend: BackendId,
    execution_device: ExecutionDevice,
) -> Result<PreparedExecutionDevice, LoadError> {
    use cudarc::driver::CudaContext;

    let ordinal_i32 =
        i32::try_from(execution_device.id.get()).map_err(|_| LoadError::InvalidConfiguration)?;
    let ordinal = usize::try_from(ordinal_i32).map_err(|_| LoadError::InvalidConfiguration)?;

    // Construct Candle's complete CUDA device first so discovery proves that the
    // exact backend path, including its BLAS and RNG dependencies, can initialize.
    let device = Device::new_cuda(ordinal).map_err(|_| {
        load_failure(
            backend,
            BackendFailureKind::DeviceInitialization,
            CODE_CUDA_INITIALIZATION,
            LoadFailureStage::ConfigurationInspection,
        )
    })?;
    let context = CudaContext::new(ordinal).map_err(|_| {
        load_failure(
            backend,
            BackendFailureKind::DeviceInitialization,
            CODE_CUDA_INITIALIZATION,
            LoadFailureStage::ConfigurationInspection,
        )
    })?;
    let display_name = context.name().map_err(|_| cuda_discovery_error(backend))?;
    let (major, minor) = context
        .compute_capability()
        .map_err(|_| cuda_discovery_error(backend))?;
    let (available, total) = context
        .mem_get_info()
        .map_err(|_| cuda_discovery_error(backend))?;
    let major = u32::try_from(major).map_err(|_| cuda_discovery_error(backend))?;
    let minor = u32::try_from(minor).map_err(|_| cuda_discovery_error(backend))?;
    let total = u64::try_from(total).map_err(|_| cuda_discovery_error(backend))?;
    let available = u64::try_from(available).map_err(|_| cuda_discovery_error(backend))?;

    Ok(PreparedExecutionDevice {
        summary: CandleDeviceSummary {
            execution_device,
            ordinal: Some(execution_device.id.get()),
            display_name: Some(display_name),
            compute_capability: Some(CudaComputeCapability { major, minor }),
            total_memory_bytes: Some(ByteCount::from_u64(total)),
            available_memory_bytes: Some(ByteCount::from_u64(available)),
            supports_bf16: device.supports_bf16(),
        },
        device,
    })
}

#[cfg(not(feature = "cuda"))]
fn prepare_cuda_device(
    backend: BackendId,
    _execution_device: ExecutionDevice,
) -> Result<PreparedExecutionDevice, LoadError> {
    Err(load_failure(
        backend,
        BackendFailureKind::Unsupported,
        CODE_CUDA_NOT_COMPILED,
        LoadFailureStage::ConfigurationInspection,
    ))
}

#[cfg(feature = "cuda")]
const fn cuda_discovery_error(backend: BackendId) -> LoadError {
    load_failure(
        backend,
        BackendFailureKind::DeviceInitialization,
        CODE_CUDA_DISCOVERY,
        LoadFailureStage::ConfigurationInspection,
    )
}
