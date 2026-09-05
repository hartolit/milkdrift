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
    assert_eq!(store.head(request.receipt().run())?, RunSequence::FIRST);
    let page = store.events(&EventPageQuery::new(
        request.receipt().run().clone(),
        None,
        PageSize::new(10)?,
    )?)?;
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].event_id(), request.events()[0].event_id());
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
        store.head(altered_request.receipt().run())?,
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
        store.head(missing_request.receipt().run())?,
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
    assert_eq!(store.head(request.receipt().run())?, RunSequence::ZERO);
    assert!(
        store
            .scope(request.receipt().run(), root.reference().scope())?
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
    let request = accepted_request(
        "run-scope-depth",
        "command-scope-depth",
        "event-scope-depth",
        "start",
    )?;
    let root = WorkspaceScope::run_root(
        request.receipt().run().clone(),
        ScopeId::new("scope-depth-root")?,
    );
    let mut workspace = request.workspace().to_vec();
    workspace.push(WorkspaceMutation::CreateScope {
        scope: root.clone(),
    });
    let mut parent = root;
    for depth in 1..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let child = WorkspaceScope::branch(
            ScopeId::new(format!("scope-depth-{depth}"))?,
            &parent,
            BranchId::new(format!("branch-depth-{depth}"))?,
        )?;
        workspace.push(WorkspaceMutation::CreateScope {
            scope: child.clone(),
        });
        parent = child;
    }
    let too_deep = WorkspaceScope::branch(
        ScopeId::new("scope-depth-overflow")?,
        &parent,
        BranchId::new("branch-depth-overflow")?,
    )?;
    workspace.push(WorkspaceMutation::CreateScope { scope: too_deep });
    let invalid = AtomicRunCommitRequest::new(
        request.receipt().clone(),
        request.events().to_vec(),
        workspace,
        request.workspace_accounting().cloned(),
        request.required_artifacts().to_vec(),
        request.newly_referenced_artifacts().to_vec(),
        request.expected_lease_revision().cloned(),
        request.result().clone(),
        request.indexes().clone(),
    );
    assert!(matches!(invalid, Err(PersistenceError::InvalidDocument(_))));
    assert_eq!(store.head(request.receipt().run())?, RunSequence::ZERO);
    let root_scope_id = ScopeId::new("scope-depth-root")?;
    assert!(
        store
            .scope(request.receipt().run(), &root_scope_id)?
            .is_none()
    );
    Ok(())
}

#[path = "journal_workspace/paging.rs"]
mod paging;
