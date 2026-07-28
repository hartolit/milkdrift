//! Allocation-free token stop-sequence matching.

use domain_contracts::TokenId;

/// One configured token stop sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StopSequence<'a> {
    /// Stable caller-defined sequence code.
    pub code: u32,
    /// Token pattern. Empty patterns never match.
    pub tokens: &'a [TokenId],
}

/// Successful suffix match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StopMatch {
    /// Caller-defined sequence code.
    pub code: u32,
    /// Number of matched trailing tokens.
    pub matched_tokens: usize,
}

/// Returns the first configured pattern matching the generated-token suffix.
#[must_use]
pub fn match_stop_suffix(
    generated_tokens: &[TokenId],
    stop_sequences: &[StopSequence<'_>],
) -> Option<StopMatch> {
    for sequence in stop_sequences {
        if sequence.tokens.is_empty() || sequence.tokens.len() > generated_tokens.len() {
            continue;
        }
        let start = generated_tokens.len().saturating_sub(sequence.tokens.len());
        let Some(suffix) = generated_tokens.get(start..) else {
            continue;
        };
        if suffix == sequence.tokens {
            return Some(StopMatch {
                code: sequence.code,
                matched_tokens: sequence.tokens.len(),
            });
        }
    }
    None
}
