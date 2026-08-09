//! Stable conversion from Candle failures into allocation-free domain errors.

use domain_contracts::{BackendFailure, BackendFailureKind, BackendId};

pub const CODE_CONFIG_READ: u32 = 1;
pub const CODE_CONFIG_DECODE: u32 = 2;
pub const CODE_WEIGHT_METADATA: u32 = 3;

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
/// A Safetensors length prefix, aggregate header bound, offset, or payload bound was invalid.
pub const CODE_HEADER_BOUNDS: u32 = 23;
/// A bounded Safetensors header could not be decoded.
pub const CODE_HEADER_DECODE: u32 = 24;
/// Host allocation for bounded header inspection failed.
pub const CODE_HEADER_ALLOCATION: u32 = 25;
/// A required Candle Llama tensor was absent or had the wrong shape.
pub const CODE_REQUIRED_TENSOR: u32 = 26;
/// An inspected tensor payload could not be read exactly from its retained file.
pub const CODE_PAYLOAD_READ: u32 = 27;
/// A source tensor could not be materialized or converted on CPU.
pub const CODE_TENSOR_MATERIALIZE: u32 = 28;
/// A converted CPU tensor could not be transferred to the selected device.
pub const CODE_TENSOR_TRANSFER: u32 = 29;
/// A load-time device synchronization failed after materialization began.
pub const CODE_LOAD_SYNCHRONIZE: u32 = 30;
/// Retryable synchronization of a failed partial load did not complete.
pub const CODE_PARTIAL_LOAD_SYNCHRONIZE: u32 = 31;
/// A retained whole shard did not match its accepted SHA-256 identity.
pub const CODE_SOURCE_IDENTITY_MISMATCH: u32 = 32;
/// The bounded model configuration exceeded one MiB.
pub const CODE_CONFIG_LIMIT: u32 = 33;
/// Host allocation for bounded configuration bytes failed.
pub const CODE_CONFIG_ALLOCATION: u32 = 34;
/// A scalar declaration was duplicated or had the wrong JSON type.
pub const CODE_DECLARATION_MALFORMED: u32 = 35;
/// A present scalar declaration string was not in the reviewed vocabulary.
pub const CODE_DECLARATION_UNSUPPORTED: u32 = 36;
/// Modern and legacy recognized scalar declarations disagreed.
pub const CODE_DECLARATION_CONFLICT: u32 = 37;
/// Explicit Llama model identity was absent, malformed, or contradictory.
pub const CODE_ARCHITECTURE: u32 = 38;
/// A per-shard or aggregate Safetensors header ceiling was exceeded.
pub const CODE_HEADER_LIMIT: u32 = 39;
/// A tensor count, name, rank, or aggregate shape ceiling was exceeded.
pub const CODE_TENSOR_LIMIT: u32 = 40;
/// A Safetensors metadata entry/string ceiling was exceeded.
pub const CODE_METADATA_LIMIT: u32 = 41;
/// The final owned inspection inventory ceiling was exceeded.
pub const CODE_INSPECTION_INVENTORY_LIMIT: u32 = 42;
/// Host allocation for bounded inspection inventory failed.
pub const CODE_INSPECTION_ALLOCATION: u32 = 43;
/// The retained Safetensors prefix/header changed after inspection.
pub const CODE_HEADER_IDENTITY_MISMATCH: u32 = 44;
/// A supplied or observed whole-shard length disagreed with the retained file.
pub const CODE_SOURCE_IDENTITY_LENGTH: u32 = 45;
/// Host allocation for the pre-sized final required-tensor map failed.
pub const CODE_TENSOR_MAP_ALLOCATION: u32 = 46;

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
