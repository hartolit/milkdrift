use std::path::PathBuf;

use domain_contracts::FinishReason;
use hf_tokenizer::HfTokenizer;

use super::preparation::{ChatPreparationRequest, build_context_units, prepare_chat};
use super::{PromptCompatibilityProfile, TINYLLAMA_CHAT_COMMIT, TINYLLAMA_CHAT_REPOSITORY};
use crate::{
    ConversationProvenance, ConversationRecord, ConversationRecordId, ConversationRetention,
    ConversationRole, ConversationTokenEstimate, GenerationSettings, ResponseAttempt,
    ResponseAttemptId, ResponseAttemptState,
};

type TestResult = Result<(), String>;

#[test]
fn tinyllama_profile_formats_roles_and_owns_eos_compatibility() -> TestResult {
    let tokenizer = chat_tokenizer()?;
    assert_eq!(
        PromptCompatibilityProfile::detect(
            TINYLLAMA_CHAT_REPOSITORY,
            TINYLLAMA_CHAT_COMMIT,
            &tokenizer,
        ),
        Some(PromptCompatibilityProfile::TinyLlamaChatV1)
    );
    assert_eq!(
        PromptCompatibilityProfile::detect(
            TINYLLAMA_CHAT_REPOSITORY,
            "different-commit",
            &tokenizer,
        ),
        None
    );
    assert_eq!(
        PromptCompatibilityProfile::detect("unknown/model", TINYLLAMA_CHAT_COMMIT, &tokenizer,),
        None
    );

    let records = vec![
        record(1, ConversationRole::System, "be concise", true),
        record(2, ConversationRole::User, "hello", false),
    ];
    let units = build_context_units(records.as_slice(), ConversationRecordId::new(2), false)
        .map_err(|error| error.to_string())?;
    let rendered = PromptCompatibilityProfile::TinyLlamaChatV1
        .render(units.as_slice(), &[0, 1])
        .map_err(|error| error.to_string())?;
    assert_eq!(
        rendered,
        "<|system|>\nbe concise</s>\n<|user|>\nhello</s>\n<|assistant|>\n"
    );
    let mut settings = GenerationSettings::default();
    settings.eos_tokens.push(domain_contracts::TokenId::new(15));
    settings.stop_sequences.push("fallback".to_owned());
    PromptCompatibilityProfile::TinyLlamaChatV1.apply_termination(&mut settings);
    assert_eq!(settings.eos_tokens, [domain_contracts::TokenId::new(2)]);
    assert!(settings.stop_sequences.is_empty());

    let prepared = prepare_chat(&ChatPreparationRequest {
        raw_records: records.as_slice(),
        target_user: ConversationRecordId::new(2),
        regenerating: false,
        profile: PromptCompatibilityProfile::TinyLlamaChatV1,
        tokenizer: &tokenizer,
        maximum_context_tokens: 32,
        maximum_prefill_tokens: 32,
        reserved_output_tokens: 4,
    })
    .map_err(|error| error.to_string())?;

    assert!(!prepared.prompt_tokens.is_empty());
    assert_eq!(prepared.diagnostics.selected.len(), 2);
    assert_eq!(prepared.diagnostics.reserved_output_tokens, 4);
    Ok(())
}

#[test]
fn exact_token_correction_drops_completed_historical_turn_atomically() -> TestResult {
    let tokenizer = chat_tokenizer()?;
    let records = vec![
        record(
            1,
            ConversationRole::User,
            "old old old old old old old old old old old old",
            false,
        ),
        completed_assistant(2, 1, "answer answer"),
        record(3, ConversationRole::User, "hello", false),
    ];
    let prepared = prepare_chat(&ChatPreparationRequest {
        raw_records: records.as_slice(),
        target_user: ConversationRecordId::new(3),
        regenerating: false,
        profile: PromptCompatibilityProfile::TinyLlamaChatV1,
        tokenizer: &tokenizer,
        maximum_context_tokens: 16,
        maximum_prefill_tokens: 16,
        reserved_output_tokens: 2,
    })
    .map_err(|error| error.to_string())?;

    assert!(prepared.diagnostics.render_attempts > 1);
    assert_eq!(
        prepared.diagnostics.selected,
        [ConversationRecordId::new(3)]
    );
    assert!(
        prepared
            .diagnostics
            .dropped
            .contains(&ConversationRecordId::new(1))
    );
    assert!(
        prepared
            .diagnostics
            .dropped
            .contains(&ConversationRecordId::new(2))
    );
    assert!(
        !prepared
            .diagnostics
            .dropped
            .contains(&ConversationRecordId::new(3))
    );
    assert!(prepared.diagnostics.actual_input_tokens + 2 <= 16);
    Ok(())
}

#[test]
fn regeneration_inventory_excludes_the_targeted_prior_assistant() -> TestResult {
    let tokenizer = chat_tokenizer()?;
    let records = vec![
        record(1, ConversationRole::User, "hello", false),
        completed_assistant(2, 1, "previous answer"),
    ];
    let prepared = prepare_chat(&ChatPreparationRequest {
        raw_records: records.as_slice(),
        target_user: ConversationRecordId::new(1),
        regenerating: true,
        profile: PromptCompatibilityProfile::TinyLlamaChatV1,
        tokenizer: &tokenizer,
        maximum_context_tokens: 32,
        maximum_prefill_tokens: 32,
        reserved_output_tokens: 2,
    })
    .map_err(|error| error.to_string())?;

    assert_eq!(
        prepared.diagnostics.selected,
        [ConversationRecordId::new(1)]
    );
    assert!(
        !prepared
            .diagnostics
            .selected
            .contains(&ConversationRecordId::new(2))
    );
    assert!(
        !prepared
            .diagnostics
            .dropped
            .contains(&ConversationRecordId::new(2))
    );
    Ok(())
}

fn completed_assistant(id: u64, responding_to: u64, content: &str) -> ConversationRecord {
    let mut record = record(id, ConversationRole::Assistant, content, false);
    record.response_attempt = Some(ResponseAttempt {
        id: ResponseAttemptId::new(id),
        responding_to: ConversationRecordId::new(responding_to),
        state: ResponseAttemptState::Completed(FinishReason::TokenLimit),
        superseded: false,
    });
    record
}

fn record(id: u64, role: ConversationRole, content: &str, pinned: bool) -> ConversationRecord {
    ConversationRecord {
        id: ConversationRecordId::new(id),
        ordinal: id,
        role,
        content: content.to_owned(),
        provenance: match role {
            ConversationRole::System => ConversationProvenance::Application,
            ConversationRole::User => ConversationProvenance::User,
            ConversationRole::Assistant => ConversationProvenance::Model,
        },
        retention: if pinned {
            ConversationRetention::Pinned
        } else {
            ConversationRetention::Retained
        },
        token_estimate: ConversationTokenEstimate::Measured(1),
        response_attempt: None,
    }
}

fn chat_tokenizer() -> Result<HfTokenizer, String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chat-tokenizer.json");
    HfTokenizer::from_file(path).map_err(|error| error.to_string())
}
