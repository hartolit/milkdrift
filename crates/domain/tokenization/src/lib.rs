#![no_std]
#![forbid(unsafe_code)]
#![doc = "Allocation-free tokenization contracts and reusable caller-owned buffers."]

mod error;
mod sink;
mod tokenizer;
mod utf8;

pub use error::TokenizationError;
pub use sink::{ByteBuffer, ByteSink, TextBuffer, TextSink, TokenBuffer, TokenSink};
pub use tokenizer::{
    DecodeOptions, DecodeReport, EncodeOptions, EncodeReport, SpecialTokenPolicy, StreamingDecoder,
    StreamingTokenizer, Tokenizer,
};
pub use utf8::IncrementalUtf8Decoder;
