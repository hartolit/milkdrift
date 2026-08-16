use std::num::NonZeroU32;

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

/// Stable E1 settings translated into E0 generation contracts.
///
/// Construction proves the model-independent token and sampling invariants.
/// Model-dependent EOS validation and stop-sequence encoding remain part of E1
/// admission because they require the loaded model and tokenizer.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerationSettings {
    maximum_new_tokens: NonZeroU32,
    sampling: SamplingConfig,
    seed: GenerationSeed,
    eos_tokens: Vec<TokenId>,
    stop_sequences: Vec<String>,
}

impl Default for GenerationSettings {
    fn default() -> Self {
        Self {
            maximum_new_tokens: NonZeroU32::new(512).unwrap_or(NonZeroU32::MIN),
            sampling: SamplingConfig::default(),
            seed: GenerationSeed::RequestId,
            eos_tokens: Vec::new(),
            stop_sequences: Vec::new(),
        }
    }
}

impl GenerationSettings {
    /// Creates settings with no explicit EOS tokens or textual stops.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::InvalidGenerationSettings`] when
    /// `maximum_new_tokens` is zero.
    pub fn new(
        maximum_new_tokens: u32,
        sampling: SamplingConfig,
    ) -> Result<Self, ApplicationError> {
        let maximum_new_tokens = NonZeroU32::new(maximum_new_tokens).ok_or(
            ApplicationError::InvalidGenerationSettings(GenerationSettingsField::MaximumNewTokens),
        )?;
        Ok(Self {
            maximum_new_tokens,
            sampling,
            seed: GenerationSeed::RequestId,
            eos_tokens: Vec::new(),
            stop_sequences: Vec::new(),
        })
    }

    /// Returns a copy using the requested deterministic seed policy.
    #[must_use]
    pub fn with_seed(mut self, seed: GenerationSeed) -> Self {
        self.seed = seed;
        self
    }

    /// Returns a copy using the supplied model-dependent EOS token identifiers.
    #[must_use]
    pub fn with_eos_tokens(mut self, eos_tokens: Vec<TokenId>) -> Self {
        self.eos_tokens = eos_tokens;
        self
    }

    /// Returns a copy using validated non-empty textual stop sequences.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::InvalidGenerationSettings`] when any stop is
    /// empty. Tokenization-dependent validation remains part of admission.
    pub fn with_stop_sequences(
        mut self,
        stop_sequences: Vec<String>,
    ) -> Result<Self, ApplicationError> {
        if stop_sequences.iter().any(String::is_empty) {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::StopSequence,
            ));
        }
        self.stop_sequences = stop_sequences;
        Ok(self)
    }

    /// Returns the non-zero continuation-token limit.
    #[must_use]
    pub const fn maximum_new_tokens(&self) -> u32 {
        self.maximum_new_tokens.get()
    }

    /// Returns the validated lower sampling policy.
    #[must_use]
    pub const fn sampling(&self) -> SamplingConfig {
        self.sampling
    }

    /// Returns the deterministic seed policy.
    #[must_use]
    pub const fn seed(&self) -> GenerationSeed {
        self.seed
    }

    /// Returns explicit model-dependent EOS token identifiers.
    #[must_use]
    pub fn eos_tokens(&self) -> &[TokenId] {
        self.eos_tokens.as_slice()
    }

    /// Returns textual stop suffixes.
    #[must_use]
    pub fn stop_sequences(&self) -> &[String] {
        self.stop_sequences.as_slice()
    }

    pub(crate) fn apply_chat_termination(&mut self, eos_token: TokenId) {
        self.eos_tokens.clear();
        self.eos_tokens.push(eos_token);
        self.stop_sequences.clear();
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        NonZeroU32,
        SamplingConfig,
        GenerationSeed,
        Vec<TokenId>,
        Vec<String>,
    ) {
        (
            self.maximum_new_tokens,
            self.sampling,
            self.seed,
            self.eos_tokens,
            self.stop_sequences,
        )
    }
}
