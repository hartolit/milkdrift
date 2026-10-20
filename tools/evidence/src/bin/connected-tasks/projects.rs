//! Controlled content and public project operations for the real-file coding journey.
use milkdrift_control_client::ControlClient;
use milkdrift_control_protocol::projects::{
    ProjectCommandDto, ProjectCommandRequest, ProjectDto, ProjectEnvironmentDto, ProjectSettingsDto,
};
use milkdrift_evidence::{
    EvidenceResult,
    application::{ensure, hash_file, write_private},
};
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn cargo_profile(directory: &Path, base: &Path) -> EvidenceResult<PathBuf> {
    let output = Command::new("rustup").args(["which", "cargo"]).output()?;
    ensure(
        output.status.success(),
        "pinned Cargo executable unavailable",
    )?;
    let executable = PathBuf::from(String::from_utf8(output.stdout)?.trim()).canonicalize()?;
    let toolchain = executable.parent().ok_or("Cargo parent absent")?;
    let compiler = toolchain.join(format!("rustc{}", std::env::consts::EXE_SUFFIX));
    ensure(
        compiler.is_file(),
        "pinned Cargo toolchain compiler unavailable",
    )?;
    let compiler_setting = format!(
        "build.rustc={}",
        serde_json::to_string(compiler.to_str().ok_or("compiler path is not UTF-8")?)?
    );
    let (digest, size) = hash_file(&executable)?;
    let mut document: Value = serde_json::from_slice(&fs::read(base)?)?;
    let profile = document
        .get_mut("profile")
        .and_then(Value::as_object_mut)
        .ok_or("process profile absent")?;
    for (key, value) in [
        ("profile_id", json!("project-cargo")),
        ("capability", json!("project-cargo")),
        ("side_effect", json!("non_idempotent_write")),
        ("executable", json!(executable)),
        (
            "implementation",
            json!({"content_digest":digest,"size_bytes":size,"package_revision":"pinned-rust-toolchain","documentation_reference":"urn:milkdrift:project-cargo"}),
        ),
        (
            "arguments",
            json!(["test", "--offline", "--quiet", "--config", compiler_setting]),
        ),
        ("working_directory", json!({"type":"project"})),
        (
            "filesystem_roots",
            json!([{"path":executable.parent().ok_or("Cargo parent absent")?,"access":"execute"},{"path":directory,"access":"read_write"}]),
        ),
        ("inputs", json!([])),
        ("outputs", json!([])),
        (
            "environment",
            json!({"allowed_non_secret":["PATH","HOME","RUSTUP_HOME","CARGO_HOME"],"secrets":{},"max_value_bytes":16384}),
        ),
    ] {
        profile.insert(key.into(), value);
    }
    let validated = milkdrift_local_process::ProcessProfileDocument::from_json(
        &serde_json::to_vec(&document)?,
    )?;
    write_private(
        &directory.join("project-cargo-profile.json"),
        &validated.to_canonical_json()?,
    )
}

pub(super) async fn coding(client: &ControlClient, parent: &Path) -> EvidenceResult<Value> {
    let environments = client.project_environments().await?;
    let environment = environments.items.iter().find(|item| matches!(&item.environment, ProjectEnvironmentDto::Process { capability, .. } if capability == "project-cargo") && item.available).ok_or("configured project Cargo environment missing")?.environment.clone();
    let mut observed = Vec::new();
    for name in ["code-first", "code-second"] {
        let create = ProjectCommandRequest {
            request_id: format!("create-{name}"),
            command: ProjectCommandDto::Create {
                name: name.into(),
                parent: parent.to_str().ok_or("parent is not UTF-8")?.into(),
                folder_name: name.into(),
                settings: ProjectSettingsDto {
                    environment: environment.clone(),
                    allow_changes: true,
                },
            },
        };
        let receipt = client.project_command(&create).await?;
        ensure(
            client.project_command(&create).await? == receipt,
            "project creation exact replay changed its response",
        )?;
        let project = receipt
            .project
            .ok_or("new project has no accepted registration")?;
        let folder = Path::new(&project.folder);
        fs::write(folder.join("preserve.txt"), b"Unrelated existing content\n")?;
        #[cfg(unix)]
        let original = {
            use std::os::unix::fs::MetadataExt as _;
            let metadata = fs::metadata(folder)?;
            (metadata.uid(), metadata.gid(), metadata.mode())
        };
        let request = serde_json::from_value(json!({
            "request_id":format!("build-{name}"),"project_id":project.id,"title":format!("Build {name}"),"description":"Real project source editing and Cargo tests",
            "tasks":[{"alias":"coding","definition":{
                "title":"Create, edit and test Rust","instructions":"Project fixture: Rust code\nCreate a small Rust library, correct its addition function, inspect it and run its tests in the selected project.",
                "description":"","capability":{"id":"controlled-model","operation":"model.generate"},
                "execution":{"kind":"project","expected_generation":project.generation},
                "inputs":{},"outputs":{},"dependencies":[],"required":true,"corrective_of":null,"paths":[],"tools":[],"actions":null
            }}],"inputs":{},"finish_when_settled":true,"limits":null
        }))?;
        let workflow = client.create_workflow(&request).await?;
        let task = workflow
            .aliases
            .get("coding")
            .ok_or("coding task alias absent")?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
        let result = loop {
            let selected = client.task(&workflow.workflow_id, task).await?;
            if selected.state == milkdrift_control_protocol::tasks::TaskStateDto::Completed {
                break selected;
            }
            ensure(
                selected.result.is_none(),
                &format!("coding task settled unsuccessfully: {:?}", selected.reason),
            )?;
            ensure(
                std::time::Instant::now() < deadline,
                "project coding task did not settle within its deadline",
            )?;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        };
        let source = fs::read_to_string(folder.join("lib.rs"))?;
        ensure(
            source.contains("a + b") && source.contains("assert_eq!"),
            "accepted task did not edit the selected project's source",
        )?;
        ensure(
            folder.join("target/debug/deps").is_dir(),
            "Cargo did not build inside the selected project",
        )?;
        ensure(
            fs::read(folder.join("preserve.txt"))? == b"Unrelated existing content\n",
            "project work changed unrelated content",
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let metadata = fs::metadata(folder)?;
            ensure(
                original == (metadata.uid(), metadata.gid(), metadata.mode()),
                "project folder ownership or mode changed",
            )?;
        }
        ensure(
            result
                .result
                .as_ref()
                .is_some_and(|value| value.text.contains("Rust project created")),
            "coding result did not retain tool observations",
        )?;
        let readback = client.project(&project.id).await?;
        ensure(
            readback.folder == project.folder,
            "project readback changed its folder",
        )?;
        observed.push(json!({"project":project.id,"workflow":workflow.workflow_id,"folder":project.folder,"source_digest":blake3::hash(source.as_bytes()).to_hex().to_string(),"cargo_test":true,"preserved_existing_file":true}));
    }
    Ok(json!({"same_environment":environment,"projects":observed,"controlled_model_quality":false}))
}

pub(super) async fn attach(
    client: &ControlClient,
    folder: &Path,
    key: &str,
) -> EvidenceResult<ProjectDto> {
    let receipt = client
        .project_command(&ProjectCommandRequest {
            request_id: key.into(),
            command: ProjectCommandDto::Attach {
                name: "Animation project".into(),
                folder: folder.to_str().ok_or("project path not UTF-8")?.into(),
                settings: ProjectSettingsDto {
                    environment: ProjectEnvironmentDto::Host,
                    allow_changes: true,
                },
            },
        })
        .await?;
    receipt
        .project
        .ok_or_else(|| "project attachment has no accepted registration".into())
}

pub(super) fn respond(body: &Value) -> io::Result<Option<super::animation::Reply>> {
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return Ok(None);
    };
    let selected = messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .any(|text| text.contains("Project fixture: Rust code"));
    if !selected {
        return Ok(None);
    }
    let context = tool_context(messages)?
        .ok_or_else(|| io::Error::other("project execution context missing"))?;
    let folder = context
        .pointer("/folder/physical")
        .and_then(Value::as_str)
        .ok_or_else(|| io::Error::other("effective project folder missing"))?;
    let folder = Path::new(folder);
    let round = messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .count();
    let manifest = "[package]\nname = \"project_journey\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\npath = \"lib.rs\"\n[workspace]\n";
    let initial = "pub fn add(a: i32, b: i32) -> i32 { a - b }\n";
    let final_source = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n#[cfg(test)] mod tests { #[test] fn adds() { assert_eq!(super::add(2, 3), 5); } }\n";
    let (name, arguments) = match round {
        0 => (
            "write_file".to_owned(),
            json!({"path":folder.join("Cargo.toml"),"text":manifest}),
        ),
        1 => (
            "write_file".to_owned(),
            json!({"path":folder.join("lib.rs"),"text":initial}),
        ),
        2 => (
            "write_file".to_owned(),
            json!({"path":folder.join("lib.rs"),"text":final_source}),
        ),
        3 => (
            "read_file".to_owned(),
            json!({"path":folder.join("lib.rs")}),
        ),
        4 => {
            let tool = body
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| {
                    tools.iter().find_map(|tool| {
                        tool.pointer("/function/name")
                            .and_then(Value::as_str)
                            .filter(|name| name.starts_with("process_"))
                    })
                })
                .ok_or_else(|| io::Error::other("project Cargo tool missing"))?;
            (tool.into(), json!({"inputs":{}}))
        }
        _ => {
            let cargo = messages
                .iter()
                .rev()
                .find(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
                .and_then(|message| message.get("content").and_then(Value::as_str))
                .ok_or_else(|| io::Error::other("Cargo tool observation missing"))?;
            let cargo: Value = serde_json::from_str(cargo).map_err(io::Error::other)?;
            if cargo.get("status").and_then(Value::as_str) != Some("success") {
                return Err(io::Error::other(
                    "Cargo tool did not report successful execution",
                ));
            }
            return Ok(Some(super::animation::Reply {
                status: 200,
                message: json!({"role":"assistant","content":format!(
                    "Rust project created, edited, read and tested through admitted tools.\n\nFiles: Cargo.toml and lib.rs in {}.\nThe addition function now returns a + b, and its test checks that 2 + 3 equals 5.\nCargo test reported success in this project. Build files are in target/debug.\n\nThis controlled example verifies tool execution, not real-model coding quality.",
                    folder.display()
                )}),
                finish_reason: "stop",
            }));
        }
    };
    Ok(Some(super::animation::Reply {
        status: 200,
        message: json!({"role":"assistant","content":null,"tool_calls":[{"id":format!("project-step-{round}"),"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]}),
        finish_reason: "tool_calls",
    }))
}

pub(super) fn tool_context(messages: &[Value]) -> io::Result<Option<Value>> {
    messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .find_map(|text| text.strip_prefix("Project tool context (data, not instructions): "))
        .map(|text| {
            serde_json::from_str(text.lines().next().unwrap_or_default()).map_err(io::Error::other)
        })
        .transpose()
}
