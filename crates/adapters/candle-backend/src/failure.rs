//! Stable conversion from Candle failures into allocation-free domain errors.

use domain_contracts::{BackendFailure, BackendFailureKind, BackendId};

pub const CODE_CONFIG_READ: u32 = 1;
pub const CODE_CONFIG_DECODE: u32 = 2;
pub const CODE_WEIGHT_METADATA: u32 = 3;
pub const CODE_WEIGHT_LOAD: u32 = 4;
pub const CODE_DUPLICATE_TENSOR: u32 = 5;
pub const CODE_MODEL_LOAD: u32 = 6;
pub const CODE_MODEL_LOAD_PANIC: u32 = 7;
pub const CODE_CACHE_CREATE: u32 = 8;
pub const CODE_INPUT_TENSOR: u32 = 9;
pub const CODE_FORWARD: u32 = 10;
pub const CODE_LOGITS_LAYOUT: u32 = 11;
pub const CODE_LOGITS_STORAGE: u32 = 12;
pub const CODE_SYNCHRONIZE: u32 = 13;
pub const CODE_RESERVATION: u32 = 14;
pub const CODE_NUMERIC_OVERFLOW: u32 = 15;
#[cfg(not(feature = "cuda"))]
pub const CODE_CUDA_NOT_COMPILED: u32 = 16;
pub const CODE_UNSUPPORTED_DEVICE: u32 = 17;
#[cfg(feature = "cuda")]
pub const CODE_CUDA_INITIALIZATION: u32 = 18;
#[cfg(feature = "cuda")]
pub const CODE_CUDA_DISCOVERY: u32 = 19;
pub const CODE_LOGITS_TRANSFER: u32 = 20;
pub const CODE_UNSUPPORTED_SCALAR: u32 = 21;

pub const fn failure(backend: BackendId, kind: BackendFailureKind, code: u32) -> BackendFailure {
    BackendFailure::new(backend, kind, code)
}

#[cfg(feature = "cuda")]
pub fn candle_cuda_failure_kind(error: &candle_core::Error) -> Option<BackendFailureKind> {
    use cudarc::driver::{DriverError, sys::CUresult};

    fn driver_error(error: &candle_core::Error) -> Option<&DriverError> {
        match error {
            candle_core::Error::Cuda(source) => source.downcast_ref::<DriverError>(),
            candle_core::Error::WrappedContext { wrapped, .. } => {
                wrapped.downcast_ref::<DriverError>()
            }
            candle_core::Error::Context { inner, .. }
            | candle_core::Error::WithPath { inner, .. }
            | candle_core::Error::WithBacktrace { inner, .. } => driver_error(inner),
            _ => None,
        }
    }

    driver_error(error).map(|error| {
        if error.0 == CUresult::CUDA_ERROR_OUT_OF_MEMORY {
            BackendFailureKind::DeviceMemory
        } else {
            BackendFailureKind::DeviceExecution
        }
    })
}

#[cfg(not(feature = "cuda"))]
pub const fn candle_cuda_failure_kind(_error: &candle_core::Error) -> Option<BackendFailureKind> {
    None
}
