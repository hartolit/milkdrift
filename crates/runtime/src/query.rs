use milkdrift_persistence::{
    EventPageQuery, MAX_PAGE_SIZE, PageSize, RunEventEnvelope, RunQueryStore, RunSequence,
};
use milkdrift_workspace::RunId;

use crate::{RunProjection, RuntimeError};

/// Folds authoritative history through fixed-size pages without retaining prior pages.
///
/// The store verifies each page. This helper additionally pins the observed head for
/// the duration of the logical read, proves every continuation cursor advances exactly
/// beyond the page tail, and proves the fold reached that head. The accumulator is the
/// only state whose size may grow with history; runtime callers use a projection or a
/// bounded scalar/set rather than an event vector.
pub(crate) fn fold_complete_history<S, T, F>(
    store: &S,
    run: &RunId,
    mut accumulator: T,
    mut fold: F,
) -> Result<T, RuntimeError>
where
    S: RunQueryStore + ?Sized,
    F: FnMut(&mut T, &RunEventEnvelope) -> Result<(), RuntimeError>,
{
    let limit = PageSize::new(MAX_PAGE_SIZE)?;
    let mut cursor = None;
    let mut observed_head = None;
    let mut folded_through = RunSequence::ZERO;
    loop {
        let page = store.events(&EventPageQuery::new(run.clone(), cursor.clone(), limit)?)?;
        if let Some(previous) = observed_head {
            if previous != page.observed_head {
                return Err(RuntimeError::InvalidHistory(
                    "journal head changed during a complete-history fold; retry from sequence one"
                        .to_owned(),
                ));
            }
        }
        observed_head = Some(page.observed_head);
        let page_was_empty = page.events.is_empty();
        for event in &page.events {
            fold(&mut accumulator, event)?;
            folded_through = event.sequence();
        }
        match page.next {
            Some(next) => {
                if page_was_empty || next.next_sequence != folded_through.next()? {
                    return Err(RuntimeError::InvalidHistory(
                        "event pagination cursor did not advance beyond the page tail".to_owned(),
                    ));
                }
                cursor = Some(next);
            }
            None => break,
        }
    }
    let observed = observed_head.unwrap_or(RunSequence::ZERO);
    if folded_through != observed {
        return Err(RuntimeError::InvalidHistory(format!(
            "folded history ended at {folded_through}, but storage observed head {observed}"
        )));
    }
    Ok(accumulator)
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

/// Materializes at most `maximum` events for diagnostics and bounded tests.
///
/// Runtime decision paths must use [`fold_complete_history`] or
/// [`project_complete_history`]. This helper never grows beyond the caller's validated
/// [`PageSize`] bound and returns an explicit bounds error instead of truncating.
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

/// Reads a complete authoritative history through bounded resumable pages.
///
/// Every page is verified by persistence. This helper additionally checks that the
/// cursor advances and the assembled result reaches the read transaction's observed
/// head; malformed or changing history is never interpreted as an empty run.
pub fn load_complete_history<S: RunQueryStore + ?Sized>(
    store: &S,
    run: &RunId,
) -> Result<Vec<RunEventEnvelope>, RuntimeError> {
    let limit = PageSize::new(1_000)?;
    let mut cursor = None;
    let mut events = Vec::new();
    let mut last_observed_head = None;
    loop {
        let page = store.events(&EventPageQuery::new(run.clone(), cursor.clone(), limit)?)?;
        if let Some(previous) = last_observed_head {
            if previous != page.observed_head {
                return Err(RuntimeError::InvalidHistory(
                    "journal head changed during a complete-history read; retry from sequence one"
                        .to_owned(),
                ));
            }
        }
        last_observed_head = Some(page.observed_head);
        let previous_len = events.len();
        events.extend(page.events);
        match page.next {
            Some(next) => {
                if events.len() == previous_len
                    || cursor
                        .as_ref()
                        .is_some_and(|prior: &milkdrift_persistence::EventCursor| {
                            next.next_sequence <= prior.next_sequence
                        })
                {
                    return Err(RuntimeError::InvalidHistory(
                        "event pagination cursor did not advance".to_owned(),
                    ));
                }
                cursor = Some(next);
            }
            None => break,
        }
    }
    let observed = last_observed_head.unwrap_or(RunSequence::ZERO);
    let assembled = events
        .last()
        .map_or(RunSequence::ZERO, RunEventEnvelope::sequence);
    if assembled != observed {
        return Err(RuntimeError::InvalidHistory(format!(
            "assembled history ended at {assembled}, but storage observed head {observed}"
        )));
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    use milkdrift_persistence::{
        EventCursor, EventPage, LeaseIndexEntry, PersistenceError, RunEventKind, RunSummaryIndex,
        RunSummaryPage, RunSummaryPageQuery, TimerIndexEntry, TimestampMillis,
    };

    use super::*;

    struct PagedStore {
        run: RunId,
        events: Vec<RunEventEnvelope>,
        largest_page_request: AtomicU32,
        page_reads: AtomicUsize,
    }

    impl RunQueryStore for PagedStore {
        fn events(&self, query: &EventPageQuery) -> Result<EventPage, PersistenceError> {
            if query.run != self.run {
                return Err(PersistenceError::InvalidCursor(
                    "test query used another run".to_owned(),
                ));
            }
            self.largest_page_request
                .fetch_max(query.limit.get(), Ordering::SeqCst);
            self.page_reads.fetch_add(1, Ordering::SeqCst);
            let start = query
                .cursor
                .as_ref()
                .map_or(0, |cursor| cursor.next_sequence.get().saturating_sub(1));
            let start = usize::try_from(start).map_err(|_| PersistenceError::Bounds {
                location: "test.event_cursor",
                reason: "cursor exceeds usize".to_owned(),
            })?;
            let end = start
                .saturating_add(query.limit.get() as usize)
                .min(self.events.len());
            let next = (end < self.events.len()).then(|| EventCursor {
                run: self.run.clone(),
                next_sequence: RunSequence::new(end as u64 + 1),
            });
            Ok(EventPage {
                events: self.events[start..end].to_vec(),
                next,
                observed_head: RunSequence::new(self.events.len() as u64),
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

        fn active_leases(
            &self,
            _limit: PageSize,
        ) -> Result<Vec<LeaseIndexEntry>, PersistenceError> {
            Ok(Vec::new())
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
}
