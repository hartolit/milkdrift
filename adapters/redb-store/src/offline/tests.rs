use super::*;
use milkdrift_authority::{ActorRef, GrantDigest, GrantId};
use milkdrift_persistence::{
    ApplicationCommandCommit, ApplicationCommandCommitOutcome, ApplicationCommandEffect,
    ApplicationCommandReceipt, ApplicationCommandResult, ApplicationCommandStore,
    ClockWatermarkStore, CommandId, IntegrityDigest, PageSize, StorageFailureClass,
    TimestampMillis,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn producer() -> BackupProducer {
    BackupProducer {
        version: "test".into(),
        binary_digest: "a".repeat(64),
        source_revision: "fixture".into(),
    }
}
fn receipt(
    command: &str,
    intent: &[u8],
) -> Result<ApplicationCommandReceipt, Box<dyn std::error::Error>> {
    Ok(ApplicationCommandReceipt::new(
        ActorRef::new("operator:offline")?,
        CommandId::new(command)?,
        1,
        IntegrityDigest::hash(intent),
        GrantId::new("grant:offline")?,
        1,
        GrantDigest::new(format!("b3_{}", "a".repeat(64)))?,
        None,
        TimestampMillis::new(10),
        TimestampMillis::new(10),
        ApplicationCommandResult::Rejected {
            document: br#"{"accepted":false,"private":"never-export-this"}"#.to_vec(),
        },
    )?)
}

fn seed(root: &Path) -> Result<ApplicationCommandReceipt, Box<dyn std::error::Error>> {
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(root).with_application_receipt_lifecycle(1, 1),
    )?;
    let first = receipt("first", b"original")?;
    for saved in [
        first.clone(),
        receipt("second", b"second")?,
        receipt("third", b"third")?,
    ] {
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: saved,
            effect: ApplicationCommandEffect::None,
        })?;
    }
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
            .find(|(f, _)| *f == InspectionFamily::ColdReceipts)
            .map(|(_, n)| *n),
        Some(2)
    );
    assert!(RedbStore::open(&root).is_err());
    let page = offline.inspect(
        InspectionFamily::ColdReceipts,
        PageSize::new(1)?,
        None,
        false,
    )?;
    assert_eq!(page.records.len(), 1);
    assert!(!serde_json::to_string(&page)?.contains("never-export-this"));
    assert!(
        offline
            .inspect(
                InspectionFamily::HotReceipts,
                PageSize::new(1)?,
                page.next.as_deref(),
                false
            )
            .is_err()
    );
    let next = offline.inspect(
        InspectionFamily::ColdReceipts,
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
    assert_eq!(
        store.application_command_receipt(saved.actor(), saved.command())?,
        Some(saved.clone())
    );
    assert!(matches!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: saved,
            effect: ApplicationCommandEffect::None
        })?,
        ApplicationCommandCommitOutcome::Replayed(_)
    ));
    assert!(matches!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: receipt("first", b"changed")?,
            effect: ApplicationCommandEffect::None
        }),
        Err(PersistenceError::ExternalCommandIdempotencyConflict { .. })
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
        manifest["producer"] = serde_json::to_value(producer)?;
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
        .open_table(crate::schema::APPLICATION_COMMAND_RECEIPTS_COLD)?
        .insert(b"bad".as_slice(), b"corrupt-protected-data".as_slice())?;
    write.commit()?;
    drop(database);
    let offline = OfflineStore::open(&root, parent.path())?;
    let page = offline.inspect(
        InspectionFamily::ColdReceipts,
        PageSize::new(128)?,
        None,
        false,
    )?;
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
