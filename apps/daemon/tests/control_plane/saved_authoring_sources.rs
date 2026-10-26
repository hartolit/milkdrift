//! Retaining shared captured values does not turn new foreign uploads into owned input.
use super::support::*;
use milkdrift_control_protocol::authoring::{
    AuthoringDocumentDto, AuthoringSourceDto, AuthoringTaskDto, CheckpointAuthoringRequest,
};

fn editors(directory: &TempDir) -> TestResult<DaemonConfig> {
    let mut config = configuration_document_with_process_profiles(directory, 32, Vec::new())?;
    let editor = config.actors.get_mut(1).ok_or("second actor missing")?;
    editor.preset = AuthorityPresetConfig::Controller;
    editor.authority = ActorGrantConfig::dangerous_administrator();
    Ok(config)
}

async fn upload(client: &ControlClient, key: &str) -> TestResult<InputValue> {
    let receipt = client
        .upload_task_input(&TaskUploadRequest {
            request_id: key.into(),
            name: "shared.txt".into(),
            media_type: "text/plain".into(),
            text: "Captured once by the first editor".into(),
        })
        .await?;
    Ok(InputValue::Uploaded {
        input_id: receipt.input_id,
    })
}

async fn empty_workflow(client: &ControlClient, key: &str) -> TestResult<String> {
    let mut request = create_request(key, false);
    request.tasks.clear();
    request.finish_when_settled = false;
    Ok(client.create_workflow(&request).await?.workflow_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_editor_retains_exact_sources_but_cannot_introduce_foreign_uploads() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = start(
        editors(&directory)?.validate(directory.path())?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let other = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let workflow = empty_workflow(&daemon.client, "shared-authoring").await?;
    let captured = upload(&daemon.client, "first-capture").await?;
    let saved = daemon
        .client
        .checkpoint_workflow_authoring(
            &workflow,
            &CheckpointAuthoringRequest {
                expected_generation: 0,
                document: AuthoringDocumentDto {
                    sources: vec![AuthoringSourceDto {
                        id: "shared-source".into(),
                        label: "Original label".into(),
                        value: Some(captured.clone()),
                        shared_name: None,
                    }],
                    changes: vec![TaskChangeDto::BindInput {
                        name: "later-input".into(),
                        value: captured.clone(),
                    }],
                    ..AuthoringDocumentDto::default()
                },
            },
        )
        .await?;
    let mut observed = other.workflow_authoring(&workflow).await?;
    assert_eq!(observed, saved);
    observed
        .document
        .sources
        .first_mut()
        .ok_or("source missing")?
        .label = "Edited by the second person".into();
    let changed = other
        .checkpoint_workflow_authoring(
            &workflow,
            &CheckpointAuthoringRequest {
                expected_generation: observed.generation,
                document: observed.document.clone(),
            },
        )
        .await?;
    assert_eq!(changed.document, observed.document);
    assert_eq!(changed.generation, saved.generation + 1);
    assert_eq!(
        changed
            .document
            .sources
            .first()
            .ok_or("source missing")?
            .value,
        Some(captured)
    );

    let mut foreign = changed.document.clone();
    foreign.sources.push(AuthoringSourceDto {
        id: "new-foreign-source".into(),
        label: "Not retained here".into(),
        value: Some(upload(&daemon.client, "new-foreign-capture").await?),
        shared_name: None,
    });
    assert!(
        matches!(other.checkpoint_workflow_authoring(&workflow, &CheckpointAuthoringRequest {
        expected_generation: changed.generation, document: foreign,
    }).await, Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput)
    );
    assert_eq!(other.workflow_authoring(&workflow).await?, changed);
    assert_eq!(daemon.client.workflow(&workflow).await?.task_count, 0);
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_edit_can_retain_exact_accepted_input_at_its_current_version() -> TestResult {
    let directory = TempDir::new()?;
    let profile = captured_file_profile(&directory)?;
    let mut config = editors(&directory)?;
    config.adapters.process_profiles = vec![profile];
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let other = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let captured = upload(&daemon.client, "accepted-input-capture").await?;
    let mut request = create_request("accepted-source", false);
    request
        .tasks
        .first_mut()
        .ok_or("task missing")?
        .definition
        .inputs
        .insert(
            "file".into(),
            InputBinding::Inline {
                value: captured.clone(),
            },
        );
    let receipt = daemon.client.create_workflow(&request).await?;
    let task_id = receipt.aliases.get("inspect").ok_or("task alias missing")?;
    let task = other.task(&receipt.workflow_id, task_id).await?;
    assert_eq!(task.started_at, None);
    let mut document = AuthoringDocumentDto {
        tasks: vec![AuthoringTaskDto {
            id: task.id.clone(),
            version: Some(task.version + 1),
            definition: task.definition.clone(),
            title_edited: true,
            contract_generation: None,
        }],
        sources: vec![AuthoringSourceDto {
            id: format!("inline:{task_id}:file"),
            label: "Accepted file".into(),
            value: Some(captured),
            shared_name: None,
        }],
        ..AuthoringDocumentDto::default()
    };
    assert!(
        matches!(other.checkpoint_workflow_authoring(&receipt.workflow_id, &CheckpointAuthoringRequest {
        expected_generation: 0, document: document.clone(),
    }).await, Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput)
    );
    let candidate = document.tasks.first_mut().ok_or("candidate missing")?;
    candidate.version = Some(task.version);
    candidate.definition.instructions = "A future improvement, still unadmitted".into();
    let saved = other
        .checkpoint_workflow_authoring(
            &receipt.workflow_id,
            &CheckpointAuthoringRequest {
                expected_generation: 0,
                document: document.clone(),
            },
        )
        .await?;
    assert_eq!(saved.document, document);
    let unchanged = daemon.client.task(&receipt.workflow_id, task_id).await?;
    assert_eq!(unchanged.version, task.version);
    assert_eq!(unchanged.definition, task.definition);
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_source_cannot_be_read_or_retained_but_can_be_removed() -> TestResult {
    let directory = TempDir::new()?;
    let mut config = editors(&directory)?;
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let workflow = empty_workflow(&daemon.client, "source-revocation").await?;
    let saved = daemon
        .client
        .checkpoint_workflow_authoring(
            &workflow,
            &CheckpointAuthoringRequest {
                expected_generation: 0,
                document: AuthoringDocumentDto {
                    sources: vec![AuthoringSourceDto {
                        id: "private-source".into(),
                        label: "Private source".into(),
                        value: Some(upload(&daemon.client, "revoked-capture").await?),
                        shared_name: None,
                    }],
                    ..AuthoringDocumentDto::default()
                },
            },
        )
        .await?;
    assert_eq!(
        client(&daemon.endpoint, OBSERVER_TOKEN)?
            .workflow_authoring(&workflow)
            .await?,
        saved
    );
    daemon.stop().await?;
    let editor = config.actors.get_mut(1).ok_or("second actor missing")?;
    editor.authority.resources.artifacts = ArtifactAuthorityScope::none();
    editor.grant_revision += 1;
    editor.revocation_generation += 1;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let other = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(
        matches!(other.workflow_authoring(&workflow).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized)
    );
    assert!(
        matches!(other.checkpoint_workflow_authoring(&workflow, &CheckpointAuthoringRequest {
        expected_generation: saved.generation, document: saved.document.clone(),
    }).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized)
    );
    let removed = other
        .checkpoint_workflow_authoring(
            &workflow,
            &CheckpointAuthoringRequest {
                expected_generation: saved.generation,
                document: AuthoringDocumentDto::default(),
            },
        )
        .await?;
    assert_eq!(removed.generation, saved.generation + 1);
    assert_eq!(other.workflow_authoring(&workflow).await?, removed);
    assert_eq!(daemon.client.workflow_authoring(&workflow).await?, removed);
    daemon.stop().await
}
