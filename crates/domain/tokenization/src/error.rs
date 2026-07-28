//! Stable tokenization failures without owned diagnostic strings.

use domain_contracts::{CapacityExhausted, TokenId};

/// Failure produced while encoding, decoding, or writing tokenization output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenizationError {
    /// A caller-owned output buffer was too small.
    CapacityExhausted(CapacityExhausted),
    /// A token does not belong to the active vocabulary.
    UnknownToken(TokenId),
    /// A byte sequence was not valid UTF-8.
    InvalidUtf8 {
        /// Byte that made the current sequence invalid.
        byte: u8,
    },
    /// Incremental decoding ended with an incomplete UTF-8 sequence.
    IncompleteUtf8 {
        /// Number of bytes retained in the decoder.
        buffered_bytes: u8,
    },
    /// Tokenizer-specific failure represented by a stable numeric code.
    Implementation {
        /// Adapter-defined error code.
        code: u32,
    },
}

impl From<CapacityExhausted> for TokenizationError {
    fn from(value: CapacityExhausted) -> Self {
        Self::CapacityExhausted(value)
    }
}
