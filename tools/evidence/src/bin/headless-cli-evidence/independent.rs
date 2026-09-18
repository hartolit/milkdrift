//! Black-box independent calls use public upload, discovery, saved requests and observations.
#[path = "independent/workflow.rs"]
mod workflow;
use super::{Arguments, TOKEN, setup};
use milkdrift_capability::{InputReference, InvocationValueReference};
use milkdrift_evidence::{
    EvidenceResult,
    application::{
        CliRunner, ensure, path_text, reserve_endpoint, start_daemon, wait_for_readiness,
        write_private, write_process_profile,
    },
};
use milkdrift_peer_protocol::{DirectDiscovery, InvocationAcceptance, ObservationPage};
use milkdrift_persistence::{
    ArtifactStore, PageSize, RunQueryStore, RunSummaryFilter, RunSummaryPageQuery,
};
use milkdrift_workspace::{ArtifactId, CausalReference};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read as _, Write as _},
    path::Path,
    sync::atomic::Ordering,
};

pub(super) fn process_fixture() -> EvidenceResult {
    let marker = std::env::args_os()
        .nth(2)
        .ok_or("external marker argument absent")?;
    let mut bytes = Vec::new();
    std::io::stdin().take(4097).read_to_end(&mut bytes)?;
    ensure(bytes.len() <= 4096, "fixture input exceeds bound")?;
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(marker)?
        .write_all(&bytes)?;
    std::io::stdout().write_all(&bytes)?;
    Ok(())
}

pub(super) fn run(arguments: &Arguments) -> EvidenceResult {
    let root = tempfile::tempdir()?;
    let directory = root.path();
    let endpoint = reserve_endpoint()?;
    let executable = std::env::current_exe()?;
    let marker = directory.join("external-entries");
    let process_profile = write_process_profile(
        directory,
        &executable,
        "independent-process-profile",
        "independent-process",
        "--fixture-independent",
        "non_idempotent_write",
        Some("stdout"),
    )?;
    let mut profile: Value = serde_json::from_slice(&fs::read(&process_profile)?)?;
    profile["profile"]["arguments"] = json!(["--fixture-independent", marker]);
    profile["profile"]["inputs"] = json!([{"input":"source","relative_path":"source.txt"}]);
    profile["profile"]["stdin"] = json!({"type":"input","input":"source","max_bytes":4096});
    fs::write(&process_profile, serde_json::to_vec(&profile)?)?;
    let token = write_private(&directory.join("operator.token"), TOKEN.as_bytes())?;
    let mut model = setup::MockModel::with_text("independent model result".to_owned())?;
    let config_path = setup::configure(
        &arguments.examples,
        directory,
        endpoint,
        &arguments.daemon,
        vec![process_profile],
        &model,
    )?;
    let mut config: Value = serde_json::to_value(toml::from_str::<toml::Value>(
        &fs::read_to_string(&config_path)?,
    )?)?;
    config["role"] = json!("execution_only");
    config["host_id"] = json!("independent-evidence-host");
    // The fixture is explicitly unbilled; this is not inferred from loopback locality.
    let model_path = directory.join("model-profile.json");
    let mut model_profile: Value = serde_json::from_slice(&fs::read(&model_path)?)?;
    model_profile["billing"] =
        json!({"type":"unbilled","source":"deterministic fixture performs no provider billing"});
    model_profile["token_limits"] = json!({"type":"byte_bpe","template_tokens_per_message":32,"template_tokens_per_request":64,"maximum_input_tokens":65536,"maximum_output_tokens":4096,"output_control":"max_tokens","source":"deterministic fixture response is one bounded token sequence"});
    model_profile["limits"]["max_request_bytes"] = json!(65536);
    model_profile["limits"]["max_response_bytes"] = json!(65536);
    fs::write(&model_path, serde_json::to_vec(&model_profile)?)?;
    fs::remove_file(&config_path)?;
    let config_path = setup::checked_config(directory, &arguments.daemon, &config)?;
    let runner = CliRunner {
        executable: arguments.cli.clone(),
        endpoint: format!("http://{endpoint}/"),
        token_file: token,
        forbidden_storage_path: directory.join("data"),
    };
    let mut daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    ensure(
        runner.success(&["daemon", "health"])?["value"]["role"] == "execution_only",
        "binary reported another role",
    )?;
    let process_bytes = b"public uploaded process input\n";
    let process_input = upload(
        &runner,
        directory,
        "process-input",
        "text/plain",
        process_bytes,
    )?;
    let process_request = request(
        &runner,
        directory,
        "independent-process",
        "process.execute",
        "process-direct",
        "source",
        process_input,
    )?;
    let (process_saved, process_execution, process_page) =
        invoke(&runner, directory, &process_request)?;
    ensure(
        fs::read(&marker)? == process_bytes,
        "process entry counter did not record exactly one input",
    )?;
    let process_output = output(&process_page, "stdout")?;
    ensure(
        download(
            &runner,
            directory,
            process_output.identity(),
            "process-result",
        )? == process_bytes,
        "direct process returned different bytes",
    )?;
    let task = json!({"schema_version":1,"request":{"messages":[{"role":"user","parts":[{"type":"text","text":"Return the deterministic independent result."}],"tool_call_id":null}],"tools":[],"structured_output":null,"session":{"type":"fresh"},"reasoning":null,"maximum_output_units":64,"streaming":true,"extensions":{}}});
    let model_input = upload(
        &runner,
        directory,
        "model-input",
        "application/json",
        &serde_json::to_vec(&task)?,
    )?;
    let model_request = request(
        &runner,
        directory,
        "operator-model",
        "model.generate",
        "model-direct",
        "milkdrift.model_task",
        model_input,
    )?;
    let (model_saved, model_execution, model_page) = invoke(&runner, directory, &model_request)?;
    let model_output = output(&model_page, "final_text")?;
    ensure(
        String::from_utf8(download(
            &runner,
            directory,
            model_output.identity(),
            "model-result",
        )?)?
        .contains("independent model result"),
        "fresh direct model result missing",
    )?;
    ensure(
        model.invocations.load(Ordering::SeqCst) == 1
            && model.direct_invocations.load(Ordering::SeqCst) == 1,
        "model did not receive exactly one explicit direct selection",
    )?;
    for (saved, execution) in [
        (&process_saved, &process_execution),
        (&model_saved, &model_execution),
    ] {
        let replay = runner.success(&["invocation", "submit", path_text(saved)?])?;
        ensure(
            replay["value"]["replayed"] == true && replay["value"]["execution"] == *execution,
            "hot exact replay changed acceptance",
        )?;
        let inspected = runner.success(&["invocation", "show", execution])?;
        ensure(
            inspected["value"]["origin"]["type"] == "direct",
            "independent invocation acquired workflow provenance",
        )?;
    }
    daemon.terminate()?;
    {
        let store = milkdrift_redb_store::RedbStore::open(directory.join("data"))?;
        ensure(
            store
                .run_summaries(&RunSummaryPageQuery {
                    filter: RunSummaryFilter::default(),
                    cursor: None,
                    limit: PageSize::new(1)?,
                })?
                .runs
                .is_empty(),
            "independent daemon wrote fake run records",
        )?;
        for artifact in [&process_output, &model_output] {
            let metadata = store
                .metadata(&ArtifactId::new(artifact.identity())?)?
                .ok_or("produced metadata absent")?;
            ensure(
                matches!(
                    metadata.provenance().producer(),
                    CausalReference::HostInvocation { .. }
                ),
                "direct output producer is not a host invocation",
            )?;
        }
    }
    daemon = start_daemon(&arguments.daemon, &config_path)?;
    wait_for_readiness(&runner, &mut daemon)?;
    for saved in [&process_saved, &model_saved] {
        ensure(
            runner.success(&["invocation", "submit", path_text(saved)?])?["value"]["replayed"]
                == true,
            "restart did not retain exact acceptance",
        )?;
    }
    ensure(
        fs::read(&marker)? == process_bytes && model.invocations.load(Ordering::SeqCst) == 1,
        "restart repeated external work",
    )?;
    daemon.terminate()?;
    workflow::run(
        arguments,
        directory,
        &config,
        &runner,
        process_bytes,
        &task,
        &model,
    )?;
    ensure(
        fs::read(&marker)? == [process_bytes.as_slice(), process_bytes.as_slice()].concat(),
        "workflow remote process did not enter the same operation exactly once",
    )?;
    ensure(
        model.invocations.load(Ordering::SeqCst) == 3
            && model.direct_invocations.load(Ordering::SeqCst) == 1,
        "workflow remote model did not retain causal selection",
    )?;
    model.finish()?;
    println!(
        "independent hosting: actual daemon/CLI process and fresh model outputs, public upload/download, exact replay/restart, direct provenance and no workflow records verified"
    );
    Ok(())
}

fn upload(
    runner: &CliRunner,
    directory: &Path,
    name: &str,
    media: &str,
    bytes: &[u8],
) -> EvidenceResult<milkdrift_capability::ArtifactReference> {
    let file = directory.join(name);
    fs::write(&file, bytes)?;
    let discovery: DirectDiscovery = serde_json::from_value(
        runner
            .success(&["invocation", "catalog"])
            .map_err(|error| format!("input owner discovery for {name}: {error}"))?["value"]
            .clone(),
    )?;
    let value = runner.success(&[
        "artifact",
        "upload",
        path_text(&file)?,
        "--host",
        discovery.host.as_str(),
        "--upload-id",
        name,
        "--media-type",
        media,
    ])?;
    let metadata: milkdrift_control_protocol::ArtifactMetadataRead =
        serde_json::from_value(value["value"].clone())?;
    Ok(milkdrift_capability::ArtifactReference::new(
        metadata.artifact_id,
        metadata.digest,
        Some(metadata.content_type),
        Some(metadata.size),
    )?)
}

fn request(
    runner: &CliRunner,
    directory: &Path,
    capability: &str,
    operation: &str,
    id: &str,
    input_name: &str,
    input: milkdrift_capability::ArtifactReference,
) -> EvidenceResult<std::path::PathBuf> {
    let value = runner.success(&["invocation", "catalog"])?;
    let discovery: DirectDiscovery = serde_json::from_value(value["value"].clone())?;
    let inputs = directory.join(format!("{id}.inputs.json"));
    fs::write(
        &inputs,
        serde_json::to_vec(&vec![InputReference::new(
            input_name,
            InvocationValueReference::Artifact { reference: input },
        )?])?,
    )?;
    let file = directory.join(format!("{id}.json"));
    runner.success(&[
        "invocation",
        "prepare",
        capability,
        operation,
        "--host",
        discovery.host.as_str(),
        "--request-id",
        id,
        "--inputs",
        path_text(&inputs)?,
        "--output",
        path_text(&file)?,
    ])?;
    Ok(file)
}

fn invoke(
    runner: &CliRunner,
    _directory: &Path,
    file: &Path,
) -> EvidenceResult<(std::path::PathBuf, String, ObservationPage)> {
    let value = runner.success(&["invocation", "submit", path_text(file)?])?;
    let acceptance: InvocationAcceptance = serde_json::from_value(value["value"].clone())?;
    let InvocationAcceptance::Accepted { execution, .. } = acceptance else {
        return Err("direct admission was not accepted".into());
    };
    let waiting = runner.run(
        &[
            "--timeout-secs",
            "8",
            "invocation",
            "wait",
            execution.as_str(),
        ],
        None,
    )?;
    ensure(
        waiting.status.success(),
        &format!("direct invocation failed: {}", waiting.stdout),
    )?;
    let value = runner.success(&[
        "invocation",
        "observations",
        execution.as_str(),
        "--limit",
        "128",
    ])?;
    let page: ObservationPage = serde_json::from_value(value["value"].clone())?;
    ensure(page.terminal, "invocation omitted terminal evidence")?;
    Ok((file.to_owned(), execution.to_string(), page))
}

fn output(
    page: &ObservationPage,
    name: &str,
) -> EvidenceResult<milkdrift_capability::ArtifactReference> {
    Ok(page
        .observations
        .iter()
        .filter_map(|observation| observation.event.kind().output())
        .find(|(candidate, _)| *candidate == name)
        .ok_or("expected output artifact absent")?
        .1
        .clone())
}

fn download(
    runner: &CliRunner,
    directory: &Path,
    artifact: &str,
    name: &str,
) -> EvidenceResult<Vec<u8>> {
    let path = directory.join(name);
    runner.success(&["artifact", "get", artifact, "--output", path_text(&path)?])?;
    Ok(fs::read(path)?)
}
