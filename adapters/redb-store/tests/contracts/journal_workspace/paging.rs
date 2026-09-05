use super::*;

#[test]
fn runnable_page_is_bounded_by_distinct_runs_not_noisy_run_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let noisy_run = RunId::new("run-noisy")?;
    let quiet_run = RunId::new("run-quiet")?;
    let noisy_entries = (0_u16..64)
        .map(|priority| {
            Ok(RunnableIndexEntry {
                run: noisy_run.clone(),
                execution: NodeExecutionId::new(format!("noisy-execution-{priority:02}"))?,
                eligible_at: TimestampMillis::new(1),
                priority,
                through_sequence: RunSequence::FIRST,
            })
        })
        .collect::<Result<Vec<_>, PersistenceError>>()?;
    store.commit_command(&accepted_request_with_runnable(
        noisy_run.as_str(),
        "command-noisy",
        "event-noisy",
        noisy_entries,
    )?)?;
    store.commit_command(&accepted_request_with_runnable(
        quiet_run.as_str(),
        "command-quiet",
        "event-quiet",
        vec![RunnableIndexEntry {
            run: quiet_run.clone(),
            execution: NodeExecutionId::new("quiet-execution")?,
            eligible_at: TimestampMillis::new(2),
            priority: 1,
            through_sequence: RunSequence::FIRST,
        }],
    )?)?;

    // A raw timestamp-ordered page of this size contains only noisy-run rows.
    // The query contract instead walks the grouped identity index and returns one
    // best candidate for each distinct run represented by the page bound.
    let mut runnable = Vec::new();
    let mut cursor = None;
    for _ in 0..16 {
        let page =
            store.runnable_page(TimestampMillis::new(10), cursor.as_ref(), PageSize::new(2)?)?;
        runnable.extend(page.entries);
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(runnable.len(), 2);
    assert!(runnable.iter().any(|entry| entry.run == quiet_run));
    let selected_noisy = runnable
        .iter()
        .find(|entry| entry.run == noisy_run)
        .ok_or("no noisy-run candidate was returned")?;
    assert_eq!(selected_noisy.priority, 63);
    assert_eq!(selected_noisy.execution.as_str(), "noisy-execution-63");

    // Even a one-item scheduler budget progresses to another run on the next
    // bounded discovery call instead of restarting at the noisy run forever.
    let mut one_at_a_time = Vec::new();
    let mut cursor = None;
    for _ in 0..16 {
        let page =
            store.runnable_page(TimestampMillis::new(10), cursor.as_ref(), PageSize::new(1)?)?;
        one_at_a_time.extend(page.entries);
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(one_at_a_time.len(), 2);
    assert_ne!(one_at_a_time[0].run, one_at_a_time[1].run);
    assert!(one_at_a_time.iter().any(|entry| entry.run == quiet_run));
    Ok(())
}

#[test]
fn runnable_pages_advance_across_future_rows_and_removed_anchors()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    for ordinal in 0..10 {
        let run = RunId::new(format!("run-future-{ordinal:02}"))?;
        store.commit_command(&accepted_request_with_runnable(
            run.as_str(),
            &format!("command-future-{ordinal:02}"),
            &format!("event-future-{ordinal:02}"),
            vec![RunnableIndexEntry {
                run: run.clone(),
                execution: NodeExecutionId::new(format!("execution-future-{ordinal:02}"))?,
                eligible_at: TimestampMillis::new(1_000),
                priority: 1,
                through_sequence: RunSequence::FIRST,
            }],
        )?)?;
    }
    let eligible_run = RunId::new("run-later-eligible")?;
    store.commit_command(&accepted_request_with_runnable(
        eligible_run.as_str(),
        "command-later-eligible",
        "event-later-eligible",
        vec![RunnableIndexEntry {
            run: eligible_run.clone(),
            execution: NodeExecutionId::new("execution-later-eligible")?,
            eligible_at: TimestampMillis::new(1),
            priority: 1,
            through_sequence: RunSequence::FIRST,
        }],
    )?)?;

    let first = store.runnable_page(TimestampMillis::new(10), None, PageSize::new(1)?)?;
    assert!(first.entries.is_empty());
    assert!(first.next.is_some());
    let second = store.runnable_page(
        TimestampMillis::new(20),
        first.next.as_ref(),
        PageSize::new(1)?,
    )?;
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.entries[0].run, eligible_run);

    let anchor_directory = TempDir::new()?;
    let anchor_store = RedbStore::open(anchor_directory.path())?;
    let first_run = RunId::new("run-anchor-a")?;
    let second_run = RunId::new("run-anchor-b")?;
    let first_execution = NodeExecutionId::new("execution-anchor-a")?;
    let second_execution = NodeExecutionId::new("execution-anchor-b")?;
    for (run, execution, suffix) in [
        (&first_run, &first_execution, "a"),
        (&second_run, &second_execution, "b"),
    ] {
        anchor_store.commit_command(&accepted_request_with_runnable(
            run.as_str(),
            &format!("command-anchor-{suffix}"),
            &format!("event-anchor-{suffix}"),
            vec![RunnableIndexEntry {
                run: run.clone(),
                execution: execution.clone(),
                eligible_at: TimestampMillis::new(1),
                priority: 1,
                through_sequence: RunSequence::FIRST,
            }],
        )?)?;
    }
    let first_page =
        anchor_store.runnable_page(TimestampMillis::new(10), None, PageSize::new(1)?)?;
    assert_eq!(first_page.entries.len(), 1);
    assert_eq!(first_page.entries[0].run, first_run);
    let removal = accepted_followup_request(
        first_run.clone(),
        "command-remove-anchor",
        "event-remove-anchor",
    )?;
    let mut runnable = removal.indexes().runnable().to_vec();
    runnable.push(RunnableIndexMutation::Remove {
        run: first_run,
        execution: first_execution,
    });
    let removal = rebuild_request_with_indexes(
        &removal,
        RunIndexUpdate::new(
            removal.indexes().summary().cloned(),
            runnable,
            removal.indexes().timers().to_vec(),
            removal.indexes().leases().to_vec(),
        ),
    )?;
    anchor_store.commit_command(&removal)?;
    let second_page = anchor_store.runnable_page(
        TimestampMillis::new(20),
        first_page.next.as_ref(),
        PageSize::new(1)?,
    )?;
    assert_eq!(second_page.entries.len(), 1);
    assert_eq!(second_page.entries[0].run, second_run);
    assert!(second_page.next.is_none());
    Ok(())
}

#[test]
fn nonterminal_run_pages_resume_past_early_runs() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    for ordinal in 0..5 {
        store.commit_command(&accepted_request(
            &format!("run-page-{ordinal}"),
            &format!("command-page-{ordinal}"),
            &format!("event-page-{ordinal}"),
            "start",
        )?)?;
    }

    let first = store.nonterminal_run_page(None, PageSize::new(2)?)?;
    assert_eq!(first.runs.len(), 2);
    let second = store.nonterminal_run_page(first.next.as_ref(), PageSize::new(2)?)?;
    assert_eq!(second.runs.len(), 2);
    let third = store.nonterminal_run_page(second.next.as_ref(), PageSize::new(2)?)?;
    assert_eq!(third.runs.len(), 1);
    assert!(third.next.is_none());

    let discovered = first
        .runs
        .iter()
        .chain(&second.runs)
        .chain(&third.runs)
        .map(|summary| summary.run.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(discovered.len(), 5);
    for ordinal in 0..5 {
        assert!(discovered.contains(format!("run-page-{ordinal}").as_str()));
    }
    Ok(())
}

#[test]
fn summary_and_nonterminal_cursors_advance_by_last_scanned_head()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    for ordinal in 0..10 {
        let request = accepted_request(
            &format!("run-filter-{ordinal:02}"),
            &format!("command-filter-{ordinal:02}"),
            &format!("event-filter-{ordinal:02}"),
            "start",
        )?;
        let mut summary = request
            .indexes()
            .summary()
            .cloned()
            .ok_or("summary missing")?;
        summary.workflow = WorkflowId::new("workflow-nonmatch")?;
        let request = rebuild_request_with_indexes(
            &request,
            RunIndexUpdate::new(
                Some(summary),
                request.indexes().runnable().to_vec(),
                request.indexes().timers().to_vec(),
                request.indexes().leases().to_vec(),
            ),
        )?;
        store.commit_command(&request)?;
    }
    let matching_run = RunId::new("run-filter-z-match")?;
    let matching = accepted_request(
        matching_run.as_str(),
        "command-filter-match",
        "event-filter-match",
        "start",
    )?;
    let mut summary = matching
        .indexes()
        .summary()
        .cloned()
        .ok_or("summary missing")?;
    summary.workflow = WorkflowId::new("workflow-match")?;
    let matching = rebuild_request_with_indexes(
        &matching,
        RunIndexUpdate::new(
            Some(summary),
            matching.indexes().runnable().to_vec(),
            matching.indexes().timers().to_vec(),
            matching.indexes().leases().to_vec(),
        ),
    )?;
    store.commit_command(&matching)?;

    let empty_filter = RunSummaryFilter {
        state: None,
        workflow: Some(WorkflowId::new("workflow-empty")?),
    };
    let first = store.run_summaries(&RunSummaryPageQuery {
        filter: empty_filter.clone(),
        cursor: None,
        limit: PageSize::new(1)?,
    })?;
    assert!(first.runs.is_empty());
    assert!(first.next.is_some());
    assert!(matches!(
        store.run_summaries(&RunSummaryPageQuery {
            filter: RunSummaryFilter {
                state: None,
                workflow: Some(WorkflowId::new("workflow-other")?),
            },
            cursor: first.next,
            limit: PageSize::new(1)?,
        }),
        Err(PersistenceError::InvalidCursor(_))
    ));

    let filter = RunSummaryFilter {
        state: None,
        workflow: Some(WorkflowId::new("workflow-match")?),
    };
    let mut cursor = None;
    let mut matches = Vec::new();
    loop {
        let page = store.run_summaries(&RunSummaryPageQuery {
            filter: filter.clone(),
            cursor,
            limit: PageSize::new(1)?,
        })?;
        matches.extend(page.runs);
        let Some(next) = page.next else {
            break;
        };
        cursor = Some(next);
    }
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].run, matching_run);

    let terminal_directory = TempDir::new()?;
    let terminal_store = RedbStore::open(terminal_directory.path())?;
    for ordinal in 0..3 {
        let request = accepted_request(
            &format!("run-terminal-{ordinal}"),
            &format!("command-terminal-{ordinal}"),
            &format!("event-terminal-{ordinal}"),
            "start",
        )?;
        let mut summary = request
            .indexes()
            .summary()
            .cloned()
            .ok_or("summary missing")?;
        summary.state = IndexedRunState::Terminal;
        let request = rebuild_request_with_indexes(
            &request,
            RunIndexUpdate::new(
                Some(summary),
                request.indexes().runnable().to_vec(),
                request.indexes().timers().to_vec(),
                request.indexes().leases().to_vec(),
            ),
        )?;
        terminal_store.commit_command(&request)?;
    }
    terminal_store.commit_command(&accepted_request(
        "run-terminal-z-active",
        "command-terminal-active",
        "event-terminal-active",
        "start",
    )?)?;
    let terminal_page = terminal_store.nonterminal_run_page(None, PageSize::new(2)?)?;
    assert_eq!(terminal_page.runs.len(), 1);
    assert_eq!(terminal_page.runs[0].run.as_str(), "run-terminal-z-active");
    assert!(terminal_page.next.is_none());
    Ok(())
}

#[test]
fn active_lease_query_is_a_bounded_complete_prefix() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    for ordinal in 0..3 {
        let run = RunId::new(format!("run-lease-{ordinal}"))?;
        store.commit_command(&accepted_request_with_lease(
            run.as_str(),
            &format!("command-lease-{ordinal}"),
            &format!("event-lease-{ordinal}"),
            LeaseIndexEntry {
                run: run.clone(),
                lease: LeaseId::new(format!("lease-{ordinal}"))?,
                attempt: AttemptId::new(format!("attempt-{ordinal}"))?,
                worker: WorkerId::new("worker-test")?,
                expires_at: TimestampMillis::new(100 + ordinal),
                through_sequence: RunSequence::FIRST,
            },
        )?)?;
    }

    assert_eq!(store.active_leases(PageSize::new(2)?)?.entries.len(), 2);
    assert_eq!(store.active_leases(PageSize::new(4)?)?.entries.len(), 3);
    Ok(())
}
