//! Current task stores gain daemon metadata without rewriting their accepted work.
#![cfg(feature = "test-admin")]

use milkdrift_blueprint::WorkflowId;
use milkdrift_capability::PeerId;
use milkdrift_persistence::{
    DaemonDisplayName, DaemonMetadataStore, PeerExecutionStore, StorageAdmin, TimestampMillis,
    WorkflowCommandAcceptance, WorkflowCommit, WorkflowCommitOutcome, WorkflowSnapshot,
    WorkflowStore,
};
use milkdrift_redb_store::{FaultPoint, RedbStore, RedbStoreConfig};
use redb::{Database, ReadableTable, TableDefinition};
use std::path::Path;

const METADATA: TableDefinition<'static, &'static str, u64> =
    TableDefinition::new("milkdrift.v1.metadata");
const DAEMON_METADATA: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.daemon.metadata");
const DAEMON_RECEIPTS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.daemon.rename_receipts");

fn task_commit() -> Result<WorkflowCommit, Box<dyn std::error::Error>> {
    Ok(WorkflowCommit {
        expected_generation: None,
        snapshot: WorkflowSnapshot {
            workflow: WorkflowId::new("workflow:preserved")?,
            generation: 1,
            title: "Preserved task".into(),
            description: "Retained across metadata adoption".into(),
            owner_actor: "alice".into(),
            updated_at: TimestampMillis::new(1),
            document: serde_json::json!({"frozen_input":"preserve these exact bytes"}),
            entered_invocations: Vec::new(),
            managed_executions: Vec::new(),
            artifact_capacity: 0,
            reserved_bytes: 0,
            reserved_receipts: 0,
        },
        command: Some(WorkflowCommandAcceptance {
            actor: "alice".into(),
            key: "original-command".into(),
            reserved_action: None,
            canonical_request: serde_json::to_vec(&serde_json::json!({"task":"original"}))?,
            result: serde_json::json!({"workflow":"workflow:preserved"}),
        }),
    })
}

fn create_previous(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let store = RedbStore::open(root)?;
    store.bind_serving_host(&PeerId::new("host:original")?)?;
    store.commit_workflow(task_commit()?)?;
    drop(store);
    let db = Database::open(root.join("milkdrift.redb"))?;
    let write = db.begin_write()?;
    assert!(write.delete_table(DAEMON_METADATA)?);
    assert!(write.delete_table(DAEMON_RECEIPTS)?);
    write
        .open_table(METADATA)?
        .insert("storage_schema_version", 19)?;
    write.commit()?;
    assert_eq!(db.begin_read()?.list_tables()?.count(), 44);
    Ok(())
}

type TaskBytes = Vec<(String, Vec<(String, Vec<u8>)>)>;

fn task_bytes(root: &Path) -> Result<TaskBytes, Box<dyn std::error::Error>> {
    let db = Database::open(root.join("milkdrift.redb"))?;
    let read = db.begin_read()?;
    let mut result = Vec::new();
    for name in [
        "milkdrift.v1.tasks.workflows",
        "milkdrift.v1.tasks.receipts",
        "milkdrift.v1.tasks.accounting",
        "milkdrift.v1.tasks.output_usage",
    ] {
        let table = read.open_table(TableDefinition::<&str, &[u8]>::new(name))?;
        let mut rows = Vec::new();
        for row in table.iter()? {
            let (key, value) = row?;
            rows.push((key.value().to_owned(), value.value().to_vec()));
        }
        result.push((name.to_owned(), rows));
    }
    let mut rows = Vec::new();
    for row in read
        .open_table(TableDefinition::<&str, &str>::new(
            "milkdrift.v1.tasks.action_owners",
        ))?
        .iter()?
    {
        let (key, value) = row?;
        rows.push((key.value().to_owned(), value.value().as_bytes().to_vec()));
    }
    result.push(("milkdrift.v1.tasks.action_owners".to_owned(), rows));
    Ok(result)
}

#[test]
fn prompt03_adoption_preserves_task_bytes_receipts_and_installation()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    create_previous(root.path())?;
    let before = task_bytes(root.path())?;
    let store = RedbStore::open(root.path())?;
    assert_eq!(store.schema_info()?.stored_version, 20);
    let host = PeerId::new("host:original")?;
    assert_eq!(
        store
            .initialize_daemon_metadata(&host, &DaemonDisplayName::default())?
            .display_name
            .as_str(),
        "Milkdrift"
    );
    assert!(
        store
            .initialize_daemon_metadata(
                &PeerId::new("host:replacement")?,
                &DaemonDisplayName::default()
            )
            .is_err()
    );
    assert_eq!(
        store.workflow(&WorkflowId::new("workflow:preserved")?)?,
        Some(task_commit()?.snapshot)
    );
    assert!(matches!(
        store.commit_workflow(task_commit()?)?,
        WorkflowCommitOutcome::Replay(_)
    ));
    drop(store);
    assert_eq!(task_bytes(root.path())?, before);
    Ok(())
}

#[test]
fn interrupted_adoption_is_wholly_old_or_new_and_reopens_without_losing_tasks()
-> Result<(), Box<dyn std::error::Error>> {
    for point in [
        FaultPoint::BeforeDaemonSchemaAdoptionCommit,
        FaultPoint::AfterDaemonSchemaAdoptionCommit,
    ] {
        let root = tempfile::tempdir()?;
        create_previous(root.path())?;
        let before = task_bytes(root.path())?;
        assert!(
            RedbStore::open_with_config(
                RedbStoreConfig::new(root.path())
                    .with_fault_injector(std::sync::Arc::new(fault::FailOnce::new(point)))
            )
            .is_err()
        );
        {
            let db = Database::open(root.path().join("milkdrift.redb"))?;
            let read = db.begin_read()?;
            let expected = if point == FaultPoint::BeforeDaemonSchemaAdoptionCommit {
                (19, 44)
            } else {
                (20, 46)
            };
            assert_eq!(
                read.open_table(METADATA)?
                    .get("storage_schema_version")?
                    .ok_or("missing marker")?
                    .value(),
                expected.0
            );
            assert_eq!(read.list_tables()?.count(), expected.1);
        }
        let store = RedbStore::open(root.path())?;
        assert_eq!(store.schema_info()?.stored_version, 20);
        assert!(
            store
                .workflow_receipt("alice", "original-command")?
                .is_some()
        );
        drop(store);
        assert_eq!(task_bytes(root.path())?, before);
    }
    Ok(())
}

#[test]
fn malformed_previous_tables_or_accounting_are_refused_before_source_writes()
-> Result<(), Box<dyn std::error::Error>> {
    for mutation in [
        "extra",
        "partial-adoption",
        "missing",
        "wrong-type",
        "accounting",
        "old-document",
    ] {
        let root = tempfile::tempdir()?;
        create_previous(root.path())?;
        let path = root.path().join("milkdrift.redb");
        let db = Database::open(&path)?;
        let write = db.begin_write()?;
        match mutation {
            "extra" => {
                drop(write.open_table(TableDefinition::<&str, u64>::new("unexpected"))?);
            }
            "partial-adoption" => {
                drop(write.open_table(DAEMON_METADATA)?);
            }
            "missing" => {
                assert!(write.delete_table(TableDefinition::<&str, &[u8]>::new(
                    "milkdrift.v1.tasks.workflows"
                ))?);
            }
            "wrong-type" => {
                assert!(write.delete_table(TableDefinition::<&str, &[u8]>::new(
                    "milkdrift.v1.tasks.workflows"
                ))?);
                drop(write.open_table(TableDefinition::<u64, u64>::new(
                    "milkdrift.v1.tasks.workflows",
                ))?);
            }
            "accounting" => {
                write
                    .open_table(TableDefinition::<&str, &[u8]>::new(
                        "milkdrift.v1.tasks.accounting",
                    ))?
                    .insert("global", b"invalid".as_slice())?;
            }
            "old-document" => {
                write
                    .open_table(METADATA)?
                    .insert("internal_document_format_version", 21)?;
            }
            _ => return Err("unknown mutation".into()),
        }
        write.commit()?;
        drop(db);
        let bytes = std::fs::read(&path)?;
        let modified = std::fs::metadata(&path)?.modified()?;
        assert!(RedbStore::open(root.path()).is_err(), "accepted {mutation}");
        assert_eq!(std::fs::read(&path)?, bytes, "modified {mutation}");
        assert_eq!(std::fs::metadata(&path)?.modified()?, modified);
    }
    Ok(())
}

#[path = "support/fault.rs"]
mod fault;
