use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
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
use milkdrift_blueprint::{AuthorRef, BlueprintRevisionDocument, RevisionId, WorkflowId};
use milkdrift_capability::{CapabilityId, ErrorClass, SideEffectClass};
use milkdrift_capability_host::{
    AdapterInvocation, CapabilityHost, CapabilitySelectionPolicy, EffectShutdownMode,
    EffectWorkerConfig, EffectWorkerHost, HostConfig, InvocationDataAccess, MaterializationLimits,
    SecretResolver, StoreInvocationDataAccess,
};
use milkdrift_control::{
    ControlCommand, ControlCommandDocument, ControlError, ControlId, ControlResult,
    ControlResultSink, ControlService, OptimisticGuard, ProposalDigest, ProposalId,
    WorkflowControlAdapter, WorkflowProposalDocument, workflow_control_descriptor,
};
use milkdrift_control_protocol::{
    ArtifactMetadataRead, AttemptRead, CapabilityRead, Command, CommandAccepted, CommandRequest,
    ContextManifestRead, Cursor, CursorBinding, DaemonState, ErrorCode, HealthRead, LayoutDocument,
    NodeRead, Page, PeerRead, ProposalDecision, ProposalRead, ResolveAction, RevisionChange,
    RevisionDiffRead, RevisionRead, RevisionSummary as PublicRevisionSummary, RunRead,
    TimelineCategory, TimelineEntry,
};
use milkdrift_local_process::{LocalProcessAdapter, ProcessProfileDocument};
use milkdrift_model_provider::{EndpointProfile, ModelEndpointAdapter, descriptor_for_profile};
use milkdrift_peer_http::{
    CorePeerArtifactStore, InsecureLoopbackMode, PeerAuthenticator, PeerClientConfig,
    PeerCredentialSource, PeerHttpClient, PeerHttpError, PeerRegistry, PeerRelationship,
    PeerServerConfig, PeerService, PeerWorkerConfig, SystemPeerClock,
};
use milkdrift_peer_protocol::{
    DelegationRef, ExecutionLimits, HardLimits, HeartbeatLease, PeerAuthority, ProtocolVersion,
    ProtocolVersionRange, SessionId,
};
use milkdrift_persistence::{
    ApplicationCommandCommit, ApplicationCommandCommitOutcome, ApplicationCommandEffect,
    ApplicationCommandReceipt, ApplicationCommandResult, ApplicationCommandStore,
    ApplicationCursor, ApplicationEffectReference, ApplicationLayoutStore, ApplicationLayoutUpdate,
    ApplicationPageQuery, ArtifactReadAuthority, ArtifactReadRequest, ArtifactStore, AttemptId,
    CommandId, CorrelationKey, EventPageQuery, EvidenceId, EvidenceKind, EvidenceReference,
    IndexedRunState, IntegrityDigest, PageSize, PersistenceError, ProposalIndexEntry,
    ProposalIndexStore, Reason, ReconciliationDecisionId, RevisionCursor, RevisionFilter,
    RevisionPageQuery, RevisionStore, RunEventKind, RunQueryStore, RunSequence, RunSummaryCursor,
    RunSummaryFilter, RunSummaryPageQuery, SecurityAuditEntry, SecurityAuditStore,
    SignalDeliveryMode, SignalId, SignalTypeId, TimestampMillis, WorkerId,
};
use milkdrift_prompt_sequence::{PromptSequenceDocument, compile as compile_prompt_sequence};
use milkdrift_redb_store::{RedbStore, RedbStoreConfig};
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

mod artifacts;
mod attempts;
mod capabilities;
mod commands;
mod definitions;
mod layouts;
mod proposals;
mod read_model;
mod receipts;
mod runs;

use read_model::*;

const OWNER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
const APPLICATION_COMMAND_SCHEMA_VERSION: u32 = 1;
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
    pub(crate) fn begin_draining(&self) -> Result<(), HostError> {
        self.mutating_admission.store(false, Ordering::SeqCst);
        self.health.set_lifecycle(Lifecycle::Draining);
        if let Some(service) = &self.peer_service {
            service
                .begin_drain()
                .map_err(|error| HostError::Shutdown(error.to_string()))?;
        }
        for registry in self.peer_registries.values() {
            let _ = registry.disconnect();
        }
        Ok(())
    }

    /// Returns the optional distinct peer route service for router composition.
    #[must_use]
    pub(crate) fn peer_service(&self) -> Option<Arc<PeerService>> {
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
        self.begin_draining()?;
        let result = match self.request(OwnerOperation::Shutdown, true).await {
            Ok(result) => result,
            Err(error) => {
                self.health.set_lifecycle(Lifecycle::Failed);
                return Err(HostError::Shutdown(error.message));
            }
        };
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
            OwnerValue::Shutdown {
                clean: true,
                unresolved: 0,
            } => Ok(()),
            OwnerValue::Shutdown { clean, unresolved } => {
                self.health.set_lifecycle(Lifecycle::Failed);
                Err(HostError::Shutdown(format!(
                    "shutdown retained or could not resolve {unresolved} invocation(s); clean={clean}"
                )))
            }
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
        unresolved: u32,
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
        if config.document.data_root.join(LEGACY_SIDECAR_FILE).exists() {
            return Err(
                "legacy control-state-v1.json is unsupported; this release refuses sidecar state instead of silently importing or ignoring idempotency truth"
                    .to_owned(),
            );
        }
        for prototype in ["peer-executions-v1", "peer-artifacts-v1"] {
            if config.document.data_root.join(prototype).exists() {
                return Err(format!(
                    "prototype {prototype} storage is unsupported; this release refuses parallel peer authorities instead of partially importing them"
                ));
            }
        }
        let store = Arc::new(
            RedbStore::open_with_config(
                RedbStoreConfig::new(&config.document.data_root).with_application_limits(
                    config.document.command_receipt_bound,
                    config.document.command_receipt_bound,
                ),
            )
            .map_err(|error| error.to_string())?,
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
        runtime
            .recover_startup_closed()
            .map_err(|error| error.to_string())?;
        store
            .application_command_receipts(&ApplicationPageQuery {
                after: None,
                limit: PageSize::new(1).map_err(|error| error.to_string())?,
            })
            .map_err(|error| error.to_string())?;
        store
            .application_layouts(&ApplicationPageQuery {
                after: None,
                limit: PageSize::new(1).map_err(|error| error.to_string())?,
            })
            .map_err(|error| error.to_string())?;
        capabilities::register_control(&capability_host, control.clone(), data.clone())?;
        capabilities::register_configured(&config, &capability_host, data, auth.resolver())?;
        let peer_runtime =
            build_peer_runtime(&config, &capability_host, store.clone(), auth.resolver())?;
        if let Some(service) = &peer_runtime.service {
            service.recover(1_024).map_err(|error| error.to_string())?;
        }
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
        runtime
            .resume_admission()
            .map_err(|error| error.to_string())?;
        health.active_effects.store(0, Ordering::SeqCst);
        Ok(Self {
            config,
            store,
            runtime,
            control,
            capability_host,
            authority,
            effect_workers: Some(effect_workers),
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
            service.begin_shutdown().map_err(|error| {
                PublicFailure::new(ErrorCode::Unavailable, bounded(&error.to_string()), true)
            })?;
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
        let shutdown_started = std::time::Instant::now();
        let peer_shutdown = self
            .peer_service
            .as_ref()
            .map(|service| service.shutdown_workers(deadline));
        let effect_deadline = deadline.saturating_sub(shutdown_started.elapsed());
        let result = self
            .effect_workers
            .take()
            .ok_or_else(|| {
                PublicFailure::new(ErrorCode::Internal, "effect owner is absent", false)
            })?
            .shutdown(mode, effect_deadline)
            .map_err(|error| {
                PublicFailure::new(ErrorCode::Unavailable, bounded(&error.to_string()), true)
            })?;
        health.active_effects.store(0, Ordering::SeqCst);
        health.set_lifecycle(Lifecycle::Stopped);
        let peer_retained = peer_shutdown.map_or(0, |report| report.retained_workers);
        info!(
            phase = "stopped",
            clean = result.clean && peer_retained == 0,
            unresolved = result
                .unresolved_invocations
                .len()
                .saturating_add(usize::from(peer_retained)),
            "runtime owner stopped"
        );
        Ok(OwnerValue::Shutdown {
            clean: result.clean && peer_retained == 0,
            unresolved: u32::try_from(
                result
                    .unresolved_invocations
                    .len()
                    .saturating_add(usize::from(peer_retained)),
            )
            .unwrap_or(u32::MAX),
        })
    }

    fn command(
        &mut self,
        session: &ActorSession,
        request: CommandRequest,
    ) -> Result<CommandAccepted, PublicFailure> {
        receipts::execute(self, session, request)
    }

    fn proposals(
        &self,
        session: &ActorSession,
        run: &str,
        cursor: Option<&Cursor>,
        limit: u32,
    ) -> Result<Page<ProposalRead>, PublicFailure> {
        proposals::page(self, session, run, cursor, limit)
    }

    fn proposal(
        &self,
        session: &ActorSession,
        run: &str,
        proposal: &str,
        revision: &str,
    ) -> Result<ProposalRead, PublicFailure> {
        proposals::exact(self, session, run, proposal, revision)
    }

    fn layout(
        &self,
        session: &ActorSession,
        workflow: &str,
        revision: &str,
    ) -> Result<LayoutDocument, PublicFailure> {
        layouts::read(self, session, workflow, revision)
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
        &self,
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
        self.store
            .append_security_audit(&SecurityAuditEntry {
                evaluated_at: TimestampMillis::new(request.evaluated_at.get()),
                actor: request.actor.clone(),
                grant: request.grant.clone(),
                grant_revision: request.grant_revision,
                grant_digest: request.grant_digest.clone(),
                operation,
                resource_digest: IntegrityDigest::new(format!("b3_{}", resource_hasher.finalize()))
                    .map_err(public_persistence)?,
                decision_digest: decision.digest().to_owned(),
                outcome: snake_debug(&decision.outcome()),
                reason_codes: decision.reason_codes().iter().map(snake_debug).collect(),
            })
            .map_err(public_persistence)?;
        Ok(())
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

fn build_peer_runtime(
    config: &ValidatedDaemonConfig,
    host: &CapabilityHost,
    store: Arc<RedbStore>,
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
            execution_filesystem: configured.execution_filesystem.clone(),
            execution_network_profiles: configured.execution_network_profiles.clone(),
            execution_network_destinations: configured.execution_network_destinations.clone(),
            execution_secrets: configured.execution_secrets.clone(),
            execution_limits: ExecutionLimits {
                artifact_bytes: configured.maximum_artifact_bytes,
                duration_ms: configured.maximum_duration_ms,
                cost_micros: configured.maximum_cost_micros,
                observations: configured.maximum_observations,
            },
            maximum_concurrent: configured.maximum_concurrent,
            maximum_requests_per_minute: configured.maximum_requests_per_minute,
            maximum_artifact_bytes: configured.maximum_artifact_bytes,
            artifact_sensitivities: configured.artifact_sensitivities.clone(),
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
            workers: PeerWorkerConfig {
                threads: config.document.runtime.effect_threads,
                maximum_global_active: config.document.runtime.global_concurrency,
                maximum_dispatch_queue: config.document.runtime.global_concurrency,
                maximum_records: u64::from(config.document.command_receipt_bound)
                    .max(u64::from(config.document.runtime.global_concurrency)),
                recovery_page: config.document.runtime.maximum_effect_claim,
                poll_interval: Duration::from_millis(
                    config.document.runtime.maintenance_interval_ms,
                ),
            },
        },
        host.clone(),
        store.clone(),
        Arc::new(
            CorePeerArtifactStore::new(
                store,
                config
                    .document
                    .peers
                    .relationships
                    .iter()
                    .map(|relationship| relationship.maximum_artifact_bytes)
                    .max()
                    .unwrap_or(1),
                10 * 1_073_741_824,
            )
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
}
