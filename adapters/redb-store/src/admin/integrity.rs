use super::cursor::{
    index_cursor_position, make_artifact_digest_cursor, make_delete_guard_cursor,
    make_integrity_cursor, parse_artifact_digest_cursor, parse_delete_guard_cursor, push_failure,
    scan_binary_bytes_phase, scan_binary_string_phase, scan_binary_u8_phase, scan_binary_u64_phase,
    scan_string_bytes_phase, scan_string_string_phase, scan_string_u8_phase, scan_string_u64_phase,
};
use super::*;
#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_index_integrity(
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
    const REVISION_PRIMARY_PHASE: u8 = 15;
    const REVISION_DIGESTS_PHASE: u8 = 16;
    const ARTIFACT_MANIFEST_PHASE: u8 = 17;
    const ARTIFACT_DIGESTS_PHASE: u8 = 18;
    const ARTIFACT_REFERENCES_PHASE: u8 = 19;
    const ARTIFACT_OWNERSHIP_PHASE: u8 = 20;
    const ARTIFACT_TEMP_MANIFEST_PHASE: u8 = 21;
    const ARTIFACT_ACCOUNTING_PHASE: u8 = 22;
    const ARTIFACT_TEMP_OWNERS_PHASE: u8 = 23;
    const SIGNAL_RECEIPTS_PHASE: u8 = 24;
    const RUNNABLE_RUN_HEADS_PHASE: u8 = 25;
    const WORKSPACE_VALUE_HEADS_PHASE: u8 = 26;
    const INVOCATION_FACTS_PHASE: u8 = 27;
    const SNAPSHOTS_PHASE: u8 = 28;
    const SNAPSHOT_LATEST_PHASE: u8 = 29;
    const ARTIFACT_PUBLICATIONS_PHASE: u8 = 30;
    const ARTIFACT_PUBLICATION_AGE_PHASE: u8 = 31;
    const ARTIFACT_RESERVATIONS_PHASE: u8 = 32;
    const ARTIFACT_PATHS_PHASE: u8 = 33;
    const ARTIFACT_DELETE_GUARDS_PHASE: u8 = 34;
    const ARTIFACT_DIGEST_RESERVATIONS_PHASE: u8 = 35;

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
    let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let nonterminal = read.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
    let commands = read.open_table(COMMAND_RESULTS).map_err(error::redb)?;
    let signal_receipts = read.open_table(SIGNAL_RECEIPTS).map_err(error::redb)?;
    let runnable_run_heads = read.open_table(RUNNABLE_RUN_HEADS).map_err(error::redb)?;
    let runnable_identities = read.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let runnable_ordered = read.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let timer_identities = read.open_table(TIMER_ENTRIES).map_err(error::redb)?;
    let timer_ordered = read.open_table(TIMER_INDEX).map_err(error::redb)?;
    let lease_identities = read.open_table(LEASE_ENTRIES).map_err(error::redb)?;
    let lease_ordered = read.open_table(LEASE_INDEX).map_err(error::redb)?;
    let scopes = read.open_table(SCOPES).map_err(error::redb)?;
    let root_scopes = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
    let values = read.open_table(VALUES).map_err(error::redb)?;
    let value_heads = read
        .open_table(WORKSPACE_VALUE_HEADS)
        .map_err(error::redb)?;
    let metadata = read.open_table(METADATA).map_err(error::redb)?;
    let snapshots = read.open_table(SNAPSHOTS).map_err(error::redb)?;
    let snapshot_latest = read.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
    let artifact_publications = read
        .open_table(ARTIFACT_PUBLICATIONS)
        .map_err(error::redb)?;
    let artifact_publications_by_age = read
        .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
        .map_err(error::redb)?;
    let artifact_reservations = read
        .open_table(ARTIFACT_RESERVATIONS)
        .map_err(error::redb)?;
    let artifact_paths = read.open_table(ARTIFACT_PATHS).map_err(error::redb)?;
    let artifact_delete_guards = read
        .open_table(ARTIFACT_DELETE_GUARDS)
        .map_err(error::redb)?;
    let artifact_digest_reservations = read
        .open_table(ARTIFACT_DIGEST_RESERVATIONS)
        .map_err(error::redb)?;
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
            crate::snapshot::validate_history_head(read, &run, head)?;
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
            let stored_usage = crate::journal::validated_workspace_domain(read, &run)?
                .ok_or_else(|| error::corruption("run head has no workspace domain"))?;
            if stored_usage != usage {
                return Err(error::corruption(
                    "run head workspace usage disagrees with its durable domain",
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
        |key, bytes| crate::journal::validate_stored_command_record(key, bytes, &heads, &events),
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
            let stored_usage = crate::journal::validated_workspace_domain(read, &run)?
                .ok_or_else(|| error::corruption("workspace usage has no budget pair"))?;
            if stored_usage != usage {
                return Err(error::corruption(
                    "workspace usage disagrees with its durable domain",
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
                    "workspace budget has no durable usage pair",
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
                    "root workspace scope has no durable workspace domain",
                ));
            }
            Ok(())
        },
    )?;
    scan_string_bytes_phase(
        REVISION_PRIMARY_PHASE,
        start_phase,
        start_key,
        &revisions,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "revision_indexes",
        |key, bytes| {
            let (_document, revision) =
                BlueprintRevisionDocument::from_json(bytes).map_err(|cause| {
                    error::corruption(format!("stored revision failed verification: {cause}"))
                })?;
            if revision.id().as_str() != key {
                return Err(error::corruption(
                    "revision primary key does not match its checked document",
                ));
            }
            let summary = RevisionSummary::from(&revision);
            let digest_key =
                codec::pair(summary.content_digest.as_str(), summary.revision.as_str())?;
            let indexed = revision_digests
                .get(digest_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("revision has no digest index row"))?;
            if crate::revision::decode_summary(indexed.value())? != summary {
                return Err(error::corruption(
                    "revision digest summary disagrees with its primary document",
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
            if key != crate::artifact::GLOBAL_ARTIFACT_BYTES_KEY {
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
    scan_binary_u64_phase(
        SIGNAL_RECEIPTS_PHASE,
        start_phase,
        start_key,
        &signal_receipts,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "signal_indexes",
        |key, sequence| {
            let components = codec::decode_components(key, 2)?;
            let run = RunId::new(components[0]).map_err(|cause| {
                error::corruption(format!("invalid signal-receipt run identity: {cause}"))
            })?;
            let signal = SignalId::new(components[1]).map_err(|cause| {
                error::corruption(format!("invalid signal-receipt identity: {cause}"))
            })?;
            crate::journal::validate_signal_receipt_row(read, &run, &signal, sequence).map(|_| ())
        },
    )?;
    scan_string_bytes_phase(
        RUNNABLE_RUN_HEADS_PHASE,
        start_phase,
        start_key,
        &runnable_run_heads,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "runnable_indexes",
        |run, bytes| crate::journal::validate_runnable_head(read, run, bytes).map(|_| ()),
    )?;
    scan_binary_bytes_phase(
        WORKSPACE_VALUE_HEADS_PHASE,
        start_phase,
        start_key,
        &value_heads,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "workspace_indexes",
        |key, bytes| {
            let reference: WorkspaceValueReference = json::decode(bytes, "workspace value head")?;
            let expected = codec::value_prefix(
                reference.scope().run().as_str(),
                reference.scope().scope().as_str(),
                reference.key().as_str(),
            )?;
            if key != expected.as_slice() {
                return Err(error::corruption(
                    "workspace value-head key does not match its document",
                ));
            }
            let value_key = crate::journal::workspace_value_key(&reference)?;
            let value = values
                .get(value_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("workspace value head names a missing value"))?;
            let value: WorkspaceValueEntry = json::decode(value.value(), "workspace value")?;
            if value.reference() != &reference {
                return Err(error::corruption(
                    "workspace value head disagrees with its authoritative value",
                ));
            }
            Ok(())
        },
    )?;
    scan_string_u64_phase(
        INVOCATION_FACTS_PHASE,
        start_phase,
        start_key,
        &metadata,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "invocation_indexes",
        |key, sequence| {
            if matches!(
                key,
                crate::schema::SCHEMA_VERSION_KEY
                    | crate::schema::INTERNAL_DOCUMENT_FORMAT_VERSION_KEY
                    | crate::schema::LEASE_SET_REVISION_KEY
                    | crate::schema::NONTERMINAL_SET_COUNT_KEY
            ) {
                Ok(())
            } else {
                crate::journal::validate_invocation_fact_row(read, key, sequence)
            }
        },
    )?;
    scan_binary_bytes_phase(
        SNAPSHOTS_PHASE,
        start_phase,
        start_key,
        &snapshots,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "snapshot_indexes",
        |key, bytes| {
            let components = codec::decode_components(key, 2)?;
            let snapshot = SnapshotDocument::from_json(bytes)?;
            if snapshot.run().as_str() != components[0]
                || snapshot.snapshot().as_str() != components[1]
            {
                return Err(error::corruption(
                    "snapshot key does not match its checked document",
                ));
            }
            let run = snapshot.run();
            let head = crate::journal::validated_run_head(&heads, &events, run)?;
            crate::journal::validate_run_history_membership(read, run, head)?;
            if snapshot.covered_sequence() > head
                || crate::snapshot::history_digest_read(read, run, snapshot.covered_sequence())?
                    != *snapshot.history_digest()
            {
                return Err(error::corruption(
                    "snapshot does not match authoritative history",
                ));
            }
            let latest_id = snapshot_latest
                .get(run.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("snapshot has no latest pointer"))?;
            let latest_key = codec::pair(run.as_str(), latest_id.value())?;
            let latest = snapshots
                .get(latest_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("latest snapshot pointer is dangling"))?;
            let latest = SnapshotDocument::from_json(latest.value())?;
            if snapshot.covered_sequence() > latest.covered_sequence()
                || (snapshot.covered_sequence() == latest.covered_sequence()
                    && snapshot.snapshot().as_str() > latest.snapshot().as_str())
            {
                return Err(error::corruption(
                    "latest snapshot pointer does not name the newest snapshot",
                ));
            }
            Ok(())
        },
    )?;
    scan_string_string_phase(
        SNAPSHOT_LATEST_PHASE,
        start_phase,
        start_key,
        &snapshot_latest,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "snapshot_indexes",
        |run, snapshot| {
            let run = RunId::new(run).map_err(|cause| {
                error::corruption(format!("invalid latest-snapshot run: {cause}"))
            })?;
            let snapshot = SnapshotId::new(snapshot).map_err(|cause| {
                error::corruption(format!("invalid latest-snapshot identity: {cause}"))
            })?;
            let key = codec::pair(run.as_str(), snapshot.as_str())?;
            let bytes = snapshots
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("latest snapshot pointer is dangling"))?;
            let document = SnapshotDocument::from_json(bytes.value())?;
            if document.run() != &run || document.snapshot() != &snapshot {
                return Err(error::corruption(
                    "latest snapshot pointer disagrees with its document",
                ));
            }
            Ok(())
        },
    )?;
    scan_string_bytes_phase(
        ARTIFACT_PUBLICATIONS_PHASE,
        start_phase,
        start_key,
        &artifact_publications,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "artifact_publication_indexes",
        |key, bytes| crate::artifact::validate_publication_scrub(read, key, bytes),
    )?;
    scan_binary_string_phase(
        ARTIFACT_PUBLICATION_AGE_PHASE,
        start_phase,
        start_key,
        &artifact_publications_by_age,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "artifact_publication_indexes",
        |key, publication| crate::artifact::validate_publication_age_scrub(read, key, publication),
    )?;
    scan_string_string_phase(
        ARTIFACT_RESERVATIONS_PHASE,
        start_phase,
        start_key,
        &artifact_reservations,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "artifact_publication_indexes",
        |run, publication| {
            crate::artifact::validate_publication_reservation_scrub(read, run, publication)
        },
    )?;
    scan_binary_bytes_phase(
        ARTIFACT_PATHS_PHASE,
        start_phase,
        start_key,
        &artifact_paths,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "artifact_path_indexes",
        |key, value| crate::artifact::validate_path_scrub(read, key, value),
    )?;
    scan_delete_guard_phase(
        ARTIFACT_DELETE_GUARDS_PHASE,
        start_phase,
        start_key,
        &artifact_delete_guards,
        &artifact_paths,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
    )?;
    scan_binary_u8_phase(
        ARTIFACT_DIGEST_RESERVATIONS_PHASE,
        start_phase,
        start_key,
        &artifact_digest_reservations,
        maximum,
        verify_artifact_content,
        result,
        last_cursor,
        more_remaining,
        "artifact_publication_indexes",
        |key, marker| crate::artifact::validate_digest_reservation_scrub(read, key, marker),
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn scan_delete_guard_phase(
    phase: u8,
    start_phase: u8,
    start_key: Option<&[u8]>,
    guards: &impl redb::ReadableTable<&'static [u8], u8>,
    paths: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    maximum: u64,
    verify_artifact_content: bool,
    result: &mut IntegrityScanResult,
    last_cursor: &mut Option<IntegrityScanCursor>,
    more_remaining: &mut bool,
) -> Result<(), PersistenceError> {
    if *more_remaining || phase < start_phase {
        return Ok(());
    }
    let resumed = if phase == start_phase {
        start_key.map(parse_delete_guard_cursor).transpose()?
    } else {
        None
    };
    let guard_lower = match resumed {
        Some((guard, true, _)) => Bound::Included(guard),
        Some((guard, false, _)) => Bound::Excluded(guard),
        None => Bound::Unbounded,
    };
    for row in guards
        .range::<&[u8]>((guard_lower, Bound::Unbounded))
        .map_err(error::redb)?
    {
        let (guard, marker) = row.map_err(error::redb)?;
        let guard_key = guard.value();
        let resume_path = resumed
            .filter(|(resumed_guard, in_progress, _)| *in_progress && *resumed_guard == guard_key)
            .and_then(|(_, _, path)| path);
        let is_resumed_guard =
            resumed.is_some_and(|(key, progress, _)| progress && key == guard_key);
        if !is_resumed_guard {
            if result.documents_checked == maximum {
                *more_remaining = true;
                break;
            }
            result.documents_checked += 1;
            if let Err(cause) =
                crate::artifact::validate_delete_guard_scrub(guard_key, marker.value())
            {
                push_failure(result, "artifact_path_indexes", &cause.to_string())?;
                *last_cursor = Some(make_delete_guard_cursor(
                    guard_key,
                    false,
                    None,
                    verify_artifact_content,
                    last_cursor.as_ref(),
                )?);
                continue;
            }
        }
        let path_lower = resume_path.map_or(Bound::Unbounded, Bound::Excluded);
        let mut found = false;
        let mut last_path = resume_path.map(<[u8]>::to_vec);
        for path in paths
            .range::<&[u8]>((path_lower, Bound::Unbounded))
            .map_err(error::redb)?
        {
            if result.documents_checked == maximum {
                *last_cursor = Some(make_delete_guard_cursor(
                    guard_key,
                    true,
                    last_path.as_deref(),
                    verify_artifact_content,
                    last_cursor.as_ref(),
                )?);
                *more_remaining = true;
                return Ok(());
            }
            let (path_key, path_value) = path.map_err(error::redb)?;
            result.documents_checked += 1;
            last_path = Some(path_key.value().to_vec());
            match crate::artifact::artifact_path_guard_key(path_key.value(), path_value.value()) {
                Ok(candidate) if candidate.as_slice() == guard_key => {
                    found = true;
                    break;
                }
                Ok(_) => {}
                Err(cause) => {
                    push_failure(result, "artifact_path_indexes", &cause.to_string())?;
                }
            }
        }
        if !found {
            push_failure(
                result,
                "artifact_path_indexes",
                "artifact delete guard has no matching path inventory row",
            )?;
        }
        *last_cursor = Some(make_delete_guard_cursor(
            guard_key,
            false,
            None,
            verify_artifact_content,
            last_cursor.as_ref(),
        )?);
    }
    Ok(())
}

pub(crate) fn validate_nonterminal_marker(
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

pub(crate) fn validate_paired_index_row(
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

pub(crate) fn artifact_occurrence_key(
    key: &[u8],
) -> Result<(String, String, String), PersistenceError> {
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
pub(crate) fn scan_artifact_digest_phase(
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
            .get(crate::artifact::GLOBAL_ARTIFACT_BYTES_KEY)
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
