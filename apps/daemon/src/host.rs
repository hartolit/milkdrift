use std::{
    collections::BTreeMap, collections::BTreeSet, sync::Arc, sync::Mutex, sync::Weak,
    sync::atomic::AtomicBool, sync::atomic::Ordering, sync::mpsc::SyncSender, thread::JoinHandle,
    time::Duration,
};

use milkdrift_authority::GrantSetEvaluator;
use milkdrift_capability::PeerId;
use milkdrift_capability_host::{
    CapabilityHost, CorePeerArtifactStore, EffectWorkerHost, PeerService,
};
use milkdrift_control_protocol::{ArtifactMetadataRead, ErrorCode, HealthRead};
use milkdrift_peer_http::PeerRegistry;
use milkdrift_persistence::TimestampMillis;
use milkdrift_redb_store::RedbStore;
use milkdrift_workspace::WorkspaceBudget;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{auth::ActorSession, auth::AuthRegistry, config::ShutdownConfig};

mod artifacts;
mod authorization;
mod capabilities;
mod clock;
mod health;
mod maintenance;
mod managed;
mod peer_store;
mod peers;
mod queue;
mod read_model;
mod requests;
mod shutdown;
mod startup;
mod storage_ports;
mod tasks;
mod uploads;
pub(crate) use tasks::TaskRequest;

use clock::DurableClock;
use health::SharedHealth;
use peers::{PeerRuntime, build_peer_runtime};
use queue::OwnerRequest;
use read_model::{clock_unavailable, internal};

const OWNER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
const LEGACY_SIDECAR_FILE: &str = "control-state-v1.json";

/// Daemon construction, owner-thread, or orderly-shutdown failure.
#[derive(Debug, Error)]
pub enum HostError {
    /// Configuration/authentication setup failed before storage opened.
    #[error("daemon configuration failed: {0}")]
    Configuration(String),
    /// Runtime owner could not initialize/recover.
    #[error("daemon startup failed: {0}")]
    Startup(String),
    /// Ordered shutdown did not complete successfully.
    #[error("daemon shutdown failed: {0}")]
    Shutdown(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublicFailure {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub details: BTreeMap<String, String>,
}

impl PublicFailure {
    fn new(code: ErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        let mut message = message.into();
        let boundary = milkdrift_contracts::truncate_utf8(&message, 4_096).len();
        message.truncate(boundary);
        Self {
            code,
            message,
            retryable,
            details: BTreeMap::new(),
        }
    }
}

/// Handle to a started host, shared by HTTP routes while one thread owns durable operations.
///
/// Cloning shares the same queues, workers, and storage lifecycle. Arrange one orderly
/// [`Self::shutdown`] (or use [`crate::serve`]) before releasing the host; socket closure alone
/// does not finish external work or its final persistence calls.
#[derive(Clone)]
pub struct DaemonHost {
    browser_origins: Arc<Vec<String>>,
    sender: Arc<SyncSender<OwnerRequest>>,
    health: Arc<SharedHealth>,
    auth: AuthRegistry,
    mutating_admission: Arc<AtomicBool>,
    join: Arc<Mutex<Option<JoinHandle<()>>>>,
    shutdown_deadline: Duration,
    peer_service: Option<Arc<PeerService>>,
    peer_registries: Arc<BTreeMap<PeerId, Arc<PeerRegistry>>>,
    revoked_peers: Arc<Mutex<BTreeSet<PeerId>>>,
    clock: DurableClock,
    managed: Option<Weak<managed::ManagedHost>>,
}

impl std::fmt::Debug for DaemonHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DaemonHost")
            .field("health", &self.health())
            .field("authentication", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl DaemonHost {
    pub(crate) fn browser_origins(&self) -> Arc<Vec<String>> {
        self.browser_origins.clone()
    }
    pub(crate) async fn now(&self) -> Result<u64, PublicFailure> {
        let clock = self.clock.clone();
        tokio::task::spawn_blocking(move || clock.now())
            .await
            .map_err(|_| clock_unavailable())?
            .map(TimestampMillis::get)
            .map_err(|_| clock_unavailable())
    }

    /// Returns current bounded liveness/readiness state without entering runtime storage.
    #[must_use]
    pub(crate) fn health(&self) -> HealthRead {
        self.health.read()
    }

    /// Coherent health snapshot and monotonic feed generation; neither is durable truth.
    #[must_use]
    pub(crate) fn health_snapshot(&self) -> (u64, HealthRead) {
        self.health.snapshot()
    }

    pub(crate) fn authenticate_header(&self, value: Option<&str>) -> Option<ActorSession> {
        let value = value?.strip_prefix("Bearer ")?;
        self.auth.authenticate(value.as_bytes())
    }

    pub(crate) fn accepting_mutations(&self) -> bool {
        self.mutating_admission.load(Ordering::SeqCst)
    }
}

pub(crate) enum StreamAuthority {
    Capabilities,
    Health,
}

pub(crate) struct ArtifactContentRead {
    pub(crate) metadata: ArtifactMetadataRead,
    pub(crate) offset: u64,
    pub(crate) bytes: Vec<u8>,
    pub(crate) end: bool,
}

struct Owner {
    host_id: PeerId,
    input_budget: WorkspaceBudget,
    recovery_controls: bool,
    request_panicked: bool,
    shutdown: ShutdownConfig,
    store: Arc<RedbStore>,
    workflow: Option<WorkflowServices>,
    capability_host: CapabilityHost,
    authority: Arc<GrantSetEvaluator>,
    effect_workers: Option<EffectWorkerHost>,
    peer_service: Option<Weak<PeerService>>,
    // Strong lifecycle lease; service-facing artifact adapters retain only a weak handle.
    _peer_artifacts: Option<Arc<CorePeerArtifactStore>>,
    // Idle HTTP handles must not retain the managed store after the owner is joined.
    _managed: Option<Arc<managed::ManagedHost>>,
    peer_registries: BTreeMap<PeerId, Arc<PeerRegistry>>,
    clock: DurableClock,
}

struct WorkflowServices {
    tasks: Arc<milkdrift_runtime::TaskRuntime>,
    task_control: milkdrift_control::TaskControlService,
}

impl Owner {
    fn workflow(&self) -> Result<&WorkflowServices, PublicFailure> {
        self.workflow.as_ref().ok_or_else(|| PublicFailure::new(
            ErrorCode::Unavailable,
            "this host has the execution_only role; workflow operations require workflow_enabled; retained history can be inspected with storage-admin",
            false,
        ))
    }
    fn now(&self) -> Result<u64, PublicFailure> {
        self.clock
            .now()
            .map(TimestampMillis::get)
            .map_err(|_| clock_unavailable())
    }
}

#[cfg(test)]
mod tests;
