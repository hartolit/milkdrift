use super::*;
#[test]
fn journal_reopens_and_idempotency_conflicts_without_duplicate_events()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let request = accepted_request("run-idempotent", "command-one", "event-one", "start")?;
    {
        let store = RedbStore::open(directory.path())?;
        assert!(matches!(
            store.commit_command(&request)?,
            AtomicRunCommitOutcome::Committed(_)
        ));
        assert!(matches!(
            store.commit_command(&request)?,
            AtomicRunCommitOutcome::Replayed(_)
        ));
        let conflict =
            accepted_request("run-idempotent", "command-one", "event-other", "different")?;
        assert!(matches!(
            store.commit_command(&conflict),
            Err(PersistenceError::IdempotencyConflict { .. })
        ));
    }
    let store = RedbStore::open(directory.path())?;
    assert_eq!(store.head(request.receipt.run())?, RunSequence::FIRST);
    let page = store.events(&EventPageQuery::new(
        request.receipt.run().clone(),
        None,
        PageSize::new(10)?,
    )?)?;
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].event_id(), request.events[0].event_id());
    Ok(())
}

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
    let mut removal = accepted_followup_request(
        first_run.clone(),
        "command-remove-anchor",
        "event-remove-anchor",
    )?;
    removal
        .indexes
        .runnable
        .push(RunnableIndexMutation::Remove {
            run: first_run,
            execution: first_execution,
        });
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
        let mut request = accepted_request(
            &format!("run-filter-{ordinal:02}"),
            &format!("command-filter-{ordinal:02}"),
            &format!("event-filter-{ordinal:02}"),
            "start",
        )?;
        request
            .indexes
            .summary
            .as_mut()
            .ok_or("summary missing")?
            .workflow = WorkflowId::new("workflow-nonmatch")?;
        store.commit_command(&request)?;
    }
    let matching_run = RunId::new("run-filter-z-match")?;
    let mut matching = accepted_request(
        matching_run.as_str(),
        "command-filter-match",
        "event-filter-match",
        "start",
    )?;
    matching
        .indexes
        .summary
        .as_mut()
        .ok_or("summary missing")?
        .workflow = WorkflowId::new("workflow-match")?;
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
        let mut request = accepted_request(
            &format!("run-terminal-{ordinal}"),
            &format!("command-terminal-{ordinal}"),
            &format!("event-terminal-{ordinal}"),
            "start",
        )?;
        request
            .indexes
            .summary
            .as_mut()
            .ok_or("summary missing")?
            .state = IndexedRunState::Terminal;
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

#[test]
fn durable_workspace_imports_require_an_exact_cross_run_source_without_ancestry()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;

    let child_root =
        WorkspaceScope::run_root(RunId::new("child-import-run")?, ScopeId::new("child-root")?);
    let child_value = WorkspaceValueEntry::initial(
        child_root.reference().clone(),
        ValueKey::new("result")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"answer": 42}))?),
    );
    let child_budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let child_usage = child_budget.admit_value(&WorkspaceUsage::EMPTY, child_value.value())?;
    let child_request = accepted_workspace_request(
        child_root.reference().run().clone(),
        "command-child-import",
        "event-child-import",
        vec![run_created_kind(
            child_root.clone(),
            child_budget.clone(),
            vec![child_value.reference().clone()],
        )?],
        vec![
            WorkspaceMutation::CreateScope {
                scope: child_root.clone(),
            },
            WorkspaceMutation::PutValue {
                entry: child_value.clone(),
            },
        ],
        WorkspaceAccounting {
            budget: child_budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: child_usage,
        },
    )?;
    store.commit_command(&child_request)?;

    let parent_root = WorkspaceScope::run_root(
        RunId::new("parent-import-run")?,
        ScopeId::new("parent-root")?,
    );
    let imported = WorkspaceValueEntry::imported(
        parent_root.reference().clone(),
        ValueKey::new("child-result")?,
        child_value.reference().clone(),
        child_value.value().clone(),
    )?;
    let parent_budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let parent_usage = parent_budget.admit_value(&WorkspaceUsage::EMPTY, imported.value())?;
    let parent_request = accepted_workspace_request(
        parent_root.reference().run().clone(),
        "command-parent-import",
        "event-parent-import",
        vec![run_created_kind(
            parent_root.clone(),
            parent_budget.clone(),
            vec![imported.reference().clone()],
        )?],
        vec![
            WorkspaceMutation::CreateScope { scope: parent_root },
            WorkspaceMutation::PutValue {
                entry: imported.clone(),
            },
        ],
        WorkspaceAccounting {
            budget: parent_budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: parent_usage,
        },
    )?;
    store.commit_command(&parent_request)?;
    assert_eq!(store.value(imported.reference())?, Some(imported));

    let altered_root = WorkspaceScope::run_root(
        RunId::new("parent-altered-import")?,
        ScopeId::new("parent-root")?,
    );
    let altered_import = WorkspaceValueEntry::imported(
        altered_root.reference().clone(),
        ValueKey::new("altered")?,
        child_value.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"answer": 43}))?),
    )?;
    let altered_budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let altered_usage =
        altered_budget.admit_value(&WorkspaceUsage::EMPTY, altered_import.value())?;
    let altered_request = accepted_workspace_request(
        altered_root.reference().run().clone(),
        "command-altered-import",
        "event-altered-import",
        vec![run_created_kind(
            altered_root.clone(),
            altered_budget.clone(),
            vec![altered_import.reference().clone()],
        )?],
        vec![
            WorkspaceMutation::CreateScope {
                scope: altered_root,
            },
            WorkspaceMutation::PutValue {
                entry: altered_import,
            },
        ],
        WorkspaceAccounting {
            budget: altered_budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: altered_usage,
        },
    )?;
    assert!(matches!(
        store.commit_command(&altered_request),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert_eq!(
        store.head(altered_request.receipt.run())?,
        RunSequence::ZERO
    );

    let missing_root = WorkspaceScope::run_root(
        RunId::new("parent-missing-import")?,
        ScopeId::new("parent-root")?,
    );
    let missing_source = WorkspaceValueEntry::initial(
        WorkspaceScope::run_root(RunId::new("absent-child-run")?, ScopeId::new("child-root")?)
            .reference()
            .clone(),
        ValueKey::new("absent")?,
        WorkspaceValue::Json(BoundedJson::new(json!(null))?),
    );
    let missing_import = WorkspaceValueEntry::imported(
        missing_root.reference().clone(),
        ValueKey::new("missing")?,
        missing_source.reference().clone(),
        missing_source.value().clone(),
    )?;
    let missing_budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let missing_usage =
        missing_budget.admit_value(&WorkspaceUsage::EMPTY, missing_import.value())?;
    let missing_request = accepted_workspace_request(
        missing_root.reference().run().clone(),
        "command-missing-import",
        "event-missing-import",
        vec![run_created_kind(
            missing_root.clone(),
            missing_budget.clone(),
            vec![missing_import.reference().clone()],
        )?],
        vec![
            WorkspaceMutation::CreateScope {
                scope: missing_root,
            },
            WorkspaceMutation::PutValue {
                entry: missing_import,
            },
        ],
        WorkspaceAccounting {
            budget: missing_budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: missing_usage,
        },
    )?;
    assert!(matches!(
        store.commit_command(&missing_request),
        Err(PersistenceError::NotFound {
            entity: "imported_workspace_value",
            ..
        })
    ));
    assert_eq!(
        store.head(missing_request.receipt.run())?,
        RunSequence::ZERO
    );
    Ok(())
}

#[test]
fn workspace_scope_and_value_envelopes_detect_valid_json_payload_tampering()
-> Result<(), Box<dyn std::error::Error>> {
    const SCOPES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.scopes");
    const VALUES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.values");

    let directory = TempDir::new()?;
    let run = RunId::new("run-envelope-tamper")?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-root")?);
    let child = WorkspaceScope::branch(
        ScopeId::new("scope-child")?,
        &root,
        BranchId::new("branch-a")?,
    )?;
    let entry = WorkspaceValueEntry::initial(
        child.reference().clone(),
        ValueKey::new("answer")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"answer": 42}))?),
    );
    let budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let usage = budget.admit_value(&WorkspaceUsage::EMPTY, entry.value())?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&accepted_workspace_request(
            run.clone(),
            "command-envelope-tamper",
            "event-envelope-tamper",
            vec![
                run_created_kind(root.clone(), budget.clone(), Vec::new())?,
                RunEventKind::BranchScopeCreated {
                    fork_execution: NodeExecutionId::new("fork-envelope-tamper")?,
                    port: PortId::new("branch-a")?,
                    branch: BranchId::new("branch-a")?,
                    scope: child.clone(),
                },
                RunEventKind::DeterministicOutputPublished {
                    execution: NodeExecutionId::new("output-envelope-tamper")?,
                    value: entry.reference().clone(),
                    artifact: None,
                },
            ],
            vec![
                WorkspaceMutation::CreateScope { scope: root },
                WorkspaceMutation::CreateScope {
                    scope: child.clone(),
                },
                WorkspaceMutation::PutValue {
                    entry: entry.clone(),
                },
            ],
            WorkspaceAccounting {
                budget,
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: usage,
            },
        )?)?;
    }

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut scopes = write.open_table(SCOPES)?;
        let (key, bytes) = {
            let mut found = None;
            for item in scopes.iter()? {
                let (key, bytes) = item?;
                if bytes
                    .value()
                    .windows(b"branch-a".len())
                    .any(|part| part == b"branch-a")
                {
                    found = Some((key.value().to_vec(), bytes.value().to_vec()));
                    break;
                }
            }
            found.ok_or("child scope envelope was not found")?
        };
        let tampered = String::from_utf8(bytes)?.replace("branch-a", "branch-b");
        scopes.insert(key.as_slice(), tampered.as_bytes())?;
    }
    {
        let mut values = write.open_table(VALUES)?;
        let (key, bytes) = {
            let mut rows = values.iter()?;
            let (key, bytes) = rows
                .next()
                .transpose()?
                .ok_or("workspace value is absent")?;
            (key.value().to_vec(), bytes.value().to_vec())
        };
        let tampered = String::from_utf8(bytes)?.replace("\"answer\":42", "\"answer\":43");
        values.insert(key.as_slice(), tampered.as_bytes())?;
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    let scope_result = store.scope(&run, child.reference().scope());
    assert_storage_corruption(scope_result);
    let value_result = store.value(entry.reference());
    assert_storage_corruption(value_result);
    Ok(())
}

#[test]
fn durable_workspace_inheritance_preserves_exact_ancestor_content()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let root =
        WorkspaceScope::run_root(RunId::new("run-inherited-content")?, ScopeId::new("root")?);
    let branch_id = BranchId::new("branch-a")?;
    let branch = WorkspaceScope::branch(ScopeId::new("branch-scope")?, &root, branch_id.clone())?;
    let root_value = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("request")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"prompt": "original"}))?),
    );
    let altered_inheritance = WorkspaceValueEntry::inherited(
        branch.reference().clone(),
        ValueKey::new("request")?,
        root_value.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"prompt": "altered"}))?),
    )?;
    let budget = WorkspaceBudget::new(2, 1024, 2048, 0, 0, 0)?;
    let root_usage = budget.admit_value(&WorkspaceUsage::EMPTY, root_value.value())?;
    let resulting_usage = budget.admit_value(&root_usage, altered_inheritance.value())?;
    let request = accepted_workspace_request(
        root.reference().run().clone(),
        "command-inherited-content",
        "event-inherited-content",
        vec![
            run_created_kind(
                root.clone(),
                budget.clone(),
                vec![root_value.reference().clone()],
            )?,
            RunEventKind::BranchScopeCreated {
                fork_execution: NodeExecutionId::new("fork-inherited-content")?,
                port: PortId::new("branch-a")?,
                branch: branch_id,
                scope: branch.clone(),
            },
            RunEventKind::DeterministicOutputPublished {
                execution: NodeExecutionId::new("output-inherited-content")?,
                value: altered_inheritance.reference().clone(),
                artifact: None,
            },
        ],
        vec![
            WorkspaceMutation::CreateScope {
                scope: root.clone(),
            },
            WorkspaceMutation::CreateScope { scope: branch },
            WorkspaceMutation::PutValue { entry: root_value },
            WorkspaceMutation::PutValue {
                entry: altered_inheritance,
            },
        ],
        WorkspaceAccounting {
            budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage,
        },
    )?;

    assert!(matches!(
        store.commit_command(&request),
        Err(PersistenceError::InvalidDocument(reason))
            if reason.contains("preserve its exact ancestor content")
    ));
    assert_eq!(store.head(request.receipt.run())?, RunSequence::ZERO);
    assert!(
        store
            .scope(request.receipt.run(), root.reference().scope())?
            .is_none()
    );
    Ok(())
}

#[test]
fn deleted_successor_history_blocks_reads_latest_and_next_version()
-> Result<(), Box<dyn std::error::Error>> {
    const VALUES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.values");

    let directory = TempDir::new()?;
    let run = RunId::new("run-deleted-successor-history")?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
    let key = ValueKey::new("stream")?;
    let first = WorkspaceValueEntry::initial(
        root.reference().clone(),
        key.clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"version": 1}))?),
    );
    let second = WorkspaceValueEntry::successor(
        first.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"version": 2}))?),
    )?;
    let third = WorkspaceValueEntry::successor(
        second.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"version": 3}))?),
    )?;
    let budget = WorkspaceBudget::new(3, 1024, 3072, 0, 0, 0)?;
    let usage_one = budget.admit_value(&WorkspaceUsage::EMPTY, first.value())?;
    let usage_two = budget.admit_value(&usage_one, second.value())?;
    let usage_three = budget.admit_value(&usage_two, third.value())?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&accepted_workspace_request(
            run.clone(),
            "command-successor-one",
            "event-successor-one",
            vec![run_created_kind(
                root.clone(),
                budget.clone(),
                vec![first.reference().clone()],
            )?],
            vec![
                WorkspaceMutation::CreateScope {
                    scope: root.clone(),
                },
                WorkspaceMutation::PutValue {
                    entry: first.clone(),
                },
            ],
            WorkspaceAccounting {
                budget: budget.clone(),
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: usage_one,
            },
        )?)?;
        store.commit_command(&accepted_workspace_followup_request(
            run.clone(),
            RunSequence::FIRST,
            "command-successor-two",
            "event-successor-two",
            vec![RunEventKind::DeterministicOutputPublished {
                execution: NodeExecutionId::new("execution-successor-two")?,
                value: second.reference().clone(),
                artifact: None,
            }],
            vec![WorkspaceMutation::PutValue {
                entry: second.clone(),
            }],
            WorkspaceAccounting {
                budget: budget.clone(),
                expected_usage: usage_one,
                resulting_usage: usage_two,
            },
        )?)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut values = write.open_table(VALUES)?;
        assert!(
            values
                .remove(stored_workspace_value_key(first.reference())?.as_slice())?
                .is_some()
        );
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_storage_corruption(store.value(second.reference()));
    assert_storage_corruption(store.latest_value(root.reference(), &key));
    let next = accepted_workspace_followup_request(
        run,
        RunSequence::new(2),
        "command-successor-three",
        "event-successor-three",
        vec![RunEventKind::DeterministicOutputPublished {
            execution: NodeExecutionId::new("execution-successor-three")?,
            value: third.reference().clone(),
            artifact: None,
        }],
        vec![WorkspaceMutation::PutValue { entry: third }],
        WorkspaceAccounting {
            budget,
            expected_usage: usage_two,
            resulting_usage: usage_three,
        },
    )?;
    assert_storage_corruption(store.commit_command(&next));
    assert_eq!(
        store.health(TimestampMillis::new(20))?.status,
        StorageHealthStatus::Degraded
    );
    Ok(())
}

#[test]
fn deleted_inherited_source_history_is_corruption() -> Result<(), Box<dyn std::error::Error>> {
    const VALUES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.values");

    let directory = TempDir::new()?;
    let run = RunId::new("run-deleted-inherited-history")?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
    let branch =
        WorkspaceScope::branch(ScopeId::new("branch")?, &root, BranchId::new("branch-a")?)?;
    let source_one = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("source")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"value": 1}))?),
    );
    let source_two = WorkspaceValueEntry::successor(
        source_one.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"value": 2}))?),
    )?;
    let inherited = WorkspaceValueEntry::inherited(
        branch.reference().clone(),
        ValueKey::new("inherited")?,
        source_two.reference().clone(),
        source_two.value().clone(),
    )?;
    let budget = WorkspaceBudget::new(3, 1024, 3072, 0, 0, 0)?;
    let usage_one = budget.admit_value(&WorkspaceUsage::EMPTY, source_one.value())?;
    let usage_two = budget.admit_value(&usage_one, source_two.value())?;
    let usage_three = budget.admit_value(&usage_two, inherited.value())?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&accepted_workspace_request(
            run.clone(),
            "command-inherited-source-one",
            "event-inherited-source-one",
            vec![run_created_kind(
                root.clone(),
                budget.clone(),
                vec![source_one.reference().clone()],
            )?],
            vec![
                WorkspaceMutation::CreateScope {
                    scope: root.clone(),
                },
                WorkspaceMutation::PutValue {
                    entry: source_one.clone(),
                },
            ],
            WorkspaceAccounting {
                budget: budget.clone(),
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: usage_one,
            },
        )?)?;
        store.commit_command(&accepted_workspace_followup_request(
            run,
            RunSequence::FIRST,
            "command-inherited-source-two",
            "event-inherited-source-two",
            vec![
                RunEventKind::BranchScopeCreated {
                    fork_execution: NodeExecutionId::new("fork-inherited-history")?,
                    port: PortId::new("branch-a")?,
                    branch: BranchId::new("branch-a")?,
                    scope: branch.clone(),
                },
                RunEventKind::DeterministicOutputPublished {
                    execution: NodeExecutionId::new("execution-source-two")?,
                    value: source_two.reference().clone(),
                    artifact: None,
                },
                RunEventKind::DeterministicOutputPublished {
                    execution: NodeExecutionId::new("execution-inherited")?,
                    value: inherited.reference().clone(),
                    artifact: None,
                },
            ],
            vec![
                WorkspaceMutation::CreateScope { scope: branch },
                WorkspaceMutation::PutValue { entry: source_two },
                WorkspaceMutation::PutValue {
                    entry: inherited.clone(),
                },
            ],
            WorkspaceAccounting {
                budget,
                expected_usage: usage_one,
                resulting_usage: usage_three,
            },
        )?)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut values = write.open_table(VALUES)?;
        assert!(
            values
                .remove(stored_workspace_value_key(source_one.reference())?.as_slice())?
                .is_some()
        );
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_storage_corruption(store.value(inherited.reference()));
    assert_eq!(
        store.health(TimestampMillis::new(20))?.status,
        StorageHealthStatus::Degraded
    );
    Ok(())
}

#[test]
fn deleted_imported_source_history_is_corruption() -> Result<(), Box<dyn std::error::Error>> {
    const VALUES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.values");

    let directory = TempDir::new()?;
    let child_run = RunId::new("run-import-source-history")?;
    let child_root = WorkspaceScope::run_root(child_run.clone(), ScopeId::new("child-root")?);
    let source_one = WorkspaceValueEntry::initial(
        child_root.reference().clone(),
        ValueKey::new("source")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"value": 1}))?),
    );
    let source_two = WorkspaceValueEntry::successor(
        source_one.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"value": 2}))?),
    )?;
    let child_budget = WorkspaceBudget::new(2, 1024, 2048, 0, 0, 0)?;
    let child_usage_one = child_budget.admit_value(&WorkspaceUsage::EMPTY, source_one.value())?;
    let child_usage_two = child_budget.admit_value(&child_usage_one, source_two.value())?;

    let parent_run = RunId::new("run-import-target-history")?;
    let parent_root = WorkspaceScope::run_root(parent_run.clone(), ScopeId::new("parent-root")?);
    let imported = WorkspaceValueEntry::imported(
        parent_root.reference().clone(),
        ValueKey::new("imported")?,
        source_two.reference().clone(),
        source_two.value().clone(),
    )?;
    let parent_budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let parent_usage = parent_budget.admit_value(&WorkspaceUsage::EMPTY, imported.value())?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&accepted_workspace_request(
            child_run.clone(),
            "command-import-source-one",
            "event-import-source-one",
            vec![run_created_kind(
                child_root.clone(),
                child_budget.clone(),
                vec![source_one.reference().clone()],
            )?],
            vec![
                WorkspaceMutation::CreateScope { scope: child_root },
                WorkspaceMutation::PutValue {
                    entry: source_one.clone(),
                },
            ],
            WorkspaceAccounting {
                budget: child_budget.clone(),
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: child_usage_one,
            },
        )?)?;
        store.commit_command(&accepted_workspace_followup_request(
            child_run,
            RunSequence::FIRST,
            "command-import-source-two",
            "event-import-source-two",
            vec![RunEventKind::DeterministicOutputPublished {
                execution: NodeExecutionId::new("execution-import-source-two")?,
                value: source_two.reference().clone(),
                artifact: None,
            }],
            vec![WorkspaceMutation::PutValue { entry: source_two }],
            WorkspaceAccounting {
                budget: child_budget,
                expected_usage: child_usage_one,
                resulting_usage: child_usage_two,
            },
        )?)?;
        store.commit_command(&accepted_workspace_request(
            parent_run,
            "command-import-target",
            "event-import-target",
            vec![run_created_kind(
                parent_root.clone(),
                parent_budget.clone(),
                vec![imported.reference().clone()],
            )?],
            vec![
                WorkspaceMutation::CreateScope { scope: parent_root },
                WorkspaceMutation::PutValue {
                    entry: imported.clone(),
                },
            ],
            WorkspaceAccounting {
                budget: parent_budget,
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: parent_usage,
            },
        )?)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut values = write.open_table(VALUES)?;
        assert!(
            values
                .remove(stored_workspace_value_key(source_one.reference())?.as_slice())?
                .is_some()
        );
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_storage_corruption(store.value(imported.reference()));
    assert_eq!(
        store.health(TimestampMillis::new(20))?.status,
        StorageHealthStatus::Degraded
    );
    Ok(())
}

#[test]
fn deleted_parent_scope_blocks_child_reads_writes_and_health()
-> Result<(), Box<dyn std::error::Error>> {
    const SCOPES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.scopes");

    let directory = TempDir::new()?;
    let run = RunId::new("run-deleted-parent-scope")?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
    let parent = WorkspaceScope::branch(
        ScopeId::new("parent")?,
        &root,
        BranchId::new("branch-parent")?,
    )?;
    let child = WorkspaceScope::branch(
        ScopeId::new("child")?,
        &parent,
        BranchId::new("branch-child")?,
    )?;
    let budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&accepted_workspace_request(
            run.clone(),
            "command-parent-scope",
            "event-parent-scope",
            vec![
                run_created_kind(root.clone(), budget.clone(), Vec::new())?,
                RunEventKind::BranchScopeCreated {
                    fork_execution: NodeExecutionId::new("fork-parent")?,
                    port: PortId::new("branch-parent")?,
                    branch: BranchId::new("branch-parent")?,
                    scope: parent.clone(),
                },
                RunEventKind::BranchScopeCreated {
                    fork_execution: NodeExecutionId::new("fork-child")?,
                    port: PortId::new("branch-child")?,
                    branch: BranchId::new("branch-child")?,
                    scope: child.clone(),
                },
            ],
            vec![
                WorkspaceMutation::CreateScope { scope: root },
                WorkspaceMutation::CreateScope {
                    scope: parent.clone(),
                },
                WorkspaceMutation::CreateScope {
                    scope: child.clone(),
                },
            ],
            WorkspaceAccounting {
                budget: budget.clone(),
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: WorkspaceUsage::EMPTY,
            },
        )?)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut scopes = write.open_table(SCOPES)?;
        assert!(
            scopes
                .remove(stored_workspace_scope_key(parent.reference())?.as_slice())?
                .is_some()
        );
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_storage_corruption(store.scope(&run, child.reference().scope()));
    assert_storage_corruption(
        store.latest_value(child.reference(), &ValueKey::new("never-written")?),
    );
    let value = WorkspaceValueEntry::initial(
        child.reference().clone(),
        ValueKey::new("blocked")?,
        WorkspaceValue::Json(BoundedJson::new(json!(true))?),
    );
    let resulting_usage = budget.admit_value(&WorkspaceUsage::EMPTY, value.value())?;
    let write_request = accepted_workspace_followup_request(
        run,
        RunSequence::new(3),
        "command-child-after-parent-delete",
        "event-child-after-parent-delete",
        vec![RunEventKind::DeterministicOutputPublished {
            execution: NodeExecutionId::new("execution-child-after-parent-delete")?,
            value: value.reference().clone(),
            artifact: None,
        }],
        vec![WorkspaceMutation::PutValue { entry: value }],
        WorkspaceAccounting {
            budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage,
        },
    )?;
    assert_storage_corruption(store.commit_command(&write_request));
    assert_eq!(
        store.health(TimestampMillis::new(20))?.status,
        StorageHealthStatus::Degraded
    );
    Ok(())
}

#[test]
fn durable_workspace_rejects_scope_lineages_beyond_the_contract_bound()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let mut request = accepted_request(
        "run-scope-depth",
        "command-scope-depth",
        "event-scope-depth",
        "start",
    )?;
    let root = WorkspaceScope::run_root(
        request.receipt.run().clone(),
        ScopeId::new("scope-depth-root")?,
    );
    request.workspace.push(WorkspaceMutation::CreateScope {
        scope: root.clone(),
    });
    let mut parent = root;
    for depth in 1..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let child = WorkspaceScope::branch(
            ScopeId::new(format!("scope-depth-{depth}"))?,
            &parent,
            BranchId::new(format!("branch-depth-{depth}"))?,
        )?;
        request.workspace.push(WorkspaceMutation::CreateScope {
            scope: child.clone(),
        });
        parent = child;
    }
    let too_deep = WorkspaceScope::branch(
        ScopeId::new("scope-depth-overflow")?,
        &parent,
        BranchId::new("branch-depth-overflow")?,
    )?;
    request
        .workspace
        .push(WorkspaceMutation::CreateScope { scope: too_deep });
    assert!(matches!(
        store.commit_command(&request),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert_eq!(store.head(request.receipt.run())?, RunSequence::ZERO);
    let root_scope_id = ScopeId::new("scope-depth-root")?;
    assert!(
        store
            .scope(request.receipt.run(), &root_scope_id)?
            .is_none()
    );
    Ok(())
}
