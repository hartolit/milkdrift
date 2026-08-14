//! Strongly typed pull-oriented UTF-8 output.

use std::num::NonZeroUsize;

use domain_contracts::{CapacityResource, RequestId};

use super::bounded::{self, Consumer, Producer};
use super::{OutputInitializationError, OutputPullError, OutputPushError};

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

/// Half-open absolute UTF-8 byte range emitted by one text accumulator.
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
    /// Resolves an absolute range from this batch to borrowed UTF-8 text.
    #[must_use]
    pub fn text_for(&self, range: TextRange) -> Option<&'a str> {
        let bytes = bounded::payload_for_range(
            self.bytes,
            self.start.get(),
            bounded::Range {
                start: range.start.get(),
                length: range.length,
            },
        )?;
        std::str::from_utf8(bytes).ok()
    }
}

/// Application-side handle for nonblocking UTF-8 and state publication.
///
/// Cloning is intended only during cold thread composition. Pushes perform no
/// reference-count operation and never block while the consumer borrows a batch.
#[derive(Clone)]
pub struct TextOutputProducer<S: Copy> {
    core: Producer<u8, TextOutputRecord<S>>,
}

/// Frontend-side handle that drains accumulated UTF-8 output.
pub struct TextOutputConsumer<S: Copy> {
    core: Consumer<u8, TextOutputRecord<S>>,
}

/// Creates one pre-allocated UTF-8 output accumulator.
///
/// # Errors
///
/// Returns [`OutputInitializationError::PayloadStorage`] or
/// [`OutputInitializationError::RecordStorage`] when cold allocation fails.
pub fn text_output_accumulator<S: Copy>(
    byte_capacity: NonZeroUsize,
    record_capacity: NonZeroUsize,
) -> Result<(TextOutputProducer<S>, TextOutputConsumer<S>), OutputInitializationError> {
    let (producer, consumer) = bounded::accumulator(
        byte_capacity,
        record_capacity,
        CapacityResource::OutputBytes,
    )?;
    Ok((
        TextOutputProducer { core: producer },
        TextOutputConsumer { core: consumer },
    ))
}

impl<S: Copy> TextOutputProducer<S> {
    /// Returns the fixed byte and record capacities admitted at initialization.
    #[must_use]
    pub fn capacities(&self) -> (usize, usize) {
        let capacities = self.core.capacities();
        (capacities.payload, capacities.records)
    }

    /// Appends one complete UTF-8 fragment and its request-scoped range record.
    ///
    /// Empty text is an explicit no-op. Every fallible capacity and cursor check
    /// completes before either retained vector is modified.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`],
    /// [`OutputPushError::CapacityExhausted`], or [`OutputPushError::Poisoned`].
    pub fn try_push_text(&self, request_id: RequestId, text: &str) -> Result<(), OutputPushError> {
        if text.is_empty() {
            return Ok(());
        }
        self.core
            .try_push_payload(text.as_bytes(), |range| TextOutputRecord {
                request_id,
                kind: TextOutputRecordKind::Text(TextRange::new(
                    TextOutputCursor::new(range.start),
                    range.length,
                )),
            })
    }

    /// Appends one application-defined state record without consuming byte capacity.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`],
    /// [`OutputPushError::CapacityExhausted`], or [`OutputPushError::Poisoned`].
    pub fn try_push_state(&self, request_id: RequestId, state: S) -> Result<(), OutputPushError> {
        self.core.try_push_record(TextOutputRecord {
            request_id,
            kind: TextOutputRecordKind::State(state),
        })
    }

    /// Returns current committed byte and record counts when the consumer is idle.
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

impl<S: Copy> TextOutputConsumer<S> {
    /// Borrows all accumulated text output, then clears its logical contents.
    ///
    /// The callback must copy any data needed after returning. Both allocations
    /// remain retained for subsequent producer pushes.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPullError::Poisoned`] after a consumer panic.
    pub fn pull<R, F>(&self, consume: F) -> Result<R, OutputPullError>
    where
        F: for<'batch> FnOnce(TextOutputBatch<'batch, S>) -> R,
    {
        self.core.pull(|batch| {
            consume(TextOutputBatch {
                start: TextOutputCursor::new(batch.start),
                end: TextOutputCursor::new(batch.end),
                bytes: batch.payload,
                records: batch.records,
            })
        })
    }
}
