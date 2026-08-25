use milkdrift_persistence::{
    EventCursor, EventPageQuery, MAX_PAGE_SIZE, PageSize, RunEventEnvelope, RunQueryStore,
    RunSequence, SnapshotLoad, SnapshotStore,
};
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{RunProjection, RuntimeError};

pub(crate) const RUN_PROJECTION_SNAPSHOT_SCHEMA_V3: u32 = 3;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectionSnapshotPayload {
    schema_version: u32,
    projection: RunProjection,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectionSnapshotPayloadRef<'a> {
    schema_version: u32,
    projection: &'a RunProjection,
}

/// Folds one authoritative history range against the head observed by the first read.
///
/// Concurrent appends after that read are intentionally ignored; callers receive a
/// consistent prefix instead of restarting from sequence one. Every page must remain
/// contiguous and may never regress behind the pinned head.
pub(crate) fn fold_history_from<S, T, F>(
    store: &S,
    run: &RunId,
    covered_through: RunSequence,
    mut accumulator: T,
    mut fold: F,
) -> Result<T, RuntimeError>
where
    S: RunQueryStore + ?Sized,
    F: FnMut(&mut T, &RunEventEnvelope) -> Result<(), RuntimeError>,
{
    let limit = PageSize::new(MAX_PAGE_SIZE)?;
    let mut cursor = if covered_through == RunSequence::ZERO {
        None
    } else {
        Some(EventCursor {
            run: run.clone(),
            next_sequence: covered_through.next()?,
        })
    };
    let mut pinned_head = None;
    let mut folded_through = covered_through;
    let mut expected = if covered_through == RunSequence::ZERO {
        RunSequence::FIRST
    } else {
        covered_through.next()?
    };

    loop {
        let page = store.events(&EventPageQuery::new(run.clone(), cursor.clone(), limit)?)?;
        let head = *pinned_head.get_or_insert(page.observed_head);
        if page.observed_head < head {
            return Err(RuntimeError::InvalidHistory(
                "journal head regressed during a pinned history fold".to_owned(),
            ));
        }
        if covered_through > head {
            return Err(RuntimeError::InvalidHistory(format!(
                "projection checkpoint covers {covered_through}, beyond observed journal head {head}"
            )));
        }
        if folded_through == head {
            return Ok(accumulator);
        }
        if page.events.is_empty() {
            return Err(RuntimeError::InvalidHistory(format!(
                "history page was empty before pinned head {head} was reached"
            )));
        }

        let mut physical_tail = None;
        for event in &page.events {
            if event.sequence() != expected {
                return Err(RuntimeError::InvalidHistory(format!(
                    "history page expected sequence {expected}, found {}",
                    event.sequence()
                )));
            }
            physical_tail = Some(event.sequence());
            expected = expected.next()?;
            if event.sequence() > head {
                break;
            }
            fold(&mut accumulator, event)?;
            folded_through = event.sequence();
            if folded_through == head {
                return Ok(accumulator);
            }
        }

        let next = page.next.ok_or_else(|| {
            RuntimeError::InvalidHistory(format!(
                "history pagination ended at {folded_through} before pinned head {head}"
            ))
        })?;
        let expected_next = physical_tail
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "non-empty history page had no physical tail".to_owned(),
                )
            })?
            .next()?;
        if next.next_sequence != expected_next {
            return Err(RuntimeError::InvalidHistory(
                "event pagination cursor did not advance beyond the page tail".to_owned(),
            ));
        }
        cursor = Some(next);
    }
}

/// Folds complete authoritative history without retaining prior pages.
pub(crate) fn fold_complete_history<S, T, F>(
    store: &S,
    run: &RunId,
    accumulator: T,
    fold: F,
) -> Result<T, RuntimeError>
where
    S: RunQueryStore + ?Sized,
    F: FnMut(&mut T, &RunEventEnvelope) -> Result<(), RuntimeError>,
{
    fold_history_from(store, run, RunSequence::ZERO, accumulator, fold)
}

/// Projects complete authoritative history while retaining only projection state and
/// the current bounded event page.
pub(crate) fn project_complete_history<S: RunQueryStore + ?Sized>(
    store: &S,
    run: &RunId,
) -> Result<RunProjection, RuntimeError> {
    fold_complete_history(store, run, RunProjection::new(), |projection, event| {
        projection.apply_replayed(event)
    })
}

fn discard_optional_snapshot<S: SnapshotStore + ?Sized>(
    store: &S,
    run: &RunId,
    snapshot: &milkdrift_persistence::SnapshotId,
    reason: &str,
) {
    if let Err(error) = store.discard_snapshot(run, snapshot) {
        warn!(
            run = %run,
            snapshot = %snapshot,
            reason = %error,
            rejection = reason,
            "invalid optional projection snapshot could not be discarded"
        );
    }
}

/// Loads a verified runtime checkpoint and replays only the authoritative tail.
/// Invalid or unsupported optional snapshots are discarded best-effort and never repair
/// or block authoritative history replay.
pub(crate) fn project_from_latest_snapshot<S>(
    store: &S,
    run: &RunId,
) -> Result<RunProjection, RuntimeError>
where
    S: RunQueryStore + SnapshotStore + ?Sized,
{
    let snapshot = match store.latest_snapshot(run)? {
        SnapshotLoad::Absent => return project_complete_history(store, run),
        SnapshotLoad::Rejected { snapshot, reason } => {
            if let Some(snapshot) = snapshot {
                discard_optional_snapshot(store, run, &snapshot, reason.as_str());
            }
            warn!(run = %run, reason = %reason.as_str(), "rejected snapshot ignored");
            return project_complete_history(store, run);
        }
        SnapshotLoad::Verified(snapshot) => snapshot,
    };

    if snapshot.projection_schema() != RUN_PROJECTION_SNAPSHOT_SCHEMA_V3 {
        discard_optional_snapshot(
            store,
            run,
            snapshot.snapshot(),
            "unsupported runtime projection schema",
        );
        return project_complete_history(store, run);
    }
    let payload: ProjectionSnapshotPayload = match serde_json::from_slice(snapshot.payload()) {
        Ok(payload) => payload,
        Err(error) => {
            warn!(run = %run, snapshot = %snapshot.snapshot(), reason = %error, "projection snapshot payload rejected");
            discard_optional_snapshot(
                store,
                run,
                snapshot.snapshot(),
                "projection snapshot payload is not valid JSON",
            );
            return project_complete_history(store, run);
        }
    };
    if payload.schema_version != RUN_PROJECTION_SNAPSHOT_SCHEMA_V3
        || payload.projection.run_id() != Some(run)
        || payload.projection.sequence() != snapshot.covered_sequence()
        || payload.projection.history_compacted_through() != snapshot.covered_sequence()
    {
        discard_optional_snapshot(
            store,
            run,
            snapshot.snapshot(),
            "projection snapshot identity or compaction boundary is inconsistent",
        );
        return project_complete_history(store, run);
    }
    if let Err(error) = payload.projection.validate_compacted_state() {
        warn!(run = %run, snapshot = %snapshot.snapshot(), reason = %error, "projection snapshot liveness references rejected");
        discard_optional_snapshot(
            store,
            run,
            snapshot.snapshot(),
            "projection snapshot contains invalid active references",
        );
        return project_complete_history(store, run);
    }

    fold_history_from(
        store,
        run,
        snapshot.covered_sequence(),
        payload.projection,
        |projection, event| projection.apply_replayed(event),
    )
}

pub(crate) fn encode_projection_snapshot(
    projection: &RunProjection,
) -> Result<Vec<u8>, RuntimeError> {
    if projection.history_compacted_through() != projection.sequence() {
        return Err(RuntimeError::InvalidHistory(
            "projection snapshot requires a live-compacted projection".to_owned(),
        ));
    }
    projection.validate_compacted_state()?;
    Ok(serde_json::to_vec(&ProjectionSnapshotPayloadRef {
        schema_version: RUN_PROJECTION_SNAPSHOT_SCHEMA_V3,
        projection,
    })?)
}

/// Materializes at most `maximum` events for diagnostics and bounded tests.
///
/// Runtime decision paths must use a projection fold. This helper never grows beyond
/// the caller's validated [`PageSize`] bound and returns an explicit bounds error.
pub(crate) fn load_bounded_history<S: RunQueryStore + ?Sized>(
    store: &S,
    run: &RunId,
    maximum: PageSize,
) -> Result<Vec<RunEventEnvelope>, RuntimeError> {
    fold_complete_history(
        store,
        run,
        Vec::with_capacity(maximum.get() as usize),
        |events, event| {
            if events.len() == maximum.get() as usize {
                return Err(milkdrift_persistence::PersistenceError::Bounds {
                    location: "runtime.bounded_history",
                    reason: format!(
                        "history exceeds the explicit diagnostic bound {}",
                        maximum.get()
                    ),
                }
                .into());
            }
            events.push(event.clone());
            Ok(())
        },
    )
}

#[cfg(test)]
mod tests {
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

    impl RunQueryStore for PagedStore {
        fn events(&self, query: &EventPageQuery) -> Result<EventPage, PersistenceError> {
            self.largest_page_request
                .fetch_max(query.limit.get(), Ordering::SeqCst);
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
            let limit =
                usize::try_from(query.limit.get()).map_err(|_| PersistenceError::Bounds {
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

        fn nonterminal_runs(
            &self,
            _limit: PageSize,
        ) -> Result<Vec<RunSummaryIndex>, PersistenceError> {
            Ok(Vec::new())
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

    fn snapshot_history_fixture(
        name: &str,
    ) -> Result<(RunId, Vec<RunEventEnvelope>), Box<dyn Error>> {
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
            RUN_PROJECTION_SNAPSHOT_SCHEMA_V3,
            encode_projection_snapshot(&prefix)?,
        )?;
        let store = PagedStore {
            run: run.clone(),
            events,
            largest_page_request: AtomicU32::new(0),
            page_reads: AtomicUsize::new(0),
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
            RUN_PROJECTION_SNAPSHOT_SCHEMA_V3,
            encode_projection_snapshot(&prefix)?,
        )?;
        let store = PagedStore {
            run: run.clone(),
            events,
            largest_page_request: AtomicU32::new(0),
            page_reads: AtomicUsize::new(0),
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
            RUN_PROJECTION_SNAPSHOT_SCHEMA_V3,
            encode_projection_snapshot(&complete)?,
        )?;
        let store = PagedStore {
            run: run.clone(),
            events,
            largest_page_request: AtomicU32::new(0),
            page_reads: AtomicUsize::new(0),
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
        assert_eq!(store.page_reads.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[test]
    fn invalid_optional_snapshot_never_blocks_authoritative_replay_when_cleanup_fails()
    -> Result<(), Box<dyn Error>> {
        let (run, events) = snapshot_history_fixture("snapshot-fallback")?;
        let mut prefix = RunProjection::new();
        prefix.apply_replayed(&events[0])?;
        let snapshot = SnapshotDocument::new(
            SnapshotId::new("snapshot-query-fallback")?,
            run.clone(),
            RunSequence::FIRST,
            history_digest(&events[..1])?,
            999,
            encode_projection_snapshot(&prefix)?,
        )?;
        let store = PagedStore {
            run: run.clone(),
            events,
            largest_page_request: AtomicU32::new(0),
            page_reads: AtomicUsize::new(0),
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
}
