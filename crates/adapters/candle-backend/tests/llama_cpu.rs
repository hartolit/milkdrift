//! End-to-end and malformed-artifact tests for the Candle CPU Llama adapter.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use candle_backend::{
    CandleLlamaLoader, CandleLlamaModel, CandleLlamaPreparedLoad, CandleLlamaSource,
};
use candle_core::{DType, Device, Tensor};
use domain_contracts::{
    BackendFailureKind, BackendId, BackendSequence, CancellationStatus, CapacityResource,
    DecodeBuffers, DecodeInput, DecodeOutcome, DeviceId, DeviceKind, ExecutionDevice,
    LoadConfiguration, LoadError, LoadPlan, LoadedModel, MemoryBudget, ModelGeneration,
    ModelHandle, ModelId, ModelLoader, PrefillBuffers, PrefillInput, PrefillOutcome, PreparedLoad,
    ScalarType, ScalarTypeSet, SequenceConfiguration, SequenceId, SequenceState, TokenId,
    decode_checked, prefill_checked,
};
use safetensors::tensor::{Dtype as SafeDtype, SafeTensors};

const BACKEND: BackendId = BackendId::new(1);
const REQUIRED_ELEMENTS: u64 = 920;
const VOCABULARY_SIZE: usize = 16;

type TestResult<T = ()> = Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WeightProfile {
    HomogeneousF32,
    HomogeneousF16,
    HomogeneousBf16,
    MixedF16F32,
    MixedBf16F32,
    MixedF16F32WithExtra,
    UnsupportedU8,
}

impl WeightProfile {
    const fn primary_scalar(self) -> ScalarType {
        match self {
            Self::HomogeneousF32 | Self::UnsupportedU8 => ScalarType::F32,
            Self::HomogeneousF16 | Self::MixedF16F32 | Self::MixedF16F32WithExtra => {
                ScalarType::F16
            }
            Self::HomogeneousBf16 | Self::MixedBf16F32 => ScalarType::Bf16,
        }
    }

    fn dtype_for(self, name: &str) -> DType {
        match self {
            Self::HomogeneousF32 => DType::F32,
            Self::HomogeneousF16 => DType::F16,
            Self::HomogeneousBf16 => DType::BF16,
            Self::MixedF16F32 | Self::MixedF16F32WithExtra => {
                if is_f32_auxiliary(name) {
                    DType::F32
                } else {
                    DType::F16
                }
            }
            Self::MixedBf16F32 => {
                if is_f32_auxiliary(name) {
                    DType::F32
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

    const fn has_extra(self) -> bool {
        matches!(self, Self::MixedF16F32WithExtra)
    }
}

fn is_f32_auxiliary(name: &str) -> bool {
    matches!(name, "model.norm.weight") || matches!(name, "extra.phase12.weight")
}

#[test]
fn homogeneous_f32_f16_and_bf16_preserve_execution_behavior() -> TestResult {
    for (profile, expected_execution) in [
        (WeightProfile::HomogeneousF32, ScalarType::F32),
        (WeightProfile::HomogeneousF16, ScalarType::F16),
        (WeightProfile::HomogeneousBf16, ScalarType::F32),
    ] {
        execute_profile(profile, Some(profile.primary_scalar()), expected_execution)?;
    }
    Ok(())
}

#[test]
fn mixed_f16_f32_loads_executes_and_reports_observed_facts() -> TestResult {
    execute_profile(
        WeightProfile::MixedF16F32,
        Some(ScalarType::F16),
        ScalarType::F16,
    )
}

#[test]
fn mixed_bf16_f32_loads_executes_as_f32_on_cpu() -> TestResult {
    execute_profile(
        WeightProfile::MixedBf16F32,
        Some(ScalarType::Bf16),
        ScalarType::F32,
    )
}

#[test]
fn absent_declaration_uses_inferred_mixed_primary() -> TestResult {
    let fixture = TinyLlamaFixture::create(WeightProfile::MixedF16F32)?;
    let source = fixture.source(None)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let descriptor = loader.inspect(&source).map_err(debug_error)?;
    assert_eq!(descriptor.metadata.configuration_declared_scalar_type, None);
    assert_eq!(
        descriptor.metadata.observed_tensor_scalar_types,
        mixed_set(ScalarType::F16)
    );

    let (plan, mut model) = prepare_and_load(&mut loader, &source, load_configuration())?;
    assert_eq!(plan.execution_scalar_type, ScalarType::F16);
    assert_eq!(model.execution_scalar_type(), ScalarType::F16);
    clean_model(&mut model)
}

#[test]
fn declaration_matching_primary_accepts_differing_f32_auxiliary() -> TestResult {
    let fixture = TinyLlamaFixture::create(WeightProfile::MixedF16F32)?;
    let source = fixture.source(Some(ScalarType::F16))?;
    let loader = CandleLlamaLoader::new(BACKEND);
    let descriptor = loader.inspect(&source).map_err(debug_error)?;

    assert_eq!(
        descriptor.metadata.configuration_declared_scalar_type,
        Some(ScalarType::F16)
    );
    assert_eq!(
        descriptor.metadata.observed_tensor_scalar_types,
        mixed_set(ScalarType::F16)
    );
    Ok(())
}

#[test]
fn contradictory_and_unsupported_declarations_are_unsupported_not_corrupt() -> TestResult {
    let mixed = TinyLlamaFixture::create(WeightProfile::MixedF16F32)?;
    let contradictory = mixed.source(Some(ScalarType::F32))?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_eq!(
        loader.inspect(&contradictory),
        Err(LoadError::UnsupportedFormat)
    );

    let f32_fixture = TinyLlamaFixture::create(WeightProfile::HomogeneousF32)?;
    let unsupported = f32_fixture.source(Some(ScalarType::I8))?;
    assert_eq!(
        loader.inspect(&unsupported),
        Err(LoadError::UnsupportedFormat)
    );
    Ok(())
}

#[test]
fn unsupported_tensor_dtype_fails_before_device_initialization() -> TestResult {
    let fixture = TinyLlamaFixture::create(WeightProfile::UnsupportedU8)?;
    let source = fixture.source(Some(ScalarType::F32))?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let mut configuration = load_configuration();
    // This identity would fail device preparation if header rejection did not
    // take precedence.
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(1), DeviceKind::Cpu);

    assert!(matches!(
        loader.prepare_load(&source, &configuration),
        Err(LoadError::Backend(failure))
            if failure.kind == BackendFailureKind::Unsupported
    ));
    Ok(())
}

#[test]
fn exact_mixed_cpu_final_and_loading_peak_accounting_matches_algorithm() -> TestResult {
    for profile in [
        WeightProfile::MixedF16F32,
        WeightProfile::MixedBf16F32,
        WeightProfile::MixedF16F32WithExtra,
    ] {
        let fixture = TinyLlamaFixture::create(profile)?;
        let source = fixture.source(Some(profile.primary_scalar()))?;
        let mut loader = CandleLlamaLoader::new(BACKEND);
        let descriptor = loader.inspect(&source).map_err(debug_error)?;
        let prepared = loader
            .prepare_load(&source, &load_configuration())
            .map_err(debug_error)?;
        let plan = *prepared.plan();
        let expected_execution_dtype = match profile.primary_scalar() {
            ScalarType::F32 | ScalarType::Bf16 => DType::F32,
            ScalarType::F16 => DType::F16,
            _ => return Err("unexpected test primary scalar".to_owned()),
        };
        let expected = expected_cpu_accounting(
            std::slice::from_ref(&fixture.weight_path),
            expected_execution_dtype,
        )?;

        assert_eq!(expected.required_elements, REQUIRED_ELEMENTS);
        assert_eq!(
            expected.required_execution_bytes,
            REQUIRED_ELEMENTS * dtype_bytes(expected_execution_dtype)?
        );
        if profile == WeightProfile::MixedF16F32 {
            assert_eq!(expected.source_bytes, 1_856);
        }
        assert_eq!(
            descriptor.estimated_footprint.host_weight_bytes,
            expected.required_execution_bytes
        );
        assert_eq!(descriptor.estimated_footprint.host_working_bytes, 0);
        assert_eq!(plan.expected_footprint, descriptor.estimated_footprint);
        assert_eq!(
            plan.loading_peak_footprint.host_weight_bytes,
            expected.required_execution_bytes
        );
        assert_eq!(
            plan.loading_peak_footprint.host_working_bytes,
            expected
                .host_peak
                .checked_sub(expected.required_execution_bytes)
                .ok_or_else(|| "host headroom underflow".to_owned())?
        );
        assert_eq!(
            plan.loading_peak_footprint.checked_host_bytes(),
            Some(expected.host_peak)
        );
        assert_eq!(
            plan.expected_footprint.cache_bytes_per_token,
            16 * dtype_bytes(expected_execution_dtype)?
        );
        if profile.has_extra() {
            assert!(expected.full_execution_bytes > expected.required_execution_bytes);
            assert!(
                plan.loading_peak_footprint.host_working_bytes
                    >= expected.full_execution_bytes - expected.required_execution_bytes
            );
        } else {
            assert_eq!(
                expected.full_execution_bytes,
                expected.required_execution_bytes
            );
        }
        drop(prepared);
    }
    Ok(())
}

#[test]
fn supported_extra_tensor_is_materialized_as_headroom_not_final_ownership() -> TestResult {
    let fixture = TinyLlamaFixture::create(WeightProfile::MixedF16F32WithExtra)?;
    let source = fixture.source(Some(ScalarType::F16))?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let (plan, mut model) = prepare_and_load(&mut loader, &source, load_configuration())?;

    assert_eq!(
        plan.expected_footprint.host_weight_bytes,
        REQUIRED_ELEMENTS * 2
    );
    assert!(plan.loading_peak_footprint.host_working_bytes >= 2);
    assert_eq!(model.accounted_footprint(), plan.expected_footprint);
    clean_model(&mut model)
}

#[test]
fn host_budget_rejects_loading_peak_before_materialization() -> TestResult {
    let fixture = TinyLlamaFixture::create(WeightProfile::MixedBf16F32)?;
    let source = fixture.source(Some(ScalarType::Bf16))?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let prepared = loader
        .prepare_load(&source, &load_configuration())
        .map_err(debug_error)?;
    let required = prepared
        .plan()
        .loading_peak_footprint
        .checked_host_bytes()
        .ok_or_else(|| "loading host total overflow".to_owned())?;
    drop(prepared);

    let mut constrained = load_configuration();
    constrained.memory_budget.host_bytes = required
        .checked_sub(1)
        .ok_or_else(|| "test loading peak must be nonzero".to_owned())?;
    assert!(matches!(
        loader.prepare_load(&source, &constrained),
        Err(LoadError::InsufficientMemory {
            kind: domain_contracts::MemoryKind::Host,
            required_bytes,
            available_bytes,
        }) if required_bytes == required && available_bytes == required - 1
    ));
    Ok(())
}

#[test]
fn prepared_load_consumes_retained_file_without_reopening_path() -> TestResult {
    let fixture = TinyLlamaFixture::create(WeightProfile::HomogeneousF32)?;
    let source = fixture.source(Some(ScalarType::F32))?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let prepared = loader
        .prepare_load(&source, &load_configuration())
        .map_err(debug_error)?;
    let accepted_plan = *prepared.plan();
    fs::remove_file(&fixture.weight_path)
        .map_err(|error| format!("remove prepared path: {error}"))?;

    let mut model = load_exact_preparation(&mut loader, prepared)?;
    assert_eq!(
        model.accounted_footprint(),
        accepted_plan.expected_footprint
    );
    clean_model(&mut model)
}

#[test]
fn prepared_load_rejects_same_inode_payload_mutation_and_cleans_partial_tensors() -> TestResult {
    let fixture = TinyLlamaFixture::create(WeightProfile::HomogeneousF32)?;
    let source = fixture.source(Some(ScalarType::F32))?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let prepared = loader
        .prepare_load(&source, &load_configuration())
        .map_err(debug_error)?;

    let mut retained_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&fixture.weight_path)
        .map_err(|error| format!("open prepared payload for mutation: {error}"))?;
    retained_file
        .seek(SeekFrom::End(-1))
        .map_err(|error| format!("seek prepared payload: {error}"))?;
    let mut final_byte = [0_u8; 1];
    retained_file
        .read_exact(&mut final_byte)
        .map_err(|error| format!("read prepared payload: {error}"))?;
    final_byte[0] ^= 1;
    retained_file
        .seek(SeekFrom::End(-1))
        .map_err(|error| format!("reseek prepared payload: {error}"))?;
    retained_file
        .write_all(&final_byte)
        .map_err(|error| format!("mutate prepared payload: {error}"))?;
    retained_file
        .sync_all()
        .map_err(|error| format!("synchronize prepared payload mutation: {error}"))?;

    let failed = match loader.load_prepared(prepared) {
        Err(failed) => failed,
        Ok(mut model) => {
            clean_model(&mut model)?;
            return Err("same-inode payload mutation unexpectedly loaded".to_owned());
        }
    };
    let (error, mut cleanup_owner) = failed.into_parts();
    assert!(matches!(
        error,
        LoadError::Backend(failure) if failure.kind == BackendFailureKind::InvalidModel
    ));
    cleanup_owner.cleanup().map_err(debug_error)?;
    cleanup_owner.cleanup().map_err(debug_error)?;
    Ok(())
}

#[test]
fn cross_shard_and_same_header_duplicates_are_rejected_deterministically() -> TestResult {
    let fixture = TinyLlamaFixture::create(WeightProfile::HomogeneousF32)?;
    let duplicate_source = CandleLlamaSource::new(
        fixture.config_path.clone(),
        vec![fixture.weight_path.clone(), fixture.weight_path.clone()],
        Some(ScalarType::F32),
    )
    .map_err(|error| error.to_string())?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert!(matches!(
        loader.inspect(&duplicate_source),
        Err(LoadError::Backend(failure))
            if failure.kind == BackendFailureKind::InvalidModel
    ));

    let same_header = TinyLlamaFixture::create(WeightProfile::HomogeneousF32)?;
    write_raw_safetensors(
        &same_header.weight_path,
        r#"{"dup":{"dtype":"F32","shape":[0],"data_offsets":[0,0]},"dup":{"dtype":"F32","shape":[0],"data_offsets":[0,0]}}"#,
        &[],
    )?;
    let source = same_header.source(Some(ScalarType::F32))?;
    assert!(matches!(
        loader.inspect(&source),
        Err(LoadError::Backend(failure))
            if failure.kind == BackendFailureKind::InvalidModel
    ));
    Ok(())
}

#[test]
fn excessive_shard_count_is_rejected_before_files_are_opened() -> TestResult {
    let fixture = TinyLlamaFixture::create(WeightProfile::HomogeneousF32)?;
    let source = CandleLlamaSource::new(
        fixture.config_path.clone(),
        vec![fixture.weight_path.clone(); 257],
        Some(ScalarType::F32),
    )
    .map_err(|error| error.to_string())?;
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
fn malformed_truncated_and_oversized_headers_are_rejected() -> TestResult {
    assert_raw_header_rejected(10_u64.to_le_bytes(), b"{}", &[])?;
    assert_raw_header_rejected(100_000_001_u64.to_le_bytes(), &[], &[])?;
    assert_raw_header_rejected(8_u64.to_le_bytes(), b"not-json", &[])?;
    Ok(())
}

#[test]
fn invalid_offsets_bounds_shape_mismatch_and_overflow_are_rejected() -> TestResult {
    for (header, payload) in [
        (
            r#"{"a":{"dtype":"F32","shape":[1],"data_offsets":[0,4]},"b":{"dtype":"F32","shape":[1],"data_offsets":[3,7]}}"#,
            vec![0_u8; 7],
        ),
        (
            r#"{"a":{"dtype":"F32","shape":[2],"data_offsets":[0,4]}}"#,
            vec![0_u8; 4],
        ),
        (
            r#"{"a":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#,
            vec![0_u8; 3],
        ),
        (
            r#"{"a":{"dtype":"F32","shape":[18446744073709551615,2],"data_offsets":[0,0]}}"#,
            Vec::new(),
        ),
    ] {
        let fixture = TinyLlamaFixture::create(WeightProfile::HomogeneousF32)?;
        write_raw_safetensors(&fixture.weight_path, header, &payload)?;
        let source = fixture.source(Some(ScalarType::F32))?;
        let loader = CandleLlamaLoader::new(BACKEND);
        assert!(matches!(
            loader.inspect(&source),
            Err(LoadError::Backend(failure))
                if failure.kind == BackendFailureKind::InvalidModel
        ));
    }
    Ok(())
}

#[test]
fn reversed_shard_selection_is_sorted_and_loads_deterministically() -> TestResult {
    let fixture = TinyLlamaFixture::create_sharded(WeightProfile::MixedF16F32)?;
    let source = CandleLlamaSource::new(
        fixture.config_path.clone(),
        vec![
            fixture.weight_path.clone(),
            fixture.second_weight_path.clone(),
        ],
        Some(ScalarType::F16),
    )
    .map_err(|error| error.to_string())?;
    assert!(
        source
            .weight_paths()
            .windows(2)
            .all(|paths| paths.first() <= paths.get(1))
    );

    let mut loader = CandleLlamaLoader::new(BACKEND);
    let (_plan, mut model) = prepare_and_load(&mut loader, &source, load_configuration())?;
    clean_model(&mut model)
}

#[test]
fn rejects_invalid_cpu_identity_and_unsupported_devices() -> TestResult {
    let fixture = TinyLlamaFixture::create(WeightProfile::HomogeneousF32)?;
    let source = fixture.source(Some(ScalarType::F32))?;
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
    let fixture = TinyLlamaFixture::create(WeightProfile::HomogeneousF32)?;
    let source = fixture.source(Some(ScalarType::F32))?;
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
    profile: WeightProfile,
    declaration: Option<ScalarType>,
    expected_execution: ScalarType,
) -> TestResult {
    let fixture = TinyLlamaFixture::create(profile)?;
    let source = fixture.source(declaration)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let descriptor = loader.inspect(&source).map_err(debug_error)?;
    assert_eq!(
        descriptor.metadata.configuration_declared_scalar_type,
        declaration
    );
    assert_eq!(
        descriptor.metadata.observed_tensor_scalar_types,
        expected_observed_set(profile)
    );

    let (plan, mut model) = prepare_and_load(&mut loader, &source, load_configuration())?;
    assert_eq!(plan.descriptor, descriptor);
    assert_eq!(plan.execution_scalar_type, expected_execution);
    assert_eq!(model.descriptor(), &descriptor);
    assert_eq!(model.execution_scalar_type(), expected_execution);
    exercise_model(&mut model)?;
    clean_model(&mut model)
}

fn exercise_model(model: &mut CandleLlamaModel) -> TestResult {
    let configuration = SequenceConfiguration::new(
        NonZeroU32::new(16).ok_or_else(|| "maximum tokens must be nonzero".to_owned())?,
        NonZeroU32::new(8).ok_or_else(|| "maximum prefill must be nonzero".to_owned())?,
    );
    let mut first = model
        .create_sequence(SequenceId::new(1), &configuration)
        .map_err(debug_error)?;
    let mut second = model
        .create_sequence(SequenceId::new(2), &configuration)
        .map_err(debug_error)?;
    let prompt = [TokenId::new(1), TokenId::new(2)];
    let mut first_logits = [0.0_f32; VOCABULARY_SIZE];
    let mut second_logits = [0.0_f32; VOCABULARY_SIZE];

    let first_prefill = prefill_checked(
        model,
        &mut first,
        PrefillInput::new(&prompt, true),
        PrefillBuffers::new(&mut first_logits),
        CancellationStatus::Running,
    )
    .map_err(debug_error)?;
    assert_eq!(
        first_prefill,
        PrefillOutcome::Ready {
            consumed_tokens: 2,
            position: 2,
            logits_written: VOCABULARY_SIZE,
        }
    );
    assert_eq!(maximum_logit_token(&first_logits)?, TokenId::new(2));

    prefill_checked(
        model,
        &mut second,
        PrefillInput::new(&prompt, true),
        PrefillBuffers::new(&mut second_logits),
        CancellationStatus::Running,
    )
    .map_err(debug_error)?;
    let decoded = decode_checked(
        model,
        &mut first,
        DecodeInput::new(TokenId::new(3)),
        DecodeBuffers::new(&mut first_logits),
        CancellationStatus::Running,
    )
    .map_err(debug_error)?;
    assert_eq!(
        decoded,
        DecodeOutcome::Ready {
            position: 3,
            logits_written: VOCABULARY_SIZE,
        }
    );
    assert_eq!(maximum_logit_token(&first_logits)?, TokenId::new(3));
    assert_eq!(second.position(), 2);

    model.destroy_sequence(&mut first).map_err(debug_error)?;
    model.destroy_sequence(&mut second).map_err(debug_error)?;
    assert_eq!(first.state(), SequenceState::Finished);
    assert_eq!(second.state(), SequenceState::Finished);
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
            let cleanup = failed.cleanup_owner_mut().cleanup();
            Err(format!(
                "prepared load failed: {primary:?}; cleanup: {cleanup:?}"
            ))
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
    fn create(profile: WeightProfile) -> TestResult<Self> {
        let fixture = Self::empty()?;
        let tensors = create_weight_tensors(profile)?;
        candle_core::safetensors::save(&tensors, &fixture.weight_path)
            .map_err(|error| format!("save weights: {error}"))?;
        Ok(fixture)
    }

    fn create_sharded(profile: WeightProfile) -> TestResult<Self> {
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
        candle_core::safetensors::save(&first, &fixture.second_weight_path)
            .map_err(|error| format!("save first shard: {error}"))?;
        candle_core::safetensors::save(&second, &fixture.weight_path)
            .map_err(|error| format!("save second shard: {error}"))?;
        Ok(fixture)
    }

    fn empty() -> TestResult<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "milkdrift-candle-phase12-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let config_path = directory.join("config.json");
        let weight_path = directory.join("z-model.safetensors");
        let second_weight_path = directory.join("a-model.safetensors");
        fs::write(&config_path, TINY_CONFIG).map_err(|error| error.to_string())?;
        Ok(Self {
            directory,
            config_path,
            weight_path,
            second_weight_path,
        })
    }

    fn source(
        &self,
        configuration_declared_scalar_type: Option<ScalarType>,
    ) -> TestResult<CandleLlamaSource> {
        CandleLlamaSource::new(
            self.config_path.clone(),
            vec![self.weight_path.clone()],
            configuration_declared_scalar_type,
        )
        .map_err(|error| error.to_string())
    }
}

impl Drop for TinyLlamaFixture {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.directory);
    }
}

fn create_weight_tensors(profile: WeightProfile) -> TestResult<HashMap<String, Tensor>> {
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
    if profile.has_extra() {
        insert_vector(&mut tensors, "extra.phase12.weight", 1, profile, &device)?;
    }
    Ok(tensors)
}

fn insert_token_matrix(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    profile: WeightProfile,
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
    profile: WeightProfile,
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
    profile: WeightProfile,
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

#[derive(Clone, Copy, Debug)]
struct ExpectedCpuAccounting {
    source_bytes: u64,
    required_elements: u64,
    required_execution_bytes: u64,
    full_execution_bytes: u64,
    host_peak: u64,
}

fn expected_cpu_accounting(
    paths: &[PathBuf],
    execution_dtype: DType,
) -> TestResult<ExpectedCpuAccounting> {
    let execution_width = dtype_bytes(execution_dtype)?;
    let mut sorted_paths = paths.to_vec();
    sorted_paths.sort();
    let mut source_bytes = 0_u64;
    let mut required_elements = 0_u64;
    let mut required_execution_bytes = 0_u64;
    let mut full_execution_bytes = 0_u64;
    let mut host_peak = 0_u64;

    for path in sorted_paths {
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let (_header_length, metadata) =
            SafeTensors::read_metadata(&bytes).map_err(|error| error.to_string())?;
        for name in metadata.offset_keys() {
            let info = metadata
                .info(&name)
                .ok_or_else(|| format!("missing metadata for {name}"))?;
            let source_width = safe_dtype_bytes(info.dtype)?;
            let elements = info.shape.iter().try_fold(1_u64, |total, dimension| {
                total
                    .checked_mul(u64::try_from(*dimension).map_err(|error| error.to_string())?)
                    .ok_or_else(|| "fixture element overflow".to_owned())
            })?;
            let tensor_source_bytes = elements
                .checked_mul(source_width)
                .ok_or_else(|| "fixture source bytes overflow".to_owned())?;
            let execution_bytes = elements
                .checked_mul(execution_width)
                .ok_or_else(|| "fixture execution bytes overflow".to_owned())?;
            let aligned_staging = tensor_source_bytes
                .checked_add(source_width.saturating_sub(1))
                .ok_or_else(|| "fixture staging overflow".to_owned())?;
            let raw_peak = full_execution_bytes
                .checked_add(aligned_staging)
                .and_then(|value| value.checked_add(tensor_source_bytes))
                .ok_or_else(|| "fixture raw peak overflow".to_owned())?;
            host_peak = host_peak.max(raw_peak);
            if source_width != execution_width {
                let cast_peak = full_execution_bytes
                    .checked_add(tensor_source_bytes)
                    .and_then(|value| value.checked_add(execution_bytes))
                    .ok_or_else(|| "fixture cast peak overflow".to_owned())?;
                host_peak = host_peak.max(cast_peak);
            }
            source_bytes = source_bytes
                .checked_add(tensor_source_bytes)
                .ok_or_else(|| "fixture total source overflow".to_owned())?;
            full_execution_bytes = full_execution_bytes
                .checked_add(execution_bytes)
                .ok_or_else(|| "fixture full execution overflow".to_owned())?;
            if name != "extra.phase12.weight" {
                required_elements = required_elements
                    .checked_add(elements)
                    .ok_or_else(|| "fixture required elements overflow".to_owned())?;
                required_execution_bytes = required_execution_bytes
                    .checked_add(execution_bytes)
                    .ok_or_else(|| "fixture required execution overflow".to_owned())?;
            }
        }
    }
    host_peak = host_peak.max(full_execution_bytes);
    Ok(ExpectedCpuAccounting {
        source_bytes,
        required_elements,
        required_execution_bytes,
        full_execution_bytes,
        host_peak,
    })
}

fn safe_dtype_bytes(dtype: SafeDtype) -> TestResult<u64> {
    match dtype {
        SafeDtype::F32 => Ok(4),
        SafeDtype::F16 | SafeDtype::BF16 => Ok(2),
        _ => Err(format!("unexpected fixture dtype: {dtype:?}")),
    }
}

fn dtype_bytes(dtype: DType) -> TestResult<u64> {
    match dtype {
        DType::F32 => Ok(4),
        DType::F16 | DType::BF16 => Ok(2),
        _ => Err(format!("unexpected execution dtype: {dtype:?}")),
    }
}

fn expected_observed_set(profile: WeightProfile) -> ScalarTypeSet {
    match profile {
        WeightProfile::HomogeneousF32 => ScalarTypeSet::from_scalar(ScalarType::F32),
        WeightProfile::HomogeneousF16 => ScalarTypeSet::from_scalar(ScalarType::F16),
        WeightProfile::HomogeneousBf16 => ScalarTypeSet::from_scalar(ScalarType::Bf16),
        WeightProfile::MixedF16F32 | WeightProfile::MixedF16F32WithExtra => {
            mixed_set(ScalarType::F16)
        }
        WeightProfile::MixedBf16F32 => mixed_set(ScalarType::Bf16),
        WeightProfile::UnsupportedU8 => ScalarTypeSet::from_scalar(ScalarType::U8)
            .union(ScalarTypeSet::from_scalar(ScalarType::F32)),
    }
}

fn mixed_set(primary: ScalarType) -> ScalarTypeSet {
    ScalarTypeSet::from_scalar(primary).union(ScalarTypeSet::from_scalar(ScalarType::F32))
}

fn write_raw_safetensors(path: &Path, header: &str, payload: &[u8]) -> TestResult {
    let header_length = u64::try_from(header.len()).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(
            8_usize
                .checked_add(header.len())
                .and_then(|value| value.checked_add(payload.len()))
                .ok_or_else(|| "raw fixture length overflow".to_owned())?,
        )
        .map_err(|error| error.to_string())?;
    bytes.extend_from_slice(&header_length.to_le_bytes());
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(payload);
    fs::write(path, bytes).map_err(|error| error.to_string())
}

fn assert_raw_header_rejected(prefix: [u8; 8], header: &[u8], payload: &[u8]) -> TestResult {
    let fixture = TinyLlamaFixture::empty()?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&prefix);
    bytes.extend_from_slice(header);
    bytes.extend_from_slice(payload);
    fs::write(&fixture.weight_path, bytes).map_err(|error| error.to_string())?;
    let source = fixture.source(Some(ScalarType::F32))?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert!(matches!(
        loader.inspect(&source),
        Err(LoadError::Backend(failure))
            if failure.kind == BackendFailureKind::InvalidModel
    ));
    Ok(())
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

const TINY_CONFIG: &str = r#"{
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
}"#;
