use std::sync::Mutex;

use milkdrift_persistence::{
    ActiveLeaseSnapshot, EventCursor, EventPage, IntegrityDigest, RunSummaryCursor, RunSummaryPage,
    RunnableCursor, RunnablePage,
};

use super::*;

type ContractResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct MemoryEventStore {
    run: RunId,
    events: Mutex<Vec<RunEventEnvelope>>,
}

impl MemoryEventStore {
    fn new(run: RunId, events: Vec<RunEventEnvelope>) -> Self {
        Self {
            run,
            events: Mutex::new(events),
        }
    }

    fn append(&self, event: RunEventEnvelope) -> Result<(), PersistenceError> {
        let mut events = self.events.lock().map_err(|_| {
            PersistenceError::InvalidDocument("memory event-store lock is poisoned".to_owned())
        })?;
        let expected = RunSequence::new(
            u64::try_from(events.len())
                .map_err(|_| PersistenceError::Bounds {
                    location: "event_page_conformance.memory_history",
                    reason: "event history length exceeds u64".to_owned(),
                })?
                .checked_add(1)
                .ok_or(PersistenceError::SequenceOverflow)?,
        );
        if event.run_id() != &self.run || event.sequence() != expected {
            return Err(PersistenceError::InvalidDocument(
                "memory event-store append must be contiguous for its configured run".to_owned(),
            ));
        }
        events.push(event);
        Ok(())
    }
}

impl RunQueryStore for MemoryEventStore {
    fn events(&self, query: &EventPageQuery) -> Result<EventPage, PersistenceError> {
        let events = self.events.lock().map_err(|_| {
            PersistenceError::InvalidDocument("memory event-store lock is poisoned".to_owned())
        })?;
        let history_len = if query.run == self.run {
            events.len()
        } else {
            0
        };
        let observed_head =
            RunSequence::new(
                u64::try_from(history_len).map_err(|_| PersistenceError::Bounds {
                    location: "event_page_conformance.memory_history",
                    reason: "event history length exceeds u64".to_owned(),
                })?,
            );
        let Some(next_sequence) = query.start_sequence(observed_head)? else {
            return Ok(EventPage {
                events: Vec::new(),
                next: None,
                observed_head,
            });
        };
        let start = usize::try_from(next_sequence.get().checked_sub(1).ok_or_else(|| {
            PersistenceError::InvalidCursor("event cursor named sequence zero".to_owned())
        })?)
        .map_err(|_| PersistenceError::Bounds {
            location: "event_page_conformance.memory_cursor",
            reason: "event cursor exceeds usize".to_owned(),
        })?;
        let limit = usize::try_from(query.limit.get()).map_err(|_| PersistenceError::Bounds {
            location: "event_page_conformance.memory_page_size",
            reason: "event page size exceeds usize".to_owned(),
        })?;
        let end = start
            .checked_add(limit)
            .ok_or_else(|| PersistenceError::Bounds {
                location: "event_page_conformance.memory_page",
                reason: "event page end overflowed usize".to_owned(),
            })?
            .min(history_len);
        let page_events = events.get(start..end).ok_or_else(|| {
            PersistenceError::Corruption(
                "memory event page range exceeds authoritative history".to_owned(),
            )
        })?;
        let mut expected = next_sequence;
        for (index, event) in page_events.iter().enumerate() {
            if event.run_id() != &query.run || event.sequence() != expected {
                return Err(PersistenceError::Corruption(
                    "memory event history is not contiguous for the queried run".to_owned(),
                ));
            }
            if index + 1 < page_events.len() {
                expected = expected.next()?;
            }
        }
        let next = if end < history_len {
            Some(EventCursor {
                run: query.run.clone(),
                next_sequence: page_events
                    .last()
                    .ok_or_else(|| {
                        PersistenceError::Corruption(
                            "non-terminal memory event page was empty".to_owned(),
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
        _cursor: Option<&RunSummaryCursor>,
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
        _cursor: Option<&RunnableCursor>,
        _limit: PageSize,
    ) -> Result<RunnablePage, PersistenceError> {
        Ok(RunnablePage {
            entries: Vec::new(),
            next: None,
        })
    }

    fn active_leases(&self, _limit: PageSize) -> Result<ActiveLeaseSnapshot, PersistenceError> {
        Ok(ActiveLeaseSnapshot {
            entries: Vec::new(),
            witness: IntegrityDigest::hash(b"empty memory event-store lease catalog"),
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

fn cursor(run: &RunId, sequence: u64) -> EventCursor {
    EventCursor {
        run: run.clone(),
        next_sequence: RunSequence::new(sequence),
    }
}

fn event_sequences(page: &EventPage) -> Vec<u64> {
    page.events
        .iter()
        .map(|event| event.sequence().get())
        .collect()
}

fn assert_invalid_cursor(result: Result<EventPage, PersistenceError>) {
    assert!(
        matches!(result, Err(PersistenceError::InvalidCursor(_))),
        "expected invalid event cursor, got {result:?}"
    );
}

fn assert_event_page_contract<S, F>(store: &S, run: &RunId, append: F) -> ContractResult
where
    S: RunQueryStore + ?Sized,
    F: FnOnce() -> ContractResult,
{
    let page_size_one = PageSize::new(1)?;
    let page_size_many = PageSize::new(10)?;
    let absent_run = RunId::new(format!("{}-absent", run.as_str()))?;

    let absent = store.events(&EventPageQuery::new(
        absent_run.clone(),
        None,
        page_size_many,
    )?)?;
    assert!(absent.events.is_empty());
    assert!(absent.next.is_none());
    assert_eq!(absent.observed_head, RunSequence::ZERO);
    assert_invalid_cursor(store.events(&EventPageQuery::new(
        absent_run.clone(),
        Some(cursor(&absent_run, 1)),
        page_size_many,
    )?));

    let wrong_run = EventPageQuery {
        run: run.clone(),
        cursor: Some(cursor(&absent_run, 1)),
        limit: page_size_many,
    };
    assert_invalid_cursor(store.events(&wrong_run));

    let first = store.events(&EventPageQuery::new(run.clone(), None, page_size_one)?)?;
    assert_eq!(event_sequences(&first), vec![1]);
    assert_eq!(first.observed_head, RunSequence::new(2));
    assert_eq!(first.next, Some(cursor(run, 2)));

    let partial = store.events(&EventPageQuery::new(
        run.clone(),
        Some(cursor(run, 2)),
        page_size_many,
    )?)?;
    assert_eq!(event_sequences(&partial), vec![2]);
    assert!(partial.next.is_none());
    assert_eq!(partial.observed_head, RunSequence::new(2));

    let exact_eof = store.events(&EventPageQuery::new(
        run.clone(),
        Some(cursor(run, 3)),
        page_size_many,
    )?)?;
    assert!(exact_eof.events.is_empty());
    assert!(exact_eof.next.is_none());
    assert_eq!(exact_eof.observed_head, RunSequence::new(2));

    assert_invalid_cursor(store.events(&EventPageQuery::new(
        run.clone(),
        Some(cursor(run, 4)),
        page_size_many,
    )?));

    let mut continuation = None;
    let mut contiguous = Vec::new();
    for _ in 0..3 {
        let page = store.events(&EventPageQuery::new(
            run.clone(),
            continuation,
            page_size_one,
        )?)?;
        assert_eq!(page.observed_head, RunSequence::new(2));
        contiguous.extend(event_sequences(&page));
        continuation = page.next;
        if continuation.is_none() {
            break;
        }
    }
    assert_eq!(contiguous, vec![1, 2]);
    assert!(continuation.is_none());

    let first_pinned = store.events(&EventPageQuery::new(run.clone(), None, page_size_one)?)?;
    let pinned_head = first_pinned.observed_head;
    assert_eq!(pinned_head, RunSequence::new(2));
    append()?;

    let mut pinned_sequences = event_sequences(&first_pinned);
    let mut continuation = first_pinned.next;
    while pinned_sequences.last().copied().unwrap_or(0) < pinned_head.get() {
        let page = store.events(&EventPageQuery::new(
            run.clone(),
            continuation,
            page_size_one,
        )?)?;
        assert!(page.observed_head >= pinned_head);
        pinned_sequences.extend(
            page.events
                .iter()
                .take_while(|event| event.sequence() <= pinned_head)
                .map(|event| event.sequence().get()),
        );
        continuation = page.next;
    }
    assert_eq!(pinned_sequences, vec![1, 2]);

    let grown = store.events(&EventPageQuery::new(run.clone(), None, page_size_many)?)?;
    assert_eq!(event_sequences(&grown), vec![1, 2, 3]);
    assert_eq!(grown.observed_head, RunSequence::new(3));
    assert!(grown.next.is_none());
    Ok(())
}

#[test]
fn redb_event_pages_satisfy_the_shared_cursor_contract() -> ContractResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let first = accepted_request(
        "run-event-page-redb",
        "command-event-page-redb-1",
        "event-page-redb-1",
        "start",
    )?;
    let run = first.receipt().run().clone();
    let second = accepted_followup_request(
        run.clone(),
        "command-event-page-redb-2",
        "event-page-redb-2",
    )?;
    let third = accepted_sequenced_followup_request(
        run.clone(),
        "command-event-page-redb-3",
        "event-page-redb-3",
        RunSequence::new(2),
        RunSequence::new(3),
        TimestampMillis::new(12),
    )?;
    assert!(matches!(
        store.commit_command(&first)?,
        AtomicRunCommitOutcome::Committed(_)
    ));
    assert!(matches!(
        store.commit_command(&second)?,
        AtomicRunCommitOutcome::Committed(_)
    ));

    assert_event_page_contract(&store, &run, || {
        assert!(matches!(
            store.commit_command(&third)?,
            AtomicRunCommitOutcome::Committed(_)
        ));
        Ok(())
    })
}

#[test]
fn in_memory_event_pages_satisfy_the_shared_cursor_contract() -> ContractResult {
    let first = accepted_request(
        "run-event-page-memory",
        "command-event-page-memory-1",
        "event-page-memory-1",
        "start",
    )?;
    let run = first.receipt().run().clone();
    let second = accepted_followup_request(
        run.clone(),
        "command-event-page-memory-2",
        "event-page-memory-2",
    )?;
    let third = accepted_sequenced_followup_request(
        run.clone(),
        "command-event-page-memory-3",
        "event-page-memory-3",
        RunSequence::new(2),
        RunSequence::new(3),
        TimestampMillis::new(12),
    )?;
    let store = MemoryEventStore::new(
        run.clone(),
        vec![first.events()[0].clone(), second.events()[0].clone()],
    );
    let third_event = third.events()[0].clone();

    assert_event_page_contract(&store, &run, || {
        store.append(third_event)?;
        Ok(())
    })
}
