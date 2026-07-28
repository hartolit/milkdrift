//! Generation outcomes, cancellation state, and scheduler control signals.

use crate::{CapacityExhausted, TokenId};

/// Stable reason for requesting cancellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CancellationReason {
    /// User explicitly cancelled the request.
    UserRequested,
    /// Model unload requested immediate cancellation.
    ModelUnload,
    /// A drain deadline elapsed and escalated to forced cancellation.
    DrainTimeout,
    /// Runtime shutdown requested cancellation.
    RuntimeShutdown,
    /// A parent orchestration task was cancelled.
    ParentTask,
}

/// Cancellation value sampled by the engine before entering one backend step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CancellationStatus {
    /// Execution may continue.
    #[default]
    Running,
    /// Execution must finish at the next safe backend boundary.
    Requested(CancellationReason),
}

impl CancellationStatus {
    /// Returns the cancellation reason when cancellation was requested.
    #[must_use]
    pub const fn reason(self) -> Option<CancellationReason> {
        match self {
            Self::Running => None,
            Self::Requested(reason) => Some(reason),
        }
    }
}

/// Stable reason for completing generation without an unchecked failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FinishReason {
    /// Model produced an end-of-sequence token.
    EndOfSequence(TokenId),
    /// Configured token limit was reached.
    TokenLimit,
    /// A configured stop condition matched.
    StopCondition,
    /// A fixed-capacity buffer could not accept the next operation.
    BufferExhausted(CapacityExhausted),
    /// Request was cancelled at a safe boundary.
    Cancelled(CancellationReason),
}

impl From<CapacityExhausted> for FinishReason {
    fn from(value: CapacityExhausted) -> Self {
        Self::BufferExhausted(value)
    }
}

/// Reason generation yielded without completing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum YieldReason {
    /// Pre-allocated output storage is full until the consumer pulls data.
    OutputBackpressure(CapacityExhausted),
    /// Runtime scheduler requested cooperative yielding.
    Scheduler,
    /// Backend submitted asynchronous work and is waiting for completion.
    BackendPending,
}

/// Control result produced by one engine iteration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationControl {
    /// Continue generation immediately.
    Continue,
    /// Yield execution while retaining all prepared state.
    Yield(YieldReason),
    /// Complete generation with a stable finish reason.
    Finish(FinishReason),
}

/// Usage counters accumulated without allocation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GenerationUsage {
    /// Number of prompt tokens accepted by prefill.
    pub prompt_tokens: u64,
    /// Number of generated tokens accepted by decode.
    pub generated_tokens: u64,
}

/// Result of a checked prefill operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefillOutcome {
    /// Prefill completed and the sequence may continue.
    Ready {
        /// Number of prompt tokens consumed.
        consumed_tokens: usize,
        /// New sequence position.
        position: usize,
        /// Number of valid logits written to the output slice.
        logits_written: usize,
    },
    /// Prefill stopped cleanly without calling unchecked behavior.
    Finished(FinishReason),
}

/// Result of a checked single-token decode operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeOutcome {
    /// Decode completed and produced valid logits.
    Ready {
        /// New sequence position.
        position: usize,
        /// Number of valid logits written to the output slice.
        logits_written: usize,
    },
    /// Decode stopped cleanly without calling unchecked behavior.
    Finished(FinishReason),
}
