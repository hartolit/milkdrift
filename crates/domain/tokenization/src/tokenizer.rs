//! Backend-neutral tokenizer contracts with statically dispatched sinks.

use domain_contracts::TokenId;

use crate::{ByteSink, TextSink, TokenSink, TokenizationError};

/// Policy controlling whether tokenizer-defined special tokens may be recognized.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpecialTokenPolicy {
    /// Treat all input as ordinary text.
    #[default]
    OrdinaryText,
    /// Recognize tokenizer-defined special-token spellings.
    Allow,
    /// Reject tokenizer-defined special-token spellings in ordinary input.
    Reject,
}

/// Encoding options supplied at a cold or coarse-grained boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncodeOptions {
    /// Special-token handling policy.
    pub special_tokens: SpecialTokenPolicy,
    /// Whether the tokenizer should add its beginning-of-sequence token.
    pub add_beginning_of_sequence: bool,
    /// Whether the tokenizer should add its end-of-sequence token.
    pub add_end_of_sequence: bool,
}

/// Incremental decoding options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Whether tokenizer-defined special tokens should be omitted from text output.
    pub skip_special_tokens: bool,
}

/// Successful encoding report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncodeReport {
    /// Number of tokens appended to the sink.
    pub tokens_written: usize,
}

/// Successful decoding report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeReport {
    /// Number of bytes appended to the sink.
    pub bytes_written: usize,
    /// Whether the token was recognized as a special token and omitted.
    pub skipped_special_token: bool,
}

/// Concrete tokenizer contract for coarse encoding and diagnostic token decoding.
///
/// Generic sink parameters intentionally force monomorphization only around
/// token or byte emission. Generation must use [`StreamingTokenizer`] because
/// many tokenizer decoders require surrounding token state.
pub trait Tokenizer {
    /// Returns the vocabulary size expected by this tokenizer.
    fn vocabulary_size(&self) -> u32;

    /// Encodes text into a caller-provided token sink.
    ///
    /// # Errors
    ///
    /// Returns an error if the input cannot be encoded, the sink cannot accept
    /// the output, or the tokenizer implementation fails.
    fn encode<S: TokenSink>(
        &self,
        text: &str,
        options: EncodeOptions,
        output: &mut S,
    ) -> Result<EncodeReport, TokenizationError>;

    /// Decodes one isolated token into a caller-provided byte sink.
    ///
    /// This method is suitable for diagnostics only. It cannot preserve decoder
    /// state such as byte fallback or whitespace decisions across tokens.
    ///
    /// # Errors
    ///
    /// Returns an error if the token is unknown, the sink cannot accept the
    /// decoded bytes, or the tokenizer implementation fails.
    fn decode_token<S: ByteSink>(
        &self,
        token: TokenId,
        options: DecodeOptions,
        output: &mut S,
    ) -> Result<DecodeReport, TokenizationError>;
}

/// Stateful decoder used for model-output streaming.
pub trait StreamingDecoder {
    /// Processes one token while retaining decoder state across calls.
    ///
    /// # Errors
    ///
    /// Returns an error if the token cannot be decoded, the sink cannot accept
    /// the decoded text, or the decoder implementation fails.
    fn step<S: TextSink>(
        &mut self,
        token: TokenId,
        output: &mut S,
    ) -> Result<DecodeReport, TokenizationError>;
}

/// Tokenizer capable of constructing a stateful decoder without dynamic dispatch.
pub trait StreamingTokenizer: Tokenizer {
    /// Concrete decoder borrowing this tokenizer.
    type Decoder<'tokenizer>: StreamingDecoder
    where
        Self: 'tokenizer;

    /// Creates a fresh request-local decoder.
    fn decoder(&self, options: DecodeOptions) -> Self::Decoder<'_>;
}
