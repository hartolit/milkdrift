//! Explicit local chat compatibility, context planning, and E1 conversation operations.

use domain_contracts::{RequestId, TokenId};
use hf_tokenizer::HfTokenizer;

use crate::{
    ApplicationActivity, ApplicationError, ApplicationRuntime, ConversationRecord,
    ConversationRecordId, ConversationRetention, GenerationSettings,
};

mod preparation;

use self::preparation::{ChatPreparationRequest, estimate_content, prepare_chat};

const TINYLLAMA_CHAT_REPOSITORY: &str = "TinyLlama/TinyLlama-1.1B-Chat-v1.0";
const TINYLLAMA_CHAT_COMMIT: &str = "fe8a4ea1ffedaf415f4da2f062534de366a451e6";
const TINYLLAMA_SYSTEM_MARKER: &str = "<|system|>";
const TINYLLAMA_USER_MARKER: &str = "<|user|>";
const TINYLLAMA_ASSISTANT_MARKER: &str = "<|assistant|>";
const TINYLLAMA_END_OF_MESSAGE: &str = "</s>";
const TINYLLAMA_EOS_TOKEN: TokenId = TokenId::new(2);

/// Verified local prompt and termination profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PromptCompatibilityProfile {
    /// `TinyLlama` 1.1B Chat v1.0 role markers with `</s>` message/EOS termination.
    TinyLlamaChatV1,
}

/// Explicit result of matching a resolved model and tokenizer to a chat profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatCompatibility {
    /// Prompt formatting and assistant-turn termination are verified by one profile.
    Supported,
    /// No verified profile is available; direct completion remains supported.
    Unsupported,
}

/// Diagnostics from the most recent successfully submitted chat context plan.
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

impl PromptCompatibilityProfile {
    fn detect(
        repository: &str,
        commit: &str,
        tokenizer: &HfTokenizer,
    ) -> Option<PromptCompatibilityProfile> {
        if repository == TINYLLAMA_CHAT_REPOSITORY
            && commit == TINYLLAMA_CHAT_COMMIT
            && tokenizer.token_id(TINYLLAMA_END_OF_MESSAGE) == Some(TINYLLAMA_EOS_TOKEN)
        {
            Some(Self::TinyLlamaChatV1)
        } else {
            None
        }
    }

    fn apply_termination(self, settings: &mut GenerationSettings) {
        match self {
            Self::TinyLlamaChatV1 => {
                settings.apply_chat_termination(TINYLLAMA_EOS_TOKEN);
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
}

impl ApplicationRuntime {
    /// Returns the canonical raw in-memory conversation history.
    #[must_use]
    pub const fn conversation(&self) -> &[ConversationRecord] {
        self.conversation.records()
    }

    /// Returns diagnostics from the most recently submitted chat request.
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
                matches!(resolved.chat_compatibility(), ChatCompatibility::Supported)
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
    /// The user record remains committed if subsequent planning or command submission
    /// fails. No assistant attempt is created until the complete lower command is
    /// successfully enqueued; E0 reports semantic admission asynchronously afterward.
    ///
    /// # Errors
    ///
    /// Returns an error when chat compatibility, lifecycle state, content, context
    /// capacity, exact tokenization, or bounded E0 command submission prevents the request.
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
        let profile = self
            .state
            .resolved()
            .and_then(crate::ResolvedModel::prompt_compatibility_profile)
            .ok_or(ApplicationError::UnsupportedChatCompatibility)?;
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
        let prepared = prepare_chat(&ChatPreparationRequest {
            raw_records: self.conversation.records(),
            target_user: user,
            regenerating: regenerate,
            profile,
            tokenizer,
            maximum_context_tokens: loaded.maximum_context_tokens(),
            maximum_prefill_tokens: loaded.maximum_prefill_batch(),
            reserved_output_tokens: settings.maximum_new_tokens(),
        })?;
        self.start_chat_generation(
            prepared.prompt_tokens,
            settings,
            user,
            regenerate,
            prepared.diagnostics,
        )
    }
}

pub(crate) fn detect_chat_profile(
    repository: &str,
    commit: &str,
    tokenizer: &HfTokenizer,
) -> Option<PromptCompatibilityProfile> {
    PromptCompatibilityProfile::detect(repository, commit, tokenizer)
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
