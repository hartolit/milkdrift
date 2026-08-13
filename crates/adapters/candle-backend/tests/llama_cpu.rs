//! End-to-end and malformed-artifact tests for the Candle CPU Llama adapter.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use candle_backend::{
    CandleLlamaLoader, CandleLlamaModel, CandleLlamaPreparedLoad, CandleLlamaSequence,
    CandleLlamaSource, CandleShardIdentity, CandleWeightShard,
};
use candle_core::{DType, Device, Tensor};
use domain_contracts::{
    BackendFailureKind, BackendId, BackendSequence, CancellationStatus, CapacityResource,
    DecodeBuffers, DecodeInput, DecodeOutcome, DeviceId, DeviceKind, ExecutionDevice,
    LoadConfiguration, LoadError, LoadPlan, LoadedModel, MemoryBudget, MemoryFootprint,
    ModelGeneration, ModelHandle, ModelId, ModelLoader, PrefillBuffers, PrefillInput,
    PrefillOutcome, PreparedLoad, ScalarType, ScalarTypeSet, SequenceConfiguration, SequenceId,
    SequenceState, TokenId, decode_checked, prefill_checked,
};
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use sha2::{Digest, Sha256};

const BACKEND: BackendId = BackendId::new(1);
const REQUIRED_ELEMENTS: u64 = 920;
const VOCABULARY_SIZE: usize = 16;
const PER_SHARD_HEADER_LIMIT: u64 = 8 * 1024 * 1024;
const F32_SEQUENCE_CACHE_BYTES_PER_TOKEN: u64 = 64;
const F16_SEQUENCE_CACHE_BYTES_PER_TOKEN: u64 = 32;
const F32_SEQUENCE_PERSISTENT_BYTES: u64 = 1_504;
const F32_SEQUENCE_TRANSIENT_BYTES: u64 = 6_052;
const F32_SEQUENCE_HOST_WORKING_BYTES: u64 = 7_556;
const HALF_SEQUENCE_PERSISTENT_BYTES: u64 = 864;
const HALF_SEQUENCE_TRANSIENT_BYTES: u64 = 6_180;
const HALF_SEQUENCE_HOST_WORKING_BYTES: u64 = 7_044;

const CPU_F32_FINAL: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 3_680,
    device_weight_bytes: 0,
    host_working_bytes: 0,
    device_working_bytes: 0,
};
const CPU_F32_LOADING_PEAK: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 3_680,
    device_weight_bytes: 0,
    host_working_bytes: 65_763,
    device_working_bytes: 0,
};
const CPU_F16_FINAL: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 1_840,
    device_weight_bytes: 0,
    host_working_bytes: 0,
    device_working_bytes: 0,
};
const CPU_F16_LOADING_PEAK: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 1_840,
    device_weight_bytes: 0,
    host_working_bytes: 65_649,
    device_working_bytes: 0,
};
const CPU_MIXED_F16_F32_LOADING_PEAK: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 1_840,
    device_weight_bytes: 0,
    host_working_bytes: 65_665,
    device_working_bytes: 0,
};
const CPU_BF16_TO_F32_LOADING_PEAK: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 3_680,
    device_weight_bytes: 0,
    host_working_bytes: 65_632,
    device_working_bytes: 0,
};
const CPU_MIXED_BF16_F32_LOADING_PEAK: MemoryFootprint = MemoryFootprint {
    host_weight_bytes: 3_680,
    device_weight_bytes: 0,
    host_working_bytes: 65_664,
    device_working_bytes: 0,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

type TestResult<T = ()> = Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequiredProfile {
    F32,
    F16,
    Bf16,
    MixedF16F32,
    MixedBf16F32,
    MixedF16Bf16,
    UnsupportedU8,
}

impl RequiredProfile {
    fn dtype_for(self, name: &str) -> DType {
        match self {
            Self::F32 => DType::F32,
            Self::F16 => DType::F16,
            Self::Bf16 => DType::BF16,
            Self::MixedF16F32 => {
                if name == "model.norm.weight" {
                    DType::F32
                } else {
                    DType::F16
                }
            }
            Self::MixedBf16F32 => {
                if name == "model.norm.weight" {
                    DType::F32
                } else {
                    DType::BF16
                }
            }
            Self::MixedF16Bf16 => {
                if name == "model.norm.weight" {
                    DType::F16
                } else {
                    DType::BF16
                }
            }
            Self::UnsupportedU8 => {
                if name == "model.norm.weight" {
                    DType::U8
                } else {
                    DType::F32
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigDeclaration {
    Absent,
    F32,
    F16,
    Bf16,
    Unsupported,
    Conflict,
}

impl ConfigDeclaration {
    fn recognized(self) -> TestResult<Option<ScalarType>> {
        match self {
            Self::Absent => Ok(None),
            Self::F32 => Ok(Some(ScalarType::F32)),
            Self::F16 => Ok(Some(ScalarType::F16)),
            Self::Bf16 => Ok(Some(ScalarType::Bf16)),
            Self::Unsupported | Self::Conflict => {
                Err("test requested a recognized value for an invalid declaration".to_owned())
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ExtraTensor {
    name: &'static str,
    dtype: &'static str,
    elements: usize,
    bytes_per_element: usize,
}

impl ExtraTensor {
    const fn new(
        name: &'static str,
        dtype: &'static str,
        elements: usize,
        bytes_per_element: usize,
    ) -> Self {
        Self {
            name,
            dtype,
            elements,
            bytes_per_element,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum PreparedMutation {
    Payload,
    SameLengthHeader,
    Truncate,
    Extend,
}

#[test]
fn homogeneous_f32_f16_and_bf16_preserve_execution_behavior() -> TestResult {
    for (profile, declaration, observed, expected_execution, expected_final, expected_loading) in [
        (
            RequiredProfile::F32,
            ConfigDeclaration::F32,
            scalar_set(&[ScalarType::F32]),
            ScalarType::F32,
            CPU_F32_FINAL,
            CPU_F32_LOADING_PEAK,
        ),
        (
            RequiredProfile::F16,
            ConfigDeclaration::F16,
            scalar_set(&[ScalarType::F16]),
            ScalarType::F16,
            CPU_F16_FINAL,
            CPU_F16_LOADING_PEAK,
        ),
        (
            RequiredProfile::Bf16,
            ConfigDeclaration::Bf16,
            scalar_set(&[ScalarType::Bf16]),
            ScalarType::F32,
            CPU_F32_FINAL,
            CPU_BF16_TO_F32_LOADING_PEAK,
        ),
    ] {
        execute_profile(
            profile,
            &[],
            declaration,
            observed,
            expected_execution,
            expected_final,
            expected_loading,
        )?;
    }
    Ok(())
}

#[test]
fn f32_required_ignores_f16_bf16_and_combined_extras_with_declared_and_absent_configs() -> TestResult
{
    let cases = [
        (
            vec![ExtraTensor::new("unused.f16", "F16", 3, 2)],
            scalar_set(&[ScalarType::F32, ScalarType::F16]),
        ),
        (
            vec![ExtraTensor::new("unused.bf16", "BF16", 5, 2)],
            scalar_set(&[ScalarType::F32, ScalarType::Bf16]),
        ),
        (
            vec![
                ExtraTensor::new("unused.f16", "F16", 3, 2),
                ExtraTensor::new("unused.bf16", "BF16", 5, 2),
            ],
            scalar_set(&[ScalarType::F32, ScalarType::F16, ScalarType::Bf16]),
        ),
    ];

    for (extras, observed) in cases {
        for declaration in [ConfigDeclaration::F32, ConfigDeclaration::Absent] {
            execute_profile(
                RequiredProfile::F32,
                extras.as_slice(),
                declaration,
                observed,
                ScalarType::F32,
                CPU_F32_FINAL,
                CPU_F32_LOADING_PEAK,
            )?;
        }
    }
    Ok(())
}

#[test]
fn mixed_required_f16_f32_ignores_bf16_u8_bool_and_other_extras() -> TestResult {
    let extras = [
        ExtraTensor::new("unused.bf16", "BF16", 2, 2),
        ExtraTensor::new("unused.u8", "U8", 3, 1),
        ExtraTensor::new("unused.bool", "BOOL", 4, 1),
        ExtraTensor::new("unused.f64", "F64", 1, 8),
    ];
    execute_profile(
        RequiredProfile::MixedF16F32,
        &extras,
        ConfigDeclaration::F16,
        scalar_set(&[
            ScalarType::F32,
            ScalarType::F16,
            ScalarType::Bf16,
            ScalarType::U8,
            ScalarType::Other(1),
        ]),
        ScalarType::F16,
        CPU_F16_FINAL,
        CPU_MIXED_F16_F32_LOADING_PEAK,
    )
}

#[test]
fn mixed_required_bf16_f32_ignores_f16_u8_bool_and_other_extras() -> TestResult {
    let extras = [
        ExtraTensor::new("unused.f16", "F16", 2, 2),
        ExtraTensor::new("unused.u8", "U8", 3, 1),
        ExtraTensor::new("unused.bool", "BOOL", 4, 1),
        ExtraTensor::new("unused.f64", "F64", 1, 8),
    ];
    execute_profile(
        RequiredProfile::MixedBf16F32,
        &extras,
        ConfigDeclaration::Bf16,
        scalar_set(&[
            ScalarType::F32,
            ScalarType::F16,
            ScalarType::Bf16,
            ScalarType::U8,
            ScalarType::Other(1),
        ]),
        ScalarType::F32,
        CPU_F32_FINAL,
        CPU_MIXED_BF16_F32_LOADING_PEAK,
    )
}

#[test]
fn complete_observed_set_includes_i8_u8_and_other_while_unused_extras_load() -> TestResult {
    let extras = [
        ExtraTensor::new("unused.i8", "I8", 2, 1),
        ExtraTensor::new("unused.u8", "U8", 2, 1),
        ExtraTensor::new("unused.bool", "BOOL", 2, 1),
        ExtraTensor::new("unused.f64", "F64", 1, 8),
    ];
    execute_profile(
        RequiredProfile::F32,
        &extras,
        ConfigDeclaration::F32,
        scalar_set(&[
            ScalarType::F32,
            ScalarType::I8,
            ScalarType::U8,
            ScalarType::Other(17),
        ]),
        ScalarType::F32,
        CPU_F32_FINAL,
        CPU_F32_LOADING_PEAK,
    )
}

#[test]
fn huge_ignored_extra_does_not_change_exact_cpu_footprints_or_working_bytes() -> TestResult {
    assert_eq!(CPU_F32_FINAL.host_weight_bytes, REQUIRED_ELEMENTS * 4);
    assert_eq!(CPU_F16_FINAL.host_weight_bytes, REQUIRED_ELEMENTS * 2);
    let base = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let base_source = base.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let base_prepared = loader
        .prepare_load(&base_source, &load_configuration())
        .map_err(debug_error)?;
    let base_plan = *base_prepared.plan();
    drop(base_prepared);

    let extras = [
        ExtraTensor::new("unused.huge.f16", "F16", 1_048_576, 2),
        ExtraTensor::new("unused.non_executable.u8", "U8", 32, 1),
    ];
    let huge = TinyLlamaFixture::create(RequiredProfile::F32, &extras)?;
    let huge_source = huge.source(ConfigDeclaration::F32)?;
    let descriptor = loader.inspect(&huge_source).map_err(debug_error)?;
    let (huge_plan, mut model) = prepare_and_load(&mut loader, &huge_source, load_configuration())?;

    assert_eq!(base_plan.final_footprint, CPU_F32_FINAL);
    assert_eq!(base_plan.loading_peak_footprint, CPU_F32_LOADING_PEAK);
    assert_eq!(huge_plan.final_footprint, base_plan.final_footprint);
    assert_eq!(
        huge_plan.loading_peak_footprint,
        base_plan.loading_peak_footprint
    );
    assert_eq!(descriptor.estimated_footprint, CPU_F32_FINAL);
    assert_eq!(
        descriptor.sequence_cache_bytes_per_token,
        F32_SEQUENCE_CACHE_BYTES_PER_TOKEN
    );
    assert_eq!(huge_plan.loading_peak_footprint.device_working_bytes, 0);
    assert_eq!(model.reported_footprint(), CPU_F32_FINAL);
    clean_model(&mut model)
}

#[test]
fn configuration_declaration_must_match_required_primary() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::MixedF16F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_unsupported(loader.inspect(&source));

    let f32_fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    for declaration in [ConfigDeclaration::F16, ConfigDeclaration::Bf16] {
        let source = f32_fixture.source(declaration)?;
        assert_unsupported(loader.inspect(&source));
    }
    Ok(())
}

#[test]
fn mixed_required_layouts_require_matching_configuration_declarations() -> TestResult {
    for (profile, declaration) in [
        (RequiredProfile::MixedF16F32, ConfigDeclaration::F16),
        (RequiredProfile::MixedBf16F32, ConfigDeclaration::Bf16),
    ] {
        let fixture = TinyLlamaFixture::create(profile, &[])?;
        let absent = fixture.source(ConfigDeclaration::Absent)?;
        let mut loader = CandleLlamaLoader::new(BACKEND);
        assert_unsupported(loader.inspect(&absent));
        assert_unsupported(loader.prepare_load(&absent, &load_configuration()));

        let declared = fixture.source(declaration)?;
        loader.inspect(&declared).map_err(debug_error)?;
        let prepared = loader
            .prepare_load(&declared, &load_configuration())
            .map_err(debug_error)?;
        drop(prepared);
    }
    Ok(())
}

#[test]
fn owned_config_bytes_remain_bound_while_local_paths_stay_late_bound() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let local_source = fixture.source(ConfigDeclaration::F32)?;
    let config_bytes = fs::read(&fixture.config_path).map_err(|error| error.to_string())?;
    let bound_source = CandleLlamaSource::from_config_bytes(
        config_bytes,
        vec![CandleWeightShard::unverified(fixture.weight_path.clone())],
    )
    .map_err(|error| error.to_string())?;

    write_tiny_config(&fixture.config_path, ConfigDeclaration::Unsupported)?;
    let loader = CandleLlamaLoader::new(BACKEND);
    let descriptor = loader.inspect(&bound_source).map_err(debug_error)?;
    assert_eq!(
        descriptor.metadata.configuration_declared_scalar_type,
        Some(ScalarType::F32)
    );
    assert_unsupported(loader.inspect(&local_source));
    Ok(())
}

#[test]
fn unsupported_and_conflicting_config_declarations_are_rejected() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let loader = CandleLlamaLoader::new(BACKEND);
    for declaration in [ConfigDeclaration::Unsupported, ConfigDeclaration::Conflict] {
        let source = fixture.source(declaration)?;
        assert_unsupported(loader.inspect(&source));
    }
    Ok(())
}

#[test]
fn genuine_required_f16_bf16_mixture_rejects_before_device_initialization() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::MixedF16Bf16, &[])?;
    let source = fixture.source(ConfigDeclaration::Absent)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let mut configuration = load_configuration();
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(1), DeviceKind::Cpu);

    assert_unsupported(loader.inspect(&source));
    assert_unsupported(loader.prepare_load(&source, &configuration));
    Ok(())
}

#[test]
fn required_unsupported_dtype_rejects_before_device_initialization() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::UnsupportedU8, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let mut configuration = load_configuration();
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(1), DeviceKind::Cpu);

    assert_unsupported(loader.inspect(&source));
    assert_unsupported(loader.prepare_load(&source, &configuration));
    Ok(())
}

#[test]
fn host_budget_rejects_exact_required_only_loading_peak() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    assert_eq!(CPU_F32_LOADING_PEAK.checked_host_bytes(), Some(69_443));

    let mut constrained = load_configuration();
    constrained.memory_budget.host_bytes = 69_442;
    assert!(matches!(
        loader.prepare_load(&source, &constrained),
        Err(LoadError::InsufficientMemory {
            kind: domain_contracts::MemoryKind::Host,
            required_bytes: 69_443,
            available_bytes: 69_442,
        })
    ));
    Ok(())
}

#[test]
fn prepared_load_consumes_retained_file_after_path_deletion() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let prepared = loader
        .prepare_load(&source, &load_configuration())
        .map_err(debug_error)?;
    let accepted_plan = *prepared.plan();
    fs::remove_file(&fixture.weight_path)
        .map_err(|error| format!("remove prepared path: {error}"))?;

    let mut model = load_exact_preparation(&mut loader, prepared)?;
    assert_eq!(model.reported_footprint(), accepted_plan.final_footprint);
    clean_model(&mut model)
}

#[test]
fn unverified_fallback_detects_same_inode_payload_mutation() -> TestResult {
    assert_unverified_prepared_mutation_rejected(PreparedMutation::Payload)
}

#[test]
fn unverified_fallback_detects_same_length_header_mutation() -> TestResult {
    assert_unverified_prepared_mutation_rejected(PreparedMutation::SameLengthHeader)
}

#[test]
fn unverified_fallback_detects_truncation_and_extension() -> TestResult {
    for mutation in [PreparedMutation::Truncate, PreparedMutation::Extend] {
        assert_unverified_prepared_mutation_rejected(mutation)?;
    }
    Ok(())
}

#[test]
fn supplied_verified_immutable_identity_succeeds_and_digest_mismatch_rejects() -> TestResult {
    let verified = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let (byte_length, sha256) = file_identity(&verified.weight_path)?;
    let source = verified.source_with_shards(
        ConfigDeclaration::F32,
        vec![CandleWeightShard::new(
            verified.weight_path.clone(),
            CandleShardIdentity::VerifiedImmutable {
                byte_length,
                sha256,
            },
        )],
    )?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let (_plan, mut model) = prepare_and_load(&mut loader, &source, load_configuration())?;
    clean_model(&mut model)?;

    let mismatched = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let (byte_length, mut sha256) = file_identity(&mismatched.weight_path)?;
    sha256[0] ^= 1;
    let source = mismatched.source_with_shards(
        ConfigDeclaration::F32,
        vec![CandleWeightShard::new(
            mismatched.weight_path.clone(),
            CandleShardIdentity::VerifiedImmutable {
                byte_length,
                sha256,
            },
        )],
    )?;
    let prepared = loader
        .prepare_load(&source, &load_configuration())
        .map_err(debug_error)?;
    assert_failed_preparation_invalid_model(&mut loader, prepared)
}

#[test]
fn project_established_mutation_rejects_before_invalid_device_initialization() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let (byte_length, sha256) = file_identity(&fixture.weight_path)?;
    let source = fixture.source_with_shards(
        ConfigDeclaration::F32,
        vec![CandleWeightShard::new(
            fixture.weight_path.clone(),
            CandleShardIdentity::ProjectEstablished {
                byte_length,
                sha256,
            },
        )],
    )?;
    mutate_prepared_file(&fixture.weight_path, PreparedMutation::Payload)?;

    let mut configuration = load_configuration();
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(1), DeviceKind::Cpu);
    let mut loader = CandleLlamaLoader::new(BACKEND);
    assert!(matches!(
        loader.prepare_load(&source, &configuration),
        Err(LoadError::Backend(failure))
            if failure.kind == BackendFailureKind::InvalidModel
    ));
    Ok(())
}

#[test]
fn same_header_same_path_and_distinct_cross_shard_duplicates_are_rejected() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let duplicate_path_source = fixture.source_with_paths(
        ConfigDeclaration::F32,
        vec![fixture.weight_path.clone(), fixture.weight_path.clone()],
    )?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_invalid_model(loader.inspect(&duplicate_path_source));

    let same_header = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    write_raw_safetensors(
        &same_header.weight_path,
        r#"{"dup":{"dtype":"F32","shape":[0],"data_offsets":[0,0]},"dup":{"dtype":"F32","shape":[0],"data_offsets":[0,0]}}"#,
        &[],
    )?;
    let same_header_source = same_header.source(ConfigDeclaration::F32)?;
    assert_invalid_model(loader.inspect(&same_header_source));

    let cross_shard = TinyLlamaFixture::create_sharded(RequiredProfile::F32, true)?;
    let cross_shard_source = cross_shard.source_with_paths(
        ConfigDeclaration::F32,
        vec![
            cross_shard.weight_path.clone(),
            cross_shard.second_weight_path.clone(),
        ],
    )?;
    assert_invalid_model(loader.inspect(&cross_shard_source));
    Ok(())
}

#[test]
fn excessive_shard_count_is_rejected_before_files_are_opened() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source_with_paths(
        ConfigDeclaration::F32,
        vec![fixture.weight_path.clone(); 257],
    )?;
    fs::remove_file(&fixture.weight_path).map_err(|error| error.to_string())?;
    let loader = CandleLlamaLoader::new(BACKEND);

    assert!(matches!(
        loader.inspect(&source),
        Err(LoadError::CapacityExhausted(capacity))
            if capacity.resource == CapacityResource::BackendScratch
                && capacity.required == 257
                && capacity.available == 256
    ));
    Ok(())
}

#[test]
fn malformed_truncated_and_eight_mib_plus_one_headers_are_rejected() -> TestResult {
    assert_raw_header_rejected(10_u64.to_le_bytes(), b"{}", &[])?;
    assert_raw_header_rejected((PER_SHARD_HEADER_LIMIT + 1).to_le_bytes(), &[], &[])?;
    assert_raw_header_rejected(8_u64.to_le_bytes(), b"not-json", &[])?;
    assert_raw_bytes_rejected(b"short")?;
    Ok(())
}

#[test]
fn overlap_explicit_gap_bounds_shape_mismatch_and_overflow_are_rejected() -> TestResult {
    for (header, payload) in [
        (
            r#"{"a":{"dtype":"F32","shape":[1],"data_offsets":[0,4]},"b":{"dtype":"F32","shape":[1],"data_offsets":[3,7]}}"#.to_owned(),
            vec![0_u8; 7],
        ),
        (
            r#"{"a":{"dtype":"F32","shape":[1],"data_offsets":[0,4]},"b":{"dtype":"F32","shape":[1],"data_offsets":[5,9]}}"#.to_owned(),
            vec![0_u8; 9],
        ),
        (
            r#"{"a":{"dtype":"F32","shape":[2],"data_offsets":[0,4]}}"#.to_owned(),
            vec![0_u8; 4],
        ),
        (
            r#"{"a":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#.to_owned(),
            vec![0_u8; 3],
        ),
        (
            r#"{"a":{"dtype":"F32","shape":[18446744073709551615,2],"data_offsets":[0,0]}}"#.to_owned(),
            Vec::new(),
        ),
    ] {
        assert_valid_prefix_header_rejected(header.as_str(), payload.as_slice())?;
    }
    Ok(())
}

#[test]
fn tensor_name_rank_and_metadata_limits_are_rejected() -> TestResult {
    let long_name = "n".repeat(513);
    let name_header =
        format!(r#"{{"{long_name}":{{"dtype":"F32","shape":[0],"data_offsets":[0,0]}}}}"#);
    assert_valid_prefix_header_rejected(name_header.as_str(), &[])?;

    let rank_header =
        r#"{"rank":{"dtype":"F32","shape":[1,1,1,1,1,1,1,1,1],"data_offsets":[0,4]}}"#;
    assert_valid_prefix_header_rejected(rank_header, &[0_u8; 4])?;

    let metadata_header = serde_json::to_string(&json!({
        "__metadata__": {"key": "v".repeat(4 * 1024 + 1)}
    }))
    .map_err(|error| error.to_string())?;
    assert_valid_prefix_header_rejected(metadata_header.as_str(), &[])?;
    Ok(())
}

#[test]
fn shard_reorder_sorts_complete_identity_pairs_and_loads() -> TestResult {
    let fixture = TinyLlamaFixture::create_sharded(RequiredProfile::MixedF16F32, false)?;
    let (z_length, z_sha256) = file_identity(&fixture.weight_path)?;
    let (a_length, a_sha256) = file_identity(&fixture.second_weight_path)?;
    let source = fixture.source_with_shards(
        ConfigDeclaration::F16,
        vec![
            CandleWeightShard::new(
                fixture.weight_path.clone(),
                CandleShardIdentity::VerifiedImmutable {
                    byte_length: z_length,
                    sha256: z_sha256,
                },
            ),
            CandleWeightShard::new(
                fixture.second_weight_path.clone(),
                CandleShardIdentity::ProjectEstablished {
                    byte_length: a_length,
                    sha256: a_sha256,
                },
            ),
        ],
    )?;

    let [first, second] = source.weight_shards() else {
        return Err("sharded source did not retain exactly two identity pairs".to_owned());
    };
    assert_eq!(first.path(), fixture.second_weight_path);
    assert_eq!(
        first.identity(),
        CandleShardIdentity::ProjectEstablished {
            byte_length: a_length,
            sha256: a_sha256,
        }
    );
    assert_eq!(second.path(), fixture.weight_path);
    assert_eq!(
        second.identity(),
        CandleShardIdentity::VerifiedImmutable {
            byte_length: z_length,
            sha256: z_sha256,
        }
    );

    let mut loader = CandleLlamaLoader::new(BACKEND);
    let (plan, mut model) = prepare_and_load(&mut loader, &source, load_configuration())?;
    assert_eq!(plan.final_footprint, CPU_F16_FINAL);
    assert_eq!(plan.loading_peak_footprint, CPU_MIXED_F16_F32_LOADING_PEAK);
    clean_model(&mut model)
}

#[test]
fn rejects_invalid_cpu_identity_and_unsupported_devices() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let mut configuration = load_configuration();
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(1), DeviceKind::Cpu);
    assert!(matches!(
        loader.prepare_load(&source, &configuration),
        Err(LoadError::InvalidConfiguration)
    ));

    for kind in [DeviceKind::Metal, DeviceKind::Accelerator(1)] {
        configuration.execution_device = ExecutionDevice::new(DeviceId::new(0), kind);
        assert!(matches!(
            loader.prepare_load(&source, &configuration),
            Err(LoadError::Backend(failure))
                if failure.kind == BackendFailureKind::Unsupported
        ));
    }
    Ok(())
}

#[cfg(not(feature = "cuda"))]
#[test]
fn cuda_request_fails_explicitly_when_support_is_not_compiled() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let mut configuration = load_configuration();
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);
    configuration.memory_budget.device_bytes = u64::MAX;

    assert!(matches!(
        loader.prepare_load(&source, &configuration),
        Err(LoadError::Backend(failure))
            if failure.kind == BackendFailureKind::Unsupported
    ));
    Ok(())
}

fn execute_profile(
    profile: RequiredProfile,
    extras: &[ExtraTensor],
    declaration: ConfigDeclaration,
    expected_observed: ScalarTypeSet,
    expected_execution: ScalarType,
    expected_final: MemoryFootprint,
    expected_loading: MemoryFootprint,
) -> TestResult {
    let fixture = TinyLlamaFixture::create(profile, extras)?;
    let source = fixture.source(declaration)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let descriptor = loader.inspect(&source).map_err(debug_error)?;
    assert_eq!(
        descriptor.metadata.configuration_declared_scalar_type,
        declaration.recognized()?
    );
    assert_eq!(
        descriptor.metadata.observed_tensor_scalar_types,
        expected_observed
    );
    assert_eq!(descriptor.estimated_footprint, expected_final);
    let expected_cache_rate = match expected_execution {
        ScalarType::F32 => F32_SEQUENCE_CACHE_BYTES_PER_TOKEN,
        ScalarType::F16 | ScalarType::Bf16 => F16_SEQUENCE_CACHE_BYTES_PER_TOKEN,
        _ => return Err("test profile selected a non-floating execution scalar".to_owned()),
    };
    assert_eq!(
        descriptor.sequence_cache_bytes_per_token,
        expected_cache_rate
    );

    let (plan, mut model) = prepare_and_load(&mut loader, &source, load_configuration())?;
    assert_eq!(plan.descriptor, descriptor);
    assert_eq!(plan.execution_scalar_type, expected_execution);
    assert_eq!(plan.final_footprint, expected_final);
    assert_eq!(plan.loading_peak_footprint, expected_loading);
    assert_eq!(plan.loading_peak_footprint.device_working_bytes, 0);
    assert_eq!(model.descriptor(), &descriptor);
    assert_eq!(model.execution_scalar_type(), expected_execution);
    assert_eq!(model.reported_footprint(), expected_final);
    exercise_model(&mut model)?;
    clean_model(&mut model)
}

fn exercise_model(model: &mut CandleLlamaModel) -> TestResult {
    let configuration = SequenceConfiguration::new(
        NonZeroU32::new(16).ok_or_else(|| "maximum tokens must be nonzero".to_owned())?,
        NonZeroU32::new(8).ok_or_else(|| "maximum prefill must be nonzero".to_owned())?,
    );
    let sequence_plan = model.plan_sequence(&configuration).map_err(debug_error)?;
    assert_cpu_sequence_reservation(model, sequence_plan)?;

    let mut first = model
        .create_sequence(SequenceId::new(1), &configuration)
        .map_err(debug_error)?;
    let mut second = model
        .create_sequence(SequenceId::new(2), &configuration)
        .map_err(debug_error)?;
    assert_eq!(first.reservation(), sequence_plan.reservation);
    assert_eq!(second.reservation(), sequence_plan.reservation);
    assert_eq!(first.reported_plan(), sequence_plan);
    assert_eq!(second.reported_plan(), sequence_plan);
    assert_eq!(first.token_staging_capacity(), 8);
    assert_eq!(first.token_staging_logical_bytes(), 32);
    exercise_repeated_prefill(model, &mut first)?;
    exercise_maximum_prefill_and_near_capacity_decode(model, &mut second)?;
    exercise_first_decode(model, &mut first)?;
    exercise_mask_free_prefill_and_decode(model)?;

    model.destroy_sequence(&mut first).map_err(debug_error)?;
    model.destroy_sequence(&mut second).map_err(debug_error)?;
    assert_eq!(first.state(), SequenceState::Finished);
    assert_eq!(second.state(), SequenceState::Finished);
    Ok(())
}

fn assert_cpu_sequence_reservation(
    model: &CandleLlamaModel,
    plan: domain_contracts::SequencePlan,
) -> TestResult {
    let expected = match model.execution_scalar_type() {
        ScalarType::F32 => (
            F32_SEQUENCE_PERSISTENT_BYTES,
            F32_SEQUENCE_TRANSIENT_BYTES,
            F32_SEQUENCE_HOST_WORKING_BYTES,
        ),
        ScalarType::F16 | ScalarType::Bf16 => (
            HALF_SEQUENCE_PERSISTENT_BYTES,
            HALF_SEQUENCE_TRANSIENT_BYTES,
            HALF_SEQUENCE_HOST_WORKING_BYTES,
        ),
        _ => return Err("test model selected a non-floating execution scalar".to_owned()),
    };
    for (footprint, host_working_bytes) in [
        (plan.reservation.persistent_footprint, expected.0),
        (plan.reservation.transient_footprint, expected.1),
        (plan.reservation.total_footprint, expected.2),
    ] {
        assert_eq!(
            footprint,
            MemoryFootprint {
                host_weight_bytes: 0,
                device_weight_bytes: 0,
                host_working_bytes,
                device_working_bytes: 0,
            }
        );
    }
    Ok(())
}

fn exercise_repeated_prefill(
    model: &mut CandleLlamaModel,
    sequence: &mut CandleLlamaSequence,
) -> TestResult {
    let mut logits = [0.0_f32; VOCABULARY_SIZE];
    for (tokens, expected_position) in [
        ([TokenId::new(1), TokenId::new(2)], 2),
        ([TokenId::new(3), TokenId::new(4)], 4),
    ] {
        let outcome = prefill_checked(
            model,
            sequence,
            PrefillInput::new(&tokens, true),
            PrefillBuffers::new(&mut logits),
            CancellationStatus::Running,
        )
        .map_err(debug_error)?;
        assert_eq!(
            outcome,
            PrefillOutcome::Ready {
                consumed_tokens: 2,
                position: expected_position,
                logits_written: VOCABULARY_SIZE,
            }
        );
        assert_eq!(maximum_logit_token(&logits)?, tokens[1]);
    }
    assert_eq!(sequence.state(), SequenceState::Ready);
    Ok(())
}

fn exercise_maximum_prefill_and_near_capacity_decode(
    model: &mut CandleLlamaModel,
    sequence: &mut CandleLlamaSequence,
) -> TestResult {
    let mut logits = [0.0_f32; VOCABULARY_SIZE];
    let prompt = [TokenId::new(6); 8];
    assert_eq!(
        prefill_checked(
            model,
            sequence,
            PrefillInput::new(&prompt, true),
            PrefillBuffers::new(&mut logits),
            CancellationStatus::Running,
        )
        .map_err(debug_error)?,
        PrefillOutcome::Ready {
            consumed_tokens: 8,
            position: 8,
            logits_written: VOCABULARY_SIZE,
        }
    );
    for expected_position in 9..=16 {
        assert_eq!(
            decode_checked(
                model,
                sequence,
                DecodeInput::new(TokenId::new(7)),
                DecodeBuffers::new(&mut logits),
                CancellationStatus::Running,
            )
            .map_err(debug_error)?,
            DecodeOutcome::Ready {
                position: expected_position,
                logits_written: VOCABULARY_SIZE,
            }
        );
    }
    Ok(())
}

fn exercise_first_decode(
    model: &mut CandleLlamaModel,
    sequence: &mut CandleLlamaSequence,
) -> TestResult {
    let mut logits = [0.0_f32; VOCABULARY_SIZE];
    assert_eq!(
        decode_checked(
            model,
            sequence,
            DecodeInput::new(TokenId::new(5)),
            DecodeBuffers::new(&mut logits),
            CancellationStatus::Running,
        )
        .map_err(debug_error)?,
        DecodeOutcome::Ready {
            position: 5,
            logits_written: VOCABULARY_SIZE,
        }
    );
    assert_eq!(maximum_logit_token(&logits)?, TokenId::new(5));
    Ok(())
}

fn exercise_mask_free_prefill_and_decode(model: &mut CandleLlamaModel) -> TestResult {
    let configuration = SequenceConfiguration::new(
        NonZeroU32::new(4).ok_or_else(|| "maximum tokens must be nonzero".to_owned())?,
        NonZeroU32::MIN,
    );
    let plan = model.plan_sequence(&configuration).map_err(debug_error)?;
    let mut sequence = model
        .create_sequence(SequenceId::new(3), &configuration)
        .map_err(debug_error)?;
    assert_eq!(sequence.reported_plan(), plan);
    assert_eq!(sequence.token_staging_capacity(), 1);
    assert_eq!(sequence.token_staging_logical_bytes(), 4);
    prefill_checked(
        model,
        &mut sequence,
        PrefillInput::new(&[TokenId::new(8)], false),
        PrefillBuffers::new(&mut []),
        CancellationStatus::Running,
    )
    .map_err(debug_error)?;
    let mut logits = [0.0_f32; VOCABULARY_SIZE];
    decode_checked(
        model,
        &mut sequence,
        DecodeInput::new(TokenId::new(9)),
        DecodeBuffers::new(&mut logits),
        CancellationStatus::Running,
    )
    .map_err(debug_error)?;
    model.destroy_sequence(&mut sequence).map_err(debug_error)?;
    assert_eq!(sequence.state(), SequenceState::Finished);
    Ok(())
}

fn clean_model(model: &mut CandleLlamaModel) -> TestResult {
    model.synchronize().map_err(debug_error)?;
    model.prepare_unload().map_err(debug_error)
}

fn prepare_and_load(
    loader: &mut CandleLlamaLoader,
    source: &CandleLlamaSource,
    configuration: LoadConfiguration,
) -> TestResult<(LoadPlan, CandleLlamaModel)> {
    let prepared = loader
        .prepare_load(source, &configuration)
        .map_err(debug_error)?;
    let plan = *prepared.plan();
    let model = load_exact_preparation(loader, prepared)?;
    Ok((plan, model))
}

fn load_exact_preparation(
    loader: &mut CandleLlamaLoader,
    prepared: CandleLlamaPreparedLoad,
) -> TestResult<CandleLlamaModel> {
    match loader.load_prepared(prepared) {
        Ok(model) => Ok(model),
        Err(mut failed) => {
            let primary = failed.primary();
            let cleanup = failed.cleanup();
            Err(format!(
                "prepared load failed: {primary:?}; cleanup: {cleanup:?}"
            ))
        }
    }
}

fn assert_failed_preparation_invalid_model(
    loader: &mut CandleLlamaLoader,
    prepared: CandleLlamaPreparedLoad,
) -> TestResult {
    let failed = match loader.load_prepared(prepared) {
        Err(failed) => failed,
        Ok(mut model) => {
            clean_model(&mut model)?;
            return Err("mutated or mismatched preparation unexpectedly loaded".to_owned());
        }
    };
    assert!(matches!(
        failed.primary(),
        LoadError::Backend(failure) if failure.kind == BackendFailureKind::InvalidModel
    ));
    let mut failed = failed;
    failed.cleanup().map_err(debug_error)?;
    failed.cleanup().map_err(debug_error)
}

fn assert_unverified_prepared_mutation_rejected(mutation: PreparedMutation) -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    assert!(
        source
            .weight_shards()
            .iter()
            .all(|shard| shard.identity() == CandleShardIdentity::Unverified)
    );
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let prepared = loader
        .prepare_load(&source, &load_configuration())
        .map_err(debug_error)?;
    mutate_prepared_file(&fixture.weight_path, mutation)?;
    assert_failed_preparation_invalid_model(&mut loader, prepared)
}

fn mutate_prepared_file(path: &Path, mutation: PreparedMutation) -> TestResult {
    match mutation {
        PreparedMutation::Payload => {
            let mut file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|error| format!("open payload for mutation: {error}"))?;
            file.seek(SeekFrom::End(-1))
                .map_err(|error| format!("seek payload: {error}"))?;
            let mut final_byte = [0_u8; 1];
            file.read_exact(&mut final_byte)
                .map_err(|error| format!("read payload: {error}"))?;
            final_byte[0] ^= 1;
            file.seek(SeekFrom::End(-1))
                .map_err(|error| format!("reseek payload: {error}"))?;
            file.write_all(&final_byte)
                .map_err(|error| format!("write payload mutation: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("sync payload mutation: {error}"))
        }
        PreparedMutation::SameLengthHeader => {
            let bytes = fs::read(path).map_err(|error| error.to_string())?;
            let header_length = read_header_length(bytes.as_slice())?;
            let header = bytes
                .get(8..8 + header_length)
                .ok_or_else(|| "fixture header is truncated".to_owned())?;
            let offset = header
                .iter()
                .rposition(|byte| *byte == b' ')
                .ok_or_else(|| "fixture header has no padding byte to mutate".to_owned())?;
            let absolute = 8_u64
                .checked_add(u64::try_from(offset).map_err(|error| error.to_string())?)
                .ok_or_else(|| "header mutation offset overflow".to_owned())?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(|error| format!("open header for mutation: {error}"))?;
            file.seek(SeekFrom::Start(absolute))
                .map_err(|error| format!("seek header: {error}"))?;
            file.write_all(b"\n")
                .map_err(|error| format!("write header mutation: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("sync header mutation: {error}"))
        }
        PreparedMutation::Truncate => {
            let file = fs::OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(|error| format!("open file for truncation: {error}"))?;
            let length = file
                .metadata()
                .map_err(|error| format!("read length for truncation: {error}"))?
                .len();
            file.set_len(
                length
                    .checked_sub(1)
                    .ok_or_else(|| "cannot truncate empty fixture".to_owned())?,
            )
            .map_err(|error| format!("truncate fixture: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("sync truncation: {error}"))
        }
        PreparedMutation::Extend => {
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(path)
                .map_err(|error| format!("open file for extension: {error}"))?;
            file.write_all(&[0])
                .map_err(|error| format!("extend fixture: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("sync extension: {error}"))
        }
    }
}

const fn load_configuration() -> LoadConfiguration {
    LoadConfiguration {
        handle: ModelHandle::new(ModelId::new(9), ModelGeneration::new(1)),
        execution_device: ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu),
        memory_budget: MemoryBudget {
            host_bytes: u64::MAX,
            device_bytes: 0,
        },
    }
}

struct TinyLlamaFixture {
    directory: PathBuf,
    config_path: PathBuf,
    weight_path: PathBuf,
    second_weight_path: PathBuf,
}

impl TinyLlamaFixture {
    fn create(profile: RequiredProfile, extras: &[ExtraTensor]) -> TestResult<Self> {
        let fixture = Self::empty()?;
        let tensors = create_weight_tensors(profile)?;
        candle_core::safetensors::save(&tensors, &fixture.weight_path)
            .map_err(|error| format!("save weights: {error}"))?;
        append_raw_extras(&fixture.weight_path, extras)?;
        Ok(fixture)
    }

    fn create_sharded(profile: RequiredProfile, duplicate_extra: bool) -> TestResult<Self> {
        let fixture = Self::empty()?;
        let tensors = create_weight_tensors(profile)?;
        let mut first = HashMap::new();
        let mut second = HashMap::new();
        for (name, tensor) in tensors {
            let destination = if name.contains("model.layers") {
                &mut second
            } else {
                &mut first
            };
            if destination.insert(name, tensor).is_some() {
                return Err("duplicate sharded tensor".to_owned());
            }
        }
        if duplicate_extra {
            let name = "unused.cross_shard_duplicate".to_owned();
            let first_duplicate =
                Tensor::zeros(1, DType::F32, &Device::Cpu).map_err(|error| error.to_string())?;
            let second_duplicate =
                Tensor::zeros(1, DType::F32, &Device::Cpu).map_err(|error| error.to_string())?;
            first.insert(name.clone(), first_duplicate);
            second.insert(name, second_duplicate);
        }
        candle_core::safetensors::save(&first, &fixture.second_weight_path)
            .map_err(|error| format!("save first shard: {error}"))?;
        candle_core::safetensors::save(&second, &fixture.weight_path)
            .map_err(|error| format!("save second shard: {error}"))?;
        Ok(fixture)
    }

    fn empty() -> TestResult<Self> {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "milkdrift-candle-loader-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let config_path = directory.join("config.json");
        let weight_path = directory.join("z-model.safetensors");
        let second_weight_path = directory.join("a-model.safetensors");
        write_tiny_config(&config_path, ConfigDeclaration::Absent)?;
        Ok(Self {
            directory,
            config_path,
            weight_path,
            second_weight_path,
        })
    }

    fn source(&self, declaration: ConfigDeclaration) -> TestResult<CandleLlamaSource> {
        self.source_with_paths(declaration, vec![self.weight_path.clone()])
    }

    fn source_with_paths(
        &self,
        declaration: ConfigDeclaration,
        paths: Vec<PathBuf>,
    ) -> TestResult<CandleLlamaSource> {
        write_tiny_config(&self.config_path, declaration)?;
        CandleLlamaSource::from_local_files(self.config_path.clone(), paths)
            .map_err(|error| error.to_string())
    }

    fn source_with_shards(
        &self,
        declaration: ConfigDeclaration,
        shards: Vec<CandleWeightShard>,
    ) -> TestResult<CandleLlamaSource> {
        write_tiny_config(&self.config_path, declaration)?;
        CandleLlamaSource::new(self.config_path.clone(), shards).map_err(|error| error.to_string())
    }
}

impl Drop for TinyLlamaFixture {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.directory);
    }
}

fn write_tiny_config(path: &Path, declaration: ConfigDeclaration) -> TestResult {
    let mut config = json!({
        "model_type": "llama",
        "hidden_size": 8,
        "intermediate_size": 16,
        "vocab_size": 16,
        "num_hidden_layers": 1,
        "num_attention_heads": 2,
        "num_key_value_heads": 2,
        "rms_norm_eps": 0.00001,
        "rope_theta": 10000.0,
        "bos_token_id": 1,
        "eos_token_id": 2,
        "rope_scaling": null,
        "max_position_embeddings": 16,
        "tie_word_embeddings": false
    });
    let object = config
        .as_object_mut()
        .ok_or_else(|| "tiny config must be an object".to_owned())?;
    match declaration {
        ConfigDeclaration::Absent => {}
        ConfigDeclaration::F32 => {
            object.insert("dtype".to_owned(), JsonValue::String("float32".to_owned()));
        }
        ConfigDeclaration::F16 => {
            object.insert("dtype".to_owned(), JsonValue::String("float16".to_owned()));
        }
        ConfigDeclaration::Bf16 => {
            object.insert("dtype".to_owned(), JsonValue::String("bfloat16".to_owned()));
        }
        ConfigDeclaration::Unsupported => {
            object.insert("dtype".to_owned(), JsonValue::String("int4".to_owned()));
        }
        ConfigDeclaration::Conflict => {
            object.insert("dtype".to_owned(), JsonValue::String("float16".to_owned()));
            object.insert(
                "torch_dtype".to_owned(),
                JsonValue::String("bfloat16".to_owned()),
            );
        }
    }
    let mut bytes = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|error| error.to_string())
}

fn create_weight_tensors(profile: RequiredProfile) -> TestResult<HashMap<String, Tensor>> {
    let device = Device::Cpu;
    let mut tensors = HashMap::new();
    insert_token_matrix(&mut tensors, "model.embed_tokens.weight", profile, &device)?;
    insert_token_matrix(&mut tensors, "lm_head.weight", profile, &device)?;
    insert_vector(&mut tensors, "model.norm.weight", 8, profile, &device)?;
    for projection in ["q_proj", "k_proj", "v_proj", "o_proj"] {
        insert_matrix(
            &mut tensors,
            &format!("model.layers.0.self_attn.{projection}.weight"),
            8,
            8,
            profile,
            &device,
        )?;
    }
    for normalization in ["input_layernorm", "post_attention_layernorm"] {
        insert_vector(
            &mut tensors,
            &format!("model.layers.0.{normalization}.weight"),
            8,
            profile,
            &device,
        )?;
    }
    for projection in ["gate_proj", "up_proj"] {
        insert_matrix(
            &mut tensors,
            &format!("model.layers.0.mlp.{projection}.weight"),
            16,
            8,
            profile,
            &device,
        )?;
    }
    insert_matrix(
        &mut tensors,
        "model.layers.0.mlp.down_proj.weight",
        8,
        16,
        profile,
        &device,
    )?;
    Ok(tensors)
}

fn insert_token_matrix(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    profile: RequiredProfile,
    device: &Device,
) -> TestResult {
    let values = (0_u32..16)
        .flat_map(|token| {
            (0_u32..8).map(move |dimension| {
                if token & (1_u32 << (dimension % 4)) == 0 {
                    -1.0_f32
                } else {
                    1.0_f32
                }
            })
        })
        .collect::<Vec<_>>();
    let tensor = Tensor::from_vec(values, (16, 8), device)
        .and_then(|tensor| tensor.to_dtype(profile.dtype_for(name)))
        .map_err(|error| error.to_string())?;
    insert_tensor(tensors, name, tensor)
}

fn insert_matrix(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    rows: usize,
    columns: usize,
    profile: RequiredProfile,
    device: &Device,
) -> TestResult {
    let tensor = Tensor::zeros((rows, columns), profile.dtype_for(name), device)
        .map_err(|error| error.to_string())?;
    insert_tensor(tensors, name, tensor)
}

fn insert_vector(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    length: usize,
    profile: RequiredProfile,
    device: &Device,
) -> TestResult {
    let tensor =
        Tensor::ones(length, profile.dtype_for(name), device).map_err(|error| error.to_string())?;
    insert_tensor(tensors, name, tensor)
}

fn insert_tensor(tensors: &mut HashMap<String, Tensor>, name: &str, tensor: Tensor) -> TestResult {
    if tensors.insert(name.to_owned(), tensor).is_some() {
        return Err(format!("duplicate fixture tensor: {name}"));
    }
    Ok(())
}

fn append_raw_extras(path: &Path, extras: &[ExtraTensor]) -> TestResult {
    if extras.is_empty() {
        return Ok(());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let header_length = read_header_length(bytes.as_slice())?;
    let data_start = 8_usize
        .checked_add(header_length)
        .ok_or_else(|| "Safetensors data start overflow".to_owned())?;
    let header_bytes = bytes
        .get(8..data_start)
        .ok_or_else(|| "saved Safetensors header is truncated".to_owned())?;
    let mut header: JsonMap<String, JsonValue> =
        serde_json::from_slice(header_bytes).map_err(|error| error.to_string())?;
    let mut payload = bytes
        .get(data_start..)
        .ok_or_else(|| "saved Safetensors payload is missing".to_owned())?
        .to_vec();

    for extra in extras {
        if header.contains_key(extra.name) {
            return Err(format!("duplicate raw extra tensor: {}", extra.name));
        }
        let start = u64::try_from(payload.len()).map_err(|error| error.to_string())?;
        let byte_length = extra
            .elements
            .checked_mul(extra.bytes_per_element)
            .ok_or_else(|| format!("raw extra byte length overflow: {}", extra.name))?;
        let new_length = payload
            .len()
            .checked_add(byte_length)
            .ok_or_else(|| format!("raw payload length overflow: {}", extra.name))?;
        payload
            .try_reserve_exact(byte_length)
            .map_err(|error| error.to_string())?;
        payload.resize(new_length, 0);
        let end = u64::try_from(new_length).map_err(|error| error.to_string())?;
        header.insert(
            extra.name.to_owned(),
            json!({
                "dtype": extra.dtype,
                "shape": [extra.elements],
                "data_offsets": [start, end]
            }),
        );
    }

    let mut encoded_header = serde_json::to_vec(&header).map_err(|error| error.to_string())?;
    let padding = (8 - encoded_header.len() % 8) % 8;
    encoded_header.resize(encoded_header.len() + padding, b' ');
    let encoded_length = u64::try_from(encoded_header.len()).map_err(|error| error.to_string())?;
    let total_length = 8_usize
        .checked_add(encoded_header.len())
        .and_then(|length| length.checked_add(payload.len()))
        .ok_or_else(|| "rebuilt Safetensors length overflow".to_owned())?;
    let mut rebuilt = Vec::new();
    rebuilt
        .try_reserve_exact(total_length)
        .map_err(|error| error.to_string())?;
    rebuilt.extend_from_slice(&encoded_length.to_le_bytes());
    rebuilt.extend_from_slice(&encoded_header);
    rebuilt.extend_from_slice(&payload);
    fs::write(path, rebuilt).map_err(|error| error.to_string())
}

fn read_header_length(bytes: &[u8]) -> TestResult<usize> {
    let prefix: [u8; 8] = bytes
        .get(..8)
        .ok_or_else(|| "Safetensors prefix is truncated".to_owned())?
        .try_into()
        .map_err(|_| "Safetensors prefix has the wrong length".to_owned())?;
    usize::try_from(u64::from_le_bytes(prefix)).map_err(|error| error.to_string())
}

fn file_identity(path: &Path) -> TestResult<(u64, [u8; 32])> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let byte_length = u64::try_from(bytes.len()).map_err(|error| error.to_string())?;
    Ok((byte_length, Sha256::digest(bytes).into()))
}

fn scalar_set(types: &[ScalarType]) -> ScalarTypeSet {
    let mut set = ScalarTypeSet::EMPTY;
    for scalar_type in types {
        set.insert(*scalar_type);
    }
    set
}

fn write_raw_safetensors(path: &Path, header: &str, payload: &[u8]) -> TestResult {
    let header_length = u64::try_from(header.len()).map_err(|error| error.to_string())?;
    let total_length = 8_usize
        .checked_add(header.len())
        .and_then(|length| length.checked_add(payload.len()))
        .ok_or_else(|| "raw fixture length overflow".to_owned())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(total_length)
        .map_err(|error| error.to_string())?;
    bytes.extend_from_slice(&header_length.to_le_bytes());
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(payload);
    fs::write(path, bytes).map_err(|error| error.to_string())
}

fn assert_valid_prefix_header_rejected(header: &str, payload: &[u8]) -> TestResult {
    let fixture = TinyLlamaFixture::empty()?;
    write_raw_safetensors(&fixture.weight_path, header, payload)?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_invalid_model(loader.inspect(&source));
    Ok(())
}

fn assert_raw_header_rejected(prefix: [u8; 8], header: &[u8], payload: &[u8]) -> TestResult {
    let fixture = TinyLlamaFixture::empty()?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&prefix);
    bytes.extend_from_slice(header);
    bytes.extend_from_slice(payload);
    fs::write(&fixture.weight_path, bytes).map_err(|error| error.to_string())?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_invalid_model(loader.inspect(&source));
    Ok(())
}

fn assert_raw_bytes_rejected(bytes: &[u8]) -> TestResult {
    let fixture = TinyLlamaFixture::empty()?;
    fs::write(&fixture.weight_path, bytes).map_err(|error| error.to_string())?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_invalid_model(loader.inspect(&source));
    Ok(())
}

fn assert_invalid_model<T>(result: Result<T, LoadError>) {
    let matches_kind = matches!(
        &result,
        Err(LoadError::Backend(failure))
            if failure.kind == BackendFailureKind::InvalidModel
    );
    drop(result);
    assert!(matches_kind);
}

fn assert_unsupported<T>(result: Result<T, LoadError>) {
    let matches_kind = matches!(
        &result,
        Err(LoadError::Backend(failure))
            if failure.kind == BackendFailureKind::Unsupported
    );
    drop(result);
    assert!(matches_kind);
}

fn maximum_logit_token(logits: &[f32]) -> TestResult<TokenId> {
    let (index, _) = logits
        .iter()
        .copied()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .ok_or_else(|| "logits must not be empty".to_owned())?;
    let token = u32::try_from(index).map_err(|error| error.to_string())?;
    Ok(TokenId::new(token))
}

fn debug_error(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}
