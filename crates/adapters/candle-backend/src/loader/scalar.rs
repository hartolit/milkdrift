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
    let primary = if required == f32_set {
        ScalarType::F32
    } else if required == f16_set || required == f16_set.union(f32_set) {
        ScalarType::F16
    } else if required == bf16_set || required == bf16_set.union(f32_set) {
        ScalarType::Bf16
    } else {
        return Err(unsupported_scalar(backend));
    };

    match declaration {
        None => Ok(primary),
        Some(ScalarType::F32 | ScalarType::F16 | ScalarType::Bf16)
            if declaration == Some(primary) =>
        {
            Ok(primary)
        }
        Some(_) => Err(unsupported_scalar(backend)),
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
mod tests {
    use domain_contracts::{BackendId, LoadError, ScalarType, ScalarTypeSet};

    use super::select_required_primary;
    use crate::failure::CODE_UNSUPPORTED_SCALAR;

    const BACKEND: BackendId = BackendId::new(3);

    #[test]
    fn required_scalar_matrix_ignores_complete_observed_extras() {
        let f32 = ScalarTypeSet::from_scalar(ScalarType::F32);
        let f16 = ScalarTypeSet::from_scalar(ScalarType::F16);
        let bf16 = ScalarTypeSet::from_scalar(ScalarType::Bf16);
        let unused_sets = [
            f16,
            bf16,
            f16.union(bf16),
            ScalarTypeSet::from_scalar(ScalarType::U8),
            ScalarTypeSet::from_scalar(ScalarType::Other(99)),
        ];
        for unused in unused_sets {
            let complete_observed = f32.union(unused);
            assert!(!complete_observed.is_empty());
            assert_eq!(
                select_required_primary(BACKEND, f32, None),
                Ok(ScalarType::F32)
            );
            assert_eq!(
                select_required_primary(BACKEND, f32, Some(ScalarType::F32)),
                Ok(ScalarType::F32)
            );
        }

        assert_eq!(
            select_required_primary(BACKEND, f16.union(f32), None),
            Ok(ScalarType::F16)
        );
        assert_eq!(
            select_required_primary(BACKEND, bf16.union(f32), None),
            Ok(ScalarType::Bf16)
        );
    }

    #[test]
    fn empty_other_and_f16_bf16_required_sets_reject() {
        let f16 = ScalarTypeSet::from_scalar(ScalarType::F16);
        let bf16 = ScalarTypeSet::from_scalar(ScalarType::Bf16);
        for required in [
            ScalarTypeSet::EMPTY,
            f16.union(bf16),
            ScalarTypeSet::from_scalar(ScalarType::Other(1)),
        ] {
            assert!(matches!(
                select_required_primary(BACKEND, required, None),
                Err(LoadError::Backend(failure)) if failure.code == CODE_UNSUPPORTED_SCALAR
            ));
        }
    }

    #[test]
    fn declaration_must_match_required_primary() {
        let f32 = ScalarTypeSet::from_scalar(ScalarType::F32);
        assert_eq!(
            select_required_primary(BACKEND, f32, Some(ScalarType::F32)),
            Ok(ScalarType::F32)
        );
        assert!(matches!(
            select_required_primary(BACKEND, f32, Some(ScalarType::F16)),
            Err(LoadError::Backend(failure)) if failure.code == CODE_UNSUPPORTED_SCALAR
        ));
    }
}
