//! Hugging Face Tokenizers adapter with stateful output decoding.

#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::Path;

use domain_contracts::{CapacityExhausted, CapacityResource, TokenId};
use tokenization::{
    ByteSink, DecodeOptions, DecodeReport, EncodeOptions, EncodeReport, SpecialTokenPolicy,
    StreamingDecoder, StreamingTokenizer, TextSink, TokenSink, TokenizationError, Tokenizer,
};
use tokenizers::tokenizer::{
    DecodeStream, DecoderWrapper, ModelWrapper, NormalizerWrapper, PostProcessorWrapper,
    PreTokenizerWrapper,
};

const ERROR_ENCODE: u32 = 1;
const ERROR_DECODE: u32 = 2;
const ERROR_ASYMMETRIC_BOUNDARY_TOKENS: u32 = 3;
const ERROR_REJECTED_SPECIAL_TOKEN: u32 = 4;

/// Failure while constructing the adapter.
#[derive(Debug)]
pub enum HfTokenizerLoadError {
    /// The tokenizer JSON could not be parsed or opened.
    InvalidTokenizer(tokenizers::Error),
    /// The upstream vocabulary does not fit the stable domain representation.
    VocabularyOverflow {
        /// Number of entries reported by the upstream tokenizer.
        vocabulary_size: usize,
    },
}

impl Display for HfTokenizerLoadError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTokenizer(error) => {
                write!(formatter, "failed to load tokenizer: {error}")
            }
            Self::VocabularyOverflow { vocabulary_size } => write!(
                formatter,
                "tokenizer vocabulary of {vocabulary_size} entries exceeds u32"
            ),
        }
    }
}

impl Error for HfTokenizerLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidTokenizer(error) => Some(error.as_ref()),
            Self::VocabularyOverflow { .. } => None,
        }
    }
}

/// Hugging Face tokenizer behind the workspace's allocation-neutral contracts.
///
/// Upstream encoding and streaming decode allocate internally. The adapter keeps
/// those allocations quarantined and validates caller-owned sink capacity before
/// committing output. It does not claim allocation-free execution.
#[derive(Clone)]
pub struct HfTokenizer {
    ordinary: tokenizers::Tokenizer,
    special: tokenizers::Tokenizer,
    vocabulary_size: u32,
}

impl HfTokenizer {
    /// Loads a serialized `tokenizer.json` file.
    ///
    /// # Errors
    ///
    /// Returns [`HfTokenizerLoadError::InvalidTokenizer`] when the file cannot be
    /// opened or parsed, or [`HfTokenizerLoadError::VocabularyOverflow`] when its
    /// vocabulary size cannot be represented as a `u32`.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, HfTokenizerLoadError> {
        let inner = tokenizers::Tokenizer::from_file(path)
            .map_err(HfTokenizerLoadError::InvalidTokenizer)?;
        Self::from_tokenizer(inner)
    }

    /// Loads a serialized `tokenizer.json` from the supplied exact bytes.
    ///
    /// # Errors
    ///
    /// Returns [`HfTokenizerLoadError::InvalidTokenizer`] when the bytes cannot be parsed, or
    /// [`HfTokenizerLoadError::VocabularyOverflow`] when the vocabulary size cannot be
    /// represented as a `u32`.
    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> Result<Self, HfTokenizerLoadError> {
        let inner = tokenizers::Tokenizer::from_bytes(bytes)
            .map_err(HfTokenizerLoadError::InvalidTokenizer)?;
        Self::from_tokenizer(inner)
    }

    /// Wraps an already constructed Hugging Face tokenizer.
    ///
    /// # Errors
    ///
    /// Returns [`HfTokenizerLoadError::VocabularyOverflow`] when the tokenizer's
    /// vocabulary size cannot be represented as a `u32`.
    pub fn from_tokenizer(inner: tokenizers::Tokenizer) -> Result<Self, HfTokenizerLoadError> {
        let raw_size = inner.get_vocab_size(true);
        let vocabulary_size =
            u32::try_from(raw_size).map_err(|_| HfTokenizerLoadError::VocabularyOverflow {
                vocabulary_size: raw_size,
            })?;
        let mut ordinary = inner;
        ordinary.set_encode_special_tokens(false);
        let mut special = ordinary.clone();
        special.set_encode_special_tokens(true);
        Ok(Self {
            ordinary,
            special,
            vocabulary_size,
        })
    }

    /// Resolves one tokenizer vocabulary spelling to a project-owned token identifier.
    #[must_use]
    pub fn token_id(&self, token: &str) -> Option<TokenId> {
        self.ordinary.token_to_id(token).map(TokenId::new)
    }

    /// Creates request-local owned streaming decode state.
    ///
    /// The owned session clones the tokenizer once at request construction and
    /// keeps only the bounded suffix required to preserve upstream streaming
    /// semantics. It does not repeatedly decode the complete generated history.
    #[must_use]
    pub fn owned_decoder(&self, options: DecodeOptions) -> HfOwnedStreamingDecoder {
        HfOwnedStreamingDecoder {
            tokenizer: self.ordinary.clone(),
            skip_special_tokens: options.skip_special_tokens,
            ids: Vec::new(),
            prefix: String::new(),
            prefix_index: 0,
        }
    }

    fn reject_special_spellings(&self, text: &str) -> Result<(), TokenizationError> {
        let contains_special = self
            .ordinary
            .get_added_tokens_decoder()
            .values()
            .any(|token| token.special && text.contains(token.content.as_str()));
        if contains_special {
            return Err(TokenizationError::Implementation {
                code: ERROR_REJECTED_SPECIAL_TOKEN,
            });
        }
        Ok(())
    }
}

impl Tokenizer for HfTokenizer {
    fn vocabulary_size(&self) -> u32 {
        self.vocabulary_size
    }

    fn encode<S: TokenSink>(
        &self,
        text: &str,
        options: EncodeOptions,
        output: &mut S,
    ) -> Result<EncodeReport, TokenizationError> {
        if options.add_beginning_of_sequence != options.add_end_of_sequence {
            return Err(TokenizationError::Implementation {
                code: ERROR_ASYMMETRIC_BOUNDARY_TOKENS,
            });
        }
        if matches!(options.special_tokens, SpecialTokenPolicy::Reject) {
            self.reject_special_spellings(text)?;
        }

        let add_special_tokens = options.add_beginning_of_sequence;
        let tokenizer = match options.special_tokens {
            SpecialTokenPolicy::Allow => &self.special,
            SpecialTokenPolicy::OrdinaryText | SpecialTokenPolicy::Reject => &self.ordinary,
        };
        let encoding = tokenizer
            .encode(text, add_special_tokens)
            .map_err(|_| TokenizationError::Implementation { code: ERROR_ENCODE })?;
        let identifiers = encoding.get_ids();
        if identifiers.len() > output.remaining_capacity() {
            return Err(CapacityExhausted::new(
                CapacityResource::Tokens,
                usize_to_u64(identifiers.len()),
                usize_to_u64(output.remaining_capacity()),
            )
            .into());
        }
        for &identifier in identifiers {
            output.push_token(TokenId::new(identifier))?;
        }
        Ok(EncodeReport {
            tokens_written: identifiers.len(),
        })
    }

    fn decode_token<S: ByteSink>(
        &self,
        token: TokenId,
        options: DecodeOptions,
        output: &mut S,
    ) -> Result<DecodeReport, TokenizationError> {
        let identifier = token.get();
        let decoded = self
            .ordinary
            .decode(&[identifier], options.skip_special_tokens)
            .map_err(|_| TokenizationError::Implementation { code: ERROR_DECODE })?;
        output.push_bytes(decoded.as_bytes())?;
        Ok(DecodeReport {
            bytes_written: decoded.len(),
            skipped_special_token: decoded.is_empty(),
        })
    }
}

/// Request-local stateful decoder borrowing the adapter tokenizer.
pub struct HfStreamingDecoder<'tokenizer> {
    inner: DecodeStream<
        'tokenizer,
        ModelWrapper,
        NormalizerWrapper,
        PreTokenizerWrapper,
        PostProcessorWrapper,
        DecoderWrapper,
    >,
}

impl StreamingTokenizer for HfTokenizer {
    type Decoder<'tokenizer>
        = HfStreamingDecoder<'tokenizer>
    where
        Self: 'tokenizer;

    fn decoder(&self, options: DecodeOptions) -> Self::Decoder<'_> {
        HfStreamingDecoder {
            inner: self.ordinary.decode_stream(options.skip_special_tokens),
        }
    }
}

impl StreamingDecoder for HfStreamingDecoder<'_> {
    fn step<S: TextSink>(
        &mut self,
        token: TokenId,
        output: &mut S,
    ) -> Result<DecodeReport, TokenizationError> {
        let identifier = token.get();
        let Some(fragment) = self
            .inner
            .step(identifier)
            .map_err(|_| TokenizationError::Implementation { code: ERROR_DECODE })?
        else {
            return Ok(DecodeReport::default());
        };
        write_fragment(fragment.as_str(), output)
    }
}

/// Request-local streaming decoder that owns its tokenizer state.
///
/// This mirrors the upstream `DecodeStream` state machine while owning the
/// tokenizer clone, avoiding a self-referential application object. The retained
/// identifier suffix is drained whenever a stable prefix is emitted.
pub struct HfOwnedStreamingDecoder {
    tokenizer: tokenizers::Tokenizer,
    skip_special_tokens: bool,
    ids: Vec<u32>,
    prefix: String,
    prefix_index: usize,
}

impl StreamingDecoder for HfOwnedStreamingDecoder {
    fn step<S: TextSink>(
        &mut self,
        token: TokenId,
        output: &mut S,
    ) -> Result<DecodeReport, TokenizationError> {
        if self.prefix.is_empty() && !self.ids.is_empty() {
            let new_prefix = self.decode_ids()?;
            if !new_prefix.ends_with('�') {
                self.prefix = new_prefix;
                self.prefix_index = self.ids.len();
            }
        }

        self.ids.push(token.get());
        let decoded = self.decode_ids()?;
        if decoded.len() <= self.prefix.len() || decoded.ends_with('�') {
            return Ok(DecodeReport::default());
        }
        let Some(fragment) = decoded.strip_prefix(self.prefix.as_str()) else {
            return Err(TokenizationError::Implementation { code: ERROR_DECODE });
        };
        let fragment = fragment.to_owned();
        let retained_index = self.prefix_index.min(self.ids.len());
        let new_prefix_index = self.ids.len().saturating_sub(retained_index);
        let retained = self.ids.drain(retained_index..).collect();
        self.ids = retained;
        self.prefix = self.decode_ids()?;
        self.prefix_index = new_prefix_index;
        write_fragment(fragment.as_str(), output)
    }
}

impl HfOwnedStreamingDecoder {
    fn decode_ids(&self) -> Result<String, TokenizationError> {
        self.tokenizer
            .decode(self.ids.as_slice(), self.skip_special_tokens)
            .map_err(|_| TokenizationError::Implementation { code: ERROR_DECODE })
    }
}

fn write_fragment<S: TextSink>(
    fragment: &str,
    output: &mut S,
) -> Result<DecodeReport, TokenizationError> {
    if fragment.len() > output.remaining_capacity() {
        return Err(CapacityExhausted::new(
            CapacityResource::DecodeBytes,
            usize_to_u64(fragment.len()),
            usize_to_u64(output.remaining_capacity()),
        )
        .into());
    }
    output.push_str(fragment)?;
    Ok(DecodeReport {
        bytes_written: fragment.len(),
        skipped_special_token: fragment.is_empty(),
    })
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
