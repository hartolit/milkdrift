//! Frame-pull output accumulator tests.

use std::num::NonZeroUsize;

use domain_contracts::{FinishReason, OutputRecordKind, RequestId, TokenId};
use host_runtime::{OutputPushError, output_accumulator};

#[test]
fn frame_pull_reuses_storage_and_advances_cursor() -> Result<(), String> {
    let (producer, consumer) = output_accumulator(non_zero(32)?, non_zero(4)?)
        .map_err(|error| format!("output initialization failed: {error:?}"))?;
    producer
        .try_push_text(RequestId::new(1), "abc")
        .map_err(|error| format!("text push failed: {error:?}"))?;
    producer
        .try_push_record(
            RequestId::new(1),
            OutputRecordKind::Finished(FinishReason::EndOfSequence(TokenId::new(2))),
        )
        .map_err(|error| format!("record push failed: {error:?}"))?;

    let first = consumer
        .pull(|batch| {
            (
                batch.start.get(),
                batch.end.get(),
                batch.bytes.to_vec(),
                batch.records.len(),
            )
        })
        .map_err(|error| format!("first pull failed: {error:?}"))?;
    if first != (0, 3, b"abc".to_vec(), 2) {
        return Err("unexpected first output batch".into());
    }

    producer
        .try_push_text(RequestId::new(2), "de")
        .map_err(|error| format!("second text push failed: {error:?}"))?;
    let second = consumer
        .pull(|batch| (batch.start.get(), batch.end.get(), batch.bytes.to_vec()))
        .map_err(|error| format!("second pull failed: {error:?}"))?;
    if second != (3, 5, b"de".to_vec()) {
        return Err("output cursor did not advance".into());
    }
    Ok(())
}

#[test]
fn capacity_failure_is_atomic() -> Result<(), String> {
    let (producer, consumer) = output_accumulator(non_zero(3)?, non_zero(1)?)
        .map_err(|error| format!("output initialization failed: {error:?}"))?;
    producer
        .try_push_text(RequestId::new(1), "abc")
        .map_err(|error| format!("initial text push failed: {error:?}"))?;
    match producer.try_push_text(RequestId::new(1), "d") {
        Err(OutputPushError::CapacityExhausted(_)) => {}
        Err(error) => return Err(format!("unexpected push failure: {error:?}")),
        Ok(()) => return Err("bounded output accepted excess text".into()),
    }
    let pulled = consumer
        .pull(|batch| (batch.bytes.to_vec(), batch.records.len()))
        .map_err(|error| format!("pull failed: {error:?}"))?;
    if pulled != (b"abc".to_vec(), 1) {
        return Err("failed push modified committed output".into());
    }
    Ok(())
}

fn non_zero(value: usize) -> Result<NonZeroUsize, String> {
    NonZeroUsize::new(value).ok_or_else(|| "non-zero output capacity".into())
}
