//! Statistical component measurements for the public production sampling API.
//!
//! # Measurement contract
//!
//! Concrete sampler targets are the Cartesian product
//! `{sample_only,restore_and_sample}/<case>/{8192,32768,131072}`. Each target
//! uses a deterministic, bounded `[-8, 8]` pseudo-random logit fixture, one
//! seeded `Sampler`, caller-owned vocabulary-sized logits/indices/seen-token
//! vectors, and the named history. `Throughput::Elements` is one vocabulary per
//! sample. Fixture construction, vector/history allocation, sampler creation,
//! and capacity reservation happen once before Criterion enters the measured
//! loop; every iteration reuses those exact capacities and sampler state.
//!
//! The tables below jointly specify every concrete sampler target: the prefix
//! defines the timed boundary, while the case segment defines its named
//! question and input. The same question is measured at all three vocabulary
//! sizes to expose scale changes rather than claim model decode throughput.
//!
//! | Target prefix | Named question | Exact timed boundary |
//! | --- | --- | --- |
//! | `sample_only/<case>/<vocabulary>` | What is the production sampling-call cost after mutable logits are ready? | The baseline-to-working-logit copy completes before `Instant::now`; timing contains public workspace-view construction, `Sampler::sample`, result checking, and `black_box`. |
//! | `restore_and_sample/<case>/<vocabulary>` | What is the cost paid by a caller that must restore an overwritten logit buffer before each production sampling call? | `Instant::now` precedes the baseline-to-working-logit copy; timing contains that copy plus the same sampling-call boundary as `sample_only`. |
//!
//! | Case segment | Named regression/performance question | Configuration and history fixture |
//! | --- | --- | --- |
//! | `greedy` | How does deterministic highest-logit selection scale with vocabulary size? | `SamplingConfig::greedy()`; empty history. |
//! | `default_top_k_top_p` | What is the component cost of the application's default top-k/top-p policy? | `SamplingConfig::default()` (currently top-k 40 and top-p 0.95); empty history. |
//! | `min_p_0_05_full_vocabulary` | What cost does min-p filtering add when top-k does not pre-truncate the vocabulary? | Default temperature, top-k 0, top-p 1.0, min-p 0.05; empty history. |
//! | `repetition_disabled_history_256` | Does supplying a long history remain a cheap baseline when repetition penalty is disabled? | Default policy with penalty 1.0 and full-history window; 256 entries cycling over four token IDs. |
//! | `repetition_enabled_empty` | What fixed cost appears when repetition processing is enabled but history is empty? | Default top-k/top-p with penalty 1.1 and full-history window; empty history. |
//! | `repetition_short_unique_8` | What is the cost of penalizing a short distinct-token history? | Same enabled policy; 8 distinct token IDs. |
//! | `repetition_medium_unique_64` | How does the enabled path change at the default-sized 64-token history? | Same enabled policy; 64 distinct token IDs. |
//! | `repetition_repeated_heavy_256` | What is the cost of scanning a longer, duplicate-heavy history? | Same enabled policy; 256 entries cycling over four token IDs. |
//!
//! Sampling storage is fixed-capacity for a target. The three public sampler
//! slices (working logits, indices, and seen-token state) reserve 96 KiB,
//! 384 KiB, and 1.5 MiB at 8K, 32K, and 128K respectively. The untimed baseline
//! logit vector adds 32 KiB, 128 KiB, and 512 KiB; history capacity ranges from
//! zero to 256 `TokenId` values. Every sampler target is Criterion statistical
//! component-regression evidence, not deterministic allocation evidence or
//! E0/E1/product-throughput evidence. The measured loops call no allocation API
//! and perform no per-iteration clone. This benchmark records time, not
//! allocator events: `tests/allocation.rs` is the deterministic
//! Rust-global-allocator enforcement evidence, and no sampler target observes
//! or makes claims about native/backend/device allocations. No native backend
//! or device resource is present in these component fixtures.
//!
//! Stop matching executes the public production `match_stop_suffix` function
//! over caller-owned storage. Its concrete targets are:
//!
//! | Target | Named question | Timed boundary and fixture/input | Evidence class and allocation/native-resource limits |
//! | --- | --- | --- | --- |
//! | `stop_matching/token_hit/1_pattern_generated_128` | What is the one-token stop cost when the sole configured sequence matches? | Only `match_stop_suffix` and `black_box` are timed; 128 generated tokens and one one-token pattern are prepared outside timing. | Criterion statistical component-regression evidence; no allocation API is called in the measured closure, allocations are not counted, and no native resource is involved. |
//! | `stop_matching/pattern_hit_last/8_patterns_generated_128` | What is the suffix-match cost when a four-token match is last among eight configured patterns? | Only the public match call and `black_box` are timed; generated tokens, seven misses, and the final hit are prepared outside timing. | Same component evidence and limits; this is not E0/E1 or product-throughput evidence. |
//! | `stop_matching/pattern_miss/8_patterns_generated_128` | What is the common no-match scan cost across eight configured patterns? | Only the public match call and `black_box` are timed; all caller-owned inputs are prepared outside timing. | Same component evidence and limits; no allocator or native-resource observation is performed. |
//!
//! Criterion results are comparative statistical evidence, not shared-CI
//! pass/fail thresholds and not end-to-end token-generation throughput.

#![forbid(unsafe_code)]

mod support;

use std::hint::black_box;
use std::process;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use sampling::{Sample, SamplingError};
use support::{SAMPLING_CASES, STOP_CASES, SamplingFixture, StopFixture, VOCABULARY_SIZES};

const BENCHMARK_FAILURE_EXIT_CODE: i32 = 2;

#[derive(Clone, Copy)]
enum TimingBoundary {
    SampleOnly,
    RestoreAndSample,
}

impl TimingBoundary {
    const fn benchmark_group(self) -> &'static str {
        match self {
            Self::SampleOnly => "sample_only",
            Self::RestoreAndSample => "restore_and_sample",
        }
    }
}

fn benchmark_sampling_pipeline(criterion: &mut Criterion) {
    benchmark_sampling_boundary(criterion, TimingBoundary::SampleOnly);
    benchmark_sampling_boundary(criterion, TimingBoundary::RestoreAndSample);
    benchmark_stop_matching(criterion);
}

fn benchmark_sampling_boundary(criterion: &mut Criterion, boundary: TimingBoundary) {
    let mut group = criterion.benchmark_group(boundary.benchmark_group());

    for case in SAMPLING_CASES {
        for &(vocabulary_size, vocabulary_elements) in &VOCABULARY_SIZES {
            let mut fixture = match SamplingFixture::new(case, vocabulary_size) {
                Ok(fixture) => fixture,
                Err(error) => benchmark_failure("sampling fixture construction", error),
            };
            group.throughput(Throughput::Elements(vocabulary_elements));
            group.bench_function(
                BenchmarkId::new(case.name(), vocabulary_size),
                |benchmark| {
                    benchmark.iter_custom(|iterations| {
                        measure_sampling_iterations(&mut fixture, boundary, iterations)
                    });
                },
            );
        }
    }

    group.finish();
}

fn measure_sampling_iterations(
    fixture: &mut SamplingFixture,
    boundary: TimingBoundary,
    iterations: u64,
) -> Duration {
    match boundary {
        TimingBoundary::SampleOnly => measure_sample_only(fixture, iterations),
        TimingBoundary::RestoreAndSample => measure_restore_and_sample(fixture, iterations),
    }
}

fn measure_sample_only(fixture: &mut SamplingFixture, iterations: u64) -> Duration {
    let mut measured = Duration::ZERO;
    for _ in 0..iterations {
        fixture.restore_logits();
        let started = Instant::now();
        black_box(sample_or_exit(fixture));
        measured = measured.saturating_add(started.elapsed());
    }
    measured
}

fn measure_restore_and_sample(fixture: &mut SamplingFixture, iterations: u64) -> Duration {
    let mut measured = Duration::ZERO;
    for _ in 0..iterations {
        let started = Instant::now();
        fixture.restore_logits();
        black_box(sample_or_exit(fixture));
        measured = measured.saturating_add(started.elapsed());
    }
    measured
}

fn sample_or_exit(fixture: &mut SamplingFixture) -> Sample {
    match fixture.sample() {
        Ok(sample) => sample,
        Err(error) => benchmark_failure("sampling execution", error),
    }
}

fn benchmark_stop_matching(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("stop_matching");

    for case in STOP_CASES {
        let fixture = case.build();
        validate_stop_fixture(case.name(), case.expected(), &fixture);
        group.bench_function(
            BenchmarkId::new(case.name(), case.parameter()),
            |benchmark| benchmark.iter(|| black_box(fixture.match_suffix())),
        );
    }

    group.finish();
}

fn validate_stop_fixture(case_name: &str, expected: Option<(u32, usize)>, fixture: &StopFixture) {
    let observed = fixture
        .match_suffix()
        .map(|matched| (matched.code, matched.matched_tokens));
    if observed != expected {
        eprintln!("stop fixture {case_name} produced {observed:?}, expected {expected:?}");
        process::exit(BENCHMARK_FAILURE_EXIT_CODE);
    }
}

fn benchmark_failure(operation: &str, error: SamplingError) -> ! {
    eprintln!("{operation} failed: {error:?}");
    process::exit(BENCHMARK_FAILURE_EXIT_CODE);
}

criterion_group!(benches, benchmark_sampling_pipeline);
criterion_main!(benches);
