use milkdrift_blueprint::BlueprintRevisionDocument;
use milkdrift_persistence::{
    BoundedDetail, IntegrityScanRequest, IntegrityScanResult, PersistenceError,
    STORAGE_SCHEMA_VERSION_V1, StorageAdmin, StorageComponentHealth, StorageHealth,
    StorageHealthStatus, StorageSchemaCompatibility, StorageSchemaInfo, TimestampMillis,
};
use milkdrift_workspace::ArtifactMetadata;
use redb::{ReadableTable, ReadableTableMetadata};

use crate::{
    RedbStore, codec, error, json,
    schema::{
        ARTIFACT_METADATA, EVENT_CHECKSUMS, METADATA, REVISIONS, RUN_EVENTS, SCHEMA_VERSION_KEY,
    },
};

impl StorageAdmin for RedbStore {
    fn schema_info(&self) -> Result<StorageSchemaInfo, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(METADATA).map_err(error::redb)?;
        let found = table
            .get(SCHEMA_VERSION_KEY)
            .map_err(error::redb)?
            .map(|value| value.value())
            .ok_or_else(|| error::corruption("storage schema version is missing"))?;
        let stored_version = u32::try_from(found).unwrap_or(u32::MAX);
        let compatibility = if stored_version == STORAGE_SCHEMA_VERSION_V1 {
            StorageSchemaCompatibility::Current
        } else if stored_version < STORAGE_SCHEMA_VERSION_V1 {
            StorageSchemaCompatibility::MigrationRequired
        } else {
            StorageSchemaCompatibility::FutureUnsupported
        };
        Ok(StorageSchemaInfo {
            stored_version,
            current_version: STORAGE_SCHEMA_VERSION_V1,
            compatibility,
        })
    }

    fn migrate_to_current(
        &self,
        expected_from: u32,
    ) -> Result<StorageSchemaInfo, PersistenceError> {
        let info = self.schema_info()?;
        if info.stored_version != expected_from {
            return Err(PersistenceError::InvalidDocument(format!(
                "migration expected schema {expected_from}, found {}",
                info.stored_version
            )));
        }
        match info.compatibility {
            StorageSchemaCompatibility::Current => Ok(info),
            StorageSchemaCompatibility::FutureUnsupported => {
                Err(PersistenceError::UnsupportedVersion {
                    document: "storage",
                    found: info.stored_version,
                    supported: info.current_version,
                })
            }
            StorageSchemaCompatibility::MigrationRequired => {
                // Schema v1 is the first physical schema. There is no implicit v0
                // table guessing and therefore no supported older migration yet.
                Err(PersistenceError::MigrationRequired {
                    found: info.stored_version,
                    target: info.current_version,
                })
            }
        }
    }

    fn health(&self, observed_at: TimestampMillis) -> Result<StorageHealth, PersistenceError> {
        let schema = self.schema_info()?;
        let scan = self.scan_integrity(IntegrityScanRequest {
            limit: milkdrift_persistence::PageSize::new(32)?,
            verify_artifact_content: false,
        })?;
        let status = if scan.failures.is_empty() {
            StorageHealthStatus::Healthy
        } else {
            StorageHealthStatus::Degraded
        };
        let mut components = Vec::with_capacity(scan.failures.len().saturating_add(1));
        components.push(StorageComponentHealth {
            component: BoundedDetail::new("storage_schema")?,
            status: StorageHealthStatus::Healthy,
            detail: BoundedDetail::new(format!(
                "physical schema {} is current",
                schema.stored_version
            ))?,
        });
        components.extend(scan.failures);
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
        let read = self.database().begin_read().map_err(error::redb)?;
        let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let checksums = read.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
        let artifacts = read.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
        let total = revisions
            .len()
            .map_err(error::redb)?
            .saturating_add(events.len().map_err(error::redb)?)
            .saturating_add(artifacts.len().map_err(error::redb)?);
        let maximum = u64::from(request.limit.get());
        let mut result = IntegrityScanResult {
            documents_checked: 0,
            artifacts_checked: 0,
            failures: Vec::new(),
            more_remaining: total > maximum,
        };

        for item in revisions.iter().map_err(error::redb)? {
            if result.documents_checked == maximum {
                break;
            }
            result.documents_checked += 1;
            let (key, bytes) = item.map_err(error::redb)?;
            match BlueprintRevisionDocument::from_json(bytes.value()) {
                Ok((_document, revision)) if revision.id().as_str() == key.value() => {}
                Ok(_) => push_failure(
                    &mut result,
                    "revision",
                    "revision key does not match its verified document",
                )?,
                Err(cause) => push_failure(&mut result, "revision", &cause.to_string())?,
            }
        }
        if result.documents_checked < maximum {
            for item in events.iter().map_err(error::redb)? {
                if result.documents_checked == maximum {
                    break;
                }
                result.documents_checked += 1;
                let (key, bytes) = item.map_err(error::redb)?;
                match milkdrift_persistence::RunEventEnvelope::from_json(bytes.value()) {
                    Ok(event) => {
                        let expected_key =
                            codec::run_sequence(event.run_id().as_str(), event.sequence())?;
                        if key.value() != expected_key.as_slice() {
                            push_failure(
                                &mut result,
                                "journal",
                                "event key does not match its verified envelope",
                            )?;
                        } else {
                            match checksums
                                .get(event.event_id().as_str())
                                .map_err(error::redb)?
                            {
                                Some(checksum) if checksum.value() == event.checksum().as_str() => {
                                }
                                _ => push_failure(
                                    &mut result,
                                    "journal",
                                    "event checksum index is missing or mismatched",
                                )?,
                            }
                        }
                    }
                    Err(cause) => push_failure(&mut result, "journal", &cause.to_string())?,
                }
            }
        }
        if result.documents_checked < maximum {
            for item in artifacts.iter().map_err(error::redb)? {
                if result.documents_checked == maximum {
                    break;
                }
                result.documents_checked += 1;
                let (key, bytes) = item.map_err(error::redb)?;
                let metadata: Result<ArtifactMetadata, _> =
                    json::decode(bytes.value(), "artifact metadata");
                match metadata {
                    Ok(metadata) if metadata.reference().artifact().as_str() == key.value() => {
                        if request.verify_artifact_content {
                            result.artifacts_checked += 1;
                            if let Err(cause) = crate::artifact::verify_blob(
                                &self.content_path(metadata.reference().digest()),
                                metadata.reference(),
                                self.max_artifact_bytes,
                            ) {
                                push_failure(&mut result, "artifact_content", &cause.to_string())?;
                            }
                        }
                    }
                    Ok(_) => push_failure(
                        &mut result,
                        "artifact_metadata",
                        "artifact key does not match its document",
                    )?,
                    Err(cause) => {
                        push_failure(&mut result, "artifact_metadata", &cause.to_string())?
                    }
                }
            }
        }
        Ok(result)
    }
}

fn push_failure(
    result: &mut IntegrityScanResult,
    component: &str,
    detail: &str,
) -> Result<(), PersistenceError> {
    result.failures.push(StorageComponentHealth {
        component: BoundedDetail::new(component)?,
        status: StorageHealthStatus::Degraded,
        detail: bounded_detail(detail)?,
    });
    Ok(())
}

fn bounded_detail(detail: &str) -> Result<BoundedDetail, PersistenceError> {
    let mut detail: String = detail
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if detail.len() > milkdrift_persistence::MAX_DETAIL_BYTES {
        let mut boundary = milkdrift_persistence::MAX_DETAIL_BYTES;
        while !detail.is_char_boundary(boundary) {
            boundary -= 1;
        }
        detail.truncate(boundary);
    }
    BoundedDetail::new(detail)
}
