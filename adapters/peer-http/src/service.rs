use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use milkdrift_authority::{CapabilityAuthorityScope, PeerId};
use milkdrift_capability::{
    CancellationBehavior, CancellationRequest, CapabilityDescriptor, DescriptorBuilder, ErrorClass,
    InvocationEvent, InvocationEventKind, InvocationFailure, InvocationTerminal, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterReporter, CapabilityHost, CatalogGenerationView,
};
use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision, ArtifactTransferDirection,
    CancellationDisposition, CatalogEntry, CatalogSnapshot, DrainState, HandshakeRequest,
    HandshakeResponse, InvocationAcceptance, InvocationLookup, ObservationCategory,
    ObservationPage, PeerAction, PeerCancellationAcknowledgement, PeerCancellationRequest,
    PeerExecutionId, PeerInvocationRequest, PeerObservation, RemoteExecutionStatus, TransferId,
};
use subtle::ConstantTimeEq as _;

use crate::{
    PeerAuthenticator, PeerHttpError,
    artifact::{PeerArtifactError, PeerArtifactStore},
    config::{PeerRelationship, PeerServerConfig},
    store::{PeerExecutionStore, PeerStoreError, StoreAcceptance, StoredExecution, acceptance},
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
    capability_host: CapabilityHost,
    executions: Arc<dyn PeerExecutionStore>,
    clock: Arc<dyn PeerClock>,
    catalogs: Mutex<BTreeMap<PeerId, CachedCatalog>>,
    rate_windows: Mutex<BTreeMap<(PeerId, String), RateWindow>>,
    revoked_peers: Mutex<BTreeSet<PeerId>>,
    drain: AtomicU8,
    artifacts: Arc<dyn PeerArtifactStore>,
    authenticator: Option<Arc<dyn PeerAuthenticator>>,
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
        let relationships = config
            .relationships
            .iter()
            .cloned()
            .map(|relationship| (relationship.remote_peer.clone(), relationship))
            .collect();
        Ok(Arc::new(Self {
            config,
            relationships,
            capability_host,
            executions,
            clock,
            catalogs: Mutex::new(BTreeMap::new()),
            rate_windows: Mutex::new(BTreeMap::new()),
            revoked_peers: Mutex::new(BTreeSet::new()),
            drain: AtomicU8::new(0),
            artifacts,
            authenticator,
        }))
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
            },
            limits: self.config.limits.intersect(request.limits),
            lease: self.config.lease,
            drain: self.drain_state(),
        })
    }

    /// Derives a complete filtered, expiring catalog from the live capability host.
    pub fn catalog(&self, authenticated_peer: &PeerId) -> Result<CatalogSnapshot, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.require(&relationship, PeerAction::ReadCatalog)?;
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
        let generation = catalogs.get(authenticated_peer).map_or(
            relationship.revocation_generation.saturating_add(1).max(1),
            |cached| cached.snapshot.generation.saturating_add(1),
        );
        let expires_at = now
            .saturating_add(relationship.catalog_ttl_ms)
            .min(relationship.expires_at_unix_ms);
        let snapshot = CatalogSnapshot::new(generation, now, expires_at, entries)
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        catalogs.insert(
            authenticated_peer.clone(),
            CachedCatalog {
                fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        Ok(snapshot)
    }

    /// Durably accepts one exact request before starting exactly one execution thread.
    pub fn invoke(
        self: &Arc<Self>,
        authenticated_peer: &PeerId,
        request: PeerInvocationRequest,
    ) -> Result<InvocationAcceptance, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.require(&relationship, PeerAction::Invoke)?;
        if self.drain_state() != DrainState::Ready {
            return Ok(rejection(
                &request,
                "draining",
                "peer is draining",
                true,
                None,
            ));
        }
        request
            .validate()
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        if let Some(record) = self
            .executions
            .by_request(authenticated_peer, &request.request_id)?
        {
            return if record.request.request_digest == request.request_digest {
                Ok(acceptance(&record, true))
            } else {
                Ok(rejection(
                    &request,
                    "idempotency_conflict",
                    "idempotency key was previously accepted with different request bytes",
                    false,
                    Some(record.execution),
                ))
            };
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
        self.authorize_invocation(&relationship, &request, now)?;
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
        let active = self
            .executions
            .recoverable(10_000)?
            .into_iter()
            .filter(|record| record.owner_peer == *authenticated_peer)
            .count();
        if active >= usize::from(relationship.maximum_concurrent) {
            return Ok(rejection(
                &request,
                "overload",
                "peer execution quota reached",
                true,
                None,
            ));
        }
        let execution = execution_identity(authenticated_peer, &request)?;
        let lease_expires = now.saturating_add(self.config.lease.execution_lease_ms);
        match self.executions.accept(
            authenticated_peer,
            &request,
            &execution,
            now,
            lease_expires,
        )? {
            StoreAcceptance::Replay(record) => Ok(acceptance(&record, true)),
            StoreAcceptance::Conflict(record) => Ok(rejection(
                &request,
                "idempotency_conflict",
                "idempotency key was previously accepted with different request bytes",
                false,
                Some(record.execution),
            )),
            StoreAcceptance::New(record) => {
                self.spawn_execution(record.clone())?;
                Ok(acceptance(&record, false))
            }
        }
    }

    /// Returns durable knowledge for a request identity without inferring from connectivity.
    pub fn lookup(
        &self,
        authenticated_peer: &PeerId,
        request: &milkdrift_peer_protocol::PeerRequestId,
    ) -> Result<InvocationLookup, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        self.require(&relationship, PeerAction::Invoke)?;
        self.check_rate(&relationship, "lookup")?;
        Ok(self
            .executions
            .by_request(authenticated_peer, request)?
            .map_or(InvocationLookup::NotAccepted, |record| record.lookup()))
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
        self.require(&relationship, PeerAction::Invoke)?;
        self.check_rate(&relationship, "observations")?;
        let record = self
            .executions
            .by_execution(authenticated_peer, execution)?
            .ok_or_else(|| PeerHttpError::NotFound("remote execution was not found".to_owned()))?;
        let observations = record
            .observations
            .iter()
            .filter(|observation| observation.sequence > after_sequence)
            .take(maximum.min(usize::from(self.config.limits.observation_items)))
            .cloned()
            .collect::<Vec<_>>();
        let terminal = record.status == RemoteExecutionStatus::Terminal;
        let page = ObservationPage {
            execution: execution.clone(),
            after_sequence,
            next_sequence: observations
                .last()
                .map_or(after_sequence, |observation| observation.sequence),
            observations,
            terminal,
            closed: terminal,
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
        self.require(&relationship, PeerAction::Cancel)?;
        self.check_rate(&relationship, "cancel")?;
        if request.sequence == 0 || request.reason.is_empty() || request.reason.len() > 512 {
            return Err(PeerHttpError::Protocol(
                "invalid peer cancellation request".to_owned(),
            ));
        }
        let record = self
            .executions
            .by_execution(authenticated_peer, &request.execution)?
            .ok_or_else(|| PeerHttpError::NotFound("remote execution was not found".to_owned()))?;
        let acknowledgement = if record.status == RemoteExecutionStatus::Terminal {
            PeerCancellationAcknowledgement {
                request_id: request.request_id.clone(),
                execution: request.execution.clone(),
                disposition: CancellationDisposition::TooLate,
                terminal_boundary: true,
                terminal_evidence: record.observations.last().cloned(),
                detail: Some("terminal evidence was already durable".to_owned()),
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
        self.executions.record_cancellation(
            authenticated_peer,
            &request.execution,
            acknowledgement.clone(),
        )?;
        Ok(acknowledgement)
    }

    /// Negotiates a metadata-first authorized upload or download.
    pub fn negotiate_artifact(
        &self,
        authenticated_peer: &PeerId,
        offer: &ArtifactMetadataOffer,
    ) -> Result<ArtifactTransferDecision, PeerHttpError> {
        let relationship = self.relationship(authenticated_peer)?;
        let action = match offer.direction {
            ArtifactTransferDirection::Upload => PeerAction::ArtifactUpload,
            ArtifactTransferDirection::Download => PeerAction::ArtifactDownload,
        };
        self.require(&relationship, action)?;
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
        self.require(&relationship, PeerAction::ArtifactUpload)?;
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
        self.require(&relationship, PeerAction::ArtifactDownload)?;
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
        if !relationship.authority.permits(PeerAction::ArtifactUpload)
            && !relationship.authority.permits(PeerAction::ArtifactDownload)
        {
            return Err(PeerHttpError::Unauthorized(
                "artifact transfer authority is not granted".to_owned(),
            ));
        }
        self.check_rate(&relationship, "artifact_abort")?;
        self.artifacts.abort(authenticated_peer, transfer)?;
        Ok(())
    }

    /// Marks all catalogs stale and stops accepting new peer invocations.
    pub fn begin_drain(&self) {
        self.drain.store(1, Ordering::SeqCst);
    }

    /// Marks shutdown state for handshake and catalog consumers.
    pub fn begin_shutdown(&self) {
        self.drain.store(2, Ordering::SeqCst);
    }

    /// Revokes one relationship immediately for inbound authentication and protocol actions.
    pub fn revoke_peer(&self, peer: &PeerId) -> Result<(), PeerHttpError> {
        if !self.relationships.contains_key(peer) {
            return Err(PeerHttpError::NotFound(
                "peer relationship is not configured".to_owned(),
            ));
        }
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

    /// Recovers durable acceptance after daemon restart without duplicating entered work.
    /// Accepted-before-entry records may enter once; entered records become explicitly uncertain.
    pub fn recover(self: &Arc<Self>, maximum: usize) -> Result<(), PeerHttpError> {
        for record in self.executions.recoverable(maximum)? {
            match record.status {
                RemoteExecutionStatus::Accepted => self.spawn_execution(record)?,
                RemoteExecutionStatus::Running | RemoteExecutionStatus::OutcomeUnknown => {
                    self.append_uncertainty(
                        &record,
                        "remote daemon restarted after adapter-entry intent; outcome is unknown",
                    )?;
                }
                RemoteExecutionStatus::Terminal => {}
            }
        }
        Ok(())
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

    fn require(
        &self,
        relationship: &PeerRelationship,
        action: PeerAction,
    ) -> Result<(), PeerHttpError> {
        if relationship.authority.permits(action) {
            Ok(())
        } else {
            Err(PeerHttpError::Unauthorized(format!(
                "peer action {action:?} is not granted"
            )))
        }
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
            _ => DrainState::ShuttingDown,
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
        let scope = CapabilityAuthorityScope::any(relationship.maximum_side_effect);
        let generations = self.capability_host.catalog_generations(&scope)?;
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
                    relationship.permits_capability(generation.descriptor.identity(), identity)
                        && contract.side_effect() <= relationship.maximum_side_effect
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
        now: u64,
    ) -> Result<(), PeerHttpError> {
        if !relationship.permits_capability(
            request.selection.capability(),
            request.selection.operation(),
        ) || request.selection.operation_contract().side_effect()
            > relationship.maximum_side_effect
            || !relationship.execution_limits.contains(request.limits)
            || request.limits.artifact_bytes > relationship.maximum_artifact_bytes
        {
            return Err(PeerHttpError::Unauthorized(
                "capability, operation, side effect, or quota is not granted".to_owned(),
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
        Ok(())
    }

    fn spawn_execution(self: &Arc<Self>, record: StoredExecution) -> Result<(), PeerHttpError> {
        self.executions
            .mark_running(&record.owner_peer, &record.execution)?;
        let service = self.clone();
        thread::Builder::new()
            .name(format!(
                "milkdrift-peer-{}",
                &record.execution.as_str()[..record.execution.as_str().len().min(48)]
            ))
            .spawn(move || service.run_execution(record))
            .map_err(|error| PeerHttpError::Unavailable(error.to_string()))?;
        Ok(())
    }

    fn run_execution(self: Arc<Self>, record: StoredExecution) {
        let reporter = PeerStoreReporter {
            owner_peer: record.owner_peer.clone(),
            execution: record.execution.clone(),
            executions: self.executions.clone(),
            clock: self.clock.clone(),
            lease_ms: self.config.lease.execution_lease_ms,
            limits: record.request.limits,
            deadline_unix_ms: record.request.deadline_unix_ms,
        };
        if let Err(error) = self.capability_host.execute_exact(
            &record.request.selection,
            &record.request.request,
            &reporter,
        ) && self
            .executions
            .by_execution(&record.owner_peer, &record.execution)
            .ok()
            .flatten()
            .is_some_and(|current| current.status != RemoteExecutionStatus::Terminal)
        {
            let _ = self.append_uncertainty(&record, &bounded(&error.to_string(), 2_048));
        }
    }

    fn append_uncertainty(
        &self,
        record: &StoredExecution,
        reason: &str,
    ) -> Result<(), PeerHttpError> {
        let current = self
            .executions
            .by_execution(&record.owner_peer, &record.execution)?
            .ok_or_else(|| PeerHttpError::NotFound("remote execution was not found".to_owned()))?;
        if current.status == RemoteExecutionStatus::Terminal {
            return Ok(());
        }
        let sequence = current
            .observations
            .last()
            .map_or(1, |observation| observation.sequence.saturating_add(1));
        let failure = InvocationFailure::new(
            ErrorClass::Unknown,
            false,
            "remote_outcome_uncertain",
            bounded(reason, 4_096),
            None,
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let terminal = InvocationTerminal::new(
            TerminalStatus::Uncertain,
            Vec::new(),
            Some(failure),
            None,
            record.request.selection.operation_contract().side_effect(),
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let event = InvocationEvent::new(
            record.request.request.invocation().clone(),
            sequence,
            InvocationEventKind::Terminal { terminal },
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        self.executions.append_observation(
            &record.owner_peer,
            &record.execution,
            PeerObservation {
                execution: record.execution.clone(),
                sequence,
                category: ObservationCategory::Uncertainty,
                event,
                observed_at_unix_ms: self.clock.now_unix_ms().max(1),
            },
        )?;
        Ok(())
    }
}

struct PeerStoreReporter {
    owner_peer: PeerId,
    execution: PeerExecutionId,
    executions: Arc<dyn PeerExecutionStore>,
    clock: Arc<dyn PeerClock>,
    lease_ms: u64,
    limits: milkdrift_peer_protocol::ExecutionLimits,
    deadline_unix_ms: u64,
}

impl AdapterReporter for PeerStoreReporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        if self.clock.now_unix_ms() > self.deadline_unix_ms {
            return Err(AdapterError::external_failure(
                "peer execution deadline elapsed",
            ));
        }
        let maximum = u64::from(self.limits.observations);
        if event.sequence() > maximum
            || (event.sequence() == maximum && event.kind().terminal().is_none())
        {
            return Err(AdapterError::external_failure(
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
                return Err(AdapterError::external_failure(
                    "peer terminal usage exceeds the accepted duration or cost quota",
                ));
            }
            let output_bytes = terminal.outputs().iter().try_fold(0_u64, |total, output| {
                output.size_bytes().and_then(|size| total.checked_add(size))
            });
            if output_bytes.is_none_or(|bytes| bytes > self.limits.artifact_bytes) {
                return Err(AdapterError::external_failure(
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
            .append_observation(
                &self.owner_peer,
                &self.execution,
                PeerObservation {
                    execution: self.execution.clone(),
                    sequence: event.sequence(),
                    category,
                    event,
                    observed_at_unix_ms: self.clock.now_unix_ms().max(1),
                },
            )
            .map(|_record| ())
            .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        if self.clock.now_unix_ms() > self.deadline_unix_ms {
            return Err(AdapterError::external_failure(
                "peer execution deadline elapsed",
            ));
        }
        self.executions
            .extend_lease(
                &self.owner_peer,
                &self.execution,
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

impl From<PeerStoreError> for PeerHttpError {
    fn from(error: PeerStoreError) -> Self {
        Self::Persistence(error.to_string())
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
            PeerArtifactError::Io(message) => Self::Persistence(message),
            PeerArtifactError::Unavailable => {
                Self::Persistence("artifact state unavailable".to_owned())
            }
        }
    }
}

struct DisabledArtifactStore;

impl PeerArtifactStore for DisabledArtifactStore {
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
