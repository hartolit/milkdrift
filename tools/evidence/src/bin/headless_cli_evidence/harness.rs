//! Actual child-process, daemon-configuration, and JSON boundary harness.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    io::{Read as _, Write as _},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::{ACTOR, EvidenceResult, ensure};
use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, DaemonConfig, ModelProfileConfig, PeerHostConfig, RuntimeHostConfig,
    SecretSourceConfig, ShutdownConfig, ShutdownEffectPolicy,
};
use serde_json::{Value, json};

pub(super) struct CliRunner {
    pub(super) executable: PathBuf,
    pub(super) endpoint: String,
    pub(super) token_file: PathBuf,
    pub(super) forbidden_storage_path: PathBuf,
}

pub(super) struct CliOutput {
    pub(super) status: ExitStatus,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

pub(super) struct EvidenceConfig {
    pub(super) process_profiles: Vec<PathBuf>,
    pub(super) model_profiles: Vec<ModelProfileConfig>,
    pub(super) secret_sources: BTreeMap<String, SecretSourceConfig>,
    pub(super) lease_duration_ms: u64,
    pub(super) authority: ActorGrantConfig,
}

impl CliRunner {
    pub(super) fn success(&self, arguments: &[&str]) -> EvidenceResult<Value> {
        self.success_with_input(arguments, &[])
    }

    pub(super) fn success_with_input(
        &self,
        arguments: &[&str],
        stdin: &[u8],
    ) -> EvidenceResult<Value> {
        let output = self.run(arguments, (!stdin.is_empty()).then_some(stdin))?;
        if !output.status.success() {
            return Err(format!(
                "CLI {:?} failed with {:?}: stdout={} stderr={}",
                arguments,
                output.status.code(),
                output.stdout,
                output.stderr
            )
            .into());
        }
        one_json_line(&output.stdout)
    }

    pub(super) fn run(
        &self,
        arguments: &[&str],
        stdin: Option<&[u8]>,
    ) -> EvidenceResult<CliOutput> {
        self.run_with_token(&self.token_file, arguments, stdin)
    }

    pub(super) fn run_with_token(
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
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        } else {
            command.stdin(Stdio::null());
        }
        let mut child = command.spawn()?;
        if let Some(bytes) = stdin {
            child
                .stdin
                .take()
                .ok_or("CLI stdin pipe is absent")?
                .write_all(bytes)?;
        }
        let output = child.wait_with_output()?;
        Ok(CliOutput {
            status: output.status,
            stdout: String::from_utf8(output.stdout)?,
            stderr: String::from_utf8(output.stderr)?,
        })
    }
}

pub(super) fn assert_error(
    output: &CliOutput,
    exit: i32,
    classification: &str,
    daemon_code: Option<&str>,
) -> EvidenceResult {
    ensure(
        output.status.code() == Some(exit),
        "CLI exit code was not stable",
    )?;
    ensure(
        output.stdout.is_empty(),
        "failed CLI command emitted success output",
    )?;
    let document = one_json_line(&output.stderr)?;
    ensure(
        document["type"] == "error",
        "failure was not a typed JSON error",
    )?;
    ensure(
        document["value"]["classification"] == classification,
        "failure classification changed",
    )?;
    match daemon_code {
        Some(code) => ensure(
            document["value"]["daemon_code"] == code,
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

pub(super) fn wait_for_readiness(runner: &CliRunner, daemon: &mut Child) -> EvidenceResult {
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
            return Err(format!("daemon did not become ready: {}", output.stderr).into());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

pub(super) fn wait_for_run<F>(runner: &CliRunner, run: &str, predicate: F) -> EvidenceResult<Value>
where
    F: Fn(&Value) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(10);
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
                "run {run} did not reach its bounded expected state: stdout={} stderr={}",
                output.stdout, output.stderr
            )
            .into());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

pub(super) fn wait_for_failed_exit(runner: &CliRunner, run: &str) -> EvidenceResult {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let output = runner.run(&["run", "show", run], None)?;
        if output.status.code() == Some(8) {
            return assert_error(&output, 8, "failed_terminal", None);
        }
        if Instant::now() >= deadline {
            return Err("failed run did not reach the stable failed-terminal exit".into());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

pub(super) fn start_daemon(executable: &Path, config: &Path) -> EvidenceResult<Child> {
    ProcessCommand::new(executable)
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(Into::into)
}

pub(super) fn stop_daemon(child: &mut Child) -> EvidenceResult {
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    let _ = child.wait()?;
    Ok(())
}

pub(super) fn reserve_endpoint() -> EvidenceResult<SocketAddr> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    drop(listener);
    Ok(address)
}

pub(super) fn write_config(
    directory: &Path,
    bind: SocketAddr,
    token_file: &Path,
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
            actor: ACTOR.to_owned(),
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
    fs::write(&path, toml::to_string_pretty(&config)?)?;
    Ok(path)
}

pub(super) fn write_process_profile(
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

fn hash_file(path: &Path) -> EvidenceResult<(String, u64)> {
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
