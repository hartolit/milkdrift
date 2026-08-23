use super::*;
use super::{
    append::{
        nonterminal_membership_path, validate_event_catalog, validate_nonterminal_membership,
        validate_nonterminal_membership_leaf, validate_run_cursor_anchor,
        validate_run_membership_leaf,
    },
    discovery::{read_ordered_index, runnable_head_path, validate_runnable_head_leaf},
};
impl RunQueryStore for RedbStore {
    fn events(&self, query: &EventPageQuery) -> Result<EventPage, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
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
