//! Pre-allocated pull-oriented output accumulation for frame-clock consumers.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, TryLockError};
use std::vec::Vec;

use domain_contracts::{
    ByteRange, CapacityExhausted, CapacityResource, OutputBatch, OutputCursor, OutputRecord,
    OutputRecordKind, RequestId,
};

/// Failure to allocate the bounded output accumulator during cold setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputInitializationError {
    /// Host allocation for UTF-8 bytes failed.
    ByteStorage,
    /// Host allocation for output records failed.
    RecordStorage,
}

/// Non-blocking producer failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputPushError {
    /// The UI currently holds the accumulator while consuming a frame batch.
    ConsumerBusy,
    /// A pre-allocated output bound cannot accept the complete record.
    CapacityExhausted(CapacityExhausted),
    /// Caller attempted to inject a text range without committing matching bytes.
    InvalidRecordKind,
    /// A prior panic poisoned the short-lived output mutex.
    Poisoned,
}

impl From<CapacityExhausted> for OutputPushError {
    fn from(value: CapacityExhausted) -> Self {
        Self::CapacityExhausted(value)
    }
}

/// Consumer-side pull failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputPullError {
    /// A prior panic poisoned the short-lived output mutex.
    Poisoned,
}

/// Inference-side handle for appending decoded text and terminal records.
///
/// Cloning is intended only during cold thread composition. The producer hot path
/// performs no reference-count operation and uses `try_lock` so UI work never
/// blocks inference.
#[derive(Clone)]
pub struct OutputProducer {
    shared: Arc<Mutex<OutputState>>,
}

/// UI-side handle that drains one accumulated batch on its own frame clock.
pub struct OutputConsumer {
    shared: Arc<Mutex<OutputState>>,
}

struct OutputState {
    start: OutputCursor,
    bytes: Vec<u8>,
    records: Vec<OutputRecord>,
    byte_capacity: usize,
    record_capacity: usize,
}

/// Creates one pre-allocated output accumulator with producer and consumer ends.
///
/// # Errors
///
/// Returns [`OutputInitializationError::ByteStorage`] or
/// [`OutputInitializationError::RecordStorage`] when the corresponding bounded
/// storage allocation fails.
pub fn output_accumulator(
    byte_capacity: NonZeroUsize,
    record_capacity: NonZeroUsize,
) -> Result<(OutputProducer, OutputConsumer), OutputInitializationError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(byte_capacity.get())
        .map_err(|_| OutputInitializationError::ByteStorage)?;
    let mut records = Vec::new();
    records
        .try_reserve_exact(record_capacity.get())
        .map_err(|_| OutputInitializationError::RecordStorage)?;
    let shared = Arc::new(Mutex::new(OutputState {
        start: OutputCursor::new(0),
        bytes,
        records,
        byte_capacity: byte_capacity.get(),
        record_capacity: record_capacity.get(),
    }));
    Ok((
        OutputProducer {
            shared: Arc::clone(&shared),
        },
        OutputConsumer { shared },
    ))
}

impl OutputProducer {
    /// Appends one complete UTF-8 fragment and its request-scoped record.
    ///
    /// Capacity is validated before either vector is modified, so failure cannot
    /// leave a partial text fragment or record.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`] when the consumer holds the
    /// accumulator, [`OutputPushError::CapacityExhausted`] when either bounded
    /// storage limit would be exceeded, or [`OutputPushError::Poisoned`] when a
    /// prior panic poisoned the accumulator mutex.
    pub fn try_push_text(&self, request_id: RequestId, text: &str) -> Result<(), OutputPushError> {
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

        let start = state.bytes.len();
        state.bytes.extend_from_slice(text.as_bytes());
        state.records.push(OutputRecord {
            request_id,
            kind: OutputRecordKind::Text(ByteRange::new(start, text.len())),
        });
        drop(state);
        Ok(())
    }

    /// Appends one non-text output record without allocating.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::InvalidRecordKind`] for a text record,
    /// [`OutputPushError::ConsumerBusy`] when the consumer holds the accumulator,
    /// [`OutputPushError::CapacityExhausted`] when the bounded record storage is
    /// full, or [`OutputPushError::Poisoned`] when a prior panic poisoned the
    /// accumulator mutex.
    pub fn try_push_record(
        &self,
        request_id: RequestId,
        kind: OutputRecordKind,
    ) -> Result<(), OutputPushError> {
        if matches!(kind, OutputRecordKind::Text(_)) {
            return Err(OutputPushError::InvalidRecordKind);
        }
        let mut state = self.try_lock()?;
        ensure_record_capacity(&state)?;
        state.records.push(OutputRecord { request_id, kind });
        drop(state);
        Ok(())
    }

    /// Returns current committed byte and record counts when the consumer is idle.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPushError::ConsumerBusy`] when the consumer holds the
    /// accumulator or [`OutputPushError::Poisoned`] when a prior panic poisoned
    /// the accumulator mutex.
    pub fn try_lengths(&self) -> Result<(usize, usize), OutputPushError> {
        let state = self.try_lock()?;
        Ok((state.bytes.len(), state.records.len()))
    }

    fn try_lock(&self) -> Result<std::sync::MutexGuard<'_, OutputState>, OutputPushError> {
        self.shared.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => OutputPushError::ConsumerBusy,
            TryLockError::Poisoned(_) => OutputPushError::Poisoned,
        })
    }
}

impl OutputConsumer {
    /// Pulls all currently accumulated output through a borrowed batch and then
    /// clears the logical contents while retaining the original allocations.
    ///
    /// A Slint adapter should call this once per native frame. The callback must
    /// copy any data it needs after returning because the next producer write may
    /// reuse the same storage.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPullError::Poisoned`] when a prior panic poisoned the
    /// accumulator mutex.
    pub fn pull<R, F>(&self, consume: F) -> Result<R, OutputPullError>
    where
        F: for<'batch> FnOnce(OutputBatch<'batch>) -> R,
    {
        let mut state = self.shared.lock().map_err(|_| OutputPullError::Poisoned)?;
        let byte_count = usize_to_u64(state.bytes.len());
        let end = OutputCursor::new(state.start.get().saturating_add(byte_count));
        let result = consume(OutputBatch {
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

fn ensure_record_capacity(state: &OutputState) -> Result<(), OutputPushError> {
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

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
