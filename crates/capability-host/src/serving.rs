mod artifact;
mod artifact_transfer;
mod auth;
mod config;
mod dispatch;
mod error;
pub(crate) mod prepared;
mod store;
pub use artifact::{
    CorePeerArtifactStore, PeerArtifactError, PeerArtifactStore, PeerArtifactTransferFacts,
};
pub(crate) use artifact_transfer::DisabledArtifactStore;
pub use auth::PeerAuthenticator;
pub use config::ServingClientPolicy;
pub use config::{PeerRelationship, PeerServerConfig, PeerWorkerConfig};
pub use error::ServingError;
mod authority;
mod catalog;
mod client_authority;
mod direct;
mod lifecycle;
mod worker;

#[cfg(test)]
use authority::peer_capability_authority;
use authority::{adapter_execution_context, peer_authority_grant};
pub(crate) use worker::{PeerUncertainty, PeerWorkerRecovery, PeerWorkerRun};

#[cfg(any(test, feature = "test-support"))]
use std::time::{SystemTime, UNIX_EPOCH};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use crate::CapabilityHost;
use milkdrift_authority::{
    AuthorityBudget, AuthorityGrant, AuthorityOperation, GrantSetEvaluator, PolicyId,
    RequestedResourceFacts,
};
use milkdrift_capability::{CancellationBehavior, CancellationRequest, PeerId};
use milkdrift_peer_protocol::{
    CancellationDisposition, CatalogSnapshot, DrainState, HandshakeRequest, HandshakeResponse,
    InvocationAcceptance, InvocationLookup, ObservationPage, PeerCancellationAcknowledgement,
    PeerCancellationRequest, PeerExecutionId, ServingInvocationRequest,
};
use milkdrift_persistence::{
    PageSize, PeerAdmission, PeerAdmissionOutcome, PeerAdmissionRejection, PeerArchivedDisposition,
    PeerExecutionPhase, PeerExecutionSnapshot, PeerExecutionStore, PersistenceError,
    ServingCallerState, StorageFailureClass,
};
use subtle::ConstantTimeEq as _;
use thiserror::Error;

use self::{
    dispatch::PeerDispatchWorkers,
    store::{acceptance, lookup as execution_lookup},
};

/// Supply boundary time for peer authority, deadlines, leases, and durable observations.
/// The embedding daemon uses its durable clock; tests can inject controlled failures and time.
pub trait PeerClock: Send + Sync {
    /// Current Unix epoch milliseconds, rejecting unavailable or backward-moving time.
    fn now_unix_ms(&self) -> Result<u64, PeerClockError>;
}

/// Failure to establish a trustworthy peer-boundary timestamp.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PeerClockError {
    /// The system clock is earlier than the Unix epoch.
    #[error("system clock precedes the Unix epoch")]
    BeforeUnixEpoch,
    /// Unix epoch milliseconds do not fit in the protocol representation.
    #[error("system clock exceeds the peer timestamp representation")]
    MillisecondOverflow,
    /// A later observation moved behind an earlier process-local observation.
    #[error("system clock moved backwards")]
    MovedBackwards,
    /// The underlying clock state cannot be observed safely.
    #[error("system clock is unavailable")]
    Unavailable,
}

/// Standalone system clock with a process-local monotonic observation check.
/// Restart-persistent rollback detection requires the daemon's injected durable clock.
#[derive(Debug, Default)]
#[cfg(any(test, feature = "test-support"))]
pub struct SystemPeerClock {
    last_unix_ms: Mutex<u64>,
}

#[cfg(any(test, feature = "test-support"))]
impl PeerClock for SystemPeerClock {
    fn now_unix_ms(&self) -> Result<u64, PeerClockError> {
        let mut last = self
            .last_unix_ms
            .lock()
            .map_err(|_| PeerClockError::Unavailable)?;
        let now = unix_millis_at(SystemTime::now())?;
        if now < *last {
            return Err(PeerClockError::MovedBackwards);
        }
        *last = now;
        Ok(now)
    }
}

#[cfg(any(test, feature = "test-support"))]
fn unix_millis_at(now: SystemTime) -> Result<u64, PeerClockError> {
    let duration = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PeerClockError::BeforeUnixEpoch)?;
    unix_millis_from_duration(duration)
}

#[cfg(any(test, feature = "test-support"))]
fn unix_millis_from_duration(duration: std::time::Duration) -> Result<u64, PeerClockError> {
    u64::try_from(duration.as_millis()).map_err(|_| PeerClockError::MillisecondOverflow)
}

#[derive(Clone)]
struct CachedCatalog {
    fingerprint: String,
    snapshot: CatalogSnapshot,
}

#[derive(Clone, Copy, Debug)]
struct RateWindow {
    started_at_unix_ms: u64,
    requests: u32,
}

/// Serving host's durable acceptance, authority, and execution-observation boundary.
///
/// Construction starts fixed workers with claims and admission closed. Register local adapters,
/// then call [`Self::recover`] before serving new work. HTTP authenticates callers and delegates
/// here; direct library callers must supply the authenticated peer identity themselves.
/// [`Self::shutdown_workers`] must run while the store and capability host can accept final writes.
pub struct PeerService {
    config: PeerServerConfig,
    relationships: BTreeMap<PeerId, PeerRelationship>,
    grants: BTreeMap<PeerId, AuthorityGrant>,
    clients: BTreeMap<milkdrift_authority::ActorRef, AuthorityGrant>,
    client_policy: Option<ServingClientPolicy>,
    authority: GrantSetEvaluator,
    capability_host: CapabilityHost,
    executions: Arc<dyn PeerExecutionStore>,
    clock: Arc<dyn PeerClock>,
    catalogs: Mutex<BTreeMap<PeerId, CachedCatalog>>,
    client_catalogs: Mutex<BTreeMap<milkdrift_authority::ActorRef, CachedCatalog>>,
    rate_windows:
        Mutex<BTreeMap<(milkdrift_peer_protocol::ServingCaller, &'static str), RateWindow>>,
    revoked_peers: Mutex<BTreeSet<PeerId>>,
    drain: AtomicU8,
    artifacts: Arc<dyn PeerArtifactStore>,
    authenticator: Option<Arc<dyn PeerAuthenticator>>,
    workers: Mutex<Option<PeerDispatchWorkers>>,
}

/// Fixed worker-owner shutdown result. A timeout reports retained owners instead of hiding them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerWorkerShutdownReport {
    /// True when every fixed worker joined before the deadline.
    pub clean: bool,
    /// Workers joined during this call.
    pub joined: u16,
    /// Workers still owned after the timeout.
    pub retained_workers: u16,
}

impl std::fmt::Debug for PeerService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerService")
            .field("local_peer", &self.config.local_peer)
            .field("session", &self.config.session)
            .field(
                "relationships",
                &self.relationships.keys().collect::<Vec<_>>(),
            )
            .field("credentials", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl PeerService {
    fn peer_caller(&self, peer: &PeerId) -> milkdrift_peer_protocol::ServingCaller {
        milkdrift_peer_protocol::ServingCaller::peer(&self.config.local_peer, peer)
    }
    /// Maximum concurrently admitted transport requests for this owner.
    pub fn connection_limit(&self) -> usize {
        usize::from(self.config.limits.connections)
    }

    /// Constructs a service with admission closed and artifact exchange disabled.
    /// Call [`Self::recover`] after local adapters register.
    pub fn new(
        config: PeerServerConfig,
        capability_host: CapabilityHost,
        executions: Arc<dyn PeerExecutionStore>,
        clock: Arc<dyn PeerClock>,
    ) -> Result<Arc<Self>, ServingError> {
        Self::new_with_artifacts(
            config,
            capability_host,
            executions,
            Arc::new(DisabledArtifactStore),
            clock,
        )
    }

    /// Constructs a service with admission closed and a verified artifact exchange port.
    pub fn new_with_artifacts(
        config: PeerServerConfig,
        capability_host: CapabilityHost,
        executions: Arc<dyn PeerExecutionStore>,
        artifacts: Arc<dyn PeerArtifactStore>,
        clock: Arc<dyn PeerClock>,
    ) -> Result<Arc<Self>, ServingError> {
        Self::new_with_artifacts_and_authenticator(
            config,
            capability_host,
            executions,
            artifacts,
            None,
            clock,
        )
    }

    /// Constructs with a request-time server authenticator for credential rotation.
    pub fn new_with_artifacts_and_authenticator(
        config: PeerServerConfig,
        capability_host: CapabilityHost,
        executions: Arc<dyn PeerExecutionStore>,
        artifacts: Arc<dyn PeerArtifactStore>,
        authenticator: Option<Arc<dyn PeerAuthenticator>>,
        clock: Arc<dyn PeerClock>,
    ) -> Result<Arc<Self>, ServingError> {
        Self::with_clients(
            config,
            capability_host,
            executions,
            artifacts,
            authenticator,
            None,
            clock,
        )
    }

    /// Constructs one serving owner for peer relationships and independently authenticated clients.
    /// The embedding authentication owner supplies the client grants; recover before admission.
    #[allow(clippy::too_many_arguments)] // One owner binds registry, persistence, artifacts, peer authentication, client policy, and clock.
    pub fn with_clients(
        config: PeerServerConfig,
        capability_host: CapabilityHost,
        executions: Arc<dyn PeerExecutionStore>,
        artifacts: Arc<dyn PeerArtifactStore>,
        authenticator: Option<Arc<dyn PeerAuthenticator>>,
        client_policy: Option<ServingClientPolicy>,
        clock: Arc<dyn PeerClock>,
    ) -> Result<Arc<Self>, ServingError> {
        config.validate()?;
        if let Some(policy) = &client_policy {
            policy.validate()?;
        }
        let clients: BTreeMap<_, _> = client_policy
            .as_ref()
            .into_iter()
            .flat_map(|policy| &policy.grants)
            .map(|grant| (grant.actor().clone(), grant.clone()))
            .collect();
        let relationships: BTreeMap<_, _> = config
            .relationships
            .iter()
            .cloned()
            .map(|relationship| (relationship.remote_peer.clone(), relationship))
            .collect();
        let grants = relationships
            .values()
            .map(|relationship| {
                peer_authority_grant(relationship)
                    .map(|grant| (relationship.remote_peer.clone(), grant))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let authority = GrantSetEvaluator::new(
            PolicyId::new("peer.relationship-authority.v1")
                .map_err(|error| ServingError::Configuration(error.to_string()))?,
            1,
            grants.values().chain(clients.values()).cloned(),
            client_policy
                .as_ref()
                .map_or_else(BTreeMap::new, |policy| policy.revocations.clone()),
        )
        .map_err(|error| ServingError::Configuration(error.to_string()))?;
        executions
            .bind_serving_host(&config.local_peer)
            .map_err(map_execution_persistence)?;
        executions
            .set_peer_admission_open(false)
            .map_err(map_execution_persistence)?;
        for relationship in relationships.values() {
            executions
                .configure_peer_relationship(&ServingCallerState {
                    caller: milkdrift_peer_protocol::ServingCaller::peer(
                        &config.local_peer,
                        &relationship.remote_peer,
                    ),
                    generation: relationship_generation(relationship),
                    enabled: relationship.enabled,
                    expires_at_unix_ms: relationship.expires_at_unix_ms,
                    maximum_active: u32::from(relationship.maximum_concurrent),
                })
                .map_err(map_execution_persistence)?;
        }
        for grant in clients.values() {
            let policy = client_policy
                .as_ref()
                .ok_or_else(|| ServingError::Configuration("client policy missing".to_owned()))?;
            let revoked = policy
                .revocations
                .get(grant.identity())
                .is_some_and(|generation| *generation > grant.revocation_generation());
            executions
                .configure_peer_relationship(&ServingCallerState {
                    caller: milkdrift_peer_protocol::ServingCaller {
                        host: config.local_peer.clone(),
                        principal: milkdrift_peer_protocol::ServingPrincipal::Client {
                            actor: grant.actor().clone(),
                        },
                    },
                    generation: grant
                        .revision()
                        .saturating_add(grant.revocation_generation()),
                    enabled: !revoked,
                    expires_at_unix_ms: grant.valid_until().get(),
                    maximum_active: policy
                        .maximum_concurrent
                        .min(
                            grant
                                .budget()
                                .concurrency
                                .unwrap_or(policy.maximum_concurrent),
                        )
                        .max(1),
                })
                .map_err(map_execution_persistence)?;
        }
        let worker_config = config.workers;
        let service = Arc::new(Self {
            config,
            relationships,
            grants,
            clients,
            client_policy,
            authority,
            capability_host,
            executions,
            clock,
            catalogs: Mutex::new(BTreeMap::new()),
            client_catalogs: Mutex::new(BTreeMap::new()),
            rate_windows: Mutex::new(BTreeMap::new()),
            revoked_peers: Mutex::new(BTreeSet::new()),
            // Recovery owns startup. Workers and inbound admission remain closed until it finishes.
            drain: AtomicU8::new(3),
            artifacts,
            authenticator,
            workers: Mutex::new(None),
        });
        let workers = PeerDispatchWorkers::start(Arc::downgrade(&service), worker_config)?;
        *service
            .workers
            .lock()
            .map_err(|_| ServingError::Unavailable("peer worker owner unavailable".to_owned()))? =
            Some(workers);
        Ok(service)
    }

    /// Authenticates only the transport bearer value and returns its configured identity.
    /// Request payload identity fields never choose this result.
    pub fn authenticate_bearer(&self, supplied: &[u8]) -> Result<PeerId, ServingError> {
        let now = self.now()?;
        if let Some(authenticator) = &self.authenticator {
            return authenticator
                .authenticate(supplied, now)
                .filter(|peer| {
                    self.relationships.contains_key(peer)
                        && !self
                            .revoked_peers
                            .lock()
                            .map_or(true, |revoked| revoked.contains(peer))
                })
                .ok_or(ServingError::Unauthenticated);
        }
        self.relationships
            .values()
            .filter(|relationship| relationship.enabled && now <= relationship.expires_at_unix_ms)
            .filter(|relationship| {
                !self
                    .revoked_peers
                    .lock()
                    .map_or(true, |revoked| revoked.contains(&relationship.remote_peer))
            })
            .find(|relationship| {
                relationship.bearer_credential.expose(|expected| {
                    expected.len() == supplied.len() && bool::from(expected.ct_eq(supplied))
                })
            })
            .map(|relationship| relationship.remote_peer.clone())
            .ok_or(ServingError::Unauthenticated)
    }

    pub(super) fn now(&self) -> Result<u64, ServingError> {
        self.clock
            .now_unix_ms()
            .map_err(|error| ServingError::Unavailable(error.to_string()))
    }

    /// Negotiates a session and cross-checks the claimed identity against authentication.
    pub fn handshake(
        &self,
        authenticated_peer: &PeerId,
        request: &HandshakeRequest,
    ) -> Result<HandshakeResponse, ServingError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.require_operation(
            &relationship,
            AuthorityOperation::NegotiatePeerSession,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        if &request.claimed_peer != authenticated_peer {
            return Err(ServingError::Unauthorized(
                "handshake identity does not match transport authentication".to_owned(),
            ));
        }
        self.check_rate(&relationship, "handshake")?;
        request
            .limits
            .validate()
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        let selected_version = self
            .config
            .versions
            .negotiate(request.versions)
            .and_then(|selected| {
                relationship
                    .versions
                    .negotiate(milkdrift_peer_protocol::ProtocolVersionRange {
                        minimum: selected,
                        maximum: selected,
                    })
            })
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        Ok(HandshakeResponse {
            peer: self.config.local_peer.clone(),
            session: self.config.session.clone(),
            selected_version,
            features: milkdrift_peer_protocol::FeatureSet {
                resumable_observations: true,
                resumable_artifacts: true,
                incremental_catalog: false,
                archived_execution_replay: true,
            },
            limits: self.config.limits.intersect(request.limits),
            lease: self.config.lease,
            drain: self.drain_state(),
        })
    }

    /// Atomically accepts one exact request into the durable bounded dispatch queue.
    /// The response confirms acceptance, not adapter entry. Exact replay is looked up before
    /// fresh catalog/capacity checks; conflicting request bytes retain the original execution.
    pub fn invoke(
        self: &Arc<Self>,
        authenticated_peer: &PeerId,
        request: ServingInvocationRequest,
    ) -> Result<InvocationAcceptance, ServingError> {
        let relationship = self.relationship(authenticated_peer)?;
        if request.authorization.caller() != self.peer_caller(authenticated_peer) {
            return Err(ServingError::Unauthorized(
                "request caller does not match peer authentication and target host".to_owned(),
            ));
        }
        request
            .validate()
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        if let Some(existing) = self
            .executions
            .peer_execution_by_request(&self.peer_caller(authenticated_peer), &request.request_id)
            .map_err(map_execution_persistence)?
        {
            self.require_execution_operation(
                &relationship,
                &existing,
                AuthorityOperation::InspectPeerExecution,
            )?;
            return if existing.request_digest() == request.request_digest {
                Ok(acceptance(&existing, true))
            } else {
                Ok(rejection(
                    &request,
                    "idempotency_conflict",
                    "idempotency key was previously accepted with different request bytes",
                    false,
                    Some(existing.execution().clone()),
                ))
            };
        }
        if self.drain_state() != DrainState::Ready {
            return Ok(rejection(
                &request,
                "draining",
                "peer is draining",
                true,
                None,
            ));
        }
        self.check_rate(&relationship, "invoke")?;
        let now = self.now()?;
        if now > request.deadline_unix_ms {
            return Ok(rejection(
                &request,
                "deadline",
                "request deadline elapsed",
                false,
                None,
            ));
        }
        let catalog = self.catalog(authenticated_peer)?;
        if catalog.generation != request.catalog_generation
            || catalog.digest != request.catalog_digest
        {
            return Ok(rejection(
                &request,
                "catalog_stale",
                "selected catalog generation is not current",
                true,
                None,
            ));
        }
        let entry = catalog
            .entries
            .iter()
            .find(|entry| {
                entry.descriptor.identity() == request.selection.capability()
                    && entry.descriptor.descriptor_revision()
                        == request.selection.descriptor_revision()
            })
            .ok_or_else(|| {
                ServingError::Unauthorized(
                    "selected capability generation is not advertised".to_owned(),
                )
            })?;
        request
            .selection
            .validate_against(&entry.descriptor)
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        if !entry
            .invocable_operations
            .contains(request.selection.operation())
        {
            return Err(ServingError::Unauthorized(
                "selected operation is not advertised".to_owned(),
            ));
        }
        let generation = self.exact_generation(&relationship, &request)?;
        let authority_decision = self.authorize_invocation(
            &relationship,
            &request,
            &generation.descriptor,
            &generation.authority_requirements,
            now,
        )?;
        self.accept_serving(
            request,
            authority_decision,
            relationship_generation(&relationship),
        )
    }

    fn accept_serving(
        &self,
        request: ServingInvocationRequest,
        authority_decision: milkdrift_authority::AuthorityDecisionSnapshot,
        authority_generation: u64,
    ) -> Result<InvocationAcceptance, ServingError> {
        let caller = request.authorization.caller();
        let now = self.now()?;
        let execution = execution_identity(&caller, &request)?;
        match self
            .executions
            .admit_peer_execution(&PeerAdmission {
                caller: &caller,
                request: &request,
                authority: &authority_decision,
                execution: &execution,
                relationship_generation: authority_generation,
                accepted_at_unix_ms: now,
                maximum_global_active: self.config.workers.maximum_global_active,
                maximum_dispatch_queue: self.config.workers.maximum_dispatch_queue,
                maximum_hot_terminal_records: self.config.workers.maximum_hot_terminal_records,
                archive_batch_size: self.config.workers.archive_batch_size,
                archive_terminal_before_or_at_unix_ms: now
                    .saturating_sub(
                        self.config
                            .workers
                            .observation_hot_retention
                            .as_millis()
                            .try_into()
                            .unwrap_or(u64::MAX),
                    )
                    .max(1),
            })
            .map_err(map_execution_persistence)?
        {
            PeerAdmissionOutcome::Replayed(record) => Ok(acceptance(&record, true)),
            PeerAdmissionOutcome::Conflict(record) => Ok(rejection(
                &request,
                "idempotency_conflict",
                "idempotency key was previously accepted with different request bytes",
                false,
                Some(record.execution().clone()),
            )),
            PeerAdmissionOutcome::Accepted(record) => {
                self.notify_workers();
                Ok(acceptance(&PeerExecutionSnapshot::Hot(record), false))
            }
            PeerAdmissionOutcome::Rejected(reason) => Ok(rejection(
                &request,
                admission_rejection_code(reason),
                admission_rejection_detail(reason),
                true,
                None,
            )),
        }
    }

    /// Returns durable knowledge for a request identity without inferring from connectivity.
    pub fn lookup(
        &self,
        authenticated_peer: &PeerId,
        request: &milkdrift_peer_protocol::PeerRequestId,
    ) -> Result<InvocationLookup, ServingError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.require_operation(
            &relationship,
            AuthorityOperation::InspectPeerExecution,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        self.check_rate(&relationship, "lookup")?;
        let existing = self
            .executions
            .peer_execution_by_request(&self.peer_caller(authenticated_peer), request)
            .map_err(map_execution_persistence)?;
        if let Some(record) = &existing {
            self.require_execution_operation(
                &relationship,
                record,
                AuthorityOperation::InspectPeerExecution,
            )?;
        }
        Ok(existing.map_or_else(
            || InvocationLookup::NotAccepted {
                request_id: request.clone(),
            },
            |record| execution_lookup(&record),
        ))
    }

    /// Returns a contiguous resumable observation page for one owned execution.
    pub fn observations(
        &self,
        authenticated_peer: &PeerId,
        execution: &PeerExecutionId,
        after_sequence: u64,
        maximum: usize,
    ) -> Result<ObservationPage, ServingError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.require_operation(
            &relationship,
            AuthorityOperation::InspectPeerExecution,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        self.check_rate(&relationship, "observations")?;
        let maximum = maximum.min(usize::from(self.config.limits.observation_items));
        let limit = PageSize::new(u32::try_from(maximum).unwrap_or(u32::MAX))
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        let page = self
            .executions
            .peer_observations(
                &self.peer_caller(authenticated_peer),
                execution,
                after_sequence,
                limit,
            )
            .map_err(map_execution_persistence)?;
        self.require_execution_operation(
            &relationship,
            &page.execution,
            AuthorityOperation::InspectPeerExecution,
        )?;
        for observation in &page.observations {
            if let Some((_, reference)) = observation.event.kind().output() {
                self.authorize_peer_artifact_metadata(&relationship, reference)?;
            }
            if let Some(terminal) = observation.event.kind().terminal() {
                for reference in terminal.outputs() {
                    self.authorize_peer_artifact_metadata(&relationship, reference)?;
                }
            }
        }
        store::observation_page(
            page,
            execution,
            after_sequence,
            usize::from(self.config.limits.observation_items),
        )
    }

    /// Routes a separately authenticated cancellation and persists its acknowledgement.
    pub fn cancel(
        &self,
        authenticated_peer: &PeerId,
        request: &PeerCancellationRequest,
    ) -> Result<PeerCancellationAcknowledgement, ServingError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.check_rate(&relationship, "cancel")?;
        if request.sequence == 0 || request.reason.is_empty() || request.reason.len() > 512 {
            return Err(ServingError::Protocol(
                "invalid peer cancellation request".to_owned(),
            ));
        }
        let before = self
            .executions
            .peer_execution(&self.peer_caller(authenticated_peer), &request.execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| ServingError::NotFound("remote execution was not found".to_owned()))?;
        self.require_execution_operation(
            &relationship,
            &before,
            AuthorityOperation::CancelPeerCapability,
        )?;
        self.cancel_owned(&self.peer_caller(authenticated_peer), request, before)
    }

    fn cancel_owned(
        &self,
        caller: &milkdrift_peer_protocol::ServingCaller,
        request: &PeerCancellationRequest,
        before: PeerExecutionSnapshot,
    ) -> Result<PeerCancellationAcknowledgement, ServingError> {
        let existing_cancellation = match &before {
            PeerExecutionSnapshot::Hot(record) => record.cancellation.as_ref(),
            PeerExecutionSnapshot::Archived(tombstone) => tombstone.cancellation.as_ref(),
        };
        if let Some(existing) = existing_cancellation
            && existing.request == *request
            && let Some(acknowledgement) = &existing.acknowledgement
        {
            return Ok(acknowledgement.clone());
        }
        if let PeerExecutionSnapshot::Archived(tombstone) = &before {
            let acknowledgement = match &tombstone.disposition {
                PeerArchivedDisposition::Terminal { observation } => {
                    PeerCancellationAcknowledgement {
                        request_id: request.request_id.clone(),
                        execution: request.execution.clone(),
                        disposition: CancellationDisposition::TooLate,
                        terminal_boundary: true,
                        terminal_evidence: Some((**observation).clone()),
                        detail: Some(
                            "terminal evidence is retained in archived summary".to_owned(),
                        ),
                    }
                }
                PeerArchivedDisposition::Uncertain { .. } => PeerCancellationAcknowledgement {
                    request_id: request.request_id.clone(),
                    execution: request.execution.clone(),
                    disposition: CancellationDisposition::Unknown,
                    terminal_boundary: false,
                    terminal_evidence: None,
                    detail: Some(
                        "archived execution retains truthful outcome uncertainty".to_owned(),
                    ),
                },
            };
            acknowledgement
                .validate()
                .map_err(|error| ServingError::Protocol(error.to_string()))?;
            return Ok(acknowledgement);
        }
        let PeerExecutionSnapshot::Hot(before) = before else {
            unreachable!("archived execution returned above")
        };
        let record = self
            .executions
            .request_peer_cancellation(caller, request, self.now()?)
            .map_err(map_execution_persistence)?;
        let acknowledgement = if matches!(before.phase, PeerExecutionPhase::Terminal { .. }) {
            PeerCancellationAcknowledgement {
                request_id: request.request_id.clone(),
                execution: request.execution.clone(),
                disposition: CancellationDisposition::TooLate,
                terminal_boundary: true,
                terminal_evidence: self.terminal_observation(caller, &before)?,
                detail: Some("terminal evidence was already durable".to_owned()),
            }
        } else if matches!(before.phase, PeerExecutionPhase::Uncertain { .. }) {
            PeerCancellationAcknowledgement {
                request_id: request.request_id.clone(),
                execution: request.execution.clone(),
                disposition: CancellationDisposition::Unknown,
                terminal_boundary: false,
                terminal_evidence: None,
                detail: Some(
                    "adapter entry is known but terminal evidence is unavailable".to_owned(),
                ),
            }
        } else if record.phase.entry_evidence().is_none() {
            let terminal = self.append_cancelled_before_entry(&record)?;
            PeerCancellationAcknowledgement {
                request_id: request.request_id.clone(),
                execution: request.execution.clone(),
                disposition: CancellationDisposition::Accepted,
                terminal_boundary: true,
                terminal_evidence: Some(terminal),
                detail: Some("durable cancellation prevented adapter entry".to_owned()),
            }
        } else if record.request.selection.operation_contract().cancellation()
            == CancellationBehavior::Unsupported
        {
            PeerCancellationAcknowledgement {
                request_id: request.request_id.clone(),
                execution: request.execution.clone(),
                disposition: CancellationDisposition::Unsupported,
                terminal_boundary: false,
                terminal_evidence: None,
                detail: Some("operation does not advertise cancellation".to_owned()),
            }
        } else {
            let local = CancellationRequest::new(
                prepared::serving_invocation(&record)
                    .map_err(|error| ServingError::Protocol(error.to_string()))?,
                request.sequence,
                request.reason.clone(),
            )
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
            match self.capability_host.cancel_exact(&local) {
                Ok(value) => PeerCancellationAcknowledgement {
                    request_id: request.request_id.clone(),
                    execution: request.execution.clone(),
                    disposition: if value.accepted() {
                        CancellationDisposition::Accepted
                    } else {
                        CancellationDisposition::Rejected
                    },
                    terminal_boundary: value.terminal_boundary(),
                    terminal_evidence: None,
                    detail: value.detail().map(str::to_owned),
                },
                Err(error) => PeerCancellationAcknowledgement {
                    request_id: request.request_id.clone(),
                    execution: request.execution.clone(),
                    disposition: CancellationDisposition::Unknown,
                    terminal_boundary: false,
                    terminal_evidence: None,
                    detail: Some(bounded(&error.to_string(), 512)),
                },
            }
        };
        acknowledgement
            .validate()
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        self.executions
            .acknowledge_peer_cancellation(caller, &acknowledgement, self.now()?)
            .map_err(map_execution_persistence)?;
        self.notify_workers();
        Ok(acknowledgement)
    }

    fn drain_state(&self) -> DrainState {
        match self.drain.load(Ordering::SeqCst) {
            0 => DrainState::Ready,
            1 => DrainState::Draining,
            2 => DrainState::ShuttingDown,
            _ => DrainState::Draining,
        }
    }
}

fn execution_identity(
    caller: &milkdrift_peer_protocol::ServingCaller,
    request: &ServingInvocationRequest,
) -> Result<PeerExecutionId, ServingError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.peer.execution.v1\0");
    hasher.update(caller.storage_key().as_bytes());
    hasher.update(request.request_id.as_str().as_bytes());
    hasher.update(request.request_digest.as_bytes());
    PeerExecutionId::new(format!("exec:{}", &hasher.finalize().to_hex()[..40]))
        .map_err(|error| ServingError::Protocol(error.to_string()))
}

fn rejection(
    request: &ServingInvocationRequest,
    code: &str,
    detail: &str,
    retryable: bool,
    known_execution: Option<PeerExecutionId>,
) -> InvocationAcceptance {
    InvocationAcceptance::Rejected {
        request_id: request.request_id.clone(),
        code: code.to_owned(),
        detail: detail.to_owned(),
        retryable,
        known_execution,
    }
}

const fn relationship_generation(relationship: &PeerRelationship) -> u64 {
    relationship.revocation_generation.saturating_add(1)
}

const fn maximum_budget(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(if left > right { left } else { right }),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

const fn admission_rejection_code(reason: PeerAdmissionRejection) -> &'static str {
    match reason {
        PeerAdmissionRejection::AdmissionClosed => "draining",
        PeerAdmissionRejection::RelationshipUnavailable => "relationship_stale",
        PeerAdmissionRejection::CatalogUnavailable => "catalog_stale",
        PeerAdmissionRejection::PeerCapacity
        | PeerAdmissionRejection::GlobalCapacity
        | PeerAdmissionRejection::DispatchCapacity => "overload",
        PeerAdmissionRejection::RetentionCapacity => "retention_capacity",
    }
}

const fn admission_rejection_detail(reason: PeerAdmissionRejection) -> &'static str {
    match reason {
        PeerAdmissionRejection::AdmissionClosed => {
            "peer lifecycle recovery or draining has closed durable admission"
        }
        PeerAdmissionRejection::RelationshipUnavailable => {
            "peer relationship generation is unavailable for new acceptance"
        }
        PeerAdmissionRejection::CatalogUnavailable => {
            "selected catalog generation is unavailable for new acceptance"
        }
        PeerAdmissionRejection::PeerCapacity => "peer execution quota reached",
        PeerAdmissionRejection::GlobalCapacity => "global peer execution quota reached",
        PeerAdmissionRejection::DispatchCapacity => "durable peer dispatch queue is full",
        PeerAdmissionRejection::RetentionCapacity => {
            "peer execution retention bound requires operator archival policy"
        }
    }
}

fn bounded(value: &str, maximum: usize) -> String {
    milkdrift_contracts::truncate_utf8(value, maximum).to_owned()
}

fn map_execution_persistence(error: PersistenceError) -> ServingError {
    match error {
        PersistenceError::Storage {
            class: StorageFailureClass::ResourceExhausted,
            ..
        } => ServingError::Overloaded("durable peer owner capacity is exhausted".to_owned()),
        PersistenceError::Storage {
            class: StorageFailureClass::Unavailable | StorageFailureClass::OwnerBusy,
            ..
        } => ServingError::Unavailable("durable peer storage is unavailable".to_owned()),
        error => ServingError::Persistence(error.to_string()),
    }
}

impl From<crate::HostError> for ServingError {
    fn from(error: crate::HostError) -> Self {
        Self::Unavailable(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PeerArtifactError;

    #[test]
    fn system_clock_conversion_rejects_pre_epoch_and_overflow() {
        assert_eq!(
            unix_millis_at(UNIX_EPOCH - std::time::Duration::from_millis(1)),
            Err(PeerClockError::BeforeUnixEpoch)
        );
        assert_eq!(
            unix_millis_from_duration(std::time::Duration::from_secs(u64::MAX)),
            Err(PeerClockError::MillisecondOverflow)
        );
    }

    #[test]
    fn bounded_owner_capacity_maps_to_typed_peer_overload() {
        let failure = map_execution_persistence(PersistenceError::Storage {
            class: StorageFailureClass::ResourceExhausted,
            message: "owner queue full".to_owned(),
        });
        assert!(matches!(failure, ServingError::Overloaded(_)));
        assert!(matches!(
            ServingError::from(PeerArtifactError::Overloaded("owner queue full".to_owned())),
            ServingError::Overloaded(_)
        ));
    }

    #[test]
    fn empty_peer_filters_are_explicit_deny_all_not_wildcards()
    -> Result<(), Box<dyn std::error::Error>> {
        let capability = milkdrift_capability::CapabilityId::new("peer-capability")?;
        let operation = milkdrift_capability::OperationId::new("peer.execute")?;
        assert!(
            peer_capability_authority(
                BTreeSet::new(),
                BTreeSet::from([operation.clone()]),
                milkdrift_capability::SideEffectClass::ReadOnly,
            )?
            .denies_all()
        );
        assert!(
            peer_capability_authority(
                BTreeSet::from([capability.clone()]),
                BTreeSet::new(),
                milkdrift_capability::SideEffectClass::ReadOnly,
            )?
            .denies_all()
        );

        let exact = peer_capability_authority(
            BTreeSet::from([capability.clone()]),
            BTreeSet::from([operation.clone()]),
            milkdrift_capability::SideEffectClass::ReadOnly,
        )?;
        assert!(
            exact
                .identity_selection()
                .is_some_and(|selection| selection.matches(&capability))
        );
        assert!(
            exact
                .operation_selection()
                .is_some_and(|selection| selection.matches(&operation))
        );
        assert!(!exact.denies_all());
        Ok(())
    }
}
