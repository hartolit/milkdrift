//! Shared hosted-E0 benchmark support and normal synthetic scenario ownership.

mod generation;
mod harness;
mod lifecycle;
mod observation;
mod synthetic;

pub(crate) use generation::{
    BACKPRESSURE_GENERATION_LIMIT, BACKPRESSURE_HOLD_MILLISECONDS, CANCELLATION_GENERATION_LIMIT,
    CANCELLATION_HOLD_MILLISECONDS, FIRST_TOKEN_GENERATION_LIMIT, POST_FIRST_TOKEN_WINDOW,
};
#[doc(hidden)]
pub use harness::{HostedE0Harness, ShutdownDurations};
pub(crate) use lifecycle::{CHECKED_PREFILL_TOKEN_COUNT, GENERATION_PROMPT_TOKEN_COUNT};
#[doc(hidden)]
pub use lifecycle::{
    CRITERION_VOCABULARY_SIZE, criterion_checked_prefill_iteration, criterion_harness,
    criterion_incremental_decode_iteration,
};
pub(crate) use synthetic::run_cycles;
