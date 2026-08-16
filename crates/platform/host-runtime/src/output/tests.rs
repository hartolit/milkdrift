use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};

use domain_contracts::{CapacityExhausted, CapacityResource, RequestId, TokenId};

use super::{
    OutputInitializationError, OutputPullError, OutputPushError, TextOutputConsumer,
    TextOutputProducer, TextOutputRecordKind, TextRange, TokenOutputConsumer, TokenOutputProducer,
    TokenOutputRecordKind, TokenRange, text_output_accumulator, token_output_accumulator,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    First,
    Second,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordKind {
    Payload { start: u64, length: usize },
    State(State),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    request: u64,
    kind: RecordKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    start: u64,
    end: u64,
    payload: Vec<u32>,
    records: Vec<Record>,
}

trait OutputHarness {
    type Producer;
    type Consumer;

    const PAYLOAD_RESOURCE: CapacityResource;

    fn create(
        payload_capacity: usize,
        record_capacity: usize,
    ) -> Result<(Self::Producer, Self::Consumer), OutputInitializationError>;
    fn capacities(producer: &Self::Producer) -> (usize, usize);
    fn push_payload(
        producer: &Self::Producer,
        request: RequestId,
        payload: &[u8],
    ) -> Result<(), OutputPushError>;
    fn push_state(
        producer: &Self::Producer,
        request: RequestId,
        state: State,
    ) -> Result<(), OutputPushError>;
    fn lengths(producer: &Self::Producer) -> Result<(usize, usize), OutputPushError>;
    fn pull(consumer: &Self::Consumer) -> Result<Snapshot, OutputPullError>;
    fn pull_with_state_push(
        consumer: &Self::Consumer,
        producer: &Self::Producer,
    ) -> Result<Result<(), OutputPushError>, OutputPullError>;
    fn pull_pointers(consumer: &Self::Consumer) -> Result<(usize, usize), OutputPullError>;
    fn pull_resolution(
        consumer: &Self::Consumer,
        earlier_start: u64,
        later_start: u64,
    ) -> Result<(bool, bool, bool), OutputPullError>;
    fn set_cursor(producer: &Self::Producer, cursor: u64) -> Result<(), OutputPullError>;
    fn poison(consumer: &Self::Consumer) -> bool;
}

struct TextHarness;

impl OutputHarness for TextHarness {
    type Producer = TextOutputProducer<State>;
    type Consumer = TextOutputConsumer<State>;

    const PAYLOAD_RESOURCE: CapacityResource = CapacityResource::OutputBytes;

    fn create(
        payload_capacity: usize,
        record_capacity: usize,
    ) -> Result<(Self::Producer, Self::Consumer), OutputInitializationError> {
        text_output_accumulator(nonzero(payload_capacity), nonzero(record_capacity))
    }

    fn capacities(producer: &Self::Producer) -> (usize, usize) {
        producer.capacities()
    }

    fn push_payload(
        producer: &Self::Producer,
        request: RequestId,
        payload: &[u8],
    ) -> Result<(), OutputPushError> {
        let text = std::str::from_utf8(payload).map_err(|_| OutputPushError::Poisoned)?;
        producer.try_push_text(request, text)
    }

    fn push_state(
        producer: &Self::Producer,
        request: RequestId,
        state: State,
    ) -> Result<(), OutputPushError> {
        producer.try_push_state(request, state)
    }

    fn lengths(producer: &Self::Producer) -> Result<(usize, usize), OutputPushError> {
        producer.try_lengths()
    }

    fn pull(consumer: &Self::Consumer) -> Result<Snapshot, OutputPullError> {
        consumer.pull(|batch| Snapshot {
            start: batch.start.get(),
            end: batch.end.get(),
            payload: batch.bytes.iter().copied().map(u32::from).collect(),
            records: batch
                .records
                .iter()
                .map(|record| Record {
                    request: record.request_id.get(),
                    kind: match record.kind {
                        TextOutputRecordKind::Text(range) => RecordKind::Payload {
                            start: range.start.get(),
                            length: range.length,
                        },
                        TextOutputRecordKind::State(state) => RecordKind::State(state),
                    },
                })
                .collect(),
        })
    }

    fn pull_with_state_push(
        consumer: &Self::Consumer,
        producer: &Self::Producer,
    ) -> Result<Result<(), OutputPushError>, OutputPullError> {
        consumer.pull(|_| producer.try_push_state(RequestId::new(99), State::Second))
    }

    fn pull_pointers(consumer: &Self::Consumer) -> Result<(usize, usize), OutputPullError> {
        consumer.pull(|batch| {
            (
                batch.bytes.as_ptr() as usize,
                batch.records.as_ptr() as usize,
            )
        })
    }

    fn pull_resolution(
        consumer: &Self::Consumer,
        earlier_start: u64,
        later_start: u64,
    ) -> Result<(bool, bool, bool), OutputPullError> {
        consumer.pull(|batch| {
            let current = TextRange::new(batch.start, batch.bytes.len());
            let earlier = TextRange::new(super::TextOutputCursor::new(earlier_start), 2);
            let later = TextRange::new(super::TextOutputCursor::new(later_start), 2);
            (
                batch.text_for(current).is_some(),
                batch.text_for(earlier).is_none(),
                batch.text_for(later).is_none(),
            )
        })
    }

    fn set_cursor(producer: &Self::Producer, cursor: u64) -> Result<(), OutputPullError> {
        producer.set_cursor_for_test(cursor)
    }

    fn poison(consumer: &Self::Consumer) -> bool {
        catch_unwind(AssertUnwindSafe(|| {
            let _ = consumer.pull(|_| assert_eq!(1_u8, 2_u8));
        }))
        .is_err()
    }
}

struct TokenHarness;

impl OutputHarness for TokenHarness {
    type Producer = TokenOutputProducer<State>;
    type Consumer = TokenOutputConsumer<State>;

    const PAYLOAD_RESOURCE: CapacityResource = CapacityResource::Tokens;

    fn create(
        payload_capacity: usize,
        record_capacity: usize,
    ) -> Result<(Self::Producer, Self::Consumer), OutputInitializationError> {
        token_output_accumulator(nonzero(payload_capacity), nonzero(record_capacity))
    }

    fn capacities(producer: &Self::Producer) -> (usize, usize) {
        producer.capacities()
    }

    fn push_payload(
        producer: &Self::Producer,
        request: RequestId,
        payload: &[u8],
    ) -> Result<(), OutputPushError> {
        let mut tokens = [TokenId::new(0); 8];
        let Some(destination) = tokens.get_mut(..payload.len()) else {
            return Err(OutputPushError::Poisoned);
        };
        for (slot, value) in destination.iter_mut().zip(payload.iter().copied()) {
            *slot = TokenId::new(u32::from(value));
        }
        producer.try_push_tokens(request, destination)
    }

    fn push_state(
        producer: &Self::Producer,
        request: RequestId,
        state: State,
    ) -> Result<(), OutputPushError> {
        producer.try_push_state(request, state)
    }

    fn lengths(producer: &Self::Producer) -> Result<(usize, usize), OutputPushError> {
        producer.try_lengths()
    }

    fn pull(consumer: &Self::Consumer) -> Result<Snapshot, OutputPullError> {
        consumer.pull(|batch| Snapshot {
            start: batch.start.get(),
            end: batch.end.get(),
            payload: batch.tokens.iter().map(|token| token.get()).collect(),
            records: batch
                .records
                .iter()
                .map(|record| Record {
                    request: record.request_id.get(),
                    kind: match record.kind {
                        TokenOutputRecordKind::Tokens(range) => RecordKind::Payload {
                            start: range.start.get(),
                            length: range.length,
                        },
                        TokenOutputRecordKind::State(state) => RecordKind::State(state),
                    },
                })
                .collect(),
        })
    }

    fn pull_with_state_push(
        consumer: &Self::Consumer,
        producer: &Self::Producer,
    ) -> Result<Result<(), OutputPushError>, OutputPullError> {
        consumer.pull(|_| producer.try_push_state(RequestId::new(99), State::Second))
    }

    fn pull_pointers(consumer: &Self::Consumer) -> Result<(usize, usize), OutputPullError> {
        consumer.pull(|batch| {
            (
                batch.tokens.as_ptr() as usize,
                batch.records.as_ptr() as usize,
            )
        })
    }

    fn pull_resolution(
        consumer: &Self::Consumer,
        earlier_start: u64,
        later_start: u64,
    ) -> Result<(bool, bool, bool), OutputPullError> {
        consumer.pull(|batch| {
            let current = TokenRange::new(batch.start, batch.tokens.len());
            let earlier = TokenRange::new(super::TokenOutputCursor::new(earlier_start), 2);
            let later = TokenRange::new(super::TokenOutputCursor::new(later_start), 2);
            (
                batch.tokens_for(current).is_some(),
                batch.tokens_for(earlier).is_none(),
                batch.tokens_for(later).is_none(),
            )
        })
    }

    fn set_cursor(producer: &Self::Producer, cursor: u64) -> Result<(), OutputPullError> {
        producer.set_cursor_for_test(cursor)
    }

    fn poison(consumer: &Self::Consumer) -> bool {
        catch_unwind(AssertUnwindSafe(|| {
            let _ = consumer.pull(|_| assert_eq!(1_u8, 2_u8));
        }))
        .is_err()
    }
}

#[test]
fn text_wrapper_conforms_to_bounded_output_invariants() -> Result<(), String> {
    run_conformance::<TextHarness>()
}

#[test]
fn token_wrapper_conforms_to_bounded_output_invariants() -> Result<(), String> {
    run_conformance::<TokenHarness>()
}

fn run_conformance<H: OutputHarness>() -> Result<(), String> {
    let request = RequestId::new(1);

    let (producer, consumer) = H::create(4, 1).map_err(debug)?;
    assert_eq!(H::capacities(&producer), (4, 1));
    H::push_payload(&producer, request, b"abcd").map_err(debug)?;
    assert_eq!(H::lengths(&producer), Ok((4, 1)));
    assert_eq!(
        H::pull(&consumer).map_err(debug)?,
        Snapshot {
            start: 0,
            end: 4,
            payload: b"abcd".iter().copied().map(u32::from).collect(),
            records: vec![Record {
                request: 1,
                kind: RecordKind::Payload {
                    start: 0,
                    length: 4,
                },
            }],
        }
    );

    let (producer, consumer) = H::create(4, 2).map_err(debug)?;
    H::push_payload(&producer, request, b"abc").map_err(debug)?;
    assert_eq!(
        H::push_payload(&producer, request, b"de"),
        Err(OutputPushError::CapacityExhausted(CapacityExhausted::new(
            H::PAYLOAD_RESOURCE,
            5,
            4
        )))
    );
    assert_eq!(H::lengths(&producer), Ok((3, 1)));
    assert_eq!(H::pull(&consumer).map_err(debug)?.payload, vec![97, 98, 99]);

    let (producer, consumer) = H::create(1, 2).map_err(debug)?;
    H::push_state(&producer, request, State::First).map_err(debug)?;
    H::push_state(&producer, request, State::Second).map_err(debug)?;
    assert_eq!(H::lengths(&producer), Ok((0, 2)));
    assert_eq!(
        H::push_state(&producer, request, State::First),
        Err(OutputPushError::CapacityExhausted(CapacityExhausted::new(
            CapacityResource::OutputRecords,
            3,
            2
        )))
    );
    assert_eq!(H::lengths(&producer), Ok((0, 2)));
    assert_eq!(H::pull(&consumer).map_err(debug)?.records.len(), 2);

    let (producer, consumer) = H::create(2, 3).map_err(debug)?;
    H::push_payload(&producer, request, b"ab").map_err(debug)?;
    H::push_state(&producer, request, State::First).map_err(debug)?;
    assert_eq!(
        H::push_payload(&producer, request, b"c"),
        Err(OutputPushError::CapacityExhausted(CapacityExhausted::new(
            H::PAYLOAD_RESOURCE,
            3,
            2
        )))
    );
    H::push_state(&producer, request, State::Second).map_err(debug)?;
    assert_eq!(H::lengths(&producer), Ok((2, 3)));
    assert_eq!(H::pull(&consumer).map_err(debug)?.records.len(), 3);

    let (producer, consumer) = H::create(4, 1).map_err(debug)?;
    H::push_state(&producer, request, State::First).map_err(debug)?;
    assert_eq!(
        H::push_payload(&producer, request, b"ab"),
        Err(OutputPushError::CapacityExhausted(CapacityExhausted::new(
            CapacityResource::OutputRecords,
            2,
            1
        )))
    );
    assert_eq!(H::lengths(&producer), Ok((0, 1)));
    assert!(H::pull(&consumer).map_err(debug)?.payload.is_empty());

    let (producer, consumer) = H::create(4, 2).map_err(debug)?;
    H::push_payload(&producer, request, b"").map_err(debug)?;
    assert_eq!(H::lengths(&producer), Ok((0, 0)));
    let empty = H::pull(&consumer).map_err(debug)?;
    assert!(empty.payload.is_empty());
    assert!(empty.records.is_empty());

    let (producer, consumer) = H::create(4, 4).map_err(debug)?;
    let other = RequestId::new(2);
    H::push_payload(&producer, request, b"ab").map_err(debug)?;
    H::push_state(&producer, request, State::First).map_err(debug)?;
    H::push_payload(&producer, other, b"c").map_err(debug)?;
    H::push_state(&producer, other, State::Second).map_err(debug)?;
    let ordered = H::pull(&consumer).map_err(debug)?;
    assert_eq!(
        ordered.records,
        vec![
            Record {
                request: 1,
                kind: RecordKind::Payload {
                    start: 0,
                    length: 2,
                },
            },
            Record {
                request: 1,
                kind: RecordKind::State(State::First),
            },
            Record {
                request: 2,
                kind: RecordKind::Payload {
                    start: 2,
                    length: 1,
                },
            },
            Record {
                request: 2,
                kind: RecordKind::State(State::Second),
            },
        ]
    );

    let (producer, consumer) = H::create(4, 2).map_err(debug)?;
    H::push_payload(&producer, request, b"ab").map_err(debug)?;
    assert_eq!(H::pull(&consumer).map_err(debug)?.end, 2);
    H::push_payload(&producer, request, b"cd").map_err(debug)?;
    assert_eq!(
        H::pull_resolution(&consumer, 0, 4).map_err(debug)?,
        (true, true, true)
    );
    H::push_payload(&producer, request, b"e").map_err(debug)?;
    let third = H::pull(&consumer).map_err(debug)?;
    assert_eq!((third.start, third.end), (4, 5));

    let (producer, consumer) = H::create(4, 3).map_err(debug)?;
    H::set_cursor(&producer, u64::MAX - 1).map_err(debug)?;
    assert_eq!(
        H::push_payload(&producer, request, b"ab"),
        Err(OutputPushError::CapacityExhausted(CapacityExhausted::new(
            H::PAYLOAD_RESOURCE,
            2,
            1
        )))
    );
    assert_eq!(H::lengths(&producer), Ok((0, 0)));
    H::push_payload(&producer, request, b"a").map_err(debug)?;
    assert_eq!(
        H::push_payload(&producer, request, b"b"),
        Err(OutputPushError::CapacityExhausted(CapacityExhausted::new(
            H::PAYLOAD_RESOURCE,
            1,
            0
        )))
    );
    let maximum = H::pull(&consumer).map_err(debug)?;
    assert_eq!((maximum.start, maximum.end), (u64::MAX - 1, u64::MAX));
    assert_eq!(maximum.payload, vec![97]);

    let (producer, consumer) = H::create(4, 2).map_err(debug)?;
    H::push_payload(&producer, request, b"a").map_err(debug)?;
    assert_eq!(
        H::pull_with_state_push(&consumer, &producer).map_err(debug)?,
        Err(OutputPushError::ConsumerBusy)
    );
    assert_eq!(H::lengths(&producer), Ok((0, 0)));

    let (producer, consumer) = H::create(4, 2).map_err(debug)?;
    H::push_payload(&producer, request, b"ab").map_err(debug)?;
    H::push_state(&producer, request, State::First).map_err(debug)?;
    let first_pointers = H::pull_pointers(&consumer).map_err(debug)?;
    H::push_payload(&producer, request, b"cd").map_err(debug)?;
    H::push_state(&producer, request, State::Second).map_err(debug)?;
    let second_pointers = H::pull_pointers(&consumer).map_err(debug)?;
    assert_eq!(first_pointers, second_pointers);

    let (producer, consumer) = H::create(2, 1).map_err(debug)?;
    H::push_payload(&producer, request, b"a").map_err(debug)?;
    assert!(H::poison(&consumer));
    assert_eq!(H::lengths(&producer), Err(OutputPushError::Poisoned));
    assert_eq!(H::pull(&consumer), Err(OutputPullError::Poisoned));

    Ok(())
}

fn nonzero(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).unwrap_or(NonZeroUsize::MIN)
}

fn debug<T: std::fmt::Debug>(error: T) -> String {
    format!("{error:?}")
}
