//! Pull-oriented output batch descriptions for frame-aligned UI consumption.

use crate::{FinishReason, RequestId, YieldReason};

/// Monotonic byte cursor into a bounded output accumulator.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputCursor(u64);

impl OutputCursor {
    /// Creates an output cursor.
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

/// Half-open byte range within an `OutputBatch` byte slice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ByteRange {
    /// Inclusive byte offset.
    pub start: usize,
    /// Number of bytes in the range.
    pub length: usize,
}

impl ByteRange {
    /// Creates a byte range.
    #[must_use]
    pub const fn new(start: usize, length: usize) -> Self {
        Self { start, length }
    }
}

/// Record kind stored separately from accumulated text bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputRecordKind {
    /// UTF-8 text committed to the batch byte storage.
    Text(ByteRange),
    /// Generation yielded because a bounded downstream resource was full.
    Yielded(YieldReason),
    /// Generation completed.
    Finished(FinishReason),
}

/// One request-scoped record in a pulled output batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputRecord {
    /// Request that produced the record.
    pub request_id: RequestId,
    /// Record payload.
    pub kind: OutputRecordKind,
}

/// Borrowed batch drained by a UI adapter on its own frame clock.
///
/// The inference runtime writes into a pre-allocated bounded accumulator. The UI
/// pulls a batch, typically once per native frame, instead of receiving one event
/// per token. If the accumulator is full, generation yields with
/// `YieldReason::OutputBackpressure` until a subsequent pull frees capacity.
pub struct OutputBatch<'a> {
    /// Cursor immediately before the first byte in this batch.
    pub start: OutputCursor,
    /// Cursor immediately after the last byte in this batch.
    pub end: OutputCursor,
    /// Contiguous UTF-8 bytes referenced by text records.
    pub bytes: &'a [u8],
    /// Contiguous records describing the byte ranges and terminal events.
    pub records: &'a [OutputRecord],
}
