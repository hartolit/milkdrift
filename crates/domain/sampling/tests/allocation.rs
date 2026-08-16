//! Allocation enforcement for the production sampling pipeline.

#![forbid(unsafe_code)]

use std::alloc::System;
use std::process::ExitCode;

use domain_contracts::TokenId;
use sampling::{Sampler, SamplingConfig, SamplingError, SamplingWorkspace};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const VOCABULARY_SIZE: usize = 256;
const REPETITION_HISTORY_LENGTH: usize = 32;
const MEASURED_SAMPLE_COUNT: usize = 64;
const REPEATED_TOKEN: u32 = 3;
const BASE_LOGIT: f32 = 0.5;
const REPETITION_PENALTY: f32 = 1.1;
const RANDOM_SEED: u64 = 7;

fn main() -> ExitCode {
    match measure_sampling_allocations() {
        Ok(allocation_change)
            if allocation_change.allocations == 0 && allocation_change.reallocations == 0 =>
        {
            ExitCode::SUCCESS
        }
        Ok(allocation_change) => {
            eprintln!("sampling allocated after preparation: {allocation_change:?}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("sampling failed during allocation measurement: {error:?}");
            ExitCode::FAILURE
        }
    }
}

fn measure_sampling_allocations() -> Result<Stats, SamplingError> {
    let configuration = SamplingConfig::new(0.8, 40, 0.95, 0.0, REPETITION_PENALTY, 64)?;
    let mut sampler = Sampler::new(configuration, RANDOM_SEED);
    let baseline_logits = [BASE_LOGIT; VOCABULARY_SIZE];
    let mut logits = baseline_logits;
    let repetition_history = [TokenId::new(REPEATED_TOKEN); REPETITION_HISTORY_LENGTH];
    let mut indices = [0_u32; VOCABULARY_SIZE];
    let mut seen_tokens = [0_u32; VOCABULARY_SIZE];

    let region = Region::new(GLOBAL);
    for _ in 0..MEASURED_SAMPLE_COUNT {
        logits.copy_from_slice(&baseline_logits);
        sampler.sample(
            &mut logits,
            &repetition_history,
            SamplingWorkspace {
                indices: &mut indices,
                seen_tokens: &mut seen_tokens,
            },
        )?;
    }
    Ok(region.change())
}
