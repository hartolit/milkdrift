//! GGUF-native tokenization backed by a vocabulary-only llama.cpp model.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use std::sync::Arc;

use domain_contracts::{CapacityExhausted, CapacityResource, TokenId};
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::token_type::LlamaTokenAttr;
use llama_cpp_2::{LlamaModelLoadError, TokenToStringError};
use tokenization::{
    ByteSink, DecodeOptions, DecodeReport, EncodeOptions, EncodeReport, IncrementalUtf8Decoder,
    SpecialTokenPolicy, StreamingDecoder, StreamingTokenizer, TextSink, TokenSink,
    TokenizationError, Tokenizer,
};

use crate::digest::{Sha256Digest, sha256_file};
use crate::loader::GgufBackendRuntime;
use crate::source::GgufSource;

const INITIAL_TOKEN_PIECE_BYTES: usize = 32;

/// Boundary-token role reported by tokenizer construction failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GgufBoundaryToken {
    /// Beginning-of-sequence token.
    BeginningOfSequence,
    /// End-of-sequence token.
    EndOfSequence,
}

impl Display for GgufBoundaryToken {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeginningOfSequence => formatter.write_str("beginning-of-sequence"),
            Self::EndOfSequence => formatter.write_str("end-of-sequence"),
        }
    }
}

/// Failure while loading and validating a GGUF vocabulary tokenizer.
#[derive(Debug)]
pub enum GgufTokenizerLoadError {
    /// The model file could not be hashed before or after loading its vocabulary.
    DigestRead(io::Error),
    /// File content did not match the required immutable identity.
    DigestMismatch {
        /// Required artifact identity.
        expected: Sha256Digest,
        /// Identity observed before vocabulary loading.
        actual: Sha256Digest,
    },
    /// File content changed while llama.cpp loaded the vocabulary.
    SourceChanged {
        /// Identity observed before vocabulary loading.
        before: Sha256Digest,
        /// Identity observed after vocabulary loading.
        after: Sha256Digest,
    },
    /// llama.cpp rejected the vocabulary-only model load.
    ModelLoad(LlamaModelLoadError),
    /// llama.cpp reported a non-positive or unrepresentable vocabulary size.
    InvalidVocabularySize {
        /// Native vocabulary size.
        size: i32,
    },
    /// llama.cpp reported an invalid configured boundary token.
    InvalidBoundaryToken {
        /// Boundary role being validated.
        boundary: GgufBoundaryToken,
        /// Native token value.
        token: i32,
    },
    /// llama.cpp returned invalid token attributes for an in-range token.
    InvalidTokenAttributes {
        /// Token whose attributes could not be classified.
        token: TokenId,
    },
    /// A recognized special/control spelling could not be decoded safely.
    InvalidSpecialTokenSpelling {
        /// Token whose spelling could not be decoded.
        token: TokenId,
    },
}

impl Display for GgufTokenizerLoadError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DigestRead(error) => write!(formatter, "failed to hash GGUF content: {error}"),
            Self::DigestMismatch { expected, actual } => write!(
                formatter,
                "GGUF SHA-256 mismatch: expected {expected}, observed {actual}"
            ),
            Self::SourceChanged { before, after } => write!(
                formatter,
                "GGUF content changed while loading vocabulary: {before} became {after}"
            ),
            Self::ModelLoad(error) => {
                write!(formatter, "failed to load GGUF vocabulary: {error}")
            }
            Self::InvalidVocabularySize { size } => {
                write!(formatter, "GGUF vocabulary size {size} is invalid")
            }
            Self::InvalidBoundaryToken { boundary, token } => {
                write!(formatter, "GGUF {boundary} token {token} is invalid")
            }
            Self::InvalidTokenAttributes { token } => write!(
                formatter,
                "GGUF token {} has invalid native attributes",
                token.get()
            ),
            Self::InvalidSpecialTokenSpelling { token } => write!(
                formatter,
                "GGUF special/control token {} has no safe spelling",
                token.get()
            ),
        }
    }
}

impl Error for GgufTokenizerLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DigestRead(error) => Some(error),
            Self::ModelLoad(error) => Some(error),
            Self::DigestMismatch { .. }
            | Self::SourceChanged { .. }
            | Self::InvalidVocabularySize { .. }
            | Self::InvalidBoundaryToken { .. }
            | Self::InvalidTokenAttributes { .. }
            | Self::InvalidSpecialTokenSpelling { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct TokenEvidence {
    special_or_control: bool,
    end_of_generation: bool,
}

struct SpecialSpelling {
    bytes: Box<[u8]>,
    token: TokenId,
}

// Field order is intentional: the native model must be dropped before the
// final process-level backend initialization token is released.
struct GgufTokenizerInner {
    model: LlamaModel,
    _runtime: GgufBackendRuntime,
    digest: Sha256Digest,
    vocabulary_size: u32,
    bos_token: Option<TokenId>,
    eos_token: Option<TokenId>,
    evidence: Box<[TokenEvidence]>,
    special_spellings: Box<[SpecialSpelling]>,
}

/// Cloneable GGUF tokenizer backed by a llama.cpp vocabulary-only model.
///
/// Every instance carries the SHA-256 identity of the exact GGUF bytes used to
/// load its vocabulary. Clones share the immutable native model and precomputed
/// control/special-token evidence.
#[derive(Clone)]
pub struct GgufTokenizer {
    inner: Arc<GgufTokenizerInner>,
}

impl GgufTokenizer {
    /// Native prompt tokenization failed.
    pub const ERROR_ENCODE: u32 = 1;
    /// Native token-to-piece decoding failed.
    pub const ERROR_DECODE: u32 = 2;
    /// BOS insertion was requested but the vocabulary has no valid BOS token.
    pub const ERROR_MISSING_BOS: u32 = 3;
    /// EOS insertion was requested but the vocabulary has no valid EOS token.
    pub const ERROR_MISSING_EOS: u32 = 4;
    /// `OrdinaryText` could not be honored because llama.cpp would parse a special spelling.
    pub const ERROR_ORDINARY_TEXT_SPECIAL: u32 = 5;
    /// `Reject` input contained a recognized control/special spelling.
    pub const ERROR_REJECTED_SPECIAL: u32 = 6;
    /// A native token identifier could not be represented safely.
    pub const ERROR_TOKEN_CONVERSION: u32 = 7;

    /// Loads a vocabulary-only tokenizer and records its observed content identity.
    ///
    /// The file is hashed before and after loading, so a concurrent content change
    /// fails construction even when no expected digest was supplied.
    ///
    /// # Errors
    ///
    /// Returns [`GgufTokenizerLoadError`] when hashing, native loading, identity
    /// validation, or vocabulary evidence construction fails.
    pub fn from_file(
        runtime: GgufBackendRuntime,
        path: impl AsRef<Path>,
    ) -> Result<Self, GgufTokenizerLoadError> {
        Self::load(runtime, path.as_ref(), None, true)
    }

    /// Loads a vocabulary-only tokenizer for one required GGUF content identity.
    ///
    /// SHA-256 is checked both before and after vocabulary loading. The returned
    /// tokenizer therefore cannot be paired with a model merely because its
    /// vocabulary size happens to match.
    ///
    /// # Errors
    ///
    /// Returns [`GgufTokenizerLoadError::DigestMismatch`] when the pre-load
    /// identity differs, [`GgufTokenizerLoadError::SourceChanged`] when bytes
    /// change during loading, or another construction error.
    pub fn from_file_verified(
        runtime: GgufBackendRuntime,
        path: impl AsRef<Path>,
        expected_digest: Sha256Digest,
    ) -> Result<Self, GgufTokenizerLoadError> {
        Self::load(runtime, path.as_ref(), Some(expected_digest), true)
    }

    /// Loads a tokenizer using a [`GgufSource`]'s path, mmap preference, and
    /// optional required content digest.
    ///
    /// # Errors
    ///
    /// Returns [`GgufTokenizerLoadError`] when hashing, native loading, identity
    /// validation, or vocabulary evidence construction fails.
    pub fn from_source(
        runtime: GgufBackendRuntime,
        source: &GgufSource,
    ) -> Result<Self, GgufTokenizerLoadError> {
        Self::load(
            runtime,
            source.path(),
            source.expected_digest(),
            source.execution().use_mmap(),
        )
    }

    /// Returns the SHA-256 identity of the GGUF bytes used for this vocabulary.
    #[must_use]
    pub fn content_digest(&self) -> Sha256Digest {
        self.inner.digest
    }

    /// Returns the validated beginning-of-sequence token, when configured.
    #[must_use]
    pub fn bos_token_id(&self) -> Option<TokenId> {
        self.inner.bos_token
    }

    /// Returns the validated end-of-sequence token, when configured.
    #[must_use]
    pub fn eos_token_id(&self) -> Option<TokenId> {
        self.inner.eos_token
    }

    /// Returns whether an in-range token is classified as control/special.
    #[must_use]
    pub fn is_special_token(&self, token: TokenId) -> bool {
        self.evidence(token)
            .is_some_and(|evidence| evidence.special_or_control)
    }

    /// Returns whether an in-range token is an end-of-generation marker.
    #[must_use]
    pub fn is_end_of_generation_token(&self, token: TokenId) -> bool {
        self.evidence(token)
            .is_some_and(|evidence| evidence.end_of_generation)
    }

    /// Resolves an exact vocabulary spelling to one token when that relationship
    /// can be demonstrated by the loaded vocabulary.
    ///
    /// Control/special spellings use the construction-time evidence table. Other
    /// spellings must tokenize to exactly one identifier and decode byte-for-byte
    /// to the original input.
    #[must_use]
    pub fn token_id(&self, spelling: &str) -> Option<TokenId> {
        if let Some(special) = self
            .inner
            .special_spellings
            .iter()
            .find(|special| special.bytes.as_ref() == spelling.as_bytes())
        {
            return Some(special.token);
        }

        let native = self
            .inner
            .model
            .str_to_token(spelling, AddBos::Never)
            .ok()?;
        let [native] = native.as_slice() else {
            return None;
        };
        let token = self.native_to_token_id(*native).ok()?;
        let piece = token_piece_bytes(&self.inner.model, *native).ok()?;
        (piece.as_slice() == spelling.as_bytes()).then_some(token)
    }

    /// Creates request-local streaming decode state that owns a tokenizer clone.
    #[must_use]
    pub fn owned_decoder(&self, options: DecodeOptions) -> GgufOwnedStreamingDecoder {
        GgufOwnedStreamingDecoder {
            tokenizer: self.clone(),
            utf8: IncrementalUtf8Decoder::new(),
            skip_special_tokens: options.skip_special_tokens,
        }
    }

    fn load(
        runtime: GgufBackendRuntime,
        path: &Path,
        expected_digest: Option<Sha256Digest>,
        use_mmap: bool,
    ) -> Result<Self, GgufTokenizerLoadError> {
        let before = sha256_file(path).map_err(GgufTokenizerLoadError::DigestRead)?;
        if let Some(expected) = expected_digest
            && before != expected
        {
            return Err(GgufTokenizerLoadError::DigestMismatch {
                expected,
                actual: before,
            });
        }

        let params = LlamaModelParams::default()
            .with_vocab_only(true)
            .with_use_mmap(use_mmap && runtime.supports_mmap())
            .with_use_mlock(false);
        let model = LlamaModel::load_from_file(runtime.native.as_ref(), path, &params)
            .map_err(GgufTokenizerLoadError::ModelLoad)?;

        let after = sha256_file(path).map_err(GgufTokenizerLoadError::DigestRead)?;
        if before != after {
            return Err(GgufTokenizerLoadError::SourceChanged { before, after });
        }
        if let Some(expected) = expected_digest
            && after != expected
        {
            return Err(GgufTokenizerLoadError::DigestMismatch {
                expected,
                actual: after,
            });
        }

        Self::from_native(runtime, model, after)
    }

    fn from_native(
        runtime: GgufBackendRuntime,
        model: LlamaModel,
        digest: Sha256Digest,
    ) -> Result<Self, GgufTokenizerLoadError> {
        let native_vocabulary_size = model.n_vocab();
        let vocabulary_size = u32::try_from(native_vocabulary_size).map_err(|_| {
            GgufTokenizerLoadError::InvalidVocabularySize {
                size: native_vocabulary_size,
            }
        })?;
        if vocabulary_size == 0 {
            return Err(GgufTokenizerLoadError::InvalidVocabularySize {
                size: native_vocabulary_size,
            });
        }
        let vocabulary_capacity = usize::try_from(vocabulary_size).map_err(|_| {
            GgufTokenizerLoadError::InvalidVocabularySize {
                size: native_vocabulary_size,
            }
        })?;

        let bos_token = validated_boundary_token(
            model.token_bos(),
            vocabulary_size,
            GgufBoundaryToken::BeginningOfSequence,
        )?;
        let eos_token = validated_boundary_token(
            model.token_eos(),
            vocabulary_size,
            GgufBoundaryToken::EndOfSequence,
        )?;

        let mut evidence = Vec::new();
        evidence
            .try_reserve_exact(vocabulary_capacity)
            .map_err(|_| GgufTokenizerLoadError::InvalidVocabularySize {
                size: native_vocabulary_size,
            })?;
        let mut special_spellings = Vec::new();

        for native_id in 0..native_vocabulary_size {
            let native = LlamaToken::new(native_id);
            let token = TokenId::new(u32::try_from(native_id).map_err(|_| {
                GgufTokenizerLoadError::InvalidVocabularySize {
                    size: native_vocabulary_size,
                }
            })?);
            let attributes = catch_unwind(AssertUnwindSafe(|| model.token_attr(native)))
                .map_err(|_| GgufTokenizerLoadError::InvalidTokenAttributes { token })?;
            let special_or_control = attributes.contains(LlamaTokenAttr::Control)
                || attributes.contains(LlamaTokenAttr::UserDefined)
                || attributes.contains(LlamaTokenAttr::Unknown);
            let end_of_generation = model.is_eog_token(native);
            evidence.push(TokenEvidence {
                special_or_control,
                end_of_generation,
            });

            if special_or_control {
                let bytes = token_piece_bytes(&model, native)
                    .map_err(|_| GgufTokenizerLoadError::InvalidSpecialTokenSpelling { token })?;
                if bytes.is_empty() {
                    continue;
                }
                special_spellings.push(SpecialSpelling {
                    bytes: bytes.into_boxed_slice(),
                    token,
                });
            }
        }

        special_spellings.sort_unstable_by(|left, right| {
            right
                .bytes
                .len()
                .cmp(&left.bytes.len())
                .then_with(|| left.bytes.cmp(&right.bytes))
                .then_with(|| left.token.cmp(&right.token))
        });
        special_spellings.dedup_by(|left, right| left.bytes == right.bytes);

        Ok(Self {
            inner: Arc::new(GgufTokenizerInner {
                model,
                _runtime: runtime,
                digest,
                vocabulary_size,
                bos_token,
                eos_token,
                evidence: evidence.into_boxed_slice(),
                special_spellings: special_spellings.into_boxed_slice(),
            }),
        })
    }

    fn contains_special_spelling(&self, text: &str) -> bool {
        self.inner
            .special_spellings
            .iter()
            .any(|special| contains_bytes(text.as_bytes(), special.bytes.as_ref()))
    }

    fn evidence(&self, token: TokenId) -> Option<TokenEvidence> {
        let index = usize::try_from(token.get()).ok()?;
        self.inner.evidence.get(index).copied()
    }

    fn native_to_token_id(&self, token: LlamaToken) -> Result<TokenId, TokenizationError> {
        if token.0 < 0 {
            return Err(TokenizationError::Implementation {
                code: Self::ERROR_TOKEN_CONVERSION,
            });
        }
        let identifier = u32::try_from(token.0).map_err(|_| TokenizationError::Implementation {
            code: Self::ERROR_TOKEN_CONVERSION,
        })?;
        if identifier >= self.inner.vocabulary_size {
            return Err(TokenizationError::Implementation {
                code: Self::ERROR_TOKEN_CONVERSION,
            });
        }
        Ok(TokenId::new(identifier))
    }

    fn token_to_native(&self, token: TokenId) -> Result<LlamaToken, TokenizationError> {
        if token.get() >= self.inner.vocabulary_size {
            return Err(TokenizationError::UnknownToken(token));
        }
        let identifier =
            i32::try_from(token.get()).map_err(|_| TokenizationError::Implementation {
                code: Self::ERROR_TOKEN_CONVERSION,
            })?;
        Ok(LlamaToken::new(identifier))
    }

    fn decoded_piece(
        &self,
        token: TokenId,
        skip_special_tokens: bool,
    ) -> Result<Option<Vec<u8>>, TokenizationError> {
        let native = self.token_to_native(token)?;
        let evidence = self
            .evidence(token)
            .ok_or(TokenizationError::UnknownToken(token))?;
        if skip_special_tokens && (evidence.special_or_control || evidence.end_of_generation) {
            return Ok(None);
        }
        token_piece_bytes(&self.inner.model, native)
            .map(Some)
            .map_err(|_| TokenizationError::Implementation {
                code: Self::ERROR_DECODE,
            })
    }
}

impl Tokenizer for GgufTokenizer {
    fn vocabulary_size(&self) -> u32 {
        self.inner.vocabulary_size
    }

    fn encode<S: TokenSink>(
        &self,
        text: &str,
        options: EncodeOptions,
        output: &mut S,
    ) -> Result<EncodeReport, TokenizationError> {
        if self.contains_special_spelling(text) {
            let code = match options.special_tokens {
                SpecialTokenPolicy::Allow => None,
                SpecialTokenPolicy::OrdinaryText => Some(Self::ERROR_ORDINARY_TEXT_SPECIAL),
                SpecialTokenPolicy::Reject => Some(Self::ERROR_REJECTED_SPECIAL),
            };
            if let Some(code) = code {
                return Err(TokenizationError::Implementation { code });
            }
        }

        let bos = if options.add_beginning_of_sequence {
            Some(
                self.inner
                    .bos_token
                    .ok_or(TokenizationError::Implementation {
                        code: Self::ERROR_MISSING_BOS,
                    })?,
            )
        } else {
            None
        };
        let eos = if options.add_end_of_sequence {
            Some(
                self.inner
                    .eos_token
                    .ok_or(TokenizationError::Implementation {
                        code: Self::ERROR_MISSING_EOS,
                    })?,
            )
        } else {
            None
        };

        let native = self
            .inner
            .model
            .str_to_token(text, AddBos::Never)
            .map_err(|_| TokenizationError::Implementation {
                code: Self::ERROR_ENCODE,
            })?;
        let boundary_count = usize::from(bos.is_some()) + usize::from(eos.is_some());
        let required =
            native
                .len()
                .checked_add(boundary_count)
                .ok_or(TokenizationError::Implementation {
                    code: Self::ERROR_TOKEN_CONVERSION,
                })?;
        let mut tokens = Vec::new();
        tokens
            .try_reserve_exact(required)
            .map_err(|_| TokenizationError::Implementation {
                code: Self::ERROR_ENCODE,
            })?;
        if let Some(token) = bos {
            tokens.push(token);
        }
        for token in native {
            tokens.push(self.native_to_token_id(token)?);
        }
        if let Some(token) = eos {
            tokens.push(token);
        }

        preflight_capacity(
            CapacityResource::Tokens,
            tokens.len(),
            output.remaining_capacity(),
        )?;
        output.push_tokens(tokens.as_slice())?;
        Ok(EncodeReport {
            tokens_written: tokens.len(),
        })
    }

    fn decode_token<S: ByteSink>(
        &self,
        token: TokenId,
        options: DecodeOptions,
        output: &mut S,
    ) -> Result<DecodeReport, TokenizationError> {
        let Some(piece) = self.decoded_piece(token, options.skip_special_tokens)? else {
            return Ok(DecodeReport {
                bytes_written: 0,
                skipped_special_token: true,
            });
        };
        preflight_capacity(
            CapacityResource::DecodeBytes,
            piece.len(),
            output.remaining_capacity(),
        )?;
        output.push_bytes(piece.as_slice())?;
        Ok(DecodeReport {
            bytes_written: piece.len(),
            skipped_special_token: false,
        })
    }
}

/// Request-local stateful decoder borrowing a [`GgufTokenizer`].
pub struct GgufStreamingDecoder<'tokenizer> {
    tokenizer: &'tokenizer GgufTokenizer,
    utf8: IncrementalUtf8Decoder,
    skip_special_tokens: bool,
}

impl GgufStreamingDecoder<'_> {
    /// Verifies that the token stream ended on a complete UTF-8 scalar value.
    ///
    /// # Errors
    ///
    /// Returns [`TokenizationError::IncompleteUtf8`] when bytes remain buffered.
    pub const fn finish(&self) -> Result<(), TokenizationError> {
        self.utf8.finish()
    }

    /// Returns the number of incomplete UTF-8 bytes retained across token steps.
    #[must_use]
    pub const fn pending_bytes(&self) -> u8 {
        self.utf8.pending_bytes()
    }
}

impl StreamingTokenizer for GgufTokenizer {
    type Decoder<'tokenizer>
        = GgufStreamingDecoder<'tokenizer>
    where
        Self: 'tokenizer;

    fn decoder(&self, options: DecodeOptions) -> Self::Decoder<'_> {
        GgufStreamingDecoder {
            tokenizer: self,
            utf8: IncrementalUtf8Decoder::new(),
            skip_special_tokens: options.skip_special_tokens,
        }
    }
}

impl StreamingDecoder for GgufStreamingDecoder<'_> {
    fn step<S: TextSink>(
        &mut self,
        token: TokenId,
        output: &mut S,
    ) -> Result<DecodeReport, TokenizationError> {
        decode_streaming_step(
            self.tokenizer,
            &mut self.utf8,
            self.skip_special_tokens,
            token,
            output,
        )
    }
}

/// Request-local stateful decoder owning a shared tokenizer handle.
pub struct GgufOwnedStreamingDecoder {
    tokenizer: GgufTokenizer,
    utf8: IncrementalUtf8Decoder,
    skip_special_tokens: bool,
}

impl GgufOwnedStreamingDecoder {
    /// Verifies that the token stream ended on a complete UTF-8 scalar value.
    ///
    /// # Errors
    ///
    /// Returns [`TokenizationError::IncompleteUtf8`] when bytes remain buffered.
    pub const fn finish(&self) -> Result<(), TokenizationError> {
        self.utf8.finish()
    }

    /// Returns the number of incomplete UTF-8 bytes retained across token steps.
    #[must_use]
    pub const fn pending_bytes(&self) -> u8 {
        self.utf8.pending_bytes()
    }

    /// Returns the immutable tokenizer shared by this request-local decoder.
    #[must_use]
    pub const fn tokenizer(&self) -> &GgufTokenizer {
        &self.tokenizer
    }
}

impl StreamingDecoder for GgufOwnedStreamingDecoder {
    fn step<S: TextSink>(
        &mut self,
        token: TokenId,
        output: &mut S,
    ) -> Result<DecodeReport, TokenizationError> {
        decode_streaming_step(
            &self.tokenizer,
            &mut self.utf8,
            self.skip_special_tokens,
            token,
            output,
        )
    }
}

fn decode_streaming_step<S: TextSink>(
    tokenizer: &GgufTokenizer,
    utf8: &mut IncrementalUtf8Decoder,
    skip_special_tokens: bool,
    token: TokenId,
    output: &mut S,
) -> Result<DecodeReport, TokenizationError> {
    let Some(piece) = tokenizer.decoded_piece(token, skip_special_tokens)? else {
        return Ok(DecodeReport {
            bytes_written: 0,
            skipped_special_token: true,
        });
    };
    let pending_before = usize::from(utf8.pending_bytes());
    let required =
        pending_before
            .checked_add(piece.len())
            .ok_or(TokenizationError::Implementation {
                code: GgufTokenizer::ERROR_DECODE,
            })?;
    preflight_capacity(
        CapacityResource::DecodeBytes,
        required,
        output.remaining_capacity(),
    )?;
    utf8.push_bytes(piece.as_slice(), output)?;
    let pending_after = usize::from(utf8.pending_bytes());
    let bytes_written =
        required
            .checked_sub(pending_after)
            .ok_or(TokenizationError::Implementation {
                code: GgufTokenizer::ERROR_DECODE,
            })?;
    Ok(DecodeReport {
        bytes_written,
        skipped_special_token: false,
    })
}

fn validated_boundary_token(
    token: LlamaToken,
    vocabulary_size: u32,
    boundary: GgufBoundaryToken,
) -> Result<Option<TokenId>, GgufTokenizerLoadError> {
    if token.0 == -1 {
        return Ok(None);
    }
    let identifier =
        u32::try_from(token.0).map_err(|_| GgufTokenizerLoadError::InvalidBoundaryToken {
            boundary,
            token: token.0,
        })?;
    if identifier >= vocabulary_size {
        return Err(GgufTokenizerLoadError::InvalidBoundaryToken {
            boundary,
            token: token.0,
        });
    }
    Ok(Some(TokenId::new(identifier)))
}

fn token_piece_bytes(model: &LlamaModel, token: LlamaToken) -> Result<Vec<u8>, TokenToStringError> {
    match model.token_to_piece_bytes(token, INITIAL_TOKEN_PIECE_BYTES, true, None) {
        Err(TokenToStringError::InsufficientBufferSpace(required)) => {
            let Some(required) = required.checked_neg() else {
                return Err(TokenToStringError::InsufficientBufferSpace(required));
            };
            let Ok(required) = usize::try_from(required) else {
                return Err(TokenToStringError::InsufficientBufferSpace(-1));
            };
            model.token_to_piece_bytes(token, required, true, None)
        }
        result => result,
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && needle.len() <= haystack.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn preflight_capacity(
    resource: CapacityResource,
    required: usize,
    available: usize,
) -> Result<(), TokenizationError> {
    if required > available {
        return Err(CapacityExhausted::new(
            resource,
            usize_to_u64(required),
            usize_to_u64(available),
        )
        .into());
    }
    Ok(())
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
