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

    for (required, declaration, expected) in [
        (f16.union(f32), ScalarType::F16, ScalarType::F16),
        (bf16.union(f32), ScalarType::Bf16, ScalarType::Bf16),
    ] {
        assert_eq!(
            select_required_primary(BACKEND, required, Some(declaration)),
            Ok(expected)
        );
        assert!(matches!(
            select_required_primary(BACKEND, required, None),
            Err(LoadError::Backend(failure)) if failure.code == CODE_UNSUPPORTED_SCALAR
        ));
    }
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
