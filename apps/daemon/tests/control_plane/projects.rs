//! Public project commands preserve files and recover durable creation intent across reconnect.
use super::support::*;
use milkdrift_control_protocol::projects::{
    ProjectCommandDto, ProjectCommandRequest, ProjectEnvironmentDto, ProjectSettingsDto,
};
fn settings(write: bool) -> ProjectSettingsDto {
    ProjectSettingsDto {
        environment: ProjectEnvironmentDto::Host,
        allow_changes: write,
    }
}
fn command(key: &str, command: ProjectCommandDto) -> ProjectCommandRequest {
    ProjectCommandRequest {
        request_id: key.into(),
        command,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_files_and_their_ancestors_are_protected_but_sibling_projects_remain_usable()
-> TestResult {
    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let mut config = configuration_document_with_process_profiles(&directory, 32, vec![])?;
    let configuration = directory.path().join("configuration");
    let credentials = directory.path().join("credentials");
    let profiles = directory.path().join("profiles");
    for folder in [&configuration, &credentials, &profiles] {
        fs::create_dir(folder)?;
    }
    let credential = credentials.join("controller.token");
    fs::rename(directory.path().join("controller.token"), &credential)?;
    config.secret_sources.insert(
        "credential:controller".into(),
        SecretSourceConfig::File { path: credential },
    );
    let protected_profile = profiles.join("process.json");
    fs::copy(profile, &protected_profile)?;
    config.adapters.process_profiles.push(protected_profile);
    let source = configuration.join("daemon.toml");
    fs::write(&source, toml::to_string(&config)?)?;
    let daemon = start(DaemonConfig::load(&source)?, CONTROLLER_TOKEN).await?;
    for (index, folder) in [
        configuration.as_path(),
        credentials.as_path(),
        profiles.as_path(),
        directory.path(),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            daemon
                .client
                .project_command(&command(
                    &format!("protected-file-{index}"),
                    ProjectCommandDto::Attach {
                        name: "Forbidden operator files".into(),
                        folder: folder.to_string_lossy().into_owned(),
                        settings: settings(false),
                    },
                ))
                .await
                .is_err()
        );
    }
    // Directory ancestry is precise: a sibling does not expose the protected file.
    let sibling = configuration.join("project");
    fs::create_dir(&sibling)?;
    let project = daemon
        .client
        .project_command(&command(
            "safe-sibling",
            ProjectCommandDto::Attach {
                name: "Sibling project".into(),
                folder: sibling.to_string_lossy().into_owned(),
                settings: settings(false),
            },
        ))
        .await?
        .project
        .ok_or("sibling project missing")?;
    assert_eq!(project.availability, "available");
    assert_eq!(
        fs::read_to_string(credentials.join("controller.token"))?,
        CONTROLLER_TOKEN
    );
    daemon.stop().await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn root_directory_choices_respect_page_size_and_actor_bound_continuations() -> TestResult {
    use milkdrift_authority::{AccessMode, FilesystemScope};
    let directory = TempDir::new()?;
    let roots = TempDir::new()?;
    let first = roots.path().join("first");
    let second = roots.path().join("second");
    fs::create_dir(&first)?;
    fs::create_dir(&second)?;
    let mut config = configuration_document_with_process_profiles(&directory, 32, vec![])?;
    config
        .actors
        .first_mut()
        .ok_or("controller absent")?
        .authority
        .resources
        .filesystem = [&first, &second]
        .into_iter()
        .map(|root| {
            FilesystemScope::from_canonical_host_path(
                root,
                std::collections::BTreeSet::from([AccessMode::Read]),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let first_page = daemon.client.project_directories(None, None, 1).await?;
    assert_eq!(first_page.items.len(), 1);
    let cursor = first_page
        .next_cursor
        .as_deref()
        .ok_or("next root page absent")?;
    let second_page = daemon
        .client
        .project_directories(None, Some(cursor), 1)
        .await?;
    assert_eq!(second_page.items.len(), 1);
    assert_ne!(
        first_page.items.first().ok_or("first root absent")?.path,
        second_page.items.first().ok_or("second root absent")?.path
    );
    assert!(second_page.next_cursor.is_none());
    assert!(
        client(&daemon.endpoint, OBSERVER_TOKEN)?
            .project_directories(None, Some(cursor), 1)
            .await
            .is_err()
    );
    daemon.stop().await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_visibility_redacts_project_execution_without_misclassifying_earlier_work()
-> TestResult {
    use milkdrift_control_protocol::task_execution::TaskExecutionDto;
    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let mut config = configuration_document_with_process_profiles(&directory, 32, vec![profile])?;
    let observer = config.actors.get_mut(1).ok_or("observer absent")?;
    observer.preset = AuthorityPresetConfig::Controller;
    observer.authority.resources.workflow_run = WorkflowRunScope::Any;
    observer.authority.dangerous_allow_broad_authority = true;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let request = create_request("private-project-task", false);
    let project = request.project_id.clone().ok_or("project absent")?;
    let accepted = daemon.client.create_workflow(&request).await?;
    let task_id = accepted.aliases.get("inspect").ok_or("task absent")?;
    let reader = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let workflow = reader.workflow(&accepted.workflow_id).await?;
    assert!(workflow.project_id.is_none());
    let task = reader.task(&accepted.workflow_id, task_id).await?;
    assert_eq!(task.definition.execution, TaskExecutionDto::Restricted);
    assert!(task.definition.paths.is_empty());
    assert!(task.definition.tools.is_empty());
    assert_eq!(
        task.definition.instructions,
        request
            .tasks
            .first()
            .ok_or("request task absent")?
            .definition
            .instructions
    );
    let discovered = reader.workflows("", None, 50).await?;
    assert!(
        discovered
            .items
            .iter()
            .any(|workflow| workflow.id == accepted.workflow_id && workflow.project_id.is_none())
    );
    assert!(
        reader
            .workflows_in_project("", None, 50, None, true)
            .await?
            .items
            .is_empty()
    );
    assert!(
        reader
            .workflows_in_project("", None, 50, Some(&project), false)
            .await
            .is_err()
    );
    let reused = reader
        .change_workflow(
            &accepted.workflow_id,
            &ChangeWorkflowRequest {
                request_id: "restricted-projection-reuse".into(),
                expected_generation: workflow.generation,
                changes: vec![TaskChangeDto::Revise {
                    task: task_id.clone(),
                    expected_version: task.version,
                    definition: task.definition,
                }],
            },
        )
        .await;
    assert!(matches!(reused,Err(ClientError::Api(error)) if error.code==ErrorCode::InvalidInput));
    daemon.stop().await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projects_persist_shared_settings_creation_and_actor_filtered_discovery() -> TestResult {
    let directory = TempDir::new()?;
    let folders = TempDir::new()?;
    let existing = folders.path().join("existing");
    std::fs::create_dir(&existing)?;
    std::fs::write(existing.join("source.rs"), "fn main() {}")?;
    let config = configuration_document_with_process_profiles(&directory, 32, vec![])?;
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let attach = command(
        "attach-project",
        ProjectCommandDto::Attach {
            name: "Existing".into(),
            folder: existing.to_string_lossy().into_owned(),
            settings: settings(false),
        },
    );
    let accepted = daemon.client.project_command(&attach).await?;
    let project = accepted.project.clone().ok_or("missing project")?;
    assert_eq!(project.generation, 1);
    assert!(!project.settings.allow_changes);
    let create = command(
        "create-project",
        ProjectCommandDto::Create {
            name: "New".into(),
            parent: folders.path().to_string_lossy().into_owned(),
            folder_name: "new".into(),
            settings: settings(false),
        },
    );
    let created = daemon.client.project_command(&create).await?;
    assert_eq!(created.status, "accepted");
    assert!(folders.path().join("new").is_dir());
    assert_eq!(daemon.client.project_command(&create).await?, created);
    let first_page = daemon.client.projects(None, 1).await?;
    assert_eq!(first_page.items.len(), 1);
    assert!(first_page.next_cursor.is_some());
    let first_folders = daemon
        .client
        .project_directories(folders.path().to_str(), None, 1)
        .await?;
    assert_eq!(first_folders.items.len(), 1);
    let second_folders = daemon
        .client
        .project_directories(
            folders.path().to_str(),
            first_folders.next_cursor.as_deref(),
            1,
        )
        .await?;
    assert_eq!(second_folders.items.len(), 1);
    assert_ne!(
        first_folders
            .items
            .first()
            .ok_or("first folder missing")?
            .path,
        second_folders
            .items
            .first()
            .ok_or("second folder missing")?
            .path
    );
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(observer.projects(None, 50).await?.items.is_empty());
    assert!(observer.project(&project.id).await.is_err());
    assert!(
        observer
            .project_directories(None, None, 50)
            .await?
            .items
            .is_empty()
    );
    let updated = daemon
        .client
        .project_command(&command(
            "project-settings",
            ProjectCommandDto::Settings {
                project_id: project.id.clone(),
                expected_generation: 1,
                settings: settings(true),
            },
        ))
        .await?;
    assert_eq!(
        updated
            .project
            .as_ref()
            .ok_or("settings missing")?
            .generation,
        2
    );
    assert!(
        daemon
            .client
            .project_command(&command(
                "stale-settings",
                ProjectCommandDto::Rename {
                    project_id: project.id.clone(),
                    expected_generation: 1,
                    name: "Stale".into()
                }
            ))
            .await
            .is_err()
    );
    assert_eq!(daemon.client.project_command(&attach).await?, accepted);
    daemon.stop().await?;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    assert!(
        daemon
            .client
            .project(&project.id)
            .await?
            .settings
            .allow_changes
    );
    assert_eq!(
        daemon.client.project_receipt("attach-project").await?,
        accepted
    );
    assert_eq!(
        std::fs::read_to_string(existing.join("source.rs"))?,
        "fn main() {}"
    );
    daemon
        .client
        .project_command(&command(
            "remove-project",
            ProjectCommandDto::Remove {
                project_id: project.id,
                expected_generation: 2,
            },
        ))
        .await?;
    assert!(existing.join("source.rs").is_file());
    daemon.stop().await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn existing_child_is_never_implicitly_adopted_and_explicit_recovery_preserves_bytes()
-> TestResult {
    let directory = TempDir::new()?;
    let folders = TempDir::new()?;
    let child = folders.path().join("existing-child");
    std::fs::create_dir(&child)?;
    std::fs::write(child.join("keep"), "untouched")?;
    let daemon = start(configuration(&directory, 32)?, CONTROLLER_TOKEN).await?;
    let request = command(
        "uncertain-create",
        ProjectCommandDto::Create {
            name: "Recovered".into(),
            parent: folders.path().to_string_lossy().into_owned(),
            folder_name: "existing-child".into(),
            settings: settings(false),
        },
    );
    let unresolved = daemon.client.project_command(&request).await?;
    assert_eq!(unresolved.status, "unresolved");
    assert!(unresolved.project.is_none());
    assert_eq!(unresolved.operation_folder.as_deref(), child.to_str());
    assert_eq!(daemon.client.project_command(&request).await?, unresolved);
    assert_eq!(daemon.client.project_operations().await?, vec![unresolved]);
    let reconciled = daemon
        .client
        .project_command(&command(
            "reconcile-create",
            ProjectCommandDto::Reconcile {
                operation_id: "uncertain-create".into(),
            },
        ))
        .await?;
    assert_eq!(reconciled.status, "accepted");
    assert!(daemon.client.project_operations().await?.is_empty());
    assert_eq!(std::fs::read_to_string(child.join("keep"))?, "untouched");
    assert!(
        daemon
            .client
            .project_command(&command(
                "traversal",
                ProjectCommandDto::Create {
                    name: "Bad".into(),
                    parent: folders.path().to_string_lossy().into_owned(),
                    folder_name: "../escape".into(),
                    settings: settings(false)
                }
            ))
            .await
            .is_err()
    );
    daemon.stop().await
}
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aliases_conflict_replaced_roots_become_unavailable_and_storage_is_protected() -> TestResult
{
    let directory = TempDir::new()?;
    let folders = TempDir::new()?;
    let original = folders.path().join("project");
    std::fs::create_dir(&original)?;
    let alias = folders.path().join("alias");
    std::os::unix::fs::symlink(&original, &alias)?;
    let daemon = start(configuration(&directory, 32)?, CONTROLLER_TOKEN).await?;
    let attached = daemon
        .client
        .project_command(&command(
            "original",
            ProjectCommandDto::Attach {
                name: "Original".into(),
                folder: original.to_string_lossy().into_owned(),
                settings: settings(false),
            },
        ))
        .await?
        .project
        .ok_or("project missing")?;
    assert!(
        daemon
            .client
            .project_command(&command(
                "alias",
                ProjectCommandDto::Attach {
                    name: "Alias".into(),
                    folder: alias.to_string_lossy().into_owned(),
                    settings: settings(false)
                }
            ))
            .await
            .is_err()
    );
    assert!(
        daemon
            .client
            .project_command(&command(
                "store",
                ProjectCommandDto::Attach {
                    name: "Forbidden".into(),
                    folder: directory.path().join("data").to_string_lossy().into_owned(),
                    settings: settings(false)
                }
            ))
            .await
            .is_err()
    );
    std::fs::rename(&original, folders.path().join("moved"))?;
    std::fs::create_dir(&original)?;
    assert_ne!(
        daemon.client.project(&attached.id).await?.availability,
        "available"
    );
    assert!(
        daemon
            .client
            .project_command(&command(
                "unavailable-settings",
                ProjectCommandDto::Settings {
                    project_id: attached.id,
                    expected_generation: 1,
                    settings: settings(true)
                }
            ))
            .await
            .is_err()
    );
    daemon.stop().await
}
