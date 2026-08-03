//! End-to-end compatibility tests for the Candle CPU Llama adapter.

use std::collections::HashMap;
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use candle_backend::{
    CandleLlamaLoader, CandleLlamaModel, CandleLlamaSequence, CandleLlamaSource, CandleScalarType,
};
use candle_core::{DType, Device, Tensor};
use domain_contracts::{
    BackendId, BackendSequence, CancellationReason, CancellationStatus, CapabilitySet,
    DecodeBuffers, DecodeInput, DecodeOutcome, DeviceId, DeviceKind, DrainTimeout, ExecutionDevice,
    LifecycleAction, LoadConfiguration, LoadedModel, MemoryBudget, ModelDescriptor,
    ModelGeneration, ModelHandle, ModelId, ModelLifecycle, ModelLoader, MonotonicMillis,
    PrefillBuffers, PrefillInput, PrefillOutcome, SequenceConfiguration, SequenceId, SequenceState,
    TokenId, UnloadPolicy, decode_checked, prefill_checked,
};

type TestResult = Result<(), &'static str>;

#[test]
fn loads_two_sequences_and_unloads_after_bounded_drain() -> TestResult {
    let fixture = TinyLlamaFixture::create()?;
    let source = fixture.source()?;
    let mut loader = CandleLlamaLoader::new(BackendId::new(1));
    let configuration = load_configuration();
    let descriptor = loader.inspect(&source).map_err(|_| "inspect model")?;
    assert_capabilities(&descriptor);
    let plan = loader
        .plan_load(&source, &configuration)
        .map_err(|_| "plan model")?;
    assert_eq!(plan.expected_footprint, descriptor.estimated_footprint);

    let mut model = loader
        .load(&source, &configuration)
        .map_err(|_| "load model")?;
    assert_eq!(model.execution_device(), configuration.execution_device);
    assert_eq!(model.resident_footprint(), plan.expected_footprint);
    let sequence_configuration = SequenceConfiguration::new(
        NonZeroU32::new(16).ok_or("maximum tokens")?,
        NonZeroU32::new(8).ok_or("maximum prefill")?,
    );
    let mut first = model
        .create_sequence(SequenceId::new(1), &sequence_configuration)
        .map_err(|_| "create first sequence")?;
    let mut second = model
        .create_sequence(SequenceId::new(2), &sequence_configuration)
        .map_err(|_| "create second sequence")?;

    exercise_sequences(&mut model, &mut first, &mut second)?;
    unload_after_bounded_drain(model, first, second)
}

fn assert_capabilities(descriptor: &ModelDescriptor) {
    let operations = descriptor.capabilities.operations;
    assert!(operations.contains(CapabilitySet::PREFILL));
    assert!(operations.contains(CapabilitySet::MULTIPLE_SEQUENCES));
    assert!(!operations.contains(CapabilitySet::ALLOCATION_FREE_HOT_PATH));
    assert!(!operations.contains(CapabilitySet::SEQUENCE_RESET));
}

fn exercise_sequences(
    model: &mut CandleLlamaModel,
    first: &mut CandleLlamaSequence,
    second: &mut CandleLlamaSequence,
) -> TestResult {
    let prompt = [TokenId::new(1), TokenId::new(2)];
    let mut first_logits = [0.0_f32; 16];
    let first_prefill = prefill_checked(
        model,
        first,
        PrefillInput::new(&prompt, true),
        PrefillBuffers::new(&mut first_logits),
        CancellationStatus::Running,
    )
    .map_err(|_| "prefill first sequence")?;
    assert_eq!(
        first_prefill,
        PrefillOutcome::Ready {
            consumed_tokens: 2,
            position: 2,
            logits_written: 16,
        }
    );
    assert_eq!(maximum_logit_token(&first_logits)?, TokenId::new(2));

    let mut second_logits = [0.0_f32; 16];
    let second_prefill = prefill_checked(
        model,
        second,
        PrefillInput::new(&prompt, true),
        PrefillBuffers::new(&mut second_logits),
        CancellationStatus::Running,
    )
    .map_err(|_| "prefill second sequence")?;
    assert_eq!(
        second_prefill,
        PrefillOutcome::Ready {
            consumed_tokens: 2,
            position: 2,
            logits_written: 16,
        }
    );

    let first_decode = decode_checked(
        model,
        first,
        DecodeInput::new(TokenId::new(3)),
        DecodeBuffers::new(&mut first_logits),
        CancellationStatus::Running,
    )
    .map_err(|_| "decode first sequence")?;
    assert_eq!(
        first_decode,
        DecodeOutcome::Ready {
            position: 3,
            logits_written: 16,
        }
    );
    assert_eq!(maximum_logit_token(&first_logits)?, TokenId::new(3));

    let second_decode = decode_checked(
        model,
        second,
        DecodeInput::new(TokenId::new(4)),
        DecodeBuffers::new(&mut second_logits),
        CancellationStatus::Running,
    )
    .map_err(|_| "decode second sequence")?;
    assert_eq!(
        second_decode,
        DecodeOutcome::Ready {
            position: 3,
            logits_written: 16,
        }
    );
    assert_eq!(maximum_logit_token(&second_logits)?, TokenId::new(4));

    let cancelled_position = first.position();
    let cancelled = decode_checked(
        model,
        first,
        DecodeInput::new(TokenId::new(5)),
        DecodeBuffers::new(&mut first_logits),
        CancellationStatus::Requested(CancellationReason::UserRequested),
    )
    .map_err(|_| "cancel first sequence")?;
    assert_eq!(
        cancelled,
        DecodeOutcome::Finished(domain_contracts::FinishReason::Cancelled(
            CancellationReason::UserRequested,
        ))
    );
    assert_eq!(first.position(), cancelled_position);
    assert_eq!(
        model.reset_sequence(first),
        Err(domain_contracts::SequenceError::Unsupported)
    );
    assert_eq!(first.state(), SequenceState::Ready);
    assert_eq!(first.position(), cancelled_position);
    Ok(())
}

fn unload_after_bounded_drain(
    mut model: CandleLlamaModel,
    mut first: CandleLlamaSequence,
    mut second: CandleLlamaSequence,
) -> TestResult {
    let mut lifecycle = ModelLifecycle::new();
    lifecycle.begin_load().map_err(|_| "begin lifecycle load")?;
    lifecycle
        .complete_load()
        .map_err(|_| "complete lifecycle load")?;
    lifecycle
        .start_request()
        .map_err(|_| "start first request")?;
    lifecycle
        .start_request()
        .map_err(|_| "start second request")?;
    let timeout = DrainTimeout::from_millis(10).map_err(|_| "create drain timeout")?;
    let action = lifecycle
        .request_unload(UnloadPolicy::Drain { timeout }, MonotonicMillis::new(100))
        .map_err(|_| "request drain")?;
    assert_eq!(action, LifecycleAction::None);
    assert_eq!(
        lifecycle
            .poll(MonotonicMillis::new(109))
            .map_err(|_| "poll drain")?,
        LifecycleAction::None
    );
    assert_eq!(
        lifecycle
            .poll(MonotonicMillis::new(110))
            .map_err(|_| "expire drain")?,
        LifecycleAction::CancelActive {
            reason: CancellationReason::DrainTimeout,
        }
    );
    assert_eq!(
        lifecycle
            .finish_request()
            .map_err(|_| "finish first request")?,
        LifecycleAction::None
    );
    assert_eq!(
        lifecycle
            .finish_request()
            .map_err(|_| "finish second request")?,
        LifecycleAction::ReleaseModel
    );

    model
        .destroy_sequence(&mut first)
        .map_err(|_| "destroy first sequence")?;
    model
        .destroy_sequence(&mut second)
        .map_err(|_| "destroy second sequence")?;
    assert_eq!(first.state(), SequenceState::Finished);
    assert_eq!(second.state(), SequenceState::Finished);
    drop(first);
    drop(second);
    model.synchronize().map_err(|_| "synchronize model")?;
    model.prepare_unload().map_err(|_| "prepare unload")?;
    drop(model);
    assert_eq!(
        lifecycle.complete_unload().map_err(|_| "complete unload")?,
        LifecycleAction::UnloadComplete
    );
    Ok(())
}

#[test]
fn advertised_scalar_types_produce_f32_vocabulary_logits() -> TestResult {
    for (index, scalar_type) in [
        CandleScalarType::F32,
        CandleScalarType::F16,
        CandleScalarType::Bf16,
    ]
    .into_iter()
    .enumerate()
    {
        let fixture = TinyLlamaFixture::create_with_scalar(scalar_type)?;
        let source = fixture.source()?;
        let mut loader = CandleLlamaLoader::new(BackendId::new(
            u64::try_from(index + 10).map_err(|_| "backend identifier")?,
        ));
        let descriptor = loader
            .inspect(&source)
            .map_err(|_| "inspect scalar fixture")?;
        if scalar_type == CandleScalarType::Bf16 {
            let source_bytes = fs::metadata(&fixture.weight_path)
                .map_err(|_| "inspect scalar weight file")?
                .len();
            assert!(descriptor.estimated_footprint.host_weight_bytes >= source_bytes * 2);
            assert_eq!(descriptor.estimated_footprint.cache_bytes_per_token % 4, 0);
        }
        let mut model = loader
            .load(&source, &load_configuration())
            .map_err(|_| "load scalar fixture")?;
        let configuration = SequenceConfiguration::new(
            NonZeroU32::new(16).ok_or("maximum tokens")?,
            NonZeroU32::new(8).ok_or("maximum prefill")?,
        );
        let mut sequence = model
            .create_sequence(SequenceId::new(10), &configuration)
            .map_err(|_| "create scalar sequence")?;
        let prompt = [TokenId::new(1), TokenId::new(2)];
        let mut logits = [0.0_f32; 16];
        let outcome = prefill_checked(
            &mut model,
            &mut sequence,
            PrefillInput::new(&prompt, true),
            PrefillBuffers::new(&mut logits),
            CancellationStatus::Running,
        )
        .map_err(|_| match scalar_type {
            CandleScalarType::F32 => "prefill failed for F32",
            CandleScalarType::F16 => "prefill failed for F16",
            CandleScalarType::Bf16 => "prefill failed for BF16",
        })?;

        assert_eq!(
            outcome,
            PrefillOutcome::Ready {
                consumed_tokens: 2,
                position: 2,
                logits_written: 16,
            }
        );
        assert_eq!(maximum_logit_token(&logits)?, TokenId::new(2));
        model
            .destroy_sequence(&mut sequence)
            .map_err(|_| "destroy scalar sequence")?;
        assert_eq!(sequence.state(), SequenceState::Finished);
        drop(sequence);
        model.prepare_unload().map_err(|_| "unload scalar model")?;
    }
    Ok(())
}

#[test]
fn rejects_weight_dtype_mismatch() -> TestResult {
    let fixture = TinyLlamaFixture::create()?;
    let source = CandleLlamaSource::new(
        fixture.config_path.clone(),
        vec![fixture.weight_path.clone()],
        CandleScalarType::F16,
    )
    .map_err(|_| "create mismatched source")?;
    let mut loader = CandleLlamaLoader::new(BackendId::new(3));
    assert!(matches!(
        loader.load(&source, &load_configuration()),
        Err(domain_contracts::LoadError::UnsupportedFormat)
    ));
    Ok(())
}

#[test]
fn rejects_invalid_cpu_identity_unsupported_devices_and_insufficient_host_memory() -> TestResult {
    let fixture = TinyLlamaFixture::create()?;
    let source = fixture.source()?;
    let loader = CandleLlamaLoader::new(BackendId::new(2));
    let mut configuration = load_configuration();
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(1), DeviceKind::Cpu);
    assert_eq!(
        loader.plan_load(&source, &configuration),
        Err(domain_contracts::LoadError::InvalidConfiguration)
    );

    for kind in [DeviceKind::Metal, DeviceKind::Accelerator(1)] {
        configuration.execution_device = ExecutionDevice::new(DeviceId::new(0), kind);
        assert!(matches!(
            loader.plan_load(&source, &configuration),
            Err(domain_contracts::LoadError::Backend(failure))
                if failure.kind == domain_contracts::BackendFailureKind::Unsupported
        ));
    }

    configuration.execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
    configuration.memory_budget.host_bytes = 1;
    assert!(matches!(
        loader.plan_load(&source, &configuration),
        Err(domain_contracts::LoadError::InsufficientMemory {
            kind: domain_contracts::MemoryKind::Host,
            ..
        })
    ));
    Ok(())
}

#[cfg(not(feature = "cuda"))]
#[test]
fn cuda_request_fails_explicitly_when_support_is_not_compiled() -> TestResult {
    let fixture = TinyLlamaFixture::create()?;
    let source = fixture.source()?;
    let loader = CandleLlamaLoader::new(BackendId::new(4));
    let mut configuration = load_configuration();
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);
    configuration.memory_budget.device_bytes = u64::MAX;

    assert!(matches!(
        loader.plan_load(&source, &configuration),
        Err(domain_contracts::LoadError::Backend(failure))
            if failure.kind == domain_contracts::BackendFailureKind::Unsupported
    ));
    Ok(())
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
    scalar_type: CandleScalarType,
}

impl TinyLlamaFixture {
    fn create() -> Result<Self, &'static str> {
        Self::create_with_scalar(CandleScalarType::F32)
    }

    fn create_with_scalar(scalar_type: CandleScalarType) -> Result<Self, &'static str> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock")?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "llm-app-candle-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).map_err(|_| "create fixture directory")?;
        let config_path = directory.join("config.json");
        let weight_path = directory.join("model.safetensors");
        fs::write(&config_path, TINY_CONFIG).map_err(|_| "write config")?;
        write_weights(&weight_path, scalar_type)?;

        Ok(Self {
            directory,
            config_path,
            weight_path,
            scalar_type,
        })
    }

    fn source(&self) -> Result<CandleLlamaSource, &'static str> {
        CandleLlamaSource::new(
            self.config_path.clone(),
            vec![self.weight_path.clone()],
            self.scalar_type,
        )
        .map_err(|_| "create source")
    }
}

impl Drop for TinyLlamaFixture {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.directory);
    }
}

fn write_weights(path: &Path, scalar_type: CandleScalarType) -> Result<(), &'static str> {
    let device = Device::Cpu;
    let dtype = scalar_dtype(scalar_type);
    let mut tensors = HashMap::new();
    insert_token_matrix(&mut tensors, "model.embed_tokens.weight", dtype, &device)?;
    insert_token_matrix(&mut tensors, "lm_head.weight", dtype, &device)?;
    insert_vector(&mut tensors, "model.norm.weight", 8, dtype, &device)?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.self_attn.q_proj.weight",
        8,
        8,
        dtype,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.self_attn.k_proj.weight",
        8,
        8,
        dtype,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.self_attn.v_proj.weight",
        8,
        8,
        dtype,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.self_attn.o_proj.weight",
        8,
        8,
        dtype,
        &device,
    )?;
    insert_vector(
        &mut tensors,
        "model.layers.0.input_layernorm.weight",
        8,
        dtype,
        &device,
    )?;
    insert_vector(
        &mut tensors,
        "model.layers.0.post_attention_layernorm.weight",
        8,
        dtype,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.mlp.gate_proj.weight",
        16,
        8,
        dtype,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.mlp.up_proj.weight",
        16,
        8,
        dtype,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.mlp.down_proj.weight",
        8,
        16,
        dtype,
        &device,
    )?;
    candle_core::safetensors::save(&tensors, path).map_err(|_| "save weights")
}

fn insert_token_matrix(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    dtype: DType,
    device: &Device,
) -> Result<(), &'static str> {
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
        .and_then(|tensor| tensor.to_dtype(dtype))
        .map_err(|_| "create token matrix")?;
    if tensors.insert(name.to_owned(), tensor).is_some() {
        return Err("duplicate token matrix name");
    }
    Ok(())
}

fn insert_matrix(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    rows: usize,
    columns: usize,
    dtype: DType,
    device: &Device,
) -> Result<(), &'static str> {
    let tensor = Tensor::zeros((rows, columns), dtype, device).map_err(|_| "create matrix")?;
    if tensors.insert(name.to_owned(), tensor).is_some() {
        return Err("duplicate matrix name");
    }
    Ok(())
}

fn insert_vector(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    length: usize,
    dtype: DType,
    device: &Device,
) -> Result<(), &'static str> {
    let tensor = Tensor::ones(length, dtype, device).map_err(|_| "create vector")?;
    if tensors.insert(name.to_owned(), tensor).is_some() {
        return Err("duplicate vector name");
    }
    Ok(())
}

fn maximum_logit_token(logits: &[f32]) -> Result<TokenId, &'static str> {
    let (index, _) = logits
        .iter()
        .copied()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .ok_or("empty logits")?;
    let token = u32::try_from(index).map_err(|_| "token identifier overflow")?;
    Ok(TokenId::new(token))
}

const fn scalar_dtype(scalar_type: CandleScalarType) -> DType {
    match scalar_type {
        CandleScalarType::F32 => DType::F32,
        CandleScalarType::F16 => DType::F16,
        CandleScalarType::Bf16 => DType::BF16,
    }
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
