//! Actual-binary local-model smoke evidence with a deterministic parser fixture.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use clap::{Parser, ValueEnum};
use milkdrift_authority::{NetworkProfileRef, NetworkScope, SecretRef};
use milkdrift_blueprint::{
    AuthorRef, BindingSource, BlueprintRevision, BlueprintRevisionDocument, ContextBudget,
    ContextCategory, ContextOrdering, ContextSemanticRole, ContextSessionPolicy, ContextTruncation,
    DataPort, Edge, EdgeId, EdgeKind, Mutation, MutationBatch, Node, NodeId, NodeKind, PortId,
    SchemaRef, TaskConfig, TaskContextPolicy, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{
    BoundedJson, CapabilityId, CapabilityRequirement, OperationId, ProviderProfileRef, SchemaId,
    SideEffectClass, StreamingMode,
};
use milkdrift_daemon::{ActorGrantConfig, ModelProfileConfig, SecretSourceConfig};
use milkdrift_local_process::{PlatformSupport, ProcessProfileDocument};
use milkdrift_model::{
    ContentPart, FinishReason, MODEL_TASK_INPUT_NAME, Message, MessageRole, ModelResponseDocument,
    ModelTaskRequest, ModelTaskRequestDocument, SessionSelection,
};
use milkdrift_model_provider::EndpointProfile;
use serde_json::{Value, json};

#[path = "headless_cli_evidence/harness.rs"]
#[allow(dead_code)]
// This binary reuses only the actual-process subset of the shared evidence harness.
mod harness;

use harness::{
    CliRunner, EvidenceConfig, reserve_endpoint, start_daemon, stop_daemon, wait_for_readiness,
    wait_for_run, write_config,
};

type EvidenceResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const ACTOR: &str = "human:local-model-evidence";
const TOKEN: &str = "local-model-evidence-control-token";
const SUCCESS_CAPABILITY: &str = "local-model-dogfood";
const FAILURE_CAPABILITY: &str = "local-model-dogfood-uncertain";
const SUCCESS_WORKFLOW: &str = "local-model-dogfood";
const SUCCESS_RUN: &str = "run-local-model-dogfood";
const FAILURE_WORKFLOW: &str = "local-model-uncertainty";
const FAILURE_RUN: &str = "run-local-model-uncertainty";

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Mode {
    Deterministic,
    OperatorRealEndpoint,
}

#[derive(Parser)]
#[command(name = "local-model-evidence")]
struct Arguments {
    /// Built `milkdrift-daemon` executable.
    #[arg(long)]
    daemon: PathBuf,
    /// Built `milkdrift` executable.
    #[arg(long)]
    cli: PathBuf,
    /// Built byte-pinned deterministic process helper used only to produce context evidence.
    #[arg(long)]
    process_helper: PathBuf,
    /// Deterministic parser fixture or an explicitly supplied real endpoint profile.
    #[arg(long, value_enum, default_value_t = Mode::Deterministic)]
    mode: Mode,
    /// Real endpoint-profile schema-v1 document. Required only in real mode.
    #[arg(long)]
    model_profile: Option<PathBuf>,
    /// Capability identity assigned to the real profile.
    #[arg(long, default_value = SUCCESS_CAPABILITY)]
    model_capability: String,
    /// Explicit `secret-reference=/private/file` mapping, repeatable.
    #[arg(long = "secret-source", value_parser = parse_secret_source)]
    secret_sources: Vec<(String, PathBuf)>,
    /// Empty untracked evidence directory; `report.json` and disposable session state are written here.
    #[arg(long, default_value = "target/local-model-evidence")]
    output: PathBuf,
}

#[derive(Clone, Debug)]
struct ModelFacts {
    profile_id: String,
    revision: u64,
    protocol: String,
    model_alias: String,
    endpoint_origin: String,
    streaming: bool,
    secret_refs: BTreeSet<String>,
}

struct ControlledEndpoint {
    address: SocketAddr,
    requests: Arc<AtomicUsize>,
    task: Option<thread::JoinHandle<std::io::Result<String>>>,
}

impl ControlledEndpoint {
    fn success() -> EvidenceResult<Self> {
        Self::start(true)
    }

    fn close_after_request() -> EvidenceResult<Self> {
        Self::start(false)
    }

    fn start(success: bool) -> EvidenceResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let task = thread::spawn(move || {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(10)))?;
            let request = read_request(&mut stream)?;
            observed.fetch_add(1, Ordering::SeqCst);
            if success {
                let body = [
                    format!(
                        "data: {}\n\n",
                        json!({"id":"fixture-response-1","model":"fixture-local-model","choices":[{"delta":{"content":"MILKDRIFT_"},"finish_reason":null}]})
                    ),
                    format!(
                        "data: {}\n\n",
                        json!({"id":"fixture-response-1","model":"fixture-local-model","choices":[{"delta":{"content":"MODEL_OK"},"finish_reason":"stop"}],"usage":{"prompt_tokens":19,"completion_tokens":4}})
                    ),
                    "data: [DONE]\n\n".to_owned(),
                ]
                .concat();
                write_response(&mut stream, "text/event-stream", &body)?;
            }
            Ok(request)
        });
        Ok(Self {
            address,
            requests,
            task: Some(task),
        })
    }

    fn join(&mut self) -> EvidenceResult<String> {
        self.task
            .take()
            .ok_or("controlled endpoint already joined")?
            .join()
            .map_err(|_| "controlled endpoint panicked")?
            .map_err(Into::into)
    }
}

impl Drop for ControlledEndpoint {
    fn drop(&mut self) {
        if self.task.is_none() {
            return;
        }
        if let Ok(mut stream) = TcpStream::connect_timeout(&self.address, Duration::from_secs(1)) {
            let _ = stream.write_all(
                b"GET /shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

fn main() {
    if let Err(error) = run(Arguments::parse()) {
        eprintln!("local model evidence failed: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: Arguments) -> EvidenceResult {
    require_file(&arguments.daemon, "daemon")?;
    require_file(&arguments.cli, "CLI")?;
    require_file(&arguments.process_helper, "process helper")?;
    validate_mode(&arguments)?;
    let process_helper = arguments.process_helper.canonicalize()?;
    let output = prepare_output(&arguments.output)?;
    let session = output.join("session");
    fs::create_dir_all(&session)?;
    let token_file = write_private(&session.join("controller.token"), TOKEN.as_bytes())?;
    let selected_profile = write_text_evidence_profile(
        &session,
        &process_helper,
        "local-model-selected-evidence",
        "local-model-selected-evidence",
        "--selected-evidence",
    )?;
    let omitted_profile = write_text_evidence_profile(
        &session,
        &process_helper,
        "local-model-omitted-evidence",
        "local-model-omitted-evidence",
        "--omitted-evidence",
    )?;

    let mut success_endpoint = None;
    let success_profile = match arguments.mode {
        Mode::Deterministic => {
            let endpoint = ControlledEndpoint::success()?;
            let path = write_model_profile(
                &session,
                "local-model-deterministic",
                endpoint.address,
                "fixture-local-model",
            )?;
            success_endpoint = Some(endpoint);
            path
        }
        Mode::OperatorRealEndpoint => arguments
            .model_profile
            .clone()
            .ok_or("real mode requires --model-profile")?,
    };
    let success_facts = inspect_profile(&success_profile)?;
    validate_real_profile(arguments.mode, &success_profile, &success_facts)?;

    let mut failure_endpoint = ControlledEndpoint::close_after_request()?;
    let failure_profile = write_model_profile(
        &session,
        "local-model-controlled-failure",
        failure_endpoint.address,
        "controlled-failure-model",
    )?;
    let failure_facts = inspect_profile(&failure_profile)?;
    let mut secret_sources = BTreeMap::new();
    for (reference, path) in &arguments.secret_sources {
        if secret_sources
            .insert(
                reference.clone(),
                SecretSourceConfig::File { path: path.clone() },
            )
            .is_some()
        {
            return Err(format!("duplicate secret source '{reference}'").into());
        }
    }
    ensure(
        success_facts
            .secret_refs
            .iter()
            .all(|reference| secret_sources.contains_key(reference)),
        "real profile secret reference lacks an explicit --secret-source mapping",
    )?;
    let mut authority = ActorGrantConfig::dangerous_administrator();
    authority.resources.network = NetworkScope::new(
        BTreeSet::from([
            NetworkProfileRef::new(success_facts.profile_id.clone())?,
            NetworkProfileRef::new(failure_facts.profile_id.clone())?,
        ]),
        BTreeSet::from([
            profile_destination(&success_facts.endpoint_origin)?,
            profile_destination(&failure_facts.endpoint_origin)?,
        ]),
    )?;
    authority.resources.secrets = success_facts
        .secret_refs
        .iter()
        .map(|reference| SecretRef::new(reference.clone()))
        .collect::<Result<_, _>>()?;
    let bind = reserve_endpoint()?;
    let config = write_config(
        &session,
        bind,
        &token_file,
        EvidenceConfig {
            process_profiles: vec![selected_profile, omitted_profile],
            model_profiles: vec![
                ModelProfileConfig {
                    capability_id: arguments.model_capability.clone(),
                    profile: success_profile.clone(),
                },
                ModelProfileConfig {
                    capability_id: FAILURE_CAPABILITY.to_owned(),
                    profile: failure_profile,
                },
            ],
            secret_sources,
            lease_duration_ms: 5_000,
            authority,
        },
    )?;
    let runner = CliRunner {
        executable: arguments.cli.clone(),
        endpoint: format!("http://{bind}/"),
        token_file,
        forbidden_storage_path: session.join("data"),
    };

    let success_blueprint = model_revision(
        SUCCESS_WORKFLOW,
        &arguments.model_capability,
        &success_facts,
        true,
    )?;
    let success_document =
        BlueprintRevisionDocument::new(&success_blueprint).to_canonical_json()?;
    let success_path = session.join("model-blueprint.json");
    fs::write(&success_path, &success_document)?;
    let failure_blueprint =
        model_revision(FAILURE_WORKFLOW, FAILURE_CAPABILITY, &failure_facts, false)?;
    let failure_document =
        BlueprintRevisionDocument::new(&failure_blueprint).to_canonical_json()?;
    let failure_path = session.join("uncertain-blueprint.json");
    fs::write(&failure_path, &failure_document)?;

    let mut daemon = start_daemon(&arguments.daemon, &config)?;
    wait_for_readiness(&runner, &mut daemon)?;
    runner.success(&["daemon", "readiness"])?;
    assert_capability(&runner, &arguments.model_capability, &success_facts)?;
    runner.success_with_input(
        &[
            "--command-id",
            "local-model-validate-1",
            "blueprint",
            "validate",
            "-",
        ],
        &success_document,
    )?;
    runner.success(&[
        "--command-id",
        "local-model-import-1",
        "blueprint",
        "import",
        path_text(&success_path)?,
    ])?;
    runner.success(&[
        "--command-id",
        "local-model-start-1",
        "run",
        "start",
        SUCCESS_RUN,
        SUCCESS_WORKFLOW,
        success_blueprint.id().as_str(),
    ])?;
    let waiting = wait_for_run(&runner, SUCCESS_RUN, |run| {
        node(run, "model-release").is_some() && node(run, "model").is_none()
    })?;
    let waiting_sequence = required_u64(&waiting, &["value", "sequence"])?;
    stop_daemon(&mut daemon)?;
    daemon = start_daemon(&arguments.daemon, &config)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let reopened_wait = runner.success(&["run", "show", SUCCESS_RUN])?;
    ensure(
        required_u64(&reopened_wait, &["value", "sequence"])? == waiting_sequence
            && node(&reopened_wait, "model").is_none(),
        "restart changed the unreleased wait boundary or entered the model",
    )?;
    let mut follower = spawn_timeline_follow(&runner, SUCCESS_RUN)?;
    runner.success(&[
        "--command-id",
        "local-model-signal-1",
        "--expected-sequence",
        &waiting_sequence.to_string(),
        "run",
        "signal",
        SUCCESS_RUN,
        "--signal-id",
        "local-model-release-1",
        "--signal-type",
        "evidence.model.release",
        "--payload",
        r#"{"release":true}"#,
    ])?;
    let completed = wait_for_run(&runner, SUCCESS_RUN, |run| {
        run["value"]["terminal"] == "succeeded"
    })?;
    let follower_output = stop_follower(&mut follower)?;
    ensure(
        follower_output.contains("run.timeline") && follower_output.contains("run.observation"),
        "timeline follow did not expose both its bounded page and resumed observations",
    )?;
    let model_node = node(&completed, "model").ok_or("model execution is absent")?;
    ensure(
        model_node["attempt_count"] == 1,
        "clean model workflow created more than one attempt",
    )?;
    let omitted_node =
        node(&completed, "omitted-evidence").ok_or("omitted evidence execution is absent")?;
    let omitted_attempt_id = required_text(omitted_node, &["latest_attempt_id"])?;
    let omitted_attempt =
        runner.success(&["attempt", "inspect", SUCCESS_RUN, &omitted_attempt_id])?;
    let omitted_artifact_id = omitted_attempt["value"]["outputs"]
        .as_array()
        .and_then(|outputs| outputs.iter().find(|output| output["name"] == "evidence"))
        .and_then(|output| output["artifact"]["artifact_id"].as_str())
        .ok_or("omitted evidence attempt did not publish its artifact")?
        .to_owned();
    let attempt_id = required_text(model_node, &["latest_attempt_id"])?;
    let attempt = runner.success(&["attempt", "inspect", SUCCESS_RUN, &attempt_id])?;
    let success_observation = assert_success_attempt(
        &runner,
        &attempt,
        &arguments.model_capability,
        &success_facts,
        &session,
        &omitted_artifact_id,
    )?;

    let captured_request = match success_endpoint.as_mut() {
        Some(endpoint) => {
            let request = endpoint.join()?;
            ensure(
                request.contains("selected architecture evidence")
                    && !request.contains("omitted unrelated evidence"),
                "deterministic provider request contradicted frozen context selection",
            )?;
            ensure(
                endpoint.requests.load(Ordering::SeqCst) == 1,
                "deterministic success endpoint observed duplicate entry",
            )?;
            true
        }
        None => false,
    };

    stop_daemon(&mut daemon)?;
    daemon = start_daemon(&arguments.daemon, &config)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let reopened = runner.success(&["run", "show", SUCCESS_RUN])?;
    let reopened_model = node(&reopened, "model").ok_or("reopened model execution is absent")?;
    ensure(
        reopened_model["attempt_count"] == 1 && reopened_model["latest_attempt_id"] == attempt_id,
        "restart duplicated or replaced the successful model attempt",
    )?;
    let reopened_attempt = runner.success(&["attempt", "inspect", SUCCESS_RUN, &attempt_id])?;
    ensure(
        reopened_attempt["value"]["outputs"] == attempt["value"]["outputs"],
        "restart changed successful model artifact publications",
    )?;

    let uncertainty = run_uncertainty_scenario(
        &runner,
        &failure_blueprint,
        &failure_document,
        &failure_path,
    )?;
    let failure_request = failure_endpoint.join()?;
    ensure(
        !failure_request.contains("HTTP/1.1 200"),
        "controlled request capture unexpectedly contains a response",
    )?;
    ensure(
        failure_endpoint.requests.load(Ordering::SeqCst) == 1,
        "unsupported-idempotency model work was automatically retried",
    )?;
    stop_daemon(&mut daemon)?;
    daemon = start_daemon(&arguments.daemon, &config)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let retained = runner.success(&["attempt", "inspect", FAILURE_RUN, &uncertainty.attempt_id])?;
    ensure(
        retained["value"]["uncertain"] == true,
        "retained uncertainty disappeared after restart",
    )?;
    stop_daemon(&mut daemon)?;

    let report = json!({
        "schema_version": 1,
        "kind": "local_model_smoke",
        "qualifying": false,
        "mode": match arguments.mode { Mode::Deterministic => "deterministic", Mode::OperatorRealEndpoint => "operator_real_endpoint" },
        "model": {
            "capability": arguments.model_capability,
            "profile": success_facts.profile_id,
            "profile_revision": success_facts.revision,
            "protocol": success_facts.protocol,
            "model_alias": success_facts.model_alias,
            "endpoint_origin": success_facts.endpoint_origin,
        },
        "success": {
            "run": SUCCESS_RUN,
            "attempt": attempt_id,
            "attempt_count": 1,
            "adapter_entry_count_observed": if captured_request { Value::from(1) } else { Value::Null },
            "terminal": "success",
            "side_effect": "unknown",
            "idempotency": "unsupported",
            "cancellation": "best_effort",
            "selected_context_observed": true,
            "omitted_context_observed": true,
            "progress_observations": success_observation.progress,
            "usage_input_supplied": success_observation.usage_input,
            "usage_output_supplied": success_observation.usage_output,
            "finish_reason_supplied": success_observation.finish_reason,
            "response_identity_supplied": success_observation.response_identity,
            "artifact_count": success_observation.artifacts,
            "restart_duplicate_free": true,
        },
        "uncertainty": {
            "run": FAILURE_RUN,
            "attempt": uncertainty.attempt_id,
            "adapter_entry_count": 1,
            "automatic_retry_refused": true,
            "authorized_retry_refused": true,
            "retained": true,
            "restart_duplicate_free": true,
        },
        "strict_external_evidence": {
            "run": false,
            "reason": "model-only smoke never qualifies the combined real-agent and real-model gate"
        }
    });
    let report_path = output.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("local model evidence passed: {}", report_path.display());
    Ok(())
}

#[derive(Clone, Debug)]
struct SuccessObservation {
    progress: u64,
    usage_input: bool,
    usage_output: bool,
    finish_reason: bool,
    response_identity: bool,
    artifacts: usize,
}

#[derive(Clone, Debug)]
struct UncertaintyObservation {
    attempt_id: String,
}

struct TimelineFollower {
    child: Child,
    lines: mpsc::Receiver<String>,
    reader: Option<thread::JoinHandle<std::io::Result<()>>>,
    observed: Vec<String>,
}

fn validate_mode(arguments: &Arguments) -> EvidenceResult {
    CapabilityId::new(&arguments.model_capability)?;
    match arguments.mode {
        Mode::Deterministic => ensure(
            arguments.model_profile.is_none() && arguments.secret_sources.is_empty(),
            "deterministic mode rejects real profile and secret arguments",
        ),
        Mode::OperatorRealEndpoint => ensure(
            arguments.model_profile.is_some(),
            "operator real-endpoint mode requires --model-profile and never falls back",
        ),
    }
}

fn prepare_output(path: &Path) -> EvidenceResult<PathBuf> {
    ensure(
        !path.components().any(|part| part == Component::ParentDir),
        "evidence output cannot contain parent traversal",
    )?;
    let current = std::env::current_dir()?.canonicalize()?;
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        current.join(path)
    };
    if absolute.starts_with(&current) {
        ensure(
            absolute.starts_with(current.join("target")),
            "evidence output inside the checkout must remain under target/",
        )?;
    }
    fs::create_dir_all(&absolute)?;
    ensure(
        fs::read_dir(&absolute)?.next().is_none(),
        "selected evidence output directory must be empty",
    )?;
    Ok(absolute)
}

fn parse_secret_source(value: &str) -> Result<(String, PathBuf), String> {
    let (reference, path) = value
        .split_once('=')
        .ok_or_else(|| "secret source must be REFERENCE=FILE".to_owned())?;
    if reference.is_empty() || path.is_empty() {
        return Err("secret source must contain a nonempty reference and file".to_owned());
    }
    Ok((reference.to_owned(), PathBuf::from(path)))
}

fn write_text_evidence_profile(
    directory: &Path,
    executable: &Path,
    profile_id: &str,
    capability: &str,
    fixture_argument: &str,
) -> EvidenceResult<PathBuf> {
    let (content_digest, executable_size) = hash_file(executable)?;
    let executable_root = executable
        .parent()
        .ok_or("evidence executable has no parent")?;
    let value = json!({
        "schema_version": 2,
        "profile": {
            "profile_id": profile_id,
            "revision": 1,
            "capability": capability,
            "descriptor_revision": 1,
            "provider_profile": null,
            "operation": "process.execute",
            "side_effect": "read_only",
            "idempotency": "unsupported",
            "cancellation": "best_effort",
            "trust_class": "trusted_host_process",
            "executable": executable,
            "implementation": {
                "content_digest": content_digest,
                "size_bytes": executable_size,
                "package_revision": "local-model-evidence-v1",
                "documentation_reference": "urn:milkdrift:local-model-evidence"
            },
            "arguments": [fixture_argument],
            "substitutions": {},
            "working_directory": {"type":"isolated_root"},
            "filesystem_roots": [
                {"path": executable_root, "access":"execute"},
                {"path": directory, "access":"read_write"}
            ],
            "inputs": [],
            "environment": {"allowed_non_secret":[],"secrets":{},"max_value_bytes":4096},
            "stdin": {"type":"disabled"},
            "stdout": {"max_capture_bytes":4096,"stream_progress":false,"max_progress_events":0,"overflow_action":"terminate","artifact_name":null},
            "stderr": {"max_capture_bytes":4096,"stream_progress":false,"max_progress_events":0,"overflow_action":"terminate","artifact_name":null},
            "outputs": [{"name":"evidence","relative_path":"evidence.txt","media_type":"text/plain","required":true}],
            "limits": {
                "max_argv_entries":8,"max_argv_bytes":4096,"max_children_observed":4,
                "max_files":8,"max_file_bytes":1048576,"max_total_materialized_bytes":2097152,
                "max_path_bytes":4096,"max_directory_depth":16,"artifact_chunk_bytes":65536,
                "max_output_files":4,"max_total_output_bytes":2097152,"wall_timeout_ms":10000,
                "graceful_termination_ms":100,"forced_termination_ms":100,"heartbeat_interval_ms":100
            },
            "restart":"retain_uncertain",
            "platform": PlatformSupport::current(),
            "max_concurrent":1,
            "extensions":{"org.milkdrift/local-model-fixture":{"deterministic":true}}
        }
    });
    let document = ProcessProfileDocument::from_json(&serde_json::to_vec(&value)?)?;
    let path = directory.join(format!("{profile_id}.json"));
    fs::write(&path, document.to_canonical_json()?)?;
    Ok(path)
}

fn hash_file(path: &Path) -> EvidenceResult<(String, u64)> {
    let mut file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    ensure(
        metadata.is_file(),
        "evidence executable is not a regular file",
    )?;
    let mut hasher = blake3::Hasher::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size = size.saturating_add(u64::try_from(read)?);
    }
    ensure(
        size == metadata.len(),
        "evidence executable changed while hashed",
    )?;
    Ok((format!("b3_{}", hasher.finalize()), size))
}

fn write_model_profile(
    directory: &Path,
    identity: &str,
    endpoint: SocketAddr,
    model: &str,
) -> EvidenceResult<PathBuf> {
    let value = json!({
        "schema_version": 1,
        "identity": identity,
        "revision": 1,
        "protocol": {"type":"open_ai_compatible","path":"v1/chat/completions"},
        "base_url": format!("http://{endpoint}"),
        "model": model,
        "auth": {"type":"no_auth"},
        "limits": {
            "connect_timeout_ms": 2000,
            "request_timeout_ms": 10000,
            "idle_timeout_ms": 5000,
            "max_headers": 64,
            "max_header_bytes": 16384,
            "max_request_bytes": 1048576,
            "max_response_bytes": 1048576,
            "max_stream_line_bytes": 65536,
            "max_stream_event_bytes": 131072,
            "max_fragment_bytes": 4096
        },
        "redirect": "deny",
        "tls": "web_pki_roots",
        "proxy": "disabled",
        "features": ["streaming", "system_role"],
        "max_concurrent": 1,
        "local_development": true,
        "allowed_hosts": ["127.0.0.1"],
        "trust_zones": ["local-model-evidence"],
        "provider_options": {}
    });
    let bytes = serde_json::to_vec(&value)?;
    EndpointProfile::from_json(&bytes)?;
    let path = directory.join(format!("{identity}.model-profile.json"));
    fs::write(&path, bytes)?;
    Ok(path)
}

fn inspect_profile(path: &Path) -> EvidenceResult<ModelFacts> {
    let bytes = fs::read(path)?;
    let profile = EndpointProfile::from_json(&bytes)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    let base_url = required_text(&value, &["base_url"])?;
    let url = url::Url::parse(&base_url)?;
    let host = match url.host() {
        Some(url::Host::Ipv6(address)) => format!("[{address}]"),
        Some(url::Host::Ipv4(address)) => address.to_string(),
        Some(url::Host::Domain(name)) => name.to_owned(),
        None => return Err("model profile endpoint has no host".into()),
    };
    let port = url
        .port()
        .map(|value| format!(":{value}"))
        .unwrap_or_default();
    let features = value["features"]
        .as_array()
        .ok_or("model profile features are absent")?;
    let secret_refs = value["auth"]
        .get("secret")
        .and_then(Value::as_str)
        .map(|reference| BTreeSet::from([reference.to_owned()]))
        .unwrap_or_default();
    Ok(ModelFacts {
        profile_id: profile.identity().as_str().to_owned(),
        revision: value["revision"]
            .as_u64()
            .ok_or("model profile revision is absent")?,
        protocol: required_text(&value, &["protocol", "type"])?,
        model_alias: required_text(&value, &["model"])?,
        endpoint_origin: format!("{}://{host}{port}", url.scheme()),
        streaming: features
            .iter()
            .any(|feature| feature.as_str() == Some("streaming")),
        secret_refs,
    })
}

fn validate_real_profile(mode: Mode, path: &Path, facts: &ModelFacts) -> EvidenceResult {
    ensure(
        facts.protocol == "open_ai_compatible",
        "local-model lane reuses only the OpenAI-compatible chat-completions mapping",
    )?;
    if mode == Mode::Deterministic {
        return Ok(());
    }
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    let url = url::Url::parse(&required_text(&value, &["base_url"])?)?;
    let loopback = url.host().is_some_and(|host| match host {
        url::Host::Ipv4(address) => address.is_loopback(),
        url::Host::Ipv6(address) => address.is_loopback(),
        url::Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
    });
    ensure(
        url.scheme() == "http"
            && loopback
            && value["local_development"] == true
            && value["redirect"] == "deny"
            && value["proxy"] == "disabled",
        "real local-model profile must use explicit loopback HTTP development policy with redirects and ambient proxies disabled",
    )
}

fn profile_destination(origin: &str) -> EvidenceResult<String> {
    let url = url::Url::parse(origin)?;
    let host = match url.host() {
        Some(url::Host::Ipv6(address)) => format!("[{address}]"),
        Some(url::Host::Ipv4(address)) => address.to_string(),
        Some(url::Host::Domain(name)) => name.to_owned(),
        None => return Err("model endpoint origin has no host".into()),
    };
    let port = url
        .port_or_known_default()
        .ok_or("model endpoint origin has no port")?;
    Ok(format!("{host}:{port}"))
}

fn model_revision(
    workflow: &str,
    capability: &str,
    facts: &ModelFacts,
    with_evidence_and_wait: bool,
) -> EvidenceResult<BlueprintRevision> {
    let model = model_node(capability, facts, with_evidence_and_wait)?;
    let done = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(PortId::new("in")?)?;
    let mutations = if with_evidence_and_wait {
        let selected = evidence_node("selected-evidence", "local-model-selected-evidence")?;
        let omitted = evidence_node("omitted-evidence", "local-model-omitted-evidence")?
            .with_control_input(PortId::new("in")?)?;
        let release = Node::new(
            NodeId::new("model-release")?,
            NodeKind::SignalWait {
                signal: OperationId::new("evidence.model.release")?,
            },
        )?
        .with_control_input(PortId::new("in")?)?
        .with_control_output(PortId::new("next")?)?;
        vec![
            Mutation::AddNode { node: selected },
            Mutation::AddNode { node: omitted },
            Mutation::AddNode { node: release },
            Mutation::AddNode {
                node: model.with_control_input(PortId::new("in")?)?,
            },
            Mutation::AddNode { node: done },
            edge("selected-omitted", "selected-evidence", "omitted-evidence")?,
            edge("omitted-release", "omitted-evidence", "model-release")?,
            edge("release-model", "model-release", "model")?,
            edge("model-done", "model", "done")?,
        ]
    } else {
        vec![
            Mutation::AddNode { node: model },
            Mutation::AddNode { node: done },
            edge("model-done", "model", "done")?,
        ]
    };
    BlueprintRevision::genesis(
        WorkflowId::new(workflow)?,
        MutationBatch::new(mutations)?,
        AuthorRef::new(ACTOR)?,
        "actual daemon/CLI local-model interoperability evidence",
    )
    .map_err(Into::into)
}

fn evidence_node(identity: &str, capability: &str) -> EvidenceResult<Node> {
    let requirement = CapabilityRequirement::new(OperationId::new("process.execute")?)
        .exact(CapabilityId::new(capability)?)
        .maximum_side_effect(SideEffectClass::ReadOnly);
    let config = TaskConfig::new(requirement, TaskContextPolicy::default())?
        .with_output_context_roles(BTreeSet::from([ContextSemanticRole::Evidence]))?;
    Ok(
        Node::new(NodeId::new(identity)?, NodeKind::Task { config })?
            .with_control_output(PortId::new("next")?)?
            .with_data_output(
                PortId::new("evidence")?,
                DataPort::output(SchemaRef::new(
                    SchemaId::new("milkdrift.artifact-reference")?,
                    1,
                )?),
            )?,
    )
}

fn model_node(capability: &str, facts: &ModelFacts, select_evidence: bool) -> EvidenceResult<Node> {
    let mut requirement = CapabilityRequirement::new(OperationId::new("model.generate")?)
        .exact(CapabilityId::new(capability)?)
        .provider_profile(ProviderProfileRef::new(&facts.profile_id)?)
        .maximum_side_effect(SideEffectClass::Unknown);
    if facts.streaming {
        requirement = requirement.streaming(StreamingMode::OutputFragments);
    }
    let selected_nodes = if select_evidence {
        BTreeSet::from([NodeId::new("selected-evidence")?])
    } else {
        BTreeSet::new()
    };
    let policy = TaskContextPolicy::new(
        false,
        None,
        selected_nodes,
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::from([
            ContextCategory::RawProgress,
            ContextCategory::ToolTrace,
            ContextCategory::VerboseCommandOutput,
            ContextCategory::PriorPrompt,
        ]),
        None,
        ContextBudget::default(),
        ContextOrdering::default(),
        ContextTruncation::default(),
        ContextSessionPolicy::Fresh,
        select_evidence,
    )?;
    let config = TaskConfig::new(requirement, policy)?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text:
                    "Use only the selected evidence and return a short structural acknowledgement."
                        .to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        SessionSelection::Fresh,
        None,
        64,
        facts.streaming,
        BTreeMap::new(),
    )?;
    let task_value: Value =
        serde_json::from_slice(&ModelTaskRequestDocument::new(task).to_canonical_json()?)?;
    let mut node = Node::new(NodeId::new("model")?, NodeKind::Task { config })?
        .with_control_output(PortId::new("next")?)?
        .with_data_input(
            PortId::new(MODEL_TASK_INPUT_NAME)?,
            DataPort::input(
                SchemaRef::new(SchemaId::new("milkdrift.model-task")?, 1)?,
                true,
                Some(BindingSource::Literal {
                    value: BoundedJson::new(task_value)?,
                }),
            )?,
        )?;
    for output in ["model_response", "final_text", "provider_metadata"] {
        node = node.with_data_output(
            PortId::new(output)?,
            DataPort::output(SchemaRef::new(
                SchemaId::new("milkdrift.artifact-reference")?,
                1,
            )?),
        )?;
    }
    Ok(node)
}

fn edge(identity: &str, source: &str, target: &str) -> EvidenceResult<Mutation> {
    Ok(Mutation::AddEdge {
        edge: Edge::new(
            EdgeId::new(identity)?,
            EdgeKind::Control,
            NodeId::new(source)?,
            PortId::new("next")?,
            NodeId::new(target)?,
            PortId::new("in")?,
        ),
    })
}

fn assert_capability(runner: &CliRunner, capability: &str, facts: &ModelFacts) -> EvidenceResult {
    let response = runner.success(&["capability", "show", capability])?;
    let generation = response["value"]
        .as_array()
        .and_then(|values| values.first())
        .ok_or("capability show omitted its generation")?;
    ensure(
        generation["capability_id"] == capability
            && generation["generation"] == facts.revision
            && generation["provider_profile"] == facts.profile_id,
        "capability show contradicted exact model profile generation",
    )?;
    let operation = generation["operation_contracts"]
        .as_array()
        .and_then(|values| {
            values
                .iter()
                .find(|value| value["operation"] == "model.generate")
        })
        .ok_or("capability show omitted model.generate contract")?;
    ensure(
        operation["side_effect"] == "unknown"
            && operation["idempotency"] == "unsupported"
            && operation["cancellation"] == "best_effort",
        "capability show reinterpreted the model external-effect contract",
    )
}

fn assert_success_attempt(
    runner: &CliRunner,
    envelope: &Value,
    capability: &str,
    facts: &ModelFacts,
    directory: &Path,
    omitted_artifact_id: &str,
) -> EvidenceResult<SuccessObservation> {
    let attempt = &envelope["value"];
    ensure(
        attempt["state"] == "terminal"
            && attempt["terminal"] == "succeeded"
            && attempt["uncertain"] == false
            && attempt["capability_id"] == capability
            && attempt["provider_profile"] == facts.profile_id,
        "clean model attempt did not expose one accepted non-uncertain terminal",
    )?;
    let contract = &attempt["operation_contract"];
    ensure(
        contract["operation"] == "model.generate"
            && contract["side_effect"] == "unknown"
            && contract["idempotency"] == "unsupported"
            && contract["cancellation"] == "best_effort"
            && attempt["idempotency_key_present"] == false,
        "attempt read contradicted its frozen model external-effect contract",
    )?;
    let provenance = &attempt["capability_provenance"];
    ensure(
        provenance["model_profile_revision"] == facts.revision
            && provenance["provider_protocol"] == facts.protocol
            && provenance["model_alias"] == facts.model_alias
            && provenance["endpoint_origin"] == facts.endpoint_origin
            && provenance["model_profile_digest"]
                .as_str()
                .is_some_and(|digest| digest.starts_with("b3_")),
        "attempt read omitted exact redacted model profile provenance",
    )?;
    ensure(
        attempt["context_access"] == "authorized",
        "controller could not inspect the frozen context manifest",
    )?;
    let entries = serde_json::to_string(&attempt["context"]["entries"])?;
    let omissions = serde_json::to_string(&attempt["context"]["omissions"])?;
    ensure(
        entries.contains("selected-evidence")
            && !entries.contains(omitted_artifact_id)
            && omissions.contains(omitted_artifact_id),
        "attempt context did not preserve selected and omitted source identities",
    )?;
    let progress = attempt["progress_observations"].as_u64().unwrap_or(0);
    if facts.streaming {
        ensure(
            progress > 0,
            "streaming profile produced no ordered fragments",
        )?;
    }
    let outputs = attempt["outputs"]
        .as_array()
        .ok_or("model attempt outputs are absent")?;
    let response = outputs
        .iter()
        .find(|output| output["name"] == "model_response")
        .ok_or("model attempt omitted its canonical response artifact")?;
    ensure(
        response["report_sequence"].as_u64().is_some()
            && response["publication_sequence"].as_u64().is_some(),
        "model output omitted attempt and publication linkage",
    )?;
    let artifact = &response["artifact"];
    let bytes = download_artifact(runner, artifact, &directory.join("model-response.json"))?;
    let document = ModelResponseDocument::from_json(&bytes)?;
    let body = document.body();
    let response_identity = body
        .provider_metadata()
        .values()
        .any(|value| value.value().get("id").is_some_and(|id| !id.is_null()));
    let metadata_supplied = outputs
        .iter()
        .any(|output| output["name"] == "provider_metadata");
    ensure(
        metadata_supplied != body.provider_metadata().is_empty(),
        "provider metadata artifact disagreed with the canonical model response",
    )?;
    let usage = &attempt["usage"];
    Ok(SuccessObservation {
        progress,
        usage_input: usage["input_units"].as_u64().is_some(),
        usage_output: usage["output_units"].as_u64().is_some(),
        finish_reason: body.finish_reason() != FinishReason::Unknown,
        response_identity,
        artifacts: outputs.len(),
    })
}

fn download_artifact(
    runner: &CliRunner,
    artifact: &Value,
    destination: &Path,
) -> EvidenceResult<Vec<u8>> {
    let identity = required_text(artifact, &["artifact_id"])?;
    let digest = required_text(artifact, &["digest"])?;
    let size = required_u64(artifact, &["size"])?;
    runner.success(&[
        "artifact",
        "get",
        &identity,
        "--output",
        path_text(destination)?,
    ])?;
    let bytes = fs::read(destination)?;
    ensure(
        u64::try_from(bytes.len())? == size && blake3::hash(&bytes).to_hex().as_str() == digest,
        "downloaded model artifact contradicted its digest or size",
    )?;
    Ok(bytes)
}

fn run_uncertainty_scenario(
    runner: &CliRunner,
    blueprint: &BlueprintRevision,
    document: &[u8],
    path: &Path,
) -> EvidenceResult<UncertaintyObservation> {
    runner.success_with_input(
        &[
            "--command-id",
            "local-model-uncertain-validate-1",
            "blueprint",
            "validate",
            "-",
        ],
        document,
    )?;
    runner.success(&[
        "--command-id",
        "local-model-uncertain-import-1",
        "blueprint",
        "import",
        path_text(path)?,
    ])?;
    runner.success(&[
        "--command-id",
        "local-model-uncertain-start-1",
        "run",
        "start",
        FAILURE_RUN,
        FAILURE_WORKFLOW,
        blueprint.id().as_str(),
    ])?;
    let uncertain = wait_for_run(runner, FAILURE_RUN, |run| {
        run["value"]["uncertainty_count"]
            .as_u64()
            .is_some_and(|count| count == 1)
    })?;
    let model = node(&uncertain, "model").ok_or("uncertain model execution is absent")?;
    ensure(
        model["attempt_count"] == 1,
        "uncertain model work was automatically retried",
    )?;
    let attempt_id = required_text(model, &["latest_attempt_id"])?;
    let attempt = runner.success(&["attempt", "inspect", FAILURE_RUN, &attempt_id])?;
    ensure(
        attempt["value"]["uncertain"] == true
            && attempt["value"]["operation_contract"]["side_effect"] == "unknown"
            && attempt["value"]["operation_contract"]["idempotency"] == "unsupported"
            && attempt["value"]["idempotency_key_present"] == false,
        "post-entry close did not retain the exact unsupported-idempotency obligation",
    )?;
    let sequence = required_u64(&uncertain, &["value", "sequence"])?;
    let retry = runner.run(
        &[
            "--yes",
            "--command-id",
            "local-model-unsafe-retry-1",
            "--expected-sequence",
            &sequence.to_string(),
            "--evidence",
            "recovery_observation=evidence-local-model-retry-1",
            "attempt",
            "resolve",
            FAILURE_RUN,
            &attempt_id,
            "local-model-retry-decision-1",
            "--action",
            "retry",
        ],
        None,
    )?;
    ensure(
        !retry.status.success()
            && retry.stdout.is_empty()
            && retry.stderr.contains("\"classification\":\"conflict\"")
            && retry.stderr.contains("manual retry"),
        "authorized retry bypassed unsupported provider idempotency",
    )?;
    let retain_state = runner.success(&["run", "show", FAILURE_RUN])?;
    let retain_sequence = required_u64(&retain_state, &["value", "sequence"])?;
    runner.success(&[
        "--command-id",
        "local-model-retain-1",
        "--expected-sequence",
        &retain_sequence.to_string(),
        "--evidence",
        "recovery_observation=evidence-local-model-retain-1",
        "attempt",
        "resolve",
        FAILURE_RUN,
        &attempt_id,
        "local-model-retain-decision-1",
        "--action",
        "retain",
    ])?;
    let retained = runner.success(&["attempt", "inspect", FAILURE_RUN, &attempt_id])?;
    ensure(
        retained["value"]["uncertain"] == true,
        "retain resolution hid the unresolved provider outcome",
    )?;
    Ok(UncertaintyObservation { attempt_id })
}

fn spawn_timeline_follow(runner: &CliRunner, run: &str) -> EvidenceResult<TimelineFollower> {
    let mut child = ProcessCommand::new(&runner.executable)
        .arg("--endpoint")
        .arg(&runner.endpoint)
        .arg("--token-file")
        .arg(&runner.token_file)
        .arg("--json")
        .args(["run", "timeline", run, "--limit", "100", "--follow"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or("timeline stdout pipe is absent")?;
    let (send, lines) = mpsc::sync_channel(64);
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if send.send(line?).is_err() {
                break;
            }
        }
        Ok(())
    });
    let initial = lines.recv_timeout(Duration::from_secs(10))?;
    ensure(
        initial.contains("run.timeline"),
        "timeline follower did not establish its bounded initial page",
    )?;
    Ok(TimelineFollower {
        child,
        lines,
        reader: Some(reader),
        observed: vec![initial],
    })
}

fn stop_follower(follower: &mut TimelineFollower) -> EvidenceResult<String> {
    if follower.child.try_wait()?.is_none() {
        follower.child.kill()?;
    }
    let _ = follower.child.wait()?;
    if let Some(reader) = follower.reader.take() {
        reader.join().map_err(|_| "timeline reader panicked")??;
    }
    follower.observed.extend(follower.lines.try_iter());
    Ok(follower.observed.join("\n"))
}

fn node<'a>(run: &'a Value, identity: &str) -> Option<&'a Value> {
    run["value"]["nodes"]
        .as_array()?
        .iter()
        .find(|node| node["node_id"] == identity)
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..header_end + 4])
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap_or(0);
            if bytes.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }
    String::from_utf8(bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn write_response(stream: &mut TcpStream, media_type: &str, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {media_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())
}

fn write_private(path: &Path, bytes: &[u8]) -> EvidenceResult<PathBuf> {
    fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(path.to_owned())
}

fn require_file(path: &Path, label: &str) -> EvidenceResult {
    ensure(
        path.is_file(),
        &format!("required {label} path is not a file"),
    )
}

fn required_text(value: &Value, path: &[&str]) -> EvidenceResult<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or("JSON field is absent")?;
    }
    current
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "JSON field is not text".into())
}

fn required_u64(value: &Value, path: &[&str]) -> EvidenceResult<u64> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or("JSON field is absent")?;
    }
    current
        .as_u64()
        .ok_or_else(|| "JSON field is not an unsigned integer".into())
}

fn path_text(path: &Path) -> EvidenceResult<&str> {
    path.to_str()
        .ok_or_else(|| "fixture path is not UTF-8".into())
}

fn ensure(condition: bool, message: &str) -> EvidenceResult {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}
