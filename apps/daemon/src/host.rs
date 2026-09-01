use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
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
    ContextManifestRead, Cursor, CursorBinding, ErrorCode, HealthRead, LayoutDocument, NodeRead,
    Page, PeerRead, ProposalDecision, ProposalRead, ResolveAction, RevisionChange,
    RevisionDiffRead, RevisionRead, RevisionSummary as PublicRevisionSummary, RunRead,
    TimelineCategory, TimelineEntry,
};
use milkdrift_local_process::{LocalProcessAdapter, ProcessProfileDocument};
use milkdrift_local_secret::LocalSecretResolver;
use milkdrift_model_provider::{EndpointProfile, ModelEndpointAdapter, descriptor_for_profile};
use milkdrift_peer_http::{
    CorePeerArtifactStore, InsecureLoopbackMode, PeerAuthenticator, PeerClientConfig,
    PeerCredentialSource, PeerHttpClient, PeerHttpError, PeerRegistry, PeerRelationship,
    PeerServerConfig, PeerService, PeerWorkerConfig, PeerWorkerShutdownReport,
};
use milkdrift_peer_protocol::{
    DelegationRef, ExecutionLimits, HardLimits, HeartbeatLease, PROTOCOL_MAJOR_V1, PeerAuthority,
    ProtocolVersion, ProtocolVersionRange, SessionId,
};
use milkdrift_persistence::{
    ApplicationCommandCommit, ApplicationCommandCommitOutcome, ApplicationCommandEffect,
    ApplicationCommandReceipt, ApplicationCommandResult, ApplicationCommandStore,
    ApplicationCursor, ApplicationEffectReference, ApplicationLayoutStore, ApplicationLayoutUpdate,
    ApplicationPageQuery, ApplicationReceiptArchiveRequest, ArtifactReadAuthority,
    ArtifactReadRequest, ArtifactStore, AttemptId, CommandId, CorrelationKey, EventPageQuery,
    EvidenceId, EvidenceKind, EvidenceReference, IndexedRunState, IntegrityDigest, NodeExecutionId,
    PageSize, PersistenceError, ProposalIndexEntry, ProposalIndexStore, Reason,
    ReconciliationDecisionId, RepeatDecisionId, RevisionCursor, RevisionFilter, RevisionPageQuery,
    RevisionStore, RunEventKind, RunQueryStore, RunSequence, RunSummaryCursor, RunSummaryFilter,
    RunSummaryPageQuery, SecurityAuditEntry, SecurityAuditStore, SignalDeliveryMode, SignalId,
    SignalTypeId, TimestampMillis, WorkerId,
};
use milkdrift_prompt_sequence::{PromptSequenceDocument, compile as compile_prompt_sequence};
use milkdrift_redb_store::{RedbStore, RedbStoreConfig};
use milkdrift_runtime::{
    ExternalWorkAction, RetryPolicy, RuntimeConfig, RuntimeService, SchedulerLimits,
    SequentialIdGenerator,
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
    auth::{ActorSession, AuthRegistry},
    config::{
        AdapterConfig, DaemonPlan, DaemonPlanParts, PeerHostConfig, PeerSideEffectConfig,
        RuntimeHostConfig, ShutdownConfig, ShutdownEffectPolicy, StoragePlan,
    },
};

mod artifacts;
mod attempts;
mod capabilities;
mod clock;
mod commands;
mod definitions;
mod health;
mod layouts;
mod peer_store;
mod proposals;
mod queue;
mod read_model;
mod receipts;
mod requests;
mod runs;

use clock::{ArtifactClockAdapter, DaemonClockSource, DurableClock, SystemDaemonClock};
use health::{Lifecycle, QueuedRequestGuard, SharedHealth};
use peer_store::{OwnerPeerArtifactStore, OwnerPeerExecutionStore};
use queue::{OwnerCallFailure, OwnerQueue};
use read_model::{
    accepted_sequence, bounded, clock_unavailable, conflict, corruption, cursor_binding, diff_keys,
    empty_attempt_read, internal, invalid, map_resolve, not_found, parse_revision_id,
    parse_run_state, public_artifact_metadata, public_attempt_usage, public_authority_decision,
    public_capability_provenance, public_control, public_execution_authority,
    public_invocation_artifact, public_persistence, public_protocol, public_revision_summary,
    public_run, public_timeline, snake_debug, unauthorized, unauthorized_decision,
};

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

/// Cloneable daemon handle shared by HTTP route state.
#[derive(Clone)]
pub struct DaemonHost {
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
    pub fn start(config: DaemonPlan) -> Result<Self, HostError> {
        Self::start_with_clock(config, Arc::new(SystemDaemonClock))
    }

    fn start_with_clock(
        config: DaemonPlan,
        clock: Arc<dyn DaemonClockSource>,
    ) -> Result<Self, HostError> {
        let DaemonPlanParts {
            storage,
            authentication,
            runtime,
            adapters,
            peers,
            shutdown,
        } = config.into_parts();
        let auth = AuthRegistry::from_plan(&authentication)
            .map_err(|error| HostError::Configuration(error.to_string()))?;
        let queue_capacity = runtime.request_queue;
        let shutdown_deadline = Duration::from_millis(shutdown.deadline_ms);
        let queue_size = usize::try_from(queue_capacity)
            .map_err(|_| HostError::Configuration("request queue exceeds platform".to_owned()))?;
        let (sender, receiver) = sync_channel(queue_size);
        let sender = Arc::new(sender);
        let (startup_sender, startup_receiver) = sync_channel(1);
        let health = Arc::new(SharedHealth::new(queue_capacity, &storage, &peers));
        let thread_health = health.clone();
        let thread_auth = auth.clone();
        let owner_sender = Arc::downgrade(&sender);
        let owner_clock = clock.clone();
        let maintenance = Duration::from_millis(runtime.maintenance_interval_ms);
        let owner_plan = OwnerPlan {
            storage,
            runtime,
            adapters,
            peers,
            shutdown,
        };
        let join = thread::Builder::new()
            .name("milkdrift-runtime-owner".to_owned())
            .spawn(move || {
                info!(phase = "startup", "runtime owner starting");
                let (mut owner, startup) = match Owner::open(
                    owner_plan,
                    thread_auth,
                    thread_health.clone(),
                    owner_sender,
                    owner_clock,
                ) {
                    Ok(opened) => opened,
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
                clock: peer_runtime.clock,
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

    /// Closes durable admission on the owner before graceful HTTP shutdown begins.
    pub(crate) async fn begin_draining(&self) -> Result<(), HostError> {
        self.mutating_admission.store(false, Ordering::SeqCst);
        let durable = self
            .dispatch(false, |owner| owner.begin_peer_drain())
            .await
            .map_err(|error| HostError::Shutdown(error.message));
        self.health.set_lifecycle(Lifecycle::Draining);
        let registries = self.peer_registries.values().cloned().collect::<Vec<_>>();
        tokio::task::spawn_blocking(move || {
            for registry in registries {
                let _ = registry.disconnect();
            }
        })
        .await
        .map_err(|_| HostError::Shutdown("peer disconnect task failed".to_owned()))?;
        durable
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
        let durable_peer = peer.clone();
        self.dispatch(false, move |owner| owner.revoke_peer(&durable_peer))
            .await
            .map_err(|error| HostError::Configuration(error.message))?;
        self.revoked_peers
            .lock()
            .map_err(|_| HostError::Configuration("peer revocation state unavailable".to_owned()))?
            .insert(peer.clone());
        self.disconnect_peer(peer).await
    }

    /// Runs ordered shutdown and joins the owner thread.
    pub async fn shutdown(&self) -> Result<(), HostError> {
        let shutdown_started = std::time::Instant::now();
        let drain_error = self.begin_draining().await.err();
        let peer_shutdown = if let Some(service) = &self.peer_service {
            let service = service.clone();
            let deadline = self
                .shutdown_deadline
                .saturating_sub(shutdown_started.elapsed());
            Some(
                match tokio::task::spawn_blocking(move || service.shutdown_workers(deadline)).await
                {
                    Ok(report) => report,
                    Err(_) => PeerWorkerShutdownReport {
                        clean: false,
                        joined: 0,
                        retained_workers: 1,
                    },
                },
            )
        } else {
            None
        };
        let effect_shutdown = match self
            .dispatch_draining(|owner| owner.take_effect_workers_for_shutdown())
            .await
        {
            Ok((workers, mode)) => {
                let deadline = self
                    .shutdown_deadline
                    .saturating_sub(shutdown_started.elapsed());
                let health = self.health.clone();
                match tokio::task::spawn_blocking(move || {
                    shutdown_effect_workers(&workers, mode, deadline, &health)
                })
                .await
                {
                    Ok(outcome) => outcome,
                    Err(_) => EffectShutdownOutcome::failed(),
                }
            }
            Err(error) => {
                warn!(
                    phase = "draining",
                    code = "effect_worker_take",
                    "{}",
                    bounded(&error.message)
                );
                self.health.failure("effect worker shutdown failed");
                EffectShutdownOutcome::failed()
            }
        };
        let health = self.health.clone();
        let result = match self
            .dispatch(true, move |owner| {
                owner.shutdown(&health, peer_shutdown, effect_shutdown)
            })
            .await
        {
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
            ShutdownOutcome {
                clean: true,
                unresolved: 0,
            } => match drain_error {
                Some(error) => Err(error),
                None => Ok(()),
            },
            ShutdownOutcome { clean, unresolved } => {
                self.health.set_lifecycle(Lifecycle::Failed);
                Err(HostError::Shutdown(format!(
                    "shutdown retained or could not resolve {unresolved} invocation(s); clean={clean}"
                )))
            }
        }
    }

    async fn dispatch<T>(
        &self,
        shutdown: bool,
        operation: impl FnOnce(&mut Owner) -> Result<T, PublicFailure> + Send + 'static,
    ) -> Result<T, PublicFailure>
    where
        T: Send + 'static,
    {
        self.dispatch_inner(shutdown, !shutdown, shutdown, operation)
            .await
    }

    async fn dispatch_draining<T>(
        &self,
        operation: impl FnOnce(&mut Owner) -> Result<T, PublicFailure> + Send + 'static,
    ) -> Result<T, PublicFailure>
    where
        T: Send + 'static,
    {
        self.dispatch_inner(false, false, true, operation).await
    }

    async fn dispatch_inner<T>(
        &self,
        stop_owner: bool,
        require_ready: bool,
        use_shutdown_deadline: bool,
        operation: impl FnOnce(&mut Owner) -> Result<T, PublicFailure> + Send + 'static,
    ) -> Result<T, PublicFailure>
    where
        T: Send + 'static,
    {
        if require_ready && !self.health.is_ready() {
            return Err(PublicFailure::new(
                ErrorCode::Unavailable,
                "daemon is not ready",
                true,
            ));
        }
        let started = tokio::time::Instant::now();
        let (reply, receiver) = oneshot::channel();
        let mut pending = OwnerRequest {
            execute: Box::new(move |owner| {
                let _ = reply.send(operation(owner));
            }),
            stop_owner,
            queued: None,
        };
        loop {
            pending.mark_queued(&self.health);
            match self.sender.try_send(pending) {
                Ok(()) => break,
                Err(TrySendError::Full(mut returned)) if use_shutdown_deadline => {
                    returned.mark_dequeued();
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
        let response_timeout = if use_shutdown_deadline {
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

pub(crate) enum StreamAuthority {
    Run(String),
    Capabilities,
    Health,
}

pub(crate) struct ArtifactContentRead {
    pub(crate) metadata: ArtifactMetadataRead,
    pub(crate) offset: u64,
    pub(crate) bytes: Vec<u8>,
    pub(crate) end: bool,
}

type OwnerTask = Box<dyn FnOnce(&mut Owner) + Send + 'static>;

struct OwnerRequest {
    execute: OwnerTask,
    stop_owner: bool,
    queued: Option<QueuedRequestGuard>,
}

impl OwnerRequest {
    fn mark_queued(&mut self, health: &Arc<SharedHealth>) {
        self.queued = Some(health.track_queued_request());
    }

    fn mark_dequeued(&mut self) {
        if let Some(queued) = self.queued.take() {
            queued.release();
        }
    }
}

struct OwnerPlan {
    storage: StoragePlan,
    runtime: RuntimeHostConfig,
    adapters: AdapterConfig,
    peers: PeerHostConfig,
    shutdown: ShutdownConfig,
}

struct Owner {
    shutdown: ShutdownConfig,
    store: Arc<RedbStore>,
    runtime: Arc<RuntimeService>,
    control: Arc<ControlService>,
    capability_host: CapabilityHost,
    authority: Arc<GrantSetEvaluator>,
    effect_workers: Option<EffectWorkerHost>,
    peer_service: Option<Weak<PeerService>>,
    // Strong lifecycle lease; service-facing artifact adapters retain only a weak handle.
    _peer_artifacts: Option<Arc<CorePeerArtifactStore>>,
    peer_registries: BTreeMap<PeerId, Arc<PeerRegistry>>,
    clock: DurableClock,
}

struct ShutdownOutcome {
    clean: bool,
    unresolved: u32,
}

struct EffectShutdownOutcome {
    clean: bool,
    unresolved_invocations: usize,
    outstanding_effects: usize,
}

impl EffectShutdownOutcome {
    const fn failed() -> Self {
        Self {
            clean: false,
            unresolved_invocations: 1,
            outstanding_effects: 1,
        }
    }
}

fn shutdown_effect_workers(
    workers: &EffectWorkerHost,
    mode: EffectShutdownMode,
    deadline: Duration,
    health: &SharedHealth,
) -> EffectShutdownOutcome {
    match workers.shutdown(mode, deadline) {
        Ok(result) => {
            let execution_work = result
                .health
                .queued_executions
                .saturating_add(result.health.active_executions);
            let cancellation_work = result
                .health
                .queued_cancellations
                .saturating_add(result.health.active_cancellations);
            EffectShutdownOutcome {
                clean: result.clean,
                unresolved_invocations: result
                    .unresolved_invocations
                    .len()
                    .max(execution_work)
                    .saturating_add(cancellation_work),
                outstanding_effects: execution_work.saturating_add(cancellation_work),
            }
        }
        Err(error) => {
            health.failure("effect worker shutdown failed");
            warn!(
                phase = "draining",
                code = "effect_worker_shutdown",
                "{}",
                bounded(&error.to_string())
            );
            let outstanding = workers.health().map_or(1, |value| {
                value
                    .queued_executions
                    .saturating_add(value.active_executions)
                    .saturating_add(value.queued_cancellations)
                    .saturating_add(value.active_cancellations)
                    .max(1)
            });
            EffectShutdownOutcome {
                clean: false,
                unresolved_invocations: outstanding,
                outstanding_effects: outstanding,
            }
        }
    }
}

struct PeerRuntime {
    service: Option<Arc<PeerService>>,
    artifacts: Option<Arc<CorePeerArtifactStore>>,
    registries: BTreeMap<PeerId, Arc<PeerRegistry>>,
    clock: DurableClock,
}

impl Owner {
    fn open(
        plan: OwnerPlan,
        auth: AuthRegistry,
        health: Arc<SharedHealth>,
        sender: Weak<SyncSender<OwnerRequest>>,
        clock_source: Arc<dyn DaemonClockSource>,
    ) -> Result<(Self, PeerRuntime), String> {
        let OwnerPlan {
            storage,
            runtime: runtime_plan,
            adapters,
            peers,
            shutdown,
        } = plan;
        fs::create_dir_all(&storage.data_root)
            .map_err(|error| format!("data root creation failed: {:?}", error.kind()))?;
        if storage.data_root.join(LEGACY_SIDECAR_FILE).exists() {
            return Err(
                "legacy control-state-v1.json is unsupported; this release refuses sidecar state instead of silently importing or ignoring idempotency truth"
                    .to_owned(),
            );
        }
        for prototype in ["peer-executions-v1", "peer-artifacts-v1"] {
            if storage.data_root.join(prototype).exists() {
                return Err(format!(
                    "prototype {prototype} storage is unsupported; this release refuses parallel peer authorities instead of partially importing them"
                ));
            }
        }
        let store = Arc::new(
            RedbStore::open_with_config(
                RedbStoreConfig::new(&storage.data_root)
                    .with_application_receipt_lifecycle(
                        storage.application_receipts.hot_receipt_bound,
                        storage.application_receipts.archive_batch_size,
                    )
                    .with_security_audit_limit(storage.security_audit_record_bound)
                    .with_artifact_clock(Arc::new(ArtifactClockAdapter(clock_source.clone()))),
            )
            .map_err(|error| error.to_string())?,
        );
        let owner_queue = OwnerQueue::new(sender, health.clone(), thread::current().id());
        let clock = DurableClock::new(
            clock_source,
            owner_queue.clone(),
            Arc::downgrade(&store),
            health.clone(),
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
                max_concurrent_per_generation: runtime_plan.global_concurrency,
                observation_stale_after_ms: 60_000,
            },
            CapabilitySelectionPolicy::priorities(BTreeMap::new()),
        )
        .map_err(|error| error.to_string())?;
        let scheduler = SchedulerLimits::new(
            runtime_plan.global_concurrency,
            runtime_plan.per_run_concurrency,
            runtime_plan.per_branch_concurrency,
            runtime_plan.per_capability_concurrency,
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
            runtime_plan.lease_duration_ms,
            runtime_plan.maximum_tick_items,
            scheduler,
            retry,
        )
        .map_err(|error| error.to_string())?;
        let startup_now = clock
            .now()
            .map(TimestampMillis::get)
            .map_err(|_| "daemon clock unavailable during startup".to_owned())?;
        let runtime = Arc::new(
            RuntimeService::open_closed_with_authority(
                store.clone(),
                Arc::new(capability_host.clone()),
                authority.clone(),
                clock.runtime_adapter(),
                Arc::new(
                    SequentialIdGenerator::new("daemon", startup_now)
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
                storage.data_root.join("execution"),
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
        health.receipt_status(
            store
                .application_receipt_status()
                .map_err(|error| error.to_string())?,
        );
        store
            .application_layouts(&ApplicationPageQuery {
                after: None,
                limit: PageSize::new(1).map_err(|error| error.to_string())?,
            })
            .map_err(|error| error.to_string())?;
        capabilities::register_control(
            &capability_host,
            control.clone(),
            data.clone(),
            startup_now,
        )?;
        capabilities::register_configured(
            &adapters,
            &capability_host,
            data,
            auth.resolver(),
            startup_now,
        )?;
        let peer_runtime = build_peer_runtime(
            &peers,
            runtime_plan.lease_duration_ms,
            &capability_host,
            store.clone(),
            auth.resolver(),
            owner_queue,
            clock.clone(),
        )?;
        if let Some(service) = &peer_runtime.service {
            service.recover(1_024).map_err(|error| error.to_string())?;
            health.peer_status(
                service
                    .execution_status()
                    .map_err(|error| error.to_string())?,
            );
        }
        let effect_workers = EffectWorkerHost::start(
            runtime.clone(),
            capability_host.clone(),
            EffectWorkerConfig {
                execution_threads: runtime_plan.effect_threads,
                execution_queue: runtime_plan.effect_queue,
                cancellation_queue: runtime_plan.cancellation_queue,
                maximum_claim_page: runtime_plan.maximum_effect_claim,
            },
        )
        .map_err(|error| error.to_string())?;
        runtime
            .resume_admission()
            .map_err(|error| error.to_string())?;
        health.set_active_effects(0);
        Ok((
            Self {
                shutdown,
                store,
                runtime,
                control,
                capability_host,
                authority,
                effect_workers: Some(effect_workers),
                peer_service: peer_runtime.service.as_ref().map(Arc::downgrade),
                _peer_artifacts: peer_runtime.artifacts.clone(),
                peer_registries: peer_runtime.registries.clone(),
                clock,
            },
            peer_runtime,
        ))
    }

    fn now(&self) -> Result<u64, PublicFailure> {
        self.clock
            .now()
            .map(TimestampMillis::get)
            .map_err(|_| clock_unavailable())
    }

    fn run(
        &mut self,
        receiver: Receiver<OwnerRequest>,
        maintenance: Duration,
        health: &SharedHealth,
    ) {
        loop {
            match receiver.recv_timeout(maintenance) {
                Ok(mut request) => {
                    request.mark_dequeued();
                    let stop_owner = request.stop_owner;
                    (request.execute)(self);
                    if stop_owner {
                        return;
                    }
                    self.maintenance(health);
                }
                Err(RecvTimeoutError::Timeout) => self.maintenance(health),
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = self.shutdown(health, None, EffectShutdownOutcome::failed());
                    return;
                }
            }
        }
    }

    fn maintenance(&self, health: &SharedHealth) {
        match self.store.application_receipt_status() {
            Ok(status) if status.hot_count >= u64::from(status.hot_bound) => {
                let outcome = self.now().and_then(|now| {
                    self.store
                        .archive_application_command_receipts(ApplicationReceiptArchiveRequest {
                            expected_generation: status.archive_generation,
                            archived_at: TimestampMillis::new(now),
                        })
                        .map_err(public_persistence)
                });
                match outcome {
                    Ok(outcome) => health.receipt_status(outcome.status),
                    Err(error) => {
                        warn!(
                            outcome = "error",
                            code = "application_receipt_archive",
                            "{}",
                            bounded(&error.message)
                        );
                        health.receipt_failure();
                    }
                }
            }
            Ok(status) => health.receipt_status(status),
            Err(error) => {
                warn!(
                    outcome = "error",
                    code = "application_receipt_status",
                    "{}",
                    bounded(&error.to_string())
                );
                health.receipt_failure();
            }
        }
        if let Some(service) = self.peer_service.as_ref().and_then(Weak::upgrade) {
            match service.maintain_retention() {
                Ok(status) => health.peer_status(status),
                Err(error) => {
                    warn!(
                        outcome = "error",
                        code = "peer_execution_archive",
                        "{}",
                        bounded(&error.to_string())
                    );
                    health.peer_failure();
                }
            }
        }
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
                health.set_active_effects(u32::try_from(active).unwrap_or(u32::MAX));
            }
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
        let now = self.now()?;
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

    fn begin_peer_drain(&self) -> Result<(), PublicFailure> {
        let Some(service) = self.peer_service.as_ref().and_then(Weak::upgrade) else {
            return if self.peer_service.is_some() {
                Err(peer_unavailable())
            } else {
                Ok(())
            };
        };
        service.begin_drain().map_err(public_peer)
    }

    fn revoke_peer(&self, peer: &PeerId) -> Result<(), PublicFailure> {
        let service = self
            .peer_service
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or_else(peer_unavailable)?;
        service.revoke_peer(peer).map_err(public_peer)
    }

    fn take_effect_workers_for_shutdown(
        &mut self,
    ) -> Result<(EffectWorkerHost, EffectShutdownMode), PublicFailure> {
        self.runtime.begin_shutdown();
        let mode = match self.shutdown.effect_policy {
            ShutdownEffectPolicy::Drain => EffectShutdownMode::Drain,
            ShutdownEffectPolicy::Cancel => EffectShutdownMode::Cancel,
            ShutdownEffectPolicy::Retain => EffectShutdownMode::Retain,
        };
        self.effect_workers
            .take()
            .map(|workers| (workers, mode))
            .ok_or_else(|| {
                PublicFailure::new(
                    ErrorCode::Unavailable,
                    "effect worker owner is unavailable",
                    true,
                )
            })
    }

    fn shutdown(
        &mut self,
        health: &SharedHealth,
        peer_shutdown: Option<PeerWorkerShutdownReport>,
        mut effect_shutdown: EffectShutdownOutcome,
    ) -> Result<ShutdownOutcome, PublicFailure> {
        info!(phase = "draining", "runtime owner closing admission");
        health.set_lifecycle(Lifecycle::Draining);
        self.runtime.begin_shutdown();
        if self.effect_workers.take().is_some() {
            health.failure("effect worker owner was not transferred before shutdown");
            effect_shutdown.clean = false;
            effect_shutdown.unresolved_invocations =
                effect_shutdown.unresolved_invocations.saturating_add(1);
            effect_shutdown.outstanding_effects =
                effect_shutdown.outstanding_effects.saturating_add(1);
        }
        let peer_retained = peer_shutdown.map_or(0, |report| report.retained_workers);
        let clean = effect_shutdown.clean && peer_retained == 0;
        health.set_active_effects(if clean {
            0
        } else {
            u32::try_from(effect_shutdown.outstanding_effects).unwrap_or(u32::MAX)
        });
        health.set_lifecycle(if clean {
            Lifecycle::Stopped
        } else {
            Lifecycle::Failed
        });
        let unresolved = effect_shutdown
            .unresolved_invocations
            .saturating_add(usize::from(peer_retained));
        info!(
            phase = if clean { "stopped" } else { "failed" },
            clean, unresolved, "runtime owner shutdown completed"
        );
        Ok(ShutdownOutcome {
            clean,
            unresolved: u32::try_from(unresolved).unwrap_or(u32::MAX),
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
        let now = self.now()?;
        let seed = format!("{}:{suffix}:{now}", session.actor.as_str());
        let digest = blake3::hash(seed.as_bytes());
        let document = ControlCommandDocument::new(
            ControlId::new(format!("query-{}", &digest.to_hex().as_str()[..32]))
                .map_err(public_control)?,
            session.context.clone(),
            TimestampMillis::new(now),
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
    peers: &PeerHostConfig,
    execution_lease_ms: u64,
    host: &CapabilityHost,
    store: Arc<RedbStore>,
    secrets: Arc<LocalSecretResolver>,
    owner_queue: OwnerQueue,
    clock: DurableClock,
) -> Result<PeerRuntime, String> {
    let PeerHostConfig::Enabled {
        local_peer_id,
        relationships: configured_relationships,
        serving,
    } = peers
    else {
        return Ok(PeerRuntime {
            service: None,
            artifacts: None,
            registries: BTreeMap::new(),
            clock,
        });
    };
    let local_peer = PeerId::new(local_peer_id.clone()).map_err(|error| error.to_string())?;
    let mut session_hasher = blake3::Hasher::new();
    session_hasher.update(b"milkdrift.peer.session.v1\0");
    session_hasher.update(local_peer.as_str().as_bytes());
    let session_now = clock
        .now()
        .map(TimestampMillis::get)
        .map_err(|_| "daemon clock unavailable during peer initialization".to_owned())?;
    session_hasher.update(&session_now.to_be_bytes());
    let session = SessionId::new(format!("session:{}", session_hasher.finalize().to_hex()))
        .map_err(|error| error.to_string())?;
    let versions = ProtocolVersionRange::default();
    let mut relationships = Vec::new();
    let mut clients = Vec::new();
    let mut authentication = Vec::new();
    for configured in configured_relationships {
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
                major: PROTOCOL_MAJOR_V1,
                minor: configured.minimum_minor,
            },
            ProtocolVersion {
                major: PROTOCOL_MAJOR_V1,
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
    let peer_clock = clock.peer_adapter();
    let direct_artifacts = Arc::new(
        CorePeerArtifactStore::new(
            store.clone(),
            configured_relationships
                .iter()
                .map(|relationship| relationship.maximum_artifact_bytes)
                .max()
                .unwrap_or(1),
            10 * 1_073_741_824,
            peer_clock.clone(),
        )
        .map_err(|error| error.to_string())?,
    );
    let executions = Arc::new(OwnerPeerExecutionStore::new(
        owner_queue.clone(),
        Arc::downgrade(&store),
    ));
    let artifacts = Arc::new(OwnerPeerArtifactStore::new(
        owner_queue,
        Arc::downgrade(&direct_artifacts),
    ));
    let service = PeerService::new_with_artifacts_and_authenticator(
        PeerServerConfig {
            local_peer,
            session,
            versions,
            limits: HardLimits::default(),
            lease: HeartbeatLease {
                heartbeat_ms: 5_000,
                idle_timeout_ms: 20_000,
                execution_lease_ms,
            },
            relationships,
            workers: PeerWorkerConfig {
                threads: serving.worker_threads,
                maximum_global_active: serving.maximum_global_active,
                maximum_dispatch_queue: serving.maximum_dispatch_queue,
                maximum_hot_terminal_records: serving.maximum_hot_terminal_records,
                archive_batch_size: serving.archive_batch_size,
                observation_hot_retention: Duration::from_millis(
                    serving.observation_hot_retention_ms,
                ),
                recovery_page: serving.recovery_page,
                poll_interval: Duration::from_millis(serving.poll_interval_ms),
            },
        },
        host.clone(),
        executions,
        artifacts,
        Some(Arc::new(ConfiguredPeerAuthenticator {
            resolver: secrets,
            relationships: authentication,
        })),
        peer_clock.clone(),
    )
    .map_err(|error| error.to_string())?;
    let mut registries = BTreeMap::new();
    for (client, relationship) in clients {
        let peer = relationship.remote_peer.clone();
        let registry = Arc::new(
            PeerRegistry::new(host.clone(), client, relationship, peer_clock.clone())
                .map_err(|error| error.to_string())?,
        );
        registries.insert(peer, registry);
    }
    Ok(PeerRuntime {
        service: Some(service),
        artifacts: Some(direct_artifacts),
        registries,
        clock,
    })
}

struct ConfiguredPeerCredential {
    resolver: Arc<LocalSecretResolver>,
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
    resolver: Arc<LocalSecretResolver>,
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

fn peer_unavailable() -> PublicFailure {
    PublicFailure::new(ErrorCode::Unavailable, "peer service is unavailable", true)
}

fn public_peer(error: PeerHttpError) -> PublicFailure {
    match error {
        PeerHttpError::Unauthenticated | PeerHttpError::Unauthorized(_) => unauthorized(),
        PeerHttpError::NotFound(_) => not_found(),
        PeerHttpError::Overloaded(_) => PublicFailure::new(
            ErrorCode::Overload,
            "peer service capacity is exhausted",
            true,
        ),
        PeerHttpError::Persistence(_)
        | PeerHttpError::Unavailable(_)
        | PeerHttpError::Transport(_) => peer_unavailable(),
        PeerHttpError::Configuration(message) | PeerHttpError::Protocol(message) => {
            invalid(&bounded(&message))
        }
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
    use std::{
        collections::BTreeMap,
        fs,
        net::SocketAddr,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };

    use super::clock::{DaemonClockError, DaemonClockSource};
    use super::{DaemonHost, TimelineCategory, read_model::timeline_summary};
    use crate::config::{
        ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
        AuthorityPresetConfig, ConfigError, DaemonConfig, PeerHostConfig, RuntimeHostConfig,
        SecretSourceConfig, ShutdownConfig,
    };

    struct ControlledDaemonClock(AtomicU64);

    impl ControlledDaemonClock {
        const fn new(now: u64) -> Self {
            Self(AtomicU64::new(now))
        }

        fn set(&self, now: u64) {
            self.0.store(now, Ordering::SeqCst);
        }
    }

    impl DaemonClockSource for ControlledDaemonClock {
        fn now_unix_ms(&self) -> Result<u64, DaemonClockError> {
            Ok(self.0.load(Ordering::SeqCst))
        }
    }

    fn clock_test_config(
        root: &std::path::Path,
        token: &std::path::Path,
    ) -> Result<crate::DaemonPlan, ConfigError> {
        DaemonConfig {
            schema_version: crate::DAEMON_CONFIG_SCHEMA_VERSION,
            data_root: root.join("data"),
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            secret_sources: BTreeMap::from([(
                "credential:operator".to_owned(),
                SecretSourceConfig::File {
                    path: token.to_path_buf(),
                },
            )]),
            actors: vec![ActorBindingConfig {
                credential_ref: "credential:operator".to_owned(),
                actor: "human:clock-operator".to_owned(),
                grant_id: "grant:clock-operator".to_owned(),
                grant_revision: 1,
                revocation_generation: 0,
                preset: AuthorityPresetConfig::Controller,
                authority: ActorGrantConfig::dangerous_administrator(),
                enabled: true,
            }],
            runtime: RuntimeHostConfig::default(),
            adapters: AdapterConfig::default(),
            peers: PeerHostConfig::default(),
            shutdown: ShutdownConfig::default(),
            application_receipts: ApplicationReceiptConfig {
                hot_receipt_bound: 100,
                archive_batch_size: 10,
            },
            security_audit_record_bound: 100,
        }
        .validate(root)
    }

    #[test]
    fn timeline_projection_never_serializes_internal_event_body() {
        assert_eq!(
            timeline_summary(TimelineCategory::Execution),
            "node execution changed"
        );
        assert!(!timeline_summary(TimelineCategory::Execution).contains("NodeScheduled"));
    }

    #[tokio::test]
    async fn daemon_restart_rejects_clock_rollback_before_readiness()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let token = root.path().join("operator.token");
        fs::write(&token, "clock-test-token")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&token, fs::Permissions::from_mode(0o600))?;
        }
        let clock = Arc::new(ControlledDaemonClock::new(100));
        let host =
            DaemonHost::start_with_clock(clock_test_config(root.path(), &token)?, clock.clone())?;
        clock.set(120);
        assert_eq!(host.now().await.map_err(|error| error.message)?, 120);
        host.shutdown().await?;

        clock.set(119);
        assert!(
            DaemonHost::start_with_clock(clock_test_config(root.path(), &token)?, clock.clone(),)
                .is_err()
        );

        clock.set(120);
        let recovered =
            DaemonHost::start_with_clock(clock_test_config(root.path(), &token)?, clock)?;
        recovered.shutdown().await?;
        Ok(())
    }
}
