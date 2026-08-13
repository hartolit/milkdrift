//! Source-locked numeric Llama configuration policy.

use candle_transformers::models::llama::Config;
use domain_contracts::{BackendId, LoadError};

use crate::failure::{CODE_CONFIG_LIMIT, CODE_NUMERIC_OVERFLOW};

use super::invalid_model_failure;

const MAX_HIDDEN_LAYERS: usize = 256;

pub(super) fn validate_numeric_config(
    backend: BackendId,
    config: &Config,
) -> Result<(), LoadError> {
    let required_non_zero = [
        config.hidden_size,
        config.intermediate_size,
        config.vocab_size,
        config.num_hidden_layers,
        config.num_attention_heads,
        config.num_key_value_heads,
        config.max_position_embeddings,
    ];
    if required_non_zero.contains(&0) {
        return Err(LoadError::InvalidSource);
    }
    if config.num_hidden_layers > MAX_HIDDEN_LAYERS {
        return Err(invalid_model_failure(backend, CODE_CONFIG_LIMIT));
    }

    let head_dimension = config.hidden_size / config.num_attention_heads;
    if !config
        .hidden_size
        .is_multiple_of(config.num_attention_heads)
        || !config
            .num_attention_heads
            .is_multiple_of(config.num_key_value_heads)
        // Candle 0.11.0 reads the KV-head dimension when deciding whether to
        // trim sequence length. Reject configurations that can enter that
        // invalid branch while this source-locked adapter is in use.
        || config.num_key_value_heads > config.max_position_embeddings
        || !head_dimension.is_multiple_of(2)
    {
        return Err(LoadError::InvalidSource);
    }
    config
        .max_position_embeddings
        .checked_mul(2)
        .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
    config
        .num_hidden_layers
        .checked_mul(9)
        .and_then(|count| count.checked_add(3))
        .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
    Ok(())
}
