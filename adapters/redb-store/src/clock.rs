use milkdrift_persistence::{
    ClockWatermarkObservation, ClockWatermarkStore, PersistenceError, TimestampMillis,
};
use redb::ReadableTable as _;

use crate::{
    RedbStore, error,
    schema::{CLOCK_WATERMARK_UNIX_MS_KEY, METADATA},
};

impl RedbStore {
    /// Samples under the write transaction so concurrent publication cannot overtake the sample.
    pub fn sample_clock(
        &self,
    ) -> Result<(TimestampMillis, ClockWatermarkObservation), PersistenceError> {
        self.observe_clock_with(|| self.clock.now())
    }

    fn observe_clock_with(
        &self,
        sample: impl FnOnce() -> Result<TimestampMillis, PersistenceError>,
    ) -> Result<(TimestampMillis, ClockWatermarkObservation), PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        let observed = sample()?;
        let outcome = observe_clock_in_transaction(&write, observed)?;
        if outcome == ClockWatermarkObservation::Advanced {
            write.commit().map_err(error::redb)?;
        }
        Ok((observed, outcome))
    }
}

impl ClockWatermarkStore for RedbStore {
    fn observe_clock(
        &self,
        observed: TimestampMillis,
    ) -> Result<ClockWatermarkObservation, PersistenceError> {
        self.observe_clock_with(|| Ok(observed))
            .map(|(_, outcome)| outcome)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RedbStoreConfig, StoreClock};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    };
    use std::{thread, time::Duration};

    struct ObservedClock {
        now: AtomicU64,
        armed: AtomicBool,
        sampled: mpsc::SyncSender<()>,
    }

    impl StoreClock for ObservedClock {
        fn now(&self) -> Result<TimestampMillis, PersistenceError> {
            let now = self.now.load(Ordering::SeqCst);
            if self.armed.load(Ordering::SeqCst) {
                let _ = self.sampled.try_send(());
            }
            Ok(TimestampMillis::new(now))
        }
    }

    #[test]
    fn fresh_clock_sampling_waits_for_concurrent_watermark_transaction()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let (sampled, sample) = mpsc::sync_channel(1);
        let clock = Arc::new(ObservedClock {
            now: AtomicU64::new(100),
            armed: AtomicBool::new(false),
            sampled,
        });
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path()).with_clock(clock.clone()),
        )?);
        let write = store.database().begin_write()?;
        clock.armed.store(true, Ordering::SeqCst);
        let (started, start) = mpsc::sync_channel(1);
        let sampling_store = store.clone();
        let sampling = thread::spawn(move || {
            let _ = started.send(());
            sampling_store.sample_clock()
        });
        start.recv_timeout(Duration::from_secs(2))?;
        let sampled_before_transaction = sample.recv_timeout(Duration::from_millis(100)).is_ok();
        clock.now.store(120, Ordering::SeqCst);
        observe_clock_in_transaction(&write, TimestampMillis::new(120))?;
        write.commit()?;
        let result = sampling.join().map_err(|_| "clock sampler panicked")??;
        assert!(
            !sampled_before_transaction,
            "fresh time was sampled before acquiring its transaction"
        );
        assert_eq!(
            result,
            (
                TimestampMillis::new(120),
                ClockWatermarkObservation::Unchanged
            )
        );
        Ok(())
    }
}
