//! External file hold and ordinary public requests; scheduling stays in the production daemon.

use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_control_protocol::projects::{
    ProjectCommandDto, ProjectCommandRequest, ProjectEnvironmentDto, ProjectSettingsDto,
};
use milkdrift_control_protocol::tasks::TaskStateDto;
use milkdrift_evidence::{
    EvidenceResult,
    application::{ensure, required_text, write_private},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) fn profile(directory: &Path, base: &Path) -> EvidenceResult<PathBuf> {
    let mut document: Value = serde_json::from_slice(&fs::read(base)?)?;
    let body = document
        .get_mut("profile")
        .and_then(Value::as_object_mut)
        .ok_or("profile missing")?;
    for (name, value) in [
        ("profile_id", json!("canvas-hold")),
        ("capability", json!("canvas-hold")),
        ("side_effect", json!("non_idempotent_write")),
        ("working_directory", json!({"type":"project"})),
        ("arguments", json!(["lifecycle-hold"])),
        ("inputs", json!([])),
        ("outputs", json!([])),
    ] {
        body.insert(name.into(), value);
    }
    *document
        .pointer_mut("/profile/limits/wall_timeout_ms")
        .ok_or("deadline missing")? = json!(90_000);
    let validated = milkdrift_local_process::ProcessProfileDocument::from_json(
        &serde_json::to_vec(&document)?,
    )?;
    write_private(
        &directory.join("canvas-hold-profile.json"),
        &validated.to_canonical_json()?,
    )
}

pub(super) fn hold() -> EvidenceResult {
    let mut entries = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("entries")?;
    writeln!(entries, "entered")?;
    entries.sync_all()?;
    let deadline = Instant::now() + Duration::from_secs(80);
    while !Path::new("release").try_exists()? {
        ensure(
            Instant::now() < deadline,
            "external lifecycle hold exceeded 80 seconds",
        )?;
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

pub(super) async fn scenario(root: &Path, release: bool) -> EvidenceResult {
    let browser: Value = serde_json::from_slice(&fs::read(root.join("browser.json"))?)?;
    ensure(
        browser.get("canvas_cases").and_then(Value::as_bool) == Some(true),
        "lifecycle case requires canvas fixture",
    )?;
    let folder = root.join("daemon-a/lifecycle-project");
    if release {
        ensure(
            root.join("lifecycle-seed.json").is_file(),
            "lifecycle case has not been admitted",
        )?;
        write_private(&folder.join("release"), b"release external fixture hold")?;
        return Ok(());
    }
    ensure(
        !root.join("lifecycle-seed.json").exists(),
        "lifecycle fixture already used",
    )?;
    fs::create_dir(&folder)?;
    let mut config = ClientConfig::new(required_text(&browser, &["a", "url"])?.parse()?);
    config.request_timeout = Duration::from_secs(10);
    config.safe_query_retries = 0;
    let client = ControlClient::new(
        config,
        BearerCredential::new(required_text(&browser, &["a", "token"])?)?,
    )?;
    let environments = client.project_environments().await?;
    let environment = environments.items.into_iter().find(|item| item.available && matches!(&item.environment, ProjectEnvironmentDto::Process { capability, .. } if capability == "canvas-hold")).ok_or("configured lifecycle process environment missing")?.environment;
    let project = client
        .project_command(&ProjectCommandRequest {
            request_id: "lifecycle-project".into(),
            command: ProjectCommandDto::Attach {
                name: "lifecycle-project".into(),
                folder: folder
                    .to_str()
                    .ok_or("lifecycle folder is not UTF-8")?
                    .into(),
                settings: ProjectSettingsDto {
                    environment,
                    allow_changes: true,
                },
            },
        })
        .await?
        .project
        .ok_or("lifecycle project attachment missing")?;
    let definition = |title: &str| {
        json!({
            "title":title,"description":"Wait for the external fixture release","instructions":"Record one actual entry and wait for release.",
            "capability":{"id":"canvas-hold","operation":"process.execute"},
            "execution":{"kind":"project","expected_generation":project.generation},
            "inputs":{},"outputs":{},"dependencies":[],"required":true,"corrective_of":null,"paths":[],"tools":[],"actions":null
        })
    };
    let title = "Lifecycle review: competing project writers";
    let request = serde_json::from_value(json!({
        "request_id":"lifecycle-writers","project_id":project.id,"title":title,"description":"Public project contention without invented user ordering",
        "tasks":[{"alias":"a","definition":definition("Project writer A")},{"alias":"b","definition":definition("Project writer B")}],
        "inputs":{},"finish_when_settled":true,"limits":null
    }))?;
    let receipt = client.create_workflow(&request).await?;
    let a = receipt.aliases.get("a").ok_or("writer A missing")?;
    let b = receipt.aliases.get("b").ok_or("writer B missing")?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let (entered, waiting) = loop {
        let first = client.task(&receipt.workflow_id, a).await?;
        let second = client.task(&receipt.workflow_id, b).await?;
        ensure(
            !matches!(first.state, TaskStateDto::Failed | TaskStateDto::Stopped)
                && !matches!(second.state, TaskStateDto::Failed | TaskStateDto::Stopped),
            "project contention settled a writer",
        )?;
        let is_entered = |task: &milkdrift_control_protocol::tasks::TaskReadDto| {
            task.frozen.as_ref().is_some_and(|start| start.entered)
        };
        let waiting = |task: &milkdrift_control_protocol::tasks::TaskReadDto| {
            task.state == TaskStateDto::Queued
                && task.frozen.as_ref().is_some_and(|start| !start.entered)
                && task.reason.as_deref() == Some("Project is in use")
        };
        if folder.join("entries").is_file() {
            if is_entered(&first) && waiting(&second) {
                break (first, second);
            }
            if is_entered(&second) && waiting(&first) {
                break (second, first);
            }
        }
        ensure(
            Instant::now() < deadline,
            "did not observe one entered and one waiting writer",
        )?;
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    ensure(
        fs::read_to_string(folder.join("entries"))? == "entered\n",
        "more than one writer entered the held project",
    )?;
    write_private(
        &root.join("lifecycle-seed.json"),
        &serde_json::to_vec_pretty(&json!({
            "workflow":receipt.workflow_id,"title":title,"first":entered.id,"first_title":entered.definition.title,
            "second":waiting.id,"second_title":waiting.definition.title,"started_at":waiting.started_at
        }))?,
    )?;
    Ok(())
}
