use std::{
    collections::BTreeMap,
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    time::{Duration, Instant},
};

use futures_util::StreamExt as _;
use milkdrift_control_client::{BearerCredential, ClientConfig, ClientError, ControlClient};
use milkdrift_control_protocol::ErrorCode;
use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, DaemonConfig, DaemonHost, PeerHostConfig, RuntimeHostConfig,
    SecretSourceConfig, ShutdownConfig, ValidatedDaemonConfig, serve,
};
use serde::Serialize;
use tokio::{sync::oneshot, task::JoinHandle};
use url::Url;

use crate::{EvidenceResult, LatencySummary, ScenarioMeasurement};

const TOKEN: &str = "operational-evidence-controller-token";

/// Machine-readable daemon load, overload, recovery, stream, and shutdown evidence.
#[derive(Clone, Debug, Serialize)]
pub struct DaemonEvidence {
    /// Fixed owner queue capacity verified through the health contract.
    pub queue_capacity: u32,
    /// Requests accepted under the saturated phase.
    pub accepted: u64,
    /// Requests rejected with the stable overload status.
    pub overloaded: u64,
    /// End-to-end request latencies in microseconds.
    pub latency: LatencySummary,
    /// A deliberately slow health-stream consumer received an observation.
    pub slow_consumer_observed: bool,
    /// A cursor-bound reconnect completed when the daemon closed the stream for shutdown.
    pub stream_reconnected: bool,
    /// A low-load request succeeded after overload.
    pub recovered: bool,
    /// Process task/thread count before the load phase, when observable.
    pub tasks_before: Option<u64>,
    /// Process task/thread count after recovery, when observable.
    pub tasks_after: Option<u64>,
    /// Graceful shutdown completed within the configured deadline.
    pub graceful_shutdown: bool,
}

struct RunningDaemon {
    client: ControlClient,
    stop: oneshot::Sender<()>,
    task: JoinHandle<Result<(), milkdrift_daemon::HostError>>,
}

/// Exercises one authenticated daemon owner request over loopback.
pub fn daemon_owner_round_trip() -> EvidenceResult<ScenarioMeasurement> {
    runtime()?.block_on(async {
        let directory = tempfile::tempdir()?;
        let running = start(configuration(&directory, 4)?).await?;
        let started = Instant::now();
        let health = running.client.health().await?;
        let elapsed = started.elapsed();
        if !health.ready || health.request_queue_capacity != 4 {
            return Err(std::io::Error::other("daemon health contract changed").into());
        }
        stop(running).await?;
        let encoded = serde_json::to_vec(&(health, elapsed.as_micros()))?;
        Ok(ScenarioMeasurement::new(
            "daemon/authenticated_owner_health_round_trip",
            1,
            u64::try_from(encoded.len())?,
            &encoded,
        ))
    })
}

/// Runs low, medium, saturated, slow-consumer, recovery, and graceful-shutdown phases.
pub fn measure_daemon_saturation(operations: u32) -> EvidenceResult<DaemonEvidence> {
    if operations < 64 {
        return Err(std::io::Error::other("daemon evidence needs at least 64 requests").into());
    }
    runtime()?.block_on(async move {
        let directory = tempfile::tempdir()?;
        let running = start(configuration(&directory, 1)?).await?;
        let initial = running.client.health().await?;
        if initial.request_queue_capacity != 1 || !initial.ready {
            return Err(std::io::Error::other("daemon queue bound is not observable").into());
        }

        for _ in 0..16 {
            if !running.client.health().await?.ready {
                return Err(std::io::Error::other("medium-load health request was not ready").into());
            }
        }

        let mut slow_stream = running.client.subscribe("v1/stream/health", None);
        let first_observation = tokio::time::timeout(Duration::from_secs(3), slow_stream.next())
            .await
            .ok()
            .flatten()
            .transpose()?;
        let slow_consumer_observed = first_observation.is_some();
        let resume_cursor = first_observation.map(|observation| observation.cursor);
        tokio::time::sleep(Duration::from_millis(750)).await;

        let tasks_before = process_task_count()?;
        let mut joins = tokio::task::JoinSet::new();
        for _ in 0..operations {
            let client = running.client.clone();
            joins.spawn(async move {
                let started = Instant::now();
                let result = client.health().await;
                (started.elapsed(), result)
            });
        }
        let mut accepted = 0_u64;
        let mut overloaded = 0_u64;
        let mut latencies = Vec::with_capacity(usize::try_from(operations)?);
        while let Some(joined) = joins.join_next().await {
            let (latency, result) = joined?;
            latencies.push(latency);
            match result {
                Ok(health) if health.ready => accepted = accepted.saturating_add(1),
                Err(ClientError::Api(error)) if error.code == ErrorCode::Overload => {
                    overloaded = overloaded.saturating_add(1);
                }
                Ok(_) => return Err(std::io::Error::other("daemon became non-ready under load").into()),
                Err(error) => return Err(error.into()),
            }
        }
        if accepted.saturating_add(overloaded) != u64::from(operations) || overloaded == 0 {
            return Err(std::io::Error::other("saturated phase did not prove bounded overload").into());
        }

        drop(slow_stream);
        let mut resumed = running.client.subscribe("v1/stream/health", resume_cursor);
        let reconnect_task = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(3), resumed.next()).await
        });
        let recovered = running.client.health().await?.ready;
        let tasks_after = process_task_count()?;
        if let (Some(before), Some(after)) = (tasks_before, tasks_after)
            && after > before.saturating_add(8)
        {
            return Err(std::io::Error::other("daemon load left unbounded process tasks").into());
        }
        let latency = LatencySummary::from_durations(latencies)?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let RunningDaemon { stop, task, .. } = running;
        let _ = stop.send(());
        let stream_reconnected = reconnect_task
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
            .is_some();
        let graceful_shutdown = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .is_ok_and(|joined| joined.is_ok_and(|result| result.is_ok()));
        if !graceful_shutdown || !recovered || !slow_consumer_observed || !stream_reconnected {
            return Err(std::io::Error::other(format!(
                "daemon evidence failed: recovered={recovered}, slow_consumer={slow_consumer_observed}, reconnected={stream_reconnected}, shutdown={graceful_shutdown}"
            ))
            .into());
        }
        Ok(DaemonEvidence {
            queue_capacity: 1,
            accepted,
            overloaded,
            latency,
            slow_consumer_observed,
            stream_reconnected,
            recovered,
            tasks_before,
            tasks_after,
            graceful_shutdown,
        })
    })
}

fn runtime() -> EvidenceResult<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?)
}

async fn start(config: ValidatedDaemonConfig) -> EvidenceResult<RunningDaemon> {
    let host = DaemonHost::start(config)?;
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    let endpoint = Url::parse(&format!("http://{address}/"))?;
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(serve(listener, host, async move {
        let _ = stopped.await;
    }));
    let mut client_config = ClientConfig::new(endpoint);
    client_config.safe_query_retries = 0;
    client_config.retry_delay = Duration::from_millis(25);
    client_config.request_timeout = Duration::from_secs(10);
    let client = ControlClient::new(client_config, BearerCredential::new(TOKEN)?)?;
    if !client.readiness().await?.ready {
        return Err(std::io::Error::other("daemon did not become ready").into());
    }
    Ok(RunningDaemon { client, stop, task })
}

async fn stop(running: RunningDaemon) -> EvidenceResult {
    let _ = running.stop.send(());
    tokio::time::timeout(Duration::from_secs(10), running.task).await???;
    Ok(())
}

fn configuration(
    directory: &tempfile::TempDir,
    request_queue: u32,
) -> EvidenceResult<ValidatedDaemonConfig> {
    let token_path = directory.path().join("controller.token");
    write_secret(&token_path, TOKEN)?;
    let config = DaemonConfig {
        schema_version: 7,
        data_root: directory.path().join("data"),
        bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        secret_sources: BTreeMap::from([(
            "credential:evidence".to_owned(),
            SecretSourceConfig::File { path: token_path },
        )]),
        actors: vec![ActorBindingConfig {
            credential_ref: "credential:evidence".to_owned(),
            actor: "system:operational-evidence".to_owned(),
            grant_id: "grant:operational-evidence".to_owned(),
            grant_revision: 1,
            revocation_generation: 0,
            preset: AuthorityPresetConfig::Controller,
            authority: ActorGrantConfig::dangerous_administrator(),
            enabled: true,
        }],
        runtime: RuntimeHostConfig {
            request_queue,
            maintenance_interval_ms: 10,
            ..RuntimeHostConfig::default()
        },
        adapters: AdapterConfig::default(),
        peers: PeerHostConfig::default(),
        shutdown: ShutdownConfig::default(),
        application_receipts: ApplicationReceiptConfig {
            hot_receipt_bound: 128,
            archive_batch_size: 16,
        },
        security_audit_record_bound: 256,
    };
    Ok(config.validate(directory.path())?)
}

fn write_secret(path: &Path, value: &str) -> EvidenceResult {
    fs::write(path, value)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn process_task_count() -> EvidenceResult<Option<u64>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Some(u64::try_from(
            fs::read_dir("/proc/self/task")?.count(),
        )?))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(None)
    }
}
