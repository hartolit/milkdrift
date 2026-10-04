//! Qualify operator-started hosts over their normal CLI endpoints, including SSH forwards.
//! This driver owns requests and evidence only; installation, transport and restart remain explicit.
use super::{
    Arguments, CliRunner, EvidenceResult, ObservationPage, Path, Value, download, ensure, fs, json,
    path_text, request, upload, workflow, write_private,
};
use milkdrift_blueprint::BlueprintRevisionDocument;
use milkdrift_capability::{ArtifactReference, TerminalStatus};
use milkdrift_evidence::application::{required_text, wait_for_run};
use milkdrift_peer_protocol::ObservationHistory;
use serde::Deserialize;
use std::io::Write as _;
use std::{
    io::Read,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

#[cfg(test)]
#[path = "installed/tests.rs"]
mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Endpoint {
    endpoint: String,
    token_file: PathBuf,
    host: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    namespace: String,
    serving: Endpoint,
    coordinator: Endpoint,
    process_capability: String,
    model_capability: String,
    coordinator_process_capability: String,
    coordinator_model_capability: String,
}

fn save(root: &Path, name: &str, value: &Value) -> EvidenceResult {
    write_private(&root.join(name), &serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn runner(args: &Arguments, endpoint: &Endpoint, root: &Path) -> EvidenceResult<CliRunner> {
    let url = url::Url::parse(&endpoint.endpoint)?;
    ensure(
        url.scheme() == "http"
            && url
                .host_str()
                .is_some_and(|h| h == "127.0.0.1" || h == "[::1]" || h == "localhost")
            && url.username().is_empty()
            && url.password().is_none(),
        "installed evidence endpoints must be explicit loopback HTTP (use authenticated SSH forwarding for another machine)",
    )?;
    Ok(CliRunner {
        executable: args.cli.canonicalize()?,
        endpoint: endpoint.endpoint.clone(),
        token_file: endpoint.token_file.canonicalize()?,
        forbidden_storage_path: root.join("no-storage-access"),
    })
}

fn observe(
    runner: &CliRunner,
    execution: &str,
    root: &Path,
    name: &str,
) -> EvidenceResult<Vec<ObservationPage>> {
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut pages = Vec::new();
    let mut after = 0_u64;
    let mut observations = 0;
    loop {
        let value = runner.success_until(
            &[
                "invocation",
                "observations",
                execution,
                "--after",
                &after.to_string(),
                "--limit",
                "128",
            ],
            deadline,
        )?;
        let page: ObservationPage =
            serde_json::from_value(value.pointer("/value").ok_or("missing /value")?.clone())?;
        page.validate(128)?;
        ensure(
            page.execution.as_str() == execution && page.after_sequence == after,
            "observation page changed its execution or cursor",
        )?;
        observations += page.observations.len();
        ensure(
            observations <= 4096 && pages.len() < 4097,
            "installed observation history exceeds the 4096-event evidence bound",
        )?;
        let closed = page.closed;
        let advanced = page.next_sequence != after;
        after = page.next_sequence;
        if advanced || closed {
            save(
                root,
                &format!("{name}-page-{}.json", pages.len() + 1),
                &serde_json::to_value(&page)?,
            )?;
            pages.push(page);
        }
        if closed {
            save(root, name, &serde_json::to_value(&pages)?)?;
            return Ok(pages);
        }
        ensure(
            Instant::now() < deadline,
            "installed invocation did not finish within 300 seconds; retain and inspect its saved acceptance",
        )?;
        if !advanced {
            thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(200)),
            );
        }
    }
}

fn successful_outputs(
    pages: &[ObservationPage],
) -> EvidenceResult<Vec<(String, ArtifactReference)>> {
    let last = pages.last().ok_or("invocation observations absent")?;
    ensure(
        last.closed && last.terminal,
        "invocation closed without terminal evidence; inspect retained uncertainty",
    )?;
    let events = pages
        .iter()
        .flat_map(|page| page.observations.iter())
        .chain(
            match &last.history {
                ObservationHistory::Hot => None,
                ObservationHistory::Archived { summary } => {
                    Some(summary.output_observations.iter())
                }
            }
            .into_iter()
            .flatten(),
        );
    let terminal = match &last.history {
        ObservationHistory::Hot => last.observations.last(),
        ObservationHistory::Archived { summary } => summary.final_observation.as_ref(),
    }
    .and_then(|observation| observation.event.kind().terminal())
    .ok_or("invocation terminal event absent")?;
    ensure(
        terminal.status() == TerminalStatus::Success,
        "installed direct invocation did not succeed; inspect retained terminal evidence",
    )?;
    Ok(events
        .filter_map(|o| o.event.kind().output())
        .map(|(name, reference)| (name.to_owned(), reference.clone()))
        .collect())
}

pub(crate) fn run(args: &Arguments) -> EvidenceResult {
    let manifest_path = args
        .installed_hosts
        .as_ref()
        .ok_or("installed manifest absent")?;
    let mut bytes = Vec::new();
    fs::File::open(manifest_path)?
        .take(16_385)
        .read_to_end(&mut bytes)?;
    ensure(bytes.len() <= 16_384, "installed manifest exceeds bound")?;
    let manifest: Manifest =
        serde_json::from_value(milkdrift_contracts::parse_json_without_duplicates(&bytes)?)?;
    ensure(
        manifest.schema_version == 1,
        "unsupported installed manifest version",
    )?;
    ensure(
        !manifest.namespace.is_empty()
            && manifest.namespace.len() <= 48
            && manifest
                .namespace
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
        "installed namespace must have 1..48 ASCII letters, digits or hyphens",
    )?;
    ensure(
        manifest.serving.host != manifest.coordinator.host,
        "installed host identities must differ",
    )?;
    let root = args
        .installed_output
        .as_ref()
        .ok_or("installed output absent")?;
    if args.installed_replay {
        ensure(
            fs::read(root.join("manifest.json"))? == bytes,
            "restart replay requires the exact retained endpoint manifest",
        )?;
    } else {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(root)?;
        write_private(&root.join("manifest.json"), &bytes)?;
    }
    let serving = runner(args, &manifest.serving, root)?;
    let coordinator = runner(args, &manifest.coordinator, root)?;
    for (name, endpoint, runner, role) in [
        ("serving", &manifest.serving, &serving, "execution_only"),
        (
            "coordinator",
            &manifest.coordinator,
            &coordinator,
            "workflow_enabled",
        ),
    ] {
        let health = runner.success(&["daemon", "health"])?;
        let catalog = runner.success(&["invocation", "catalog"])?;
        ensure(
            health.pointer("/value/role").ok_or("missing /value/role")? == role
                && catalog
                    .pointer("/value/host")
                    .ok_or("missing /value/host")?
                    == endpoint.host.as_str(),
            "installed role or host differs from the explicit manifest",
        )?;
        if !args.installed_replay {
            save(
                root,
                &format!("{name}-identity.json"),
                &json!({"health":health,"catalog":catalog}),
            )?;
        }
    }
    if args.installed_replay {
        return replay(&serving, &coordinator, root);
    }
    let process_bytes = b"Milkdrift installed-host file transfer\n";
    let task = json!({"schema_version":1,"request":{"messages":[{"role":"user","parts":[{"type":"text","text":"Write one brief sentence acknowledging this fresh Milkdrift integration request."}],"tool_call_id":null}],"tools":[],"structured_output":null,"session":{"type":"fresh"},"reasoning":null,"maximum_output_units":64,"streaming":true,"extensions":{"org.milkdrift.openai/request":{"reasoning_effort":"none"}}}});
    let model_bytes = serde_json::to_vec(&task)?;
    let cases = [
        (
            "process",
            manifest.process_capability.as_str(),
            "process.execute",
            "source",
            "stdout",
            "text/plain",
            process_bytes.as_slice(),
        ),
        (
            "model",
            manifest.model_capability.as_str(),
            "model.generate",
            "milkdrift.model_task",
            "final_text",
            "application/json",
            model_bytes.as_slice(),
        ),
    ];
    let mut direct = Vec::new();
    for (name, capability, operation, input_name, output_name, media, input) in cases {
        let id = format!("{}-direct-{name}", manifest.namespace);
        let artifact = upload(&serving, root, &format!("{id}-input"), media, input)?;
        let path = request(
            &serving, root, capability, operation, &id, input_name, artifact,
        )?;
        let accepted = serving.success(&["invocation", "submit", path_text(&path)?])?;
        save(root, &format!("{id}-acceptance.json"), &accepted)?;
        let execution = required_text(&accepted, &["value", "execution"])?;
        let observation_file = format!("{id}-observations.json");
        let pages = observe(&serving, &execution, root, &observation_file)?;
        let artifact = successful_outputs(&pages)?
            .into_iter()
            .find(|(name, _)| name == output_name)
            .ok_or("installed output artifact absent")?
            .1;
        let result = download(&serving, root, artifact.identity(), &format!("{id}-output"))?;
        check_result(name, &result, process_bytes)?;
        let state = serving.success(&["invocation", "show", &execution])?;
        ensure(
            state
                .pointer("/value/origin/type")
                .ok_or("missing /value/origin/type")?
                == "direct",
            "independent call acquired workflow provenance",
        )?;
        direct.push(json!({"request":path.file_name().and_then(|n| n.to_str()).ok_or("request filename absent")?,"execution":execution,"output":artifact,"output_file":format!("{id}-output"),"state":state}));
    }
    coordinator.success(&["peer", "connect", &manifest.serving.host])?;
    let mut workflows = Vec::new();
    for (name, capability, operation, input_name, output_name, media, input) in cases {
        let id = format!("{}-workflow-{name}", manifest.namespace);
        let artifact = upload(&coordinator, root, &format!("{id}-input"), media, input)?;
        let catalog = coordinator.success(&["capability", "list"])?;
        let requested = if name == "process" {
            &manifest.coordinator_process_capability
        } else {
            &manifest.coordinator_model_capability
        };
        let candidates = catalog
            .pointer("/value")
            .ok_or("missing /value")?
            .as_array()
            .ok_or("coordinator catalog absent")?
            .iter()
            .filter(|entry| {
                entry["capability_id"] == *requested && entry["peer_id"] == manifest.serving.host
            })
            .collect::<Vec<_>>();
        ensure(
            candidates.len() == 1,
            "expected one unambiguous remote capability",
        )?;
        let remote = required_text(
            candidates.first().ok_or("remote capability absent")?,
            &["capability_id"],
        )?;
        let revision =
            workflow::blueprint(&id, &remote, operation, input_name, &artifact, output_name)?;
        let path = root.join(format!("{id}.json"));
        write_private(
            &path,
            &BlueprintRevisionDocument::new(&revision).to_canonical_json()?,
        )?;
        coordinator.success(&[
            "--command-id",
            &format!("import-{id}"),
            "blueprint",
            "import",
            path_text(&path)?,
        ])?;
        let command = format!("start-{id}");
        let accepted = coordinator.success(&[
            "--command-id",
            &command,
            "run",
            "start",
            &id,
            &id,
            revision.id().as_str(),
        ])?;
        save(root, &format!("{id}-acceptance.json"), &accepted)?;
        let state = wait_for_run(&coordinator, &id, Duration::from_secs(300), |v| {
            v.pointer("/value/terminal")
                .is_some_and(|terminal| !terminal.is_null())
                || v.pointer("/value/uncertainty_count")
                    .and_then(Value::as_u64)
                    .is_some_and(|count| count != 0)
        })?;
        save(root, &format!("{id}-state.json"), &state)?;
        ensure(
            state
                .pointer("/value/terminal")
                .ok_or("missing /value/terminal")?
                == "succeeded",
            "remote workflow did not succeed; inspect retained state",
        )?;
        let node = state
            .pointer("/value/nodes")
            .ok_or("missing /value/nodes")?
            .as_array()
            .and_then(|nodes| nodes.iter().find(|node| node["node_id"] == "operation"))
            .ok_or("remote operation absent")?;
        let attempt_id = required_text(node, &["latest_attempt_id"])?;
        let attempt = coordinator.success(&["attempt", "inspect", &id, &attempt_id])?;
        ensure(
            attempt
                .pointer("/value/peer_id")
                .ok_or("missing /value/peer_id")?
                == manifest.serving.host.as_str()
                && attempt
                    .pointer("/value/capability_provenance/peer/remote_capability_id")
                    .ok_or("missing /value/capability_provenance/peer/remote_capability_id")?
                    == capability,
            "workflow execution lost the selected serving peer",
        )?;
        let artifact = attempt
            .pointer("/value/outputs")
            .ok_or("missing /value/outputs")?
            .as_array()
            .and_then(|outputs| outputs.iter().find(|o| o["name"] == output_name))
            .ok_or("remote output absent")?["artifact"]
            .clone();
        let output_id = required_text(&artifact, &["artifact_id"])?;
        let result = download(&coordinator, root, &output_id, &format!("{id}-output"))?;
        check_result(name, &result, process_bytes)?;
        save(root, &format!("{id}-attempt.json"), &attempt)?;
        workflows.push(json!({"run":id,"command":command,"revision":revision.id(),"attempt":attempt_id,"invocation":attempt.pointer("/value/invocation_id").ok_or("missing /value/invocation_id")?,"output":artifact,"output_file":format!("{id}-output")}));
    }
    save(
        root,
        "completed.json",
        &json!({"schema_version":1,"direct":direct,"workflows":workflows,
        "limits":"Operator-started host topology; physical identities and external entry counters require separate host observations. Real-model text is checked for a nonempty result, not application quality."}),
    )?;
    writeln!(
        std::io::stdout().lock(),
        "installed direct process/model and coordinator workflows completed; retain this directory for explicit restart replay"
    )?;
    Ok(())
}

fn check_result(name: &str, result: &[u8], process: &[u8]) -> EvidenceResult {
    ensure(
        if name == "process" {
            result == process
        } else {
            !result.is_empty() && std::str::from_utf8(result).is_ok()
        },
        "installed operation returned invalid or different bytes",
    )
}

fn replay(serving: &CliRunner, coordinator: &CliRunner, root: &Path) -> EvidenceResult {
    let completed: Value = serde_json::from_slice(&fs::read(root.join("completed.json"))?)?;
    let downloads = tempfile::Builder::new()
        .prefix("replay-")
        .tempdir_in(root)?;
    for record in completed
        .pointer("/direct")
        .ok_or("missing /direct")?
        .as_array()
        .ok_or("direct records absent")?
    {
        let path = root.join(required_text(record, &["request"])?);
        let result = serving.success(&["invocation", "submit", path_text(&path)?])?;
        ensure(
            (result.pointer("/value/replayed").and_then(Value::as_bool) == Some(true)
                || result.pointer("/value/type").and_then(Value::as_str) == Some("archived"))
                && result
                    .pointer("/value/execution")
                    .ok_or("missing /value/execution")?
                    == record.get("execution").ok_or("execution absent")?,
            "restart changed direct acceptance",
        )?;
        let execution = required_text(record, &["execution"])?;
        let pages = observe(
            serving,
            &execution,
            downloads.path(),
            &format!("{execution}-observations.json"),
        )?;
        ensure(
            successful_outputs(&pages)?.iter().any(|(_, artifact)| {
                serde_json::to_value(artifact).ok().as_ref() == Some(&record["output"])
            }),
            "restart lost direct output",
        )?;
        let name = required_text(record, &["output_file"])?;
        let artifact = required_text(record, &["output", "identity"])?;
        let bytes = download(serving, downloads.path(), &artifact, &name)?;
        ensure(
            bytes == fs::read(root.join(&name))?,
            "restart changed direct output bytes",
        )?;
    }
    for record in completed
        .pointer("/workflows")
        .ok_or("missing /workflows")?
        .as_array()
        .ok_or("workflow records absent")?
    {
        let run = required_text(record, &["run"])?;
        let command = required_text(record, &["command"])?;
        let revision = required_text(record, &["revision"])?;
        coordinator.success(&[
            "--command-id",
            &command,
            "run",
            "start",
            &run,
            &run,
            &revision,
        ])?;
        let state = coordinator.success(&["run", "show", &run])?;
        ensure(
            state
                .pointer("/value/terminal")
                .ok_or("missing /value/terminal")?
                == "succeeded",
            "restart lost successful workflow",
        )?;
        let node = state
            .pointer("/value/nodes")
            .ok_or("missing /value/nodes")?
            .as_array()
            .and_then(|nodes| nodes.iter().find(|n| n["node_id"] == "operation"))
            .ok_or("replayed operation absent")?;
        ensure(
            node["attempt_count"] == 1 && node["latest_attempt_id"] == record["attempt"],
            "restart created another workflow attempt",
        )?;
        let attempt = required_text(record, &["attempt"])?;
        let inspected = coordinator.success(&["attempt", "inspect", &run, &attempt])?;
        ensure(
            inspected
                .pointer("/value/invocation_id")
                .ok_or("missing /value/invocation_id")?
                == record.get("invocation").ok_or("invocation absent")?,
            "restart changed remote invocation",
        )?;
        let name = required_text(record, &["output_file"])?;
        let artifact = required_text(record, &["output", "artifact_id"])?;
        let bytes = download(coordinator, downloads.path(), &artifact, &name)?;
        ensure(
            bytes == fs::read(root.join(&name))?,
            "restart changed workflow output bytes",
        )?;
    }
    save(
        &downloads.keep(),
        "replay.json",
        &json!({"schema_version":1,"direct_acceptances_retained":true,"single_workflow_attempts_retained":true,"output_bytes_rechecked":true}),
    )?;
    writeln!(
        std::io::stdout().lock(),
        "installed restart replay retained direct acceptances, outputs and single workflow attempts; external counters must be checked separately"
    )?;
    Ok(())
}
