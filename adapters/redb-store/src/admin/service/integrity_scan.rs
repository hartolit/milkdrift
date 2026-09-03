//! One administrative read transaction driving typed integrity phases in physical order.

use milkdrift_blueprint::BlueprintRevisionDocument;
use milkdrift_persistence::{
    IntegrityScanCursor, IntegrityScanFamily, IntegrityScanRequest, IntegrityScanResult,
    PersistenceError, RunSequence,
};
use milkdrift_workspace::ArtifactMetadata;
use std::ops::Bound;

use super::super::{
    ARTIFACT_MANIFEST, ARTIFACT_METADATA, METADATA, REVISIONS, RUN_EVENTS, RedbStore,
    SIGNAL_RECEIPTS, codec, error, json,
};
use super::super::{
    cursor::{
        integrity_cursor_state, integrity_cursor_str, make_integrity_cursor, push_failure,
        storage_anchor, validate_integrity_cursor,
    },
    integrity::scan_index_integrity,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PhaseResult {
    Complete,
    LimitReached,
}

struct IntegrityDriver<'request> {
    request: &'request IntegrityScanRequest,
    start_family: IntegrityScanFamily,
    maximum: u64,
    anchor: [u8; 32],
    result: IntegrityScanResult,
    last_cursor: Option<IntegrityScanCursor>,
}

impl<'request> IntegrityDriver<'request> {
    fn new(request: &'request IntegrityScanRequest, anchor: [u8; 32]) -> Self {
        Self {
            request,
            start_family: request
                .cursor
                .as_ref()
                .map_or(IntegrityScanFamily::Revisions, IntegrityScanCursor::family),
            maximum: u64::from(request.limit.get()),
            anchor,
            result: IntegrityScanResult {
                documents_checked: 0,
                artifacts_checked: 0,
                failures: Vec::new(),
                next_cursor: None,
            },
            last_cursor: None,
        }
    }

    fn admit_document(&mut self) -> bool {
        if self.result.documents_checked == self.maximum {
            return false;
        }
        self.result.documents_checked += 1;
        true
    }

    fn finish(mut self, phase: PhaseResult) -> IntegrityScanResult {
        if phase == PhaseResult::LimitReached {
            self.result.next_cursor = self.last_cursor;
        }
        self.result
    }

    fn scan_revisions(
        &mut self,
        revisions: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    ) -> Result<PhaseResult, PersistenceError> {
        if self.start_family > IntegrityScanFamily::Revisions {
            return Ok(PhaseResult::Complete);
        }
        let lower = if self.start_family == IntegrityScanFamily::Revisions {
            self.request
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
            if !self.admit_document() {
                return Ok(PhaseResult::LimitReached);
            }
            let (key, bytes) = item.map_err(error::redb)?;
            self.last_cursor = Some(make_integrity_cursor(
                IntegrityScanFamily::Revisions,
                key.value().as_bytes(),
                self.request.verify_artifact_content,
                self.anchor,
            )?);
            match BlueprintRevisionDocument::from_json(bytes.value()) {
                Ok((_document, revision)) if revision.id().as_str() == key.value() => {}
                Ok(_) => push_failure(
                    &mut self.result,
                    "revision",
                    "revision key does not match its verified document",
                )?,
                Err(cause) => push_failure(&mut self.result, "revision", &cause.to_string())?,
            }
        }
        Ok(PhaseResult::Complete)
    }

    fn scan_run_events(
        &mut self,
        read: &redb::ReadTransaction,
        events: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
        signal_receipts: &impl redb::ReadableTable<&'static [u8], u64>,
        metadata: &impl redb::ReadableTable<&'static str, u64>,
    ) -> Result<PhaseResult, PersistenceError> {
        if self.start_family > IntegrityScanFamily::RunEvents {
            return Ok(PhaseResult::Complete);
        }
        let mut previous_event_position = if self.start_family == IntegrityScanFamily::RunEvents {
            self.request
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
        let lower = if self.start_family == IntegrityScanFamily::RunEvents {
            self.request
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
            if !self.admit_document() {
                return Ok(PhaseResult::LimitReached);
            }
            let (key, bytes) = item.map_err(error::redb)?;
            self.last_cursor = Some(make_integrity_cursor(
                IntegrityScanFamily::RunEvents,
                key.value(),
                self.request.verify_artifact_content,
                self.anchor,
            )?);
            match milkdrift_persistence::RunEventEnvelope::from_json(bytes.value()) {
                Ok(event) => {
                    self.validate_event(
                        read,
                        signal_receipts,
                        metadata,
                        key.value(),
                        &event,
                        &mut previous_event_position,
                    )?;
                }
                Err(cause) => push_failure(&mut self.result, "journal", &cause.to_string())?,
            }
        }
        Ok(PhaseResult::Complete)
    }

    fn validate_event(
        &mut self,
        read: &redb::ReadTransaction,
        signal_receipts: &impl redb::ReadableTable<&'static [u8], u64>,
        metadata: &impl redb::ReadableTable<&'static str, u64>,
        key: &[u8],
        event: &milkdrift_persistence::RunEventEnvelope,
        previous_event_position: &mut Option<(milkdrift_workspace::RunId, RunSequence)>,
    ) -> Result<(), PersistenceError> {
        let contiguous = match previous_event_position.as_ref() {
            Some((previous_run, previous_sequence)) if previous_run == event.run_id() => {
                previous_sequence
                    .next()
                    .is_ok_and(|expected| expected == event.sequence())
            }
            Some(_) | None => event.sequence() == RunSequence::FIRST,
        };
        if !contiguous {
            push_failure(
                &mut self.result,
                "journal_history",
                "event table is not contiguous from sequence one within its run",
            )?;
        }
        *previous_event_position = Some((event.run_id().clone(), event.sequence()));
        let expected_key = codec::run_sequence(event.run_id().as_str(), event.sequence())?;
        if key != expected_key.as_slice() {
            push_failure(
                &mut self.result,
                "journal",
                "event key does not match its verified envelope",
            )?;
            return Ok(());
        }
        if let Err(cause) = crate::snapshot::validate_history_link(read, event) {
            push_failure(&mut self.result, "journal_history", &cause.to_string())?;
        }
        if let Err(cause) = crate::controller_account::validate_event_link(read, event) {
            push_failure(&mut self.result, "controller_accounts", &cause.to_string())?;
        }
        if let milkdrift_persistence::RunEventKind::SignalReceived { signal, .. } = event.kind() {
            let receipt_key = codec::pair(event.run_id().as_str(), signal.as_str())?;
            let indexed = signal_receipts
                .get(receipt_key.as_slice())
                .map_err(error::redb)?
                .map(|sequence| sequence.value());
            if indexed != Some(event.sequence().get()) {
                push_failure(
                    &mut self.result,
                    "signal_indexes",
                    "signal-received event is missing its exact receipt index",
                )?;
            }
        }
        if let milkdrift_persistence::RunEventKind::NodeScheduled { invocation, .. } = event.kind()
        {
            let invocation_key = crate::journal::invocation_fact_key(event.run_id(), invocation);
            let indexed = metadata
                .get(invocation_key.as_str())
                .map_err(error::redb)?
                .map(|sequence| sequence.value());
            if indexed != Some(event.sequence().get()) {
                push_failure(
                    &mut self.result,
                    "invocation_indexes",
                    "node-scheduled event is missing its exact invocation fact",
                )?;
            }
        }
        Ok(())
    }

    fn scan_artifacts(
        &mut self,
        store: &RedbStore,
        artifacts: &impl redb::ReadableTable<&'static str, &'static [u8]>,
        artifact_manifest: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    ) -> Result<PhaseResult, PersistenceError> {
        if self.start_family > IntegrityScanFamily::Artifacts {
            return Ok(PhaseResult::Complete);
        }
        let lower = if self.start_family == IntegrityScanFamily::Artifacts {
            self.request
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
            if !self.admit_document() {
                return Ok(PhaseResult::LimitReached);
            }
            let (key, bytes) = item.map_err(error::redb)?;
            self.last_cursor = Some(make_integrity_cursor(
                IntegrityScanFamily::Artifacts,
                key.value().as_bytes(),
                self.request.verify_artifact_content,
                self.anchor,
            )?);
            let metadata: Result<ArtifactMetadata, _> =
                json::decode(bytes.value(), "artifact metadata");
            match metadata {
                Ok(metadata) if metadata.reference().artifact().as_str() == key.value() => {
                    self.validate_artifact(store, artifact_manifest, key.value(), &metadata)?;
                }
                Ok(_) => push_failure(
                    &mut self.result,
                    "artifact_metadata",
                    "artifact key does not match its document",
                )?,
                Err(cause) => {
                    push_failure(&mut self.result, "artifact_metadata", &cause.to_string())?
                }
            }
        }
        Ok(PhaseResult::Complete)
    }

    fn validate_artifact(
        &mut self,
        store: &RedbStore,
        artifact_manifest: &impl redb::ReadableTable<&'static str, &'static [u8]>,
        key: &str,
        metadata: &ArtifactMetadata,
    ) -> Result<(), PersistenceError> {
        let manifest = artifact_manifest
            .get(key)
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "artifact manifest"))
            .transpose();
        if manifest.as_ref().ok() != Some(&Some(metadata.clone())) {
            push_failure(
                &mut self.result,
                "artifact_indexes",
                "artifact metadata is missing its exact authoritative manifest",
            )?;
        }
        if self.request.verify_artifact_content {
            self.result.artifacts_checked += 1;
            if let Err(cause) = crate::artifact::verify_blob(
                &store.content_path(metadata.reference().digest()),
                metadata.reference(),
                store.max_artifact_bytes,
            ) {
                push_failure(&mut self.result, "artifact_content", &cause.to_string())?;
            }
        }
        Ok(())
    }

    fn scan_indexes(
        &mut self,
        read: &redb::ReadTransaction,
    ) -> Result<PhaseResult, PersistenceError> {
        if self.start_family > IntegrityScanFamily::Indexes {
            return Ok(PhaseResult::Complete);
        }
        let mut more_remaining = false;
        scan_index_integrity(
            read,
            if self.start_family == IntegrityScanFamily::Indexes {
                self.request.cursor.as_ref()
            } else {
                None
            },
            self.maximum,
            self.request.verify_artifact_content,
            self.anchor,
            &mut self.result,
            &mut self.last_cursor,
            &mut more_remaining,
        )?;
        Ok(if more_remaining {
            PhaseResult::LimitReached
        } else {
            PhaseResult::Complete
        })
    }
}

pub(super) fn scan(
    store: &RedbStore,
    request: IntegrityScanRequest,
) -> Result<IntegrityScanResult, PersistenceError> {
    let read = store.database().begin_read().map_err(error::redb)?;
    let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let signal_receipts = read.open_table(SIGNAL_RECEIPTS).map_err(error::redb)?;
    let metadata = read.open_table(METADATA).map_err(error::redb)?;
    let artifacts = read.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    let artifact_manifest = read.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
    let anchor = storage_anchor(&read)?;
    validate_integrity_cursor(&request, &read, &revisions, &events, &artifacts)?;
    let mut driver = IntegrityDriver::new(&request, anchor);
    let mut phase = driver.scan_revisions(&revisions)?;
    if phase == PhaseResult::Complete {
        phase = driver.scan_run_events(&read, &events, &signal_receipts, &metadata)?;
    }
    if phase == PhaseResult::Complete {
        phase = driver.scan_artifacts(store, &artifacts, &artifact_manifest)?;
    }
    if phase == PhaseResult::Complete {
        phase = driver.scan_indexes(&read)?;
    }
    Ok(driver.finish(phase))
}
