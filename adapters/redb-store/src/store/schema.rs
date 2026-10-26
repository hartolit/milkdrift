use crate::{
    error,
    fault::FaultInjector,
    schema::{
        ARTIFACT_ACCOUNTING, CLOCK_WATERMARK_UNIX_MS_KEY, EVALUATION_PENDING_COUNT_KEY,
        INTERNAL_DOCUMENT_FORMAT_VERSION, INTERNAL_DOCUMENT_FORMAT_VERSION_KEY,
        MANAGED_ACTIVE_COUNT_KEY, MANAGED_ACTIVE_INSTALLATIONS, MANAGED_PENDING_EVALUATIONS,
        METADATA, PEER_EXECUTION_ACCOUNTING, PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY,
        SCHEMA_VERSION_KEY, SECURITY_AUDIT, SECURITY_AUDIT_COUNT_KEY,
        SECURITY_AUDIT_NEXT_SEQUENCE_KEY, STORAGE_SCHEMA_VERSION, TASK_ACCOUNTING,
    },
};
use milkdrift_persistence::{PersistenceError, TimestampMillis};
use redb::{Database, ReadableTable, ReadableTableMetadata};

pub(crate) fn validate_format(database: &Database) -> Result<(), PersistenceError> {
    format_for_open(database, false).map(|_| ())
}

fn format_for_open(database: &Database, allow_previous: bool) -> Result<bool, PersistenceError> {
    let read = database.begin_read().map_err(error::redb)?;
    let table = read.open_table(METADATA).map_err(error::redb)?;
    let mut previous = false;
    let mut physical = STORAGE_SCHEMA_VERSION;
    for (key, supported, document) in [
        (SCHEMA_VERSION_KEY, STORAGE_SCHEMA_VERSION, "storage"),
        (
            INTERNAL_DOCUMENT_FORMAT_VERSION_KEY,
            INTERNAL_DOCUMENT_FORMAT_VERSION,
            "redb internal document envelope",
        ),
    ] {
        let found = table
            .get(key)
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("storage format marker is missing"))?
            .value();
        if key == SCHEMA_VERSION_KEY && allow_previous && matches!(found, 19..=21) {
            previous = true;
            physical = found;
        } else if found != supported {
            return Err(PersistenceError::UnsupportedVersion {
                document,
                found: u32::try_from(found).unwrap_or(u32::MAX),
                supported: u32::try_from(supported)
                    .map_err(|_| error::corruption("schema exceeds u32"))?,
            });
        }
    }
    if physical == 19 {
        crate::schema::validate_previous_tables(&read)?;
    } else if physical == 20 {
        crate::schema::validate_daemon_tables(&read)?;
    } else if physical == 21 {
        crate::schema::validate_project_tables(&read)?;
    } else {
        crate::schema::validate_tables(&read)?;
    }
    Ok(previous)
}

/// Preflight the current or explicitly adoptable physical format before opening for writing.
pub(crate) fn validate_for_open(database: &Database) -> Result<bool, PersistenceError> {
    let previous = format_for_open(database, true)?;
    validate_shared_state(database)?;
    let read = database.begin_read().map_err(error::redb)?;
    let physical = read
        .open_table(METADATA)
        .map_err(error::redb)?
        .get(SCHEMA_VERSION_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("schema missing"))?
        .value();
    if physical >= 20 {
        crate::daemon::validate(&database.begin_read().map_err(error::redb)?)?;
    }
    if physical >= 21 {
        crate::project::validate(&read)?;
    }
    Ok(previous)
}

/// Only schemas 19 through 21/internal 22 are adoptable. Existing row bytes are untouched;
/// missing task-authoring, project and daemon families appear with the marker atomically.
pub(crate) fn adopt_supported_schema(
    database: &Database,
    faults: &dyn FaultInjector,
) -> Result<(), PersistenceError> {
    if !validate_for_open(database)? {
        return Ok(());
    }
    let write = database.begin_write().map_err(error::redb)?;
    crate::schema::initialize_daemon_tables(&write)?;
    crate::schema::initialize_project_tables(&write)?;
    crate::schema::initialize_authoring_tables(&write)?;
    write
        .open_table(METADATA)
        .map_err(error::redb)?
        .insert(SCHEMA_VERSION_KEY, STORAGE_SCHEMA_VERSION)
        .map_err(error::redb)?;
    faults.check(crate::fault::FaultPoint::BeforeDaemonSchemaAdoptionCommit)?;
    write.commit().map_err(error::redb)?;
    faults.check(crate::fault::FaultPoint::AfterDaemonSchemaAdoptionCommit)
}

pub(crate) fn initialize_schema(
    database: &Database,
    faults: &dyn FaultInjector,
    startup_observation: TimestampMillis,
) -> Result<(), PersistenceError> {
    let write = database.begin_write().map_err(error::redb)?;
    crate::schema::initialize_tables(&write)?;
    crate::task::initialize_accounting(&write)?;
    {
        let mut table = write.open_table(METADATA).map_err(error::redb)?;
        for (key, value) in [
            (SCHEMA_VERSION_KEY, STORAGE_SCHEMA_VERSION),
            (
                INTERNAL_DOCUMENT_FORMAT_VERSION_KEY,
                INTERNAL_DOCUMENT_FORMAT_VERSION,
            ),
            (CLOCK_WATERMARK_UNIX_MS_KEY, startup_observation.get()),
            (SECURITY_AUDIT_NEXT_SEQUENCE_KEY, 1),
            (SECURITY_AUDIT_COUNT_KEY, 0),
            (MANAGED_ACTIVE_COUNT_KEY, 0),
            (EVALUATION_PENDING_COUNT_KEY, 0),
        ] {
            table.insert(key, value).map_err(error::redb)?;
        }
    }
    let bytes = crate::json::encode(
        &crate::artifact::ArtifactAccountingRecord::EMPTY,
        "artifact accounting",
    )?;
    write
        .open_table(ARTIFACT_ACCOUNTING)
        .map_err(error::redb)?
        .insert(crate::artifact::GLOBAL_ARTIFACT_BYTES_KEY, bytes.as_slice())
        .map_err(error::redb)?;
    let bytes = crate::json::encode(
        &crate::peer::GlobalPeerAccounting::EMPTY,
        "peer global accounting",
    )?;
    write
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?
        .insert(PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY, bytes.as_slice())
        .map_err(error::redb)?;
    faults.check(crate::fault::FaultPoint::BeforeSchemaCommit)?;
    write.commit().map_err(error::redb)?;
    faults.check(crate::fault::FaultPoint::AfterSchemaCommit)
}

pub(crate) fn validate_schema(database: &Database) -> Result<(), PersistenceError> {
    validate_format(database)?;
    validate_shared_state(database)?;
    let read = database.begin_read().map_err(error::redb)?;
    crate::daemon::validate(&read)?;
    crate::project::validate(&read)
}

fn validate_shared_state(database: &Database) -> Result<(), PersistenceError> {
    let read = database.begin_read().map_err(error::redb)?;
    let metadata = read.open_table(METADATA).map_err(error::redb)?;
    for key in [
        CLOCK_WATERMARK_UNIX_MS_KEY,
        SECURITY_AUDIT_NEXT_SEQUENCE_KEY,
        SECURITY_AUDIT_COUNT_KEY,
    ] {
        if metadata.get(key).map_err(error::redb)?.is_none() {
            return Err(error::corruption("required durable metadata is missing"));
        }
    }
    for (index, key) in [
        (MANAGED_ACTIVE_INSTALLATIONS, MANAGED_ACTIVE_COUNT_KEY),
        (MANAGED_PENDING_EVALUATIONS, EVALUATION_PENDING_COUNT_KEY),
    ] {
        super::capacity::verify_read_count(&read, index, key)?;
    }
    let audit = read.open_table(SECURITY_AUDIT).map_err(error::redb)?;
    let count = metadata
        .get(SECURITY_AUDIT_COUNT_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("audit count absent"))?
        .value();
    let next = metadata
        .get(SECURITY_AUDIT_NEXT_SEQUENCE_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("audit sequence absent"))?
        .value();
    if audit.len().map_err(error::redb)? != count
        || audit
            .last()
            .map_err(error::redb)?
            .map_or(1, |(key, _)| key.value().saturating_add(1))
            != next
    {
        return Err(error::corruption(
            "security audit accounting differs from retained records",
        ));
    }
    let tasks = read.open_table(TASK_ACCOUNTING).map_err(error::redb)?;
    let task_bytes = tasks
        .get("global")
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("task accounting missing"))?;
    crate::task::validate_accounting_row("global", task_bytes.value())?;
    let accounting = read
        .open_table(PEER_EXECUTION_ACCOUNTING)
        .map_err(error::redb)?;
    let bytes = accounting
        .get(PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("peer global accounting is missing"))?;
    let _: crate::peer::GlobalPeerAccounting =
        crate::json::decode(bytes.value(), "peer global accounting")?;
    Ok(())
}
