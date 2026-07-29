//! Contract tests for the Hugging Face tokenizer adapter.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::str::Utf8Error;

use domain_contracts::TokenId;
use hf_tokenizer::{HfTokenizer, HfTokenizerLoadError};
use tokenization::{
    DecodeOptions, EncodeOptions, StreamingDecoder, StreamingTokenizer, TextBuffer, TokenBuffer,
    TokenizationError, Tokenizer,
};
use tokenizers::models::wordlevel::WordLevel;
use tokenizers::pre_tokenizers::whitespace::Whitespace;

#[derive(Debug)]
enum TestError {
    Builder(Box<dyn Error + Send + Sync>),
    Load(HfTokenizerLoadError),
    Tokenization(TokenizationError),
    Utf8(Utf8Error),
}

impl Display for TestError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builder(error) => {
                write!(formatter, "tokenizer fixture construction failed: {error}")
            }
            Self::Load(error) => {
                write!(formatter, "tokenizer adapter construction failed: {error}")
            }
            Self::Tokenization(error) => {
                write!(formatter, "tokenization operation failed: {error:?}")
            }
            Self::Utf8(error) => {
                write!(formatter, "decoded text was not valid UTF-8: {error}")
            }
        }
    }
}

impl Error for TestError {}

impl From<Box<dyn Error + Send + Sync>> for TestError {
    fn from(error: Box<dyn Error + Send + Sync>) -> Self {
        Self::Builder(error)
    }
}

impl From<HfTokenizerLoadError> for TestError {
    fn from(error: HfTokenizerLoadError) -> Self {
        Self::Load(error)
    }
}

impl From<TokenizationError> for TestError {
    fn from(error: TokenizationError) -> Self {
        Self::Tokenization(error)
    }
}

impl From<Utf8Error> for TestError {
    fn from(error: Utf8Error) -> Self {
        Self::Utf8(error)
    }
}

fn fixture() -> Result<HfTokenizer, TestError> {
    let vocabulary = [
        ("[UNK]".to_owned(), 0),
        ("hello".to_owned(), 1),
        ("world".to_owned(), 2),
    ]
    .into_iter()
    .collect();
    let model = WordLevel::builder()
        .vocab(vocabulary)
        .unk_token("[UNK]".to_owned())
        .build()?;
    let mut tokenizer = tokenizers::Tokenizer::new(model);
    tokenizer.with_pre_tokenizer(Some(Whitespace));
    Ok(HfTokenizer::from_tokenizer(tokenizer)?)
}

#[test]
fn vocabulary_lookup_returns_project_owned_identifiers() -> Result<(), TestError> {
    let tokenizer = fixture()?;
    assert_eq!(tokenizer.token_id("hello"), Some(TokenId::new(1)));
    assert_eq!(tokenizer.token_id("missing"), None);
    Ok(())
}

#[test]
fn encodes_into_caller_owned_storage() -> Result<(), TestError> {
    let tokenizer = fixture()?;
    let mut storage = [TokenId::new(0); 4];
    let mut output = TokenBuffer::new(&mut storage);
    let report = tokenizer.encode("hello world", EncodeOptions::default(), &mut output)?;
    assert_eq!(report.tokens_written, 2);
    assert_eq!(output.as_slice(), &[TokenId::new(1), TokenId::new(2)]);
    Ok(())
}

#[test]
fn streaming_decoder_writes_valid_text() -> Result<(), TestError> {
    let tokenizer = fixture()?;
    let mut decoder = tokenizer.decoder(DecodeOptions::default());
    let mut storage = [0_u8; 32];
    let mut output = TextBuffer::new(&mut storage);
    let _report = decoder.step(TokenId::new(1), &mut output)?;
    assert_eq!(output.as_str()?, "hello");
    Ok(())
}

#[test]
fn owned_streaming_decoder_preserves_state_across_steps() -> Result<(), TestError> {
    let tokenizer = fixture()?;
    let mut decoder = tokenizer.owned_decoder(DecodeOptions::default());

    let mut first_storage = [0_u8; 32];
    let mut first = TextBuffer::new(&mut first_storage);
    let _first_report = decoder.step(TokenId::new(1), &mut first)?;
    assert_eq!(first.as_str()?, "hello");

    let mut second_storage = [0_u8; 32];
    let mut second = TextBuffer::new(&mut second_storage);
    let _second_report = decoder.step(TokenId::new(2), &mut second)?;
    assert_eq!(second.as_str()?, " world");
    Ok(())
}
