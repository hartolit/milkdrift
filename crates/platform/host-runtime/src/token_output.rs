//! Pre-allocated pull-oriented token accumulation for application-clock consumers.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, TryLockError};
use std::vec::Vec;

use domain_contracts::{CapacityExhausted, CapacityResource, RequestId, TokenId};

use crate::output::{OutputPullError, OutputPushError};

/// Failure to allocate the bounded token output accumulator during cold setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenOutputInitializationError {
    /// Host allocation for token identifiers failed.
    TokenStorage,
    /// Host allocation for token output records failed.
    RecordStorage,
}

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

/// Half-open absolute token range emitted by one accumulator.
///
/// `start` is monotonic across pulls. `length` tokens beginning at `start` are
/// stored in the batch containing this range.
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
    ///
    /// Returns `None` when the range does not belong wholly to this batch.
    #[must_use]
    pub fn tokens_for(&self, range: TokenRange) -> Option<&'a [TokenId]> {
        let offset = range.start.get().checked_sub(self.start.get())?;
        let offset = usize::try_from(offset).ok()?;
        let end = offset.checked_add(range.length)?;
        self.tokens.get(offset..end)
    }
}

/// Inference-side handle for nonblocking token and state publication.
///
/// Cloning is intended only during cold thread composition. Pushes use
/// `try_lock`, so a consumer holding a borrowed batch never blocks inference.
#[derive(Clone)]
pub struct TokenOutputProducer<S: Copy> {
    shared: Arc<Mutex<TokenOutputState<S>>>,
    token_capacity: usize,
    record_capacity: usize,
}

/// Application-side handle that drains accumulated token output.
pub struct TokenOutputConsumer<S: Copy> {
    shared: Arc<Mutex<TokenOutputState<S>>>,
}

struct TokenOutputState<S: Copy> {
    start: TokenOutputCursor,
    tokens: Vec<TokenId>,
    records: Vec<TokenOutputRecord<S>>,
    token_capacity: usize,
    record_capacity: usize,
}

/// Creates one pre-allocated token output accumulator.
///
/// The state payload is supplied by the inference layer and must be `Copy`, so
/// publishing and inspecting state records requires no ownership transfer or
/// allocation.
///
/// # Errors
///
/// Returns [`TokenOutputInitializationError::TokenStorage`] or
/// [`TokenOutputInitializationError::RecordStorage`] when the corresponding
/// bounded storage allocation fails.
pub fn token_output_accumulator<S: Copy>(
    token_capacity: NonZeroUsize,
    record_capacity: NonZeroUsize,
) -> Result<(TokenOutputProducer<S>, TokenOutputConsumer<S>), TokenOutputInitializationError> {
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(token_capacity.get())
        .map_err(|_| TokenOutputInitializationError::TokenStorage)?;
    let mut records = Vec::new();
    records
        .try_reserve_exact(record_capacity.get())
        .map_err(|_| TokenOutputInitializationError::RecordStorage)?;
    let shared = Arc::new(Mutex::new(TokenOutputState {
        start: TokenOutputCursor::new(0),
        tokens,
        records,
        token_capacity: token_capacity.get(),
        record_capacity: record_capacity.get(),
    }));
    Ok((
        TokenOutputProducer {
            shared: Arc::clone(&shared),
            token_capacity: token_capacity.get(),
            record_capacity: record_capacity.get(),
        },
        TokenOutputConsumer { shared },
    ))
}

impl<S: Copy> TokenOutputProducer<S> {
    /// Returns the fixed token and record capacities reserved during initialization.
    #[must_use]
    pub const fn capacities(&self) -> (usize, usize) {
        (self.token_capacity, self.record_capacity)
    }

    /// Appends one token and its request-scoped range record without blocking.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`] when the consumer holds the
    /// accumulator, [`OutputPushError::CapacityExhausted`] when bounded token or
    /// record storage is full, or [`OutputPushError::Poisoned`] after a panic
    /// poisons the accumulator mutex.
    pub fn try_push_token(
        &self,
        request_id: RequestId,
        token: TokenId,
    ) -> Result<(), OutputPushError> {
        self.try_push_tokens(request_id, std::slice::from_ref(&token))
    }

    /// Appends a contiguous token slice and one request-scoped range record.
    ///
    /// Empty slices are accepted as a no-op. Capacity is checked before either
    /// vector changes, so every failed push is atomic.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`] when the consumer holds the
    /// accumulator, [`OutputPushError::CapacityExhausted`] when bounded token or
    /// record storage is full, or [`OutputPushError::Poisoned`] after a panic
    /// poisons the accumulator mutex.
    pub fn try_push_tokens(
        &self,
        request_id: RequestId,
        tokens: &[TokenId],
    ) -> Result<(), OutputPushError> {
        if tokens.is_empty() {
            return Ok(());
        }

        let mut state = self.try_lock()?;
        let required_tokens = state.tokens.len().saturating_add(tokens.len());
        if required_tokens > state.token_capacity {
            return Err(CapacityExhausted::new(
                CapacityResource::Tokens,
                usize_to_u64(required_tokens),
                usize_to_u64(state.token_capacity),
            )
            .into());
        }
        ensure_record_capacity(&state)?;

        let range_start = checked_cursor(&state, state.tokens.len())?;
        let _range_end = checked_cursor(&state, required_tokens)?;
        state.tokens.extend_from_slice(tokens);
        state.records.push(TokenOutputRecord {
            request_id,
            kind: TokenOutputRecordKind::Tokens(TokenRange::new(range_start, tokens.len())),
        });
        drop(state);
        Ok(())
    }

    /// Appends one inference-defined state record without blocking or allocating.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`] when the consumer holds the
    /// accumulator, [`OutputPushError::CapacityExhausted`] when bounded record
    /// storage is full, or [`OutputPushError::Poisoned`] after a panic poisons
    /// the accumulator mutex.
    pub fn try_push_state(
        &self,
        request_id: RequestId,
        state_payload: S,
    ) -> Result<(), OutputPushError> {
        let mut state = self.try_lock()?;
        ensure_record_capacity(&state)?;
        state.records.push(TokenOutputRecord {
            request_id,
            kind: TokenOutputRecordKind::State(state_payload),
        });
        drop(state);
        Ok(())
    }

    /// Returns current committed token and record counts when the consumer is idle.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`] when the consumer holds the
    /// accumulator or [`OutputPushError::Poisoned`] after a panic poisons the
    /// accumulator mutex.
    pub fn try_lengths(&self) -> Result<(usize, usize), OutputPushError> {
        let state = self.try_lock()?;
        Ok((state.tokens.len(), state.records.len()))
    }

    fn try_lock(&self) -> Result<std::sync::MutexGuard<'_, TokenOutputState<S>>, OutputPushError> {
        self.shared.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => OutputPushError::ConsumerBusy,
            TryLockError::Poisoned(_) => OutputPushError::Poisoned,
        })
    }
}

impl<S: Copy> TokenOutputConsumer<S> {
    /// Borrows all accumulated token output, then clears its logical contents.
    ///
    /// The callback must copy any data needed after returning. Both vectors retain
    /// their cold-path allocations for subsequent producer pushes.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPullError::Poisoned`] when a prior panic poisoned the
    /// accumulator mutex.
    pub fn pull<R, F>(&self, consume: F) -> Result<R, OutputPullError>
    where
        F: for<'batch> FnOnce(TokenOutputBatch<'batch, S>) -> R,
    {
        let mut state = self.shared.lock().map_err(|_| OutputPullError::Poisoned)?;
        let end = checked_pull_cursor(&state);
        let result = consume(TokenOutputBatch {
            start: state.start,
            end,
            tokens: state.tokens.as_slice(),
            records: state.records.as_slice(),
        });
        state.start = end;
        state.tokens.clear();
        state.records.clear();
        drop(state);
        Ok(result)
    }
}

fn ensure_record_capacity<S: Copy>(state: &TokenOutputState<S>) -> Result<(), OutputPushError> {
    let required = state.records.len().saturating_add(1);
    if required > state.record_capacity {
        return Err(CapacityExhausted::new(
            CapacityResource::OutputRecords,
            usize_to_u64(required),
            usize_to_u64(state.record_capacity),
        )
        .into());
    }
    Ok(())
}

fn checked_cursor<S: Copy>(
    state: &TokenOutputState<S>,
    batch_offset: usize,
) -> Result<TokenOutputCursor, OutputPushError> {
    let offset = usize_to_u64(batch_offset);
    state
        .start
        .get()
        .checked_add(offset)
        .map(TokenOutputCursor::new)
        .ok_or_else(|| {
            CapacityExhausted::new(
                CapacityResource::Tokens,
                offset,
                u64::MAX.saturating_sub(state.start.get()),
            )
            .into()
        })
}

fn checked_pull_cursor<S: Copy>(state: &TokenOutputState<S>) -> TokenOutputCursor {
    TokenOutputCursor::new(
        state
            .start
            .get()
            .saturating_add(usize_to_u64(state.tokens.len())),
    )
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
