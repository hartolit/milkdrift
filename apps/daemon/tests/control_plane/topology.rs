//! Public topology is bounded, authority-aware and independent of instruction/result reads.

use super::support::*;
use milkdrift_control_protocol::tasks::{
    TaskDependenciesDto, TaskOrderConditionDto, TaskOutputDeclarationDto,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topology_preserves_data_and_order_meaning_without_task_text() -> TestResult {
    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let daemon = start(
        configuration_with_process_profiles(&directory, 32, vec![profile])?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let mut request = create_request("topology-meaning", false);
    let producer = request.tasks.first_mut().ok_or("task absent")?;
    producer.definition.instructions = "Instruction text must not enter a topology response".into();
    producer.definition.outputs.insert(
        "report".into(),
        TaskOutputDeclarationDto {
            required: false,
            media_type: Some("text/plain".into()),
            max_bytes: 1024,
        },
    );
    let mut consumer = task_definition("golden-local-process", "process.execute");
    consumer.inputs.insert(
        "report".into(),
        InputBinding::TaskOutput {
            task: "inspect".into(),
            output: "report".into(),
            optional: true,
        },
    );
    consumer.dependencies = TaskDependenciesDto::Ordered(BTreeMap::from([(
        "inspect".into(),
        TaskOrderConditionDto::Success,
    )]));
    request.tasks.push(NewTaskDto {
        alias: "consume".into(),
        definition: consumer,
    });
    for index in 0..6 {
        let mut definition = task_definition("golden-local-process", "process.execute");
        definition.inputs.insert(
            "note".into(),
            InputBinding::Inline {
                value: InputValue::Text {
                    text: "a".repeat(16_384),
                },
            },
        );
        request.tasks.push(NewTaskDto {
            alias: format!("note-{index}"),
            definition,
        });
    }
    let receipt = daemon.client.create_workflow(&request).await?;
    let topology = daemon
        .client
        .workflow_topology(&receipt.workflow_id, None)
        .await?;
    assert!(!topology.truncated);
    let consumer = topology
        .tasks
        .iter()
        .find(|task| Some(&task.id) == receipt.aliases.get("consume"))
        .ok_or("consumer missing")?;
    let producer = receipt.aliases.get("inspect").ok_or("producer missing")?;
    assert_eq!(
        consumer.dependencies.get(producer),
        Some(&TaskOrderConditionDto::Success)
    );
    assert!(!consumer.dependencies_legacy);
    let input = consumer.inputs.get("report").ok_or("data edge missing")?;
    assert_eq!(input.source, *producer);
    assert_eq!(input.output.as_deref(), Some("report"));
    assert!(input.optional);
    assert_eq!(
        consumer
            .inputs
            .get("brief")
            .ok_or("shared edge missing")?
            .source,
        "supplied:brief"
    );
    assert_eq!(
        consumer
            .inputs
            .get("brief")
            .and_then(|input| input.preview.as_ref())
            .map(|preview| preview.availability.as_str()),
        Some("missing")
    );
    let mut preview_bytes = 0usize;
    for input in topology.tasks.iter().flat_map(|task| task.inputs.values()) {
        if let Some(preview) = &input.preview {
            preview_bytes = preview_bytes.saturating_add(serde_json::to_vec(preview)?.len());
        }
    }
    assert!(preview_bytes <= 65_536);
    assert!(preview_bytes > 32_768);
    let value = serde_json::to_string(&topology)?;
    assert!(!value.contains("Instruction text must not enter"));
    assert!(!value.contains("\"instructions\""));
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topology_paging_binds_workflow_generation_actor_and_daemon() -> TestResult {
    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let daemon = start(
        configuration_with_process_profiles(&directory, 32, vec![profile])?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let mut request = create_request("topology-pages", false);
    request.tasks = (0..32)
        .map(|index| NewTaskDto {
            alias: format!("task-{index}"),
            definition: task_definition("golden-local-process", "process.execute"),
        })
        .collect();
    let created = daemon.client.create_workflow(&request).await?;
    let workflow = &created.workflow_id;
    let generation = daemon.client.workflow(workflow).await?.generation;
    let mut receipt = daemon
        .client
        .change_workflow(
            workflow,
            &ChangeWorkflowRequest {
                request_id: "topology-pause".into(),
                expected_generation: generation,
                changes: vec![TaskChangeDto::Wait {
                    question: "Keep these tasks unstarted".into(),
                }],
            },
        )
        .await?;
    for start in (32..201).step_by(32) {
        let changes = (start..(start + 32).min(201))
            .map(|index| {
                let mut definition = task_definition("golden-local-process", "process.execute");
                // The workflow is paused. Avoid reserving a maximum-sized future shared input
                // for every row: this case exercises topology paging, not input capacity.
                definition.inputs.clear();
                TaskChangeDto::Add {
                    alias: format!("task-{index}"),
                    definition,
                }
            })
            .collect();
        receipt = daemon
            .client
            .change_workflow(
                workflow,
                &ChangeWorkflowRequest {
                    request_id: format!("topology-add-{start}"),
                    expected_generation: receipt.generation,
                    changes,
                },
            )
            .await?;
    }
    let first = daemon.client.workflow_topology(workflow, None).await?;
    assert_eq!(first.tasks.len(), 200);
    assert!(first.truncated);
    let cursor = first.next_cursor.as_deref().ok_or("cursor missing")?;
    let last = daemon
        .client
        .workflow_topology(workflow, Some(cursor))
        .await?;
    assert_eq!(last.tasks.len(), 1);
    assert!(!last.truncated);
    assert!(last.next_cursor.is_none());
    assert_eq!(first.workflow_generation, last.workflow_generation);
    let identities: std::collections::BTreeSet<_> = first
        .tasks
        .iter()
        .chain(&last.tasks)
        .map(|task| &task.id)
        .collect();
    assert_eq!(identities.len(), 201);
    assert!(serde_json::to_vec(&first)?.len() < 2 * 1024 * 1024);
    let other = daemon
        .client
        .create_workflow(&create_request("topology-other", false))
        .await?;
    assert!(
        daemon
            .client
            .workflow_topology(&other.workflow_id, Some(cursor))
            .await
            .is_err()
    );
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(
        observer
            .workflow_topology(workflow, Some(cursor))
            .await
            .is_err()
    );
    let raw = reqwest::Client::new()
        .get(
            daemon
                .endpoint
                .join(&format!("v1/workflows/{workflow}/topology"))?,
        )
        .header("x-milkdrift-daemon", "another-installation")
        .bearer_auth(CONTROLLER_TOKEN)
        .send()
        .await?;
    assert!(raw.status().is_client_error());
    daemon
        .client
        .change_workflow(
            workflow,
            &ChangeWorkflowRequest {
                request_id: "topology-rename".into(),
                expected_generation: receipt.generation,
                changes: vec![TaskChangeDto::Rename {
                    title: "New title".into(),
                    description: "".into(),
                }],
            },
        )
        .await?;
    assert!(
        matches!(daemon.client.workflow_topology(workflow, Some(cursor)).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topology_source_previews_recheck_current_artifact_sensitivity() -> TestResult {
    let directory = TempDir::new()?;
    let profile = captured_file_profile(&directory)?;
    let mut config = configuration_document_with_process_profiles(&directory, 32, vec![profile])?;
    let observer = config.actors.get_mut(1).ok_or("observer actor absent")?;
    observer.authority.resources.workflow_run = WorkflowRunScope::Any;
    observer.authority.dangerous_allow_broad_authority = true;
    observer.authority.resources.artifacts = ArtifactAuthorityScope::new(
        milkdrift_authority::Selection::any(),
        std::collections::BTreeSet::from([milkdrift_workspace::ArtifactSensitivity::Public]),
    )?;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let upload = daemon
        .client
        .upload_task_input(&TaskUploadRequest {
            request_id: "topology-sensitive".into(),
            name: "private.txt".into(),
            media_type: "text/plain".into(),
            text: "Private source must remain private".into(),
        })
        .await?;
    let mut request = create_request("topology-private-source", false);
    request
        .tasks
        .first_mut()
        .ok_or("task absent")?
        .definition
        .inputs
        .insert(
            "file".into(),
            InputBinding::Supplied {
                name: "file".into(),
                optional: false,
            },
        );
    request.inputs.insert(
        "file".into(),
        InputValue::Uploaded {
            input_id: upload.input_id.clone(),
        },
    );
    let receipt = daemon.client.create_workflow(&request).await?;
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let page = observer
        .workflow_topology(&receipt.workflow_id, None)
        .await?;
    let preview = page
        .tasks
        .first()
        .ok_or("task absent")?
        .inputs
        .get("file")
        .ok_or("source edge absent")?
        .preview
        .as_ref()
        .ok_or("availability absent")?;
    assert_eq!(preview.availability, "restricted");
    assert!(preview.value.is_none());
    let body = serde_json::to_string(&page)?;
    assert!(!body.contains("Private source must remain private"));
    assert!(!body.contains(&upload.input_id));
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topology_returns_named_output_metadata_without_releasing_result_text() -> TestResult {
    let directory = TempDir::new()?;
    let profile = captured_file_profile(&directory)?;
    let daemon = start(
        configuration_with_process_profiles(&directory, 32, vec![profile])?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let upload = daemon
        .client
        .upload_task_input(&TaskUploadRequest {
            request_id: "topology-result-source".into(),
            name: "source.txt".into(),
            media_type: "text/plain".into(),
            text: "Retained result body must stay out of topology".into(),
        })
        .await?;
    let mut request = create_request("topology-result", true);
    let definition = &mut request.tasks.first_mut().ok_or("task absent")?.definition;
    definition.inputs.insert(
        "file".into(),
        InputBinding::Supplied {
            name: "file".into(),
            optional: false,
        },
    );
    definition.outputs.insert(
        "report".into(),
        TaskOutputDeclarationDto {
            required: true,
            media_type: Some("text/plain".into()),
            max_bytes: 1024,
        },
    );
    request.inputs.insert(
        "file".into(),
        InputValue::Uploaded {
            input_id: upload.input_id,
        },
    );
    let receipt = daemon.client.create_workflow(&request).await?;
    let task = receipt.aliases.get("inspect").ok_or("task missing")?;
    wait_task(
        &daemon.client,
        &receipt.workflow_id,
        task,
        TaskStateDto::Completed,
    )
    .await?;
    let generation = daemon
        .client
        .workflow(&receipt.workflow_id)
        .await?
        .generation;
    daemon
        .client
        .change_workflow(
            &receipt.workflow_id,
            &ChangeWorkflowRequest {
                request_id: "topology-new-source".into(),
                expected_generation: generation,
                changes: vec![TaskChangeDto::BindInput {
                    name: "file".into(),
                    value: InputValue::Text {
                        text: "New value for future work".into(),
                    },
                }],
            },
        )
        .await?;
    let page = daemon
        .client
        .workflow_topology(&receipt.workflow_id, None)
        .await?;
    let task = page.tasks.first().ok_or("task missing")?;
    let output = task
        .published_outputs
        .get("report")
        .ok_or("named publication missing")?;
    assert!(output.available);
    assert!(output.artifact_id.is_some());
    assert_eq!(output.media_type.as_deref(), Some("text/plain"));
    assert!(output.bytes > 0);
    let text = serde_json::to_string(&page)?;
    assert!(!text.contains("Retained result body must stay out"));
    assert!(text.contains("New value for future work"));
    daemon.stop().await
}
