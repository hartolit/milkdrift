use milkdrift_persistence::{
    ClockWatermarkObservation, ClockWatermarkStore, PersistenceError, TimestampMillis,
};
use redb::ReadableTable as _;

use crate::{
    RedbStore, error,
    schema::{CLOCK_WATERMARK_UNIX_MS_KEY, METADATA},
};

impl ClockWatermarkStore for RedbStore {
    fn observe_clock(
        &self,
        observed: TimestampMillis,
    ) -> Result<ClockWatermarkObservation, PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        let outcome = observe_clock_in_transaction(&write, observed)?;
        if outcome == ClockWatermarkObservation::Advanced {
            write.commit().map_err(error::redb)?;
        }
        Ok(outcome)
    }

    fn clock_watermark(&self) -> Result<Option<TimestampMillis>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let metadata = read.open_table(METADATA).map_err(error::redb)?;
        metadata
            .get(CLOCK_WATERMARK_UNIX_MS_KEY)
            .map_err(error::redb)
            .and_then(|stored| {
                stored
                    .map(|value| Some(TimestampMillis::new(value.value())))
                    .ok_or_else(|| {
                        error::corruption("boundary-clock high-water evidence is missing")
                    })
            })
    }
}

pub(crate) fn observe_clock_in_transaction(
    write: &redb::WriteTransaction,
    observed: TimestampMillis,
) -> Result<ClockWatermarkObservation, PersistenceError> {
    let mut metadata = write.open_table(METADATA).map_err(error::redb)?;
    let stored = metadata
        .get(CLOCK_WATERMARK_UNIX_MS_KEY)
        .map_err(error::redb)?
        .map(|value| value.value());
    match stored {
        Some(watermark) if observed.get() < watermark => {
            Ok(ClockWatermarkObservation::RejectedRollback {
                watermark: TimestampMillis::new(watermark),
            })
        }
        Some(watermark) if observed.get() == watermark => Ok(ClockWatermarkObservation::Unchanged),
        _ => {
            metadata
                .insert(CLOCK_WATERMARK_UNIX_MS_KEY, observed.get())
                .map_err(error::redb)?;
            Ok(ClockWatermarkObservation::Advanced)
        }
    }
}

pub(crate) fn require_accepted_clock(
    observation: ClockWatermarkObservation,
) -> Result<(), PersistenceError> {
    match observation {
        ClockWatermarkObservation::Advanced | ClockWatermarkObservation::Unchanged => Ok(()),
        ClockWatermarkObservation::RejectedRollback { .. } => Err(PersistenceError::Storage {
            class: milkdrift_persistence::StorageFailureClass::Unavailable,
            message: "boundary clock moved behind durable high-water evidence".to_owned(),
        }),
    }
}

pub(crate) fn require_clock_in_transaction(
    write: &redb::WriteTransaction,
    observed: TimestampMillis,
) -> Result<(), PersistenceError> {
    require_accepted_clock(observe_clock_in_transaction(write, observed)?)
}
