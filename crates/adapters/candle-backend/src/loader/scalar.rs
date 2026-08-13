//! Required-scalar compatibility and device execution policy.

use candle_core::DType;
use domain_contracts::{BackendId, DeviceKind, LoadError, ScalarType, ScalarTypeSet};

use super::unsupported_scalar;

pub(super) fn select_required_primary(
    backend: BackendId,
    required: ScalarTypeSet,
    declaration: Option<ScalarType>,
) -> Result<ScalarType, LoadError> {
    let f32_set = ScalarTypeSet::from_scalar(ScalarType::F32);
    let f16_set = ScalarTypeSet::from_scalar(ScalarType::F16);
    let bf16_set = ScalarTypeSet::from_scalar(ScalarType::Bf16);
    let (primary, declaration_required) = if required == f32_set {
        (ScalarType::F32, false)
    } else if required == f16_set {
        (ScalarType::F16, false)
    } else if required == bf16_set {
        (ScalarType::Bf16, false)
    } else if required == f16_set.union(f32_set) {
        (ScalarType::F16, true)
    } else if required == bf16_set.union(f32_set) {
        (ScalarType::Bf16, true)
    } else {
        return Err(unsupported_scalar(backend));
    };

    match declaration {
        Some(declared) if declared == primary => Ok(primary),
        None if !declaration_required => Ok(primary),
        None | Some(_) => Err(unsupported_scalar(backend)),
    }
}

pub(super) fn select_execution_dtype(
    backend: BackendId,
    primary: ScalarType,
    device_kind: DeviceKind,
    supports_bf16: bool,
) -> Result<DType, LoadError> {
    match (primary, device_kind) {
        (ScalarType::F32, DeviceKind::Cpu | DeviceKind::Cuda)
        | (ScalarType::Bf16, DeviceKind::Cpu) => Ok(DType::F32),
        (ScalarType::F16, DeviceKind::Cpu | DeviceKind::Cuda) => Ok(DType::F16),
        (ScalarType::Bf16, DeviceKind::Cuda) if supports_bf16 => Ok(DType::BF16),
        (ScalarType::Bf16, DeviceKind::Cuda) => Err(unsupported_scalar(backend)),
        (_, DeviceKind::Cpu | DeviceKind::Cuda) => Err(LoadError::UnsupportedFormat),
        _ => Err(LoadError::InvalidConfiguration),
    }
}

pub(super) fn execution_scalar_type(
    backend: BackendId,
    dtype: DType,
) -> Result<ScalarType, LoadError> {
    match dtype {
        DType::F32 => Ok(ScalarType::F32),
        DType::F16 => Ok(ScalarType::F16),
        DType::BF16 => Ok(ScalarType::Bf16),
        _ => Err(unsupported_scalar(backend)),
    }
}

#[cfg(test)]
mod tests;
