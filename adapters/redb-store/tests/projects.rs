//! Project registrations preserve user folders, exact receipts and unresolved creation reservations.
use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{ProjectEnvironment, ProjectSettings, WorkflowId};
use milkdrift_persistence::{
    DaemonDisplayName, NativePathRoot, ProjectMutation, ProjectOperation, ProjectOperationState,
    ProjectRecord, ProjectStore, TimestampMillis, WorkflowCommit, WorkflowSnapshot, WorkflowStore,
};
use milkdrift_redb_store::RedbStore;
use std::path::Path;
fn root(path: &Path, identity: &str) -> NativePathRoot {
    NativePathRoot {
        requested: path.into(),
        physical: path.into(),
        identity: identity.into(),
        write: false,
    }
}
fn project(path: &Path, id: &str) -> Result<ProjectRecord, Box<dyn std::error::Error>> {
    Ok(ProjectRecord {
        id: id.into(),
        name: DaemonDisplayName::new("Existing source")?,
        generation: 1,
        folder: root(path, id),
        settings: ProjectSettings {
            environment: ProjectEnvironment::Host,
            allow_changes: false,
        },
    })
}
fn operation(key: &str) -> Result<ProjectOperation, Box<dyn std::error::Error>> {
    Ok(ProjectOperation {
        id: format!("operation:{key}"),
        actor: ActorRef::new("alice")?,
        key: key.into(),
        canonical_request: serde_json::to_vec(&serde_json::json!({"request_id":key}))?,
        state: ProjectOperationState::Accepted {
            project: None,
            workflow: None,
        },
    })
}
fn commit(
    workflow: &str,
    project_id: Option<String>,
    guard: Option<(String, u64)>,
) -> Result<WorkflowCommit, Box<dyn std::error::Error>> {
    Ok(WorkflowCommit {
        authoring: None,
        project_id,
        project_guard: guard,
        expected_generation: None,
        snapshot: WorkflowSnapshot {
            workflow: WorkflowId::new(workflow)?,
            generation: 1,
            title: "Earlier workflow".into(),
            description: "Accepted history is unchanged".into(),
            owner_actor: "alice".into(),
            updated_at: TimestampMillis::new(1),
            document: serde_json::json!({"original":"content"}),
            entered_invocations: vec![],
            managed_executions: vec![],
            artifact_capacity: 0,
            reserved_bytes: 0,
            reserved_receipts: 0,
        },
        command: None,
    })
}
#[test]
fn exact_replay_precedes_changed_defaults_and_registration_removal_preserves_source()
-> Result<(), Box<dyn std::error::Error>> {
    let db = tempfile::tempdir()?;
    let folder = tempfile::tempdir()?;
    std::fs::write(folder.path().join("Cargo.toml"), "original bytes")?;
    let store = RedbStore::open(db.path())?;
    let original = project(folder.path(), "project:a")?;
    let attach = operation("attach")?;
    let receipt =
        store.mutate_project(attach.clone(), ProjectMutation::Attach(original.clone()))?;
    let mut revised = original.clone();
    revised.generation = 2;
    revised.name = DaemonDisplayName::new("Renamed")?;
    revised.settings.allow_changes = true;
    store.mutate_project(
        operation("settings")?,
        ProjectMutation::Update {
            project: revised.clone(),
            expected_generation: 1,
        },
    )?;
    assert_eq!(
        store.mutate_project(attach.clone(), ProjectMutation::Attach(revised))?,
        receipt
    );
    let mut changed = attach;
    changed.canonical_request = serde_json::to_vec(&serde_json::json!({"different":true}))?;
    assert!(
        store
            .mutate_project(
                changed,
                ProjectMutation::Remove {
                    id: original.id.clone(),
                    expected_generation: 2
                }
            )
            .is_err()
    );
    assert!(
        store
            .project_operation(&ActorRef::new("bob")?, "attach")?
            .is_none()
    );
    store.mutate_project(
        operation("remove")?,
        ProjectMutation::Remove {
            id: original.id.clone(),
            expected_generation: 2,
        },
    )?;
    assert!(store.project(&original.id)?.is_none());
    assert_eq!(
        std::fs::read_to_string(folder.path().join("Cargo.toml"))?,
        "original bytes"
    );
    drop(store);
    let store = RedbStore::open(db.path())?;
    assert_eq!(
        store.project_operation(&ActorRef::new("alice")?, "attach")?,
        Some(receipt)
    );
    Ok(())
}
#[test]
fn aliases_share_registration_and_association_does_not_rewrite_history()
-> Result<(), Box<dyn std::error::Error>> {
    let db = tempfile::tempdir()?;
    let folder = tempfile::tempdir()?;
    let store = RedbStore::open(db.path())?;
    let original = project(folder.path(), "project:a")?;
    store.mutate_project(
        operation("attach")?,
        ProjectMutation::Attach(original.clone()),
    )?;
    let mut alias = original.clone();
    alias.id = "project:b".into();
    alias.folder.requested = folder.path().join("alias");
    assert!(
        store
            .mutate_project(operation("alias")?, ProjectMutation::Attach(alias))
            .is_err()
    );
    let old = commit("workflow:earlier", None, None)?;
    let snapshot = old.snapshot.clone();
    store.commit_workflow(old)?;
    assert!(store.project_for_workflow(&snapshot.workflow)?.is_none());
    store.mutate_project(
        operation("associate")?,
        ProjectMutation::Associate {
            id: original.id.clone(),
            expected_generation: 1,
            workflow: snapshot.workflow.clone(),
        },
    )?;
    assert_eq!(store.workflow(&snapshot.workflow)?, Some(snapshot));
    assert_eq!(
        store.project_for_workflow(&WorkflowId::new("workflow:earlier")?)?,
        Some(original.clone())
    );
    assert!(
        store
            .mutate_project(
                operation("remove")?,
                ProjectMutation::Remove {
                    id: original.id,
                    expected_generation: 1
                }
            )
            .is_err()
    );
    Ok(())
}
#[test]
fn stale_project_guard_refuses_new_workflow_and_association_atomically()
-> Result<(), Box<dyn std::error::Error>> {
    let db = tempfile::tempdir()?;
    let folder = tempfile::tempdir()?;
    let store = RedbStore::open(db.path())?;
    let mut value = project(folder.path(), "project:a")?;
    store.mutate_project(operation("attach")?, ProjectMutation::Attach(value.clone()))?;
    value.generation = 2;
    store.mutate_project(
        operation("rename")?,
        ProjectMutation::Update {
            project: value.clone(),
            expected_generation: 1,
        },
    )?;
    let mut proposed = commit(
        "workflow:new",
        Some(value.id.clone()),
        Some((value.id.clone(), 1)),
    )?;
    proposed.command = Some(milkdrift_persistence::WorkflowCommandAcceptance {
        actor: "alice".into(),
        key: "guarded-create".into(),
        reserved_action: None,
        canonical_request: br#"{"title":"guarded"}"#.to_vec(),
        result: serde_json::json!({"workflow":"workflow:new"}),
    });
    assert!(store.commit_workflow(proposed.clone()).is_err());
    assert!(store.workflow(&WorkflowId::new("workflow:new")?)?.is_none());
    assert!(store.workflow_receipt("alice", "guarded-create")?.is_none());
    assert!(
        store
            .project_for_workflow(&WorkflowId::new("workflow:new")?)?
            .is_none()
    );
    proposed.project_guard = Some((value.id.clone(), 2));
    store.commit_workflow(proposed.clone())?;
    assert_eq!(
        store.project_for_workflow(&WorkflowId::new("workflow:new")?)?,
        Some(value.clone())
    );
    value.generation = 3;
    store.mutate_project(
        operation("next-defaults")?,
        ProjectMutation::Update {
            project: value,
            expected_generation: 2,
        },
    )?;
    assert!(matches!(
        store.commit_workflow(proposed.clone())?,
        milkdrift_persistence::WorkflowCommitOutcome::Replay(_)
    ));
    let original = proposed.snapshot.document.clone();
    proposed.project_id = None;
    proposed.project_guard = None;
    proposed.command = None;
    proposed.expected_generation = Some(1);
    proposed.snapshot.generation = 2;
    proposed.snapshot.title = "Only the title changed".into();
    store.commit_workflow(proposed)?;
    assert_eq!(
        store
            .workflow(&WorkflowId::new("workflow:new")?)?
            .ok_or("workflow absent")?
            .document,
        original
    );
    Ok(())
}
#[test]
fn creation_intent_survives_restart_and_only_explicit_resolution_adopts_existing_child()
-> Result<(), Box<dyn std::error::Error>> {
    let db = tempfile::tempdir()?;
    let parent = tempfile::tempdir()?;
    let store = RedbStore::open(db.path())?;
    let child = parent.path().join("new-project");
    let mut intent = operation("create")?;
    intent.state = ProjectOperationState::Creating {
        parent: root(parent.path(), "parent"),
        folder_name: "new-project".into(),
        project: project(&child, "project:new")?,
    };
    store.mutate_project(intent.clone(), ProjectMutation::BeginCreate)?;
    std::fs::create_dir(&child)?;
    std::fs::write(child.join("external.txt"), "preserve")?;
    drop(store);
    let store = RedbStore::open(db.path())?;
    assert_eq!(
        store.project_operations(&ActorRef::new("alice")?)?,
        vec![intent.clone()]
    );
    assert_eq!(
        store.mutate_project(intent.clone(), ProjectMutation::BeginCreate)?,
        intent
    );
    assert!(store.project("project:new")?.is_none());
    let reconciliation = store.mutate_project(
        operation("reconcile")?,
        ProjectMutation::Reconcile {
            original_key: "create".into(),
            folder: Some(root(&child, "captured-child")),
        },
    )?;
    assert!(matches!(
        reconciliation.state,
        ProjectOperationState::Accepted { .. }
    ));
    assert!(
        store
            .project_operations(&ActorRef::new("alice")?)?
            .is_empty()
    );
    assert_eq!(
        std::fs::read_to_string(child.join("external.txt"))?,
        "preserve"
    );
    assert_eq!(
        store
            .project("project:new")?
            .ok_or("project missing")?
            .folder
            .identity,
        "captured-child"
    );
    Ok(())
}
#[test]
fn abandonment_preserves_directory_and_changed_reconciliation_cannot_replay()
-> Result<(), Box<dyn std::error::Error>> {
    let db = tempfile::tempdir()?;
    let parent = tempfile::tempdir()?;
    let store = RedbStore::open(db.path())?;
    let child = parent.path().join("unresolved");
    let mut intent = operation("create")?;
    intent.state = ProjectOperationState::Creating {
        parent: root(parent.path(), "parent"),
        folder_name: "unresolved".into(),
        project: project(&child, "project:new")?,
    };
    store.mutate_project(intent, ProjectMutation::BeginCreate)?;
    std::fs::create_dir(&child)?;
    let command = operation("abandon")?;
    let result = store.mutate_project(
        command.clone(),
        ProjectMutation::Reconcile {
            original_key: "create".into(),
            folder: None,
        },
    )?;
    assert_eq!(result.state, ProjectOperationState::Abandoned);
    assert!(child.is_dir());
    assert_eq!(
        store.mutate_project(
            command,
            ProjectMutation::Reconcile {
                original_key: "create".into(),
                folder: None
            }
        )?,
        result
    );
    assert!(store.project("project:new")?.is_none());
    Ok(())
}
#[cfg(feature = "test-admin")]
#[test]
fn before_and_after_commit_faults_recover_registration_without_duplicate_acceptance()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_redb_store::{FaultPoint, RedbStoreConfig};
    for point in [
        FaultPoint::BeforeProjectCommit,
        FaultPoint::AfterProjectCommit,
    ] {
        let db = tempfile::tempdir()?;
        let folder = tempfile::tempdir()?;
        drop(RedbStore::open(db.path())?);
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(db.path())
                .with_fault_injector(std::sync::Arc::new(fault::FailOnce::new(point))),
        )?;
        let project = project(folder.path(), "project:a")?;
        let command = operation("attach")?;
        assert!(
            store
                .mutate_project(command.clone(), ProjectMutation::Attach(project.clone()))
                .is_err()
        );
        drop(store);
        let store = RedbStore::open(db.path())?;
        assert_eq!(
            store.project(&project.id)?.is_some(),
            point == FaultPoint::AfterProjectCommit
        );
        store.mutate_project(command, ProjectMutation::Attach(project))?;
        assert_eq!(store.projects(None, 200)?.len(), 1);
    }
    Ok(())
}
#[cfg(feature = "test-admin")]
#[path = "support/fault.rs"]
mod fault;

#[cfg(feature = "test-admin")]
#[test]
fn creation_intent_and_post_effect_settlement_faults_never_invent_acceptance()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_redb_store::{FaultPoint, RedbStoreConfig};
    for settling in [false, true] {
        for point in [
            FaultPoint::BeforeProjectCommit,
            FaultPoint::AfterProjectCommit,
        ] {
            let db = tempfile::tempdir()?;
            let parent = tempfile::tempdir()?;
            let child = parent.path().join("new");
            let mut intent = operation("create")?;
            intent.state = ProjectOperationState::Creating {
                parent: root(parent.path(), "parent"),
                folder_name: "new".into(),
                project: project(&child, "project:new")?,
            };
            let store = RedbStore::open(db.path())?;
            if settling {
                store.mutate_project(intent.clone(), ProjectMutation::BeginCreate)?;
                std::fs::create_dir(&child)?;
                std::fs::write(child.join("keep"), "preserve")?;
            }
            drop(store);
            let store = RedbStore::open_with_config(
                RedbStoreConfig::new(db.path())
                    .with_fault_injector(std::sync::Arc::new(fault::FailOnce::new(point))),
            )?;
            if settling {
                assert!(
                    store
                        .settle_project_creation(
                            &ActorRef::new("alice")?,
                            "create",
                            Some(root(&child, "captured"))
                        )
                        .is_err()
                );
            } else {
                assert!(
                    store
                        .mutate_project(intent.clone(), ProjectMutation::BeginCreate)
                        .is_err()
                );
            }
            drop(store);
            let store = RedbStore::open(db.path())?;
            let saved = store.project_operation(&ActorRef::new("alice")?, "create")?;
            if !settling && point == FaultPoint::BeforeProjectCommit {
                assert!(saved.is_none());
                assert!(!child.exists());
            } else if settling && point == FaultPoint::AfterProjectCommit {
                assert!(matches!(
                    saved.ok_or("missing operation")?.state,
                    ProjectOperationState::Accepted { .. }
                ));
            } else {
                assert!(matches!(
                    saved.ok_or("missing intent")?.state,
                    ProjectOperationState::Creating { .. }
                ));
                assert!(store.project("project:new")?.is_none());
            }
            if settling {
                assert_eq!(std::fs::read_to_string(child.join("keep"))?, "preserve");
            }
        }
    }
    Ok(())
}
