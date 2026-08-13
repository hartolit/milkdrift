//! Frontend-neutral in-memory conversation records and response-attempt provenance.

use domain_contracts::{CancellationReason, FinishReason, RequestId};

use crate::{
    ApplicationError, ApplicationFailure, ApplicationFailureKind, GenerationTerminalOutcome,
};

/// Stable identity of one raw conversation record.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConversationRecordId(u64);

impl ConversationRecordId {
    /// Creates a stable conversation-record identity.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the stable numeric identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Stable identity of one assistant response attempt.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResponseAttemptId(u64);

impl ResponseAttemptId {
    /// Creates a stable response-attempt identity.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the stable numeric identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Frontend-neutral semantic role stored before model-specific rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationRole {
    /// Application or user supplied system instruction.
    System,
    /// Committed user input.
    User,
    /// Model-produced response attempt.
    Assistant,
}

/// Origin of semantic conversation content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationProvenance {
    /// Application-defined instruction.
    Application,
    /// Direct user input.
    User,
    /// Generated model output.
    Model,
}

/// Context-retention policy stored independently from one request-local plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationRetention {
    /// Content must be retained whenever it is eligible for the active context view.
    Pinned,
    /// Content competes normally for context capacity.
    Retained,
    /// Content is removed before retained content at equal priority.
    Ephemeral,
}

/// Token estimate retained with one semantic record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationTokenEstimate {
    /// Token count measured by encoding the unwrapped semantic content.
    Measured(u32),
    /// Native generated-token usage for assistant text before any later re-tokenization.
    Generated(u32),
    /// Conservative count used when content measurement could not complete.
    Conservative(u32),
}

impl ConversationTokenEstimate {
    pub(crate) const fn tokens(self) -> u32 {
        match self {
            Self::Measured(tokens) | Self::Generated(tokens) | Self::Conservative(tokens) => tokens,
        }
    }
}

/// Terminal or active state of one assistant response attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseAttemptState {
    /// The request is active and text may continue to stream.
    Streaming,
    /// Generation completed successfully.
    Completed(FinishReason),
    /// Generation ended because cancellation reached a safe execution boundary.
    Cancelled(CancellationReason),
    /// Generation failed; partial text remains inspectable.
    Failed(ApplicationFailure),
}

/// Assistant-attempt metadata attached only to assistant records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseAttempt {
    /// Stable attempt identity.
    pub id: ResponseAttemptId,
    /// User record to which this attempt responds.
    pub responding_to: ConversationRecordId,
    /// Current or terminal attempt state.
    pub state: ResponseAttemptState,
    /// Whether a later regeneration replaced this attempt in the active-context view.
    pub superseded: bool,
}

/// One canonical raw conversation record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationRecord {
    /// Stable record identity.
    pub id: ConversationRecordId,
    /// Monotonic raw-history order.
    pub ordinal: u64,
    /// Semantic role.
    pub role: ConversationRole,
    /// UTF-8 semantic content without model wrappers.
    pub content: String,
    /// Content origin.
    pub provenance: ConversationProvenance,
    /// Stored context-retention policy.
    pub retention: ConversationRetention,
    /// Measured or conservative semantic-content token estimate.
    pub token_estimate: ConversationTokenEstimate,
    /// Assistant response metadata, absent for system and user records.
    pub response_attempt: Option<ResponseAttempt>,
}

impl ConversationRecord {
    /// Returns whether this record is eligible for ordinary future context.
    #[must_use]
    pub const fn is_active_context(&self) -> bool {
        match &self.response_attempt {
            None => true,
            Some(attempt) => {
                !attempt.superseded && matches!(attempt.state, ResponseAttemptState::Completed(_))
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ActiveResponse {
    request_id: RequestId,
    record_id: ConversationRecordId,
}

/// Preallocated, identity-checked semantic commit for one submitted generation request.
///
/// Preparation may reserve storage but does not publish an assistant record or supersede
/// history. Once the corresponding E0 command is submitted, applying this value is infallible.
pub(crate) struct ConversationResponseCommit {
    request_id: RequestId,
    responding_to: ConversationRecordId,
    supersede_previous: bool,
    record_id: ConversationRecordId,
    next_record_id: u64,
    attempt_id: ResponseAttemptId,
    next_attempt_id: u64,
}

pub struct ConversationState {
    records: Vec<ConversationRecord>,
    active_response: Option<ActiveResponse>,
    next_record_id: u64,
    next_attempt_id: u64,
}

impl Default for ConversationState {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            active_response: None,
            next_record_id: 1,
            next_attempt_id: 1,
        }
    }
}

impl ConversationState {
    pub(super) const fn records(&self) -> &[ConversationRecord] {
        self.records.as_slice()
    }

    pub(crate) const fn has_active_response(&self) -> bool {
        self.active_response.is_some()
    }

    pub(crate) fn commit_user(
        &mut self,
        content: String,
        retention: ConversationRetention,
        token_estimate: ConversationTokenEstimate,
    ) -> Result<ConversationRecordId, ApplicationError> {
        let id = self.next_record_id()?;
        self.records.push(ConversationRecord {
            id,
            ordinal: id.get(),
            role: ConversationRole::User,
            content,
            provenance: ConversationProvenance::User,
            retention,
            token_estimate,
            response_attempt: None,
        });
        Ok(id)
    }

    pub(crate) fn commit_system(
        &mut self,
        content: String,
        token_estimate: ConversationTokenEstimate,
    ) -> Result<ConversationRecordId, ApplicationError> {
        let id = self.next_record_id()?;
        self.records.push(ConversationRecord {
            id,
            ordinal: id.get(),
            role: ConversationRole::System,
            content,
            provenance: ConversationProvenance::Application,
            retention: ConversationRetention::Pinned,
            token_estimate,
            response_attempt: None,
        });
        Ok(id)
    }

    pub(crate) fn prepare_response(
        &mut self,
        request_id: RequestId,
        responding_to: ConversationRecordId,
        supersede_previous: bool,
    ) -> Result<ConversationResponseCommit, ApplicationError> {
        if self.active_response.is_some()
            || !self
                .records
                .iter()
                .any(|record| record.id == responding_to && record.role == ConversationRole::User)
        {
            return Err(ApplicationFailure::new(
                ApplicationFailureKind::Worker,
                "conversation response preparation found inconsistent target state",
            )
            .into());
        }
        self.records
            .try_reserve(1)
            .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::Worker, error))?;
        let next_record_id = self
            .next_record_id
            .checked_add(1)
            .ok_or(ApplicationError::ConversationIdentityExhausted)?;
        let next_attempt_id = self
            .next_attempt_id
            .checked_add(1)
            .ok_or(ApplicationError::ConversationIdentityExhausted)?;
        Ok(ConversationResponseCommit {
            request_id,
            responding_to,
            supersede_previous,
            record_id: ConversationRecordId::new(self.next_record_id),
            next_record_id,
            attempt_id: ResponseAttemptId::new(self.next_attempt_id),
            next_attempt_id,
        })
    }

    pub(crate) fn commit_response(&mut self, commit: &ConversationResponseCommit) {
        let &ConversationResponseCommit {
            request_id,
            responding_to,
            supersede_previous,
            record_id,
            next_record_id,
            attempt_id,
            next_attempt_id,
        } = commit;
        debug_assert!(self.records.len() < self.records.capacity());
        debug_assert_eq!(self.next_record_id, record_id.get());
        debug_assert_eq!(self.next_attempt_id, attempt_id.get());
        if supersede_previous {
            for record in &mut self.records {
                if let Some(attempt) = record.response_attempt.as_mut()
                    && attempt.responding_to == responding_to
                {
                    attempt.superseded = true;
                }
            }
        }
        self.next_record_id = next_record_id;
        self.next_attempt_id = next_attempt_id;
        self.records.push(ConversationRecord {
            id: record_id,
            ordinal: record_id.get(),
            role: ConversationRole::Assistant,
            content: String::new(),
            provenance: ConversationProvenance::Model,
            retention: ConversationRetention::Retained,
            token_estimate: ConversationTokenEstimate::Measured(0),
            response_attempt: Some(ResponseAttempt {
                id: attempt_id,
                responding_to,
                state: ResponseAttemptState::Streaming,
                superseded: false,
            }),
        });
        self.active_response = Some(ActiveResponse {
            request_id,
            record_id,
        });
    }

    pub(crate) fn append_active_text(&mut self, request_id: RequestId, text: &str) {
        let Some(active) = self
            .active_response
            .filter(|active| active.request_id == request_id)
        else {
            return;
        };
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.id == active.record_id)
        {
            record.content.push_str(text);
        }
    }

    pub(crate) fn finish_active(
        &mut self,
        request_id: RequestId,
        outcome: &GenerationTerminalOutcome,
        generated_tokens: u64,
    ) {
        let Some(active) = self
            .active_response
            .filter(|active| active.request_id == request_id)
        else {
            return;
        };
        let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.id == active.record_id)
        else {
            self.active_response = None;
            return;
        };
        record.token_estimate = ConversationTokenEstimate::Generated(
            u32::try_from(generated_tokens).unwrap_or(u32::MAX),
        );
        if let Some(attempt) = record.response_attempt.as_mut() {
            attempt.state = match outcome {
                GenerationTerminalOutcome::Finished(FinishReason::Cancelled(reason)) => {
                    ResponseAttemptState::Cancelled(*reason)
                }
                GenerationTerminalOutcome::Finished(reason) => {
                    ResponseAttemptState::Completed(*reason)
                }
                GenerationTerminalOutcome::Failed(failure) => {
                    ResponseAttemptState::Failed(failure.clone())
                }
            };
        }
        self.active_response = None;
    }

    pub(crate) fn last_regenerable_user(&self) -> Option<ConversationRecordId> {
        self.records.last().and_then(|record| {
            (record.role == ConversationRole::Assistant)
                .then_some(record.response_attempt.as_ref())
                .flatten()
                .map(|attempt| attempt.responding_to)
        })
    }

    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.active_response = None;
    }

    fn next_record_id(&mut self) -> Result<ConversationRecordId, ApplicationError> {
        let value = self.next_record_id;
        self.next_record_id = value
            .checked_add(1)
            .ok_or(ApplicationError::ConversationIdentityExhausted)?;
        Ok(ConversationRecordId(value))
    }
}

#[cfg(test)]
mod tests {
    use domain_contracts::{CancellationReason, FinishReason, RequestId};

    use super::{
        ConversationRetention, ConversationState, ConversationTokenEstimate, ResponseAttemptState,
    };
    use crate::{ApplicationFailure, ApplicationFailureKind, GenerationTerminalOutcome};

    #[test]
    fn unsuccessful_and_superseded_attempts_remain_raw_but_leave_active_context()
    -> Result<(), crate::ApplicationError> {
        let mut conversation = ConversationState::default();
        let user = conversation.commit_user(
            "question".to_owned(),
            ConversationRetention::Retained,
            ConversationTokenEstimate::Measured(1),
        )?;
        let first_request = RequestId::new(1);
        let first = conversation.prepare_response(first_request, user, false)?;
        conversation.commit_response(&first);
        conversation.append_active_text(first_request, "partial");
        conversation.finish_active(
            first_request,
            &GenerationTerminalOutcome::Finished(FinishReason::Cancelled(
                CancellationReason::UserRequested,
            )),
            1,
        );

        let second_request = RequestId::new(2);
        let second = conversation.prepare_response(second_request, user, true)?;
        conversation.commit_response(&second);
        conversation.append_active_text(second_request, "failed partial");
        conversation.finish_active(
            second_request,
            &GenerationTerminalOutcome::Failed(ApplicationFailure::new(
                ApplicationFailureKind::Inference,
                "failure",
            )),
            2,
        );

        assert_eq!(conversation.records().len(), 3);
        let first = conversation
            .records()
            .get(1)
            .and_then(|record| record.response_attempt.as_ref());
        assert!(first.is_some_and(|attempt| {
            attempt.superseded
                && matches!(
                    attempt.state,
                    ResponseAttemptState::Cancelled(CancellationReason::UserRequested)
                )
        }));
        let second = conversation.records().get(2);
        assert!(second.is_some_and(|record| {
            record.content == "failed partial"
                && !record.is_active_context()
                && matches!(
                    record
                        .response_attempt
                        .as_ref()
                        .map(|attempt| &attempt.state),
                    Some(ResponseAttemptState::Failed(_))
                )
        }));
        Ok(())
    }

    #[test]
    fn later_unanswered_user_blocks_regeneration_of_an_older_turn()
    -> Result<(), crate::ApplicationError> {
        let mut conversation = ConversationState::default();
        let first_user = conversation.commit_user(
            "first".to_owned(),
            ConversationRetention::Retained,
            ConversationTokenEstimate::Measured(1),
        )?;
        let first_request = RequestId::new(1);
        let first = conversation.prepare_response(first_request, first_user, false)?;
        conversation.commit_response(&first);
        conversation.finish_active(
            first_request,
            &GenerationTerminalOutcome::Finished(FinishReason::TokenLimit),
            1,
        );

        assert_eq!(conversation.last_regenerable_user(), Some(first_user));

        let second_user = conversation.commit_user(
            "second".to_owned(),
            ConversationRetention::Retained,
            ConversationTokenEstimate::Measured(1),
        )?;

        assert_ne!(first_user, second_user);
        assert_eq!(conversation.last_regenerable_user(), None);
        Ok(())
    }
}
