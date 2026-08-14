//! Strongly typed pull-oriented token output.

use std::num::NonZeroUsize;

use domain_contracts::{CapacityResource, RequestId, TokenId};

use super::bounded::{self, Consumer, Producer};
use super::{OutputInitializationError, OutputPullError, OutputPushError};

/// Monotonic token position in one token output accumulator.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenOutputCursor(u64);

impl TokenOutputCursor {
    /// Creates a token output cursor.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw cursor value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Half-open absolute token range emitted by one token accumulator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenRange {
    /// Inclusive absolute token cursor.
    pub start: TokenOutputCursor,
    /// Number of contiguous tokens in the range.
    pub length: usize,
}

impl TokenRange {
    /// Creates an absolute token range.
    #[must_use]
    pub const fn new(start: TokenOutputCursor, length: usize) -> Self {
        Self { start, length }
    }
}

/// Request-scoped token output record payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenOutputRecordKind<S: Copy> {
    /// Tokens committed to the batch token storage.
    Tokens(TokenRange),
    /// Inference-defined generation or cleanup state.
    State(S),
}

/// One request-scoped record in a pulled token output batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenOutputRecord<S: Copy> {
    /// Request that produced the record.
    pub request_id: RequestId,
    /// Token range or inference-defined state payload.
    pub kind: TokenOutputRecordKind<S>,
}

/// Borrowed token output drained by an application adapter on its own cadence.
pub struct TokenOutputBatch<'a, S: Copy> {
    /// Cursor immediately before the first token in this batch.
    pub start: TokenOutputCursor,
    /// Cursor immediately after the last token in this batch.
    pub end: TokenOutputCursor,
    /// Contiguous token identifiers referenced by token range records.
    pub tokens: &'a [TokenId],
    /// Ordered request-scoped token and state records.
    pub records: &'a [TokenOutputRecord<S>],
}

impl<'a, S: Copy> TokenOutputBatch<'a, S> {
    /// Resolves an absolute range from this batch to its borrowed token slice.
    #[must_use]
    pub fn tokens_for(&self, range: TokenRange) -> Option<&'a [TokenId]> {
        bounded::payload_for_range(
            self.tokens,
            self.start.get(),
            bounded::Range {
                start: range.start.get(),
                length: range.length,
            },
        )
    }
}

/// Inference-side handle for nonblocking token and state publication.
///
/// Cloning is intended only during cold thread composition. Pushes perform no
/// reference-count operation and never block while the consumer borrows a batch.
#[derive(Clone)]
pub struct TokenOutputProducer<S: Copy> {
    core: Producer<TokenId, TokenOutputRecord<S>>,
}

/// Application-side handle that drains accumulated token output.
pub struct TokenOutputConsumer<S: Copy> {
    core: Consumer<TokenId, TokenOutputRecord<S>>,
}

/// Creates one pre-allocated token output accumulator.
///
/// # Errors
///
/// Returns [`OutputInitializationError::PayloadStorage`] or
/// [`OutputInitializationError::RecordStorage`] when cold allocation fails.
pub fn token_output_accumulator<S: Copy>(
    token_capacity: NonZeroUsize,
    record_capacity: NonZeroUsize,
) -> Result<(TokenOutputProducer<S>, TokenOutputConsumer<S>), OutputInitializationError> {
    let (producer, consumer) =
        bounded::accumulator(token_capacity, record_capacity, CapacityResource::Tokens)?;
    Ok((
        TokenOutputProducer { core: producer },
        TokenOutputConsumer { core: consumer },
    ))
}

impl<S: Copy> TokenOutputProducer<S> {
    /// Returns the fixed token and record capacities admitted at initialization.
    #[must_use]
    pub fn capacities(&self) -> (usize, usize) {
        let capacities = self.core.capacities();
        (capacities.payload, capacities.records)
    }

    /// Appends one token and its request-scoped range record without blocking.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`],
    /// [`OutputPushError::CapacityExhausted`], or [`OutputPushError::Poisoned`].
    pub fn try_push_token(
        &self,
        request_id: RequestId,
        token: TokenId,
    ) -> Result<(), OutputPushError> {
        self.try_push_tokens(request_id, std::slice::from_ref(&token))
    }

    /// Appends a contiguous token slice and one request-scoped range record.
    ///
    /// Empty slices are an explicit no-op. Every fallible capacity and cursor
    /// check completes before either retained vector is modified.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`],
    /// [`OutputPushError::CapacityExhausted`], or [`OutputPushError::Poisoned`].
    pub fn try_push_tokens(
        &self,
        request_id: RequestId,
        tokens: &[TokenId],
    ) -> Result<(), OutputPushError> {
        if tokens.is_empty() {
            return Ok(());
        }
        self.core
            .try_push_payload(tokens, |range| TokenOutputRecord {
                request_id,
                kind: TokenOutputRecordKind::Tokens(TokenRange::new(
                    TokenOutputCursor::new(range.start),
                    range.length,
                )),
            })
    }

    /// Appends one inference-defined state record without consuming token capacity.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`],
    /// [`OutputPushError::CapacityExhausted`], or [`OutputPushError::Poisoned`].
    pub fn try_push_state(&self, request_id: RequestId, state: S) -> Result<(), OutputPushError> {
        self.core.try_push_record(TokenOutputRecord {
            request_id,
            kind: TokenOutputRecordKind::State(state),
        })
    }

    /// Returns current committed token and record counts when the consumer is idle.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`] or [`OutputPushError::Poisoned`].
    pub fn try_lengths(&self) -> Result<(usize, usize), OutputPushError> {
        self.core.try_lengths()
    }

    #[cfg(test)]
    pub(super) fn set_cursor_for_test(&self, cursor: u64) -> Result<(), OutputPullError> {
        self.core.set_cursor_for_test(cursor)
    }
}

impl<S: Copy> TokenOutputConsumer<S> {
    /// Borrows all accumulated token output, then clears its logical contents.
    ///
    /// The callback must copy any data needed after returning. Both allocations
    /// remain retained for subsequent producer pushes.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPullError::Poisoned`] after a consumer panic.
    pub fn pull<R, F>(&self, consume: F) -> Result<R, OutputPullError>
    where
        F: for<'batch> FnOnce(TokenOutputBatch<'batch, S>) -> R,
    {
        self.core.pull(|batch| {
            consume(TokenOutputBatch {
                start: TokenOutputCursor::new(batch.start),
                end: TokenOutputCursor::new(batch.end),
                tokens: batch.payload,
                records: batch.records,
            })
        })
    }
}
