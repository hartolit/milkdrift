use super::*;
use super::{
    cursor::{
        integrity_cursor_state, integrity_cursor_str, make_integrity_cursor, push_failure,
        scan_index_sample, validate_integrity_cursor,
    },
    integrity::{scan_authenticated_catalog_integrity, scan_index_integrity},
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

    fn health(&self, observed_at: TimestampMillis) -> Result<StorageHealth, PersistenceError> {
        let schema = self.schema_info()?;
        let scan = self.scan_integrity(IntegrityScanRequest {
            limit: milkdrift_persistence::PageSize::new(32)?,
            verify_artifact_content: false,
            cursor: None,
        })?;
        let index_scan = scan_index_sample(self, 32)?;
        let status = if scan.failures.is_empty() && index_scan.failures.is_empty() {
            StorageHealthStatus::Healthy
        } else {
            StorageHealthStatus::Degraded
        };
        let mut components = Vec::with_capacity(
            scan.failures
                .len()
                .saturating_add(index_scan.failures.len())
                .saturating_add(3),
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
                "bounded integrity sample is clean; additional records remain for a complete scan"
            } else {
                "bounded integrity check reached the current end of storage"
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
                "bounded index sample is clean; additional records remain for a complete scan"
            } else {
                "bounded index check reached the current end of every checked index"
            })?,
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
        let read = self.database().begin_read().map_err(error::redb)?;
        let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let checksums = read.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
        let artifacts = read.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
        let anchor = crate::trie::root_anchor(&read)?;
        validate_integrity_cursor(&request, &read, &revisions, &events, &artifacts)?;
        let maximum = u64::from(request.limit.get());
        let mut result = IntegrityScanResult {
            documents_checked: 0,
            artifacts_checked: 0,
            failures: Vec::new(),
            next_cursor: None,
        };
        let start_family = request
            .cursor
            .as_ref()
            .map_or(IntegrityScanFamily::Revisions, IntegrityScanCursor::family);
        let mut last_cursor = None;
        let mut more_remaining = false;

        if start_family <= IntegrityScanFamily::Revisions {
            let lower = if start_family == IntegrityScanFamily::Revisions {
                request
                    .cursor
                    .as_ref()
                    .map(|cursor| integrity_cursor_str(cursor, "revision"))
                    .transpose()?
                    .map_or(Bound::Unbounded, Bound::Excluded)
            } else {
                Bound::Unbounded
            };
            for item in revisions
                .range::<&str>((lower, Bound::Unbounded))
                .map_err(error::redb)?
            {
                if result.documents_checked == maximum {
                    more_remaining = true;
                    break;
                }
                result.documents_checked += 1;
                let (key, bytes) = item.map_err(error::redb)?;
                last_cursor = Some(make_integrity_cursor(
                    IntegrityScanFamily::Revisions,
                    key.value().as_bytes(),
                    request.verify_artifact_content,
                    anchor,
                )?);
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
        }
        if !more_remaining && start_family <= IntegrityScanFamily::RunEvents {
            let lower = if start_family == IntegrityScanFamily::RunEvents {
                request
                    .cursor
                    .as_ref()
                    .map(|cursor| integrity_cursor_state(cursor).map(|(_, key)| key))
                    .transpose()?
                    .map_or(Bound::Unbounded, Bound::Excluded)
            } else {
                Bound::Unbounded
            };
            for item in events
                .range::<&[u8]>((lower, Bound::Unbounded))
                .map_err(error::redb)?
            {
                if result.documents_checked == maximum {
                    more_remaining = true;
                    break;
                }
                result.documents_checked += 1;
                let (key, bytes) = item.map_err(error::redb)?;
                last_cursor = Some(make_integrity_cursor(
                    IntegrityScanFamily::RunEvents,
                    key.value(),
                    request.verify_artifact_content,
                    anchor,
                )?);
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
        if !more_remaining && start_family <= IntegrityScanFamily::Artifacts {
            let lower = if start_family == IntegrityScanFamily::Artifacts {
                request
                    .cursor
                    .as_ref()
                    .map(|cursor| integrity_cursor_str(cursor, "artifact"))
                    .transpose()?
                    .map_or(Bound::Unbounded, Bound::Excluded)
            } else {
                Bound::Unbounded
            };
            for item in artifacts
                .range::<&str>((lower, Bound::Unbounded))
                .map_err(error::redb)?
            {
                if result.documents_checked == maximum {
                    more_remaining = true;
                    break;
                }
                result.documents_checked += 1;
                let (key, bytes) = item.map_err(error::redb)?;
                last_cursor = Some(make_integrity_cursor(
                    IntegrityScanFamily::Artifacts,
                    key.value().as_bytes(),
                    request.verify_artifact_content,
                    anchor,
                )?);
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
        if !more_remaining && start_family <= IntegrityScanFamily::Indexes {
            scan_index_integrity(
                &read,
                if start_family == IntegrityScanFamily::Indexes {
                    request.cursor.as_ref()
                } else {
                    None
                },
                maximum,
                request.verify_artifact_content,
                anchor,
                &mut result,
                &mut last_cursor,
                &mut more_remaining,
            )?;
        }
        if !more_remaining && start_family <= IntegrityScanFamily::AuthenticatedCatalogs {
            scan_authenticated_catalog_integrity(
                &read,
                if start_family == IntegrityScanFamily::AuthenticatedCatalogs {
                    request.cursor.as_ref()
                } else {
                    None
                },
                maximum,
                request.verify_artifact_content,
                anchor,
                &mut result,
                &mut last_cursor,
                &mut more_remaining,
            )?;
        }
        if more_remaining {
            result.next_cursor = last_cursor;
        }
        Ok(result)
    }
}
