//! Checked sequence inputs and pre-allocated buffer wrappers.

use crate::TokenId;
use crate::capacity::{CapacityExhausted, CapacityResource};

/// Stable lifecycle state of one backend sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SequenceState {
    /// Sequence was created but has not accepted tokens.
    Empty,
    /// Sequence contains prompt or generated state and can continue decoding.
    Ready,
    /// Sequence has completed and requires reset or destruction.
    Finished,
    /// Sequence is being reset or synchronized.
    Transitioning,
}

/// Borrowed input for one prompt-prefill operation.
#[derive(Clone, Copy, Debug)]
pub struct PrefillInput<'a> {
    /// Flat prompt token slice.
    pub tokens: &'a [TokenId],
    /// Whether the caller requires final-position logits.
    pub emit_logits: bool,
}

impl<'a> PrefillInput<'a> {
    /// Creates a prefill input.
    #[must_use]
    pub const fn new(tokens: &'a [TokenId], emit_logits: bool) -> Self {
        Self {
            tokens,
            emit_logits,
        }
    }
}

/// Input token consumed by one incremental decode step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeInput {
    /// Token appended to the existing sequence state.
    pub token: TokenId,
}

impl DecodeInput {
    /// Creates a decode input.
    #[must_use]
    pub const fn new(token: TokenId) -> Self {
        Self { token }
    }
}

/// Required caller-owned buffers for prefill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrefillBufferRequirements {
    /// Minimum number of `f32` logits elements.
    pub logits: usize,
}

/// Required caller-owned buffers for one decode step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeBufferRequirements {
    /// Minimum number of `f32` logits elements.
    pub logits: usize,
}

/// Unvalidated caller-owned buffers for prefill.
pub struct PrefillBuffers<'a> {
    logits: &'a mut [f32],
}

impl<'a> PrefillBuffers<'a> {
    /// Creates an unvalidated prefill buffer view.
    #[must_use]
    pub const fn new(logits: &'a mut [f32]) -> Self {
        Self { logits }
    }

    /// Validates capacity and returns a wrapper suitable for backend entry.
    pub(crate) const fn prepare(
        self,
        requirements: PrefillBufferRequirements,
    ) -> Result<PreparedPrefillBuffers<'a>, CapacityExhausted> {
        if self.logits.len() < requirements.logits {
            return Err(CapacityExhausted::new(
                CapacityResource::Logits,
                requirements.logits as u64,
                self.logits.len() as u64,
            ));
        }

        Ok(PreparedPrefillBuffers {
            logits: self.logits,
            required_logits: requirements.logits,
        })
    }
}

/// Capacity-validated prefill buffers passed into a backend implementation.
pub struct PreparedPrefillBuffers<'a> {
    logits: &'a mut [f32],
    required_logits: usize,
}

impl PreparedPrefillBuffers<'_> {
    /// Returns the complete caller-provided logits storage.
    pub const fn logits_mut(&mut self) -> &mut [f32] {
        self.logits
    }

    /// Returns the number of logits elements required for this operation.
    #[must_use]
    pub const fn required_logits(&self) -> usize {
        self.required_logits
    }
}

/// Unvalidated caller-owned buffers for one decode step.
pub struct DecodeBuffers<'a> {
    logits: &'a mut [f32],
}

impl<'a> DecodeBuffers<'a> {
    /// Creates an unvalidated decode buffer view.
    #[must_use]
    pub const fn new(logits: &'a mut [f32]) -> Self {
        Self { logits }
    }

    /// Validates capacity and returns a wrapper suitable for backend entry.
    pub(crate) const fn prepare(
        self,
        requirements: DecodeBufferRequirements,
    ) -> Result<PreparedDecodeBuffers<'a>, CapacityExhausted> {
        if self.logits.len() < requirements.logits {
            return Err(CapacityExhausted::new(
                CapacityResource::Logits,
                requirements.logits as u64,
                self.logits.len() as u64,
            ));
        }

        Ok(PreparedDecodeBuffers {
            logits: self.logits,
            required_logits: requirements.logits,
        })
    }
}

/// Capacity-validated decode buffers passed into a backend implementation.
pub struct PreparedDecodeBuffers<'a> {
    logits: &'a mut [f32],
    required_logits: usize,
}

impl PreparedDecodeBuffers<'_> {
    /// Returns the complete caller-provided logits storage.
    pub const fn logits_mut(&mut self) -> &mut [f32] {
        self.logits
    }

    /// Returns the number of logits elements required for this operation.
    #[must_use]
    pub const fn required_logits(&self) -> usize {
        self.required_logits
    }
}
