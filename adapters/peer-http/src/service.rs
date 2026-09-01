use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
    time::{SystemTime, UNIX_EPOCH},
};

use milkdrift_authority::{
    ActorRef, ArtifactAuthorityScope, AuthorityBudget, AuthorityEvaluator,
    AuthorityExecutionProvenance, AuthorityGrant, AuthorityGrantBuilder, AuthorityOperation,
    AuthorityRequest, BoundaryTimeMillis, CapabilityAuthorityScope,
    CapabilityAuthorityScopeBuilder, CapabilityExecutionRequirements, DaemonAuthorityScope,
    DecisionId, GrantId, GrantSetEvaluator, LayoutAuthorityScope, NetworkScope, PeerAuthorityScope,
    PeerId, PolicyId, RequestedResourceFacts, ResourceScope, Selection, WorkflowRunScope,
    WorkspaceAuthorityScope,
};
use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_capability::{
    CancellationBehavior, CancellationRequest, CapabilityDescriptor, DescriptorBuilder, ErrorClass,
    InvocationEvent, InvocationEventKind, InvocationFailure, InvocationTerminal, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterExecutionContext, AdapterReporter, CapabilityHost, CatalogGenerationView,
};
use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision, ArtifactTransferDirection,
    CancellationDisposition, CatalogEntry, CatalogSnapshot, DrainState, HandshakeRequest,
    HandshakeResponse, InvocationAcceptance, InvocationLookup, ObservationCategory,
    ObservationHistory, ObservationPage, PeerAction, PeerCancellationAcknowledgement,
    PeerCancellationRequest, PeerExecutionId, PeerInvocationRequest, PeerObservation,
    RemoteExecutionStatus, TransferId,
};
use milkdrift_persistence::{
    AttemptId, NodeExecutionId, PageSize, PeerAdmission, PeerAdmissionOutcome,
    PeerAdmissionRejection, PeerArchivedDisposition, PeerCatalogState, PeerClaimOutcome,
    PeerDispatchClaimRequest, PeerEntryOutcome, PeerEntryRequest, PeerExecutionPhase,
    PeerExecutionRecord, PeerExecutionSnapshot, PeerExecutionStatus, PeerExecutionStore,
    PeerRelationshipState, PeerRetentionRequest, PersistenceError, StorageFailureClass,
    TimestampMillis, WorkerId,
};
use milkdrift_workspace::RunId;
use subtle::ConstantTimeEq as _;

use crate::{
    PeerAuthenticator, PeerHttpError,
    artifact::{PeerArtifactError, PeerArtifactStore},
    config::{PeerRelationship, PeerServerConfig},
    dispatch::PeerDispatchWorkers,
    store::{acceptance, archived_summary, lookup as execution_lookup, snapshot_status},
};

/// Caller-supplied boundary clock for deterministic protocol and restart tests.
pub trait PeerClock: Send + Sync {
    /// Current Unix epoch milliseconds.
    fn now_unix_ms(&self) -> u64;
}

/// Production boundary clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemPeerClock;

impl PeerClock for SystemPeerClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
            })
    }
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
        let now = self.clock.now_unix_ms();
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

    /// Derives a complete filtered, expiring catalog from the live capability host.
    pub fn catalog(&self, authenticated_peer: &PeerId) -> Result<CatalogSnapshot, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.require_operation(
            &relationship,
            AuthorityOperation::InspectPeer,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        self.require_operation(
            &relationship,
            AuthorityOperation::ListCapabilities,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        self.require_operation(
            &relationship,
            AuthorityOperation::InspectCapabilityHealth,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        self.require_operation(
            &relationship,
            AuthorityOperation::InspectProviderProfile,
            RequestedResourceFacts::empty(),
            AuthorityBudget::default(),
        )?;
        self.check_rate(&relationship, "catalog")?;
        let now = self.clock.now_unix_ms();
        let entries = self.catalog_entries(&relationship, now)?;
        let fingerprint = catalog_fingerprint(&entries)?;
        let mut catalogs = self
            .catalogs
            .lock()
            .map_err(|_| PeerHttpError::Unavailable("catalog cache unavailable".to_owned()))?;
        if let Some(cached) = catalogs.get(authenticated_peer)
            && cached.fingerprint == fingerprint
            && cached.snapshot.is_live_at(now)
        {
            return Ok(cached.snapshot.clone());
        }
        let durable_generation = self
            .executions
            .peer_catalog(authenticated_peer)
            .map_err(map_execution_persistence)?
            .map_or(0, |catalog| catalog.generation);
        let generation = catalogs.get(authenticated_peer).map_or(
            durable_generation.saturating_add(1).max(1),
            |cached| {
                cached
                    .snapshot
                    .generation
                    .max(durable_generation)
                    .saturating_add(1)
            },
        );
        let expires_at = now
            .saturating_add(relationship.catalog_ttl_ms)
            .min(relationship.expires_at_unix_ms);
        let snapshot = CatalogSnapshot::new(generation, now, expires_at, entries)
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        self.executions
            .publish_peer_catalog(&PeerCatalogState {
                peer: authenticated_peer.clone(),
                relationship_generation: relationship_generation(&relationship),
                generation: snapshot.generation,
                digest: snapshot.digest.as_str().to_owned(),
                expires_at_unix_ms: snapshot.expires_at_unix_ms,
            })
            .map_err(map_execution_persistence)?;
        catalogs.insert(
            authenticated_peer.clone(),
            CachedCatalog {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        Ok(snapshot)
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
        let now = self.clock.now_unix_ms();
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
            .request_peer_cancellation(authenticated_peer, request, self.clock.now_unix_ms().max(1))
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
            .acknowledge_peer_cancellation(
                authenticated_peer,
                &acknowledgement,
                self.clock.now_unix_ms().max(1),
            )
            .map_err(map_execution_persistence)?;
        self.notify_workers();
        Ok(acknowledgement)
    }

    /// Negotiates a metadata-first authorized upload or download.
    pub fn negotiate_artifact(
        &self,
        authenticated_peer: &PeerId,
        offer: &ArtifactMetadataOffer,
    ) -> Result<ArtifactTransferDecision, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        let operation = match offer.direction {
            ArtifactTransferDirection::Upload => AuthorityOperation::PeerArtifactUpload,
            ArtifactTransferDirection::Download => AuthorityOperation::PeerArtifactDownload,
        };
        let snapshot = self
            .executions
            .peer_execution(authenticated_peer, &offer.execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| {
                PeerHttpError::Unauthorized(
                    "artifact is not bound to an execution owned by this peer".to_owned(),
                )
            })?;
        let record = match snapshot {
            PeerExecutionSnapshot::Hot(record) => record,
            PeerExecutionSnapshot::Archived(_)
                if offer.direction == ArtifactTransferDirection::Download =>
            {
                return Err(PeerHttpError::NotFound(
                    "archived execution observation-to-artifact history was compacted; core artifact retention is unchanged"
                        .to_owned(),
                ));
            }
            PeerExecutionSnapshot::Archived(tombstone) => {
                return Err(PeerHttpError::Unauthorized(format!(
                    "artifact upload cannot target archived execution {}",
                    tombstone.execution
                )));
            }
        };
        if offer.direction == ArtifactTransferDirection::Download {
            if offer.source_peer != self.config.local_peer {
                return Err(PeerHttpError::Unauthorized(
                    "download source is not the serving peer".to_owned(),
                ));
            }
            let mut produced = false;
            for sequence in 1..=record.last_observation_sequence {
                if self
                    .executions
                    .peer_observation_artifact(&record.execution, sequence)
                    .map_err(map_execution_persistence)?
                    .as_ref()
                    .is_some_and(|artifact| {
                        workspace_artifact_matches_capability(&offer.artifact, artifact)
                    })
                {
                    produced = true;
                    break;
                }
            }
            if !produced {
                return Err(PeerHttpError::Unauthorized(
                    "artifact is not a durable output of the claimed execution".to_owned(),
                ));
            }
        }
        self.require_operation(
            &relationship,
            operation,
            artifact_resource_facts(&offer.artifact, offer.sensitivity),
            AuthorityBudget {
                artifact_bytes: Some(offer.artifact.size_bytes()),
                ..AuthorityBudget::default()
            },
        )?;
        self.check_rate(
            &relationship,
            match offer.direction {
                ArtifactTransferDirection::Upload => "artifact_upload_negotiate",
                ArtifactTransferDirection::Download => "artifact_download_negotiate",
            },
        )?;
        if self.clock.now_unix_ms() > offer.expires_at_unix_ms {
            return Err(PeerHttpError::Unauthorized(
                "artifact transfer authority expired".to_owned(),
            ));
        }
        self.artifacts
            .negotiate(
                authenticated_peer,
                offer,
                relationship.maximum_artifact_bytes,
            )
            .map_err(Into::into)
    }

    /// Accepts one sequential bounded artifact chunk.
    pub fn write_artifact_chunk(
        &self,
        authenticated_peer: &PeerId,
        chunk: &ArtifactChunk,
    ) -> Result<ArtifactTransferDecision, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        let facts = self
            .artifacts
            .transfer_facts(authenticated_peer, &chunk.transfer)?;
        if facts.direction != ArtifactTransferDirection::Upload {
            return Err(PeerHttpError::Unauthorized(
                "artifact transfer direction is not upload".to_owned(),
            ));
        }
        self.require_operation(
            &relationship,
            AuthorityOperation::PeerArtifactUpload,
            artifact_resource_facts(&facts.artifact, facts.sensitivity),
            AuthorityBudget {
                artifact_bytes: Some(u64::try_from(chunk.bytes.len()).unwrap_or(u64::MAX)),
                ..AuthorityBudget::default()
            },
        )?;
        self.check_rate(&relationship, "artifact_upload_chunk")?;
        self.artifacts
            .write_chunk(
                authenticated_peer,
                chunk,
                self.config.limits.artifact_chunk_bytes,
            )
            .map_err(Into::into)
    }

    /// Returns one authorized verified artifact range.
    pub fn read_artifact_chunk(
        &self,
        authenticated_peer: &PeerId,
        transfer: &TransferId,
        offset: u64,
        maximum_bytes: u32,
    ) -> Result<ArtifactChunk, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        let facts = self
            .artifacts
            .transfer_facts(authenticated_peer, transfer)?;
        if facts.direction != ArtifactTransferDirection::Download {
            return Err(PeerHttpError::Unauthorized(
                "artifact transfer direction is not download".to_owned(),
            ));
        }
        self.require_operation(
            &relationship,
            AuthorityOperation::PeerArtifactDownload,
            artifact_resource_facts(&facts.artifact, facts.sensitivity),
            AuthorityBudget {
                artifact_bytes: Some(u64::from(maximum_bytes)),
                ..AuthorityBudget::default()
            },
        )?;
        self.check_rate(&relationship, "artifact_download_chunk")?;
        self.artifacts
            .read_chunk(
                authenticated_peer,
                transfer,
                offset,
                maximum_bytes.min(self.config.limits.artifact_chunk_bytes),
            )
            .map_err(Into::into)
    }

    /// Aborts an incomplete artifact transfer and removes temporary bytes.
    pub fn abort_artifact(
        &self,
        authenticated_peer: &PeerId,
        transfer: &TransferId,
    ) -> Result<(), PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        let facts = self
            .artifacts
            .transfer_facts(authenticated_peer, transfer)?;
        let operation = match facts.direction {
            ArtifactTransferDirection::Upload => AuthorityOperation::PeerArtifactUpload,
            ArtifactTransferDirection::Download => AuthorityOperation::PeerArtifactDownload,
        };
        self.require_operation(
            &relationship,
            operation,
            artifact_resource_facts(&facts.artifact, facts.sensitivity),
            AuthorityBudget::default(),
        )?;
        self.check_rate(&relationship, "artifact_abort")?;
        self.artifacts.abort(authenticated_peer, transfer)?;
        Ok(())
    }

    /// Marks all catalogs stale and stops accepting new peer invocations.
    pub fn begin_drain(&self) -> Result<(), PeerHttpError> {
        self.executions
            .set_peer_admission_open(false)
            .map_err(map_execution_persistence)?;
        self.drain.store(1, Ordering::SeqCst);
        self.notify_workers();
        Ok(())
    }

    /// Marks shutdown state for handshake and catalog consumers.
    pub fn begin_shutdown(&self) -> Result<(), PeerHttpError> {
        let closed = self
            .executions
            .set_peer_admission_open(false)
            .map_err(map_execution_persistence);
        self.drain.store(2, Ordering::SeqCst);
        self.notify_workers();
        closed
    }

    /// Stops durable claims and joins the fixed worker owner up to the supplied deadline.
    pub fn shutdown_workers(&self, timeout: Duration) -> PeerWorkerShutdownReport {
        let admission_closed = self.begin_shutdown().is_ok();
        let Ok(mut workers) = self.workers.lock() else {
            return PeerWorkerShutdownReport {
                clean: false,
                joined: 0,
                retained_workers: self.config.workers.threads,
            };
        };
        let mut report = workers.as_mut().map_or(
            PeerWorkerShutdownReport {
                clean: true,
                joined: 0,
                retained_workers: 0,
            },
            |owner| owner.shutdown(timeout),
        );
        report.clean &= admission_closed;
        report
    }

    /// Revokes one relationship immediately for inbound authentication and protocol actions.
    pub fn revoke_peer(&self, peer: &PeerId) -> Result<(), PeerHttpError> {
        let Some(relationship) = self.relationships.get(peer) else {
            return Err(PeerHttpError::NotFound(
                "peer relationship is not configured".to_owned(),
            ));
        };
        self.executions
            .configure_peer_relationship(&PeerRelationshipState {
                peer: peer.clone(),
                generation: relationship_generation(relationship).saturating_add(1),
                enabled: false,
                expires_at_unix_ms: relationship.expires_at_unix_ms,
                maximum_active: u32::from(relationship.maximum_concurrent),
            })
            .map_err(map_execution_persistence)?;
        self.revoked_peers
            .lock()
            .map_err(|_| {
                PeerHttpError::Unavailable("peer revocation state unavailable".to_owned())
            })?
            .insert(peer.clone());
        self.catalogs
            .lock()
            .map_err(|_| PeerHttpError::Unavailable("catalog cache unavailable".to_owned()))?
            .remove(peer);
        Ok(())
    }

    /// Recovers bounded prior-owner claims. Pre-entry work requeues; entered work becomes uncertain.
    pub fn recover(self: &Arc<Self>, maximum: usize) -> Result<(), PeerHttpError> {
        let configured = usize::from(self.config.workers.recovery_page);
        let bounded = maximum.min(configured).max(1);
        let limit = PageSize::new(u32::try_from(bounded).unwrap_or(u32::MAX))
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        loop {
            let recovered = self
                .executions
                .recover_peer_claims(self.clock.now_unix_ms().max(1), limit)
                .map_err(map_execution_persistence)?;
            if !recovered.more {
                break;
            }
        }
        self.executions
            .verify_peer_execution_integrity()
            .map_err(map_execution_persistence)?;
        self.maintain_retention()?;
        self.executions
            .verify_peer_execution_integrity()
            .map_err(map_execution_persistence)?;
        self.executions
            .set_peer_admission_open(true)
            .map_err(map_execution_persistence)?;
        self.drain.store(0, Ordering::SeqCst);
        self.notify_workers();
        Ok(())
    }

    /// Compacts one bounded page beyond the configured hot observation horizon.
    pub fn maintain_retention(&self) -> Result<PeerExecutionStatus, PeerHttpError> {
        let now = self.clock.now_unix_ms().max(1);
        let retention_ms = u64::try_from(self.config.workers.observation_hot_retention.as_millis())
            .unwrap_or(u64::MAX);
        self.executions
            .archive_peer_executions(&PeerRetentionRequest {
                terminal_before_or_at: TimestampMillis::new(
                    now.saturating_sub(retention_ms).max(1),
                ),
                archived_at: TimestampMillis::new(now),
                limit: PageSize::new(self.config.workers.archive_batch_size)
                    .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
            })
            .map_err(map_execution_persistence)?;
        self.executions
            .peer_execution_status()
            .map_err(map_execution_persistence)
    }

    /// Returns redacted serving execution accounting for daemon health projection.
    pub fn execution_status(&self) -> Result<PeerExecutionStatus, PeerHttpError> {
        self.executions
            .peer_execution_status()
            .map_err(map_execution_persistence)
    }

    fn relationship(&self, peer: &PeerId) -> Result<PeerRelationship, PeerHttpError> {
        let relationship = self
            .relationships
            .get(peer)
            .cloned()
            .ok_or(PeerHttpError::Unauthenticated)?;
        if !relationship.enabled
            || self.clock.now_unix_ms() > relationship.expires_at_unix_ms
            || self
                .revoked_peers
                .lock()
                .map_or(true, |revoked| revoked.contains(peer))
        {
            return Err(PeerHttpError::Unauthenticated);
        }
        Ok(relationship)
    }

    fn require_operation(
        &self,
        relationship: &PeerRelationship,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        budget: AuthorityBudget,
    ) -> Result<(), PeerHttpError> {
        let decision = self.evaluate_operation(relationship, operation, resources, budget)?;
        if decision.is_allowed() {
            Ok(())
        } else {
            Err(PeerHttpError::Unauthorized(format!(
                "peer authority denied the operation ({})",
                decision
                    .reason_codes()
                    .iter()
                    .map(|reason| format!("{reason:?}").to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join(",")
            )))
        }
    }

    fn evaluate_operation(
        &self,
        relationship: &PeerRelationship,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        budget: AuthorityBudget,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, PeerHttpError> {
        self.evaluate_operation_with_provenance(
            relationship,
            operation,
            resources,
            budget,
            AuthorityExecutionProvenance::default(),
        )
    }

    fn evaluate_operation_with_provenance(
        &self,
        relationship: &PeerRelationship,
        operation: AuthorityOperation,
        mut resources: RequestedResourceFacts,
        budget: AuthorityBudget,
        provenance: AuthorityExecutionProvenance,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, PeerHttpError> {
        let grant = self
            .grants
            .get(&relationship.remote_peer)
            .ok_or_else(|| PeerHttpError::Unauthorized("peer grant is absent".to_owned()))?;
        resources.peer = Some(relationship.remote_peer.clone());
        let now = self.clock.now_unix_ms();
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.peer-authority.v1\0");
        hasher.update(relationship.remote_peer.as_str().as_bytes());
        hasher.update(format!("{operation:?}{resources:?}{budget:?}{now}").as_bytes());
        let request = AuthorityRequest {
            decision: DecisionId::new(format!("decision:{}", hasher.finalize()))
                .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
            actor: grant.actor().clone(),
            grant: grant.identity().clone(),
            grant_revision: grant.revision(),
            grant_digest: grant
                .digest()
                .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
            revocation_generation: grant.revocation_generation(),
            operation,
            resources,
            budget,
            evaluated_at: BoundaryTimeMillis::new(now),
            provenance,
        };
        self.authority
            .evaluate(&request)
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))
    }

    fn check_rate(
        &self,
        relationship: &PeerRelationship,
        bucket: &str,
    ) -> Result<(), PeerHttpError> {
        let now = self.clock.now_unix_ms();
        let key = (relationship.remote_peer.clone(), bucket.to_owned());
        let mut windows = self
            .rate_windows
            .lock()
            .map_err(|_| PeerHttpError::Unavailable("peer rate state unavailable".to_owned()))?;
        let window = windows.entry(key).or_insert(RateWindow {
            started_at_unix_ms: now,
            requests: 0,
        });
        if now >= window.started_at_unix_ms.saturating_add(60_000) {
            *window = RateWindow {
                started_at_unix_ms: now,
                requests: 0,
            };
        }
        if window.requests >= relationship.maximum_requests_per_minute {
            return Err(PeerHttpError::Overloaded(
                "authenticated peer request-rate quota reached".to_owned(),
            ));
        }
        window.requests = window.requests.saturating_add(1);
        Ok(())
    }

    fn drain_state(&self) -> DrainState {
        match self.drain.load(Ordering::SeqCst) {
            0 => DrainState::Ready,
            1 => DrainState::Draining,
            2 => DrainState::ShuttingDown,
            _ => DrainState::Draining,
        }
    }

    fn catalog_entries(
        &self,
        relationship: &PeerRelationship,
        now: u64,
    ) -> Result<Vec<CatalogEntry>, PeerHttpError> {
        if self.drain_state() != DrainState::Ready {
            return Ok(Vec::new());
        }
        let scope = &self
            .grants
            .get(&relationship.remote_peer)
            .ok_or_else(|| PeerHttpError::Unauthorized("peer grant is absent".to_owned()))?
            .resources()
            .capability;
        let generations = self.capability_host.catalog_generations(scope)?;
        let mut entries = Vec::new();
        for generation in generations {
            if !generation.current || generation.draining {
                continue;
            }
            let Some(ref observation) = generation.observation else {
                continue;
            };
            if !observation.available() {
                continue;
            }
            let operations = generation
                .descriptor
                .operations()
                .iter()
                .filter(|(identity, contract)| {
                    scope
                        .operation_selection()
                        .is_some_and(|selection| selection.matches(*identity))
                        && contract.side_effect() <= scope.maximum_side_effect()
                })
                .map(|(identity, contract)| (identity.clone(), contract.clone()))
                .collect::<BTreeMap<_, _>>();
            if operations.is_empty() {
                continue;
            }
            let invocable_operations = operations.keys().cloned().collect::<BTreeSet<_>>();
            let descriptor = filtered_descriptor(&generation, operations)?;
            let observation = milkdrift_capability::CapabilityObservation::new(
                descriptor.identity().clone(),
                observation
                    .observed_at_unix_ms()
                    .max(now.saturating_sub(300_000)),
                observation.available(),
                observation.current_load(),
                bounded(observation.health_summary(), 512),
            )
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
            entries.push(CatalogEntry {
                descriptor,
                invocable_operations,
                observation,
                draining: false,
            });
        }
        Ok(entries)
    }

    fn authorize_invocation(
        &self,
        relationship: &PeerRelationship,
        request: &PeerInvocationRequest,
        descriptor: &CapabilityDescriptor,
        requirements: &CapabilityExecutionRequirements,
        now: u64,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, PeerHttpError> {
        let _validated_context = adapter_execution_context(request)?;
        if !relationship.execution_limits.contains(request.limits)
            || request.limits.artifact_bytes > relationship.maximum_artifact_bytes
        {
            return Err(PeerHttpError::Unauthorized(
                "capability, operation, side effect, or quota is not granted".to_owned(),
            ));
        }
        let mut resources = RequestedResourceFacts::empty();
        resources.capability = Some(descriptor.identity().clone());
        resources.category = Some(descriptor.category().clone());
        resources.capability_operation = Some(request.selection.operation().clone());
        resources.provider_profile = descriptor.provider_profile().cloned();
        resources.trust_zones = descriptor.trust_zones().clone();
        resources.execution_trust_class = Some(descriptor.execution_trust());
        resources.locality = Some(descriptor.locality());
        resources.peer = descriptor.peer().cloned();
        resources.side_effect = request.selection.operation_contract().side_effect();
        resources.filesystem = requirements.filesystem.clone();
        resources.network_profiles = requirements.network_profiles.clone();
        resources.network_destinations = requirements.network_destinations.clone();
        resources.secrets = requirements.secrets.clone();
        let delegated = &request.delegation.provenance;
        let provenance = AuthorityExecutionProvenance {
            revision: Some(parse_revision(&delegated.revision)?),
            node: Some(
                NodeId::new(delegated.node.clone())
                    .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
            ),
            execution: Some(delegated.execution.clone()),
            attempt: Some(delegated.attempt.clone()),
            descriptor_revision: Some(request.selection.descriptor_revision()),
            peer: Some(relationship.remote_peer.clone()),
            idempotency: Some(request.selection.operation_contract().idempotency()),
        };
        let decision = self.evaluate_operation_with_provenance(
            relationship,
            AuthorityOperation::InvokePeerCapability,
            resources,
            AuthorityBudget {
                cost_minor: maximum_budget(
                    Some(request.limits.cost_micros.saturating_add(9_999) / 10_000),
                    requirements.budget.cost_minor,
                ),
                duration_ms: maximum_budget(
                    Some(request.limits.duration_ms),
                    requirements.budget.duration_ms,
                ),
                invocations: maximum_budget(Some(1), requirements.budget.invocations),
                artifact_bytes: maximum_budget(
                    Some(request.limits.artifact_bytes),
                    requirements.budget.artifact_bytes,
                ),
                units: requirements.budget.units,
                concurrency: Some(requirements.budget.concurrency.unwrap_or(1).max(1)),
            },
            provenance,
        )?;
        if !decision.is_allowed() {
            return Err(PeerHttpError::Unauthorized(
                "peer capability invocation authority is not granted".to_owned(),
            ));
        }
        let expected_actor = milkdrift_authority::ActorRef::new(format!(
            "peer:{}",
            relationship.remote_peer.as_str()
        ))
        .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        let delegation = &request.delegation;
        if delegation.reference != relationship.delegation
            || delegation.issuer_peer != relationship.remote_peer
            || delegation.target_peer != self.config.local_peer
            || delegation.actor != expected_actor
            || delegation.expires_at_unix_ms < now
            || delegation.expires_at_unix_ms > relationship.expires_at_unix_ms
        {
            return Err(PeerHttpError::Unauthorized(
                "delegation record is absent, expired, or does not match authenticated facts"
                    .to_owned(),
            ));
        }
        Ok(decision)
    }

    fn exact_generation(
        &self,
        relationship: &PeerRelationship,
        request: &PeerInvocationRequest,
    ) -> Result<CatalogGenerationView, PeerHttpError> {
        let scope = &self
            .grants
            .get(&relationship.remote_peer)
            .ok_or_else(|| PeerHttpError::Unauthorized("peer grant is absent".to_owned()))?
            .resources()
            .capability;
        self.capability_host
            .catalog_generations(scope)?
            .into_iter()
            .find(|generation| {
                generation.descriptor.identity() == request.selection.capability()
                    && generation.descriptor.descriptor_revision()
                        == request.selection.descriptor_revision()
                    && scope
                        .operation_selection()
                        .is_some_and(|selection| selection.matches(request.selection.operation()))
                    && generation
                        .descriptor
                        .operation(request.selection.operation())
                        .is_some()
            })
            .ok_or_else(|| {
                PeerHttpError::Unauthorized(
                    "selected local capability generation is no longer registered".to_owned(),
                )
            })
    }

    pub(crate) fn worker_claims_enabled(&self) -> bool {
        self.drain.load(Ordering::SeqCst) == 0
    }

    pub(crate) fn claim_for_worker(
        &self,
        worker: &WorkerId,
    ) -> Result<PeerClaimOutcome, PeerHttpError> {
        let now = self.clock.now_unix_ms().max(1);
        self.executions
            .claim_peer_dispatch(&PeerDispatchClaimRequest {
                worker,
                claimed_at_unix_ms: now,
                lease_expires_at_unix_ms: now.saturating_add(self.config.lease.execution_lease_ms),
            })
            .map_err(map_execution_persistence)
    }

    pub(crate) fn run_claimed(&self, record: PeerExecutionRecord) -> Result<(), PeerHttpError> {
        let claim = record.phase.claim().cloned().ok_or_else(|| {
            PeerHttpError::Persistence("claimed peer work lacks a claim".to_owned())
        })?;
        if matches!(
            &record.phase,
            PeerExecutionPhase::CancellationRequested { evidence: None, .. }
        ) {
            let terminal = self.append_cancelled_before_entry(&record)?;
            if record
                .cancellation
                .as_ref()
                .is_some_and(|value| value.acknowledgement.is_none())
            {
                let cancellation = record.cancellation.as_ref().ok_or_else(|| {
                    PeerHttpError::Persistence("cancellation facts disappeared".to_owned())
                })?;
                self.executions
                    .acknowledge_peer_cancellation(
                        &record.owner_peer,
                        &PeerCancellationAcknowledgement {
                            request_id: cancellation.request.request_id.clone(),
                            execution: record.execution.clone(),
                            disposition: CancellationDisposition::Accepted,
                            terminal_boundary: true,
                            terminal_evidence: Some(terminal),
                            detail: Some("durable cancellation prevented adapter entry".to_owned()),
                        },
                        self.clock.now_unix_ms().max(1),
                    )
                    .map_err(map_execution_persistence)?;
            }
            return Ok(());
        }
        if self.clock.now_unix_ms() > record.request.deadline_unix_ms {
            return self.append_pre_entry_failure(
                &record,
                "peer execution deadline elapsed before adapter entry",
            );
        }
        if self.drain_state() != DrainState::Ready {
            return self.append_pre_entry_failure(
                &record,
                "peer service stopped admission before adapter entry",
            );
        }
        let relationship = match self.relationship(&record.owner_peer) {
            Ok(relationship) => relationship,
            Err(_) => {
                return self.append_pre_entry_failure(
                    &record,
                    "peer relationship was revoked or expired before adapter entry",
                );
            }
        };
        let generation = match self.exact_generation(&relationship, &record.request) {
            Ok(generation) => generation,
            Err(_) => {
                return self.append_pre_entry_failure(
                    &record,
                    "selected capability generation was unavailable before adapter entry",
                );
            }
        };
        let entry_authority = match self.authorize_invocation(
            &relationship,
            &record.request,
            &generation.descriptor,
            &generation.authority_requirements,
            self.clock.now_unix_ms(),
        ) {
            Ok(decision) => decision,
            Err(_) => {
                return self.append_pre_entry_failure(
                    &record,
                    "peer execution authority was denied before adapter entry",
                );
            }
        };
        let entered = match self
            .executions
            .mark_peer_entered(&PeerEntryRequest {
                owner: &record.owner_peer,
                execution: &record.execution,
                worker: &claim.worker,
                claim_generation: claim.generation,
                relationship_generation: relationship_generation(&relationship),
                entered_at_unix_ms: self.clock.now_unix_ms().max(1),
                authority: &entry_authority,
            })
            .map_err(map_execution_persistence)?
        {
            PeerEntryOutcome::Entered(entered) => *entered,
            PeerEntryOutcome::AdmissionClosed => {
                return self.append_pre_entry_failure(
                    &record,
                    "peer service stopped admission before adapter entry",
                );
            }
            PeerEntryOutcome::RelationshipUnavailable => {
                return self.append_pre_entry_failure(
                    &record,
                    "peer relationship was revoked or expired before adapter entry",
                );
            }
        };
        let reporter = PeerStoreReporter {
            owner_peer: entered.owner_peer.clone(),
            execution: entered.execution.clone(),
            executions: self.executions.clone(),
            clock: self.clock.clone(),
            lease_ms: self.config.lease.execution_lease_ms,
            limits: entered.request.limits,
            input_artifact_bytes: entered
                .request
                .input_artifact_bytes()
                .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
            deadline_unix_ms: entered.request.deadline_unix_ms,
            worker: claim.worker.clone(),
            claim_generation: claim.generation,
        };
        let context = adapter_execution_context(&entered.request)?;
        let result = self.capability_host.execute_exact_with_context(
            &entered.request.selection,
            &entered.request.request,
            &context,
            &reporter,
        );
        let current = self
            .executions
            .peer_execution(&entered.owner_peer, &entered.execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| PeerHttpError::NotFound("remote execution was not found".to_owned()))?;
        let PeerExecutionSnapshot::Hot(current) = current else {
            return Ok(());
        };
        if matches!(
            current.phase,
            PeerExecutionPhase::Terminal { .. } | PeerExecutionPhase::Uncertain { .. }
        ) {
            return Ok(());
        }
        let reason = result.map_or_else(
            |error| bounded(&error.to_string(), 2_048),
            |()| "peer adapter returned without terminal evidence".to_owned(),
        );
        self.executions
            .mark_peer_uncertain(
                &entered.owner_peer,
                &entered.execution,
                &claim.worker,
                claim.generation,
                self.clock.now_unix_ms().max(1),
                &reason,
            )
            .map_err(map_execution_persistence)?;
        Ok(())
    }

    pub(crate) fn recover_panicked_worker(
        &self,
        claimed: &PeerExecutionRecord,
        worker: &WorkerId,
    ) -> Result<(), PeerHttpError> {
        let current = self
            .executions
            .peer_execution(&claimed.owner_peer, &claimed.execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| PeerHttpError::NotFound("remote execution was not found".to_owned()))?;
        let PeerExecutionSnapshot::Hot(current) = current else {
            return Ok(());
        };
        let Some(claim) = current.phase.claim() else {
            return Ok(());
        };
        if claim.worker != *worker {
            return Ok(());
        }
        if current.phase.entry_evidence().is_some() {
            self.executions
                .mark_peer_uncertain(
                    &current.owner_peer,
                    &current.execution,
                    worker,
                    claim.generation,
                    self.clock.now_unix_ms().max(1),
                    "peer worker panicked after durable adapter entry",
                )
                .map_err(map_execution_persistence)?;
        } else {
            self.executions
                .release_peer_claim(
                    &current.owner_peer,
                    &current.execution,
                    worker,
                    claim.generation,
                    self.clock.now_unix_ms().max(1),
                )
                .map_err(map_execution_persistence)?;
            self.notify_workers();
        }
        Ok(())
    }

    fn append_pre_entry_failure(
        &self,
        record: &PeerExecutionRecord,
        reason: &str,
    ) -> Result<(), PeerHttpError> {
        let sequence = record.last_observation_sequence.saturating_add(1);
        let failure = InvocationFailure::new(
            ErrorClass::Adapter,
            false,
            "peer_host_failure_before_entry",
            bounded(reason, 2_048),
            None,
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let terminal = InvocationTerminal::new(
            TerminalStatus::Failure,
            Vec::new(),
            Some(failure),
            None,
            milkdrift_capability::SideEffectClass::None,
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let event = InvocationEvent::new(
            record.request.request.invocation().clone(),
            sequence,
            InvocationEventKind::Terminal { terminal },
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        self.executions
            .append_peer_observation(
                &record.owner_peer,
                &record.execution,
                &PeerObservation {
                    execution: record.execution.clone(),
                    sequence,
                    category: ObservationCategory::Terminal,
                    event,
                    observed_at_unix_ms: self.clock.now_unix_ms().max(1),
                },
            )
            .map_err(map_execution_persistence)?;
        Ok(())
    }

    fn append_cancelled_before_entry(
        &self,
        record: &PeerExecutionRecord,
    ) -> Result<PeerObservation, PeerHttpError> {
        let current = self
            .executions
            .peer_execution(&record.owner_peer, &record.execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| PeerHttpError::NotFound("remote execution was not found".to_owned()))?;
        let PeerExecutionSnapshot::Hot(current) = current else {
            return Err(PeerHttpError::Persistence(
                "active cancellation unexpectedly resolved an archived execution".to_owned(),
            ));
        };
        if let PeerExecutionPhase::Terminal { sequence, .. } = current.phase {
            return self
                .terminal_observation(&record.owner_peer, &current)?
                .filter(|observation| observation.sequence == sequence)
                .ok_or_else(|| {
                    PeerHttpError::Persistence(
                        "terminal cancellation evidence is missing".to_owned(),
                    )
                });
        }
        let sequence = current.last_observation_sequence.saturating_add(1);
        let terminal = InvocationTerminal::new(
            TerminalStatus::Cancelled,
            Vec::new(),
            None,
            None,
            milkdrift_capability::SideEffectClass::None,
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let event = InvocationEvent::new(
            current.request.request.invocation().clone(),
            sequence,
            InvocationEventKind::Terminal { terminal },
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let observation = PeerObservation {
            execution: current.execution.clone(),
            sequence,
            category: ObservationCategory::Terminal,
            event,
            observed_at_unix_ms: self.clock.now_unix_ms().max(1),
        };
        self.executions
            .append_peer_observation(&current.owner_peer, &current.execution, &observation)
            .map_err(map_execution_persistence)?;
        Ok(observation)
    }

    fn terminal_observation(
        &self,
        owner: &PeerId,
        record: &PeerExecutionRecord,
    ) -> Result<Option<PeerObservation>, PeerHttpError> {
        if record.last_observation_sequence == 0 {
            return Ok(None);
        }
        let page = self
            .executions
            .peer_observations(
                owner,
                &record.execution,
                record.last_observation_sequence.saturating_sub(1),
                PageSize::new(1).map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
            )
            .map_err(map_execution_persistence)?;
        Ok(page
            .observations
            .into_iter()
            .next()
            .filter(|observation| observation.event.kind().terminal().is_some()))
    }

    fn notify_workers(&self) {
        if let Ok(workers) = self.workers.lock()
            && let Some(workers) = workers.as_ref()
        {
            workers.notify();
        }
    }
}

fn adapter_execution_context(
    request: &PeerInvocationRequest,
) -> Result<AdapterExecutionContext, PeerHttpError> {
    let provenance = &request.delegation.provenance;
    Ok(AdapterExecutionContext::new(
        RunId::new(provenance.run.clone())
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
        parse_revision(&provenance.revision)?,
        NodeId::new(provenance.node.clone())
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
        NodeExecutionId::new(provenance.execution.clone())
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
        AttemptId::new(provenance.attempt.clone())
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
    ))
}

fn parse_revision(value: &str) -> Result<RevisionId, PeerHttpError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))
}

fn peer_authority_grant(relationship: &PeerRelationship) -> Result<AuthorityGrant, PeerHttpError> {
    let actions = relationship.authority.actions();
    let mut operations = BTreeSet::new();
    if actions.is_empty() {
        // Authority grants require a nonempty closed vocabulary. This operation is never used by
        // the peer transport and therefore preserves the relationship's default deny behavior.
        operations.insert(AuthorityOperation::Inspect);
    } else {
        operations.insert(AuthorityOperation::NegotiatePeerSession);
    }
    if actions.contains(&PeerAction::ReadCatalog) {
        operations.extend([
            AuthorityOperation::InspectPeer,
            AuthorityOperation::ListCapabilities,
            AuthorityOperation::InspectCapabilityHealth,
            AuthorityOperation::InspectProviderProfile,
        ]);
    }
    if actions.contains(&PeerAction::Invoke) {
        operations.extend([
            AuthorityOperation::InvokePeerCapability,
            AuthorityOperation::InspectPeerExecution,
        ]);
    }
    if actions.contains(&PeerAction::Cancel) {
        operations.insert(AuthorityOperation::CancelPeerCapability);
    }
    if actions.contains(&PeerAction::ArtifactUpload) {
        operations.insert(AuthorityOperation::PeerArtifactUpload);
    }
    if actions.contains(&PeerAction::ArtifactDownload) {
        operations.insert(AuthorityOperation::PeerArtifactDownload);
    }
    if actions.contains(&PeerAction::Administer) {
        operations.insert(AuthorityOperation::AdministerPeer);
    }

    let identities: BTreeSet<_> = relationship
        .capability_allow
        .difference(&relationship.capability_deny)
        .cloned()
        .collect();
    let capability = peer_capability_authority(
        identities,
        relationship.operation_allow.clone(),
        relationship.maximum_side_effect,
    )?;
    let resource_scope = ResourceScope {
        workflow_run: WorkflowRunScope::Any,
        capability,
        filesystem: relationship.execution_filesystem.clone(),
        network: NetworkScope::new(
            relationship.execution_network_profiles.clone(),
            relationship.execution_network_destinations.clone(),
        )
        .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
        secrets: relationship.execution_secrets.clone(),
        artifacts: if (actions.contains(&PeerAction::ArtifactUpload)
            || actions.contains(&PeerAction::ArtifactDownload))
            && !relationship.artifact_sensitivities.is_empty()
        {
            ArtifactAuthorityScope::new(
                Selection::any(),
                relationship.artifact_sensitivities.clone(),
            )
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?
        } else {
            ArtifactAuthorityScope::none()
        },
        layouts: LayoutAuthorityScope::none(),
        peers: PeerAuthorityScope::new(BTreeSet::from([relationship.remote_peer.clone()]), false)
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
        daemon: DaemonAuthorityScope::default(),
        workspace: WorkspaceAuthorityScope::none(),
    };
    let peer_hash = blake3::hash(relationship.remote_peer.as_str().as_bytes());
    AuthorityGrantBuilder::new(
        GrantId::new(format!("grant:peer-{}", &peer_hash.to_hex().as_str()[..24]))
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
        relationship.revocation_generation.saturating_add(1).max(1),
        ActorRef::new(format!("peer:{}", relationship.remote_peer.as_str()))
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
    )
    .operations(operations)
    .resources(resource_scope)
    .budget(AuthorityBudget {
        cost_minor: Some(
            relationship
                .execution_limits
                .cost_micros
                .saturating_add(9_999)
                / 10_000,
        ),
        duration_ms: Some(relationship.execution_limits.duration_ms),
        invocations: Some(1),
        artifact_bytes: Some(relationship.maximum_artifact_bytes),
        concurrency: Some(u32::from(relationship.maximum_concurrent)),
        ..AuthorityBudget::default()
    })
    .validity(
        BoundaryTimeMillis::new(0),
        BoundaryTimeMillis::new(relationship.expires_at_unix_ms),
    )
    .revocation_generation(relationship.revocation_generation)
    .build()
    .map_err(|error| PeerHttpError::Configuration(error.to_string()))
}

fn peer_capability_authority(
    identities: BTreeSet<milkdrift_capability::CapabilityId>,
    operations: BTreeSet<milkdrift_capability::OperationId>,
    maximum_side_effect: milkdrift_capability::SideEffectClass,
) -> Result<CapabilityAuthorityScope, PeerHttpError> {
    if identities.is_empty() || operations.is_empty() {
        Ok(CapabilityAuthorityScope::deny_all())
    } else {
        Ok(CapabilityAuthorityScopeBuilder::new(maximum_side_effect)
            .only_capabilities(identities)
            .and_then(|builder| builder.only_operations(operations))
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?
            .build())
    }
}

struct PeerStoreReporter {
    owner_peer: PeerId,
    execution: PeerExecutionId,
    executions: Arc<dyn PeerExecutionStore>,
    clock: Arc<dyn PeerClock>,
    lease_ms: u64,
    limits: milkdrift_peer_protocol::ExecutionLimits,
    input_artifact_bytes: u64,
    deadline_unix_ms: u64,
    worker: WorkerId,
    claim_generation: u64,
}

impl PeerStoreReporter {
    fn reject_report(&self, code: &str, detail: &str) -> AdapterError {
        let reason = bounded(&format!("{code}: {detail}"), 2_048);
        let _ = self.executions.mark_peer_uncertain(
            &self.owner_peer,
            &self.execution,
            &self.worker,
            self.claim_generation,
            self.clock.now_unix_ms().max(1),
            &reason,
        );
        AdapterError::external_failure(reason)
    }
}

impl AdapterReporter for PeerStoreReporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        if self.clock.now_unix_ms() > self.deadline_unix_ms {
            return Err(
                self.reject_report("peer_report_deadline", "peer execution deadline elapsed")
            );
        }
        let maximum = u64::from(self.limits.observations);
        if event.sequence() > maximum
            || (event.sequence() == maximum && event.kind().terminal().is_none())
        {
            return Err(self.reject_report(
                "peer_report_observation_quota",
                "peer observation quota reached before terminal evidence",
            ));
        }
        if let InvocationEventKind::Terminal { terminal } = event.kind() {
            if terminal.usage().is_some_and(|usage| {
                usage
                    .duration_ms()
                    .is_some_and(|duration| duration > self.limits.duration_ms)
                    || usage
                        .cost_micros()
                        .is_some_and(|cost| cost > self.limits.cost_micros)
            }) {
                return Err(self.reject_report(
                    "peer_report_usage_quota",
                    "peer terminal usage exceeds the accepted duration or cost quota",
                ));
            }
            let output_bytes = terminal
                .outputs()
                .iter()
                .try_fold(self.input_artifact_bytes, |total, output| {
                    output.size_bytes().and_then(|size| total.checked_add(size))
                });
            if output_bytes.is_none_or(|bytes| bytes > self.limits.artifact_bytes) {
                return Err(self.reject_report(
                    "peer_report_artifact_quota",
                    "peer output artifact bytes are absent or exceed the accepted quota",
                ));
            }
        }
        let category = match event.kind() {
            InvocationEventKind::Progress { .. } => ObservationCategory::Progress,
            InvocationEventKind::Output { .. } => ObservationCategory::Artifact,
            InvocationEventKind::Terminal { terminal }
                if terminal.status() == TerminalStatus::Uncertain =>
            {
                ObservationCategory::Uncertainty
            }
            InvocationEventKind::Terminal { .. } => ObservationCategory::Terminal,
        };
        self.executions
            .append_peer_observation(
                &self.owner_peer,
                &self.execution,
                &PeerObservation {
                    execution: self.execution.clone(),
                    sequence: event.sequence(),
                    category,
                    event,
                    observed_at_unix_ms: self.clock.now_unix_ms().max(1),
                },
            )
            .map(|_outcome| ())
            .map_err(|error| {
                self.reject_report("peer_report_rejected", &bounded(&error.to_string(), 1_900))
            })
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        if self.clock.now_unix_ms() > self.deadline_unix_ms {
            return Err(
                self.reject_report("peer_heartbeat_deadline", "peer execution deadline elapsed")
            );
        }
        self.executions
            .extend_peer_claim(
                &self.owner_peer,
                &self.execution,
                &self.worker,
                self.claim_generation,
                self.clock.now_unix_ms().saturating_add(self.lease_ms),
            )
            .map_err(|error| AdapterError::external_failure(error.to_string()))
    }
}

fn filtered_descriptor(
    generation: &CatalogGenerationView,
    operations: BTreeMap<
        milkdrift_capability::OperationId,
        milkdrift_capability::OperationContract,
    >,
) -> Result<CapabilityDescriptor, PeerHttpError> {
    DescriptorBuilder::new(
        generation.descriptor.identity().clone(),
        generation.descriptor.descriptor_revision(),
        generation.descriptor.category().clone(),
        generation.descriptor.admission().clone(),
        generation.descriptor.locality(),
    )
    .provider_profile(generation.descriptor.provider_profile().cloned())
    .operations(operations)
    .trust_zones(generation.descriptor.trust_zones().clone())
    .execution_trust(generation.descriptor.execution_trust())
    .resource_observations(generation.descriptor.resource_observations().cloned())
    .labels(generation.descriptor.labels().clone())
    .extensions(generation.descriptor.extensions().clone())
    .build()
    .map_err(|error| PeerHttpError::Protocol(error.to_string()))
}

fn catalog_fingerprint(entries: &[CatalogEntry]) -> Result<String, PeerHttpError> {
    let bytes =
        serde_json::to_vec(entries).map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
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

fn artifact_resource_facts(
    artifact: &milkdrift_workspace::ArtifactReference,
    sensitivity: milkdrift_workspace::ArtifactSensitivity,
) -> RequestedResourceFacts {
    let mut resources = RequestedResourceFacts::empty();
    resources.artifact = Some(artifact.artifact().clone());
    resources.artifact_sensitivity = Some(sensitivity);
    resources
}

fn workspace_artifact_matches_capability(
    workspace: &milkdrift_workspace::ArtifactReference,
    capability: &milkdrift_capability::ArtifactReference,
) -> bool {
    capability.identity() == workspace.artifact().as_str()
        && capability.digest() == workspace.digest().to_string()
        && capability.media_type() == Some(workspace.media_type().as_str())
        && capability.size_bytes() == Some(workspace.size_bytes())
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

impl From<PeerArtifactError> for PeerHttpError {
    fn from(error: PeerArtifactError) -> Self {
        match error {
            PeerArtifactError::Rejected(message) => Self::Unauthorized(message),
            PeerArtifactError::Conflict(message) | PeerArtifactError::Verification(message) => {
                Self::Protocol(message)
            }
            PeerArtifactError::Persistence(message) => Self::Persistence(message),
            PeerArtifactError::Overloaded(message) => Self::Overloaded(message),
            PeerArtifactError::Unavailable => {
                Self::Persistence("artifact state unavailable".to_owned())
            }
        }
    }
}

struct DisabledArtifactStore;

impl PeerArtifactStore for DisabledArtifactStore {
    fn transfer_facts(
        &self,
        _owner_peer: &PeerId,
        _transfer: &TransferId,
    ) -> Result<crate::PeerArtifactTransferFacts, PeerArtifactError> {
        Err(PeerArtifactError::Rejected(
            "peer artifact transfer is disabled".to_owned(),
        ))
    }

    fn negotiate(
        &self,
        _owner_peer: &PeerId,
        _offer: &ArtifactMetadataOffer,
        _maximum_artifact_bytes: u64,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError> {
        Err(PeerArtifactError::Rejected(
            "peer artifact storage is not configured".to_owned(),
        ))
    }

    fn write_chunk(
        &self,
        _owner_peer: &PeerId,
        _chunk: &ArtifactChunk,
        _maximum_chunk_bytes: u32,
    ) -> Result<ArtifactTransferDecision, PeerArtifactError> {
        Err(PeerArtifactError::Rejected(
            "peer artifact storage is not configured".to_owned(),
        ))
    }

    fn read_chunk(
        &self,
        _owner_peer: &PeerId,
        _transfer: &TransferId,
        _offset: u64,
        _maximum_bytes: u32,
    ) -> Result<ArtifactChunk, PeerArtifactError> {
        Err(PeerArtifactError::Rejected(
            "peer artifact storage is not configured".to_owned(),
        ))
    }

    fn abort(&self, _owner_peer: &PeerId, _transfer: &TransferId) -> Result<(), PeerArtifactError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
