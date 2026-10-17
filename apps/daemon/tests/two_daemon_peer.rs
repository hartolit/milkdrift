//! Deterministic two-daemon authenticated catalog and local registration coverage.

#[path = "two_daemon_peer/placement.rs"]
mod placement;
#[path = "support/process.rs"]
mod process;
use process::configured_process_profile;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::Ipv4Addr,
    time::Duration,
};

use milkdrift_authority::{AccessMode, FilesystemScope, NetworkProfileRef, NetworkScope};
use milkdrift_capability::PeerId;
use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_control_protocol::tasks::{
    CreateWorkflowRequest, TaskReadDto, TaskStateDto, WorkflowReceipt,
};
use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, AuthorityPresetConfig, DaemonConfig,
    DaemonHost, DaemonPlan, PeerHostConfig, PeerRelationshipConfig, PeerSideEffectConfig,
    RuntimeHostConfig, SecretSourceConfig, ServingHostConfig, ShutdownConfig, serve,
};
use milkdrift_peer_protocol::{
    DecodeLimits, FeatureSet, HandshakeRequest, HandshakeResponse, HardLimits, PeerAction,
    ProtocolEnvelope, ProtocolVersion as PeerProtocolVersion, ProtocolVersionRange, SessionId,
    decode_envelope,
};

use tempfile::TempDir;
use tokio::{sync::oneshot, task::JoinHandle};
use url::Url;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const OPERATOR_TOKEN: &str = "two-daemon-operator-token";
const PEER_TOKEN: &str = "two-daemon-peer-token";
const DELEGATED_CALL_MS: u64 = 30_000;
// Observe the full accepted call allowance and its subsequent durable reporting.
const RUN_OBSERVATION_TIMEOUT: Duration = Duration::from_millis(DELEGATED_CALL_MS + 10_000);

struct RunningDaemon {
    client: ControlClient,
    stop: oneshot::Sender<()>,
    task: JoinHandle<Result<(), milkdrift_daemon::HostError>>,
}

impl RunningDaemon {
    async fn stop(self) -> TestResult {
        let signal = self.stop.send(());
        tokio::time::timeout(Duration::from_secs(10), self.task).await???;
        signal.map_err(|()| "daemon shutdown receiver disappeared")?;
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn authenticated_catalog_registers_and_disconnect_drains_remote_generation() -> TestResult {
    exercise_peer_execution_turnover(5).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual release-mode peer retention longevity lane"]
async fn peer_execution_retention_longevity_survives_turnover_and_restart() -> TestResult {
    exercise_peer_execution_turnover(100).await
}

async fn exercise_peer_execution_turnover(turnovers: usize) -> TestResult {
    let root_a = tempfile::tempdir()?;
    let root_b = tempfile::tempdir()?;
    let listener_a = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let listener_b = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address_a = listener_a.local_addr()?;
    let address_b = listener_b.local_addr()?;
    let endpoint_a = Url::parse(&format!("http://{address_a}/"))?;
    let endpoint_b = Url::parse(&format!("http://{address_b}/"))?;
    let daemon_a = start(&root_a, "peer-a", "peer-b", &endpoint_b, listener_a).await?;
    let daemon_b = start(&root_b, "peer-b", "peer-a", &endpoint_a, listener_b).await?;

    assert_peer_protocol_boundary(&endpoint_a).await?;

    let unauthenticated = reqwest::Client::new()
        .get(endpoint_a.join("peer/v1/catalog")?)
        .send()
        .await?;
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);

    let before = daemon_b.client.peer("peer-a").await?;
    assert!(!before.connected);
    assert_eq!(before.health, "disconnected");
    let connected = daemon_b.client.peer_action("peer-a", "connect").await?;
    assert!(connected.connected);
    assert_eq!(connected.health, "authenticated_catalog_live");
    assert!(connected.session_id.is_some());
    assert!(connected.catalog_generation.is_some());
    assert!(connected.catalog_digest.is_some());
    assert_eq!(connected.registered_capabilities, 1);
    assert!(connected.catalog_expires_at_unix_ms.is_some());
    assert!(
        daemon_b
            .client
            .peer_action("peer-a", "reload")
            .await?
            .connected
    );
    assert!(
        daemon_a
            .client
            .peer_action("peer-b", "connect")
            .await?
            .connected
    );

    let capabilities = daemon_b.client.capabilities().await?;
    assert!(
        capabilities.iter().any(|capability| {
            capability.capability_id.starts_with("peer:") && capability.current
        })
    );

    let disconnected = daemon_b.client.peer_action("peer-a", "disconnect").await?;
    assert!(!disconnected.connected);
    assert_eq!(disconnected.health, "disconnected");
    assert_eq!(disconnected.registered_capabilities, 0);

    write_secret(&root_a.path().join("peer.token"), "rotated-peer-token")?;
    write_secret(&root_b.path().join("peer.token"), "rotated-peer-token")?;
    let reconnected = daemon_b.client.peer_action("peer-a", "connect").await?;
    assert!(reconnected.connected);

    let receipt = submit_remote(&daemon_b.client, "peer-process-start", &["peer-a"]).await?;
    let first = completed_tasks(&daemon_b.client, &receipt.workflow_id).await?;
    assert_eq!(first.len(), 1);
    let first_task = first.first().ok_or("completed peer task absent")?;
    assert!(first_task.definition.capability.id.starts_with("peer:"));
    assert!(
        first_task
            .frozen
            .as_ref()
            .is_some_and(|start| start.entered)
    );
    // Cross the tiny serving hot-history bound repeatedly before the restart half.
    for index in 0..turnovers {
        // Keep the entire bounded operation inside a live catalog. The serving peer caches
        // snapshots until expiry, so wait out a nearly expired snapshot before reconnecting.
        let peer = daemon_b.client.peer_action("peer-a", "reload").await?;
        assert!(peer.connected);
        let expires = peer
            .catalog_expires_at_unix_ms
            .ok_or("catalog expiry absent")?;
        let now = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis(),
        )?;
        let remaining = expires.saturating_sub(now);
        if remaining < DELEGATED_CALL_MS {
            tokio::time::sleep(Duration::from_millis(remaining + 1)).await;
            let renewed = daemon_b.client.peer_action("peer-a", "reload").await?;
            assert!(renewed.connected);
            assert!(
                renewed
                    .catalog_expires_at_unix_ms
                    .is_some_and(|value| value > expires)
            );
        }
        let receipt = submit_remote(
            &daemon_b.client,
            &format!("peer-turnover-{index}"),
            &["peer-a"],
        )
        .await?;
        completed_tasks(&daemon_b.client, &receipt.workflow_id).await?;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let serving_health = daemon_a.client.health().await?.peer_executions;
    assert!(serving_health.tombstone_count >= u64::try_from(turnovers.saturating_sub(1))?);
    assert!(serving_health.hot_terminal_count <= 2);

    daemon_b.stop().await?;
    daemon_a.stop().await?;
    let listener_a = tokio::net::TcpListener::bind(address_a).await?;
    let listener_b = tokio::net::TcpListener::bind(address_b).await?;
    let daemon_a = start(&root_a, "peer-a", "peer-b", &endpoint_b, listener_a).await?;
    let daemon_b = start(&root_b, "peer-b", "peer-a", &endpoint_a, listener_b).await?;
    assert!(
        daemon_b
            .client
            .peer_action("peer-a", "connect")
            .await?
            .connected
    );
    let restarted =
        submit_remote(&daemon_b.client, "peer-process-restart-start", &["peer-a"]).await?;
    completed_tasks(&daemon_b.client, &restarted.workflow_id).await?;
    assert_eq!(
        completed_tasks(&daemon_b.client, &receipt.workflow_id).await?,
        first
    );

    let revoked = daemon_b.client.peer_action("peer-a", "revoke").await?;
    assert!(!revoked.connected);
    assert!(revoked.revoked);
    assert_eq!(revoked.health, "revoked");
    assert!(
        daemon_a
            .client
            .peer_action("peer-b", "reload")
            .await
            .is_err()
    );
    assert!(!daemon_a.client.peer("peer-b").await?.connected);

    daemon_b.stop().await?;
    daemon_a.stop().await?;
    Ok(())
}

async fn assert_peer_protocol_boundary(endpoint: &Url) -> TestResult {
    let client = reqwest::Client::new();
    let request = HandshakeRequest {
        claimed_peer: PeerId::new("peer-b")?,
        session: SessionId::new("session:protocol-boundary")?,
        versions: ProtocolVersionRange::default(),
        features: FeatureSet::default(),
        limits: HardLimits::default(),
    };
    let current = serde_json::to_value(ProtocolEnvelope::v1(request.clone()))?;
    let current_minor = milkdrift_peer_protocol::PROTOCOL_MINOR_V1;
    let future_minor = current_minor
        .checked_add(1)
        .ok_or("protocol minor exhausted")?;
    for minor in (0..current_minor).chain(std::iter::once(future_minor)) {
        let mut incompatible = current.clone();
        *incompatible
            .pointer_mut("/protocol/minor")
            .ok_or("fixture field /protocol/minor absent")? = serde_json::json!(minor);
        let response = client
            .post(endpoint.join("peer/v1/handshake")?)
            .bearer_auth(PEER_TOKEN)
            .json(&incompatible)
            .send()
            .await?;
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    }

    let response = client
        .post(endpoint.join("peer/v1/handshake")?)
        .bearer_auth(PEER_TOKEN)
        .json(&ProtocolEnvelope::v1(request))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let envelope: ProtocolEnvelope<HandshakeResponse> =
        decode_envelope(&response.bytes().await?, DecodeLimits::default())?;
    assert_eq!(envelope.protocol, PeerProtocolVersion::V1_6);
    assert_eq!(envelope.message.selected_version, PeerProtocolVersion::V1_6);
    Ok(())
}

async fn start(
    root: &TempDir,
    local_peer: &str,
    remote_peer: &str,
    remote_endpoint: &Url,
    listener: tokio::net::TcpListener,
) -> TestResult<RunningDaemon> {
    let config = configuration(root, local_peer, remote_peer, remote_endpoint)?;
    start_plan(config, listener).await
}

async fn start_plan(
    config: DaemonPlan,
    listener: tokio::net::TcpListener,
) -> TestResult<RunningDaemon> {
    let host = DaemonHost::start(config)?;
    let endpoint = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(serve(listener, host, async move {
        // Sender loss also requests shutdown when a test exits early.
        if stopped.await.is_err() {
            tracing::debug!("fixture caller exited; shutting down daemon");
        }
    }));
    let client = ControlClient::new(
        ClientConfig::new(endpoint),
        BearerCredential::new(OPERATOR_TOKEN)?,
    )?;
    client.readiness().await?;
    Ok(RunningDaemon { client, stop, task })
}

fn configuration(
    root: &TempDir,
    local_peer: &str,
    remote_peer: &str,
    remote_endpoint: &Url,
) -> TestResult<DaemonPlan> {
    configuration_document(root, local_peer, remote_peer, remote_endpoint)?
        .validate(root.path())
        .map_err(Into::into)
}

fn configuration_document(
    root: &TempDir,
    local_peer: &str,
    remote_peer: &str,
    remote_endpoint: &Url,
) -> TestResult<DaemonConfig> {
    let operator = root.path().join("operator.token");
    let peer = root.path().join("peer.token");
    write_secret(&operator, OPERATOR_TOKEN)?;
    write_secret(&peer, PEER_TOKEN)?;
    // This immutable fixture value must remain identical across the restart half of the test.
    let expires = 4_000_000_000_000_u64;
    let remote_destination = format!(
        "{}:{}",
        remote_endpoint
            .host_str()
            .ok_or("peer endpoint host absent")?,
        remote_endpoint
            .port_or_known_default()
            .ok_or("peer endpoint port absent")?
    );
    let executable_root = process::executable()?
        .parent()
        .ok_or("canonical test executable has no parent")?
        .to_owned();
    let temporary_root = std::env::temp_dir().canonicalize()?;
    let mut operator_authority = ActorGrantConfig::dangerous_administrator();
    operator_authority.resources.network = NetworkScope::new(
        BTreeSet::from([NetworkProfileRef::new(format!("peer:{remote_peer}"))?]),
        BTreeSet::from([remote_destination]),
    )?;
    let process_profiles = if local_peer == "peer-a" {
        vec![configured_process_profile(root)?]
    } else {
        Vec::new()
    };
    Ok(DaemonConfig {
        schema_version: milkdrift_daemon::DAEMON_CONFIG_SCHEMA_VERSION,
        host_id: local_peer.to_owned(),
        name: "Milkdrift".to_owned(),
        serving: ServingHostConfig {
            worker_threads: 2,
            maximum_global_active: 2,
            maximum_dispatch_queue: 2,
            maximum_hot_terminal_records: 2,
            archive_batch_size: 1,
            observation_hot_retention_ms: 1,
            recovery_page: 2,
            poll_interval_ms: 5,
            ..ServingHostConfig::default()
        },
        role: if local_peer == "peer-a" {
            milkdrift_control_protocol::HostRole::ExecutionOnly
        } else {
            milkdrift_control_protocol::HostRole::WorkflowEnabled
        },
        data_root: root.path().join("data"),
        bind: "127.0.0.1:0".parse()?,
        secret_sources: BTreeMap::from([
            (
                "credential:operator".to_owned(),
                SecretSourceConfig::File { path: operator },
            ),
            (
                "credential:peer".to_owned(),
                SecretSourceConfig::File { path: peer },
            ),
        ]),
        actors: vec![ActorBindingConfig {
            credential_ref: "credential:operator".to_owned(),
            actor: format!("human:{local_peer}-operator"),
            grant_id: format!("grant:{local_peer}-operator"),
            grant_revision: 2,
            revocation_generation: 0,
            preset: AuthorityPresetConfig::Controller,
            authority: operator_authority,
            enabled: true,
        }],
        runtime: RuntimeHostConfig {
            maintenance_interval_ms: 10,
            ..RuntimeHostConfig::default()
        },
        adapters: AdapterConfig {
            managed_linux: None,
            process_profiles,
            ..AdapterConfig::default()
        },
        peers: PeerHostConfig::Enabled {
            relationships: vec![PeerRelationshipConfig {
                nested_invocations: None,
                peer_id: remote_peer.to_owned(),
                endpoint: remote_endpoint.to_string(),
                credential_ref: "credential:peer".to_owned(),
                insecure_loopback_development: true,
                minimum_minor: milkdrift_peer_protocol::PROTOCOL_MINOR_V1,
                maximum_minor: milkdrift_peer_protocol::PROTOCOL_MINOR_V1,
                actions: BTreeSet::from([
                    PeerAction::ReadCatalog,
                    PeerAction::Invoke,
                    PeerAction::Cancel,
                    PeerAction::ArtifactUpload,
                    PeerAction::ArtifactDownload,
                ]),
                capability_allow: BTreeSet::from(["golden-local-process".to_owned()]),
                capability_deny: BTreeSet::new(),
                operation_allow: BTreeSet::from(["process.execute".to_owned()]),
                maximum_side_effect: PeerSideEffectConfig::None,
                execution_filesystem: vec![
                    FilesystemScope::from_canonical_host_path(
                        &executable_root,
                        BTreeSet::from([AccessMode::Execute]),
                    )?,
                    FilesystemScope::from_canonical_host_path(
                        &temporary_root,
                        BTreeSet::from([AccessMode::Read, AccessMode::Write]),
                    )?,
                ],
                execution_network_profiles: BTreeSet::new(),
                execution_network_destinations: BTreeSet::new(),
                execution_secrets: BTreeSet::new(),
                maximum_concurrent: 2,
                maximum_requests_per_minute: 600,
                maximum_artifact_bytes: 1_048_576,
                artifact_sensitivities: BTreeSet::from([
                    milkdrift_workspace::ArtifactSensitivity::Internal,
                    milkdrift_workspace::ArtifactSensitivity::Restricted,
                ]),
                maximum_duration_ms: DELEGATED_CALL_MS,
                maximum_cost_micros: 0,
                cost_currency: None,
                maximum_input_units: None,
                maximum_output_units: None,
                maximum_observations: 128,
                // Setup and the placement fixture's two sequential calls share this catalog.
                catalog_ttl_ms: DELEGATED_CALL_MS * 3,
                trust_zone: "two-daemon-test".to_owned(),
                delegation_ref: "delegation:two-daemon".to_owned(),
                revocation_generation: 0,
                expires_at_unix_ms: expires,
                enabled: true,
            }],
        },
        shutdown: ShutdownConfig::default(),
        security_audit_record_bound: 100,
    })
}

async fn submit_remote(
    client: &ControlClient,
    key: &str,
    hosts: &[&str],
) -> TestResult<WorkflowReceipt> {
    let capabilities = client.capabilities().await?;
    let tasks=hosts.iter().enumerate().map(|(index,host)| {
        let selected=capabilities.iter().find(|candidate|candidate.peer_id.as_deref()==Some(*host) && candidate.current && !candidate.draining && candidate.operations.iter().any(|operation|operation=="process.execute")).ok_or("requested peer has no authorized configured process")?;
        Ok(serde_json::json!({"alias":format!("repository-{index}"),"definition":{
            "title":format!("Inspect repository on {host}"),"description":"Physical placement fixture","instructions":"Inspect the repository assigned to this exact host",
            "capability":{"id":selected.capability_id,"operation":"process.execute"},"inputs":{},"outputs":{},
            "dependencies":if index==0 {Vec::<String>::new()} else {vec![format!("repository-{}",index-1)]},"required":true,"corrective_of":null,"paths":[]
        }}))
    }).collect::<TestResult<Vec<_>>>()?;
    let request: CreateWorkflowRequest = serde_json::from_value(
        serde_json::json!({"request_id":key,"title":key,"description":"Authenticated peer execution", "tasks":tasks,"inputs":{},"finish_when_settled":true,"limits":null}),
    )?;
    Ok(client.create_workflow(&request).await?)
}

async fn completed_tasks(client: &ControlClient, workflow: &str) -> TestResult<Vec<TaskReadDto>> {
    let deadline = tokio::time::Instant::now() + RUN_OBSERVATION_TIMEOUT * 2;
    loop {
        let read = client.workflow(workflow).await?;
        if read.tasks.iter().any(|task| {
            matches!(
                task.state,
                TaskStateDto::Failed | TaskStateDto::Stopped | TaskStateDto::Uncertain
            )
        }) {
            let mut details = Vec::new();
            for task in &read.tasks {
                details.push(client.task(workflow, &task.id).await?);
            }
            return Err(format!("peer task execution failed: {details:?}").into());
        }
        if read
            .tasks
            .iter()
            .all(|task| task.state == TaskStateDto::Completed)
        {
            let mut tasks = Vec::new();
            for task in &read.tasks {
                tasks.push(client.task(workflow, &task.id).await?);
            }
            tasks.sort_by(|a, b| a.definition.title.cmp(&b.definition.title));
            return Ok(tasks);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("peer workflow did not settle: {read:?}").into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn write_secret(path: &std::path::Path, value: &str) -> TestResult {
    fs::write(path, value)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
