use domain_contracts::TokenId;
use inference_runtime::SamplingConfig;

use crate::{ApplicationError, GenerationSettingsField};

/// Stable seed policy for one direct-completion request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GenerationSeed {
    /// Derive a deterministic seed from the application request identity.
    #[default]
    RequestId,
    /// Use one exact caller-provided seed.
    Fixed(u64),
}

/// Stable E1 settings translated into E0 sampling and generation contracts.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerationSettings {
    /// Maximum sampled continuation tokens.
    pub maximum_new_tokens: u32,
    /// Positive finite softmax temperature.
    pub temperature: f32,
    /// Maximum retained candidates after ordering. Zero retains all.
    pub top_k: u32,
    /// Cumulative probability threshold in `(0, 1]`.
    pub top_p: f32,
    /// Minimum probability relative to the highest-probability token in `[0, 1]`.
    pub min_p: f32,
    /// Positive finite repetition penalty. One disables the penalty.
    pub repetition_penalty: f32,
    /// Number of trailing tokens considered for repetition. Zero uses full history.
    pub repetition_window: u32,
    /// Deterministic seed policy.
    pub seed: GenerationSeed,
    /// Explicit token identifiers treated as end-of-sequence markers.
    pub eos_tokens: Vec<TokenId>,
    /// Textual stop suffixes encoded once before E0 admission.
    pub stop_sequences: Vec<String>,
}

impl Default for GenerationSettings {
    fn default() -> Self {
        Self {
            maximum_new_tokens: 128,
            temperature: 0.8,
            top_k: 40,
            top_p: 0.95,
            min_p: 0.0,
            repetition_penalty: 1.0,
            repetition_window: 64,
            seed: GenerationSeed::RequestId,
            eos_tokens: Vec::new(),
            stop_sequences: Vec::new(),
        }
    }
}

impl GenerationSettings {
    pub(super) fn validate(&self) -> Result<SamplingConfig, ApplicationError> {
        if self.maximum_new_tokens == 0 {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::MaximumNewTokens,
            ));
        }
        if !self.temperature.is_finite() || self.temperature <= 0.0 {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::Temperature,
            ));
        }
        if !self.top_p.is_finite() || self.top_p <= 0.0 || self.top_p > 1.0 {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::TopP,
            ));
        }
        if !self.min_p.is_finite() || self.min_p < 0.0 || self.min_p > 1.0 {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::MinP,
            ));
        }
        if !self.repetition_penalty.is_finite() || self.repetition_penalty <= 0.0 {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::RepetitionPenalty,
            ));
        }
        if self
            .stop_sequences
            .iter()
            .any(std::string::String::is_empty)
        {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::StopSequence,
            ));
        }
        Ok(SamplingConfig {
            temperature: self.temperature,
            top_k: self.top_k,
            top_p: self.top_p,
            min_p: self.min_p,
            repetition_penalty: self.repetition_penalty,
            repetition_window: self.repetition_window,
        })
    }
}
