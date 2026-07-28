//! Allocation enforcement for the production sampling pipeline.

#![forbid(unsafe_code)]

use std::alloc::System;

use domain_contracts::TokenId;
use sampling::{Sampler, SamplingConfig, SamplingError, SamplingWorkspace};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const VOCABULARY_SIZE: usize = 256;
const REPETITION_HISTORY_LENGTH: usize = 32;
const MEASURED_SAMPLE_COUNT: usize = 64;
const REPEATED_TOKEN: u32 = 3;
const BASE_LOGIT: f32 = 0.5;
const REPETITION_PENALTY: f32 = 1.1;
const RANDOM_SEED: u64 = 7;

#[test]
fn sampling_reuses_prepared_logits_and_workspace_without_allocating() -> Result<(), SamplingError> {
    let configuration = SamplingConfig {
        repetition_penalty: REPETITION_PENALTY,
        ..SamplingConfig::default()
    };
    let mut sampler = Sampler::new(configuration, RANDOM_SEED)?;
    let baseline_logits = [BASE_LOGIT; VOCABULARY_SIZE];
    let mut logits = baseline_logits;
    let repetition_history = [TokenId::new(REPEATED_TOKEN); REPETITION_HISTORY_LENGTH];
    let mut indices = [0_u32; VOCABULARY_SIZE];
    let mut seen_tokens = [0_u32; VOCABULARY_SIZE];

    let region = Region::new(GLOBAL);
    let mut every_sample_succeeded = true;
    for _ in 0..MEASURED_SAMPLE_COUNT {
        logits.copy_from_slice(&baseline_logits);
        every_sample_succeeded &= sampler
            .sample(
                &mut logits,
                &repetition_history,
                SamplingWorkspace {
                    indices: &mut indices,
                    seen_tokens: &mut seen_tokens,
                },
            )
            .is_ok();
    }
    let allocation_change = region.change();

    assert!(every_sample_succeeded);
    assert_eq!(allocation_change.allocations, 0, "{allocation_change:?}");
    assert_eq!(allocation_change.reallocations, 0, "{allocation_change:?}");
    Ok(())
}
