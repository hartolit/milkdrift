use std::fs::{self, File};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use domain_contracts::{BackendFailureKind, BackendId, LoadError, ScalarType};

use super::{
    InspectionBudget, InspectionLimits, SourceTensorDType, TEST_INSPECTION_ALLOCATION_FAILURES,
    map_parse_failure, parse_header,
};
use crate::failure::{
    CODE_HEADER_ALLOCATION, CODE_HEADER_LIMIT, CODE_INSPECTION_ALLOCATION,
    CODE_INSPECTION_INVENTORY_LIMIT, CODE_METADATA_LIMIT, CODE_TENSOR_LIMIT,
};
use crate::source::CandleWeightShard;

use super::super::manifest::inspect_weight_shards;

const BACKEND: BackendId = BackendId::new(11);
static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

fn backend_code(error: LoadError) -> Option<(BackendFailureKind, u32)> {
    match error {
        LoadError::Backend(failure) => Some((failure.kind, failure.code)),
        _ => None,
    }
}

fn required_error<T>(result: Result<T, LoadError>, context: &str) -> Result<LoadError, String> {
    result.err().ok_or_else(|| context.to_owned())
}

type ParserLimitCase = (fn(&mut InspectionLimits), &'static [u8], u32);

#[test]
fn safetensors_08_dtype_classification_is_complete_and_stable() {
    let direct = [
        (SourceTensorDType::F32, ScalarType::F32),
        (SourceTensorDType::F16, ScalarType::F16),
        (SourceTensorDType::Bf16, ScalarType::Bf16),
        (SourceTensorDType::I8, ScalarType::I8),
        (SourceTensorDType::U8, ScalarType::U8),
    ];
    for (dtype, scalar) in direct {
        assert_eq!(dtype.scalar_type(), scalar);
    }
    for dtype in [
        SourceTensorDType::Bool,
        SourceTensorDType::F4,
        SourceTensorDType::F6E2M3,
        SourceTensorDType::F6E3M2,
        SourceTensorDType::F8E5M2,
        SourceTensorDType::F8E4M3,
        SourceTensorDType::F8E8M0,
        SourceTensorDType::F8E4M3Fnuz,
        SourceTensorDType::F8E5M2Fnuz,
        SourceTensorDType::I16,
        SourceTensorDType::U16,
        SourceTensorDType::I32,
        SourceTensorDType::U32,
        SourceTensorDType::C64,
        SourceTensorDType::F64,
        SourceTensorDType::I64,
        SourceTensorDType::U64,
    ] {
        assert!(matches!(dtype.scalar_type(), ScalarType::Other(_)));
        assert_eq!(dtype.executable_dtype(), None);
    }
}

#[test]
fn bitpacked_widths_are_calculated_without_rounding() -> Result<(), String> {
    let mut limits = InspectionLimits::PRODUCTION;
    let mut budget = InspectionBudget::new(&limits);
    let tensors = parse_header(
        BACKEND,
        br#"{"packed":{"dtype":"F4","shape":[2],"data_offsets":[0,1]}}"#,
        &mut budget,
    )
    .map_err(|error| format!("parse aligned F4 tensor: {error:?}"))?;
    let tensor = tensors
        .first()
        .ok_or_else(|| "aligned F4 tensor was not retained".to_owned())?;
    assert_eq!(tensor.source_bytes, 1);

    limits.tensors = 1;
    let mut budget = InspectionBudget::new(&limits);
    assert!(
        parse_header(
            BACKEND,
            br#"{"packed":{"dtype":"F4","shape":[1],"data_offsets":[0,1]}}"#,
            &mut budget,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn every_injectable_parser_limit_uses_small_fixtures() -> Result<(), String> {
    let cases: &[ParserLimitCase] = &[
        (
            |limits| limits.tensors = 0,
            br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
            CODE_TENSOR_LIMIT,
        ),
        (
            |limits| limits.tensor_name_bytes = 0,
            br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
            CODE_TENSOR_LIMIT,
        ),
        (
            |limits| limits.aggregate_tensor_name_bytes = 0,
            br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
            CODE_TENSOR_LIMIT,
        ),
        (
            |limits| limits.rank = 0,
            br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
            CODE_TENSOR_LIMIT,
        ),
        (
            |limits| limits.shape_dimension_extent = 0,
            br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
            CODE_TENSOR_LIMIT,
        ),
        (
            |limits| limits.aggregate_shape_dimensions = 0,
            br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
            CODE_TENSOR_LIMIT,
        ),
        (
            |limits| limits.metadata_entries = 0,
            br#"{"__metadata__":{"x":"y"}}"#,
            CODE_METADATA_LIMIT,
        ),
        (
            |limits| limits.metadata_key_bytes = 0,
            br#"{"__metadata__":{"x":"y"}}"#,
            CODE_METADATA_LIMIT,
        ),
        (
            |limits| limits.metadata_value_bytes = 0,
            br#"{"__metadata__":{"x":"y"}}"#,
            CODE_METADATA_LIMIT,
        ),
        (
            |limits| limits.aggregate_metadata_string_bytes = 1,
            br#"{"__metadata__":{"x":"y"}}"#,
            CODE_METADATA_LIMIT,
        ),
        (
            |limits| limits.final_inventory_bytes = 0,
            br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
            CODE_INSPECTION_INVENTORY_LIMIT,
        ),
    ];

    for (configure, header, expected_code) in cases {
        let mut limits = InspectionLimits::PRODUCTION;
        configure(&mut limits);
        let mut budget = InspectionBudget::new(&limits);
        let error = required_error(
            parse_header(BACKEND, header, &mut budget),
            "configured structural ceiling must fail",
        )?;
        assert_eq!(
            backend_code(error),
            Some((BackendFailureKind::InvalidModel, *expected_code))
        );
    }
    Ok(())
}

#[test]
fn shard_header_and_final_inventory_limits_use_small_files() -> Result<(), String> {
    let first_path = write_safetensors(br"{}", &[])?;
    let second_path = write_safetensors(br"{}", &[])?;
    let first = CandleWeightShard::unverified_local(first_path.clone());
    let second = CandleWeightShard::unverified_local(second_path.clone());

    let mut limits = InspectionLimits::PRODUCTION;
    limits.shards = 0;
    assert!(matches!(
        inspect_weight_shards(BACKEND, std::slice::from_ref(&first), &limits),
        Err(LoadError::CapacityExhausted(_))
    ));

    limits = InspectionLimits::PRODUCTION;
    limits.per_shard_header_bytes = 1;
    let error = required_error(
        inspect_weight_shards(BACKEND, std::slice::from_ref(&first), &limits),
        "per-shard header ceiling must fail",
    )?;
    assert_eq!(
        backend_code(error),
        Some((BackendFailureKind::InvalidModel, CODE_HEADER_LIMIT))
    );

    limits = InspectionLimits::PRODUCTION;
    limits.aggregate_header_bytes = 3;
    let error = required_error(
        inspect_weight_shards(BACKEND, &[first.clone(), second], &limits),
        "aggregate header ceiling must fail",
    )?;
    assert_eq!(
        backend_code(error),
        Some((BackendFailureKind::InvalidModel, CODE_HEADER_LIMIT))
    );

    limits = InspectionLimits::PRODUCTION;
    limits.final_inventory_bytes = 0;
    let error = required_error(
        inspect_weight_shards(BACKEND, std::slice::from_ref(&first), &limits),
        "final inventory ceiling must include each shard",
    )?;
    assert_eq!(
        backend_code(error),
        Some((
            BackendFailureKind::InvalidModel,
            CODE_INSPECTION_INVENTORY_LIMIT,
        ))
    );

    TEST_INSPECTION_ALLOCATION_FAILURES.with(|remaining| remaining.set(3));
    let error = required_error(
        inspect_weight_shards(
            BACKEND,
            std::slice::from_ref(&first),
            &InspectionLimits::PRODUCTION,
        ),
        "third checked allocation is the bounded header buffer",
    )?;
    assert_eq!(
        backend_code(error),
        Some((BackendFailureKind::HostMemory, CODE_HEADER_ALLOCATION))
    );

    fs::remove_file(first_path).map_err(|error| error.to_string())?;
    fs::remove_file(second_path).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn parser_allocation_failure_is_stable() -> Result<(), String> {
    let limits = InspectionLimits::PRODUCTION;
    let mut budget = InspectionBudget::new(&limits);
    TEST_INSPECTION_ALLOCATION_FAILURES.with(|remaining| remaining.set(1));
    let error = required_error(
        parse_header(
            BACKEND,
            br#"{"x":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
            &mut budget,
        ),
        "injected parser allocation must fail",
    )?;
    assert_eq!(
        backend_code(error),
        Some((BackendFailureKind::HostMemory, CODE_INSPECTION_ALLOCATION))
    );
    assert_eq!(
        backend_code(map_parse_failure(
            BACKEND,
            Some(super::ParseFailure::Allocation)
        )),
        Some((BackendFailureKind::HostMemory, CODE_INSPECTION_ALLOCATION))
    );
    Ok(())
}

fn write_safetensors(header: &[u8], payload: &[u8]) -> Result<std::path::PathBuf, String> {
    let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "milkdrift-candle-manifest-{}-{sequence}.safetensors",
        std::process::id()
    ));
    let mut file = File::create(&path).map_err(|error| error.to_string())?;
    file.write_all(
        &u64::try_from(header.len())
            .map_err(|error| error.to_string())?
            .to_le_bytes(),
    )
    .map_err(|error| error.to_string())?;
    file.write_all(header).map_err(|error| error.to_string())?;
    file.write_all(payload).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    Ok(path)
}
