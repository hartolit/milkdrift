use crate::{PersistenceError, TimestampMillis};

/// Result of comparing one external clock observation with durable high-water evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockWatermarkObservation {
    /// The observation advanced the durable high-water mark.
    Advanced,
    /// The observation matched the durable high-water mark.
    Unchanged,
    /// The observation was older than durable evidence and must not be trusted.
    RejectedRollback {
        /// Latest durably accepted timestamp.
        watermark: TimestampMillis,
    },
}

/// Durable high-water evidence for an externally owned boundary clock.
///
/// The store does not read wall time. Callers supply an observation, and the store atomically
/// advances or rejects it so a process restart cannot forget an already observed later time.
pub trait ClockWatermarkStore: Send + Sync {
    /// Compares and, when newer, durably records one boundary-clock observation.
    fn observe_clock(
        &self,
        observed: TimestampMillis,
    ) -> Result<ClockWatermarkObservation, PersistenceError>;

    /// Returns the latest durably accepted observation, if this store has observed one.
    fn clock_watermark(&self) -> Result<Option<TimestampMillis>, PersistenceError>;
}
