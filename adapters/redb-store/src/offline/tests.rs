use super::*;
use milkdrift_persistence::{
    ClockWatermarkStore, PageSize, StorageFailureClass, TimestampMillis, WorkflowCommandAcceptance,
    WorkflowCommit, WorkflowCommitOutcome, WorkflowSnapshot, WorkflowStore,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn producer() -> BackupProducer {
    BackupProducer {
        version: "test".into(),
        binary_digest: "a".repeat(64),
        source_revision: "fixture".into(),
    }
}
fn receipt(command: &str, intent: &[u8]) -> Result<WorkflowCommit, Box<dyn std::error::Error>> {
    Ok(WorkflowCommit {
        authoring: None,
        project_guard: None,
        project_id: None,
        expected_generation: None,
        snapshot: WorkflowSnapshot {
            workflow: milkdrift_blueprint::WorkflowId::new("offline-workflow")?,
            generation: 1,
            title: "Private offline fixture".into(),
            description: String::new(),
            owner_actor: "operator:offline".into(),
            updated_at: TimestampMillis::new(10),
            document: serde_json::json!({"private":"never-export-this"}),
            entered_invocations: Vec::new(),
            managed_executions: Vec::new(),
            artifact_capacity: 0,
            reserved_bytes: 0,
            reserved_receipts: 0,
        },
        command: Some(WorkflowCommandAcceptance {
            actor: "operator:offline".into(),
            key: command.into(),
            reserved_action: None,
            canonical_request: serde_json::to_vec(
                &serde_json::json!({"intent": String::from_utf8_lossy(intent)}),
            )?,
            result: serde_json::json!({"private":"never-export-this"}),
        }),
    })
}
fn seed(root: &Path) -> Result<WorkflowCommit, Box<dyn std::error::Error>> {
    let store = RedbStore::open(root)?;
    let first = receipt("first", b"original")?;
    store.commit_workflow(first.clone())?;
    let mut second = receipt("second", b"second")?;
    second.expected_generation = Some(1);
    second.snapshot.generation = 2;
    store.commit_workflow(second)?;
    Ok(first)
}

#[test]
fn inspection_preserves_bytes_clock_retention_and_refuses_live_writer() -> TestResult {
    let parent = super::private_test_directory()?;
    let root = parent.path().join("source");
    seed(&root)?;
    let writer = RedbStore::open(&root)?;
    assert!(matches!(
        OfflineStore::open(&root, parent.path()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::OwnerBusy,
            ..
        })
    ));
    drop(writer);
    let before = fs::read(root.join(DATABASE_FILENAME))?;
    let modified = fs::metadata(root.join(DATABASE_FILENAME))?.modified()?;
    let offline = OfflineStore::open(&root, parent.path())?;
    assert!(offline._scratch.path().join(INSPECTION_MARKER).is_file());
    assert!(RedbStore::open(offline._scratch.path()).is_err());
    let overview = offline.overview()?;
    assert_eq!(
        overview
            .row_counts
            .iter()
            .find(|(f, _)| *f == InspectionFamily::Receipts)
            .map(|(_, n)| *n),
        Some(2)
    );
    assert!(RedbStore::open(&root).is_err());
    let page = offline.inspect(InspectionFamily::Receipts, PageSize::new(1)?, None, false)?;
    assert_eq!(page.records.len(), 1);
    assert!(!serde_json::to_string(&page)?.contains("never-export-this"));
    assert!(
        offline
            .inspect(
                InspectionFamily::Workflows,
                PageSize::new(1)?,
                page.next.as_deref(),
                false
            )
            .is_err()
    );
    let next = offline.inspect(
        InspectionFamily::Receipts,
        PageSize::new(1)?,
        page.next.as_deref(),
        false,
    )?;
    assert_eq!(next.records.len(), 1);
    assert!(next.next.is_none());
    let scan = offline.scan_integrity(IntegrityScanRequest {
        limit: PageSize::new(128)?,
        verify_artifact_content: true,
        cursor: None,
    })?;
    assert!(scan.failures.is_empty(), "{:?}", scan.failures);
    drop(offline);
    assert_eq!(before, fs::read(root.join(DATABASE_FILENAME))?);
    assert_eq!(
        modified,
        fs::metadata(root.join(DATABASE_FILENAME))?.modified()?
    );
    assert_eq!(
        fs::read_dir(parent.path())?.count(),
        1,
        "scratch copy must be cleaned up"
    );
    Ok(())
}

#[test]
fn backup_restore_preserves_cold_exact_replay_conflict_and_clock_with_execution_guard() -> TestResult
{
    let parent = super::private_test_directory()?;
    let root = parent.path().join("source");
    let saved = seed(&root)?;
    let before = fs::read(root.join(DATABASE_FILENAME))?;
    let destination = parent.path().join("backup");
    let restored = parent.path().join("restored");
    let offline = OfflineStore::open(&root, parent.path())?;
    let manifest = offline.backup(&destination, parent.path(), producer())?;
    assert_eq!(manifest.integrity_failures, 0);
    assert!(
        offline
            .backup(&destination, parent.path(), producer())
            .is_err()
    );
    drop(offline);
    assert_eq!(before, fs::read(root.join(DATABASE_FILENAME))?);
    OfflineStore::verify_backup(&destination, parent.path())?;
    OfflineStore::restore(&destination, &restored, parent.path())?;
    assert_eq!(before, fs::read(restored.join(DATABASE_FILENAME))?);
    assert!(OfflineStore::restore(&destination, &restored, parent.path()).is_err());
    assert!(RedbStore::open(&restored).is_err());
    // The isolated fixture explicitly authorizes activation after preserving the source.
    fs::remove_file(restored.join(INSPECTION_MARKER))?;
    let store = RedbStore::open(&restored)?;
    assert!(store.clock_watermark()?.ok_or("clock")?.get() >= manifest.clock_high_water_unix_ms);
    let accepted = saved.command.as_ref().ok_or("missing command")?;
    let retained = store
        .workflow_receipt(&accepted.actor, &accepted.key)?
        .ok_or("missing exact receipt")?;
    assert_eq!(retained.acceptance, *accepted);
    assert!(matches!(
        store.commit_workflow(saved)?,
        WorkflowCommitOutcome::Replay(_)
    ));
    assert!(matches!(
        store.commit_workflow(receipt("first", b"changed")?),
        Err(PersistenceError::WorkflowIdempotencyConflict { .. })
    ));
    Ok(())
}

#[test]
fn malformed_backup_provenance_is_refused_before_copy_and_on_verification() -> TestResult {
    let parent = super::private_test_directory()?;
    let root = parent.path().join("source");
    seed(&root)?;
    let offline = OfflineStore::open(&root, parent.path())?;
    let backup = parent.path().join("backup");
    offline.backup(&backup, parent.path(), producer())?;
    let manifest_path = backup.join("milkdrift-backup.json");
    let original = fs::read(&manifest_path)?;
    let mut invalid = Vec::new();
    let mut value = producer();
    value.version.clear();
    invalid.push(value);
    let mut value = producer();
    value.version = "v".repeat(129);
    invalid.push(value);
    let mut value = producer();
    value.source_revision.clear();
    invalid.push(value);
    let mut value = producer();
    value.source_revision = "r".repeat(257);
    invalid.push(value);
    let mut value = producer();
    value.binary_digest = "z".repeat(64);
    invalid.push(value);
    for producer in invalid {
        let destination = parent.path().join("refused");
        assert!(
            offline
                .backup(&destination, parent.path(), producer.clone())
                .is_err()
        );
        assert!(!destination.exists());
        let mut manifest: serde_json::Value = serde_json::from_slice(&original)?;
        *manifest
            .get_mut("producer")
            .ok_or("missing test manifest producer")? = serde_json::to_value(producer)?;
        fs::write(&manifest_path, serde_json::to_vec(&manifest)?)?;
        assert!(OfflineStore::verify_backup(&backup, parent.path()).is_err());
        assert!(OfflineStore::restore(&backup, &destination, parent.path()).is_err());
        assert!(!destination.exists());
    }
    fs::write(manifest_path, original)?;
    OfflineStore::verify_backup(&backup, parent.path())?;
    Ok(())
}

#[test]
fn interrupted_and_tampered_backups_never_verify_or_run() -> TestResult {
    let parent = super::private_test_directory()?;
    let root = parent.path().join("source");
    seed(&root)?;
    let before = fs::read(root.join(DATABASE_FILENAME))?;
    let offline = OfflineStore::open(&root, parent.path())?;
    let partial = parent.path().join("partial");
    backup::COPY_INTERRUPT.with(|hook| hook.set(true));
    let result = offline.backup(&partial, parent.path(), producer());
    backup::COPY_INTERRUPT.with(|hook| hook.set(false));
    assert!(result.is_err());
    assert!(partial.join(INSPECTION_MARKER).exists());
    assert!(!partial.join("milkdrift-backup.json").exists());
    assert!(OfflineStore::verify_backup(&partial, parent.path()).is_err());
    assert!(RedbStore::open(&partial).is_err());
    let backup = parent.path().join("complete");
    offline.backup(&backup, parent.path(), producer())?;
    let guard = fs::read(backup.join(INSPECTION_MARKER))?;
    fs::write(
        backup.join(INSPECTION_MARKER),
        b"{\"inspection_only\":false}",
    )?;
    assert!(OfflineStore::verify_backup(&backup, parent.path()).is_err());
    fs::remove_file(backup.join(INSPECTION_MARKER))?;
    assert!(OfflineStore::verify_backup(&backup, parent.path()).is_err());
    fs::write(backup.join(INSPECTION_MARKER), guard)?;
    fs::write(backup.join("artifacts/.tmp/unlisted.part"), b"extra")?;
    assert!(OfflineStore::verify_backup(&backup, parent.path()).is_err());
    drop(offline);
    assert_eq!(before, fs::read(root.join(DATABASE_FILENAME))?);
    Ok(())
}

#[test]
fn linked_roots_and_source_nested_scratch_are_refused() -> TestResult {
    let parent = super::private_test_directory()?;
    let root = parent.path().join("source");
    seed(&root)?;
    let before = fs::read(root.join(DATABASE_FILENAME))?;
    let nested = root.join("scratch");
    paths::create_directory(&nested)?;
    assert!(OfflineStore::open(&root, &nested).is_err());
    let linked = parent.path().join("linked");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&root, &linked)?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        let status = std::process::Command::new(
            Path::new(&std::env::var_os("SystemRoot").ok_or("SystemRoot")?)
                .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        )
        .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:MILKDRIFT_TEST_LINK -Target $env:MILKDRIFT_TEST_TARGET | Out-Null"])
        .env("MILKDRIFT_TEST_LINK", &linked)
        .env("MILKDRIFT_TEST_TARGET", &root)
        .creation_flags(0x0800_0000)
        .stdout(std::process::Stdio::null())
        .status()?;
        assert!(status.success());
    }
    assert!(OfflineStore::open(&linked, parent.path()).is_err());
    assert_eq!(before, fs::read(root.join(DATABASE_FILENAME))?);
    Ok(())
}

#[test]
fn broadly_readable_scratch_and_backup_parents_are_refused() -> TestResult {
    let parent = super::private_test_directory()?;
    let root = parent.path().join("source");
    seed(&root)?;
    let shared = parent.path().join("shared");
    paths::create_directory(&shared)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        let script = "$ErrorActionPreference='Stop'; $acl=[System.IO.Directory]::GetAccessControl($env:MILKDRIFT_TEST_SHARED); $everyone=New-Object System.Security.Principal.SecurityIdentifier('S-1-1-0'); $rule=New-Object System.Security.AccessControl.FileSystemAccessRule($everyone,'ReadAndExecute','ContainerInherit,ObjectInherit','None','Allow'); $acl.AddAccessRule($rule); [System.IO.Directory]::SetAccessControl($env:MILKDRIFT_TEST_SHARED,$acl)";
        let status = std::process::Command::new(
            Path::new(&std::env::var_os("SystemRoot").ok_or("SystemRoot")?)
                .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        )
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("MILKDRIFT_TEST_SHARED", &shared)
        .creation_flags(0x0800_0000)
        .status()?;
        assert!(status.success());
    }
    assert!(OfflineStore::open(&root, &shared).is_err());
    assert_eq!(fs::read_dir(&shared)?.count(), 0);
    let offline = OfflineStore::open(&root, parent.path())?;
    let destination = shared.join("backup");
    assert!(
        offline
            .backup(&destination, parent.path(), producer())
            .is_err()
    );
    assert!(!destination.exists());
    Ok(())
}

#[test]
fn unsupported_schema_corrupt_record_missing_root_and_sensitive_extra_files_are_explicit()
-> TestResult {
    let parent = super::private_test_directory()?;
    let root = parent.path().join("source");
    assert!(OfflineStore::open(&root, parent.path()).is_err());
    assert!(!root.exists());
    seed(&root)?;
    let database = Database::open(root.join(DATABASE_FILENAME))?;
    let write = database.begin_write()?;
    write
        .open_table(crate::schema::TASK_RECEIPTS)?
        .insert("bad", b"corrupt-protected-data".as_slice())?;
    write.commit()?;
    drop(database);
    let offline = OfflineStore::open(&root, parent.path())?;
    let page = offline.inspect(InspectionFamily::Receipts, PageSize::new(128)?, None, false)?;
    assert!(
        page.records
            .iter()
            .any(|v| matches!(v, InspectionRecord::Failure { .. }))
    );
    assert!(!serde_json::to_string(&page)?.contains("corrupt-protected-data"));
    fs::write(root.join("secret.env"), b"do-not-copy")?;
    let destination = parent.path().join("refused");
    assert!(
        offline
            .backup(&destination, parent.path(), producer())
            .is_err()
    );
    assert!(!destination.exists());
    drop(offline);
    let database = Database::open(root.join(DATABASE_FILENAME))?;
    let write = database.begin_write()?;
    write
        .open_table(crate::schema::METADATA)?
        .insert(crate::schema::SCHEMA_VERSION_KEY, 999)?;
    write
        .open_table(crate::schema::METADATA)?
        .remove(crate::schema::INTERNAL_DOCUMENT_FORMAT_VERSION_KEY)?;
    write.commit()?;
    drop(database);
    let before = fs::read(root.join(DATABASE_FILENAME))?;
    assert!(matches!(
        OfflineStore::open(&root, parent.path()),
        Err(PersistenceError::UnsupportedVersion { found: 999, .. })
    ));
    assert_eq!(before, fs::read(root.join(DATABASE_FILENAME))?);
    Ok(())
}
