//! One private bounded concurrency and storage state machine.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, TryLockError};
use std::vec::Vec;

use domain_contracts::{CapacityExhausted, CapacityResource};

use super::{OutputInitializationError, OutputPullError, OutputPushError};

#[derive(Clone, Copy)]
pub(super) struct Capacities {
    pub(super) payload: usize,
    pub(super) records: usize,
}

#[derive(Clone, Copy)]
pub(super) struct Range {
    pub(super) start: u64,
    pub(super) length: usize,
}

pub(super) fn payload_for_range<P>(payload: &[P], batch_start: u64, range: Range) -> Option<&[P]> {
    let offset = range.start.checked_sub(batch_start)?;
    let offset = usize::try_from(offset).ok()?;
    let end = offset.checked_add(range.length)?;
    payload.get(offset..end)
}

pub(super) struct Batch<'a, P, R> {
    pub(super) start: u64,
    pub(super) end: u64,
    pub(super) payload: &'a [P],
    pub(super) records: &'a [R],
}

pub(super) struct Producer<P, R> {
    shared: Arc<Shared<P, R>>,
}

impl<P, R> Clone for Producer<P, R> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

pub(super) struct Consumer<P, R> {
    shared: Arc<Shared<P, R>>,
}

pub(super) type Ends<P, R> = (Producer<P, R>, Consumer<P, R>);

struct Shared<P, R> {
    state: Mutex<State<P, R>>,
    capacities: Capacities,
    payload_resource: CapacityResource,
}

struct State<P, R> {
    start: u64,
    end: u64,
    payload: Vec<P>,
    records: Vec<R>,
}

pub(super) fn accumulator<P: Copy, R>(
    payload_capacity: NonZeroUsize,
    record_capacity: NonZeroUsize,
    payload_resource: CapacityResource,
) -> Result<Ends<P, R>, OutputInitializationError> {
    let capacities = Capacities {
        payload: payload_capacity.get(),
        records: record_capacity.get(),
    };
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(capacities.payload)
        .map_err(|_| OutputInitializationError::PayloadStorage)?;
    let mut records = Vec::new();
    records
        .try_reserve_exact(capacities.records)
        .map_err(|_| OutputInitializationError::RecordStorage)?;
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            start: 0,
            end: 0,
            payload,
            records,
        }),
        capacities,
        payload_resource,
    });
    Ok((
        Producer {
            shared: Arc::clone(&shared),
        },
        Consumer { shared },
    ))
}

impl<P: Copy, R> Producer<P, R> {
    pub(super) fn capacities(&self) -> Capacities {
        self.shared.capacities
    }

    pub(super) fn try_push_payload<F>(
        &self,
        payload: &[P],
        make_record: F,
    ) -> Result<(), OutputPushError>
    where
        F: FnOnce(Range) -> R,
    {
        let mut state = self.try_lock()?;
        let required_payload = state.payload.len().saturating_add(payload.len());
        if required_payload > self.shared.capacities.payload {
            return Err(capacity_error(
                self.shared.payload_resource,
                required_payload,
                self.shared.capacities.payload,
            ));
        }
        ensure_record_capacity(state.records.len(), self.shared.capacities.records)?;

        let payload_length = u64::try_from(payload.len()).map_err(|_| {
            OutputPushError::CapacityExhausted(CapacityExhausted::new(
                self.shared.payload_resource,
                u64::MAX,
                u64::MAX.saturating_sub(state.end),
            ))
        })?;
        let Some(end) = state.end.checked_add(payload_length) else {
            return Err(OutputPushError::CapacityExhausted(CapacityExhausted::new(
                self.shared.payload_resource,
                payload_length,
                u64::MAX.saturating_sub(state.end),
            )));
        };

        let record = make_record(Range {
            start: state.end,
            length: payload.len(),
        });
        state.payload.extend_from_slice(payload);
        state.records.push(record);
        state.end = end;
        Ok(())
    }

    pub(super) fn try_push_record(&self, record: R) -> Result<(), OutputPushError> {
        let mut state = self.try_lock()?;
        ensure_record_capacity(state.records.len(), self.shared.capacities.records)?;
        state.records.push(record);
        Ok(())
    }

    pub(super) fn try_lengths(&self) -> Result<(usize, usize), OutputPushError> {
        let state = self.try_lock()?;
        Ok((state.payload.len(), state.records.len()))
    }

    fn try_lock(&self) -> Result<std::sync::MutexGuard<'_, State<P, R>>, OutputPushError> {
        self.shared.state.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => OutputPushError::ConsumerBusy,
            TryLockError::Poisoned(_) => OutputPushError::Poisoned,
        })
    }

    #[cfg(test)]
    pub(super) fn set_cursor_for_test(&self, cursor: u64) -> Result<(), OutputPullError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| OutputPullError::Poisoned)?;
        state.start = cursor;
        state.end = cursor;
        state.payload.clear();
        state.records.clear();
        Ok(())
    }
}

impl<P, R> Consumer<P, R> {
    pub(super) fn pull<T, F>(&self, consume: F) -> Result<T, OutputPullError>
    where
        F: for<'batch> FnOnce(Batch<'batch, P, R>) -> T,
    {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| OutputPullError::Poisoned)?;
        let result = consume(Batch {
            start: state.start,
            end: state.end,
            payload: state.payload.as_slice(),
            records: state.records.as_slice(),
        });
        state.start = state.end;
        state.payload.clear();
        state.records.clear();
        Ok(result)
    }
}

fn ensure_record_capacity(current: usize, capacity: usize) -> Result<(), OutputPushError> {
    let required = current.saturating_add(1);
    if required > capacity {
        return Err(capacity_error(
            CapacityResource::OutputRecords,
            required,
            capacity,
        ));
    }
    Ok(())
}

fn capacity_error(
    resource: CapacityResource,
    required: usize,
    available: usize,
) -> OutputPushError {
    CapacityExhausted::new(resource, usize_to_u64(required), usize_to_u64(available)).into()
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
