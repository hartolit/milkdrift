//! Pre-allocated pull-oriented UTF-8 accumulation for application-clock consumers.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, TryLockError};
use std::vec::Vec;

use domain_contracts::{CapacityExhausted, CapacityResource, RequestId};

use crate::output::{OutputPullError, OutputPushError};

/// Failure to allocate the bounded text output accumulator during cold setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextOutputInitializationError {
    /// Host allocation for UTF-8 bytes failed.
    ByteStorage,
    /// Host allocation for output records failed.
    RecordStorage,
}

/// Monotonic byte position in one text output accumulator.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextOutputCursor(u64);

impl TextOutputCursor {
    /// Creates a text output cursor.
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

/// Half-open absolute UTF-8 byte range emitted by one accumulator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextRange {
    /// Inclusive absolute byte cursor.
    pub start: TextOutputCursor,
    /// Number of contiguous UTF-8 bytes in the range.
    pub length: usize,
}

impl TextRange {
    /// Creates an absolute text range.
    #[must_use]
    pub const fn new(start: TextOutputCursor, length: usize) -> Self {
        Self { start, length }
    }
}

/// Request-scoped text output record payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextOutputRecordKind<S: Copy> {
    /// UTF-8 text committed to the batch byte storage.
    Text(TextRange),
    /// Application-defined generation or cleanup state.
    State(S),
}

/// One request-scoped record in a pulled text output batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextOutputRecord<S: Copy> {
    /// Request that produced the record.
    pub request_id: RequestId,
    /// Text range or application-defined state payload.
    pub kind: TextOutputRecordKind<S>,
}

/// Borrowed UTF-8 output drained by a frontend on its own cadence.
pub struct TextOutputBatch<'a, S: Copy> {
    /// Cursor immediately before the first byte in this batch.
    pub start: TextOutputCursor,
    /// Cursor immediately after the last byte in this batch.
    pub end: TextOutputCursor,
    /// Contiguous UTF-8 bytes referenced by text records.
    pub bytes: &'a [u8],
    /// Ordered request-scoped text and state records.
    pub records: &'a [TextOutputRecord<S>],
}

impl<'a, S: Copy> TextOutputBatch<'a, S> {
    /// Resolves an absolute range from this batch to its borrowed UTF-8 text.
    #[must_use]
    pub fn text_for(&self, range: TextRange) -> Option<&'a str> {
        let offset = range.start.get().checked_sub(self.start.get())?;
        let offset = usize::try_from(offset).ok()?;
        let end = offset.checked_add(range.length)?;
        let bytes = self.bytes.get(offset..end)?;
        std::str::from_utf8(bytes).ok()
    }
}

/// Application-side handle for nonblocking UTF-8 and state publication.
///
/// Cloning is intended only during cold thread composition. Pushes use
/// `try_lock`, so a frontend holding a borrowed batch never blocks the producer.
#[derive(Clone)]
pub struct TextOutputProducer<S: Copy> {
    shared: Arc<Mutex<TextOutputState<S>>>,
    byte_capacity: usize,
    record_capacity: usize,
}

/// Frontend-side handle that drains accumulated UTF-8 output.
pub struct TextOutputConsumer<S: Copy> {
    shared: Arc<Mutex<TextOutputState<S>>>,
}

struct TextOutputState<S: Copy> {
    start: TextOutputCursor,
    bytes: Vec<u8>,
    records: Vec<TextOutputRecord<S>>,
    byte_capacity: usize,
    record_capacity: usize,
}

/// Creates one pre-allocated UTF-8 output accumulator.
///
/// The state payload is application-defined and must be `Copy`, so publishing
/// state records does not require ownership transfer or additional allocation.
///
/// # Errors
///
/// Returns [`TextOutputInitializationError::ByteStorage`] or
/// [`TextOutputInitializationError::RecordStorage`] when the corresponding
/// bounded storage allocation fails.
pub fn text_output_accumulator<S: Copy>(
    byte_capacity: NonZeroUsize,
    record_capacity: NonZeroUsize,
) -> Result<(TextOutputProducer<S>, TextOutputConsumer<S>), TextOutputInitializationError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(byte_capacity.get())
        .map_err(|_| TextOutputInitializationError::ByteStorage)?;
    let mut records = Vec::new();
    records
        .try_reserve_exact(record_capacity.get())
        .map_err(|_| TextOutputInitializationError::RecordStorage)?;
    let shared = Arc::new(Mutex::new(TextOutputState {
        start: TextOutputCursor::new(0),
        bytes,
        records,
        byte_capacity: byte_capacity.get(),
        record_capacity: record_capacity.get(),
    }));
    Ok((
        TextOutputProducer {
            shared: Arc::clone(&shared),
            byte_capacity: byte_capacity.get(),
            record_capacity: record_capacity.get(),
        },
        TextOutputConsumer { shared },
    ))
}

impl<S: Copy> TextOutputProducer<S> {
    /// Returns the fixed byte and record capacities reserved during initialization.
    #[must_use]
    pub const fn capacities(&self) -> (usize, usize) {
        (self.byte_capacity, self.record_capacity)
    }

    /// Appends one complete UTF-8 fragment and its request-scoped range record.
    ///
    /// Capacity is checked before either vector changes, so every failed push is
    /// atomic and the caller can retry the same fragment after a pull.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`] when the frontend holds the
    /// accumulator, [`OutputPushError::CapacityExhausted`] when bounded byte or
    /// record storage is full, or [`OutputPushError::Poisoned`] after a panic
    /// poisons the accumulator mutex.
    pub fn try_push_text(&self, request_id: RequestId, text: &str) -> Result<(), OutputPushError> {
        if text.is_empty() {
            return Ok(());
        }

        let mut state = self.try_lock()?;
        let required_bytes = state.bytes.len().saturating_add(text.len());
        if required_bytes > state.byte_capacity {
            return Err(CapacityExhausted::new(
                CapacityResource::OutputBytes,
                usize_to_u64(required_bytes),
                usize_to_u64(state.byte_capacity),
            )
            .into());
        }
        ensure_record_capacity(&state)?;

        let range_start = checked_cursor(&state, state.bytes.len())?;
        let _range_end = checked_cursor(&state, required_bytes)?;
        state.bytes.extend_from_slice(text.as_bytes());
        state.records.push(TextOutputRecord {
            request_id,
            kind: TextOutputRecordKind::Text(TextRange::new(range_start, text.len())),
        });
        drop(state);
        Ok(())
    }

    /// Appends one application-defined state record without blocking or allocating.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`] when the frontend holds the
    /// accumulator, [`OutputPushError::CapacityExhausted`] when record storage is
    /// full, or [`OutputPushError::Poisoned`] after a panic poisons the mutex.
    pub fn try_push_state(
        &self,
        request_id: RequestId,
        state_payload: S,
    ) -> Result<(), OutputPushError> {
        let mut state = self.try_lock()?;
        ensure_record_capacity(&state)?;
        state.records.push(TextOutputRecord {
            request_id,
            kind: TextOutputRecordKind::State(state_payload),
        });
        drop(state);
        Ok(())
    }

    /// Returns current committed byte and record counts when the consumer is idle.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`] when the consumer holds the
    /// accumulator or [`OutputPushError::Poisoned`] after a panic poisons it.
    pub fn try_lengths(&self) -> Result<(usize, usize), OutputPushError> {
        let state = self.try_lock()?;
        Ok((state.bytes.len(), state.records.len()))
    }

    fn try_lock(&self) -> Result<std::sync::MutexGuard<'_, TextOutputState<S>>, OutputPushError> {
        self.shared.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => OutputPushError::ConsumerBusy,
            TryLockError::Poisoned(_) => OutputPushError::Poisoned,
        })
    }
}

impl<S: Copy> TextOutputConsumer<S> {
    /// Borrows all accumulated text output, then clears its logical contents.
    ///
    /// The callback must copy any data needed after returning. Both vectors retain
    /// their cold-path allocations for subsequent producer pushes.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPullError::Poisoned`] when a prior panic poisoned the mutex.
    pub fn pull<R, F>(&self, consume: F) -> Result<R, OutputPullError>
    where
        F: for<'batch> FnOnce(TextOutputBatch<'batch, S>) -> R,
    {
        let mut state = self.shared.lock().map_err(|_| OutputPullError::Poisoned)?;
        let end = checked_pull_cursor(&state);
        let result = consume(TextOutputBatch {
            start: state.start,
            end,
            bytes: state.bytes.as_slice(),
            records: state.records.as_slice(),
        });
        state.start = end;
        state.bytes.clear();
        state.records.clear();
        drop(state);
        Ok(result)
    }
}

fn ensure_record_capacity<S: Copy>(state: &TextOutputState<S>) -> Result<(), OutputPushError> {
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
    state: &TextOutputState<S>,
    batch_offset: usize,
) -> Result<TextOutputCursor, OutputPushError> {
    let offset = usize_to_u64(batch_offset);
    state
        .start
        .get()
        .checked_add(offset)
        .map(TextOutputCursor::new)
        .ok_or_else(|| {
            CapacityExhausted::new(
                CapacityResource::OutputBytes,
                offset,
                u64::MAX.saturating_sub(state.start.get()),
            )
            .into()
        })
}

fn checked_pull_cursor<S: Copy>(state: &TextOutputState<S>) -> TextOutputCursor {
    TextOutputCursor::new(
        state
            .start
            .get()
            .saturating_add(usize_to_u64(state.bytes.len())),
    )
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
