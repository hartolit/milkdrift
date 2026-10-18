//! Finite actual-binary evidence through the current CLI and independent public client.

use super::{Daemon, ready, start};
use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_control_protocol::tasks::{
    CreateWorkflowRequest, TaskResultDto, TaskStateDto, WorkflowReceipt,
};
use milkdrift_evidence::{
    EvidenceResult,
    application::{CliRunner, ensure, path_text, required_text, wait_for_workflow, write_private},
};
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};

pub(super) fn client(instance: &Daemon) -> EvidenceResult<ControlClient> {
    let mut config = ClientConfig::new(instance.endpoint.parse()?);
    config.request_timeout = Duration::from_secs(10);
    config.safe_query_retries = 0;
    Ok(ControlClient::new(
        config,
        BearerCredential::new(instance.token.clone())?,
    )?)
}

fn request(
    key: &str,
    capability: &str,
    operation: &str,
    input: &str,
) -> EvidenceResult<CreateWorkflowRequest> {
    Ok(serde_json::from_value(json!({
        "request_id":key,"title":key,"description":"Finite production task evidence",
        "tasks":[{"alias":"analysis","definition":{
            "title":key,"description":"Inspect the supplied Cargo manifest",
            "instructions":"Read only the captured Cargo manifest and report the package and declared dependencies.",
            "capability":{"id":capability,"operation":operation},
            "inputs":{"brief":{"kind":"supplied","name":"brief","optional":false},"file":{"kind":"supplied","name":"file","optional":false}},
            "dependencies":[],"required":true,"corrective_of":null,"paths":[]
        }}],
        "inputs":{"brief":{"kind":"text","text":"private-brief-sentinel"},"file":{"kind":"uploaded","input_id":input}},
        "finish_when_settled":true,"limits":null
    }))?)
}

fn accepted(record: &Value) -> EvidenceResult<WorkflowReceipt> {
    Ok(serde_json::from_value(
        record
            .get("value")
            .ok_or("CLI acceptance omitted value")?
            .clone(),
    )?)
}

fn task_id(receipt: &WorkflowReceipt) -> EvidenceResult<&str> {
    receipt
        .aliases
        .get("analysis")
        .map(String::as_str)
        .ok_or_else(|| "task alias absent".into())
}

pub(super) async fn run(
    root: &Path,
    cli: &Path,
    daemon_binary: &Path,
    instances: &mut [Daemon],
) -> EvidenceResult<Value> {
    let instance = instances.first_mut().ok_or("daemon A absent")?;
    let runner = CliRunner {
        executable: cli.canonicalize()?,
        endpoint: instance.endpoint.clone(),
        token_file: root.join("daemon-a/operator.token"),
        forbidden_storage_path: root.join("daemon-a/data"),
    };
    let input_path = write_private(
        &root.join("Cargo.toml"),
        b"[package]\nname='retained-manifest'\nversion='0.1.0'\n[dependencies]\nserde='1'\n",
    )?;
    let upload = runner.success(&[
        "workflow",
        "upload",
        path_text(&input_path)?,
        "--request",
        "ci-upload",
    ])?;
    let input = required_text(&upload, &["value", "input_id"])?;
    let create = request("ci-cli-process", "cargo-report", "process.execute", &input)?;
    let bytes = serde_json::to_vec(&create)?;
    let receipt = accepted(&runner.success_with_input(&["workflow", "create", "-"], &bytes)?)?;
    ensure(
        accepted(&runner.success_with_input(&["workflow", "create", "-"], &bytes)?)? == receipt,
        "CLI exact create replay changed acceptance",
    )?;
    // Local edits after upload must never change the daemon's captured input.
    fs::write(
        &input_path,
        "[package]\nname='later-local-edit'\nversion='9.0.0'\n",
    )?;
    wait_for_workflow(
        &runner,
        &receipt.workflow_id,
        Duration::from_secs(30),
        |record| record.pointer("/value/state").and_then(Value::as_str) == Some("completed"),
    )?;
    let before: TaskResultDto = serde_json::from_value(
        runner
            .success(&[
                "workflow",
                "result",
                &receipt.workflow_id,
                task_id(&receipt)?,
            ])?
            .get("value")
            .ok_or("result missing")?
            .clone(),
    )?;
    ensure(
        before.text.contains("Package: retained-manifest")
            && before.text.contains("Dependency: serde")
            && !before.text.contains("later-local-edit"),
        "process result did not follow the captured manifest",
    )?;
    let initial = client(instance)?;
    let started = initial
        .task(&receipt.workflow_id, task_id(&receipt)?)
        .await?;
    ensure(
        started.state == TaskStateDto::Completed
            && started.frozen.as_ref().is_some_and(|frozen| frozen.entered),
        "CLI task omitted durable entry evidence",
    )?;
    drop(initial);
    instance.child.shutdown()?;
    instance.child = start(daemon_binary, &instance.config, &instance.log)?;
    ready(instance).await?;
    // A completely new public client knows only the daemon address and credential.
    let recovered = client(instance)?;
    let page = recovered.workflows("ci-cli-process", None, 50).await?;
    ensure(
        page.items.len() == 1
            && page
                .items
                .first()
                .is_some_and(|row| row.id == receipt.workflow_id),
        "fresh public discovery did not find the unique accepted task",
    )?;
    ensure(
        recovered.workflow_receipt(&create.request_id).await? == receipt,
        "durable receipt changed across daemon restart",
    )?;
    ensure(
        recovered.create_workflow(&create).await? == receipt,
        "public replay changed the CLI acceptance",
    )?;
    ensure(
        recovered
            .task_result(&receipt.workflow_id, task_id(&receipt)?)
            .await?
            == before,
        "retained result changed across daemon restart",
    )?;
    ensure(
        recovered
            .task(&receipt.workflow_id, task_id(&receipt)?)
            .await?
            == started,
        "restart or replay duplicated execution history",
    )?;
    let mut changed = create.clone();
    changed.title = "changed reuse must conflict".into();
    let error = recovered
        .create_workflow(&changed)
        .await
        .err()
        .ok_or("changed request identity was accepted")?;
    ensure(
        matches!(error, milkdrift_control_client::ClientError::Api(ref envelope) if envelope.code == milkdrift_control_protocol::ErrorCode::Conflict),
        "changed reuse did not return a typed conflict",
    )?;
    ensure(
        !error.to_string().contains("private-brief-sentinel")
            && !error.to_string().contains(&instance.token),
        "request refusal disclosed private input or credential",
    )?;
    // Exercise the configured model through this same owner and the same captured file.
    let model = request(
        "ci-public-model",
        "controlled-model",
        "model.generate",
        &input,
    )?;
    let model_receipt = recovered.create_workflow(&model).await?;
    wait_for_workflow(
        &runner,
        &model_receipt.workflow_id,
        Duration::from_secs(30),
        |record| record.pointer("/value/state").and_then(Value::as_str) == Some("completed"),
    )?;
    let model_result = recovered
        .task_result(&model_receipt.workflow_id, task_id(&model_receipt)?)
        .await?;
    ensure(
        model_result.text.contains("Controlled analysis")
            && model_result.text.contains("input_digest"),
        "model result omitted captured input analysis",
    )?;
    let captured: Value =
        serde_json::from_slice(&fs::read(root.join("controlled-model/request-0.json"))?)?;
    let model_body = serde_json::to_string(&captured)?;
    ensure(
        model_body.contains("retained-manifest")
            && model_body.contains("private-brief-sentinel")
            && !model_body.contains("later-local-edit"),
        "model did not receive exactly selected captured inputs",
    )?;
    let captures =
        fs::read_dir(root.join("controlled-model"))?.try_fold(0usize, |count, entry| {
            entry?;
            count
                .checked_add(1)
                .ok_or_else(|| std::io::Error::other("model capture count overflow"))
        })?;
    ensure(captures == 2, "model execution was duplicated")?;
    let other = client(instances.get(1).ok_or("daemon B absent")?)?;
    ensure(
        other.workflows("ci-", None, 50).await?.items.is_empty(),
        "independent daemon disclosed another daemon's workflows",
    )?;
    let animation = super::animation_check::run(
        root,
        daemon_binary,
        instances.first_mut().ok_or("daemon A absent")?,
    )
    .await?;
    let report = json!({
        "schema_version":1,"outcome":"passed","fixture_mode":true,"qualifying":false,
        "cli_submission":true,"public_client_discovery":true,"exact_replay_after_restart":true,
        "changed_reuse":"conflict","captured_input_survives_local_edit":true,
        "process_task":receipt.workflow_id,"controlled_model_task":model_receipt.workflow_id,
        "isolated_daemons":2,"model_requests":1,"animation":animation,"shutdown":"confirmed before report publication"
    });
    let text = report.to_string();
    ensure(
        !text.contains("private-brief-sentinel") && !text.contains("retained-manifest"),
        "public evidence report disclosed task payloads",
    )?;
    Ok(report)
}
