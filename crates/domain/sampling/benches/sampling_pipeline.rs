//! Statistical latency and throughput measurements for production sampling.

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::process;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use domain_contracts::TokenId;
use sampling::{Sampler, SamplingConfig, SamplingWorkspace};

const VOCABULARY_SIZE: usize = 32_768;
const VOCABULARY_ELEMENTS: u64 = 32_768;
const REPETITION_HISTORY_LENGTH: usize = 64;
const REPEATED_TOKEN: u32 = 17;
const RANDOM_SEED: u64 = 29;
const INITIAL_LOGIT: f32 = -8.0;
const LOGIT_STEP: f32 = 0.000_5;
const BENCHMARK_FAILURE_EXIT_CODE: i32 = 2;

fn benchmark_sampling_pipeline(criterion: &mut Criterion) {
    let baseline_logits = build_logits();
    let mut logits = baseline_logits.clone();
    let repetition_history = [TokenId::new(REPEATED_TOKEN); REPETITION_HISTORY_LENGTH];
    let mut indices = vec![0_u32; VOCABULARY_SIZE];
    let mut seen_tokens = vec![0_u32; VOCABULARY_SIZE];
    let mut sampler = match Sampler::new(SamplingConfig::default(), RANDOM_SEED) {
        Ok(sampler) => sampler,
        Err(error) => benchmark_failure("sampling configuration", error),
    };

    let mut group = criterion.benchmark_group("sampling_pipeline");
    group.throughput(Throughput::Elements(VOCABULARY_ELEMENTS));
    group.bench_function("default_policy", |benchmark| {
        benchmark.iter(|| {
            logits.copy_from_slice(black_box(&baseline_logits));
            match sampler.sample(
                &mut logits,
                black_box(&repetition_history),
                SamplingWorkspace {
                    indices: &mut indices,
                    seen_tokens: &mut seen_tokens,
                },
            ) {
                Ok(sample) => black_box(sample),
                Err(error) => benchmark_failure("sampling execution", error),
            }
        });
    });
    group.finish();
}

fn build_logits() -> Vec<f32> {
    let mut value = INITIAL_LOGIT;
    (0..VOCABULARY_SIZE)
        .map(|_| {
            value += LOGIT_STEP;
            value
        })
        .collect()
}

fn benchmark_failure(operation: &str, error: sampling::SamplingError) -> ! {
    eprintln!("{operation} failed: {error:?}");
    process::exit(BENCHMARK_FAILURE_EXIT_CODE);
}

criterion_group!(benches, benchmark_sampling_pipeline);
criterion_main!(benches);
