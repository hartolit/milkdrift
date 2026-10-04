//! Own application children while scenarios observe public configuration and control behavior.
//!
//! [`DaemonLaunch`] checks configuration through the product
//! binary and waits for authenticated readiness. [`CliRunner`]
//! executes a real CLI and checks its machine output. Keep restart intent explicit:
//! [`terminate`](crate::application::OwnedChild::terminate) kills and reaps;
//! [`shutdown`](crate::application::OwnedChild::shutdown) exercises graceful Ctrl-C shutdown only
//! on Unix. Neither is a filesystem power-loss test.

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

mod diagnostics;

// Ordinary CLI calls retain the existing ten-second watchdog. A wait/follow command's
// explicit CLI budget replaces the eight seconds of work, leaving two seconds to exit.
const CLI_WORK: Duration = Duration::from_secs(8);
const CLI_EXIT_GRACE: Duration = Duration::from_secs(2);

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
    ///
    /// # Errors
    /// Returns launch, credential, early-exit, or readiness-deadline failure. A failed startup
    /// also reports whether termination/reaping succeeded before returning the original failure.
    pub async fn start(&self, token: &str) -> EvidenceResult<(OwnedChild, ControlClient)> {
        let mut child = start_daemon(&self.executable, &self.config)?;
        let ready = async {
            let mut config = ClientConfig::new(self.endpoint.clone());
            config.safe_query_retries = 0;
            let client = ControlClient::new(config, BearerCredential::new(token)?)?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                ensure(
                    child.try_wait()?.is_none(),
                    "daemon exited before readiness",
                )?;
                // Bound each startup probe without shortening subsequent scenario requests.
                let remaining = deadline.saturating_duration_since(Instant::now());
                ensure(
                    !remaining.is_zero(),
                    "daemon readiness exceeded its deadline",
                )?;
                if tokio::time::timeout(remaining.min(Duration::from_secs(2)), client.readiness())
                    .await
                    .is_ok_and(|result| result.is_ok_and(|ready| ready.ready))
                {
                    return Ok(client);
                }
                ensure(
                    Instant::now() < deadline,
                    "daemon readiness exceeded its deadline",
                )?;
                tokio::time::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(25)),
                )
                .await;
            }
        }
        .await;
        match ready {
            Ok(client) => Ok((child, client)),
            Err(error) => child.finish(Err(error)),
        }
    }
}

/// Owns an immediate child and attempts termination/reaping when a scenario exits early.
///
/// This is harness cleanup, not process-tree containment. Use an explicit lifecycle method
/// when its success or failure is part of the evidence instead of relying on best-effort drop.
/// A failed explicit termination remains an error; Drop does not retry and erase that evidence.
pub struct OwnedChild {
    child: Child,
    cleanup_attempted: bool,
}

impl OwnedChild {
    /// Starts a child whose lifetime is bounded by this owner.
    pub fn spawn(command: &mut ProcessCommand) -> EvidenceResult<Self> {
        Ok(Self {
            child: command.spawn()?,
            cleanup_attempted: false,
        })
    }

    /// Observes exit without blocking.
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Process identity, used only for process observation and owned signal delivery.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Transfers a configured stdout pipe to a bounded evidence reader.
    pub fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.stdout.take()
    }

    /// Abrupt restart boundary; kill and reap within a hard deadline.
    ///
    /// # Errors
    /// Process observation, kill, or bounded reaping can fail. Such a result is not stop evidence;
    /// a later explicit call may retry, but Drop does not silently repeat this attempt.
    pub fn terminate(&mut self) -> EvidenceResult {
        self.cleanup_attempted = true;
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
        }
        self.wait_until(Instant::now() + Duration::from_secs(2))?;
        Ok(())
    }

    fn finish<T>(&mut self, outcome: EvidenceResult<T>) -> EvidenceResult<T> {
        match (outcome, self.terminate()) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(format!("{error}; cleanup=reaped").into()),
            (Ok(_), Err(cleanup)) => Err(format!("child cleanup unconfirmed: {cleanup}").into()),
            (Err(error), Err(cleanup)) => {
                Err(format!("{error}; child cleanup unconfirmed: {cleanup}").into())
            }
        }
    }

    fn wait_until(&mut self, deadline: Instant) -> EvidenceResult<ExitStatus> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            ensure(
                Instant::now() < deadline,
                "owned child did not exit before its deadline",
            )?;
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Uses the daemon's public Ctrl-C boundary and verifies a successful exit on Unix.
    ///
    /// Other platforms return an error: forced termination cannot stand in for this observation.
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
        if !self.cleanup_attempted
            && let Err(error) = self.terminate()
        {
            #[expect(
                clippy::print_stderr,
                reason = "Cancellation or unwind has no result channel; report bounded child cleanup uncertainty without treating it as stop evidence."
            )]
            {
                eprintln!("evidence child {} cleanup unconfirmed: {error}", self.id());
            }
        }
    }
}

/// Captures a command in temporary files so stdout/stderr pipes cannot block its exit.
///
/// The harness polls output sizes and a deadline, then bounds each captured read. These checks
/// are observed limits; they do not impose an OS disk quota between polls. The child owner
/// attempts cleanup on error, and callers decide which captured text is safe to publish.
///
/// # Errors
/// Refuses unrepresentable/exhausted deadlines and excessive input or captured output. File I/O,
/// process launch/observation, UTF-8 decoding, and cleanup failures propagate with cleanup evidence.
pub fn run_command(
    command: &mut ProcessCommand,
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> EvidenceResult<CliOutput> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("child command deadline overflow")?;
    run_command_until(command, stdin, deadline)
}

fn run_command_until(
    command: &mut ProcessCommand,
    stdin: Option<&[u8]>,
    deadline: Instant,
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
    ensure(
        Instant::now() < deadline,
        "child command deadline exhausted before spawn",
    )?;
    let started = Instant::now();
    let mut child = OwnedChild::spawn(command)?;
    let outcome = (|| -> EvidenceResult<CliOutput> {
        let status = loop {
            let overflow =
                stdout.metadata()?.len() > MAX_CAPTURE || stderr.metadata()?.len() > MAX_CAPTURE;
            if overflow || Instant::now() >= deadline {
                let cause = if overflow {
                    "child output exceeds evidence bound"
                } else {
                    "child command exceeded its deadline"
                };
                let elapsed = started.elapsed();
                let observed_stdout_bytes = stdout.metadata().ok().map(|metadata| metadata.len());
                let captured = (|| -> std::io::Result<String> {
                    stdout.rewind()?;
                    let mut bytes = Vec::new();
                    std::io::Read::by_ref(&mut stdout)
                        .take(64 * 1024)
                        .read_to_end(&mut bytes)?;
                    Ok(diagnostics::captured_summary(
                        &String::from_utf8_lossy(&bytes),
                        usize::try_from(stderr.metadata()?.len()).unwrap_or(usize::MAX),
                    ))
                })()
                .unwrap_or_else(|_| "capture summary unavailable".to_owned());
                return Err(format!(
                "{cause}; elapsed_ms={}; pid={}; stdout_observed_bytes={observed_stdout_bytes:?}; capture_prefix_limit_bytes=65536; {captured}",
                elapsed.as_millis(),
                child.id()
            )
            .into());
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(10)),
            );
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
            elapsed: started.elapsed(),
            stdout: read(&mut stdout)?,
            stderr: read(&mut stderr)?,
        })
    })();
    child.finish(outcome)
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
    // Time spent waiting for this child, excluding failure diagnostics.
    elapsed: Duration,
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
        let result = self.run(arguments, (!stdin.is_empty()).then_some(stdin));
        self.success_result(arguments, result)
    }

    /// Requires JSON success without letting a nested probe restart an enclosing wait budget.
    pub fn success_until(&self, arguments: &[&str], deadline: Instant) -> EvidenceResult<Value> {
        let result = self.run_until(arguments, None, deadline);
        self.success_result(arguments, result)
    }

    fn success_result(
        &self,
        arguments: &[&str],
        result: EvidenceResult<CliOutput>,
    ) -> EvidenceResult<Value> {
        let output = result?;
        if !output.status.success() {
            return Err(format!(
                "CLI {} failed with exit {:?}; elapsed_ms={}; {}",
                diagnostics::command_identity(arguments),
                output.status.code(),
                output.elapsed.as_millis(),
                diagnostics::output_summary(&output)
            )
            .into());
        }
        one_json_line(&output.stdout)
    }

    /// Runs through the configured credential with a hard deadline.
    pub fn run(&self, arguments: &[&str], stdin: Option<&[u8]>) -> EvidenceResult<CliOutput> {
        self.run_with_token(&self.token_file, arguments, stdin)
    }

    /// Runs a probe under both its command allowance and the remaining enclosing budget.
    pub fn run_until(
        &self,
        arguments: &[&str],
        stdin: Option<&[u8]>,
        deadline: Instant,
    ) -> EvidenceResult<CliOutput> {
        self.run_bounded(&self.token_file, arguments, stdin, Some(deadline))
    }

    /// Uses an explicit credential file for authentication-refusal evidence.
    pub fn run_with_token(
        &self,
        token_file: &Path,
        arguments: &[&str],
        stdin: Option<&[u8]>,
    ) -> EvidenceResult<CliOutput> {
        self.run_bounded(token_file, arguments, stdin, None)
    }

    fn run_bounded(
        &self,
        token_file: &Path,
        arguments: &[&str],
        stdin: Option<&[u8]>,
        enclosing: Option<Instant>,
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
            .arg("--json");
        let explicit = arguments.iter().enumerate().find_map(|(index, value)| {
            if *value == "--timeout-secs" {
                arguments.get(index + 1).copied()
            } else {
                value.strip_prefix("--timeout-secs=")
            }
        });
        let work = match explicit {
            Some(value) => Duration::from_secs(value.parse()?),
            None => {
                command
                    .arg("--timeout-secs")
                    .arg(CLI_WORK.as_secs().to_string());
                CLI_WORK
            }
        };
        let watchdog = work
            .checked_add(CLI_EXIT_GRACE)
            .ok_or("CLI budget overflow")?;
        let deadline = Instant::now()
            .checked_add(watchdog)
            .ok_or("CLI deadline overflow")?;
        command.args(arguments);
        if arguments.windows(2).any(|pair| pair == ["run", "start"])
            && !arguments.contains(&"--request-file")
        {
            // Finite scenarios own these explicit recovery files with their credential directory.
            // Each submission gets a destination, including deliberate canonical replay/conflict
            // probes. Recovery scenarios pass their own retained path instead.
            static REQUEST_FILE: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);
            let index = REQUEST_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let directory = self
                .token_file
                .parent()
                .ok_or("credential directory is absent")?;
            command
                .arg("--request-file")
                .arg(directory.join(format!("run-request-{}-{index}.json", std::process::id())));
        }
        run_command_until(
            &mut command,
            stdin,
            enclosing.map_or(deadline, |outer| outer.min(deadline)),
        )
        .map_err(|error| {
            format!(
                "CLI {}: {error}; work_ms={}; exit_grace_ms={}",
                diagnostics::command_identity(arguments),
                work.as_millis(),
                CLI_EXIT_GRACE.as_millis()
            )
            .into()
        })
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
            diagnostics::output_summary(output)
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
        let output = runner.run_until(&["daemon", "readiness"], None, deadline)?;
        if output.status.success() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("daemon did not become ready before its deadline".into());
        }
        thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(25)),
        );
    }
}

/// Polls actual CLI run state until an independent scenario predicate holds.
///
/// # Errors
/// Refuses an unrepresentable deadline or a failed/malformed CLI observation. When the predicate
/// remains false until expiry, returns the last bounded observation and finite failure diagnostics.
pub fn wait_for_run<F>(
    runner: &CliRunner,
    run: &str,
    timeout: Duration,
    predicate: F,
) -> EvidenceResult<Value>
where
    F: Fn(&Value) -> bool,
{
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("run observation deadline overflow")?;
    loop {
        let output = match runner.run_until(&["run", "show", run], None, deadline) {
            Ok(output) => output,
            Err(error) => {
                return Err(format!("{error}; {}", runner.run_diagnostics(run, None)).into());
            }
        };
        if output.status.success() {
            let value = one_json_line(&output.stdout)?;
            if predicate(&value) {
                return Ok(value);
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "run did not reach its bounded expected state within {} ms; last={}; {}",
                timeout.as_millis(),
                diagnostics::output_summary(&output),
                runner.run_diagnostics(run, None)
            )
            .into());
        }
        thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(25)),
        );
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
        role: milkdrift_control_protocol::HostRole::WorkflowEnabled,
        host_id: "host:local".to_owned(),
        serving: Default::default(),
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
            managed_linux: None,
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
    let mut file = fs::File::open(path).map_err(|error| {
        format!(
            "evidence executable could not be opened for byte pinning: {:?}",
            error.kind()
        )
    })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrepresentable_command_deadline_is_refused_before_spawn() -> EvidenceResult {
        let error = run_command(
            &mut ProcessCommand::new("must-not-start-for-an-invalid-deadline"),
            None,
            Duration::MAX,
        )
        .err()
        .ok_or("unrepresentable deadline accepted")?;
        assert_eq!(error.to_string(), "child command deadline overflow");
        Ok(())
    }

    #[test]
    fn unrepresentable_observation_deadline_is_refused_before_cli_entry() -> EvidenceResult {
        let runner = CliRunner {
            executable: PathBuf::from("must-not-start-for-an-invalid-deadline"),
            endpoint: "http://127.0.0.1:9/".into(),
            token_file: PathBuf::from("unused-credential"),
            forbidden_storage_path: PathBuf::from("unused-store"),
        };
        let error = wait_for_run(&runner, "unused-run", Duration::MAX, |_| true)
            .err()
            .ok_or("unrepresentable observation deadline accepted")?;
        assert_eq!(error.to_string(), "run observation deadline overflow");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn failed_operation_retains_its_error_and_reaps_the_owned_child() -> EvidenceResult {
        let mut child = OwnedChild::spawn(ProcessCommand::new("/bin/sleep").arg("30"))?;
        assert!(child.try_wait()?.is_none());
        let error = child
            .finish::<()>(Err("primary fixture refusal".into()))
            .err()
            .ok_or("primary failure was discarded")?;
        assert_eq!(error.to_string(), "primary fixture refusal; cleanup=reaped");
        assert!(child.try_wait()?.is_some());
        child.terminate()?;
        Ok(())
    }
}
