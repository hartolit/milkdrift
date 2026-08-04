//! Authoritative public-E1 generation workloads for the external baseline.

mod observer;
mod summary;
mod validation;
mod workload;

pub(super) use summary::{duration_ns, sampling_metadata};
pub(super) use workload::{
    CANCELLATION_MAXIMUM_NEW_TOKENS, CHAT_MAXIMUM_NEW_TOKENS, CHAT_MESSAGE,
    CHAT_MESSAGE_IDENTIFIER, DIRECT_COMPLETION_PROMPT, DIRECT_COMPLETION_PROMPT_IDENTIFIER,
    DIRECT_MAXIMUM_NEW_TOKENS, PrimaryWorkloadEvidence, SAMPLE_COUNT, WARMUP_COUNT,
    run_primary_workload, run_stability_workload,
};
