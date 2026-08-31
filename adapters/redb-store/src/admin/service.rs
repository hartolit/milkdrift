use super::*;
use super::{
    cursor::{
        integrity_cursor_state, integrity_cursor_str, make_integrity_cursor, push_failure,
        scan_index_sample, storage_anchor, validate_integrity_cursor,
    },
    integrity::scan_index_integrity,
};
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
        let read = self.database().begin_read().map_err(error::redb)?;
        let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let signal_receipts = read.open_table(SIGNAL_RECEIPTS).map_err(error::redb)?;
        let metadata = read.open_table(METADATA).map_err(error::redb)?;
        let artifacts = read.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
        let artifact_manifest = read.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
        let anchor = storage_anchor(&read)?;
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
            let mut previous_event_position = if start_family == IntegrityScanFamily::RunEvents {
                request
                    .cursor
                    .as_ref()
                    .map(|cursor| -> Result<_, PersistenceError> {
                        let (_, key) = integrity_cursor_state(cursor)?;
                        let event = events.get(key).map_err(error::redb)?.and_then(|bytes| {
                            milkdrift_persistence::RunEventEnvelope::from_json(bytes.value()).ok()
                        });
                        Ok(event.map(|event| (event.run_id().clone(), event.sequence())))
                    })
                    .transpose()?
                    .flatten()
            } else {
                None
            };
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
                        let contiguous = match &previous_event_position {
                            Some((previous_run, previous_sequence))
                                if previous_run == event.run_id() =>
                            {
                                previous_sequence
                                    .next()
                                    .is_ok_and(|expected| expected == event.sequence())
                            }
                            Some(_) | None => event.sequence() == RunSequence::FIRST,
                        };
                        if !contiguous {
                            push_failure(
                                &mut result,
                                "journal_history",
                                "event table is not contiguous from sequence one within its run",
                            )?;
                        }
                        previous_event_position = Some((event.run_id().clone(), event.sequence()));
                        let expected_key =
                            codec::run_sequence(event.run_id().as_str(), event.sequence())?;
                        if key.value() != expected_key.as_slice() {
                            push_failure(
                                &mut result,
                                "journal",
                                "event key does not match its verified envelope",
                            )?;
                        } else {
                            if let Err(cause) =
                                crate::snapshot::validate_history_link(&read, &event)
                            {
                                push_failure(&mut result, "journal_history", &cause.to_string())?;
                            }
                            if let milkdrift_persistence::RunEventKind::SignalReceived {
                                signal,
                                ..
                            } = event.kind()
                            {
                                let receipt_key =
                                    codec::pair(event.run_id().as_str(), signal.as_str())?;
                                let indexed = signal_receipts
                                    .get(receipt_key.as_slice())
                                    .map_err(error::redb)?
                                    .map(|sequence| sequence.value());
                                if indexed != Some(event.sequence().get()) {
                                    push_failure(
                                        &mut result,
                                        "signal_indexes",
                                        "signal-received event is missing its exact receipt index",
                                    )?;
                                }
                            }
                            if let milkdrift_persistence::RunEventKind::NodeScheduled {
                                invocation,
                                ..
                            } = event.kind()
                            {
                                let invocation_key =
                                    crate::journal::invocation_fact_key(event.run_id(), invocation);
                                let indexed = metadata
                                    .get(invocation_key.as_str())
                                    .map_err(error::redb)?
                                    .map(|sequence| sequence.value());
                                if indexed != Some(event.sequence().get()) {
                                    push_failure(
                                        &mut result,
                                        "invocation_indexes",
                                        "node-scheduled event is missing its exact invocation fact",
                                    )?;
                                }
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
                        let manifest = artifact_manifest
                            .get(key.value())
                            .map_err(error::redb)?
                            .map(|bytes| json::decode(bytes.value(), "artifact manifest"))
                            .transpose();
                        if manifest.as_ref().ok() != Some(&Some(metadata.clone())) {
                            push_failure(
                                &mut result,
                                "artifact_indexes",
                                "artifact metadata is missing its exact authoritative manifest",
                            )?;
                        }
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
        if more_remaining {
            result.next_cursor = last_cursor;
        }
        Ok(result)
    }
}
