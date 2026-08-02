//! Shared deterministic cases and fixtures for sampling benchmark execution.

use std::hint::black_box;

use domain_contracts::TokenId;
use sampling::{
    Sample, Sampler, SamplingConfig, SamplingError, SamplingWorkspace, StopMatch, StopSequence,
    match_stop_suffix,
};

pub(crate) const VOCABULARY_SIZES: [(usize, u64); 3] =
    [(8_192, 8_192), (32_768, 32_768), (131_072, 131_072)];

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
pub(crate) struct SamplingCase {
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

pub(crate) const SAMPLING_CASES: [SamplingCase; 8] = [
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

pub(crate) struct SamplingFixture {
    baseline_logits: Vec<f32>,
    logits: Vec<f32>,
    history: Vec<TokenId>,
    indices: Vec<u32>,
    seen_tokens: Vec<u32>,
    sampler: Sampler,
}

impl SamplingCase {
    pub(crate) const fn name(self) -> &'static str {
        self.name
    }
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
    fn build(self, vocabulary_size: usize) -> Result<Vec<TokenId>, SamplingError> {
        match self {
            Self::Empty => Ok(Vec::new()),
            Self::Unique(length) => build_unique_history(vocabulary_size, length),
            Self::RepeatedHeavy(length) => build_repeated_heavy_history(vocabulary_size, length),
        }
    }
}

impl SamplingFixture {
    pub(crate) fn new(case: SamplingCase, vocabulary_size: usize) -> Result<Self, SamplingError> {
        Ok(Self {
            baseline_logits: build_logits(vocabulary_size),
            logits: vec![0.0; vocabulary_size],
            history: case.history.build(vocabulary_size)?,
            indices: vec![0_u32; vocabulary_size],
            seen_tokens: vec![0_u32; vocabulary_size],
            sampler: Sampler::new(case.configuration.build(), RANDOM_SEED)?,
        })
    }

    pub(crate) fn restore_logits(&mut self) {
        let baseline_logits = black_box(self.baseline_logits.as_slice());
        self.logits.copy_from_slice(baseline_logits);
    }

    pub(crate) fn sample(&mut self) -> Result<Sample, SamplingError> {
        self.sampler.sample(
            &mut self.logits,
            black_box(self.history.as_slice()),
            SamplingWorkspace {
                indices: &mut self.indices,
                seen_tokens: &mut self.seen_tokens,
            },
        )
    }
}

#[derive(Clone, Copy)]
pub(crate) struct StopCase {
    name: &'static str,
    parameter: &'static str,
    fixture: StopFixtureKind,
    expected: Option<(u32, usize)>,
}

#[derive(Clone, Copy)]
enum StopFixtureKind {
    TokenHit,
    PatternHitLast,
    PatternMiss,
}

pub(crate) const STOP_CASES: [StopCase; 3] = [
    StopCase {
        name: "token_hit",
        parameter: "1_pattern_generated_128",
        fixture: StopFixtureKind::TokenHit,
        expected: Some((1, STOP_TOKEN_PATTERN.len())),
    },
    StopCase {
        name: "pattern_hit_last",
        parameter: "8_patterns_generated_128",
        fixture: StopFixtureKind::PatternHitLast,
        expected: Some((17, MATCHING_PATTERN.len())),
    },
    StopCase {
        name: "pattern_miss",
        parameter: "8_patterns_generated_128",
        fixture: StopFixtureKind::PatternMiss,
        expected: None,
    },
];

pub(crate) struct StopFixture {
    generated: Vec<TokenId>,
    sequences: &'static [StopSequence<'static>],
}

impl StopCase {
    pub(crate) const fn name(self) -> &'static str {
        self.name
    }

    pub(crate) const fn parameter(self) -> &'static str {
        self.parameter
    }

    pub(crate) const fn expected(self) -> Option<(u32, usize)> {
        self.expected
    }

    pub(crate) fn build(self) -> StopFixture {
        match self.fixture {
            StopFixtureKind::TokenHit => StopFixture {
                generated: vec![TokenId::new(STOP_TOKEN); GENERATED_TOKEN_COUNT],
                sequences: &TOKEN_STOP_SEQUENCES,
            },
            StopFixtureKind::PatternHitLast => StopFixture {
                generated: build_generated_tokens(&MATCHING_PATTERN),
                sequences: &LATE_MATCH_STOP_SEQUENCES,
            },
            StopFixtureKind::PatternMiss => StopFixture {
                generated: build_generated_tokens(&MATCHING_PATTERN),
                sequences: &MISS_STOP_SEQUENCES,
            },
        }
    }
}

impl StopFixture {
    pub(crate) fn match_suffix(&self) -> Option<StopMatch> {
        match_stop_suffix(
            black_box(self.generated.as_slice()),
            black_box(self.sequences),
        )
    }
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

fn build_unique_history(
    vocabulary_size: usize,
    length: usize,
) -> Result<Vec<TokenId>, SamplingError> {
    let mut token = maximum_token_id(vocabulary_size)?;
    let mut history = Vec::with_capacity(length);
    for _ in 0..length {
        history.push(TokenId::new(token));
        token = token.saturating_sub(1);
    }
    Ok(history)
}

fn build_repeated_heavy_history(
    vocabulary_size: usize,
    length: usize,
) -> Result<Vec<TokenId>, SamplingError> {
    let maximum_token = maximum_token_id(vocabulary_size)?;
    let mut offset = 0_u32;
    let mut history = Vec::with_capacity(length);
    for _ in 0..length {
        history.push(TokenId::new(maximum_token.saturating_sub(offset)));
        offset += 1;
        if offset == REPEATED_UNIQUE_TOKENS {
            offset = 0;
        }
    }
    Ok(history)
}

fn maximum_token_id(vocabulary_size: usize) -> Result<u32, SamplingError> {
    u32::try_from(vocabulary_size.saturating_sub(1))
        .map_err(|_| SamplingError::VocabularyTooLarge { vocabulary_size })
}

fn build_generated_tokens(suffix: &[TokenId]) -> Vec<TokenId> {
    let prefix_length = GENERATED_TOKEN_COUNT.saturating_sub(suffix.len());
    let mut generated = Vec::with_capacity(GENERATED_TOKEN_COUNT);
    generated.resize(prefix_length, TokenId::new(GENERATED_PREFIX_TOKEN));
    generated.extend_from_slice(suffix);
    generated
}
