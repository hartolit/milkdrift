#![no_std]
#![forbid(unsafe_code)]
#![doc = "Deterministic zero-allocation sampling over pre-allocated flat slices."]

mod random;
mod stop;

use core::cmp::Ordering;

use domain_contracts::{CapacityExhausted, CapacityResource, TokenId};

pub use random::SplitMix64;
pub use stop::{StopMatch, StopSequence, match_stop_suffix};

/// Immutable sampler configuration validated before generation begins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SamplingConfig {
    /// Positive finite softmax temperature.
    pub temperature: f32,
    /// Maximum candidates retained after logit ordering. Zero retains all.
    pub top_k: u32,
    /// Cumulative probability threshold in `(0, 1]`.
    pub top_p: f32,
    /// Minimum probability relative to the highest-probability token in `[0, 1]`.
    pub min_p: f32,
    /// Positive finite repetition penalty. One disables the penalty.
    pub repetition_penalty: f32,
    /// Number of trailing tokens considered for repetition. Zero uses full history.
    pub repetition_window: u32,
}

impl SamplingConfig {
    /// Returns deterministic highest-logit selection.
    #[must_use]
    pub const fn greedy() -> Self {
        Self {
            temperature: 1.0,
            top_k: 1,
            top_p: 1.0,
            min_p: 0.0,
            repetition_penalty: 1.0,
            repetition_window: 0,
        }
    }

    /// Validates all numeric bounds.
    ///
    /// # Errors
    ///
    /// Returns [`SamplingError::InvalidConfiguration`] when a numeric field is
    /// non-finite or outside its documented bounds.
    pub fn validate(self) -> Result<Self, SamplingError> {
        if !self.temperature.is_finite() || self.temperature <= 0.0 {
            return Err(SamplingError::InvalidConfiguration(
                SamplingConfigurationField::Temperature,
            ));
        }
        if !self.top_p.is_finite() || self.top_p <= 0.0 || self.top_p > 1.0 {
            return Err(SamplingError::InvalidConfiguration(
                SamplingConfigurationField::TopP,
            ));
        }
        if !self.min_p.is_finite() || self.min_p < 0.0 || self.min_p > 1.0 {
            return Err(SamplingError::InvalidConfiguration(
                SamplingConfigurationField::MinP,
            ));
        }
        if !self.repetition_penalty.is_finite() || self.repetition_penalty <= 0.0 {
            return Err(SamplingError::InvalidConfiguration(
                SamplingConfigurationField::RepetitionPenalty,
            ));
        }
        Ok(self)
    }
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            temperature: 0.8,
            top_k: 40,
            top_p: 0.95,
            min_p: 0.0,
            repetition_penalty: 1.0,
            repetition_window: 64,
        }
    }
}

/// Configuration field that failed validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplingConfigurationField {
    /// Temperature was non-finite or non-positive.
    Temperature,
    /// Top-p was outside `(0, 1]`.
    TopP,
    /// Min-p was outside `[0, 1]`.
    MinP,
    /// Repetition penalty was non-finite or non-positive.
    RepetitionPenalty,
}

/// Stable sampling failure.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum SamplingError {
    /// One configuration field is invalid.
    InvalidConfiguration(SamplingConfigurationField),
    /// Logit input was empty.
    EmptyLogits,
    /// No candidate had usable finite or positive-infinite weight.
    NoCandidate,
    /// A history token is outside the current vocabulary.
    HistoryTokenOutOfRange {
        /// Invalid token.
        token: TokenId,
        /// Current vocabulary size.
        vocabulary_size: usize,
    },
    /// The vocabulary cannot be represented by `TokenId`.
    VocabularyTooLarge {
        /// Number of supplied logits.
        vocabulary_size: usize,
    },
    /// Caller-owned sampling workspace is too small.
    CapacityExhausted(CapacityExhausted),
}

impl From<CapacityExhausted> for SamplingError {
    fn from(value: CapacityExhausted) -> Self {
        Self::CapacityExhausted(value)
    }
}

/// Caller-owned scratch required for one sampling pass.
pub struct SamplingWorkspace<'a> {
    /// Vocabulary-sized token-index storage.
    pub indices: &'a mut [u32],
    /// Vocabulary-sized epoch table used by repetition processing.
    pub seen_tokens: &'a mut [u32],
}

/// Result of one successful sampling pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Selected token.
    pub token: TokenId,
    /// Probability after top-k, min-p, and top-p filtering.
    pub probability: f32,
}

/// Concrete sampler whose policy does not add generic axes to the generation loop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampler {
    configuration: SamplingConfig,
    random: SplitMix64,
    repetition_epoch: u32,
}

impl Sampler {
    /// Creates a sampler after validating its immutable configuration.
    ///
    /// # Errors
    ///
    /// Returns [`SamplingError::InvalidConfiguration`] when `configuration`
    /// contains a non-finite value or a value outside its documented bounds.
    pub fn new(configuration: SamplingConfig, seed: u64) -> Result<Self, SamplingError> {
        Ok(Self {
            configuration: configuration.validate()?,
            random: SplitMix64::new(seed),
            repetition_epoch: 0,
        })
    }

    /// Returns the active immutable configuration.
    #[must_use]
    pub const fn configuration(&self) -> SamplingConfig {
        self.configuration
    }

    /// Samples one token while reusing all supplied storage.
    ///
    /// The logit slice is working memory: repetition penalties and normalized
    /// candidate weights overwrite its original values.
    ///
    /// # Errors
    ///
    /// Returns [`SamplingError`] when the inputs or workspace are invalid, no
    /// usable candidate exists, or the vocabulary exceeds supported limits.
    pub fn sample(
        &mut self,
        logits: &mut [f32],
        repetition_history: &[TokenId],
        workspace: SamplingWorkspace<'_>,
    ) -> Result<Sample, SamplingError> {
        if logits.is_empty() {
            return Err(SamplingError::EmptyLogits);
        }
        if logits.len() > u32::MAX as usize {
            return Err(SamplingError::VocabularyTooLarge {
                vocabulary_size: logits.len(),
            });
        }
        validate_workspace(logits.len(), &workspace)?;

        let SamplingWorkspace {
            indices,
            seen_tokens,
        } = workspace;
        let vocabulary_size = logits.len();
        let index_capacity_available = indices.len();
        let mask_capacity_available = seen_tokens.len();
        let Some(index_working_set) = indices.get_mut(..vocabulary_size) else {
            return Err(index_capacity(vocabulary_size, index_capacity_available));
        };
        let Some(seen_working_set) = seen_tokens.get_mut(..vocabulary_size) else {
            return Err(mask_capacity(vocabulary_size, mask_capacity_available));
        };

        if !is_exactly_one(self.configuration.repetition_penalty) {
            self.repetition_epoch = self.repetition_epoch.wrapping_add(1);
            if self.repetition_epoch == 0 {
                seen_working_set.fill(0);
                self.repetition_epoch = 1;
            }
            apply_repetition_penalty(
                logits,
                repetition_history,
                seen_working_set,
                self.repetition_epoch,
                self.configuration.repetition_penalty,
                self.configuration.repetition_window,
            )?;
        }

        for (index, slot) in index_working_set.iter_mut().enumerate() {
            *slot = u32::try_from(index).map_err(|_| SamplingError::VocabularyTooLarge {
                vocabulary_size: logits.len(),
            })?;
        }
        for logit in logits.iter_mut() {
            if logit.is_nan() {
                *logit = f32::NEG_INFINITY;
            }
        }

        let candidate_count = if self.configuration.top_k == 0 {
            logits.len()
        } else {
            logits.len().min(self.configuration.top_k as usize)
        };
        {
            let immutable_logits: &[f32] = logits;
            if candidate_count < index_working_set.len() {
                let partition_index = candidate_count.saturating_sub(1);
                index_working_set.select_nth_unstable_by(partition_index, |left, right| {
                    compare_logits(immutable_logits, *left, *right)
                });
            }
            let index_working_capacity = index_working_set.len();
            let Some(selected_indices) = index_working_set.get_mut(..candidate_count) else {
                return Err(index_capacity(candidate_count, index_working_capacity));
            };
            selected_indices
                .sort_unstable_by(|left, right| compare_logits(immutable_logits, *left, *right));
        }

        let Some(candidates) = index_working_set.get(..candidate_count) else {
            return Err(index_capacity(candidate_count, index_working_set.len()));
        };

        let first_token = token_at(candidates, 0)?;
        let first_index = first_token.get() as usize;
        let maximum_logit = *logits.get(first_index).ok_or(SamplingError::NoCandidate)?;

        if maximum_logit == f32::INFINITY {
            return self.sample_positive_infinities(candidates, logits);
        }
        if !maximum_logit.is_finite() {
            return Err(SamplingError::NoCandidate);
        }

        self.sample_finite(candidates, logits, maximum_logit)
    }

    fn sample_finite(
        &mut self,
        candidates: &[u32],
        logits: &mut [f32],
        maximum_logit: f32,
    ) -> Result<Sample, SamplingError> {
        for &candidate in candidates {
            let index = candidate as usize;
            let Some(logit) = logits.get_mut(index) else {
                return Err(SamplingError::NoCandidate);
            };
            let scaled = (*logit - maximum_logit) / self.configuration.temperature;
            *logit = libm::expf(scaled);
        }

        let mut eligible_count = 0_usize;
        let mut eligible_weight = 0.0_f32;
        for &candidate in candidates {
            let index = candidate as usize;
            let Some(&weight) = logits.get(index) else {
                return Err(SamplingError::NoCandidate);
            };
            if !weight.is_finite() || weight <= 0.0 || weight < self.configuration.min_p {
                break;
            }
            eligible_count += 1;
            eligible_weight += weight;
        }
        if eligible_count == 0 || !eligible_weight.is_finite() || eligible_weight <= 0.0 {
            return Err(SamplingError::NoCandidate);
        }

        let mut allowed_count = 0_usize;
        let mut allowed_weight = 0.0_f32;
        let Some(eligible) = candidates.get(..eligible_count) else {
            return Err(index_capacity(eligible_count, candidates.len()));
        };
        for &candidate in eligible {
            let index = candidate as usize;
            let Some(&weight) = logits.get(index) else {
                return Err(SamplingError::NoCandidate);
            };
            allowed_count += 1;
            allowed_weight += weight;
            if allowed_weight / eligible_weight >= self.configuration.top_p {
                break;
            }
        }
        if allowed_count == 0 || !allowed_weight.is_finite() || allowed_weight <= 0.0 {
            return Err(SamplingError::NoCandidate);
        }

        let target = self.random.next_unit_f32() * allowed_weight;
        let Some(allowed) = eligible.get(..allowed_count) else {
            return Err(index_capacity(allowed_count, eligible.len()));
        };
        let mut cumulative = 0.0_f32;
        let mut selected = token_at(allowed, allowed_count.saturating_sub(1))?;
        for &candidate in allowed {
            let index = candidate as usize;
            let Some(&weight) = logits.get(index) else {
                return Err(SamplingError::NoCandidate);
            };
            cumulative += weight;
            if target < cumulative {
                selected = TokenId::new(candidate);
                break;
            }
        }

        let selected_index = selected.get() as usize;
        let selected_weight = *logits
            .get(selected_index)
            .ok_or(SamplingError::NoCandidate)?;
        Ok(Sample {
            token: selected,
            probability: selected_weight / allowed_weight,
        })
    }

    fn sample_positive_infinities(
        &mut self,
        candidates: &[u32],
        logits: &[f32],
    ) -> Result<Sample, SamplingError> {
        let mut count = 0_usize;
        for &candidate in candidates {
            let Some(&value) = logits.get(candidate as usize) else {
                return Err(SamplingError::NoCandidate);
            };
            if value == f32::INFINITY {
                count += 1;
            } else {
                break;
            }
        }
        if count == 0 {
            return Err(SamplingError::NoCandidate);
        }
        let allowed_count = uniform_top_p_count(count, self.configuration.top_p);
        let random_index = self.random.next_u64() % allowed_count as u64;
        let offset = usize::try_from(random_index).map_err(|_| SamplingError::NoCandidate)?;
        let token = token_at(candidates, offset)?;
        Ok(Sample {
            token,
            probability: 1.0 / count_as_f32(allowed_count),
        })
    }
}

const fn is_exactly_one(value: f32) -> bool {
    value.to_bits() == 1.0_f32.to_bits()
}

/// Converts a candidate count into the sampler's `f32` probability domain.
///
/// Counts above `f32`'s exact integer range are intentionally rounded because
/// the configured thresholds and returned probabilities are themselves `f32`.
#[allow(clippy::cast_precision_loss)]
const fn count_as_f32(count: usize) -> f32 {
    count as f32
}

fn uniform_top_p_count(candidate_count: usize, top_p: f32) -> usize {
    if candidate_count <= 1 {
        return candidate_count;
    }

    let candidate_count_f32 = count_as_f32(candidate_count);
    let mut allowed_count = 1_usize;

    while allowed_count < candidate_count
        && count_as_f32(allowed_count) / candidate_count_f32 < top_p
    {
        allowed_count += 1;
    }

    allowed_count
}

const fn validate_workspace(
    required: usize,
    workspace: &SamplingWorkspace<'_>,
) -> Result<(), SamplingError> {
    if workspace.indices.len() < required {
        return Err(index_capacity(required, workspace.indices.len()));
    }
    if workspace.seen_tokens.len() < required {
        return Err(mask_capacity(required, workspace.seen_tokens.len()));
    }
    Ok(())
}

fn apply_repetition_penalty(
    logits: &mut [f32],
    history: &[TokenId],
    seen_tokens: &mut [u32],
    epoch: u32,
    penalty: f32,
    repetition_window: u32,
) -> Result<(), SamplingError> {
    if is_exactly_one(penalty) {
        return Ok(());
    }

    let history_start = if repetition_window == 0 {
        0
    } else {
        history.len().saturating_sub(repetition_window as usize)
    };
    let Some(recent_history) = history.get(history_start..) else {
        return Ok(());
    };

    for &token in recent_history {
        let index = token.get() as usize;
        if index >= logits.len() {
            return Err(SamplingError::HistoryTokenOutOfRange {
                token,
                vocabulary_size: logits.len(),
            });
        }
        let Some(seen) = seen_tokens.get_mut(index) else {
            return Err(mask_capacity(logits.len(), seen_tokens.len()));
        };
        if *seen == epoch {
            continue;
        }
        *seen = epoch;
        let Some(logit) = logits.get_mut(index) else {
            return Err(SamplingError::HistoryTokenOutOfRange {
                token,
                vocabulary_size: logits.len(),
            });
        };
        if *logit >= 0.0 {
            *logit /= penalty;
        } else {
            *logit *= penalty;
        }
    }
    Ok(())
}

fn compare_logits(logits: &[f32], left: u32, right: u32) -> Ordering {
    match (logits.get(left as usize), logits.get(right as usize)) {
        (Some(left_value), Some(right_value)) => right_value
            .total_cmp(left_value)
            .then_with(|| left.cmp(&right)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.cmp(&right),
    }
}

fn token_at(candidates: &[u32], position: usize) -> Result<TokenId, SamplingError> {
    candidates
        .get(position)
        .copied()
        .map(TokenId::new)
        .ok_or(SamplingError::NoCandidate)
}

const fn index_capacity(required: usize, available: usize) -> SamplingError {
    SamplingError::CapacityExhausted(CapacityExhausted::new(
        CapacityResource::SamplingIndices,
        required as u64,
        available as u64,
    ))
}

const fn mask_capacity(required: usize, available: usize) -> SamplingError {
    SamplingError::CapacityExhausted(CapacityExhausted::new(
        CapacityResource::SamplingMask,
        required as u64,
        available as u64,
    ))
}
