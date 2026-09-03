use super::{
    ApplicationCommandStore, BoundedDetail, IntegrityScanRequest, IntegrityScanResult, METADATA,
    PersistenceError, RedbStore, SCHEMA_VERSION_KEY, StorageAdmin, StorageComponentHealth,
    StorageHealth, StorageHealthStatus, StorageSchemaCompatibility, StorageSchemaInfo,
    TimestampMillis, cursor::scan_index_sample, error,
};

mod integrity_scan;

impl StorageAdmin for RedbStore {
    fn schema_info(&self) -> Result<StorageSchemaInfo, PersistenceError> {
        let current_version = u32::try_from(crate::schema::STORAGE_SCHEMA_VERSION)
            .map_err(|_| error::corruption("storage schema version exceeds the public range"))?;
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(METADATA).map_err(error::redb)?;
        let found = table
            .get(SCHEMA_VERSION_KEY)
            .map_err(error::redb)?
            .map(|value| value.value())
            .ok_or_else(|| error::corruption("storage schema version is missing"))?;
        let stored_version = u32::try_from(found).unwrap_or(u32::MAX);
        let compatibility = if stored_version == current_version {
            StorageSchemaCompatibility::Current
        } else if stored_version < current_version {
            StorageSchemaCompatibility::MigrationRequired
        } else {
            StorageSchemaCompatibility::FutureUnsupported
        };
        Ok(StorageSchemaInfo {
            stored_version,
            current_version,
            compatibility,
        })
    }

    fn health(&self, observed_at: TimestampMillis) -> Result<StorageHealth, PersistenceError> {
        let schema = self.schema_info()?;
        let scan = self.scan_integrity(IntegrityScanRequest {
            limit: milkdrift_persistence::PageSize::new(32)?,
            verify_artifact_content: false,
            cursor: None,
        })?;
        let index_scan = scan_index_sample(self, 32)?;
        let receipt_status = self.application_receipt_status()?;
        let status = if scan.failures.is_empty() && index_scan.failures.is_empty() {
            StorageHealthStatus::Healthy
        } else {
            StorageHealthStatus::Degraded
        };
        let mut components = Vec::with_capacity(
            scan.failures
                .len()
                .saturating_add(index_scan.failures.len())
                .saturating_add(4),
        );
        components.push(StorageComponentHealth {
            component: BoundedDetail::new("storage_schema")?,
            status: StorageHealthStatus::Healthy,
            detail: BoundedDetail::new(format!(
                "physical schema {} is current",
                schema.stored_version
            ))?,
        });
        components.push(StorageComponentHealth {
            component: BoundedDetail::new("integrity_sample")?,
            status: if scan.failures.is_empty() {
                StorageHealthStatus::Healthy
            } else {
                StorageHealthStatus::Degraded
            },
            detail: BoundedDetail::new(if scan.next_cursor.is_some() {
                "bounded metadata sample is clean; additional records remain and artifact content was not rehashed"
            } else {
                "bounded metadata sample reached the current end without artifact-content rehashing; this is not a complete content-integrity proof"
            })?,
        });
        components.push(StorageComponentHealth {
            component: BoundedDetail::new("index_integrity_sample")?,
            status: if index_scan.failures.is_empty() {
                StorageHealthStatus::Healthy
            } else {
                StorageHealthStatus::Degraded
            },
            detail: BoundedDetail::new(if index_scan.next_cursor.is_some() {
                "bounded index sample is clean; additional records remain and the sample is not a complete-store proof"
            } else {
                "bounded index sample reached the current end of the checked indexes; it does not prove artifact-content integrity"
            })?,
        });
        components.push(StorageComponentHealth {
            component: BoundedDetail::new("application_receipt_lifecycle")?,
            status: StorageHealthStatus::Healthy,
            detail: BoundedDetail::new(format!(
                "hot {}/{}; cold {}; archive generation {}; last archive time {}",
                receipt_status.hot_count,
                receipt_status.hot_bound,
                receipt_status.cold_count,
                receipt_status.archive_generation,
                receipt_status
                    .last_archived_at
                    .map_or_else(|| "none".to_owned(), |value| value.get().to_string())
            ))?,
        });
        components.extend(scan.failures);
        components.extend(index_scan.failures);
        Ok(StorageHealth {
            status,
            schema,
            observed_at,
            components,
        })
    }

    fn scan_integrity(
        &self,
        request: IntegrityScanRequest,
    ) -> Result<IntegrityScanResult, PersistenceError> {
        integrity_scan::scan(self, request)
    }
}
