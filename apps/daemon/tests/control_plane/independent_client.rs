//! Independent JSON and executable CLI consumers use the same public task operations.
use super::support::*;
use serde_json::{Value, json};

async fn raw(
    daemon: &RunningDaemon,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> TestResult<Value> {
    let mut request = reqwest::Client::new()
        .request(method, daemon.endpoint.join(path)?)
        .bearer_auth(CONTROLLER_TOKEN);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await?;
    let status = response.status();
    let envelope: Value = response.json().await?;
    assert!(status.is_success(), "{status}: {envelope}");
    assert_eq!(
        envelope.get("protocol").ok_or("protocol absent")?,
        &json!({"major":3,"minor":1})
    );
    Ok(envelope.get("value").ok_or("value absent")?.clone())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_json_client_submits_edits_discovers_and_downloads_captured_output()
-> TestResult {
    let directory = TempDir::new()?;
    let profile = captured_file_profile(&directory)?;
    let plan = configuration_with_process_profiles(&directory, 32, vec![profile])?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let upload = raw(&daemon, reqwest::Method::POST, "v1/task-inputs", Some(json!({"request_id":"raw-upload","name":"notes.txt","media_type":"text/plain","text":"Independent captured content\n"}))).await?;
    let input_id = upload.get("input_id").ok_or("input id absent")?;
    let mut body = json!({"request_id":"raw-create","title":"Independent project notes","description":"No native client types construct this request","tasks":[{"alias":"notes","definition":{"title":"Copy the captured evidence","description":"Ordinary temporary-file work","instructions":"Preserve the captured file bytes","capability":{"id":"golden-local-process","operation":"process.execute"},"inputs":{"file":{"kind":"supplied","name":"file","optional":false},"brief":{"kind":"supplied","name":"brief","optional":false}},"dependencies":[],"required":true,"corrective_of":null,"paths":[]}}],"inputs":{"file":{"kind":"uploaded","input_id":input_id}},"finish_when_settled":true,"limits":null});
    let accepted = raw(
        &daemon,
        reqwest::Method::POST,
        "v1/workflows",
        Some(body.clone()),
    )
    .await?;
    assert_eq!(
        raw(
            &daemon,
            reqwest::Method::POST,
            "v1/workflows",
            Some(body.clone())
        )
        .await?,
        accepted
    );
    let workflow = accepted
        .get("workflow_id")
        .and_then(Value::as_str)
        .ok_or("workflow id absent")?;
    let task = accepted
        .get("aliases")
        .and_then(|aliases| aliases.get("notes"))
        .and_then(Value::as_str)
        .ok_or("task id absent")?;
    let shown = raw(
        &daemon,
        reqwest::Method::GET,
        &format!("v1/workflows/{workflow}"),
        None,
    )
    .await?;
    let generation = shown.get("generation").ok_or("generation absent")?;
    let changed = raw(&daemon, reqwest::Method::POST, &format!("v1/workflows/{workflow}/changes"), Some(json!({"request_id":"raw-edit","expected_generation":generation,"changes":[{"kind":"bind_input","name":"brief","value":{"kind":"text","text":"Start the captured file work"}}]}))).await?;
    assert_eq!(
        changed.get("workflow_id").and_then(Value::as_str),
        Some(workflow)
    );
    wait_task(&daemon.client, workflow, task, TaskStateDto::Completed).await?;
    let result = raw(
        &daemon,
        reqwest::Method::GET,
        &format!("v1/workflows/{workflow}/tasks/{task}/result"),
        None,
    )
    .await?;
    let result_text = result
        .get("text")
        .and_then(Value::as_str)
        .ok_or("result text absent")?;
    assert!(result_text.contains("Independent captured content"));
    let download = reqwest::Client::new()
        .get(daemon.endpoint.join(&format!(
            "v1/workflows/{workflow}/tasks/{task}/result/content"
        ))?)
        .bearer_auth(CONTROLLER_TOKEN)
        .send()
        .await?;
    assert_eq!(download.status(), reqwest::StatusCode::OK);
    assert_eq!(download.text().await?, result_text);
    // After dropping all prior client state, search and receipt discovery need no local ID file.
    daemon.stop().await?;
    let restarted = start(plan, CONTROLLER_TOKEN).await?;
    let discovered = raw(
        &restarted,
        reqwest::Method::GET,
        "v1/workflows?search=Independent&limit=10",
        None,
    )
    .await?;
    assert!(
        discovered
            .get("items")
            .and_then(Value::as_array)
            .is_some_and(|items| items
                .iter()
                .any(|item| item.get("id").and_then(Value::as_str) == Some(workflow)))
    );
    assert_eq!(
        raw(
            &restarted,
            reqwest::Method::GET,
            "v1/workflow-requests/raw-create",
            None
        )
        .await?,
        accepted
    );
    *body.get_mut("title").ok_or("request title absent")? = json!("Conflicting changed reuse");
    let refused = reqwest::Client::new()
        .post(restarted.endpoint.join("v1/workflows")?)
        .bearer_auth(CONTROLLER_TOKEN)
        .json(&body)
        .send()
        .await?;
    assert_eq!(refused.status(), reqwest::StatusCode::CONFLICT);
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn executable_cli_upload_create_discovery_edit_and_stop_need_no_recovery_file() -> TestResult
{
    let directory = TempDir::new()?;
    let profile = captured_file_profile(&directory)?;
    let daemon = start(
        configuration_with_process_profiles(&directory, 32, vec![profile])?,
        CONTROLLER_TOKEN,
    )
    .await?;
    fs::write(directory.path().join("notes.txt"), "CLI captured file\n")?;
    let upload = cli_ok(
        &daemon,
        &directory,
        "cli-upload",
        &["workflow", "upload", "notes.txt", "--request", "cli-file"],
    )?;
    let mut request = create_request("cli-task", false);
    request
        .tasks
        .first_mut()
        .ok_or("request task absent")?
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
            input_id: upload
                .get("input_id")
                .and_then(Value::as_str)
                .ok_or("input absent")?
                .into(),
        },
    );
    fs::write(
        directory.path().join("task.json"),
        serde_json::to_vec(&request)?,
    )?;
    let accepted = cli_ok(
        &daemon,
        &directory,
        "cli-create",
        &["workflow", "create", "task.json"],
    )?;
    let workflow = accepted
        .get("workflow_id")
        .and_then(Value::as_str)
        .ok_or("workflow absent")?;
    let task = accepted
        .get("aliases")
        .and_then(|aliases| aliases.get("inspect"))
        .and_then(Value::as_str)
        .ok_or("task absent")?;
    let discovery = cli_ok(
        &daemon,
        &directory,
        "cli-list",
        &["workflow", "list", "--search", "cli-task"],
    )?;
    assert!(
        discovery
            .get("items")
            .and_then(Value::as_array)
            .is_some_and(|items| items
                .iter()
                .any(|item| item.get("id").and_then(Value::as_str) == Some(workflow)))
    );
    assert_eq!(
        cli_ok(
            &daemon,
            &directory,
            "cli-receipt",
            &["workflow", "receipt", "cli-task"]
        )?,
        accepted
    );
    let snapshot = cli_ok(
        &daemon,
        &directory,
        "cli-show",
        &["workflow", "show", workflow],
    )?;
    let generation = snapshot.get("generation").ok_or("generation absent")?;
    let edit = json!({"request_id":"cli-edit","expected_generation":generation,"changes":[{"kind":"wait","question":"Waiting for the person to provide the brief"}]});
    fs::write(
        directory.path().join("edit.json"),
        serde_json::to_vec(&edit)?,
    )?;
    cli_ok(
        &daemon,
        &directory,
        "cli-edit",
        &["workflow", "edit", workflow, "edit.json"],
    )?;
    cli_ok(
        &daemon,
        &directory,
        "cli-stop",
        &["workflow", "stop", workflow, task],
    )?;
    let stopped = cli_ok(
        &daemon,
        &directory,
        "cli-task-read",
        &["workflow", "task", workflow, task],
    )?;
    assert_eq!(
        stopped.get("state").and_then(Value::as_str),
        Some("stopped")
    );
    assert!(stopped.get("started_at").is_some_and(Value::is_null));
    daemon.stop().await
}
