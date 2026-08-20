use std::ops::Bound;

use crate::{
    RedbStore, codec, error, json,
    schema::{
        ARTIFACT_ACCOUNTING, ARTIFACT_MANIFEST, ARTIFACT_METADATA, ARTIFACT_REFERENCES,
        ARTIFACT_TEMP_MANIFEST, ARTIFACT_TEMP_OWNERS, ARTIFACTS_BY_DIGEST, COMMAND_RESULTS,
        EVENT_CHECKSUMS, LEASE_ENTRIES, LEASE_INDEX, METADATA, NONTERMINAL_RUNS, REVISIONS,
        REVISIONS_BY_DIGEST, ROOT_SCOPES, RUN_ARTIFACT_OWNERSHIP, RUN_EVENTS, RUN_HEADS,
        RUN_SUMMARIES, RUNNABLE_ENTRIES, RUNNABLE_INDEX, SCHEMA_VERSION_KEY, SCOPES, TIMER_ENTRIES,
        TIMER_INDEX, VALUES, WORKSPACE_BUDGETS, WORKSPACE_USAGE,
    },
};
use milkdrift_blueprint::BlueprintRevisionDocument;
use milkdrift_persistence::{
    ArtifactPublicationId, BoundedDetail, IndexedRunState, IntegrityScanCursor,
    IntegrityScanFamily, IntegrityScanRequest, IntegrityScanResult, LeaseIndexEntry,
    PersistenceError, RevisionSummary, RunnableIndexEntry, STORAGE_SCHEMA_VERSION_V1, StorageAdmin,
    StorageComponentHealth, StorageHealth, StorageHealthStatus, StorageSchemaCompatibility,
    StorageSchemaInfo, TimerIndexEntry, TimestampMillis,
};
use milkdrift_workspace::{
    ArtifactMetadata, ArtifactReference, RunId, ScopeId, ScopeKind, WorkspaceBudget,
    WorkspaceScope, WorkspaceUsage, WorkspaceValueEntry,
};
const GLOBAL_ARTIFACT_BYTES_KEY: &str = "artifact_content_bytes";

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
        if more_remaining {
            result.next_cursor = last_cursor;
        }
        Ok(result)
    }
}

const INTEGRITY_CURSOR_VERSION: u8 = 1;
const INTEGRITY_CURSOR_PREFIX_BYTES: usize = 33;

fn make_integrity_cursor(
    family: IntegrityScanFamily,
    key: &[u8],
    verify_artifact_content: bool,
    anchor: [u8; 32],
) -> Result<IntegrityScanCursor, PersistenceError> {
    let mut opaque = Vec::with_capacity(INTEGRITY_CURSOR_PREFIX_BYTES.saturating_add(key.len()));
    opaque.push(INTEGRITY_CURSOR_VERSION);
    opaque.extend_from_slice(&anchor);
    opaque.extend_from_slice(key);
    IntegrityScanCursor::new(family, opaque, verify_artifact_content)
}

fn integrity_cursor_state(
    cursor: &IntegrityScanCursor,
) -> Result<([u8; 32], &[u8]), PersistenceError> {
    if cursor.after_key().len() <= INTEGRITY_CURSOR_PREFIX_BYTES
        || cursor.after_key()[0] != INTEGRITY_CURSOR_VERSION
    {
        return Err(PersistenceError::InvalidCursor(
            "integrity cursor has an invalid authenticated-anchor prefix".to_owned(),
        ));
    }
    let mut anchor = [0_u8; 32];
    anchor.copy_from_slice(&cursor.after_key()[1..INTEGRITY_CURSOR_PREFIX_BYTES]);
    Ok((anchor, &cursor.after_key()[INTEGRITY_CURSOR_PREFIX_BYTES..]))
}

fn integrity_cursor_anchor(
    cursor: Option<&IntegrityScanCursor>,
) -> Result<[u8; 32], PersistenceError> {
    let cursor = cursor.ok_or_else(|| {
        error::corruption("integrity scan lost its authenticated root anchor")
    })?;
    integrity_cursor_state(cursor).map(|(anchor, _)| anchor)
}

fn integrity_cursor_str<'a>(
    cursor: &'a IntegrityScanCursor,
    family: &str,
) -> Result<&'a str, PersistenceError> {
    let (_, state) = integrity_cursor_state(cursor)?;
    std::str::from_utf8(state).map_err(|_| {
        PersistenceError::InvalidCursor(format!("{family} integrity cursor is not valid UTF-8"))
    })
}

fn scan_index_sample(
    store: &RedbStore,
    maximum: u64,
) -> Result<IntegrityScanResult, PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    let mut result = IntegrityScanResult {
        documents_checked: 0,
        artifacts_checked: 0,
        failures: Vec::new(),
        next_cursor: None,
    };
    let mut last_cursor = None;
    let mut more_remaining = false;
    let anchor = crate::trie::root_anchor(&read)?;
    scan_index_integrity(
        &read,
        None,
        maximum,
        false,
        anchor,
        &mut result,
        &mut last_cursor,
        &mut more_remaining,
    )?;
    if more_remaining {
        result.next_cursor = last_cursor;
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn scan_index_integrity(
    read: &redb::ReadTransaction,
    cursor: Option<&IntegrityScanCursor>,
    maximum: u64,
    verify_artifact_content: bool,
    anchor: [u8; 32],
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
) -> Result<(), PersistenceError> {
    const HEADS: u8 = 0;
    const SUMMARIES: u8 = 1;
    const NONTERMINAL: u8 = 2;
    const COMMANDS: u8 = 3;
    const RUNNABLE_IDENTITIES: u8 = 4;
    const RUNNABLE_ORDERED: u8 = 5;
    const TIMER_IDENTITIES: u8 = 6;
    const TIMER_ORDERED: u8 = 7;
    const LEASE_IDENTITIES: u8 = 8;
    const LEASE_ORDERED: u8 = 9;
    const SCOPES_PHASE: u8 = 10;
    const VALUES_PHASE: u8 = 11;
    const USAGE_PHASE: u8 = 12;
    const BUDGET_PHASE: u8 = 13;
    const ROOT_SCOPES_PHASE: u8 = 14;
    const REVISION_DIGESTS_PHASE: u8 = 15;
    const ARTIFACT_MANIFEST_PHASE: u8 = 16;
    const ARTIFACT_DIGESTS_PHASE: u8 = 17;
    const ARTIFACT_REFERENCES_PHASE: u8 = 18;
    const ARTIFACT_OWNERSHIP_PHASE: u8 = 19;
    const ARTIFACT_TEMP_MANIFEST_PHASE: u8 = 20;
    const ARTIFACT_ACCOUNTING_PHASE: u8 = 21;
    const ARTIFACT_TEMP_OWNERS_PHASE: u8 = 22;

    let (start_phase, start_key) = index_cursor_position(cursor)?;
    if last_cursor.is_none() {
        *last_cursor = Some(make_integrity_cursor(
            IntegrityScanFamily::Indexes,
            &[0],
            verify_artifact_content,
            anchor,
        )?);
    }
    let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let checksums = read.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
    let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let nonterminal = read.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
    let commands = read.open_table(COMMAND_RESULTS).map_err(error::redb)?;
    let runnable_identities = read.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let runnable_ordered = read.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let timer_identities = read.open_table(TIMER_ENTRIES).map_err(error::redb)?;
    let timer_ordered = read.open_table(TIMER_INDEX).map_err(error::redb)?;
    let lease_identities = read.open_table(LEASE_ENTRIES).map_err(error::redb)?;
    let lease_ordered = read.open_table(LEASE_INDEX).map_err(error::redb)?;
    let scopes = read.open_table(SCOPES).map_err(error::redb)?;
    let root_scopes = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
    let values = read.open_table(VALUES).map_err(error::redb)?;
    let usage = read.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    let budgets = read.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
    let revision_digests = read.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
    let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
    let artifact_metadata = read.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    let artifact_manifest = read.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
    let artifacts_by_digest = read.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
    let artifact_references = read.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
    let artifact_ownership = read
        .open_table(RUN_ARTIFACT_OWNERSHIP)
        .map_err(error::redb)?;
    let artifact_temp_manifest = read
        .open_table(ARTIFACT_TEMP_MANIFEST)
        .map_err(error::redb)?;
    let artifact_temp_owners = read.open_table(ARTIFACT_TEMP_OWNERS).map_err(error::redb)?;
    let artifact_accounting = read.open_table(ARTIFACT_ACCOUNTING).map_err(error::redb)?;

    if let Err(cause) = crate::trie::validate_roots(read) {
        push_failure(result, "authenticated_catalog", &cause.to_string())?;
    }
    for (family, table, label) in [
        (
            crate::trie::CatalogFamily::RunnableIdentity,
            &runnable_identities,
            "runnable_identity_catalog",
        ),
        (
            crate::trie::CatalogFamily::RunnableOrdered,
            &runnable_ordered,
            "runnable_ordered_catalog",
        ),
        (
            crate::trie::CatalogFamily::TimerIdentity,
            &timer_identities,
            "timer_identity_catalog",
        ),
        (
            crate::trie::CatalogFamily::TimerOrdered,
            &timer_ordered,
            "timer_ordered_catalog",
        ),
        (
            crate::trie::CatalogFamily::LeaseIdentity,
            &lease_identities,
            "lease_identity_catalog",
        ),
        (
            crate::trie::CatalogFamily::LeaseOrdered,
            &lease_ordered,
            "lease_ordered_catalog",
        ),
    ] {
        if let Err(cause) = validate_catalog_prefix(read, family, table, 4) {
            push_failure(result, label, &cause.to_string())?;
        }
    }
    if let Err(cause) = crate::journal::validate_workspace_value_accounting(read) {
        push_failure(result, "workspace_value_accounting", &cause.to_string())?;
    }

    scan_string_u64_phase(
        HEADS,
        start_phase,
        start_key,
        &heads,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "run_indexes",
        |key, stored_head| {
            let run = RunId::new(key).map_err(|cause| {
                error::corruption(format!("invalid run-head identity: {cause}"))
            })?;
            let head = crate::journal::validated_run_head(&heads, &events, &run)?;
            if head.get() != stored_head {
                return Err(error::corruption(
                    "run-head key changed within one read transaction",
                ));
            }
            let summary_bytes = summaries
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("run head is missing its summary"))?;
            let summary: milkdrift_persistence::RunSummaryIndex =
                json::decode(summary_bytes.value(), "run summary")?;
            if summary.run != run || summary.through_sequence != head {
                return Err(error::corruption(
                    "run summary does not match its authoritative head",
                ));
            }
            validate_nonterminal_marker(&nonterminal, &summary)?;
            let usage_bytes = usage
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("run head is missing workspace usage"))?;
            let budget_bytes = budgets
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("run head is missing its workspace budget"))?;
            let usage: WorkspaceUsage = json::decode(usage_bytes.value(), "workspace usage")?;
            let _budget: WorkspaceBudget = json::decode(budget_bytes.value(), "workspace budget")?;
            let authenticated = crate::journal::validated_workspace_domain(read, &run)?
                .ok_or_else(|| error::corruption("run head has no authenticated workspace domain"))?;
            if authenticated != usage {
                return Err(error::corruption(
                    "run head workspace usage disagrees with its authenticated domain",
                ));
            }
            Ok(())
        },
    )?;
    scan_string_bytes_phase(
        SUMMARIES,
        start_phase,
        start_key,
        &summaries,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "run_indexes",
        |key, bytes| {
            let summary: milkdrift_persistence::RunSummaryIndex =
                json::decode(bytes, "run summary")?;
            if summary.run.as_str() != key {
                return Err(error::corruption(
                    "run-summary key does not match its document",
                ));
            }
            let head = crate::journal::validated_run_head(&heads, &events, &summary.run)?;
            if heads.get(key).map_err(error::redb)?.is_none() || head != summary.through_sequence {
                return Err(error::corruption(
                    "run summary does not match an authoritative head",
                ));
            }
            validate_nonterminal_marker(&nonterminal, &summary)
        },
    )?;
    scan_string_u8_phase(
        NONTERMINAL,
        start_phase,
        start_key,
        &nonterminal,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "run_indexes",
        |key, marker| {
            if marker != 1 {
                return Err(error::corruption(
                    "nonterminal index contains an invalid marker",
                ));
            }
            let run = RunId::new(key).map_err(|cause| {
                error::corruption(format!("invalid nonterminal run identity: {cause}"))
            })?;
            let _head = crate::journal::validated_run_head(&heads, &events, &run)?;
            if heads.get(key).map_err(error::redb)?.is_none() {
                return Err(error::corruption(
                    "nonterminal index names a run without an authoritative head",
                ));
            }
            let bytes = summaries
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("nonterminal index is missing its summary"))?;
            let summary: milkdrift_persistence::RunSummaryIndex =
                json::decode(bytes.value(), "run summary")?;
            if summary.run != run || summary.state == IndexedRunState::Terminal {
                return Err(error::corruption(
                    "nonterminal index disagrees with its run summary",
                ));
            }
            Ok(())
        },
    )?;
    scan_binary_bytes_phase(
        COMMANDS,
        start_phase,
        start_key,
        &commands,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "command_indexes",
        |key, bytes| {
            crate::journal::validate_stored_command_record(key, bytes, &heads, &events, &checksums)
        },
    )?;

    scan_binary_bytes_phase(
        RUNNABLE_IDENTITIES,
        start_phase,
        start_key,
        &runnable_identities,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "runnable_indexes",
        |key, bytes| {
            let entry: RunnableIndexEntry = json::decode(bytes, "runnable index")?;
            let identity = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
            let ordered = crate::journal::runnable_order_key(&entry)?;
            validate_paired_index_row(
                key,
                bytes,
                &identity,
                &ordered,
                &runnable_ordered,
                "runnable",
            )
        },
    )?;
    scan_binary_bytes_phase(
        RUNNABLE_ORDERED,
        start_phase,
        start_key,
        &runnable_ordered,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "runnable_indexes",
        |key, bytes| {
            let entry: RunnableIndexEntry = json::decode(bytes, "runnable index")?;
            let ordered = crate::journal::runnable_order_key(&entry)?;
            let identity = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
            validate_paired_index_row(
                key,
                bytes,
                &ordered,
                &identity,
                &runnable_identities,
                "runnable",
            )
        },
    )?;
    scan_binary_bytes_phase(
        TIMER_IDENTITIES,
        start_phase,
        start_key,
        &timer_identities,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "timer_indexes",
        |key, bytes| {
            let entry: TimerIndexEntry = json::decode(bytes, "timer index")?;
            let identity = codec::pair(entry.run.as_str(), entry.timer.as_str())?;
            let ordered = crate::journal::timer_order_key(&entry)?;
            validate_paired_index_row(key, bytes, &identity, &ordered, &timer_ordered, "timer")
        },
    )?;
    scan_binary_bytes_phase(
        TIMER_ORDERED,
        start_phase,
        start_key,
        &timer_ordered,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "timer_indexes",
        |key, bytes| {
            let entry: TimerIndexEntry = json::decode(bytes, "timer index")?;
            let ordered = crate::journal::timer_order_key(&entry)?;
            let identity = codec::pair(entry.run.as_str(), entry.timer.as_str())?;
            validate_paired_index_row(key, bytes, &ordered, &identity, &timer_identities, "timer")
        },
    )?;
    scan_binary_bytes_phase(
        LEASE_IDENTITIES,
        start_phase,
        start_key,
        &lease_identities,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "lease_indexes",
        |key, bytes| {
            let entry: LeaseIndexEntry = json::decode(bytes, "lease index")?;
            let identity = codec::pair(entry.run.as_str(), entry.lease.as_str())?;
            let ordered = crate::journal::lease_order_key(&entry)?;
            validate_paired_index_row(key, bytes, &identity, &ordered, &lease_ordered, "lease")
        },
    )?;
    scan_binary_bytes_phase(
        LEASE_ORDERED,
        start_phase,
        start_key,
        &lease_ordered,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "lease_indexes",
        |key, bytes| {
            let entry: LeaseIndexEntry = json::decode(bytes, "lease index")?;
            let ordered = crate::journal::lease_order_key(&entry)?;
            let identity = codec::pair(entry.run.as_str(), entry.lease.as_str())?;
            validate_paired_index_row(key, bytes, &ordered, &identity, &lease_identities, "lease")
        },
    )?;
    scan_binary_bytes_phase(
        SCOPES_PHASE,
        start_phase,
        start_key,
        &scopes,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "workspace_indexes",
        |key, bytes| {
            let scope: WorkspaceScope = json::decode(bytes, "workspace scope")?;
            let expected = codec::pair(
                scope.reference().run().as_str(),
                scope.reference().scope().as_str(),
            )?;
            if key != expected.as_slice() {
                return Err(error::corruption(
                    "workspace-scope key does not match its checked document",
                ));
            }
            match (scope.kind(), scope.parent()) {
                (&ScopeKind::RunRoot, None) => match root_scopes
                    .get(scope.reference().run().as_str())
                    .map_err(error::redb)?
                {
                    Some(root) if root.value() == scope.reference().scope().as_str() => {}
                    _ => {
                        return Err(error::corruption(
                            "run-root scope is missing or mismatched in its root index",
                        ));
                    }
                },
                (&ScopeKind::RunRoot, Some(_)) => {
                    return Err(error::corruption("run-root scope has a parent"));
                }
                (_, Some(parent)) => {
                    crate::journal::validate_owning_workspace_scope(&scopes, &root_scopes, parent)?;
                }
                (_, None) => {
                    return Err(error::corruption("non-root workspace scope has no parent"));
                }
            }
            Ok(())
        },
    )?;
    scan_binary_bytes_phase(
        VALUES_PHASE,
        start_phase,
        start_key,
        &values,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "workspace_indexes",
        |key, bytes| {
            let entry: WorkspaceValueEntry = json::decode(bytes, "workspace value")?;
            let expected = crate::journal::workspace_value_key(entry.reference())?;
            if key != expected.as_slice() {
                return Err(error::corruption(
                    "workspace-value key does not match its checked document",
                ));
            }
            crate::journal::validate_workspace_value_provenance(
                &values,
                &scopes,
                &root_scopes,
                &entry,
                false,
            )
        },
    )?;
    scan_string_bytes_phase(
        USAGE_PHASE,
        start_phase,
        start_key,
        &usage,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "workspace_indexes",
        |key, bytes| {
            let usage: WorkspaceUsage = json::decode(bytes, "workspace usage")?;
            let run = RunId::new(key).map_err(|cause| {
                error::corruption(format!("invalid workspace-usage run identity: {cause}"))
            })?;
            if budgets.get(key).map_err(error::redb)?.is_none() {
                return Err(error::corruption(
                    "workspace usage is missing its immutable budget",
                ));
            }
            let authenticated = crate::journal::validated_workspace_domain(read, &run)?
                .ok_or_else(|| error::corruption("workspace usage has no authenticated domain"))?;
            if authenticated != usage {
                return Err(error::corruption(
                    "workspace usage disagrees with its authenticated domain",
                ));
            }
            Ok(())
        },
    )?;
    scan_string_bytes_phase(
        BUDGET_PHASE,
        start_phase,
        start_key,
        &budgets,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "workspace_indexes",
        |key, bytes| {
            let _budget: WorkspaceBudget = json::decode(bytes, "workspace budget")?;
            let run = RunId::new(key).map_err(|cause| {
                error::corruption(format!("invalid workspace-budget run identity: {cause}"))
            })?;
            if usage.get(key).map_err(error::redb)?.is_none() {
                return Err(error::corruption(
                    "workspace budget is missing its usage accounting",
                ));
            }
            if crate::journal::validated_workspace_domain(read, &run)?.is_none() {
                return Err(error::corruption(
                    "workspace budget has no authenticated domain",
                ));
            }
            Ok(())
        },
    )?;
    scan_string_string_phase(
        ROOT_SCOPES_PHASE,
        start_phase,
        start_key,
        &root_scopes,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "workspace_indexes",
        |run, scope| {
            let run = RunId::new(run).map_err(|cause| {
                error::corruption(format!("invalid root-scope run identity: {cause}"))
            })?;
            let scope_id = ScopeId::new(scope).map_err(|cause| {
                error::corruption(format!("invalid root-scope identity: {cause}"))
            })?;
            let key = codec::pair(run.as_str(), scope_id.as_str())?;
            let bytes = scopes
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("root-scope index pointer is dangling"))?;
            let document: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
            if document.reference().run() != &run
                || document.reference().scope() != &scope_id
                || document.kind() != &ScopeKind::RunRoot
                || document.parent().is_some()
            {
                return Err(error::corruption(
                    "root-scope index does not match its checked root document",
                ));
            }
            if crate::journal::validated_workspace_domain(read, &run)?.is_none() {
                return Err(error::corruption(
                    "root workspace scope has no authenticated domain",
                ));
            }
            Ok(())
        },
    )?;
    scan_binary_bytes_phase(
        REVISION_DIGESTS_PHASE,
        start_phase,
        start_key,
        &revision_digests,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "revision_indexes",
        |key, bytes| {
            let summary = crate::revision::decode_summary(bytes)?;
            let expected = codec::pair(summary.content_digest.as_str(), summary.revision.as_str())?;
            if key != expected.as_slice() {
                return Err(error::corruption(
                    "revision digest key does not match its checked summary",
                ));
            }
            let document = revisions
                .get(summary.revision.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("revision digest index pointer is dangling"))?;
            let (_document, revision) = BlueprintRevisionDocument::from_json(document.value())
                .map_err(|cause| {
                    error::corruption(format!("stored revision failed verification: {cause}"))
                })?;
            if RevisionSummary::from(&revision) != summary {
                return Err(error::corruption(
                    "revision digest summary disagrees with its authoritative revision",
                ));
            }
            Ok(())
        },
    )?;
    scan_string_bytes_phase(
        ARTIFACT_MANIFEST_PHASE,
        start_phase,
        start_key,
        &artifact_manifest,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "artifact_indexes",
        |key, bytes| {
            let manifest: ArtifactMetadata = json::decode(bytes, "artifact manifest")?;
            if manifest.reference().artifact().as_str() != key {
                return Err(error::corruption(
                    "artifact manifest key does not match its checked document",
                ));
            }
            let metadata = artifact_metadata
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact manifest has no metadata row"))?;
            let metadata: ArtifactMetadata = json::decode(metadata.value(), "artifact metadata")?;
            if metadata != manifest {
                return Err(error::corruption(
                    "artifact manifest disagrees with its metadata row",
                ));
            }
            let digest = manifest.reference().digest().to_hex();
            let digest_key = codec::pair(&digest, key)?;
            let indexed = artifacts_by_digest
                .get(digest_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact manifest has no digest index row"))?;
            let indexed: ArtifactMetadata = json::decode(indexed.value(), "artifact metadata")?;
            if indexed != manifest {
                return Err(error::corruption(
                    "artifact digest index disagrees with its manifest",
                ));
            }
            Ok(())
        },
    )?;
    scan_artifact_digest_phase(
        ARTIFACT_DIGESTS_PHASE,
        start_phase,
        start_key,
        &artifacts_by_digest,
        &artifact_metadata,
        &artifact_manifest,
        &artifact_accounting,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
    )?;
    scan_binary_bytes_phase(
        ARTIFACT_REFERENCES_PHASE,
        start_phase,
        start_key,
        &artifact_references,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "artifact_indexes",
        |key, bytes| {
            let (digest, artifact, run) = artifact_occurrence_key(key)?;
            let reference: ArtifactReference = json::decode(bytes, "artifact reference")?;
            if digest != reference.digest().to_hex() || artifact != reference.artifact().as_str() {
                return Err(error::corruption(
                    "artifact-reference key does not match its checked document",
                ));
            }
            let ownership_key = codec::components(&[&run, &digest, &artifact])?;
            let owned = artifact_ownership
                .get(ownership_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact occurrence has no ownership row"))?;
            let owned: ArtifactReference = json::decode(owned.value(), "run artifact ownership")?;
            if owned != reference {
                return Err(error::corruption(
                    "artifact occurrence disagrees with its ownership row",
                ));
            }
            Ok(())
        },
    )?;
    scan_binary_bytes_phase(
        ARTIFACT_OWNERSHIP_PHASE,
        start_phase,
        start_key,
        &artifact_ownership,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "artifact_indexes",
        |key, bytes| {
            let components = codec::decode_components(key, 3)?;
            let reference: ArtifactReference = json::decode(bytes, "run artifact ownership")?;
            let digest = reference.digest().to_hex();
            if components[1] != digest || components[2] != reference.artifact().as_str() {
                return Err(error::corruption(
                    "run artifact-ownership key does not match its checked document",
                ));
            }
            let prefix = codec::components(&[&digest, components[2], components[0]])?;
            let end = codec::prefix_end(prefix.clone()).ok_or_else(|| {
                error::corruption("artifact-reference ownership prefix has no end")
            })?;
            let occurrence = artifact_references
                .range::<&[u8]>(prefix.as_slice()..end.as_slice())
                .map_err(error::redb)?
                .next()
                .transpose()
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact ownership has no occurrence row"))?;
            let occurrence: ArtifactReference =
                json::decode(occurrence.1.value(), "artifact reference")?;
            if occurrence != reference {
                return Err(error::corruption(
                    "artifact ownership disagrees with its occurrence row",
                ));
            }
            Ok(())
        },
    )?;
    scan_string_bytes_phase(
        ARTIFACT_TEMP_MANIFEST_PHASE,
        start_phase,
        start_key,
        &artifact_temp_manifest,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "artifact_indexes",
        |key, bytes| {
            let publication: ArtifactPublicationId =
                json::decode(bytes, "artifact temporary manifest")?;
            match artifact_temp_owners.get(key).map_err(error::redb)? {
                Some(owner) if owner.value() == publication.as_str() => Ok(()),
                _ => Err(error::corruption(
                    "artifact temporary manifest has no matching owner row",
                )),
            }
        },
    )?;
    scan_string_bytes_phase(
        ARTIFACT_ACCOUNTING_PHASE,
        start_phase,
        start_key,
        &artifact_accounting,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "artifact_indexes",
        |key, bytes| {
            if key != GLOBAL_ARTIFACT_BYTES_KEY {
                return Err(error::corruption(
                    "artifact accounting contains an unknown record",
                ));
            }
            let record: crate::artifact::ArtifactAccountingRecord =
                json::decode(bytes, "artifact accounting")?;
            if record.schema_version != 3 {
                return Err(PersistenceError::UnsupportedVersion {
                    document: "artifact_accounting",
                    found: record.schema_version,
                    supported: 3,
                });
            }
            Ok(())
        },
    )?;
    scan_string_string_phase(
        ARTIFACT_TEMP_OWNERS_PHASE,
        start_phase,
        start_key,
        &artifact_temp_owners,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "artifact_indexes",
        |key, owner| {
            let publication = ArtifactPublicationId::new(owner).map_err(|cause| {
                error::corruption(format!("invalid artifact temporary owner: {cause}"))
            })?;
            let manifested = artifact_temp_manifest
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("temporary owner has no manifest row"))?;
            let manifested: ArtifactPublicationId =
                json::decode(manifested.value(), "artifact temporary manifest")?;
            if manifested != publication {
                return Err(error::corruption(
                    "artifact temporary owner disagrees with its manifest",
                ));
            }
            Ok(())
        },
    )?;
    Ok(())
}

fn validate_catalog_prefix<T>(
    read: &redb::ReadTransaction,
    family: crate::trie::CatalogFamily,
    table: &T,
    limit: usize,
) -> Result<(), PersistenceError>
where
    T: redb::ReadableTable<&'static [u8], &'static [u8]>,
{
    let page = crate::trie::page(read, family, None, None, limit)?;
    for leaf in page.leaves {
        let bytes = table
            .get(leaf.logical_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authenticated catalog leaf is dangling"))?;
        if leaf.payload_digest != crate::trie::digest_payload(family, bytes.value()) {
            return Err(error::corruption(
                "authenticated catalog payload disagrees with its physical row",
            ));
        }
    }
    Ok(())
}

fn validate_nonterminal_marker(
    index: &impl redb::ReadableTable<&'static str, u8>,
    summary: &milkdrift_persistence::RunSummaryIndex,
) -> Result<(), PersistenceError> {
    let marker = index
        .get(summary.run.as_str())
        .map_err(error::redb)?
        .map(|value| value.value());
    match (summary.state, marker) {
        (IndexedRunState::Terminal, None) | (_, Some(1)) => Ok(()),
        (IndexedRunState::Terminal, Some(_)) => Err(error::corruption(
            "terminal run is present in the nonterminal index",
        )),
        (_, None) => Err(error::corruption(
            "nonterminal run is missing from its discovery index",
        )),
        (_, Some(_)) => Err(error::corruption(
            "nonterminal index contains an invalid marker",
        )),
    }
}

fn validate_paired_index_row(
    actual_key: &[u8],
    actual_value: &[u8],
    expected_key: &[u8],
    paired_key: &[u8],
    paired: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    family: &str,
) -> Result<(), PersistenceError> {
    if actual_key != expected_key {
        return Err(error::corruption(format!(
            "{family} index key does not match its checked document"
        )));
    }
    match paired.get(paired_key).map_err(error::redb)? {
        Some(value) if value.value() == actual_value => Ok(()),
        _ => Err(error::corruption(format!(
            "{family} identity/ordered index pair is missing or mismatched"
        ))),
    }
}

fn artifact_occurrence_key(key: &[u8]) -> Result<(String, String, String), PersistenceError> {
    let components = match codec::decode_components(key, 4) {
        Ok(components) => components,
        Err(_) => {
            let components = codec::decode_components(key, 5)?;
            if components[3] != "publication" {
                return Err(error::corruption(
                    "five-part artifact occurrence key has an unknown owner kind",
                ));
            }
            components
        }
    };
    Ok((
        components[0].to_owned(),
        components[1].to_owned(),
        components[2].to_owned(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn scan_artifact_digest_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    by_digest: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    metadata: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    manifest: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    accounting: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
) -> Result<(), PersistenceError> {
    if *more_remaining || phase < start_phase {
        return Ok(());
    }
    let (mut total, mut current_digest, mut current_size, after_key) = if phase == start_phase {
        let state = start_key.ok_or_else(|| {
            PersistenceError::InvalidCursor(
                "artifact digest integrity cursor has no state".to_owned(),
            )
        })?;
        parse_artifact_digest_cursor(state)?
    } else {
        (0, None, 0, None)
    };
    let lower = after_key.map_or(Bound::Unbounded, Bound::Excluded);
    for item in by_digest
        .range::<&[u8]>((lower, Bound::Unbounded))
        .map_err(error::redb)?
    {
        if result.documents_checked == maximum {
            *more_remaining = true;
            break;
        }
        let (key, value) = item.map_err(error::redb)?;
        result.documents_checked += 1;
        let checked = (|| {
            let components = codec::decode_components(key.value(), 2)?;
            let document: ArtifactMetadata = json::decode(value.value(), "artifact metadata")?;
            let digest = document.reference().digest().to_hex();
            let artifact = document.reference().artifact().as_str();
            if components[0] != digest || components[1] != artifact {
                return Err(error::corruption(
                    "artifact digest key does not match its checked metadata",
                ));
            }
            let primary = metadata
                .get(artifact)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact digest index has no metadata row"))?;
            let primary: ArtifactMetadata = json::decode(primary.value(), "artifact metadata")?;
            let manifested = manifest
                .get(artifact)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact digest index has no manifest row"))?;
            let manifested: ArtifactMetadata =
                json::decode(manifested.value(), "artifact manifest")?;
            if primary != document || manifested != document {
                return Err(error::corruption(
                    "artifact digest index disagrees with metadata or manifest",
                ));
            }
            Ok((digest, document.reference().size_bytes()))
        })();
        match checked {
            Ok((digest, size)) => {
                if current_digest.as_deref() == Some(digest.as_str()) {
                    if current_size != size {
                        push_failure(
                            result,
                            "artifact_indexes",
                            "artifact metadata disagrees on size for one content digest",
                        )?;
                    }
                } else {
                    match total.checked_add(size) {
                        Some(next) => total = next,
                        None => push_failure(
                            result,
                            "artifact_indexes",
                            "derived artifact content-byte total overflows",
                        )?,
                    }
                    current_digest = Some(digest);
                    current_size = size;
                }
            }
            Err(cause) => push_failure(result, "artifact_indexes", &cause.to_string())?,
        }
        *last_cursor = Some(make_artifact_digest_cursor(
            phase,
            key.value(),
            total,
            current_digest.as_deref(),
            current_size,
            verify_artifact_content,
            last_cursor.as_ref(),
        )?);
    }
    if !*more_remaining {
        let stored = accounting
            .get(GLOBAL_ARTIFACT_BYTES_KEY)
            .map_err(error::redb)?
            .map(|bytes| {
                json::decode::<crate::artifact::ArtifactAccountingRecord>(
                    bytes.value(),
                    "artifact accounting",
                )
            })
            .transpose();
        match stored {
            Ok(None) if total == 0 => {}
            Ok(Some(record))
                if record.schema_version == 3 && record.committed_content_bytes == total => {}
            Ok(Some(record)) if record.schema_version != 3 => push_failure(
                result,
                "artifact_indexes",
                "artifact accounting has an unsupported schema version",
            )?,
            Ok(_) => push_failure(
                result,
                "artifact_indexes",
                "artifact accounting does not equal the derived unique-digest byte total",
            )?,
            Err(cause) => push_failure(result, "artifact_indexes", &cause.to_string())?,
        }
    }
    Ok(())
}

fn make_artifact_digest_cursor(
    phase: u8,
    physical_key: &[u8],
    total: u64,
    digest: Option<&str>,
    size: u64,
    verify_artifact_content: bool,
    prior: Option<&IntegrityScanCursor>,
) -> Result<IntegrityScanCursor, PersistenceError> {
    let digest = digest.unwrap_or("").as_bytes();
    let digest_length = u16::try_from(digest.len()).map_err(|_| PersistenceError::Bounds {
        location: "integrity_cursor",
        reason: "artifact digest cursor state exceeds u16".to_owned(),
    })?;
    let mut opaque = Vec::with_capacity(
        1_usize
            .saturating_add(8)
            .saturating_add(8)
            .saturating_add(2)
            .saturating_add(digest.len())
            .saturating_add(physical_key.len()),
    );
    opaque.push(phase);
    opaque.extend_from_slice(&total.to_be_bytes());
    opaque.extend_from_slice(&size.to_be_bytes());
    opaque.extend_from_slice(&digest_length.to_be_bytes());
    opaque.extend_from_slice(digest);
    opaque.extend_from_slice(physical_key);
    make_integrity_cursor(
        IntegrityScanFamily::Indexes,
        &opaque,
        verify_artifact_content,
        integrity_cursor_anchor(prior)?,
    )
}

type ArtifactDigestCursorState<'a> = (u64, Option<String>, u64, Option<&'a [u8]>);

fn parse_artifact_digest_cursor(
    state: &[u8],
) -> Result<ArtifactDigestCursorState<'_>, PersistenceError> {
    if state.len() < 18 {
        return Err(PersistenceError::InvalidCursor(
            "artifact digest integrity cursor state is truncated".to_owned(),
        ));
    }
    let total = u64::from_be_bytes(state[0..8].try_into().map_err(|_| {
        PersistenceError::InvalidCursor("artifact digest total is malformed".to_owned())
    })?);
    let size = u64::from_be_bytes(state[8..16].try_into().map_err(|_| {
        PersistenceError::InvalidCursor("artifact digest size is malformed".to_owned())
    })?);
    let digest_length = usize::from(u16::from_be_bytes(state[16..18].try_into().map_err(
        |_| PersistenceError::InvalidCursor("artifact digest length is malformed".to_owned()),
    )?));
    let digest_end = 18_usize.checked_add(digest_length).ok_or_else(|| {
        PersistenceError::InvalidCursor("artifact digest cursor length overflows".to_owned())
    })?;
    let digest_bytes = state.get(18..digest_end).ok_or_else(|| {
        PersistenceError::InvalidCursor("artifact digest cursor is truncated".to_owned())
    })?;
    let digest = if digest_bytes.is_empty() {
        None
    } else {
        Some(
            std::str::from_utf8(digest_bytes)
                .map_err(|_| {
                    PersistenceError::InvalidCursor(
                        "artifact digest cursor is not valid UTF-8".to_owned(),
                    )
                })?
                .to_owned(),
        )
    };
    let key = state
        .get(digest_end..)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            PersistenceError::InvalidCursor("artifact digest cursor has no physical key".to_owned())
        })?;
    Ok((total, digest, size, Some(key)))
}

fn index_cursor_position(
    cursor: Option<&IntegrityScanCursor>,
) -> Result<(u8, Option<&[u8]>), PersistenceError> {
    let Some(cursor) = cursor else {
        return Ok((0, None));
    };
    let (_, state) = integrity_cursor_state(cursor)?;
    let Some((&phase, key)) = state.split_first() else {
        return Err(PersistenceError::InvalidCursor(
            "index integrity cursor has no phase".to_owned(),
        ));
    };
    if phase > 22 || key.is_empty() {
        return Err(PersistenceError::InvalidCursor(
            "index integrity cursor has an unknown phase or empty key".to_owned(),
        ));
    }
    Ok((phase, Some(key)))
}

fn make_index_cursor(
    phase: u8,
    key: &[u8],
    verify_artifact_content: bool,
    prior: Option<&IntegrityScanCursor>,
) -> Result<IntegrityScanCursor, PersistenceError> {
    let mut opaque = Vec::with_capacity(key.len().saturating_add(1));
    opaque.push(phase);
    opaque.extend_from_slice(key);
    make_integrity_cursor(
        IntegrityScanFamily::Indexes,
        &opaque,
        verify_artifact_content,
        integrity_cursor_anchor(prior)?,
    )
}

#[allow(clippy::too_many_arguments)]
fn scan_binary_bytes_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl FnMut(&[u8], &[u8]) -> Result<(), PersistenceError>,
) -> Result<(), PersistenceError> {
    if *more_remaining || phase < start_phase {
        return Ok(());
    }
    let lower = if phase == start_phase {
        start_key.map_or(Bound::Unbounded, Bound::Excluded)
    } else {
        Bound::Unbounded
    };
    for item in table
        .range::<&[u8]>((lower, Bound::Unbounded))
        .map_err(error::redb)?
    {
        if result.documents_checked == maximum {
            *more_remaining = true;
            break;
        }
        let (key, value) = item.map_err(error::redb)?;
        result.documents_checked += 1;
        *last_cursor = Some(make_index_cursor(
            phase,
            key.value(),
            verify_artifact_content,
            last_cursor.as_ref(),
        )?);
        if let Err(cause) = validate(key.value(), value.value()) {
            push_failure(result, component, &cause.to_string())?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn scan_string_bytes_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl FnMut(&str, &[u8]) -> Result<(), PersistenceError>,
) -> Result<(), PersistenceError> {
    if *more_remaining || phase < start_phase {
        return Ok(());
    }
    let lower = if phase == start_phase {
        start_key
            .map(|key| std::str::from_utf8(key))
            .transpose()
            .map_err(|_| {
                PersistenceError::InvalidCursor(
                    "string index integrity cursor is not valid UTF-8".to_owned(),
                )
            })?
            .map_or(Bound::Unbounded, Bound::Excluded)
    } else {
        Bound::Unbounded
    };
    for item in table
        .range::<&str>((lower, Bound::Unbounded))
        .map_err(error::redb)?
    {
        if result.documents_checked == maximum {
            *more_remaining = true;
            break;
        }
        let (key, value) = item.map_err(error::redb)?;
        result.documents_checked += 1;
        *last_cursor = Some(make_index_cursor(
            phase,
            key.value().as_bytes(),
            verify_artifact_content,
            last_cursor.as_ref(),
        )?);
        if let Err(cause) = validate(key.value(), value.value()) {
            push_failure(result, component, &cause.to_string())?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn scan_string_string_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static str, &'static str>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl FnMut(&str, &str) -> Result<(), PersistenceError>,
) -> Result<(), PersistenceError> {
    if *more_remaining || phase < start_phase {
        return Ok(());
    }
    let lower = if phase == start_phase {
        start_key
            .map(|key| std::str::from_utf8(key))
            .transpose()
            .map_err(|_| {
                PersistenceError::InvalidCursor(
                    "string index integrity cursor is not valid UTF-8".to_owned(),
                )
            })?
            .map_or(Bound::Unbounded, Bound::Excluded)
    } else {
        Bound::Unbounded
    };
    for item in table
        .range::<&str>((lower, Bound::Unbounded))
        .map_err(error::redb)?
    {
        if result.documents_checked == maximum {
            *more_remaining = true;
            break;
        }
        let (key, value) = item.map_err(error::redb)?;
        result.documents_checked += 1;
        *last_cursor = Some(make_index_cursor(
            phase,
            key.value().as_bytes(),
            verify_artifact_content,
            last_cursor.as_ref(),
        )?);
        if let Err(cause) = validate(key.value(), value.value()) {
            push_failure(result, component, &cause.to_string())?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn scan_string_u64_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static str, u64>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl FnMut(&str, u64) -> Result<(), PersistenceError>,
) -> Result<(), PersistenceError> {
    scan_string_scalar_phase(
        phase,
        start_phase,
        start_key,
        table,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        component,
        |value| value,
        &mut validate,
    )
}

#[allow(clippy::too_many_arguments)]
fn scan_string_u8_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static str, u8>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    mut validate: impl FnMut(&str, u8) -> Result<(), PersistenceError>,
) -> Result<(), PersistenceError> {
    scan_string_scalar_phase(
        phase,
        start_phase,
        start_key,
        table,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        component,
        |value| value,
        &mut validate,
    )
}

#[allow(clippy::too_many_arguments)]
fn scan_string_scalar_phase<V: redb::Value + 'static, T: Copy>(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    table: &impl redb::ReadableTable<&'static str, V>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
    component: &str,
    scalar: impl for<'a> Fn(V::SelfType<'a>) -> T,
    validate: &mut impl FnMut(&str, T) -> Result<(), PersistenceError>,
) -> Result<(), PersistenceError> {
    if *more_remaining || phase < start_phase {
        return Ok(());
    }
    let lower = if phase == start_phase {
        start_key
            .map(|key| std::str::from_utf8(key))
            .transpose()
            .map_err(|_| {
                PersistenceError::InvalidCursor(
                    "string index integrity cursor is not valid UTF-8".to_owned(),
                )
            })?
            .map_or(Bound::Unbounded, Bound::Excluded)
    } else {
        Bound::Unbounded
    };
    for item in table
        .range::<&str>((lower, Bound::Unbounded))
        .map_err(error::redb)?
    {
        if result.documents_checked == maximum {
            *more_remaining = true;
            break;
        }
        let (key, value) = item.map_err(error::redb)?;
        result.documents_checked += 1;
        *last_cursor = Some(make_index_cursor(
            phase,
            key.value().as_bytes(),
            verify_artifact_content,
            last_cursor.as_ref(),
        )?);
        if let Err(cause) = validate(key.value(), scalar(value.value())) {
            push_failure(result, component, &cause.to_string())?;
        }
    }
    Ok(())
}

fn validate_integrity_cursor(
    request: &IntegrityScanRequest,
    read: &redb::ReadTransaction,
    revisions: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    events: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    artifacts: &impl redb::ReadableTable<&'static str, &'static [u8]>,
) -> Result<(), PersistenceError> {
    let Some(cursor) = request.cursor.as_ref() else {
        return Ok(());
    };
    if cursor.verify_artifact_content() != request.verify_artifact_content {
        return Err(PersistenceError::InvalidCursor(
            "integrity cursor belongs to a different artifact-verification mode".to_owned(),
        ));
    }
    let (cursor_anchor, cursor_key) = integrity_cursor_state(cursor)?;
    if cursor_anchor != crate::trie::root_anchor(read)? {
        return Err(PersistenceError::InvalidCursor(
            "integrity cursor belongs to a different authenticated storage root".to_owned(),
        ));
    }
    let exists = match cursor.family() {
        IntegrityScanFamily::Revisions => revisions
            .get(integrity_cursor_str(cursor, "revision")?)
            .map_err(error::redb)?
            .is_some(),
        IntegrityScanFamily::RunEvents => events
            .get(cursor_key)
            .map_err(error::redb)?
            .is_some(),
        IntegrityScanFamily::Artifacts => artifacts
            .get(integrity_cursor_str(cursor, "artifact")?)
            .map_err(error::redb)?
            .is_some(),
        IntegrityScanFamily::Indexes => index_integrity_cursor_exists(read, cursor)?,
    };
    if !exists {
        return Err(PersistenceError::InvalidCursor(
            "integrity cursor does not name a durable record".to_owned(),
        ));
    }
    Ok(())
}

fn index_integrity_cursor_exists(
    read: &redb::ReadTransaction,
    cursor: &IntegrityScanCursor,
) -> Result<bool, PersistenceError> {
    let (phase, key) = index_cursor_position(Some(cursor))?;
    let key = key.ok_or_else(|| {
        PersistenceError::InvalidCursor("index integrity cursor is missing its key".to_owned())
    })?;
    let string_key = || {
        std::str::from_utf8(key).map_err(|_| {
            PersistenceError::InvalidCursor(
                "string index integrity cursor is not valid UTF-8".to_owned(),
            )
        })
    };
    match phase {
        0 => read
            .open_table(RUN_HEADS)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        1 => read
            .open_table(RUN_SUMMARIES)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        2 => read
            .open_table(NONTERMINAL_RUNS)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        3 => binary_cursor_exists(read, COMMAND_RESULTS, key),
        4 => binary_cursor_exists(read, RUNNABLE_ENTRIES, key),
        5 => binary_cursor_exists(read, RUNNABLE_INDEX, key),
        6 => binary_cursor_exists(read, TIMER_ENTRIES, key),
        7 => binary_cursor_exists(read, TIMER_INDEX, key),
        8 => binary_cursor_exists(read, LEASE_ENTRIES, key),
        9 => binary_cursor_exists(read, LEASE_INDEX, key),
        10 => binary_cursor_exists(read, SCOPES, key),
        11 => binary_cursor_exists(read, VALUES, key),
        12 => read
            .open_table(WORKSPACE_USAGE)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        13 => read
            .open_table(WORKSPACE_BUDGETS)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        14 => read
            .open_table(ROOT_SCOPES)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        15 => binary_cursor_exists(read, REVISIONS_BY_DIGEST, key),
        16 => read
            .open_table(ARTIFACT_MANIFEST)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        17 => {
            let (_total, _digest, _size, physical_key) = parse_artifact_digest_cursor(key)?;
            binary_cursor_exists(
                read,
                ARTIFACTS_BY_DIGEST,
                physical_key.ok_or_else(|| {
                    PersistenceError::InvalidCursor(
                        "artifact digest cursor has no physical key".to_owned(),
                    )
                })?,
            )
        }
        18 => binary_cursor_exists(read, ARTIFACT_REFERENCES, key),
        19 => binary_cursor_exists(read, RUN_ARTIFACT_OWNERSHIP, key),
        20 => read
            .open_table(ARTIFACT_TEMP_MANIFEST)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        21 => read
            .open_table(ARTIFACT_ACCOUNTING)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        22 => read
            .open_table(ARTIFACT_TEMP_OWNERS)
            .map_err(error::redb)?
            .get(string_key()?)
            .map_err(error::redb)
            .map(|row| row.is_some()),
        _ => Err(PersistenceError::InvalidCursor(
            "index integrity cursor has an unknown phase".to_owned(),
        )),
    }
}

fn binary_cursor_exists(
    read: &redb::ReadTransaction,
    definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    key: &[u8],
) -> Result<bool, PersistenceError> {
    read.open_table(definition)
        .map_err(error::redb)?
        .get(key)
        .map_err(error::redb)
        .map(|row| row.is_some())
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
