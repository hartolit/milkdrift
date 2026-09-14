//! Actual executable composition: no configuration/credentials/host are required offline.
use milkdrift_redb_store::RedbStore;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn admin(root: &Path, scratch: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_milkdrift-daemon"))
        .env_remove("MILKDRIFT_DAEMON_CONFIG")
        .args(["storage-admin", "--root"])
        .arg(root)
        .arg("--scratch")
        .arg(scratch)
        .args(args)
        .output()
}

#[test]
fn maintenance_binary_inspects_backs_up_restores_and_refuses_existing_destination() -> TestResult {
    let parent = milkdrift_redb_store::testing::private_offline_directory()?;
    let root = parent.path().join("source");
    let writer = RedbStore::open(&root)?;
    assert!(!admin(&root, parent.path(), &["overview"])?.status.success());
    drop(writer);
    let before = fs::read(root.join("milkdrift.redb"))?;
    for args in [
        vec!["overview"],
        vec!["runs", "--limit", "1"],
        vec!["records", "--family", "leases", "--limit", "1"],
        vec!["records", "--family", "accounts"],
        vec!["records", "--family", "peer-tombstones"],
        vec!["scan", "--hash-artifacts"],
    ] {
        let output = admin(&root, parent.path(), &args)?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(report["kind"], "inspection_report");
        assert_eq!(report["execution_authorized"], false);
    }
    let backup = parent.path().join("backup");
    let restored = parent.path().join("restored");
    let output = admin(
        &root,
        parent.path(),
        &["backup", "--destination", backup.to_str().ok_or("path")?],
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        admin(&backup, parent.path(), &["verify-backup"])?
            .status
            .success()
    );
    let output = admin(
        &backup,
        parent.path(),
        &["restore", "--destination", restored.to_str().ok_or("path")?],
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !admin(
            &backup,
            parent.path(),
            &["restore", "--destination", restored.to_str().ok_or("path")?]
        )?
        .status
        .success()
    );
    assert!(RedbStore::open(&restored).is_err());
    assert_eq!(before, fs::read(root.join("milkdrift.redb"))?);
    assert_eq!(before, fs::read(restored.join("milkdrift.redb"))?);
    Ok(())
}

#[test]
fn maintenance_parsing_and_missing_roots_never_initialize_storage() -> TestResult {
    let parent = milkdrift_redb_store::testing::private_offline_directory()?;
    let root = parent.path().join("missing");
    for args in [
        vec!["overview"],
        vec!["records", "--family", "grants"],
        vec!["records", "--family", "leases", "--limit", "0"],
        vec!["scan", "--limit", "129"],
        vec!["run", "--run", "unknown", "--maximum-events", "0"],
    ] {
        let output = admin(&root, parent.path(), &args)?;
        assert!(!output.status.success());
        assert!(!root.exists());
    }
    Ok(())
}
