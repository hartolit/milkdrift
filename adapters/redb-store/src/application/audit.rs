//! Independently retained protected-operation audit transactions and reads.
use super::validate_stored;
use crate::{
    RedbStore, error, json, schema::METADATA, schema::SECURITY_AUDIT,
    schema::SECURITY_AUDIT_COUNT_KEY, schema::SECURITY_AUDIT_NEXT_SEQUENCE_KEY,
};
use milkdrift_persistence::{
    ApplicationCursor, ApplicationPage, ApplicationPageQuery, PersistenceError, SecurityAuditEntry,
    SecurityAuditRecord, SecurityAuditStore,
};
use redb::ReadableTableMetadata as _;
use std::ops::Bound;

const SECURITY_AUDIT_FAMILY: &str = "security audit record";

impl SecurityAuditStore for RedbStore {
    fn append_security_audit(
        &self,
        entry: &SecurityAuditEntry,
    ) -> Result<SecurityAuditRecord, PersistenceError> {
        entry.validate()?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let authoritative_audit_count = write
            .open_table(SECURITY_AUDIT)
            .map_err(error::redb)?
            .len()
            .map_err(error::redb)?;
        let (sequence, count) = {
            let mut metadata = write.open_table(METADATA).map_err(error::redb)?;
            let sequence = metadata
                .get(SECURITY_AUDIT_NEXT_SEQUENCE_KEY)
                .map_err(error::redb)?
                .map(|value| value.value())
                .ok_or_else(|| error::corruption("security audit next sequence is missing"))?;
            let count = metadata
                .get(SECURITY_AUDIT_COUNT_KEY)
                .map_err(error::redb)?
                .map(|value| value.value())
                .ok_or_else(|| error::corruption("security audit count is missing"))?;
            if count != authoritative_audit_count {
                return Err(error::corruption(
                    "security audit count disagrees with its authoritative table",
                ));
            }
            metadata
                .insert(
                    SECURITY_AUDIT_NEXT_SEQUENCE_KEY,
                    sequence
                        .checked_add(1)
                        .ok_or(PersistenceError::SequenceOverflow)?,
                )
                .map_err(error::redb)?;
            (sequence, count)
        };
        let record = SecurityAuditRecord {
            sequence,
            entry: entry.clone(),
        };
        let remove_count = count
            .saturating_add(1)
            .saturating_sub(u64::from(self.max_security_audit_records));
        {
            let mut audit = write.open_table(SECURITY_AUDIT).map_err(error::redb)?;
            let bytes = json::encode(&record, SECURITY_AUDIT_FAMILY)?;
            audit
                .insert(sequence, bytes.as_slice())
                .map_err(error::redb)?;
            for _ in 0..remove_count {
                let oldest = audit
                    .first()
                    .map_err(error::redb)?
                    .map(|(key, _)| key.value())
                    .ok_or_else(|| error::corruption("security audit count exceeds empty table"))?;
                audit.remove(oldest).map_err(error::redb)?;
            }
        }
        {
            let mut metadata = write.open_table(METADATA).map_err(error::redb)?;
            metadata
                .insert(
                    SECURITY_AUDIT_COUNT_KEY,
                    count.saturating_add(1).saturating_sub(remove_count),
                )
                .map_err(error::redb)?;
        }
        write.commit().map_err(error::redb)?;
        Ok(record)
    }

    fn security_audit(
        &self,
        query: &ApplicationPageQuery,
    ) -> Result<ApplicationPage<SecurityAuditRecord>, PersistenceError> {
        let after = query
            .after
            .as_ref()
            .map(|cursor| {
                let bytes: [u8; 8] = cursor.as_bytes().try_into().map_err(|_| {
                    PersistenceError::InvalidCursor(
                        "security audit cursor must contain one u64 sequence".to_owned(),
                    )
                })?;
                Ok::<u64, PersistenceError>(u64::from_be_bytes(bytes))
            })
            .transpose()?;
        let lower = after.map_or(Bound::Unbounded, Bound::Excluded);
        let read = self.database().begin_read().map_err(error::redb)?;
        let audit = read.open_table(SECURITY_AUDIT).map_err(error::redb)?;
        let rows = audit
            .range::<u64>((lower, Bound::Unbounded))
            .map_err(error::redb)?;
        let limit = usize::try_from(query.limit.get()).map_err(|_| PersistenceError::Bounds {
            location: "security_audit_page",
            reason: "page size exceeds platform".to_owned(),
        })?;
        let mut items = Vec::with_capacity(limit);
        let mut last = None;
        let mut more = false;
        for (index, row) in rows.enumerate() {
            let (sequence, value) = row.map_err(error::redb)?;
            if index == limit {
                more = true;
                break;
            }
            let sequence = sequence.value();
            let record = decode_security_audit(sequence, value.value())?;
            items.push(record);
            last = Some(sequence);
        }
        Ok(ApplicationPage {
            items,
            next: if more {
                last.map(|sequence| ApplicationCursor::new(sequence.to_be_bytes().to_vec()))
                    .transpose()?
            } else {
                None
            },
        })
    }
}

pub(crate) fn decode_security_audit(
    sequence: u64,
    bytes: &[u8],
) -> Result<SecurityAuditRecord, PersistenceError> {
    let record: SecurityAuditRecord = json::decode(bytes, SECURITY_AUDIT_FAMILY)?;
    if record.sequence != sequence {
        return Err(PersistenceError::Corruption(
            "security audit key does not match its record".to_owned(),
        ));
    }
    validate_stored(record.entry.validate(), SECURITY_AUDIT_FAMILY)?;
    Ok(record)
}
use redb::ReadableTable as _;
