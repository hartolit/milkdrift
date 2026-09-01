use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

#[derive(Debug)]
struct MutableArtifactClock(AtomicU64);

impl MutableArtifactClock {
    const fn new(now: u64) -> Self {
        Self(AtomicU64::new(now))
    }

    fn set(&self, now: u64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl ArtifactClock for MutableArtifactClock {
    fn now(&self) -> Result<TimestampMillis, PersistenceError> {
        Ok(TimestampMillis::new(self.0.load(Ordering::SeqCst)))
    }
}

#[test]
fn durable_clock_watermark_rejects_rollback_after_reopen() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = TempDir::new()?;
    let clock = Arc::new(MutableArtifactClock::new(100));
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path()).with_artifact_clock(clock.clone()),
    )?;
    assert_eq!(store.clock_watermark()?, Some(TimestampMillis::new(100)));
    assert_eq!(
        store.observe_clock(TimestampMillis::new(120))?,
        ClockWatermarkObservation::Advanced
    );
    drop(store);

    clock.set(119);
    let rollback = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path()).with_artifact_clock(clock.clone()),
    );
    assert!(matches!(
        rollback,
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Unavailable,
            ..
        })
    ));

    clock.set(120);
    let reopened = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path()).with_artifact_clock(clock),
    )?;
    assert_eq!(reopened.clock_watermark()?, Some(TimestampMillis::new(120)));
    Ok(())
}

#[test]
fn schema_nine_refuses_missing_clock_watermark() -> Result<(), Box<dyn std::error::Error>> {
    const METADATA: TableDefinition<'static, &'static str, u64> =
        TableDefinition::new("milkdrift.v1.metadata");
    const CLOCK_WATERMARK_KEY: &str = "boundary_clock_high_water_unix_ms";

    let directory = TempDir::new()?;
    drop(RedbStore::open(directory.path())?);
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut metadata = write.open_table(METADATA)?;
        assert!(metadata.remove(CLOCK_WATERMARK_KEY)?.is_some());
    }
    write.commit()?;
    drop(database);

    assert!(matches!(
        RedbStore::open(directory.path()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    Ok(())
}

#[test]
fn artifact_acceptance_and_clock_advance_share_one_transaction()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let clock = Arc::new(MutableArtifactClock::new(100));
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_artifact_clock(clock.clone())
            .with_fault_injector(Arc::new(FailOnce::new(
                FaultPoint::BeforeArtifactBeginCommit,
            ))),
    )?;
    clock.set(200);
    let bytes = b"transactional clock watermark";
    let request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-clock-transaction")?,
        RunId::new("run-clock-transaction")?,
        artifact_metadata(
            "artifact-clock-transaction",
            bytes,
            ArtifactSensitivity::Public,
        )?,
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;

    assert!(store.begin_publication(&request).is_err());
    assert_eq!(store.clock_watermark()?, Some(TimestampMillis::new(100)));
    Ok(())
}
