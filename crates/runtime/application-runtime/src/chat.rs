//! Explicit local chat compatibility, context planning, and E1 conversation operations.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use context_planner::{
    ContextBudget, ContextContent, ContextEntry, ContextEntryId, ContextPersistence,
    ContextPriority, ContextRole, ContextSource, PlanWorkspace, PlanningError,
    exact_token_correction_candidate_index, plan,
};
use domain_contracts::{RequestId, TokenId};
use hf_tokenizer::HfTokenizer;
use tokenization::{SpecialTokenPolicy, TokenizationError, Tokenizer};

use crate::conversation::ConversationTokenEstimate;
use crate::generation::encode_text_with_policy;
use crate::{
    ApplicationActivity, ApplicationError, ApplicationFailure, ApplicationFailureKind,
    ApplicationRuntime, ConversationRecord, ConversationRecordId, ConversationRetention,
    ConversationRole, GenerationSettings,
};

const TINYLLAMA_CHAT_REPOSITORY: &str = "TinyLlama/TinyLlama-1.1B-Chat-v1.0";
const TINYLLAMA_CHAT_COMMIT: &str = "fe8a4ea1ffedaf415f4da2f062534de366a451e6";
const TINYLLAMA_SYSTEM_MARKER: &str = "<|system|>";
const TINYLLAMA_USER_MARKER: &str = "<|user|>";
const TINYLLAMA_ASSISTANT_MARKER: &str = "<|assistant|>";
const TINYLLAMA_END_OF_MESSAGE: &str = "</s>";
const TINYLLAMA_EOS_TOKEN: TokenId = TokenId::new(2);
const PINNED_PRIORITY: ContextPriority = ContextPriority::new(u8::MAX);
const HISTORICAL_PRIORITY: ContextPriority = ContextPriority::new(128);

/// Verified local prompt and termination profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptCompatibilityProfile {
    /// `TinyLlama` 1.1B Chat v1.0 role markers with `</s>` message/EOS termination.
    TinyLlamaChatV1,
}

/// Explicit result of matching a resolved model and tokenizer to a chat profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatCompatibility {
    /// Prompt formatting and assistant-turn termination are verified by one profile.
    Supported(PromptCompatibilityProfile),
    /// No verified profile is available; direct completion remains supported.
    Unsupported,
}

/// Diagnostics from the most recent successfully admitted chat context plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextDiagnostics {
    /// Raw conversation records selected for rendering, in conversation order.
    pub selected: Vec<ConversationRecordId>,
    /// Eligible records omitted by estimate planning or exact-token correction.
    pub dropped: Vec<ConversationRecordId>,
    /// Sum of semantic-content estimates selected by the planner.
    pub estimated_input_tokens: u32,
    /// Exact tokenized rendered-prompt size submitted to E0.
    pub actual_input_tokens: u32,
    /// Output positions reserved before context selection.
    pub reserved_output_tokens: u32,
    /// Loaded model context capacity.
    pub maximum_context_tokens: u32,
    /// Number of render/tokenize attempts used by bounded correction.
    pub render_attempts: u32,
}

struct PreparedChat {
    prompt_tokens: Box<[TokenId]>,
    diagnostics: ContextDiagnostics,
}

/// One atomic context-planning unit. A completed historical user/assistant turn is
/// selected or dropped together while raw conversation records remain independent.
struct ContextPlanningUnit<'a> {
    primary: &'a ConversationRecord,
    paired_assistant: Option<&'a ConversationRecord>,
    planning_content: Cow<'a, str>,
    persistence: ContextPersistence,
    estimated_tokens: u32,
}

impl<'a> ContextPlanningUnit<'a> {
    fn new(
        primary: &'a ConversationRecord,
        paired_assistant: Option<&'a ConversationRecord>,
        target_user: ConversationRecordId,
    ) -> Self {
        let planning_content = paired_assistant.map_or_else(
            || Cow::Borrowed(primary.content.as_str()),
            |assistant| {
                let capacity = primary
                    .content
                    .len()
                    .saturating_add(assistant.content.len())
                    .saturating_add(1);
                let mut content = String::with_capacity(capacity);
                content.push_str(primary.content.as_str());
                content.push('\n');
                content.push_str(assistant.content.as_str());
                Cow::Owned(content)
            },
        );
        let persistence = unit_persistence(primary, paired_assistant, target_user);
        let estimated_tokens = primary.token_estimate.tokens().saturating_add(
            paired_assistant.map_or(0, |assistant| assistant.token_estimate.tokens()),
        );
        Self {
            primary,
            paired_assistant,
            planning_content,
            persistence,
            estimated_tokens,
        }
    }

    fn append_record_ids(&self, output: &mut Vec<ConversationRecordId>) {
        output.push(self.primary.id);
        if let Some(assistant) = self.paired_assistant {
            output.push(assistant.id);
        }
    }
}

impl PromptCompatibilityProfile {
    fn detect(repository: &str, commit: &str, tokenizer: &HfTokenizer) -> ChatCompatibility {
        if repository == TINYLLAMA_CHAT_REPOSITORY
            && commit == TINYLLAMA_CHAT_COMMIT
            && tokenizer.token_id(TINYLLAMA_END_OF_MESSAGE) == Some(TINYLLAMA_EOS_TOKEN)
        {
            ChatCompatibility::Supported(Self::TinyLlamaChatV1)
        } else {
            ChatCompatibility::Unsupported
        }
    }

    fn apply_termination(self, settings: &mut GenerationSettings) {
        match self {
            Self::TinyLlamaChatV1 => {
                settings.eos_tokens.clear();
                settings.eos_tokens.push(TINYLLAMA_EOS_TOKEN);
                settings.stop_sequences.clear();
            }
        }
    }

    fn validate_content(self, content: &str) -> Result<(), ApplicationError> {
        if content.trim().is_empty() {
            return Err(ApplicationError::EmptyConversationMessage);
        }
        let reserved = match self {
            Self::TinyLlamaChatV1 => [
                TINYLLAMA_SYSTEM_MARKER,
                TINYLLAMA_USER_MARKER,
                TINYLLAMA_ASSISTANT_MARKER,
                TINYLLAMA_END_OF_MESSAGE,
            ],
        };
        if reserved.iter().any(|marker| content.contains(marker)) {
            return Err(ApplicationError::ReservedChatMarker);
        }
        Ok(())
    }

    fn render(
        self,
        units: &[ContextPlanningUnit<'_>],
        selected_indices: &[usize],
    ) -> Result<String, ApplicationError> {
        let mut prompt = String::new();
        for &index in selected_indices {
            let unit = units.get(index).ok_or_else(|| {
                ApplicationFailure::new(
                    ApplicationFailureKind::Worker,
                    "context plan referenced an unavailable planning unit",
                )
            })?;
            self.render_record(&mut prompt, unit.primary)?;
            if let Some(assistant) = unit.paired_assistant {
                self.render_record(&mut prompt, assistant)?;
            }
        }
        writeln!(prompt, "{TINYLLAMA_ASSISTANT_MARKER}").map_err(render_failure)?;
        Ok(prompt)
    }

    fn render_record(
        self,
        prompt: &mut String,
        record: &ConversationRecord,
    ) -> Result<(), ApplicationError> {
        let marker = match (self, record.role) {
            (Self::TinyLlamaChatV1, ConversationRole::System) => TINYLLAMA_SYSTEM_MARKER,
            (Self::TinyLlamaChatV1, ConversationRole::User) => TINYLLAMA_USER_MARKER,
            (Self::TinyLlamaChatV1, ConversationRole::Assistant) => TINYLLAMA_ASSISTANT_MARKER,
        };
        writeln!(prompt, "{marker}").map_err(render_failure)?;
        prompt.push_str(record.content.as_str());
        prompt.push_str(TINYLLAMA_END_OF_MESSAGE);
        prompt.push('\n');
        Ok(())
    }
}

impl ApplicationRuntime {
    /// Returns the canonical raw in-memory conversation history.
    #[must_use]
    pub const fn conversation(&self) -> &[ConversationRecord] {
        self.conversation.records()
    }

    /// Returns diagnostics from the most recently admitted chat request.
    #[must_use]
    pub const fn context_diagnostics(&self) -> Option<&ContextDiagnostics> {
        self.context_diagnostics.as_ref()
    }

    /// Returns whether the current loaded model and lifecycle state can accept chat input.
    #[must_use]
    pub fn can_submit_chat_message(&self) -> bool {
        self.state.can_start_generation()
            && !self.conversation.has_active_response()
            && self.tokenizer.is_some()
            && self.state.resolved().is_some_and(|resolved| {
                matches!(
                    resolved.chat_compatibility(),
                    ChatCompatibility::Supported(_)
                )
            })
    }

    /// Returns whether the latest response attempt may be regenerated now.
    #[must_use]
    pub fn can_regenerate_response(&self) -> bool {
        self.can_submit_chat_message() && self.conversation.last_regenerable_user().is_some()
    }

    /// Adds one pinned system instruction to an empty, inactive conversation.
    ///
    /// # Errors
    ///
    /// Returns an error when chat compatibility is unknown, the conversation is not
    /// empty, content is invalid, or tokenizer/model state is unavailable.
    pub fn set_system_instruction(
        &mut self,
        content: &str,
    ) -> Result<ConversationRecordId, ApplicationError> {
        if !self.conversation.records().is_empty() {
            return Err(ApplicationError::SystemInstructionRequiresEmptyConversation);
        }
        let (profile, tokenizer, maximum_context_tokens) = self.chat_prerequisites()?;
        profile.validate_content(content)?;
        let estimate = estimate_content(tokenizer, content, maximum_context_tokens);
        self.conversation
            .commit_system(content.to_owned(), estimate)
    }

    /// Commits one user message and starts a compatible assistant response attempt.
    ///
    /// The user record remains committed if subsequent planning or request admission
    /// fails. No assistant attempt is created until E0 accepts the complete command.
    ///
    /// # Errors
    ///
    /// Returns an error when chat compatibility, lifecycle state, content, context
    /// capacity, exact tokenization, or bounded E0 admission prevents the request.
    pub fn submit_user_message(
        &mut self,
        content: &str,
        settings: GenerationSettings,
    ) -> Result<RequestId, ApplicationError> {
        let (profile, tokenizer, maximum_context_tokens) = self.chat_prerequisites()?;
        profile.validate_content(content)?;
        let estimate = estimate_content(tokenizer, content, maximum_context_tokens);
        let user = self.conversation.commit_user(
            content.to_owned(),
            ConversationRetention::Retained,
            estimate,
        )?;
        self.start_chat_response(user, profile, settings, false)
    }

    /// Starts a replacement attempt for the latest user turn while preserving raw provenance.
    ///
    /// # Errors
    ///
    /// Returns an error when no prior attempt is regenerable or ordinary chat admission fails.
    pub fn regenerate_last_response(
        &mut self,
        settings: GenerationSettings,
    ) -> Result<RequestId, ApplicationError> {
        let user = self
            .conversation
            .last_regenerable_user()
            .ok_or(ApplicationError::NoRegenerableResponse)?;
        let (profile, _, _) = self.chat_prerequisites()?;
        self.start_chat_response(user, profile, settings, true)
    }

    /// Clears raw conversation history after every response attempt is terminal.
    ///
    /// # Errors
    ///
    /// Returns an error while generation or its backend cleanup lifecycle remains active.
    /// Callers cancel when needed and wait for E0 release before clearing semantic history.
    pub fn clear_conversation(&mut self) -> Result<(), ApplicationError> {
        if let Some(active) = self.state.active_generation() {
            return Err(ApplicationError::GenerationAlreadyActive(active.request_id));
        }
        self.conversation.clear();
        self.context_diagnostics = None;
        Ok(())
    }

    fn chat_prerequisites(
        &self,
    ) -> Result<(PromptCompatibilityProfile, &HfTokenizer, u32), ApplicationError> {
        if let Some(active) = self.state.active_generation() {
            return Err(ApplicationError::GenerationAlreadyActive(active.request_id));
        }
        if self.state.activity() != ApplicationActivity::Idle {
            return Err(ApplicationError::Busy(self.state.activity()));
        }
        if !self.state.inference_available() {
            return Err(ApplicationError::RuntimeDisconnected);
        }
        let loaded = self.state.loaded().ok_or(ApplicationError::NoLoadedModel)?;
        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or(ApplicationError::NoTokenizer)?;
        let compatibility = self
            .state
            .resolved()
            .map_or(ChatCompatibility::Unsupported, |resolved| {
                resolved.chat_compatibility()
            });
        let ChatCompatibility::Supported(profile) = compatibility else {
            return Err(ApplicationError::UnsupportedChatCompatibility);
        };
        Ok((profile, tokenizer, loaded.maximum_context_tokens()))
    }

    fn start_chat_response(
        &mut self,
        user: ConversationRecordId,
        profile: PromptCompatibilityProfile,
        mut settings: GenerationSettings,
        regenerate: bool,
    ) -> Result<RequestId, ApplicationError> {
        profile.apply_termination(&mut settings);
        let loaded = self.state.loaded().ok_or(ApplicationError::NoLoadedModel)?;
        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or(ApplicationError::NoTokenizer)?;
        let prepared = prepare_chat(
            self.conversation.records(),
            user,
            regenerate,
            profile,
            tokenizer,
            loaded.maximum_context_tokens(),
            loaded.maximum_prefill_batch(),
            settings.maximum_new_tokens,
        )?;
        self.conversation.ensure_response_capacity()?;
        let request_id = self.start_generation_tokens(prepared.prompt_tokens, settings)?;
        if let Err(error) = self
            .conversation
            .begin_response(request_id, user, regenerate)
        {
            let _cancellation = self.cancel_generation(request_id);
            return Err(error);
        }
        self.context_diagnostics = Some(prepared.diagnostics);
        Ok(request_id)
    }
}

pub fn detect_chat_compatibility(
    repository: &str,
    commit: &str,
    tokenizer: &HfTokenizer,
) -> ChatCompatibility {
    PromptCompatibilityProfile::detect(repository, commit, tokenizer)
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "chat preparation keeps bounded selection, correction, rendering, and exact admission visibly contiguous"
)]
fn prepare_chat<T: Tokenizer + ?Sized>(
    raw_records: &[ConversationRecord],
    target_user: ConversationRecordId,
    regenerating: bool,
    profile: PromptCompatibilityProfile,
    tokenizer: &T,
    maximum_context_tokens: u32,
    maximum_prefill_tokens: u32,
    reserved_output_tokens: u32,
) -> Result<PreparedChat, ApplicationError> {
    let effective_input_tokens = maximum_context_tokens
        .checked_sub(reserved_output_tokens)
        .ok_or_else(|| ApplicationError::ContextCapacityExceeded {
            required: u64::from(reserved_output_tokens),
            available: u64::from(maximum_context_tokens),
        })?
        .min(maximum_prefill_tokens);
    let planning_maximum = effective_input_tokens
        .checked_add(reserved_output_tokens)
        .ok_or_else(|| ApplicationError::ContextCapacityExceeded {
            required: u64::from(effective_input_tokens)
                .saturating_add(u64::from(reserved_output_tokens)),
            available: u64::from(maximum_context_tokens),
        })?;
    let budget = ContextBudget::new(planning_maximum, reserved_output_tokens).map_err(|_| {
        ApplicationError::ContextCapacityExceeded {
            required: u64::from(reserved_output_tokens),
            available: u64::from(maximum_context_tokens),
        }
    })?;

    let units = build_context_units(raw_records, target_user, regenerating)?;
    let entries: Vec<ContextEntry<'_>> = units.iter().map(context_entry).collect();
    let mut ordering = vec![0_usize; entries.len()];
    let mut selected_workspace = vec![0_usize; entries.len()];
    let mut dropped_workspace = vec![0_usize; entries.len()];
    let planned = plan(
        entries.as_slice(),
        budget,
        PlanWorkspace {
            ordering: ordering.as_mut_slice(),
            selected: selected_workspace.as_mut_slice(),
            dropped: dropped_workspace.as_mut_slice(),
        },
    )
    .map_err(planning_failure)?;
    let mut selected = planned.selected_indices().to_vec();
    let mut dropped = expand_unit_record_ids(units.as_slice(), planned.dropped_indices());
    let maximum_attempts = selected
        .iter()
        .filter(|index| {
            entries
                .get(**index)
                .is_some_and(|entry| entry.persistence != ContextPersistence::Pinned)
        })
        .count()
        .saturating_add(1);

    for attempt in 1..=maximum_attempts {
        let rendered = profile.render(units.as_slice(), selected.as_slice())?;
        match encode_text_with_policy(
            tokenizer,
            rendered.as_str(),
            effective_input_tokens,
            SpecialTokenPolicy::Allow,
        ) {
            Ok(prompt_tokens) => {
                let actual_input_tokens = u32::try_from(prompt_tokens.len()).map_err(|_| {
                    ApplicationError::ContextCapacityExceeded {
                        required: u64::try_from(prompt_tokens.len()).unwrap_or(u64::MAX),
                        available: u64::from(effective_input_tokens),
                    }
                })?;
                return Ok(PreparedChat {
                    prompt_tokens,
                    diagnostics: ContextDiagnostics {
                        selected: expand_unit_record_ids(units.as_slice(), selected.as_slice()),
                        dropped,
                        estimated_input_tokens: selected_estimated_tokens(
                            entries.as_slice(),
                            selected.as_slice(),
                        ),
                        actual_input_tokens,
                        reserved_output_tokens,
                        maximum_context_tokens,
                        render_attempts: u32::try_from(attempt).unwrap_or(u32::MAX),
                    },
                });
            }
            Err(TokenizationError::CapacityExhausted(_)) => {}
            Err(error) => {
                return Err(ApplicationFailure::from_debug(
                    ApplicationFailureKind::Tokenizer,
                    "chat prompt encoding failed",
                    error,
                )
                .into());
            }
        }

        let Some(candidate) =
            exact_token_correction_candidate_index(entries.as_slice(), selected.as_slice())
        else {
            return Err(ApplicationError::PinnedBudgetExceeded {
                required_at_least: u64::from(effective_input_tokens).saturating_add(1),
                available: u64::from(effective_input_tokens),
            });
        };
        let previous_length = selected.len();
        selected.retain(|index| *index != candidate);
        if selected.len() >= previous_length {
            return Err(ApplicationError::UnchangedContextCorrection);
        }
        let unit = units.get(candidate).ok_or_else(|| {
            ApplicationFailure::new(
                ApplicationFailureKind::Worker,
                "context correction referenced an unavailable planning unit",
            )
        })?;
        unit.append_record_ids(&mut dropped);
    }

    Err(ApplicationError::UnchangedContextCorrection)
}

fn build_context_units(
    raw_records: &[ConversationRecord],
    target_user: ConversationRecordId,
    regenerating: bool,
) -> Result<Vec<ContextPlanningUnit<'_>>, ApplicationError> {
    let mut active_assistants = BTreeMap::new();
    for record in raw_records {
        if record.role != ConversationRole::Assistant || !record.is_active_context() {
            continue;
        }
        let Some(attempt) = record.response_attempt.as_ref() else {
            continue;
        };
        if regenerating && attempt.responding_to == target_user {
            continue;
        }
        if active_assistants
            .insert(attempt.responding_to, record)
            .is_some()
        {
            return Err(ApplicationFailure::new(
                ApplicationFailureKind::Worker,
                "multiple active assistant responses reference one user turn",
            )
            .into());
        }
    }

    let mut units = Vec::with_capacity(raw_records.len().saturating_sub(active_assistants.len()));
    for record in raw_records {
        match record.role {
            ConversationRole::Assistant => {}
            ConversationRole::System => {
                units.push(ContextPlanningUnit::new(record, None, target_user));
            }
            ConversationRole::User => {
                let assistant = active_assistants.remove(&record.id);
                units.push(ContextPlanningUnit::new(record, assistant, target_user));
            }
        }
    }
    if let Some((responding_to, _)) = active_assistants.first_key_value() {
        return Err(ApplicationFailure::new(
            ApplicationFailureKind::Worker,
            format!(
                "active assistant response references missing user record {}",
                responding_to.get()
            ),
        )
        .into());
    }
    Ok(units)
}

fn unit_persistence(
    primary: &ConversationRecord,
    paired_assistant: Option<&ConversationRecord>,
    target_user: ConversationRecordId,
) -> ContextPersistence {
    if primary.id == target_user
        || primary.retention == ConversationRetention::Pinned
        || paired_assistant
            .is_some_and(|assistant| assistant.retention == ConversationRetention::Pinned)
    {
        ContextPersistence::Pinned
    } else if primary.retention == ConversationRetention::Retained
        || paired_assistant
            .is_some_and(|assistant| assistant.retention == ConversationRetention::Retained)
    {
        ContextPersistence::Retained
    } else {
        ContextPersistence::Ephemeral
    }
}

fn context_entry<'unit>(unit: &'unit ContextPlanningUnit<'_>) -> ContextEntry<'unit> {
    let role = match unit.primary.role {
        ConversationRole::System => ContextRole::System,
        ConversationRole::User => ContextRole::User,
        ConversationRole::Assistant => ContextRole::Assistant,
    };
    let source = match unit.primary.role {
        ConversationRole::System => ContextSource::Application,
        ConversationRole::User => ContextSource::User,
        ConversationRole::Assistant => ContextSource::Model,
    };
    ContextEntry {
        id: ContextEntryId::new(unit.primary.id.get()),
        ordinal: unit.primary.ordinal,
        role,
        source,
        priority: if unit.persistence == ContextPersistence::Pinned {
            PINNED_PRIORITY
        } else {
            HISTORICAL_PRIORITY
        },
        persistence: unit.persistence,
        estimated_tokens: unit.estimated_tokens,
        content: ContextContent::Text(unit.planning_content.as_ref()),
    }
}

fn expand_unit_record_ids(
    units: &[ContextPlanningUnit<'_>],
    indices: &[usize],
) -> Vec<ConversationRecordId> {
    let mut record_ids = Vec::new();
    for &index in indices {
        if let Some(unit) = units.get(index) {
            unit.append_record_ids(&mut record_ids);
        }
    }
    record_ids
}

fn selected_estimated_tokens(entries: &[ContextEntry<'_>], selected: &[usize]) -> u32 {
    selected.iter().fold(0_u32, |total, index| {
        total.saturating_add(
            entries
                .get(*index)
                .map_or(0, |entry| entry.estimated_tokens),
        )
    })
}

fn estimate_content<T: Tokenizer + ?Sized>(
    tokenizer: &T,
    content: &str,
    maximum_context_tokens: u32,
) -> ConversationTokenEstimate {
    encode_text_with_policy(
        tokenizer,
        content,
        maximum_context_tokens,
        SpecialTokenPolicy::OrdinaryText,
    )
    .map_or_else(
        |_| ConversationTokenEstimate::Conservative(maximum_context_tokens.saturating_add(1)),
        |tokens| {
            ConversationTokenEstimate::Measured(u32::try_from(tokens.len()).unwrap_or(u32::MAX))
        },
    )
}

fn planning_failure(error: PlanningError) -> ApplicationError {
    match error {
        PlanningError::PinnedBudgetExceeded {
            required_tokens,
            available_tokens,
        } => ApplicationError::PinnedBudgetExceeded {
            required_at_least: required_tokens,
            available: available_tokens,
        },
        other => ApplicationFailure::from_debug(
            ApplicationFailureKind::Worker,
            "context planning failed",
            other,
        )
        .into(),
    }
}

fn render_failure(error: std::fmt::Error) -> ApplicationError {
    ApplicationFailure::new(ApplicationFailureKind::Worker, error).into()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use domain_contracts::FinishReason;
    use hf_tokenizer::HfTokenizer;

    use super::{
        ChatCompatibility, PromptCompatibilityProfile, TINYLLAMA_CHAT_COMMIT,
        TINYLLAMA_CHAT_REPOSITORY, build_context_units, detect_chat_compatibility, prepare_chat,
    };

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
            detect_chat_compatibility(TINYLLAMA_CHAT_REPOSITORY, TINYLLAMA_CHAT_COMMIT, &tokenizer),
            ChatCompatibility::Supported(PromptCompatibilityProfile::TinyLlamaChatV1)
        );
        assert_eq!(
            detect_chat_compatibility(TINYLLAMA_CHAT_REPOSITORY, "different-commit", &tokenizer),
            ChatCompatibility::Unsupported
        );
        assert_eq!(
            detect_chat_compatibility("unknown/model", TINYLLAMA_CHAT_COMMIT, &tokenizer),
            ChatCompatibility::Unsupported
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

        let prepared = prepare_chat(
            records.as_slice(),
            ConversationRecordId::new(2),
            false,
            PromptCompatibilityProfile::TinyLlamaChatV1,
            &tokenizer,
            32,
            32,
            4,
        )
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
        let prepared = prepare_chat(
            records.as_slice(),
            ConversationRecordId::new(3),
            false,
            PromptCompatibilityProfile::TinyLlamaChatV1,
            &tokenizer,
            16,
            16,
            2,
        )
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
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chat-tokenizer.json");
        HfTokenizer::from_file(path).map_err(|error| error.to_string())
    }
}
