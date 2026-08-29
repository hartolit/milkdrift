use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Write as _,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityEvaluator,
    AuthorityExecutionProvenance, AuthorityOperation, AuthorityRequest, BoundaryTimeMillis,
    DecisionId, GrantSetEvaluator, LayoutOwner, PeerId, PolicyId, RequestedResourceFacts,
    SecretRef, WorkflowRunScope,
};
use milkdrift_blueprint::{BlueprintRevisionDocument, RevisionId, WorkflowId};
use milkdrift_capability::{CapabilityId, ErrorClass, SideEffectClass};
use milkdrift_capability_host::{
    AdapterInvocation, CapabilityHost, CapabilitySelectionPolicy, EffectShutdownMode,
    EffectWorkerConfig, EffectWorkerHost, HostConfig, InvocationDataAccess, MaterializationLimits,
    SecretResolver, StoreInvocationDataAccess,
};
use milkdrift_control::{
    AuthorityContextRef, AuthorityContextResolver, ControlCommand, ControlCommandDocument,
    ControlError, ControlId, ControlResult, ControlResultSink, ControlService, OptimisticGuard,
    ProposalDigest, ProposalId, WorkflowControlAdapter, WorkflowProposalDocument,
    workflow_control_descriptor,
};
use milkdrift_control_protocol::{
    ArtifactMetadataRead, AttemptRead, CapabilityRead, Command, CommandAccepted, CommandRequest,
    Cursor, CursorBinding, DaemonState, ErrorCode, HealthRead, LayoutDocument, NodeRead, Page,
    PeerRead, ProposalDecision, ProposalRead, ResolveAction, RevisionChange, RevisionDiffRead,
    RevisionRead, RevisionSummary as PublicRevisionSummary, RunRead, TimelineCategory,
    TimelineEntry,
};
use milkdrift_local_process::{LocalProcessAdapter, ProcessProfileDocument};
use milkdrift_model_provider::{EndpointProfile, ModelEndpointAdapter, descriptor_for_profile};
use milkdrift_peer_http::{
    FilePeerArtifactStore, FilePeerExecutionStore, InsecureLoopbackMode, PeerAuthenticator,
    PeerClientConfig, PeerCredentialSource, PeerHttpClient, PeerHttpError, PeerRegistry,
    PeerRelationship, PeerServerConfig, PeerService, SystemPeerClock,
};
use milkdrift_peer_protocol::{
    DelegationRef, ExecutionLimits, HardLimits, HeartbeatLease, PeerAuthority, ProtocolVersion,
    ProtocolVersionRange, SessionId,
};
use milkdrift_persistence::{
    ArtifactReadAuthority, ArtifactReadRequest, ArtifactStore, AttemptId, CorrelationKey,
    EvidenceId, EvidenceKind, EvidenceReference, IndexedRunState, PageSize, PersistenceError,
    Reason, ReconciliationDecisionId, RevisionCursor, RevisionFilter, RevisionPageQuery,
    RevisionStore, RunQueryStore, RunSequence, RunSummaryCursor, RunSummaryFilter,
    RunSummaryPageQuery, SignalDeliveryMode, SignalId, SignalTypeId, TimestampMillis, WorkerId,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    ExternalWorkAction, RetryPolicy, RuntimeConfig, RuntimeService, SchedulerLimits,
    SequentialIdGenerator, SystemBoundaryClock,
};
use milkdrift_workspace::ArtifactSensitivity;
use milkdrift_workspace::{ArtifactId, RunId, ScopeId, WorkspaceBudget, WorkspaceScope};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use subtle::ConstantTimeEq as _;
use thiserror::Error;
use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::{
    auth::{ActorSession, AuthRegistry, ConfiguredSecretResolver},
    config::{PeerSideEffectConfig, ShutdownEffectPolicy, ValidatedDaemonConfig},
};

const LOCAL_STATE_SCHEMA_VERSION: u32 = 2;
const OWNER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// Daemon construction, owner-thread, or orderly-shutdown failure.
#[derive(Debug, Error)]
pub enum HostError {
    /// Configuration/authentication setup failed before storage opened.
    #[error("daemon configuration failed: {0}")]
    Configuration(String),
    /// Runtime owner could not initialize/recover.
    #[error("daemon startup failed: {0}")]
    Startup(String),
    /// The bounded owner queue is full or disconnected.
    #[error("daemon runtime owner is unavailable")]
    OwnerUnavailable,
    /// The runtime owner did not respond within its bounded deadline.
    #[error("daemon runtime owner response deadline elapsed")]
    OwnerTimeout,
    /// Ordered shutdown did not complete successfully.
    #[error("daemon shutdown failed: {0}")]
    Shutdown(String),
}

#[derive(Clone, Debug)]
pub(crate) struct PublicFailure {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub details: BTreeMap<String, String>,
}

impl PublicFailure {
    fn new(code: ErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        let mut message = message.into();
        if message.len() > 4_096 {
            let mut end = 4_096;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
        }
        Self {
            code,
            message,
            retryable,
            details: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum Lifecycle {
    Starting = 0,
    Ready = 1,
    Draining = 2,
    Stopped = 3,
    Failed = 4,
}

impl Lifecycle {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Ready,
            2 => Self::Draining,
            3 => Self::Stopped,
            4 => Self::Failed,
            _ => Self::Starting,
        }
    }

    const fn public(self) -> DaemonState {
        match self {
            Self::Starting => DaemonState::Starting,
            Self::Ready => DaemonState::Ready,
            Self::Draining => DaemonState::Draining,
            Self::Stopped => DaemonState::Stopped,
            Self::Failed => DaemonState::Failed,
        }
    }
}

struct SharedHealth {
    lifecycle: AtomicU8,
    generation: AtomicU64,
    queued: AtomicU32,
    capacity: u32,
    active_effects: AtomicU32,
    last_failure: Mutex<Option<String>>,
}

impl SharedHealth {
    fn set_lifecycle(&self, lifecycle: Lifecycle) {
        self.lifecycle.store(lifecycle as u8, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn failure(&self, message: &str) {
        if let Ok(mut failure) = self.last_failure.lock() {
            *failure = Some(bounded(message));
        }
    }

    fn read(&self) -> HealthRead {
        let lifecycle = Lifecycle::from_u8(self.lifecycle.load(Ordering::SeqCst));
        HealthRead {
            state: lifecycle.public(),
            live: !matches!(lifecycle, Lifecycle::Stopped | Lifecycle::Failed),
            ready: lifecycle == Lifecycle::Ready,
            draining: lifecycle == Lifecycle::Draining,
            queued_requests: self.queued.load(Ordering::SeqCst),
            request_queue_capacity: self.capacity,
            active_effects: self.active_effects.load(Ordering::SeqCst),
            last_failure: self
                .last_failure
                .lock()
                .ok()
                .and_then(|failure| failure.clone()),
        }
    }
}

/// Cloneable daemon handle shared by HTTP route state.
#[derive(Clone)]
pub struct DaemonHost {
    sender: SyncSender<OwnerRequest>,
    health: Arc<SharedHealth>,
    auth: AuthRegistry,
    mutating_admission: Arc<AtomicBool>,
    join: Arc<Mutex<Option<JoinHandle<()>>>>,
    shutdown_deadline: Duration,
    peer_service: Option<Arc<PeerService>>,
    peer_registries: Arc<BTreeMap<PeerId, Arc<PeerRegistry>>>,
    revoked_peers: Arc<Mutex<BTreeSet<PeerId>>>,
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
    /// Starts the dedicated owner, completes recovery/adapters/workers, then returns ready.
    pub fn start(config: ValidatedDaemonConfig) -> Result<Self, HostError> {
        let auth = AuthRegistry::from_config(&config)
            .map_err(|error| HostError::Configuration(error.to_string()))?;
        let queue_capacity = config.document.runtime.request_queue;
        let shutdown_deadline = Duration::from_millis(config.document.shutdown.deadline_ms);
        let queue_size = usize::try_from(queue_capacity)
            .map_err(|_| HostError::Configuration("request queue exceeds platform".to_owned()))?;
        let (sender, receiver) = sync_channel(queue_size);
        let (startup_sender, startup_receiver) = sync_channel(1);
        let health = Arc::new(SharedHealth {
            lifecycle: AtomicU8::new(Lifecycle::Starting as u8),
            generation: AtomicU64::new(1),
            queued: AtomicU32::new(0),
            capacity: queue_capacity,
            active_effects: AtomicU32::new(0),
            last_failure: Mutex::new(None),
        });
        let thread_health = health.clone();
        let thread_auth = auth.clone();
        let maintenance = Duration::from_millis(config.document.runtime.maintenance_interval_ms);
        let join = thread::Builder::new()
            .name("milkdrift-runtime-owner".to_owned())
            .spawn(move || {
                info!(phase = "startup", "runtime owner starting");
                let mut owner = match Owner::open(config, thread_auth, thread_health.clone()) {
                    Ok(owner) => owner,
                    Err(failure) => {
                        warn!(
                            phase = "startup",
                            outcome = "failed",
                            code = "initialization",
                            "runtime owner failed before readiness"
                        );
                        thread_health.failure("daemon startup initialization failed");
                        thread_health.set_lifecycle(Lifecycle::Failed);
                        let _ = startup_sender.send(Err(failure));
                        return;
                    }
                };
                let startup = PeerRuntime {
                    service: owner.peer_service.clone(),
                    registries: owner.peer_registries.clone(),
                };
                thread_health.set_lifecycle(Lifecycle::Ready);
                let _ = startup_sender.send(Ok(startup));
                info!(phase = "ready", "runtime owner ready after recovery");
                owner.run(receiver, maintenance, &thread_health);
            })
            .map_err(|error| HostError::Startup(error.to_string()))?;
        match startup_receiver.recv() {
            Ok(Ok(peer_runtime)) => Ok(Self {
                sender,
                health,
                auth,
                mutating_admission: Arc::new(AtomicBool::new(true)),
                join: Arc::new(Mutex::new(Some(join))),
                shutdown_deadline,
                peer_service: peer_runtime.service,
                peer_registries: Arc::new(peer_runtime.registries),
                revoked_peers: Arc::new(Mutex::new(BTreeSet::new())),
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(HostError::Startup(error))
            }
            Err(_) => {
                let _ = join.join();
                Err(HostError::Startup(
                    "runtime owner ended before startup result".to_owned(),
                ))
            }
        }
    }

    /// Returns current bounded liveness/readiness state without entering runtime storage.
    #[must_use]
    pub(crate) fn health(&self) -> HealthRead {
        self.health.read()
    }

    /// Monotonic in-process health feed generation; not a durable run event.
    #[must_use]
    pub(crate) fn health_generation(&self) -> u64 {
        self.health.generation.load(Ordering::SeqCst)
    }

    pub(crate) fn authenticate_header(&self, value: Option<&str>) -> Option<ActorSession> {
        let value = value?.strip_prefix("Bearer ")?;
        self.auth.authenticate(value.as_bytes())
    }

    pub(crate) fn accepting_mutations(&self) -> bool {
        self.mutating_admission.load(Ordering::SeqCst)
    }

    /// Closes mutation admission synchronously before graceful HTTP shutdown begins.
    pub(crate) fn begin_draining(&self) {
        self.mutating_admission.store(false, Ordering::SeqCst);
        self.health.set_lifecycle(Lifecycle::Draining);
        if let Some(service) = &self.peer_service {
            service.begin_drain();
        }
        for registry in self.peer_registries.values() {
            let _ = registry.disconnect();
        }
    }

    /// Returns the optional distinct peer route service for router composition.
    #[must_use]
    pub fn peer_service(&self) -> Option<Arc<PeerService>> {
        self.peer_service.clone()
    }

    /// Returns stable sorted peer health/catalog observations without secret values.
    #[must_use]
    pub(crate) fn peers(&self) -> Vec<PeerRead> {
        let revoked = self.revoked_peers.lock().ok();
        self.peer_registries
            .values()
            .map(|registry| {
                let status = registry.status();
                let registered_capabilities = registry.registration_count();
                let is_revoked = revoked
                    .as_ref()
                    .is_some_and(|peers| peers.contains(registry.remote_peer()));
                PeerRead {
                    peer_id: registry.remote_peer().as_str().to_owned(),
                    connected: status.connected && !is_revoked,
                    health: if is_revoked {
                        "revoked".to_owned()
                    } else {
                        status.health
                    },
                    session_id: status
                        .remote_session
                        .map(|session| session.as_str().to_owned()),
                    catalog_generation: status.catalog_generation,
                    catalog_digest: status
                        .catalog_digest
                        .map(|digest| digest.as_str().to_owned()),
                    registered_capabilities,
                    catalog_expires_at_unix_ms: status.catalog_expires_at_unix_ms,
                    revoked: is_revoked,
                }
            })
            .collect()
    }

    /// Manually authenticates and refreshes one configured remote peer catalog.
    pub(crate) async fn connect_peer(&self, peer: &PeerId) -> Result<PeerRead, HostError> {
        if self
            .revoked_peers
            .lock()
            .map_err(|_| HostError::Configuration("peer revocation state unavailable".to_owned()))?
            .contains(peer)
        {
            return Err(HostError::Configuration(
                "peer is revoked until daemon configuration reload/restart".to_owned(),
            ));
        }
        let registry = self
            .peer_registries
            .get(peer)
            .cloned()
            .ok_or_else(|| HostError::Configuration("peer is not configured".to_owned()))?;
        tokio::task::spawn_blocking(move || registry.connect())
            .await
            .map_err(|_| HostError::Startup("peer connector task failed".to_owned()))?
            .map_err(|error| HostError::Startup(error.to_string()))?;
        self.peers()
            .into_iter()
            .find(|status| status.peer_id == peer.as_str())
            .ok_or_else(|| HostError::Startup("peer status disappeared".to_owned()))
    }

    /// Explicitly disconnects and drains one peer's local adapter registrations.
    pub(crate) async fn disconnect_peer(&self, peer: &PeerId) -> Result<PeerRead, HostError> {
        let registry = self
            .peer_registries
            .get(peer)
            .cloned()
            .ok_or_else(|| HostError::Configuration("peer is not configured".to_owned()))?;
        tokio::task::spawn_blocking(move || registry.disconnect())
            .await
            .map_err(|_| HostError::Shutdown("peer disconnect task failed".to_owned()))?
            .map_err(|error| HostError::Shutdown(error.to_string()))?;
        self.peers()
            .into_iter()
            .find(|status| status.peer_id == peer.as_str())
            .ok_or_else(|| HostError::Shutdown("peer status disappeared".to_owned()))
    }

    /// Revokes one live relationship and drains its registrations until reload/restart.
    pub(crate) async fn revoke_peer(&self, peer: &PeerId) -> Result<PeerRead, HostError> {
        self.revoked_peers
            .lock()
            .map_err(|_| HostError::Configuration("peer revocation state unavailable".to_owned()))?
            .insert(peer.clone());
        if let Some(service) = &self.peer_service {
            service
                .revoke_peer(peer)
                .map_err(|error| HostError::Configuration(error.to_string()))?;
        }
        self.disconnect_peer(peer).await
    }

    /// Runs ordered shutdown and joins the owner thread.
    pub(crate) async fn shutdown(&self) -> Result<(), HostError> {
        self.begin_draining();
        let result = self
            .request(OwnerOperation::Shutdown, true)
            .await
            .map_err(|error| HostError::Shutdown(error.message))?;
        let join = self
            .join
            .lock()
            .map_err(|_| HostError::Shutdown("owner join state is unavailable".to_owned()))?
            .take();
        if let Some(join) = join {
            tokio::task::spawn_blocking(move || join.join())
                .await
                .map_err(|_| HostError::Shutdown("owner join task failed".to_owned()))?
                .map_err(|_| HostError::Shutdown("runtime owner panicked".to_owned()))?;
        }
        match result {
            OwnerValue::Shutdown { clean: true } => Ok(()),
            OwnerValue::Shutdown { clean: false } => Err(HostError::Shutdown(
                "effect workers exceeded shutdown deadline".to_owned(),
            )),
            _ => Err(HostError::Shutdown(
                "runtime owner returned an invalid shutdown result".to_owned(),
            )),
        }
    }

    pub(crate) async fn request(
        &self,
        operation: OwnerOperation,
        shutdown: bool,
    ) -> Result<OwnerValue, PublicFailure> {
        if !shutdown
            && Lifecycle::from_u8(self.health.lifecycle.load(Ordering::SeqCst)) != Lifecycle::Ready
        {
            return Err(PublicFailure::new(
                ErrorCode::Unavailable,
                "daemon is not ready",
                true,
            ));
        }
        let started = tokio::time::Instant::now();
        let (reply, receiver) = oneshot::channel();
        let mut pending = OwnerRequest { operation, reply };
        loop {
            match self.sender.try_send(pending) {
                Ok(()) => {
                    self.health.queued.fetch_add(1, Ordering::SeqCst);
                    break;
                }
                Err(TrySendError::Full(returned)) if shutdown => {
                    if started.elapsed() >= self.shutdown_deadline {
                        return Err(PublicFailure::new(
                            ErrorCode::Timeout,
                            "runtime owner shutdown could not enter the bounded queue before its deadline",
                            true,
                        ));
                    }
                    pending = returned;
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Err(TrySendError::Full(_)) => {
                    return Err(PublicFailure::new(
                        ErrorCode::Overload,
                        "runtime owner request queue is full",
                        true,
                    ));
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(PublicFailure::new(
                        ErrorCode::Unavailable,
                        "runtime owner is unavailable",
                        true,
                    ));
                }
            }
        }
        let response_timeout = if shutdown {
            self.shutdown_deadline.saturating_sub(started.elapsed())
        } else {
            OWNER_RESPONSE_TIMEOUT
        };
        if response_timeout.is_zero() {
            return Err(PublicFailure::new(
                ErrorCode::Timeout,
                "runtime owner shutdown response deadline elapsed",
                true,
            ));
        }
        match tokio::time::timeout(response_timeout, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(PublicFailure::new(
                ErrorCode::Unavailable,
                "runtime owner response channel closed",
                true,
            )),
            Err(_) => Err(PublicFailure::new(
                ErrorCode::Timeout,
                "runtime owner response deadline elapsed",
                true,
            )),
        }
    }
}

pub(crate) enum OwnerOperation {
    Version {
        session: ActorSession,
    },
    Health {
        session: ActorSession,
    },
    Readiness {
        session: ActorSession,
    },
    Authority {
        session: ActorSession,
    },
    Peers {
        session: ActorSession,
    },
    Peer {
        session: ActorSession,
        peer: String,
    },
    AdministerPeer {
        session: ActorSession,
        peer: String,
    },
    StreamAuthority {
        session: ActorSession,
        stream: StreamAuthority,
    },
    Command {
        session: ActorSession,
        request: Box<CommandRequest>,
    },
    Revision {
        session: ActorSession,
        revision: String,
    },
    Revisions {
        session: ActorSession,
        workflow: Option<String>,
        cursor: Option<Cursor>,
        limit: u32,
    },
    RevisionDiff {
        session: ActorSession,
        from: String,
        to: String,
    },
    Run {
        session: ActorSession,
        run: String,
    },
    Node {
        session: ActorSession,
        run: String,
        execution: String,
    },
    Attempt {
        session: ActorSession,
        run: String,
        attempt: String,
    },
    Runs {
        session: ActorSession,
        state: Option<String>,
        workflow: Option<String>,
        cursor: Option<Cursor>,
        limit: u32,
    },
    Timeline {
        session: ActorSession,
        run: String,
        cursor: Option<Cursor>,
        limit: u32,
    },
    Proposals {
        session: ActorSession,
        run: String,
        cursor: Option<Cursor>,
        limit: u32,
    },
    Proposal {
        session: ActorSession,
        run: String,
        proposal: String,
        revision: String,
    },
    Capabilities {
        session: ActorSession,
    },
    ArtifactMetadata {
        session: ActorSession,
        artifact: String,
    },
    ArtifactRange {
        session: ActorSession,
        artifact: String,
        offset: u64,
        maximum: u32,
        evidence: String,
    },
    Layout {
        session: ActorSession,
        workflow: String,
        revision: String,
    },
    Shutdown,
}

pub(crate) enum OwnerValue {
    Authorized(String),
    Authority(milkdrift_control_protocol::AuthorityRead),
    PeerIds(BTreeSet<String>),
    Command(CommandAccepted),
    Revision(RevisionRead),
    Revisions(Page<PublicRevisionSummary>),
    RevisionDiff(RevisionDiffRead),
    Run(RunRead),
    Node(NodeRead),
    Attempt(AttemptRead),
    Runs(Page<RunRead>),
    Timeline(Page<TimelineEntry>),
    Proposals(Page<ProposalRead>),
    Proposal(ProposalRead),
    Capabilities(Vec<CapabilityRead>),
    ArtifactMetadata(ArtifactMetadataRead),
    ArtifactRange {
        metadata: ArtifactMetadataRead,
        offset: u64,
        bytes: Vec<u8>,
        end: bool,
    },
    Layout(LayoutDocument),
    Shutdown {
        clean: bool,
    },
}

pub(crate) enum StreamAuthority {
    Run(String),
    Capabilities,
    Health,
}

struct OwnerRequest {
    operation: OwnerOperation,
    reply: oneshot::Sender<Result<OwnerValue, PublicFailure>>,
}

struct Owner {
    config: ValidatedDaemonConfig,
    store: Arc<RedbStore>,
    runtime: Arc<RuntimeService>,
    control: Arc<ControlService>,
    capability_host: CapabilityHost,
    authority: Arc<GrantSetEvaluator>,
    effect_workers: Option<EffectWorkerHost>,
    persistent: LocalState,
    peer_service: Option<Arc<PeerService>>,
    peer_registries: BTreeMap<PeerId, Arc<PeerRegistry>>,
}

struct PeerRuntime {
    service: Option<Arc<PeerService>>,
    registries: BTreeMap<PeerId, Arc<PeerRegistry>>,
}

impl Owner {
    fn open(
        config: ValidatedDaemonConfig,
        auth: AuthRegistry,
        health: Arc<SharedHealth>,
    ) -> Result<Self, String> {
        fs::create_dir_all(&config.document.data_root)
            .map_err(|error| format!("data root creation failed: {:?}", error.kind()))?;
        let store = Arc::new(
            RedbStore::open(&config.document.data_root).map_err(|error| error.to_string())?,
        );
        let authority = Arc::new(
            GrantSetEvaluator::new(
                PolicyId::new("daemon.authority.v1").map_err(|error| error.to_string())?,
                1,
                auth.grants(),
                auth.revocations(),
            )
            .map_err(|error| error.to_string())?,
        );
        let capability_host = CapabilityHost::new(
            HostConfig {
                max_registrations: 1_024,
                max_generations_per_capability: 16,
                max_concurrent_per_generation: config.document.runtime.global_concurrency,
                observation_stale_after_ms: 60_000,
            },
            CapabilitySelectionPolicy::priorities(BTreeMap::new()),
        )
        .map_err(|error| error.to_string())?;
        let scheduler = SchedulerLimits::new(
            config.document.runtime.global_concurrency,
            config.document.runtime.per_run_concurrency,
            config.document.runtime.per_branch_concurrency,
            config.document.runtime.per_capability_concurrency,
        )
        .map_err(|error| error.to_string())?;
        let retry = RetryPolicy::new(
            3,
            vec![
                ErrorClass::RateLimit,
                ErrorClass::Transport,
                ErrorClass::Provider,
            ],
            250,
            30_000,
            500,
        )
        .map_err(|error| error.to_string())?;
        let runtime_config = RuntimeConfig::new(
            WorkerId::new("daemon-worker").map_err(|error| error.to_string())?,
            ActorRef::new("service:daemon-runtime").map_err(|error| error.to_string())?,
            config.document.runtime.lease_duration_ms,
            config.document.runtime.maximum_tick_items,
            scheduler,
            retry,
        )
        .map_err(|error| error.to_string())?;
        let runtime = Arc::new(
            RuntimeService::open_closed_with_authority(
                store.clone(),
                Arc::new(capability_host.clone()),
                authority.clone(),
                Arc::new(SystemBoundaryClock),
                Arc::new(
                    SequentialIdGenerator::new("daemon", unix_millis())
                        .map_err(|error| error.to_string())?,
                ),
                runtime_config,
            )
            .map_err(|error| error.to_string())?,
        );
        let control = Arc::new(ControlService::new(
            store.clone(),
            runtime.clone(),
            authority.clone(),
        ));
        let data = Arc::new(
            StoreInvocationDataAccess::new(
                store.clone(),
                config.document.data_root.join("execution"),
                ArtifactReadAuthority::Authorized {
                    actor: ActorRef::new("service:daemon-runtime")
                        .map_err(|error| error.to_string())?,
                    evidence: EvidenceId::new("daemon-materialization")
                        .map_err(|error| error.to_string())?,
                },
                default_workspace_budget().map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?,
        );
        register_control(
            &capability_host,
            control.clone(),
            auth.contexts(),
            data.clone(),
        )?;
        register_configured_adapters(&config, &capability_host, data, auth.resolver())?;
        let peer_runtime = build_peer_runtime(&config, &capability_host, auth.resolver())?;
        runtime
            .initialize_startup()
            .map_err(|error| error.to_string())?;
        let effect_workers = EffectWorkerHost::start(
            runtime.clone(),
            capability_host.clone(),
            EffectWorkerConfig {
                execution_threads: config.document.runtime.effect_threads,
                execution_queue: config.document.runtime.effect_queue,
                cancellation_queue: config.document.runtime.cancellation_queue,
                maximum_claim_page: config.document.runtime.maximum_effect_claim,
            },
        )
        .map_err(|error| error.to_string())?;
        let persistent = LocalState::load(
            config.document.data_root.join("control-state-v1.json"),
            config.document.command_ledger_bound,
        )?;
        if let Some(service) = &peer_runtime.service {
            service.recover(1_024).map_err(|error| error.to_string())?;
        }
        health.active_effects.store(0, Ordering::SeqCst);
        Ok(Self {
            config,
            store,
            runtime,
            control,
            capability_host,
            authority,
            effect_workers: Some(effect_workers),
            persistent,
            peer_service: peer_runtime.service,
            peer_registries: peer_runtime.registries,
        })
    }

    fn run(
        &mut self,
        receiver: Receiver<OwnerRequest>,
        maintenance: Duration,
        health: &SharedHealth,
    ) {
        loop {
            match receiver.recv_timeout(maintenance) {
                Ok(request) => {
                    health.queued.fetch_sub(1, Ordering::SeqCst);
                    let is_shutdown = matches!(request.operation, OwnerOperation::Shutdown);
                    let result = if is_shutdown {
                        self.shutdown(health)
                    } else {
                        self.execute(request.operation)
                    };
                    let _ = request.reply.send(result);
                    if is_shutdown {
                        return;
                    }
                    self.maintenance(health);
                }
                Err(RecvTimeoutError::Timeout) => self.maintenance(health),
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = self.shutdown(health);
                    return;
                }
            }
        }
    }

    fn maintenance(&self, health: &SharedHealth) {
        if let Err(error) = self.runtime.scheduler_tick() {
            warn!(
                outcome = "error",
                code = "runtime_tick",
                "{}",
                bounded(&error.to_string())
            );
            health.failure("bounded runtime scheduler maintenance failed");
        }
        if let Some(workers) = &self.effect_workers {
            if let Err(error) = workers.poll() {
                warn!(
                    outcome = "error",
                    code = "effect_poll",
                    "{}",
                    bounded(&error.to_string())
                );
            }
            if let Ok(worker_health) = workers.health() {
                let active = worker_health
                    .active_executions
                    .saturating_add(worker_health.active_cancellations);
                health
                    .active_effects
                    .store(u32::try_from(active).unwrap_or(u32::MAX), Ordering::SeqCst);
            }
        }
    }

    fn execute(&mut self, operation: OwnerOperation) -> Result<OwnerValue, PublicFailure> {
        match operation {
            OwnerOperation::Version { session } => self
                .authorize(
                    &session,
                    AuthorityOperation::NegotiateControlProtocol,
                    RequestedResourceFacts::empty(),
                    "read:version",
                )
                .map(|decision| OwnerValue::Authorized(decision.digest().to_owned())),
            OwnerOperation::Health { session } => {
                let mut resources = RequestedResourceFacts::empty();
                resources.daemon_detailed_health = true;
                self.authorize(
                    &session,
                    AuthorityOperation::InspectDaemonHealth,
                    resources,
                    "read:health",
                )
                .map(|decision| OwnerValue::Authorized(decision.digest().to_owned()))
            }
            OwnerOperation::Readiness { session } => {
                let mut resources = RequestedResourceFacts::empty();
                resources.daemon_readiness = true;
                self.authorize(
                    &session,
                    AuthorityOperation::ReadReadiness,
                    resources,
                    "read:readiness",
                )
                .map(|decision| OwnerValue::Authorized(decision.digest().to_owned()))
            }
            OwnerOperation::Authority { session } => {
                let mut resources = RequestedResourceFacts::empty();
                resources.daemon_own_authority = true;
                self.authorize(
                    &session,
                    AuthorityOperation::InspectOwnAuthority,
                    resources,
                    "read:own-authority",
                )?;
                let operations = session
                    .grant
                    .operations()
                    .iter()
                    .filter_map(|operation| serde_json::to_value(operation).ok())
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect();
                Ok(OwnerValue::Authority(
                    milkdrift_control_protocol::AuthorityRead {
                        actor: session.actor.as_str().to_owned(),
                        grant_id: session.grant.identity().as_str().to_owned(),
                        grant_revision: session.grant.revision(),
                        revocation_generation: session.grant.revocation_generation(),
                        operations,
                    },
                ))
            }
            OwnerOperation::Peers { session } => {
                let mut visible = BTreeSet::new();
                for peer in self.peer_registries.keys() {
                    let mut resources = RequestedResourceFacts::empty();
                    resources.peer = Some(peer.clone());
                    if self
                        .evaluate_authority(
                            &session,
                            AuthorityOperation::InspectPeer,
                            resources,
                            "read:peers",
                        )?
                        .is_allowed()
                    {
                        visible.insert(peer.as_str().to_owned());
                    }
                }
                Ok(OwnerValue::PeerIds(visible))
            }
            OwnerOperation::Peer { session, peer } => {
                self.authorize_peer(
                    &session,
                    &peer,
                    AuthorityOperation::InspectPeer,
                    "read:peer",
                )?;
                Ok(OwnerValue::Authorized(String::new()))
            }
            OwnerOperation::AdministerPeer { session, peer } => {
                let decision = self.authorize_peer(
                    &session,
                    &peer,
                    AuthorityOperation::AdministerPeer,
                    "command:administer-peer",
                )?;
                self.record_security_decision(&decision)?;
                self.persistent
                    .flush()
                    .map_err(|error| PublicFailure::new(ErrorCode::Corruption, error, false))?;
                Ok(OwnerValue::Authorized(String::new()))
            }
            OwnerOperation::StreamAuthority { session, stream } => {
                let decision = match stream {
                    StreamAuthority::Run(run) => self.authorize_run_read(
                        &session,
                        &run,
                        AuthorityOperation::InspectTimeline,
                        "stream:run",
                    )?,
                    StreamAuthority::Capabilities => {
                        self.authorize(
                            &session,
                            AuthorityOperation::ListCapabilities,
                            RequestedResourceFacts::empty(),
                            "stream:capabilities",
                        )?;
                        self.authorize(
                            &session,
                            AuthorityOperation::InspectCapabilityHealth,
                            RequestedResourceFacts::empty(),
                            "stream:capability-health",
                        )?;
                        self.authorize(
                            &session,
                            AuthorityOperation::InspectProviderProfile,
                            RequestedResourceFacts::empty(),
                            "stream:provider-profile",
                        )?
                    }
                    StreamAuthority::Health => {
                        let mut resources = RequestedResourceFacts::empty();
                        resources.daemon_detailed_health = true;
                        self.authorize(
                            &session,
                            AuthorityOperation::InspectDaemonHealth,
                            resources,
                            "stream:daemon-health",
                        )?
                    }
                };
                Ok(OwnerValue::Authorized(decision.digest().to_owned()))
            }
            OwnerOperation::Command { session, request } => {
                self.command(&session, *request).map(OwnerValue::Command)
            }
            OwnerOperation::Revision { session, revision } => {
                self.revision(&session, &revision).map(OwnerValue::Revision)
            }
            OwnerOperation::Revisions {
                session,
                workflow,
                cursor,
                limit,
            } => self
                .revisions(&session, workflow.as_deref(), cursor.as_ref(), limit)
                .map(OwnerValue::Revisions),
            OwnerOperation::RevisionDiff { session, from, to } => self
                .revision_diff(&session, &from, &to)
                .map(OwnerValue::RevisionDiff),
            OwnerOperation::Run { session, run } => {
                self.run_read(&session, &run).map(OwnerValue::Run)
            }
            OwnerOperation::Node {
                session,
                run,
                execution,
            } => self
                .node_read(&session, &run, &execution)
                .map(OwnerValue::Node),
            OwnerOperation::Attempt {
                session,
                run,
                attempt,
            } => self
                .attempt_read(&session, &run, &attempt)
                .map(OwnerValue::Attempt),
            OwnerOperation::Runs {
                session,
                state,
                workflow,
                cursor,
                limit,
            } => self
                .runs(
                    &session,
                    state.as_deref(),
                    workflow.as_deref(),
                    cursor.as_ref(),
                    limit,
                )
                .map(OwnerValue::Runs),
            OwnerOperation::Timeline {
                session,
                run,
                cursor,
                limit,
            } => self
                .timeline(&session, &run, cursor.as_ref(), limit)
                .map(OwnerValue::Timeline),
            OwnerOperation::Proposals {
                session,
                run,
                cursor,
                limit,
            } => self
                .proposals(&session, &run, cursor.as_ref(), limit)
                .map(OwnerValue::Proposals),
            OwnerOperation::Proposal {
                session,
                run,
                proposal,
                revision,
            } => self
                .proposal(&session, &run, &proposal, &revision)
                .map(OwnerValue::Proposal),
            OwnerOperation::Capabilities { session } => {
                self.capabilities(&session).map(OwnerValue::Capabilities)
            }
            OwnerOperation::ArtifactMetadata { session, artifact } => self
                .artifact_metadata(&session, &artifact)
                .map(OwnerValue::ArtifactMetadata),
            OwnerOperation::ArtifactRange {
                session,
                artifact,
                offset,
                maximum,
                evidence,
            } => self.artifact_range(&session, &artifact, offset, maximum, &evidence),
            OwnerOperation::Layout {
                session,
                workflow,
                revision,
            } => self
                .layout(&session, &workflow, &revision)
                .map(OwnerValue::Layout),
            OwnerOperation::Shutdown => Err(PublicFailure::new(
                ErrorCode::Internal,
                "shutdown was dispatched through the ordinary operation path",
                false,
            )),
        }
    }

    fn authorize(
        &self,
        session: &ActorSession,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        boundary: &str,
    ) -> Result<AuthorityDecisionSnapshot, PublicFailure> {
        let decision = self.evaluate_authority(session, operation, resources, boundary)?;
        if decision.is_allowed() {
            Ok(decision)
        } else {
            Err(unauthorized_decision(&decision))
        }
    }

    fn evaluate_authority(
        &self,
        session: &ActorSession,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        boundary: &str,
    ) -> Result<AuthorityDecisionSnapshot, PublicFailure> {
        let claim = session.context.authority();
        let now = unix_millis();
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.daemon-authority.v1\0");
        hasher.update(session.actor.as_str().as_bytes());
        hasher.update(boundary.as_bytes());
        hasher.update(format!("{operation:?}{resources:?}{now}").as_bytes());
        let request = AuthorityRequest {
            decision: DecisionId::new(format!("decision:{}", hasher.finalize()))
                .map_err(|error| invalid(&error.to_string()))?,
            actor: session.actor.clone(),
            grant: claim.grant().clone(),
            grant_revision: claim.grant_revision(),
            grant_digest: claim.grant_digest().clone(),
            revocation_generation: claim.revocation_generation(),
            operation,
            resources,
            budget: AuthorityBudget::default(),
            evaluated_at: BoundaryTimeMillis::new(now),
            provenance: AuthorityExecutionProvenance::default(),
        };
        self.authority.evaluate(&request).map_err(|_| internal())
    }

    fn authorize_peer(
        &self,
        session: &ActorSession,
        peer: &str,
        operation: AuthorityOperation,
        boundary: &str,
    ) -> Result<AuthorityDecisionSnapshot, PublicFailure> {
        let peer = PeerId::new(peer.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let mut resources = RequestedResourceFacts::empty();
        resources.peer = Some(peer.clone());
        let decision = self.authorize(session, operation, resources, boundary)?;
        if !self.peer_registries.contains_key(&peer) {
            return Err(not_found());
        }
        Ok(decision)
    }

    fn shutdown(&mut self, health: &SharedHealth) -> Result<OwnerValue, PublicFailure> {
        info!(phase = "draining", "runtime owner closing admission");
        health.set_lifecycle(Lifecycle::Draining);
        if let Some(service) = &self.peer_service {
            service.begin_shutdown();
        }
        for registry in self.peer_registries.values() {
            let _ = registry.disconnect();
        }
        self.runtime.begin_shutdown();
        let mode = match self.config.document.shutdown.effect_policy {
            ShutdownEffectPolicy::Drain => EffectShutdownMode::Drain,
            ShutdownEffectPolicy::Cancel => EffectShutdownMode::Cancel,
            ShutdownEffectPolicy::Retain => EffectShutdownMode::Retain,
        };
        let deadline = Duration::from_millis(self.config.document.shutdown.deadline_ms);
        let result = self
            .effect_workers
            .take()
            .ok_or_else(|| {
                PublicFailure::new(ErrorCode::Internal, "effect owner is absent", false)
            })?
            .shutdown(mode, deadline)
            .map_err(|error| {
                PublicFailure::new(ErrorCode::Unavailable, bounded(&error.to_string()), true)
            })?;
        self.persistent
            .flush()
            .map_err(|error| PublicFailure::new(ErrorCode::Corruption, error, false))?;
        health.active_effects.store(0, Ordering::SeqCst);
        health.set_lifecycle(Lifecycle::Stopped);
        info!(
            phase = "stopped",
            clean = result.clean,
            unresolved = result.unresolved_invocations.len(),
            "runtime owner stopped"
        );
        Ok(OwnerValue::Shutdown {
            clean: result.clean,
        })
    }

    fn command(
        &mut self,
        session: &ActorSession,
        mut request: CommandRequest,
    ) -> Result<CommandAccepted, PublicFailure> {
        request.validate().map_err(public_protocol)?;
        let ledger_key = format!("{}|{}", session.actor.as_str(), request.command_id);
        let fingerprint = command_fingerprint(session, &request)?;
        if let Some(existing) = self.persistent.document.commands.get(&ledger_key) {
            if existing.fingerprint != fingerprint {
                return Err(PublicFailure::new(
                    ErrorCode::Conflict,
                    "command identity was already used with different content",
                    false,
                ));
            }
            let mut accepted = existing.result.clone();
            accepted.replayed = true;
            self.persistent
                .flush()
                .map_err(|error| PublicFailure::new(ErrorCode::Corruption, error, false))?;
            return Ok(accepted);
        }
        if self.persistent.document.commands.len()
            >= usize::try_from(self.persistent.command_bound).unwrap_or(usize::MAX)
        {
            return Err(PublicFailure::new(
                ErrorCode::Overload,
                "durable command idempotency ledger reached its configured bound",
                false,
            ));
        }
        if let Command::PutLayout { layout } = &mut request.command {
            layout.author = session.actor.as_str().to_owned();
            layout.digest = layout.computed_digest().map_err(public_protocol)?;
        }
        let result = self.execute_new_command(session, &request)?;
        let proposal = proposal_ledger_ref(&request.command, &result)?;
        self.persistent.document.commands.insert(
            ledger_key,
            LedgerEntry {
                fingerprint,
                result: result.clone(),
                proposal,
            },
        );
        self.persistent
            .flush()
            .map_err(|error| PublicFailure::new(ErrorCode::Corruption, error, false))?;
        Ok(result)
    }

    fn execute_new_command(
        &mut self,
        session: &ActorSession,
        request: &CommandRequest,
    ) -> Result<CommandAccepted, PublicFailure> {
        match &request.command {
            Command::ImportBlueprint { document } => {
                let bytes =
                    serde_json::to_vec(document).map_err(|_| invalid("invalid blueprint JSON"))?;
                let (_document, revision) = BlueprintRevisionDocument::from_json(&bytes)
                    .map_err(|error| invalid(&bounded(&error.to_string())))?;
                let mut resources = RequestedResourceFacts::empty();
                resources.workflow = Some(revision.semantic().workflow().clone());
                resources.revision = Some(revision.id().clone());
                let decision = self.authorize(
                    session,
                    AuthorityOperation::ImportBlueprint,
                    resources,
                    "command:import-blueprint",
                )?;
                let outcome = self
                    .store
                    .put_revision(&revision)
                    .map_err(public_persistence)?;
                self.record_security_decision(&decision)?;
                Ok(CommandAccepted {
                    command_id: request.command_id.clone(),
                    replayed: matches!(
                        outcome,
                        milkdrift_persistence::ImmutableRevisionPut::AlreadyPresent
                    ),
                    resulting_sequence: None,
                    result_type: "blueprint_imported".to_owned(),
                    value: json!({
                        "revision_id": revision.id().as_str(),
                        "workflow_id": revision.semantic().workflow().as_str(),
                        "semantic_digest": revision.content_digest().as_str(),
                    }),
                })
            }
            Command::ValidateBlueprint { document } => {
                let bytes =
                    serde_json::to_vec(document).map_err(|_| invalid("invalid blueprint JSON"))?;
                let (_document, revision) = BlueprintRevisionDocument::from_json(&bytes)
                    .map_err(|error| invalid(&bounded(&error.to_string())))?;
                let mut resources = RequestedResourceFacts::empty();
                resources.workflow = Some(revision.semantic().workflow().clone());
                resources.revision = Some(revision.id().clone());
                self.authorize(
                    session,
                    AuthorityOperation::ValidateBlueprint,
                    resources,
                    "command:validate-blueprint",
                )?;
                Ok(CommandAccepted {
                    command_id: request.command_id.clone(),
                    replayed: false,
                    resulting_sequence: None,
                    result_type: "blueprint_valid".to_owned(),
                    value: json!({"revision_id": revision.id().as_str(), "semantic_digest": revision.content_digest().as_str()}),
                })
            }
            Command::StartRun {
                run_id,
                workflow_id,
                revision_id,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let workflow = WorkflowId::new(workflow_id.clone())
                    .map_err(|error| invalid(&error.to_string()))?;
                let revision = parse_revision_id(revision_id)?;
                let root_scope = WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("root").map_err(|error| invalid(&error.to_string()))?,
                );
                let create = ControlCommand::CreateRun {
                    run: run.clone(),
                    workflow,
                    revision,
                    root_scope,
                    workspace_budget: default_workspace_budget()
                        .map_err(|error| invalid(&error.to_string()))?,
                    inputs: Vec::new(),
                };
                let create_sequence =
                    self.execute_control(session, request, Some(0), create, "create")?;
                let start = ControlCommand::StartRun { run };
                let sequence =
                    self.execute_control(session, request, Some(create_sequence), start, "start")?;
                accepted_sequence(request, sequence, "run_started")
            }
            Command::PauseRun { run_id } => {
                self.simple_run_command(session, request, run_id, "pause", |run| {
                    ControlCommand::PauseRun { run }
                })
            }
            Command::ResumeRun { run_id } => {
                self.simple_run_command(session, request, run_id, "resume", |run| {
                    ControlCommand::ResumeRun { run }
                })
            }
            Command::CancelRun { run_id } => {
                self.simple_run_command(session, request, run_id, "cancel", |run| {
                    ControlCommand::RequestCancellation { run }
                })
            }
            Command::SignalRun {
                run_id,
                signal_id,
                signal_type,
                correlation,
                broadcast,
                payload,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let command = ControlCommand::Signal {
                    run,
                    signal: SignalId::new(signal_id.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                    signal_type: SignalTypeId::new(signal_type.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                    correlation: correlation
                        .as_ref()
                        .map(|value| CorrelationKey::new(value.clone()))
                        .transpose()
                        .map_err(|error| invalid(&error.to_string()))?,
                    mode: if *broadcast {
                        SignalDeliveryMode::Broadcast
                    } else {
                        SignalDeliveryMode::OneShot
                    },
                    payload: milkdrift_capability::BoundedJson::new(payload.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                };
                let sequence = self.execute_control(
                    session,
                    request,
                    request.expected_sequence,
                    command,
                    "signal",
                )?;
                accepted_sequence(request, sequence, "signal_delivered")
            }
            Command::ResolveWork {
                run_id,
                attempt_id,
                decision_id,
                action,
                remediation_node,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let command = ControlCommand::ResolveExternalWork {
                    run,
                    attempt: AttemptId::new(attempt_id.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                    decision: ReconciliationDecisionId::new(decision_id.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                    action: map_resolve(*action),
                    remediation_node: remediation_node
                        .as_ref()
                        .map(|value| milkdrift_blueprint::NodeId::new(value.clone()))
                        .transpose()
                        .map_err(|error| invalid(&error.to_string()))?,
                };
                let sequence = self.execute_control(
                    session,
                    request,
                    request.expected_sequence,
                    command,
                    "resolve",
                )?;
                accepted_sequence(request, sequence, "external_work_resolved")
            }
            Command::SubmitProposal { document } => {
                let bytes =
                    serde_json::to_vec(document).map_err(|_| invalid("invalid proposal JSON"))?;
                let proposal = WorkflowProposalDocument::from_json(&bytes)
                    .map_err(|error| invalid(&bounded(&error.to_string())))?;
                let digest = proposal.proposal().digest().clone();
                let command = ControlCommand::SubmitProposal { proposal };
                let value = self.execute_control_result(
                    session,
                    request,
                    request.expected_sequence,
                    Some(digest),
                    command,
                    "proposal",
                )?;
                match value {
                    ControlResult::ProposalSubmitted { value } => Ok(CommandAccepted {
                        command_id: request.command_id.clone(),
                        replayed: false,
                        resulting_sequence: value
                            .reconciliation
                            .as_ref()
                            .and_then(|item| item.applied_sequence)
                            .map(|sequence| sequence.get()),
                        result_type: "proposal_submitted".to_owned(),
                        value: serde_json::to_value(value).map_err(|_| internal())?,
                    }),
                    _ => Err(internal()),
                }
            }
            Command::DecideProposal {
                run_id,
                proposal_id,
                proposal_digest,
                proposed_revision,
                decision_id,
                decision,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let proposal = ProposalId::new(proposal_id.clone())
                    .map_err(|error| invalid(&error.to_string()))?;
                let digest: ProposalDigest =
                    serde_json::from_value(Value::String(proposal_digest.clone()))
                        .map_err(|error| invalid(&error.to_string()))?;
                let revision = parse_revision_id(proposed_revision)?;
                let decision_id = ReconciliationDecisionId::new(decision_id.clone())
                    .map_err(|error| invalid(&error.to_string()))?;
                let command = match decision {
                    ProposalDecision::Approve => ControlCommand::ApproveProposal {
                        run,
                        proposal,
                        proposal_digest: digest.clone(),
                        proposed_revision: revision,
                        decision: decision_id,
                    },
                    ProposalDecision::Reject => ControlCommand::RejectProposal {
                        run,
                        proposal,
                        proposal_digest: digest.clone(),
                        proposed_revision: revision,
                        decision: decision_id,
                    },
                };
                let sequence = self.execute_control_guarded(
                    session,
                    request,
                    request.expected_sequence,
                    Some(digest),
                    command,
                    "decision",
                )?;
                accepted_sequence(request, sequence, "proposal_decided")
            }
            Command::ApplyProposal {
                run_id,
                proposal_id,
                proposal_digest,
                proposed_revision,
            } => {
                let run =
                    RunId::new(run_id.clone()).map_err(|error| invalid(&error.to_string()))?;
                let digest: ProposalDigest =
                    serde_json::from_value(Value::String(proposal_digest.clone()))
                        .map_err(|error| invalid(&error.to_string()))?;
                let command = ControlCommand::ApplyProposal {
                    run,
                    proposal: ProposalId::new(proposal_id.clone())
                        .map_err(|error| invalid(&error.to_string()))?,
                    proposal_digest: digest.clone(),
                    proposed_revision: parse_revision_id(proposed_revision)?,
                };
                let sequence = self.execute_control_guarded(
                    session,
                    request,
                    request.expected_sequence,
                    Some(digest),
                    command,
                    "apply",
                )?;
                accepted_sequence(request, sequence, "proposal_applied")
            }
            Command::PutLayout { layout } => {
                layout.validate().map_err(public_protocol)?;
                let revision = parse_revision_id(&layout.revision_id)?;
                let stored_revision = self
                    .store
                    .revision(&revision)
                    .map_err(public_persistence)?
                    .ok_or_else(not_found)?;
                if stored_revision.semantic().workflow().as_str() != layout.workflow_id {
                    return Err(invalid(
                        "layout workflow/revision association does not match durable revision",
                    ));
                }
                let mut resources = RequestedResourceFacts::empty();
                resources.workflow = Some(stored_revision.semantic().workflow().clone());
                resources.revision = Some(revision);
                resources.layout_owner = Some(LayoutOwner::Shared);
                let decision = self.authorize(
                    session,
                    AuthorityOperation::WriteLayout,
                    resources,
                    "command:put-layout",
                )?;
                let key = layout_key(&layout.workflow_id, &layout.revision_id);
                if let Some(current) = self.persistent.document.layouts.get(&key) {
                    if current.digest == layout.digest {
                        return Ok(CommandAccepted {
                            command_id: request.command_id.clone(),
                            replayed: true,
                            resulting_sequence: None,
                            result_type: "layout_updated".to_owned(),
                            value: serde_json::to_value(layout).map_err(|_| internal())?,
                        });
                    }
                    if layout.generation != current.generation.saturating_add(1) {
                        return Err(conflict("layout generation is stale"));
                    }
                } else if layout.generation != 1 {
                    return Err(conflict("first layout generation must be one"));
                }
                self.persistent.document.layouts.insert(key, layout.clone());
                self.record_security_decision(&decision)?;
                Ok(CommandAccepted {
                    command_id: request.command_id.clone(),
                    replayed: false,
                    resulting_sequence: None,
                    result_type: "layout_updated".to_owned(),
                    value: serde_json::to_value(layout).map_err(|_| internal())?,
                })
            }
        }
    }

    fn simple_run_command<F>(
        &self,
        session: &ActorSession,
        request: &CommandRequest,
        run_id: &str,
        suffix: &str,
        build: F,
    ) -> Result<CommandAccepted, PublicFailure>
    where
        F: FnOnce(RunId) -> ControlCommand,
    {
        let run = RunId::new(run_id.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let sequence = self.execute_control(
            session,
            request,
            request.expected_sequence,
            build(run),
            suffix,
        )?;
        accepted_sequence(request, sequence, &format!("run_{suffix}d"))
    }

    fn execute_control(
        &self,
        session: &ActorSession,
        request: &CommandRequest,
        expected_sequence: Option<u64>,
        command: ControlCommand,
        suffix: &str,
    ) -> Result<u64, PublicFailure> {
        let result = self.execute_control_result(
            session,
            request,
            expected_sequence,
            None,
            command,
            suffix,
        )?;
        match result {
            ControlResult::RuntimeCommand { resulting_sequence } => Ok(resulting_sequence.get()),
            _ => Err(internal()),
        }
    }

    fn execute_control_guarded(
        &self,
        session: &ActorSession,
        request: &CommandRequest,
        expected_sequence: Option<u64>,
        proposal_digest: Option<ProposalDigest>,
        command: ControlCommand,
        suffix: &str,
    ) -> Result<u64, PublicFailure> {
        let result = self.execute_control_result(
            session,
            request,
            expected_sequence,
            proposal_digest,
            command,
            suffix,
        )?;
        match result {
            ControlResult::RuntimeCommand { resulting_sequence } => Ok(resulting_sequence.get()),
            _ => Err(internal()),
        }
    }

    fn execute_control_result(
        &self,
        session: &ActorSession,
        request: &CommandRequest,
        expected_sequence: Option<u64>,
        proposal_digest: Option<ProposalDigest>,
        command: ControlCommand,
        suffix: &str,
    ) -> Result<ControlResult, PublicFailure> {
        let document = ControlCommandDocument::new(
            internal_control_id(session, request, suffix)?,
            session.context.clone(),
            TimestampMillis::new(unix_millis()),
            OptimisticGuard {
                expected_run_sequence: expected_sequence.map(RunSequence::new),
                expected_revision: request
                    .expected_revision
                    .as_deref()
                    .map(parse_revision_id)
                    .transpose()?,
                expected_proposal_digest: proposal_digest,
            },
            Reason::new(request.reason.clone()).map_err(public_persistence)?,
            evidence(request)?,
            command,
        )
        .map_err(public_control)?;
        self.control.execute(&document).map_err(public_control)
    }

    fn revision(
        &self,
        session: &ActorSession,
        revision: &str,
    ) -> Result<RevisionRead, PublicFailure> {
        let revision_id = parse_revision_id(revision)?;
        let command = ControlCommand::InspectRevision {
            revision: revision_id.clone(),
        };
        let result = self.inspect_control(session, command, None, "revision")?;
        let ControlResult::RevisionInspection { value } = result else {
            return Err(internal());
        };
        let stored = self
            .store
            .revision(&revision_id)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let document = BlueprintRevisionDocument::new(&stored)
            .to_canonical_json()
            .map_err(|error| invalid(&error.to_string()))?;
        Ok(RevisionRead {
            summary: PublicRevisionSummary {
                revision_id: value.revision.as_str().to_owned(),
                workflow_id: value.workflow.as_str().to_owned(),
                lineage_sequence: value.lineage_sequence,
                semantic_digest: value.content_digest.as_str().to_owned(),
                parents: value
                    .parents
                    .iter()
                    .map(|parent| parent.as_str().to_owned())
                    .collect(),
            },
            author: value.author.as_str().to_owned(),
            reason: value.reason,
            node_count: u32::try_from(value.node_count).unwrap_or(u32::MAX),
            edge_count: u32::try_from(value.edge_count).unwrap_or(u32::MAX),
            document: serde_json::from_slice(&document).ok(),
        })
    }

    fn revisions(
        &self,
        session: &ActorSession,
        workflow: Option<&str>,
        cursor: Option<&Cursor>,
        limit: u32,
    ) -> Result<Page<PublicRevisionSummary>, PublicFailure> {
        let requested_workflow = workflow
            .map(|value| WorkflowId::new(value.to_owned()))
            .transpose()
            .map_err(|error| invalid(&error.to_string()))?;
        let workflow_id = match &session.grant.resources().workflow_run {
            WorkflowRunScope::Any => requested_workflow,
            WorkflowRunScope::Workflow { workflow: allowed } => {
                if requested_workflow
                    .as_ref()
                    .is_some_and(|value| value != allowed)
                {
                    return Err(unauthorized());
                }
                Some(allowed.clone())
            }
            WorkflowRunScope::Run { .. } => return Err(unauthorized()),
        };
        let feed = format!(
            "revisions:{}",
            workflow_id.as_ref().map_or("*", WorkflowId::as_str)
        );
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = workflow_id.clone();
        let decision = self.authorize(
            session,
            AuthorityOperation::InspectRevision,
            resources,
            "read:revisions",
        )?;
        let binding = cursor_binding(session, &feed)?;
        let filter = RevisionFilter {
            workflow: workflow_id,
        };
        let internal_cursor = cursor
            .map(|cursor| {
                cursor
                    .key_for_bound(&feed, &binding, session.cursor_key())
                    .map_err(public_protocol)
            })
            .transpose()?
            .map(|value| parse_revision_id(&value))
            .transpose()?
            .map(|revision| RevisionCursor::new(revision, filter.clone()));
        let page = self
            .store
            .revisions(&RevisionPageQuery {
                filter,
                cursor: internal_cursor,
                limit: PageSize::new(limit).map_err(public_persistence)?,
            })
            .map_err(public_persistence)?;
        let next_cursor = page
            .next
            .as_ref()
            .map(|cursor| {
                Cursor::new_bound_key(
                    &feed,
                    cursor.after_revision().as_str(),
                    binding.clone(),
                    decision.digest(),
                    session.cursor_key(),
                )
                .map_err(public_protocol)
            })
            .transpose()?;
        Ok(Page {
            items: page.revisions.iter().map(public_revision_summary).collect(),
            next_cursor,
            observed_cursor: None,
        })
    }

    fn revision_diff(
        &self,
        session: &ActorSession,
        from: &str,
        to: &str,
    ) -> Result<RevisionDiffRead, PublicFailure> {
        let left = self.revision(session, from)?;
        let right = self.revision(session, to)?;
        if left.summary.workflow_id != right.summary.workflow_id {
            return Err(invalid("revision diff requires one workflow lineage"));
        }
        let left_revision = self
            .store
            .revision(&parse_revision_id(from)?)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let right_revision = self
            .store
            .revision(&parse_revision_id(to)?)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let mut changes = Vec::new();
        diff_keys(
            "node",
            left_revision.semantic().nodes(),
            right_revision.semantic().nodes(),
            &mut changes,
        );
        diff_keys(
            "edge",
            left_revision.semantic().edges(),
            right_revision.semantic().edges(),
            &mut changes,
        );
        let truncated = changes.len() > 1_024;
        changes.truncate(1_024);
        Ok(RevisionDiffRead {
            from_revision: from.to_owned(),
            to_revision: to.to_owned(),
            changes,
            truncated,
        })
    }

    fn run_read(&self, session: &ActorSession, run: &str) -> Result<RunRead, PublicFailure> {
        let run_id = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let result = self.inspect_control(
            session,
            ControlCommand::InspectRun { run: run_id },
            None,
            "run",
        )?;
        let ControlResult::RunInspection { value } = result else {
            return Err(internal());
        };
        Ok(public_run(value))
    }

    fn node_read(
        &self,
        session: &ActorSession,
        run: &str,
        execution: &str,
    ) -> Result<NodeRead, PublicFailure> {
        self.authorize_run_read(
            session,
            run,
            AuthorityOperation::InspectNodeExecution,
            "read:node-execution",
        )?;
        self.run_read(session, run)?
            .nodes
            .into_iter()
            .find(|node| node.execution_id == execution)
            .ok_or_else(not_found)
    }

    fn attempt_read(
        &self,
        session: &ActorSession,
        run: &str,
        attempt: &str,
    ) -> Result<AttemptRead, PublicFailure> {
        self.authorize_run_read(
            session,
            run,
            AuthorityOperation::InspectAttempt,
            "read:attempt",
        )?;
        self.run_read(session, run)?
            .nodes
            .into_iter()
            .filter_map(|node| node.latest_attempt)
            .find(|value| value.attempt_id == attempt)
            .ok_or_else(not_found)
    }

    fn authorize_run_read(
        &self,
        session: &ActorSession,
        run: &str,
        operation: AuthorityOperation,
        boundary: &str,
    ) -> Result<AuthorityDecisionSnapshot, PublicFailure> {
        let run = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let summary = self
            .store
            .run_summary(&run)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = Some(summary.workflow);
        resources.run = Some(run);
        self.authorize(session, operation, resources, boundary)
    }

    fn runs(
        &self,
        session: &ActorSession,
        state: Option<&str>,
        workflow: Option<&str>,
        cursor: Option<&Cursor>,
        limit: u32,
    ) -> Result<Page<RunRead>, PublicFailure> {
        let indexed_state = state.map(parse_run_state).transpose()?;
        let requested_workflow = workflow
            .map(|value| WorkflowId::new(value.to_owned()))
            .transpose()
            .map_err(|error| invalid(&error.to_string()))?;
        if let WorkflowRunScope::Run {
            run,
            workflow: allowed_workflow,
        } = &session.grant.resources().workflow_run
        {
            if cursor.is_some()
                || requested_workflow.as_ref().is_some_and(|value| {
                    allowed_workflow
                        .as_ref()
                        .is_some_and(|allowed| value != allowed)
                })
            {
                return Err(unauthorized());
            }
            let summary = self
                .store
                .run_summary(run)
                .map_err(public_persistence)?
                .ok_or_else(not_found)?;
            let state_matches =
                state.is_none_or(|expected| snake_debug(&summary.state) == expected);
            let value = self.run_read(session, run.as_str())?;
            let workflow_matches = requested_workflow
                .as_ref()
                .is_none_or(|expected| value.workflow_id.as_deref() == Some(expected.as_str()));
            return Ok(Page {
                items: if state_matches && workflow_matches {
                    vec![value]
                } else {
                    Vec::new()
                },
                next_cursor: None,
                observed_cursor: None,
            });
        }
        let workflow_id = match &session.grant.resources().workflow_run {
            WorkflowRunScope::Any => requested_workflow,
            WorkflowRunScope::Workflow { workflow: allowed } => {
                if requested_workflow
                    .as_ref()
                    .is_some_and(|value| value != allowed)
                {
                    return Err(unauthorized());
                }
                Some(allowed.clone())
            }
            WorkflowRunScope::Run { .. } => unreachable!("exact run scope returned above"),
        };
        let feed = format!(
            "runs:{}:{}",
            state.unwrap_or("*"),
            workflow_id.as_ref().map_or("*", WorkflowId::as_str)
        );
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = workflow_id.clone();
        let decision = self.authorize(
            session,
            AuthorityOperation::InspectRun,
            resources,
            "read:runs",
        )?;
        let binding = cursor_binding(session, &feed)?;
        let filter = RunSummaryFilter {
            state: indexed_state,
            workflow: workflow_id,
        };
        let internal_cursor = cursor
            .map(|cursor| {
                cursor
                    .key_for_bound(&feed, &binding, session.cursor_key())
                    .map_err(public_protocol)
            })
            .transpose()?
            .map(|value| RunId::new(value).map_err(|error| invalid(&error.to_string())))
            .transpose()?
            .map(|run| RunSummaryCursor::for_query(run, filter.clone()));
        let page = self
            .store
            .run_summaries(&RunSummaryPageQuery {
                filter,
                cursor: internal_cursor,
                limit: PageSize::new(limit).map_err(public_persistence)?,
            })
            .map_err(public_persistence)?;
        let mut runs = Vec::with_capacity(page.runs.len());
        for summary in &page.runs {
            runs.push(self.run_read(session, summary.run.as_str())?);
        }
        let next_cursor = page
            .next
            .as_ref()
            .map(|cursor| {
                Cursor::new_bound_key(
                    &feed,
                    cursor.after_run().as_str(),
                    binding.clone(),
                    decision.digest(),
                    session.cursor_key(),
                )
                .map_err(public_protocol)
            })
            .transpose()?;
        Ok(Page {
            items: runs,
            next_cursor,
            observed_cursor: None,
        })
    }

    fn timeline(
        &self,
        session: &ActorSession,
        run: &str,
        cursor: Option<&Cursor>,
        limit: u32,
    ) -> Result<Page<TimelineEntry>, PublicFailure> {
        let run_id = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let feed = format!("timeline:{run}");
        let mut resources = RequestedResourceFacts::empty();
        resources.run = Some(run_id.clone());
        let workflow = self
            .store
            .run_summary(&run_id)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?
            .workflow;
        resources.workflow = Some(workflow);
        let decision = self.authorize(
            session,
            AuthorityOperation::InspectTimeline,
            resources,
            "read:timeline",
        )?;
        let binding = cursor_binding(session, &feed)?;
        let next_sequence = cursor
            .map(|cursor| {
                cursor
                    .position_for_bound(&feed, &binding, session.cursor_key())
                    .map_err(public_protocol)
            })
            .transpose()?
            .map(|position| position.saturating_add(1))
            .unwrap_or(1);
        let result = self.inspect_control(
            session,
            ControlCommand::InspectTimeline {
                run: run_id,
                after: Some(RunSequence::new(next_sequence)),
                limit: PageSize::new(limit).map_err(public_persistence)?,
            },
            None,
            "timeline",
        )?;
        let ControlResult::Timeline { value } = result else {
            return Err(internal());
        };
        let items = value.events.iter().map(public_timeline).collect::<Vec<_>>();
        let next_cursor = value
            .next_sequence
            .map(|sequence| {
                Cursor::new_bound(
                    &feed,
                    sequence.get().saturating_sub(1),
                    binding.clone(),
                    decision.digest(),
                    session.cursor_key(),
                )
                .map_err(public_protocol)
            })
            .transpose()?;
        let observed_cursor = if value.observed_head == RunSequence::ZERO {
            None
        } else {
            Some(
                Cursor::new_bound(
                    &feed,
                    value.observed_head.get(),
                    binding,
                    decision.digest(),
                    session.cursor_key(),
                )
                .map_err(public_protocol)?,
            )
        };
        Ok(Page {
            items,
            next_cursor,
            observed_cursor,
        })
    }

    fn capabilities(&self, session: &ActorSession) -> Result<Vec<CapabilityRead>, PublicFailure> {
        self.authorize(
            session,
            AuthorityOperation::ListCapabilities,
            RequestedResourceFacts::empty(),
            "read:capabilities",
        )?;
        self.authorize(
            session,
            AuthorityOperation::InspectCapabilityHealth,
            RequestedResourceFacts::empty(),
            "read:capability-health",
        )?;
        self.authorize(
            session,
            AuthorityOperation::InspectProviderProfile,
            RequestedResourceFacts::empty(),
            "read:provider-profile",
        )?;
        let scope = &session.grant.resources().capability;
        self.capability_host
            .generations(scope, unix_millis())
            .map_err(|error| {
                PublicFailure::new(ErrorCode::Unavailable, bounded(&error.to_string()), true)
            })
            .map(|views| {
                views
                    .into_iter()
                    .map(|view| CapabilityRead {
                        capability_id: view.capability.as_str().to_owned(),
                        generation: view.descriptor_revision,
                        descriptor_digest: view.descriptor_digest,
                        category: snake_debug(&view.category),
                        operations: view
                            .operations
                            .iter()
                            .map(|operation| operation.as_str().to_owned())
                            .collect(),
                        provider_profile: view
                            .provider_profile
                            .map(|profile| profile.as_str().to_owned()),
                        locality: snake_debug(&view.locality),
                        peer_id: view.peer.map(|peer| peer.as_str().to_owned()),
                        trust_zones: view
                            .trust_zones
                            .iter()
                            .map(|zone| zone.as_str().to_owned())
                            .collect(),
                        current: view.current,
                        draining: view.draining,
                        health: snake_debug(&view.health),
                        available: view.available,
                        active_permits: view.active_permits,
                        permit_limit: view.permit_limit,
                    })
                    .collect()
            })
    }

    fn proposals(
        &self,
        session: &ActorSession,
        run: &str,
        cursor: Option<&Cursor>,
        limit: u32,
    ) -> Result<Page<ProposalRead>, PublicFailure> {
        let run_id = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let feed = format!("proposals:{run}");
        let summary = self
            .store
            .run_summary(&run_id)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = Some(summary.workflow);
        resources.run = Some(run_id.clone());
        let decision = self.authorize(
            session,
            AuthorityOperation::InspectProposal,
            resources,
            "read:proposals",
        )?;
        let binding = cursor_binding(session, &feed)?;
        let after = cursor
            .map(|cursor| {
                cursor
                    .key_for_bound(&feed, &binding, session.cursor_key())
                    .map_err(public_protocol)
            })
            .transpose()?;
        let limit = usize::try_from(PageSize::new(limit).map_err(public_persistence)?.get())
            .map_err(|_| invalid("proposal page limit exceeds platform"))?;
        let mut scanned = 0_usize;
        let mut last = None;
        let mut items = Vec::new();
        let start = after
            .map(std::ops::Bound::Excluded)
            .unwrap_or(std::ops::Bound::Unbounded);
        for (key, entry) in self
            .persistent
            .document
            .commands
            .range((start, std::ops::Bound::Unbounded))
            .take(limit)
        {
            scanned += 1;
            last = Some(key.clone());
            let Some(proposal) = &entry.proposal else {
                continue;
            };
            if proposal.run != run_id.as_str() {
                continue;
            }
            items.push(self.proposal(session, run, &proposal.proposal, &proposal.revision)?);
        }
        let next_cursor = if scanned == limit {
            last.map(|key| {
                Cursor::new_bound_key(
                    &feed,
                    &key,
                    binding.clone(),
                    decision.digest(),
                    session.cursor_key(),
                )
                .map_err(public_protocol)
            })
            .transpose()?
        } else {
            None
        };
        Ok(Page {
            items,
            next_cursor,
            observed_cursor: None,
        })
    }

    fn proposal(
        &self,
        session: &ActorSession,
        run: &str,
        proposal: &str,
        revision: &str,
    ) -> Result<ProposalRead, PublicFailure> {
        let run = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let proposal =
            ProposalId::new(proposal.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let revision = parse_revision_id(revision)?;
        let result = self.inspect_control(
            session,
            ControlCommand::QueryProposal {
                run,
                proposal,
                proposed_revision: revision,
            },
            None,
            "proposal-status",
        )?;
        let ControlResult::ProposalStatus { value } = result else {
            return Err(internal());
        };
        Ok(ProposalRead {
            proposal_id: value.proposal.as_str().to_owned(),
            proposed_revision: value.proposed_revision.as_str().to_owned(),
            status: snake_debug(&value.reconciliation.state),
            approved: value.reconciliation.approved,
            applied_sequence: value
                .reconciliation
                .applied_sequence
                .map(|sequence| sequence.get()),
        })
    }

    fn artifact_metadata(
        &mut self,
        session: &ActorSession,
        artifact: &str,
    ) -> Result<ArtifactMetadataRead, PublicFailure> {
        let artifact =
            ArtifactId::new(artifact.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let metadata = self
            .store
            .metadata(&artifact)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let mut resources = RequestedResourceFacts::empty();
        resources.artifact = Some(artifact);
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        let decision = self.authorize(
            session,
            AuthorityOperation::ReadArtifactMetadata,
            resources,
            "read:artifact-metadata",
        )?;
        if metadata.sensitivity() != ArtifactSensitivity::Public {
            self.record_security_decision(&decision)?;
            self.persistent
                .flush()
                .map_err(|error| PublicFailure::new(ErrorCode::Corruption, error, false))?;
        }
        Ok(public_artifact_metadata(&metadata))
    }

    fn artifact_range(
        &mut self,
        session: &ActorSession,
        artifact: &str,
        offset: u64,
        maximum: u32,
        evidence: &str,
    ) -> Result<OwnerValue, PublicFailure> {
        let artifact_id =
            ArtifactId::new(artifact.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let metadata = self
            .store
            .metadata(&artifact_id)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let mut resources = RequestedResourceFacts::empty();
        resources.artifact = Some(artifact_id);
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        let decision = self.authorize(
            session,
            AuthorityOperation::ReadArtifactContent,
            resources,
            "read:artifact-content",
        )?;
        let authority = ArtifactReadAuthority::Authorized {
            actor: session.actor.clone(),
            evidence: EvidenceId::new(format!("{evidence}-{}", decision.digest()))
                .map_err(public_persistence)?,
        };
        let chunk = self
            .store
            .read_chunk(
                &ArtifactReadRequest::new(metadata.reference().clone(), offset, maximum, authority)
                    .map_err(public_persistence)?,
            )
            .map_err(public_persistence)?;
        self.record_security_decision(&decision)?;
        self.persistent
            .flush()
            .map_err(|error| PublicFailure::new(ErrorCode::Corruption, error, false))?;
        Ok(OwnerValue::ArtifactRange {
            metadata: public_artifact_metadata(&metadata),
            offset: chunk.offset,
            bytes: chunk.bytes,
            end: chunk.end_of_artifact,
        })
    }

    fn layout(
        &self,
        session: &ActorSession,
        workflow: &str,
        revision: &str,
    ) -> Result<LayoutDocument, PublicFailure> {
        let workflow_id =
            WorkflowId::new(workflow.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let revision_id = parse_revision_id(revision)?;
        let mut resources = RequestedResourceFacts::empty();
        resources.workflow = Some(workflow_id);
        resources.revision = Some(revision_id);
        resources.layout_owner = Some(LayoutOwner::Shared);
        self.authorize(
            session,
            AuthorityOperation::ReadLayout,
            resources,
            "read:layout",
        )?;
        self.persistent
            .document
            .layouts
            .get(&layout_key(workflow, revision))
            .cloned()
            .ok_or_else(not_found)
    }

    fn inspect_control(
        &self,
        session: &ActorSession,
        command: ControlCommand,
        expected_sequence: Option<u64>,
        suffix: &str,
    ) -> Result<ControlResult, PublicFailure> {
        let seed = format!("{}:{suffix}:{}", session.actor.as_str(), unix_millis());
        let digest = blake3::hash(seed.as_bytes());
        let document = ControlCommandDocument::new(
            ControlId::new(format!("query-{}", &digest.to_hex().as_str()[..32]))
                .map_err(public_control)?,
            session.context.clone(),
            TimestampMillis::new(unix_millis()),
            OptimisticGuard {
                expected_run_sequence: expected_sequence.map(RunSequence::new),
                expected_revision: None,
                expected_proposal_digest: None,
            },
            Reason::new("authenticated control query").map_err(public_persistence)?,
            Vec::new(),
            command,
        )
        .map_err(public_control)?;
        self.control.execute(&document).map_err(public_control)
    }

    fn record_security_decision(
        &mut self,
        decision: &AuthorityDecisionSnapshot,
    ) -> Result<(), PublicFailure> {
        let request = decision.request();
        let operation = serde_json::to_value(request.operation)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(internal)?;
        let mut resource_hasher = blake3::Hasher::new();
        resource_hasher.update(b"milkdrift.audit-resource.v1\0");
        resource_hasher.update(format!("{:?}", request.resources).as_bytes());
        let sequence = self.persistent.document.next_audit_sequence.max(1);
        self.persistent.document.next_audit_sequence = sequence.saturating_add(1);
        self.persistent.document.audit.push(SecurityDecisionRecord {
            sequence,
            evaluated_at_ms: request.evaluated_at.get(),
            actor: request.actor.as_str().to_owned(),
            grant_id: request.grant.as_str().to_owned(),
            grant_revision: request.grant_revision,
            grant_digest: request.grant_digest.as_str().to_owned(),
            operation,
            resource_digest: format!("b3_{}", resource_hasher.finalize()),
            decision_digest: decision.digest().to_owned(),
            outcome: snake_debug(&decision.outcome()),
            reason_codes: decision.reason_codes().iter().map(snake_debug).collect(),
        });
        let bound = usize::try_from(self.persistent.command_bound).unwrap_or(usize::MAX);
        if self.persistent.document.audit.len() > bound {
            let excess = self.persistent.document.audit.len().saturating_sub(bound);
            self.persistent.document.audit.drain(..excess);
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LedgerEntry {
    fingerprint: String,
    result: CommandAccepted,
    #[serde(default)]
    proposal: Option<ProposalLedgerRef>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProposalLedgerRef {
    run: String,
    proposal: String,
    revision: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SecurityDecisionRecord {
    sequence: u64,
    evaluated_at_ms: u64,
    actor: String,
    grant_id: String,
    grant_revision: u64,
    grant_digest: String,
    operation: String,
    resource_digest: String,
    decision_digest: String,
    outcome: String,
    reason_codes: Vec<String>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LocalStateDocument {
    schema_version: u32,
    layouts: BTreeMap<String, LayoutDocument>,
    commands: BTreeMap<String, LedgerEntry>,
    #[serde(default)]
    next_audit_sequence: u64,
    #[serde(default)]
    audit: Vec<SecurityDecisionRecord>,
}

struct LocalState {
    path: PathBuf,
    command_bound: u32,
    document: LocalStateDocument,
}

impl LocalState {
    fn load(path: PathBuf, command_bound: u32) -> Result<Self, String> {
        let document = match fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 64 * 1024 * 1024 {
                    return Err("control state exceeds 64 MiB safety bound".to_owned());
                }
                let value = milkdrift_contracts::parse_json_without_duplicates(&bytes)
                    .map_err(|error| format!("control state JSON failed verification: {error}"))?;
                let document: LocalStateDocument = serde_json::from_value(value)
                    .map_err(|error| format!("control state failed decoding: {error}"))?;
                if document.schema_version != LOCAL_STATE_SCHEMA_VERSION {
                    return Err("control state schema version is unsupported".to_owned());
                }
                if document.commands.len() > usize::try_from(command_bound).unwrap_or(usize::MAX) {
                    return Err("control command ledger exceeds configured bound".to_owned());
                }
                if document.audit.len() > usize::try_from(command_bound).unwrap_or(usize::MAX) {
                    return Err("security decision audit exceeds configured bound".to_owned());
                }
                for layout in document.layouts.values() {
                    layout
                        .validate()
                        .map_err(|error| format!("stored layout failed verification: {error}"))?;
                }
                document
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => LocalStateDocument {
                schema_version: LOCAL_STATE_SCHEMA_VERSION,
                ..LocalStateDocument::default()
            },
            Err(error) => return Err(format!("control state read failed: {:?}", error.kind())),
        };
        Ok(Self {
            path,
            command_bound,
            document,
        })
    }

    fn flush(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec(&self.document)
            .map_err(|_| "control state encoding failed".to_owned())?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("control state exceeds 64 MiB safety bound".to_owned());
        }
        let temporary = self.path.with_extension("json.tmp");
        let mut file = File::create(&temporary).map_err(|error| {
            format!("control state temporary create failed: {:?}", error.kind())
        })?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("control state flush failed: {:?}", error.kind()))?;
        fs::rename(&temporary, &self.path)
            .map_err(|error| format!("control state publish failed: {:?}", error.kind()))?;
        if let Some(parent) = self.path.parent() {
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| {
                    format!("control state directory flush failed: {:?}", error.kind())
                })?;
        }
        Ok(())
    }
}

struct StaticContexts(BTreeMap<String, milkdrift_control::ActorAuthorityContext>);

impl AuthorityContextResolver for StaticContexts {
    fn resolve(
        &self,
        reference: &AuthorityContextRef,
    ) -> Result<milkdrift_control::ActorAuthorityContext, ControlError> {
        self.0
            .get(reference.as_str())
            .cloned()
            .ok_or_else(|| ControlError::AuthorizationDenied {
                reasons: vec![milkdrift_authority::DecisionReasonCode::GrantNotFound],
                decision_digest: None,
            })
    }
}

struct ResultSink {
    data: Arc<dyn InvocationDataAccess>,
}

impl ControlResultSink for ResultSink {
    fn publish(
        &self,
        invocation: &AdapterInvocation<'_>,
        bytes: &[u8],
    ) -> Result<milkdrift_capability::ArtifactReference, ControlError> {
        let context = invocation.context().ok_or_else(|| {
            ControlError::InvalidContract(
                "control result publication requires durable context".to_owned(),
            )
        })?;
        self.data
            .publish_bytes(
                context,
                invocation.request(),
                "control_result",
                "application/vnd.milkdrift.control-result+json",
                bytes,
                MaterializationLimits {
                    max_files: 4,
                    max_file_bytes: 1_310_720,
                    max_total_bytes: 2_621_440,
                    max_path_bytes: 256,
                    max_directory_depth: 8,
                    chunk_bytes: 262_144,
                }
                .validate()
                .map_err(|error| ControlError::InvalidContract(error.to_string()))?,
            )
            .map_err(|error| ControlError::InvalidContract(error.to_string()))
    }
}

fn register_control(
    host: &CapabilityHost,
    control: Arc<ControlService>,
    contexts: BTreeMap<String, milkdrift_control::ActorAuthorityContext>,
    data: Arc<dyn InvocationDataAccess>,
) -> Result<(), String> {
    let adapter = Arc::new(WorkflowControlAdapter::new(
        control,
        Arc::new(StaticContexts(contexts)),
        Arc::new(ResultSink { data }),
    ));
    let descriptor = workflow_control_descriptor().map_err(|error| error.to_string())?;
    let capability = descriptor.identity().clone();
    let revision = descriptor.descriptor_revision();
    host.register(descriptor, adapter, None)
        .map_err(|error| error.to_string())?;
    host.refresh_health(&capability, revision, unix_millis())
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn register_configured_adapters(
    config: &ValidatedDaemonConfig,
    host: &CapabilityHost,
    data: Arc<dyn InvocationDataAccess>,
    secrets: Arc<ConfiguredSecretResolver>,
) -> Result<(), String> {
    for path in &config.document.adapters.process_profiles {
        let bytes = fs::read(path)
            .map_err(|error| format!("process profile read failed: {:?}", error.kind()))?;
        let profile = ProcessProfileDocument::from_json(&bytes)
            .map_err(|error| error.to_string())?
            .into_profile();
        let descriptor = profile.descriptor().map_err(|error| error.to_string())?;
        let adapter = Arc::new(
            LocalProcessAdapter::new(profile, data.clone(), secrets.clone())
                .map_err(|error| error.to_string())?,
        );
        let capability = descriptor.identity().clone();
        let revision = descriptor.descriptor_revision();
        host.register(descriptor, adapter, None)
            .map_err(|error| error.to_string())?;
        host.refresh_health(&capability, revision, unix_millis())
            .map_err(|error| error.to_string())?;
    }
    for configured in &config.document.adapters.model_profiles {
        let bytes = fs::read(&configured.profile)
            .map_err(|error| format!("model profile read failed: {:?}", error.kind()))?;
        let profile = EndpointProfile::from_json(&bytes).map_err(|error| error.to_string())?;
        let capability = CapabilityId::new(configured.capability_id.clone())
            .map_err(|error| error.to_string())?;
        let descriptor = descriptor_for_profile(capability.clone(), &profile)
            .map_err(|error| error.to_string())?;
        let adapter = Arc::new(
            ModelEndpointAdapter::new(capability, profile, secrets.clone(), data.clone())
                .map_err(|error| error.to_string())?,
        );
        let capability = descriptor.identity().clone();
        let revision = descriptor.descriptor_revision();
        host.register(descriptor, adapter, None)
            .map_err(|error| error.to_string())?;
        host.refresh_health(&capability, revision, unix_millis())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn build_peer_runtime(
    config: &ValidatedDaemonConfig,
    host: &CapabilityHost,
    secrets: Arc<ConfiguredSecretResolver>,
) -> Result<PeerRuntime, String> {
    if !config.document.peers.enabled {
        return Ok(PeerRuntime {
            service: None,
            registries: BTreeMap::new(),
        });
    }
    let local_peer = PeerId::new(
        config
            .document
            .peers
            .local_peer_id
            .clone()
            .ok_or_else(|| "enabled peer support lacks local identity".to_owned())?,
    )
    .map_err(|error| error.to_string())?;
    let mut session_hasher = blake3::Hasher::new();
    session_hasher.update(b"milkdrift.peer.session.v1\0");
    session_hasher.update(local_peer.as_str().as_bytes());
    session_hasher.update(&unix_millis().to_be_bytes());
    let session = SessionId::new(format!("session:{}", session_hasher.finalize().to_hex()))
        .map_err(|error| error.to_string())?;
    let versions = ProtocolVersionRange::new(
        ProtocolVersion { major: 1, minor: 0 },
        ProtocolVersion { major: 1, minor: 0 },
    )
    .map_err(|error| error.to_string())?;
    let mut relationships = Vec::new();
    let mut clients = Vec::new();
    let mut authentication = Vec::new();
    for configured in &config.document.peers.relationships {
        let reference =
            SecretRef::new(configured.credential_ref.clone()).map_err(|error| error.to_string())?;
        let credential = Arc::new(
            secrets
                .resolve(&reference)
                .map_err(|error| error.to_string())?,
        );
        let credential_source = Arc::new(ConfiguredPeerCredential {
            resolver: secrets.clone(),
            reference: reference.clone(),
        });
        let remote_peer =
            PeerId::new(configured.peer_id.clone()).map_err(|error| error.to_string())?;
        let relationship_versions = ProtocolVersionRange::new(
            ProtocolVersion {
                major: 1,
                minor: configured.minimum_minor,
            },
            ProtocolVersion {
                major: 1,
                minor: configured.maximum_minor,
            },
        )
        .map_err(|error| error.to_string())?;
        let capability_allow = configured
            .capability_allow
            .iter()
            .cloned()
            .map(CapabilityId::new)
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|error| error.to_string())?;
        let capability_deny = configured
            .capability_deny
            .iter()
            .cloned()
            .map(CapabilityId::new)
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|error| error.to_string())?;
        let operation_allow = configured
            .operation_allow
            .iter()
            .cloned()
            .map(milkdrift_capability::OperationId::new)
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|error| error.to_string())?;
        let maximum_side_effect = match configured.maximum_side_effect {
            PeerSideEffectConfig::None => SideEffectClass::None,
            PeerSideEffectConfig::ReadOnly => SideEffectClass::ReadOnly,
            PeerSideEffectConfig::IdempotentWrite => SideEffectClass::IdempotentWrite,
            PeerSideEffectConfig::NonIdempotentWrite => SideEffectClass::NonIdempotentWrite,
            PeerSideEffectConfig::Unknown => SideEffectClass::Unknown,
        };
        let relationship = PeerRelationship {
            remote_peer: remote_peer.clone(),
            bearer_credential: credential.clone(),
            versions: relationship_versions,
            authority: PeerAuthority {
                actions: configured.actions.clone(),
            },
            capability_allow,
            capability_deny,
            operation_allow,
            maximum_side_effect,
            execution_limits: ExecutionLimits {
                artifact_bytes: configured.maximum_artifact_bytes,
                duration_ms: configured.maximum_duration_ms,
                cost_micros: configured.maximum_cost_micros,
                observations: configured.maximum_observations,
            },
            maximum_concurrent: configured.maximum_concurrent,
            maximum_requests_per_minute: configured.maximum_requests_per_minute,
            maximum_artifact_bytes: configured.maximum_artifact_bytes,
            catalog_ttl_ms: configured.catalog_ttl_ms,
            trust_zone: milkdrift_capability::TrustZone::new(configured.trust_zone.clone())
                .map_err(|error| error.to_string())?,
            delegation: DelegationRef::new(configured.delegation_ref.clone())
                .map_err(|error| error.to_string())?,
            revocation_generation: configured.revocation_generation,
            expires_at_unix_ms: configured.expires_at_unix_ms,
            enabled: configured.enabled,
        };
        relationship.validate().map_err(|error| error.to_string())?;
        let endpoint = url::Url::parse(&configured.endpoint).map_err(|error| error.to_string())?;
        let client = PeerHttpClient::new_with_credential_source(
            PeerClientConfig {
                endpoint,
                local_peer: local_peer.clone(),
                expected_remote_peer: remote_peer,
                session: session.clone(),
                versions: relationship_versions,
                bearer_credential: credential,
                insecure_loopback: if configured.insecure_loopback_development {
                    InsecureLoopbackMode::AllowInsecureLoopbackDevelopment
                } else {
                    InsecureLoopbackMode::Disabled
                },
                request_timeout: Duration::from_secs(30),
                observation_poll_interval: Duration::from_millis(100),
            },
            credential_source,
        )
        .map_err(|error| error.to_string())?;
        authentication.push(ConfiguredPeerAuthentication {
            peer: relationship.remote_peer.clone(),
            reference,
            enabled: relationship.enabled,
            expires_at_unix_ms: relationship.expires_at_unix_ms,
        });
        clients.push((client, relationship.clone()));
        relationships.push(relationship);
    }
    let service = PeerService::new_with_artifacts_and_authenticator(
        PeerServerConfig {
            local_peer,
            session,
            versions,
            limits: HardLimits::default(),
            lease: HeartbeatLease {
                heartbeat_ms: 5_000,
                idle_timeout_ms: 20_000,
                execution_lease_ms: config.document.runtime.lease_duration_ms,
            },
            relationships,
        },
        host.clone(),
        Arc::new(
            FilePeerExecutionStore::open(config.document.data_root.join("peer-executions-v1"))
                .map_err(|error| error.to_string())?,
        ),
        Arc::new(
            FilePeerArtifactStore::open(config.document.data_root.join("peer-artifacts-v1"))
                .map_err(|error| error.to_string())?,
        ),
        Some(Arc::new(ConfiguredPeerAuthenticator {
            resolver: secrets,
            relationships: authentication,
        })),
        Arc::new(SystemPeerClock),
    )
    .map_err(|error| error.to_string())?;
    let mut registries = BTreeMap::new();
    for (client, relationship) in clients {
        let peer = relationship.remote_peer.clone();
        let registry = Arc::new(
            PeerRegistry::new(host.clone(), client, relationship)
                .map_err(|error| error.to_string())?,
        );
        registries.insert(peer, registry);
    }
    Ok(PeerRuntime {
        service: Some(service),
        registries,
    })
}

struct ConfiguredPeerCredential {
    resolver: Arc<ConfiguredSecretResolver>,
    reference: SecretRef,
}

impl PeerCredentialSource for ConfiguredPeerCredential {
    fn resolve(&self) -> Result<milkdrift_authority::SensitiveSecret, PeerHttpError> {
        self.resolver.resolve(&self.reference).map_err(|_| {
            PeerHttpError::Unavailable("peer credential source unavailable".to_owned())
        })
    }
}

struct ConfiguredPeerAuthentication {
    peer: PeerId,
    reference: SecretRef,
    enabled: bool,
    expires_at_unix_ms: u64,
}

struct ConfiguredPeerAuthenticator {
    resolver: Arc<ConfiguredSecretResolver>,
    relationships: Vec<ConfiguredPeerAuthentication>,
}

impl PeerAuthenticator for ConfiguredPeerAuthenticator {
    fn authenticate(&self, supplied: &[u8], now_unix_ms: u64) -> Option<PeerId> {
        self.relationships
            .iter()
            .filter(|relationship| {
                relationship.enabled && now_unix_ms <= relationship.expires_at_unix_ms
            })
            .find(|relationship| {
                self.resolver
                    .resolve(&relationship.reference)
                    .ok()
                    .is_some_and(|expected| {
                        expected.expose(|bytes| {
                            bytes.len() == supplied.len() && bool::from(bytes.ct_eq(supplied))
                        })
                    })
            })
            .map(|relationship| relationship.peer.clone())
    }
}

fn default_workspace_budget() -> Result<WorkspaceBudget, milkdrift_workspace::WorkspaceError> {
    WorkspaceBudget::new(
        10_000,
        1_048_576,
        64 * 1_048_576,
        10_000,
        64 * 1_048_576,
        10 * 1_073_741_824,
    )
}

fn evidence(request: &CommandRequest) -> Result<Vec<EvidenceReference>, PublicFailure> {
    request
        .evidence
        .iter()
        .map(|item| {
            let kind = match item.kind.as_str() {
                "authority_decision" => EvidenceKind::AuthorityDecision,
                "worker_observation" => EvidenceKind::WorkerObservation,
                "external_receipt" => EvidenceKind::ExternalReceipt,
                "artifact" => EvidenceKind::Artifact,
                "recovery_observation" => EvidenceKind::RecoveryObservation,
                _ => return Err(invalid("unsupported evidence kind")),
            };
            Ok(EvidenceReference {
                id: EvidenceId::new(item.id.clone()).map_err(public_persistence)?,
                kind,
            })
        })
        .collect()
}

fn internal_control_id(
    session: &ActorSession,
    request: &CommandRequest,
    suffix: &str,
) -> Result<ControlId, PublicFailure> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.daemon-control-id.v1\0");
    hasher.update(session.actor.as_str().as_bytes());
    hasher.update(request.command_id.as_bytes());
    hasher.update(suffix.as_bytes());
    ControlId::new(format!(
        "api-{}",
        &hasher.finalize().to_hex().as_str()[..32]
    ))
    .map_err(public_control)
}

fn command_fingerprint(
    session: &ActorSession,
    request: &CommandRequest,
) -> Result<String, PublicFailure> {
    let bytes = milkdrift_control_protocol::encode_json(request).map_err(public_protocol)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.daemon-command.v1\0");
    hasher.update(session.actor.as_str().as_bytes());
    hasher.update(session.grant.identity().as_str().as_bytes());
    hasher.update(&session.grant.revision().to_le_bytes());
    hasher.update(
        session
            .grant
            .digest()
            .map_err(|_| internal())?
            .as_str()
            .as_bytes(),
    );
    hasher.update(&bytes);
    Ok(format!("b3_{}", hasher.finalize()))
}

fn proposal_ledger_ref(
    command: &Command,
    result: &CommandAccepted,
) -> Result<Option<ProposalLedgerRef>, PublicFailure> {
    let Command::SubmitProposal { document } = command else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec(document).map_err(|_| invalid("invalid proposal JSON"))?;
    let proposal = WorkflowProposalDocument::from_json(&bytes)
        .map_err(|error| invalid(&bounded(&error.to_string())))?;
    let Some(run) = proposal.proposal().run() else {
        return Ok(None);
    };
    let revision = result
        .value
        .get("proposed_revision")
        .and_then(Value::as_str)
        .ok_or_else(internal)?;
    Ok(Some(ProposalLedgerRef {
        run: run.as_str().to_owned(),
        proposal: proposal.proposal().identity().as_str().to_owned(),
        revision: revision.to_owned(),
    }))
}

fn accepted_sequence(
    request: &CommandRequest,
    sequence: u64,
    kind: &str,
) -> Result<CommandAccepted, PublicFailure> {
    Ok(CommandAccepted {
        command_id: request.command_id.clone(),
        replayed: false,
        resulting_sequence: Some(sequence),
        result_type: kind.to_owned(),
        value: json!({"resulting_sequence": sequence}),
    })
}

fn map_resolve(action: ResolveAction) -> ExternalWorkAction {
    match action {
        ResolveAction::Query => ExternalWorkAction::Query,
        ResolveAction::Retry => ExternalWorkAction::Retry,
        ResolveAction::Compensate => ExternalWorkAction::Compensate,
        ResolveAction::Retain => ExternalWorkAction::Retain,
        ResolveAction::ResolveSucceeded => ExternalWorkAction::ResolveSucceeded,
        ResolveAction::ResolveFailed => ExternalWorkAction::ResolveFailed,
    }
}

fn public_revision_summary(
    value: &milkdrift_persistence::RevisionSummary,
) -> PublicRevisionSummary {
    PublicRevisionSummary {
        revision_id: value.revision.as_str().to_owned(),
        workflow_id: value.workflow.as_str().to_owned(),
        lineage_sequence: value.lineage_sequence,
        semantic_digest: value.content_digest.as_str().to_owned(),
        parents: value
            .parents
            .iter()
            .map(|parent| parent.as_str().to_owned())
            .collect(),
    }
}

fn public_run(value: milkdrift_control::RunInspection) -> RunRead {
    let (lifecycle, terminal) = match value.lifecycle {
        milkdrift_runtime::RunLifecycle::Uncreated => ("uncreated".to_owned(), None),
        milkdrift_runtime::RunLifecycle::Created => ("created".to_owned(), None),
        milkdrift_runtime::RunLifecycle::Running => ("running".to_owned(), None),
        milkdrift_runtime::RunLifecycle::Paused => ("paused".to_owned(), None),
        milkdrift_runtime::RunLifecycle::Cancelling => ("cancelling".to_owned(), None),
        milkdrift_runtime::RunLifecycle::Terminal(outcome) => {
            ("terminal".to_owned(), Some(snake_debug(&outcome)))
        }
    };
    let nodes = value
        .executions
        .into_iter()
        .map(|node| NodeRead {
            execution_id: node.execution.as_str().to_owned(),
            node_id: node.node.as_str().to_owned(),
            revision_id: node.revision.as_str().to_owned(),
            state: snake_debug(&node.state),
            attempt_count: node.attempt_count,
            latest_attempt: node.latest_attempt.map(public_attempt),
        })
        .collect::<Vec<_>>();
    let uncertainty_count = u32::try_from(
        nodes
            .iter()
            .filter(|node| {
                node.latest_attempt
                    .as_ref()
                    .is_some_and(|attempt| attempt.uncertain)
            })
            .count(),
    )
    .unwrap_or(u32::MAX);
    RunRead {
        run_id: value.run.as_str().to_owned(),
        sequence: value.sequence.get(),
        lifecycle,
        terminal,
        workflow_id: value.workflow.map(|workflow| workflow.as_str().to_owned()),
        revision_id: value.revision.map(|revision| revision.as_str().to_owned()),
        semantic_digest: value
            .revision_digest
            .map(|digest| digest.as_str().to_owned()),
        nodes,
        uncertainty_count,
    }
}

fn public_attempt(value: milkdrift_control::AttemptInspection) -> AttemptRead {
    let capability_id = value
        .capability
        .as_ref()
        .map(|capability| capability.capability().as_str().to_owned());
    let context_manifest = value.context_manifest.map(|artifact| ArtifactMetadataRead {
        artifact_id: artifact.identity().to_owned(),
        digest: artifact.digest().to_owned(),
        size: artifact.size_bytes().unwrap_or(0),
        content_type: artifact
            .media_type()
            .unwrap_or("application/octet-stream")
            .to_owned(),
        disposition_name: None,
        sensitivity: "restricted".to_owned(),
    });
    AttemptRead {
        attempt_id: value.attempt.as_str().to_owned(),
        invocation_id: value
            .invocation
            .map(|invocation| invocation.as_str().to_owned()),
        state: snake_debug(&value.state),
        capability_id,
        context_manifest,
        terminal: value.terminal.as_ref().map(snake_debug),
        uncertain: value.external_outcome.is_some(),
    }
}

fn public_timeline(event: &milkdrift_persistence::RunEventEnvelope) -> TimelineEntry {
    let kind = serde_json::to_value(event.kind()).unwrap_or(Value::Null);
    let kind_name = kind
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("runtime_fact");
    let category = timeline_category(kind_name);
    let actor = kind
        .get("actor")
        .and_then(Value::as_str)
        .unwrap_or("service:runtime")
        .to_owned();
    let node_id = string_field(&kind, &["node", "node_id"]);
    let attempt_id = string_field(&kind, &["attempt", "attempt_id"]);
    let revision_id = string_field(&kind, &["revision", "to_revision", "from_revision"]);
    TimelineEntry {
        sequence: event.sequence().get(),
        timestamp_ms: event.occurred_at().get(),
        category,
        actor,
        run_id: event.run_id().as_str().to_owned(),
        node_id,
        attempt_id,
        revision_id,
        summary: timeline_summary(category),
        detail: json!({"event_id": event.event_id().as_str()}),
    }
}

fn timeline_category(kind: &str) -> TimelineCategory {
    if kind.contains("artifact") || kind.contains("output") {
        TimelineCategory::Artifact
    } else if kind.contains("reconciliation") || kind.contains("revision_adoption") {
        TimelineCategory::Reconciliation
    } else if kind.contains("recovery") || kind.contains("re_leased") {
        TimelineCategory::Recovery
    } else if kind.contains("uncertain")
        || kind.contains("retained")
        || kind.contains("late_terminal")
    {
        TimelineCategory::Uncertainty
    } else if kind.contains("signal") || kind.contains("timer") || kind.contains("wait") {
        TimelineCategory::Coordination
    } else if kind.contains("decision") || kind.contains("authority") {
        TimelineCategory::Authority
    } else if kind.contains("node") || kind.contains("lease") || kind.contains("attempt") {
        TimelineCategory::Execution
    } else if kind.contains("progress") || kind.contains("usage") {
        TimelineCategory::Progress
    } else {
        TimelineCategory::Lifecycle
    }
}

fn timeline_summary(category: TimelineCategory) -> String {
    match category {
        TimelineCategory::Lifecycle => "run lifecycle changed",
        TimelineCategory::Execution => "node execution changed",
        TimelineCategory::Progress => "execution progress observed",
        TimelineCategory::Artifact => "artifact or output published",
        TimelineCategory::Coordination => "workflow coordination changed",
        TimelineCategory::Authority => "authority decision recorded",
        TimelineCategory::Recovery => "recovery fact recorded",
        TimelineCategory::Reconciliation => "revision reconciliation changed",
        TimelineCategory::Uncertainty => "external outcome requires attention",
    }
    .to_owned()
}

fn string_field(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str).map(str::to_owned))
}

fn public_artifact_metadata(value: &milkdrift_workspace::ArtifactMetadata) -> ArtifactMetadataRead {
    ArtifactMetadataRead {
        artifact_id: value.reference().artifact().as_str().to_owned(),
        digest: value.reference().digest().to_hex(),
        size: value.reference().size_bytes(),
        content_type: value.reference().media_type().as_str().to_owned(),
        disposition_name: None,
        sensitivity: snake_debug(&value.sensitivity()),
    }
}

fn diff_keys<K, V>(
    subject: &str,
    left: &BTreeMap<K, V>,
    right: &BTreeMap<K, V>,
    output: &mut Vec<RevisionChange>,
) where
    K: Ord + ToString,
    V: PartialEq,
{
    for key in left.keys().chain(right.keys()).collect::<BTreeSet<_>>() {
        let change = match (left.get(key), right.get(key)) {
            (None, Some(_)) => "added",
            (Some(_), None) => "removed",
            (Some(left), Some(right)) if left != right => "changed",
            _ => continue,
        };
        output.push(RevisionChange {
            change: change.to_owned(),
            subject: subject.to_owned(),
            identity: Some(key.to_string()),
            detail: Value::Null,
        });
    }
}

fn parse_run_state(value: &str) -> Result<IndexedRunState, PublicFailure> {
    match value {
        "created" => Ok(IndexedRunState::Created),
        "runnable" => Ok(IndexedRunState::Runnable),
        "active" => Ok(IndexedRunState::Active),
        "paused" => Ok(IndexedRunState::Paused),
        "cancelling" => Ok(IndexedRunState::Cancelling),
        "waiting" => Ok(IndexedRunState::Waiting),
        "uncertain" => Ok(IndexedRunState::Uncertain),
        "terminal" => Ok(IndexedRunState::Terminal),
        _ => Err(invalid("unknown run state filter")),
    }
}

fn parse_revision_id(value: &str) -> Result<RevisionId, PublicFailure> {
    serde_json::from_value(Value::String(value.to_owned()))
        .map_err(|error| invalid(&error.to_string()))
}

fn layout_key(workflow: &str, revision: &str) -> String {
    format!("{workflow}\0{revision}")
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn snake_debug(value: &impl std::fmt::Debug) -> String {
    let source = format!("{value:?}");
    let mut result = String::with_capacity(source.len() + 4);
    for (index, character) in source.chars().enumerate() {
        if character.is_ascii_uppercase() && index != 0 {
            result.push('_');
        }
        result.extend(character.to_lowercase());
    }
    result
}

fn bounded(value: &str) -> String {
    if value.len() <= 4_096 {
        return value.to_owned();
    }
    let mut end = 4_096;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn cursor_binding(
    session: &ActorSession,
    exact_resource_and_filter: &str,
) -> Result<CursorBinding, PublicFailure> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.continuation-scope.v1\0");
    hasher.update(exact_resource_and_filter.as_bytes());
    Ok(CursorBinding {
        actor: session.actor.as_str().to_owned(),
        grant_id: session.grant.identity().as_str().to_owned(),
        grant_revision: session.grant.revision(),
        grant_digest: session
            .grant
            .digest()
            .map_err(|_| internal())?
            .as_str()
            .to_owned(),
        scope_digest: format!("b3_{}", hasher.finalize()),
    })
}

fn invalid(message: &str) -> PublicFailure {
    PublicFailure::new(ErrorCode::InvalidInput, bounded(message), false)
}

fn conflict(message: &str) -> PublicFailure {
    PublicFailure::new(ErrorCode::Conflict, message, false)
}

fn unauthorized() -> PublicFailure {
    PublicFailure::new(
        ErrorCode::Unauthorized,
        "authority denied the operation",
        false,
    )
}

fn unauthorized_decision(decision: &AuthorityDecisionSnapshot) -> PublicFailure {
    let mut failure = unauthorized();
    failure
        .details
        .insert("decision_digest".to_owned(), decision.digest().to_owned());
    failure.details.insert(
        "reason_codes".to_owned(),
        decision
            .reason_codes()
            .iter()
            .map(snake_debug)
            .collect::<Vec<_>>()
            .join(","),
    );
    failure
}

fn not_found() -> PublicFailure {
    PublicFailure::new(
        ErrorCode::NotFound,
        "requested resource was not found",
        false,
    )
}

fn internal() -> PublicFailure {
    PublicFailure::new(
        ErrorCode::Internal,
        "internal control operation failed",
        false,
    )
}

fn public_protocol(error: milkdrift_control_protocol::ProtocolError) -> PublicFailure {
    match error {
        milkdrift_control_protocol::ProtocolError::UnsupportedMajor { .. } => PublicFailure::new(
            ErrorCode::UnsupportedVersion,
            bounded(&error.to_string()),
            false,
        ),
        milkdrift_control_protocol::ProtocolError::Bounds(_) => {
            PublicFailure::new(ErrorCode::Overload, bounded(&error.to_string()), false)
        }
        _ => invalid(&error.to_string()),
    }
}

fn public_control(error: ControlError) -> PublicFailure {
    match error {
        ControlError::AuthorizationDenied {
            reasons,
            decision_digest,
        } => {
            let mut failure = unauthorized();
            if let Some(digest) = decision_digest {
                failure.details.insert("decision_digest".to_owned(), digest);
            }
            failure.details.insert(
                "reason_codes".to_owned(),
                reasons
                    .iter()
                    .map(snake_debug)
                    .collect::<Vec<_>>()
                    .join(","),
            );
            failure
        }
        ControlError::StaleRunSequence { expected, actual } => {
            let mut failure = conflict("run sequence guard is stale");
            failure
                .details
                .insert("expected_sequence".to_owned(), expected.get().to_string());
            failure
                .details
                .insert("actual_sequence".to_owned(), actual.get().to_string());
            failure
        }
        ControlError::ApprovalRequired { .. }
        | ControlError::ProposalState(_)
        | ControlError::BaseRevisionMismatch => conflict(&bounded(&error.to_string())),
        ControlError::BaseRevisionNotFound => not_found(),
        ControlError::UnsupportedVersion { .. } => PublicFailure::new(
            ErrorCode::UnsupportedVersion,
            bounded(&error.to_string()),
            false,
        ),
        ControlError::Persistence(error) => public_persistence(error),
        ControlError::Runtime(milkdrift_runtime::RuntimeError::Persistence(error)) => {
            public_persistence(error)
        }
        ControlError::Runtime(milkdrift_runtime::RuntimeError::AuthorizationDenied { .. }) => {
            unauthorized()
        }
        ControlError::Runtime(error) if error.to_string().contains("transition") => {
            conflict(&bounded(&error.to_string()))
        }
        _ => invalid(&bounded(&error.to_string())),
    }
}

fn public_persistence(error: PersistenceError) -> PublicFailure {
    match error {
        PersistenceError::SequenceConflict {
            expected, actual, ..
        } => {
            let mut failure = conflict("run sequence guard is stale");
            failure
                .details
                .insert("expected_sequence".to_owned(), expected.get().to_string());
            failure
                .details
                .insert("actual_sequence".to_owned(), actual.get().to_string());
            failure
        }
        PersistenceError::IdempotencyConflict { .. }
        | PersistenceError::ImmutableConflict { .. }
        | PersistenceError::WorkspaceUsageConflict { .. }
        | PersistenceError::LeaseRevisionConflict { .. } => conflict(&bounded(&error.to_string())),
        PersistenceError::NotFound { .. } => not_found(),
        PersistenceError::ArtifactAccessDenied(_) => unauthorized(),
        PersistenceError::Corruption(_) => PublicFailure::new(
            ErrorCode::Corruption,
            "durable integrity verification failed",
            false,
        ),
        PersistenceError::UnsupportedVersion { .. }
        | PersistenceError::MigrationRequired { .. } => PublicFailure::new(
            ErrorCode::UnsupportedVersion,
            bounded(&error.to_string()),
            false,
        ),
        PersistenceError::Storage { class, .. } => {
            let code = match class {
                milkdrift_persistence::StorageFailureClass::Corruption => ErrorCode::Corruption,
                milkdrift_persistence::StorageFailureClass::ResourceExhausted => {
                    ErrorCode::Overload
                }
                milkdrift_persistence::StorageFailureClass::Unavailable
                | milkdrift_persistence::StorageFailureClass::OwnerBusy => ErrorCode::Unavailable,
                milkdrift_persistence::StorageFailureClass::Migration => {
                    ErrorCode::UnsupportedVersion
                }
                milkdrift_persistence::StorageFailureClass::Internal => ErrorCode::Internal,
            };
            PublicFailure::new(
                code,
                "durable storage operation failed",
                matches!(code, ErrorCode::Unavailable | ErrorCode::Overload),
            )
        }
        _ => invalid(&bounded(&error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_projection_never_serializes_internal_event_body() {
        assert_eq!(
            timeline_summary(TimelineCategory::Execution),
            "node execution changed"
        );
        assert!(!timeline_summary(TimelineCategory::Execution).contains("NodeScheduled"));
    }

    #[test]
    fn daemon_state_key_keeps_layout_outside_revision_identity() {
        assert_ne!(
            layout_key("workflow", "revision-a"),
            layout_key("workflow", "revision-b")
        );
    }
}
