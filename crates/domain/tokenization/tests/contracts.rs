//! Integration tests for allocation-free tokenization contracts and buffers.

use domain_contracts::{CapacityResource, TokenId};
use tokenization::{
    ByteBuffer, ByteSink, DecodeOptions, DecodeReport, EncodeOptions, EncodeReport,
    IncrementalUtf8Decoder, TextBuffer, TokenBuffer, TokenSink, TokenizationError, Tokenizer,
};

struct ByteTokenizer;

impl Tokenizer for ByteTokenizer {
    fn vocabulary_size(&self) -> u32 {
        256
    }

    fn encode<S: TokenSink>(
        &self,
        text: &str,
        _options: EncodeOptions,
        output: &mut S,
    ) -> Result<EncodeReport, TokenizationError> {
        let before = output.remaining_capacity();
        for &byte in text.as_bytes() {
            output.push_token(TokenId::new(u32::from(byte)))?;
        }
        Ok(EncodeReport {
            tokens_written: before.saturating_sub(output.remaining_capacity()),
        })
    }

    fn decode_token<S: ByteSink>(
        &self,
        token: TokenId,
        _options: DecodeOptions,
        output: &mut S,
    ) -> Result<DecodeReport, TokenizationError> {
        let value = token.get();
        let byte = u8::try_from(value).map_err(|_| TokenizationError::UnknownToken(token))?;
        output.push_byte(byte)?;
        Ok(DecodeReport {
            bytes_written: 1,
            skipped_special_token: false,
        })
    }
}

#[test]
fn encoding_stops_before_overflow() {
    let tokenizer = ByteTokenizer;
    let mut storage = [TokenId::new(0); 2];
    let mut output = TokenBuffer::new(&mut storage);

    let result = tokenizer.encode("abc", EncodeOptions::default(), &mut output);

    assert!(matches!(
        result,
        Err(TokenizationError::CapacityExhausted(
            domain_contracts::CapacityExhausted {
                resource: CapacityResource::Tokens,
                required: 1,
                available: 0,
            }
        ))
    ));
    assert_eq!(
        output.as_slice(),
        &[TokenId::new(u32::from(b'a')), TokenId::new(u32::from(b'b'))]
    );
}

#[test]
fn incremental_utf8_crosses_token_boundaries() -> Result<(), TokenizationError> {
    let mut decoder = IncrementalUtf8Decoder::new();
    let mut storage = [0_u8; 8];
    let mut text = TextBuffer::new(&mut storage);

    decoder.push_bytes(&[0xF0, 0x9F], &mut text)?;
    assert_eq!(decoder.pending_bytes(), 2);
    decoder.push_bytes(&[0xA6, 0x80], &mut text)?;
    decoder.finish()?;

    assert_eq!(text.as_str()?, "🦀");
    Ok(())
}

#[test]
fn token_decode_uses_caller_owned_bytes() -> Result<(), TokenizationError> {
    let tokenizer = ByteTokenizer;
    let mut storage = [0_u8; 4];
    let mut output = ByteBuffer::new(&mut storage);

    tokenizer.decode_token(
        TokenId::new(u32::from(b'R')),
        DecodeOptions::default(),
        &mut output,
    )?;

    assert_eq!(output.as_slice(), b"R");
    Ok(())
}
