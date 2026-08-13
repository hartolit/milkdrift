use std::io::Cursor;

use candle_transformers::models::llama::Config;
use domain_contracts::{BackendFailureKind, BackendId, LoadError, ScalarType};

use super::super::configuration_policy::validate_numeric_config;
use super::{
    MAX_CONFIG_BYTES, ScalarDeclaration, TEST_CONFIG_ALLOCATION_FAILURES, parse_bytes, parse_facts,
    read_bounded,
};
use crate::failure::{
    CODE_ARCHITECTURE, CODE_CONFIG_ALLOCATION, CODE_CONFIG_LIMIT, CODE_DECLARATION_CONFLICT,
    CODE_DECLARATION_MALFORMED, CODE_DECLARATION_UNSUPPORTED,
};

const BACKEND: BackendId = BackendId::new(9);

fn backend_code(error: LoadError) -> Option<(BackendFailureKind, u32)> {
    match error {
        LoadError::Backend(failure) => Some((failure.kind, failure.code)),
        _ => None,
    }
}

#[test]
fn declaration_status_matrix_is_exact() -> Result<(), String> {
    for (json, expected) in [
        (r#"{"model_type":"llama"}"#, None),
        (
            r#"{"model_type":"llama","dtype":null,"torch_dtype":null}"#,
            None,
        ),
        (
            r#"{"model_type":"llama","dtype":"float16"}"#,
            Some(ScalarType::F16),
        ),
        (
            r#"{"model_type":"llama","dtype":null,"torch_dtype":"bf16"}"#,
            Some(ScalarType::Bf16),
        ),
        (
            r#"{"model_type":"llama","dtype":"f32","torch_dtype":"float32"}"#,
            Some(ScalarType::F32),
        ),
    ] {
        let facts = parse_facts(json.as_bytes()).map_err(|error| error.to_string())?;
        assert_eq!(
            facts
                .resolve_declaration(BACKEND)
                .map_err(|error| format!("resolve declaration: {error:?}"))?,
            expected
        );
    }

    for json in [
        r#"{"model_type":"llama","dtype":"float8_e4m3fn"}"#,
        r#"{"model_type":"llama","dtype":"float16","torch_dtype":"unknown"}"#,
    ] {
        let error = parse_facts(json.as_bytes())
            .map_err(|error| error.to_string())?
            .resolve_declaration(BACKEND)
            .err()
            .ok_or_else(|| "unsupported declaration was accepted".to_owned())?;
        assert_eq!(
            backend_code(error),
            Some((
                BackendFailureKind::Unsupported,
                CODE_DECLARATION_UNSUPPORTED
            ))
        );
    }

    let conflict = parse_facts(
        r#"{"model_type":"llama","dtype":"float16","torch_dtype":"bfloat16"}"#.as_bytes(),
    )
    .map_err(|error| error.to_string())?
    .resolve_declaration(BACKEND)
    .err()
    .ok_or_else(|| "conflicting declarations were accepted".to_owned())?;
    assert_eq!(
        backend_code(conflict),
        Some((BackendFailureKind::Unsupported, CODE_DECLARATION_CONFLICT))
    );

    for json in [
        r#"{"model_type":"llama","dtype":16}"#,
        r#"{"model_type":"llama","dtype":{}}"#,
        r#"{"model_type":"llama","dtype":"f32","dtype":"f16"}"#,
    ] {
        let error = parse_facts(json.as_bytes())
            .map_err(|error| error.to_string())?
            .resolve_declaration(BACKEND)
            .err()
            .ok_or_else(|| "malformed declaration was accepted".to_owned())?;
        assert_eq!(
            backend_code(error),
            Some((BackendFailureKind::InvalidModel, CODE_DECLARATION_MALFORMED))
        );
    }
    Ok(())
}

#[test]
fn unsupported_modern_declaration_never_falls_back_to_recognized_legacy() -> Result<(), String> {
    let facts = parse_facts(
        r#"{"model_type":"llama","dtype":"float8_e4m3fn","torch_dtype":"float16"}"#.as_bytes(),
    )
    .map_err(|error| error.to_string())?;
    assert_eq!(facts.dtype, ScalarDeclaration::Unsupported);
    assert_eq!(
        facts.torch_dtype,
        ScalarDeclaration::Recognized(ScalarType::F16)
    );

    let error = facts
        .resolve_declaration(BACKEND)
        .err()
        .ok_or_else(|| "unsupported modern dtype fell back to legacy torch_dtype".to_owned())?;
    assert_eq!(
        backend_code(error),
        Some((
            BackendFailureKind::Unsupported,
            CODE_DECLARATION_UNSUPPORTED
        ))
    );
    Ok(())
}

#[test]
fn explicit_llama_identity_rejects_absence_and_contradictions() -> Result<(), String> {
    for json in [
        r"{}",
        r#"{"model_type":null}"#,
        r#"{"model_type":"mistral"}"#,
        r#"{"model_type":"llama","architectures":[]}"#,
        r#"{"model_type":"llama","architectures":["LlamaModel","MistralModel"]}"#,
        r#"{"model_type":"llama","architectures":"LlamaModel"}"#,
    ] {
        let error = parse_facts(json.as_bytes())
            .map_err(|error| error.to_string())?
            .validate_architecture(BACKEND)
            .err()
            .ok_or_else(|| "invalid architecture identity was accepted".to_owned())?;
        assert_eq!(
            backend_code(error),
            Some((BackendFailureKind::Unsupported, CODE_ARCHITECTURE))
        );
    }
    for json in [
        r#"{"model_type":"llama"}"#,
        r#"{"model_type":"LLAMA","architectures":null}"#,
        r#"{"model_type":"llama","architectures":["LlamaForCausalLM","LlamaModel"]}"#,
    ] {
        parse_facts(json.as_bytes())
            .map_err(|error| error.to_string())?
            .validate_architecture(BACKEND)
            .map_err(|error| format!("valid Llama identity: {error:?}"))?;
    }
    Ok(())
}

#[test]
fn numeric_config_rejects_locked_cache_hazards() {
    let mut config = numeric_fixture();
    config.num_key_value_heads = 2;
    config.num_attention_heads = 2;
    config.max_position_embeddings = 1;
    assert_eq!(
        validate_numeric_config(BACKEND, &config),
        Err(LoadError::InvalidSource)
    );

    let mut config = numeric_fixture();
    config.max_position_embeddings = usize::MAX;
    assert!(matches!(
        validate_numeric_config(BACKEND, &config),
        Err(LoadError::Backend(failure))
            if failure.kind == BackendFailureKind::InvalidModel
                && failure.code == crate::failure::CODE_NUMERIC_OVERFLOW
    ));
}

fn numeric_fixture() -> Config {
    Config {
        hidden_size: 8,
        intermediate_size: 16,
        vocab_size: 16,
        num_hidden_layers: 1,
        num_attention_heads: 2,
        num_key_value_heads: 2,
        use_flash_attn: false,
        rms_norm_eps: 1e-5,
        rope_theta: 10_000.0,
        bos_token_id: None,
        eos_token_id: None,
        rope_scaling: None,
        max_position_embeddings: 16,
        tie_word_embeddings: false,
    }
}

#[test]
fn exact_config_ceiling_and_allocation_failure_are_deterministic() -> Result<(), String> {
    let exact_length = usize::try_from(MAX_CONFIG_BYTES).map_err(|error| error.to_string())?;
    let retained_length = read_bounded(BACKEND, &mut Cursor::new(vec![b' '; exact_length]))
        .map_err(|error| format!("exact ceiling was rejected: {error:?}"))?
        .len();
    assert_eq!(retained_length, exact_length);

    let error = read_bounded(
        BACKEND,
        &mut Cursor::new(vec![b' '; exact_length.saturating_add(1)]),
    )
    .err()
    .ok_or_else(|| "ceiling plus one was accepted".to_owned())?;
    assert_eq!(
        backend_code(error),
        Some((BackendFailureKind::InvalidModel, CODE_CONFIG_LIMIT))
    );

    let oversized_owned = vec![b' '; exact_length.saturating_add(1)];
    let error = parse_bytes(BACKEND, oversized_owned.as_slice())
        .err()
        .ok_or_else(|| "oversized owned config bytes were accepted".to_owned())?;
    assert_eq!(
        backend_code(error),
        Some((BackendFailureKind::InvalidModel, CODE_CONFIG_LIMIT))
    );

    TEST_CONFIG_ALLOCATION_FAILURES.with(|remaining| remaining.set(1));
    let error = read_bounded(BACKEND, &mut Cursor::new([1_u8]))
        .err()
        .ok_or_else(|| "injected allocation failure was not observed".to_owned())?;
    assert_eq!(
        backend_code(error),
        Some((BackendFailureKind::HostMemory, CODE_CONFIG_ALLOCATION))
    );
    Ok(())
}
