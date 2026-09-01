mod artifact_transfer;
mod authority;
mod catalog;
mod lifecycle;
mod worker;

use artifact_transfer::DisabledArtifactStore;
#[cfg(test)]
use authority::peer_capability_authority;
use authority::{adapter_execution_context, peer_authority_grant};

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use milkdrift_authority::{
    AuthorityBudget, AuthorityGrant, AuthorityOperation, GrantSetEvaluator, PeerId, PolicyId,
    RequestedResourceFacts,
};
use milkdrift_capability::{CancellationBehavior, CancellationRequest};
use milkdrift_capability_host::CapabilityHost;
use milkdrift_peer_protocol::{
    CancellationDisposition, CatalogSnapshot, DrainState, HandshakeRequest, HandshakeResponse,
    InvocationAcceptance, InvocationLookup, ObservationHistory, ObservationPage,
    PeerCancellationAcknowledgement, PeerCancellationRequest, PeerExecutionId,
    PeerInvocationRequest, RemoteExecutionStatus,
};
use milkdrift_persistence::{
    PageSize, PeerAdmission, PeerAdmissionOutcome, PeerAdmissionRejection, PeerArchivedDisposition,
    PeerExecutionPhase, PeerExecutionSnapshot, PeerExecutionStore, PeerRelationshipState,
    PersistenceError, StorageFailureClass,
};
use subtle::ConstantTimeEq as _;
use thiserror::Error;

use crate::{
    PeerAuthenticator, PeerHttpError,
    artifact::PeerArtifactStore,
    config::{PeerRelationship, PeerServerConfig},
    dispatch::PeerDispatchWorkers,
    store::{acceptance, archived_summary, lookup as execution_lookup, snapshot_status},
};

/// Caller-supplied boundary clock for deterministic protocol and restart tests.
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

/// Production boundary clock.
#[derive(Debug, Default)]
pub struct SystemPeerClock {
    last_unix_ms: Mutex<u64>,
}

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

fn unix_millis_at(now: SystemTime) -> Result<u64, PeerClockError> {
    let duration = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PeerClockError::BeforeUnixEpoch)?;
    unix_millis_from_duration(duration)
}

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

/// Authenticated peer application service shared by HTTP routes and deterministic tests.
pub struct PeerService {
    config: PeerServerConfig,
    relationships: BTreeMap<PeerId, PeerRelationship>,
    grants: BTreeMap<PeerId, AuthorityGrant>,
    authority: GrantSetEvaluator,
    capability_host: CapabilityHost,
    executions: Arc<dyn PeerExecutionStore>,
    clock: Arc<dyn PeerClock>,
    catalogs: Mutex<BTreeMap<PeerId, CachedCatalog>>,
    rate_windows: Mutex<BTreeMap<(PeerId, String), RateWindow>>,
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
    pub(crate) fn http_connection_limit(&self) -> usize {
        usize::from(self.config.limits.connections)
    }

    /// Constructs a ready service. Call [`Self::recover`] after local adapters register.
    pub fn new(
        config: PeerServerConfig,
        capability_host: CapabilityHost,
        executions: Arc<dyn PeerExecutionStore>,
        clock: Arc<dyn PeerClock>,
    ) -> Result<Arc<Self>, PeerHttpError> {
        Self::new_with_artifacts(
            config,
            capability_host,
            executions,
            Arc::new(DisabledArtifactStore),
            clock,
        )
    }

    /// Constructs a ready service with a verified artifact exchange port.
    pub fn new_with_artifacts(
        config: PeerServerConfig,
        capability_host: CapabilityHost,
        executions: Arc<dyn PeerExecutionStore>,
        artifacts: Arc<dyn PeerArtifactStore>,
        clock: Arc<dyn PeerClock>,
    ) -> Result<Arc<Self>, PeerHttpError> {
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
    ) -> Result<Arc<Self>, PeerHttpError> {
        config.validate()?;
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
                .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
            1,
            grants.values().cloned(),
            BTreeMap::new(),
        )
        .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        executions
            .set_peer_admission_open(false)
            .map_err(map_execution_persistence)?;
        for relationship in relationships.values() {
            executions
                .configure_peer_relationship(&PeerRelationshipState {
                    peer: relationship.remote_peer.clone(),
                    generation: relationship_generation(relationship),
                    enabled: relationship.enabled,
                    expires_at_unix_ms: relationship.expires_at_unix_ms,
                    maximum_active: u32::from(relationship.maximum_concurrent),
                })
                .map_err(map_execution_persistence)?;
        }
        let worker_config = config.workers;
        let service = Arc::new(Self {
            config,
            relationships,
            grants,
            authority,
            capability_host,
            executions,
            clock,
            catalogs: Mutex::new(BTreeMap::new()),
            rate_windows: Mutex::new(BTreeMap::new()),
            revoked_peers: Mutex::new(BTreeSet::new()),
            // Recovery owns startup. Workers and inbound admission remain closed until it finishes.
            drain: AtomicU8::new(3),
            artifacts,
            authenticator,
            workers: Mutex::new(None),
        });
        let workers = PeerDispatchWorkers::start(Arc::downgrade(&service), worker_config)?;
        *service.workers.lock().map_err(|_| {
            PeerHttpError::Unavailable("peer worker owner unavailable".to_owned())
        })? = Some(workers);
        Ok(service)
    }

    /// Authenticates only the transport bearer value and returns its configured identity.
    /// Request payload identity fields never choose this result.
    pub fn authenticate_bearer(&self, supplied: &[u8]) -> Result<PeerId, PeerHttpError> {
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
                .ok_or(PeerHttpError::Unauthenticated);
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
            .ok_or(PeerHttpError::Unauthenticated)
    }

    pub(super) fn now(&self) -> Result<u64, PeerHttpError> {
        self.clock
            .now_unix_ms()
            .map_err(|error| PeerHttpError::Unavailable(error.to_string()))
    }

    /// Negotiates a session and cross-checks the claimed identity against authentication.
    pub fn handshake(
        &self,
        authenticated_peer: &PeerId,
        request: &HandshakeRequest,
    ) -> Result<HandshakeResponse, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.require_operation(
            &relationship,
            AuthorityOperation::NegotiatePeerSession,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        if &request.claimed_peer != authenticated_peer {
            return Err(PeerHttpError::Unauthorized(
                "handshake identity does not match transport authentication".to_owned(),
            ));
        }
        self.check_rate(&relationship, "handshake")?;
        request
            .limits
            .validate()
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
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
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
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
    pub fn invoke(
        self: &Arc<Self>,
        authenticated_peer: &PeerId,
        request: PeerInvocationRequest,
    ) -> Result<InvocationAcceptance, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        request
            .validate()
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        if let Some(existing) = self
            .executions
            .peer_execution_by_request(authenticated_peer, &request.request_id)
            .map_err(map_execution_persistence)?
        {
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
        self.check_rate(
            &relationship,
            &format!("invoke:{}", request.selection.operation().as_str()),
        )?;
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
                PeerHttpError::Unauthorized(
                    "selected capability generation is not advertised".to_owned(),
                )
            })?;
        request
            .selection
            .validate_against(&entry.descriptor)
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        if !entry
            .invocable_operations
            .contains(request.selection.operation())
        {
            return Err(PeerHttpError::Unauthorized(
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
        let execution = execution_identity(authenticated_peer, &request)?;
        match self
            .executions
            .admit_peer_execution(&PeerAdmission {
                owner_peer: authenticated_peer,
                request: &request,
                authority: &authority_decision,
                execution: &execution,
                relationship_generation: relationship_generation(&relationship),
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
    ) -> Result<InvocationLookup, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.require_operation(
            &relationship,
            AuthorityOperation::InspectPeerExecution,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        self.check_rate(&relationship, "lookup")?;
        Ok(self
            .executions
            .peer_execution_by_request(authenticated_peer, request)
            .map_err(map_execution_persistence)?
            .map_or_else(
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
    ) -> Result<ObservationPage, PeerHttpError> {
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
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let page = self
            .executions
            .peer_observations(authenticated_peer, execution, after_sequence, limit)
            .map_err(map_execution_persistence)?;
        let status = snapshot_status(&page.execution);
        let history = match &page.execution {
            PeerExecutionSnapshot::Hot(_) => ObservationHistory::Hot,
            PeerExecutionSnapshot::Archived(tombstone) => ObservationHistory::Archived {
                summary: Box::new(archived_summary(tombstone)),
            },
        };
        let terminal = status == RemoteExecutionStatus::Terminal;
        let archived = matches!(page.execution, PeerExecutionSnapshot::Archived(_));
        let page = ObservationPage {
            execution: execution.clone(),
            after_sequence,
            next_sequence: page
                .observations
                .last()
                .map_or(after_sequence, |observation| observation.sequence),
            observations: page.observations,
            terminal,
            closed: terminal || archived,
            history,
        };
        page.validate(usize::from(self.config.limits.observation_items))
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        Ok(page)
    }

    /// Routes a separately authenticated cancellation and persists its acknowledgement.
    pub fn cancel(
        &self,
        authenticated_peer: &PeerId,
        request: &PeerCancellationRequest,
    ) -> Result<PeerCancellationAcknowledgement, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.check_rate(&relationship, "cancel")?;
        if request.sequence == 0 || request.reason.is_empty() || request.reason.len() > 512 {
            return Err(PeerHttpError::Protocol(
                "invalid peer cancellation request".to_owned(),
            ));
        }
        let before = self
            .executions
            .peer_execution(authenticated_peer, &request.execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| PeerHttpError::NotFound("remote execution was not found".to_owned()))?;
        let mut resources = RequestedResourceFacts::empty();
        match &before {
            PeerExecutionSnapshot::Hot(record) => {
                resources.capability = Some(record.request.selection.capability().clone());
                resources.capability_operation = Some(record.request.selection.operation().clone());
                resources.side_effect = record.request.selection.operation_contract().side_effect();
            }
            PeerExecutionSnapshot::Archived(tombstone) => {
                resources.capability = Some(tombstone.capability.clone());
                resources.capability_operation = Some(tombstone.operation.clone());
                resources.side_effect = tombstone.side_effect;
            }
        }
        self.require_operation(
            &relationship,
            AuthorityOperation::CancelPeerCapability,
            resources,
            AuthorityBudget::default(),
        )?;
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
                .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
            return Ok(acknowledgement);
        }
        let PeerExecutionSnapshot::Hot(before) = before else {
            unreachable!("archived execution returned above")
        };
        let record = self
            .executions
            .request_peer_cancellation(authenticated_peer, request, self.now()?)
            .map_err(map_execution_persistence)?;
        let acknowledgement = if matches!(before.phase, PeerExecutionPhase::Terminal { .. }) {
            PeerCancellationAcknowledgement {
                request_id: request.request_id.clone(),
                execution: request.execution.clone(),
                disposition: CancellationDisposition::TooLate,
                terminal_boundary: true,
                terminal_evidence: self.terminal_observation(authenticated_peer, &before)?,
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
                record.request.request.invocation().clone(),
                request.sequence,
                request.reason.clone(),
            )
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
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
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        self.executions
            .acknowledge_peer_cancellation(authenticated_peer, &acknowledgement, self.now()?)
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
    peer: &PeerId,
    request: &PeerInvocationRequest,
) -> Result<PeerExecutionId, PeerHttpError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.peer.execution.v1\0");
    hasher.update(peer.as_str().as_bytes());
    hasher.update(request.request_id.as_str().as_bytes());
    hasher.update(request.request_digest.as_bytes());
    PeerExecutionId::new(format!("exec:{}", &hasher.finalize().to_hex()[..40]))
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))
}

fn rejection(
    request: &PeerInvocationRequest,
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
    if value.len() <= maximum {
        return value.to_owned();
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_owned()
}

fn map_execution_persistence(error: PersistenceError) -> PeerHttpError {
    match error {
        PersistenceError::Storage {
            class: StorageFailureClass::ResourceExhausted,
            ..
        } => PeerHttpError::Overloaded("durable peer owner capacity is exhausted".to_owned()),
        PersistenceError::Storage {
            class: StorageFailureClass::Unavailable | StorageFailureClass::OwnerBusy,
            ..
        } => PeerHttpError::Unavailable("durable peer storage is unavailable".to_owned()),
        error => PeerHttpError::Persistence(error.to_string()),
    }
}

impl From<milkdrift_capability_host::HostError> for PeerHttpError {
    fn from(error: milkdrift_capability_host::HostError) -> Self {
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
        assert!(matches!(failure, PeerHttpError::Overloaded(_)));
        assert!(matches!(
            PeerHttpError::from(PeerArtifactError::Overloaded("owner queue full".to_owned())),
            PeerHttpError::Overloaded(_)
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
