//! Deterministic two-daemon authenticated catalog and local registration coverage.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::Ipv4Addr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, AuthorityPresetConfig, DaemonConfig,
    DaemonHost, PeerHostConfig, PeerRelationshipConfig, PeerSideEffectConfig, RuntimeHostConfig,
    SecretSourceConfig, ShutdownConfig, ValidatedDaemonConfig, serve,
};
use milkdrift_peer_protocol::PeerAction;
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
    let root_a = tempfile::tempdir()?;
    let root_b = tempfile::tempdir()?;
    let listener_a = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let listener_b = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let endpoint_a = Url::parse(&format!("http://{}/", listener_a.local_addr()?))?;
    let endpoint_b = Url::parse(&format!("http://{}/", listener_b.local_addr()?))?;
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
) -> TestResult<ValidatedDaemonConfig> {
    let operator = root.path().join("operator.token");
    let peer = root.path().join("peer.token");
    write_secret(&operator, OPERATOR_TOKEN)?;
    write_secret(&peer, PEER_TOKEN)?;
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .saturating_add(600_000);
    let expires = u64::try_from(expires)?;
    DaemonConfig {
        schema_version: 2,
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
            authority: ActorGrantConfig::dangerous_administrator(),
            enabled: true,
        }],
        runtime: RuntimeHostConfig {
            maintenance_interval_ms: 10,
            ..RuntimeHostConfig::default()
        },
        adapters: AdapterConfig::default(),
        peers: PeerHostConfig {
            enabled: true,
            local_peer_id: Some(local_peer.to_owned()),
            relationships: vec![PeerRelationshipConfig {
                peer_id: remote_peer.to_owned(),
                endpoint: remote_endpoint.to_string(),
                credential_ref: "credential:peer".to_owned(),
                insecure_loopback_development: true,
                minimum_minor: 0,
                maximum_minor: 0,
                actions: BTreeSet::from([
                    PeerAction::ReadCatalog,
                    PeerAction::Invoke,
                    PeerAction::Cancel,
                ]),
                capability_allow: BTreeSet::from(["milkdrift-workflow-control".to_owned()]),
                capability_deny: BTreeSet::new(),
                operation_allow: BTreeSet::from(["workflow.inspect".to_owned()]),
                maximum_side_effect: PeerSideEffectConfig::ReadOnly,
                maximum_concurrent: 2,
                maximum_requests_per_minute: 600,
                maximum_artifact_bytes: 1_048_576,
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
        command_ledger_bound: 100,
    }
    .validate(root.path())
    .map_err(Into::into)
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
