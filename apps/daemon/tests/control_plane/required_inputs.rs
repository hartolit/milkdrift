//! Explicit absence crosses the public API but never satisfies a required supplied binding.

use super::support::*;
use serde_json::json;

fn recording_plan(directory: &TempDir) -> TestResult<DaemonPlan> {
    let profile = configured_process_profile(directory)?;
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&profile)?)?;
    *document
        .pointer_mut("/profile/arguments")
        .ok_or("profile arguments absent")? =
        json!(["placement", directory.path(), "required-input-fixture"]);
    *document
        .pointer_mut("/profile/side_effect")
        .ok_or("profile effect classification absent")? = json!("non_idempotent_write");
    fs::write(&profile, serde_json::to_vec(&document)?)?;
    fs::write(directory.path().join("repository.txt"), "input-readiness")?;
    configuration_with_process_profiles(directory, 32, vec![profile])
}

fn entries(directory: &TempDir) -> TestResult<usize> {
    match fs::read_to_string(directory.path().join("entries")) {
        Ok(records) => {
            for record in records.lines() {
                assert_eq!(
                    record,
                    "host=required-input-fixture repository=input-readiness"
                );
            }
            Ok(records.lines().count())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

fn task(alias: &str, optional: bool) -> NewTaskDto {
    let mut definition = task_definition("golden-local-process", "process.execute");
    definition.title = format!("{alias} supplied input");
    definition.inputs = BTreeMap::from([(
        "brief".to_owned(),
        InputBinding::Supplied {
            name: "shared".to_owned(),
            optional,
        },
    )]);
    NewTaskDto {
        alias: alias.to_owned(),
        definition,
    }
}

fn request(id: &str, tasks: Vec<NewTaskDto>, value: InputValue) -> CreateWorkflowRequest {
    CreateWorkflowRequest {
        project_id: fixture_project_id(),
        request_id: id.to_owned(),
        title: format!("Input availability {id}"),
        description: "Count actual process entries for supplied input readiness".to_owned(),
        tasks,
        inputs: BTreeMap::from([("shared".to_owned(), value)]),
        finish_when_settled: true,
        limits: None,
    }
}

async fn assert_waiting(client: &ControlClient, workflow: &str, task: &str) -> TestResult {
    // Wait for an actual scheduler decision, and fail immediately if it freezes or enters.
    let value = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = client.task(workflow, task).await?;
            if value.reason.is_some() || value.state != TaskStateDto::Queued {
                return Ok::<_, ClientError>(value);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    assert_eq!(value.state, TaskStateDto::Queued);
    assert_eq!(
        value.reason.as_deref(),
        Some("missing supplied input shared")
    );
    assert!(value.started_at.is_none());
    assert!(value.frozen.is_none());
    assert!(value.result.is_none());
    Ok(())
}

async fn supply_text(
    client: &ControlClient,
    workflow: &str,
    key: &str,
    text: &str,
) -> TestResult<ChangeWorkflowRequest> {
    let request = ChangeWorkflowRequest {
        request_id: key.to_owned(),
        expected_generation: client.workflow(workflow).await?.generation,
        changes: vec![TaskChangeDto::BindInput {
            name: "shared".to_owned(),
            value: InputValue::Text {
                text: text.to_owned(),
            },
        }],
    };
    let accepted = client.change_workflow(workflow, &request).await?;
    assert_eq!(client.change_workflow(workflow, &request).await?, accepted);
    Ok(request)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_wait_survives_input_restart_and_replayed_earlier_continue() -> TestResult {
    let directory = TempDir::new()?;
    let plan = recording_plan(&directory)?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let accepted = daemon
        .client
        .create_workflow(&request(
            "explicit-wait-input",
            vec![task("required", false)],
            InputValue::Absent,
        ))
        .await?;
    let workflow = &accepted.workflow_id;
    let task = accepted.aliases.get("required").ok_or("task missing")?;
    assert_waiting(&daemon.client, workflow, task).await?;
    let first_wait = ChangeWorkflowRequest {
        request_id: "first-question".into(),
        expected_generation: daemon.client.workflow(workflow).await?.generation,
        changes: vec![TaskChangeDto::Wait {
            question: "Review first brief".into(),
        }],
    };
    daemon.client.change_workflow(workflow, &first_wait).await?;
    let earlier_continue = ChangeWorkflowRequest {
        request_id: "continue-first-question".into(),
        expected_generation: daemon.client.workflow(workflow).await?.generation,
        changes: vec![TaskChangeDto::Continue],
    };
    let continued = daemon
        .client
        .change_workflow(workflow, &earlier_continue)
        .await?;
    let later_wait = ChangeWorkflowRequest {
        request_id: "later-question".into(),
        expected_generation: daemon.client.workflow(workflow).await?.generation,
        changes: vec![TaskChangeDto::Wait {
            question: "Review the changed budget".into(),
        }],
    };
    daemon.client.change_workflow(workflow, &later_wait).await?;
    assert_eq!(
        daemon
            .client
            .change_workflow(workflow, &earlier_continue)
            .await?,
        continued
    );
    let mut stale_continue = earlier_continue.clone();
    stale_continue.request_id = "delayed-first-question".into();
    assert!(
        matches!(daemon.client.change_workflow(workflow, &stale_continue).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict)
    );
    let inputs = ChangeWorkflowRequest {
        request_id: "supply-without-answering".into(),
        expected_generation: daemon.client.workflow(workflow).await?.generation,
        changes: vec![
            TaskChangeDto::BindInput {
                name: "shared".into(),
                value: InputValue::Text {
                    text: "Ready input".into(),
                },
            },
            TaskChangeDto::BindInput {
                name: "unrelated".into(),
                value: InputValue::Text {
                    text: "More context".into(),
                },
            },
        ],
    };
    let input_receipt = daemon.client.change_workflow(workflow, &inputs).await?;
    assert_eq!(
        daemon
            .client
            .workflow(workflow)
            .await?
            .wait_question
            .as_deref(),
        Some("Review the changed budget")
    );
    assert_eq!(entries(&directory)?, 0);
    daemon.stop().await?;

    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    assert_eq!(
        daemon.client.change_workflow(workflow, &inputs).await?,
        input_receipt
    );
    assert_eq!(
        daemon
            .client
            .change_workflow(workflow, &earlier_continue)
            .await?,
        continued
    );
    let paused = daemon.client.workflow(workflow).await?;
    assert_eq!(
        paused.wait_question.as_deref(),
        Some("Review the changed budget")
    );
    let summary = paused
        .tasks
        .iter()
        .find(|item| &item.id == task)
        .ok_or("summary missing")?;
    assert_eq!(summary.state, TaskStateDto::Queued);
    assert!(!summary.entered);
    assert!(summary.started_at.is_none());
    assert_eq!(entries(&directory)?, 0);
    let resume = ChangeWorkflowRequest {
        request_id: "continue-later-question".into(),
        expected_generation: paused.generation,
        changes: vec![TaskChangeDto::Continue],
    };
    // A native client must make the same scoped decision; the browser owns no resume policy.
    let path = directory.path().join("resume.json");
    fs::write(&path, serde_json::to_vec(&resume)?)?;
    cli_ok(
        &daemon,
        &directory,
        "resume-later-via-cli",
        &[
            "workflow",
            "edit",
            workflow,
            path.to_str().ok_or("non-UTF-8 fixture path")?,
        ],
    )?;
    let completed = wait_task(&daemon.client, workflow, task, TaskStateDto::Completed).await?;
    assert!(completed.frozen.as_ref().is_some_and(|start| start.entered));
    let summary = daemon
        .client
        .workflow(workflow)
        .await?
        .tasks
        .into_iter()
        .find(|item| &item.id == task)
        .ok_or("summary missing")?;
    assert!(summary.entered);
    assert_eq!(entries(&directory)?, 1);
    let receipt = daemon.client.workflow_receipt(&resume.request_id).await?;
    daemon.stop().await?;
    let restarted = start(plan, CONTROLLER_TOKEN).await?;
    assert_eq!(
        restarted.client.change_workflow(workflow, &resume).await?,
        receipt
    );
    assert_eq!(restarted.client.task(workflow, task).await?, completed);
    restarted.stop().await?;
    assert_eq!(entries(&directory)?, 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn required_absent_waits_across_restart_then_public_edit_enters_once() -> TestResult {
    let directory = TempDir::new()?;
    let plan = recording_plan(&directory)?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let request = request(
        "required-absent",
        vec![task("required", false)],
        InputValue::Absent,
    );
    let accepted = daemon.client.create_workflow(&request).await?;
    let task = accepted
        .aliases
        .get("required")
        .ok_or("required alias absent")?;
    assert_waiting(&daemon.client, &accepted.workflow_id, task).await?;
    assert_eq!(entries(&directory)?, 0);
    assert_eq!(daemon.client.create_workflow(&request).await?, accepted);
    daemon.stop().await?;

    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    assert_waiting(&daemon.client, &accepted.workflow_id, task).await?;
    assert_eq!(entries(&directory)?, 0);
    let edit = supply_text(
        &daemon.client,
        &accepted.workflow_id,
        "supply-required",
        "A real required value",
    )
    .await?;
    let completed = wait_task(
        &daemon.client,
        &accepted.workflow_id,
        task,
        TaskStateDto::Completed,
    )
    .await?;
    let frozen = completed.frozen.as_ref().ok_or("completed start absent")?;
    assert!(frozen.entered);
    let input = frozen.inputs.get("brief").ok_or("frozen brief absent")?;
    assert_eq!(input.kind, "text");
    assert_eq!(input.text.as_deref(), Some("A real required value"));
    assert_eq!(entries(&directory)?, 1);
    let edit_receipt = daemon.client.workflow_receipt(&edit.request_id).await?;
    daemon.stop().await?;

    let restarted = start(plan, CONTROLLER_TOKEN).await?;
    assert_eq!(restarted.client.create_workflow(&request).await?, accepted);
    assert_eq!(
        restarted
            .client
            .change_workflow(&accepted.workflow_id, &edit)
            .await?,
        edit_receipt
    );
    assert_eq!(
        restarted.client.task(&accepted.workflow_id, task).await?,
        completed
    );
    restarted.stop().await?;
    assert_eq!(entries(&directory)?, 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn optional_absence_and_required_empty_text_are_distinct_valid_public_inputs() -> TestResult {
    let directory = TempDir::new()?;
    let daemon = start(recording_plan(&directory)?, CONTROLLER_TOKEN).await?;
    for (id, optional, value, kind, text) in [
        ("optional-absent", true, InputValue::Absent, "absent", None),
        (
            "required-empty",
            false,
            InputValue::Text {
                text: String::new(),
            },
            "text",
            Some(""),
        ),
    ] {
        let accepted = daemon
            .client
            .create_workflow(&request(id, vec![task("consumer", optional)], value))
            .await?;
        let task = accepted
            .aliases
            .get("consumer")
            .ok_or("consumer alias absent")?;
        let completed = wait_task(
            &daemon.client,
            &accepted.workflow_id,
            task,
            TaskStateDto::Completed,
        )
        .await?;
        let input = completed
            .frozen
            .as_ref()
            .ok_or("start absent")?
            .inputs
            .get("brief")
            .ok_or("brief absent")?;
        assert_eq!(input.kind, kind);
        assert_eq!(input.text.as_deref(), text);
    }
    daemon.stop().await?;
    assert_eq!(entries(&directory)?, 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_absent_input_waits_only_for_required_consumer_and_preserves_started_optional()
-> TestResult {
    let directory = TempDir::new()?;
    let daemon = start(recording_plan(&directory)?, CONTROLLER_TOKEN).await?;
    let accepted = daemon
        .client
        .create_workflow(&request(
            "shared-absence",
            vec![task("required", false), task("optional", true)],
            InputValue::Absent,
        ))
        .await?;
    let required = accepted
        .aliases
        .get("required")
        .ok_or("required alias absent")?;
    let optional = accepted
        .aliases
        .get("optional")
        .ok_or("optional alias absent")?;
    let optional_start = wait_task(
        &daemon.client,
        &accepted.workflow_id,
        optional,
        TaskStateDto::Completed,
    )
    .await?;
    assert_waiting(&daemon.client, &accepted.workflow_id, required).await?;
    assert_eq!(entries(&directory)?, 1);
    assert_eq!(
        optional_start
            .frozen
            .as_ref()
            .ok_or("optional start absent")?
            .inputs
            .get("brief")
            .ok_or("optional brief absent")?
            .kind,
        "absent"
    );
    supply_text(
        &daemon.client,
        &accepted.workflow_id,
        "supply-shared",
        "Only future starts select this text",
    )
    .await?;
    let required_start = wait_task(
        &daemon.client,
        &accepted.workflow_id,
        required,
        TaskStateDto::Completed,
    )
    .await?;
    assert_eq!(
        required_start
            .frozen
            .as_ref()
            .ok_or("required start absent")?
            .inputs
            .get("brief")
            .ok_or("required brief absent")?
            .text
            .as_deref(),
        Some("Only future starts select this text")
    );
    let mut optional_after = daemon.client.task(&accepted.workflow_id, optional).await?;
    assert_eq!(
        optional_after
            .supplied_inputs
            .get("shared")
            .and_then(|input| input.value.as_ref())
            .and_then(|input| input.text.as_deref()),
        Some("Only future starts select this text")
    );
    // The live shared-source projection changes; every retained task fact stays identical.
    optional_after.supplied_inputs = optional_start.supplied_inputs.clone();
    assert_eq!(optional_after, optional_start);
    daemon.stop().await?;
    assert_eq!(entries(&directory)?, 2);
    Ok(())
}
