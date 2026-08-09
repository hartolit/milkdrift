//! Supported Candle Llama tensor schema and required-tensor marking.

use std::fmt::Write;

use candle_transformers::models::llama::Config;
use domain_contracts::{BackendId, LoadError, ScalarTypeSet};

use crate::failure::{
    CODE_DUPLICATE_TENSOR, CODE_INSPECTION_ALLOCATION, CODE_NUMERIC_OVERFLOW, CODE_REQUIRED_TENSOR,
};

use super::manifest::InspectedShard;
use super::{host_memory_failure, invalid_model_failure, unsupported_scalar};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RequiredSchema {
    pub(super) scalar_types: ScalarTypeSet,
    pub(super) tensor_count: usize,
}

#[derive(Clone, Copy, Debug)]
struct TensorLocation {
    shard: usize,
    tensor: usize,
}

#[derive(Debug)]
struct TensorIndex {
    locations: Vec<TensorLocation>,
}

pub(super) fn validate_and_mark(
    backend: BackendId,
    config: &Config,
    shards: &mut [InspectedShard],
) -> Result<RequiredSchema, LoadError> {
    let index = build_index(backend, shards)?;
    let base_tensor_count = if config.tie_word_embeddings { 2 } else { 3 };
    let expected_count = config
        .num_hidden_layers
        .checked_mul(9)
        .and_then(|count| count.checked_add(base_tensor_count))
        .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
    if expected_count > index.locations.len() {
        return Err(invalid_model_failure(backend, CODE_REQUIRED_TENSOR));
    }

    let mut scalar_types = ScalarTypeSet::EMPTY;
    mark_required(
        backend,
        shards,
        &index,
        "model.embed_tokens.weight",
        &[config.vocab_size, config.hidden_size],
        &mut scalar_types,
    )?;
    if !config.tie_word_embeddings {
        mark_required(
            backend,
            shards,
            &index,
            "lm_head.weight",
            &[config.vocab_size, config.hidden_size],
            &mut scalar_types,
        )?;
    }
    mark_required(
        backend,
        shards,
        &index,
        "model.norm.weight",
        &[config.hidden_size],
        &mut scalar_types,
    )?;

    mark_layers(backend, config, shards, &index, &mut scalar_types)?;
    Ok(RequiredSchema {
        scalar_types,
        tensor_count: expected_count,
    })
}

fn build_index(backend: BackendId, shards: &[InspectedShard]) -> Result<TensorIndex, LoadError> {
    let tensor_count = shards.iter().try_fold(0_usize, |total, shard| {
        total
            .checked_add(shard.tensors.len())
            .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))
    })?;
    let mut locations = Vec::new();
    locations
        .try_reserve_exact(tensor_count)
        .map_err(|_| host_memory_failure(backend, CODE_INSPECTION_ALLOCATION))?;
    for (shard_index, shard) in shards.iter().enumerate() {
        locations.extend(shard.tensors.iter().enumerate().map(|(tensor_index, _)| {
            TensorLocation {
                shard: shard_index,
                tensor: tensor_index,
            }
        }));
    }
    locations.sort_unstable_by(|left, right| {
        tensor_name(shards, *left)
            .unwrap_or_default()
            .cmp(tensor_name(shards, *right).unwrap_or_default())
    });
    if locations.windows(2).any(|pair| {
        pair.first().and_then(|left| tensor_name(shards, *left))
            == pair.get(1).and_then(|right| tensor_name(shards, *right))
    }) {
        return Err(invalid_model_failure(backend, CODE_DUPLICATE_TENSOR));
    }
    Ok(TensorIndex { locations })
}

fn tensor_name(shards: &[InspectedShard], location: TensorLocation) -> Option<&str> {
    shards
        .get(location.shard)
        .and_then(|shard| shard.tensors.get(location.tensor))
        .map(|tensor| tensor.name.as_str())
}

fn mark_layers(
    backend: BackendId,
    config: &Config,
    shards: &mut [InspectedShard],
    index: &TensorIndex,
    scalar_types: &mut ScalarTypeSet,
) -> Result<(), LoadError> {
    let head_dimension = config.hidden_size / config.num_attention_heads;
    let query_size = head_dimension
        .checked_mul(config.num_attention_heads)
        .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
    let key_value_size = head_dimension
        .checked_mul(config.num_key_value_heads)
        .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;

    for layer in 0..config.num_hidden_layers {
        mark_layer_tensor(
            backend,
            shards,
            index,
            scalar_types,
            layer,
            ".self_attn.q_proj.weight",
            &[query_size, config.hidden_size],
        )?;
        for suffix in [".self_attn.k_proj.weight", ".self_attn.v_proj.weight"] {
            mark_layer_tensor(
                backend,
                shards,
                index,
                scalar_types,
                layer,
                suffix,
                &[key_value_size, config.hidden_size],
            )?;
        }
        mark_layer_tensor(
            backend,
            shards,
            index,
            scalar_types,
            layer,
            ".self_attn.o_proj.weight",
            &[config.hidden_size, query_size],
        )?;
        for suffix in [
            ".input_layernorm.weight",
            ".post_attention_layernorm.weight",
        ] {
            mark_layer_tensor(
                backend,
                shards,
                index,
                scalar_types,
                layer,
                suffix,
                &[config.hidden_size],
            )?;
        }
        for suffix in [".mlp.gate_proj.weight", ".mlp.up_proj.weight"] {
            mark_layer_tensor(
                backend,
                shards,
                index,
                scalar_types,
                layer,
                suffix,
                &[config.intermediate_size, config.hidden_size],
            )?;
        }
        mark_layer_tensor(
            backend,
            shards,
            index,
            scalar_types,
            layer,
            ".mlp.down_proj.weight",
            &[config.hidden_size, config.intermediate_size],
        )?;
    }
    Ok(())
}

fn mark_layer_tensor(
    backend: BackendId,
    shards: &mut [InspectedShard],
    index: &TensorIndex,
    scalar_types: &mut ScalarTypeSet,
    layer: usize,
    suffix: &str,
    expected_shape: &[usize],
) -> Result<(), LoadError> {
    let name = layer_tensor_name(backend, layer, suffix)?;
    mark_required(
        backend,
        shards,
        index,
        name.as_str(),
        expected_shape,
        scalar_types,
    )
}

fn layer_tensor_name(backend: BackendId, layer: usize, suffix: &str) -> Result<String, LoadError> {
    const PREFIX: &str = "model.layers.";
    let decimal_digits = if layer == 0 {
        1
    } else {
        layer.ilog10() as usize + 1
    };
    let capacity = PREFIX
        .len()
        .checked_add(decimal_digits)
        .and_then(|length| length.checked_add(suffix.len()))
        .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
    let mut name = String::new();
    name.try_reserve_exact(capacity)
        .map_err(|_| host_memory_failure(backend, CODE_INSPECTION_ALLOCATION))?;
    write!(&mut name, "{PREFIX}{layer}{suffix}")
        .map_err(|_| invalid_model_failure(backend, CODE_REQUIRED_TENSOR))?;
    Ok(name)
}

fn mark_required(
    backend: BackendId,
    shards: &mut [InspectedShard],
    index: &TensorIndex,
    name: &str,
    expected_shape: &[usize],
    scalar_types: &mut ScalarTypeSet,
) -> Result<(), LoadError> {
    let location = find_location(shards, index, name)
        .ok_or_else(|| invalid_model_failure(backend, CODE_REQUIRED_TENSOR))?;
    let tensor = shards
        .get_mut(location.shard)
        .and_then(|shard| shard.tensors.get_mut(location.tensor))
        .ok_or_else(|| invalid_model_failure(backend, CODE_REQUIRED_TENSOR))?;
    if tensor.shape.as_slice() != expected_shape {
        return Err(invalid_model_failure(backend, CODE_REQUIRED_TENSOR));
    }
    if tensor.source_dtype.executable_dtype().is_none() {
        return Err(unsupported_scalar(backend));
    }
    tensor.required = true;
    scalar_types.insert(tensor.source_dtype.scalar_type());
    Ok(())
}

fn find_location(
    shards: &[InspectedShard],
    index: &TensorIndex,
    name: &str,
) -> Option<TensorLocation> {
    index
        .locations
        .binary_search_by(|location| tensor_name(shards, *location).unwrap_or_default().cmp(name))
        .ok()
        .and_then(|position| index.locations.get(position).copied())
}
