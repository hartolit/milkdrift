//! Wrapper-specific typed range resolution tests.

use domain_contracts::TokenId;
use host_runtime::{
    TextOutputBatch, TextOutputCursor, TextRange, TokenOutputBatch, TokenOutputCursor, TokenRange,
};

#[test]
fn text_range_resolution_validates_utf8_and_batch_ownership() {
    let invalid = [0xff_u8];
    let invalid_batch = TextOutputBatch::<u8> {
        start: TextOutputCursor::new(8),
        end: TextOutputCursor::new(9),
        bytes: &invalid,
        records: &[],
    };
    assert_eq!(
        invalid_batch.text_for(TextRange::new(TextOutputCursor::new(8), 1)),
        None
    );

    let valid = *b"hello";
    let valid_batch = TextOutputBatch::<u8> {
        start: TextOutputCursor::new(10),
        end: TextOutputCursor::new(15),
        bytes: &valid,
        records: &[],
    };
    assert_eq!(
        valid_batch.text_for(TextRange::new(TextOutputCursor::new(10), 5)),
        Some("hello")
    );
    assert_eq!(
        valid_batch.text_for(TextRange::new(TextOutputCursor::new(9), 1)),
        None
    );
    assert_eq!(
        valid_batch.text_for(TextRange::new(TextOutputCursor::new(15), 1)),
        None
    );
}

#[test]
fn token_range_resolution_returns_only_the_current_typed_slice() {
    let tokens = [TokenId::new(3), TokenId::new(5), TokenId::new(8)];
    let batch = TokenOutputBatch::<u8> {
        start: TokenOutputCursor::new(21),
        end: TokenOutputCursor::new(24),
        tokens: &tokens,
        records: &[],
    };
    assert_eq!(
        batch.tokens_for(TokenRange::new(TokenOutputCursor::new(22), 2)),
        Some(&tokens[1..])
    );
    assert_eq!(
        batch.tokens_for(TokenRange::new(TokenOutputCursor::new(20), 1)),
        None
    );
    assert_eq!(
        batch.tokens_for(TokenRange::new(TokenOutputCursor::new(24), 1)),
        None
    );
}
