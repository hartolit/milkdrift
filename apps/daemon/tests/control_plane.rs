//! Loopback-only integration coverage for the durable daemon control plane.

use std::{
    collections::BTreeMap,
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use futures_util::StreamExt as _;
use milkdrift_authority::{
    ArtifactAuthorityScope, AuthorityBudget, BoundaryTimeMillis, CapabilityAuthorityScope,
    DaemonAuthorityScope, LayoutAuthorityScope, NetworkScope, PeerAuthorityScope, ResourceScope,
    WorkflowRunScope, WorkspaceAuthorityScope,
};
use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, BlueprintRevisionDocument, Edge, EdgeId, EdgeKind, Mutation,
    MutationBatch, Node, NodeId, NodeKind, PortId, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{CapabilityId, CapabilityRequirement, OperationId, SideEffectClass};
use milkdrift_control_client::{BearerCredential, ClientConfig, ClientError, ControlClient};
use milkdrift_control_protocol::{
    Command, CommandRequest, ErrorCode, LayoutDocument, LayoutPoint, Observation, PageRequest,
    ProtocolVersion, decode_json,
};
use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, AuthorityPresetConfig, DaemonConfig,
    DaemonHost, PeerHostConfig, RuntimeHostConfig, SecretSourceConfig, ShutdownConfig,
    ValidatedDaemonConfig, serve,
};
use milkdrift_persistence::{ArtifactPublicationId, ArtifactStore, BeginArtifactPublication};
use milkdrift_redb_store::RedbStore;
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType, RunId,
    WorkspaceBudget, WorkspaceUsage,
};
use tempfile::TempDir;
use tokio::{sync::oneshot, task::JoinHandle};
use url::Url;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const CONTROLLER_TOKEN: &str = "controller-integration-token";
const OBSERVER_TOKEN: &str = "observer-integration-token";

struct RunningDaemon {
    endpoint: Url,
    client: ControlClient,
    stop: oneshot::Sender<()>,
    task: JoinHandle<Result<(), milkdrift_daemon::HostError>>,
}

impl RunningDaemon {
    async fn stop(self) -> TestResult {
        let Self { stop, task, .. } = self;
        let _ = stop.send(());
        tokio::time::timeout(Duration::from_secs(10), task).await???;
        Ok(())
    }
}

async fn start(config: ValidatedDaemonConfig, token: &str) -> TestResult<RunningDaemon> {
    let host = DaemonHost::start(config)?;
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    let endpoint = Url::parse(&format!("http://{address}/"))?;
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(serve(listener, host, async move {
        let _ = stopped.await;
    }));
    let client = client(&endpoint, token)?;
    let health = client.readiness().await?;
    assert!(health.ready);
    Ok(RunningDaemon {
        endpoint,
        client,
        stop,
        task,
    })
}

fn client(endpoint: &Url, token: &str) -> Result<ControlClient, ClientError> {
    let mut config = ClientConfig::new(endpoint.clone());
    config.safe_query_retries = 0;
    config.retry_delay = Duration::from_millis(10);
    ControlClient::new(config, BearerCredential::new(token)?)
}

fn configuration(directory: &TempDir, request_queue: u32) -> TestResult<ValidatedDaemonConfig> {
    configuration_with_process_profiles(directory, request_queue, Vec::new())
}

fn configuration_with_process_profiles(
    directory: &TempDir,
    request_queue: u32,
    process_profiles: Vec<std::path::PathBuf>,
) -> TestResult<ValidatedDaemonConfig> {
    write_secret(&directory.path().join("controller.token"), CONTROLLER_TOKEN)?;
    write_secret(&directory.path().join("observer.token"), OBSERVER_TOKEN)?;
    let runtime = RuntimeHostConfig {
        request_queue,
        maintenance_interval_ms: 10,
        ..RuntimeHostConfig::default()
    };
    DaemonConfig {
        schema_version: 3,
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
                grant_revision: 1,
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
            process_profiles,
            ..AdapterConfig::default()
        },
        peers: PeerHostConfig::default(),
        shutdown: ShutdownConfig::default(),
        command_ledger_bound: 1_000,
    }
    .validate(directory.path())
    .map_err(Into::into)
}

fn observer_authority() -> TestResult<ActorGrantConfig> {
    Ok(ActorGrantConfig {
        resources: ResourceScope {
            workflow_run: WorkflowRunScope::Workflow {
                workflow: WorkflowId::new("golden")?,
            },
            capability: CapabilityAuthorityScope::new(
                std::collections::BTreeSet::from([CapabilityId::new(
                    "milkdrift-workflow-control",
                )?]),
                std::collections::BTreeSet::new(),
                std::collections::BTreeSet::from([OperationId::new("workflow.inspect")?]),
                std::collections::BTreeSet::new(),
                std::collections::BTreeSet::new(),
                std::collections::BTreeSet::new(),
                SideEffectClass::ReadOnly,
            )?,
            filesystem: Vec::new(),
            network: NetworkScope::empty(),
            secrets: std::collections::BTreeSet::new(),
            artifacts: ArtifactAuthorityScope::none(),
            layouts: LayoutAuthorityScope::none(),
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

fn write_secret(path: &std::path::Path, value: &str) -> TestResult {
    fs::write(path, value)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn publish_restricted_test_artifact(root: &std::path::Path) -> TestResult<(String, Vec<u8>)> {
    let artifact_id = "artifact-daemon-protected-read";
    let bytes = b"protected daemon artifact".to_vec();
    let metadata = ArtifactMetadata::new(
        ArtifactReference::new(
            ArtifactId::new(artifact_id)?,
            ContentDigest::for_bytes(&bytes),
            MediaType::new("application/octet-stream")?,
            u64::try_from(bytes.len())?,
        ),
        ArtifactSensitivity::Restricted,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("daemon-authorization-integration")?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-daemon-protected-read")?,
        RunId::new("run-daemon-protected-read")?,
        metadata,
        WorkspaceBudget::new(0, 0, 0, 1, 1_024, 1_024)?,
        WorkspaceUsage::EMPTY,
    )?;
    let store = RedbStore::open(root)?;
    store.begin_publication(&publication)?;
    store.write_chunk(publication.publication(), 0, &bytes)?;
    store.commit_publication(publication.publication())?;
    Ok((artifact_id.to_owned(), bytes))
}

fn request(command_id: &str, expected_sequence: Option<u64>, command: Command) -> CommandRequest {
    CommandRequest {
        protocol: ProtocolVersion::CURRENT,
        command_id: command_id.to_owned(),
        expected_sequence,
        expected_revision: None,
        reason: "deterministic daemon integration test".to_owned(),
        evidence: Vec::new(),
        command,
    }
}

fn blueprint() -> TestResult<serde_json::Value> {
    decode_json(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/blueprint/tests/fixtures/revision-v2.json"
    )))
    .map_err(Into::into)
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
        WorkflowId::new("daemon-process")?,
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
        AuthorRef::new("human:daemon-integration")?,
        "configured process adapter integration",
    )?;
    let bytes = BlueprintRevisionDocument::new(&revision).to_canonical_json()?;
    decode_json(&bytes).map_err(Into::into)
}

async fn import_blueprint(client: &ControlClient, command_id: &str) -> TestResult<String> {
    let accepted = client
        .submit(&request(
            command_id,
            None,
            Command::ImportBlueprint {
                document: blueprint()?,
            },
        ))
        .await?;
    accepted
        .value
        .get("revision_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "import response did not contain a revision identity".into())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_auth_startup_readiness_and_authority() -> TestResult {
    let directory = tempfile::tempdir()?;
    let daemon = start(configuration(&directory, 16)?, CONTROLLER_TOKEN).await?;
    let raw = reqwest::Client::new();
    let unauthenticated = raw.get(daemon.endpoint.join("v1/health")?).send().await?;
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);
    let invalid = raw
        .get(daemon.endpoint.join("v1/health")?)
        .bearer_auth("invalid-token")
        .send()
        .await?;
    assert_eq!(invalid.status(), reqwest::StatusCode::UNAUTHORIZED);

    let authority = daemon.client.authority().await?;
    assert_eq!(authority.actor, "human:integration-controller");
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let denied = observer
        .submit(&request(
            "observer-import-denied",
            None,
            Command::ImportBlueprint {
                document: blueprint()?,
            },
        ))
        .await;
    assert!(matches!(
        denied,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_read_matrix_and_continuations_fail_closed() -> TestResult {
    let directory = tempfile::tempdir()?;
    let profile = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../adapters/local-process/tests/fixtures/process-profile-v1.json")
        .canonicalize()?;
    let config = configuration_with_process_profiles(&directory, 16, vec![profile])?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let golden_revision = import_blueprint(&daemon.client, "matrix-import-golden").await?;
    let process_import = daemon
        .client
        .submit(&request(
            "matrix-import-process",
            None,
            Command::ImportBlueprint {
                document: process_blueprint()?,
            },
        ))
        .await?;
    let process_revision = process_import
        .value
        .get("revision_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("process import omitted revision")?
        .to_owned();
    daemon
        .client
        .submit(&request(
            "matrix-start-golden",
            None,
            Command::StartRun {
                run_id: "run-matrix".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: golden_revision.clone(),
            },
        ))
        .await?;
    let layout = LayoutDocument {
        schema_version: 1,
        workflow_id: "golden".to_owned(),
        revision_id: golden_revision.clone(),
        generation: 1,
        author: "human:client-placeholder".to_owned(),
        digest: String::new(),
        nodes: BTreeMap::from([(
            "first".to_owned(),
            LayoutPoint {
                x: 1.0,
                y: 2.0,
                width: None,
                height: None,
            },
        )]),
        collapsed_groups: std::collections::BTreeSet::new(),
        annotations: BTreeMap::new(),
        viewport: None,
    }
    .seal()?;
    daemon
        .client
        .submit(&request(
            "matrix-put-layout",
            None,
            Command::PutLayout { layout },
        ))
        .await?;

    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(observer.readiness().await?.ready);
    assert!(matches!(
        observer.health().await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    let visible = observer.capabilities().await?;
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].capability_id, "milkdrift-workflow-control");
    assert!(visible[0].provider_profile.is_none());
    assert_eq!(
        observer
            .revisions(
                Some("golden"),
                &PageRequest {
                    cursor: None,
                    limit: 10,
                },
            )
            .await?
            .items
            .len(),
        1
    );
    assert!(matches!(
        observer.revision(&process_revision).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert!(matches!(
        observer.revisions(
            Some("daemon-process"),
            &PageRequest { cursor: None, limit: 10 },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert_eq!(
        observer.run("run-matrix").await?.workflow_id.as_deref(),
        Some("golden")
    );
    assert!(
        observer
            .timeline(
                "run-matrix",
                &PageRequest {
                    cursor: None,
                    limit: 10,
                },
            )
            .await?
            .items
            .iter()
            .all(|event| event.run_id == "run-matrix")
    );
    assert!(
        observer
            .proposals(
                "run-matrix",
                &PageRequest {
                    cursor: None,
                    limit: 10,
                },
            )
            .await?
            .items
            .is_empty()
    );
    assert!(matches!(
        observer.runs(
            None,
            Some("daemon-process"),
            &PageRequest { cursor: None, limit: 10 },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert!(matches!(
        observer.layout("golden", &golden_revision).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));

    let first_page = daemon
        .client
        .revisions(
            None,
            &PageRequest {
                cursor: None,
                limit: 1,
            },
        )
        .await?;
    let cursor = first_page
        .next_cursor
        .ok_or("expected a second revision page")?;
    assert!(matches!(
        observer.revisions(
            None,
            &PageRequest {
                cursor: Some(cursor.clone()),
                limit: 1,
            },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput
    ));

    let mut health = daemon.client.subscribe("v1/stream/health", None);
    let health_cursor = tokio::time::timeout(Duration::from_secs(3), health.next())
        .await?
        .ok_or("health stream closed before an observation")??
        .cursor;
    drop(health);
    daemon.stop().await?;

    let mut narrowed = config;
    narrowed.document.actors[0].grant_revision = 2;
    narrowed.document.actors[0].authority.resources.capability =
        CapabilityAuthorityScope::none(SideEffectClass::None);
    let restarted = start(narrowed, CONTROLLER_TOKEN).await?;
    assert!(matches!(
        restarted.client.revisions(
            None,
            &PageRequest {
                cursor: Some(cursor),
                limit: 1,
            },
        ).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput
    ));
    let mut reconnect = restarted
        .client
        .subscribe("v1/stream/health", Some(health_cursor));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), reconnect.next())
            .await?
            .ok_or("narrowed stream ended without a typed error")?,
        Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput
    ));
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_command_idempotency_restart_and_stale_conflict() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let revision = import_blueprint(&daemon.client, "import-retry").await?;
    let replay = daemon
        .client
        .submit(&request(
            "import-retry",
            None,
            Command::ImportBlueprint {
                document: blueprint()?,
            },
        ))
        .await?;
    assert!(replay.replayed);

    let started = daemon
        .client
        .submit(&request(
            "start-run",
            None,
            Command::StartRun {
                run_id: "run-integration".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: revision,
            },
        ))
        .await?;
    assert!(started.resulting_sequence.is_some());
    let stale = daemon
        .client
        .submit(&request(
            "pause-stale",
            Some(0),
            Command::PauseRun {
                run_id: "run-integration".to_owned(),
            },
        ))
        .await;
    assert!(matches!(
        stale,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict
    ));
    daemon.stop().await?;

    let restarted = start(config, CONTROLLER_TOKEN).await?;
    let restart_replay = restarted
        .client
        .submit(&request(
            "import-retry",
            None,
            Command::ImportBlueprint {
                document: blueprint()?,
            },
        ))
        .await?;
    assert!(restart_replay.replayed);
    assert!(restarted.client.run("run-integration").await?.sequence > 0);
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn daemon_bounded_overload_returns_stable_error() -> TestResult {
    let directory = tempfile::tempdir()?;
    let daemon = start(configuration(&directory, 1)?, CONTROLLER_TOKEN).await?;
    let raw = reqwest::Client::new();
    let target = daemon.endpoint.join("v1/revisions?limit=100")?;
    let mut overload = None;

    for _round in 0..4 {
        let barrier = Arc::new(tokio::sync::Barrier::new(129));
        let mut requests = Vec::new();
        for _ in 0..128 {
            let barrier = barrier.clone();
            let raw = raw.clone();
            let target = target.clone();
            requests.push(tokio::spawn(async move {
                barrier.wait().await;
                raw.get(target).bearer_auth(CONTROLLER_TOKEN).send().await
            }));
        }
        barrier.wait().await;
        for request in requests {
            let response = request.await??;
            if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                overload = Some(response.bytes().await?);
            }
        }
        if overload.is_some() {
            break;
        }
    }

    let bytes = overload.ok_or("concurrent requests did not exercise the queue bound")?;
    let error: milkdrift_control_protocol::ErrorEnvelope = decode_json(&bytes)?;
    assert_eq!(error.code, ErrorCode::Overload);
    assert!(error.retryable);
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_stream_reconnect_auth_rotation_and_shutdown() -> TestResult {
    let directory = tempfile::tempdir()?;
    let daemon = start(configuration(&directory, 16)?, CONTROLLER_TOKEN).await?;
    let revision = import_blueprint(&daemon.client, "stream-import").await?;
    daemon
        .client
        .submit(&request(
            "stream-start",
            None,
            Command::StartRun {
                run_id: "run-stream".to_owned(),
                workflow_id: "golden".to_owned(),
                revision_id: revision,
            },
        ))
        .await?;

    let mut stream = daemon.client.subscribe("v1/runs/run-stream/stream", None);
    let first = tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await?
        .ok_or("run stream closed before its first observation")??;
    assert_eq!(first.feed, "run:run-stream");
    let first_position = first.cursor.position_for("run:run-stream")?;
    drop(stream);

    let mut resumed = daemon
        .client
        .subscribe("v1/runs/run-stream/stream", Some(first.cursor));
    let second = tokio::time::timeout(Duration::from_secs(3), resumed.next())
        .await?
        .ok_or("resumed run stream closed")??;
    assert!(second.cursor.position_for("run:run-stream")? > first_position);
    drop(resumed);

    let mut capabilities = daemon.client.subscribe("v1/stream/capabilities", None);
    let capability = tokio::time::timeout(Duration::from_secs(3), capabilities.next())
        .await?
        .ok_or("capability stream closed before its first observation")??;
    assert_eq!(capability.feed, "capability-health");
    assert!(matches!(capability.observation, Observation::Capability(_)));
    drop(capabilities);

    let mut health = daemon.client.subscribe("v1/stream/health", None);
    let initial_health = tokio::time::timeout(Duration::from_secs(3), health.next())
        .await?
        .ok_or("health stream closed before its first observation")??;
    assert!(matches!(
        initial_health.observation,
        Observation::DaemonHealth(_)
    ));
    write_secret(
        &directory.path().join("controller.token"),
        "rotated-controller-token",
    )?;
    let closing = tokio::time::timeout(Duration::from_secs(3), health.next())
        .await?
        .ok_or("health stream closed without a rotation observation")??;
    assert!(matches!(
        closing.observation,
        Observation::StreamClosing { .. }
    ));
    assert!(
        matches!(daemon.client.health().await, Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthenticated)
    );
    assert!(
        client(&daemon.endpoint, "rotated-controller-token")?
            .readiness()
            .await?
            .ready
    );
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_graceful_shutdown_and_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    daemon.stop().await?;
    let restarted = start(config, CONTROLLER_TOKEN).await?;
    assert!(restarted.client.readiness().await?.ready);
    restarted.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_configured_process_adapter_executes_to_terminal() -> TestResult {
    let directory = tempfile::tempdir()?;
    let profile = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../adapters/local-process/tests/fixtures/process-profile-v1.json")
        .canonicalize()?;
    let config = configuration_with_process_profiles(&directory, 16, vec![profile])?;
    let (artifact_id, artifact_bytes) =
        publish_restricted_test_artifact(&config.document.data_root)?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    assert!(
        daemon
            .client
            .capabilities()
            .await?
            .iter()
            .any(|capability| capability.capability_id == "golden-local-process")
    );
    let imported = daemon
        .client
        .submit(&request(
            "process-import",
            None,
            Command::ImportBlueprint {
                document: process_blueprint()?,
            },
        ))
        .await?;
    let revision = imported
        .value
        .get("revision_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("process import response omitted revision identity")?;
    daemon
        .client
        .submit(&request(
            "process-start",
            None,
            Command::StartRun {
                run_id: "run-process".to_owned(),
                workflow_id: "daemon-process".to_owned(),
                revision_id: revision.to_owned(),
            },
        ))
        .await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let run = daemon.client.run("run-process").await?;
        if run.lifecycle == "terminal" {
            assert_eq!(run.terminal.as_deref(), Some("succeeded"));
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let health = daemon.client.health().await?;
            let capabilities = daemon.client.capabilities().await?;
            let timeline = daemon
                .client
                .timeline(
                    "run-process",
                    &PageRequest {
                        cursor: None,
                        limit: 100,
                    },
                )
                .await?;
            return Err(format!(
                "configured process invocation did not reach terminal state: run={run:?}, health={health:?}, capabilities={capabilities:?}, timeline={:?}",
                timeline.items
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let attempt_id = daemon
        .client
        .timeline(
            "run-process",
            &PageRequest {
                cursor: None,
                limit: 100,
            },
        )
        .await?
        .items
        .into_iter()
        .find(|entry| entry.node_id.as_deref() == Some("process") && entry.attempt_id.is_some())
        .and_then(|entry| entry.attempt_id)
        .ok_or("process run omitted its exact attempt")?;
    let attempt = daemon.client.attempt("run-process", &attempt_id).await?;
    assert_eq!(attempt.context_access, "authorized");
    assert_eq!(
        attempt.capability_id.as_deref(),
        Some("golden-local-process")
    );
    assert_eq!(attempt.descriptor_revision, Some(1));
    let manifest_metadata = attempt
        .context_manifest
        .as_ref()
        .ok_or("process attempt omitted context manifest metadata")?;
    assert_eq!(
        manifest_metadata.content_type,
        "application/vnd.milkdrift.context-manifest.v2+json"
    );
    let inspected_context = attempt
        .context
        .clone()
        .ok_or("authorized process attempt omitted context details")?;
    assert_eq!(inspected_context.schema_version, 2);
    assert!(inspected_context.digest.starts_with("b3_"));
    assert!(inspected_context.entries.is_empty());
    assert!(!inspected_context.truncated);
    let metadata = daemon.client.artifact_metadata(&artifact_id).await?;
    assert_eq!(metadata.artifact_id, artifact_id);
    let range = daemon
        .client
        .artifact_range(&artifact_id, 0, metadata.size.saturating_sub(1))
        .await?;
    assert_eq!(range.bytes, artifact_bytes);
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(matches!(
        observer.artifact_metadata(&artifact_id).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    assert!(matches!(
        observer.artifact_range(&artifact_id, 0, 0).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Unauthorized
    ));
    let sidecar: serde_json::Value = serde_json::from_slice(&fs::read(
        directory.path().join("data/control-state-v1.json"),
    )?)?;
    let audit = sidecar["audit"]
        .as_array()
        .ok_or("control sidecar omitted the bounded security audit")?;
    assert!(audit.iter().any(|entry| {
        entry["operation"] == serde_json::json!("read_artifact_content")
            && entry["grant_revision"] == serde_json::json!(1)
            && entry["decision_digest"]
                .as_str()
                .is_some_and(|digest| digest.starts_with("b3_"))
    }));
    daemon.stop().await?;
    let restarted = start(config, CONTROLLER_TOKEN).await?;
    let reopened = restarted.client.attempt("run-process", &attempt_id).await?;
    assert_eq!(reopened.context_access, "authorized");
    assert_eq!(reopened.context, Some(inspected_context));
    assert_eq!(reopened.context_manifest, attempt.context_manifest);
    restarted.stop().await
}

#[test]
fn daemon_startup_corruption_refuses_command_admission() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration(&directory, 16)?;
    fs::create_dir_all(&config.document.data_root)?;
    fs::write(
        config.document.data_root.join("control-state-v1.json"),
        br#"{"schema_version":1,"layouts":{},"commands":{"broken":true}}"#,
    )?;
    let result = DaemonHost::start(config);
    assert!(result.is_err());
    Ok(())
}

#[test]
fn page_requests_remain_explicitly_bounded() -> TestResult {
    PageRequest {
        cursor: None,
        limit: 100,
    }
    .validate()?;
    Ok(())
}
