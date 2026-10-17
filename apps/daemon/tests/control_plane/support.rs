//! Fixtures for ordinary tasks and retained independent serving boundaries.
pub(super) use crate::process::configured_process_profile;
pub(super) use milkdrift_authority::{
    ArtifactAuthorityScope, AuthorityBudget, BoundaryTimeMillis, CapabilityAuthorityScope,
    DaemonAuthorityScope, NetworkScope, PeerAuthorityScope, ResourceScope, WorkflowRunScope,
    WorkspaceAuthorityScope,
};
pub(super) use milkdrift_blueprint::WorkflowId;
pub(super) use milkdrift_capability::{CapabilityId, OperationId};
pub(super) use milkdrift_control_client::{
    BearerCredential, ClientConfig, ClientError, ControlClient,
};
pub(super) use milkdrift_control_protocol::tasks::{
    ChangeWorkflowRequest, CreateWorkflowRequest, InputBinding, InputValue, NewTaskDto,
    TaskCapability, TaskChangeDto, TaskDefinitionDto, TaskReadDto, TaskStateDto, TaskUploadRequest,
};
pub(super) use milkdrift_control_protocol::{ErrorCode, PageRequest, ProtocolVersion, decode_json};
pub(super) use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, AuthorityPresetConfig, DaemonConfig,
    DaemonHost, DaemonPlan, PeerHostConfig, RuntimeHostConfig, SecretSourceConfig, ShutdownConfig,
    serve,
};
pub(super) use milkdrift_persistence::{ArtifactStore, WorkflowStore};
pub(super) use milkdrift_redb_store::RedbStore;
pub(super) use milkdrift_workspace::{ArtifactId, CausalReference};
pub(super) use std::{
    collections::BTreeMap,
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
pub(super) use tempfile::TempDir;
pub(super) use tokio::{sync::oneshot, task::JoinHandle};
pub(super) use url::Url;

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(super) const CONTROLLER_TOKEN: &str = "controller-integration-token";
pub(super) const OBSERVER_TOKEN: &str = "observer-integration-token";

pub(super) struct RunningDaemon {
    pub(super) endpoint: Url,
    pub(super) client: ControlClient,
    pub(super) stop: oneshot::Sender<()>,
    pub(super) task: JoinHandle<Result<(), milkdrift_daemon::HostError>>,
}

pub(super) fn cli(
    daemon: &RunningDaemon,
    directory: &TempDir,
    id: &str,
    args: &[&str],
    json_output: bool,
) -> TestResult<(bool, String)> {
    let executable = std::env::current_exe()?
        .parent()
        .and_then(|path| path.parent())
        .ok_or("binary directory")?
        .join(format!("milkdrift{}", std::env::consts::EXE_SUFFIX));
    let mut command = std::process::Command::new(executable);
    command
        .arg("--endpoint")
        .arg(daemon.endpoint.as_str())
        .arg("--token-file")
        .arg(directory.path().join("controller.token"))
        .args(["--timeout-secs", "20", "--yes", "--command-id", id])
        .args(args)
        .current_dir(directory.path())
        .stdin(std::process::Stdio::null());
    if json_output {
        command.arg("--json");
    }
    let output = command.output()?;
    Ok((
        output.status.success(),
        String::from_utf8(output.stdout)? + &String::from_utf8_lossy(&output.stderr),
    ))
}

pub(super) fn cli_ok(
    daemon: &RunningDaemon,
    directory: &TempDir,
    id: &str,
    args: &[&str],
) -> TestResult<serde_json::Value> {
    let (ok, text) = cli(daemon, directory, id, args, true)?;
    assert!(ok, "{args:?}: {text}");
    let value: serde_json::Value =
        serde_json::from_str(text.lines().last().ok_or("missing CLI output")?)?;
    Ok(value
        .pointer("/value")
        .ok_or("fixture field /value absent")?
        .clone())
}

impl RunningDaemon {
    pub(super) async fn stop(self) -> TestResult {
        let Self { stop, task, .. } = self;
        let signal = stop.send(());
        // Allow the longest configured ten-second shutdown to settle its durable result.
        let mut task = task;
        match tokio::time::timeout(Duration::from_secs(15), &mut task).await {
            Ok(result) => result??,
            Err(_) => {
                task.abort();
                let joined = task.await;
                return Err(format!("daemon shutdown deadline; task joined: {joined:?}").into());
            }
        }
        signal.map_err(|()| "daemon shutdown receiver disappeared")?;
        Ok(())
    }
}

pub(super) async fn start(config: DaemonPlan, token: &str) -> TestResult<RunningDaemon> {
    let host = DaemonHost::start(config)?;
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    let endpoint = Url::parse(&format!("http://{address}/"))?;
    let client = client(&endpoint, token)?;
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(serve(listener, host, async move {
        // Sender loss also requests shutdown when a test exits early.
        if stopped.await.is_err() {
            tracing::debug!("fixture caller exited; shutting down daemon");
        }
    }));
    let daemon = RunningDaemon {
        endpoint,
        client,
        stop,
        task,
    };
    let client = &daemon.client;
    let mut health = client.readiness().await;
    for _ in 0..20 {
        if !matches!(&health, Err(ClientError::Api(error)) if error.code == ErrorCode::Overload) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        health = client.readiness().await;
    }
    match health {
        Ok(health) if health.ready => Ok(daemon),
        result => {
            let cleanup = daemon.stop().await;
            Err(format!("daemon did not become ready: {result:?}; cleanup: {cleanup:?}").into())
        }
    }
}

pub(super) fn client(endpoint: &Url, token: &str) -> Result<ControlClient, ClientError> {
    let mut config = ClientConfig::new(endpoint.clone());
    config.safe_query_retries = 0;
    config.retry_delay = Duration::from_millis(10);
    ControlClient::new(config, BearerCredential::new(token)?)
}

pub(super) fn configuration(directory: &TempDir, request_queue: u32) -> TestResult<DaemonPlan> {
    configuration_with_process_profiles(directory, request_queue, Vec::new())
}

pub(super) fn configuration_with_process_profiles(
    directory: &TempDir,
    request_queue: u32,
    process_profiles: Vec<std::path::PathBuf>,
) -> TestResult<DaemonPlan> {
    configuration_document_with_process_profiles(directory, request_queue, process_profiles)?
        .validate(directory.path())
        .map_err(Into::into)
}

pub(super) fn configuration_document_with_process_profiles(
    directory: &TempDir,
    request_queue: u32,
    process_profiles: Vec<std::path::PathBuf>,
) -> TestResult<DaemonConfig> {
    write_secret(&directory.path().join("controller.token"), CONTROLLER_TOKEN)?;
    write_secret(&directory.path().join("observer.token"), OBSERVER_TOKEN)?;
    let runtime = RuntimeHostConfig {
        request_queue,
        maintenance_interval_ms: 10,
        ..RuntimeHostConfig::default()
    };
    Ok(DaemonConfig {
        schema_version: milkdrift_daemon::DAEMON_CONFIG_SCHEMA_VERSION,
        role: milkdrift_control_protocol::HostRole::WorkflowEnabled,
        host_id: "host:local".to_owned(),
        name: "Milkdrift".to_owned(),
        serving: Default::default(),
        data_root: directory.path().join("data"),
        bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        secret_sources: BTreeMap::from([
            (
                "credential:controller".to_owned(),
                SecretSourceConfig::File {
                    path: directory.path().join("controller.token"),
                },
            ),
            (
                "credential:observer".to_owned(),
                SecretSourceConfig::File {
                    path: directory.path().join("observer.token"),
                },
            ),
        ]),
        actors: vec![
            ActorBindingConfig {
                credential_ref: "credential:controller".to_owned(),
                actor: "human:integration-controller".to_owned(),
                grant_id: "grant:integration-controller".to_owned(),
                grant_revision: 2,
                revocation_generation: 0,
                preset: AuthorityPresetConfig::Controller,
                authority: ActorGrantConfig::dangerous_administrator(),
                enabled: true,
            },
            ActorBindingConfig {
                credential_ref: "credential:observer".to_owned(),
                actor: "human:integration-observer".to_owned(),
                grant_id: "grant:integration-observer".to_owned(),
                grant_revision: 1,
                revocation_generation: 0,
                preset: AuthorityPresetConfig::Observer,
                authority: observer_authority()?,
                enabled: true,
            },
        ],
        runtime,
        adapters: AdapterConfig {
            managed_linux: None,
            process_profiles,
            ..AdapterConfig::default()
        },
        peers: PeerHostConfig::default(),
        shutdown: ShutdownConfig::default(),
        security_audit_record_bound: 1_000,
    })
}

pub(super) fn observer_authority() -> TestResult<ActorGrantConfig> {
    Ok(ActorGrantConfig {
        resources: ResourceScope {
            workflow_run: WorkflowRunScope::Workflow {
                workflow: WorkflowId::new("golden")?,
            },
            capability: CapabilityAuthorityScope::deny_all(),
            filesystem: Vec::new(),
            network: NetworkScope::empty(),
            secrets: std::collections::BTreeSet::new(),
            artifacts: ArtifactAuthorityScope::none(),
            peers: PeerAuthorityScope::none(),
            daemon: DaemonAuthorityScope {
                readiness: true,
                detailed_health: false,
                own_authority: true,
                configuration: false,
                audit: false,
            },
            workspace: WorkspaceAuthorityScope::none(),
        },
        budget: AuthorityBudget {
            cost_minor: Some(1_000),
            duration_ms: Some(300_000),
            invocations: Some(1_000),
            artifact_bytes: Some(67_108_864),
            units: Some(1_000_000),
            concurrency: Some(4),
        },
        valid_from: BoundaryTimeMillis::new(0),
        valid_until: BoundaryTimeMillis::new(4_102_444_800_000),
        dangerous_allow_broad_authority: false,
    })
}

pub(super) fn write_secret(path: &std::path::Path, value: &str) -> TestResult {
    fs::write(path, value)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub(super) fn task_definition(capability: &str, operation: &str) -> TaskDefinitionDto {
    TaskDefinitionDto {
        actions: None,
        tools: Vec::new(),
        title: "Inspect supplied project".into(),
        description: "A task submitted through the ordinary public client".into(),
        instructions: "Use only the supplied brief and captured file to report the project facts."
            .into(),
        capability: TaskCapability {
            id: capability.into(),
            operation: operation.into(),
        },
        inputs: BTreeMap::from([(
            "brief".into(),
            InputBinding::Supplied {
                name: "brief".into(),
                optional: false,
            },
        )]),
        dependencies: vec![],
        outputs: BTreeMap::new(),
        required: true,
        corrective_of: None,
        paths: vec![],
    }
}

pub(super) fn captured_file_profile(directory: &TempDir) -> TestResult<std::path::PathBuf> {
    let path = configured_process_profile(directory)?;
    let mut profile: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    let body = profile
        .get_mut("profile")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("profile missing")?;
    body.insert(
        "arguments".into(),
        serde_json::json!(["copy", "source.txt", "result.txt"]),
    );
    body.insert(
        "inputs".into(),
        serde_json::json!([{"input":"file","relative_path":"source.txt"}]),
    );
    body.insert("outputs".into(), serde_json::json!([{"name":"report","relative_path":"result.txt","media_type":"text/plain","required":true}]));
    fs::write(&path, serde_json::to_vec(&profile)?)?;
    Ok(path)
}

pub(super) fn create_request(id: &str, supplied: bool) -> CreateWorkflowRequest {
    CreateWorkflowRequest {
        request_id: id.into(),
        title: format!("Project analysis {id}"),
        description: "Retained integration work".into(),
        tasks: vec![NewTaskDto {
            alias: "inspect".into(),
            definition: task_definition("golden-local-process", "process.execute"),
        }],
        inputs: if supplied {
            BTreeMap::from([(
                "brief".into(),
                InputValue::Text {
                    text: "Captured project brief".into(),
                },
            )])
        } else {
            BTreeMap::new()
        },
        finish_when_settled: true,
        limits: None,
    }
}

pub(super) async fn wait_task(
    client: &ControlClient,
    workflow: &str,
    task: &str,
    expected: TaskStateDto,
) -> TestResult<TaskReadDto> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let value = client.task(workflow, task).await?;
        if value.state == expected && (expected != TaskStateDto::Queued || value.reason.is_some()) {
            return Ok(value);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("task did not reach {expected:?}: {value:?}").into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
