//! Shared admission freezes reviewed project settings and keeps original request replay.
#![cfg(target_os = "linux")]
use super::support::*;
use milkdrift_control_protocol::{
    projects::{
        ProjectCommandDto, ProjectCommandRequest, ProjectEnvironmentDto, ProjectSettingsDto,
    },
    task_execution::{TaskExecutionDto, TaskProjectAccessDto},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_project_bindings_survive_settings_edits_and_exact_replay() -> TestResult {
    let directory = TempDir::new()?;
    let project_folder = directory.path().join("bound-project");
    fs::create_dir(&project_folder)?;
    let profile = configured_process_profile(&directory)?;
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&profile)?)?;
    *document
        .pointer_mut("/profile/working_directory")
        .ok_or("working directory missing")? = serde_json::json!({"type":"project"});
    fs::write(&profile, serde_json::to_vec(&document)?)?;
    let config = configuration_document_with_process_profiles(&directory, 32, vec![profile])?;
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let process_settings = ProjectSettingsDto {
        environment: ProjectEnvironmentDto::Process {
            capability: "golden-local-process".into(),
            operation: "process.execute".into(),
            generation: 2,
        },
        allow_changes: true,
    };
    let project = daemon
        .client
        .project_command(&ProjectCommandRequest {
            request_id: "task-project".into(),
            command: ProjectCommandDto::Attach {
                name: "Bound task project".into(),
                folder: project_folder.to_string_lossy().into_owned(),
                settings: process_settings.clone(),
            },
        })
        .await?
        .project
        .ok_or("project missing")?;
    let mut create = create_request("project-bound-work", false);
    create.project_id = Some(project.id.clone());
    create
        .tasks
        .first_mut()
        .ok_or("task missing")?
        .definition
        .execution = TaskExecutionDto::Project {
        expected_generation: 1,
    };
    let accepted = daemon.client.create_workflow(&create).await?;
    let task_id = accepted
        .aliases
        .get("inspect")
        .ok_or("accepted alias missing")?;
    let original = wait_task(
        &daemon.client,
        &accepted.workflow_id,
        task_id,
        TaskStateDto::Queued,
    )
    .await?;
    let TaskExecutionDto::Bound { binding } = &original.definition.execution else {
        return Err("accepted project task lacks binding".into());
    };
    assert_eq!(binding.generation, 1);
    assert_eq!(binding.access, TaskProjectAccessDto::Write);
    assert_eq!(
        binding.folder.physical,
        project_folder.canonicalize()?.to_string_lossy()
    );
    assert!(
        binding.tools.is_empty(),
        "direct process needs no model tool catalog"
    );

    daemon
        .client
        .project_command(&ProjectCommandRequest {
            request_id: "change-defaults".into(),
            command: ProjectCommandDto::Settings {
                project_id: project.id.clone(),
                expected_generation: 1,
                settings: ProjectSettingsDto {
                    environment: ProjectEnvironmentDto::Host,
                    allow_changes: false,
                },
            },
        })
        .await?;
    assert_eq!(daemon.client.create_workflow(&create).await?, accepted);
    let mut definition = original.definition.clone();
    definition.title = "Only the title changed".into();
    let edit = ChangeWorkflowRequest {
        request_id: "bound-title-edit".into(),
        expected_generation: daemon
            .client
            .workflow(&accepted.workflow_id)
            .await?
            .generation,
        changes: vec![TaskChangeDto::Revise {
            task: task_id.clone(),
            expected_version: original.version,
            definition,
        }],
    };
    let edited = daemon
        .client
        .change_workflow(&accepted.workflow_id, &edit)
        .await?;
    let retained = daemon.client.task(&accepted.workflow_id, task_id).await?;
    assert_eq!(retained.definition.execution, original.definition.execution);
    assert_eq!(retained.definition.paths, original.definition.paths);

    let mut stale = create.clone();
    stale.request_id = "stale-project-choice".into();
    let error = daemon
        .client
        .create_workflow(&stale)
        .await
        .err()
        .ok_or("stale choice accepted")?;
    assert!(matches!(error, ClientError::Api(error) if error.code == ErrorCode::Conflict));
    let mut explicit = create.clone();
    explicit.request_id = "explicit-current-choice".into();
    explicit
        .tasks
        .first_mut()
        .ok_or("task missing")?
        .definition
        .execution = TaskExecutionDto::Explicit {
        expected_generation: 2,
        settings: process_settings,
        access: TaskProjectAccessDto::Write,
        tools: Vec::new(),
    };
    let override_receipt = daemon.client.create_workflow(&explicit).await?;
    let overridden = daemon
        .client
        .task(
            &override_receipt.workflow_id,
            override_receipt
                .aliases
                .get("inspect")
                .ok_or("override alias missing")?,
        )
        .await?;
    assert!(matches!(overridden.definition.execution,
        TaskExecutionDto::Bound { binding } if binding.generation == 2
            && binding.access == TaskProjectAccessDto::Write));
    daemon.stop().await?;

    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    assert_eq!(daemon.client.create_workflow(&create).await?, accepted);
    assert_eq!(
        daemon
            .client
            .change_workflow(&accepted.workflow_id, &edit)
            .await?,
        edited
    );
    assert_eq!(
        daemon
            .client
            .task(&accepted.workflow_id, task_id)
            .await?
            .definition
            .execution,
        original.definition.execution
    );
    daemon.stop().await
}
