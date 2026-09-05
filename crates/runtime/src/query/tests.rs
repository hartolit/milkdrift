//! Query tests.

use std::{
    error::Error,
    sync::{
        Mutex,
        atomic::{AtomicU32, AtomicUsize, Ordering},
    },
};

use milkdrift_blueprint::{ContentDigest, NodeId, RevisionId, WorkflowId};
use milkdrift_persistence::{
    ActiveLeaseSnapshot, EventCursor, EventId, EventPage, IntegrityDigest, LeaseIndexEntry,
    NodeExecutionId, NodeExecutionMode, NodeOutcome, PersistenceError, Reason, RunEventKind,
    RunSummaryIndex, RunSummaryPage, RunSummaryPageQuery, SnapshotDocument, SnapshotId,
    TimerIndexEntry, TimestampMillis, history_digest,
};
use milkdrift_workspace::{ScopeId, WorkspaceBudget, WorkspaceScope};

use super::*;

struct PagedStore {
    run: RunId,
    events: Vec<RunEventEnvelope>,
    largest_page_request: AtomicU32,
    page_reads: AtomicUsize,
    history_from_start_reads: AtomicUsize,
    grow_head_after_first_page: bool,
    snapshot: Mutex<SnapshotLoad>,
    discard_snapshot_error: bool,
}

impl SnapshotStore for PagedStore {
    fn history_digest(
        &self,
        run: &RunId,
        through: RunSequence,
    ) -> Result<IntegrityDigest, PersistenceError> {
        if run != &self.run || through == RunSequence::ZERO {
            return Err(PersistenceError::InvalidDocument(
                "test snapshot digest request is outside the configured run".to_owned(),
            ));
        }
        let end = usize::try_from(through.get()).map_err(|_| PersistenceError::Bounds {
            location: "test.snapshot_history",
            reason: "sequence exceeds usize".to_owned(),
        })?;
        history_digest(self.events.get(..end).ok_or_else(|| {
            PersistenceError::InvalidDocument(
                "test snapshot digest request exceeds history".to_owned(),
            )
        })?)
    }

    fn put_snapshot(&self, snapshot: &SnapshotDocument) -> Result<(), PersistenceError> {
        *self.snapshot.lock().map_err(|_| {
            PersistenceError::InvalidDocument("test snapshot lock is poisoned".to_owned())
        })? = SnapshotLoad::Verified(snapshot.clone());
        Ok(())
    }

    fn latest_snapshot(&self, _run: &RunId) -> Result<SnapshotLoad, PersistenceError> {
        self.snapshot
            .lock()
            .map_err(|_| {
                PersistenceError::InvalidDocument("test snapshot lock is poisoned".to_owned())
            })
            .map(|snapshot| snapshot.clone())
    }

    fn discard_snapshot(
        &self,
        _run: &RunId,
        _snapshot: &SnapshotId,
    ) -> Result<(), PersistenceError> {
        if self.discard_snapshot_error {
            return Err(PersistenceError::InvalidDocument(
                "simulated snapshot cleanup failure".to_owned(),
            ));
        }
        *self.snapshot.lock().map_err(|_| {
            PersistenceError::InvalidDocument("test snapshot lock is poisoned".to_owned())
        })? = SnapshotLoad::Absent;
        Ok(())
    }
}

#[test]
fn projection_payload_cardinality_is_rejected_by_lexical_preflight() {
    let mut payload = br#"{"schema_version":4,"projection":{"oversized":["#.to_vec();
    for index in 0..=PROJECTION_PAYLOAD_JSON_LIMITS.maximum_container_items {
        if index != 0 {
            payload.push(b',');
        }
        payload.extend_from_slice(b"null");
    }
    payload.extend_from_slice(b"]}}");
    let result = decode_projection_snapshot_payload(&payload);
    assert!(
        result.is_err(),
        "oversized projection array passed lexical preflight"
    );
    if let Err(error) = result {
        assert!(error.contains("Array"), "unexpected rejection: {error}");
    }
}

impl RunQueryStore for PagedStore {
    fn events(&self, query: &EventPageQuery) -> Result<EventPage, PersistenceError> {
        self.largest_page_request
            .fetch_max(query.limit.get(), Ordering::SeqCst);
        if query.cursor.is_none() {
            self.history_from_start_reads.fetch_add(1, Ordering::SeqCst);
        }
        let read_index = self.page_reads.fetch_add(1, Ordering::SeqCst);
        let effective_len = if query.run != self.run {
            0
        } else if self.grow_head_after_first_page && read_index == 0 {
            self.events.len().saturating_sub(1)
        } else {
            self.events.len()
        };
        let observed_head = RunSequence::new(u64::try_from(effective_len).map_err(|_| {
            PersistenceError::Bounds {
                location: "test.event_history",
                reason: "event history length exceeds u64".to_owned(),
            }
        })?);
        let Some(next_sequence) = query.start_sequence(observed_head)? else {
            return Ok(EventPage {
                events: Vec::new(),
                next: None,
                observed_head,
            });
        };
        let start_offset = next_sequence.get().checked_sub(1).ok_or_else(|| {
            PersistenceError::InvalidCursor("event cursor named sequence zero".to_owned())
        })?;
        let start = usize::try_from(start_offset).map_err(|_| PersistenceError::Bounds {
            location: "test.event_cursor",
            reason: "cursor exceeds usize".to_owned(),
        })?;
        let limit = usize::try_from(query.limit.get()).map_err(|_| PersistenceError::Bounds {
            location: "test.event_page_size",
            reason: "page size exceeds usize".to_owned(),
        })?;
        let end = start
            .checked_add(limit)
            .ok_or_else(|| PersistenceError::Bounds {
                location: "test.event_page",
                reason: "event page end overflowed usize".to_owned(),
            })?
            .min(effective_len);
        let page_events = self.events.get(start..end).ok_or_else(|| {
            PersistenceError::Corruption(
                "test event page range exceeds authoritative history".to_owned(),
            )
        })?;
        let mut expected = next_sequence;
        for (index, event) in page_events.iter().enumerate() {
            if event.run_id() != &query.run || event.sequence() != expected {
                return Err(PersistenceError::Corruption(
                    "test event history is not contiguous for the queried run".to_owned(),
                ));
            }
            if index + 1 < page_events.len() {
                expected = expected.next()?;
            }
        }
        let next = if end < effective_len {
            Some(EventCursor {
                run: query.run.clone(),
                next_sequence: page_events
                    .last()
                    .ok_or_else(|| {
                        PersistenceError::Corruption(
                            "non-terminal test event page was empty".to_owned(),
                        )
                    })?
                    .sequence()
                    .next()?,
            })
        } else {
            None
        };
        Ok(EventPage {
            events: page_events.to_vec(),
            next,
            observed_head,
        })
    }

    fn signal_receipt(
        &self,
        run: &RunId,
        signal: &milkdrift_persistence::SignalId,
    ) -> Result<Option<RunEventEnvelope>, PersistenceError> {
        if run != &self.run {
            return Ok(None);
        }
        Ok(self
            .events
            .iter()
            .find(|event| {
                matches!(
                    event.kind(),
                    RunEventKind::SignalReceived {
                        signal: received,
                        ..
                    } if received == signal
                )
            })
            .cloned())
    }

    fn run_summary(&self, _run: &RunId) -> Result<Option<RunSummaryIndex>, PersistenceError> {
        Ok(None)
    }

    fn run_summaries(
        &self,
        _query: &RunSummaryPageQuery,
    ) -> Result<RunSummaryPage, PersistenceError> {
        Ok(RunSummaryPage {
            runs: Vec::new(),
            next: None,
        })
    }

    fn nonterminal_run_page(
        &self,
        _cursor: Option<&milkdrift_persistence::RunSummaryCursor>,
        _limit: PageSize,
    ) -> Result<RunSummaryPage, PersistenceError> {
        Ok(RunSummaryPage {
            runs: Vec::new(),
            next: None,
        })
    }

    fn runnable_page(
        &self,
        _eligible_through: TimestampMillis,
        _cursor: Option<&milkdrift_persistence::RunnableCursor>,
        _limit: PageSize,
    ) -> Result<milkdrift_persistence::RunnablePage, PersistenceError> {
        Ok(milkdrift_persistence::RunnablePage {
            entries: Vec::new(),
            next: None,
        })
    }

    fn active_leases(&self, _limit: PageSize) -> Result<ActiveLeaseSnapshot, PersistenceError> {
        Ok(ActiveLeaseSnapshot {
            entries: Vec::new(),
            revision: IntegrityDigest::hash(b"empty test lease revision"),
        })
    }

    fn due_timers(
        &self,
        _due_through: TimestampMillis,
        _limit: PageSize,
    ) -> Result<Vec<TimerIndexEntry>, PersistenceError> {
        Ok(Vec::new())
    }

    fn expired_leases(
        &self,
        _expired_through: TimestampMillis,
        _limit: PageSize,
    ) -> Result<Vec<LeaseIndexEntry>, PersistenceError> {
        Ok(Vec::new())
    }
}

#[test]
fn complete_history_fold_holds_only_bounded_pages() -> Result<(), RuntimeError> {
    let run = RunId::new("run-paged-fold")
        .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
    let mut events = Vec::new();
    for sequence in 1..=2_501_u64 {
        events.push(RunEventEnvelope::new(
            milkdrift_persistence::EventId::new(format!("event-{sequence}"))?,
            run.clone(),
            RunSequence::new(sequence),
            TimestampMillis::new(sequence),
            RunEventKind::RunStarted,
        )?);
    }
    let store = PagedStore {
        run: run.clone(),
        events,
        largest_page_request: AtomicU32::new(0),
        page_reads: AtomicUsize::new(0),
        history_from_start_reads: AtomicUsize::new(0),
        grow_head_after_first_page: false,
        snapshot: Mutex::new(SnapshotLoad::Absent),
        discard_snapshot_error: false,
    };

    let count = fold_complete_history(&store, &run, 0_usize, |count, _event| {
        *count = count.saturating_add(1);
        Ok(())
    })?;

    assert_eq!(count, 2_501);
    assert_eq!(store.largest_page_request.load(Ordering::SeqCst), 1_000);
    assert_eq!(store.page_reads.load(Ordering::SeqCst), 3);
    assert!(matches!(
        load_bounded_history(&store, &run, PageSize::new(1_000)?),
        Err(RuntimeError::Persistence(PersistenceError::Bounds {
            location: "runtime.bounded_history",
            ..
        }))
    ));
    Ok(())
}

#[test]
fn complete_history_fold_pins_the_first_observed_head_during_concurrent_append()
-> Result<(), RuntimeError> {
    let run = RunId::new("run-pinned-fold")
        .map_err(|error| RuntimeError::InvalidHistory(error.to_string()))?;
    let mut events = Vec::new();
    for sequence in 1..=2_502_u64 {
        events.push(RunEventEnvelope::new(
            milkdrift_persistence::EventId::new(format!("pinned-event-{sequence}"))?,
            run.clone(),
            RunSequence::new(sequence),
            TimestampMillis::new(sequence),
            RunEventKind::RunStarted,
        )?);
    }
    let store = PagedStore {
        run: run.clone(),
        events,
        largest_page_request: AtomicU32::new(0),
        page_reads: AtomicUsize::new(0),
        history_from_start_reads: AtomicUsize::new(0),
        grow_head_after_first_page: true,
        snapshot: Mutex::new(SnapshotLoad::Absent),
        discard_snapshot_error: false,
    };

    let count = fold_complete_history(&store, &run, 0_usize, |count, _event| {
        *count = count.saturating_add(1);
        Ok(())
    })?;

    assert_eq!(count, 2_501);
    assert_eq!(store.page_reads.load(Ordering::SeqCst), 3);
    Ok(())
}

fn snapshot_history_fixture(name: &str) -> Result<(RunId, Vec<RunEventEnvelope>), Box<dyn Error>> {
    let run = RunId::new(format!("run-{name}"))?;
    let workflow = WorkflowId::new(format!("workflow-{name}"))?;
    let revision: RevisionId = serde_json::from_str(&format!("\"rev_{}\"", "a".repeat(64)))?;
    let revision_digest: ContentDigest =
        serde_json::from_str(&format!("\"b3_{}\"", "1".repeat(64)))?;
    let root_scope = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
    let budget = WorkspaceBudget::new(100, 10_000, 100_000, 100, 100_000, 1_000_000)?;
    let created = RunEventEnvelope::new(
        EventId::new(format!("event-{name}-created"))?,
        run.clone(),
        RunSequence::FIRST,
        TimestampMillis::new(1),
        RunEventKind::RunCreated {
            workflow,
            revision,
            revision_digest,
            root_scope,
            workspace_budget: budget,
            inputs: Vec::new(),
        },
    )?;
    let started = RunEventEnvelope::new(
        EventId::new(format!("event-{name}-started"))?,
        run.clone(),
        RunSequence::new(2),
        TimestampMillis::new(2),
        RunEventKind::RunStarted,
    )?;
    Ok((run, vec![created, started]))
}

fn assert_projection_payload_falls_back(
    name: &str,
    mutate: impl FnOnce(&mut serde_json::Value) -> Result<(), Box<dyn Error>>,
    canonicalize_known_schema: bool,
) -> Result<(), Box<dyn Error>> {
    let (run, events) = snapshot_history_fixture(name)?;
    let mut prefix = RunProjection::new();
    prefix.apply_replayed(&events[0])?;
    let mut value: serde_json::Value =
        serde_json::from_slice(&encode_projection_snapshot(&prefix)?)?;
    mutate(&mut value)?;
    let payload = if canonicalize_known_schema {
        let decoded: ProjectionSnapshotPayload = serde_json::from_value(value)?;
        serde_json::to_vec(&decoded)?
    } else {
        serde_json::to_vec(&value)?
    };
    let snapshot = SnapshotDocument::new(
        SnapshotId::new(format!("snapshot-{name}"))?,
        run.clone(),
        RunSequence::FIRST,
        history_digest(&events[..1])?,
        RUN_PROJECTION_SNAPSHOT_SCHEMA_V4,
        payload,
    )?;
    let expected = RunProjection::replay(&events)?;
    let store = PagedStore {
        run: run.clone(),
        events,
        largest_page_request: AtomicU32::new(0),
        page_reads: AtomicUsize::new(0),
        history_from_start_reads: AtomicUsize::new(0),
        grow_head_after_first_page: false,
        snapshot: Mutex::new(SnapshotLoad::Verified(snapshot)),
        discard_snapshot_error: false,
    };

    let projected = project_from_latest_snapshot(&store, &run)?;

    assert_eq!(projected, expected);
    assert_eq!(store.history_from_start_reads.load(Ordering::SeqCst), 1);
    assert!(matches!(
        *store
            .snapshot
            .lock()
            .map_err(|_| "snapshot fallback test lock is poisoned")?,
        SnapshotLoad::Absent
    ));
    Ok(())
}

fn compacted_projection_snapshot_golden()
-> Result<(SnapshotDocument, RunProjection), Box<dyn Error>> {
    let (run, mut events) = snapshot_history_fixture("snapshot-golden")?;
    let scope = match events[0].kind() {
        RunEventKind::RunCreated { root_scope, .. } => root_scope.reference().clone(),
        _ => return Err("snapshot golden fixture does not begin with run creation".into()),
    };
    let execution = NodeExecutionId::new("snapshot-golden-execution")?;
    events.extend([
        RunEventEnvelope::new(
            EventId::new("event-snapshot-golden-eligible")?,
            run.clone(),
            RunSequence::new(3),
            TimestampMillis::new(3),
            RunEventKind::NodeBecameEligible {
                node: NodeId::new("work")?,
                execution: execution.clone(),
                scope,
                mode: NodeExecutionMode::Runtime,
            },
        )?,
        RunEventEnvelope::new(
            EventId::new("event-snapshot-golden-terminal")?,
            run.clone(),
            RunSequence::new(4),
            TimestampMillis::new(4),
            RunEventKind::DeterministicNodeTerminal {
                execution: execution.clone(),
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?,
        RunEventEnvelope::new(
            EventId::new("event-snapshot-golden-scan")?,
            run.clone(),
            RunSequence::new(5),
            TimestampMillis::new(5),
            RunEventKind::StructuredSuccessorScanCompleted { execution },
        )?,
    ]);
    let projection = RunProjection::replay(&events)?;
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-runtime-projection-v4-golden")?,
        run,
        RunSequence::new(5),
        history_digest(&events)?,
        RUN_PROJECTION_SNAPSHOT_SCHEMA_V4,
        encode_projection_snapshot(&projection)?,
    )?;
    Ok((snapshot, projection))
}

#[test]
fn exact_snapshot_envelope_and_compacted_projection_v4_match_reviewed_golden()
-> Result<(), Box<dyn Error>> {
    let (snapshot, projection) = compacted_projection_snapshot_golden()?;
    let wire_with_newline = include_bytes!(
        "../../tests/fixtures/projection-snapshot-envelope-v2-projection-v4-wire.json"
    );
    let wire = wire_with_newline
        .strip_suffix(b"\n")
        .unwrap_or(wire_with_newline);
    let unsupported_v1_wire = include_bytes!(
        "../../tests/fixtures/unsupported-projection-snapshot-envelope-v1-projection-v3-wire.json"
    );
    assert!(wire.len().saturating_mul(2) < unsupported_v1_wire.len());
    assert_eq!(snapshot.to_canonical_json()?, wire);
    let decoded_wire = SnapshotDocument::from_json(wire)?;
    assert_eq!(decoded_wire, snapshot);
    assert_eq!(decoded_wire.to_canonical_json()?, wire);

    let golden: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/projection-snapshot-envelope-v2-projection-v4.json"
    ))?;
    let envelope = golden
        .get("snapshot_envelope")
        .ok_or("snapshot golden envelope is absent")?;
    assert_eq!(envelope["schema_version"], serde_json::json!(2));
    assert_eq!(
        envelope["snapshot"],
        serde_json::to_value(snapshot.snapshot())?
    );
    assert_eq!(envelope["run"], serde_json::to_value(snapshot.run())?);
    assert_eq!(
        envelope["covered_sequence"],
        serde_json::to_value(snapshot.covered_sequence())?
    );
    assert_eq!(
        envelope["history_digest"],
        serde_json::to_value(snapshot.history_digest())?
    );
    assert_eq!(
        envelope["projection_payload_schema"],
        serde_json::json!(snapshot.projection_payload_schema())
    );
    assert_eq!(
        envelope["checksum"],
        serde_json::to_value(snapshot.checksum())?
    );

    let decoded: ProjectionSnapshotPayload = serde_json::from_value(
        golden
            .get("projection_payload")
            .ok_or("snapshot golden projection payload is absent")?
            .clone(),
    )?;
    assert_eq!(decoded.schema_version, RUN_PROJECTION_SNAPSHOT_SCHEMA_V4);
    assert_eq!(decoded.projection, projection);
    let canonical = serde_json::to_vec(&ProjectionSnapshotPayloadRef {
        schema_version: decoded.schema_version,
        projection: &decoded.projection,
    })?;
    assert_eq!(canonical, snapshot.payload());
    let redecoded: ProjectionSnapshotPayload = serde_json::from_slice(&canonical)?;
    assert_eq!(redecoded.projection, decoded.projection);
    Ok(())
}

#[test]
fn compatible_snapshot_replays_only_the_authoritative_tail() -> Result<(), Box<dyn Error>> {
    let (run, events) = snapshot_history_fixture("snapshot-tail")?;
    let mut prefix = RunProjection::new();
    prefix.apply_replayed(&events[0])?;
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-query-tail")?,
        run.clone(),
        RunSequence::FIRST,
        history_digest(&events[..1])?,
        RUN_PROJECTION_SNAPSHOT_SCHEMA_V4,
        encode_projection_snapshot(&prefix)?,
    )?;
    let store = PagedStore {
        run: run.clone(),
        events,
        largest_page_request: AtomicU32::new(0),
        page_reads: AtomicUsize::new(0),
        history_from_start_reads: AtomicUsize::new(0),
        grow_head_after_first_page: false,
        snapshot: Mutex::new(SnapshotLoad::Verified(snapshot)),
        discard_snapshot_error: false,
    };

    let projected = project_from_latest_snapshot(&store, &run)?;
    let replayed = project_complete_history(&store, &run)?;

    assert_eq!(projected.sequence(), RunSequence::new(2));
    assert_eq!(projected.history_compacted_through(), RunSequence::new(2));
    assert!(matches!(
        projected.lifecycle(),
        crate::RunLifecycle::Running
    ));
    assert_eq!(projected, replayed);
    assert_eq!(store.page_reads.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn compacted_execution_snapshot_plus_tail_equals_full_replay() -> Result<(), Box<dyn Error>> {
    let (run, mut events) = snapshot_history_fixture("snapshot-compacted-execution")?;
    let scope = match events[0].kind() {
        RunEventKind::RunCreated { root_scope, .. } => root_scope.reference().clone(),
        _ => return Err("snapshot fixture does not begin with run creation".into()),
    };
    let execution = NodeExecutionId::new("snapshot-settled-execution")?;
    events.extend([
        RunEventEnvelope::new(
            EventId::new("event-snapshot-settled-eligible")?,
            run.clone(),
            RunSequence::new(3),
            TimestampMillis::new(3),
            RunEventKind::NodeBecameEligible {
                node: NodeId::new("work")?,
                execution: execution.clone(),
                scope,
                mode: NodeExecutionMode::Runtime,
            },
        )?,
        RunEventEnvelope::new(
            EventId::new("event-snapshot-settled-terminal")?,
            run.clone(),
            RunSequence::new(4),
            TimestampMillis::new(4),
            RunEventKind::DeterministicNodeTerminal {
                execution: execution.clone(),
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?,
        RunEventEnvelope::new(
            EventId::new("event-snapshot-settled-successor-scan")?,
            run.clone(),
            RunSequence::new(5),
            TimestampMillis::new(5),
            RunEventKind::StructuredSuccessorScanCompleted {
                execution: execution.clone(),
            },
        )?,
        RunEventEnvelope::new(
            EventId::new("event-snapshot-settled-tail-pause")?,
            run.clone(),
            RunSequence::new(6),
            TimestampMillis::new(6),
            RunEventKind::RunPaused {
                reason: Reason::new("verify compact snapshot tail")?,
                evidence: Vec::new(),
            },
        )?,
    ]);
    let prefix = RunProjection::replay(&events[..5])?;
    assert!(prefix.node_executions().is_empty());
    assert_eq!(prefix.settled_node_executions().len(), 1);
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-query-compacted-execution")?,
        run.clone(),
        RunSequence::new(5),
        history_digest(&events[..5])?,
        RUN_PROJECTION_SNAPSHOT_SCHEMA_V4,
        encode_projection_snapshot(&prefix)?,
    )?;
    let store = PagedStore {
        run: run.clone(),
        events,
        largest_page_request: AtomicU32::new(0),
        page_reads: AtomicUsize::new(0),
        history_from_start_reads: AtomicUsize::new(0),
        grow_head_after_first_page: false,
        snapshot: Mutex::new(SnapshotLoad::Verified(snapshot)),
        discard_snapshot_error: false,
    };

    let projected = project_from_latest_snapshot(&store, &run)?;
    let replayed = project_complete_history(&store, &run)?;

    assert_eq!(projected, replayed);
    assert_eq!(projected.sequence(), RunSequence::new(6));
    assert!(matches!(projected.lifecycle(), crate::RunLifecycle::Paused));
    assert!(projected.node_executions().is_empty());
    assert_eq!(projected.settled_node_executions().len(), 1);
    Ok(())
}

#[test]
fn compatible_snapshot_at_journal_head_loads_without_replaying_an_event()
-> Result<(), Box<dyn Error>> {
    let (run, events) = snapshot_history_fixture("snapshot-at-head")?;
    let mut complete = RunProjection::new();
    for event in &events {
        complete.apply_replayed(event)?;
    }
    let head = RunSequence::new(2);
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-query-at-head")?,
        run.clone(),
        head,
        history_digest(&events)?,
        RUN_PROJECTION_SNAPSHOT_SCHEMA_V4,
        encode_projection_snapshot(&complete)?,
    )?;
    let store = PagedStore {
        run: run.clone(),
        events,
        largest_page_request: AtomicU32::new(0),
        page_reads: AtomicUsize::new(0),
        history_from_start_reads: AtomicUsize::new(0),
        grow_head_after_first_page: false,
        snapshot: Mutex::new(SnapshotLoad::Verified(snapshot)),
        discard_snapshot_error: false,
    };

    let projected = project_from_latest_snapshot(&store, &run)?;

    assert_eq!(projected.sequence(), head);
    assert_eq!(projected.history_compacted_through(), head);
    assert!(matches!(
        projected.lifecycle(),
        crate::RunLifecycle::Running
    ));
    assert_eq!(store.history_from_start_reads.load(Ordering::SeqCst), 0);
    assert!(matches!(
        *store
            .snapshot
            .lock()
            .map_err(|_| "snapshot positive test lock is poisoned")?,
        SnapshotLoad::Verified(_)
    ));
    assert_eq!(store.page_reads.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn payload_schema_mismatch_discards_snapshot_and_replays_from_start() -> Result<(), Box<dyn Error>>
{
    assert_projection_payload_falls_back(
        "snapshot-payload-schema-mismatch",
        |value| {
            value["schema_version"] = serde_json::json!(2);
            Ok(())
        },
        true,
    )
}

#[test]
fn payload_run_mismatch_discards_snapshot_and_replays_from_start() -> Result<(), Box<dyn Error>> {
    assert_projection_payload_falls_back(
        "snapshot-payload-run-mismatch",
        |value| {
            value["projection"]["run_id"] =
                serde_json::to_value(RunId::new("run-another-snapshot")?)?;
            Ok(())
        },
        true,
    )
}

#[test]
fn payload_sequence_mismatch_discards_snapshot_and_replays_from_start() -> Result<(), Box<dyn Error>>
{
    assert_projection_payload_falls_back(
        "snapshot-payload-sequence-mismatch",
        |value| {
            value["projection"]["sequence"] = serde_json::json!(2);
            Ok(())
        },
        true,
    )
}

#[test]
fn payload_compaction_mismatch_discards_snapshot_and_replays_from_start()
-> Result<(), Box<dyn Error>> {
    assert_projection_payload_falls_back(
        "snapshot-payload-compaction-mismatch",
        |value| {
            value["projection"]["history_compacted_through"] = serde_json::json!(0);
            Ok(())
        },
        true,
    )
}

#[test]
fn nested_unknown_projection_field_discards_snapshot_and_replays_from_start()
-> Result<(), Box<dyn Error>> {
    assert_projection_payload_falls_back(
        "snapshot-nested-unknown-field",
        |value| {
            value["projection"]["future_projection_field"] = serde_json::json!(true);
            Ok(())
        },
        false,
    )
}

#[test]
fn unsupported_v3_snapshot_never_blocks_authoritative_replay_when_cleanup_fails()
-> Result<(), Box<dyn Error>> {
    let (run, events) = snapshot_history_fixture("snapshot-fallback")?;
    let mut prefix = RunProjection::new();
    prefix.apply_replayed(&events[0])?;
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-query-fallback")?,
        run.clone(),
        RunSequence::FIRST,
        history_digest(&events[..1])?,
        3,
        encode_projection_snapshot(&prefix)?,
    )?;
    let store = PagedStore {
        run: run.clone(),
        events,
        largest_page_request: AtomicU32::new(0),
        page_reads: AtomicUsize::new(0),
        history_from_start_reads: AtomicUsize::new(0),
        grow_head_after_first_page: false,
        snapshot: Mutex::new(SnapshotLoad::Verified(snapshot)),
        discard_snapshot_error: true,
    };

    let projected = project_from_latest_snapshot(&store, &run)?;

    assert_eq!(projected.sequence(), RunSequence::new(2));
    assert_eq!(projected.history_compacted_through(), RunSequence::new(2));
    assert!(matches!(
        projected.lifecycle(),
        crate::RunLifecycle::Running
    ));
    Ok(())
}
