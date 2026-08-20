use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Bound,
};

use milkdrift_persistence::{
    ActiveLeaseSnapshot, ActorRef, AtomicRunCommitOutcome, AtomicRunCommitRequest,
    COMMAND_RESULT_SCHEMA_VERSION_V1, CommandId, CommandReceipt, CommandResultDocument,
    EventCursor, EventPage, EventPageQuery, IndexedRunState, IntegrityDigest, LeaseIndexEntry,
    LeaseIndexMutation, MAX_VALUE_PROVENANCE_DEPTH, PageSize, PersistenceError, RunEventKind,
    RunJournal, RunQueryStore, RunSequence, RunSummaryIndex, RunSummaryPage, RunSummaryPageQuery,
    RunnableCursor, RunnableIndexEntry, RunnableIndexMutation, RunnablePage, TimerIndexEntry,
    TimerIndexMutation, TimestampMillis, WorkspaceMutation, WorkspaceStore,
};
use milkdrift_workspace::{
    ArtifactReference, MAX_SCOPE_DEPTH, RunId, ScopeId, ScopeKind, ScopeReference, ValueKey,
    ValueOrigin, WorkspaceScope, WorkspaceUsage, WorkspaceValueEntry, WorkspaceValueReference,
};
use redb::{ReadableTable, ReadableTableMetadata, Table};
use serde::{Deserialize, Serialize};

use crate::{
    RedbStore, codec, error,
    fault::FaultPoint,
    json,
    schema::{
        ARTIFACT_METADATA, ARTIFACT_RESERVATIONS, COMMAND_RESULTS, EVENT_CHECKSUMS, LEASE_ENTRIES,
        LEASE_INDEX, NONTERMINAL_RUNS, ROOT_SCOPES, RUN_EVENTS, RUN_HEADS, RUN_SUMMARIES,
        RUNNABLE_ENTRIES, RUNNABLE_INDEX, RUNNABLE_RUN_HEADS, SCOPES, TIMER_ENTRIES, TIMER_INDEX,
        VALUES, WORKSPACE_BUDGETS, WORKSPACE_USAGE, WORKSPACE_VALUE_ACCOUNTING,
        WORKSPACE_VALUE_HEADS,
    },
};

const COMMAND_RECORD_SCHEMA_VERSION: u32 = 1;

struct RunnableHeadState {
    previous_bytes: Option<Vec<u8>>,
    previous_witness: Option<[u8; 32]>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCommandRecord<'a> {
    schema_version: u32,
    command: &'a CommandId,
    run: &'a RunId,
    actor: &'a ActorRef,
    expected_sequence: RunSequence,
    submitted_at: TimestampMillis,
    canonical_document: &'a [u8],
    fingerprint: &'a IntegrityDigest,
    result: &'a CommandResultDocument,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnedCommandRecord {
    schema_version: u32,
    command: CommandId,
    run: RunId,
    actor: ActorRef,
    expected_sequence: RunSequence,
    submitted_at: TimestampMillis,
    canonical_document: Vec<u8>,
    fingerprint: IntegrityDigest,
    result: CommandResultDocument,
}

impl RunJournal for RedbStore {
    #[tracing::instrument(
        name = "milkdrift.redb_store.commit_command",
        skip_all,
        fields(
            run = %request.receipt.run(),
            command = %request.receipt.command(),
            expected_sequence = request.receipt.expected_sequence().get(),
            event_count = request.events.len()
        )
    )]
    fn commit_command(
        &self,
        request: &AtomicRunCommitRequest,
    ) -> Result<AtomicRunCommitOutcome, PersistenceError> {
        let verified_request = AtomicRunCommitRequest::new(
            request.receipt.clone(),
            request.events.clone(),
            request.workspace.clone(),
            request.workspace_accounting.clone(),
            request.required_artifacts.clone(),
            request.newly_referenced_artifacts.clone(),
            request.expected_lease_catalog.clone(),
            request.result.clone(),
            request.indexes.clone(),
        )?;
        if &verified_request != request {
            return Err(PersistenceError::InvalidDocument(
                "atomic run commit is not canonical".to_owned(),
            ));
        }
        if let Some(accounting) = &request.workspace_accounting {
            for event in &request.events {
                if let RunEventKind::RunCreated {
                    workspace_budget, ..
                } = event.kind()
                {
                    if workspace_budget != &accounting.budget {
                        return Err(PersistenceError::InvalidDocument(
                            "run-created workspace budget differs from atomic accounting budget"
                                .to_owned(),
                        ));
                    }
                }
            }
        }
        let command_key = codec::pair(
            request.receipt.run().as_str(),
            request.receipt.command().as_str(),
        )?;
        let command_bytes = encode_command_record(request)?;
        let write = self.database().begin_write().map_err(error::redb)?;
        crate::trie::validate_roots_in_transaction(&write)?;
        crate::artifact::validate_artifact_catalog(&write)?;
        let _workspace_value_accounting =
            validate_workspace_value_accounting_in_transaction(&write)?;

        let actual_head = {
            let heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
            let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
            validated_run_head(&heads, &events, request.receipt.run())?
        };
        let previous_membership = validate_run_history_membership_in_transaction(
            &write,
            request.receipt.run(),
            actual_head,
        )?;

        // Idempotency deliberately precedes the optimistic sequence conflict,
        // but replay never bypasses authoritative journal-integrity checks.
        {
            let commands = write.open_table(COMMAND_RESULTS).map_err(error::redb)?;
            if let Some(stored) = commands.get(command_key.as_slice()).map_err(error::redb)? {
                let stored_bytes = stored.value().to_vec();
                validate_command_catalog_in_transaction(&write, &command_key, Some(&stored_bytes))?;
                let stored = decode_command_record(&stored_bytes)?;
                if stored.run != *request.receipt.run()
                    || stored.command != *request.receipt.command()
                {
                    return Err(error::corruption(
                        "command-result key does not match its stored identities",
                    ));
                }
                let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
                let checksums = write.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
                validate_command_record_history(&stored, actual_head, &events, &checksums)?;
                drop(checksums);
                drop(events);
                validate_command_event_catalog_in_transaction(&write, &stored)?;
                if stored.fingerprint == *request.receipt.fingerprint() {
                    return Ok(AtomicRunCommitOutcome::Replayed(stored.result));
                }
                return Err(PersistenceError::IdempotencyConflict {
                    run: request.receipt.run().clone(),
                    command: request.receipt.command().clone(),
                    existing: stored.fingerprint,
                    supplied: request.receipt.fingerprint().clone(),
                });
            }
        }
        validate_command_catalog_in_transaction(&write, &command_key, None)?;
        if actual_head != request.receipt.expected_sequence() {
            return Err(PersistenceError::SequenceConflict {
                run: request.receipt.run().clone(),
                expected: request.receipt.expected_sequence(),
                actual: actual_head,
            });
        }
        if let Some(expected) = &request.expected_lease_catalog {
            let root = crate::trie::family_root_in_transaction(
                &write,
                crate::trie::CatalogFamily::LeaseIdentity,
            )?;
            let actual = IntegrityDigest::new(format!("b3_{}", blake3::Hash::from_bytes(root)))?;
            if &actual != expected {
                return Err(PersistenceError::LeaseCatalogConflict {
                    expected: expected.clone(),
                    actual,
                });
            }
        }

        validate_required_artifacts(self, &write, &request.required_artifacts)?;
        validate_artifact_accounting_references(&write, request)?;
        validate_workspace_accounting(&write, request)?;
        append_events(&write, request)?;
        apply_workspace(&write, request)?;
        apply_indexes(&write, request)?;
        record_artifact_references(&write, request)?;

        {
            let mut commands = write.open_table(COMMAND_RESULTS).map_err(error::redb)?;
            commands
                .insert(command_key.as_slice(), command_bytes.as_slice())
                .map_err(error::redb)?;
        }
        let command_family = crate::trie::CatalogFamily::Command;
        if crate::trie::put(
            &write,
            command_family,
            crate::trie::hashed_path(command_family, &command_key),
            &command_key,
            crate::trie::digest_payload(command_family, &command_bytes),
        )?
        .is_some()
        {
            return Err(error::corruption(
                "new command unexpectedly replaced an authenticated catalog leaf",
            ));
        }
        if !request.events.is_empty() {
            let mut heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
            heads
                .insert(
                    request.receipt.run().as_str(),
                    request.result.resulting_sequence().get(),
                )
                .map_err(error::redb)?;
            persist_run_membership(
                &write,
                request.receipt.run(),
                request.result.resulting_sequence(),
                previous_membership,
            )?;
        }
        persist_workspace_accounting(&write, request)?;
        validate_workspace_value_accounting_in_transaction(&write)?;
        crate::trie::validate_roots_in_transaction(&write)?;

        self.faults.check(FaultPoint::BeforeCommandCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterCommandCommit)?;
        Ok(AtomicRunCommitOutcome::Committed(request.result.clone()))
    }

    fn head(&self, run: &RunId) -> Result<RunSequence, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, run)?;
        validate_run_history_membership(&read, run, head)?;
        Ok(head)
    }

    fn command_result(
        &self,
        run: &RunId,
        command: &CommandId,
    ) -> Result<Option<CommandResultDocument>, PersistenceError> {
        let key = codec::pair(run.as_str(), command.as_str())?;
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, run)?;
        let _membership = validate_run_history_membership(&read, run, head)?;
        let table = read.open_table(COMMAND_RESULTS).map_err(error::redb)?;
        let Some(record) = table.get(key.as_slice()).map_err(error::redb)? else {
            validate_command_catalog(&read, &key, None)?;
            return Ok(None);
        };
        let record_bytes = record.value().to_vec();
        validate_command_catalog(&read, &key, Some(&record_bytes))?;
        let record = decode_command_record(&record_bytes)?;
        if record.run != *run || record.command != *command {
            return Err(error::corruption(
                "command-result key does not match its stored identities",
            ));
        }
        let checksums = read.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
        validate_command_record_history(&record, head, &events, &checksums)?;
        validate_command_event_catalog(&read, &record)?;
        Ok(Some(record.result))
    }
}

fn encode_command_record(request: &AtomicRunCommitRequest) -> Result<Vec<u8>, PersistenceError> {
    json::encode(
        &StoredCommandRecord {
            schema_version: COMMAND_RECORD_SCHEMA_VERSION,
            command: request.receipt.command(),
            run: request.receipt.run(),
            actor: request.receipt.actor(),
            expected_sequence: request.receipt.expected_sequence(),
            submitted_at: request.receipt.submitted_at(),
            canonical_document: request.receipt.canonical_document(),
            fingerprint: request.receipt.fingerprint(),
            result: &request.result,
        },
        "command record",
    )
}

fn decode_command_record(bytes: &[u8]) -> Result<OwnedCommandRecord, PersistenceError> {
    let record: OwnedCommandRecord = json::decode(bytes, "command record")?;
    if record.schema_version != COMMAND_RECORD_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "command_record",
            found: record.schema_version,
            supported: COMMAND_RECORD_SCHEMA_VERSION,
        });
    }
    let receipt = CommandReceipt::new(
        record.command.clone(),
        record.run.clone(),
        record.actor.clone(),
        record.expected_sequence,
        record.submitted_at,
        record.canonical_document.clone(),
    )
    .map_err(|cause| {
        PersistenceError::Corruption(format!("stored command receipt failed validation: {cause}"))
    })?;
    if receipt.fingerprint() != &record.fingerprint {
        return Err(error::corruption(
            "stored command receipt fingerprint does not match its bytes",
        ));
    }
    validate_command_result(&record.result)?;
    if record.result.command() != &record.command
        || record.result.run() != &record.run
        || record.result.command_fingerprint() != &record.fingerprint
    {
        return Err(error::corruption(
            "stored command result does not match its receipt",
        ));
    }
    let emitted = u64::try_from(record.result.event_ids().len())
        .map_err(|_| error::corruption("stored command result event count exceeds u64"))?;
    let expected_resulting = record
        .expected_sequence
        .get()
        .checked_add(emitted)
        .ok_or_else(|| error::corruption("stored command result sequence overflows"))?;
    if record.result.resulting_sequence().get() != expected_resulting {
        return Err(error::corruption(
            "stored command result sequence does not match its receipt and event count",
        ));
    }
    Ok(record)
}

fn validate_command_result(result: &CommandResultDocument) -> Result<(), PersistenceError> {
    if result.schema_version() != COMMAND_RESULT_SCHEMA_VERSION_V1 {
        return Err(PersistenceError::UnsupportedVersion {
            document: "command_result",
            found: result.schema_version(),
            supported: COMMAND_RESULT_SCHEMA_VERSION_V1,
        });
    }
    let rebuilt = CommandResultDocument::new(
        result.command().clone(),
        result.run().clone(),
        result.command_fingerprint().clone(),
        result.disposition(),
        result.resulting_sequence(),
        result.event_ids().to_vec(),
        result.result().clone(),
    )
    .map_err(|cause| {
        PersistenceError::Corruption(format!("stored command result failed validation: {cause}"))
    })?;
    if &rebuilt != result {
        return Err(error::corruption(
            "stored command result is not canonical schema v1",
        ));
    }
    Ok(())
}

fn validate_command_result_head(
    result: &CommandResultDocument,
    head: RunSequence,
) -> Result<(), PersistenceError> {
    if result.resulting_sequence() > head
        || (result.disposition() == milkdrift_persistence::CommandDisposition::Accepted
            && result.resulting_sequence() == RunSequence::ZERO)
    {
        return Err(error::corruption(
            "stored command result is beyond the authoritative journal head",
        ));
    }
    Ok(())
}

fn validate_command_record_history<E, C>(
    record: &OwnedCommandRecord,
    head: RunSequence,
    events: &E,
    checksums: &C,
) -> Result<(), PersistenceError>
where
    E: redb::ReadableTable<&'static [u8], &'static [u8]>,
    C: redb::ReadableTable<&'static str, &'static str>,
{
    validate_command_result_head(&record.result, head)?;
    let mut sequence = record.expected_sequence;
    for expected_event in record.result.event_ids() {
        sequence = sequence.next()?;
        let key = codec::run_sequence(record.run.as_str(), sequence)?;
        let bytes = events
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| {
                error::corruption(format!(
                    "stored command result names missing event sequence {sequence}"
                ))
            })?;
        let event = decode_stored_event(bytes.value())?;
        if event.run_id() != &record.run
            || event.sequence() != sequence
            || event.event_id() != expected_event
        {
            return Err(error::corruption(
                "stored command result does not match its authoritative event range",
            ));
        }
        let checksum = checksums
            .get(expected_event.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("command event checksum index entry is missing"))?;
        if checksum.value() != event.checksum().as_str() {
            return Err(error::corruption(
                "command event checksum index does not match its envelope",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_stored_command_record<H, E, C>(
    key: &[u8],
    bytes: &[u8],
    heads: &H,
    events: &E,
    checksums: &C,
) -> Result<(), PersistenceError>
where
    H: redb::ReadableTable<&'static str, u64>,
    E: redb::ReadableTable<&'static [u8], &'static [u8]>,
    C: redb::ReadableTable<&'static str, &'static str>,
{
    let record = decode_command_record(bytes)?;
    if codec::pair(record.run.as_str(), record.command.as_str())?.as_slice() != key {
        return Err(error::corruption(
            "command-result key does not match its stored identities",
        ));
    }
    let head = validated_run_head(heads, events, &record.run)?;
    validate_command_record_history(&record, head, events, checksums)
}

fn validate_command_event_catalog_in_transaction(
    write: &redb::WriteTransaction,
    record: &OwnedCommandRecord,
) -> Result<(), PersistenceError> {
    let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    let mut sequence = record.expected_sequence;
    for _ in record.result.event_ids() {
        sequence = sequence.next()?;
        let key = codec::run_sequence(record.run.as_str(), sequence)?;
        let bytes = events
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("command event is absent from the journal"))?
            .value()
            .to_vec();
        validate_event_catalog_in_transaction(write, &record.run, sequence, &key, &bytes)?;
    }
    Ok(())
}

fn validate_command_event_catalog(
    read: &redb::ReadTransaction,
    record: &OwnedCommandRecord,
) -> Result<(), PersistenceError> {
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let mut sequence = record.expected_sequence;
    for _ in record.result.event_ids() {
        sequence = sequence.next()?;
        let key = codec::run_sequence(record.run.as_str(), sequence)?;
        let bytes = events
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("command event is absent from the journal"))?
            .value()
            .to_vec();
        validate_event_catalog(read, &record.run, sequence, &key, &bytes)?;
    }
    Ok(())
}

pub(crate) fn validate_run_history_membership_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<Option<[u8; 32]>, PersistenceError> {
    let family = crate::trie::CatalogFamily::RunMembership;
    let summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let expected = expected_run_membership(&summaries, run, head)?;
    drop(summaries);
    let witness = crate::trie::verify_member_in_transaction(
        write,
        family,
        crate::trie::hashed_path(family, run.as_str().as_bytes()),
        run.as_str().as_bytes(),
    )?;
    let result = validate_run_membership_witness(expected, witness)?;
    validate_event_boundary_in_transaction(write, run, head)?;
    if let Some(run_payload) = result {
        let summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        let bytes = summaries
            .get(run.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authenticated run has no summary"))?;
        let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
        validate_nonterminal_membership_in_transaction(write, &summary, run_payload)?;
    }
    Ok(result)
}

pub(crate) fn validate_run_history_membership(
    read: &redb::ReadTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<Option<[u8; 32]>, PersistenceError> {
    let family = crate::trie::CatalogFamily::RunMembership;
    let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let expected = expected_run_membership(&summaries, run, head)?;
    drop(summaries);
    let witness = crate::trie::verify_member(
        read,
        family,
        crate::trie::hashed_path(family, run.as_str().as_bytes()),
        run.as_str().as_bytes(),
    )?;
    let result = validate_run_membership_witness(expected, witness)?;
    validate_event_boundary(read, run, head)?;
    if let Some(run_payload) = result {
        let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        let bytes = summaries
            .get(run.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("authenticated run has no summary"))?;
        let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
        validate_nonterminal_membership(read, &summary, run_payload)?;
    }
    Ok(result)
}

fn expected_run_membership<S>(
    summaries: &S,
    run: &RunId,
    head: RunSequence,
) -> Result<Option<[u8; 32]>, PersistenceError>
where
    S: redb::ReadableTable<&'static str, &'static [u8]>,
{
    let summary = summaries.get(run.as_str()).map_err(error::redb)?;
    match (head == RunSequence::ZERO, summary) {
        (true, None) => Ok(None),
        (true, Some(_)) => Err(error::corruption(
            "run summary exists without an authoritative journal head",
        )),
        (false, None) => Err(error::corruption(
            "authoritative run head has no discoverability summary",
        )),
        (false, Some(bytes)) => {
            let summary_bytes = bytes.value();
            let summary: RunSummaryIndex = json::decode(summary_bytes, "run summary")?;
            if summary.run != *run || summary.through_sequence != head {
                return Err(error::corruption(
                    "run membership summary disagrees with its head or identity",
                ));
            }
            Ok(Some(run_membership_payload(run, head, summary_bytes)))
        }
    }
}

fn run_membership_payload(run: &RunId, head: RunSequence, summary: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.redb.run-membership.v1\0");
    hasher.update(&(run.as_str().len() as u64).to_be_bytes());
    hasher.update(run.as_str().as_bytes());
    hasher.update(&head.get().to_be_bytes());
    hasher.update(&(summary.len() as u64).to_be_bytes());
    hasher.update(summary);
    *hasher.finalize().as_bytes()
}

fn nonterminal_membership_path(run: &RunId) -> [u8; 32] {
    let family = crate::trie::CatalogFamily::NonterminalRun;
    crate::trie::hashed_path(family, run.as_str().as_bytes())
}

fn nonterminal_membership_payload(run_payload: [u8; 32]) -> [u8; 32] {
    crate::trie::digest_payload(crate::trie::CatalogFamily::NonterminalRun, &run_payload)
}

fn validate_nonterminal_membership(
    read: &redb::ReadTransaction,
    summary: &RunSummaryIndex,
    run_payload: [u8; 32],
) -> Result<(), PersistenceError> {
    let marker = read
        .open_table(NONTERMINAL_RUNS)
        .map_err(error::redb)?
        .get(summary.run.as_str())
        .map_err(error::redb)?
        .map(|marker| marker.value());
    let family = crate::trie::CatalogFamily::NonterminalRun;
    let witness = crate::trie::verify_member(
        read,
        family,
        nonterminal_membership_path(&summary.run),
        summary.run.as_str().as_bytes(),
    )?;
    match (summary.state, marker, witness) {
        (IndexedRunState::Terminal, None, None) => Ok(()),
        (IndexedRunState::Terminal, _, _) => Err(error::corruption(
            "terminal run remains in authenticated nonterminal discovery",
        )),
        (_, Some(1), Some(witness)) if witness == nonterminal_membership_payload(run_payload) => {
            Ok(())
        }
        (_, Some(1), Some(_)) => Err(error::corruption(
            "nonterminal discovery disagrees with the run membership",
        )),
        (_, Some(_), _) => Err(error::corruption(
            "nonterminal discovery contains an invalid marker",
        )),
        (_, None, _) => Err(error::corruption(
            "nonterminal run is absent from authenticated discovery",
        )),
    }
}

fn validate_nonterminal_membership_in_transaction(
    write: &redb::WriteTransaction,
    summary: &RunSummaryIndex,
    run_payload: [u8; 32],
) -> Result<(), PersistenceError> {
    let marker = write
        .open_table(NONTERMINAL_RUNS)
        .map_err(error::redb)?
        .get(summary.run.as_str())
        .map_err(error::redb)?
        .map(|marker| marker.value());
    let family = crate::trie::CatalogFamily::NonterminalRun;
    let witness = crate::trie::verify_member_in_transaction(
        write,
        family,
        nonterminal_membership_path(&summary.run),
        summary.run.as_str().as_bytes(),
    )?;
    match (summary.state, marker, witness) {
        (IndexedRunState::Terminal, None, None) => Ok(()),
        (IndexedRunState::Terminal, _, _) => Err(error::corruption(
            "terminal run remains in authenticated nonterminal discovery",
        )),
        (_, Some(1), Some(witness)) if witness == nonterminal_membership_payload(run_payload) => {
            Ok(())
        }
        (_, Some(1), Some(_)) => Err(error::corruption(
            "nonterminal discovery disagrees with the run membership",
        )),
        (_, Some(_), _) => Err(error::corruption(
            "nonterminal discovery contains an invalid marker",
        )),
        (_, None, _) => Err(error::corruption(
            "nonterminal run is absent from authenticated discovery",
        )),
    }
}

pub(crate) fn migrate_nonterminal_membership(
    write: &redb::WriteTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<(), PersistenceError> {
    let summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let bytes = summaries
        .get(run.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("legacy run is missing its summary"))?;
    let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
    if summary.run != *run || summary.through_sequence != head {
        return Err(error::corruption(
            "legacy run summary disagrees with its head or identity",
        ));
    }
    let marker = write
        .open_table(NONTERMINAL_RUNS)
        .map_err(error::redb)?
        .get(run.as_str())
        .map_err(error::redb)?
        .map(|marker| marker.value());
    match (summary.state, marker) {
        (IndexedRunState::Terminal, None) => Ok(()),
        (IndexedRunState::Terminal, Some(_)) => Err(error::corruption(
            "legacy terminal run remains in nonterminal discovery",
        )),
        (_, Some(1)) => {
            let run_payload = run_membership_payload(run, head, bytes.value());
            let family = crate::trie::CatalogFamily::NonterminalRun;
            if crate::trie::put(
                write,
                family,
                nonterminal_membership_path(run),
                run.as_str().as_bytes(),
                nonterminal_membership_payload(run_payload),
            )?
            .is_some()
            {
                return Err(error::corruption(
                    "legacy nonterminal discovery contains a duplicate run",
                ));
            }
            Ok(())
        }
        (_, Some(_)) => Err(error::corruption(
            "legacy nonterminal discovery contains an invalid marker",
        )),
        (_, None) => Err(error::corruption(
            "legacy nonterminal run is missing from discovery",
        )),
    }
}

fn validate_run_membership_witness(
    expected: Option<[u8; 32]>,
    witness: Option<[u8; 32]>,
) -> Result<Option<[u8; 32]>, PersistenceError> {
    match (expected, witness) {
        (None, None) => Ok(None),
        (Some(expected), Some(witness)) if expected == witness => Ok(Some(witness)),
        (None, Some(_)) => Err(error::corruption(
            "run membership witness names an absent aggregate",
        )),
        (Some(_), None) => Err(error::corruption(
            "durable run aggregate has no authenticated membership witness",
        )),
        (Some(_), Some(_)) => Err(error::corruption(
            "run membership witness disagrees with its authoritative head and summary",
        )),
    }
}

pub(crate) fn persist_run_membership(
    write: &redb::WriteTransaction,
    run: &RunId,
    head: RunSequence,
    previous: Option<[u8; 32]>,
) -> Result<(), PersistenceError> {
    let summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let expected = expected_run_membership(&summaries, run, head)?
        .ok_or_else(|| error::corruption("nonempty run lost its membership payload"))?;
    drop(summaries);
    let family = crate::trie::CatalogFamily::RunMembership;
    let replaced = crate::trie::put(
        write,
        family,
        crate::trie::hashed_path(family, run.as_str().as_bytes()),
        run.as_str().as_bytes(),
        expected,
    )?;
    if replaced != previous {
        return Err(error::corruption(
            "run membership changed outside the authoritative command transaction",
        ));
    }
    Ok(())
}

fn run_membership_path(run: &RunId) -> [u8; 32] {
    let family = crate::trie::CatalogFamily::RunMembership;
    crate::trie::hashed_path(family, run.as_str().as_bytes())
}

pub(crate) fn validate_run_membership_leaf(
    read: &redb::ReadTransaction,
    leaf: &crate::trie::TrieLeaf,
) -> Result<RunSummaryIndex, PersistenceError> {
    let run_text = std::str::from_utf8(&leaf.logical_key)
        .map_err(|_| error::corruption("run membership contains a non-UTF-8 identity"))?;
    let run = RunId::new(run_text)
        .map_err(|cause| error::corruption(format!("invalid run membership identity: {cause}")))?;
    if leaf.path != run_membership_path(&run) {
        return Err(error::corruption(
            "run membership path disagrees with its logical identity",
        ));
    }
    let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let head = validated_run_head(&heads, &events, &run)?;
    let expected = validate_run_history_membership(read, &run, head)?
        .ok_or_else(|| error::corruption("authenticated run membership has no aggregate"))?;
    if expected != leaf.payload_digest {
        return Err(error::corruption(
            "run membership leaf disagrees with its authoritative aggregate",
        ));
    }
    let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let bytes = summaries
        .get(run.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("authenticated run is missing its summary"))?;
    let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
    if summary.run != run || summary.through_sequence != head {
        return Err(error::corruption(
            "authenticated run summary disagrees with its key or head",
        ));
    }
    validate_nonterminal_membership(read, &summary, expected)?;
    Ok(summary)
}

fn validate_run_cursor_anchor(
    read: &redb::ReadTransaction,
    run: &RunId,
) -> Result<[u8; 32], PersistenceError> {
    let path = run_membership_path(run);
    let family = crate::trie::CatalogFamily::RunMembership;
    let witness = crate::trie::verify_member(read, family, path, run.as_str().as_bytes())?;
    let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let head = validated_run_head(&heads, &events, run)?;
    let expected = validate_run_history_membership(read, run, head)?;
    match (witness, expected) {
        (Some(witness), Some(expected)) if witness == expected => Ok(path),
        (None, None) => Err(PersistenceError::InvalidCursor(
            "run-summary cursor does not name an authenticated run".to_owned(),
        )),
        _ => Err(error::corruption(
            "run-summary cursor aggregate disagrees with its authenticated membership",
        )),
    }
}

fn validate_nonterminal_membership_leaf(
    read: &redb::ReadTransaction,
    leaf: &crate::trie::TrieLeaf,
) -> Result<RunSummaryIndex, PersistenceError> {
    let run_text = std::str::from_utf8(&leaf.logical_key)
        .map_err(|_| error::corruption("nonterminal catalog contains a non-UTF-8 identity"))?;
    let run = RunId::new(run_text).map_err(|cause| {
        error::corruption(format!("invalid nonterminal catalog identity: {cause}"))
    })?;
    if leaf.path != nonterminal_membership_path(&run) {
        return Err(error::corruption(
            "nonterminal catalog path disagrees with its run identity",
        ));
    }
    let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let head = validated_run_head(&heads, &events, &run)?;
    let run_payload = validate_run_history_membership(read, &run, head)?
        .ok_or_else(|| error::corruption("nonterminal catalog names an absent run"))?;
    let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let bytes = summaries
        .get(run.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("nonterminal catalog names a missing summary"))?;
    let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
    if summary.run != run || summary.through_sequence != head {
        return Err(error::corruption(
            "nonterminal summary disagrees with its run identity or head",
        ));
    }
    validate_nonterminal_membership(read, &summary, run_payload)?;
    if summary.state == IndexedRunState::Terminal
        || leaf.payload_digest != nonterminal_membership_payload(run_payload)
    {
        return Err(error::corruption(
            "nonterminal catalog leaf disagrees with its run summary",
        ));
    }
    Ok(summary)
}

pub(crate) fn event_catalog_path(
    run: &RunId,
    sequence: RunSequence,
    logical_key: &[u8],
) -> Result<[u8; 32], PersistenceError> {
    let family = crate::trie::CatalogFamily::Event;
    let run_hash = crate::trie::hashed_path(family, run.as_str().as_bytes());
    let mut prefix = [0; 24];
    prefix[..16].copy_from_slice(&run_hash[..16]);
    prefix[16..].copy_from_slice(&sequence.get().to_be_bytes());
    crate::trie::ordered_path(family, &prefix, logical_key)
}

fn validate_event_catalog_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    sequence: RunSequence,
    key: &[u8],
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let family = crate::trie::CatalogFamily::Event;
    let witness = crate::trie::verify_member_in_transaction(
        write,
        family,
        event_catalog_path(run, sequence, key)?,
        key,
    )?;
    validate_catalog_document(family, witness, Some(bytes), "event")
}

fn validate_event_catalog(
    read: &redb::ReadTransaction,
    run: &RunId,
    sequence: RunSequence,
    key: &[u8],
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let family = crate::trie::CatalogFamily::Event;
    let witness =
        crate::trie::verify_member(read, family, event_catalog_path(run, sequence, key)?, key)?;
    validate_catalog_document(family, witness, Some(bytes), "event")
}

fn validate_event_boundary_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<(), PersistenceError> {
    let after = validate_head_event_in_transaction(write, run, head)?;
    let page =
        crate::trie::page_in_transaction(write, crate::trie::CatalogFamily::Event, None, after, 1)?;
    reject_event_beyond_head(run, page.leaves.first())
}

fn validate_event_boundary(
    read: &redb::ReadTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<(), PersistenceError> {
    let after = validate_head_event(read, run, head)?;
    let page = crate::trie::page(read, crate::trie::CatalogFamily::Event, None, after, 1)?;
    reject_event_beyond_head(run, page.leaves.first())
}

fn validate_head_event_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<Option<[u8; 32]>, PersistenceError> {
    if head == RunSequence::ZERO {
        return event_group_predecessor(run);
    }
    let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    let key = codec::run_sequence(run.as_str(), head)?;
    let bytes = events
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("authoritative event head is missing"))?
        .value()
        .to_vec();
    validate_event_catalog_in_transaction(write, run, head, &key, &bytes)?;
    Ok(Some(event_catalog_path(run, head, &key)?))
}

fn validate_head_event(
    read: &redb::ReadTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<Option<[u8; 32]>, PersistenceError> {
    if head == RunSequence::ZERO {
        return event_group_predecessor(run);
    }
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let key = codec::run_sequence(run.as_str(), head)?;
    let bytes = events
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("authoritative event head is missing"))?
        .value()
        .to_vec();
    validate_event_catalog(read, run, head, &key, &bytes)?;
    Ok(Some(event_catalog_path(run, head, &key)?))
}

fn event_group_predecessor(run: &RunId) -> Result<Option<[u8; 32]>, PersistenceError> {
    let family = crate::trie::CatalogFamily::Event;
    let run_hash = crate::trie::hashed_path(family, run.as_str().as_bytes());
    let mut first = [0_u8; 32];
    first[..16].copy_from_slice(&run_hash[..16]);
    Ok(predecessor_path(first))
}

fn predecessor_path(mut path: [u8; 32]) -> Option<[u8; 32]> {
    for index in (0..path.len()).rev() {
        if path[index] != 0 {
            path[index] -= 1;
            path[index + 1..].fill(u8::MAX);
            return Some(path);
        }
    }
    None
}

fn reject_event_beyond_head(
    run: &RunId,
    candidate: Option<&crate::trie::TrieLeaf>,
) -> Result<(), PersistenceError> {
    let Some(candidate) = candidate else {
        return Ok(());
    };
    let family = crate::trie::CatalogFamily::Event;
    let run_hash = crate::trie::hashed_path(family, run.as_str().as_bytes());
    if candidate.path[..16] != run_hash[..16] {
        return Ok(());
    }
    let run_prefix = codec::component(run.as_str())?;
    if !candidate.logical_key.starts_with(&run_prefix) {
        return Err(error::corruption(
            "authenticated event ordering prefix collides across run identities",
        ));
    }
    Err(error::corruption(
        "authenticated event catalog contains facts beyond the authoritative head",
    ))
}

fn validate_command_catalog_in_transaction(
    write: &redb::WriteTransaction,
    key: &[u8],
    stored: Option<&[u8]>,
) -> Result<(), PersistenceError> {
    let family = crate::trie::CatalogFamily::Command;
    let witness = crate::trie::verify_member_in_transaction(
        write,
        family,
        crate::trie::hashed_path(family, key),
        key,
    )?;
    validate_catalog_document(family, witness, stored, "command")
}

fn validate_command_catalog(
    read: &redb::ReadTransaction,
    key: &[u8],
    stored: Option<&[u8]>,
) -> Result<(), PersistenceError> {
    let family = crate::trie::CatalogFamily::Command;
    let witness =
        crate::trie::verify_member(read, family, crate::trie::hashed_path(family, key), key)?;
    validate_catalog_document(family, witness, stored, "command")
}

fn validate_catalog_document(
    family: crate::trie::CatalogFamily,
    witness: Option<[u8; 32]>,
    stored: Option<&[u8]>,
    label: &'static str,
) -> Result<(), PersistenceError> {
    match (witness, stored) {
        (None, None) => Ok(()),
        (Some(witness), Some(bytes)) if witness == crate::trie::digest_payload(family, bytes) => {
            Ok(())
        }
        (Some(_), Some(_)) => Err(error::corruption(format!(
            "{label} document disagrees with its authenticated catalog"
        ))),
        (Some(_), None) => Err(error::corruption(format!(
            "{label} authenticated catalog names a missing document"
        ))),
        (None, Some(_)) => Err(error::corruption(format!(
            "{label} document is absent from its authenticated catalog"
        ))),
    }
}

fn validate_required_artifacts(
    store: &RedbStore,
    write: &redb::WriteTransaction,
    required: &[ArtifactReference],
) -> Result<(), PersistenceError> {
    let table = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    for reference in required {
        let Some(bytes) = table
            .get(reference.artifact().as_str())
            .map_err(error::redb)?
        else {
            return Err(PersistenceError::ArtifactNotCommitted(
                reference.artifact().to_string(),
            ));
        };
        let metadata: milkdrift_workspace::ArtifactMetadata =
            json::decode(bytes.value(), "artifact metadata")?;
        if metadata.reference() != reference {
            return Err(error::corruption(format!(
                "artifact metadata {} does not match the required reference",
                reference.artifact()
            )));
        }
        crate::artifact::verify_blob(
            &store.content_path(reference.digest()),
            reference,
            store.max_artifact_bytes,
        )?;
    }
    Ok(())
}

fn validate_artifact_accounting_references(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let newly_referenced: BTreeSet<_> = request.newly_referenced_artifacts.iter().collect();
    for reference in &request.required_artifacts {
        let previously_referenced =
            crate::artifact::validated_run_artifact_reference_in_transaction(
                write,
                request.receipt.run(),
                reference,
            )?;
        if newly_referenced.contains(reference) == previously_referenced {
            return Err(PersistenceError::InvalidDocument(format!(
                "artifact {} must be charged exactly on its first reference by run {}",
                reference.artifact(),
                request.receipt.run()
            )));
        }
    }
    Ok(())
}

fn validate_workspace_accounting(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(accounting) = &request.workspace_accounting else {
        return Ok(());
    };
    let actual =
        validate_or_initialize_workspace_domain(write, request.receipt.run(), &accounting.budget)?;
    let reservations = write
        .open_table(ARTIFACT_RESERVATIONS)
        .map_err(error::redb)?;
    if reservations
        .get(request.receipt.run().as_str())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(PersistenceError::Storage {
            class: milkdrift_persistence::StorageFailureClass::OwnerBusy,
            message: format!(
                "run {} has an active artifact publication",
                request.receipt.run()
            ),
        });
    }
    if actual != accounting.expected_usage {
        return Err(PersistenceError::WorkspaceUsageConflict {
            run: request.receipt.run().clone(),
        });
    }
    Ok(())
}

pub(crate) fn validate_workspace_value_accounting(
    read: &redb::ReadTransaction,
) -> Result<(), PersistenceError> {
    crate::trie::validate_roots(read)?;
    let accounting = read
        .open_table(WORKSPACE_VALUE_ACCOUNTING)
        .map_err(error::redb)?;
    if accounting.len().map_err(error::redb)? != 0 {
        return Err(error::corruption(
            "deprecated workspace integrity accounting must be empty in current storage",
        ));
    }
    Ok(())
}

pub(crate) fn validate_workspace_value_accounting_in_transaction(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    crate::trie::validate_roots_in_transaction(write)?;
    let accounting = write
        .open_table(WORKSPACE_VALUE_ACCOUNTING)
        .map_err(error::redb)?;
    if accounting.len().map_err(error::redb)? != 0 {
        return Err(error::corruption(
            "deprecated workspace integrity accounting must be empty in current storage",
        ));
    }
    Ok(())
}

fn workspace_domain_path(run: &RunId) -> [u8; 32] {
    let family = crate::trie::CatalogFamily::WorkspaceDomain;
    crate::trie::hashed_path(family, run.as_str().as_bytes())
}

fn workspace_domain_payload(
    budget: &milkdrift_workspace::WorkspaceBudget,
    usage: WorkspaceUsage,
) -> Result<[u8; 32], PersistenceError> {
    let budget = json::encode(budget, "workspace budget")?;
    let usage = json::encode(&usage, "workspace usage")?;
    let budget_length = u64::try_from(budget.len())
        .map_err(|_| error::corruption("workspace budget document length exceeds u64"))?;
    let usage_length = u64::try_from(usage.len())
        .map_err(|_| error::corruption("workspace usage document length exceeds u64"))?;
    let capacity = budget
        .len()
        .checked_add(usage.len())
        .and_then(|length| length.checked_add(16))
        .ok_or_else(|| error::corruption("workspace domain document length overflowed"))?;
    let mut document = Vec::with_capacity(capacity);
    document.extend_from_slice(&budget_length.to_be_bytes());
    document.extend_from_slice(&budget);
    document.extend_from_slice(&usage_length.to_be_bytes());
    document.extend_from_slice(&usage);
    Ok(crate::trie::digest_payload(
        crate::trie::CatalogFamily::WorkspaceDomain,
        &document,
    ))
}

fn workspace_domain_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<
    Option<(
        milkdrift_workspace::WorkspaceBudget,
        WorkspaceUsage,
        [u8; 32],
    )>,
    PersistenceError,
> {
    let budget: Option<milkdrift_workspace::WorkspaceBudget> = {
        let table = write.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
        table
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace budget"))
            .transpose()?
    };
    let usage = {
        let table = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
        table
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace usage"))
            .transpose()?
    };
    let family = crate::trie::CatalogFamily::WorkspaceDomain;
    let witness = crate::trie::verify_member_in_transaction(
        write,
        family,
        workspace_domain_path(run),
        run.as_str().as_bytes(),
    )?;
    match (budget, usage, witness) {
        (None, None, None) => Ok(None),
        (Some(budget), Some(usage), Some(witness)) => {
            budget.validate_usage(&usage).map_err(|cause| {
                error::corruption(format!("workspace usage exceeds its budget: {cause}"))
            })?;
            let expected = workspace_domain_payload(&budget, usage)?;
            if witness != expected {
                return Err(error::corruption(
                    "workspace domain disagrees with its authenticated catalog",
                ));
            }
            Ok(Some((budget, usage, witness)))
        }
        _ => Err(error::corruption(
            "workspace budget, usage, and authenticated domain are incomplete",
        )),
    }
}

fn persist_workspace_domain(
    write: &redb::WriteTransaction,
    run: &RunId,
    budget: &milkdrift_workspace::WorkspaceBudget,
    usage: WorkspaceUsage,
    previous: Option<[u8; 32]>,
) -> Result<(), PersistenceError> {
    let family = crate::trie::CatalogFamily::WorkspaceDomain;
    let replaced = crate::trie::put(
        write,
        family,
        workspace_domain_path(run),
        run.as_str().as_bytes(),
        workspace_domain_payload(budget, usage)?,
    )?;
    if replaced != previous {
        return Err(error::corruption(
            "workspace domain changed outside its authoritative transaction",
        ));
    }
    Ok(())
}

pub(crate) fn validate_workspace_value_usage_accounting_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    usage: WorkspaceUsage,
) -> Result<(), PersistenceError> {
    validate_workspace_value_accounting_in_transaction(write)?;
    let Some((_budget, stored_usage, _witness)) = workspace_domain_in_transaction(write, run)?
    else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if stored_usage != usage {
        return Err(error::corruption(
            "workspace usage argument disagrees with its durable document",
        ));
    }
    Ok(())
}

pub(crate) fn persist_workspace_value_usage_accounting_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    usage: WorkspaceUsage,
) -> Result<(), PersistenceError> {
    validate_workspace_value_accounting_in_transaction(write)?;
    let Some((budget, stored_usage, previous)) = workspace_domain_in_transaction(write, run)?
    else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if stored_usage != usage {
        return Err(error::corruption(
            "workspace usage argument disagrees with its durable document",
        ));
    }
    persist_workspace_domain(write, run, &budget, usage, Some(previous))
}

pub(crate) fn validate_workspace_domain_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    supplied: &milkdrift_workspace::WorkspaceBudget,
) -> Result<WorkspaceUsage, PersistenceError> {
    validate_workspace_value_accounting_in_transaction(write)?;
    let Some((budget, usage, _witness)) = workspace_domain_in_transaction(write, run)? else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if &budget != supplied {
        return Err(PersistenceError::ImmutableConflict {
            entity: "workspace_budget",
            identity: run.to_string(),
        });
    }
    budget.validate_usage(&usage).map_err(|cause| {
        error::corruption(format!(
            "workspace usage exceeds its durable budget: {cause}"
        ))
    })?;
    Ok(usage)
}

pub(crate) fn validate_or_initialize_workspace_domain(
    write: &redb::WriteTransaction,
    run: &RunId,
    supplied: &milkdrift_workspace::WorkspaceBudget,
) -> Result<WorkspaceUsage, PersistenceError> {
    validate_workspace_value_accounting_in_transaction(write)?;
    match workspace_domain_in_transaction(write, run)? {
        None => {
            supplied
                .validate_usage(&WorkspaceUsage::EMPTY)
                .map_err(|cause| {
                    PersistenceError::InvalidDocument(format!(
                        "empty workspace usage violates the supplied budget: {cause}"
                    ))
                })?;
            let budget_bytes = json::encode(supplied, "workspace budget")?;
            let usage_bytes = json::encode(&WorkspaceUsage::EMPTY, "workspace usage")?;
            write
                .open_table(WORKSPACE_BUDGETS)
                .map_err(error::redb)?
                .insert(run.as_str(), budget_bytes.as_slice())
                .map_err(error::redb)?;
            write
                .open_table(WORKSPACE_USAGE)
                .map_err(error::redb)?
                .insert(run.as_str(), usage_bytes.as_slice())
                .map_err(error::redb)?;
            persist_workspace_domain(write, run, supplied, WorkspaceUsage::EMPTY, None)?;
            Ok(WorkspaceUsage::EMPTY)
        }
        Some((_budget, _usage, _witness)) => {
            validate_workspace_domain_in_transaction(write, run, supplied)
        }
    }
}

pub(crate) fn advance_workspace_global_usage_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    expected: WorkspaceUsage,
    resulting: WorkspaceUsage,
) -> Result<(), PersistenceError> {
    validate_workspace_value_accounting_in_transaction(write)?;
    let Some((budget, actual, previous)) = workspace_domain_in_transaction(write, run)? else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if actual != expected {
        return Err(error::corruption(
            "workspace usage changed before artifact accounting advance",
        ));
    }
    budget.validate_usage(&resulting).map_err(|cause| {
        PersistenceError::InvalidDocument(format!("workspace usage exceeds budget: {cause}"))
    })?;
    persist_workspace_domain(write, run, &budget, resulting, Some(previous))
}

fn persist_workspace_accounting(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(accounting) = &request.workspace_accounting else {
        return Ok(());
    };
    let Some((budget, actual, previous)) =
        workspace_domain_in_transaction(write, request.receipt.run())?
    else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if actual != accounting.expected_usage {
        return Err(error::corruption(
            "workspace usage changed before command accounting persistence",
        ));
    }
    let bytes = json::encode(&accounting.resulting_usage, "workspace usage")?;
    let mut table = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    table
        .insert(request.receipt.run().as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    drop(table);
    persist_workspace_domain(
        write,
        request.receipt.run(),
        &budget,
        accounting.resulting_usage,
        Some(previous),
    )
}

fn append_events(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let mut events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    let mut checksums = write.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
    for event in &request.events {
        let key = codec::run_sequence(event.run_id().as_str(), event.sequence())?;
        if events.get(key.as_slice()).map_err(error::redb)?.is_some() {
            return Err(error::corruption(format!(
                "event slot {}:{} already exists beyond the authoritative head",
                event.run_id(),
                event.sequence()
            )));
        }
        if checksums
            .get(event.event_id().as_str())
            .map_err(error::redb)?
            .is_some()
        {
            return Err(PersistenceError::ImmutableConflict {
                entity: "event",
                identity: event.event_id().to_string(),
            });
        }
        let document = event.to_canonical_json()?;
        if events
            .insert(key.as_slice(), document.as_slice())
            .map_err(error::redb)?
            .is_some()
        {
            return Err(error::corruption("event append overwrote an existing slot"));
        }
        if checksums
            .insert(event.event_id().as_str(), event.checksum().as_str())
            .map_err(error::redb)?
            .is_some()
        {
            return Err(error::corruption(
                "event append overwrote an existing checksum identity",
            ));
        }
        let family = crate::trie::CatalogFamily::Event;
        if crate::trie::put(
            write,
            family,
            event_catalog_path(event.run_id(), event.sequence(), &key)?,
            &key,
            crate::trie::digest_payload(family, &document),
        )?
        .is_some()
        {
            return Err(error::corruption(
                "event append replaced an authenticated event leaf",
            ));
        }
        crate::snapshot::append_history_checkpoint(write, event)?;
    }
    Ok(())
}

fn apply_workspace(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    validate_workspace_value_accounting_in_transaction(write)?;
    let mut scopes = write.open_table(SCOPES).map_err(error::redb)?;
    let mut roots = write.open_table(ROOT_SCOPES).map_err(error::redb)?;
    let mut values = write.open_table(VALUES).map_err(error::redb)?;

    for mutation in &request.workspace {
        match mutation {
            WorkspaceMutation::CreateScope { scope } => {
                put_scope(write, &mut scopes, &mut roots, scope)?;
            }
            WorkspaceMutation::PutValue { entry } => {
                put_value(write, &scopes, &roots, &mut values, entry)?;
            }
        }
    }
    let Some(accounting) = &request.workspace_accounting else {
        return Ok(());
    };
    let value_delta = accounting
        .resulting_usage
        .value_versions()
        .checked_sub(accounting.expected_usage.value_versions())
        .ok_or_else(|| error::corruption("workspace value accounting moved backwards"))?;
    let mutation_count = u64::try_from(
        request
            .workspace
            .iter()
            .filter(|mutation| matches!(mutation, WorkspaceMutation::PutValue { .. }))
            .count(),
    )
    .map_err(|_| error::corruption("workspace value mutation count exceeds u64"))?;
    if value_delta != mutation_count {
        return Err(error::corruption(
            "workspace usage delta does not match inserted value versions",
        ));
    }
    drop(values);
    drop(roots);
    drop(scopes);
    Ok(())
}

fn workspace_scope_run_group(run: &RunId) -> [u8; 16] {
    let family = crate::trie::CatalogFamily::WorkspaceScope;
    let hash = crate::trie::hashed_path(family, run.as_str().as_bytes());
    let mut group = [0_u8; 16];
    group.copy_from_slice(&hash[..16]);
    group
}

fn workspace_scope_catalog_path(
    reference: &ScopeReference,
    logical_key: &[u8],
) -> Result<[u8; 32], PersistenceError> {
    crate::trie::ordered_path(
        crate::trie::CatalogFamily::WorkspaceScope,
        &workspace_scope_run_group(reference.run()),
        logical_key,
    )
}

fn ensure_workspace_run_has_no_scopes(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<(), PersistenceError> {
    let mut first = [0_u8; 32];
    first[..16].copy_from_slice(&workspace_scope_run_group(run));
    let page = crate::trie::page_in_transaction(
        write,
        crate::trie::CatalogFamily::WorkspaceScope,
        None,
        predecessor_path(first),
        1,
    )?;
    if page
        .leaves
        .first()
        .is_some_and(|leaf| leaf.path[..16] == first[..16])
    {
        return Err(error::corruption(
            "run has an authenticated workspace scope but no root-scope index",
        ));
    }
    Ok(())
}

fn validate_scope_catalog_lineage_in_transaction(
    write: &redb::WriteTransaction,
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    reference: &ScopeReference,
) -> Result<(), PersistenceError> {
    let mut current = reference.clone();
    let mut seen = BTreeSet::new();
    for depth in 0..MAX_SCOPE_DEPTH {
        if !seen.insert(current.clone()) {
            return Err(error::corruption(
                "workspace scope lineage contains a cycle",
            ));
        }
        let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
        let family = crate::trie::CatalogFamily::WorkspaceScope;
        let witness = crate::trie::verify_member_in_transaction(
            write,
            family,
            workspace_scope_catalog_path(&current, &key)?,
            &key,
        )?;
        let bytes = scopes.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = bytes else {
            return if witness.is_some() || depth > 0 {
                Err(error::corruption("workspace scope lineage is incomplete"))
            } else {
                Err(PersistenceError::NotFound {
                    entity: "workspace_scope",
                    identity: format!("{}/{}", current.run(), current.scope()),
                })
            };
        };
        if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
            return Err(error::corruption(
                "workspace scope disagrees with its authenticated catalog",
            ));
        }
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        if scope.reference() != &current {
            return Err(error::corruption(
                "workspace-scope key does not match its document",
            ));
        }
        match (scope.kind(), scope.parent()) {
            (ScopeKind::RunRoot, None) => {
                let root = roots
                    .get(current.run().as_str())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption("run-root scope is missing from its root index")
                    })?;
                if root.value() != current.scope().as_str() {
                    return Err(error::corruption(
                        "run-root scope disagrees with its root index",
                    ));
                }
                return Ok(());
            }
            (ScopeKind::RunRoot, Some(_)) => {
                return Err(error::corruption("run-root scope has a parent"));
            }
            (_, Some(parent)) => current = parent.clone(),
            (_, None) => {
                return Err(error::corruption("non-root workspace scope has no parent"));
            }
        }
    }
    Err(error::corruption(format!(
        "workspace scope lineage exceeds {MAX_SCOPE_DEPTH} entries"
    )))
}

fn validate_scope_catalog_lineage(
    read: &redb::ReadTransaction,
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    reference: &ScopeReference,
) -> Result<(), PersistenceError> {
    require_run_history_membership(read, reference.run())?;
    let mut current = reference.clone();
    let mut seen = BTreeSet::new();
    for depth in 0..MAX_SCOPE_DEPTH {
        if !seen.insert(current.clone()) {
            return Err(error::corruption(
                "workspace scope lineage contains a cycle",
            ));
        }
        let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
        let family = crate::trie::CatalogFamily::WorkspaceScope;
        let witness = crate::trie::verify_member(
            read,
            family,
            workspace_scope_catalog_path(&current, &key)?,
            &key,
        )?;
        let bytes = scopes.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = bytes else {
            return if witness.is_some() || depth > 0 {
                Err(error::corruption("workspace scope lineage is incomplete"))
            } else {
                Err(PersistenceError::NotFound {
                    entity: "workspace_scope",
                    identity: format!("{}/{}", current.run(), current.scope()),
                })
            };
        };
        if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
            return Err(error::corruption(
                "workspace scope disagrees with its authenticated catalog",
            ));
        }
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        if scope.reference() != &current {
            return Err(error::corruption(
                "workspace-scope key does not match its document",
            ));
        }
        match (scope.kind(), scope.parent()) {
            (ScopeKind::RunRoot, None) => {
                let root = roots
                    .get(current.run().as_str())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption("run-root scope is missing from its root index")
                    })?;
                if root.value() != current.scope().as_str() {
                    return Err(error::corruption(
                        "run-root scope disagrees with its root index",
                    ));
                }
                return Ok(());
            }
            (ScopeKind::RunRoot, Some(_)) => {
                return Err(error::corruption("run-root scope has a parent"));
            }
            (_, Some(parent)) => current = parent.clone(),
            (_, None) => {
                return Err(error::corruption("non-root workspace scope has no parent"));
            }
        }
    }
    Err(error::corruption(format!(
        "workspace scope lineage exceeds {MAX_SCOPE_DEPTH} entries"
    )))
}

fn require_run_history_membership(
    read: &redb::ReadTransaction,
    run: &RunId,
) -> Result<(), PersistenceError> {
    let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let head = validated_run_head(&heads, &events, run)?;
    if validate_run_history_membership(read, run, head)?.is_none() {
        return Err(error::corruption(
            "durable workspace fact has no authenticated owning run",
        ));
    }
    Ok(())
}

fn require_prior_run_history_membership_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<(), PersistenceError> {
    let head = {
        let heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
        heads
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|head| RunSequence::new(head.value()))
            .ok_or_else(|| {
                error::corruption("cross-run workspace provenance has no durable run head")
            })?
    };
    let summary_bytes = {
        let summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        summaries
            .get(run.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("cross-run workspace provenance has no run summary"))?
            .value()
            .to_vec()
    };
    let summary: RunSummaryIndex = json::decode(&summary_bytes, "run summary")?;
    if summary.run != *run || summary.through_sequence != head {
        return Err(error::corruption(
            "cross-run workspace provenance summary disagrees with its head",
        ));
    }
    let family = crate::trie::CatalogFamily::RunMembership;
    let witness = crate::trie::verify_member_in_transaction(
        write,
        family,
        run_membership_path(run),
        run.as_str().as_bytes(),
    )?;
    let payload = run_membership_payload(run, head, &summary_bytes);
    if witness != Some(payload) {
        return Err(error::corruption(
            "cross-run workspace provenance has no authenticated run membership",
        ));
    }
    validate_nonterminal_membership_in_transaction(write, &summary, payload)
}

fn validate_workspace_value_catalog_provenance_in_transaction(
    write: &redb::WriteTransaction,
    values: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    selected: &WorkspaceValueEntry,
    proposed: bool,
) -> Result<(), PersistenceError> {
    let mut current = selected.clone();
    for depth in 0..MAX_VALUE_PROVENANCE_DEPTH {
        if current.reference().scope().run() != selected.reference().scope().run() {
            require_prior_run_history_membership_in_transaction(
                write,
                current.reference().scope().run(),
            )?;
        }
        validate_scope_catalog_lineage_in_transaction(
            write,
            scopes,
            roots,
            current.reference().scope(),
        )?;
        if !(proposed && depth == 0) {
            let key = workspace_value_key(current.reference())?;
            let bytes = values
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("workspace provenance value is missing"))?;
            let family = crate::trie::CatalogFamily::WorkspaceValue;
            let witness = crate::trie::verify_member_in_transaction(
                write,
                family,
                crate::trie::hashed_path(family, &key),
                &key,
            )?;
            if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
                return Err(error::corruption(
                    "workspace value disagrees with its authenticated catalog",
                ));
            }
        }
        let (source, missing_entity) = match current.origin() {
            ValueOrigin::Initial => return Ok(()),
            ValueOrigin::Successor { previous } => (previous, "previous_workspace_value"),
            ValueOrigin::Inherited { source } => (source, "inherited_workspace_value"),
            ValueOrigin::Imported { source } => (source, "imported_workspace_value"),
        };
        let key = workspace_value_key(source)?;
        let stored = values
            .get(key.as_slice())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec());
        let family = crate::trie::CatalogFamily::WorkspaceValue;
        let witness = crate::trie::verify_member_in_transaction(
            write,
            family,
            crate::trie::hashed_path(family, &key),
            &key,
        )?;
        current = match (stored.as_deref(), witness) {
            (Some(bytes), Some(witness))
                if witness == crate::trie::digest_payload(family, bytes) =>
            {
                let source_entry: WorkspaceValueEntry = json::decode(bytes, "workspace value")?;
                if source_entry.reference() != source {
                    return Err(error::corruption(
                        "workspace provenance key disagrees with its document",
                    ));
                }
                source_entry
            }
            (None, None) if proposed && depth == 0 => {
                return Err(PersistenceError::NotFound {
                    entity: missing_entity,
                    identity: format!(
                        "{}/{}/{}/{}",
                        source.scope().run(),
                        source.scope().scope(),
                        source.key(),
                        source.version()
                    ),
                });
            }
            _ => {
                return Err(error::corruption(
                    "workspace provenance source and authenticated catalog disagree",
                ));
            }
        };
    }
    Err(error::corruption(format!(
        "workspace value provenance exceeds {MAX_VALUE_PROVENANCE_DEPTH} entries"
    )))
}

fn validate_workspace_value_catalog_provenance(
    read: &redb::ReadTransaction,
    values: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    selected: &WorkspaceValueEntry,
) -> Result<(), PersistenceError> {
    let mut current = selected.clone();
    for _ in 0..MAX_VALUE_PROVENANCE_DEPTH {
        validate_scope_catalog_lineage(read, scopes, roots, current.reference().scope())?;
        let key = workspace_value_key(current.reference())?;
        let bytes = values
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("workspace provenance value is missing"))?;
        let family = crate::trie::CatalogFamily::WorkspaceValue;
        let witness =
            crate::trie::verify_member(read, family, crate::trie::hashed_path(family, &key), &key)?;
        if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
            return Err(error::corruption(
                "workspace value disagrees with its authenticated catalog",
            ));
        }
        let source = match current.origin() {
            ValueOrigin::Initial => return Ok(()),
            ValueOrigin::Successor { previous } => previous,
            ValueOrigin::Inherited { source } | ValueOrigin::Imported { source } => source,
        };
        current = load_provenance_value(values, source, true, "workspace_value")?;
    }
    Err(error::corruption(format!(
        "workspace value provenance exceeds {MAX_VALUE_PROVENANCE_DEPTH} entries"
    )))
}

fn update_workspace_value_head(
    write: &redb::WriteTransaction,
    reference: &WorkspaceValueReference,
    value_key: &[u8],
    value_bytes: &[u8],
) -> Result<(), PersistenceError> {
    let head_key = codec::value_prefix(
        reference.scope().run().as_str(),
        reference.scope().scope().as_str(),
        reference.key().as_str(),
    )?;
    let previous_bytes = {
        let heads = write
            .open_table(WORKSPACE_VALUE_HEADS)
            .map_err(error::redb)?;
        heads
            .get(head_key.as_slice())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec())
    };
    let family = crate::trie::CatalogFamily::WorkspaceValueHead;
    let path = crate::trie::hashed_path(family, &head_key);
    let previous_witness =
        crate::trie::verify_member_in_transaction(write, family, path, &head_key)?;
    match previous_bytes.as_deref() {
        None if previous_witness.is_none() && reference.version().get() == 1 => {}
        Some(bytes) if previous_witness == Some(crate::trie::digest_payload(family, bytes)) => {
            let previous: WorkspaceValueReference = json::decode(bytes, "workspace value head")?;
            let expected_version = previous
                .version()
                .get()
                .checked_add(1)
                .ok_or_else(|| error::corruption("workspace value version overflowed"))?;
            if previous.scope() != reference.scope()
                || previous.key() != reference.key()
                || expected_version != reference.version().get()
            {
                return Err(error::corruption(
                    "workspace value head is not the immediate predecessor",
                ));
            }
        }
        None if previous_witness.is_none() => {
            return Err(error::corruption(
                "workspace value sequence begins after version one",
            ));
        }
        _ => {
            return Err(error::corruption(
                "workspace value head disagrees with its authenticated catalog",
            ));
        }
    }
    let head_bytes = json::encode(reference, "workspace value head")?;
    {
        let mut heads = write
            .open_table(WORKSPACE_VALUE_HEADS)
            .map_err(error::redb)?;
        let replaced = heads
            .insert(head_key.as_slice(), head_bytes.as_slice())
            .map_err(error::redb)?;
        if replaced.as_ref().map(|bytes| bytes.value()) != previous_bytes.as_deref() {
            return Err(error::corruption(
                "workspace value head changed outside its authoritative transaction",
            ));
        }
    }
    let replaced_witness = crate::trie::put(
        write,
        family,
        path,
        &head_key,
        crate::trie::digest_payload(family, &head_bytes),
    )?;
    if replaced_witness != previous_witness {
        return Err(error::corruption(
            "workspace value head witness changed outside its transaction",
        ));
    }
    let value_family = crate::trie::CatalogFamily::WorkspaceValue;
    let value_witness = crate::trie::verify_member_in_transaction(
        write,
        value_family,
        crate::trie::hashed_path(value_family, value_key),
        value_key,
    )?;
    if value_witness != Some(crate::trie::digest_payload(value_family, value_bytes)) {
        return Err(error::corruption(
            "workspace value head does not name an authenticated value",
        ));
    }
    Ok(())
}

fn put_scope(
    write: &redb::WriteTransaction,
    scopes: &mut Table<'_, &[u8], &[u8]>,
    roots: &mut Table<'_, &str, &str>,
    scope: &WorkspaceScope,
) -> Result<(), PersistenceError> {
    let reference = scope.reference();
    let key = codec::pair(reference.run().as_str(), reference.scope().as_str())?;
    let family = crate::trie::CatalogFamily::WorkspaceScope;
    if crate::trie::verify_member_in_transaction(
        write,
        family,
        workspace_scope_catalog_path(reference, &key)?,
        &key,
    )?
    .is_some()
    {
        return Err(error::corruption(
            "workspace scope catalog names an existing immutable scope",
        ));
    }
    if scopes.get(key.as_slice()).map_err(error::redb)?.is_some() {
        return Err(PersistenceError::ImmutableConflict {
            entity: "workspace_scope",
            identity: format!("{}/{}", reference.run(), reference.scope()),
        });
    }
    match (scope.kind(), scope.parent()) {
        (ScopeKind::RunRoot, None) => {
            ensure_workspace_run_has_no_scopes(write, reference.run())?;
            if roots
                .get(reference.run().as_str())
                .map_err(error::redb)?
                .is_some()
            {
                return Err(PersistenceError::ImmutableConflict {
                    entity: "workspace_root_scope",
                    identity: reference.run().to_string(),
                });
            }
            roots
                .insert(reference.run().as_str(), reference.scope().as_str())
                .map_err(error::redb)?;
        }
        (_, Some(parent)) => {
            validate_new_scope_depth(write, scopes, roots, parent)?;
        }
        _ => {
            return Err(PersistenceError::InvalidDocument(
                "workspace scope kind/parent invariant failed".to_owned(),
            ));
        }
    }
    let bytes = json::encode(scope, "workspace scope")?;
    if scopes
        .insert(key.as_slice(), bytes.as_slice())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(error::corruption(
            "workspace scope insert replaced an existing document",
        ));
    }
    if crate::trie::put(
        write,
        family,
        workspace_scope_catalog_path(reference, &key)?,
        &key,
        crate::trie::digest_payload(family, &bytes),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "workspace scope insert replaced an authenticated leaf",
        ));
    }
    Ok(())
}

fn put_value(
    write: &redb::WriteTransaction,
    scopes: &Table<'_, &[u8], &[u8]>,
    roots: &Table<'_, &str, &str>,
    values: &mut Table<'_, &[u8], &[u8]>,
    entry: &WorkspaceValueEntry,
) -> Result<(), PersistenceError> {
    let reference = entry.reference();
    let scope = reference.scope();
    validate_scope_catalog_lineage_in_transaction(write, scopes, roots, scope)?;
    let key = workspace_value_key(reference)?;
    let family = crate::trie::CatalogFamily::WorkspaceValue;
    if crate::trie::verify_member_in_transaction(
        write,
        family,
        crate::trie::hashed_path(family, &key),
        &key,
    )?
    .is_some()
    {
        return Err(error::corruption(
            "workspace value catalog names an existing immutable value",
        ));
    }
    if values.get(key.as_slice()).map_err(error::redb)?.is_some() {
        return Err(PersistenceError::ImmutableConflict {
            entity: "workspace_value",
            identity: format!(
                "{}/{}/{}/{}",
                scope.run(),
                scope.scope(),
                reference.key(),
                reference.version()
            ),
        });
    }
    validate_workspace_value_catalog_provenance_in_transaction(
        write, values, scopes, roots, entry, true,
    )?;
    validate_workspace_value_provenance(values, scopes, roots, entry, true)?;
    let bytes = json::encode(entry, "workspace value")?;
    if values
        .insert(key.as_slice(), bytes.as_slice())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(error::corruption(
            "workspace value insert replaced an existing document",
        ));
    }
    if crate::trie::put(
        write,
        family,
        crate::trie::hashed_path(family, &key),
        &key,
        crate::trie::digest_payload(family, &bytes),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "workspace value insert replaced an authenticated leaf",
        ));
    }
    update_workspace_value_head(write, reference, &key, &bytes)?;
    Ok(())
}

pub(crate) fn migrate_workspace_catalogs(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    crate::trie::validate_roots_in_transaction(write)?;
    {
        let heads = write
            .open_table(WORKSPACE_VALUE_HEADS)
            .map_err(error::redb)?;
        if heads.len().map_err(error::redb)? != 0 {
            return Err(error::corruption(
                "legacy storage unexpectedly contains workspace value heads",
            ));
        }
    }

    let budgets = write.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
    let usages = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    let mut expected_versions = 0_u64;
    let mut expected_inline_bytes = 0_u64;
    for item in budgets.iter().map_err(error::redb)? {
        let (run_key, budget_bytes) = item.map_err(error::redb)?;
        let run = RunId::new(run_key.value()).map_err(|cause| {
            error::corruption(format!("invalid legacy workspace-domain identity: {cause}"))
        })?;
        let budget: milkdrift_workspace::WorkspaceBudget =
            json::decode(budget_bytes.value(), "workspace budget")?;
        let usage_bytes = usages
            .get(run.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("legacy workspace budget has no usage document"))?;
        let usage: WorkspaceUsage = json::decode(usage_bytes.value(), "workspace usage")?;
        budget.validate_usage(&usage).map_err(|cause| {
            error::corruption(format!(
                "legacy workspace usage exceeds its budget: {cause}"
            ))
        })?;
        expected_versions = expected_versions
            .checked_add(usage.value_versions())
            .ok_or_else(|| error::corruption("legacy workspace value total overflowed"))?;
        expected_inline_bytes = expected_inline_bytes
            .checked_add(usage.inline_bytes())
            .ok_or_else(|| error::corruption("legacy workspace byte total overflowed"))?;
        persist_workspace_domain(write, &run, &budget, usage, None)?;
    }
    for item in usages.iter().map_err(error::redb)? {
        let (run, _) = item.map_err(error::redb)?;
        if budgets.get(run.value()).map_err(error::redb)?.is_none() {
            return Err(error::corruption(
                "legacy workspace usage has no immutable budget",
            ));
        }
    }
    drop(usages);
    drop(budgets);

    let scopes = write.open_table(SCOPES).map_err(error::redb)?;
    let roots = write.open_table(ROOT_SCOPES).map_err(error::redb)?;
    for item in scopes.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        let expected_key = codec::pair(
            scope.reference().run().as_str(),
            scope.reference().scope().as_str(),
        )?;
        if key.value() != expected_key.as_slice() {
            return Err(error::corruption(
                "legacy workspace-scope key disagrees with its document",
            ));
        }
        let family = crate::trie::CatalogFamily::WorkspaceScope;
        if crate::trie::put(
            write,
            family,
            workspace_scope_catalog_path(scope.reference(), key.value())?,
            key.value(),
            crate::trie::digest_payload(family, bytes.value()),
        )?
        .is_some()
        {
            return Err(error::corruption(
                "legacy workspace scope duplicates an authenticated catalog leaf",
            ));
        }
    }
    for item in scopes.iter().map_err(error::redb)? {
        let (_, bytes) = item.map_err(error::redb)?;
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        validate_scope_catalog_lineage_in_transaction(write, &scopes, &roots, scope.reference())?;
    }

    let values = write.open_table(VALUES).map_err(error::redb)?;
    let mut observed_versions = 0_u64;
    let mut observed_inline_bytes = 0_u64;
    let mut current_run: Option<RunId> = None;
    let mut current_versions = 0_u64;
    let mut current_inline_bytes = 0_u64;
    for item in values.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let entry: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
        let expected_key = workspace_value_key(entry.reference())?;
        if key.value() != expected_key.as_slice() {
            return Err(error::corruption(
                "legacy workspace-value key disagrees with its document",
            ));
        }
        let run = entry.reference().scope().run();
        if current_run.as_ref().is_some_and(|current| current != run) {
            validate_migrated_value_usage(
                write,
                current_run.as_ref().ok_or_else(|| {
                    error::corruption("legacy workspace value run tracker is empty")
                })?,
                current_versions,
                current_inline_bytes,
            )?;
            current_versions = 0;
            current_inline_bytes = 0;
        }
        current_run = Some(run.clone());
        current_versions = current_versions
            .checked_add(1)
            .ok_or_else(|| error::corruption("legacy per-run value count overflowed"))?;
        observed_versions = observed_versions
            .checked_add(1)
            .ok_or_else(|| error::corruption("legacy workspace value count overflowed"))?;
        if let Some(inline) = entry.value().as_json() {
            let inline_bytes = u64::try_from(
                serde_json::to_vec(inline)
                    .map_err(|cause| {
                        error::corruption(format!(
                            "legacy inline workspace value cannot be encoded: {cause}"
                        ))
                    })?
                    .len(),
            )
            .map_err(|_| error::corruption("legacy inline workspace value exceeds u64"))?;
            current_inline_bytes = current_inline_bytes
                .checked_add(inline_bytes)
                .ok_or_else(|| error::corruption("legacy per-run inline bytes overflowed"))?;
            observed_inline_bytes = observed_inline_bytes
                .checked_add(inline_bytes)
                .ok_or_else(|| error::corruption("legacy workspace inline bytes overflowed"))?;
        }
        validate_owning_workspace_scope(&scopes, &roots, entry.reference().scope())?;
        let family = crate::trie::CatalogFamily::WorkspaceValue;
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
                "legacy workspace value duplicates an authenticated catalog leaf",
            ));
        }
        update_workspace_value_head(write, entry.reference(), key.value(), bytes.value())?;
    }
    if let Some(run) = &current_run {
        validate_migrated_value_usage(write, run, current_versions, current_inline_bytes)?;
    }
    if observed_versions != expected_versions || observed_inline_bytes != expected_inline_bytes {
        return Err(error::corruption(
            "legacy workspace values disagree with aggregate usage",
        ));
    }
    for item in values.iter().map_err(error::redb)? {
        let (_, bytes) = item.map_err(error::redb)?;
        let entry: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
        validate_workspace_value_provenance(&values, &scopes, &roots, &entry, false)?;
        validate_scope_catalog_lineage_in_transaction(
            write,
            &scopes,
            &roots,
            entry.reference().scope(),
        )?;
        validate_workspace_value_catalog_provenance_in_transaction(
            write, &values, &scopes, &roots, &entry, false,
        )?;
    }
    Ok(())
}

fn validate_migrated_value_usage(
    write: &redb::WriteTransaction,
    run: &RunId,
    value_versions: u64,
    inline_bytes: u64,
) -> Result<(), PersistenceError> {
    let usages = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    let usage = usages
        .get(run.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("legacy workspace value has no usage domain"))?;
    let usage: WorkspaceUsage = json::decode(usage.value(), "workspace usage")?;
    if usage.value_versions() != value_versions || usage.inline_bytes() != inline_bytes {
        return Err(error::corruption(
            "legacy per-run workspace values disagree with durable usage",
        ));
    }
    Ok(())
}

fn load_provenance_value(
    values: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    reference: &WorkspaceValueReference,
    missing_is_corruption: bool,
    missing_entity: &'static str,
) -> Result<WorkspaceValueEntry, PersistenceError> {
    let key = workspace_value_key(reference)?;
    let bytes = values.get(key.as_slice()).map_err(error::redb)?;
    let bytes = match bytes {
        Some(bytes) => bytes,
        None if missing_is_corruption => {
            return Err(error::corruption(format!(
                "workspace provenance source {}/{}/{}/{} is missing",
                reference.scope().run(),
                reference.scope().scope(),
                reference.key(),
                reference.version()
            )));
        }
        None => {
            return Err(PersistenceError::NotFound {
                entity: missing_entity,
                identity: format!(
                    "{}/{}/{}/{}",
                    reference.scope().run(),
                    reference.scope().scope(),
                    reference.key(),
                    reference.version()
                ),
            });
        }
    };
    let stored: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
    if stored.reference() != reference {
        return Err(error::corruption(
            "workspace-value key does not match its document",
        ));
    }
    Ok(stored)
}

pub(crate) fn validate_workspace_value_provenance(
    values: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    selected: &WorkspaceValueEntry,
    proposed: bool,
) -> Result<(), PersistenceError> {
    let mut current = selected.clone();
    let mut seen = BTreeSet::new();
    for depth in 0..MAX_VALUE_PROVENANCE_DEPTH {
        if !seen.insert(current.reference().clone()) {
            return Err(error::corruption(
                "workspace value provenance contains a cycle",
            ));
        }
        validate_owning_workspace_scope(scopes, roots, current.reference().scope())?;
        let (source, missing_entity) = match current.origin() {
            ValueOrigin::Initial => return Ok(()),
            ValueOrigin::Successor { previous } => (previous, "previous_workspace_value"),
            ValueOrigin::Inherited { source } => {
                require_ancestor(scopes, source.scope(), current.reference().scope()).map_err(
                    |cause| {
                        if proposed && depth == 0 {
                            cause
                        } else {
                            error::corruption(format!(
                                "stored inherited workspace ancestry is invalid: {cause}"
                            ))
                        }
                    },
                )?;
                (source, "inherited_workspace_value")
            }
            ValueOrigin::Imported { source } => (source, "imported_workspace_value"),
        };
        let source_entry =
            load_provenance_value(values, source, !proposed || depth > 0, missing_entity)?;
        match current.origin() {
            ValueOrigin::Inherited { .. } if source_entry.value() != current.value() => {
                let message =
                    "an inherited workspace value must preserve its exact ancestor content";
                return if proposed && depth == 0 {
                    Err(PersistenceError::InvalidDocument(message.to_owned()))
                } else {
                    Err(error::corruption(message))
                };
            }
            ValueOrigin::Imported { .. } if source_entry.value() != current.value() => {
                let message = "an imported workspace value must preserve its exact source content";
                return if proposed && depth == 0 {
                    Err(PersistenceError::InvalidDocument(message.to_owned()))
                } else {
                    Err(error::corruption(message))
                };
            }
            _ => {}
        }
        current = source_entry;
    }
    if proposed {
        Err(PersistenceError::Bounds {
            location: "workspace_value.provenance",
            reason: format!("provenance may contain at most {MAX_VALUE_PROVENANCE_DEPTH} entries"),
        })
    } else {
        Err(error::corruption(format!(
            "workspace value provenance exceeds {MAX_VALUE_PROVENANCE_DEPTH} entries"
        )))
    }
}

fn validate_new_scope_depth(
    write: &redb::WriteTransaction,
    scopes: &Table<'_, &[u8], &[u8]>,
    roots: &Table<'_, &str, &str>,
    parent: &ScopeReference,
) -> Result<(), PersistenceError> {
    let mut current = Some(parent.clone());
    let mut total_depth = 1_usize; // Include the new child being validated.
    let mut seen = BTreeSet::new();
    while let Some(reference) = current {
        total_depth = total_depth.saturating_add(1);
        if total_depth > MAX_SCOPE_DEPTH {
            return Err(PersistenceError::InvalidDocument(format!(
                "workspace scope lineage may contain at most {MAX_SCOPE_DEPTH} entries"
            )));
        }
        if !seen.insert(reference.clone()) {
            return Err(error::corruption(
                "workspace scope lineage contains a cycle",
            ));
        }
        let key = codec::pair(reference.run().as_str(), reference.scope().as_str())?;
        let bytes = scopes.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = bytes else {
            let family = crate::trie::CatalogFamily::WorkspaceScope;
            if crate::trie::verify_member_in_transaction(
                write,
                family,
                workspace_scope_catalog_path(&reference, &key)?,
                &key,
            )?
            .is_some()
            {
                return Err(error::corruption(
                    "parent workspace scope catalog names a missing document",
                ));
            }
            return Err(PersistenceError::NotFound {
                entity: "parent_workspace_scope",
                identity: format!("{}/{}", reference.run(), reference.scope()),
            });
        };
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        if scope.reference() != &reference {
            return Err(error::corruption(
                "workspace-scope key does not match its document",
            ));
        }
        let family = crate::trie::CatalogFamily::WorkspaceScope;
        let witness = crate::trie::verify_member_in_transaction(
            write,
            family,
            workspace_scope_catalog_path(&reference, &key)?,
            &key,
        )?;
        if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
            return Err(error::corruption(
                "parent workspace scope disagrees with its authenticated catalog",
            ));
        }
        match (scope.kind(), scope.parent()) {
            (ScopeKind::RunRoot, None) => {
                let indexed = roots
                    .get(reference.run().as_str())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption("parent lineage root is missing from its root index")
                    })?;
                if indexed.value() != reference.scope().as_str() {
                    return Err(error::corruption(
                        "parent lineage root disagrees with its root index",
                    ));
                }
                current = None;
            }
            (ScopeKind::RunRoot, Some(_)) => {
                return Err(error::corruption("parent lineage run root has a parent"));
            }
            (_, Some(next)) => current = Some(next.clone()),
            (_, None) => {
                return Err(error::corruption(
                    "parent lineage non-root scope has no parent",
                ));
            }
        }
    }
    Ok(())
}

fn require_ancestor(
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    candidate: &ScopeReference,
    leaf: &ScopeReference,
) -> Result<(), PersistenceError> {
    let mut current = leaf.clone();
    for _ in 0..MAX_SCOPE_DEPTH {
        if &current == candidate {
            return Ok(());
        }
        let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
        let bytes = scopes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("workspace scope lineage is incomplete"))?;
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        if scope.reference() != &current {
            return Err(error::corruption(
                "workspace-scope key does not match its document",
            ));
        }
        let Some(parent) = scope.parent() else {
            break;
        };
        current = parent.clone();
    }
    Err(PersistenceError::InvalidDocument(format!(
        "scope {candidate:?} is not an ancestor of {leaf:?}"
    )))
}

pub(crate) fn workspace_value_key(
    reference: &WorkspaceValueReference,
) -> Result<Vec<u8>, PersistenceError> {
    codec::value(
        reference.scope().run().as_str(),
        reference.scope().scope().as_str(),
        reference.key().as_str(),
        reference.version().get(),
    )
}

fn apply_indexes(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(summary) = &request.indexes.summary else {
        return Ok(());
    };
    let summary_bytes = json::encode(summary, "run summary")?;
    let previous = {
        let mut summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        let previous = summaries
            .get(summary.run.as_str())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec());
        summaries
            .insert(summary.run.as_str(), summary_bytes.as_slice())
            .map_err(error::redb)?;
        previous
    };
    transition_nonterminal_membership(write, summary, &summary_bytes, previous.as_deref())?;
    apply_runnable_mutations(write, &request.indexes.runnable)?;
    apply_timer_mutations(write, &request.indexes.timers)?;
    apply_lease_mutations(write, &request.indexes.leases)?;
    Ok(())
}

fn transition_nonterminal_membership(
    write: &redb::WriteTransaction,
    summary: &RunSummaryIndex,
    summary_bytes: &[u8],
    previous_summary_bytes: Option<&[u8]>,
) -> Result<(), PersistenceError> {
    let family = crate::trie::CatalogFamily::NonterminalRun;
    let path = nonterminal_membership_path(&summary.run);
    let previous_witness = crate::trie::verify_member_in_transaction(
        write,
        family,
        path,
        summary.run.as_str().as_bytes(),
    )?;
    let previous_marker = {
        let nonterminal = write.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
        nonterminal
            .get(summary.run.as_str())
            .map_err(error::redb)?
            .map(|marker| marker.value())
    };
    let previous_expected = match previous_summary_bytes {
        None => {
            if previous_marker.is_some() || previous_witness.is_some() {
                return Err(error::corruption(
                    "nonterminal discovery exists without a prior run summary",
                ));
            }
            None
        }
        Some(bytes) => {
            let previous: RunSummaryIndex = json::decode(bytes, "run summary")?;
            if previous.run != summary.run {
                return Err(error::corruption(
                    "prior run-summary key disagrees with its document",
                ));
            }
            match (previous.state, previous_marker) {
                (IndexedRunState::Terminal, None) => None,
                (IndexedRunState::Terminal, Some(_)) => {
                    return Err(error::corruption(
                        "prior terminal run remains in nonterminal discovery",
                    ));
                }
                (_, Some(1)) => Some(nonterminal_membership_payload(run_membership_payload(
                    &previous.run,
                    previous.through_sequence,
                    bytes,
                ))),
                (_, Some(_)) => {
                    return Err(error::corruption(
                        "prior nonterminal discovery marker is invalid",
                    ));
                }
                (_, None) => {
                    return Err(error::corruption(
                        "prior nonterminal run is missing from discovery",
                    ));
                }
            }
        }
    };
    if previous_witness != previous_expected {
        return Err(error::corruption(
            "prior nonterminal discovery disagrees with its authenticated catalog",
        ));
    }

    {
        let mut nonterminal = write.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
        if summary.state == IndexedRunState::Terminal {
            let removed = nonterminal
                .remove(summary.run.as_str())
                .map_err(error::redb)?;
            if removed.as_ref().map(|marker| marker.value()) != previous_marker {
                return Err(error::corruption(
                    "nonterminal marker changed outside the command transaction",
                ));
            }
        } else {
            let replaced = nonterminal
                .insert(summary.run.as_str(), 1)
                .map_err(error::redb)?;
            if replaced.as_ref().map(|marker| marker.value()) != previous_marker {
                return Err(error::corruption(
                    "nonterminal marker changed outside the command transaction",
                ));
            }
        }
    }
    let replaced = if summary.state == IndexedRunState::Terminal {
        crate::trie::remove(write, family, path, summary.run.as_str().as_bytes())?
    } else {
        let run_payload =
            run_membership_payload(&summary.run, summary.through_sequence, summary_bytes);
        crate::trie::put(
            write,
            family,
            path,
            summary.run.as_str().as_bytes(),
            nonterminal_membership_payload(run_payload),
        )?
    };
    if replaced != previous_witness {
        return Err(error::corruption(
            "nonterminal witness changed outside the command transaction",
        ));
    }
    Ok(())
}

fn apply_runnable_mutations(
    write: &redb::WriteTransaction,
    mutations: &[RunnableIndexMutation],
) -> Result<(), PersistenceError> {
    let mut heads = BTreeMap::new();
    for mutation in mutations {
        let run = match mutation {
            RunnableIndexMutation::Upsert { entry } => &entry.run,
            RunnableIndexMutation::Remove { run, .. } => run,
        };
        if !heads.contains_key(run) {
            let loaded = load_runnable_head_in_transaction(write, run)?;
            let state = match loaded {
                Some((_entry, bytes, witness)) => RunnableHeadState {
                    previous_bytes: Some(bytes),
                    previous_witness: Some(witness),
                },
                None => RunnableHeadState {
                    previous_bytes: None,
                    previous_witness: None,
                },
            };
            heads.insert(run.clone(), state);
        }
    }
    let mut entries = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let mut ordered = write.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    for mutation in mutations {
        let (run, execution) = match mutation {
            RunnableIndexMutation::Upsert { entry } => (&entry.run, &entry.execution),
            RunnableIndexMutation::Remove { run, execution } => (run, execution),
        };
        let identity = codec::pair(run.as_str(), execution.as_str())?;
        let had_previous =
            if let Some(previous) = entries.get(identity.as_slice()).map_err(error::redb)? {
                let previous_bytes = previous.value().to_vec();
                let previous: RunnableIndexEntry = json::decode(&previous_bytes, "runnable index")?;
                remove_runnable_catalog_entry(write, &identity, &previous, &previous_bytes)?;
                let key = runnable_order_key(&previous)?;
                let removed = ordered.remove(key.as_slice()).map_err(error::redb)?;
                if removed.as_ref().map(|value| value.value()) != Some(previous_bytes.as_slice()) {
                    return Err(error::corruption(
                        "runnable identity row is missing or mismatched in its ordered index",
                    ));
                }
                true
            } else {
                ensure_catalog_absent(
                    write,
                    crate::trie::CatalogFamily::RunnableIdentity,
                    runnable_catalog_identity_path(run, &identity)?,
                    &identity,
                    "runnable",
                )?;
                false
            };
        match mutation {
            RunnableIndexMutation::Upsert { entry } => {
                let bytes = json::encode(entry, "runnable index")?;
                let order_key = runnable_order_key(entry)?;
                let replaced_identity = entries
                    .insert(identity.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
                if replaced_identity.is_some() != had_previous {
                    return Err(error::corruption(
                        "runnable identity upsert found unexpected physical state",
                    ));
                }
                if ordered
                    .insert(order_key.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?
                    .is_some()
                {
                    return Err(error::corruption(
                        "runnable ordered upsert would overwrite an unauthenticated row",
                    ));
                }
                insert_runnable_catalog_entry(write, &identity, entry, &bytes)?;
            }
            RunnableIndexMutation::Remove { .. } => {
                let _removed = entries.remove(identity.as_slice()).map_err(error::redb)?;
            }
        }
    }
    drop(ordered);
    drop(entries);
    for (run, state) in heads {
        let selected = best_runnable_for_run_in_transaction(write, &run, None)?;
        persist_runnable_head(
            write,
            &run,
            state.previous_bytes.as_deref(),
            state.previous_witness,
            selected.as_ref(),
        )?;
    }
    Ok(())
}

fn apply_timer_mutations(
    write: &redb::WriteTransaction,
    mutations: &[TimerIndexMutation],
) -> Result<(), PersistenceError> {
    let mut entries = write.open_table(TIMER_ENTRIES).map_err(error::redb)?;
    let mut ordered = write.open_table(TIMER_INDEX).map_err(error::redb)?;
    for mutation in mutations {
        let (run, timer) = match mutation {
            TimerIndexMutation::Upsert { entry } => (&entry.run, &entry.timer),
            TimerIndexMutation::Remove { run, timer } => (run, timer),
        };
        let identity = codec::pair(run.as_str(), timer.as_str())?;
        let had_previous =
            if let Some(previous) = entries.get(identity.as_slice()).map_err(error::redb)? {
                let previous_bytes = previous.value().to_vec();
                let previous: TimerIndexEntry = json::decode(&previous_bytes, "timer index")?;
                remove_timer_catalog_entry(write, &identity, &previous, &previous_bytes)?;
                let key = timer_order_key(&previous)?;
                let removed = ordered.remove(key.as_slice()).map_err(error::redb)?;
                if removed.as_ref().map(|value| value.value()) != Some(previous_bytes.as_slice()) {
                    return Err(error::corruption(
                        "timer identity row is missing or mismatched in its ordered index",
                    ));
                }
                true
            } else {
                ensure_catalog_absent(
                    write,
                    crate::trie::CatalogFamily::TimerIdentity,
                    crate::trie::hashed_path(crate::trie::CatalogFamily::TimerIdentity, &identity),
                    &identity,
                    "timer",
                )?;
                false
            };
        match mutation {
            TimerIndexMutation::Upsert { entry } => {
                let bytes = json::encode(entry, "timer index")?;
                let order_key = timer_order_key(entry)?;
                let replaced_identity = entries
                    .insert(identity.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
                if replaced_identity.is_some() != had_previous {
                    return Err(error::corruption(
                        "timer identity upsert found unexpected physical state",
                    ));
                }
                if ordered
                    .insert(order_key.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?
                    .is_some()
                {
                    return Err(error::corruption(
                        "timer ordered upsert would overwrite an unauthenticated row",
                    ));
                }
                insert_timer_catalog_entry(write, &identity, entry, &bytes)?;
            }
            TimerIndexMutation::Remove { .. } => {
                let _removed = entries.remove(identity.as_slice()).map_err(error::redb)?;
            }
        }
    }
    Ok(())
}

fn apply_lease_mutations(
    write: &redb::WriteTransaction,
    mutations: &[LeaseIndexMutation],
) -> Result<(), PersistenceError> {
    let mut entries = write.open_table(LEASE_ENTRIES).map_err(error::redb)?;
    let mut ordered = write.open_table(LEASE_INDEX).map_err(error::redb)?;
    for mutation in mutations {
        let (run, lease) = match mutation {
            LeaseIndexMutation::Upsert { entry } => (&entry.run, &entry.lease),
            LeaseIndexMutation::Remove { run, lease } => (run, lease),
        };
        let identity = codec::pair(run.as_str(), lease.as_str())?;
        let had_previous =
            if let Some(previous) = entries.get(identity.as_slice()).map_err(error::redb)? {
                let previous_bytes = previous.value().to_vec();
                let previous: LeaseIndexEntry = json::decode(&previous_bytes, "lease index")?;
                remove_lease_catalog_entry(write, &identity, &previous, &previous_bytes)?;
                let key = lease_order_key(&previous)?;
                let removed = ordered.remove(key.as_slice()).map_err(error::redb)?;
                if removed.as_ref().map(|value| value.value()) != Some(previous_bytes.as_slice()) {
                    return Err(error::corruption(
                        "lease identity row is missing or mismatched in its ordered index",
                    ));
                }
                true
            } else {
                ensure_catalog_absent(
                    write,
                    crate::trie::CatalogFamily::LeaseIdentity,
                    crate::trie::hashed_path(crate::trie::CatalogFamily::LeaseIdentity, &identity),
                    &identity,
                    "lease",
                )?;
                false
            };
        match mutation {
            LeaseIndexMutation::Upsert { entry } => {
                let bytes = json::encode(entry, "lease index")?;
                let order_key = lease_order_key(entry)?;
                let replaced_identity = entries
                    .insert(identity.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?;
                if replaced_identity.is_some() != had_previous {
                    return Err(error::corruption(
                        "lease identity upsert found unexpected physical state",
                    ));
                }
                if ordered
                    .insert(order_key.as_slice(), bytes.as_slice())
                    .map_err(error::redb)?
                    .is_some()
                {
                    return Err(error::corruption(
                        "lease ordered upsert would overwrite an unauthenticated row",
                    ));
                }
                insert_lease_catalog_entry(write, &identity, entry, &bytes)?;
            }
            LeaseIndexMutation::Remove { .. } => {
                let _removed = entries.remove(identity.as_slice()).map_err(error::redb)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn runnable_catalog_identity_path(
    _run: &RunId,
    identity: &[u8],
) -> Result<[u8; 32], PersistenceError> {
    Ok(crate::trie::hashed_path(
        crate::trie::CatalogFamily::RunnableIdentity,
        identity,
    ))
}

fn runnable_group(family: crate::trie::CatalogFamily, key: &[u8]) -> [u8; 16] {
    let run_hash = crate::trie::hashed_path(family, key);
    let mut group = [0_u8; 16];
    group.copy_from_slice(&run_hash[..16]);
    group
}

pub(crate) fn runnable_bucket_key(
    run: &RunId,
    eligible_at: TimestampMillis,
) -> Result<Vec<u8>, PersistenceError> {
    codec::components(&[run.as_str(), &eligible_at.get().to_string()])
}

pub(crate) fn runnable_bucket_path(
    run: &RunId,
    eligible_at: TimestampMillis,
    key: &[u8],
) -> Result<[u8; 32], PersistenceError> {
    let family = crate::trie::CatalogFamily::RunnableBucket;
    let mut prefix = [0_u8; 24];
    prefix[..16].copy_from_slice(&runnable_group(family, run.as_str().as_bytes()));
    prefix[16..].copy_from_slice(&eligible_at.get().to_be_bytes());
    crate::trie::ordered_path(family, &prefix, key)
}

pub(crate) fn runnable_bucket_entry_path(
    bucket_key: &[u8],
    identity: &[u8],
    priority: u16,
) -> Result<[u8; 32], PersistenceError> {
    let family = crate::trie::CatalogFamily::RunnableBucketEntry;
    let mut prefix = [0_u8; 18];
    prefix[..16].copy_from_slice(&runnable_group(family, bucket_key));
    prefix[16..].copy_from_slice(&(u16::MAX - priority).to_be_bytes());
    crate::trie::ordered_path(family, &prefix, identity)
}

fn first_path_in_group(group: [u8; 16]) -> Option<[u8; 32]> {
    let mut first = [0_u8; 32];
    first[..16].copy_from_slice(&group);
    predecessor_path(first)
}

fn runnable_head_path(run: &RunId) -> [u8; 32] {
    let family = crate::trie::CatalogFamily::RunnableRunHead;
    crate::trie::hashed_path(family, run.as_str().as_bytes())
}

fn load_runnable_head_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<Option<(RunnableIndexEntry, Vec<u8>, [u8; 32])>, PersistenceError> {
    let stored = {
        let table = write.open_table(RUNNABLE_RUN_HEADS).map_err(error::redb)?;
        table
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec())
    };
    let family = crate::trie::CatalogFamily::RunnableRunHead;
    let witness = crate::trie::verify_member_in_transaction(
        write,
        family,
        runnable_head_path(run),
        run.as_str().as_bytes(),
    )?;
    match (stored, witness) {
        (None, None) => Ok(None),
        (Some(bytes), Some(witness)) if witness == crate::trie::digest_payload(family, &bytes) => {
            let entry: RunnableIndexEntry = json::decode(&bytes, "runnable run head")?;
            if entry.run != *run {
                return Err(error::corruption(
                    "runnable run-head key disagrees with its document",
                ));
            }
            validate_runnable_entry_in_transaction(write, &entry)?;
            if first_runnable_for_run_in_transaction(write, run)?.as_ref() != Some(&entry) {
                return Err(error::corruption(
                    "runnable run head is not the canonical earliest eligible entry",
                ));
            }
            Ok(Some((entry, bytes, witness)))
        }
        _ => Err(error::corruption(
            "runnable run head disagrees with its authenticated catalog",
        )),
    }
}

fn persist_runnable_head(
    write: &redb::WriteTransaction,
    run: &RunId,
    previous_bytes: Option<&[u8]>,
    previous_witness: Option<[u8; 32]>,
    selected: Option<&RunnableIndexEntry>,
) -> Result<(), PersistenceError> {
    let family = crate::trie::CatalogFamily::RunnableRunHead;
    let new_bytes = selected
        .map(|entry| {
            if entry.run != *run {
                return Err(error::corruption(
                    "runnable run head belongs to another run",
                ));
            }
            validate_runnable_entry_in_transaction(write, entry)?;
            json::encode(entry, "runnable run head")
        })
        .transpose()?;
    {
        let mut heads = write.open_table(RUNNABLE_RUN_HEADS).map_err(error::redb)?;
        let replaced = if let Some(bytes) = &new_bytes {
            heads
                .insert(run.as_str(), bytes.as_slice())
                .map_err(error::redb)?
        } else {
            heads.remove(run.as_str()).map_err(error::redb)?
        };
        if replaced.as_ref().map(|bytes| bytes.value()) != previous_bytes {
            return Err(error::corruption(
                "runnable run head changed outside its command transaction",
            ));
        }
    }
    let replaced = if let Some(bytes) = &new_bytes {
        crate::trie::put(
            write,
            family,
            runnable_head_path(run),
            run.as_str().as_bytes(),
            crate::trie::digest_payload(family, bytes),
        )?
    } else {
        crate::trie::remove(
            write,
            family,
            runnable_head_path(run),
            run.as_str().as_bytes(),
        )?
    };
    if replaced != previous_witness {
        return Err(error::corruption(
            "runnable run-head witness changed outside its command transaction",
        ));
    }
    Ok(())
}

fn validate_runnable_entry_in_transaction(
    write: &redb::WriteTransaction,
    entry: &RunnableIndexEntry,
) -> Result<(), PersistenceError> {
    let identity = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
    let bytes = {
        let entries = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
        entries
            .get(identity.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("runnable run head names a missing identity row"))?
            .value()
            .to_vec()
    };
    let stored: RunnableIndexEntry = json::decode(&bytes, "runnable index")?;
    if &stored != entry {
        return Err(error::corruption(
            "runnable run head disagrees with its identity row",
        ));
    }
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    let identity_witness = crate::trie::verify_member_in_transaction(
        write,
        identity_family,
        runnable_catalog_identity_path(&entry.run, &identity)?,
        &identity,
    )?;
    if identity_witness != Some(crate::trie::digest_payload(identity_family, &bytes)) {
        return Err(error::corruption(
            "runnable identity row disagrees with its authenticated catalog",
        ));
    }
    let order_key = runnable_order_key(entry)?;
    let ordered = write.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let ordered_bytes = ordered
        .get(order_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable run head has no ordered row"))?;
    if ordered_bytes.value() != bytes.as_slice() {
        return Err(error::corruption(
            "runnable identity and ordered rows disagree",
        ));
    }
    let ordered_family = crate::trie::CatalogFamily::RunnableOrdered;
    let ordered_witness = crate::trie::verify_member_in_transaction(
        write,
        ordered_family,
        runnable_catalog_ordered_path(&identity, entry)?,
        &identity,
    )?;
    if ordered_witness != Some(crate::trie::digest_payload(ordered_family, &bytes)) {
        return Err(error::corruption(
            "runnable ordered row disagrees with its authenticated catalog",
        ));
    }
    Ok(())
}

fn best_runnable_for_run_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    _eligible_through: Option<TimestampMillis>,
) -> Result<Option<RunnableIndexEntry>, PersistenceError> {
    first_runnable_for_run_in_transaction(write, run)
}

fn first_runnable_for_run_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<Option<RunnableIndexEntry>, PersistenceError> {
    let bucket_family = crate::trie::CatalogFamily::RunnableBucket;
    let run_group = runnable_group(bucket_family, run.as_str().as_bytes());
    let buckets = crate::trie::page_in_transaction(
        write,
        bucket_family,
        None,
        first_path_in_group(run_group),
        1,
    )?;
    let Some(bucket) = buckets.leaves.first() else {
        return Ok(None);
    };
    if bucket.path[..16] != run_group {
        return Ok(None);
    }
    let components = codec::decode_components(&bucket.logical_key, 2)?;
    let bucket_run = RunId::new(components[0]).map_err(|cause| {
        error::corruption(format!(
            "runnable bucket has an invalid run identity: {cause}"
        ))
    })?;
    let eligible_at = components[1]
        .parse::<u64>()
        .map_err(|_| error::corruption("runnable bucket has an invalid eligibility timestamp"))?;
    let eligible_at = TimestampMillis::new(eligible_at);
    if bucket_run != *run
        || bucket.path != runnable_bucket_path(run, eligible_at, &bucket.logical_key)?
        || bucket.payload_digest != crate::trie::digest_payload(bucket_family, &bucket.logical_key)
    {
        return Err(error::corruption(
            "runnable eligible bucket disagrees with its authenticated key",
        ));
    }

    let entry_family = crate::trie::CatalogFamily::RunnableBucketEntry;
    let entry_group = runnable_group(entry_family, &bucket.logical_key);
    let entries = crate::trie::page_in_transaction(
        write,
        entry_family,
        None,
        first_path_in_group(entry_group),
        1,
    )?;
    let entry_leaf = entries
        .leaves
        .first()
        .ok_or_else(|| error::corruption("runnable eligible bucket is empty"))?;
    if entry_leaf.path[..16] != entry_group {
        return Err(error::corruption(
            "runnable eligible bucket has no authenticated entries",
        ));
    }
    let identities = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let ordered = write.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let identity_bytes = identities
        .get(entry_leaf.logical_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable bucket entry is dangling"))?
        .value()
        .to_vec();
    if entry_leaf.payload_digest != crate::trie::digest_payload(entry_family, &identity_bytes) {
        return Err(error::corruption(
            "runnable bucket entry disagrees with its physical document",
        ));
    }
    let entry: RunnableIndexEntry = json::decode(&identity_bytes, "runnable index")?;
    if entry.run != *run
        || entry.eligible_at != eligible_at
        || entry_leaf.path
            != runnable_bucket_entry_path(
                &bucket.logical_key,
                &entry_leaf.logical_key,
                entry.priority,
            )?
    {
        return Err(error::corruption(
            "runnable bucket entry disagrees with its checked document",
        ));
    }
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    let identity_witness = crate::trie::verify_member_in_transaction(
        write,
        identity_family,
        runnable_catalog_identity_path(run, &entry_leaf.logical_key)?,
        &entry_leaf.logical_key,
    )?;
    let entry = validate_runnable_leaf_in_transaction(
        write,
        &identities,
        &ordered,
        &crate::trie::TrieLeaf {
            path: runnable_catalog_identity_path(run, &entry_leaf.logical_key)?,
            logical_key: entry_leaf.logical_key.clone(),
            payload_digest: identity_witness.ok_or_else(|| {
                error::corruption("runnable bucket entry has no authenticated identity")
            })?,
        },
    )?;
    Ok(Some(entry))
}

fn first_runnable_for_run(
    read: &redb::ReadTransaction,
    run: &RunId,
) -> Result<Option<RunnableIndexEntry>, PersistenceError> {
    let bucket_family = crate::trie::CatalogFamily::RunnableBucket;
    let run_group = runnable_group(bucket_family, run.as_str().as_bytes());
    let buckets = crate::trie::page(read, bucket_family, None, first_path_in_group(run_group), 1)?;
    let Some(bucket) = buckets.leaves.first() else {
        return Ok(None);
    };
    if bucket.path[..16] != run_group {
        return Ok(None);
    }
    let components = codec::decode_components(&bucket.logical_key, 2)?;
    let bucket_run = RunId::new(components[0]).map_err(|cause| {
        error::corruption(format!(
            "runnable bucket has an invalid run identity: {cause}"
        ))
    })?;
    let eligible_at =
        TimestampMillis::new(components[1].parse::<u64>().map_err(|_| {
            error::corruption("runnable bucket has an invalid eligibility timestamp")
        })?);
    if bucket_run != *run
        || bucket.path != runnable_bucket_path(run, eligible_at, &bucket.logical_key)?
        || bucket.payload_digest != crate::trie::digest_payload(bucket_family, &bucket.logical_key)
    {
        return Err(error::corruption(
            "runnable eligible bucket disagrees with its authenticated key",
        ));
    }
    let entry_family = crate::trie::CatalogFamily::RunnableBucketEntry;
    let entry_group = runnable_group(entry_family, &bucket.logical_key);
    let entries_page = crate::trie::page(
        read,
        entry_family,
        None,
        first_path_in_group(entry_group),
        1,
    )?;
    let entry_leaf = entries_page
        .leaves
        .first()
        .ok_or_else(|| error::corruption("runnable eligible bucket is empty"))?;
    if entry_leaf.path[..16] != entry_group {
        return Err(error::corruption(
            "runnable eligible bucket has no authenticated entries",
        ));
    }
    let identities = read.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let ordered = read.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let identity_bytes = identities
        .get(entry_leaf.logical_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable bucket entry is dangling"))?
        .value()
        .to_vec();
    if entry_leaf.payload_digest != crate::trie::digest_payload(entry_family, &identity_bytes) {
        return Err(error::corruption(
            "runnable bucket entry disagrees with its physical document",
        ));
    }
    let entry: RunnableIndexEntry = json::decode(&identity_bytes, "runnable index")?;
    if entry.run != *run
        || entry.eligible_at != eligible_at
        || entry_leaf.path
            != runnable_bucket_entry_path(
                &bucket.logical_key,
                &entry_leaf.logical_key,
                entry.priority,
            )?
    {
        return Err(error::corruption(
            "runnable bucket entry disagrees with its checked document",
        ));
    }
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    let identity_witness = crate::trie::verify_member(
        read,
        identity_family,
        runnable_catalog_identity_path(run, &entry_leaf.logical_key)?,
        &entry_leaf.logical_key,
    )?;
    Ok(Some(validate_runnable_leaf(
        read,
        &identities,
        &ordered,
        &crate::trie::TrieLeaf {
            path: runnable_catalog_identity_path(run, &entry_leaf.logical_key)?,
            logical_key: entry_leaf.logical_key.clone(),
            payload_digest: identity_witness.ok_or_else(|| {
                error::corruption("runnable bucket entry has no authenticated identity")
            })?,
        },
    )?))
}

pub(crate) fn migrate_runnable_run_heads(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    let heads = write.open_table(RUNNABLE_RUN_HEADS).map_err(error::redb)?;
    if heads.len().map_err(error::redb)? != 0 {
        return Err(error::corruption(
            "legacy storage unexpectedly contains runnable run heads",
        ));
    }
    drop(heads);
    let entries = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let mut current_run: Option<RunId> = None;
    let mut best: Option<RunnableIndexEntry> = None;
    for item in entries.iter().map_err(error::redb)? {
        let (identity, bytes) = item.map_err(error::redb)?;
        let entry: RunnableIndexEntry = json::decode(bytes.value(), "runnable index")?;
        if current_run.as_ref().is_some_and(|run| run != &entry.run) {
            let run = current_run
                .as_ref()
                .ok_or_else(|| error::corruption("legacy runnable run tracker is empty"))?;
            let selected = best
                .as_ref()
                .ok_or_else(|| error::corruption("legacy runnable run has no selected head"))?;
            persist_migrated_runnable_head(write, run, selected)?;
            best = None;
        }
        current_run = Some(entry.run.clone());
        insert_runnable_bucket_entry(write, identity.value(), &entry, bytes.value())?;
        if best
            .as_ref()
            .is_none_or(|selected| runnable_precedes(&entry, selected))
        {
            best = Some(entry);
        }
    }
    if let Some(run) = &current_run {
        let selected = best
            .as_ref()
            .ok_or_else(|| error::corruption("legacy runnable run has no selected head"))?;
        persist_migrated_runnable_head(write, run, selected)?;
    }
    Ok(())
}

fn persist_migrated_runnable_head(
    write: &redb::WriteTransaction,
    run: &RunId,
    selected: &RunnableIndexEntry,
) -> Result<(), PersistenceError> {
    if selected.run != *run {
        return Err(error::corruption(
            "legacy runnable head belongs to another run",
        ));
    }
    let bytes = json::encode(selected, "runnable run head")?;
    if write
        .open_table(RUNNABLE_RUN_HEADS)
        .map_err(error::redb)?
        .insert(run.as_str(), bytes.as_slice())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(error::corruption(
            "legacy runnable head migration replaced an existing row",
        ));
    }
    let family = crate::trie::CatalogFamily::RunnableRunHead;
    if crate::trie::put(
        write,
        family,
        runnable_head_path(run),
        run.as_str().as_bytes(),
        crate::trie::digest_payload(family, &bytes),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "legacy runnable head migration replaced an authenticated leaf",
        ));
    }
    Ok(())
}

pub(crate) fn runnable_catalog_ordered_path(
    identity: &[u8],
    entry: &RunnableIndexEntry,
) -> Result<[u8; 32], PersistenceError> {
    crate::trie::ordered_path(
        crate::trie::CatalogFamily::RunnableOrdered,
        &entry.eligible_at.get().to_be_bytes(),
        identity,
    )
}

fn insert_runnable_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &RunnableIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    insert_discovery_catalog_entry(
        write,
        identity,
        bytes,
        crate::trie::CatalogFamily::RunnableIdentity,
        runnable_catalog_identity_path(&entry.run, identity)?,
        crate::trie::CatalogFamily::RunnableOrdered,
        runnable_catalog_ordered_path(identity, entry)?,
        "runnable",
    )?;
    insert_runnable_bucket_entry(write, identity, entry, bytes)
}

fn remove_runnable_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &RunnableIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    remove_runnable_bucket_entry(write, identity, entry, bytes)?;
    remove_discovery_catalog_entry(
        write,
        identity,
        bytes,
        crate::trie::CatalogFamily::RunnableIdentity,
        runnable_catalog_identity_path(&entry.run, identity)?,
        crate::trie::CatalogFamily::RunnableOrdered,
        runnable_catalog_ordered_path(identity, entry)?,
        "runnable",
    )
}

fn insert_runnable_bucket_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &RunnableIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let bucket_key = runnable_bucket_key(&entry.run, entry.eligible_at)?;
    let bucket_family = crate::trie::CatalogFamily::RunnableBucket;
    let bucket_path = runnable_bucket_path(&entry.run, entry.eligible_at, &bucket_key)?;
    let bucket_witness =
        crate::trie::verify_member_in_transaction(write, bucket_family, bucket_path, &bucket_key)?;
    let expected_bucket = crate::trie::digest_payload(bucket_family, &bucket_key);
    match bucket_witness {
        Some(witness) if witness == expected_bucket => {}
        Some(_) => {
            return Err(error::corruption(
                "runnable eligible bucket has an invalid authenticated payload",
            ));
        }
        None => {
            if crate::trie::put(
                write,
                bucket_family,
                bucket_path,
                &bucket_key,
                expected_bucket,
            )?
            .is_some()
            {
                return Err(error::corruption(
                    "runnable eligible bucket appeared during insertion",
                ));
            }
        }
    }
    let entry_family = crate::trie::CatalogFamily::RunnableBucketEntry;
    if crate::trie::put(
        write,
        entry_family,
        runnable_bucket_entry_path(&bucket_key, identity, entry.priority)?,
        identity,
        crate::trie::digest_payload(entry_family, bytes),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "runnable eligible bucket would overwrite an authenticated entry",
        ));
    }
    Ok(())
}

fn remove_runnable_bucket_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &RunnableIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let bucket_key = runnable_bucket_key(&entry.run, entry.eligible_at)?;
    let bucket_family = crate::trie::CatalogFamily::RunnableBucket;
    let bucket_path = runnable_bucket_path(&entry.run, entry.eligible_at, &bucket_key)?;
    let bucket_witness =
        crate::trie::verify_member_in_transaction(write, bucket_family, bucket_path, &bucket_key)?;
    if bucket_witness != Some(crate::trie::digest_payload(bucket_family, &bucket_key)) {
        return Err(error::corruption(
            "runnable entry is missing its authenticated eligible bucket",
        ));
    }
    let entry_family = crate::trie::CatalogFamily::RunnableBucketEntry;
    let removed = crate::trie::remove(
        write,
        entry_family,
        runnable_bucket_entry_path(&bucket_key, identity, entry.priority)?,
        identity,
    )?;
    if removed != Some(crate::trie::digest_payload(entry_family, bytes)) {
        return Err(error::corruption(
            "runnable entry disagrees with its authenticated eligible bucket",
        ));
    }
    let group = runnable_group(entry_family, &bucket_key);
    let page =
        crate::trie::page_in_transaction(write, entry_family, None, first_path_in_group(group), 1)?;
    let bucket_empty = page
        .leaves
        .first()
        .is_none_or(|leaf| leaf.path[..16] != group);
    if bucket_empty
        && crate::trie::remove(write, bucket_family, bucket_path, &bucket_key)?
            != Some(crate::trie::digest_payload(bucket_family, &bucket_key))
    {
        return Err(error::corruption(
            "empty runnable eligible bucket disappeared unexpectedly",
        ));
    }
    Ok(())
}

pub(crate) fn timer_catalog_ordered_path(
    identity: &[u8],
    entry: &TimerIndexEntry,
) -> Result<[u8; 32], PersistenceError> {
    crate::trie::ordered_path(
        crate::trie::CatalogFamily::TimerOrdered,
        &entry.fire_at.get().to_be_bytes(),
        identity,
    )
}

fn insert_timer_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &TimerIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let identity_family = crate::trie::CatalogFamily::TimerIdentity;
    insert_discovery_catalog_entry(
        write,
        identity,
        bytes,
        identity_family,
        crate::trie::hashed_path(identity_family, identity),
        crate::trie::CatalogFamily::TimerOrdered,
        timer_catalog_ordered_path(identity, entry)?,
        "timer",
    )
}

fn remove_timer_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &TimerIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let identity_family = crate::trie::CatalogFamily::TimerIdentity;
    remove_discovery_catalog_entry(
        write,
        identity,
        bytes,
        identity_family,
        crate::trie::hashed_path(identity_family, identity),
        crate::trie::CatalogFamily::TimerOrdered,
        timer_catalog_ordered_path(identity, entry)?,
        "timer",
    )
}

pub(crate) fn lease_catalog_ordered_path(
    identity: &[u8],
    entry: &LeaseIndexEntry,
) -> Result<[u8; 32], PersistenceError> {
    crate::trie::ordered_path(
        crate::trie::CatalogFamily::LeaseOrdered,
        &entry.expires_at.get().to_be_bytes(),
        identity,
    )
}

fn insert_lease_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &LeaseIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let identity_family = crate::trie::CatalogFamily::LeaseIdentity;
    insert_discovery_catalog_entry(
        write,
        identity,
        bytes,
        identity_family,
        crate::trie::hashed_path(identity_family, identity),
        crate::trie::CatalogFamily::LeaseOrdered,
        lease_catalog_ordered_path(identity, entry)?,
        "lease",
    )
}

fn remove_lease_catalog_entry(
    write: &redb::WriteTransaction,
    identity: &[u8],
    entry: &LeaseIndexEntry,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let identity_family = crate::trie::CatalogFamily::LeaseIdentity;
    remove_discovery_catalog_entry(
        write,
        identity,
        bytes,
        identity_family,
        crate::trie::hashed_path(identity_family, identity),
        crate::trie::CatalogFamily::LeaseOrdered,
        lease_catalog_ordered_path(identity, entry)?,
        "lease",
    )
}

#[allow(clippy::too_many_arguments)]
fn insert_discovery_catalog_entry(
    write: &redb::WriteTransaction,
    logical_key: &[u8],
    bytes: &[u8],
    identity_family: crate::trie::CatalogFamily,
    identity_path: [u8; 32],
    ordered_family: crate::trie::CatalogFamily,
    ordered_path: [u8; 32],
    label: &'static str,
) -> Result<(), PersistenceError> {
    for (family, path) in [
        (identity_family, identity_path),
        (ordered_family, ordered_path),
    ] {
        let digest = crate::trie::digest_payload(family, bytes);
        if crate::trie::put(write, family, path, logical_key, digest)?.is_some() {
            return Err(error::corruption(format!(
                "{label} authenticated catalog unexpectedly replaced a committed leaf"
            )));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn remove_discovery_catalog_entry(
    write: &redb::WriteTransaction,
    logical_key: &[u8],
    bytes: &[u8],
    identity_family: crate::trie::CatalogFamily,
    identity_path: [u8; 32],
    ordered_family: crate::trie::CatalogFamily,
    ordered_path: [u8; 32],
    label: &'static str,
) -> Result<(), PersistenceError> {
    for (family, path) in [
        (identity_family, identity_path),
        (ordered_family, ordered_path),
    ] {
        let expected = crate::trie::digest_payload(family, bytes);
        if crate::trie::remove(write, family, path, logical_key)? != Some(expected) {
            return Err(error::corruption(format!(
                "{label} row disagrees with its authenticated catalog"
            )));
        }
    }
    Ok(())
}

fn ensure_catalog_absent(
    write: &redb::WriteTransaction,
    family: crate::trie::CatalogFamily,
    path: [u8; 32],
    logical_key: &[u8],
    label: &'static str,
) -> Result<(), PersistenceError> {
    if crate::trie::verify_member_in_transaction(write, family, path, logical_key)?.is_some() {
        return Err(error::corruption(format!(
            "{label} authenticated catalog names a missing primary row"
        )));
    }
    Ok(())
}

pub(crate) fn runnable_order_key(entry: &RunnableIndexEntry) -> Result<Vec<u8>, PersistenceError> {
    codec::ordered_timestamp(
        entry.eligible_at.get(),
        &format!("{}\0{}", entry.run, entry.execution),
    )
}

pub(crate) fn timer_order_key(entry: &TimerIndexEntry) -> Result<Vec<u8>, PersistenceError> {
    codec::ordered_timestamp(
        entry.fire_at.get(),
        &format!("{}\0{}", entry.run, entry.timer),
    )
}

pub(crate) fn lease_order_key(entry: &LeaseIndexEntry) -> Result<Vec<u8>, PersistenceError> {
    codec::ordered_timestamp(
        entry.expires_at.get(),
        &format!("{}\0{}", entry.run, entry.lease),
    )
}

fn record_artifact_references(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    crate::artifact::validate_artifact_catalog(write)?;
    for reference in &request.required_artifacts {
        let digest = reference.digest().to_hex();
        let key = codec::components(&[
            &digest,
            reference.artifact().as_str(),
            request.receipt.run().as_str(),
            request.receipt.command().as_str(),
        ])?;
        crate::artifact::persist_artifact_reference_occurrence(write, &key, reference)?;
        crate::artifact::persist_run_artifact_ownership(write, request.receipt.run(), reference)?;
    }
    crate::artifact::validate_artifact_catalog(write)?;
    Ok(())
}

impl RunQueryStore for RedbStore {
    fn events(&self, query: &EventPageQuery) -> Result<EventPage, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events_table = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let observed_head = validated_run_head(&heads, &events_table, &query.run)?;
        validate_run_history_membership(&read, &query.run, observed_head)?;
        let mut next_sequence = query
            .cursor
            .as_ref()
            .map_or(RunSequence::FIRST, |cursor| cursor.next_sequence);
        if observed_head == RunSequence::ZERO {
            if query.cursor.is_some() {
                return Err(PersistenceError::InvalidCursor(
                    "event cursor names an absent run".to_owned(),
                ));
            }
            return Ok(EventPage {
                events: Vec::new(),
                next: None,
                observed_head,
            });
        }
        if next_sequence > observed_head {
            return Err(PersistenceError::InvalidCursor(format!(
                "event cursor sequence {next_sequence} is beyond observed head {observed_head}"
            )));
        }
        let checksum_table = read.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
        let mut events = Vec::with_capacity(query.limit.get() as usize);
        while next_sequence <= observed_head && events.len() < query.limit.get() as usize {
            let key = codec::run_sequence(query.run.as_str(), next_sequence)?;
            let bytes = events_table
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| {
                    error::corruption(format!(
                        "run {} is missing authoritative event sequence {next_sequence}",
                        query.run
                    ))
                })?;
            let event_bytes = bytes.value().to_vec();
            validate_event_catalog(&read, &query.run, next_sequence, &key, &event_bytes)?;
            let event = decode_stored_event(&event_bytes)?;
            if event.run_id() != &query.run || event.sequence() != next_sequence {
                return Err(error::corruption(
                    "stored event key does not match its envelope",
                ));
            }
            let checksum = checksum_table
                .get(event.event_id().as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("event checksum index entry is missing"))?;
            if checksum.value() != event.checksum().as_str() {
                return Err(error::corruption(
                    "event checksum index does not match its envelope",
                ));
            }
            events.push(event);
            if next_sequence == observed_head {
                break;
            }
            next_sequence = next_sequence.next()?;
        }
        let next = if events.len() == query.limit.get() as usize
            && events
                .last()
                .is_some_and(|event| event.sequence() < observed_head)
        {
            Some(EventCursor {
                run: query.run.clone(),
                next_sequence: events
                    .last()
                    .ok_or_else(|| error::corruption("event page lost its cursor"))?
                    .sequence()
                    .next()?,
            })
        } else {
            None
        };
        Ok(EventPage {
            events,
            next,
            observed_head,
        })
    }

    fn run_summary(&self, run: &RunId) -> Result<Option<RunSummaryIndex>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        let table = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, run)?;
        let membership = validate_run_history_membership(&read, run, head)?;
        let Some(bytes) = table.get(run.as_str()).map_err(error::redb)? else {
            return if head == RunSequence::ZERO && membership.is_none() {
                Ok(None)
            } else {
                Err(error::corruption(
                    "an existing run is missing its discoverability summary",
                ))
            };
        };
        let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
        if summary.run != *run {
            return Err(error::corruption(
                "run-summary key does not match its document",
            ));
        }
        validate_summary_head(&heads, &events, &summary)?;
        validate_nonterminal_membership(
            &read,
            &summary,
            membership.ok_or_else(|| {
                error::corruption("stored run summary has no authenticated membership")
            })?,
        )?;
        Ok(Some(summary))
    }

    fn run_summaries(
        &self,
        query: &RunSummaryPageQuery,
    ) -> Result<RunSummaryPage, PersistenceError> {
        const MIN_SUMMARY_SCAN_ROWS: usize = 8;
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        let after = if let Some(cursor) = &query.cursor {
            if !cursor.matches_query(&query.filter) {
                return Err(PersistenceError::InvalidCursor(
                    "run-summary cursor belongs to a different filter".to_owned(),
                ));
            }
            Some(validate_run_cursor_anchor(&read, cursor.after_run())?)
        } else {
            None
        };
        let page_limit = page_size_usize(query.limit)?;
        let mut runs = Vec::with_capacity(page_limit);
        let mut last_scanned = None;
        let scan_budget = page_limit.max(MIN_SUMMARY_SCAN_ROWS);
        let page = crate::trie::page(
            &read,
            crate::trie::CatalogFamily::RunMembership,
            None,
            after,
            scan_budget,
        )?;
        let mut processed = 0_usize;
        let mut stopped_for_results = false;
        for leaf in &page.leaves {
            let summary = validate_run_membership_leaf(&read, leaf)?;
            processed += 1;
            last_scanned = Some(summary.run.clone());
            if query
                .filter
                .state
                .is_some_and(|state| state != summary.state)
                || query
                    .filter
                    .workflow
                    .as_ref()
                    .is_some_and(|workflow| workflow != &summary.workflow)
            {
                continue;
            }
            runs.push(summary);
            if runs.len() == page_limit {
                stopped_for_results = true;
                break;
            }
        }
        let has_more =
            stopped_for_results && processed < page.leaves.len() || page.next_path.is_some();
        let next = if has_more {
            let after_run = last_scanned
                .ok_or_else(|| error::corruption("advancing summary page lost its scan cursor"))?;
            Some(milkdrift_persistence::RunSummaryCursor::for_query(
                after_run,
                query.filter.clone(),
            ))
        } else {
            None
        };
        Ok(RunSummaryPage { runs, next })
    }

    fn nonterminal_run_page(
        &self,
        cursor: Option<&milkdrift_persistence::RunSummaryCursor>,
        limit: PageSize,
    ) -> Result<RunSummaryPage, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        let after = if let Some(cursor) = cursor {
            if !cursor.is_nonterminal() {
                return Err(PersistenceError::InvalidCursor(
                    "run-summary cursor does not belong to nonterminal discovery".to_owned(),
                ));
            }
            Some(nonterminal_membership_path(cursor.after_run()))
        } else {
            None
        };
        let page_limit = page_size_usize(limit)?;
        let mut results = Vec::with_capacity(page_limit);
        let mut last_scanned = None;
        let page = crate::trie::page(
            &read,
            crate::trie::CatalogFamily::NonterminalRun,
            None,
            after,
            page_limit,
        )?;
        for leaf in &page.leaves {
            let summary = validate_nonterminal_membership_leaf(&read, leaf)?;
            let run = summary.run.clone();
            last_scanned = Some(run.clone());
            results.push(summary);
        }
        let has_more = page.next_path.is_some();
        let next = if has_more {
            let after_run = last_scanned.ok_or_else(|| {
                error::corruption("advancing nonterminal page lost its scan cursor")
            })?;
            Some(milkdrift_persistence::RunSummaryCursor::for_nonterminal(
                after_run,
            ))
        } else {
            None
        };
        Ok(RunSummaryPage {
            runs: results,
            next,
        })
    }

    fn runnable_page(
        &self,
        eligible_through: TimestampMillis,
        cursor: Option<&RunnableCursor>,
        limit: PageSize,
    ) -> Result<RunnablePage, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        // The continuation keeps the first page's eligibility boundary, but is
        // intentionally not root-bound: dispatch removes run heads normally. A
        // stable run-identity path remains an exclusive successor anchor even
        // after its physical/authenticated head has been removed.
        let scan_through = cursor.map_or(eligible_through, RunnableCursor::eligible_through);
        let page_limit = page_size_usize(limit)?;
        let mut results = Vec::with_capacity(page_limit);
        let after = cursor.map(|cursor| runnable_head_path(cursor.after_run()));
        let page = crate::trie::page(
            &read,
            crate::trie::CatalogFamily::RunnableRunHead,
            None,
            after,
            page_limit,
        )?;
        let mut last_scanned = None;
        for leaf in &page.leaves {
            let head = validate_runnable_head_leaf(&read, leaf)?;
            last_scanned = Some(head.clone());
            if head.eligible_at <= scan_through {
                results.push(head);
            }
        }
        let next = if page.next_path.is_some() {
            let scanned = last_scanned.ok_or_else(|| {
                error::corruption("advancing runnable page lost its authenticated run cursor")
            })?;
            Some(RunnableCursor::new(scanned.run, scan_through))
        } else {
            None
        };
        Ok(RunnablePage {
            entries: results,
            next,
        })
    }

    fn active_leases(&self, limit: PageSize) -> Result<ActiveLeaseSnapshot, PersistenceError> {
        let (entries, root) = read_ordered_index(
            self,
            LEASE_ENTRIES,
            LEASE_INDEX,
            TimestampMillis::new(u64::MAX),
            limit,
            "lease index",
            crate::trie::CatalogFamily::LeaseIdentity,
            crate::trie::CatalogFamily::LeaseOrdered,
            |entry: &LeaseIndexEntry| entry.expires_at,
            lease_order_key,
            |entry: &LeaseIndexEntry| codec::pair(entry.run.as_str(), entry.lease.as_str()),
            |identity, _entry| {
                Ok(crate::trie::hashed_path(
                    crate::trie::CatalogFamily::LeaseIdentity,
                    identity,
                ))
            },
            lease_catalog_ordered_path,
        )?;
        Ok(ActiveLeaseSnapshot {
            entries,
            witness: IntegrityDigest::new(format!("b3_{}", blake3::Hash::from_bytes(root)))?,
        })
    }

    fn due_timers(
        &self,
        due_through: TimestampMillis,
        limit: PageSize,
    ) -> Result<Vec<TimerIndexEntry>, PersistenceError> {
        read_ordered_index(
            self,
            TIMER_ENTRIES,
            TIMER_INDEX,
            due_through,
            limit,
            "timer index",
            crate::trie::CatalogFamily::TimerIdentity,
            crate::trie::CatalogFamily::TimerOrdered,
            |entry: &TimerIndexEntry| entry.fire_at,
            timer_order_key,
            |entry: &TimerIndexEntry| codec::pair(entry.run.as_str(), entry.timer.as_str()),
            |identity, _entry| {
                Ok(crate::trie::hashed_path(
                    crate::trie::CatalogFamily::TimerIdentity,
                    identity,
                ))
            },
            timer_catalog_ordered_path,
        )
        .map(|(entries, _root)| entries)
    }

    fn expired_leases(
        &self,
        expired_through: TimestampMillis,
        limit: PageSize,
    ) -> Result<Vec<LeaseIndexEntry>, PersistenceError> {
        read_ordered_index(
            self,
            LEASE_ENTRIES,
            LEASE_INDEX,
            expired_through,
            limit,
            "lease index",
            crate::trie::CatalogFamily::LeaseIdentity,
            crate::trie::CatalogFamily::LeaseOrdered,
            |entry: &LeaseIndexEntry| entry.expires_at,
            lease_order_key,
            |entry: &LeaseIndexEntry| codec::pair(entry.run.as_str(), entry.lease.as_str()),
            |identity, _entry| {
                Ok(crate::trie::hashed_path(
                    crate::trie::CatalogFamily::LeaseIdentity,
                    identity,
                ))
            },
            lease_catalog_ordered_path,
        )
        .map(|(entries, _root)| entries)
    }
}

pub(crate) fn validate_catalog_leaf(
    read: &redb::ReadTransaction,
    family: crate::trie::CatalogFamily,
    leaf: &crate::trie::TrieLeaf,
) -> Result<(), PersistenceError> {
    use crate::trie::CatalogFamily;

    match family {
        CatalogFamily::RunMembership => validate_run_membership_leaf(read, leaf).map(|_| ()),
        CatalogFamily::NonterminalRun => {
            validate_nonterminal_membership_leaf(read, leaf).map(|_| ())
        }
        CatalogFamily::Event => {
            let bytes = read
                .open_table(RUN_EVENTS)
                .map_err(error::redb)?
                .get(leaf.logical_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("event catalog leaf is dangling"))?
                .value()
                .to_vec();
            let event = decode_stored_event(&bytes)?;
            let expected_key = codec::run_sequence(event.run_id().as_str(), event.sequence())?;
            if expected_key != leaf.logical_key
                || leaf.path
                    != event_catalog_path(event.run_id(), event.sequence(), &expected_key)?
                || leaf.payload_digest != crate::trie::digest_payload(family, &bytes)
            {
                return Err(error::corruption(
                    "event catalog leaf disagrees with its checked envelope",
                ));
            }
            let checksum = read
                .open_table(EVENT_CHECKSUMS)
                .map_err(error::redb)?
                .get(event.event_id().as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("event checksum index is incomplete"))?;
            if checksum.value() != event.checksum().as_str() {
                return Err(error::corruption(
                    "event checksum index disagrees with its envelope",
                ));
            }
            let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
            let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
            let head = validated_run_head(&heads, &events, event.run_id())?;
            if event.sequence() > head
                || validate_run_history_membership(read, event.run_id(), head)?.is_none()
            {
                return Err(error::corruption(
                    "authenticated event is outside its owning run aggregate",
                ));
            }
            Ok(())
        }
        CatalogFamily::Command => {
            let bytes = read
                .open_table(COMMAND_RESULTS)
                .map_err(error::redb)?
                .get(leaf.logical_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("command catalog leaf is dangling"))?
                .value()
                .to_vec();
            if leaf.path != crate::trie::hashed_path(family, &leaf.logical_key)
                || leaf.payload_digest != crate::trie::digest_payload(family, &bytes)
            {
                return Err(error::corruption(
                    "command catalog leaf disagrees with its checked document",
                ));
            }
            let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
            let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
            let checksums = read.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
            validate_stored_command_record(
                &leaf.logical_key,
                &bytes,
                &heads,
                &events,
                &checksums,
            )?;
            let record = decode_command_record(&bytes)?;
            let head = validated_run_head(&heads, &events, &record.run)?;
            if validate_run_history_membership(read, &record.run, head)?.is_none() {
                return Err(error::corruption(
                    "authenticated command has no owning run aggregate",
                ));
            }
            Ok(())
        }
        CatalogFamily::RunnableIdentity | CatalogFamily::RunnableOrdered => {
            validate_discovery_catalog_leaf::<RunnableIndexEntry>(
                read,
                leaf,
                family,
                RUNNABLE_ENTRIES,
                RUNNABLE_INDEX,
                "runnable",
                |entry| codec::pair(entry.run.as_str(), entry.execution.as_str()),
                |identity, entry| runnable_catalog_identity_path(&entry.run, identity),
                runnable_catalog_ordered_path,
            )
        }
        CatalogFamily::TimerIdentity | CatalogFamily::TimerOrdered => {
            validate_discovery_catalog_leaf::<TimerIndexEntry>(
                read,
                leaf,
                family,
                TIMER_ENTRIES,
                TIMER_INDEX,
                "timer",
                |entry| codec::pair(entry.run.as_str(), entry.timer.as_str()),
                |identity, _entry| Ok(crate::trie::hashed_path(CatalogFamily::TimerIdentity, identity)),
                timer_catalog_ordered_path,
            )
        }
        CatalogFamily::LeaseIdentity | CatalogFamily::LeaseOrdered => {
            validate_discovery_catalog_leaf::<LeaseIndexEntry>(
                read,
                leaf,
                family,
                LEASE_ENTRIES,
                LEASE_INDEX,
                "lease",
                |entry| codec::pair(entry.run.as_str(), entry.lease.as_str()),
                |identity, _entry| Ok(crate::trie::hashed_path(CatalogFamily::LeaseIdentity, identity)),
                lease_catalog_ordered_path,
            )
        }
        CatalogFamily::WorkspaceDomain => {
            let run_text = std::str::from_utf8(&leaf.logical_key)
                .map_err(|_| error::corruption("workspace domain identity is not UTF-8"))?;
            let run = RunId::new(run_text).map_err(|cause| {
                error::corruption(format!("invalid workspace domain identity: {cause}"))
            })?;
            let _usage = validated_workspace_domain(read, &run)?
                .ok_or_else(|| error::corruption("workspace domain catalog is dangling"))?;
            let witness = crate::trie::verify_member(
                read,
                family,
                workspace_domain_path(&run),
                run.as_str().as_bytes(),
            )?
            .ok_or_else(|| error::corruption("workspace domain catalog is dangling"))?;
            if leaf.path != workspace_domain_path(&run) || leaf.payload_digest != witness {
                return Err(error::corruption(
                    "workspace domain leaf disagrees with its checked documents",
                ));
            }
            Ok(())
        }
        CatalogFamily::WorkspaceScope => {
            let bytes = read
                .open_table(SCOPES)
                .map_err(error::redb)?
                .get(leaf.logical_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("workspace scope catalog is dangling"))?
                .value()
                .to_vec();
            let scope: WorkspaceScope = json::decode(&bytes, "workspace scope")?;
            let expected_key = codec::pair(
                scope.reference().run().as_str(),
                scope.reference().scope().as_str(),
            )?;
            if expected_key != leaf.logical_key
                || leaf.path != workspace_scope_catalog_path(scope.reference(), &expected_key)?
                || leaf.payload_digest != crate::trie::digest_payload(family, &bytes)
            {
                return Err(error::corruption(
                    "workspace scope leaf disagrees with its checked document",
                ));
            }
            let scopes = read.open_table(SCOPES).map_err(error::redb)?;
            let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
            validate_scope_catalog_lineage(read, &scopes, &roots, scope.reference())
        }
        CatalogFamily::WorkspaceValue => {
            let bytes = read
                .open_table(VALUES)
                .map_err(error::redb)?
                .get(leaf.logical_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("workspace value catalog is dangling"))?
                .value()
                .to_vec();
            let entry: WorkspaceValueEntry = json::decode(&bytes, "workspace value")?;
            let expected_key = workspace_value_key(entry.reference())?;
            if expected_key != leaf.logical_key
                || leaf.path != crate::trie::hashed_path(family, &expected_key)
                || leaf.payload_digest != crate::trie::digest_payload(family, &bytes)
            {
                return Err(error::corruption(
                    "workspace value leaf disagrees with its checked document",
                ));
            }
            let values = read.open_table(VALUES).map_err(error::redb)?;
            let scopes = read.open_table(SCOPES).map_err(error::redb)?;
            let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
            validate_workspace_value_catalog_provenance(
                read, &values, &scopes, &roots, &entry,
            )
        }
        CatalogFamily::WorkspaceValueHead => {
            let bytes = read
                .open_table(WORKSPACE_VALUE_HEADS)
                .map_err(error::redb)?
                .get(leaf.logical_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("workspace value-head catalog is dangling"))?
                .value()
                .to_vec();
            let reference: WorkspaceValueReference =
                json::decode(&bytes, "workspace value head")?;
            let expected_key = codec::value_prefix(
                reference.scope().run().as_str(),
                reference.scope().scope().as_str(),
                reference.key().as_str(),
            )?;
            if expected_key != leaf.logical_key
                || leaf.path != crate::trie::hashed_path(family, &expected_key)
                || leaf.payload_digest != crate::trie::digest_payload(family, &bytes)
            {
                return Err(error::corruption(
                    "workspace value-head leaf disagrees with its checked pointer",
                ));
            }
            let value_key = workspace_value_key(&reference)?;
            let value = read
                .open_table(VALUES)
                .map_err(error::redb)?
                .get(value_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("workspace value head is dangling"))?
                .value()
                .to_vec();
            let entry: WorkspaceValueEntry = json::decode(&value, "workspace value")?;
            if entry.reference() != &reference {
                return Err(error::corruption(
                    "workspace value-head pointer disagrees with its value",
                ));
            }
            let values = read.open_table(VALUES).map_err(error::redb)?;
            let scopes = read.open_table(SCOPES).map_err(error::redb)?;
            let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
            validate_workspace_value_catalog_provenance(
                read, &values, &scopes, &roots, &entry,
            )
        }
        CatalogFamily::RunnableRunHead => validate_runnable_head_leaf(read, leaf).map(|_| ()),
        CatalogFamily::RunnableBucket => {
            let components = codec::decode_components(&leaf.logical_key, 2)?;
            let run = RunId::new(components[0]).map_err(|cause| {
                error::corruption(format!("invalid runnable bucket run: {cause}"))
            })?;
            let eligible_at = TimestampMillis::new(components[1].parse::<u64>().map_err(|_| {
                error::corruption("runnable bucket eligibility is not an integer")
            })?);
            if leaf.path != runnable_bucket_path(&run, eligible_at, &leaf.logical_key)?
                || leaf.payload_digest != crate::trie::digest_payload(family, &leaf.logical_key)
            {
                return Err(error::corruption(
                    "runnable bucket leaf disagrees with its logical key",
                ));
            }
            let group = runnable_group(CatalogFamily::RunnableBucketEntry, &leaf.logical_key);
            let page = crate::trie::page(
                read,
                CatalogFamily::RunnableBucketEntry,
                None,
                first_path_in_group(group),
                1,
            )?;
            if page.leaves.first().is_none_or(|entry| entry.path[..16] != group) {
                return Err(error::corruption("authenticated runnable bucket is empty"));
            }
            Ok(())
        }
        CatalogFamily::RunnableBucketEntry => {
            let bytes = read
                .open_table(RUNNABLE_ENTRIES)
                .map_err(error::redb)?
                .get(leaf.logical_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("runnable bucket entry is dangling"))?
                .value()
                .to_vec();
            let entry: RunnableIndexEntry = json::decode(&bytes, "runnable index")?;
            let bucket_key = runnable_bucket_key(&entry.run, entry.eligible_at)?;
            if leaf.path
                != runnable_bucket_entry_path(&bucket_key, &leaf.logical_key, entry.priority)?
                || leaf.payload_digest != crate::trie::digest_payload(family, &bytes)
            {
                return Err(error::corruption(
                    "runnable bucket entry disagrees with its checked document",
                ));
            }
            let bucket_family = CatalogFamily::RunnableBucket;
            let bucket = crate::trie::verify_member(
                read,
                bucket_family,
                runnable_bucket_path(&entry.run, entry.eligible_at, &bucket_key)?,
                &bucket_key,
            )?;
            if bucket != Some(crate::trie::digest_payload(bucket_family, &bucket_key)) {
                return Err(error::corruption(
                    "runnable bucket entry has no authenticated bucket",
                ));
            }
            Ok(())
        }
        _ => Err(error::corruption(
            "journal catalog validator received another family's leaf",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_discovery_catalog_leaf<T>(
    read: &redb::ReadTransaction,
    leaf: &crate::trie::TrieLeaf,
    family: crate::trie::CatalogFamily,
    identities_definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    ordered_definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    label: &'static str,
    identity_key: impl Fn(&T) -> Result<Vec<u8>, PersistenceError>,
    identity_path: impl Fn(&[u8], &T) -> Result<[u8; 32], PersistenceError>,
    ordered_path: impl Fn(&[u8], &T) -> Result<[u8; 32], PersistenceError>,
) -> Result<(), PersistenceError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let identities = read.open_table(identities_definition).map_err(error::redb)?;
    let ordered = read.open_table(ordered_definition).map_err(error::redb)?;
    let bytes = identities
        .get(leaf.logical_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption(format!("{label} catalog leaf is dangling")))?
        .value()
        .to_vec();
    let entry: T = json::decode(&bytes, label)?;
    let expected_identity = identity_key(&entry)?;
    let (identity_family, ordered_family) = match family {
        crate::trie::CatalogFamily::RunnableIdentity
        | crate::trie::CatalogFamily::RunnableOrdered => (
            crate::trie::CatalogFamily::RunnableIdentity,
            crate::trie::CatalogFamily::RunnableOrdered,
        ),
        crate::trie::CatalogFamily::TimerIdentity | crate::trie::CatalogFamily::TimerOrdered => (
            crate::trie::CatalogFamily::TimerIdentity,
            crate::trie::CatalogFamily::TimerOrdered,
        ),
        crate::trie::CatalogFamily::LeaseIdentity | crate::trie::CatalogFamily::LeaseOrdered => (
            crate::trie::CatalogFamily::LeaseIdentity,
            crate::trie::CatalogFamily::LeaseOrdered,
        ),
        _ => {
            return Err(error::corruption(
                "discovery catalog validator received another family's leaf",
            ));
        }
    };
    let expected_path = if family == identity_family {
        identity_path(&expected_identity, &entry)?
    } else {
        ordered_path(&expected_identity, &entry)?
    };
    let ordered_key = match family {
        crate::trie::CatalogFamily::RunnableIdentity
        | crate::trie::CatalogFamily::RunnableOrdered => {
            let runnable: RunnableIndexEntry = json::decode(&bytes, "runnable index")?;
            runnable_order_key(&runnable)?
        }
        crate::trie::CatalogFamily::TimerIdentity | crate::trie::CatalogFamily::TimerOrdered => {
            let timer: TimerIndexEntry = json::decode(&bytes, "timer index")?;
            timer_order_key(&timer)?
        }
        crate::trie::CatalogFamily::LeaseIdentity | crate::trie::CatalogFamily::LeaseOrdered => {
            let lease: LeaseIndexEntry = json::decode(&bytes, "lease index")?;
            lease_order_key(&lease)?
        }
        _ => {
            return Err(error::corruption(
                "discovery catalog validator received another family's leaf",
            ));
        }
    };
    let ordered_bytes = ordered
        .get(ordered_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption(format!("{label} ordered index is incomplete")))?;
    if expected_identity != leaf.logical_key
        || expected_path != leaf.path
        || ordered_bytes.value() != bytes.as_slice()
        || leaf.payload_digest != crate::trie::digest_payload(family, &bytes)
    {
        return Err(error::corruption(format!(
            "{label} catalog leaf disagrees with its physical indexes"
        )));
    }
    let peer_family = if family == identity_family {
        ordered_family
    } else {
        identity_family
    };
    let peer_path = if peer_family == identity_family {
        identity_path(&expected_identity, &entry)?
    } else {
        ordered_path(&expected_identity, &entry)?
    };
    if crate::trie::verify_member(read, peer_family, peer_path, &expected_identity)?
        != Some(crate::trie::digest_payload(peer_family, &bytes))
    {
        return Err(error::corruption(format!(
            "{label} catalog is missing its paired authenticated leaf"
        )));
    }
    Ok(())
}

fn validate_runnable_leaf_in_transaction<I, O>(
    write: &redb::WriteTransaction,
    identities: &I,
    ordered: &O,
    leaf: &crate::trie::TrieLeaf,
) -> Result<RunnableIndexEntry, PersistenceError>
where
    I: ReadableTable<&'static [u8], &'static [u8]>,
    O: ReadableTable<&'static [u8], &'static [u8]>,
{
    let bytes = identities
        .get(leaf.logical_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable authenticated catalog is dangling"))?;
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    if leaf.payload_digest != crate::trie::digest_payload(identity_family, bytes.value()) {
        return Err(error::corruption(
            "runnable identity row disagrees with its authenticated catalog",
        ));
    }
    let entry: RunnableIndexEntry = json::decode(bytes.value(), "runnable index")?;
    let expected_identity = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
    if leaf.logical_key != expected_identity
        || leaf.path != runnable_catalog_identity_path(&entry.run, &expected_identity)?
    {
        return Err(error::corruption(
            "runnable identity leaf disagrees with its checked document",
        ));
    }
    let ordered_key = runnable_order_key(&entry)?;
    let ordered_bytes = ordered
        .get(ordered_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable ordered index is incomplete"))?;
    if ordered_bytes.value() != bytes.value() {
        return Err(error::corruption(
            "runnable identity and ordered rows disagree",
        ));
    }
    let ordered_family = crate::trie::CatalogFamily::RunnableOrdered;
    let ordered_witness = crate::trie::verify_member_in_transaction(
        write,
        ordered_family,
        runnable_catalog_ordered_path(&expected_identity, &entry)?,
        &expected_identity,
    )?;
    if ordered_witness != Some(crate::trie::digest_payload(ordered_family, bytes.value())) {
        return Err(error::corruption(
            "runnable ordered row disagrees with its authenticated catalog",
        ));
    }
    Ok(entry)
}

fn validate_runnable_leaf<I, O>(
    read: &redb::ReadTransaction,
    identities: &I,
    ordered: &O,
    leaf: &crate::trie::TrieLeaf,
) -> Result<RunnableIndexEntry, PersistenceError>
where
    I: ReadableTable<&'static [u8], &'static [u8]>,
    O: ReadableTable<&'static [u8], &'static [u8]>,
{
    let bytes = identities
        .get(leaf.logical_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable authenticated catalog is dangling"))?;
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    if leaf.payload_digest != crate::trie::digest_payload(identity_family, bytes.value()) {
        return Err(error::corruption(
            "runnable identity row disagrees with its authenticated catalog",
        ));
    }
    let entry: RunnableIndexEntry = json::decode(bytes.value(), "runnable index")?;
    let expected_identity = codec::pair(entry.run.as_str(), entry.execution.as_str())?;
    if leaf.logical_key != expected_identity
        || leaf.path != runnable_catalog_identity_path(&entry.run, &expected_identity)?
    {
        return Err(error::corruption(
            "runnable identity leaf disagrees with its checked document",
        ));
    }
    let ordered_key = runnable_order_key(&entry)?;
    let ordered_bytes = ordered
        .get(ordered_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable ordered index is incomplete"))?;
    if ordered_bytes.value() != bytes.value() {
        return Err(error::corruption(
            "runnable identity and ordered rows disagree",
        ));
    }
    let ordered_family = crate::trie::CatalogFamily::RunnableOrdered;
    let ordered_witness = crate::trie::verify_member(
        read,
        ordered_family,
        runnable_catalog_ordered_path(&expected_identity, &entry)?,
        &expected_identity,
    )?;
    if ordered_witness != Some(crate::trie::digest_payload(ordered_family, bytes.value())) {
        return Err(error::corruption(
            "runnable ordered row disagrees with its authenticated catalog",
        ));
    }
    Ok(entry)
}

pub(crate) fn validate_runnable_head_leaf(
    read: &redb::ReadTransaction,
    leaf: &crate::trie::TrieLeaf,
) -> Result<RunnableIndexEntry, PersistenceError> {
    let run_text = std::str::from_utf8(&leaf.logical_key)
        .map_err(|_| error::corruption("runnable run-head identity is not UTF-8"))?;
    let run = RunId::new(run_text)
        .map_err(|cause| error::corruption(format!("invalid runnable run identity: {cause}")))?;
    if leaf.path != runnable_head_path(&run) {
        return Err(error::corruption(
            "runnable run-head path disagrees with its run identity",
        ));
    }
    let bytes = read
        .open_table(RUNNABLE_RUN_HEADS)
        .map_err(error::redb)?
        .get(run.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("runnable run-head catalog is dangling"))?
        .value()
        .to_vec();
    let family = crate::trie::CatalogFamily::RunnableRunHead;
    if leaf.payload_digest != crate::trie::digest_payload(family, &bytes) {
        return Err(error::corruption(
            "runnable run-head row disagrees with its authenticated catalog",
        ));
    }
    let entry: RunnableIndexEntry = json::decode(&bytes, "runnable run head")?;
    if entry.run != run {
        return Err(error::corruption(
            "runnable run-head key disagrees with its document",
        ));
    }
    let identity = codec::pair(run.as_str(), entry.execution.as_str())?;
    let identity_family = crate::trie::CatalogFamily::RunnableIdentity;
    let identity_witness = crate::trie::verify_member(
        read,
        identity_family,
        runnable_catalog_identity_path(&run, &identity)?,
        &identity,
    )?
    .ok_or_else(|| error::corruption("runnable run head names an unauthenticated entry"))?;
    let identities = read.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    let ordered = read.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    let active = validate_runnable_leaf(
        read,
        &identities,
        &ordered,
        &crate::trie::TrieLeaf {
            path: runnable_catalog_identity_path(&run, &identity)?,
            logical_key: identity,
            payload_digest: identity_witness,
        },
    )?;
    if active != entry {
        return Err(error::corruption(
            "runnable run head disagrees with its active entry",
        ));
    }
    drop(ordered);
    drop(identities);
    if first_runnable_for_run(read, &run)?.as_ref() != Some(&entry) {
        return Err(error::corruption(
            "runnable run head is not the canonical earliest eligible entry",
        ));
    }
    Ok(entry)
}

fn runnable_precedes(candidate: &RunnableIndexEntry, selected: &RunnableIndexEntry) -> bool {
    candidate.eligible_at < selected.eligible_at
        || (candidate.eligible_at == selected.eligible_at
            && (candidate.priority > selected.priority
                || (candidate.priority == selected.priority
                    && candidate.execution < selected.execution)))
}

fn page_size_usize(limit: PageSize) -> Result<usize, PersistenceError> {
    usize::try_from(limit.get()).map_err(|_| PersistenceError::Bounds {
        location: "page_size",
        reason: "page size cannot be represented on this platform".to_owned(),
    })
}

pub(crate) fn validated_run_head<H, E>(
    heads: &H,
    events: &E,
    run: &RunId,
) -> Result<RunSequence, PersistenceError>
where
    H: redb::ReadableTable<&'static str, u64>,
    E: redb::ReadableTable<&'static [u8], &'static [u8]>,
{
    let head = heads
        .get(run.as_str())
        .map_err(error::redb)?
        .map_or(RunSequence::ZERO, |value| RunSequence::new(value.value()));
    let prefix = codec::component(run.as_str())?;
    let end = codec::prefix_end(prefix.clone())
        .ok_or_else(|| error::corruption("run-event prefix has no range end"))?;

    if head == RunSequence::ZERO {
        if events
            .range::<&[u8]>(prefix.as_slice()..end.as_slice())
            .map_err(error::redb)?
            .next()
            .transpose()
            .map_err(error::redb)?
            .is_some()
        {
            return Err(error::corruption(format!(
                "run {run} has events but no authoritative journal head"
            )));
        }
        return Ok(head);
    }

    let head_key = codec::run_sequence(run.as_str(), head)?;
    if events
        .get(head_key.as_slice())
        .map_err(error::redb)?
        .is_none()
    {
        return Err(error::corruption(format!(
            "run {run} authoritative head {head} has no event"
        )));
    }
    if events
        .range::<&[u8]>((
            Bound::Excluded(head_key.as_slice()),
            Bound::Excluded(end.as_slice()),
        ))
        .map_err(error::redb)?
        .next()
        .transpose()
        .map_err(error::redb)?
        .is_some()
    {
        return Err(error::corruption(format!(
            "run {run} has events beyond authoritative head {head}"
        )));
    }
    Ok(head)
}

pub(crate) fn validated_run_head_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<RunSequence, PersistenceError> {
    let heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
    let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    validated_run_head(&heads, &events, run)
}

fn validate_summary_head<H, E>(
    heads: &H,
    events: &E,
    summary: &RunSummaryIndex,
) -> Result<(), PersistenceError>
where
    H: redb::ReadableTable<&'static str, u64>,
    E: redb::ReadableTable<&'static [u8], &'static [u8]>,
{
    let head = validated_run_head(heads, events, &summary.run)?;
    if head != summary.through_sequence {
        return Err(error::corruption(
            "run summary sequence does not match the authoritative journal head",
        ));
    }
    Ok(())
}

pub(crate) fn decode_stored_event(
    bytes: &[u8],
) -> Result<milkdrift_persistence::RunEventEnvelope, PersistenceError> {
    milkdrift_persistence::RunEventEnvelope::from_json(bytes).map_err(|cause| match cause {
        PersistenceError::UnsupportedVersion { .. } | PersistenceError::Corruption(_) => cause,
        other => {
            PersistenceError::Corruption(format!("stored run event failed verification: {other}"))
        }
    })
}

#[allow(clippy::too_many_arguments)] // Closed table/key functions keep one verifier shared.
fn read_ordered_index<T: for<'de> Deserialize<'de> + Serialize>(
    store: &RedbStore,
    identity_definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    definition: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    through: TimestampMillis,
    limit: PageSize,
    family: &'static str,
    identity_family: crate::trie::CatalogFamily,
    ordered_family: crate::trie::CatalogFamily,
    timestamp: impl Fn(&T) -> TimestampMillis,
    order_key: impl Fn(&T) -> Result<Vec<u8>, PersistenceError>,
    identity_key: impl Fn(&T) -> Result<Vec<u8>, PersistenceError>,
    identity_path: impl Fn(&[u8], &T) -> Result<[u8; 32], PersistenceError>,
    ordered_path: impl Fn(&[u8], &T) -> Result<[u8; 32], PersistenceError>,
) -> Result<(Vec<T>, [u8; 32]), PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    let identities = read.open_table(identity_definition).map_err(error::redb)?;
    let table = read.open_table(definition).map_err(error::redb)?;
    let identity_root = crate::trie::family_root(&read, identity_family)?;
    let catalog = crate::trie::page(&read, ordered_family, None, None, page_size_usize(limit)?)?;
    if catalog.leaves.is_empty()
        && (identities.len().map_err(error::redb)? != 0 || table.len().map_err(error::redb)? != 0)
    {
        return Err(error::corruption(format!(
            "{family} derived rows exist without an authenticated catalog"
        )));
    }
    let mut results = Vec::with_capacity(limit.get() as usize);
    for leaf in catalog.leaves {
        let value = identities
            .get(leaf.logical_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| {
                error::corruption(format!(
                    "{family} authenticated catalog names a missing identity row"
                ))
            })?;
        let value_bytes = value.value().to_vec();
        let entry: T = json::decode(&value_bytes, family)?;
        let expected_identity = identity_key(&entry)?;
        if expected_identity != leaf.logical_key {
            return Err(error::corruption(format!(
                "{family} identity key does not match its document"
            )));
        }
        let expected_order_key = order_key(&entry)?;
        if ordered_path(&expected_identity, &entry)? != leaf.path {
            return Err(error::corruption(format!(
                "{family} authenticated order path does not match its document"
            )));
        }
        if leaf.payload_digest != crate::trie::digest_payload(ordered_family, &value_bytes) {
            return Err(error::corruption(format!(
                "{family} ordered catalog payload disagrees with its identity row"
            )));
        }
        match table
            .get(expected_order_key.as_slice())
            .map_err(error::redb)?
        {
            Some(ordered_value) if ordered_value.value() == value_bytes.as_slice() => {}
            _ => {
                return Err(error::corruption(format!(
                    "{family} authenticated catalog is missing its ordered row"
                )));
            }
        }
        let authenticated_identity = crate::trie::verify_member(
            &read,
            identity_family,
            identity_path(&expected_identity, &entry)?,
            &expected_identity,
        )?;
        if authenticated_identity
            != Some(crate::trie::digest_payload(identity_family, &value_bytes))
        {
            return Err(error::corruption(format!(
                "{family} identity row disagrees with its authenticated catalog"
            )));
        }
        if timestamp(&entry) > through {
            break;
        }
        results.push(entry);
        if results.len() == limit.get() as usize {
            break;
        }
    }
    Ok((results, identity_root))
}

pub(crate) fn validated_workspace_domain(
    read: &redb::ReadTransaction,
    run: &RunId,
) -> Result<Option<WorkspaceUsage>, PersistenceError> {
    validate_workspace_value_accounting(read)?;
    let budget: Option<milkdrift_workspace::WorkspaceBudget> = {
        let budgets = read.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
        budgets
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace budget"))
            .transpose()?
    };
    let usage = {
        let usages = read.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
        usages
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace usage"))
            .transpose()?
    };
    let family = crate::trie::CatalogFamily::WorkspaceDomain;
    let witness = crate::trie::verify_member(
        read,
        family,
        workspace_domain_path(run),
        run.as_str().as_bytes(),
    )?;
    match (budget, usage, witness) {
        (None, None, None) => Ok(None),
        (Some(budget), Some(usage), Some(witness)) => {
            budget.validate_usage(&usage).map_err(|cause| {
                error::corruption(format!("workspace usage exceeds its budget: {cause}"))
            })?;
            if witness != workspace_domain_payload(&budget, usage)? {
                return Err(error::corruption(
                    "workspace domain disagrees with its authenticated catalog",
                ));
            }
            Ok(Some(usage))
        }
        _ => Err(error::corruption(
            "workspace budget, usage, and authenticated domain are incomplete",
        )),
    }
}

impl WorkspaceStore for RedbStore {
    fn workspace_usage(&self, run: &RunId) -> Result<WorkspaceUsage, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let usage = validated_workspace_domain(&read, run)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, run)?;
        let membership = validate_run_history_membership(&read, run, head)?;
        match usage {
            Some(usage) if head == RunSequence::ZERO && membership.is_none() => Ok(usage),
            Some(usage) if membership.is_some() => Ok(usage),
            Some(_) => Err(error::corruption(
                "workspace usage belongs to an unauthenticated run aggregate",
            )),
            None if head == RunSequence::ZERO && membership.is_none() => Ok(WorkspaceUsage::EMPTY),
            None => Err(error::corruption(
                "an existing run is missing its durable workspace usage",
            )),
        }
    }

    fn scope(
        &self,
        run: &RunId,
        scope: &ScopeId,
    ) -> Result<Option<WorkspaceScope>, PersistenceError> {
        let key = codec::pair(run.as_str(), scope.as_str())?;
        let reference = ScopeReference::new(run.clone(), scope.clone());
        let read = self.database().begin_read().map_err(error::redb)?;
        validate_workspace_value_accounting(&read)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, run)?;
        let _membership = validate_run_history_membership(&read, run, head)?;
        let table = read.open_table(SCOPES).map_err(error::redb)?;
        let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
        let stored = table.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = stored else {
            let family = crate::trie::CatalogFamily::WorkspaceScope;
            if crate::trie::verify_member(
                &read,
                family,
                workspace_scope_catalog_path(&reference, &key)?,
                &key,
            )?
            .is_some()
            {
                return Err(error::corruption(
                    "workspace scope catalog names a missing document",
                ));
            }
            if roots
                .get(run.as_str())
                .map_err(error::redb)?
                .is_some_and(|root| root.value() == scope.as_str())
            {
                return Err(error::corruption(
                    "root-scope index points to a missing workspace scope",
                ));
            }
            return Ok(None);
        };
        if validated_workspace_domain(&read, run)?.is_none() {
            return Err(error::corruption(
                "stored workspace scope has no accounting domain",
            ));
        }
        let stored = {
            let stored: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
            if stored.reference().run() != run || stored.reference().scope() != scope {
                return Err(error::corruption(
                    "workspace-scope key does not match its document",
                ));
            }
            validate_scope_catalog_lineage(&read, &table, &roots, stored.reference())?;
            stored
        };
        Ok(Some(stored))
    }

    fn value(
        &self,
        reference: &WorkspaceValueReference,
    ) -> Result<Option<WorkspaceValueEntry>, PersistenceError> {
        let key = workspace_value_key(reference)?;
        let read = self.database().begin_read().map_err(error::redb)?;
        validate_workspace_value_accounting(&read)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, reference.scope().run())?;
        let _membership = validate_run_history_membership(&read, reference.scope().run(), head)?;
        let table = read.open_table(VALUES).map_err(error::redb)?;
        let scopes = read.open_table(SCOPES).map_err(error::redb)?;
        let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
        validate_scope_catalog_lineage(&read, &scopes, &roots, reference.scope())?;
        let stored = table.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = stored else {
            let family = crate::trie::CatalogFamily::WorkspaceValue;
            if crate::trie::verify_member(
                &read,
                family,
                crate::trie::hashed_path(family, &key),
                &key,
            )?
            .is_some()
            {
                return Err(error::corruption(
                    "workspace value catalog names a missing document",
                ));
            }
            return Ok(None);
        };
        if validated_workspace_domain(&read, reference.scope().run())?.is_none() {
            return Err(error::corruption(
                "stored workspace value has no accounting domain",
            ));
        }
        let stored = {
            let stored: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
            if stored.reference() != reference {
                return Err(error::corruption(
                    "workspace-value key does not match its document",
                ));
            }
            validate_workspace_value_provenance(&table, &scopes, &roots, &stored, false)?;
            validate_scope_catalog_lineage(&read, &scopes, &roots, stored.reference().scope())?;
            validate_workspace_value_catalog_provenance(&read, &table, &scopes, &roots, &stored)?;
            stored
        };
        Ok(Some(stored))
    }

    fn latest_value(
        &self,
        scope: &ScopeReference,
        key: &ValueKey,
    ) -> Result<Option<WorkspaceValueEntry>, PersistenceError> {
        let head_key =
            codec::value_prefix(scope.run().as_str(), scope.scope().as_str(), key.as_str())?;
        let read = self.database().begin_read().map_err(error::redb)?;
        if validated_workspace_domain(&read, scope.run())?.is_none() {
            return Err(error::corruption(
                "workspace value lookup has no accounting domain",
            ));
        }
        let table = read.open_table(VALUES).map_err(error::redb)?;
        let scopes = read.open_table(SCOPES).map_err(error::redb)?;
        let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
        validate_scope_catalog_lineage(&read, &scopes, &roots, scope)?;
        let head = {
            let heads = read
                .open_table(WORKSPACE_VALUE_HEADS)
                .map_err(error::redb)?;
            heads
                .get(head_key.as_slice())
                .map_err(error::redb)?
                .map(|bytes| bytes.value().to_vec())
        };
        let head_family = crate::trie::CatalogFamily::WorkspaceValueHead;
        let witness = crate::trie::verify_member(
            &read,
            head_family,
            crate::trie::hashed_path(head_family, &head_key),
            &head_key,
        )?;
        let Some(head) = head else {
            if witness.is_some() {
                return Err(error::corruption(
                    "workspace value-head catalog names a missing document",
                ));
            }
            let end = codec::prefix_end(head_key.clone()).ok_or_else(|| {
                error::corruption("workspace value prefix has no exclusive range end")
            })?;
            if table
                .range(head_key.as_slice()..end.as_slice())
                .map_err(error::redb)?
                .next()
                .transpose()
                .map_err(error::redb)?
                .is_some()
            {
                return Err(error::corruption(
                    "workspace values exist without an authenticated latest-value head",
                ));
            }
            return Ok(None);
        };
        if witness != Some(crate::trie::digest_payload(head_family, &head)) {
            return Err(error::corruption(
                "workspace value head disagrees with its authenticated catalog",
            ));
        }
        let reference: WorkspaceValueReference = json::decode(&head, "workspace value head")?;
        if reference.scope() != scope || reference.key() != key {
            return Err(error::corruption(
                "workspace value-head key disagrees with its document",
            ));
        }
        let stored_key = workspace_value_key(&reference)?;
        let bytes = table
            .get(stored_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("workspace value head names a missing value"))?;
        let entry: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
        if entry.reference().scope() != scope || entry.reference().key() != key {
            return Err(error::corruption(
                "workspace latest-value range contains a mismatched document",
            ));
        }
        if stored_key != workspace_value_key(entry.reference())? {
            return Err(error::corruption(
                "workspace-value key does not match its document",
            ));
        }
        validate_workspace_value_provenance(&table, &scopes, &roots, &entry, false)?;
        validate_workspace_value_catalog_provenance(&read, &table, &scopes, &roots, &entry)?;
        Ok(Some(entry))
    }

    fn scope_lineage(
        &self,
        leaf: &ScopeReference,
    ) -> Result<Vec<WorkspaceScope>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        validate_workspace_value_accounting(&read)?;
        require_run_history_membership(&read, leaf.run())?;
        let table = read.open_table(SCOPES).map_err(error::redb)?;
        let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
        let mut current = leaf.clone();
        let mut reversed = Vec::new();
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_SCOPE_DEPTH {
            if !seen.insert(current.clone()) {
                return Err(error::corruption(
                    "workspace scope lineage contains a cycle",
                ));
            }
            let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
            let bytes = table.get(key.as_slice()).map_err(error::redb)?;
            let Some(bytes) = bytes else {
                let family = crate::trie::CatalogFamily::WorkspaceScope;
                if crate::trie::verify_member(
                    &read,
                    family,
                    workspace_scope_catalog_path(&current, &key)?,
                    &key,
                )?
                .is_some()
                {
                    return Err(error::corruption(
                        "workspace scope lineage catalog names a missing document",
                    ));
                }
                return Err(PersistenceError::NotFound {
                    entity: "workspace_scope",
                    identity: format!("{}/{}", current.run(), current.scope()),
                });
            };
            let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
            if reversed.is_empty() && validated_workspace_domain(&read, leaf.run())?.is_none() {
                return Err(error::corruption(
                    "workspace scope lineage has no accounting domain",
                ));
            }
            if scope.reference() != &current {
                return Err(error::corruption(
                    "workspace-scope key does not match its document",
                ));
            }
            let family = crate::trie::CatalogFamily::WorkspaceScope;
            let witness = crate::trie::verify_member(
                &read,
                family,
                workspace_scope_catalog_path(&current, &key)?,
                &key,
            )?;
            if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
                return Err(error::corruption(
                    "workspace scope disagrees with its authenticated catalog",
                ));
            }
            let parent = scope.parent().cloned();
            reversed.push(scope);
            match parent {
                Some(parent) => current = parent,
                None => {
                    let root = reversed.last().ok_or_else(|| {
                        error::corruption("workspace scope lineage unexpectedly became empty")
                    })?;
                    let indexed_root = roots
                        .get(root.reference().run().as_str())
                        .map_err(error::redb)?
                        .ok_or_else(|| {
                            error::corruption("run-root scope is missing from its root index")
                        })?;
                    if indexed_root.value() != root.reference().scope().as_str() {
                        return Err(error::corruption(
                            "run-root scope disagrees with its root index",
                        ));
                    }
                    reversed.reverse();
                    milkdrift_workspace::ScopeLineage::new(reversed.clone()).map_err(|cause| {
                        error::corruption(format!(
                            "stored workspace scope lineage failed validation: {cause}"
                        ))
                    })?;
                    return Ok(reversed);
                }
            }
        }
        Err(error::corruption(format!(
            "workspace scope lineage exceeds {MAX_SCOPE_DEPTH} entries"
        )))
    }
}

pub(crate) fn validate_owning_workspace_scope(
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    reference: &ScopeReference,
) -> Result<(), PersistenceError> {
    let mut current = reference.clone();
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_SCOPE_DEPTH {
        if !seen.insert(current.clone()) {
            return Err(error::corruption(
                "workspace scope lineage contains a cycle",
            ));
        }
        let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
        let bytes = scopes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("workspace scope lineage is incomplete"))?;
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        if scope.reference() != &current {
            return Err(error::corruption(
                "workspace-scope key does not match its document",
            ));
        }
        match (scope.kind(), scope.parent()) {
            (ScopeKind::RunRoot, None) => {
                let root = roots
                    .get(current.run().as_str())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption("run-root scope is missing from its root index")
                    })?;
                if root.value() != current.scope().as_str() {
                    return Err(error::corruption(
                        "run-root scope disagrees with its root index",
                    ));
                }
                return Ok(());
            }
            (ScopeKind::RunRoot, Some(_)) => {
                return Err(error::corruption("run-root scope has a parent"));
            }
            (_, Some(parent)) => current = parent.clone(),
            (_, None) => {
                return Err(error::corruption("non-root workspace scope has no parent"));
            }
        }
    }
    Err(error::corruption(format!(
        "workspace scope lineage exceeds {MAX_SCOPE_DEPTH} entries"
    )))
}
