use super::*;
pub(crate) const LEGACY_GLOBAL_ARTIFACT_BYTES_KEY: &str = "artifact_content_bytes";
const DISCOVERY_ACCOUNTING_KEY: &str = "active_index_counts";
const DISCOVERY_ACCOUNTING_SCHEMA_VERSION: u32 = 1;
const WORKSPACE_VALUE_TOTAL_KEY: &str = "";
const WORKSPACE_VALUE_ACCOUNTING_SCHEMA_VERSION: u32 = 1;
const INTEGRITY_ACCOUNTING_KEY: &str = "global_counts";
const INTEGRITY_ACCOUNTING_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyDiscoveryAccounting {
    schema_version: u32,
    runnable_count: u64,
    timer_count: u64,
    lease_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyWorkspaceValueAccountingRecord {
    schema_version: u32,
    value_versions: u64,
    inline_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyWorkspaceGlobalAccountingRecord {
    schema_version: u32,
    value_versions: u64,
    inline_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyIntegrityAccounting {
    schema_version: u32,
    run_count: u64,
    event_count: u64,
    command_count: u64,
    revision_count: u64,
}

impl LegacyWorkspaceValueAccountingRecord {
    const fn from_usage(usage: milkdrift_workspace::WorkspaceUsage) -> Self {
        Self {
            schema_version: WORKSPACE_VALUE_ACCOUNTING_SCHEMA_VERSION,
            value_versions: usage.value_versions(),
            inline_bytes: usage.inline_bytes(),
        }
    }
}

fn load_legacy_discovery_accounting<T>(
    table: &T,
) -> Result<LegacyDiscoveryAccounting, PersistenceError>
where
    T: redb::ReadableTable<&'static str, &'static [u8]> + redb::ReadableTableMetadata,
{
    if table.len().map_err(error::redb)? != 1 {
        return Err(error::corruption(
            "discovery accounting must contain exactly one checked document",
        ));
    }
    let bytes = table
        .get(DISCOVERY_ACCOUNTING_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("discovery accounting document is missing"))?;
    let accounting: LegacyDiscoveryAccounting =
        crate::json::decode(bytes.value(), "discovery accounting")?;
    if accounting.schema_version != DISCOVERY_ACCOUNTING_SCHEMA_VERSION {
        return Err(error::corruption(
            "discovery accounting has an unsupported document version",
        ));
    }
    Ok(accounting)
}

fn load_legacy_workspace_value_accounting<T>(
    table: &T,
    key: &str,
) -> Result<LegacyWorkspaceValueAccountingRecord, PersistenceError>
where
    T: redb::ReadableTable<&'static str, &'static [u8]>,
{
    let bytes = table
        .get(key)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("workspace value accounting document is missing"))?;
    let accounting: LegacyWorkspaceValueAccountingRecord =
        crate::json::decode(bytes.value(), "workspace value accounting")?;
    if accounting.schema_version != WORKSPACE_VALUE_ACCOUNTING_SCHEMA_VERSION {
        return Err(error::corruption(
            "workspace value accounting has an unsupported document version",
        ));
    }
    Ok(accounting)
}

fn load_legacy_integrity_accounting<T>(
    table: &T,
) -> Result<LegacyIntegrityAccounting, PersistenceError>
where
    T: redb::ReadableTable<&'static str, &'static [u8]> + redb::ReadableTableMetadata,
{
    if table.len().map_err(error::redb)? != 1 {
        return Err(error::corruption(
            "integrity accounting must contain exactly one checked document",
        ));
    }
    let bytes = table
        .get(INTEGRITY_ACCOUNTING_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("integrity accounting document is missing"))?;
    let accounting: LegacyIntegrityAccounting =
        crate::json::decode(bytes.value(), "integrity accounting")?;
    if accounting.schema_version != INTEGRITY_ACCOUNTING_SCHEMA_VERSION {
        return Err(error::corruption(
            "integrity accounting has an unsupported document version",
        ));
    }
    Ok(accounting)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MigratedArtifactAccountingRecord {
    schema_version: u32,
    committed_content_bytes: u64,
}
pub(crate) fn migrate_internal_documents(
    database: &Database,
    faults: &dyn FaultInjector,
) -> Result<(), PersistenceError> {
    let write = database.begin_write().map_err(error::redb)?;
    let observed = {
        let metadata = write.open_table(METADATA).map_err(error::redb)?;
        metadata
            .get(INTERNAL_DOCUMENT_FORMAT_VERSION_KEY)
            .map_err(error::redb)?
            .map(|value| value.value())
    };
    let observed_version = observed.unwrap_or(0);
    let migrate_legacy_raw_documents = match observed {
        Some(found) if found > INTERNAL_DOCUMENT_FORMAT_VERSION => {
            return Err(PersistenceError::UnsupportedVersion {
                document: "redb internal document envelope",
                found: u32::try_from(found).unwrap_or(u32::MAX),
                supported: INTERNAL_DOCUMENT_FORMAT_VERSION as u32,
            });
        }
        Some(found) if found == INTERNAL_DOCUMENT_FORMAT_VERSION => return Ok(()),
        Some(1 | 2) => false,
        Some(0) | None => true,
        Some(_) => {
            return Err(error::corruption(
                "redb internal document format has an unsupported historical version",
            ));
        }
    };

    crate::trie::initialize(&write)?;

    if migrate_legacy_raw_documents {
        migrate_binary_json_table(&write, REVISIONS_BY_DIGEST, "revision summary")?;
        migrate_binary_json_table(&write, COMMAND_RESULTS, "command record")?;
        migrate_string_json_table(&write, RUN_SUMMARIES, "run summary")?;
        migrate_binary_json_table(&write, RUNNABLE_ENTRIES, "runnable index")?;
        migrate_binary_json_table(&write, RUNNABLE_INDEX, "runnable index")?;
        migrate_binary_json_table(&write, TIMER_ENTRIES, "timer index")?;
        migrate_binary_json_table(&write, TIMER_INDEX, "timer index")?;
        migrate_binary_json_table(&write, LEASE_ENTRIES, "lease index")?;
        migrate_binary_json_table(&write, LEASE_INDEX, "lease index")?;
        migrate_binary_json_table(&write, SCOPES, "workspace scope")?;
        migrate_binary_json_table(&write, VALUES, "workspace value")?;
        migrate_string_json_table(&write, ARTIFACT_METADATA, "artifact metadata")?;
        migrate_string_json_table(&write, ARTIFACT_MANIFEST, "artifact manifest")?;
        migrate_string_json_table(&write, ARTIFACT_PUBLICATIONS, "artifact publication")?;
        migrate_string_json_table(
            &write,
            ARTIFACT_TEMP_MANIFEST,
            "artifact temporary manifest",
        )?;
        migrate_binary_json_table(&write, ARTIFACTS_BY_DIGEST, "artifact metadata")?;
        migrate_binary_json_table(&write, ARTIFACT_REFERENCES, "artifact reference")?;
        migrate_binary_json_table(&write, RUN_ARTIFACT_OWNERSHIP, "run artifact ownership")?;
        migrate_string_json_table(&write, WORKSPACE_USAGE, "workspace usage")?;
        migrate_string_json_table(&write, WORKSPACE_BUDGETS, "workspace budget")?;
        migrate_string_json_table(&write, ARTIFACT_ACCOUNTING, "artifact accounting")?;
        backfill_artifact_integrity_documents(&write)?;
    }
    backfill_discovery_accounting(&write)?;
    backfill_integrity_accounting(&write)?;
    crate::snapshot::migrate_snapshot_catalogs(&write)?;
    if observed_version == 2 {
        validate_v2_workspace_value_accounting(&write)?;
    } else {
        let accounting = write
            .open_table(WORKSPACE_VALUE_ACCOUNTING)
            .map_err(error::redb)?;
        if accounting.len().map_err(error::redb)? != 0 {
            return Err(error::corruption(
                "pre-v2 storage unexpectedly contains workspace value accounting",
            ));
        }
    }
    crate::artifact::materialize_legacy_writable_workspace_domains(&write)?;
    let _validated_scope_counts = validate_migrated_workspace_scopes(&write)?;
    crate::journal::migrate_workspace_catalogs(&write)?;
    retire_workspace_value_accounting(&write)?;
    crate::artifact::upgrade_artifact_accounting(&write)?;

    {
        let mut metadata = write.open_table(METADATA).map_err(error::redb)?;
        metadata
            .insert(
                INTERNAL_DOCUMENT_FORMAT_VERSION_KEY,
                INTERNAL_DOCUMENT_FORMAT_VERSION,
            )
            .map_err(error::redb)?;
    }
    faults.check(crate::fault::FaultPoint::BeforeMigrationCommit)?;
    write.commit().map_err(error::redb)?;
    faults.check(crate::fault::FaultPoint::AfterMigrationCommit)
}

pub(crate) fn backfill_discovery_accounting(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    let runnable_entries = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let runnable_ordered = write.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let timer_entries = write.open_table(TIMER_ENTRIES).map_err(error::redb)?;
    let timer_ordered = write.open_table(TIMER_INDEX).map_err(error::redb)?;
    let lease_entries = write.open_table(LEASE_ENTRIES).map_err(error::redb)?;
    let lease_ordered = write.open_table(LEASE_INDEX).map_err(error::redb)?;
    for item in runnable_entries.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let entry: milkdrift_persistence::RunnableIndexEntry =
            crate::json::decode(bytes.value(), "runnable index")?;
        let identity = crate::codec::pair(entry.run.as_str(), entry.execution.as_str())?;
        let ordered = crate::journal::runnable_order_key(&entry)?;
        validate_migrated_discovery_pair(
            key.value(),
            bytes.value(),
            &identity,
            &ordered,
            &runnable_ordered,
            "runnable",
        )?;
        migrate_discovery_catalog_leaf(
            write,
            crate::trie::CatalogFamily::RunnableIdentity,
            crate::journal::runnable_catalog_identity_path(&entry.run, key.value())?,
            crate::trie::CatalogFamily::RunnableOrdered,
            crate::journal::runnable_catalog_ordered_path(key.value(), &entry)?,
            key.value(),
            bytes.value(),
            "runnable",
        )?;
    }
    for item in runnable_ordered.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let entry: milkdrift_persistence::RunnableIndexEntry =
            crate::json::decode(bytes.value(), "runnable index")?;
        let identity = crate::codec::pair(entry.run.as_str(), entry.execution.as_str())?;
        let ordered = crate::journal::runnable_order_key(&entry)?;
        validate_migrated_discovery_pair(
            key.value(),
            bytes.value(),
            &ordered,
            &identity,
            &runnable_entries,
            "runnable",
        )?;
    }
    for item in timer_entries.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let entry: milkdrift_persistence::TimerIndexEntry =
            crate::json::decode(bytes.value(), "timer index")?;
        let identity = crate::codec::pair(entry.run.as_str(), entry.timer.as_str())?;
        let ordered = crate::journal::timer_order_key(&entry)?;
        validate_migrated_discovery_pair(
            key.value(),
            bytes.value(),
            &identity,
            &ordered,
            &timer_ordered,
            "timer",
        )?;
        let identity_family = crate::trie::CatalogFamily::TimerIdentity;
        migrate_discovery_catalog_leaf(
            write,
            identity_family,
            crate::trie::hashed_path(identity_family, key.value()),
            crate::trie::CatalogFamily::TimerOrdered,
            crate::journal::timer_catalog_ordered_path(key.value(), &entry)?,
            key.value(),
            bytes.value(),
            "timer",
        )?;
    }
    for item in timer_ordered.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let entry: milkdrift_persistence::TimerIndexEntry =
            crate::json::decode(bytes.value(), "timer index")?;
        let identity = crate::codec::pair(entry.run.as_str(), entry.timer.as_str())?;
        let ordered = crate::journal::timer_order_key(&entry)?;
        validate_migrated_discovery_pair(
            key.value(),
            bytes.value(),
            &ordered,
            &identity,
            &timer_entries,
            "timer",
        )?;
    }
    for item in lease_entries.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let entry: milkdrift_persistence::LeaseIndexEntry =
            crate::json::decode(bytes.value(), "lease index")?;
        let identity = crate::codec::pair(entry.run.as_str(), entry.lease.as_str())?;
        let ordered = crate::journal::lease_order_key(&entry)?;
        validate_migrated_discovery_pair(
            key.value(),
            bytes.value(),
            &identity,
            &ordered,
            &lease_ordered,
            "lease",
        )?;
        let identity_family = crate::trie::CatalogFamily::LeaseIdentity;
        migrate_discovery_catalog_leaf(
            write,
            identity_family,
            crate::trie::hashed_path(identity_family, key.value()),
            crate::trie::CatalogFamily::LeaseOrdered,
            crate::journal::lease_catalog_ordered_path(key.value(), &entry)?,
            key.value(),
            bytes.value(),
            "lease",
        )?;
    }
    for item in lease_ordered.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let entry: milkdrift_persistence::LeaseIndexEntry =
            crate::json::decode(bytes.value(), "lease index")?;
        let identity = crate::codec::pair(entry.run.as_str(), entry.lease.as_str())?;
        let ordered = crate::journal::lease_order_key(&entry)?;
        validate_migrated_discovery_pair(
            key.value(),
            bytes.value(),
            &ordered,
            &identity,
            &lease_entries,
            "lease",
        )?;
    }
    let runnable_count = runnable_entries.len().map_err(error::redb)?;
    let timer_count = timer_entries.len().map_err(error::redb)?;
    let lease_count = lease_entries.len().map_err(error::redb)?;
    if runnable_count != runnable_ordered.len().map_err(error::redb)?
        || timer_count != timer_ordered.len().map_err(error::redb)?
        || lease_count != lease_ordered.len().map_err(error::redb)?
    {
        return Err(error::corruption(
            "legacy discovery identity and ordered indexes have different cardinality",
        ));
    }
    drop(lease_ordered);
    drop(lease_entries);
    drop(timer_ordered);
    drop(timer_entries);
    drop(runnable_ordered);
    drop(runnable_entries);
    crate::journal::migrate_runnable_run_heads(write)?;

    let expected = LegacyDiscoveryAccounting {
        schema_version: DISCOVERY_ACCOUNTING_SCHEMA_VERSION,
        runnable_count,
        timer_count,
        lease_count,
    };
    let mut accounting = write
        .open_table(DISCOVERY_ACCOUNTING)
        .map_err(error::redb)?;
    if accounting.len().map_err(error::redb)? != 0
        && load_legacy_discovery_accounting(&accounting)? != expected
    {
        return Err(error::corruption(
            "existing discovery accounting disagrees with legacy index cardinalities",
        ));
    }
    let _removed = accounting
        .remove(DISCOVERY_ACCOUNTING_KEY)
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn validate_migrated_discovery_pair<T>(
    actual_key: &[u8],
    actual_value: &[u8],
    expected_key: &[u8],
    paired_key: &[u8],
    paired: &T,
    family: &'static str,
) -> Result<(), PersistenceError>
where
    T: redb::ReadableTable<&'static [u8], &'static [u8]>,
{
    if actual_key != expected_key {
        return Err(error::corruption(format!(
            "legacy {family} index key disagrees with its checked document"
        )));
    }
    let paired_value = paired
        .get(paired_key)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption(format!("legacy {family} index pair is incomplete")))?;
    if paired_value.value() != actual_value {
        return Err(error::corruption(format!(
            "legacy {family} identity and ordered rows disagree"
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn migrate_discovery_catalog_leaf(
    write: &redb::WriteTransaction,
    identity_family: crate::trie::CatalogFamily,
    identity_path: [u8; 32],
    ordered_family: crate::trie::CatalogFamily,
    ordered_path: [u8; 32],
    logical_key: &[u8],
    bytes: &[u8],
    label: &'static str,
) -> Result<(), PersistenceError> {
    for (family, path) in [
        (identity_family, identity_path),
        (ordered_family, ordered_path),
    ] {
        let payload = crate::trie::digest_payload(family, bytes);
        if crate::trie::put(write, family, path, logical_key, payload)?.is_some() {
            return Err(error::corruption(format!(
                "legacy {label} catalog contains a duplicate authenticated identity"
            )));
        }
    }
    Ok(())
}

pub(crate) fn backfill_integrity_accounting(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    let heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
    let checksums = write.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
    let event_count = events.len().map_err(error::redb)?;
    if checksums.len().map_err(error::redb)? != event_count {
        return Err(error::corruption(
            "legacy event rows and checksum index have different cardinality",
        ));
    }
    let mut current_run: Option<milkdrift_workspace::RunId> = None;
    let mut current_sequence = milkdrift_persistence::RunSequence::ZERO;
    let mut event_run_count = 0_u64;
    for item in events.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let key_bytes = key.value().to_vec();
        let event_bytes = bytes.value().to_vec();
        let event = RunEventEnvelope::from_json(&event_bytes).map_err(|cause| {
            error::corruption(format!("legacy run event failed verification: {cause}"))
        })?;
        if key_bytes.as_slice()
            != crate::codec::run_sequence(event.run_id().as_str(), event.sequence())?.as_slice()
        {
            return Err(error::corruption(
                "legacy event key does not match its verified envelope",
            ));
        }
        let checksum = checksums
            .get(event.event_id().as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("legacy event checksum index is incomplete"))?;
        if checksum.value() != event.checksum().as_str() {
            return Err(error::corruption(
                "legacy event checksum index disagrees with its envelope",
            ));
        }
        if current_run.as_ref() != Some(event.run_id()) {
            if let Some(previous_run) = current_run.as_ref() {
                validate_migrated_run_head(&heads, previous_run, current_sequence)?;
                crate::journal::persist_run_membership(
                    write,
                    previous_run,
                    current_sequence,
                    None,
                )?;
                crate::journal::migrate_nonterminal_membership(
                    write,
                    previous_run,
                    current_sequence,
                )?;
            }
            if event.sequence() != milkdrift_persistence::RunSequence::FIRST {
                return Err(error::corruption(
                    "legacy event stream does not begin at sequence one",
                ));
            }
            current_run = Some(event.run_id().clone());
            current_sequence = event.sequence();
            event_run_count = event_run_count
                .checked_add(1)
                .ok_or_else(|| error::corruption("legacy event run count overflowed"))?;
        } else {
            let expected = current_sequence.next()?;
            if event.sequence() != expected {
                return Err(error::corruption(
                    "legacy event stream contains a sequence gap",
                ));
            }
            current_sequence = event.sequence();
        }
        let family = crate::trie::CatalogFamily::Event;
        if crate::trie::put(
            write,
            family,
            crate::journal::event_catalog_path(event.run_id(), event.sequence(), &key_bytes)?,
            &key_bytes,
            crate::trie::digest_payload(family, &event_bytes),
        )?
        .is_some()
        {
            return Err(error::corruption(
                "legacy event catalog contains a duplicate authenticated event",
            ));
        }
        crate::snapshot::append_history_checkpoint(write, &event)?;
    }
    if let Some(previous_run) = current_run.as_ref() {
        validate_migrated_run_head(&heads, previous_run, current_sequence)?;
        crate::journal::persist_run_membership(write, previous_run, current_sequence, None)?;
        crate::journal::migrate_nonterminal_membership(write, previous_run, current_sequence)?;
    }
    if heads.len().map_err(error::redb)? != event_run_count {
        return Err(error::corruption(
            "legacy run heads and event streams have different run cardinality",
        ));
    }
    let summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    if summaries.len().map_err(error::redb)? != event_run_count {
        return Err(error::corruption(
            "legacy run summaries and event streams have different run cardinality",
        ));
    }
    drop(summaries);

    let commands = write.open_table(COMMAND_RESULTS).map_err(error::redb)?;
    let command_count = commands.len().map_err(error::redb)?;
    for item in commands.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        crate::journal::validate_stored_command_record(
            key.value(),
            bytes.value(),
            &heads,
            &events,
            &checksums,
        )?;
        let family = crate::trie::CatalogFamily::Command;
        if crate::trie::put(
            write,
            family,
            crate::trie::hashed_path(family, key.value()),
            key.value(),
            crate::trie::digest_payload(family, bytes.value()),
        )?
        .is_some()
        {
            return Err(error::corruption(
                "legacy command catalog contains a duplicate identity",
            ));
        }
    }
    drop(commands);
    drop(checksums);
    drop(heads);
    drop(events);
    let revisions = write.open_table(REVISIONS).map_err(error::redb)?;
    let by_digest = write.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
    let revision_count = revisions.len().map_err(error::redb)?;
    if by_digest.len().map_err(error::redb)? != revision_count {
        return Err(error::corruption(
            "legacy revision primary and digest tables have different cardinality",
        ));
    }
    for item in revisions.iter().map_err(error::redb)? {
        let (key, document) = item.map_err(error::redb)?;
        let revision = crate::revision::decode_revision(document.value())?;
        if key.value() != revision.id().as_str() {
            return Err(error::corruption(
                "legacy revision key disagrees with its verified document",
            ));
        }
        let digest_key =
            crate::codec::pair(revision.content_digest().as_str(), revision.id().as_str())?;
        let summary = by_digest
            .get(digest_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("legacy revision digest index is incomplete"))?;
        if crate::revision::decode_summary(summary.value())? != RevisionSummary::from(&revision) {
            return Err(error::corruption(
                "legacy revision digest summary disagrees with its revision",
            ));
        }
        crate::revision::migrate_revision_catalog(
            write,
            &revision,
            document.value(),
            &digest_key,
            summary.value(),
        )?;
    }
    for item in by_digest.iter().map_err(error::redb)? {
        let (key, summary) = item.map_err(error::redb)?;
        let summary = crate::revision::decode_summary(summary.value())?;
        let expected_key =
            crate::codec::pair(summary.content_digest.as_str(), summary.revision.as_str())?;
        if key.value() != expected_key.as_slice() {
            return Err(error::corruption(
                "legacy revision digest key disagrees with its summary",
            ));
        }
        let document = revisions
            .get(summary.revision.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("legacy revision digest index is dangling"))?;
        let revision = crate::revision::decode_revision(document.value())?;
        if RevisionSummary::from(&revision) != summary {
            return Err(error::corruption(
                "legacy revision digest row disagrees with its primary document",
            ));
        }
    }
    drop(by_digest);
    drop(revisions);

    let expected = LegacyIntegrityAccounting {
        schema_version: INTEGRITY_ACCOUNTING_SCHEMA_VERSION,
        run_count: event_run_count,
        event_count,
        command_count,
        revision_count,
    };
    let mut accounting = write
        .open_table(INTEGRITY_ACCOUNTING)
        .map_err(error::redb)?;
    if accounting.len().map_err(error::redb)? != 0
        && load_legacy_integrity_accounting(&accounting)? != expected
    {
        return Err(error::corruption(
            "legacy integrity accounting disagrees with verified physical records",
        ));
    }
    let _removed = accounting
        .remove(INTEGRITY_ACCOUNTING_KEY)
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn validate_migrated_run_head(
    heads: &impl redb::ReadableTable<&'static str, u64>,
    run: &milkdrift_workspace::RunId,
    sequence: milkdrift_persistence::RunSequence,
) -> Result<(), PersistenceError> {
    let stored = heads
        .get(run.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("legacy event stream has no authoritative run head"))?;
    if stored.value() != sequence.get() {
        return Err(error::corruption(
            "legacy event stream does not terminate at its authoritative run head",
        ));
    }
    Ok(())
}

pub(crate) fn validate_v2_workspace_value_accounting(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    let values = write.open_table(VALUES).map_err(error::redb)?;
    let usage = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    let accounting = write
        .open_table(WORKSPACE_VALUE_ACCOUNTING)
        .map_err(error::redb)?;
    let expected_rows = usage
        .len()
        .map_err(error::redb)?
        .checked_add(1)
        .ok_or_else(|| error::corruption("legacy workspace accounting row count overflowed"))?;
    if accounting.len().map_err(error::redb)? != expected_rows {
        return Err(error::corruption(
            "v2 workspace accounting does not have one row per usage document",
        ));
    }
    let global_bytes = accounting
        .get(WORKSPACE_VALUE_TOTAL_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("v2 global workspace accounting is missing"))?;
    let global: LegacyWorkspaceGlobalAccountingRecord =
        crate::json::decode(global_bytes.value(), "workspace value accounting")?;
    if global.schema_version != WORKSPACE_VALUE_ACCOUNTING_SCHEMA_VERSION
        || global.value_versions != values.len().map_err(error::redb)?
    {
        return Err(error::corruption(
            "v2 global workspace accounting disagrees with stored values",
        ));
    }
    let mut total_inline_bytes = 0_u64;
    for item in usage.iter().map_err(error::redb)? {
        let (run, bytes) = item.map_err(error::redb)?;
        let stored_usage: WorkspaceUsage = crate::json::decode(bytes.value(), "workspace usage")?;
        total_inline_bytes = total_inline_bytes
            .checked_add(stored_usage.inline_bytes())
            .ok_or_else(|| error::corruption("v2 workspace inline-byte count overflowed"))?;
        if load_legacy_workspace_value_accounting(&accounting, run.value())?
            != LegacyWorkspaceValueAccountingRecord::from_usage(stored_usage)
        {
            return Err(error::corruption(
                "v2 per-run workspace accounting disagrees with usage",
            ));
        }
    }
    if global.inline_bytes != total_inline_bytes {
        return Err(error::corruption(
            "v2 global workspace inline-byte accounting is inconsistent",
        ));
    }
    Ok(())
}

pub(crate) fn retire_workspace_value_accounting(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    let mut accounting = write
        .open_table(WORKSPACE_VALUE_ACCOUNTING)
        .map_err(error::redb)?;
    loop {
        let key = accounting
            .first()
            .map_err(error::redb)?
            .map(|(key, _)| key.value().to_owned());
        let Some(key) = key else {
            break;
        };
        let _removed = accounting.remove(key.as_str()).map_err(error::redb)?;
    }
    Ok(())
}

pub(crate) fn validate_migrated_workspace_scopes(
    write: &redb::WriteTransaction,
) -> Result<(u64, u64), PersistenceError> {
    let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    let scopes = write.open_table(SCOPES).map_err(error::redb)?;
    let roots = write.open_table(ROOT_SCOPES).map_err(error::redb)?;
    let budgets = write.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
    let usage = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    let mut declared_scopes = 0_u64;
    let mut declared_roots = 0_u64;
    for item in events.iter().map_err(error::redb)? {
        let (_, bytes) = item.map_err(error::redb)?;
        let event = RunEventEnvelope::from_json(bytes.value()).map_err(|cause| {
            error::corruption(format!("legacy run event failed verification: {cause}"))
        })?;
        let declared = match event.kind() {
            RunEventKind::RunCreated {
                root_scope,
                workspace_budget,
                ..
            } => {
                let budget_bytes = budgets
                    .get(event.run_id().as_str())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption(
                            "legacy run-created event is missing its workspace budget",
                        )
                    })?;
                let stored_budget: milkdrift_workspace::WorkspaceBudget =
                    crate::json::decode(budget_bytes.value(), "workspace budget")?;
                if &stored_budget != workspace_budget {
                    return Err(error::corruption(
                        "legacy run-created budget disagrees with workspace storage",
                    ));
                }
                let usage_bytes = usage
                    .get(event.run_id().as_str())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption("legacy run-created event is missing workspace usage")
                    })?;
                let stored_usage: WorkspaceUsage =
                    crate::json::decode(usage_bytes.value(), "workspace usage")?;
                stored_budget
                    .validate_usage(&stored_usage)
                    .map_err(|cause| {
                        error::corruption(format!(
                            "legacy run-created usage exceeds its durable budget: {cause}"
                        ))
                    })?;
                declared_roots = declared_roots
                    .checked_add(1)
                    .ok_or_else(|| error::corruption("legacy root scope count overflowed"))?;
                Some(root_scope)
            }
            RunEventKind::BranchScopeCreated { scope, .. }
            | RunEventKind::RepeatIterationCreated { scope, .. }
            | RunEventKind::SubworkflowCreated { scope, .. } => Some(scope),
            _ => None,
        };
        let Some(scope) = declared else {
            continue;
        };
        declared_scopes = declared_scopes
            .checked_add(1)
            .ok_or_else(|| error::corruption("legacy workspace scope count overflowed"))?;
        let scope_key = crate::codec::pair(
            scope.reference().run().as_str(),
            scope.reference().scope().as_str(),
        )?;
        let stored = scopes
            .get(scope_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("legacy event-declared scope is missing"))?;
        let stored: milkdrift_workspace::WorkspaceScope =
            crate::json::decode(stored.value(), "workspace scope")?;
        if &stored != scope {
            return Err(error::corruption(
                "legacy event-declared scope disagrees with workspace storage",
            ));
        }
        crate::journal::validate_owning_workspace_scope(&scopes, &roots, scope.reference())?;
    }
    if scopes.len().map_err(error::redb)? != declared_scopes
        || roots.len().map_err(error::redb)? != declared_roots
    {
        return Err(error::corruption(
            "legacy workspace scope tables disagree with authoritative event declarations",
        ));
    }
    Ok((declared_scopes, declared_roots))
}

pub(crate) fn backfill_artifact_integrity_documents(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    let temporary_owners = write
        .open_table(ARTIFACT_TEMP_OWNERS)
        .map_err(error::redb)?;
    let mut temporary_manifest = write
        .open_table(ARTIFACT_TEMP_MANIFEST)
        .map_err(error::redb)?;
    for item in temporary_owners.iter().map_err(error::redb)? {
        let (temporary_name, publication) = item.map_err(error::redb)?;
        let publication = ArtifactPublicationId::new(publication.value()).map_err(|cause| {
            error::corruption(format!(
                "legacy temporary artifact owner has an invalid publication identity: {cause}"
            ))
        })?;
        let bytes = crate::json::encode(&publication, "artifact temporary manifest")?;
        let existing = temporary_manifest
            .get(temporary_name.value())
            .map_err(error::redb)?
            .map(|value| value.value().to_vec());
        match existing {
            Some(existing) => {
                let existing: ArtifactPublicationId =
                    crate::json::decode(&existing, "artifact temporary manifest")?;
                if existing != publication {
                    return Err(error::corruption(
                        "legacy temporary artifact owner conflicts with its manifest",
                    ));
                }
            }
            None => {
                temporary_manifest
                    .insert(temporary_name.value(), bytes.as_slice())
                    .map_err(error::redb)?;
            }
        }
    }
    drop(temporary_manifest);
    drop(temporary_owners);

    let metadata = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    let mut manifest = write.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
    for item in metadata.iter().map_err(error::redb)? {
        let (key, value) = item.map_err(error::redb)?;
        let document: ArtifactMetadata = crate::json::decode(value.value(), "artifact metadata")?;
        if document.reference().artifact().as_str() != key.value() {
            return Err(error::corruption(
                "legacy artifact metadata key does not match its checked document",
            ));
        }
        let manifest_bytes = crate::json::encode(&document, "artifact manifest")?;
        let existing = manifest
            .get(key.value())
            .map_err(error::redb)?
            .map(|value| value.value().to_vec());
        match existing {
            Some(existing) => {
                let existing: ArtifactMetadata =
                    crate::json::decode(&existing, "artifact manifest")?;
                if existing != document {
                    return Err(error::corruption(
                        "legacy artifact metadata conflicts with its manifest",
                    ));
                }
            }
            None => {
                manifest
                    .insert(key.value(), manifest_bytes.as_slice())
                    .map_err(error::redb)?;
            }
        }
    }
    drop(manifest);
    drop(metadata);

    let references = write.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
    let mut ownership = write
        .open_table(RUN_ARTIFACT_OWNERSHIP)
        .map_err(error::redb)?;
    for item in references.iter().map_err(error::redb)? {
        let (key, value) = item.map_err(error::redb)?;
        let components = match crate::codec::decode_components(key.value(), 4) {
            Ok(components) => components,
            Err(_) => {
                let components = crate::codec::decode_components(key.value(), 5)?;
                if components[3] != "publication" {
                    return Err(error::corruption(
                        "legacy five-part artifact-reference key has an unknown owner kind",
                    ));
                }
                components
            }
        };
        let reference: ArtifactReference =
            crate::json::decode(value.value(), "artifact reference")?;
        if components[0] != reference.digest().to_hex()
            || components[1] != reference.artifact().as_str()
        {
            return Err(error::corruption(
                "legacy artifact-reference key does not match its checked document",
            ));
        }
        let ownership_key =
            crate::codec::components(&[components[2], components[0], components[1]])?;
        let ownership_bytes = crate::json::encode(&reference, "run artifact ownership")?;
        let existing = ownership
            .get(ownership_key.as_slice())
            .map_err(error::redb)?
            .map(|value| value.value().to_vec());
        match existing {
            Some(existing) => {
                let existing: ArtifactReference =
                    crate::json::decode(&existing, "run artifact ownership")?;
                if existing != reference {
                    return Err(error::corruption(
                        "legacy artifact references conflict for one run ownership key",
                    ));
                }
            }
            None => {
                ownership
                    .insert(ownership_key.as_slice(), ownership_bytes.as_slice())
                    .map_err(error::redb)?;
            }
        }
    }
    drop(ownership);
    drop(references);

    let by_digest = write.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
    let mut previous_digest: Option<(String, u64)> = None;
    let mut derived_bytes = 0_u64;
    for item in by_digest.iter().map_err(error::redb)? {
        let (key, value) = item.map_err(error::redb)?;
        let components = crate::codec::decode_components(key.value(), 2)?;
        let document: ArtifactMetadata = crate::json::decode(value.value(), "artifact metadata")?;
        if components[0] != document.reference().digest().to_hex()
            || components[1] != document.reference().artifact().as_str()
        {
            return Err(error::corruption(
                "legacy artifact digest key does not match its checked document",
            ));
        }
        if previous_digest.as_ref().is_some_and(|(digest, size)| {
            digest == components[0] && *size != document.reference().size_bytes()
        }) {
            return Err(error::corruption(
                "legacy artifact metadata disagrees on content size for one digest",
            ));
        }
        if previous_digest
            .as_ref()
            .is_none_or(|(digest, _)| digest != components[0])
        {
            derived_bytes = derived_bytes
                .checked_add(document.reference().size_bytes())
                .ok_or_else(|| error::corruption("legacy artifact byte accounting overflows"))?;
            previous_digest = Some((components[0].to_owned(), document.reference().size_bytes()));
        }
    }
    drop(by_digest);

    let legacy_bytes = {
        let metadata = write.open_table(METADATA).map_err(error::redb)?;
        metadata
            .get(LEGACY_GLOBAL_ARTIFACT_BYTES_KEY)
            .map_err(error::redb)?
            .map(|value| value.value())
    };
    if legacy_bytes.is_some_and(|stored| stored != derived_bytes)
        || (legacy_bytes.is_none() && derived_bytes != 0)
    {
        return Err(error::corruption(
            "legacy aggregate artifact byte accounting is missing or inconsistent",
        ));
    }
    if derived_bytes != 0 {
        let record = MigratedArtifactAccountingRecord {
            schema_version: 1,
            committed_content_bytes: derived_bytes,
        };
        let bytes = crate::json::encode(&record, "artifact accounting")?;
        let mut accounting = write.open_table(ARTIFACT_ACCOUNTING).map_err(error::redb)?;
        let existing = accounting
            .get(LEGACY_GLOBAL_ARTIFACT_BYTES_KEY)
            .map_err(error::redb)?
            .map(|value| value.value().to_vec());
        match existing {
            Some(existing) => {
                let existing: MigratedArtifactAccountingRecord =
                    crate::json::decode(&existing, "artifact accounting")?;
                if existing != record {
                    return Err(error::corruption(
                        "legacy artifact accounting conflicts with the derived catalog total",
                    ));
                }
            }
            None => {
                accounting
                    .insert(LEGACY_GLOBAL_ARTIFACT_BYTES_KEY, bytes.as_slice())
                    .map_err(error::redb)?;
            }
        }
    }
    let mut metadata = write.open_table(METADATA).map_err(error::redb)?;
    let _legacy = metadata
        .remove(LEGACY_GLOBAL_ARTIFACT_BYTES_KEY)
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn migrate_string_json_table(
    write: &redb::WriteTransaction,
    definition: redb::TableDefinition<'static, &'static str, &'static [u8]>,
    family: &'static str,
) -> Result<(), PersistenceError> {
    let mut after: Option<String> = None;
    loop {
        let next = {
            let table = write.open_table(definition).map_err(error::redb)?;
            let lower = after.as_deref().map_or(Bound::Unbounded, Bound::Excluded);
            table
                .range::<&str>((lower, Bound::Unbounded))
                .map_err(error::redb)?
                .next()
                .transpose()
                .map_err(error::redb)?
                .map(|(key, value)| (key.value().to_owned(), value.value().to_vec()))
        };
        let Some((key, legacy)) = next else {
            break;
        };
        let migrated = crate::json::migrate_legacy(&legacy, family)?;
        let mut table = write.open_table(definition).map_err(error::redb)?;
        table
            .insert(key.as_str(), migrated.as_slice())
            .map_err(error::redb)?;
        after = Some(key);
    }
    Ok(())
}

pub(crate) fn migrate_binary_json_table(
    write: &redb::WriteTransaction,
    definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    family: &'static str,
) -> Result<(), PersistenceError> {
    let mut after: Option<Vec<u8>> = None;
    loop {
        let next = {
            let table = write.open_table(definition).map_err(error::redb)?;
            let lower = after.as_deref().map_or(Bound::Unbounded, Bound::Excluded);
            table
                .range::<&[u8]>((lower, Bound::Unbounded))
                .map_err(error::redb)?
                .next()
                .transpose()
                .map_err(error::redb)?
                .map(|(key, value)| (key.value().to_vec(), value.value().to_vec()))
        };
        let Some((key, legacy)) = next else {
            break;
        };
        let migrated = crate::json::migrate_legacy(&legacy, family)?;
        let mut table = write.open_table(definition).map_err(error::redb)?;
        table
            .insert(key.as_slice(), migrated.as_slice())
            .map_err(error::redb)?;
        after = Some(key);
    }
    Ok(())
}

pub(crate) fn internal(message: impl Into<String>) -> PersistenceError {
    PersistenceError::Storage {
        class: StorageFailureClass::Internal,
        message: message.into(),
    }
}
