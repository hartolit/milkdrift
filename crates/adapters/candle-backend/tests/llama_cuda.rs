//! Opt-in CUDA execution evidence for the committed synthetic Llama fixture.

#![cfg(feature = "cuda")]

use std::collections::HashMap;
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use candle_backend::{CandleLlamaLoader, CandleLlamaSource, CandleScalarType};
use candle_core::{DType, Device, Tensor};
use domain_contracts::{
    BackendFailureKind, BackendId, CancellationStatus, DecodeBuffers, DecodeInput, DecodeOutcome,
    DeviceId, DeviceKind, ExecutionDevice, LoadConfiguration, LoadedModel, MemoryBudget,
    MemoryFootprint, ModelGeneration, ModelHandle, ModelId, ModelLoader, PrefillBuffers,
    PrefillInput, PrefillOutcome, SequenceConfiguration, SequenceId, TokenId, decode_checked,
    prefill_checked,
};

const BACKEND: BackendId = BackendId::new(501);
const CPU: ExecutionDevice = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
const CUDA_0: ExecutionDevice = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);
const VOCABULARY_SIZE: usize = 16;

type TestResult<T = ()> = Result<T, String>;

#[test]
fn cuda_enabled_binary_can_explicitly_execute_cpu() -> TestResult {
    let source = fixture_source(CandleScalarType::F32, fixture_weight_path())?;
    let result = execute_fixture(&source, CPU)?;

    assert_eq!(result.execution_device, CPU);
    assert_eq!(result.execution_scalar, CandleScalarType::F32);
    assert!(result.model_footprint.host_weight_bytes > 0);
    assert_eq!(result.model_footprint.device_weight_bytes, 0);
    assert!(result.sequence_footprint.host_working_bytes > 0);
    assert_eq!(result.sequence_footprint.device_working_bytes, 0);
    assert_eq!(
        maximum_logit_token(&result.prefill_logits)?,
        TokenId::new(2)
    );
    assert_eq!(maximum_logit_token(&result.decode_logits)?, TokenId::new(3));
    Ok(())
}

#[test]
fn invalid_cuda_ordinal_is_rejected_before_driver_initialization() {
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_eq!(
        loader.discover_device(ExecutionDevice::new(
            DeviceId::new(u64::MAX),
            DeviceKind::Cuda,
        )),
        Err(domain_contracts::LoadError::InvalidConfiguration)
    );
}

#[test]
#[ignore = "requires MILKDRIFT_CUDA_TEST=1 and CUDA ordinal 0"]
fn cuda_ordinal_zero_executes_fixture_and_matches_cpu_logits() -> TestResult {
    require_cuda_opt_in()?;
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
            if failure.kind == BackendFailureKind::DeviceInitialization
    ));

    let cpu = execute_fixture(
        &fixture_source(CandleScalarType::F32, fixture_weight_path())?,
        CPU,
    )?;
    let cuda = execute_fixture(
        &fixture_source(CandleScalarType::F32, fixture_weight_path())?,
        CUDA_0,
    )?;

    assert_eq!(cuda.execution_device, CUDA_0);
    assert_eq!(cuda.model_footprint.host_weight_bytes, 0);
    assert!(cuda.model_footprint.device_weight_bytes > 0);
    assert!(cuda.model_footprint.host_working_bytes > 0);
    assert_eq!(cuda.sequence_footprint.host_working_bytes, 0);
    assert!(cuda.sequence_footprint.device_working_bytes > 0);
    assert!(cuda.sequence_footprint.cache_bytes_per_token > 0);
    assert_logits_close(&cpu.prefill_logits, &cuda.prefill_logits, 1.0e-3)?;
    assert_logits_close(&cpu.decode_logits, &cuda.decode_logits, 1.0e-3)?;
    assert_eq!(maximum_logit_token(&cuda.prefill_logits)?, TokenId::new(2));
    assert_eq!(maximum_logit_token(&cuda.decode_logits)?, TokenId::new(3));
    Ok(())
}

#[test]
#[ignore = "requires MILKDRIFT_CUDA_TEST=1 and CUDA ordinal 0"]
fn cuda_bf16_source_executes_as_bf16() -> TestResult {
    require_cuda_opt_in()?;
    let converted = ConvertedFixture::bf16()?;
    let source = fixture_source(CandleScalarType::Bf16, converted.weight_path.clone())?;
    let result = execute_fixture(&source, CUDA_0)?;

    assert_eq!(result.execution_device, CUDA_0);
    assert_eq!(result.execution_scalar, CandleScalarType::Bf16);
    assert!(result.model_footprint.device_weight_bytes > 0);
    assert_eq!(result.model_footprint.host_weight_bytes, 0);
    assert!(result.sequence_footprint.device_working_bytes > 0);
    assert_eq!(
        maximum_logit_token(&result.prefill_logits)?,
        TokenId::new(2)
    );
    Ok(())
}

struct FixtureExecution {
    execution_device: ExecutionDevice,
    execution_scalar: CandleScalarType,
    model_footprint: MemoryFootprint,
    sequence_footprint: MemoryFootprint,
    prefill_logits: Vec<f32>,
    decode_logits: Vec<f32>,
}

fn execute_fixture(
    source: &CandleLlamaSource,
    execution_device: ExecutionDevice,
) -> TestResult<FixtureExecution> {
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let configuration = LoadConfiguration {
        handle: ModelHandle::new(ModelId::new(1), ModelGeneration::new(1)),
        execution_device,
        memory_budget: MemoryBudget {
            host_bytes: u64::MAX,
            device_bytes: u64::MAX,
        },
    };
    let plan = loader
        .plan_load(source, &configuration)
        .map_err(|error| format!("plan fixture on {execution_device:?}: {error:?}"))?;
    let mut model = loader
        .load(source, &configuration)
        .map_err(|error| format!("load fixture on {execution_device:?}: {error:?}"))?;
    assert_eq!(model.execution_device(), execution_device);
    assert_eq!(model.resident_footprint(), plan.expected_footprint);

    let sequence_configuration = SequenceConfiguration::new(
        NonZeroU32::new(16).ok_or_else(|| "maximum tokens must be nonzero".to_owned())?,
        NonZeroU32::new(8).ok_or_else(|| "maximum prefill must be nonzero".to_owned())?,
    );
    let sequence_plan = model
        .plan_sequence(&sequence_configuration)
        .map_err(|error| format!("plan sequence: {error:?}"))?;
    assert_eq!(
        sequence_plan.expected_footprint.cache_bytes_per_token,
        plan.expected_footprint.cache_bytes_per_token,
        "sequence accounting must use the accepted execution dtype"
    );
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
    let execution_scalar = model.execution_scalar_type();
    model
        .prepare_unload()
        .map_err(|error| format!("prepare unload: {error:?}"))?;

    Ok(FixtureExecution {
        execution_device,
        execution_scalar,
        model_footprint: plan.expected_footprint,
        sequence_footprint: sequence_plan.expected_footprint,
        prefill_logits,
        decode_logits,
    })
}

fn fixture_source(
    scalar_type: CandleScalarType,
    weight_path: PathBuf,
) -> TestResult<CandleLlamaSource> {
    CandleLlamaSource::new(
        fixture_directory().join("config.json"),
        vec![weight_path],
        scalar_type,
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

struct ConvertedFixture {
    directory: PathBuf,
    weight_path: PathBuf,
}

impl ConvertedFixture {
    fn bf16() -> TestResult<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "milkdrift-cuda-bf16-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let weight_path = directory.join("model.safetensors");
        convert_weights_to_bf16(&fixture_weight_path(), &weight_path)?;
        Ok(Self {
            directory,
            weight_path,
        })
    }
}

impl Drop for ConvertedFixture {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.directory);
    }
}

fn convert_weights_to_bf16(source: &Path, destination: &Path) -> TestResult {
    let tensors = candle_core::safetensors::load(source, &Device::Cpu)
        .map_err(|error| format!("load F32 fixture for BF16 conversion: {error}"))?;
    let converted = tensors
        .into_iter()
        .map(|(name, tensor)| {
            let tensor = tensor
                .to_dtype(DType::BF16)
                .map_err(|error| format!("convert {name} to BF16: {error}"))?;
            Ok((name, tensor))
        })
        .collect::<TestResult<HashMap<String, Tensor>>>()?;
    candle_core::safetensors::save(&converted, destination)
        .map_err(|error| format!("save BF16 fixture: {error}"))
}
