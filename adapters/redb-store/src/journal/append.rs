use super::*;
use super::{
    discovery::{apply_indexes, record_artifact_references},
    queries::{decode_stored_event, validated_run_head},
    workspace::apply_workspace,
};
const COMMAND_RECORD_SCHEMA_VERSION: u32 = 1;

pub(crate) struct RunnableHeadState {
    pub(crate) previous_bytes: Option<Vec<u8>>,
    pub(crate) previous_witness: Option<[u8; 32]>,
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
pub(crate) struct OwnedCommandRecord {
    pub(crate) schema_version: u32,
    pub(crate) command: CommandId,
    pub(crate) run: RunId,
    pub(crate) actor: ActorRef,
    pub(crate) expected_sequence: RunSequence,
    pub(crate) submitted_at: TimestampMillis,
    pub(crate) canonical_document: Vec<u8>,
    pub(crate) fingerprint: IntegrityDigest,
    pub(crate) result: CommandResultDocument,
}

impl RunJournal for RedbStore {
    #[tracing::instrument(
        name = "milkdrift.redb_store.commit_command",
        skip_all,
        fields(
            run = %request.receipt().run(),
            command = %request.receipt().command(),
            expected_sequence = request.receipt().expected_sequence().get(),
            event_count = request.events().len()
        )
    )]
    fn commit_command(
        &self,
        request: &AtomicRunCommitRequest,
    ) -> Result<AtomicRunCommitOutcome, PersistenceError> {
        if let Some(accounting) = request.workspace_accounting() {
            for event in request.events() {
                if let RunEventKind::RunCreated {
                    workspace_budget, ..
                } = event.kind()
                    && workspace_budget != &accounting.budget
                {
                    return Err(PersistenceError::InvalidDocument(
                        "run-created workspace budget differs from atomic accounting budget"
                            .to_owned(),
                    ));
                }
            }
        }
        let command_key = codec::pair(
            request.receipt().run().as_str(),
            request.receipt().command().as_str(),
        )?;
        let command_bytes = encode_command_record(request)?;
        let write = self.database().begin_write().map_err(error::redb)?;
        crate::trie::validate_roots_in_transaction(&write)?;
        crate::artifact::validate_artifact_catalog(&write)?;
        crate::trie::validate_roots_in_transaction(&write)?;

        let actual_head = {
            let heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
            let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
            validated_run_head(&heads, &events, request.receipt().run())?
        };
        let previous_membership = validate_run_history_membership_in_transaction(
            &write,
            request.receipt().run(),
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
                if stored.run != *request.receipt().run()
                    || stored.command != *request.receipt().command()
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
                if stored.fingerprint == *request.receipt().fingerprint() {
                    return Ok(AtomicRunCommitOutcome::Replayed(stored.result));
                }
                return Err(PersistenceError::IdempotencyConflict {
                    run: request.receipt().run().clone(),
                    command: request.receipt().command().clone(),
                    existing: stored.fingerprint,
                    supplied: request.receipt().fingerprint().clone(),
                });
            }
        }
        validate_command_catalog_in_transaction(&write, &command_key, None)?;
        if actual_head != request.receipt().expected_sequence() {
            return Err(PersistenceError::SequenceConflict {
                run: request.receipt().run().clone(),
                expected: request.receipt().expected_sequence(),
                actual: actual_head,
            });
        }
        if let Some(expected) = request.expected_lease_catalog() {
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

        validate_required_artifacts(self, &write, request.required_artifacts())?;
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
        if !request.events().is_empty() {
            let mut heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
            heads
                .insert(
                    request.receipt().run().as_str(),
                    request.result().resulting_sequence().get(),
                )
                .map_err(error::redb)?;
            persist_run_membership(
                &write,
                request.receipt().run(),
                request.result().resulting_sequence(),
                previous_membership,
            )?;
        }
        persist_workspace_accounting(&write, request)?;
        crate::trie::validate_roots_in_transaction(&write)?;
        crate::trie::validate_roots_in_transaction(&write)?;

        self.faults.check(FaultPoint::BeforeCommandCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterCommandCommit)?;
        Ok(AtomicRunCommitOutcome::Committed(request.result().clone()))
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

pub(crate) fn encode_command_record(
    request: &AtomicRunCommitRequest,
) -> Result<Vec<u8>, PersistenceError> {
    json::encode(
        &StoredCommandRecord {
            schema_version: COMMAND_RECORD_SCHEMA_VERSION,
            command: request.receipt().command(),
            run: request.receipt().run(),
            actor: request.receipt().actor(),
            expected_sequence: request.receipt().expected_sequence(),
            submitted_at: request.receipt().submitted_at(),
            canonical_document: request.receipt().canonical_document(),
            fingerprint: request.receipt().fingerprint(),
            result: request.result(),
        },
        "command record",
    )
}

pub(crate) fn decode_command_record(bytes: &[u8]) -> Result<OwnedCommandRecord, PersistenceError> {
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

pub(crate) fn validate_command_result(
    result: &CommandResultDocument,
) -> Result<(), PersistenceError> {
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

pub(crate) fn validate_command_result_head(
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

pub(crate) fn validate_command_record_history<E, C>(
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

pub(crate) fn validate_command_event_catalog_in_transaction(
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

pub(crate) fn validate_command_event_catalog(
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

pub(crate) fn expected_run_membership<S>(
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

pub(crate) fn run_membership_payload(run: &RunId, head: RunSequence, summary: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.redb.run-membership.v1\0");
    hasher.update(&(run.as_str().len() as u64).to_be_bytes());
    hasher.update(run.as_str().as_bytes());
    hasher.update(&head.get().to_be_bytes());
    hasher.update(&(summary.len() as u64).to_be_bytes());
    hasher.update(summary);
    *hasher.finalize().as_bytes()
}

pub(crate) fn nonterminal_membership_path(run: &RunId) -> [u8; 32] {
    let family = crate::trie::CatalogFamily::NonterminalRun;
    crate::trie::hashed_path(family, run.as_str().as_bytes())
}

pub(crate) fn nonterminal_membership_payload(run_payload: [u8; 32]) -> [u8; 32] {
    crate::trie::digest_payload(crate::trie::CatalogFamily::NonterminalRun, &run_payload)
}

pub(crate) fn validate_nonterminal_membership(
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

pub(crate) fn validate_nonterminal_membership_in_transaction(
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

pub(crate) fn validate_run_membership_witness(
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

pub(crate) fn run_membership_path(run: &RunId) -> [u8; 32] {
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

pub(crate) fn validate_run_cursor_anchor(
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

pub(crate) fn validate_nonterminal_membership_leaf(
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

pub(crate) fn validate_event_catalog_in_transaction(
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

pub(crate) fn validate_event_catalog(
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

pub(crate) fn validate_event_boundary_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<(), PersistenceError> {
    let after = validate_head_event_in_transaction(write, run, head)?;
    let page =
        crate::trie::page_in_transaction(write, crate::trie::CatalogFamily::Event, None, after, 1)?;
    reject_event_beyond_head(run, page.leaves.first())
}

pub(crate) fn validate_event_boundary(
    read: &redb::ReadTransaction,
    run: &RunId,
    head: RunSequence,
) -> Result<(), PersistenceError> {
    let after = validate_head_event(read, run, head)?;
    let page = crate::trie::page(read, crate::trie::CatalogFamily::Event, None, after, 1)?;
    reject_event_beyond_head(run, page.leaves.first())
}

pub(crate) fn validate_head_event_in_transaction(
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

pub(crate) fn validate_head_event(
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

pub(crate) fn event_group_predecessor(run: &RunId) -> Result<Option<[u8; 32]>, PersistenceError> {
    let family = crate::trie::CatalogFamily::Event;
    let run_hash = crate::trie::hashed_path(family, run.as_str().as_bytes());
    let mut first = [0_u8; 32];
    first[..16].copy_from_slice(&run_hash[..16]);
    Ok(predecessor_path(first))
}

pub(crate) fn predecessor_path(mut path: [u8; 32]) -> Option<[u8; 32]> {
    for index in (0..path.len()).rev() {
        if path[index] != 0 {
            path[index] -= 1;
            path[index + 1..].fill(u8::MAX);
            return Some(path);
        }
    }
    None
}

pub(crate) fn reject_event_beyond_head(
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

pub(crate) fn validate_command_catalog_in_transaction(
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

pub(crate) fn validate_command_catalog(
    read: &redb::ReadTransaction,
    key: &[u8],
    stored: Option<&[u8]>,
) -> Result<(), PersistenceError> {
    let family = crate::trie::CatalogFamily::Command;
    let witness =
        crate::trie::verify_member(read, family, crate::trie::hashed_path(family, key), key)?;
    validate_catalog_document(family, witness, stored, "command")
}

pub(crate) fn validate_catalog_document(
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

pub(crate) fn validate_required_artifacts(
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

pub(crate) fn validate_artifact_accounting_references(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let newly_referenced: BTreeSet<_> = request.newly_referenced_artifacts().iter().collect();
    for reference in request.required_artifacts() {
        let previously_referenced =
            crate::artifact::validated_run_artifact_reference_in_transaction(
                write,
                request.receipt().run(),
                reference,
            )?;
        if newly_referenced.contains(reference) == previously_referenced {
            return Err(PersistenceError::InvalidDocument(format!(
                "artifact {} must be charged exactly on its first reference by run {}",
                reference.artifact(),
                request.receipt().run()
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_workspace_accounting(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(accounting) = request.workspace_accounting() else {
        return Ok(());
    };
    let actual = validate_or_initialize_workspace_domain(
        write,
        request.receipt().run(),
        &accounting.budget,
    )?;
    let reservations = write
        .open_table(ARTIFACT_RESERVATIONS)
        .map_err(error::redb)?;
    if reservations
        .get(request.receipt().run().as_str())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(PersistenceError::Storage {
            class: milkdrift_persistence::StorageFailureClass::OwnerBusy,
            message: format!(
                "run {} has an active artifact publication",
                request.receipt().run()
            ),
        });
    }
    if actual != accounting.expected_usage {
        return Err(PersistenceError::WorkspaceUsageConflict {
            run: request.receipt().run().clone(),
        });
    }
    Ok(())
}

pub(crate) fn workspace_domain_path(run: &RunId) -> [u8; 32] {
    let family = crate::trie::CatalogFamily::WorkspaceDomain;
    crate::trie::hashed_path(family, run.as_str().as_bytes())
}

pub(crate) fn workspace_domain_payload(
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

pub(crate) fn workspace_domain_in_transaction(
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

pub(crate) fn persist_workspace_domain(
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

pub(crate) fn persist_workspace_value_usage_accounting_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    usage: WorkspaceUsage,
) -> Result<(), PersistenceError> {
    crate::trie::validate_roots_in_transaction(write)?;
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
    crate::trie::validate_roots_in_transaction(write)?;
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
    crate::trie::validate_roots_in_transaction(write)?;
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
    crate::trie::validate_roots_in_transaction(write)?;
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

pub(crate) fn persist_workspace_accounting(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let Some(accounting) = request.workspace_accounting() else {
        return Ok(());
    };
    let Some((budget, actual, previous)) =
        workspace_domain_in_transaction(write, request.receipt().run())?
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
        .insert(request.receipt().run().as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    drop(table);
    persist_workspace_domain(
        write,
        request.receipt().run(),
        &budget,
        accounting.resulting_usage,
        Some(previous),
    )
}

pub(crate) fn append_events(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    let mut events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    let mut checksums = write.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
    for event in request.events() {
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
