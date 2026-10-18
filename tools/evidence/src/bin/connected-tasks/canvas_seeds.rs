//! Bounded browser review cases admitted through the same public API as ordinary work.

use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_control_protocol::tasks::{
    TaskReadDto, TaskStateDto, WorkflowReadDto, WorkflowReceipt,
};
use milkdrift_evidence::{
    EvidenceResult,
    application::{ensure, required_text, write_private},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

pub(super) fn report(kind: &str) -> EvidenceResult {
    ensure(
        matches!(kind, "source" | "compatible" | "wrong-media" | "oversized"),
        "unknown canvas output fixture",
    )?;
    fs::write(
        "report.txt",
        if kind == "wrong-media" {
            "{\"report\":\"correction\"}"
        } else {
            "Retained canvas report\n"
        },
    )?;
    if kind != "source" {
        let evidence = if kind == "oversized" {
            "x".repeat(2048)
        } else {
            "Correction evidence\n".into()
        };
        fs::write("evidence.txt", evidence)?;
    }
    Ok(())
}

pub(super) async fn seed(root: &Path) -> EvidenceResult {
    ensure(
        !root.join("canvas-seeds.json").exists(),
        "canvas review cases already exist",
    )?;
    let browser: Value = serde_json::from_slice(&fs::read(root.join("browser.json"))?)?;
    ensure(
        browser.get("canvas_cases").and_then(Value::as_bool) == Some(true),
        "start a fresh fixture with serve --canvas-cases before seeding review cases",
    )?;
    let mut config = ClientConfig::new(required_text(&browser, &["a", "url"])?.parse()?);
    config.request_timeout = Duration::from_secs(10);
    config.safe_query_retries = 0;
    let client = ControlClient::new(
        config,
        BearerCredential::new(required_text(&browser, &["a", "token"])?)?,
    )?;
    let offpage = offpage(&client).await?;
    let historical = historical(&client).await?;
    let proposal = proposal(&client).await?;
    let denied = denied_source(&client).await?;
    write_private(
        &root.join("canvas-seeds.json"),
        &serde_json::to_vec_pretty(
            &json!({"offpage":offpage,"historical":historical,"proposal":proposal,"denied":denied}),
        )?,
    )?;
    Ok(())
}

async fn denied_source(client: &ControlClient) -> EvidenceResult<Value> {
    let title = "Canvas review: restricted source";
    let text = "Private canvas review context";
    let upload = client
        .upload_task_input(&milkdrift_control_protocol::tasks::TaskUploadRequest {
            request_id: "canvas-denied-source-upload".into(),
            name: "private-context.txt".into(),
            media_type: "text/plain".into(),
            text: text.into(),
        })
        .await?;
    let mut task = definition("Awaiting authorized context", "controlled-model");
    field(
        &mut task,
        "inputs",
        json!({
            "context":{"kind":"supplied","name":"private-context","optional":false},
            "hold":{"kind":"supplied","name":"review-hold","optional":false}
        }),
    )?;
    let request = serde_json::from_value(json!({
        "request_id":"canvas-denied-source","title":title,"description":"An inspectable workflow does not grant access to its captured source or daemon folders.",
        "tasks":[{"alias":"consumer","definition":task}],
        "inputs":{"private-context":{"kind":"uploaded","input_id":upload.input_id}},
        "finish_when_settled":false,"limits":null
    }))?;
    let receipt = client.create_workflow(&request).await?;
    Ok(
        json!({"workflow":receipt.workflow_id,"title":title,"task":alias(&receipt,"consumer")?,"artifact_id":upload.input_id,"private_text":text}),
    )
}

fn definition(title: &str, capability: &str) -> Value {
    json!({"title":title,"description":"Bounded canvas review case","instructions":"Produce the configured finite review output.","capability":{"id":capability,"operation":if capability.starts_with("canvas-") { "process.execute" } else { "model.generate" }},"inputs":{},"outputs":{},"dependencies":[],"required":true,"corrective_of":null,"paths":[],"tools":[],"actions":null})
}

async fn create(
    client: &ControlClient,
    key: &str,
    title: &str,
    tasks: Vec<Value>,
) -> EvidenceResult<WorkflowReceipt> {
    Ok(client.create_workflow(&serde_json::from_value(json!({"request_id":key,"title":title,"description":"Public API canvas review fixture","tasks":tasks,"inputs":{},"finish_when_settled":false,"limits":null}))?).await?)
}

async fn change(
    client: &ControlClient,
    workflow: &str,
    key: &str,
    changes: Vec<Value>,
) -> EvidenceResult<WorkflowReceipt> {
    let current = client.workflow(workflow).await?;
    Ok(client.change_workflow(workflow, &serde_json::from_value(json!({"request_id":key,"expected_generation":current.generation,"changes":changes}))?).await?)
}

fn alias(receipt: &WorkflowReceipt, name: &str) -> EvidenceResult<String> {
    receipt
        .aliases
        .get(name)
        .cloned()
        .ok_or_else(|| "fixture alias missing".into())
}

async fn settled(
    client: &ControlClient,
    workflow: &str,
    task: &str,
) -> EvidenceResult<TaskReadDto> {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let current = client.task(workflow, task).await?;
        if matches!(
            current.state,
            TaskStateDto::Completed | TaskStateDto::Failed | TaskStateDto::Stopped
        ) {
            return Ok(current);
        }
        ensure(
            Instant::now() < deadline,
            "canvas fixture task did not settle",
        )?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn offpage(client: &ControlClient) -> EvidenceResult<Value> {
    let title = "Canvas review: paged tasks";
    let task = |index: usize| -> EvidenceResult<Value> {
        let mut task = definition(&format!("Paged task {index:02}"), "controlled-model");
        field(
            &mut task,
            "inputs",
            json!({"hold":{"kind":"supplied","name":"review-hold","optional":false}}),
        )?;
        field(
            &mut task,
            "outputs",
            json!({"model_response":{"required":true,"media_type":"application/vnd.milkdrift.model-response.v1+json","max_bytes":2097152}}),
        )?;
        Ok(task)
    };
    let initial = create(
        client,
        "canvas-page-first",
        title,
        (0..32)
            .map(|index| Ok(json!({"alias":format!("page{index}"),"definition":task(index)?})))
            .collect::<EvidenceResult<Vec<_>>>()?,
    )
    .await?;
    let added = change(
        client,
        &initial.workflow_id,
        "canvas-page-second",
        (32..60)
            .map(|index| {
                Ok(json!({"kind":"add","alias":format!("page{index}"),"definition":task(index)?}))
            })
            .collect::<EvidenceResult<Vec<_>>>()?,
    )
    .await?;
    let mut ids: Vec<_> = initial
        .aliases
        .values()
        .chain(added.aliases.values())
        .cloned()
        .collect();
    ids.sort();
    let producer = ids.first().ok_or("paged producer absent")?;
    let consumer = ids.last().ok_or("paged consumer absent")?;
    let current = client.task(&initial.workflow_id, consumer).await?;
    let mut revised = serde_json::to_value(&current.definition)?;
    field(&mut revised, "title", json!("Paged consumer"))?;
    let inputs = revised.get_mut("inputs").ok_or("paged inputs missing")?;
    field(
        inputs,
        "result",
        json!({"kind":"task_output","task":producer,"output":"model_response","optional":false}),
    )?;
    field(&mut revised, "dependencies", json!([producer]))?;
    change(client, &initial.workflow_id, "canvas-page-wire", vec![json!({"kind":"revise","task":consumer,"expected_version":current.version,"definition":revised})]).await?;
    Ok(
        json!({"workflow":initial.workflow_id,"title":title,"producer":producer,"consumer":consumer,"task_count":60}),
    )
}

async fn historical(client: &ControlClient) -> EvidenceResult<Value> {
    let title = "Canvas review: retained correction";
    let mut source = definition("Historical failed report", "canvas-source");
    field(
        &mut source,
        "outputs",
        json!({"evidence":{"required":true,"media_type":"text/plain","max_bytes":1024}}),
    )?;
    let initial = create(
        client,
        "canvas-history-source",
        title,
        vec![json!({"alias":"source","definition":source})],
    )
    .await?;
    let original = alias(&initial, "source")?;
    let failed = settled(client, &initial.workflow_id, &original).await?;
    ensure(
        failed.state == TaskStateDto::Failed
            && !failed.definition.outputs.contains_key("report")
            && failed
                .result
                .as_ref()
                .is_some_and(|result| result.outputs.contains_key("report")),
        "historical case lacks a failed undeclared published report",
    )?;
    let mut consumer = definition("Future report consumer", "controlled-model");
    field(
        &mut consumer,
        "inputs",
        json!({"report":{"kind":"task_output","task":original,"output":"report","optional":false}}),
    )?;
    field(&mut consumer, "dependencies", json!([original]))?;
    let mut changes = vec![json!({"kind":"add","alias":"consumer","definition":consumer})];
    for (name, label) in [
        ("compatible", "Compatible correction"),
        ("wrong-media", "Wrong media correction"),
        ("oversized", "Oversized evidence correction"),
    ] {
        let mut correction = definition(label, &format!("canvas-{name}"));
        field(&mut correction, "corrective_of", json!(original))?;
        changes.push(json!({"kind":"add","alias":name,"definition":correction}));
    }
    let added = change(
        client,
        &initial.workflow_id,
        "canvas-history-future",
        changes,
    )
    .await?;
    for name in ["compatible", "wrong-media", "oversized"] {
        ensure(
            settled(client, &initial.workflow_id, &alias(&added, name)?)
                .await?
                .state
                == TaskStateDto::Completed,
            "correction fixture did not complete through its configured process",
        )?;
    }
    Ok(
        json!({"workflow":initial.workflow_id,"title":title,"original":original,"consumer":alias(&added,"consumer")?,"compatible":alias(&added,"compatible")?,"wrong_media":alias(&added,"wrong-media")?,"oversized":alias(&added,"oversized")?,"oversize_contract":"original evidence declaration is 1024 bytes; replacement evidence is 2048 bytes within the aggregate task allowance"}),
    )
}

async fn proposal(client: &ControlClient) -> EvidenceResult<Value> {
    let title = "Canvas review: proposal and human wait";
    let mut task = definition("Rejected follow-up proposal", "controlled-planner");
    field(
        &mut task,
        "instructions",
        json!(
            "Animation fixture role: canvas-rejected\nReturn the controlled invalid follow-up batch for review."
        ),
    )?;
    field(&mut task, "actions", json!({"automatic_repair":false}))?;
    field(
        &mut task,
        "outputs",
        json!({"model_response":{"required":true,"media_type":"application/vnd.milkdrift.model-response.v1+json","max_bytes":2097152}}),
    )?;
    let initial = create(
        client,
        "canvas-proposal-create",
        title,
        vec![json!({"alias":"proposal","definition":task})],
    )
    .await?;
    let id = alias(&initial, "proposal")?;
    let deadline = Instant::now() + Duration::from_secs(45);
    let current: WorkflowReadDto = loop {
        let current = client.workflow(&initial.workflow_id).await?;
        if current
            .admissions
            .iter()
            .any(|admission| admission.task_id == id && admission.status == "rejected")
        {
            break current;
        }
        ensure(
            Instant::now() < deadline,
            "controlled follow-up proposal was not rejected",
        )?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    ensure(
        current
            .tasks
            .iter()
            .any(|task| task.id == id && task.state == TaskStateDto::Completed),
        "proposal task did not retain its completed response",
    )?;
    change(
        client,
        &initial.workflow_id,
        "canvas-proposal-human-wait",
        vec![json!({"kind":"wait","question":"Approve the separate review budget"})],
    )
    .await?;
    Ok(
        json!({"workflow":initial.workflow_id,"title":title,"task":id,"wait_question":"Approve the separate review budget"}),
    )
}

fn field(value: &mut Value, name: &str, replacement: Value) -> EvidenceResult {
    value
        .as_object_mut()
        .ok_or("canvas fixture definition must be an object")?
        .insert(name.into(), replacement);
    Ok(())
}
