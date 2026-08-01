//! Explicit maintenance generator for the committed Candle integration fixture.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Tensor};

const CONFIG: &str = r#"{
  "model_type": "llama",
  "vocab_size": 16,
  "hidden_size": 8,
  "intermediate_size": 16,
  "num_hidden_layers": 1,
  "num_attention_heads": 2,
  "num_key_value_heads": 2,
  "max_position_embeddings": 16,
  "rms_norm_eps": 0.00001,
  "rope_theta": 10000.0,
  "rope_scaling": null,
  "bos_token_id": 1,
  "eos_token_id": 2,
  "tie_word_embeddings": false
}
"#;
const TOKEN_MATRIX_MAGNITUDE: f32 = 0.5;

type TestResult = Result<(), String>;

#[test]
#[ignore = "explicit source-tree fixture maintenance operation"]
fn regenerate_committed_candle_fixture() -> TestResult {
    let directory = fixture_directory();
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "failed to create fixture directory {}: {error}",
            directory.display()
        )
    })?;
    fs::write(directory.join("config.json"), CONFIG)
        .map_err(|error| format!("failed to write fixture config: {error}"))?;
    write_weights(&directory.join("model.safetensors"))
}

fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/inference-runtime/tests/fixtures/candle-llama")
}

fn write_weights(path: &Path) -> TestResult {
    let device = Device::Cpu;
    let mut tensors = HashMap::new();
    insert_token_matrix(&mut tensors, "model.embed_tokens.weight", &device)?;
    insert_token_matrix(&mut tensors, "lm_head.weight", &device)?;
    insert_vector(&mut tensors, "model.norm.weight", 8, &device)?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.self_attn.q_proj.weight",
        8,
        8,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.self_attn.k_proj.weight",
        8,
        8,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.self_attn.v_proj.weight",
        8,
        8,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.self_attn.o_proj.weight",
        8,
        8,
        &device,
    )?;
    insert_vector(
        &mut tensors,
        "model.layers.0.input_layernorm.weight",
        8,
        &device,
    )?;
    insert_vector(
        &mut tensors,
        "model.layers.0.post_attention_layernorm.weight",
        8,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.mlp.gate_proj.weight",
        16,
        8,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.mlp.up_proj.weight",
        16,
        8,
        &device,
    )?;
    insert_matrix(
        &mut tensors,
        "model.layers.0.mlp.down_proj.weight",
        8,
        16,
        &device,
    )?;
    candle_core::safetensors::save(&tensors, path)
        .map_err(|error| format!("failed to save fixture weights: {error}"))
}

fn insert_token_matrix(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    device: &Device,
) -> TestResult {
    let values = (0_u32..16)
        .flat_map(|token| {
            (0_u32..8).map(move |dimension| {
                if token & (1_u32 << (dimension % 4)) == 0 {
                    -TOKEN_MATRIX_MAGNITUDE
                } else {
                    TOKEN_MATRIX_MAGNITUDE
                }
            })
        })
        .collect::<Vec<_>>();
    let tensor = Tensor::from_vec(values, (16, 8), device)
        .map_err(|error| format!("failed to create token matrix: {error}"))?;
    insert_tensor(tensors, name, tensor)
}

fn insert_matrix(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    rows: usize,
    columns: usize,
    device: &Device,
) -> TestResult {
    let tensor = Tensor::zeros((rows, columns), DType::F32, device)
        .map_err(|error| format!("failed to create zero matrix: {error}"))?;
    insert_tensor(tensors, name, tensor)
}

fn insert_vector(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    length: usize,
    device: &Device,
) -> TestResult {
    let tensor = Tensor::ones(length, DType::F32, device)
        .map_err(|error| format!("failed to create unit vector: {error}"))?;
    insert_tensor(tensors, name, tensor)
}

fn insert_tensor(tensors: &mut HashMap<String, Tensor>, name: &str, tensor: Tensor) -> TestResult {
    if tensors.insert(name.to_owned(), tensor).is_some() {
        return Err(format!("duplicate fixture tensor name: {name}"));
    }
    Ok(())
}
