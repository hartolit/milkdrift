use crate::{
    error, fault::FaultInjector, schema::APPLICATION_COLD_RECEIPT_COUNT_KEY,
    schema::APPLICATION_COMMAND_RECEIPTS_COLD, schema::APPLICATION_COMMAND_RECEIPTS_HOT,
    schema::APPLICATION_HOT_RECEIPT_COUNT_KEY, schema::APPLICATION_HOT_RECEIPTS_BY_COMPLETION,
    schema::APPLICATION_RECEIPT_ARCHIVE_GENERATION_KEY,
    schema::APPLICATION_RECEIPT_LAST_ARCHIVED_AT_KEY, schema::ARTIFACT_ACCOUNTING,
    schema::CLOCK_WATERMARK_UNIX_MS_KEY, schema::INTERNAL_DOCUMENT_FORMAT_VERSION,
    schema::INTERNAL_DOCUMENT_FORMAT_VERSION_KEY, schema::LEASE_SET_REVISION_KEY, schema::METADATA,
    schema::NONTERMINAL_SET_COUNT_KEY, schema::PEER_EXECUTION_ACCOUNTING,
    schema::PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY, schema::SCHEMA_VERSION_KEY,
    schema::SECURITY_AUDIT, schema::SECURITY_AUDIT_COUNT_KEY,
    schema::SECURITY_AUDIT_NEXT_SEQUENCE_KEY, schema::STORAGE_SCHEMA_VERSION,
};
use milkdrift_persistence::{PersistenceError, TimestampMillis};
use redb::{Database, ReadableTable, ReadableTableMetadata};
pub(crate) fn initialize_schema(
    database: &Database,
    faults: &dyn FaultInjector,
    startup_observation: TimestampMillis,
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
        table
            .insert(CLOCK_WATERMARK_UNIX_MS_KEY, startup_observation.get())
            .map_err(error::redb)?;
        table
            .insert(LEASE_SET_REVISION_KEY, 0)
            .map_err(error::redb)?;
        table
            .insert(NONTERMINAL_SET_COUNT_KEY, 0)
            .map_err(error::redb)?;
        table
            .insert(APPLICATION_HOT_RECEIPT_COUNT_KEY, 0)
            .map_err(error::redb)?;
        table
            .insert(APPLICATION_COLD_RECEIPT_COUNT_KEY, 0)
            .map_err(error::redb)?;
        table
            .insert(APPLICATION_RECEIPT_ARCHIVE_GENERATION_KEY, 0)
            .map_err(error::redb)?;
        table
            .insert(APPLICATION_RECEIPT_LAST_ARCHIVED_AT_KEY, 0)
            .map_err(error::redb)?;
        table
            .insert(SECURITY_AUDIT_NEXT_SEQUENCE_KEY, 1)
            .map_err(error::redb)?;
        table
            .insert(SECURITY_AUDIT_COUNT_KEY, 0)
            .map_err(error::redb)?;
    }
    crate::schema::initialize_tables(&write)?;
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
        let mut table = write
            .open_table(PEER_EXECUTION_ACCOUNTING)
            .map_err(error::redb)?;
        let bytes = crate::json::encode(
            &crate::peer::GlobalPeerAccounting::EMPTY,
            "peer global accounting",
        )?;
        table
            .insert(PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY, bytes.as_slice())
            .map_err(error::redb)?;
    }
    faults.check(crate::fault::FaultPoint::BeforeSchemaCommit)?;
    write.commit().map_err(error::redb)?;
    faults.check(crate::fault::FaultPoint::AfterSchemaCommit)
}

pub(crate) fn validate_schema(database: &Database) -> Result<(), PersistenceError> {
    let read = database.begin_read().map_err(error::redb)?;
    let (
        found,
        internal_document_format,
        clock_watermark,
        lease_set_revision,
        nonterminal_set_count,
        application_hot_receipt_count,
        application_cold_receipt_count,
        application_receipt_archive_generation,
        application_receipt_last_archived_at,
        security_audit_next_sequence,
        security_audit_count,
    ) = {
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
        let clock_watermark = table
            .get(CLOCK_WATERMARK_UNIX_MS_KEY)
            .map_err(error::redb)?
            .map(|value| value.value());
        let lease_set_revision = table
            .get(LEASE_SET_REVISION_KEY)
            .map_err(error::redb)?
            .map(|value| value.value());
        let nonterminal_set_count = table
            .get(NONTERMINAL_SET_COUNT_KEY)
            .map_err(error::redb)?
            .map(|value| value.value());
        let application_hot_receipt_count = table
            .get(APPLICATION_HOT_RECEIPT_COUNT_KEY)
            .map_err(error::redb)?
            .map(|value| value.value());
        let application_cold_receipt_count = table
            .get(APPLICATION_COLD_RECEIPT_COUNT_KEY)
            .map_err(error::redb)?
            .map(|value| value.value());
        let application_receipt_archive_generation = table
            .get(APPLICATION_RECEIPT_ARCHIVE_GENERATION_KEY)
            .map_err(error::redb)?
            .map(|value| value.value());
        let application_receipt_last_archived_at = table
            .get(APPLICATION_RECEIPT_LAST_ARCHIVED_AT_KEY)
            .map_err(error::redb)?
            .map(|value| value.value());
        let security_audit_next_sequence = table
            .get(SECURITY_AUDIT_NEXT_SEQUENCE_KEY)
            .map_err(error::redb)?
            .map(|value| value.value());
        let security_audit_count = table
            .get(SECURITY_AUDIT_COUNT_KEY)
            .map_err(error::redb)?
            .map(|value| value.value());
        (
            found,
            internal_document_format,
            clock_watermark,
            lease_set_revision,
            nonterminal_set_count,
            application_hot_receipt_count,
            application_cold_receipt_count,
            application_receipt_archive_generation,
            application_receipt_last_archived_at,
            security_audit_next_sequence,
            security_audit_count,
        )
    };
    if found != STORAGE_SCHEMA_VERSION {
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
    clock_watermark
        .ok_or_else(|| error::corruption("boundary-clock high-water evidence is missing"))?;
    lease_set_revision.ok_or_else(|| error::corruption("lease-set revision is missing"))?;
    nonterminal_set_count.ok_or_else(|| error::corruption("nonterminal-set count is missing"))?;
    let application_hot_receipt_count = application_hot_receipt_count
        .ok_or_else(|| error::corruption("hot application receipt count is missing"))?;
    let application_cold_receipt_count = application_cold_receipt_count
        .ok_or_else(|| error::corruption("cold application receipt count is missing"))?;
    let application_receipt_archive_generation = application_receipt_archive_generation
        .ok_or_else(|| error::corruption("application receipt archive generation is missing"))?;
    let application_receipt_last_archived_at = application_receipt_last_archived_at
        .ok_or_else(|| error::corruption("application receipt archive time is missing"))?;
    if application_receipt_archive_generation == 0
        && (application_cold_receipt_count != 0 || application_receipt_last_archived_at != 0)
    {
        return Err(error::corruption(
            "cold application receipts exist before any archive generation",
        ));
    }
    let security_audit_next_sequence = security_audit_next_sequence
        .ok_or_else(|| error::corruption("security audit next sequence is missing"))?;
    let security_audit_count =
        security_audit_count.ok_or_else(|| error::corruption("security audit count is missing"))?;
    drop(read);

    let read = database.begin_read().map_err(error::redb)?;

    crate::schema::validate_tables(&read)?;
    {
        let hot = read
            .open_table(APPLICATION_COMMAND_RECEIPTS_HOT)
            .map_err(error::redb)?;
        let cold = read
            .open_table(APPLICATION_COMMAND_RECEIPTS_COLD)
            .map_err(error::redb)?;
        let ordered = read
            .open_table(APPLICATION_HOT_RECEIPTS_BY_COMPLETION)
            .map_err(error::redb)?;
        if hot.len().map_err(error::redb)? != application_hot_receipt_count {
            return Err(error::corruption(
                "hot application receipt count disagrees with its authoritative table",
            ));
        }
        if cold.len().map_err(error::redb)? != application_cold_receipt_count {
            return Err(error::corruption(
                "cold application receipt count disagrees with its authoritative table",
            ));
        }
        if ordered.len().map_err(error::redb)? != application_hot_receipt_count {
            return Err(error::corruption(
                "hot application receipt count disagrees with its completion index",
            ));
        }
        for row in hot.iter().map_err(error::redb)? {
            let (key, bytes) = row.map_err(error::redb)?;
            if cold.get(key.value()).map_err(error::redb)?.is_some() {
                return Err(error::corruption(
                    "application receipt has both hot and cold ownership",
                ));
            }
            let receipt = crate::application::decode_receipt(key.value(), bytes.value())?;
            let order_key = crate::application::receipt_order_key(&receipt)?;
            let indexed = ordered
                .get(order_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| {
                    error::corruption("hot application receipt is missing its completion index")
                })?;
            if indexed.value() != key.value() {
                return Err(error::corruption(
                    "hot application receipt completion index points to another identity",
                ));
            }
        }
    }
    {
        let table = read.open_table(SECURITY_AUDIT).map_err(error::redb)?;
        if table.len().map_err(error::redb)? != security_audit_count {
            return Err(error::corruption(
                "security audit count disagrees with its authoritative table",
            ));
        }
        let expected_next = table
            .last()
            .map_err(error::redb)?
            .map_or(1, |(sequence, _)| sequence.value().saturating_add(1));
        if security_audit_next_sequence != expected_next {
            return Err(error::corruption(
                "security audit next sequence disagrees with its authoritative table",
            ));
        }
    }
    {
        let accounting = read
            .open_table(PEER_EXECUTION_ACCOUNTING)
            .map_err(error::redb)?;
        let bytes = accounting
            .get(PEER_EXECUTION_GLOBAL_ACCOUNTING_KEY)
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("peer global accounting is missing"))?;
        let _: crate::peer::GlobalPeerAccounting =
            crate::json::decode(bytes.value(), "peer global accounting")?;
    }
    Ok(())
}
