//! Black-box setup consumes maintained operator files, never daemon construction internals.
use milkdrift_authority::{AccessMode, FilesystemScope};
use milkdrift_evidence::{
    EvidenceResult,
    application::{ensure, run_command, write_private},
    http_fixture::read_request,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write as _,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

pub(super) struct MockModel {
    pub(super) address: SocketAddr,
    pub(super) invocations: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<std::io::Result<()>>>,
}

impl MockModel {
    pub(super) fn start() -> EvidenceResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let invocations = Arc::new(AtomicUsize::new(0));
        let stopped = stop.clone();
        let entered = invocations.clone();
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                read_request(&mut stream)?;
                entered.fetch_add(1, Ordering::SeqCst);
                let body = concat!(
                    "data: {\"id\":\"operator-response-1\",\"model\":\"operator-model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"ack\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"id\":\"operator-response-1\",\"model\":\"operator-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":1,\"total_tokens\":6}}\n\n",
                    "data: [DONE]\n\n"
                );
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )?;
            }
            Ok(())
        });
        Ok(Self {
            address,
            invocations,
            stop,
            worker: Some(worker),
        })
    }

    pub(super) fn finish(&mut self) -> EvidenceResult {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| "mock endpoint panicked")??;
        }
        Ok(())
    }
}

impl Drop for MockModel {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

pub(super) fn exercise_starter(examples: &Path, daemon: &Path, cli: &Path) -> EvidenceResult {
    use milkdrift_evidence::application::{
        CliRunner, path_text, required_text, reserve_endpoint, start_daemon, wait_for_readiness,
    };
    let directory = tempfile::tempdir()?;
    let endpoint = reserve_endpoint()?;
    let token_file = write_private(
        &directory.path().join("operator.token"),
        super::TOKEN.as_bytes(),
    )?;
    let template: toml::Value = toml::from_str(&fs::read_to_string(examples.join("daemon.toml"))?)?;
    let mut config = serde_json::to_value(template)?;
    // Only connection coordinates change. The maintained safe authority remains exact.
    config["bind"] = json!(endpoint.to_string());
    config["secret_sources"] =
        json!({"credential:operator": {"type":"file", "path":"operator.token"}});
    let config_path = checked_config(directory.path(), daemon, &config)?;
    let runner = CliRunner {
        executable: cli.to_owned(),
        endpoint: format!("http://{endpoint}/"),
        token_file,
        forbidden_storage_path: directory.path().join("data"),
    };
    let mut child = start_daemon(daemon, &config_path)?;
    wait_for_readiness(&runner, &mut child)?;
    let document = examples.join("starter.json");
    runner.success(&["blueprint", "validate", path_text(&document)?])?;
    let imported = runner.success(&[
        "--command-id",
        "starter-import",
        "blueprint",
        "import",
        path_text(&document)?,
    ])?;
    let revision = required_text(&imported, &["value", "value", "revision_id"])?;
    runner.success(&[
        "--command-id",
        "starter-start",
        "run",
        "start",
        "starter",
        "operator-starter",
        &revision,
    ])?;
    runner.success(&[
        "--timeout-secs",
        "5",
        "run",
        "wait",
        "starter",
        "--terminal",
        "succeeded",
    ])?;
    ensure(
        runner.success(&["run", "show", "starter"])?["value"]["controller_accounting"]["state"]
            == "inactive",
        "ordinary workflow acquired controller accounting",
    )?;
    child.terminate()?;
    Ok(())
}

pub(super) fn exercise_model(
    runner: &milkdrift_evidence::application::CliRunner,
    examples: &Path,
    directory: &Path,
) -> EvidenceResult {
    use milkdrift_evidence::application::{path_text, required_text};
    let blueprint = examples.join("model.json");
    runner.success(&[
        "--command-id",
        "operator-model-validate",
        "blueprint",
        "validate",
        path_text(&blueprint)?,
    ])?;
    let imported = runner.success(&[
        "--command-id",
        "operator-model-import",
        "blueprint",
        "import",
        path_text(&blueprint)?,
    ])?;
    let revision = required_text(&imported, &["value", "value", "revision_id"])?;
    let capability = runner.success(&["capability", "show", "operator-model"])?;
    ensure(
        capability["value"][0]["generation"] == 1
            && capability["value"][0]["provider_profile"] == "local-model-loopback",
        "model registration identity changed",
    )?;
    runner.success(&["provider", "show", "local-model-loopback"])?;
    let start = [
        "--command-id",
        "operator-model-start",
        "run",
        "start",
        "run-operator-model",
        "operator-starter",
        &revision,
    ];
    runner.success(&start)?;
    let replay = runner.success(&start)?;
    ensure(
        replay["value"]["replayed"] == true,
        "model start was not replayed",
    )?;
    let waited = runner.success(&[
        "--timeout-secs",
        "5",
        "run",
        "wait",
        "run-operator-model",
        "--terminal",
        "succeeded",
    ]);
    let run = match waited {
        Ok(run) => run,
        Err(error) => {
            let timeline =
                runner.success(&["run", "timeline", "run-operator-model", "--limit", "100"])?;
            let health = runner.success(&["daemon", "health"])?;
            let capability = runner.success(&["capability", "show", "operator-model"])?;
            return Err(format!(
                "{error}; timeline={timeline}; health={health}; capability={capability}"
            )
            .into());
        }
    };
    let node = run["value"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["node_id"] == "model"))
        .ok_or("model node absent")?;
    let execution = required_text(node, &["execution_id"])?;
    let attempt = required_text(node, &["latest_attempt_id"])?;
    runner.success(&["node", "run-operator-model", &execution])?;
    let inspected = runner.success(&["attempt", "inspect", "run-operator-model", &attempt])?;
    let value = &inspected["value"];
    ensure(
        value["descriptor_revision"] == 1
            && value["provider_profile"] == "local-model-loopback"
            && value["uncertain"] == false,
        "model attempt generation or outcome changed",
    )?;
    ensure(
        value["context_access"] == "authorized" && value["context_manifest"].is_object(),
        "model context provenance absent",
    )?;
    ensure(
        value["progress_observations"]
            .as_u64()
            .is_some_and(|count| count > 0 && count < 10),
        "model observations unbounded or absent",
    )?;
    ensure(
        value["usage"]["input_units"] == 5 && value["usage"]["output_units"] == 1,
        "supplied usage was lost",
    )?;
    let outputs = value["outputs"].as_array().ok_or("model outputs absent")?;
    ensure(outputs.len() == 3, "model outputs incomplete")?;
    for (index, output) in outputs.iter().enumerate() {
        let artifact = required_text(output, &["artifact", "artifact_id"])?;
        runner.success(&["artifact", "metadata", &artifact])?;
        runner.success(&[
            "artifact",
            "get",
            &artifact,
            "--output",
            path_text(&directory.join(format!("model-output-{index}.json")))?,
        ])?;
        if output["name"] == "provider_metadata" {
            let metadata: Value = serde_json::from_slice(&fs::read(
                directory.join(format!("model-output-{index}.json")),
            )?)?;
            ensure(
                metadata["org.milkdrift.openai/response"]["id"] == "operator-response-1"
                    && metadata["org.milkdrift.openai/response"]["model"] == "operator-model",
                "supplied model identity was not retained",
            )?;
        }
    }
    let timeline = runner.success(&["run", "timeline", "run-operator-model", "--limit", "100"])?;
    let events = timeline["value"]["items"]
        .as_array()
        .ok_or("timeline absent")?;
    ensure(
        events
            .windows(2)
            .all(|pair| pair[0]["sequence"].as_u64() < pair[1]["sequence"].as_u64()),
        "model timeline is out of order",
    )?;
    Ok(())
}

pub(super) fn configure(
    examples: &Path,
    directory: &Path,
    endpoint: SocketAddr,
    daemon: &Path,
    profiles: Vec<PathBuf>,
    model: &MockModel,
) -> EvidenceResult<PathBuf> {
    let template = fs::read_to_string(examples.join("daemon.toml"))?;
    let config: toml::Value = toml::from_str(&template)?;
    let mut config = serde_json::to_value(config)?;
    config["bind"] = json!(endpoint.to_string());
    config["secret_sources"] =
        json!({"credential:operator": {"type":"file","path":"operator.token"}});
    config["actors"][0]["actor"] = json!(super::ACTOR);
    let authority = &mut config["actors"][0]["authority"];
    authority["dangerous_allow_broad_authority"] = json!(true);
    let resources = &mut authority["resources"];
    // The scenario explicitly acknowledges several workflows and generated artifact/scope IDs.
    resources["workflow_run"] = json!({"type":"any"});
    resources["capability"] = json!({"type":"allow", "maximum_side_effect":"unknown", "identities":{"type":"any"},"categories":{"type":"any"},"operations":{"type":"any"},"provider_profiles":{"type":"any"},"trust_zones":{"type":"any"},"execution_trust_classes":{"type":"any"},"localities":{"type":"any"},"peers":{"type":"any"}});
    resources["artifacts"] = json!({"type":"allow", "identities":{"type":"any"},"sensitivities":["public","internal","restricted"]});
    resources["layouts"] = json!({"type":"shared", "revisions":{"type":"any"}});
    resources["workspace"] = json!({"scopes":[],"allow_any_in_run":true});
    resources["network"] =
        json!({"profiles":["local-model-loopback"],"destinations":[model.address.to_string()]});
    let executable = std::env::current_exe()?.canonicalize()?;
    let roots = [
        directory.canonicalize()?,
        executable.parent().ok_or("executable parent")?.to_owned(),
    ];
    let filesystem = roots
        .iter()
        .map(|root| {
            FilesystemScope::from_canonical_host_path(
                root,
                [AccessMode::Read, AccessMode::Write, AccessMode::Execute].into(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    resources["filesystem"] = serde_json::to_value(filesystem)?;
    let mut model_profile: Value = serde_json::from_slice(&fs::read(
        examples.join("../local-model/openai-compatible-loopback.example.json"),
    )?)?;
    model_profile["base_url"] = json!(format!("http://{}", model.address));
    model_profile["model"] = json!("operator-model");
    let model_path = directory.join("model-profile.json");
    fs::write(&model_path, serde_json::to_vec(&model_profile)?)?;
    config["adapters"] = json!({"process_profiles": profiles, "model_profiles":[{"capability_id":"operator-model","profile":model_path}]});
    config["runtime"]["lease_duration_ms"] = json!(5_000);
    config["runtime"]["maintenance_interval_ms"] = json!(10);
    checked_config(directory, daemon, &config)
}

fn checked_config(directory: &Path, daemon: &Path, config: &Value) -> EvidenceResult<PathBuf> {
    let path = write_private(
        &directory.join("daemon.toml"),
        toml::to_string_pretty(config)?.as_bytes(),
    )?;
    let checked = run_command(
        std::process::Command::new(daemon)
            .arg("--config")
            .arg(&path)
            .arg("--check-config"),
        None,
        Duration::from_secs(10),
    )?;
    ensure(
        checked.status.success(),
        &format!("operator configuration was refused: {}", checked.stderr),
    )?;
    Ok(path)
}
