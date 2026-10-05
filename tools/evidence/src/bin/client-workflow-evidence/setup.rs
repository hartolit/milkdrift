use super::{Journey, fixture};
use milkdrift_evidence::{
    EvidenceResult,
    application::{CliRunner, ensure, path_text, reserve_endpoint, run_command, write_private},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub(super) fn configure(
    root: &Path,
    daemon: &Path,
    cli: PathBuf,
    supplied: Option<&Path>,
    model: Option<&fixture::Model>,
) -> EvidenceResult<(CliRunner, PathBuf)> {
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut profile: Value = serde_json::from_slice(&fs::read(supplied.map_or_else(
        || examples.join("local-model/openai-compatible-loopback.example.json"),
        Path::to_owned,
    ))?)?;
    if let Some(model) = model {
        replace(
            &mut profile,
            "/base_url",
            json!(format!("http://{}", model.server.address())),
        )?;
        replace(&mut profile, "/model", json!("controlled-writer"))?;
        replace(&mut profile, "/features", json!(["system_role"]))?;
    }
    let url = url::Url::parse(
        profile
            .get("base_url")
            .and_then(Value::as_str)
            .ok_or("profile base URL absent")?,
    )?;
    ensure(
        url.scheme() == "http" && url.host_str() == Some("127.0.0.1"),
        "this lane requires an explicit loopback endpoint",
    )?;
    ensure(
        profile.pointer("/auth/type").and_then(Value::as_str) == Some("no_auth"),
        "this finite lane expects a local endpoint without credentials",
    )?;
    replace(&mut profile, "/limits/request_timeout_ms", json!(180_000))?;
    replace(&mut profile, "/limits/idle_timeout_ms", json!(180_000))?;
    let profile_id = profile
        .get("identity")
        .and_then(Value::as_str)
        .ok_or("profile identity absent")?
        .to_owned();
    let zones = profile
        .get("trust_zones")
        .and_then(Value::as_array)
        .ok_or("profile trust zones absent")?
        .iter()
        .cloned()
        .chain([json!("milkdrift-control")])
        .collect::<Vec<_>>();
    let profile_path = write_private(
        &root.join("model-profile.json"),
        &serde_json::to_vec_pretty(&profile)?,
    )?;
    let template: toml::Value =
        toml::from_str(&fs::read_to_string(examples.join("operator/daemon.toml"))?)?;
    let mut config = serde_json::to_value(template)?;
    let endpoint = reserve_endpoint()?;
    replace(&mut config, "/bind", json!(endpoint.to_string()))?;
    replace(
        &mut config,
        "/secret_sources",
        json!({"credential:operator":{"type":"file","path":"operator.token"}}),
    )?;
    let resources = config
        .pointer_mut("/actors/0/authority/resources")
        .ok_or("authority absent")?;
    replace(
        resources,
        "/workflow_run",
        json!({"type":"workflows","workflows":["release-notes","meeting-summary","independent-notes"]}),
    )?;
    replace(
        resources,
        "/capability",
        json!({"type":"allow","maximum_side_effect":"unknown",
        "identities":{"type":"only","values":["operator-model","milkdrift-workflow-control"]},
        "categories":{"type":"any"},"operations":{"type":"only","values":["model.generate","workflow.accept_result"]},
        "provider_profiles":{"type":"any"},"trust_zones":{"type":"only","values":zones},
        "execution_trust_classes":{"type":"any"},"localities":{"type":"any"},"peers":{"type":"any"}}),
    )?;
    replace(
        resources,
        "/network",
        json!({"profiles":[profile_id],"destinations":[format!("127.0.0.1:{}",url.port_or_known_default().ok_or("endpoint port absent")?)]}),
    )?;
    replace(
        resources,
        "/artifacts",
        json!({"type":"allow","identities":{"type":"any"},"sensitivities":["public","internal","restricted"]}),
    )?;
    replace(
        resources,
        "/workspace",
        json!({"scopes":[],"allow_any_in_run":true}),
    )?;
    replace(
        &mut config,
        "/actors/0/authority/dangerous_allow_broad_authority",
        json!(true),
    )?;
    replace(
        &mut config,
        "/adapters",
        json!({"process_profiles":[],"model_profiles":[{"capability_id":"operator-model","profile":profile_path}]}),
    )?;
    let path = write_private(
        &root.join("daemon.toml"),
        toml::to_string_pretty(&config)?.as_bytes(),
    )?;
    let token = write_private(
        &root.join("operator.token"),
        b"client-workflow-evidence-token",
    )?;
    let checked = run_command(
        Command::new(daemon).args(["--config", path_text(&path)?, "--check-config"]),
        None,
        Duration::from_secs(10),
    )?;
    ensure(
        checked.status.success(),
        &format!("configuration refused: {}", checked.stderr),
    )?;
    for name in [
        "draft.txt",
        "review.txt",
        "meeting.txt",
        "harbor-brief.txt",
        "lantern-brief.txt",
    ] {
        write_private(
            &root.join(name),
            &fs::read(examples.join("operator/release-notes").join(name))?,
        )?;
    }
    Ok((
        CliRunner {
            executable: cli,
            endpoint: format!("http://{endpoint}/"),
            token_file: token,
            forbidden_storage_path: root.join("data"),
        },
        path,
    ))
}

fn replace(document: &mut Value, pointer: &str, value: Value) -> EvidenceResult {
    *document
        .pointer_mut(pointer)
        .ok_or_else(|| format!("template field absent: {pointer}"))? = value;
    Ok(())
}

pub(super) fn text(value: &Value, pointer: &str) -> EvidenceResult<String> {
    Ok(value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing text {pointer}: {value}"))?
        .to_owned())
}

pub(super) fn sequence(journey: &Journey, run: &str) -> EvidenceResult<String> {
    Ok(journey
        .call(&["run", "show", run])?
        .pointer("/value/sequence")
        .and_then(Value::as_u64)
        .ok_or("run sequence absent")?
        .to_string())
}
