//! Statistical component measurements for the public production sampling API.
//!
//! # Measurement contract
//!
//! Concrete sampler targets are the Cartesian product
//! `{sample_only,restore_and_sample}/<case>/{8192,32768,131072}`. Each target
//! uses a deterministic, bounded `[-8, 8]` pseudo-random logit fixture, one
//! seeded [`Sampler`], caller-owned vocabulary-sized logits/indices/seen-token
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
//! Stop matching executes the public production [`match_stop_suffix`] function
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

use std::hint::black_box;
use std::process;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use domain_contracts::TokenId;
use sampling::{
    Sample, Sampler, SamplingConfig, SamplingWorkspace, StopSequence, match_stop_suffix,
};

const VOCABULARY_SIZES: [(usize, u64); 3] = [(8_192, 8_192), (32_768, 32_768), (131_072, 131_072)];
const RANDOM_SEED: u64 = 29;
const MIN_P_THRESHOLD: f32 = 0.05;
const REPETITION_PENALTY: f32 = 1.1;
const SHORT_HISTORY_LENGTH: usize = 8;
const MEDIUM_HISTORY_LENGTH: usize = 64;
const HEAVY_HISTORY_LENGTH: usize = 256;
const REPEATED_UNIQUE_TOKENS: u32 = 4;
const LOGIT_STATE_SEED: u32 = 0xA341_316C;
const LOGIT_MULTIPLIER: u32 = 1_664_525;
const LOGIT_INCREMENT: u32 = 1_013_904_223;
const MINIMUM_LOGIT: f32 = -8.0;
const LOGIT_SCALE: f32 = 16.0 / 65_535.0;
const GENERATED_TOKEN_COUNT: usize = 128;
const GENERATED_PREFIX_TOKEN: u32 = 7;
const STOP_TOKEN: u32 = 17;
const BENCHMARK_FAILURE_EXIT_CODE: i32 = 2;
const STOP_TOKEN_PATTERN: [TokenId; 1] = [TokenId::new(STOP_TOKEN)];
const MISS_0: [TokenId; 1] = [TokenId::new(101)];
const MISS_1: [TokenId; 2] = [TokenId::new(102), TokenId::new(103)];
const MISS_2: [TokenId; 3] = [TokenId::new(104), TokenId::new(105), TokenId::new(106)];
const MISS_3: [TokenId; 4] = [
    TokenId::new(107),
    TokenId::new(108),
    TokenId::new(109),
    TokenId::new(110),
];
const MISS_4: [TokenId; 5] = [
    TokenId::new(111),
    TokenId::new(112),
    TokenId::new(113),
    TokenId::new(114),
    TokenId::new(115),
];
const MISS_5: [TokenId; 6] = [
    TokenId::new(116),
    TokenId::new(117),
    TokenId::new(118),
    TokenId::new(119),
    TokenId::new(120),
    TokenId::new(121),
];
const NEAR_MISS: [TokenId; 4] = [
    TokenId::new(201),
    TokenId::new(202),
    TokenId::new(203),
    TokenId::new(205),
];
const MATCHING_PATTERN: [TokenId; 4] = [
    TokenId::new(201),
    TokenId::new(202),
    TokenId::new(203),
    TokenId::new(204),
];
const FINAL_MISS: [TokenId; 4] = [
    TokenId::new(211),
    TokenId::new(212),
    TokenId::new(213),
    TokenId::new(214),
];
const TOKEN_STOP_SEQUENCES: [StopSequence<'static>; 1] = [StopSequence {
    code: 1,
    tokens: &STOP_TOKEN_PATTERN,
}];
const LATE_MATCH_STOP_SEQUENCES: [StopSequence<'static>; 8] = [
    StopSequence {
        code: 10,
        tokens: &MISS_0,
    },
    StopSequence {
        code: 11,
        tokens: &MISS_1,
    },
    StopSequence {
        code: 12,
        tokens: &MISS_2,
    },
    StopSequence {
        code: 13,
        tokens: &MISS_3,
    },
    StopSequence {
        code: 14,
        tokens: &MISS_4,
    },
    StopSequence {
        code: 15,
        tokens: &MISS_5,
    },
    StopSequence {
        code: 16,
        tokens: &NEAR_MISS,
    },
    StopSequence {
        code: 17,
        tokens: &MATCHING_PATTERN,
    },
];
const MISS_STOP_SEQUENCES: [StopSequence<'static>; 8] = [
    StopSequence {
        code: 20,
        tokens: &MISS_0,
    },
    StopSequence {
        code: 21,
        tokens: &MISS_1,
    },
    StopSequence {
        code: 22,
        tokens: &MISS_2,
    },
    StopSequence {
        code: 23,
        tokens: &MISS_3,
    },
    StopSequence {
        code: 24,
        tokens: &MISS_4,
    },
    StopSequence {
        code: 25,
        tokens: &MISS_5,
    },
    StopSequence {
        code: 26,
        tokens: &NEAR_MISS,
    },
    StopSequence {
        code: 27,
        tokens: &FINAL_MISS,
    },
];

#[derive(Clone, Copy)]
struct SamplingCase {
    name: &'static str,
    configuration: ConfigurationFixture,
    history: HistoryFixture,
}

#[derive(Clone, Copy)]
enum ConfigurationFixture {
    Greedy,
    Default,
    MinP,
    RepetitionDisabled,
    RepetitionEnabled,
}

#[derive(Clone, Copy)]
enum HistoryFixture {
    Empty,
    Unique(usize),
    RepeatedHeavy(usize),
}

#[derive(Clone, Copy)]
enum TimingBoundary {
    SampleOnly,
    RestoreAndSample,
}

const SAMPLING_CASES: [SamplingCase; 8] = [
    SamplingCase {
        name: "greedy",
        configuration: ConfigurationFixture::Greedy,
        history: HistoryFixture::Empty,
    },
    SamplingCase {
        name: "default_top_k_top_p",
        configuration: ConfigurationFixture::Default,
        history: HistoryFixture::Empty,
    },
    SamplingCase {
        name: "min_p_0_05_full_vocabulary",
        configuration: ConfigurationFixture::MinP,
        history: HistoryFixture::Empty,
    },
    SamplingCase {
        name: "repetition_disabled_history_256",
        configuration: ConfigurationFixture::RepetitionDisabled,
        history: HistoryFixture::RepeatedHeavy(HEAVY_HISTORY_LENGTH),
    },
    SamplingCase {
        name: "repetition_enabled_empty",
        configuration: ConfigurationFixture::RepetitionEnabled,
        history: HistoryFixture::Empty,
    },
    SamplingCase {
        name: "repetition_short_unique_8",
        configuration: ConfigurationFixture::RepetitionEnabled,
        history: HistoryFixture::Unique(SHORT_HISTORY_LENGTH),
    },
    SamplingCase {
        name: "repetition_medium_unique_64",
        configuration: ConfigurationFixture::RepetitionEnabled,
        history: HistoryFixture::Unique(MEDIUM_HISTORY_LENGTH),
    },
    SamplingCase {
        name: "repetition_repeated_heavy_256",
        configuration: ConfigurationFixture::RepetitionEnabled,
        history: HistoryFixture::RepeatedHeavy(HEAVY_HISTORY_LENGTH),
    },
];

struct SamplingFixture {
    baseline_logits: Vec<f32>,
    logits: Vec<f32>,
    history: Vec<TokenId>,
    indices: Vec<u32>,
    seen_tokens: Vec<u32>,
    sampler: Sampler,
}

impl ConfigurationFixture {
    fn build(self) -> SamplingConfig {
        match self {
            Self::Greedy => SamplingConfig::greedy(),
            Self::Default => SamplingConfig::default(),
            Self::MinP => SamplingConfig {
                top_k: 0,
                top_p: 1.0,
                min_p: MIN_P_THRESHOLD,
                ..SamplingConfig::default()
            },
            Self::RepetitionDisabled => SamplingConfig {
                repetition_penalty: 1.0,
                repetition_window: 0,
                ..SamplingConfig::default()
            },
            Self::RepetitionEnabled => SamplingConfig {
                repetition_penalty: REPETITION_PENALTY,
                repetition_window: 0,
                ..SamplingConfig::default()
            },
        }
    }
}

impl HistoryFixture {
    fn build(self, vocabulary_size: usize) -> Vec<TokenId> {
        match self {
            Self::Empty => Vec::new(),
            Self::Unique(length) => build_unique_history(vocabulary_size, length),
            Self::RepeatedHeavy(length) => build_repeated_heavy_history(vocabulary_size, length),
        }
    }
}

impl TimingBoundary {
    const fn benchmark_group(self) -> &'static str {
        match self {
            Self::SampleOnly => "sample_only",
            Self::RestoreAndSample => "restore_and_sample",
        }
    }
}

impl SamplingFixture {
    fn new(case: SamplingCase, vocabulary_size: usize) -> Self {
        let sampler = match Sampler::new(case.configuration.build(), RANDOM_SEED) {
            Ok(sampler) => sampler,
            Err(error) => benchmark_failure("sampling configuration", error),
        };

        Self {
            baseline_logits: build_logits(vocabulary_size),
            logits: vec![0.0; vocabulary_size],
            history: case.history.build(vocabulary_size),
            indices: vec![0_u32; vocabulary_size],
            seen_tokens: vec![0_u32; vocabulary_size],
            sampler,
        }
    }

    fn restore_logits(&mut self) {
        let baseline_logits = black_box(self.baseline_logits.as_slice());
        self.logits.copy_from_slice(baseline_logits);
    }

    fn sample(&mut self) -> Sample {
        let result = self.sampler.sample(
            &mut self.logits,
            black_box(self.history.as_slice()),
            SamplingWorkspace {
                indices: &mut self.indices,
                seen_tokens: &mut self.seen_tokens,
            },
        );
        match result {
            Ok(sample) => sample,
            Err(error) => benchmark_failure("sampling execution", error),
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
            let mut fixture = SamplingFixture::new(case, vocabulary_size);
            group.throughput(Throughput::Elements(vocabulary_elements));
            group.bench_function(BenchmarkId::new(case.name, vocabulary_size), |benchmark| {
                benchmark.iter_custom(|iterations| {
                    measure_sampling_iterations(&mut fixture, boundary, iterations)
                });
            });
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
        black_box(fixture.sample());
        measured = measured.saturating_add(started.elapsed());
    }
    measured
}

fn measure_restore_and_sample(fixture: &mut SamplingFixture, iterations: u64) -> Duration {
    let mut measured = Duration::ZERO;
    for _ in 0..iterations {
        let started = Instant::now();
        fixture.restore_logits();
        black_box(fixture.sample());
        measured = measured.saturating_add(started.elapsed());
    }
    measured
}

fn benchmark_stop_matching(criterion: &mut Criterion) {
    let generated_token_hit = vec![TokenId::new(STOP_TOKEN); GENERATED_TOKEN_COUNT];
    let generated_pattern = build_generated_tokens(&MATCHING_PATTERN);

    let mut group = criterion.benchmark_group("stop_matching");
    group.bench_function(
        BenchmarkId::new("token_hit", "1_pattern_generated_128"),
        |benchmark| {
            benchmark.iter(|| {
                black_box(match_stop_suffix(
                    black_box(generated_token_hit.as_slice()),
                    black_box(TOKEN_STOP_SEQUENCES.as_slice()),
                ))
            });
        },
    );
    group.bench_function(
        BenchmarkId::new("pattern_hit_last", "8_patterns_generated_128"),
        |benchmark| {
            benchmark.iter(|| {
                black_box(match_stop_suffix(
                    black_box(generated_pattern.as_slice()),
                    black_box(LATE_MATCH_STOP_SEQUENCES.as_slice()),
                ))
            });
        },
    );
    group.bench_function(
        BenchmarkId::new("pattern_miss", "8_patterns_generated_128"),
        |benchmark| {
            benchmark.iter(|| {
                black_box(match_stop_suffix(
                    black_box(generated_pattern.as_slice()),
                    black_box(MISS_STOP_SEQUENCES.as_slice()),
                ))
            });
        },
    );
    group.finish();
}

fn build_logits(vocabulary_size: usize) -> Vec<f32> {
    let mut state = LOGIT_STATE_SEED;
    let mut logits = Vec::with_capacity(vocabulary_size);
    for _ in 0..vocabulary_size {
        state = state
            .wrapping_mul(LOGIT_MULTIPLIER)
            .wrapping_add(LOGIT_INCREMENT);
        let [high, low, _, _] = state.to_be_bytes();
        let bucket = u16::from_be_bytes([high, low]);
        logits.push(f32::from(bucket).mul_add(LOGIT_SCALE, MINIMUM_LOGIT));
    }
    logits
}

fn build_unique_history(vocabulary_size: usize, length: usize) -> Vec<TokenId> {
    let mut token = maximum_token_id(vocabulary_size);
    let mut history = Vec::with_capacity(length);
    for _ in 0..length {
        history.push(TokenId::new(token));
        token = token.saturating_sub(1);
    }
    history
}

fn build_repeated_heavy_history(vocabulary_size: usize, length: usize) -> Vec<TokenId> {
    let maximum_token = maximum_token_id(vocabulary_size);
    let mut offset = 0_u32;
    let mut history = Vec::with_capacity(length);
    for _ in 0..length {
        history.push(TokenId::new(maximum_token.saturating_sub(offset)));
        offset += 1;
        if offset == REPEATED_UNIQUE_TOKENS {
            offset = 0;
        }
    }
    history
}

fn maximum_token_id(vocabulary_size: usize) -> u32 {
    let maximum_index = vocabulary_size.saturating_sub(1);
    match u32::try_from(maximum_index) {
        Ok(token) => token,
        Err(_) => benchmark_fixture_failure("history vocabulary conversion"),
    }
}

fn build_generated_tokens(suffix: &[TokenId]) -> Vec<TokenId> {
    let prefix_length = GENERATED_TOKEN_COUNT.saturating_sub(suffix.len());
    let mut generated = Vec::with_capacity(GENERATED_TOKEN_COUNT);
    generated.resize(prefix_length, TokenId::new(GENERATED_PREFIX_TOKEN));
    generated.extend_from_slice(suffix);
    generated
}

fn benchmark_failure(operation: &str, error: sampling::SamplingError) -> ! {
    eprintln!("{operation} failed: {error:?}");
    process::exit(BENCHMARK_FAILURE_EXIT_CODE);
}

fn benchmark_fixture_failure(operation: &str) -> ! {
    eprintln!("{operation} failed");
    process::exit(BENCHMARK_FAILURE_EXIT_CODE);
}

criterion_group!(benches, benchmark_sampling_pipeline);
criterion_main!(benches);
