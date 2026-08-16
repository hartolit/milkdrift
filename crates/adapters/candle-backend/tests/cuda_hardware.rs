//! Opt-in CUDA execution evidence for homogeneous and mixed Llama fixtures.

#![cfg(feature = "cuda")]

use std::collections::HashMap;
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use candle_backend::{
    CandleHardwareLoadFault, CandleLlamaLoader, CandleLlamaModel, CandleLlamaSource,
    CandleLoadCleanupOutcome, CandleLoadObservation, CandleLoadObservationOutcome,
};
use candle_core::{DType, Device, Tensor};
use domain_contracts::{
    BackendFailureKind, BackendId, CancellationStatus, DecodeBuffers, DecodeInput, DecodeOutcome,
    DeviceId, DeviceKind, ExecutionDevice, LoadConfiguration, LoadError, LoadFailureStage,
    LoadPlan, LoadedModel, MemoryBudget, MemoryFootprint, ModelGeneration, ModelHandle, ModelId,
    ModelLoader, PrefillBuffers, PrefillInput, PrefillOutcome, PreparedLoad, ScalarType,
    ScalarTypeSet, SequenceConfiguration, SequenceId, TensorFailureLocation, TokenId,
    decode_checked, prefill_checked,
};
use serde_json::{Value as JsonValue, json};

const BACKEND: BackendId = BackendId::new(501);
const CPU: ExecutionDevice = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
const CUDA_0: ExecutionDevice = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);
const VOCABULARY_SIZE: usize = 16;
const F32_EXECUTION_WEIGHT_BYTES: u64 = 3_680;
const F32_CACHE_BYTES_PER_TOKEN: u64 = 64;
const F32_CPU_SEQUENCE_HOST_WORKING_BYTES: u64 = 7_556;
const F32_CUDA_SEQUENCE_HOST_WORKING_BYTES: u64 = 160;
const F32_CUDA_SEQUENCE_DEVICE_WORKING_BYTES: u64 = 7_396;
const F32_CPU_LOADING_WORKING_BYTES: u64 = 65_763;
const F32_CUDA_HOST_LOADING_PEAK_BYTES: u64 = 77_571;
const HALF_EXECUTION_WEIGHT_BYTES: u64 = 1_840;
const HALF_CACHE_BYTES_PER_TOKEN: u64 = 32;
const HALF_CUDA_SEQUENCE_HOST_WORKING_BYTES: u64 = 160;
const HALF_CUDA_SEQUENCE_DEVICE_WORKING_BYTES: u64 = 6_884;
const HALF_HOMOGENEOUS_CUDA_HOST_LOADING_PEAK_BYTES: u64 = 75_617;
const HALF_MIXED_CUDA_HOST_LOADING_PEAK_BYTES: u64 = 75_665;

type TestResult<T = ()> = Result<T, String>;

struct HardwareCase {
    name: &'static str,
    run: fn() -> TestResult,
}

macro_rules! hardware_cases {
    (
        $(
            $(#[$case_attribute:meta])*
            fn $case:ident() -> TestResult $body:block
        )+
    ) => {
        $(
            $(#[$case_attribute])*
            fn $case() -> TestResult $body
        )+

        const HARDWARE_CASES: &[HardwareCase] = &[
            $(HardwareCase {
                name: stringify!($case),
                run: $case,
            }),+
        ];
    };
}

fn run_hardware_suite() -> TestResult {
    require_cuda_opt_in()?;
    if HARDWARE_CASES.is_empty() {
        return Err("CUDA adapter hardware suite registered zero cases".to_owned());
    }

    let mut executed = 0_usize;
    for case in HARDWARE_CASES {
        executed = executed.saturating_add(1);
        eprintln!("running CUDA adapter case: {}", case.name);
        (case.run)().map_err(|error| format!("CUDA adapter case {} failed: {error}", case.name))?;
    }
    if executed != HARDWARE_CASES.len() {
        return Err(format!(
            "CUDA adapter suite executed {executed} of {} registered cases",
            HARDWARE_CASES.len()
        ));
    }
    eprintln!("CUDA adapter suite passed {executed} cases");
    Ok(())
}

fn main() -> ExitCode {
    match run_hardware_suite() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("CUDA adapter hardware suite failed: {error}");
            ExitCode::FAILURE
        }
    }
}

hardware_cases!(
    fn cuda_enabled_binary_can_explicitly_execute_cpu() -> TestResult {
        let source = fixture_source()?;
        let result = execute_fixture(&source, CPU)?;

        assert_eq!(result.declared_scalar_type, Some(ScalarType::F32));
        assert_eq!(
            result.observed_scalar_types,
            ScalarTypeSet::from_scalar(ScalarType::F32)
        );
        assert_eq!(result.execution_scalar_type, ScalarType::F32);
        assert_eq!(result.execution_device, CPU);
        assert_exact_f32_cpu_footprints(&result);
        assert_eq!(result.transfer_batches, 0);
        assert_eq!(result.loading_device_synchronizations, 0);
        assert!(result.sequence_footprint.host_working_bytes > 0);
        assert_eq!(result.sequence_footprint.device_working_bytes, 0);
        assert_eq!(
            maximum_logit_token(&result.prefill_logits)?,
            TokenId::new(2)
        );
        assert_eq!(maximum_logit_token(&result.decode_logits)?, TokenId::new(3));
        Ok(())
    }

    fn invalid_cuda_ordinal_is_rejected_before_driver_initialization() -> TestResult {
        let loader = CandleLlamaLoader::new(BACKEND);
        assert_eq!(
            loader.discover_device(ExecutionDevice::new(
                DeviceId::new(u64::MAX),
                DeviceKind::Cuda,
            )),
            Err(domain_contracts::LoadError::InvalidConfiguration)
        );
        Ok(())
    }

    fn cuda_ordinal_zero_executes_fixture_and_matches_cpu_logits() -> TestResult {
        let loader = CandleLlamaLoader::new(BACKEND);
        let summary = loader
            .discover_device(CUDA_0)
            .map_err(|error| format!("discover CUDA 0: {error:?}"))?;
        assert_eq!(summary.execution_device, CUDA_0);
        assert_eq!(summary.ordinal, Some(0));
        assert!(summary.supports_bf16);
        assert!(
            summary
                .display_name
                .as_deref()
                .is_some_and(|name| name.contains("RTX 5070 Ti")),
            "unexpected CUDA device name: {:?}",
            summary.display_name
        );
        assert_eq!(
            summary.compute_capability,
            Some(candle_backend::CudaComputeCapability {
                major: 12,
                minor: 0,
            })
        );
        assert!(summary.total_memory_bytes.is_some_and(|bytes| bytes > 0));
        assert!(
            summary
                .available_memory_bytes
                .is_some_and(|bytes| bytes > 0)
        );
        assert!(matches!(
            loader.discover_device(ExecutionDevice::new(
                DeviceId::new(9_999),
                DeviceKind::Cuda,
            )),
            Err(domain_contracts::LoadError::Backend(failure))
                if failure.failure.kind == BackendFailureKind::DeviceInitialization
        ));

        let source = fixture_source()?;
        let cpu = execute_fixture(&source, CPU)?;
        let cuda = execute_fixture(&source, CUDA_0)?;

        assert_eq!(cuda.execution_scalar_type, ScalarType::F32);
        assert_eq!(cuda.execution_device, CUDA_0);
        assert_exact_f32_cuda_footprints(&cuda);
        assert_eq!(cuda.transfer_batches, 1);
        assert_eq!(cuda.loading_device_synchronizations, 1);
        assert_eq!(
            cuda.sequence_footprint.host_working_bytes,
            F32_CUDA_SEQUENCE_HOST_WORKING_BYTES
        );
        assert_eq!(
            cuda.sequence_footprint.device_working_bytes,
            F32_CUDA_SEQUENCE_DEVICE_WORKING_BYTES
        );
        assert_logits_close(&cpu.prefill_logits, &cuda.prefill_logits, 1.0e-3)?;
        assert_logits_close(&cpu.decode_logits, &cuda.decode_logits, 1.0e-3)?;
        assert_eq!(maximum_logit_token(&cuda.prefill_logits)?, TokenId::new(2));
        assert_eq!(maximum_logit_token(&cuda.decode_logits)?, TokenId::new(3));
        Ok(())
    }

    fn cuda_transfer_failure_retains_exact_context_and_cleans_up() -> TestResult {
        let source = fixture_source()?;
        let (observation, recorder) = CandleLoadObservation::channel();
        let mut loader = CandleLlamaLoader::with_load_observation(BACKEND, recorder)
            .with_hardware_load_fault(CandleHardwareLoadFault::AfterDeviceTransfer {
                shard_ordinal: 0,
                tensor_ordinal: 0,
            });
        let configuration = LoadConfiguration {
            handle: ModelHandle::new(ModelId::new(2), ModelGeneration::new(1)),
            execution_device: CUDA_0,
            memory_budget: MemoryBudget {
                host_bytes: u64::MAX,
                device_bytes: u64::MAX,
            },
        };
        let prepared = loader
            .prepare_load(&source, &configuration)
            .map_err(|error| format!("prepare transfer fault: {error:?}"))?;
        let mut failed = loader
            .load_prepared(prepared)
            .err()
            .ok_or_else(|| "injected CUDA transfer fault unexpectedly loaded".to_owned())?;
        let LoadError::Backend(primary) = failed.primary() else {
            return Err(format!(
                "unexpected transfer failure: {:?}",
                failed.primary()
            ));
        };
        assert_eq!(primary.failure.kind, BackendFailureKind::DeviceExecution);
        assert_eq!(primary.failure.code, 29);
        let context = primary
            .context
            .ok_or_else(|| "CUDA transfer failure omitted context".to_owned())?;
        assert_eq!(context.stage, LoadFailureStage::DeviceTransfer);
        assert_eq!(
            context.tensor,
            Some(TensorFailureLocation::new(
                0,
                0,
                0xab67_eb51_5e1b_6f0d,
                Some(ScalarType::F32),
            ))
        );

        failed
            .cleanup()
            .map_err(|error| format!("cleanup transfer fault: {error:?}"))?;
        assert!(failed.cleanup_complete());
        let snapshot = observation.snapshot();
        assert_eq!(
            snapshot.outcome,
            CandleLoadObservationOutcome::MaterializationFailed
        );
        assert_eq!(
            snapshot.cleanup_outcome,
            CandleLoadCleanupOutcome::Succeeded
        );
        assert_eq!(snapshot.cleanup_attempts, 1);
        assert_eq!(snapshot.cleanup_failures, 0);
        assert_eq!(snapshot.transfer_batches, 1);
        Ok(())
    }

    fn cuda_homogeneous_bf16_source_executes_as_bf16() -> TestResult {
        let converted = ConvertedFixture::create(DType::BF16, false)?;
        let source = converted.source()?;
        let result = execute_fixture(&source, CUDA_0)?;

        assert_eq!(result.declared_scalar_type, Some(ScalarType::Bf16));
        assert_eq!(
            result.observed_scalar_types,
            ScalarTypeSet::from_scalar(ScalarType::Bf16)
        );
        assert_eq!(result.execution_scalar_type, ScalarType::Bf16);
        assert_exact_half_cuda_footprints(&result, HALF_HOMOGENEOUS_CUDA_HOST_LOADING_PEAK_BYTES);
        assert_eq!(result.transfer_batches, 1);
        assert_eq!(result.loading_device_synchronizations, 1);
        assert_eq!(
            maximum_logit_token(&result.prefill_logits)?,
            TokenId::new(2)
        );
        Ok(())
    }

    fn cuda_mixed_f16_f32_executes_as_f16() -> TestResult {
        let converted = ConvertedFixture::create(DType::F16, true)?;
        let source = converted.source()?;
        let result = execute_fixture(&source, CUDA_0)?;

        assert_eq!(result.declared_scalar_type, Some(ScalarType::F16));
        assert_eq!(result.observed_scalar_types, mixed_set(ScalarType::F16));
        assert_eq!(result.execution_scalar_type, ScalarType::F16);
        assert_eq!(result.execution_device, CUDA_0);
        assert_exact_half_cuda_footprints(&result, HALF_MIXED_CUDA_HOST_LOADING_PEAK_BYTES);
        assert_eq!(result.transfer_batches, 1);
        assert_eq!(result.loading_device_synchronizations, 1);
        assert_eq!(maximum_logit_token(&result.decode_logits)?, TokenId::new(3));
        Ok(())
    }

    fn cuda_mixed_bf16_f32_executes_as_bf16() -> TestResult {
        let converted = ConvertedFixture::create(DType::BF16, true)?;
        let source = converted.source()?;
        let result = execute_fixture(&source, CUDA_0)?;

        assert_eq!(result.declared_scalar_type, Some(ScalarType::Bf16));
        assert_eq!(result.observed_scalar_types, mixed_set(ScalarType::Bf16));
        assert_eq!(result.execution_scalar_type, ScalarType::Bf16);
        assert_eq!(result.execution_device, CUDA_0);
        assert_exact_half_cuda_footprints(&result, HALF_MIXED_CUDA_HOST_LOADING_PEAK_BYTES);
        assert_eq!(result.transfer_batches, 1);
        assert_eq!(result.loading_device_synchronizations, 1);
        assert_eq!(maximum_logit_token(&result.decode_logits)?, TokenId::new(3));
        Ok(())
    }
);

fn assert_exact_f32_cpu_footprints(result: &FixtureExecution) {
    assert_eq!(
        result.reported_footprint,
        MemoryFootprint {
            host_weight_bytes: F32_EXECUTION_WEIGHT_BYTES,
            device_weight_bytes: 0,
            host_working_bytes: 0,
            device_working_bytes: 0,
        }
    );
    assert_eq!(
        result.loading_peak_footprint,
        MemoryFootprint {
            host_weight_bytes: F32_EXECUTION_WEIGHT_BYTES,
            device_weight_bytes: 0,
            host_working_bytes: F32_CPU_LOADING_WORKING_BYTES,
            device_working_bytes: 0,
        }
    );
    assert_eq!(
        result.sequence_cache_bytes_per_token,
        F32_CACHE_BYTES_PER_TOKEN
    );
    assert_eq!(
        result.sequence_footprint,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 0,
            host_working_bytes: F32_CPU_SEQUENCE_HOST_WORKING_BYTES,
            device_working_bytes: 0,
        }
    );
}

fn assert_exact_f32_cuda_footprints(result: &FixtureExecution) {
    assert_eq!(
        result.reported_footprint,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: F32_EXECUTION_WEIGHT_BYTES,
            host_working_bytes: 0,
            device_working_bytes: 0,
        }
    );
    assert_eq!(
        result.loading_peak_footprint,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: F32_EXECUTION_WEIGHT_BYTES,
            host_working_bytes: F32_CUDA_HOST_LOADING_PEAK_BYTES,
            device_working_bytes: 0,
        }
    );
    assert_eq!(
        result.sequence_cache_bytes_per_token,
        F32_CACHE_BYTES_PER_TOKEN
    );
    assert_eq!(
        result.sequence_footprint,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 0,
            host_working_bytes: F32_CUDA_SEQUENCE_HOST_WORKING_BYTES,
            device_working_bytes: F32_CUDA_SEQUENCE_DEVICE_WORKING_BYTES,
        }
    );
}

fn assert_exact_half_cuda_footprints(
    result: &FixtureExecution,
    expected_host_loading_peak_bytes: u64,
) {
    assert_eq!(
        result.reported_footprint,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: HALF_EXECUTION_WEIGHT_BYTES,
            host_working_bytes: 0,
            device_working_bytes: 0,
        }
    );
    assert_eq!(
        result.loading_peak_footprint,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: HALF_EXECUTION_WEIGHT_BYTES,
            host_working_bytes: expected_host_loading_peak_bytes,
            device_working_bytes: 0,
        }
    );
    assert_eq!(
        result.sequence_cache_bytes_per_token,
        HALF_CACHE_BYTES_PER_TOKEN
    );
    assert_eq!(
        result.sequence_footprint,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 0,
            host_working_bytes: HALF_CUDA_SEQUENCE_HOST_WORKING_BYTES,
            device_working_bytes: HALF_CUDA_SEQUENCE_DEVICE_WORKING_BYTES,
        }
    );
}

struct FixtureExecution {
    declared_scalar_type: Option<ScalarType>,
    observed_scalar_types: ScalarTypeSet,
    execution_scalar_type: ScalarType,
    execution_device: ExecutionDevice,
    reported_footprint: MemoryFootprint,
    loading_peak_footprint: MemoryFootprint,
    sequence_cache_bytes_per_token: u64,
    sequence_footprint: MemoryFootprint,
    transfer_batches: u64,
    loading_device_synchronizations: u64,
    prefill_logits: Vec<f32>,
    decode_logits: Vec<f32>,
}

fn execute_fixture(
    source: &CandleLlamaSource,
    execution_device: ExecutionDevice,
) -> TestResult<FixtureExecution> {
    let LoadedFixture {
        mut model,
        plan,
        declared_scalar_type,
        observed_scalar_types,
        load_observation,
    } = load_fixture(source, execution_device)?;
    let sequence_configuration = SequenceConfiguration::new(
        NonZeroU32::new(16).ok_or_else(|| "maximum tokens must be nonzero".to_owned())?,
        NonZeroU32::new(8).ok_or_else(|| "maximum prefill must be nonzero".to_owned())?,
    );
    let sequence_plan = model
        .plan_sequence(&sequence_configuration)
        .map_err(|error| format!("plan sequence: {error:?}"))?;
    let sequence_cache_bytes_per_token = plan.descriptor.sequence_cache_bytes_per_token;
    let mut sequence = model
        .create_sequence(SequenceId::new(1), &sequence_configuration)
        .map_err(|error| format!("create sequence: {error:?}"))?;
    let prompt = [TokenId::new(1), TokenId::new(2)];
    let mut prefill_logits = vec![0.0_f32; VOCABULARY_SIZE];
    let prefill = prefill_checked(
        &mut model,
        &mut sequence,
        PrefillInput::new(&prompt, true),
        PrefillBuffers::new(&mut prefill_logits),
        CancellationStatus::Running,
    )
    .map_err(|error| format!("prefill: {error:?}"))?;
    assert_eq!(
        prefill,
        PrefillOutcome::Ready {
            consumed_tokens: 2,
            position: 2,
            logits_written: VOCABULARY_SIZE,
        }
    );

    let mut decode_logits = vec![0.0_f32; VOCABULARY_SIZE];
    let decode = decode_checked(
        &mut model,
        &mut sequence,
        DecodeInput::new(TokenId::new(3)),
        DecodeBuffers::new(&mut decode_logits),
        CancellationStatus::Running,
    )
    .map_err(|error| format!("decode: {error:?}"))?;
    assert_eq!(
        decode,
        DecodeOutcome::Ready {
            position: 3,
            logits_written: VOCABULARY_SIZE,
        }
    );

    model
        .synchronize()
        .map_err(|error| format!("synchronize: {error:?}"))?;
    model
        .destroy_sequence(&mut sequence)
        .map_err(|error| format!("destroy sequence: {error:?}"))?;
    drop(sequence);
    let execution_scalar_type = model.execution_scalar_type();
    let reported_footprint = model.reported_footprint();
    model
        .prepare_unload()
        .map_err(|error| format!("prepare unload: {error:?}"))?;
    drop(model);

    Ok(FixtureExecution {
        declared_scalar_type,
        observed_scalar_types,
        execution_scalar_type,
        execution_device,
        reported_footprint,
        loading_peak_footprint: plan.loading_peak_footprint,
        sequence_cache_bytes_per_token,
        sequence_footprint: sequence_plan.reservation.total_footprint,
        transfer_batches: load_observation.transfer_batches,
        loading_device_synchronizations: load_observation.loading_device_synchronizations,
        prefill_logits,
        decode_logits,
    })
}

struct LoadedFixture {
    model: CandleLlamaModel,
    plan: LoadPlan,
    declared_scalar_type: Option<ScalarType>,
    observed_scalar_types: ScalarTypeSet,
    load_observation: candle_backend::CandleLoadObservationSnapshot,
}

fn load_fixture(
    source: &CandleLlamaSource,
    execution_device: ExecutionDevice,
) -> TestResult<LoadedFixture> {
    let (load_observation, load_observation_recorder) = CandleLoadObservation::channel();
    let mut loader = CandleLlamaLoader::with_load_observation(BACKEND, load_observation_recorder);
    let configuration = LoadConfiguration {
        handle: ModelHandle::new(ModelId::new(1), ModelGeneration::new(1)),
        execution_device,
        memory_budget: MemoryBudget {
            host_bytes: u64::MAX,
            device_bytes: u64::MAX,
        },
    };
    let prepared = loader
        .prepare_load(source, &configuration)
        .map_err(|error| format!("prepare fixture on {execution_device:?}: {error:?}"))?;
    let plan = *prepared.plan();
    let declared_scalar_type = plan.descriptor.metadata.configuration_declared_scalar_type;
    let observed_scalar_types = plan.descriptor.metadata.observed_tensor_scalar_types;
    let model = match loader.load_prepared(prepared) {
        Ok(model) => model,
        Err(mut failed) => {
            let primary = failed.primary();
            let cleanup = failed.cleanup();
            return Err(format!(
                "load fixture on {execution_device:?}: {primary:?}; cleanup: {cleanup:?}"
            ));
        }
    };
    assert_eq!(model.descriptor(), &plan.descriptor);
    assert_eq!(model.execution_scalar_type(), plan.execution_scalar_type);
    assert_eq!(model.execution_device(), execution_device);
    assert_eq!(model.reported_footprint(), plan.final_footprint);
    let load_observation = load_observation.snapshot();
    assert_eq!(load_observation.plan, Some(plan));
    assert_eq!(
        load_observation.outcome,
        candle_backend::CandleLoadObservationOutcome::Succeeded
    );

    Ok(LoadedFixture {
        model,
        plan,
        declared_scalar_type,
        observed_scalar_types,
        load_observation,
    })
}

fn fixture_source() -> TestResult<CandleLlamaSource> {
    CandleLlamaSource::from_local_files(
        fixture_directory().join("config.json"),
        vec![fixture_weight_path()],
    )
    .map_err(|error| error.to_string())
}

fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/inference-runtime/tests/fixtures/candle-llama")
}

fn fixture_weight_path() -> PathBuf {
    fixture_directory().join("model.safetensors")
}

fn require_cuda_opt_in() -> TestResult {
    if std::env::var("MILKDRIFT_CUDA_TEST").as_deref() == Ok("1") {
        Ok(())
    } else {
        Err("set MILKDRIFT_CUDA_TEST=1 to execute CUDA hardware tests".to_owned())
    }
}

fn assert_logits_close(left: &[f32], right: &[f32], tolerance: f32) -> TestResult {
    if left.len() != right.len() {
        return Err(format!(
            "logit lengths differ: {} != {}",
            left.len(),
            right.len()
        ));
    }
    for (index, (left, right)) in left.iter().zip(right).enumerate() {
        if (left - right).abs() > tolerance {
            return Err(format!(
                "logit {index} differs: {left} != {right} with tolerance {tolerance}"
            ));
        }
    }
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

fn mixed_set(primary: ScalarType) -> ScalarTypeSet {
    ScalarTypeSet::from_scalar(primary).union(ScalarTypeSet::from_scalar(ScalarType::F32))
}

struct ConvertedFixture {
    directory: PathBuf,
    config_path: PathBuf,
    weight_path: PathBuf,
}

impl ConvertedFixture {
    fn create(primary_dtype: DType, mixed_f32: bool) -> TestResult<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "milkdrift-cuda-phase12-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let config_path = directory.join("config.json");
        let weight_path = directory.join("model.safetensors");
        write_converted_config(&config_path, primary_dtype)?;
        convert_weights(
            &fixture_weight_path(),
            &weight_path,
            primary_dtype,
            mixed_f32,
        )?;
        Ok(Self {
            directory,
            config_path,
            weight_path,
        })
    }

    fn source(&self) -> TestResult<CandleLlamaSource> {
        CandleLlamaSource::from_local_files(
            self.config_path.clone(),
            vec![self.weight_path.clone()],
        )
        .map_err(|error| error.to_string())
    }
}

impl Drop for ConvertedFixture {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.directory);
    }
}

fn write_converted_config(destination: &Path, primary_dtype: DType) -> TestResult {
    let bytes = fs::read(fixture_directory().join("config.json"))
        .map_err(|error| format!("read conversion config: {error}"))?;
    let mut config: JsonValue = serde_json::from_slice(&bytes)
        .map_err(|error| format!("decode conversion config: {error}"))?;
    let object = config
        .as_object_mut()
        .ok_or_else(|| "conversion config must be a JSON object".to_owned())?;
    let declaration = match primary_dtype {
        DType::F32 => "float32",
        DType::F16 => "float16",
        DType::BF16 => "bfloat16",
        _ => {
            return Err(format!(
                "unsupported converted config dtype: {primary_dtype:?}"
            ));
        }
    };
    object.insert("model_type".to_owned(), json!("llama"));
    object.insert("dtype".to_owned(), json!(declaration));
    object.remove("torch_dtype");
    let mut encoded = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("encode conversion config: {error}"))?;
    encoded.push(b'\n');
    fs::write(destination, encoded).map_err(|error| format!("write conversion config: {error}"))
}

fn convert_weights(
    source: &Path,
    destination: &Path,
    primary_dtype: DType,
    mixed_f32: bool,
) -> TestResult {
    let tensors = candle_core::safetensors::load(source, &Device::Cpu)
        .map_err(|error| format!("load F32 conversion source: {error}"))?;
    let converted = tensors
        .into_iter()
        .map(|(name, tensor)| {
            let dtype = if mixed_f32 && name == "model.norm.weight" {
                DType::F32
            } else {
                primary_dtype
            };
            let tensor = tensor
                .to_dtype(dtype)
                .map_err(|error| format!("convert {name} to {dtype:?}: {error}"))?;
            Ok((name, tensor))
        })
        .collect::<TestResult<HashMap<String, Tensor>>>()?;
    candle_core::safetensors::save(&converted, destination)
        .map_err(|error| format!("save converted fixture: {error}"))
}
