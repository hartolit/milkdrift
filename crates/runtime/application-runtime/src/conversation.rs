//! Frontend-neutral in-memory conversation records and response-attempt provenance.

use domain_contracts::{CancellationReason, FinishReason, RequestId};

use crate::{ApplicationError, ApplicationFailure, GenerationTerminalOutcome};

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
    /// Exact token count measured for the unwrapped semantic content.
    Measured(u32),
    /// Conservative count used when exact content measurement was unavailable.
    Conservative(u32),
}

impl ConversationTokenEstimate {
    pub(crate) const fn tokens(self) -> u32 {
        match self {
            Self::Measured(tokens) | Self::Conservative(tokens) => tokens,
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

    pub(crate) fn ensure_response_capacity(&self) -> Result<(), ApplicationError> {
        self.next_record_id
            .checked_add(1)
            .ok_or(ApplicationError::ConversationIdentityExhausted)?;
        self.next_attempt_id
            .checked_add(1)
            .ok_or(ApplicationError::ConversationIdentityExhausted)?;
        Ok(())
    }

    pub(crate) fn begin_response(
        &mut self,
        request_id: RequestId,
        responding_to: ConversationRecordId,
        supersede_previous: bool,
    ) -> Result<ResponseAttemptId, ApplicationError> {
        if supersede_previous {
            for record in &mut self.records {
                if let Some(attempt) = record.response_attempt.as_mut()
                    && attempt.responding_to == responding_to
                {
                    attempt.superseded = true;
                }
            }
        }
        let record_id = self.next_record_id()?;
        let attempt_id = self.next_attempt_id()?;
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
        Ok(attempt_id)
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
        record.token_estimate = ConversationTokenEstimate::Measured(
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
        self.records.iter().rev().find_map(|record| {
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

    fn next_attempt_id(&mut self) -> Result<ResponseAttemptId, ApplicationError> {
        let value = self.next_attempt_id;
        self.next_attempt_id = value
            .checked_add(1)
            .ok_or(ApplicationError::ConversationIdentityExhausted)?;
        Ok(ResponseAttemptId(value))
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
        conversation.begin_response(first_request, user, false)?;
        conversation.append_active_text(first_request, "partial");
        conversation.finish_active(
            first_request,
            &GenerationTerminalOutcome::Finished(FinishReason::Cancelled(
                CancellationReason::UserRequested,
            )),
            1,
        );

        let second_request = RequestId::new(2);
        conversation.begin_response(second_request, user, true)?;
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
}
