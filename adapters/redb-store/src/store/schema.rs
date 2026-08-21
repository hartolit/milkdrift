use super::*;
pub(crate) fn initialize_schema(
    database: &Database,
    faults: &dyn FaultInjector,
) -> Result<(), PersistenceError> {
    let write = database.begin_write().map_err(error::redb)?;
    {
        let mut table = write.open_table(METADATA).map_err(error::redb)?;
        table
            .insert(SCHEMA_VERSION_KEY, STORAGE_SCHEMA_VERSION)
            .map_err(error::redb)?;
        table
            .insert(
                INTERNAL_DOCUMENT_FORMAT_VERSION_KEY,
                INTERNAL_DOCUMENT_FORMAT_VERSION,
            )
            .map_err(error::redb)?;
    }
    // Opening each definition records its exact key/value encoding in redb.
    {
        let _table = write.open_table(REVISIONS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(RUN_HEADS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(EVENT_HISTORY_DIGESTS)
            .map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(RUN_HISTORY_ACCUMULATORS)
            .map_err(error::redb)?;
    }
    {
        let _table = write.open_table(COMMAND_RESULTS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(RUNNABLE_RUN_HEADS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(TIMER_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(TIMER_INDEX).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(LEASE_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(LEASE_INDEX).map_err(error::redb)?;
    }
    crate::trie::initialize(&write)?;
    {
        let _table = write.open_table(SNAPSHOTS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(SCOPES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(ROOT_SCOPES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(VALUES).map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(WORKSPACE_VALUE_HEADS)
            .map_err(error::redb)?;
    }
    {
        let _table = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(ARTIFACT_PUBLICATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
            .map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(ARTIFACT_RESERVATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(ARTIFACT_TEMP_OWNERS)
            .map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(ARTIFACT_TEMP_MANIFEST)
            .map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(ARTIFACT_DIGEST_RESERVATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = write.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(RUN_ARTIFACT_OWNERSHIP)
            .map_err(error::redb)?;
    }
    {
        let mut table = write.open_table(ARTIFACT_ACCOUNTING).map_err(error::redb)?;
        let bytes = crate::json::encode(
            &crate::artifact::ArtifactAccountingRecord::EMPTY,
            "artifact accounting",
        )?;
        table
            .insert(crate::artifact::GLOBAL_ARTIFACT_BYTES_KEY, bytes.as_slice())
            .map_err(error::redb)?;
    }
    {
        let _table = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
    }
    faults.check(crate::fault::FaultPoint::BeforeSchemaCommit)?;
    write.commit().map_err(error::redb)?;
    faults.check(crate::fault::FaultPoint::AfterSchemaCommit)
}

pub(crate) fn validate_schema(database: &Database) -> Result<(), PersistenceError> {
    let read = database.begin_read().map_err(error::redb)?;
    let (found, internal_document_format) = {
        let table = read.open_table(METADATA).map_err(error::redb)?;
        let found = table
            .get(SCHEMA_VERSION_KEY)
            .map_err(error::redb)?
            .map(|value| value.value())
            .ok_or_else(|| error::corruption("storage schema version is missing"))?;
        let internal_document_format = table
            .get(INTERNAL_DOCUMENT_FORMAT_VERSION_KEY)
            .map_err(error::redb)?
            .map(|value| value.value());
        (found, internal_document_format)
    };
    if found > STORAGE_SCHEMA_VERSION {
        let found = u32::try_from(found).unwrap_or(u32::MAX);
        return Err(PersistenceError::UnsupportedVersion {
            document: "storage",
            found,
            supported: STORAGE_SCHEMA_VERSION as u32,
        });
    }
    if found < STORAGE_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "storage",
            found: u32::try_from(found).unwrap_or(u32::MAX),
            supported: STORAGE_SCHEMA_VERSION as u32,
        });
    }
    let internal_document_format = internal_document_format
        .ok_or_else(|| error::corruption("redb internal document format marker is missing"))?;
    if internal_document_format != INTERNAL_DOCUMENT_FORMAT_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "redb internal document envelope",
            found: u32::try_from(internal_document_format).unwrap_or(u32::MAX),
            supported: INTERNAL_DOCUMENT_FORMAT_VERSION as u32,
        });
    }
    drop(read);

    let read = database.begin_read().map_err(error::redb)?;

    // A successful typed open is the schema's physical type check.
    {
        let _table = read.open_table(REVISIONS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(RUN_HEADS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(EVENT_HISTORY_DIGESTS)
            .map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(RUN_HISTORY_ACCUMULATORS)
            .map_err(error::redb)?;
    }
    {
        let _table = read.open_table(COMMAND_RESULTS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(RUNNABLE_RUN_HEADS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(TIMER_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(TIMER_INDEX).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(LEASE_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(LEASE_INDEX).map_err(error::redb)?;
    }
    crate::trie::validate_roots(&read)?;
    {
        let _table = read.open_table(SNAPSHOTS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(SCOPES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(VALUES).map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(WORKSPACE_VALUE_HEADS)
            .map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(ARTIFACT_PUBLICATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
            .map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(ARTIFACT_RESERVATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ARTIFACT_TEMP_OWNERS).map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(ARTIFACT_TEMP_MANIFEST)
            .map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(ARTIFACT_DIGEST_RESERVATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(RUN_ARTIFACT_OWNERSHIP)
            .map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ARTIFACT_ACCOUNTING).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
    }
    Ok(())
}
