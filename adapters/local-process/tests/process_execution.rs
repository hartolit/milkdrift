//! Deterministic configuration, execution, output, cancellation, and tree-cleanup tests.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use milkdrift_authority::SecretRef;
use milkdrift_blueprint::NodeId;
use milkdrift_capability::{
    ArtifactReference, CancellationRequest, CapabilityObservation, InputReference, InvocationEvent,
    InvocationId, InvocationRequest, InvocationValueReference, ResolvedCapabilitySnapshot,
    TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterExecutionContext, AdapterReporter, CapabilityHost,
    CapabilitySelectionPolicy, HostConfig, InMemorySecretResolver, InputMaterialization,
    InvocationDataAccess, InvocationDataError, MaterializationLimits, MaterializedExecution,
};
use milkdrift_local_process::{LocalProcessAdapter, PlatformSupport, ProcessProfileDocument};
use milkdrift_persistence::{AttemptId, NodeExecutionId};
use milkdrift_runtime::TaskExecutor;
use milkdrift_workspace::RunId;
use serde_json::{Value, json};
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct ReporterState {
    events: Vec<InvocationEvent>,
}

#[derive(Default)]
struct TestReporter {
    state: Mutex<ReporterState>,
    changed: Condvar,
    heartbeats: AtomicUsize,
}

impl TestReporter {
    fn events(&self) -> TestResult<Vec<InvocationEvent>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| "reporter lock poisoned")?
            .events
            .clone())
    }

    fn wait_until_started(&self) -> TestResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = self.state.lock().map_err(|_| "reporter lock poisoned")?;
        while state.events.is_empty() && Instant::now() < deadline {
            let wait = self
                .changed
                .wait_timeout(state, Duration::from_millis(10))
                .map_err(|_| "reporter lock poisoned")?;
            state = wait.0;
        }
        if state.events.is_empty() {
            return Err("process did not report start".into());
        }
        Ok(())
    }
}

impl AdapterReporter for TestReporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AdapterError::external_failure("reporter lock poisoned"))?;
        state.events.push(event);
        self.changed.notify_all();
        Ok(())
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        self.heartbeats.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct TestWorkspace {
    _directory: TempDir,
    root: PathBuf,
    inputs: BTreeMap<String, PathBuf>,
}

impl MaterializedExecution for TestWorkspace {
    fn root(&self) -> &Path {
        &self.root
    }

    fn input_path(&self, input_name: &str) -> Option<&Path> {
        self.inputs.get(input_name).map(PathBuf::as_path)
    }
}

struct TestDataAccess {
    _root_owner: TempDir,
    root: PathBuf,
    outputs: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl TestDataAccess {
    fn new() -> TestResult<Self> {
        let root_owner = tempfile::tempdir()?;
        let root = root_owner.path().canonicalize()?;
        Ok(Self {
            _root_owner: root_owner,
            root,
            outputs: Mutex::new(BTreeMap::new()),
        })
    }

    fn output(&self, name: &str) -> TestResult<Option<Vec<u8>>> {
        Ok(self
            .outputs
            .lock()
            .map_err(|_| "output lock poisoned")?
            .get(name)
            .cloned())
    }

    fn publish(&self, name: &str, bytes: &[u8]) -> Result<ArtifactReference, InvocationDataError> {
        self.outputs
            .lock()
            .map_err(|_| InvocationDataError::Publication("output lock poisoned".to_owned()))?
            .insert(name.to_owned(), bytes.to_vec());
        ArtifactReference::new(
            format!("test-{name}"),
            blake3::hash(bytes).to_hex().to_string(),
            Some("application/octet-stream".to_owned()),
            Some(bytes.len() as u64),
        )
        .map_err(|error| InvocationDataError::Publication(error.to_string()))
    }
}

impl InvocationDataAccess for TestDataAccess {
    fn materialize(
        &self,
        _context: &AdapterExecutionContext,
        request: &InvocationRequest,
        inputs: &[InputMaterialization],
        _limits: MaterializationLimits,
    ) -> Result<Box<dyn MaterializedExecution>, InvocationDataError> {
        let directory = tempfile::tempdir_in(&self.root)
            .map_err(|error| InvocationDataError::Filesystem(error.to_string()))?;
        let root = directory
            .path()
            .canonicalize()
            .map_err(|error| InvocationDataError::Filesystem(error.to_string()))?;
        let mut paths = BTreeMap::new();
        for specification in inputs {
            let input = request
                .inputs()
                .iter()
                .find(|input| input.name() == specification.input_name())
                .ok_or_else(|| InvocationDataError::Rejected("missing input".to_owned()))?;
            let InvocationValueReference::Inline { value } = input.value() else {
                return Err(InvocationDataError::Rejected(
                    "test access supports inline inputs only".to_owned(),
                ));
            };
            let path = root.join(specification.relative_path());
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| InvocationDataError::Filesystem(error.to_string()))?;
            }
            fs::write(
                &path,
                serde_json::to_vec(value.value())
                    .map_err(|error| InvocationDataError::Integrity(error.to_string()))?,
            )
            .map_err(|error| InvocationDataError::Filesystem(error.to_string()))?;
            paths.insert(specification.input_name().to_owned(), path);
        }
        Ok(Box::new(TestWorkspace {
            _directory: directory,
            root,
            inputs: paths,
        }))
    }

    fn publish_file(
        &self,
        _context: &AdapterExecutionContext,
        _request: &InvocationRequest,
        workspace: &dyn MaterializedExecution,
        output_name: &str,
        relative_path: &Path,
        _media_type: &str,
        _limits: MaterializationLimits,
    ) -> Result<ArtifactReference, InvocationDataError> {
        let path = workspace.root().join(relative_path);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| InvocationDataError::Filesystem(error.to_string()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(InvocationDataError::Rejected(
                "output must be a regular non-symlink file".to_owned(),
            ));
        }
        let canonical = path
            .canonicalize()
            .map_err(|error| InvocationDataError::Filesystem(error.to_string()))?;
        if !canonical.starts_with(workspace.root()) {
            return Err(InvocationDataError::Rejected(
                "output escaped root".to_owned(),
            ));
        }
        let bytes = fs::read(canonical)
            .map_err(|error| InvocationDataError::Filesystem(error.to_string()))?;
        self.publish(output_name, &bytes)
    }

    fn publish_bytes(
        &self,
        _context: &AdapterExecutionContext,
        _request: &InvocationRequest,
        output_name: &str,
        _media_type: &str,
        bytes: &[u8],
        _limits: MaterializationLimits,
    ) -> Result<ArtifactReference, InvocationDataError> {
        self.publish(output_name, bytes)
    }
}

fn profile_value(root: &Path, arguments: Vec<Value>) -> Value {
    let executable = env!("CARGO_BIN_EXE_milkdrift-process-test-helper");
    let executable_root = Path::new(executable).parent().unwrap_or(Path::new("/"));
    json!({
        "schema_version": 1,
        "profile": {
            "profile_id": "fixture-profile",
            "revision": 1,
            "capability": "local-process-fixture",
            "descriptor_revision": 1,
            "provider_profile": null,
            "operation": "process.execute",
            "side_effect": "unknown",
            "idempotency": "unsupported",
            "cancellation": "best_effort",
            "executable": executable,
            "arguments": arguments,
            "substitutions": {},
            "working_directory": { "type": "isolated_root" },
            "filesystem_roots": [
                { "path": executable_root, "access": "execute" },
                { "path": root, "access": "read_write" }
            ],
            "inputs": [],
            "environment": {
                "allowed_non_secret": [],
                "secrets": {},
                "max_value_bytes": 4096
            },
            "stdin": { "type": "disabled" },
            "stdout": {
                "max_capture_bytes": 1048576,
                "stream_progress": true,
                "max_progress_events": 8,
                "overflow_action": "continue_truncated",
                "artifact_name": "stdout"
            },
            "stderr": {
                "max_capture_bytes": 1048576,
                "stream_progress": true,
                "max_progress_events": 8,
                "overflow_action": "continue_truncated",
                "artifact_name": "stderr"
            },
            "outputs": [],
            "limits": {
                "max_argv_entries": 64,
                "max_argv_bytes": 65536,
                "max_children_observed": 32,
                "max_files": 64,
                "max_file_bytes": 2097152,
                "max_total_materialized_bytes": 4194304,
                "max_path_bytes": 4096,
                "max_directory_depth": 32,
                "artifact_chunk_bytes": 65536,
                "max_output_files": 64,
                "max_total_output_bytes": 4194304,
                "wall_timeout_ms": 5000,
                "graceful_termination_ms": 100,
                "forced_termination_ms": 100,
                "heartbeat_interval_ms": 1000
            },
            "restart": "retain_uncertain",
            "platform": PlatformSupport::current(),
            "max_concurrent": 4,
            "extensions": {}
        }
    })
}

fn parse_profile(value: &Value) -> TestResult<milkdrift_local_process::ProcessProfile> {
    Ok(ProcessProfileDocument::from_json(&serde_json::to_vec(value)?)?.into_profile())
}

fn input(name: &str, value: Value) -> TestResult<InputReference> {
    Ok(InputReference::new(
        name,
        InvocationValueReference::Inline {
            value: milkdrift_capability::BoundedJson::new(value)?,
        },
    )?)
}

fn request(
    profile: &milkdrift_local_process::ProcessProfile,
    invocation: &str,
    inputs: Vec<InputReference>,
) -> TestResult<InvocationRequest> {
    Ok(InvocationRequest::new(
        InvocationId::new(invocation)?,
        profile.capability().clone(),
        profile.operation().clone(),
        None,
        None,
        inputs,
        BTreeMap::new(),
    )?)
}

fn context() -> TestResult<AdapterExecutionContext> {
    Ok(AdapterExecutionContext::new(
        RunId::new("run-process")?,
        serde_json::from_value(json!(format!("rev_{}", "0".repeat(64))))?,
        NodeId::new("node-process")?,
        NodeExecutionId::new("execution-process")?,
        AttemptId::new("attempt-process")?,
    ))
}

fn setup(
    profile: milkdrift_local_process::ProcessProfile,
    data: Arc<TestDataAccess>,
    secrets: Arc<InMemorySecretResolver>,
) -> TestResult<(CapabilityHost, ResolvedCapabilitySnapshot)> {
    let descriptor = profile.descriptor()?;
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(&descriptor, profile.operation())?;
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 4,
            max_generations_per_capability: 2,
            max_concurrent_per_generation: 4,
            observation_stale_after_ms: 10_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    let observation =
        CapabilityObservation::new(descriptor.identity().clone(), 1, true, 0, "fixture ready")?;
    host.register(
        descriptor,
        Arc::new(LocalProcessAdapter::new(profile, data, secrets)?),
        Some(observation),
    )?;
    Ok((host, snapshot))
}

fn terminal_status(events: &[InvocationEvent]) -> Option<TerminalStatus> {
    events.iter().find_map(|event| {
        event
            .kind()
            .terminal()
            .map(milkdrift_capability::InvocationTerminal::status)
    })
}

#[test]
fn argv_metacharacters_stdin_environment_and_outputs_are_bounded_and_literal() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let mut value = profile_value(
        &data.root,
        vec![
            json!("inspect"),
            json!("result.json"),
            json!("VISIBLE"),
            json!("{{literal}}"),
        ],
    );
    let profile = value["profile"].as_object_mut().ok_or("missing profile")?;
    profile.insert(
        "substitutions".to_owned(),
        json!({ "literal": { "type": "input_text", "input": "literal" } }),
    );
    profile.insert(
        "inputs".to_owned(),
        json!([{ "input": "prompt", "relative_path": "prompt.txt" }]),
    );
    profile.insert(
        "stdin".to_owned(),
        json!({ "type": "input", "input": "prompt", "max_bytes": 1024 }),
    );
    profile.insert(
        "environment".to_owned(),
        json!({
            "allowed_non_secret": [],
            "secrets": { "VISIBLE": "secret:fixture-visible" },
            "max_value_bytes": 4096
        }),
    );
    profile.insert(
        "stdout".to_owned(),
        json!({
            "max_capture_bytes": 1048576,
            "stream_progress": false,
            "max_progress_events": 0,
            "overflow_action": "continue_truncated",
            "artifact_name": "stdout"
        }),
    );
    profile.insert(
        "stderr".to_owned(),
        json!({
            "max_capture_bytes": 1048576,
            "stream_progress": false,
            "max_progress_events": 0,
            "overflow_action": "continue_truncated",
            "artifact_name": "stderr"
        }),
    );
    profile.insert(
        "outputs".to_owned(),
        json!([{
            "name": "result",
            "relative_path": "result.json",
            "media_type": "application/json",
            "required": true
        }]),
    );
    let profile = parse_profile(&value)?;
    let secrets = Arc::new(InMemorySecretResolver::new());
    let secret_ref = SecretRef::new("secret:fixture-visible")?;
    secrets.insert(secret_ref, b"top-secret".to_vec())?;
    let request = request(
        &profile,
        "invocation-inspect",
        vec![
            input("prompt", json!("hello from stdin"))?,
            input("literal", json!("; touch /tmp/never-created && $(false)"))?,
        ],
    )?;
    let (host, snapshot) = setup(profile, data.clone(), secrets)?;
    let reporter = TestReporter::default();
    host.execute_exact_with_context(&snapshot, &request, &context()?, &reporter)?;
    let events = reporter.events()?;
    assert_eq!(terminal_status(&events), Some(TerminalStatus::Success));
    let output = data.output("result")?.ok_or("missing result output")?;
    let result: Value = serde_json::from_slice(&output)?;
    assert_eq!(
        result["literal"],
        json!("; touch /tmp/never-created && $(false)")
    );
    assert_eq!(result["stdin"], json!("\"hello from stdin\""));
    assert_eq!(result["selected_environment"], json!("top-secret"));
    assert_eq!(result["ambient_home"], Value::Null);
    assert_eq!(result["ambient_path"], Value::Null);
    let serialized_events = serde_json::to_vec(&events)?;
    assert!(
        !serialized_events
            .windows(b"top-secret".len())
            .any(|value| value == b"top-secret")
    );
    Ok(())
}

#[test]
fn simultaneous_large_streams_do_not_deadlock_and_are_truncated() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let mut value = profile_value(&data.root, vec![json!("emit"), json!("2097152")]);
    for name in ["stdout", "stderr"] {
        let capture = value["profile"][name]
            .as_object_mut()
            .ok_or("missing capture")?;
        capture.insert("max_capture_bytes".to_owned(), json!(65536));
        capture.insert("max_progress_events".to_owned(), json!(2));
    }
    let profile = parse_profile(&value)?;
    let request = request(&profile, "invocation-large", Vec::new())?;
    let (host, snapshot) = setup(
        profile,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let reporter = TestReporter::default();
    host.execute_exact_with_context(&snapshot, &request, &context()?, &reporter)?;
    assert_eq!(
        terminal_status(&reporter.events()?),
        Some(TerminalStatus::Success)
    );
    assert_eq!(data.output("stdout")?.ok_or("missing stdout")?.len(), 65536);
    assert_eq!(data.output("stderr")?.ok_or("missing stderr")?.len(), 65536);
    Ok(())
}

#[test]
fn secret_echo_is_redacted_from_capture_artifacts_and_events() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let mut value = profile_value(&data.root, vec![json!("echo-env"), json!("TOKEN")]);
    value["profile"]["environment"] = json!({
        "allowed_non_secret": [],
        "secrets": { "TOKEN": "secret:echo-token" },
        "max_value_bytes": 4096
    });
    for name in ["stdout", "stderr"] {
        value["profile"][name]["stream_progress"] = json!(false);
        value["profile"][name]["max_progress_events"] = json!(0);
    }
    let profile = parse_profile(&value)?;
    let request = request(&profile, "invocation-secret-echo", Vec::new())?;
    let secrets = Arc::new(InMemorySecretResolver::new());
    secrets.insert(
        SecretRef::new("secret:echo-token")?,
        b"echoed-secret".to_vec(),
    )?;
    let (host, snapshot) = setup(profile, data.clone(), secrets)?;
    let reporter = TestReporter::default();
    host.execute_exact_with_context(&snapshot, &request, &context()?, &reporter)?;
    let stdout = data.output("stdout")?.ok_or("missing stdout capture")?;
    assert!(
        !stdout
            .windows(b"echoed-secret".len())
            .any(|part| part == b"echoed-secret")
    );
    assert!(
        stdout
            .windows(b"[redacted]".len())
            .any(|part| part == b"[redacted]")
    );
    assert!(
        !serde_json::to_vec(&reporter.events()?)?
            .windows(b"echoed-secret".len())
            .any(|part| part == b"echoed-secret")
    );
    Ok(())
}

#[test]
fn nonzero_exit_signal_and_timeout_are_typed_failures() -> TestResult {
    for (suffix, arguments, timeout) in [
        ("nonzero", vec![json!("exit"), json!("7")], 5000_u64),
        ("signal", vec![json!("signal")], 5000_u64),
        ("timeout", vec![json!("sleep"), json!("5000")], 50_u64),
    ] {
        let data = Arc::new(TestDataAccess::new()?);
        let mut value = profile_value(&data.root, arguments);
        value["profile"]["limits"]["wall_timeout_ms"] = json!(timeout);
        value["profile"]["limits"]["heartbeat_interval_ms"] = json!(25);
        let profile = parse_profile(&value)?;
        let request = request(&profile, &format!("invocation-{suffix}"), Vec::new())?;
        let (host, snapshot) = setup(profile, data, Arc::new(InMemorySecretResolver::new()))?;
        let reporter = TestReporter::default();
        host.execute_exact_with_context(&snapshot, &request, &context()?, &reporter)?;
        assert_eq!(
            terminal_status(&reporter.events()?),
            Some(TerminalStatus::Failure)
        );
    }
    Ok(())
}

#[test]
fn explicit_cancellation_observes_terminal_group_cleanup() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let profile = parse_profile(&profile_value(
        &data.root,
        vec![json!("sleep"), json!("10000")],
    ))?;
    let request = request(&profile, "invocation-cancel", Vec::new())?;
    let (host, snapshot) = setup(profile, data, Arc::new(InMemorySecretResolver::new()))?;
    let reporter = Arc::new(TestReporter::default());
    let thread_host = host.clone();
    let thread_reporter = reporter.clone();
    let thread_request = request.clone();
    let context = context()?;
    let handle = thread::spawn(move || {
        thread_host.execute_exact_with_context(
            &snapshot,
            &thread_request,
            &context,
            thread_reporter.as_ref(),
        )
    });
    reporter.wait_until_started()?;
    let acknowledgement = TaskExecutor::cancel(
        &host,
        &CancellationRequest::new(request.invocation().clone(), 1, "test cancellation")?,
    )?;
    assert!(acknowledgement.accepted());
    assert!(!acknowledgement.terminal_boundary());
    handle.join().map_err(|_| "execution thread panicked")??;
    assert_eq!(
        terminal_status(&reporter.events()?),
        Some(TerminalStatus::Cancelled)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn cancellation_terminates_child_and_grandchild_in_owned_group() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let pid_owner = tempfile::NamedTempFile::new()?;
    let pid_path = pid_owner.path().to_string_lossy().to_string();
    let profile = parse_profile(&profile_value(
        &data.root,
        vec![json!("tree"), json!(pid_path), json!("10000")],
    ))?;
    let request = request(&profile, "invocation-tree", Vec::new())?;
    let (host, snapshot) = setup(profile, data, Arc::new(InMemorySecretResolver::new()))?;
    let reporter = Arc::new(TestReporter::default());
    let thread_host = host.clone();
    let thread_reporter = reporter.clone();
    let thread_request = request.clone();
    let context = context()?;
    let handle = thread::spawn(move || {
        thread_host.execute_exact_with_context(
            &snapshot,
            &thread_request,
            &context,
            thread_reporter.as_ref(),
        )
    });
    reporter.wait_until_started()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let pids = fs::read_to_string(&pid_path)?;
        if pids.lines().count() >= 3 || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let pids = fs::read_to_string(&pid_path)?;
    assert!(pids.lines().count() >= 3);
    let _ = TaskExecutor::cancel(
        &host,
        &CancellationRequest::new(request.invocation().clone(), 1, "tree cancellation")?,
    )?;
    handle.join().map_err(|_| "execution thread panicked")??;
    for pid in pids.lines() {
        let pid: i32 = pid.parse()?;
        let pid = rustix::process::Pid::from_raw(pid).ok_or("invalid fixture pid")?;
        assert_eq!(
            rustix::process::test_kill_process(pid).err(),
            Some(rustix::io::Errno::SRCH)
        );
    }
    Ok(())
}

#[test]
fn configuration_rejects_unknown_placeholders_traversal_and_missing_secrets() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let mut unknown = profile_value(&data.root, vec![json!("{{unknown}}")]);
    assert!(parse_profile(&unknown).is_err());
    unknown["profile"]["arguments"] = json!(["inspect"]);
    unknown["profile"]["inputs"] = json!([{
        "input": "bad",
        "relative_path": "../escape"
    }]);
    assert!(parse_profile(&unknown).is_err());

    unknown["profile"]["inputs"] = json!([]);
    unknown["profile"]["working_directory"] = json!({
        "type": "isolated_subdirectory",
        "relative_path": "../escape"
    });
    assert!(parse_profile(&unknown).is_err());

    let mut denied_executable = profile_value(&data.root, vec![json!("exit"), json!("0")]);
    denied_executable["profile"]["filesystem_roots"][0]["path"] =
        json!(data.root.to_string_lossy());
    let denied_executable = parse_profile(&denied_executable)?;
    assert!(
        LocalProcessAdapter::new(
            denied_executable,
            data.clone(),
            Arc::new(InMemorySecretResolver::new()),
        )
        .is_err()
    );

    let denied_root_owner = tempfile::tempdir()?;
    let mut denied_root = profile_value(&data.root, vec![json!("exit"), json!("0")]);
    denied_root["profile"]["filesystem_roots"][1]["path"] =
        json!(denied_root_owner.path().canonicalize()?.to_string_lossy());
    let denied_root = parse_profile(&denied_root)?;
    let denied_request = request(&denied_root, "invocation-working-root-denied", Vec::new())?;
    let (denied_host, denied_snapshot) = setup(
        denied_root,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let denied_reporter = TestReporter::default();
    denied_host.execute_exact_with_context(
        &denied_snapshot,
        &denied_request,
        &context()?,
        &denied_reporter,
    )?;
    assert_eq!(
        terminal_status(&denied_reporter.events()?),
        Some(TerminalStatus::Rejected)
    );

    let mut missing = profile_value(
        &data.root,
        vec![
            json!("inspect"),
            json!("result.json"),
            json!("MISSING"),
            json!("literal"),
        ],
    );
    missing["profile"]["environment"] = json!({
        "allowed_non_secret": [],
        "secrets": { "MISSING": "secret:not-configured" },
        "max_value_bytes": 4096
    });
    for name in ["stdout", "stderr"] {
        missing["profile"][name]["stream_progress"] = json!(false);
        missing["profile"][name]["max_progress_events"] = json!(0);
    }
    let profile = parse_profile(&missing)?;
    let request = request(&profile, "invocation-secret-missing", Vec::new())?;
    let (host, snapshot) = setup(profile, data, Arc::new(InMemorySecretResolver::new()))?;
    let reporter = TestReporter::default();
    host.execute_exact_with_context(&snapshot, &request, &context()?, &reporter)?;
    assert_eq!(
        terminal_status(&reporter.events()?),
        Some(TerminalStatus::Rejected)
    );
    Ok(())
}

#[test]
fn undeclared_process_files_are_not_published() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let mut value = profile_value(
        &data.root,
        vec![
            json!("inspect"),
            json!("undeclared.json"),
            json!("UNSET"),
            json!("literal"),
        ],
    );
    for stream in ["stdout", "stderr"] {
        value["profile"][stream]["artifact_name"] = Value::Null;
    }
    let profile = parse_profile(&value)?;
    let request = request(&profile, "invocation-undeclared-output", Vec::new())?;
    let (host, snapshot) = setup(
        profile,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let reporter = TestReporter::default();
    host.execute_exact_with_context(&snapshot, &request, &context()?, &reporter)?;
    assert_eq!(
        terminal_status(&reporter.events()?),
        Some(TerminalStatus::Success)
    );
    assert!(data.output("undeclared")?.is_none());
    Ok(())
}

#[test]
fn profile_schema_canonical_round_trip_is_stable() -> TestResult {
    let data = TestDataAccess::new()?;
    let value = profile_value(&data.root, vec![json!("exit"), json!("0")]);
    let document = ProcessProfileDocument::from_json(&serde_json::to_vec(&value)?)?;
    let canonical = document.to_canonical_json()?;
    let reparsed = ProcessProfileDocument::from_json(&canonical)?;
    assert_eq!(document, reparsed);
    assert_eq!(document.schema_version(), 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn schema_v1_golden_profile_remains_readable_and_canonicalizable() -> TestResult {
    let document =
        ProcessProfileDocument::from_json(include_bytes!("fixtures/process-profile-v1.json"))?;
    assert_eq!(document.profile().profile_id(), "golden-local-process");
    assert_eq!(document.profile().descriptor_revision(), 1);
    let canonical = document.to_canonical_json()?;
    assert_eq!(ProcessProfileDocument::from_json(&canonical)?, document);
    Ok(())
}
