//! Pull-oriented token output accumulator tests.

use std::num::NonZeroUsize;

use domain_contracts::{CapacityExhausted, CapacityResource, RequestId, TokenId};
use host_runtime::{
    OutputPushError, TokenOutputCursor, TokenOutputRecord, TokenOutputRecordKind, TokenRange,
    token_output_accumulator,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenerationState {
    Yielded,
    Finished,
    CleanupPending,
    Released,
}

#[test]
fn preserves_order_request_identity_and_absolute_token_ranges() -> Result<(), String> {
    let (producer, consumer) =
        token_output_accumulator::<GenerationState>(non_zero(8)?, non_zero(8)?)
            .map_err(|error| format!("token output initialization failed: {error:?}"))?;
    let first_request = RequestId::new(11);
    let second_request = RequestId::new(22);
    producer
        .try_push_tokens(first_request, &[TokenId::new(1), TokenId::new(2)])
        .map_err(|error| format!("first token push failed: {error:?}"))?;
    producer
        .try_push_state(first_request, GenerationState::Yielded)
        .map_err(|error| format!("yielded state push failed: {error:?}"))?;
    producer
        .try_push_token(second_request, TokenId::new(3))
        .map_err(|error| format!("second token push failed: {error:?}"))?;
    producer
        .try_push_state(second_request, GenerationState::Finished)
        .map_err(|error| format!("finished state push failed: {error:?}"))?;
    consumer
        .pull(|batch| {
            assert_eq!(batch.start, TokenOutputCursor::new(0));
            assert_eq!(batch.end, TokenOutputCursor::new(3));
            assert_eq!(
                batch.tokens,
                &[TokenId::new(1), TokenId::new(2), TokenId::new(3)]
            );
            assert_eq!(
                batch.records,
                &[
                    TokenOutputRecord {
                        request_id: first_request,
                        kind: TokenOutputRecordKind::Tokens(TokenRange::new(
                            TokenOutputCursor::new(0),
                            2,
                        )),
                    },
                    TokenOutputRecord {
                        request_id: first_request,
                        kind: TokenOutputRecordKind::State(GenerationState::Yielded),
                    },
                    TokenOutputRecord {
                        request_id: second_request,
                        kind: TokenOutputRecordKind::Tokens(TokenRange::new(
                            TokenOutputCursor::new(2),
                            1,
                        )),
                    },
                    TokenOutputRecord {
                        request_id: second_request,
                        kind: TokenOutputRecordKind::State(GenerationState::Finished),
                    },
                ]
            );
            assert_eq!(
                batch.tokens_for(TokenRange::new(TokenOutputCursor::new(0), 2)),
                Some(&[TokenId::new(1), TokenId::new(2)][..])
            );
        })
        .map_err(|error| format!("first pull failed: {error:?}"))?;
    Ok(())
}

#[test]
fn cursor_advances_without_duplicate_delivery() -> Result<(), String> {
    let (producer, consumer) =
        token_output_accumulator::<GenerationState>(non_zero(8)?, non_zero(8)?)
            .map_err(|error| format!("token output initialization failed: {error:?}"))?;
    let request_id = RequestId::new(11);
    producer
        .try_push_tokens(
            request_id,
            &[TokenId::new(1), TokenId::new(2), TokenId::new(3)],
        )
        .map_err(|error| format!("initial token push failed: {error:?}"))?;
    consumer
        .pull(|batch| {
            assert_eq!(batch.start, TokenOutputCursor::new(0));
            assert_eq!(batch.end, TokenOutputCursor::new(3));
        })
        .map_err(|error| format!("first pull failed: {error:?}"))?;
    consumer
        .pull(|batch| {
            assert_eq!(batch.start, TokenOutputCursor::new(3));
            assert_eq!(batch.end, TokenOutputCursor::new(3));
            assert!(batch.tokens.is_empty());
            assert!(batch.records.is_empty());
        })
        .map_err(|error| format!("empty pull failed: {error:?}"))?;

    producer
        .try_push_tokens(request_id, &[TokenId::new(4), TokenId::new(5)])
        .map_err(|error| format!("post-pull token push failed: {error:?}"))?;
    producer
        .try_push_state(request_id, GenerationState::CleanupPending)
        .map_err(|error| format!("cleanup state push failed: {error:?}"))?;
    producer
        .try_push_state(request_id, GenerationState::Released)
        .map_err(|error| format!("released state push failed: {error:?}"))?;
    consumer
        .pull(|batch| {
            assert_eq!(batch.start, TokenOutputCursor::new(3));
            assert_eq!(batch.end, TokenOutputCursor::new(5));
            assert_eq!(
                batch.records,
                &[
                    TokenOutputRecord {
                        request_id,
                        kind: TokenOutputRecordKind::Tokens(TokenRange::new(
                            TokenOutputCursor::new(3),
                            2,
                        )),
                    },
                    TokenOutputRecord {
                        request_id,
                        kind: TokenOutputRecordKind::State(GenerationState::CleanupPending),
                    },
                    TokenOutputRecord {
                        request_id,
                        kind: TokenOutputRecordKind::State(GenerationState::Released),
                    },
                ]
            );
        })
        .map_err(|error| format!("second populated pull failed: {error:?}"))?;
    Ok(())
}

#[test]
fn capacity_failures_are_resource_specific_and_atomic() -> Result<(), String> {
    let (producer, consumer) =
        token_output_accumulator::<GenerationState>(non_zero(2)?, non_zero(2)?)
            .map_err(|error| format!("token output initialization failed: {error:?}"))?;
    let request_id = RequestId::new(7);
    producer
        .try_push_tokens(request_id, &[TokenId::new(10), TokenId::new(20)])
        .map_err(|error| format!("initial token push failed: {error:?}"))?;

    assert_eq!(
        producer.try_push_token(request_id, TokenId::new(30)),
        Err(OutputPushError::CapacityExhausted(CapacityExhausted::new(
            CapacityResource::Tokens,
            3,
            2,
        )))
    );
    assert_eq!(
        producer
            .try_lengths()
            .map_err(|error| format!("length read failed: {error:?}"))?,
        (2, 1)
    );

    producer
        .try_push_state(request_id, GenerationState::Finished)
        .map_err(|error| format!("state push failed: {error:?}"))?;
    assert_eq!(
        producer.try_push_state(request_id, GenerationState::Released),
        Err(OutputPushError::CapacityExhausted(CapacityExhausted::new(
            CapacityResource::OutputRecords,
            3,
            2,
        )))
    );

    consumer
        .pull(|batch| {
            assert_eq!(batch.tokens, &[TokenId::new(10), TokenId::new(20)]);
            assert_eq!(
                batch.records,
                &[
                    TokenOutputRecord {
                        request_id,
                        kind: TokenOutputRecordKind::Tokens(TokenRange::new(
                            TokenOutputCursor::new(0),
                            2,
                        )),
                    },
                    TokenOutputRecord {
                        request_id,
                        kind: TokenOutputRecordKind::State(GenerationState::Finished),
                    },
                ]
            );
        })
        .map_err(|error| format!("capacity test pull failed: {error:?}"))?;
    Ok(())
}

#[test]
fn pull_retains_storage_and_producer_never_blocks_on_consumer() -> Result<(), String> {
    let (producer, consumer) =
        token_output_accumulator::<GenerationState>(non_zero(4)?, non_zero(2)?)
            .map_err(|error| format!("token output initialization failed: {error:?}"))?;
    let request_id = RequestId::new(9);
    producer
        .try_push_tokens(request_id, &[TokenId::new(1), TokenId::new(2)])
        .map_err(|error| format!("first token push failed: {error:?}"))?;
    producer
        .try_push_state(request_id, GenerationState::Yielded)
        .map_err(|error| format!("first state push failed: {error:?}"))?;

    let (first_token_pointer, first_record_pointer) = consumer
        .pull(|batch| {
            assert_eq!(
                producer.try_push_token(request_id, TokenId::new(99)),
                Err(OutputPushError::ConsumerBusy)
            );
            (batch.tokens.as_ptr(), batch.records.as_ptr())
        })
        .map_err(|error| format!("first pull failed: {error:?}"))?;

    producer
        .try_push_tokens(request_id, &[TokenId::new(3), TokenId::new(4)])
        .map_err(|error| format!("reused token push failed: {error:?}"))?;
    producer
        .try_push_state(request_id, GenerationState::Released)
        .map_err(|error| format!("reused state push failed: {error:?}"))?;
    consumer
        .pull(|batch| {
            assert_eq!(batch.tokens.as_ptr(), first_token_pointer);
            assert_eq!(batch.records.as_ptr(), first_record_pointer);
            assert_eq!(batch.start, TokenOutputCursor::new(2));
            assert_eq!(batch.end, TokenOutputCursor::new(4));
        })
        .map_err(|error| format!("second pull failed: {error:?}"))?;
    Ok(())
}

fn non_zero(value: usize) -> Result<NonZeroUsize, String> {
    NonZeroUsize::new(value).ok_or_else(|| "non-zero token output capacity".into())
}
