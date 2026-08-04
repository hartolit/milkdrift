//! Readiness, conversation, outcome, release, and cleanup invariants.

use std::fmt::Display;

use application_runtime::{
    ApplicationActivity, ApplicationRuntime, ConversationProvenance, ConversationRole,
    ConversationTokenEstimate, GenerationTerminal, GenerationTerminalKind,
    GenerationTerminalOutcome, LoadedModel, ResponseAttemptState,
};
use domain_contracts::{CancellationReason, FinishReason, GenerationUsage};

use crate::error::{BenchmarkError, BenchmarkResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GenerationExpectation {
    Chat { maximum_new_tokens: u32 },
    DirectTokenLimit { maximum_new_tokens: u32 },
    Cancellation { maximum_new_tokens: u32 },
}

impl GenerationExpectation {
    pub(super) const fn chat(maximum_new_tokens: u32) -> Self {
        Self::Chat { maximum_new_tokens }
    }

    pub(super) const fn direct_token_limit(maximum_new_tokens: u32) -> Self {
        Self::DirectTokenLimit { maximum_new_tokens }
    }

    pub(super) const fn cancellation(maximum_new_tokens: u32) -> Self {
        Self::Cancellation { maximum_new_tokens }
    }

    pub(super) const fn maximum_new_tokens(self) -> u32 {
        match self {
            Self::Chat { maximum_new_tokens }
            | Self::DirectTokenLimit { maximum_new_tokens }
            | Self::Cancellation { maximum_new_tokens } => maximum_new_tokens,
        }
    }

    pub(super) const fn requires_cancellation(self) -> bool {
        matches!(self, Self::Cancellation { .. })
    }
}

pub(super) fn validate_chat_ready(
    runtime: &ApplicationRuntime,
    loaded: &LoadedModel,
) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.loaded() != Some(loaded)
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
        || !runtime.can_submit_chat_message()
        || !runtime.conversation().is_empty()
        || runtime.context_diagnostics().is_some()
    {
        return Err(BenchmarkError::new(
            "compatible-chat proof did not begin from a connected, loaded, empty E1 state",
        ));
    }
    Ok(())
}

pub(super) fn validate_direct_ready(runtime: &ApplicationRuntime) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.loaded().is_none()
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
    {
        return Err(BenchmarkError::new(
            "direct completion did not begin from a connected, loaded, idle E1 state",
        ));
    }
    validate_direct_conversation_state(runtime)
}

pub(super) fn validate_direct_conversation_state(runtime: &ApplicationRuntime) -> BenchmarkResult {
    if !runtime.conversation().is_empty() || runtime.context_diagnostics().is_some() {
        return Err(BenchmarkError::new(
            "direct completion unexpectedly retained chat conversation state or diagnostics",
        ));
    }
    Ok(())
}

pub(super) fn validate_released_runtime(
    runtime: &ApplicationRuntime,
    terminal: &GenerationTerminal,
) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.loaded().is_none()
        || state.active_generation().is_some()
        || state.last_generation() != Some(terminal)
        || !state.hub_available()
        || !state.inference_available()
    {
        return Err(BenchmarkError::new(
            "public E1 state did not retain the matching released terminal generation cleanly",
        ));
    }
    Ok(())
}

pub(super) fn validate_chat_conversation(
    runtime: &ApplicationRuntime,
    loaded: &LoadedModel,
    finish_reason: FinishReason,
    usage: GenerationUsage,
    decoded_byte_count: u64,
    maximum_new_tokens: u32,
    expected_message: &str,
) -> BenchmarkResult {
    let records = runtime.conversation();
    if records.len() != 2 {
        return Err(BenchmarkError::new(format!(
            "compatible-chat proof retained {} conversation records instead of one user and one assistant record",
            records.len()
        )));
    }
    let user = records
        .first()
        .ok_or_else(|| BenchmarkError::new("compatible-chat user record disappeared"))?;
    let assistant = records
        .get(1)
        .ok_or_else(|| BenchmarkError::new("compatible-chat assistant record disappeared"))?;
    if user.role != ConversationRole::User
        || user.provenance != ConversationProvenance::User
        || user.content != expected_message
        || user.response_attempt.is_some()
        || assistant.role != ConversationRole::Assistant
        || assistant.provenance != ConversationProvenance::Model
        || assistant.content.is_empty()
        || u64::try_from(assistant.content.len()).ok() != Some(decoded_byte_count)
        || assistant.token_estimate
            != ConversationTokenEstimate::Generated(u32::try_from(usage.generated_tokens).map_err(
                |_| BenchmarkError::new("chat generated usage could not fit its token estimate"),
            )?)
        || !assistant.is_active_context()
    {
        return Err(BenchmarkError::new(
            "compatible-chat conversation did not retain the expected user turn and non-empty active model response",
        ));
    }
    let attempt = assistant.response_attempt.as_ref().ok_or_else(|| {
        BenchmarkError::new("compatible-chat assistant record had no response-attempt provenance")
    })?;
    if attempt.responding_to != user.id
        || attempt.superseded
        || attempt.state != ResponseAttemptState::Completed(finish_reason)
    {
        return Err(BenchmarkError::new(format!(
            "compatible-chat assistant attempt did not match the released terminal outcome: {attempt:?}"
        )));
    }

    let diagnostics = runtime.context_diagnostics().ok_or_else(|| {
        BenchmarkError::new("compatible-chat context diagnostics were not retained")
    })?;
    if diagnostics.actual_input_tokens == 0
        || u64::from(diagnostics.actual_input_tokens) != usage.prompt_tokens
        || diagnostics.reserved_output_tokens != maximum_new_tokens
        || diagnostics.maximum_context_tokens != loaded.maximum_context_tokens()
        || diagnostics
            .actual_input_tokens
            .checked_add(diagnostics.reserved_output_tokens)
            .is_none_or(|required| required > diagnostics.maximum_context_tokens)
    {
        return Err(BenchmarkError::new(format!(
            "compatible-chat context diagnostics were incomplete: {diagnostics:?}"
        )));
    }
    Ok(())
}

pub(super) fn clear_chat_conversation(runtime: &mut ApplicationRuntime) -> BenchmarkResult {
    runtime.clear_conversation().map_err(|error| {
        BenchmarkError::new(format!(
            "compatible-chat conversation could not be cleared after release: {error}"
        ))
    })?;
    if !runtime.conversation().is_empty() || runtime.context_diagnostics().is_some() {
        return Err(BenchmarkError::new(
            "compatible-chat conversation or diagnostics remained after public clear",
        ));
    }
    Ok(())
}

pub(super) fn validate_nonempty_generation(
    usage: GenerationUsage,
    decoded_byte_count: u64,
) -> BenchmarkResult {
    if usage.prompt_tokens == 0 || usage.generated_tokens == 0 || decoded_byte_count == 0 {
        return Err(BenchmarkError::new(format!(
            "generation did not publish non-zero prompt usage, generated usage, and decoded bytes: usage={usage:?}, decoded_bytes={decoded_byte_count}"
        )));
    }
    Ok(())
}

pub(super) fn validate_expected_outcome(
    expectation: GenerationExpectation,
    finish_reason: FinishReason,
    usage: GenerationUsage,
) -> BenchmarkResult {
    match expectation {
        GenerationExpectation::Chat { maximum_new_tokens } => match finish_reason {
            FinishReason::TokenLimit if usage.generated_tokens == u64::from(maximum_new_tokens) => {
                Ok(())
            }
            FinishReason::EndOfSequence(_)
                if usage.generated_tokens <= u64::from(maximum_new_tokens) =>
            {
                Ok(())
            }
            _ => Err(BenchmarkError::new(format!(
                "compatible-chat proof returned a finish reason or usage inconsistent with its {maximum_new_tokens}-token bound: reason={finish_reason:?}, usage={usage:?}"
            ))),
        },
        GenerationExpectation::DirectTokenLimit { maximum_new_tokens } => {
            if finish_reason == FinishReason::TokenLimit
                && usage.generated_tokens == u64::from(maximum_new_tokens)
            {
                Ok(())
            } else {
                Err(BenchmarkError::new(format!(
                    "controlled direct completion did not reach the exact {maximum_new_tokens}-token limit: reason={finish_reason:?}, usage={usage:?}"
                )))
            }
        }
        GenerationExpectation::Cancellation { maximum_new_tokens } => {
            if finish_reason == FinishReason::Cancelled(CancellationReason::UserRequested)
                && usage.generated_tokens > 0
                && usage.generated_tokens < u64::from(maximum_new_tokens)
            {
                Ok(())
            } else {
                Err(BenchmarkError::new(format!(
                    "progress-triggered cancellation did not finish as Cancelled(UserRequested) strictly before the {maximum_new_tokens}-token bound: reason={finish_reason:?}, usage={usage:?}"
                )))
            }
        }
    }
}

pub(super) fn terminal_kind_for_outcome(
    outcome: &GenerationTerminalOutcome,
) -> GenerationTerminalKind {
    match outcome {
        GenerationTerminalOutcome::Finished(reason) => GenerationTerminalKind::Finished(*reason),
        GenerationTerminalOutcome::Failed(_) => GenerationTerminalKind::Failed,
    }
}

pub(super) fn validate_terminal_consistency(
    terminal_output: Option<GenerationTerminalKind>,
    released_output: Option<GenerationTerminalKind>,
    event_kind: GenerationTerminalKind,
) -> BenchmarkResult {
    if terminal_output != Some(event_kind) || released_output != Some(event_kind) {
        return Err(BenchmarkError::new(format!(
            "terminal, released, and GenerationFinished outcomes did not match: terminal={terminal_output:?}, released={released_output:?}, event={event_kind:?}"
        )));
    }
    Ok(())
}

pub(super) fn output_cleanup_pending_error() -> BenchmarkError {
    BenchmarkError::new("generation output entered cleanup-pending state")
}

pub(super) fn output_cleanup_exhausted_error() -> BenchmarkError {
    BenchmarkError::new("generation output exhausted cleanup while retaining ownership")
}

pub(super) fn generation_cleanup_pending_error(
    exhausted: bool,
    failure: &impl Display,
) -> BenchmarkError {
    BenchmarkError::new(format!(
        "generation cleanup remained pending (exhausted={exhausted}): {failure}"
    ))
}

#[cfg(test)]
mod tests {
    use application_runtime::GenerationTerminalKind;
    use domain_contracts::{CancellationReason, FinishReason, GenerationUsage};

    use super::{GenerationExpectation, validate_expected_outcome, validate_terminal_consistency};

    #[test]
    fn terminal_validation_rejects_mismatched_terminal_and_released_outcomes() {
        let terminal = GenerationTerminalKind::Finished(FinishReason::TokenLimit);
        let released = GenerationTerminalKind::Finished(FinishReason::StopCondition);
        assert!(validate_terminal_consistency(Some(terminal), Some(released), terminal).is_err());
        assert!(validate_terminal_consistency(Some(terminal), Some(terminal), released).is_err());
    }

    #[test]
    fn terminal_validation_accepts_one_matching_terminal_release_and_event() -> Result<(), String> {
        let expected = GenerationTerminalKind::Finished(FinishReason::TokenLimit);
        validate_terminal_consistency(Some(expected), Some(expected), expected)
            .map_err(|error| error.to_string())
    }

    #[test]
    fn exact_direct_and_bounded_cancellation_outcomes_are_enforced() -> Result<(), String> {
        let direct_usage = GenerationUsage {
            prompt_tokens: 12,
            generated_tokens: 32,
        };
        validate_expected_outcome(
            GenerationExpectation::direct_token_limit(32),
            FinishReason::TokenLimit,
            direct_usage,
        )
        .map_err(|error| error.to_string())?;
        assert!(
            validate_expected_outcome(
                GenerationExpectation::direct_token_limit(32),
                FinishReason::TokenLimit,
                GenerationUsage {
                    generated_tokens: 31,
                    ..direct_usage
                },
            )
            .is_err()
        );

        let cancellation_usage = GenerationUsage {
            prompt_tokens: 12,
            generated_tokens: 7,
        };
        validate_expected_outcome(
            GenerationExpectation::cancellation(128),
            FinishReason::Cancelled(CancellationReason::UserRequested),
            cancellation_usage,
        )
        .map_err(|error| error.to_string())?;
        assert!(
            validate_expected_outcome(
                GenerationExpectation::cancellation(128),
                FinishReason::Cancelled(CancellationReason::UserRequested),
                GenerationUsage {
                    generated_tokens: 128,
                    ..cancellation_usage
                },
            )
            .is_err()
        );
        assert!(
            validate_expected_outcome(
                GenerationExpectation::cancellation(128),
                FinishReason::Cancelled(CancellationReason::ParentTask),
                cancellation_usage,
            )
            .is_err()
        );
        Ok(())
    }
}
