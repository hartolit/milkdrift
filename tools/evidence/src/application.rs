//! Actual child-process, daemon-configuration, and JSON boundary harness.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    io::{Read as _, Seek as _, Write as _},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::EvidenceResult;
use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, DaemonConfig, ModelProfileConfig, PeerHostConfig, RuntimeHostConfig,
    SecretSourceConfig, ShutdownConfig, ShutdownEffectPolicy,
};
use serde_json::{Value, json};
use url::Url;

/// Immutable child launch inputs; configuration compilation stays with the daemon.
pub struct DaemonLaunch {
    executable: PathBuf,
    config: PathBuf,
    endpoint: Url,
}

impl DaemonLaunch {
    /// Writes the owner-defined configuration and checks it using the product binary.
    pub fn write(
        executable: PathBuf,
        directory: &Path,
        config: &DaemonConfig,
    ) -> EvidenceResult<Self> {
        let path = directory.join("daemon.toml");
        write_private(&path, toml::to_string_pretty(config)?.as_bytes())?;
        let output = run_command(
            ProcessCommand::new(&executable)
                .arg("--config")
                .arg(&path)
                .arg("--check-config"),
            None,
            Duration::from_secs(10),
        )?;
        ensure(
            output.status.success(),
            "daemon binary refused evidence configuration",
        )?;
        Ok(Self {
            executable,
            config: path,
            endpoint: Url::parse(&format!("http://{}/", config.bind))?,
        })
    }

    /// Starts the real daemon and waits for public authenticated readiness.
    pub async fn start(&self, token: &str) -> EvidenceResult<(OwnedChild, ControlClient)> {
        let mut child = start_daemon(&self.executable, &self.config)?;
        let mut config = ClientConfig::new(self.endpoint.clone());
        config.safe_query_retries = 0;
        config.request_timeout = Duration::from_secs(2);
        let client = ControlClient::new(config, BearerCredential::new(token)?)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            ensure(
                child.try_wait()?.is_none(),
                "daemon exited before readiness",
            )?;
            if client.readiness().await.is_ok_and(|ready| ready.ready) {
                return Ok((child, client));
            }
            ensure(
                Instant::now() < deadline,
                "daemon readiness exceeded its deadline",
            )?;
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

/// One owned child, including cleanup on every early return.
pub struct OwnedChild(Child);

impl OwnedChild {
    /// Starts a child whose lifetime is bounded by this owner.
    pub fn spawn(command: &mut ProcessCommand) -> EvidenceResult<Self> {
        Ok(Self(command.spawn()?))
    }

    /// Observes exit without blocking.
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.0.try_wait()
    }

    /// Process identity, used only for process observation and owned signal delivery.
    pub fn id(&self) -> u32 {
        self.0.id()
    }

    /// Transfers a configured stdout pipe to a bounded evidence reader.
    pub fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.0.stdout.take()
    }

    /// Abrupt restart boundary; kill and reap within a hard deadline.
    pub fn terminate(&mut self) -> EvidenceResult {
        if self.0.try_wait()?.is_none() {
            self.0.kill()?;
        }
        self.wait_until(Instant::now() + Duration::from_secs(2))?;
        Ok(())
    }

    fn wait_until(&mut self, deadline: Instant) -> EvidenceResult<ExitStatus> {
        loop {
            if let Some(status) = self.0.try_wait()? {
                return Ok(status);
            }
            ensure(
                Instant::now() < deadline,
                "owned child did not exit before its deadline",
            )?;
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Uses the daemon's public Ctrl-C boundary and verifies a successful exit.
    pub fn shutdown(&mut self) -> EvidenceResult {
        #[cfg(unix)]
        {
            let output = run_command(
                ProcessCommand::new("kill")
                    .arg("-INT")
                    .arg(self.id().to_string()),
                None,
                Duration::from_secs(2),
            )?;
            ensure(
                output.status.success(),
                "owned daemon signal delivery failed",
            )?;
            let status = self.wait_until(Instant::now() + Duration::from_secs(30))?;
            ensure(status.success(), "daemon did not shut down successfully")
        }
        #[cfg(not(unix))]
        {
            Err("graceful child signal delivery requires a Unix host; forced exit is not graceful proof".into())
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

/// Runs a command with bounded output files and a deadline, without pipe deadlocks.
pub fn run_command(
    command: &mut ProcessCommand,
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> EvidenceResult<CliOutput> {
    const MAX_CAPTURE: u64 = 8 * 1024 * 1024;
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    if let Some(bytes) = stdin {
        ensure(
            u64::try_from(bytes.len())? <= MAX_CAPTURE,
            "child input exceeds evidence bound",
        )?;
        let mut input = tempfile::tempfile()?;
        input.write_all(bytes)?;
        input.rewind()?;
        command.stdin(input);
    } else {
        command.stdin(Stdio::null());
    }
    command
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?);
    let mut child = OwnedChild::spawn(command)?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        ensure(
            stdout.metadata()?.len() <= MAX_CAPTURE && stderr.metadata()?.len() <= MAX_CAPTURE,
            "child output exceeds evidence bound",
        )?;
        if let Some(status) = child.try_wait()? {
            break status;
        }
        ensure(
            Instant::now() < deadline,
            "child command exceeded its deadline",
        )?;
        thread::sleep(Duration::from_millis(10));
    };
    let read = |file: &mut fs::File| -> EvidenceResult<String> {
        file.rewind()?;
        let mut bytes = Vec::new();
        file.take(MAX_CAPTURE + 1).read_to_end(&mut bytes)?;
        ensure(
            u64::try_from(bytes.len())? <= MAX_CAPTURE,
            "child output exceeds evidence bound",
        )?;
        Ok(String::from_utf8(bytes)?)
    };
    Ok(CliOutput {
        status,
        stdout: read(&mut stdout)?,
        stderr: read(&mut stderr)?,
    })
}

/// Locates a previously built sibling application, including Cargo's `deps` layout.
pub fn application_binary(name: &str) -> EvidenceResult<PathBuf> {
    let executable = std::env::current_exe()?;
    let mut directory = executable
        .parent()
        .ok_or("executable directory is absent")?;
    if directory.file_name() == Some(OsStr::new("deps")) {
        directory = directory
            .parent()
            .ok_or("Cargo profile directory is absent")?;
    }
    let path = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    ensure(
        path.is_file(),
        "required application is absent; build milkdrift-daemon and milkdrift first",
    )?;
    Ok(path)
}

/// An actual CLI invocation boundary; the database is never an argument.
pub struct CliRunner {
    /// Built CLI executable.
    pub executable: PathBuf,
    /// Loopback daemon endpoint.
    pub endpoint: String,
    /// Private credential reference.
    pub token_file: PathBuf,
    /// Storage path forbidden in CLI arguments.
    pub forbidden_storage_path: PathBuf,
}

/// Bounded captured child result.
pub struct CliOutput {
    /// Actual process exit status.
    pub status: ExitStatus,
    /// Bounded standard output.
    pub stdout: String,
    /// Bounded standard error; callers must not publish secret-bearing content.
    pub stderr: String,
}

/// Scenario inputs to the canonical daemon configuration document.
pub struct EvidenceConfig {
    /// Byte-pinned process profile paths.
    pub process_profiles: Vec<PathBuf>,
    /// Explicit endpoint registrations.
    pub model_profiles: Vec<ModelProfileConfig>,
    /// Secret references, never secret values.
    pub secret_sources: BTreeMap<String, SecretSourceConfig>,
    /// Scenario lease duration.
    pub lease_duration_ms: u64,
    /// Explicit scenario authority.
    pub authority: ActorGrantConfig,
}

impl CliRunner {
    /// Requires a successful one-document JSON response.
    pub fn success(&self, arguments: &[&str]) -> EvidenceResult<Value> {
        self.success_with_input(arguments, &[])
    }

    /// Supplies a bounded input document and requires a successful JSON response.
    pub fn success_with_input(&self, arguments: &[&str], stdin: &[u8]) -> EvidenceResult<Value> {
        let output = self.run(arguments, (!stdin.is_empty()).then_some(stdin))?;
        if !output.status.success() {
            return Err(format!(
                "CLI command failed with exit {:?}: {}",
                output.status.code(),
                output.stdout
            )
            .into());
        }
        one_json_line(&output.stdout)
    }

    /// Runs through the configured credential with a hard deadline.
    pub fn run(&self, arguments: &[&str], stdin: Option<&[u8]>) -> EvidenceResult<CliOutput> {
        self.run_with_token(&self.token_file, arguments, stdin)
    }

    /// Uses an explicit credential file for authentication-refusal evidence.
    pub fn run_with_token(
        &self,
        token_file: &Path,
        arguments: &[&str],
        stdin: Option<&[u8]>,
    ) -> EvidenceResult<CliOutput> {
        let forbidden = self.forbidden_storage_path.as_os_str();
        ensure(
            arguments
                .iter()
                .all(|argument| OsStr::new(argument) != forbidden),
            "CLI invocation received the database path",
        )?;
        let mut command = ProcessCommand::new(&self.executable);
        command
            .arg("--endpoint")
            .arg(&self.endpoint)
            .arg("--token-file")
            .arg(token_file)
            .arg("--json")
            .args(arguments);
        run_command(&mut command, stdin, Duration::from_secs(10))
    }
}

/// Verifies the stable JSON failure and process exit classifications.
pub fn assert_error(
    output: &CliOutput,
    exit: i32,
    classification: &str,
    daemon_code: Option<&str>,
) -> EvidenceResult {
    ensure(
        output.status.code() == Some(exit),
        &format!(
            "CLI exit code was not stable: expected {exit}, observed {:?}: {}",
            output.status.code(),
            output.stdout
        ),
    )?;
    ensure(
        output.stderr.is_empty(),
        "JSON failure leaked diagnostics to stderr",
    )?;
    let document = one_json_line(&output.stdout)?;
    ensure(
        document["status"] == "failure"
            && document["schema_version"] == 2
            && document["final"] == true,
        "failure was not a typed JSON error",
    )?;
    ensure(
        document["error"]["classification"] == classification,
        "failure classification changed",
    )?;
    match daemon_code {
        Some(code) => ensure(
            document["error"]["daemon_code"] == code,
            "daemon error code changed",
        ),
        None => Ok(()),
    }
}

fn one_json_line(text: &str) -> EvidenceResult<Value> {
    let mut lines = text.lines();
    let line = lines.next().ok_or("expected one JSON document")?;
    ensure(lines.next().is_none(), "expected exactly one JSON document")?;
    ensure(
        !line.contains('\u{1b}'),
        "JSON output contained an ANSI escape",
    )?;
    serde_json::from_str(line).map_err(Into::into)
}

/// Polls actual CLI readiness while checking child exit and a hard deadline.
pub fn wait_for_readiness(runner: &CliRunner, daemon: &mut OwnedChild) -> EvidenceResult {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = daemon.try_wait()? {
            return Err(format!("daemon exited before readiness: {status}").into());
        }
        let output = runner.run(&["daemon", "readiness"], None)?;
        if output.status.success() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("daemon did not become ready before its deadline".into());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

/// Polls actual CLI run state until an independent scenario predicate holds.
pub fn wait_for_run<F>(
    runner: &CliRunner,
    run: &str,
    timeout: Duration,
    predicate: F,
) -> EvidenceResult<Value>
where
    F: Fn(&Value) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let output = runner.run(&["run", "show", run], None)?;
        if output.status.success() {
            let value = one_json_line(&output.stdout)?;
            if predicate(&value) {
                return Ok(value);
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "run {run} did not reach its bounded expected state: {}",
                output.stdout
            )
            .into());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

/// Requires the CLI failed-terminal exit within a hard deadline.
pub fn wait_for_failed_exit(runner: &CliRunner, run: &str) -> EvidenceResult {
    let output = runner.run(&["--timeout-secs", "5", "run", "wait", run], None)?;
    assert_error(&output, 8, "failed_terminal", None)
}
/// Starts the product daemon with private diagnostics and owned cleanup.
pub fn start_daemon(executable: &Path, config: &Path) -> EvidenceResult<OwnedChild> {
    OwnedChild::spawn(
        ProcessCommand::new(executable)
            .arg("--config")
            .arg(config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
}

/// Chooses an ephemeral loopback endpoint; bind races fail at startup.
pub fn reserve_endpoint() -> EvidenceResult<SocketAddr> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    drop(listener);
    Ok(address)
}

/// Writes the canonical daemon document from explicit scenario inputs.
pub fn write_config(
    directory: &Path,
    bind: SocketAddr,
    token_file: &Path,
    actor: &str,
    inputs: EvidenceConfig,
) -> EvidenceResult<PathBuf> {
    let mut secret_sources = inputs.secret_sources;
    secret_sources.insert(
        "credential:headless-cli".to_owned(),
        SecretSourceConfig::File {
            path: token_file.to_owned(),
        },
    );
    let config = DaemonConfig {
        schema_version: milkdrift_daemon::DAEMON_CONFIG_SCHEMA_VERSION,
        data_root: directory.join("data"),
        bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), bind.port()),
        secret_sources,
        actors: vec![ActorBindingConfig {
            credential_ref: "credential:headless-cli".to_owned(),
            actor: actor.to_owned(),
            grant_id: "grant:headless-cli-evidence".to_owned(),
            grant_revision: 1,
            revocation_generation: 0,
            preset: AuthorityPresetConfig::Controller,
            authority: inputs.authority,
            enabled: true,
        }],
        runtime: RuntimeHostConfig {
            request_queue: 8,
            maintenance_interval_ms: 10,
            lease_duration_ms: inputs.lease_duration_ms,
            effect_threads: 1,
            effect_queue: 8,
            cancellation_queue: 8,
            ..RuntimeHostConfig::default()
        },
        adapters: AdapterConfig {
            process_profiles: inputs.process_profiles,
            model_profiles: inputs.model_profiles,
        },
        peers: PeerHostConfig::default(),
        shutdown: ShutdownConfig {
            deadline_ms: 1_000,
            effect_policy: ShutdownEffectPolicy::Retain,
        },
        application_receipts: ApplicationReceiptConfig {
            hot_receipt_bound: 1_000,
            archive_batch_size: 64,
        },
        security_audit_record_bound: 1_000,
    };
    let path = directory.join("daemon.toml");
    write_private(&path, toml::to_string_pretty(&config)?.as_bytes())
}

/// Builds the deterministic evidence process profile with exact executable bytes.
pub fn write_process_profile(
    directory: &Path,
    executable: &Path,
    profile_id: &str,
    capability: &str,
    fixture_argument: &str,
    side_effect: &str,
    stdout_artifact: Option<&str>,
) -> EvidenceResult<PathBuf> {
    let (content_digest, executable_size) = hash_file(executable)?;
    let executable_root = executable
        .parent()
        .ok_or("evidence executable has no parent")?;
    let profile = json!({
        "schema_version": 2,
        "profile": {
            "profile_id": profile_id,
            "revision": 1,
            "capability": capability,
            "descriptor_revision": 1,
            "provider_profile": null,
            "operation": "process.execute",
            "side_effect": side_effect,
            "idempotency": "unsupported",
            "cancellation": "best_effort",
            "trust_class": "trusted_host_process",
            "executable": executable,
            "implementation": {
                "content_digest": content_digest,
                "size_bytes": executable_size,
                "package_revision": "headless-cli-evidence-v1",
                "documentation_reference": "urn:milkdrift:headless-cli-evidence"
            },
            "arguments": [fixture_argument],
            "substitutions": {},
            "working_directory": {"type": "isolated_root"},
            "filesystem_roots": [
                {"path": executable_root, "access": "execute"},
                {"path": directory, "access": "read_write"}
            ],
            "inputs": [],
            "environment": {
                "allowed_non_secret": [],
                "secrets": {},
                "max_value_bytes": 4096
            },
            "stdin": {"type": "disabled"},
            "stdout": {
                "max_capture_bytes": 4096,
                "stream_progress": false,
                "max_progress_events": 0,
                "overflow_action": "terminate",
                "artifact_name": stdout_artifact
            },
            "stderr": {
                "max_capture_bytes": 4096,
                "stream_progress": false,
                "max_progress_events": 0,
                "overflow_action": "terminate",
                "artifact_name": null
            },
            "outputs": [],
            "limits": {
                "max_argv_entries": 8,
                "max_argv_bytes": 4096,
                "max_children_observed": 4,
                "max_files": 8,
                "max_file_bytes": 1048576,
                "max_total_materialized_bytes": 2097152,
                "max_path_bytes": 4096,
                "max_directory_depth": 16,
                "artifact_chunk_bytes": 65536,
                "max_output_files": 4,
                "max_total_output_bytes": 2097152,
                "wall_timeout_ms": 10000,
                "graceful_termination_ms": 100,
                "forced_termination_ms": 100,
                "heartbeat_interval_ms": 100
            },
            "restart": "retain_uncertain",
            "platform": milkdrift_local_process::PlatformSupport::current(),
            "max_concurrent": 1,
            "extensions": {}
        }
    });
    let path = directory.join(format!("{profile_id}.json"));
    fs::write(&path, serde_json::to_vec(&profile)?)?;
    Ok(path)
}

/// Hashes a regular executable and verifies its observed size.
pub fn hash_file(path: &Path) -> EvidenceResult<(String, u64)> {
    let mut file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    ensure(
        metadata.is_file(),
        "evidence executable is not a regular file",
    )?;
    let mut hasher = blake3::Hasher::new();
    let mut observed_size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        observed_size = observed_size.saturating_add(u64::try_from(read)?);
    }
    ensure(
        observed_size == metadata.len(),
        "evidence executable changed while it was hashed",
    )?;
    Ok((format!("b3_{}", hasher.finalize()), observed_size))
}

/// Creates a private evidence file.
pub fn write_private(path: &Path, bytes: &[u8]) -> EvidenceResult<PathBuf> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)?;
    Ok(path.to_owned())
}

/// Reads a required textual JSON field without defaulting missing evidence.
pub fn required_text(value: &Value, path: &[&str]) -> EvidenceResult<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or("JSON field is absent")?;
    }
    current
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "JSON field is not text".into())
}

/// Reads a required unsigned JSON field without defaulting missing evidence.
pub fn required_u64(value: &Value, path: &[&str]) -> EvidenceResult<u64> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or("JSON field is absent")?;
    }
    current
        .as_u64()
        .ok_or_else(|| "JSON field is not an unsigned integer".into())
}

/// Requires a UTF-8 path at the CLI argument boundary.
pub fn path_text(path: &Path) -> EvidenceResult<&str> {
    path.to_str()
        .ok_or_else(|| "fixture path is not UTF-8".into())
}

/// Rejects a failed independent evidence assertion.
pub fn ensure(condition: bool, message: &str) -> EvidenceResult {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}
