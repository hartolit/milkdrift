//! Deterministic two-daemon authenticated catalog and local registration coverage.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::Ipv4Addr,
    time::Duration,
};

use milkdrift_authority::{AccessMode, FilesystemScope, NetworkProfileRef, NetworkScope, PeerId};
use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, BlueprintRevisionDocument, Edge, EdgeId, EdgeKind, Mutation,
    MutationBatch, Node, NodeId, NodeKind, PortId, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{CapabilityRequirement, OperationId};
use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_control_protocol::{Command, CommandRequest, PageRequest, ProtocolVersion};
use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, DaemonConfig, DaemonHost, DaemonPlan, PeerHostConfig,
    PeerRelationshipConfig, PeerServingConfig, PeerSideEffectConfig, RuntimeHostConfig,
    SecretSourceConfig, ShutdownConfig, serve,
};
use milkdrift_peer_protocol::PeerAction;
use milkdrift_peer_protocol::PeerRequestId;
use milkdrift_persistence::{PageSize, PeerExecutionStore};
use milkdrift_redb_store::RedbStore;
use tempfile::TempDir;
use tokio::{sync::oneshot, task::JoinHandle};
use url::Url;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const OPERATOR_TOKEN: &str = "two-daemon-operator-token";
const PEER_TOKEN: &str = "two-daemon-peer-token";

struct RunningDaemon {
    client: ControlClient,
    stop: oneshot::Sender<()>,
    task: JoinHandle<Result<(), milkdrift_daemon::HostError>>,
}

impl RunningDaemon {
    async fn stop(self) -> TestResult {
        let _ = self.stop.send(());
        tokio::time::timeout(Duration::from_secs(10), self.task).await???;
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

    let unauthenticated = reqwest::Client::new()
        .get(endpoint_a.join("peer/v1/catalog")?)
        .send()
        .await?;
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);

    let before = daemon_b.client.peer("peer-a").await?;
    assert!(!before.connected);
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
    assert_eq!(disconnected.registered_capabilities, 0);

    write_secret(&root_a.path().join("peer.token"), "rotated-peer-token")?;
    write_secret(&root_b.path().join("peer.token"), "rotated-peer-token")?;
    let reconnected = daemon_b.client.peer_action("peer-a", "connect").await?;
    assert!(reconnected.connected);

    let imported = daemon_b
        .client
        .submit(&command(
            "peer-process-import",
            Command::ImportBlueprint {
                document: process_blueprint()?,
            },
        ))
        .await?;
    let revision = imported
        .value
        .get("revision_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("peer process import omitted revision")?;
    daemon_b
        .client
        .submit(&command(
            "peer-process-start",
            Command::StartRun {
                run_id: "run-peer-process".to_owned(),
                workflow_id: "daemon-peer-process".to_owned(),
                revision_id: revision.to_owned(),
            },
        ))
        .await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let run = daemon_b.client.run("run-peer-process").await?;
        if run.lifecycle == "terminal" {
            if run.terminal.as_deref() != Some("succeeded") {
                let health_a = daemon_a.client.health().await?;
                let health_b = daemon_b.client.health().await?;
                let timeline = daemon_b
                    .client
                    .timeline(
                        "run-peer-process",
                        &PageRequest {
                            cursor: None,
                            limit: 100,
                        },
                    )
                    .await?;
                let attempt_id = timeline
                    .items
                    .iter()
                    .find_map(|entry| entry.attempt_id.as_deref())
                    .ok_or("failed peer run omitted attempt")?;
                let attempt = daemon_b
                    .client
                    .attempt("run-peer-process", attempt_id)
                    .await?;
                let invocation = attempt
                    .invocation_id
                    .as_deref()
                    .ok_or("failed peer attempt omitted invocation")?;
                daemon_b.stop().await?;
                daemon_a.stop().await?;
                let peer_store = RedbStore::open(root_a.path().join("data"))?;
                let record = peer_store
                    .peer_execution_by_request(
                        &PeerId::new("peer-b")?,
                        &PeerRequestId::new(format!("request:{invocation}"))?,
                    )?
                    .ok_or("serving peer omitted accepted execution")?;
                let observations = peer_store.peer_observations(
                    &PeerId::new("peer-b")?,
                    record.execution(),
                    0,
                    PageSize::new(128)?,
                )?;
                return Err(format!(
                    "remote process run failed: run={run:?}, timeline={:?}, attempt={attempt:?}, remote_record={record:?}, remote_observations={:?}, health_a={health_a:?}, health_b={health_b:?}",
                    timeline.items, observations.observations
                )
                .into());
            }
            let timeline = daemon_b
                .client
                .timeline(
                    "run-peer-process",
                    &PageRequest {
                        cursor: None,
                        limit: 100,
                    },
                )
                .await?;
            let attempt_id = timeline
                .items
                .iter()
                .find_map(|entry| entry.attempt_id.as_deref())
                .ok_or("peer process run omitted its attempt")?;
            let attempt = daemon_b
                .client
                .attempt("run-peer-process", attempt_id)
                .await?;
            assert_eq!(attempt.peer_id.as_deref(), Some("peer-a"));
            assert!(
                attempt
                    .capability_id
                    .as_deref()
                    .is_some_and(|identity| identity.starts_with("peer:"))
            );
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let capabilities = daemon_b.client.capabilities().await?;
            let health = daemon_b.client.health().await?;
            return Err(format!(
                "remote process run did not terminate: run={run:?}, capabilities={capabilities:?}, health={health:?}"
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Cross the tiny serving hot-history bound repeatedly before the restart half.
    for index in 0..turnovers {
        let run_id = format!("run-peer-turnover-{index}");
        daemon_b
            .client
            .submit(&command(
                &format!("peer-turnover-start-{index}"),
                Command::StartRun {
                    run_id: run_id.clone(),
                    workflow_id: "daemon-peer-process".to_owned(),
                    revision_id: revision.to_owned(),
                },
            ))
            .await?;
        let turnover_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            let run = daemon_b.client.run(&run_id).await?;
            if run.lifecycle == "terminal" {
                assert_eq!(run.terminal.as_deref(), Some("succeeded"));
                break;
            }
            if tokio::time::Instant::now() >= turnover_deadline {
                return Err(format!("remote turnover run stalled: {run:?}").into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
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
    daemon_b
        .client
        .submit(&command(
            "peer-process-restart-start",
            Command::StartRun {
                run_id: "run-peer-process-after-restart".to_owned(),
                workflow_id: "daemon-peer-process".to_owned(),
                revision_id: revision.to_owned(),
            },
        ))
        .await?;
    let restart_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let run = daemon_b
            .client
            .run("run-peer-process-after-restart")
            .await?;
        if run.lifecycle == "terminal" {
            assert_eq!(run.terminal.as_deref(), Some("succeeded"));
            break;
        }
        if tokio::time::Instant::now() >= restart_deadline {
            return Err(format!("remote process run after daemon restart stalled: {run:?}").into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

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

async fn start(
    root: &TempDir,
    local_peer: &str,
    remote_peer: &str,
    remote_endpoint: &Url,
    listener: tokio::net::TcpListener,
) -> TestResult<RunningDaemon> {
    let config = configuration(root, local_peer, remote_peer, remote_endpoint)?;
    let host = DaemonHost::start(config)?;
    let endpoint = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(serve(listener, host, async move {
        let _ = stopped.await;
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
    DaemonConfig {
        schema_version: milkdrift_daemon::DAEMON_CONFIG_SCHEMA_VERSION,
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
            grant_revision: 1,
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
            process_profiles,
            ..AdapterConfig::default()
        },
        peers: PeerHostConfig::Enabled {
            local_peer_id: local_peer.to_owned(),
            serving: PeerServingConfig {
                worker_threads: 2,
                maximum_global_active: 2,
                maximum_dispatch_queue: 2,
                maximum_hot_terminal_records: 2,
                archive_batch_size: 1,
                observation_hot_retention_ms: 1,
                recovery_page: 2,
                poll_interval_ms: 5,
            },
            relationships: vec![PeerRelationshipConfig {
                peer_id: remote_peer.to_owned(),
                endpoint: remote_endpoint.to_string(),
                credential_ref: "credential:peer".to_owned(),
                insecure_loopback_development: true,
                minimum_minor: 1,
                maximum_minor: 1,
                actions: BTreeSet::from([
                    PeerAction::ReadCatalog,
                    PeerAction::Invoke,
                    PeerAction::Cancel,
                ]),
                capability_allow: BTreeSet::from(["golden-local-process".to_owned()]),
                capability_deny: BTreeSet::new(),
                operation_allow: BTreeSet::from(["process.execute".to_owned()]),
                maximum_side_effect: PeerSideEffectConfig::None,
                execution_filesystem: vec![
                    FilesystemScope::new("/usr/bin", BTreeSet::from([AccessMode::Execute]))?,
                    FilesystemScope::new(
                        "/tmp",
                        BTreeSet::from([AccessMode::Read, AccessMode::Write]),
                    )?,
                ],
                execution_network_profiles: BTreeSet::new(),
                execution_network_destinations: BTreeSet::new(),
                execution_secrets: BTreeSet::new(),
                maximum_concurrent: 2,
                maximum_requests_per_minute: 600,
                maximum_artifact_bytes: 1_048_576,
                artifact_sensitivities: BTreeSet::new(),
                maximum_duration_ms: 30_000,
                maximum_cost_micros: 0,
                maximum_observations: 128,
                catalog_ttl_ms: 30_000,
                trust_zone: "two-daemon-test".to_owned(),
                delegation_ref: "delegation:two-daemon".to_owned(),
                revocation_generation: 0,
                expires_at_unix_ms: expires,
                enabled: true,
            }],
        },
        shutdown: ShutdownConfig::default(),
        application_receipts: ApplicationReceiptConfig {
            hot_receipt_bound: 100,
            archive_batch_size: 10,
        },
        security_audit_record_bound: 100,
    }
    .validate(root.path())
    .map_err(Into::into)
}

fn configured_process_profile(directory: &TempDir) -> TestResult<std::path::PathBuf> {
    let executable = std::path::Path::new("/bin/echo");
    let bytes = fs::read(executable)?;
    let mut profile: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../adapters/local-process/tests/fixtures/process-profile-v2.json"
    ))?;
    profile["profile"]["implementation"]["content_digest"] =
        serde_json::json!(format!("b3_{}", blake3::hash(&bytes)));
    profile["profile"]["implementation"]["size_bytes"] = serde_json::json!(bytes.len());
    let path = directory.path().join("process-profile-v2.json");
    fs::write(&path, serde_json::to_vec(&profile)?)?;
    Ok(path)
}

fn command(identity: &str, command: Command) -> CommandRequest {
    CommandRequest {
        protocol: ProtocolVersion::CURRENT,
        command_id: identity.to_owned(),
        expected_sequence: None,
        expected_revision: None,
        reason: "two-daemon peer execution test".to_owned(),
        evidence: Vec::new(),
        command,
    }
}

fn process_blueprint() -> TestResult<serde_json::Value> {
    let task = Node::new(
        NodeId::new("process")?,
        NodeKind::task_direct_inputs(CapabilityRequirement::new(OperationId::new(
            "process.execute",
        )?))?,
    )?
    .with_control_output(PortId::new("next")?)?;
    let terminal = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(PortId::new("in")?)?;
    let revision = BlueprintRevision::genesis(
        WorkflowId::new("daemon-peer-process")?,
        MutationBatch::new(vec![
            Mutation::AddNode { node: task },
            Mutation::AddNode { node: terminal },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("process-done")?,
                    EdgeKind::Control,
                    NodeId::new("process")?,
                    PortId::new("next")?,
                    NodeId::new("done")?,
                    PortId::new("in")?,
                ),
            },
        ])?,
        AuthorRef::new("human:two-daemon-test")?,
        "execute a process through the authenticated peer adapter",
    )?;
    Ok(serde_json::from_slice(
        &BlueprintRevisionDocument::new(&revision).to_canonical_json()?,
    )?)
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
