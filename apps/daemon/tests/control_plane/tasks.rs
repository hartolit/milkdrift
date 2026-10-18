//! Ordinary task admission, prospective edits, frozen inputs and durable discovery.
use super::support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_source_sensitivity_is_restricted_without_weakening_artifact_lookup_masking()
-> TestResult {
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
            request_id: "sensitive-captured-source".into(),
            name: "private.txt".into(),
            media_type: "text/plain".into(),
            text: "private captured bytes".into(),
        })
        .await?;
    let mut request = create_request("sensitive-shared-source", false);
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
    assert!(
        matches!(observer.artifact_metadata(&upload.input_id).await, Err(ClientError::Api(error)) if error.code == ErrorCode::NotFound)
    );
    let workflow = observer.workflow(&receipt.workflow_id).await?;
    let source = workflow
        .supplied_inputs
        .get("file")
        .ok_or("source missing")?;
    assert_eq!(source.availability, "restricted");
    assert!(source.value.is_none());
    let task = observer
        .task(
            &receipt.workflow_id,
            receipt.aliases.get("inspect").ok_or("task missing")?,
        )
        .await?;
    assert_eq!(task.supplied_inputs.get("file"), Some(source));
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_choices_expose_only_current_read_write_grants() -> TestResult {
    use milkdrift_authority::{AccessMode, FilesystemScope};
    use std::collections::BTreeSet;
    let directory = TempDir::new()?;
    let project = directory.path().join("read-only-project");
    let executable = directory.path().join("execute-only");
    fs::create_dir(&project)?;
    fs::create_dir(&executable)?;
    let mut config = configuration_document_with_process_profiles(&directory, 32, vec![])?;
    config
        .actors
        .first_mut()
        .ok_or("controller actor absent")?
        .authority
        .resources
        .filesystem = vec![
        FilesystemScope::from_canonical_host_path(
            &project.canonicalize()?,
            BTreeSet::from([AccessMode::Read]),
        )?,
        FilesystemScope::from_canonical_host_path(
            &executable.canonicalize()?,
            BTreeSet::from([AccessMode::Execute]),
        )?,
    ];
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let session = daemon.client.task_session().await?;
    assert_eq!(session.project_roots.len(), 1);
    let root = session.project_roots.first().ok_or("project root absent")?;
    assert_eq!(root.path, project.canonicalize()?.to_string_lossy());
    assert_eq!(root.label, "read-only-project");
    assert!(root.read && !root.write);
    assert!(session.authoring.is_empty());
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authoring_generation_refuses_changed_choices_but_preserves_exact_replay() -> TestResult {
    let directory = TempDir::new()?;
    let profile = captured_file_profile(&directory)?;
    let plan = configuration_with_process_profiles(&directory, 32, vec![profile.clone()])?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let generation = daemon
        .client
        .task_session()
        .await?
        .authoring
        .into_iter()
        .find(|contract| contract.capability.id == "golden-local-process")
        .ok_or("configured authoring metadata missing")?
        .generation;
    let mut request = create_request("authoring-generation", false);
    request
        .tasks
        .first_mut()
        .ok_or("task absent")?
        .definition
        .authoring_generation = Some(generation);
    let accepted = daemon.client.create_workflow(&request).await?;
    daemon.stop().await?;
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&profile)?)?;
    let body = document
        .get_mut("profile")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("profile object absent")?;
    body.insert("revision".into(), (generation + 1).into());
    body.insert("descriptor_revision".into(), (generation + 1).into());
    fs::write(&profile, serde_json::to_vec(&document)?)?;
    let restarted = start(plan, CONTROLLER_TOKEN).await?;
    assert_eq!(restarted.client.create_workflow(&request).await?, accepted);
    let mut stale = request.clone();
    stale.request_id = "authoring-generation-stale".into();
    assert!(
        matches!(restarted.client.create_workflow(&stale).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    assert_eq!(
        restarted
            .client
            .workflows("authoring-generation", None, 50)
            .await?
            .items
            .len(),
        1
    );
    stale
        .tasks
        .first_mut()
        .ok_or("task absent")?
        .definition
        .authoring_generation = Some(generation + 1);
    assert!(restarted.client.create_workflow(&stale).await.is_ok());
    let mut changed_reuse = request;
    changed_reuse
        .tasks
        .first_mut()
        .ok_or("task absent")?
        .definition
        .authoring_generation = Some(generation + 1);
    assert!(
        matches!(restarted.client.create_workflow(&changed_reuse).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_requests_missing_inputs_and_frozen_file_bytes_survive_restart() -> TestResult {
    let directory = TempDir::new()?;
    let profile = captured_file_profile(&directory)?;
    let plan = configuration_with_process_profiles(&directory, 32, vec![profile])?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let session = daemon.client.task_session().await?;
    let contract = session
        .authoring
        .iter()
        .find(|contract| contract.capability.id == "golden-local-process")
        .ok_or("process authoring contract absent")?;
    assert_eq!(
        session.task_output_limit_bytes,
        milkdrift_blueprint::MAX_TASK_OUTPUT_BYTES
    );
    assert_eq!(contract.generation, 2);
    assert!(!contract.allows_additional_inputs);
    assert_eq!(
        contract
            .inputs
            .iter()
            .map(|input| input.name.as_str())
            .collect::<Vec<_>>(),
        vec!["file"]
    );
    assert!(contract.outputs.iter().any(|output| output.name == "report"
        && output.required
        && output.media_type.as_deref() == Some("text/plain")));
    let source = directory.path().join("Cargo.toml");
    let captured = "[package]\nname = \"captured-project\"\nversion = \"0.1.0\"\n";
    fs::write(&source, captured)?;
    let upload = TaskUploadRequest {
        request_id: "captured-file".into(),
        name: "Cargo.toml".into(),
        media_type: "text/plain".into(),
        text: fs::read_to_string(&source)?,
    };
    let file = daemon.client.upload_task_input(&upload).await?;
    assert_eq!(daemon.client.upload_task_input(&upload).await?, file);
    let mut changed_upload = upload.clone();
    changed_upload.text.push_str("changed");
    assert!(
        daemon
            .client
            .upload_task_input(&changed_upload)
            .await
            .is_err()
    );
    fs::write(source, "bytes edited after upload")?;
    let mut request = create_request("captured-work", false);
    request
        .tasks
        .first_mut()
        .ok_or("fixture task absent")?
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
            input_id: file.input_id,
        },
    );
    let accepted = daemon.client.create_workflow(&request).await?;
    assert_eq!(daemon.client.create_workflow(&request).await?, accepted);
    assert_eq!(
        daemon.client.workflow_receipt(&request.request_id).await?,
        accepted
    );
    let mut conflict = request.clone();
    conflict.title.push_str(" changed");
    assert!(
        matches!(daemon.client.create_workflow(&conflict).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    let task = accepted
        .aliases
        .get("inspect")
        .ok_or("accepted alias missing")?;
    let queued = wait_task(
        &daemon.client,
        &accepted.workflow_id,
        task,
        TaskStateDto::Queued,
    )
    .await?;
    assert_eq!(queued.state, TaskStateDto::Queued);
    assert!(queued.started_at.is_none() && queued.frozen.is_none());
    let captured_read = queued
        .supplied_inputs
        .get("file")
        .and_then(|input| input.value.as_ref())
        .ok_or("queued captured source missing")?;
    assert_eq!(captured_read.text.as_deref(), Some(captured));
    assert_eq!(captured_read.name.as_deref(), Some("Cargo.toml"));
    assert!(!queued.supplied_inputs.contains_key("brief"));
    assert!(
        queued
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("brief"))
    );
    assert!(
        daemon
            .client
            .task_result(&accepted.workflow_id, task)
            .await
            .is_err()
    );
    let workflow = daemon.client.workflow(&accepted.workflow_id).await?;
    assert_eq!(
        workflow
            .supplied_inputs
            .get("file")
            .and_then(|input| input.value.as_ref())
            .and_then(|value| value.text.as_deref()),
        Some(captured)
    );
    let mut definition = queued.definition;
    definition.instructions =
        "Use the exact uploaded manifest and the later supplied brief.".into();
    let edit = ChangeWorkflowRequest {
        request_id: "future-edit".into(),
        expected_generation: workflow.generation,
        changes: vec![TaskChangeDto::Revise {
            task: task.clone(),
            expected_version: queued.version,
            definition,
        }],
    };
    let revised = daemon
        .client
        .change_workflow(&accepted.workflow_id, &edit)
        .await?;
    assert_eq!(
        daemon
            .client
            .change_workflow(&accepted.workflow_id, &edit)
            .await?,
        revised
    );
    let mut stale = edit.clone();
    stale.request_id = "stale-edit".into();
    assert!(
        daemon
            .client
            .change_workflow(&accepted.workflow_id, &stale)
            .await
            .is_err()
    );
    wait_task(
        &daemon.client,
        &accepted.workflow_id,
        task,
        TaskStateDto::Queued,
    )
    .await?;
    let current_generation = daemon
        .client
        .workflow(&accepted.workflow_id)
        .await?
        .generation;
    daemon
        .client
        .change_workflow(
            &accepted.workflow_id,
            &ChangeWorkflowRequest {
                request_id: "supply-brief".into(),
                expected_generation: current_generation,
                changes: vec![TaskChangeDto::BindInput {
                    name: "brief".into(),
                    value: InputValue::Text {
                        text: "Analyze the captured project".into(),
                    },
                }],
            },
        )
        .await?;
    let complete = wait_task(
        &daemon.client,
        &accepted.workflow_id,
        task,
        TaskStateDto::Completed,
    )
    .await?;
    let frozen = complete.frozen.as_ref().ok_or("frozen start missing")?;
    let input = frozen.inputs.get("file").ok_or("file missing")?;
    assert_eq!(input.text.as_deref(), Some(captured));
    assert_eq!(input.name.as_deref(), Some("Cargo.toml"));
    assert_eq!(input.digest.as_deref(), Some(file.digest.as_str()));
    assert!(frozen.entered && complete.started_at.is_some() && complete.ended_at.is_some());
    stale.expected_generation = daemon
        .client
        .workflow(&accepted.workflow_id)
        .await?
        .generation;
    assert!(
        daemon
            .client
            .change_workflow(&accepted.workflow_id, &stale)
            .await
            .is_err(),
        "started definition was rewritten"
    );
    let result = daemon
        .client
        .task_result(&accepted.workflow_id, task)
        .await?;
    assert!(result.text.contains(captured));
    let published = result
        .outputs
        .get("report")
        .ok_or("named report output missing")?;
    assert!(published.available && published.artifact_id.is_some());
    assert_eq!(published.media_type.as_deref(), Some("text/plain"));
    assert_eq!(published.bytes, captured.len() as u64);
    daemon.stop().await?;
    let restarted = start(plan, CONTROLLER_TOKEN).await?;
    assert_eq!(restarted.client.create_workflow(&request).await?, accepted);
    let page = restarted
        .client
        .workflows("captured-work", None, 10)
        .await?;
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items.first().ok_or("workflow page item absent")?.id,
        accepted.workflow_id
    );
    assert_eq!(
        restarted.client.task(&accepted.workflow_id, task).await?,
        complete
    );
    assert_eq!(
        restarted
            .client
            .task_result(&accepted.workflow_id, task)
            .await?,
        result
    );
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_edits_have_one_winner_and_queued_stop_has_no_execution_start() -> TestResult {
    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let daemon = start(
        configuration_with_process_profiles(&directory, 32, vec![profile])?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let accepted = daemon
        .client
        .create_workflow(&create_request("edit-race", false))
        .await?;
    let task = accepted
        .aliases
        .get("inspect")
        .ok_or("accepted alias missing")?;
    wait_task(
        &daemon.client,
        &accepted.workflow_id,
        task,
        TaskStateDto::Queued,
    )
    .await?;
    let generation = daemon
        .client
        .workflow(&accepted.workflow_id)
        .await?
        .generation;
    let first = ChangeWorkflowRequest {
        request_id: "edit-first".into(),
        expected_generation: generation,
        changes: vec![TaskChangeDto::Wait {
            question: "First editor waiting for a brief".into(),
        }],
    };
    let second = ChangeWorkflowRequest {
        request_id: "edit-second".into(),
        expected_generation: generation,
        changes: vec![TaskChangeDto::Wait {
            question: "Second editor waiting for a brief".into(),
        }],
    };
    let (first, second) = tokio::join!(
        daemon.client.change_workflow(&accepted.workflow_id, &first),
        daemon
            .client
            .change_workflow(&accepted.workflow_id, &second)
    );
    assert_ne!(
        first.is_ok(),
        second.is_ok(),
        "both edits had the same observed generation"
    );
    let workflow = daemon.client.workflow(&accepted.workflow_id).await?;
    let queued = daemon.client.task(&accepted.workflow_id, task).await?;
    let stop = ChangeWorkflowRequest {
        request_id: queued.stop_request_id.clone(),
        expected_generation: workflow.generation,
        changes: vec![TaskChangeDto::Stop { task: task.clone() }],
    };
    let stopped = daemon
        .client
        .change_workflow(&accepted.workflow_id, &stop)
        .await?;
    assert_eq!(
        daemon
            .client
            .change_workflow(&accepted.workflow_id, &stop)
            .await?,
        stopped
    );
    let task = daemon.client.task(&accepted.workflow_id, task).await?;
    assert_eq!(task.state, TaskStateDto::Stopped);
    assert!(task.started_at.is_none() && task.frozen.is_none());
    assert!(task.result.is_none());
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_discovery_and_uploads_recheck_actor_scope_and_cursor_binding() -> TestResult {
    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let daemon = start(
        configuration_with_process_profiles(&directory, 32, vec![profile])?,
        CONTROLLER_TOKEN,
    )
    .await?;
    let first = daemon
        .client
        .create_workflow(&create_request("first-private", false))
        .await?;
    daemon
        .client
        .create_workflow(&create_request("second-private", false))
        .await?;
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(observer.workflows("", None, 10).await?.items.is_empty());
    assert!(observer.workflow(&first.workflow_id).await.is_err());
    assert!(observer.workflow_receipt("first-private").await.is_err());
    let one = daemon.client.workflows("", None, 1).await?;
    assert_eq!(one.items.len(), 1);
    let cursor = one
        .next_cursor
        .as_deref()
        .ok_or("bounded page lacks cursor")?;
    assert!(observer.workflows("", Some(cursor), 1).await.is_err());
    assert!(
        daemon
            .client
            .workflows("changed search", Some(cursor), 1)
            .await
            .is_err()
    );
    assert!(daemon.client.workflows("", None, 0).await.is_err());
    assert!(daemon.client.workflows("", None, 201).await.is_err());
    let upload = TaskUploadRequest {
        request_id: "private-upload".into(),
        name: "private.txt".into(),
        media_type: "text/plain".into(),
        text: "private captured context".into(),
    };
    assert!(observer.upload_task_input(&upload).await.is_err());
    let raw = reqwest::Client::new()
        .get(daemon.endpoint.join("v1/session")?)
        .header("x-milkdrift-daemon", "another-installation")
        .bearer_auth(CONTROLLER_TOKEN)
        .send()
        .await?;
    assert!(raw.status().is_client_error());
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn escaped_descriptions_page_by_wire_bytes_without_losing_workflows_or_tasks() -> TestResult {
    use milkdrift_control_protocol::tasks::{WorkflowPageDto, WorkflowReadDto};
    use std::collections::BTreeSet;

    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let daemon = start(
        configuration_with_process_profiles(&directory, 32, vec![profile])?,
        CONTROLLER_TOKEN,
    )
    .await?;
    // This valid description expands sixfold in JSON. An item-count limit alone overflows.
    let description = "\u{0001}".repeat(4096);
    let mut expected_workflows = BTreeSet::new();
    let mut many = create_request("escaped-pages-many", false);
    many.description.clone_from(&description);
    many.tasks = (0..30)
        .map(|index| {
            let mut definition = task_definition("golden-local-process", "process.execute");
            definition.description.clone_from(&description);
            NewTaskDto {
                alias: format!("task-{index}"),
                definition,
            }
        })
        .collect();
    let accepted = daemon.client.create_workflow(&many).await?;
    expected_workflows.insert(accepted.workflow_id.clone());
    let mut expected_tasks: BTreeSet<_> = accepted.aliases.values().cloned().collect();
    for task in &expected_tasks {
        wait_task(
            &daemon.client,
            &accepted.workflow_id,
            task,
            TaskStateDto::Queued,
        )
        .await?;
    }
    let generation = daemon
        .client
        .workflow(&accepted.workflow_id)
        .await?
        .generation;
    let additions = (30..60)
        .map(|index| {
            let mut definition = task_definition("golden-local-process", "process.execute");
            definition.description.clone_from(&description);
            TaskChangeDto::Add {
                alias: format!("task-{index}"),
                definition,
            }
        })
        .collect();
    let added = daemon
        .client
        .change_workflow(
            &accepted.workflow_id,
            &ChangeWorkflowRequest {
                request_id: "escaped-pages-add".into(),
                expected_generation: generation,
                changes: additions,
            },
        )
        .await?;
    expected_tasks.extend(added.aliases.values().cloned());
    for index in 1..40 {
        let mut request = create_request(&format!("escaped-pages-{index}"), false);
        request.description.clone_from(&description);
        expected_workflows.insert(daemon.client.create_workflow(&request).await?.workflow_id);
    }

    let http = reqwest::Client::new();
    let mut cursor: Option<String> = None;
    let mut observed = BTreeSet::new();
    let mut list_pages = 0;
    loop {
        let mut url = daemon.endpoint.join("v1/workflows")?;
        url.query_pairs_mut()
            .append_pair("search", "escaped-pages")
            .append_pair("limit", "200");
        if let Some(cursor) = &cursor {
            url.query_pairs_mut().append_pair("cursor", cursor);
        }
        let page: WorkflowPageDto = bounded_task_read(&http, url).await?;
        list_pages += 1;
        assert!(
            !page.items.is_empty(),
            "nonterminal byte page made no progress"
        );
        for workflow in page.items {
            assert_eq!(workflow.description, description);
            assert!(
                observed.insert(workflow.id),
                "workflow appeared twice across pages"
            );
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
        assert!(list_pages <= 40, "workflow cursor did not converge");
    }
    assert!(
        list_pages > 1,
        "escaped JSON must reach the byte limit before 200 items"
    );
    assert_eq!(observed, expected_workflows);

    let mut cursor: Option<String> = None;
    let mut observed = BTreeSet::new();
    let mut task_pages = 0;
    loop {
        let mut url = daemon
            .endpoint
            .join(&format!("v1/workflows/{}", accepted.workflow_id))?;
        if let Some(cursor) = &cursor {
            url.query_pairs_mut().append_pair("cursor", cursor);
        }
        let page: WorkflowReadDto = bounded_task_read(&http, url).await?;
        task_pages += 1;
        assert_eq!(page.task_count, 60);
        assert!(!page.tasks.is_empty());
        assert!(
            page.tasks.len() < 50,
            "escaped JSON must reach the byte limit before 50 tasks"
        );
        for task in page.tasks {
            assert_eq!(task.description, description);
            assert!(observed.insert(task.id), "task appeared twice across pages");
        }
        cursor = page.next_task_cursor;
        if cursor.is_none() {
            break;
        }
        assert!(task_pages <= 60, "task cursor did not converge");
    }
    assert!(task_pages > 1);
    assert_eq!(observed, expected_tasks);
    daemon.stop().await
}

async fn bounded_task_read<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    url: Url,
) -> TestResult<T> {
    let response = http.get(url).bearer_auth(CONTROLLER_TOKEN).send().await?;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let bytes = response.bytes().await?;
    assert!(bytes.len() <= milkdrift_control_protocol::MAX_DOCUMENT_BYTES);
    let envelope: milkdrift_control_protocol::ResponseEnvelope<T> = decode_json(&bytes)?;
    Ok(envelope.value)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_text_is_a_bounded_preview_while_download_and_digest_cover_exact_bytes()
-> TestResult {
    let directory = TempDir::new()?;
    let profile = captured_file_profile(&directory)?;
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&profile)?)?;
    let literal = format!(
        "private-result-marker\n{}\nretained-tail-marker",
        "雪".repeat(7_000)
    );
    let output = serde_json::to_string(
        &serde_json::json!({"literal":literal,"stdin":"","selected_environment":null,"ambient_home":null,"ambient_path":null}),
    )?;
    *document
        .pointer_mut("/profile/arguments")
        .ok_or("profile arguments absent")? =
        serde_json::json!(["inspect", "result.txt", "ABSENT_TEST_ENVIRONMENT", literal]);
    for field in [
        "max_argv_bytes",
        "max_file_bytes",
        "max_total_materialized_bytes",
        "max_total_output_bytes",
    ] {
        *document
            .pointer_mut(&format!("/profile/limits/{field}"))
            .ok_or("profile limit absent")? = serde_json::json!(131_072);
    }
    fs::write(&profile, serde_json::to_vec(&document)?)?;
    let mut config = configuration_document_with_process_profiles(&directory, 32, vec![profile])?;
    let observer = config.actors.get_mut(1).ok_or("observer actor missing")?;
    observer.authority.resources.workflow_run = WorkflowRunScope::Any;
    observer.authority.dangerous_allow_broad_authority = true;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    // Three-byte Unicode forces the preview to cut at an actual UTF-8 boundary.
    let text = "雪".repeat(5_000);
    let file = daemon
        .client
        .upload_task_input(&TaskUploadRequest {
            request_id: "large-preview-file".into(),
            name: "captured.txt".into(),
            media_type: "text/plain".into(),
            text: text.clone(),
        })
        .await?;
    let mut request = create_request("large-preview-work", true);
    request.inputs.insert(
        "brief".into(),
        InputValue::Text {
            text: "b".repeat(6_000),
        },
    );
    request
        .tasks
        .first_mut()
        .ok_or("fixture task absent")?
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
            input_id: file.input_id,
        },
    );
    let accepted = daemon.client.create_workflow(&request).await?;
    let task = accepted
        .aliases
        .get("inspect")
        .ok_or("task alias missing")?;
    let completed = wait_task(
        &daemon.client,
        &accepted.workflow_id,
        task,
        TaskStateDto::Completed,
    )
    .await?;
    let frozen = completed.frozen.as_ref().ok_or("frozen inputs absent")?;
    let selected = frozen.inputs.get("file").ok_or("captured input absent")?;
    assert!(selected.text_truncated);
    assert_eq!(selected.name.as_deref(), Some("captured.txt"));
    assert_eq!(selected.digest.as_deref(), Some(file.digest.as_str()));
    let input_preview_bytes: usize = frozen
        .inputs
        .values()
        .filter_map(|input| input.text.as_ref())
        .map(String::len)
        .sum();
    assert!(input_preview_bytes <= 16_384);
    assert!(text.starts_with(selected.text.as_deref().ok_or("input preview absent")?));
    let result = daemon
        .client
        .task_result(&accepted.workflow_id, task)
        .await?;
    assert!(result.text_truncated);
    assert!(result.text.len() <= 16_384);
    assert!(output.starts_with(&result.text));
    assert!(!result.text.contains("retained-tail-marker"));
    assert_eq!(
        result.digest,
        format!("b3_{}", blake3::hash(output.as_bytes()))
    );
    assert_eq!(completed.result.as_ref(), Some(&result));
    assert!(!completed.result_restricted);
    let url = daemon.endpoint.join(&format!(
        "v1/workflows/{}/tasks/{task}/result/content",
        accepted.workflow_id
    ))?;
    let http = reqwest::Client::new();
    let response = http
        .get(url.clone())
        .bearer_auth(CONTROLLER_TOKEN)
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(response.text().await?, output);

    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let visible = observer.task(&accepted.workflow_id, task).await?;
    assert_eq!(visible.state, TaskStateDto::Completed);
    assert_eq!(visible.started_at, completed.started_at);
    assert_eq!(visible.ended_at, completed.ended_at);
    assert!(visible.result_restricted && visible.result.is_none());
    let restricted = visible
        .frozen
        .as_ref()
        .and_then(|start| start.inputs.get("file"))
        .ok_or("restricted frozen input absent")?;
    assert!(restricted.restricted);
    assert!(
        restricted.name.is_none()
            && restricted.text.is_none()
            && restricted.digest.is_none()
            && restricted.artifact_id.is_none()
    );
    assert!(!restricted.text_truncated);
    let supplied = visible
        .supplied_inputs
        .get("file")
        .ok_or("restricted shared source missing")?;
    assert_eq!(supplied.availability, "restricted");
    assert!(supplied.value.is_none());
    let summary = observer.workflow(&accepted.workflow_id).await?;
    assert!(
        summary
            .tasks
            .iter()
            .any(|value| value.id == *task && value.has_result)
    );
    assert_eq!(
        observer
            .workflows("large-preview-work", None, 10)
            .await?
            .items
            .len(),
        1
    );
    assert!(
        observer
            .workflows("private-result-marker", None, 10)
            .await?
            .items
            .is_empty(),
        "output search leaked contents outside the artifact grant"
    );
    assert!(
        matches!(observer.task_result(&accepted.workflow_id, task).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized)
    );
    let denied = http.get(url).bearer_auth(OBSERVER_TOKEN).send().await?;
    assert_eq!(denied.status(), reqwest::StatusCode::FORBIDDEN);
    assert!(!denied.text().await?.contains("private-result-marker"));
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maximum_text_outputs_allow_bounded_separators_and_non_utf8_keeps_lifecycle_readable()
-> TestResult {
    for non_utf8 in [false, true] {
        let directory = TempDir::new()?;
        let profile = configured_process_profile(&directory)?;
        let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&profile)?)?;
        let invalid_bytes = [0xff, 0xfe, b'a'];
        if non_utf8 {
            let source = directory.path().join("non-utf8.txt");
            fs::write(&source, invalid_bytes)?;
            *document
                .pointer_mut("/profile/arguments")
                .ok_or("profile arguments absent")? =
                serde_json::json!(["copy", source, "result.txt"]);
            *document
                .pointer_mut("/profile/outputs")
                .ok_or("profile outputs absent")? = serde_json::json!([{"name":"report","relative_path":"result.txt","media_type":"text/plain","required":true}]);
        } else {
            *document
                .pointer_mut("/profile/arguments")
                .ok_or("profile arguments absent")? =
                serde_json::json!(["text-output-pair", "1048576"]);
            *document
                .pointer_mut("/profile/outputs")
                .ok_or("profile outputs absent")? = serde_json::json!([
                {"name":"stdout","relative_path":"stdout.txt","media_type":"text/plain","required":true},
                {"name":"stderr","relative_path":"stderr.txt","media_type":"text/plain","required":true}
            ]);
            *document
                .pointer_mut("/profile/limits/max_file_bytes")
                .ok_or("profile max_file_bytes absent")? = serde_json::json!(1_048_576);
            *document
                .pointer_mut("/profile/limits/max_total_materialized_bytes")
                .ok_or("profile max_total_materialized_bytes absent")? =
                serde_json::json!(2_097_152);
            *document
                .pointer_mut("/profile/limits/max_total_output_bytes")
                .ok_or("profile max_total_output_bytes absent")? = serde_json::json!(2_097_152);
            *document
                .pointer_mut("/profile/limits/max_output_files")
                .ok_or("profile max_output_files absent")? = serde_json::json!(2);
            *document
                .pointer_mut("/profile/limits/artifact_chunk_bytes")
                .ok_or("profile artifact_chunk_bytes absent")? = serde_json::json!(65_536);
        }
        fs::write(&profile, serde_json::to_vec(&document)?)?;
        let daemon = start(
            configuration_with_process_profiles(&directory, 32, vec![profile])?,
            CONTROLLER_TOKEN,
        )
        .await?;
        let mut request = create_request(
            if non_utf8 {
                "non-utf8-result"
            } else {
                "maximum-result"
            },
            true,
        );
        if non_utf8 {
            request
                .tasks
                .first_mut()
                .ok_or("fixture task absent")?
                .definition
                .paths
                .push(milkdrift_control_protocol::tasks::PathAccess {
                    path: directory
                        .path()
                        .to_str()
                        .ok_or("fixture path is not UTF-8")?
                        .into(),
                    write: false,
                });
        }
        let accepted = daemon.client.create_workflow(&request).await?;
        let task = accepted
            .aliases
            .get("inspect")
            .ok_or("task alias missing")?;
        let completed = wait_task(
            &daemon.client,
            &accepted.workflow_id,
            task,
            TaskStateDto::Completed,
        )
        .await?;
        let result = completed.result.as_ref().ok_or("retained result absent")?;
        assert!(!completed.result_restricted);
        let response = reqwest::Client::new()
            .get(daemon.endpoint.join(&format!(
                "v1/workflows/{}/tasks/{task}/result/content",
                accepted.workflow_id
            ))?)
            .bearer_auth(CONTROLLER_TOKEN)
            .send()
            .await?;
        if non_utf8 {
            assert!(result.text_unavailable);
            assert!(result.text.is_empty() && !result.text_truncated);
            assert_eq!(
                result.digest,
                format!("b3_{}", blake3::hash(&invalid_bytes))
            );
            assert_eq!(result.artifacts.len(), 1);
            let artifact = result.artifacts.first().ok_or("output artifact missing")?;
            assert_eq!(
                daemon.client.artifact_range(artifact, 0, 2).await?.bytes,
                invalid_bytes
            );
            assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
            assert!(response.text().await?.contains("individual artifacts"));
        } else {
            assert!(!result.text_unavailable && result.text_truncated);
            assert_eq!(result.artifacts.len(), 2);
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            let bytes = response.bytes().await?;
            assert_eq!(bytes.len(), 2_097_153);
            assert_eq!(
                bytes.iter().filter(|byte| **byte == b'o').count(),
                1_048_576
            );
            assert_eq!(
                bytes.iter().filter(|byte| **byte == b'e').count(),
                1_048_576
            );
            assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
            assert_eq!(result.digest, format!("b3_{}", blake3::hash(&bytes)));
        }
        daemon.stop().await?;
    }
    Ok(())
}
