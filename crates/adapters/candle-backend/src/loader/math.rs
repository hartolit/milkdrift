//! Execution-scalar widths and loader arithmetic failure ownership.

use candle_core::DType;
use domain_contracts::{BackendId, LoadError};

use crate::failure::CODE_NUMERIC_OVERFLOW;

use super::invalid_model_failure;

/// Returns the byte width of one supported execution scalar.
///
/// Source-format dtype classification remains owned by the Safetensors parser;
/// this function covers only concrete execution dtypes used by loader math.
pub(super) const fn execution_dtype_bytes(dtype: DType) -> Option<u64> {
    match dtype {
        DType::F32 => Some(4),
        DType::F16 | DType::BF16 => Some(2),
        _ => None,
    }
}

pub(super) const fn numeric_overflow(backend: BackendId) -> LoadError {
    invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW)
}
