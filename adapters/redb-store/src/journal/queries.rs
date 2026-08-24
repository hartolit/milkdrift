use super::*;
use super::{
    append::{validate_nonterminal_membership, validate_run_history_membership},
    discovery::{lease_set_revision, read_ordered_index, validate_runnable_head},
};

impl RunQueryStore for RedbStore {
    fn events(&self, query: &EventPageQuery) -> Result<EventPage, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events_table = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let observed_head = validated_run_head(&heads, &events_table, &query.run)?;
        validate_run_history_membership(&read, &query.run, observed_head)?;
        let Some(mut next_sequence) = query.start_sequence(observed_head)? else {
            return Ok(EventPage {
                events: Vec::new(),
                next: None,
                observed_head,
            });
        };
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
            let event = decode_stored_event(bytes.value())?;
            if event.run_id() != &query.run || event.sequence() != next_sequence {
                return Err(error::corruption(
                    "stored event key does not match its envelope",
                ));
            }
            crate::snapshot::validate_history_link(&read, &event)?;
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
        validate_nonterminal_membership(&read, &summary)?;
        Ok(Some(summary))
    }

    fn run_summaries(
        &self,
        query: &RunSummaryPageQuery,
    ) -> Result<RunSummaryPage, PersistenceError> {
        const MIN_SUMMARY_SCAN_ROWS: usize = 8;
        let read = self.database().begin_read().map_err(error::redb)?;
        let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        let after = if let Some(cursor) = &query.cursor {
            if !cursor.matches_query(&query.filter) {
                return Err(PersistenceError::InvalidCursor(
                    "run-summary cursor belongs to a different filter".to_owned(),
                ));
            }
            if summaries
                .get(cursor.after_run().as_str())
                .map_err(error::redb)?
                .is_none()
            {
                return Err(PersistenceError::InvalidCursor(
                    "run-summary cursor does not name a durable summary".to_owned(),
                ));
            }
            Bound::Excluded(cursor.after_run().as_str())
        } else {
            Bound::Unbounded
        };
        let page_limit = page_size_usize(query.limit)?;
        let scan_budget = page_limit.max(MIN_SUMMARY_SCAN_ROWS);
        let mut rows = summaries
            .range::<&str>((after, Bound::Unbounded))
            .map_err(error::redb)?;
        let mut runs = Vec::with_capacity(page_limit);
        let mut last_scanned = None;
        let mut processed = 0_usize;
        while processed < scan_budget && runs.len() < page_limit {
            let Some(row) = rows.next() else {
                break;
            };
            let (key, bytes) = row.map_err(error::redb)?;
            let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
            if summary.run.as_str() != key.value() {
                return Err(error::corruption(
                    "run-summary key does not match its document",
                ));
            }
            let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
            let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
            validate_summary_head(&heads, &events, &summary)?;
            validate_run_history_membership(&read, &summary.run, summary.through_sequence)?;
            validate_nonterminal_membership(&read, &summary)?;
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
        }
        let has_more = rows.next().transpose().map_err(error::redb)?.is_some();
        let next = if has_more {
            Some(milkdrift_persistence::RunSummaryCursor::for_query(
                last_scanned.ok_or_else(|| {
                    error::corruption("advancing summary page lost its scan cursor")
                })?,
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
        let nonterminal = read.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
        let lower = if let Some(cursor) = cursor {
            if !cursor.is_nonterminal() {
                return Err(PersistenceError::InvalidCursor(
                    "run-summary cursor does not belong to nonterminal discovery".to_owned(),
                ));
            }
            if nonterminal
                .get(cursor.after_run().as_str())
                .map_err(error::redb)?
                .is_none()
            {
                return Err(PersistenceError::InvalidCursor(
                    "nonterminal cursor does not name a durable marker".to_owned(),
                ));
            }
            Bound::Excluded(cursor.after_run().as_str())
        } else {
            Bound::Unbounded
        };
        let page_limit = page_size_usize(limit)?;
        let mut rows = nonterminal
            .range::<&str>((lower, Bound::Unbounded))
            .map_err(error::redb)?;
        let mut results = Vec::with_capacity(page_limit);
        let mut last_scanned = None;
        while results.len() < page_limit {
            let Some(row) = rows.next() else {
                break;
            };
            let (run_key, marker) = row.map_err(error::redb)?;
            if marker.value() != 1 {
                return Err(error::corruption(
                    "nonterminal discovery contains an invalid marker",
                ));
            }
            let run = RunId::new(run_key.value()).map_err(|cause| {
                error::corruption(format!("invalid nonterminal run identity: {cause}"))
            })?;
            let summary = load_checked_summary(&read, &run)?;
            if summary.state == IndexedRunState::Terminal {
                return Err(error::corruption(
                    "terminal run remains in nonterminal discovery",
                ));
            }
            last_scanned = Some(run);
            results.push(summary);
        }
        let has_more = rows.next().transpose().map_err(error::redb)?.is_some();
        let next = if has_more {
            Some(milkdrift_persistence::RunSummaryCursor::for_nonterminal(
                last_scanned.ok_or_else(|| {
                    error::corruption("advancing nonterminal page lost its scan cursor")
                })?,
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
        let heads = read.open_table(RUNNABLE_RUN_HEADS).map_err(error::redb)?;
        let scan_through = cursor.map_or(eligible_through, RunnableCursor::eligible_through);
        let lower = cursor.map_or(Bound::Unbounded, |cursor| {
            Bound::Excluded(cursor.after_run().as_str())
        });
        let page_limit = page_size_usize(limit)?;
        let mut rows = heads
            .range::<&str>((lower, Bound::Unbounded))
            .map_err(error::redb)?;
        let mut results = Vec::with_capacity(page_limit);
        let mut last_scanned = None;
        let mut scanned = 0_usize;
        while scanned < page_limit {
            let Some(row) = rows.next() else {
                break;
            };
            let (run, bytes) = row.map_err(error::redb)?;
            let head = validate_runnable_head(&read, run.value(), bytes.value())?;
            last_scanned = Some(head.run.clone());
            if head.eligible_at <= scan_through {
                results.push(head);
            }
            scanned += 1;
        }
        let has_more = rows.next().transpose().map_err(error::redb)?.is_some();
        let next = if has_more {
            Some(RunnableCursor::new(
                last_scanned.ok_or_else(|| {
                    error::corruption("advancing runnable page lost its run cursor")
                })?,
                scan_through,
            ))
        } else {
            None
        };
        Ok(RunnablePage {
            entries: results,
            next,
        })
    }

    fn active_leases(&self, limit: PageSize) -> Result<ActiveLeaseSnapshot, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let entries = read_ordered_index(
            &read,
            LEASE_ENTRIES,
            LEASE_INDEX,
            TimestampMillis::new(u64::MAX),
            limit,
            "lease index",
            |entry: &LeaseIndexEntry| entry.expires_at,
            lease_order_key,
            |entry: &LeaseIndexEntry| codec::pair(entry.run.as_str(), entry.lease.as_str()),
        )?;
        Ok(ActiveLeaseSnapshot {
            entries,
            revision: lease_set_revision(&read)?,
        })
    }

    fn due_timers(
        &self,
        due_through: TimestampMillis,
        limit: PageSize,
    ) -> Result<Vec<TimerIndexEntry>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        read_ordered_index(
            &read,
            TIMER_ENTRIES,
            TIMER_INDEX,
            due_through,
            limit,
            "timer index",
            |entry: &TimerIndexEntry| entry.fire_at,
            timer_order_key,
            |entry: &TimerIndexEntry| codec::pair(entry.run.as_str(), entry.timer.as_str()),
        )
    }

    fn expired_leases(
        &self,
        expired_through: TimestampMillis,
        limit: PageSize,
    ) -> Result<Vec<LeaseIndexEntry>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        read_ordered_index(
            &read,
            LEASE_ENTRIES,
            LEASE_INDEX,
            expired_through,
            limit,
            "lease index",
            |entry: &LeaseIndexEntry| entry.expires_at,
            lease_order_key,
            |entry: &LeaseIndexEntry| codec::pair(entry.run.as_str(), entry.lease.as_str()),
        )
    }
}

fn load_checked_summary(
    read: &redb::ReadTransaction,
    run: &RunId,
) -> Result<RunSummaryIndex, PersistenceError> {
    let summaries = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    let bytes = summaries
        .get(run.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("nonterminal marker names a missing summary"))?;
    let summary: RunSummaryIndex = json::decode(bytes.value(), "run summary")?;
    if summary.run != *run {
        return Err(error::corruption(
            "run-summary key does not match its document",
        ));
    }
    let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    validate_summary_head(&heads, &events, &summary)?;
    validate_run_history_membership(read, run, summary.through_sequence)?;
    validate_nonterminal_membership(read, &summary)?;
    Ok(summary)
}

pub(crate) fn page_size_usize(limit: PageSize) -> Result<usize, PersistenceError> {
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

pub(crate) fn validate_summary_head<H, E>(
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
