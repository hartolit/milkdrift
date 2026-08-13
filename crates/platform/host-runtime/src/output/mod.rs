//! Statically dispatched bounded output for host/runtime boundaries.

mod bounded;
mod text;
mod token;

pub use text::{
    TextOutputBatch, TextOutputConsumer, TextOutputCursor, TextOutputProducer, TextOutputRecord,
    TextOutputRecordKind, TextRange, text_output_accumulator,
};
pub use token::{
    TokenOutputBatch, TokenOutputConsumer, TokenOutputCursor, TokenOutputProducer,
    TokenOutputRecord, TokenOutputRecordKind, TokenRange, token_output_accumulator,
};

use domain_contracts::CapacityExhausted;

/// Failure to allocate one bounded output accumulator during cold setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputInitializationError {
    /// Host allocation for typed payload storage failed.
    PayloadStorage,
    /// Host allocation for ordered output records failed.
    RecordStorage,
}

/// Nonblocking producer failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputPushError {
    /// The consumer currently holds the accumulator while borrowing a batch.
    ConsumerBusy,
    /// A fixed payload, record, or absolute-cursor bound cannot admit the push.
    CapacityExhausted(CapacityExhausted),
    /// A prior consumer panic poisoned the short-lived output mutex.
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
    /// A prior consumer panic poisoned the short-lived output mutex.
    Poisoned,
}

#[cfg(test)]
mod tests;
