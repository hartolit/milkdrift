//! Integration tests for bounded text-output accumulation.

use std::num::NonZeroUsize;

use domain_contracts::RequestId;
use host_runtime::{TextOutputRecordKind, text_output_accumulator};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Done,
}

#[test]
fn text_output_preserves_order_and_reuses_storage() -> Result<(), String> {
    let (producer, consumer) = text_output_accumulator::<State>(
        NonZeroUsize::new(32).ok_or("byte capacity")?,
        NonZeroUsize::new(4).ok_or("record capacity")?,
    )
    .map_err(|error| format!("initialization failed: {error:?}"))?;
    let request = RequestId::new(7);
    producer
        .try_push_text(request, "hello")
        .map_err(|error| format!("text push failed: {error:?}"))?;
    producer
        .try_push_state(request, State::Done)
        .map_err(|error| format!("state push failed: {error:?}"))?;

    consumer
        .pull(|batch| {
            assert_eq!(batch.records.len(), 2);
            let first = batch.records.first();
            let second = batch.records.get(1);
            assert!(matches!(
                first.map(|record| record.kind),
                Some(TextOutputRecordKind::Text(_))
            ));
            if let Some(TextOutputRecordKind::Text(range)) = first.map(|record| record.kind) {
                assert_eq!(batch.text_for(range), Some("hello"));
            }
            assert_eq!(
                second.map(|record| record.kind),
                Some(TextOutputRecordKind::State(State::Done))
            );
        })
        .map_err(|error| format!("pull failed: {error:?}"))?;

    assert_eq!(
        producer
            .try_lengths()
            .map_err(|error| format!("lengths: {error:?}"))?,
        (0, 0)
    );
    Ok(())
}
