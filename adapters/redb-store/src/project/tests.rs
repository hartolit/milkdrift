//! Capacity reserves recovery before allowing any external directory effect.
use super::*;
use milkdrift_blueprint::{ProjectEnvironment, ProjectSettings};
use milkdrift_persistence::DaemonDisplayName;
fn operation(key: &str) -> Result<ProjectOperation, Box<dyn std::error::Error>> {
    Ok(ProjectOperation {
        id: format!("operation:{key}"),
        actor: ActorRef::new("alice")?,
        key: key.into(),
        canonical_request: serde_json::to_vec(&serde_json::json!({"request_id":key}))?,
        state: ProjectOperationState::Abandoned,
    })
}
#[test]
fn full_normal_capacity_preserves_reserved_creation_reconciliation_and_exact_replay()
-> Result<(), Box<dyn std::error::Error>> {
    let db = tempfile::tempdir()?;
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(db.path())?;
    let parent = NativePathRoot {
        requested: directory.path().into(),
        physical: directory.path().into(),
        identity: "parent".into(),
        write: true,
    };
    let child = directory.path().join("new");
    let project = ProjectRecord {
        id: "project:new".into(),
        name: DaemonDisplayName::new("New")?,
        generation: 1,
        folder: NativePathRoot {
            requested: child.clone(),
            physical: child.clone(),
            identity: "unresolved".into(),
            write: false,
        },
        settings: ProjectSettings {
            environment: ProjectEnvironment::Host,
            allow_changes: false,
        },
    };
    let mut intent = operation("create")?;
    intent.state = ProjectOperationState::Creating {
        parent,
        folder_name: "new".into(),
        project,
    };
    store.mutate_project(intent.clone(), ProjectMutation::BeginCreate)?;
    // Seed complete valid historical operations in one transaction; the production
    // admission below must honor the one slot already promised to this creation.
    let write = store.database().begin_write()?;
    {
        let mut table = write.open_table(PROJECT_OPERATIONS)?;
        for index in 1..(MAX_PROJECT_OPERATIONS - 1) {
            let historical = operation(&format!("history-{index}"))?;
            let bytes = json::encode(&historical, "project operation")?;
            table.insert(
                receipt_key(&historical.actor, &historical.key)?.as_str(),
                bytes.as_slice(),
            )?;
        }
    }
    write.commit()?;
    assert!(matches!(
        store.mutate_project(operation("new-command")?, ProjectMutation::BeginCreate),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::ResourceExhausted,
            ..
        })
    ));
    assert_eq!(
        store.mutate_project(intent.clone(), ProjectMutation::BeginCreate)?,
        intent
    );
    std::fs::create_dir(&child)?;
    std::fs::write(child.join("keep"), "still here")?;
    let request = operation("reconcile")?;
    let result = store.mutate_project(
        request.clone(),
        ProjectMutation::Reconcile {
            original_key: "create".into(),
            folder: Some(NativePathRoot {
                requested: child.clone(),
                physical: child.clone(),
                identity: "captured".into(),
                write: false,
            }),
        },
    )?;
    assert!(matches!(
        result.state,
        ProjectOperationState::Accepted { .. }
    ));
    assert_eq!(
        store.mutate_project(
            request,
            ProjectMutation::Reconcile {
                original_key: "create".into(),
                folder: None
            }
        )?,
        result
    );
    assert_eq!(std::fs::read_to_string(child.join("keep"))?, "still here");
    drop(store);
    let store = RedbStore::open(db.path())?;
    assert_eq!(
        store.project_operation(&ActorRef::new("alice")?, "reconcile")?,
        Some(result)
    );
    Ok(())
}
