use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use crate::application::{
    DaemonLaunch, OwnedChild, application_binary, reserve_endpoint, write_private,
};
use futures_util::StreamExt as _;
use milkdrift_control_client::{ClientError, ControlClient};
use milkdrift_control_protocol::{ErrorCode, Observation};
use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, DaemonConfig, PeerHostConfig, RuntimeHostConfig, SecretSourceConfig,
    ShutdownConfig,
};
use serde::Serialize;

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
    /// A cursor-bound reconnect received a fresh authenticated health observation.
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
    process: OwnedChild,
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
        let slow_consumer_observed = first_observation.as_ref().is_some_and(|item| {
            item.feed == "daemon-health"
                && matches!(&item.observation, Observation::DaemonHealth(health) if health.ready)
        });
        let resume_cursor = first_observation.map(|observation| observation.cursor);
        tokio::time::sleep(Duration::from_millis(750)).await;

        let tasks_before = process_task_count(running.process.id())?;
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
        // A reconnect authorizes through the same single-slot owner queue. Finish
        // the low-load recovery read before introducing another concurrent caller.
        let recovery_deadline = Instant::now() + Duration::from_secs(3);
        let recovered = loop {
            match running.client.health().await {
                Ok(health) => break health.ready,
                Err(ClientError::Api(error))
                    if error.code == ErrorCode::Overload && Instant::now() < recovery_deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error.into()),
            }
        };
        let tasks_after = process_task_count(running.process.id())?;
        if let (Some(before), Some(after)) = (tasks_before, tasks_after)
            && after > before.saturating_add(8)
        {
            return Err(std::io::Error::other("daemon load left unbounded process tasks").into());
        }
        let latency = LatencySummary::from_durations(latencies)?;
        let mut resumed = running
            .client
            .subscribe("v1/stream/health", resume_cursor.clone());
        let observation = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match resumed.next().await {
                    Some(Ok(observation)) => return EvidenceResult::Ok(observation),
                    Some(Err(error)) if error.retryable() => continue,
                    Some(Err(error)) => return Err(error.into()),
                    None => return Err("reconnected health stream ended without an observation".into()),
                }
            }
        })
        .await??;
        let stream_reconnected = Some(&observation.cursor) != resume_cursor.as_ref()
            && observation.feed == "daemon-health"
            && matches!(observation.observation, Observation::DaemonHealth(health) if health.ready);
        drop(resumed);
        let RunningDaemon { mut process, .. } = running;
        let graceful_shutdown = process.shutdown().is_ok();
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

async fn start(config: DaemonLaunch) -> EvidenceResult<RunningDaemon> {
    let (process, client) = config.start(TOKEN).await?;
    Ok(RunningDaemon { client, process })
}

async fn stop(mut running: RunningDaemon) -> EvidenceResult {
    running.process.shutdown()
}

fn configuration(
    directory: &tempfile::TempDir,
    request_queue: u32,
) -> EvidenceResult<DaemonLaunch> {
    let token_path = directory.path().join("controller.token");
    write_private(&token_path, TOKEN.as_bytes())?;
    let config = DaemonConfig {
        schema_version: milkdrift_daemon::DAEMON_CONFIG_SCHEMA_VERSION,
        data_root: directory.path().join("data"),
        bind: reserve_endpoint()?,
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
    DaemonLaunch::write(
        application_binary("milkdrift-daemon")?,
        directory.path(),
        &config,
    )
}

fn process_task_count(process_id: u32) -> EvidenceResult<Option<u64>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Some(u64::try_from(
            std::fs::read_dir(format!("/proc/{process_id}/task"))?.count(),
        )?))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = process_id;
        Ok(None)
    }
}
