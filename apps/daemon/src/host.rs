use std::{
    collections::BTreeMap, collections::BTreeSet, sync::Arc, sync::Mutex, sync::Weak,
    sync::atomic::AtomicBool, sync::atomic::Ordering, sync::mpsc::SyncSender, thread::JoinHandle,
    time::Duration,
};

use milkdrift_authority::{
    AuthorityDecisionSnapshot, AuthorityOperation, GrantSetEvaluator, RequestedResourceFacts,
    WorkflowRunScope,
};
use milkdrift_blueprint::{AuthorRef, BlueprintRevisionDocument, WorkflowId};
use milkdrift_capability::PeerId;
use milkdrift_capability_host::{CapabilityHost, EffectWorkerHost};
use milkdrift_control::{
    ControlCommand, ControlCommandDocument, ControlId, ControlResult, ControlService,
    OptimisticGuard, ProposalDigest, ProposalId, WorkflowProposalDocument,
};
use milkdrift_control_protocol::{
    ArtifactMetadataRead, CommandAccepted, CommandRequest, ContextManifestRead, ErrorCode,
    HealthRead, ProposalDecision,
};
use milkdrift_peer_http::{CorePeerArtifactStore, PeerRegistry, PeerService};
use milkdrift_persistence::{
    ArtifactReadAuthority, ArtifactStore, AttemptId, CorrelationKey, EvidenceId, EvidenceKind,
    EvidenceReference, NodeExecutionId, Reason, ReconciliationDecisionId, RepeatDecisionId,
    RevisionStore, RunQueryStore, RunSequence, SignalDeliveryMode, SignalId, SignalTypeId,
    TimestampMillis,
};
use milkdrift_prompt_sequence::{PromptSequenceDocument, compile as compile_prompt_sequence};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::RuntimeService;
use milkdrift_workspace::{ArtifactId, RunId, ScopeId, WorkspaceBudget, WorkspaceScope};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{auth::ActorSession, auth::AuthRegistry, config::ShutdownConfig};

mod artifacts;
mod attempts;
mod authorization;
mod capabilities;
mod clock;
mod commands;
mod definitions;
mod health;
mod layouts;
mod maintenance;
mod peer_store;
mod peers;
mod proposals;
mod queue;
mod read_model;
mod receipts;
mod requests;
mod runs;
mod shutdown;
mod startup;

use clock::DurableClock;
use health::SharedHealth;
use peers::{PeerRuntime, build_peer_runtime};
use queue::OwnerRequest;
use read_model::{
    accepted_sequence, bounded, clock_unavailable, corruption, empty_attempt_read, internal,
    invalid, map_resolve, not_found, parse_revision_id, public_attempt_usage,
    public_authority_decision, public_capability_provenance, public_control,
    public_execution_authority, public_invocation_artifact, public_operation_contract,
    public_persistence, snake_debug, unauthorized,
};
use shutdown::EffectShutdownOutcome;

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

struct Owner {
    request_panicked: bool,
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

impl Owner {
    fn now(&self) -> Result<u64, PublicFailure> {
        self.clock
            .now()
            .map(TimestampMillis::get)
            .map_err(|_| clock_unavailable())
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
mod tests;
